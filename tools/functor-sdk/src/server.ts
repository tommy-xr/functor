import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
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

/** A {@link FunctorClient} whose `functor` process is owned by the SDK.
 *
 * Supports `await using` for automatic shutdown:
 *
 * ```ts
 * await using game = await FunctorRunner.launch({
 *   gameDir: "examples/hello",
 *   functorLangPath: "examples/hello/game.fun",
 * });
 * ```
 */
export class FunctorRunner extends FunctorClient implements AsyncDisposable {
  private constructor(
    http: HttpClient,
    /** The debug server's actual TCP port (with dynamic allocation — the
     * default — this is the OS-assigned port parsed from the runtime's own
     * "listening" line, not the requested one). */
    public readonly port: number,
    private readonly child: ChildProcess | undefined,
    private readonly logLines: string[],
    /** Shared with the spawn-time 'error' listener (attached before the
     * runner exists), so a spawn failure is visible however early it lands. */
    private readonly spawnErrorRef: { current?: Error },
  ) {
    super(http);
  }

  /** Recent stdout/stderr from the spawned runtime (ring buffer). */
  logs(): string[] {
    return [...this.logLines];
  }

  /** Connect to an already-running debug runtime; does not own the process. */
  static async connect(baseUrl = "http://127.0.0.1:8077"): Promise<FunctorRunner> {
    const url = new URL(baseUrl);
    const port = Number(url.port) || (url.protocol === "https:" ? 443 : 80);
    const runner = new FunctorRunner(new HttpClient(baseUrl), port, undefined, [], {});
    await runner.state();
    return runner;
  }

