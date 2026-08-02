// `functor mcp` E2E: the MULTIPLAYER surface — ordered stepping, quiescence,
// and a multi-entry project that survives `save_project`.
//
// It runs `examples/orbs` for real: one authoritative `server` role plus two
// `client` roles — three separate runtimes talking over a localhost socket.
// Both roles are inline modules of ONE file, so each is named with `entry`
// rather than inferred from a path, which is also the shape that made
// `save_project` corrupt the manifest.
// What it proves (docs/mcp.md, "Running a multiplayer session"):
//
//   1. `step_all` steps a group STRICTLY SEQUENTIALLY in the caller's declared
//      order — producer → authority → observer — and returns each session's
//      post-step summary in that order, each with its steps fully landed.
//   2. That ordering is REPRODUCIBLE: the whole run is performed twice, from
//      scratch, and the authority's per-round world trace must be identical.
//      (Stepping the same group concurrently is a race between a packet and
//      its receiver's step, which is why the tool has an order at all.)
//   3. `pending_net` is the quiescence signal a baseline needs, and
//      `model_revision` moves on network folds that no clock step caused —
//      pause freezes the clock, not the network.
//   4. `save_project` preserves a multi-entry `functor.json`. Reconstructing
//      one loses the roles: the saved directory builds green and then runs the
//      wrong entry.
//
// Run manually (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/mcp-step-all.mjs
//
// Set FUNCTOR_BIN when the build uses a shared CARGO_TARGET_DIR.
import { spawn } from "node:child_process";
import { connect } from "node:net";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = process.env.FUNCTOR_BIN ?? `${ROOT}target/debug/functor`;
const DIR = "examples/orbs";
/** The ws port `examples/orbs` binds (`let bind = "127.0.0.1:9101"`). */
const WS_PORT = 9101;
/** Lockstep rounds per run. Enough to see the world move, short enough to run twice. */
const ROUNDS = 12;
const DTS = 0.016;

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
    const result = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out\nstderr:\n${this.stderr}`));
      }, 60000);
      this.pending.set(id, { resolve, reject, timer });
    });
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return result;
  }

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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Poll `get_state` until `predicate` holds, or fail loudly with the last state. */
async function waitForState(rpc, session, predicate, what, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  for (;;) {
    last = await rpc.call("get_state", { session });
    if (predicate(last)) return last;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${what}; last state: ${JSON.stringify(last.model)}`);
    }
    await sleep(100);
  }
}

/** Wait until something is listening on a localhost TCP port.
 *
 * An orbs client dials ONCE with no retry, so a client launched before the
 * server's `Sub.listen` has bound lands in "error" and never converges. The
 * SDK's own multiplayer test gates on the same port for the same reason. */
async function waitForPort(port, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const open = await new Promise((resolve) => {
      const socket = connect({ host: "127.0.0.1", port });
      socket.once("connect", () => (socket.destroy(), resolve(true)));
      socket.once("error", () => (socket.destroy(), resolve(false)));
    });
    if (open) return;
    if (Date.now() > deadline) throw new Error(`nothing listening on 127.0.0.1:${port}`);
    await sleep(100);
  }
}

/** The authority's ships: `world.pilots` is a list of `{ ship, intent }`. */
const shipsOf = (state) => state.model.world.pilots.map((p) => p.ship);

/** Every ship's position keyed by pid, the baseline a trace is measured from. */
const positionsOf = (ships) => new Map(ships.map((s) => [s.pid, { x: s.x, y: s.y }]));

/** The authority's world as a comparable string: each ship's DISPLACEMENT
 * from the paused baseline, in pid order.
 *
 * Displacement rather than absolute position, because the absolute one is not
 * a property of the stepping at all: it depends on how many wall-clock frames
 * ran between launch and pause, which no driver controls. What the ordering
 * law claims — and what this compares — is that N ordered rounds move the
 * world by exactly the same amount every time. */
