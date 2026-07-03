import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { StringDecoder } from "node:string_decoder";

import { HttpClient } from "./client.js";
import { FunctorClient } from "./game.js";
import type { LaunchOptions, WaitForOptions } from "./types.js";

/** Resolve once a TCP connection to `host:port` succeeds, or throw on timeout.
 * Useful to wait for a game's `Sub.listen` socket to be bound before launching
 * clients (the debug `/state` readiness only proves the render loop is running,
 * not that a game-level listener has bound yet). */
export async function waitForPort(
  host: string,
  port: number,
  opts: WaitForOptions = {},
): Promise<void> {
  const timeoutMs = opts.timeoutMs ?? 10_000;
  const intervalMs = opts.intervalMs ?? 100;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await tryConnect(host, port)) {
      return;
    }
    if (Date.now() >= deadline) {
      const what = opts.description ? ` (${opts.description})` : "";
      throw new Error(`${host}:${port} not accepting connections after ${timeoutMs}ms${what}`);
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
}

function tryConnect(host: string, port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host, port });
    const settle = (ok: boolean) => {
      socket.destroy();
      resolve(ok);
    };
    socket.once("connect", () => settle(true));
    socket.once("error", () => settle(false));
    socket.setTimeout(1_000, () => settle(false));
  });
}

/** Walk up from a directory until a cargo workspace root is found. */
export function findRepoRoot(startDir: string): string | undefined {
  let dir = startDir;
  for (;;) {
    const manifest = join(dir, "Cargo.toml");
    if (
      existsSync(manifest) &&
      readFileSync(manifest, "utf8").includes("[workspace]")
    ) {
      return dir;
    }
    const parent = dirname(dir);
    if (parent === dir) {
      return undefined;
    }
    dir = parent;
  }
}

/** The platform-specific default game dylib filename produced by build-native. */
export function defaultDylibName(): string {
  switch (process.platform) {
    case "darwin":
      return "libgame_native.dylib";
    case "win32":
      return "game_native.dll";
    default:
      return "libgame_native.so";
  }
}

const MAX_LOG_LINES = 2000;
const MAX_ERROR_LOG_LINES = 120;

/** Pick the most useful slice of runtime output for an error message: from the
 * last panic line onward (with a little context), else the last ~30 lines. */
export function formatCrashOutput(logLines: string[]): string {
  const panicIndex = logLines.findLastIndex((line) =>
    line.includes("panicked at"),
  );
  const start = panicIndex >= 0 ? Math.max(0, panicIndex - 2) : -30;
  return logLines.slice(start).slice(0, MAX_ERROR_LOG_LINES).join("\n");
}

/** A {@link FunctorClient} whose `functor-runner` process is owned by the SDK.
 *
 * Supports `await using` for automatic shutdown:
 *
 * ```ts
 * await using game = await FunctorRunner.launch({ gameDir: "examples/hello" });
 * ```
 */
export class FunctorRunner extends FunctorClient implements AsyncDisposable {
  /** Set if the spawned child emitted an 'error' (e.g. it couldn't be spawned). */
  private spawnError?: Error;

  private constructor(
    http: HttpClient,
    private readonly child: ChildProcess | undefined,
    private readonly logLines: string[],
  ) {
    super(http);
  }

  /** Recent stdout/stderr from the spawned runtime (ring buffer). */
  logs(): string[] {
    return [...this.logLines];
  }

  /** Connect to an already-running debug runtime; does not own the process. */
  static async connect(baseUrl = "http://127.0.0.1:8077"): Promise<FunctorRunner> {
    const runner = new FunctorRunner(new HttpClient(baseUrl), undefined, []);
    await runner.state();
    return runner;
  }