  /** Spawn `functor -d <gameDir> run native` (which drives the desktop runtime's
   * `--functor-lang --debug-port` loop in-process) and wait until the render loop is
   * serving requests. Requires the CLI binary to already be built
   * (`cargo build --bin functor`). Post-E3 there is a single `functor` binary;
   * the game source is the project's `functor.json` entry. */
  static async launch(options: LaunchOptions): Promise<FunctorRunner> {
    // Port 0 (the default) = OS-assigned: the runtime binds a free port and
    // reports it on its "[debug-server] listening" line, which launch parses.
    // Collision-free under parallel test files / sessions by construction.
    const port = options.port ?? 0;
    // Resolve the game dir to an absolute path up front, so the spawn cwd and
    // repo-root discovery are consistent regardless of the caller's process cwd.
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
    if (options.headless && options.visible) {
      throw new Error("headless and visible are mutually exclusive");
    }
    // The game is an `.fun` source; `functor run native` reads the entry from
    // the project's functor.json (which the sample games set to game.fun).
    const gamePath = isAbsolute(options.functorLangPath)
      ? options.functorLangPath
      : resolve(options.functorLangPath);

    for (const [label, path] of [
      ["functor CLI", runnerBin],
      ["functor-lang game source", gamePath],
    ] as const) {
      if (!existsSync(path)) {
        throw new Error(
          `${label} not found at ${path}. Build it first ` +
            `(e.g. \`cargo build --bin functor\`).`,
        );
      }
    }

    // `functor run native` is project-oriented: it reads the entry (and detects
    // the Functor Lang language) from a `functor.json` in the game dir, NOT from an
    // explicit game-path — so the launched source is the functor.json entry.
    // Real games ship one; synthetic/temp game dirs (tests that just write a
    // bare `.fun`) may not — write a minimal config pointing at the entry so the
    // single `functor` binary can run them, matching what `functor-runner --functor-lang`
    // did directly pre-consolidation.
    const functorJson = join(gameDir, "functor.json");
    const wantEntry = relative(gameDir, gamePath) || "game.fun";
    // For a multi-entry project, the role selected out of `entries` via the
    // CLI's `--entry <name>` flag (placed before the subcommand, ahead of the
    // trailing runner args).
    const entryArgs: string[] = [];
    // `entry` names a role out of an `entries` map. A project without one has
    // no role to select, so honouring the request is impossible — say so
    // rather than silently launching its sole entry.
    const noRoles = () => {
      if (options.entry) {
        throw new Error(
          `the project in ${gameDir} declares no \`entries\` map, but launch ` +
            `requested entry "${options.entry}" — \`entry\` selects one of a ` +
            "multi-entry project's roles.",
        );
      }
    };
    if (existsSync(functorJson)) {
      // Don't clobber a real project's config — but since the CLI launches its
      // entry (not `functorLangPath`), verify they agree, or the SDK would silently run
      // a DIFFERENT game than the caller asked for.
      const cfg = JSON.parse(readFileSync(functorJson, "utf8"));
      // A role is either a bare path (roles-as-files) or an object naming the
      // file plus the inline `module`/`prefix` it resolves the contract in
      // (two roles in ONE file — examples/orbs).
      const entries =
        cfg.entries && typeof cfg.entries === "object"
          ? (cfg.entries as Record<string, string | { file?: string }>)
          : undefined;
      const fileOf = (role: string | { file?: string }): string =>
        typeof role === "string" ? role : String(role?.file ?? "");
      if (entries) {
        // Multi-entry project: select the role explicitly (the CLI's default
        // would be `client`). Same-file roles are indistinguishable by path,
        // so the caller must name one; otherwise the path picks it.
        const matches = Object.keys(entries).filter(
          (k) => resolve(gameDir, fileOf(entries[k])) === gamePath,
        );
        if (!options.entry && matches.length > 1) {
          throw new Error(
            `functor.json in ${gameDir} maps {${matches.join(", ")}} to the same ` +
              `file ${gamePath} (roles as inline modules of one file), so the path ` +
              "cannot say which you meant — pass `entry` to name the role.",
          );
        }
        const name = options.entry ?? matches[0];
        if (!name || !Object.keys(entries).includes(name)) {
          throw new Error(
            `functor.json in ${gameDir} declares entries ` +
              `{${Object.keys(entries).join(", ")}}, but launch requested ` +
              `${options.entry ? `entry "${options.entry}"` : `functorLangPath ${gamePath}`}, ` +
              `which matches none of them.`,
          );
        }
        // Whichever way the role was chosen, it must be the file the caller
        // asked for — otherwise the SDK would silently run a different source.
        if (resolve(gameDir, fileOf(entries[name])) !== gamePath) {
          throw new Error(
            `functor.json in ${gameDir} maps entry "${name}" to ` +
              `"${fileOf(entries[name])}", but launch requested functorLangPath ` +
              `${gamePath}. They must match.`,
          );
        }
        entryArgs.push("--entry", name);
      } else {
        noRoles();
        const cfgEntry: string = cfg.entry ?? "game.fun";
        if (resolve(gameDir, cfgEntry) !== gamePath) {
          throw new Error(
            `functor.json in ${gameDir} points at entry "${cfgEntry}", but launch ` +
              `requested functorLangPath ${gamePath}. They must match — \`functor run native\` ` +
              `launches the functor.json entry.`,
          );
        }
      }
    } else {
      noRoles();
      writeFileSync(functorJson, JSON.stringify({ language: "functor-lang", entry: wantEntry }));
    }

    // Forward the runtime flags after `--`; the CLI prepends `--functor-lang`
    // --game-path <entry>` and runs the desktop loop in-process.
    const runnerArgs = [
      "-d",
      gameDir,
      ...entryArgs,
      "run",
      "native",
      "--",
      "--debug-port",
      String(port),
    ];
    if (options.headless) {
      runnerArgs.push("--headless");
    } else if (!options.visible) {
      // Hidden by default: the window keeps its GL context (capture() works)
      // but is never shown and never steals focus or the cursor.
      runnerArgs.push("--hidden");
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

    // Resolved with the ACTUAL bound port when the child's own debug server
    // reports in. Watching the stream (not polling the ring buffer) can't
    // lose the line, and gating readiness on it means launch never adopts a
    // FOREIGN process that happens to answer on the requested port.
    let reportListening: (port: number) => void;
    const listening = new Promise<number>((r) => {
      reportListening = r;
    });

    // Decode stdout/stderr line-by-line, holding any trailing partial line (and
    // any split multibyte char) until the rest arrives, so log lines — and the
    // listening/panic lines the SDK watches for — aren't fragmented across
    // chunks. PER STREAM: sharing one decoder/residual would let interleaved
    // stdout/stderr chunks splice each other's lines (the listening line lands
    // on stderr while the runtime chats on stdout).
    const makeCapture = () => {
      const decoder = new StringDecoder("utf8");
      let residual = "";
      return (chunk: Buffer) => {
        const lines = (residual + decoder.write(chunk)).split("\n");
        residual = lines.pop() ?? "";
        for (const line of lines) {
          logLines.push(line);
          if (logLines.length > MAX_LOG_LINES) logLines.shift();
          if (options.echoLogs) process.stderr.write(`[functor] ${line}\n`);
          const bound = parseListeningPort(line);
          if (bound !== undefined) reportListening(bound);
        }
      };
    };
    child.stdout?.on("data", makeCapture());
    child.stderr?.on("data", makeCapture());

    // A spawn failure (e.g. EACCES, ENOMEM — not caught by the existsSync checks
    // above, which are also TOCTOU) emits 'error' with no other signal; without
    // a listener Node rethrows it as a fatal uncaught exception. Record it so
    // readiness fails fast. ('error' is emitted asynchronously, so attaching
    // here — before the first await — cannot miss it.)
    const spawnErrorRef: { current?: Error } = {};
    child.once("error", (err) => {
      spawnErrorRef.current = err;
    });

    const timeoutMs = options.launchTimeoutMs ?? 60_000;
    const deadline = Date.now() + timeoutMs;
    try {
      const boundPort = await waitForListeningLine(child, spawnErrorRef, listening, deadline);
      const runner = new FunctorRunner(
        new HttpClient(`http://127.0.0.1:${boundPort}`),
        boundPort,
        child,
        logLines,
        spawnErrorRef,
      );
      await runner.waitUntilReady(Math.max(1, deadline - Date.now()));
      return runner;
    } catch (error) {
      // Covers both phases: runner.shutdown() is shutdownChild(child), so the
      // pre-runner path needs no separate wrapper.
      await shutdownChild(child);
      throw new Error(
        `functor failed to start: ${error}\nRecent output:\n${formatCrashOutput(logLines)}`,
      );
    }
  }

  private async waitUntilReady(timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      if (this.spawnErrorRef.current) {
        throw this.spawnErrorRef.current;
      }
      if (this.child && hasExited(this.child)) {
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
    if (this.child !== undefined) {
      await shutdownChild(this.child);
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.shutdown();
  }
}

/** SIGTERM the child, escalating to SIGKILL. Shared by `shutdown()` and the
 * pre-runner launch failure path (listening never reported). */
async function shutdownChild(child: ChildProcess): Promise<void> {
  if (hasExited(child)) {
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

/** The bound port from a runtime `[debug-server] listening on http://…:PORT`
 * line, or undefined for any other line. Tolerant of IPv6 bracket rendering
 * and a trailing `\r` (a Windows-piped runtime would leave one after the
 * `\n` split). */
export function parseListeningPort(line: string): number | undefined {
  const bound = line.match(/\[debug-server\] listening on http:\/\/.*:(\d+)\s*$/);
  return bound ? Number(bound[1]) : undefined;
}

/** Wait for the child's own `[debug-server] listening on …` line and return
 * the bound port it reports. This is the only trustworthy source of the port
 * (with port 0 it's OS-assigned), and requiring OUR child to report it means
 * a foreign process squatting a requested port can never be silently adopted
 * as the launched game — the child exits with "Address already in use"
 * instead, which surfaces here as an early exit. */
function waitForListeningLine(
  child: ChildProcess,
  spawnErrorRef: { current?: Error },
  listening: Promise<number>,
  deadline: number,
): Promise<number> {
  return new Promise<number>((resolvePort, reject) => {
    let settled = false;
    const onExit = () =>
      fail(
        new Error(
          `process exited early (code ${child.exitCode}, signal ${child.signalCode})`,
        ),
      );
    const cleanup = () => {
      settled = true;
      clearTimeout(timer);
      child.removeListener("exit", onExit);
      child.removeListener("error", fail);
    };
    const fail = (error: Error) => {
      if (settled) return;
      cleanup();
      reject(error);
    };
    const timer = setTimeout(
      () =>
        fail(
          new Error(
            "debug server never reported listening (no `[debug-server] listening on …` line)",
          ),
        ),
      Math.max(1, deadline - Date.now()),
    );
    // Pre-checks: an 'exit'/'error' that already fired will not fire again
    // for the listeners below.
    if (spawnErrorRef.current) {
      fail(spawnErrorRef.current);
      return;
    }
    if (hasExited(child)) {
      onExit();
      return;
    }
    child.once("exit", onExit);
    child.once("error", fail);
    void listening.then((port) => {
      if (settled) return;
      cleanup();
      resolvePort(port);
    });
  });
}

function runnerExe(): string {
  return process.platform === "win32" ? "functor.exe" : "functor";
}

/** A child has exited if it has either an exit code or a terminating signal
 * (a signal-killed process reports `exitCode === null`, `signalCode` set). */
function hasExited(child: ChildProcess): boolean {
  return child.exitCode !== null || child.signalCode !== null;
}
