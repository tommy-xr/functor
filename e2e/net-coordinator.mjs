// Net-coordinator e2e: a WHOLE multiplayer session across player IFRAMES —
// one authoritative server pane and two client panes, routed by the host page's
// coordinator (site/src/net-coordinator.ts) over the embedder seam, with NO
// sockets and no server process.
//
// This runs examples/mp the way the sandbox will: three independent
// `player.html?net=embedder` runtimes, each with its own model and its own
// render loop, whose packets cross the postMessage boundary and are routed by
// the host.
//
// It asserts:
//
//   1. every pane boots and ticks (a dead pane would make the rest vacuous);
//   2. the coordinator routes the connect handshake — both clients get
//      `connected`, and so does the server, twice;
//   3. traffic flows BOTH ways (client -> server Move, server -> client
//      Snapshot broadcasts);
//   4. the server's model tracks two players, and each client's world contains
//      BOTH players — i.e. client 1's state reached client 2 through the server;
//   5. input propagates: a keydown in client 1 produces a client-1 -> server
//      packet that client 2 does not produce.
//
// A pane's model is read through its PAUSED inspector trace: pausing makes the
// player relay `functor-inspector-trace`, whose replayed bindings carry the
// live values (there is no model-json export on the web runtime).
//
// Run manually (needs the web-runtime wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/net-coordinator.mjs
import { spawn, spawnSync } from "node:child_process";
import { cp, mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import esbuild from "esbuild";

const PORT = Number(process.env.FUNCTOR_NET_PORT ?? 8124);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const DIST = `${ROOT}site/dist`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = (ok, what, detail = "") => {
  console.log(`${ok ? "  ok" : "FAIL"}  ${what}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures += 1;
};

// Build the site, then add the three things this host page needs that the
// site itself does not ship: the coordinator as a standalone ES module, a copy
// of examples/mp (not one of the sandbox's examples), and the host page.
const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

await esbuild.build({
  entryPoints: [`${ROOT}site/src/net-coordinator.ts`],
  outfile: `${DIST}/net-coordinator.js`,
  bundle: true,
  format: "esm",
  logLevel: "warning",
});
await mkdir(`${DIST}/examples/mp`, { recursive: true });
for (const file of ["server.fun", "client.fun", "protocol.fun", "view.fun"]) {
  await cp(`${ROOT}examples/mp/${file}`, `${DIST}/examples/mp/${file}`);
}
await cp(`${ROOT}e2e/fixtures/net-host.html`, `${DIST}/net-host.html`);

try {
  await fetch(BASE);
  console.error(`port ${PORT} is already in use — kill the process on it first`);
  process.exit(1);
} catch {
  // Nothing listening: good.
}
const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let i = 0; ; i++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (i > 50) throw new Error("site server never came up");
    await sleep(200);
  }
}

const PANES = ["server", "client 1", "client 2"];

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  const pageErrors = [];
  page.on("pageerror", (e) => pageErrors.push(String(e)));
  await page.goto(`${BASE}/net-host.html`);

  // 1. Every pane boots and ticks.
  const framesAt = () => page.evaluate((ids) => ids.map((id) => window.__netHost.frame(id)), PANES);
  for (let i = 0; i < 100; i++) {
    const frames = await framesAt();
    if (frames.every((f) => typeof f === "number" && f > 2)) break;
    await sleep(200);
  }
  const first = await framesAt();
  await sleep(1500);
  const second = await framesAt();
  check(
    first.every((f) => typeof f === "number") && second.every((f, i) => f > first[i]),
    "all three panes boot and tick",
    `${JSON.stringify(first)} -> ${JSON.stringify(second)}`
  );

  const packets = await page.evaluate(() => window.__netHost.packets());
  const of = (kind) => packets.filter((p) => p.kind === kind);

  // 2. The connect handshake reached both ends.
  const connected = of("connected");
  check(
    connected.filter((p) => p.to === "client 1").length === 1 &&
      connected.filter((p) => p.to === "client 2").length === 1 &&
      connected.filter((p) => p.to === "server").length === 2,
    "the coordinator routed both connects to both ends",
    connected.map((p) => `${p.from}->${p.to}#${p.conn}`).join(" ")
  );
  check(
    of("error").length === 0,
    "no connect was refused",
    JSON.stringify(of("error").map((p) => p.to))
  );

  // 3. Traffic in both directions.
  const messages = of("message");
  const toClients = messages.filter((p) => p.from === "server");
  const toServer = messages.filter((p) => p.to === "server");
  check(
    toClients.length > 10 && toClients.every((p) => p.size > 0),
    "the server broadcasts snapshots to the clients",
    `${toClients.length} packets`
  );
  check(
    toServer.length >= 2,
    "the clients send to the server",
    `${toServer.length} packets from ${[...new Set(toServer.map((p) => p.from))].join(", ")}`
  );

  // 5a. Input path: `w` in client 1 sends a Move — and only from client 1.
  // The server then integrates that player's z, which is what 5b reads back
  // out of the OTHER client's world.
  const sendsFrom = (id, log) => log.filter((p) => p.kind === "message" && p.from === id).length;
  const before = { c1: sendsFrom("client 1", packets), c2: sendsFrom("client 2", packets) };
  await page.evaluate(() => window.__netHost.key("client 1", "KeyW", true));
  await sleep(600);
  const afterLog = await page.evaluate(() => window.__netHost.packets());
  const after = { c1: sendsFrom("client 1", afterLog), c2: sendsFrom("client 2", afterLog) };
  check(
    after.c1 > before.c1 && after.c2 === before.c2,
    "a keydown in client 1 sends to the server, and only from client 1",
    `client 1 ${before.c1}->${after.c1}, client 2 ${before.c2}->${after.c2}`
  );
  // Release the key: the held `w` is a velocity, and the server integrates and
  // WRAPS it at the arena edge, so a mover left running would eventually wrap
  // back past its spawn lane. The release pins its z for 5b below.
  await page.evaluate(() => window.__netHost.key("client 1", "KeyW", false));
  await sleep(200);

  // 4. Model state, read from each pane's paused inspector trace.
  await page.evaluate((ids) => ids.forEach((id) => window.__netHost.pause(id)), PANES);
  await sleep(1200);
  const models = await page.evaluate(
    (ids) => Object.fromEntries(ids.map((id) => [id, window.__netHost.model(id)])),
    PANES
  );
  if (process.env.FUNCTOR_NET_DUMP) {
    await writeFile("/tmp/net-models.json", JSON.stringify(models, null, 2));
  }
  check(
    PANES.every((id) => typeof models[id] === "string"),
    "every pane's paused model is readable",
    JSON.stringify(models["client 1"])
  );
  check(
    models["client 1"].includes('status: "in-world"') &&
      models["client 2"].includes('status: "in-world"'),
    "both clients joined the server's world",
    `${models["client 1"]}`
  );
  check(
    (models["server"].match(/cid: /g) ?? []).length === 2,
    "the server tracks both players",
    models["server"]
  );
  // 5b. Cross-client propagation. Each client's world must hold BOTH rows,
  // and the row for the player that pressed `w` must have moved in z in the
  // OTHER client's copy — client 1's input reached client 2 through the
  // server.
  //
  // Which pid that is depends on JOIN ORDER, which nothing here sequences:
  // the three panes boot as independent iframes, the coordinator hands out
  // connection ids in arrival order (site/src/net-coordinator.ts), and the
  // server assigns `pid = nextPid` per Joined (examples/mp/server.fun). So
  // client 1 is pid 0 only when it wins the boot race. Read the mover's
  // identity out of the data instead: the coordinator's log says which
  // connection is client 1's, and the server's model maps that cid to a pid.
  const rowsOf = (text) =>
    [...text.matchAll(/\{ pid: (-?[\d.]+), x: (-?[\d.]+), z: (-?[\d.]+) \}/g)].map((m) => ({
      pid: Number(m[1]),
      x: Number(m[2]),
      z: Number(m[3]),
    }));
  const worlds = { 1: rowsOf(models["client 1"]), 2: rowsOf(models["client 2"]) };
  // The two joiners are pids 0 and 1 whichever pane won the race — join order
  // decides WHO owns which, not which pids exist.
  const pidsOf = (rows) => [...rows.map((r) => r.pid)].sort().join(",");
  check(
    pidsOf(worlds[1]) === "0,1" && pidsOf(worlds[2]) === "0,1",
    "each client's world contains both players",
    `client 1: [${pidsOf(worlds[1])}], client 2: [${pidsOf(worlds[2])}]`
  );
  // Client 1's connection id, then the pid the server gave that cid.
  const connOfClient1 = connected.find((p) => p.to === "client 1")?.conn;
  const serverPlayers = [...models["server"].matchAll(/cid: (-?[\d.]+), pid: (-?[\d.]+)/g)].map(
    (m) => ({ cid: Number(m[1]), pid: Number(m[2]) })
  );
  const moverPid = serverPlayers.find((p) => p.cid === connOfClient1)?.pid;
  // Each player spawns on its own z-lane: `z = pid * 1.8 - 1.8` (server.fun).
  // The other player never moves in z (its auto-move is +x only, and the
  // server's integration of a zero velocity is exact), so its z is its spawn.
  const spawnZ = (pid) => pid * 1.8 - 1.8;
  const moved = worlds[2].find((r) => r.pid === moverPid);
  const untouched = worlds[2].find((r) => r.pid !== moverPid);
  check(
    moverPid !== undefined &&
      !!moved &&
      moved.z > spawnZ(moverPid) &&
      !!untouched &&
      untouched.z === spawnZ(untouched.pid),
    "client 1's `w` moved its player in CLIENT 2's world (client -> server -> client)",
    `client 1 is conn ${connOfClient1} = pid ${moverPid} (spawn z ${spawnZ(
      moverPid
    )}); client 2's world ${JSON.stringify(worlds[2])}; server players ${JSON.stringify(
      serverPlayers
    )}`
  );

  // 6. Teardown: reloading a pane closes its connection at BOTH ends — proved
  // in the SERVER's model, not just in the packet log. After client 2 reloads,
  // its reboot rejoins as a third player id; the server must hold TWO players
  // (the old one dropped) with a `pid: 2` among them. A disconnect that was
  // logged but never delivered would leave three.
  const connOfClient2 = connected.find((p) => p.to === "client 2")?.conn;
  await page.evaluate(() => ["server", "client 2"].forEach((id) => window.__netHost.resume(id)));
  await page.evaluate(() => window.__netHost.reload("client 2"));
  await sleep(4000);
  const closedLog = await page.evaluate(() => window.__netHost.packets());
  const dropped = closedLog.filter((p) => p.kind === "disconnected" && p.conn === connOfClient2);
  check(
    dropped.length === 2 && new Set(dropped.map((p) => p.to)).size === 2,
    "reloading a pane routes a disconnect to both ends of its connection",
    dropped.map((p) => `${p.to}#${p.conn}`).join(" ")
  );
  await page.evaluate(() => window.__netHost.pause("server"));
  await sleep(1200);
  const rejoined = await page.evaluate(() => window.__netHost.model("server"));
  check(
    (rejoined.match(/cid: /g) ?? []).length === 2 && rejoined.includes("pid: 2"),
    "the server dropped the reloaded pane's player and accepted its rejoin",
    rejoined
  );

  const lines = await page.evaluate(() => window.__netHost.console());
  const errors = lines.filter((l) => l.includes("error:"));
  check(errors.length === 0, "no pane reported a runtime error", errors.slice(0, 3).join(" | "));
  check(pageErrors.length === 0, "the host page threw nothing", pageErrors.slice(0, 3).join(" | "));
} finally {
  await browser.close();
  server.kill();
}

console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
