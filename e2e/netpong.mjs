// `examples/netpong` E2E: the multiplayer claims, over a REAL wire.
//
// Netpong's claims are all about two processes agreeing, which no inline
// `expect` can reach — the expects pin the pure simulation, and this pins the
// session. It launches the roles as INDEPENDENT `functor` processes (one
// `--entry server`, two `--entry client`), each with its own debug port, and
// lets their game traffic use the sample's actual WebSocket listener. Every
// wait is a poll on observable state, never a sleep.
//
// It asserts:
//
//   1. a client started BEFORE the listener converges by itself — `Sub.connect`
//      surfaces the failure, then retries into a seat and advancing snapshots,
//      in the same process (no relaunch);
//   2. restarting only the server does the same again: the client sees the
//      disconnect, loses its seat, and the SAME process rejoins — adopting the
//      restarted server's fresh snapshot sequence rather than waiting out the
//      old one;
//   3. two clients take seats 0 and 1 from the authority;
//   4. real key input at a client reaches the server as a sequenced
//      `PaddleIntent` on that client's own seat — nobody steers a paddle they
//      were not given;
//   5. a real rally scores and BOTH clients converge on the server scoreboard;
//   6. the match is won, `R` requests a rematch, and the monotonic snapshot
//      sequence lets the already-connected clients accept the reset;
//   7. killing a client cleans up its seat server-side, and a replacement
//      rejoins and resumes snapshots.
//
// Run (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/netpong.mjs
//
// Set FUNCTOR_BIN when the build uses a shared CARGO_TARGET_DIR.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { BIN as runner, ROOT, portIsBound, sleep, waitForPort } from "./mcp-rpc.mjs";

const GAME_DIR = resolve(ROOT, "examples/netpong");
const basePort = Number(process.env.NETPONG_DEBUG_PORT ?? 8318);
// The sample's own listener, from `Protocol.bind` — real WebSocket traffic. It
// is baked into the .fun, so unlike the debug ports it cannot be moved.
const GAME_PORT = 9108;
const children = [];

// Fixed ports mean a leftover process from a crashed run — or a hand-launched
// `functor -d examples/netpong run native --entry server` — would be mistaken
// for the server this test starts. Fail loudly instead of asserting against
// somebody else's game.
for (const port of [GAME_PORT, basePort, basePort + 1, basePort + 2]) {
  if (await portIsBound(port)) {
    console.error(`port ${port} is already in use — kill the process on it first`);
    process.exit(1);
  }
}

/** Wait until nothing is listening on `port` — a killed server must release
 *  its listener before the replacement can bind it. */
const waitPortFree = async (port, timeoutMs = 15000) => {
  const deadline = Date.now() + timeoutMs;
  while (await portIsBound(port)) {
    if (Date.now() > deadline) throw new Error(`port ${port} never freed`);
    await sleep(100);
  }
};

const start = (entry, port) => {
  const child = spawn(runner, [
    "-d", GAME_DIR, "--entry", entry, "run", "native", "--",
    "--debug-port", String(port), "--headless",
  ], { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] });
  let log = "";
  child.stdout.on("data", (b) => { log += b; });
  child.stderr.on("data", (b) => { log += b; });
  child.__log = () => log;
  children.push(child);
  return child;
};

const getState = async (port) => {
  const response = await fetch(`http://127.0.0.1:${port}/state`);
  if (!response.ok) throw new Error(`state ${port}: ${response.status} ${await response.text()}`);
  return response.json();
};

