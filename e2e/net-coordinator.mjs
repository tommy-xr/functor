// Net-coordinator e2e: a WHOLE multiplayer session across player IFRAMES —
// one authoritative server pane and two client panes, routed by the host page's
// coordinator (site/src/net-coordinator.ts) over the embedder seam, with NO
// sockets and no server process.
//
// This runs examples/orbs the way the sandbox does: three independent
// `player.html?net=embedder` runtimes, each with its own model and its own
// render loop, whose packets cross the postMessage boundary and are routed by
// the host. Orbs keeps BOTH roles in one `game.fun` as inline `module Client` /
// `module Server` blocks, so a pane's role travels as `?module=`.
//
// It asserts:
//
//   1. every pane boots and ticks (a dead pane would make the rest vacuous);
//   2. the coordinator routes the connect handshake — both clients get
//      `connected`, and so does the server, twice;
//   3. traffic flows BOTH ways (client -> server Steer, server -> client
//      Snapshot broadcasts);
//   4. the server's model seats two pilots, and each client's board contains
//      BOTH ships — i.e. client 1's state reached client 2 through the server;
//   5. input propagates: holding `w` in client 1 reaches the server as THAT
//      pilot's intent, and only that pilot's.
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
// of examples/orbs at its own path (not the sandbox's flattened copy), and the
// host page.
const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

await esbuild.build({
  entryPoints: [`${ROOT}site/src/net-coordinator.ts`],
  outfile: `${DIST}/net-coordinator.js`,
  bundle: true,
  format: "esm",
  logLevel: "warning",
});
await mkdir(`${DIST}/examples/orbs`, { recursive: true });
await cp(`${ROOT}examples/orbs/game.fun`, `${DIST}/examples/orbs/game.fun`);
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

  // Input path: HOLD `w` in client 1. Orbs' client streams a Steer every tick
  // whichever keys are down, so — unlike a game that only sends on an input
  // CHANGE — a packet count cannot tell the two clients apart. What can is the
  // content, so the assertions move to the paused models below: the server
  // must carry that thrust against exactly one pilot, and it must be client
  // 1's (5a), and the thrust must have moved that ship in the OTHER client's
  // board (5b).
  await page.evaluate(() => window.__netHost.key("client 1", "KeyW", true));
  await sleep(600);

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
  // The handshake did its job at the CLIENT end: `myPid` starts at -1 and only
  // a Welcome off the wire replaces it, so a seated pid is the server's answer.
  const myPidOf = (text) => Number(text.match(/myPid: (-?[\d.]+)/)?.[1] ?? NaN);
  check(
    myPidOf(models["client 1"]) >= 0 && myPidOf(models["client 2"]) >= 0,
    "both clients were seated by the server (a pid off the wire)",
    `client 1 myPid ${myPidOf(models["client 1"])}, client 2 myPid ${myPidOf(models["client 2"])}`
  );
  check(
    (models["server"].match(/cid: /g) ?? []).length === 2,
    "the server holds a seat for each client",
    models["server"]
  );
  // Which pid client 1 is depends on JOIN ORDER, which nothing here sequences:
  // the three panes boot as independent iframes, the coordinator hands out
  // connection ids in arrival order (site/src/net-coordinator.ts), and the
  // server allocates `pid = nextPid` per Join (examples/orbs' `join`). So
  // client 1 is pid 0 only when it wins the boot race. Read the mover's
  // identity out of the data instead: the coordinator's log says which
  // connection is client 1's, and the server's SEATS map that cid to a pid.
  const connOfClient1 = connected.find((p) => p.to === "client 1")?.conn;
  const seats = [...models["server"].matchAll(/cid: (-?[\d.]+), pid: (-?[\d.]+)/g)].map((m) => ({
    cid: Number(m[1]),
    pid: Number(m[2]),
  }));
  const moverPid = seats.find((s) => s.cid === connOfClient1)?.pid;

  // How a Ship renders in a model's Debug text — spelled ONCE, because both
  // probes below read it and a field-order change must break them together.
  const SHIP = String.raw`\{ pid: (-?[\d.]+), x: (-?[\d.]+), y: (-?[\d.]+), rot: (-?[\d.]+) \}`;
  const shipOf = (m) => ({ pid: Number(m[1]), x: Number(m[2]), y: Number(m[3]) });

  // 5a. The held `w` arrived at the server as client 1's INTENT — and only
  // client 1's. A pilot renders as its ship beside the intent the server last
  // folded in for it, so one regex carries both the identity and the thrust.
  const pilotsOf = (text) =>
    [
      ...text.matchAll(
        new RegExp(`ship: ${SHIP}, intent: \\{ turn: (-?[\\d.]+), thrust: (true|false)`, "g")
      ),
    ].map((m) => ({ ...shipOf(m), thrust: m[6] === "true" }));
  const pilots = pilotsOf(models["server"]);
  const thrusting = pilots.filter((p) => p.thrust);
  check(
    moverPid !== undefined &&
      pilots.length === 2 &&
      thrusting.length === 1 &&
      thrusting[0].pid === moverPid,
    "holding `w` in client 1 reaches the server as THAT pilot's intent, and only its",
    `client 1 is conn ${connOfClient1} = pid ${moverPid}; server pilots ${JSON.stringify(pilots)}`
  );

  // 5b. Cross-client propagation. Each client's board must hold BOTH ships,
  // and the ship of the pilot holding `w` must have moved in y in the OTHER
  // client's copy — client 1's input reached client 2 through the server.
  const shipsOf = (text) => [...text.matchAll(new RegExp(SHIP, "g"))].map(shipOf);
  const boards = { 1: shipsOf(models["client 1"]), 2: shipsOf(models["client 2"]) };
  // The two joiners are pids 0 and 1 whichever pane won the race — join order
  // decides WHO owns which, not which pids exist.
  const pidsOf = (ships) => [...ships.map((s) => s.pid)].sort().join(",");
  check(
    pidsOf(boards[1]) === "0,1" && pidsOf(boards[2]) === "0,1",
    "each client's board contains both ships",
    `client 1: [${pidsOf(boards[1])}], client 2: [${pidsOf(boards[2])}]`
  );
  // Every ship spawns at y = 0 and thrust at rot 0 is straight up (+y, clamped
  // at the arena wall), so the mover's y is strictly positive while the ship
  // nobody is steering sits exactly on its spawn.
  const moved = boards[2].find((s) => s.pid === moverPid);
  const untouched = boards[2].find((s) => s.pid !== moverPid);
  check(
    moverPid !== undefined &&
      !!moved &&
      moved.y > 0 &&
      !!untouched &&
      untouched.y === 0,
    "client 1's `w` moved its ship in CLIENT 2's board (client -> server -> client)",
    `client 2's board ${JSON.stringify(boards[2])}; server seats ${JSON.stringify(seats)}`
  );

  // 6. Teardown: reloading a pane closes its connection at BOTH ends — proved
  // in the SERVER's model, not just in the packet log. After client 2 reloads,
  // its reboot Joins as a third pid; the server must hold TWO seats (the old
  // one released) with a `pid: 2` among them. A disconnect that was logged but
  // never delivered would leave three.
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
    "the server released the reloaded pane's seat and accepted its rejoin",
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