const traceOf = (ships, base) =>
  [...ships]
    .sort((a, b) => a.pid - b.pid)
    .map((s) => {
      const from = base.get(s.pid);
      if (!from) return `${s.pid}@unseated-at-baseline`;
      return `${s.pid}@${(s.x - from.x).toFixed(6)},${(s.y - from.y).toFixed(6)}`;
    })
    .join(" ");

/** One complete multiplayer run: launch, settle, pause, step in lockstep. */
async function lockstepRun(rpc, label) {
  // Server FIRST, and only launch a client once its listener is really bound:
  // an orbs client dials once and never retries.
  const server = (await rpc.call("launch_game", { dir: DIR, entry: "server", mode: "headless" })).session;
  await waitForPort(WS_PORT);

  // One client at a time, each seated before the next launches, so the pids
  // the server hands out are the launch order rather than a race — the trace
  // below is per-pid, and "which client is P0" must not vary between runs.
  const c1 = (await rpc.call("launch_game", { dir: DIR, entry: "client", mode: "headless" })).session;
  await waitForState(rpc, c1, (s) => s.model.myPid === 0, "client 1 to be seated as P0");
  const c2 = (await rpc.call("launch_game", { dir: DIR, entry: "client", mode: "headless" })).session;
  await waitForState(rpc, c2, (s) => s.model.myPid === 1, "client 2 to be seated as P1");

  // Both clients drawing the server's world: the handshake is over and this is
  // a three-party session rather than a race we got lucky in.
  for (const [name, session] of [["1", c1], ["2", c2]]) {
    await waitForState(rpc, session, (s) => s.model.ships.length === 2, `client ${name} to see both ships`);
  }

  // Nothing in orbs moves on its own — a pilot moves while THRUST is held. Hold
  // it on client 1 (level state, so it stays held across every step below) and
  // wait for the authority to have that intent, which is the game-level
  // convergence check `step_all` deliberately does not pretend to provide.
  await rpc.call("send_input", { session: c1, command: { type: "key", key: "w", down: true } });
  const seated = await waitForState(
    rpc,
    server,
    (s) => s.model.world.pilots.length === 2
      && s.model.world.pilots.some((p) => p.ship.pid === 0 && p.intent.thrust === true),
    "the server to seat both pilots and see P0 thrusting",
  );

  for (const session of [c1, server, c2]) await rpc.call("pause", { session });
  // Quiescence: pausing pinned the CLOCK, not the sockets. `pending_net` is
  // what says nothing already received is still unprocessed — without it a
  // baseline can be snapshotted mid-delivery.
  const quiescent = {};
  for (const session of [c1, server, c2]) {
    quiescent[session] = await waitForState(
      rpc,
      session,
      (s) => s.pending_net === 0,
      `session ${session} to go quiescent`,
    );
  }

  const base = await rpc.call("get_state", { session: server });
  const baseline = positionsOf(shipsOf(base));
  const trace = [];
  let firstResponse = null;
  for (let round = 0; round < ROUNDS; round += 1) {
    // The ordering law: producer → authority → observer.
    const stepped = await rpc.call("step_all", { sessions: [c1, server, c2], dts: DTS });
    firstResponse ??= stepped;
    trace.push(traceOf(shipsOf(await rpc.call("get_state", { session: server })), baseline));
  }

  const final = await rpc.call("get_state", { session: server });
  await rpc.call("send_input", { session: c1, command: { type: "key", key: "w", down: false } });
  for (const session of [c1, c2, server]) await rpc.call("stop_game", { session }).catch(() => {});

  return {
    label,
    sessions: [c1, server, c2],
    seatedAt: seated.frame,
    quiescent,
    baseRevision: base.model_revision,
    finalRevision: final.model_revision,
    firstResponse,
    trace,
  };
}

const proc = spawn(BIN, ["mcp"], { cwd: ROOT, stdio: ["pipe", "pipe", "pipe"] });
const rpc = new Rpc(proc);