const post = async (port, path, body) => {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${path} ${port}: ${response.status} ${await response.text()}`);
};

const waitFor = async (description, probe, timeoutMs = 30000) => {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await probe();
      if (last) return last;
    } catch (error) {
      last = error;
    }
    await sleep(50);
  }
  throw new Error(`timeout waiting for ${description}; last=${String(last)}`);
};

const waitDebug = (port) => waitFor(`debug port ${port}`, async () => {
  const state = await getState(port);
  return state.frame > 0 ? state : false;
});

const ctor = (value) => value?.$ctor;

try {
  // Start the client before any listener exists. `Sub.connect` must retain the
  // subscription and converge without replacing this process.
  const clientA = start("client", basePort + 1);
  const clientAPid = clientA.pid;
  await waitDebug(basePort + 1);
  const earlyFailure = await waitFor("connect-before-listen error", async () => {
    const state = await getState(basePort + 1);
    return state.model.status.startsWith("NET ERROR:") ? state : false;
  });
  assert.equal(ctor(earlyFailure.model.conn), "Offline");

  const connectBeforeListenStartedAt = Date.now();
  let server = start("server", basePort);
  await waitDebug(basePort);
  await waitForPort(GAME_PORT);

  const firstLive = await waitFor("original client to reconnect after listener starts", async () => {
    const state = await getState(basePort + 1);
    return state.model.status === "LIVE" && state.model.lastSnapshotSeq > 1
      && (state.model.seat === 0 || state.model.seat === 1) ? state : false;
  });
  const connectBeforeListenMs = Date.now() - connectBeforeListenStartedAt;
  const firstLiveSeq = firstLive.model.lastSnapshotSeq;
  await waitFor("original client snapshots to advance", async () => {
    const state = await getState(basePort + 1);
    return state.model.lastSnapshotSeq > firstLiveSeq ? state : false;
  });
  assert.equal(clientA.exitCode, null, "client A is still the original live process");

  // Restart only the authoritative server. The same original client must
  // observe the disconnect, retry, receive a fresh seat, and resume snapshots —
  // on the RESTARTED server's sequence, which counts from zero again.
  const seqBeforeKill = (await getState(basePort + 1)).model.lastSnapshotSeq;
  server.kill("SIGTERM");
  await new Promise((done) => server.once("exit", done));
  const disconnected = await waitFor("original client to observe server shutdown", async () => {
    const state = await getState(basePort + 1);
    return state.model.status === "RECONNECTING" && state.model.seat === -1
      && ctor(state.model.conn) === "Offline" ? state : false;
  });
  assert.equal(disconnected.model.seat, -1);
  await waitPortFree(GAME_PORT);
  const serverRestartStartedAt = Date.now();
  server = start("server", basePort);
  await waitDebug(basePort);
  await waitForPort(GAME_PORT);
  const liveAfterServerRestart = await waitFor("original client after server restart", async () => {
    const state = await getState(basePort + 1);
    return state.model.status === "LIVE" && state.model.lastSnapshotSeq >= 1
      && (state.model.seat === 0 || state.model.seat === 1) ? state : false;
  });
  const serverRestartMs = Date.now() - serverRestartStartedAt;
  // The point of the reset: the client goes LIVE again on a seq BELOW the one
  // it held before the kill. Carrying the old high-water mark over would make
  // it wait out the whole old count before drawing anything.
  assert.ok(
    liveAfterServerRestart.model.lastSnapshotSeq < seqBeforeKill,
    `reconnect must adopt the restarted server's fresh sequence ` +
      `(${liveAfterServerRestart.model.lastSnapshotSeq} < ${seqBeforeKill})`
  );
  const restartedServer = await getState(basePort);
  assert.equal(restartedServer.model.players.length, 1);
  assert.equal(restartedServer.model.players[0].seat, liveAfterServerRestart.model.seat);
  const restartLiveSeq = liveAfterServerRestart.model.lastSnapshotSeq;
  await waitFor("original client snapshots to advance after server restart", async () => {
    const state = await getState(basePort + 1);
    return state.model.lastSnapshotSeq > restartLiveSeq ? state : false;
  });
  assert.equal(clientA.exitCode, null, "client A is still the original live process");

  let clientB = start("client", basePort + 2);
  await waitDebug(basePort + 2);

  const joined = await waitFor("server to own two connections", async () => {
    const state = await getState(basePort);
    return state.model.players.length === 2 ? state : false;
  });
  assert.deepEqual(joined.model.players.map((p) => p.seat).sort(), [0, 1]);

  for (const port of [basePort + 1, basePort + 2]) {
    const synced = await waitFor(`client ${port} snapshot convergence`, async () => {
      const state = await getState(port);
      return state.model.lastSnapshotSeq > 1 && state.model.status === "LIVE" ? state : false;
    });
    assert.ok(synced.model.seat === 0 || synced.model.seat === 1);
  }

  const clientASeat = (await getState(basePort + 1)).model.seat;
  const intentSeqBefore = (await getState(basePort)).model.players
    .find((p) => p.seat === clientASeat).intentSeq;

  // Drive real input at client A and assert that the authoritative server saw it.
  await post(basePort + 1, "/input", { type: "key", key: "S", down: true });
  const intentSeen = await waitFor("client intent to reach authoritative server", async () => {
    const state = await getState(basePort);
    return state.model.players.some((p) => p.seat === clientASeat
      && p.axis === -1 && p.intentSeq > intentSeqBefore) ? state : false;
  });
  assert.ok(intentSeen.model.players.find((p) => p.seat === clientASeat).intentSeq > 0);

  // Client A is still holding `S`, so its paddle sits low and a real rally
  // scores. Baseline first: the attract-mode AI has been playing through two
  // reconnects, so "score > 0" alone could be satisfied by a point that was
  // already on the board. Both clients must converge on the server's scoreboard.
  const scoreBefore = (await getState(basePort)).model;
  const totalBefore = scoreBefore.leftScore + scoreBefore.rightScore;
  const scored = await waitFor("a real authoritative point", async () => {
    const state = await getState(basePort);
    return state.model.leftScore + state.model.rightScore > totalBefore ? state : false;
  }, 60000);
  for (const port of [basePort + 1, basePort + 2]) {
    const converged = await waitFor(`client ${port} scoreboard convergence`, async () => {
      const state = await getState(port);
      const t = state.model.target;
      return t.leftScore === scored.model.leftScore && t.rightScore === scored.model.rightScore
        ? state : false;
    });
    assert.equal(converged.model.target.leftScore + converged.model.target.rightScore,
                 scored.model.leftScore + scored.model.rightScore);
  }

  const won = await waitFor("authoritative win state", async () => {
    const state = await getState(basePort);
    return ctor(state.model.phase) === "Protocol.Won" ? state : false;
  }, 180000);
  const wonSeq = won.model.snapshotSeq;
  await post(basePort + 1, "/input", { type: "key", key: "R", down: true });
  await post(basePort + 1, "/input", { type: "key", key: "R", down: false });
  const rematched = await waitFor("rematch with monotonic snapshots", async () => {
    const state = await getState(basePort);
    return ctor(state.model.phase) === "Protocol.Serving"
      && state.model.leftScore === 0 && state.model.rightScore === 0
      && state.model.snapshotSeq > wonSeq ? state : false;
  });
  for (const port of [basePort + 1, basePort + 2]) {
    await waitFor(`client ${port} rematch convergence`, async () => {
      const state = await getState(port);
      return state.model.lastSnapshotSeq > wonSeq
        && state.model.target.leftScore === 0 && state.model.target.rightScore === 0
        ? state : false;
    });
  }
  await post(basePort + 1, "/input", { type: "key", key: "S", down: false });

  // Disconnect and relaunch a whole client process. The server must remove the
  // old connection, accept the new one, and snapshots must resume on that end.
  clientB.kill("SIGTERM");
  await new Promise((done) => clientB.once("exit", done));
  const afterDrop = await waitFor("server disconnect cleanup", async () => {
    const state = await getState(basePort);
    return state.model.players.length === 1 ? state : false;
  });
  assert.equal(afterDrop.model.players.length, 1);
  clientB = start("client", basePort + 2);
  await waitDebug(basePort + 2);
  const rejoined = await waitFor("replacement client to rejoin", async () => {
    const state = await getState(basePort);
    return state.model.players.length === 2 ? state : false;
  });
  assert.equal(rejoined.model.players.length, 2);
  const resumed = await waitFor("replacement client snapshots", async () => {
    const state = await getState(basePort + 2);
    return state.model.lastSnapshotSeq > rejoined.model.snapshotSeq ? state : false;
  });
  assert.equal(resumed.model.status, "LIVE");

  console.log(JSON.stringify({
    ok: true,
    processes: 3,
    // The client that dialled before the listener existed is the SAME process
    // at the end of the run — the two reconnects below were `Sub.connect`'s.
    originalClientPid: clientAPid,
    originalClientSurvivedServerRestart: clientA.exitCode === null,
    connectBeforeListenMs,
    serverRestartMs,
    serverRestartSeat: liveAfterServerRestart.model.seat,
    authoritativePoint: [scored.model.leftScore, scored.model.rightScore],
    rematchSnapshotSeq: rematched.model.snapshotSeq,
    serverPlayersAfterReconnect: rejoined.model.players.length,
    clientSnapshots: [
      (await getState(basePort + 1)).model.lastSnapshotSeq,
      resumed.model.lastSnapshotSeq,
    ],
    serverPhase: ctor(rejoined.model.phase),
  }, null, 2));
} catch (error) {
  for (const [index, child] of children.entries()) {
    if (child.__log) process.stderr.write(`\n--- child ${index} ---\n${child.__log()}\n`);
  }
  throw error;
} finally {
  // Await every exit: the next run's port preflight (and a back-to-back
  // invocation of this one) must not race a still-shutting-down listener.
  await Promise.all(
    children.map((child) => {
      if (child.exitCode !== null || child.signalCode !== null) return null;
      child.kill("SIGTERM");
      return new Promise((done) => child.once("exit", done));
    })
  );
}