  /** Spawn `functor-runner --debug-port` against a built game dylib and wait
   * until the render loop is serving requests. Requires the runner binary and
   * the game dylib to already be built. */
  static async launch(options: LaunchOptions): Promise<FunctorRunner> {
    const port = options.port ?? 8077;
    // Resolve the game dir to an absolute path up front, so the dylib path, the
    // spawn cwd, and repo-root discovery are all consistent regardless of the
    // caller's process cwd.
    const gameDir = isAbsolute(options.gameDir)
      ? options.gameDir
      : resolve(options.gameDir);
    const repoRoot = options.repoRoot ?? findRepoRoot(gameDir);
    if (repoRoot === undefined) {
      throw new Error(
        "Could not find cargo workspace root; pass repoRoot explicitly",
      );
    }

    const runnerBin =
      options.runnerBin ?? join(repoRoot, "target", "debug", runnerExe());
    // An .mle source runs through the interpreter; otherwise a built dylib.
    const gamePath = options.mlePath
      ? isAbsolute(options.mlePath)
        ? options.mlePath
        : resolve(options.mlePath)
      : (options.dylibPath ??
        join(gameDir, "build-native", "target", "debug", defaultDylibName()));

    for (const [label, path] of [
      ["functor-runner", runnerBin],
      [options.mlePath ? "mle game source" : "game dylib", gamePath],
    ] as const) {
      if (!existsSync(path)) {
        throw new Error(
          `${label} not found at ${path}. Build it first ` +
            `(e.g. \`functor -d ${options.gameDir} build native\` and ` +
            `\`cargo build --bin functor-runner\`).`,
        );
      }
    }

    const runnerArgs = ["--game-path", gamePath, "--debug-port", String(port)];
    if (options.mlePath) {
      runnerArgs.push("--mle");
    }
    if (options.headless) {
      runnerArgs.push("--headless");
    }

    const logLines: string[] = [];
    const child = spawn(runnerBin, runnerArgs, {
        cwd: gameDir,
        env: {
          ...process.env,
          RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? "1",
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );

    // Decode stdout/stderr line-by-line, holding any trailing partial line (and
    // any split multibyte char) until the rest arrives, so log lines — and the
    // panic line formatCrashOutput looks for — aren't fragmented across chunks.
    const decoder = new StringDecoder("utf8");
    let residual = "";
    const capture = (chunk: Buffer) => {
      const lines = (residual + decoder.write(chunk)).split("\n");
      residual = lines.pop() ?? "";
      for (const line of lines) {
        logLines.push(line);
        if (logLines.length > MAX_LOG_LINES) logLines.shift();
        if (options.echoLogs) process.stderr.write(`[functor-runner] ${line}\n`);
      }
    };
    child.stdout?.on("data", capture);
    child.stderr?.on("data", capture);

    const runner = new FunctorRunner(
      new HttpClient(`http://127.0.0.1:${port}`),
      child,
      logLines,
    );
    // A spawn failure (e.g. EACCES, ENOMEM — not caught by the existsSync checks
    // above, which are also TOCTOU) emits 'error' with no other signal; without
    // a listener Node rethrows it as a fatal uncaught exception. Record it so
    // readiness fails fast. ('error' is emitted asynchronously, so attaching
    // here — before the first await — cannot miss it.)
    child.once("error", (err) => {
      runner.spawnError = err;
    });

    try {
      await runner.waitUntilReady(options.launchTimeoutMs ?? 60_000);
    } catch (error) {
      await runner.shutdown();
      throw new Error(
        `functor-runner failed to start: ${error}\nRecent output:\n${formatCrashOutput(logLines)}`,
      );
    }

    return runner;
  }

  private async waitUntilReady(timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      if (this.spawnError) {
        throw this.spawnError;
      }
      if (
        this.child &&
        (this.child.exitCode !== null || this.child.signalCode !== null)
      ) {
        throw new Error(
          `process exited early (code ${this.child.exitCode}, signal ${this.child.signalCode})`,
        );
      }
      try {
        // /state round-trips through the per-frame request channel, so it only
        // succeeds once the render loop is actually running (the HTTP thread
        // starts first and would answer too early on its own).
        await this.state();
        return;
      } catch {
        if (Date.now() >= deadline) {
          throw new Error(`runtime not ready after ${timeoutMs}ms`);
        }
        await new Promise((r) => setTimeout(r, 500));
      }
    }
  }

  /** Stop the spawned runtime (SIGTERM, escalating to SIGKILL). No-op if this
   * runner connected to an externally-owned process. */
  async shutdown(): Promise<void> {
    const child = this.child;
    if (child === undefined || hasExited(child)) {
      return;
    }
    await new Promise<void>((settle) => {
      // Re-check inside the promise: the child may have exited between the guard
      // above and attaching the listener — otherwise we'd await an 'exit' that
      // already fired and hang forever (a signal-killed child has exitCode null).
      if (hasExited(child)) {
        settle();
        return;
      }
      const killTimer = setTimeout(() => child.kill("SIGKILL"), 5_000);
      child.once("exit", () => {
        clearTimeout(killTimer);
        settle();
      });
      child.kill("SIGTERM");
    });
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.shutdown();
  }
}

function runnerExe(): string {
  return process.platform === "win32" ? "functor-runner.exe" : "functor-runner";
}

/** A child has exited if it has either an exit code or a terminating signal
 * (a signal-killed process reports `exitCode === null`, `signalCode` set). */
function hasExited(child: ChildProcess): boolean {
  return child.exitCode !== null || child.signalCode !== null;
}
