// `functor mcp` E2E: the NATIVE COORDINATOR — the host process IS the network.
//
// It runs `examples/orbs` as a session GROUP: one `server` role and two
// `client` roles, each launched on the embedder transport
// (`--net-transport embedder`), so no runtime opens a socket at all. Every
// `Sub.listen` / `Sub.connect` / `Effect.send` crosses THIS process, which
// routes it and records it.
//
// What it proves (docs/mcp.md, "Running a multiplayer session"; Addendum 7):
//
//   1. ZERO REAL SOCKETS. `examples/orbs` binds `127.0.0.1:9101` over a real
//      transport; under the coordinator nothing ever listens there, checked
//      repeatedly — before the group converges, after it converges, and after
//      a dozen lockstep rounds. The clients still converge, which is the
//      point: the routing is real, the sockets are not.
//   2. `wire_log` is the wire AS DATA, and its `payload_text` is undecoded on
//      purpose — this test decodes the `fun:`-framed JSON itself, which is
//      exactly what an agent does.
//   3. `step_all` and `wire_log` COMPOSE: take the last `seq`, step a round,
//      and the new rows are that round's traffic (client→server intents and
//      server→client snapshots), in both directions.
//   4. Reloading the AUTHORITY mid-group is survivable: the group keeps
//      routing and the clients keep tracking the world.
//
// Determinism, honestly: delivery is immediate and in-order, and `step_all`
// pumps at each session boundary with the background pump held off — so the
// COUNT and DIRECTION of a round's packets is what this asserts, per round.
// Absolute frame numbers are not: the group runs on wall-clock time between
// tool calls, so how many frames elapse before the pause is not controlled.
// Whole-environment reproducibility waits on barrier stepping.
//
// Run manually (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/mcp-session-group.mjs
//
// Set FUNCTOR_BIN when the build uses a shared CARGO_TARGET_DIR.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { check, failures, portIsBound, ROOT, startMcp, waitForState } from "./mcp-rpc.mjs";

const DIR = "examples/orbs";
/** The ws port `examples/orbs` declares (`let bind = "127.0.0.1:9101"`). Under
 * the embedder transport it must NEVER be bound. */
const WS_PORT = 9101;
const ROUNDS = 12;
const DTS = 0.016;

const { proc, rpc } = await startMcp("functor-e2e-session-group");
let group = null;

