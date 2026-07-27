// `functor mcp` E2E: prove the MCP server really drives a game.
//
// This is the agent-verifiability proof for the MCP surface (docs/mcp.md): it
// speaks raw JSON-RPC 2.0 over the server's stdio — no MCP client library — so
// what is asserted is exactly what a coding agent's client would see.
//
//   1. initialize / notifications/initialized / tools/list, asserting the whole
//      documented tool surface is advertised.
//   2. launch_game on examples/counter in HEADLESS mode (no GL, so this runs
//      anywhere CI does), and assert /state's structured `model_json` arrives.
//   3. pause, then step twice — asserting the frame advances and that `step`
//      only returns once the queued steps have landed (pending_steps == 0).
//   4. send_input: counter's model reacts to a UI click (its `update` handles
//      Inc), so a `ui_event` on slot 0 must increment `count` after a step.
//      A `key` event is also injected to exercise that shape end to end.
//   5. stop_game, then a clean shutdown of the server itself.
//
// Run manually (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/mcp-server.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;

/** Every tool the server must advertise (docs/mcp.md). */
const EXPECTED_TOOLS = [
  "launch_game",
  "connect_game",
  "list_sessions",
  "stop_game",
  "get_state",
  "get_scene",
  "get_trace",
  "capture_frame",
  "send_input",
  "pause",
  "step",
  "resume",
  "rewind",
  "reload_source",
  "reload_project",
];

/** A line-delimited JSON-RPC client over a child's stdio. */
class Rpc {
  constructor(proc) {
    this.proc = proc;
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = "";
    this.stderr = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => this.#onData(chunk));
    proc.stderr.on("data", (chunk) => (this.stderr += chunk));
  }

