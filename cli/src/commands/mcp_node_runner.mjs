// Playwright-style code host for `run_game_code_unsafe`.
//
// This process is deliberately NOT a security sandbox. Submitted code can use
// Node globals, import modules, access files, and start processes with the same
// authority as this child. The process boundary provides direct-child
// lifecycle, timeout, and crash containment for the Rust MCP server; it does
// not confine code or track subprocesses that submitted code starts.
//
// stdout is reserved as a newline-delimited RPC channel to the Rust parent.
// Ordinary `process.stdout.write` calls are redirected into bounded log
// records below. Arbitrary trusted code can still bypass that redirection, so
// the resulting trace is observability, not a security attestation.

import readline from "node:readline";

const rawWrite = process.stdout.write.bind(process.stdout);
const pending = new Map();
let nextCallId = 1;
let started = false;
let finished = false;

function write(message, callback) {
  rawWrite(`${JSON.stringify(message)}\n`, callback);
}

function finish(message, exitCode) {
  if (finished) return;
  finished = true;
  write(message, () => process.exit(exitCode));
}

function errorRecord(error) {
  const value = error instanceof Error ? error : new Error(String(error));
  return {
    name: String(value.name || "Error").slice(0, 256),
    message: String(value.message || value).slice(0, 16 * 1024),
    stack:
      typeof value.stack === "string"
        ? value.stack.slice(0, 32 * 1024)
        : undefined,
  };
}

function logText(values) {
  return values
    .map((value) => {
      if (typeof value === "string") return value;
      try {
        const encoded = JSON.stringify(value);
        return encoded === undefined ? String(value) : encoded;
      } catch {
        return String(value);
      }
    })
    .join(" ")
    .slice(0, 64 * 1024);
}

for (const level of ["log", "info", "warn", "error", "debug"]) {
  console[level] = (...values) => {
    write({ type: "log", level, text: logText(values) });
  };
}

process.stdout.write = (chunk, encoding, callback) => {
  if (typeof encoding === "function") {
    callback = encoding;
    encoding = undefined;
  }
  let text;
  try {
    text =
      Buffer.isBuffer(chunk) || chunk instanceof Uint8Array
        ? Buffer.from(chunk).toString(
            typeof encoding === "string" ? encoding : "utf8",
          )
        : String(chunk);
  } catch {
    text = String(chunk);
  }
  write(
    { type: "log", level: "stdout", text: logText([text]) },
    typeof callback === "function" ? callback : undefined,
  );
  return true;
};

function canonicalKeyName(key) {
  if (/^[0-9]$/.test(key)) return `Num${key}`;
  return key.length === 0
    ? key
    : key[0].toUpperCase() + key.slice(1).toLowerCase();
}

function revive(value) {
  if (
    value !== null &&
    typeof value === "object" &&
    typeof value.$functor_buffer_base64 === "string"
  ) {
    return Buffer.from(value.$functor_buffer_base64, "base64");
  }
  return value;
}