try {
  check(!(await portIsBound(WS_PORT)), `nothing is on 127.0.0.1:${WS_PORT} before the run`);

  console.log("▸ launch_session_group wires three orbs roles to each other");
  const launched = await rpc.call("launch_session_group", {
    dir: DIR,
    roles: ["server", "client", "client"],
    mode: "headless",
  });
  group = launched.group;
  const [server, c1, c2] = launched.sessions.map((s) => s.session);
  check(
    launched.sessions.map((s) => s.label).join(",") === "server,client1,client2",
    `the group labels its roles for the wire log (${launched.sessions.map((s) => s.label)})`,
  );
  check(
    launched.step_order.join(",") === [server, c1, c2].join(","),
    "the group reports the step order the roles were launched in",
  );

  console.log("\n▸ the clients converge over the coordinator — with no socket anywhere");
  await waitForState(rpc, c1, (s) => s.model.myPid === 0, "client 1 to be seated as P0");
  await waitForState(rpc, c2, (s) => s.model.myPid === 1, "client 2 to be seated as P1");
  for (const [name, session] of [["1", c1], ["2", c2]]) {
    await waitForState(
      rpc,
      session,
      (s) => s.model.ships.length === 2,
      `client ${name} to see both ships`,
    );
  }
  check(
    !(await portIsBound(WS_PORT)),
    `the group converged with NOTHING listening on 127.0.0.1:${WS_PORT}`,
  );

  console.log("\n▸ wire_log is the wire as data");
  const log = await rpc.call("wire_log", { group, limit: 500 });
  check(log.rows.length > 0, `the coordinator routed ${log.routed} packet(s)`);
  check(
    log.sessions.join(",") === "server,client1,client2",
    `wire_log names the group's sessions (${log.sessions})`,
  );
  const connectRow = log.rows.find((r) => r.kind === "connected");
  check(connectRow !== undefined, "the handshake is in the log as a `connected` row");
  const messages = log.rows.filter((r) => r.kind === "message");
  check(messages.length > 0, `${messages.length} message row(s) carry payload_text`);
  // The framing belongs to the GAME: `Effect.sendMsg` prefixes U+0001 + "fun:"
  // and then serde's EffectValue JSON (site/src/wire-value.ts documents the
  // shape). The coordinator deliberately does not decode any of it, so this
  // test does — which is exactly what an agent reading the log does.
  const TYPED_MSG_PREFIX = "\u0001fun:";
  const decodable = messages.filter((row) => {
    const text = row.payload_text ?? "";
    if (!text.startsWith(TYPED_MSG_PREFIX)) return false;
    const body = text.slice(TYPED_MSG_PREFIX.length);
    try {
      JSON.parse(body);
      return true;
    } catch {
      return false;
    }
  });
  check(
    decodable.length === messages.length,
    `every payload_text decodes agent-side (${decodable.length}/${messages.length}); ` +
      `sample: ${JSON.stringify(messages[0]?.payload_text?.slice(0, 80))}`,
  );
  check(
    messages.every((row) => row.size > 0 && row.from !== row.to),
    "message rows carry a byte size and route between two distinct sessions",
  );
  check(
    log.rows.every((row) => row.frame === null),
    "v1 delivery is unscheduled, so every row's frame is null (barrier stepping fills it in)",
  );

  console.log("\n▸ a filtered wire_log narrows by link and direction");
  const fromC1 = await rpc.call("wire_log", { group, link: "client1", direction: "sent", limit: 200 });
  check(
    fromC1.rows.length > 0 && fromC1.rows.every((row) => row.from === "client1"),
    `link+direction returns only client1's sends (${fromC1.rows.length} rows)`,
  );
  const pair = await rpc.call("wire_log", { group, link: "client2:server", limit: 200 });
  check(
    pair.rows.every((row) => new Set([row.from, row.to]).size === 2
      && ["client2", "server"].includes(row.from)
      && ["client2", "server"].includes(row.to)),
    "a pair link returns only traffic between those two ends",
  );
  let badLabel = null;
  try {
    await rpc.call("wire_log", { group, link: "cleint1" });
  } catch (error) {
    badLabel = error.message;
  }
  check(
    badLabel !== null && /unknown session label/.test(badLabel),
    "a mistyped label is a teaching error, not an empty result",
  );

  console.log("\n▸ step_all and wire_log compose");
  // Nothing in orbs moves on its own — hold thrust on client 1 so each round
  // has real client→server traffic to route.
  await rpc.call("send_input", { session: c1, command: { type: "key", key: "w", down: true } });
  await waitForState(
    rpc,
    server,
    (s) => s.model.world.pilots.some((p) => p.ship.pid === 0 && p.intent.thrust === true),
    "the server to see P0 thrusting",
  );
  for (const session of [c1, server, c2]) await rpc.call("pause", { session });

  const perRound = [];
  let cursor = (await rpc.call("wire_log", { group, limit: 1 })).rows.at(-1)?.seq ?? null;
  for (let round = 0; round < ROUNDS; round += 1) {
    await rpc.call("step_all", { sessions: [c1, server, c2], dts: DTS });
    const window = await rpc.call("wire_log", {
      group,
      ...(cursor === null ? {} : { since: cursor }),
      limit: 500,
    });
    perRound.push(window.rows);
    if (window.rows.length > 0) cursor = window.rows.at(-1).seq;
  }
  check(
    perRound.every((rows) => rows.every((row) => row.seq > 0)),
    "every windowed row falls after the cursor taken before its round",
  );
  const steady = perRound.slice(2);
  check(
    steady.every((rows) => rows.length > 0),
    `each stepped round routed traffic (${perRound.map((r) => r.length).join(",")})`,
  );
  check(
    steady.every((rows) => rows.some((r) => r.from === "client1" && r.to === "server")),
    "each round carries client1's intent to the authority",
  );
  check(
    steady.every((rows) => rows.some((r) => r.from === "server" && r.to === "client2")),
    "…and the authority's broadcast back out to client2",
  );
  const counts = new Set(steady.map((rows) => rows.length));
  check(
    counts.size === 1,
    `an ordered round routes the same packet count every time (${[...counts].join(",")})`,
  );
  check(
    !(await portIsBound(WS_PORT)),
    `still NOTHING listening on 127.0.0.1:${WS_PORT} after ${ROUNDS} rounds`,
  );

  console.log("\n▸ reloading the authority mid-group");
  // A model-preserving push of the SAME source. The connections belong to the
  // COORDINATOR, not to the game, so a reload of the authority does not tear
  // them down: the model (and with it the seats and connection ids) survives,
  // and routing continues across it. That is the documented behaviour this
  // pins — a client that is dropped can never re-dial on its own, so silently
  // losing the group here would be invisible until a session went quiet.
  const before = (await rpc.call("get_state", { session: server })).model_revision;
  await rpc.call("reload_project", {
    session: server,
    files: [["game.fun", readFileSync(join(ROOT, DIR, "game.fun"), "utf8")]],
  });
  for (const session of [c1, server, c2]) await rpc.call("resume", { session });
  const after = await waitForState(
    rpc,
    server,
    (s) => s.model_revision > before && s.model.world.pilots.length === 2,
    "the reloaded authority to keep both pilots seated",
  );
  check(after.model.world.pilots.length === 2, "the group survived a reload of its authority");
  const post = await rpc.call("wire_log", { group, since: cursor, limit: 200 });
  check(
    post.rows.some((r) => r.from === "server"),
    "the reloaded authority is still sending on the coordinator's connections",
  );
  check(
    !(await portIsBound(WS_PORT)),
    `no socket appeared across the reload either (127.0.0.1:${WS_PORT})`,
  );

  console.log("\n▸ stopping the group's sessions retires the coordinator");
  await rpc.call("send_input", { session: c1, command: { type: "key", key: "w", down: false } });
  for (const session of [c1, c2, server]) await rpc.call("stop_game", { session });
  let gone = null;
  try {
    await rpc.call("wire_log", { group });
  } catch (error) {
    gone = error.message;
  }
  check(gone !== null, "the group is gone once its last session stops");
} catch (error) {
  failures.push(String(error?.stack ?? error));
  console.error(error);
} finally {
  proc.stdin.end();
}

console.log(
  failures.length === 0 ? "\nAll coordinator MCP checks passed." : `\n${failures.length} failure(s):`,
);
for (const failure of failures) console.log(`  - ${failure}`);
process.exit(failures.length === 0 ? 0 : 1);