  #onData(chunk) {
    this.buffer += chunk;
    let index;
    while ((index = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, index).trim();
      this.buffer = this.buffer.slice(index + 1);
      if (!line) continue;
      const message = JSON.parse(line);
      const waiter = this.pending.get(message.id);
      if (waiter) {
        this.pending.delete(message.id);
        clearTimeout(waiter.timer);
        message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
      }
    }
  }

  notify(method, params) {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  request(method, params) {
    const id = this.nextId++;
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out\nstderr:\n${this.stderr}`));
      }, 60000);
      this.pending.set(id, { resolve, reject, timer });
    });
  }

  /** Call a tool and return its single text content block, parsed if it is JSON. */
  async call(name, args = {}) {
    const result = await this.request("tools/call", { name, arguments: args });
    const text = result.content.map((block) => block.text ?? "").join("");
    if (result.isError) throw new Error(`${name} failed: ${text}`);
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`  ${ok ? "✓" : "✗"} ${what}`);
  if (!ok) failures.push(what);
};

const proc = spawn(BIN, ["mcp"], { cwd: ROOT, stdio: ["pipe", "pipe", "pipe"] });
const rpc = new Rpc(proc);
let session;

try {
  console.log("▸ the MCP handshake");
  const initialized = await rpc.request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "functor-e2e", version: "0" },
  });
  rpc.notify("notifications/initialized", {});
  check(typeof initialized.protocolVersion === "string", `the server negotiated ${initialized.protocolVersion}`);
  check(initialized.capabilities?.tools !== undefined, "the server advertises the tools capability");

  const { tools } = await rpc.request("tools/list", {});
  const names = tools.map((tool) => tool.name).sort();
  const missing = EXPECTED_TOOLS.filter((name) => !names.includes(name));
  check(missing.length === 0, `tools/list advertises all ${EXPECTED_TOOLS.length} tools${missing.length ? ` (missing ${missing})` : ""}`);
  check(
    tools.every((tool) => (tool.description ?? "").length > 20),
    "every tool carries a real description (the agent-facing docs)",
  );

  console.log("\n▸ launching a game headlessly");
  const launched = await rpc.call("launch_game", { dir: "examples/counter", mode: "headless" });
  session = launched.session;
  check(/^s\d+$/.test(session), `launch_game returned a session id (${session})`);
  check(launched.discovery?.service === "functor debug runtime", "the child answered the debug-runtime discovery endpoint");
  check(launched.owned === true, "the session is owned (this server spawned it)");

  const listed = await rpc.call("list_sessions");
  check(listed.sessions.length === 1 && listed.sessions[0].alive === true, "list_sessions reports it alive");

  console.log("\n▸ the structured model view (protocol v4)");
  const state = await rpc.call("get_state", { session });
  check(
    state.model_json !== null && typeof state.model_json === "object",
    "get_state carries model_json as a structured object",
  );
  check(state.model_json?.count === 0, `the counter starts at 0 (model_json.count = ${state.model_json?.count})`);

  console.log("\n▸ pause + step is a deterministic clock");
  await rpc.call("pause", { session });
  const paused = await rpc.call("get_state", { session });
  const first = await rpc.call("step", { session });
  const second = await rpc.call("step", { session });
  check(first.frame > paused.frame, `the first step advanced the frame (${paused.frame} → ${first.frame})`);
  check(second.frame > first.frame, `the second step advanced it again (→ ${second.frame})`);
  check(second.pending_steps === 0, "step returns only once its queued steps have landed");
  check(
    Math.abs(second.tts - first.tts - 0.016) < 1e-4,
    `each step advanced time by its dts (tts ${first.tts.toFixed(3)} → ${second.tts.toFixed(3)})`,
  );

  console.log("\n▸ injected input reaches the game");
  // counter's `update` handles Inc, delivered by clicking the button — slot 0,
  // the first interactive widget in its `ui` tree.
  const before = (await rpc.call("get_state", { session })).model_json.count;
  await rpc.call("send_input", { session, command: { type: "ui_event", slot: 0, kind: "Clicked" } });
  const clicked = await rpc.call("step", { session });
  check(clicked.model_json.count === before + 1, `a ui_event click incremented the model (${before} → ${clicked.model_json.count})`);

  // A key event exercises the keyboard shape end to end. counter has no `input`
  // hook, so the assertion is that the runtime accepted and sampled it — not a
  // model change it cannot make.
  await rpc.call("send_input", { session, command: { type: "key", key: "w", down: true } });
  const held = await rpc.call("step", { session });
  check(held.input.held_keys.includes("W"), `an injected key is held level state (held_keys = ${JSON.stringify(held.input.held_keys)})`);

  console.log("\n▸ errors are teaching errors, not silence");
  let capture = null;
  try {
    await rpc.call("capture_frame", { session });
  } catch (error) {
    capture = error.message;
  }
  check(capture !== null && /headless/.test(capture), "capture_frame on a headless session explains why there are no pixels");

  let unknown = null;
  try {
    await rpc.call("get_state", { session: "s99" });
  } catch (error) {
    unknown = error.message;
  }
  check(unknown !== null && unknown.includes(session), "an unknown session id names the sessions that do exist");

  console.log("\n▸ shutdown");
  const stopped = await rpc.call("stop_game", { session });
  session = null;
  check(/killed/.test(stopped), "stop_game killed the launched runtime");
  const empty = await rpc.call("list_sessions");
  check(empty.sessions.length === 0, "the registry is empty again");
} finally {
  if (session) {
    try {
      await rpc.call("stop_game", { session });
    } catch {
      // best effort
    }
  }
  proc.stdin.end();
  const exit = await new Promise((resolve) => {
    proc.on("exit", resolve);
    setTimeout(() => resolve("timeout"), 5000);
  });
  check(exit === 0, `the server exited cleanly on stdin close (${exit})`);
}

if (failures.length) {
  console.error(`\n✗ mcp-server failed: ${failures.join(", ")}`);
  if (rpc.stderr) console.error(`\nserver stderr:\n${rpc.stderr}`);
  process.exit(1);
}
console.log("\n✓ mcp-server passed");