function call(method, args = []) {
  const id = nextCallId++;
  write({ type: "call", id, method, args });
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

const DEFAULT_STEP_DT = 1 / 60;

function createGame() {
  const game = {
    DEFAULT_STEP_DT,

    state: () => call("state"),
    scene: () => call("scene"),
    trace: () => call("trace"),
    capture: () => call("capture"),

    reloadSource: (source) => call("reloadSource", [source]),
    reloadProject: (files) => call("reloadProject", [files]),
    loadProject: (files) => call("loadProject", [files]),
    reloadAsset: (path, bytes) =>
      call("reloadAsset", [path, Buffer.from(bytes).toString("base64")]),
    async reloadAssets(files) {
      for (const [path, bytes] of files) {
        await game.reloadAsset(path, bytes);
      }
      return call("syncAssets", [files.map(([path]) => path)]);
    },
    rewind: (frame) => call("rewind", [frame]),

    input: (command) => call("input", [command]),
    key: (key, down) => call("key", [key, down]),
    keyDown: (key) => call("keyDown", [key]),
    keyUp: (key) => call("keyUp", [key]),
    async pressKey(key, dts = DEFAULT_STEP_DT) {
      let actionError;
      try {
        await game.keyDown(key);
        await game.step(dts);
      } catch (error) {
        actionError = error;
      }
      try {
        await game.keyUp(key);
      } catch (releaseError) {
        if (actionError !== undefined) {
          throw new Error(
            `${String(actionError)}; best-effort key release also failed: ${String(releaseError)}`,
          );
        }
        throw releaseError;
      }
      if (actionError !== undefined) throw actionError;
    },
    mouseMove: (x, y) => call("mouseMove", [x, y]),
    mouseWheel: (delta) => call("mouseWheel", [delta]),
    mouseButton: (button, down) => call("mouseButton", [button, down]),
    mouseDown: (button = "left") => call("mouseDown", [button]),
    mouseUp: (button = "left") => call("mouseUp", [button]),
    xr: (sample) => call("xr", [sample]),
    xrClear: () => call("xrClear"),
    uiClick: (slot) => call("uiClick", [slot]),

    pause: (tts) =>
      tts === undefined ? call("pause") : call("pause", [tts]),
    step: (dts = DEFAULT_STEP_DT) => call("step", [dts]),
    stepFrames: (frames, dts = DEFAULT_STEP_DT) =>
      call("stepFrames", [frames, dts]),
    resume: () => call("resume"),

    async heldKeys() {
      return (await game.state()).input.held_keys;
    },
    async isKeyDown(key) {
      const wanted = canonicalKeyName(key);
      return (await game.heldKeys()).some((held) => held === wanted);
    },
    async xrInput() {
      return (await game.state()).input.xr;
    },

    async waitForState(predicate, options = {}) {
      if (typeof predicate !== "function") {
        throw new TypeError("waitForState predicate must be a function");
      }
      const timeoutMs = options.timeoutMs ?? 10_000;
      const intervalMs = options.intervalMs ?? 100;
      if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
        throw new TypeError("waitForState timeoutMs must be a non-negative number");
      }
      if (!Number.isFinite(intervalMs) || intervalMs < 0) {
        throw new TypeError("waitForState intervalMs must be a non-negative number");
      }
      const deadline = Date.now() + timeoutMs;
      let lastError;
      for (;;) {
        try {
          const state = await game.state();
          lastError = undefined;
          if (await predicate(state)) return state;
        } catch (error) {
          lastError = error;
        }
        const remaining = deadline - Date.now();
        if (remaining <= 0) {
          const what = options.description
            ? ` waiting for ${options.description}`
            : "";
          const cause =
            lastError === undefined ? "" : ` (last error: ${String(lastError)})`;
          throw new Error(
            `waitForState timed out after ${timeoutMs}ms${what}${cause}`,
          );
        }
        await new Promise((resolve) =>
          setTimeout(resolve, Math.min(intervalMs, remaining)),
        );
      }
    },

    async stepUntil(predicate, options = {}) {
      if (typeof predicate !== "function") {
        throw new TypeError("stepUntil predicate must be a function");
      }
      const maxFrames = options.maxFrames ?? 600;
      const dts = options.dts ?? DEFAULT_STEP_DT;
      if (
        !Number.isInteger(maxFrames) ||
        maxFrames < 0 ||
        maxFrames > 10_000
      ) {
        throw new TypeError(
          `stepUntil maxFrames must be an integer between 0 and 10000, got ${maxFrames}`,
        );
      }
      if (!Number.isFinite(dts) || dts <= 0) {
        throw new TypeError(
          `stepUntil dts must be a finite positive number, got ${dts}`,
        );
      }

      let state = await game.state();
      if (await predicate(state)) return state;
      for (let frame = 0; frame < maxFrames; frame++) {
        await game.step(dts);
        state = await game.state();
        if (await predicate(state)) return state;
      }
      const what = options.description
        ? ` waiting for ${options.description}`
        : "";
      throw new Error(`stepUntil exhausted ${maxFrames} frames${what}`);
    },
  };
  return Object.freeze(game);
}

function jsonReturnValue(value) {
  if (value === undefined) return null;
  const encoded = JSON.stringify(value);
  if (encoded === undefined) {
    throw new TypeError("code return value is not JSON-serializable");
  }
  return JSON.parse(encoded);
}

async function run(code) {
  try {
    const program = (0, eval)(`(${code}\n)`);
    if (typeof program !== "function") {
      throw new TypeError(
        "code must evaluate to a function, for example: async (game) => { ... }",
      );
    }
    const value = await program(createGame());
    finish({ type: "complete", ok: true, value: jsonReturnValue(value) }, 0);
  } catch (error) {
    finish({ type: "complete", ok: false, error: errorRecord(error) }, 1);
  }
}

const major = Number(process.versions.node.split(".")[0]);
if (!Number.isInteger(major) || major < 20) {
  finish(
    {
      type: "fatal",
      error: `run_game_code_unsafe requires Node.js 20 or newer; found ${process.version}`,
    },
    1,
  );
} else {
  write({ type: "ready", node_version: process.version });
}

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

lines.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    finish(
      { type: "fatal", error: `parent sent invalid JSON: ${String(error)}` },
      1,
    );
    return;
  }

  if (message.type === "result") {
    const waiter = pending.get(message.id);
    if (waiter === undefined) {
      finish(
        { type: "fatal", error: `parent answered unknown call ${message.id}` },
        1,
      );
      return;
    }
    pending.delete(message.id);
    if (message.ok) {
      waiter.resolve(revive(message.value));
    } else {
      const error = new Error(
        message.error?.message ?? String(message.error ?? "game call failed"),
      );
      error.name = message.error?.name ?? "FunctorSdkError";
      waiter.reject(error);
    }
    return;
  }

  if (message.type === "run" && !started) {
    started = true;
    void run(message.code);
    return;
  }

  finish(
    {
      type: "fatal",
      error: `unexpected parent message ${String(message.type)}`,
    },
    1,
  );
});

lines.on("close", () => {
  if (!finished) {
    finish(
      { type: "fatal", error: "parent closed stdin before code completed" },
      1,
    );
  }
});