try {
  await rpc.request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "functor-e2e-step-all", version: "0" },
  });
  rpc.notify("notifications/initialized", {});

  console.log("▸ step_all validates the group before anything steps");
  let empty = null;
  try {
    await rpc.call("step_all", { sessions: [] });
  } catch (error) {
    empty = error.message;
  }
  check(empty !== null && /producer/.test(empty), "an empty session list is a teaching error naming the order");

  console.log("\n▸ run 1: three roles of examples/orbs, stepped in order");
  const first = await lockstepRun(rpc, "run-1");
  check(
    first.firstResponse.sessions.map((s) => s.session).join(",") === first.sessions.join(","),
    `step_all reports the sessions in the caller's order (${first.firstResponse.sessions.map((s) => s.session)})`,
  );
  check(
    first.firstResponse.sessions.every((s) => s.pending_steps === 0),
    "every session's steps had fully landed before step_all returned",
  );
  check(
    first.firstResponse.sessions.every((s) => typeof s.model_revision === "number"),
    "each summary carries the networked model_revision",
  );
  check(
    first.finalRevision > first.baseRevision,
    `the authority's model_revision advanced over the run (${first.baseRevision} → ${first.finalRevision})`,
  );
  check(new Set(first.trace).size > 1, "the world actually moved (the trace is not a constant)");

  // The port must be free again before the second server binds.
  await sleep(2000);

  console.log("\n▸ run 2: the same run, from scratch");
  const second = await lockstepRun(rpc, "run-2");
  const same = first.trace.join(" | ") === second.trace.join(" | ");
  check(same, "ORDERED stepping is exactly reproducible: both runs produced the same world trace");
  if (!same) {
    console.log(`    run-1: ${first.trace.join(" | ")}`);
    console.log(`    run-2: ${second.trace.join(" | ")}`);
  }

  console.log("\n▸ save_project keeps a multi-entry project multi-entry");
  // orbs is the SAME-FILE role shape: two inline modules of one `game.fun`, so
  // the file set alone cannot say whether this is one role or two. That is
  // exactly the manifest a reconstructed `{"entry":"game.fun"}` destroys.
  const files = [
    ["game.fun", readFileSync(join(ROOT, DIR, "game.fun"), "utf8")],
    ["functor.json", readFileSync(join(ROOT, DIR, "functor.json"), "utf8")],
  ];

  const inline = await rpc.call("launch_game", { files, entry: "server", mode: "headless" });
  const state = await rpc.call("get_state", { session: inline.session });
  check(state.model.world !== undefined, "the inline multi-entry project booted the SERVER role");

  const saveDir = mkdtempSync(join(tmpdir(), "functor-mcp-mp-save-"));
  const saved = await rpc.call("save_project", { session: inline.session, dir: saveDir, overwrite: true });
  check(saved.files.includes("functor.json"), `save_project wrote ${saved.files}`);
  const manifest = JSON.parse(readFileSync(join(saveDir, "functor.json"), "utf8"));
  check(
    manifest.entries !== undefined && manifest.entries.server !== undefined,
    `the saved manifest still declares the ROLES (${JSON.stringify(manifest)})`,
  );
  check(manifest.entry === undefined, "it was not reconstructed as a single-entry project");
  await rpc.call("stop_game", { session: inline.session });

  console.log("\n▸ launch_game's response is the version, not the whole index");
  check(typeof inline.protocol_version === "number", `launch_game returns protocol_version ${inline.protocol_version}`);
  check(inline.discovery === undefined, "the endpoint index is not repeated on every launch");
  const verbose = await rpc.call("launch_game", { dir: "examples/counter", mode: "headless", discovery: true });
  check(verbose.discovery?.service === "functor debug runtime", "discovery: true still returns the full index");
  await rpc.call("stop_game", { session: verbose.session });
} catch (error) {
  failures.push(String(error?.stack ?? error));
  console.error(error);
} finally {
  proc.stdin.end();
}

console.log(failures.length === 0 ? "\nAll multiplayer MCP checks passed." : `\n${failures.length} failure(s):`);
for (const failure of failures) console.log(`  - ${failure}`);
process.exit(failures.length === 0 ? 0 : 1);
