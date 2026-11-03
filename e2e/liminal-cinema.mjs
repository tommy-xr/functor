// liminal-office's cinema (C) E2E: the real physics world, a hidden window,
// the clock paused between `/time` steps.
//
//   1. the walk: from the parking lot in through the entrance, across the
//      lobby, through the arch up the numbered corridor, round its bend and
//      in at auditorium 3's door; up its east aisle to the top of the rake,
//      then down it to the screen. Every waypoint is reached, each room is
//      its own cell, the rake rises and falls with the aisle, and the eye
//      is in a cell of the active cinema at every tick (`uncovered` 0);
//   2. draws per view: variants of cinema.fun's `roomOf` and `unlitOf`
//      drawing each cell model 0×, 1× or once per group count the static
//      draws that submit (seats aside): at most 30 per view;
//   3. residency (`--json` frame stats): entering the lobby from the lot's
//      far corner, where the cinema is its shell, misses no GPU cache;
//   4. the cinema goldens render byte-identically across runs.
//
//   node e2e/liminal-cinema.mjs        (FUNCTOR_BIN picks the CLI)
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BIN, ROOT, check, failures, portIsBound, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`
const WALK = 2.2 / 60; // metres per step
const CENTRE = 0.92; // the capsule's centre above its feet
// cinemaLayout.fun's room ids, and block.fun's cinema town; the cinema
// stands at the origin, unturned, so its space is the world's.
const CELL = { apron: 0, lobby: 1, corridorA: 2, corridorB: 3, east: 4 };
const TOWN = 6;
// The rake: 12 rows, 0.3 m each.
const RAKE_TOP = 12 * 0.3;

async function launch(port, flags = []) {
  // Another game on the port would answer for this one.
  if (await portIsBound(port)) throw new Error(`port ${port} is taken: stop what's listening there`);
  const proc = spawn(BIN, [...flags, "-d", DIR, "run", "native", "--hidden", "--debug-port", String(port)], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  const base = `http://127.0.0.1:${port}`;
  const post = async (path, body) => {
    const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
    if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${await res.text()}`);
    return res;
  };
  const state = async () => {
    const res = await fetch(`${base}/state`);
    if (!res.ok) throw new Error(`GET /state → ${res.status}`);
    return res.json();
  };
  const game = {
    base,
    proc,
    post,
    state,
    log: () => log,
    async ready() {
      const deadline = Date.now() + 90000;
      while (Date.now() < deadline) {
        try {
          if ((await state()).frame > 0) return post("/time", { type: "set", tts: 0 });
        } catch {
          // not listening yet
        }
        await sleep(100);
      }
      throw new Error("runtime never reported a frame");
    },
    async step(n = 1) {
      await post("/time", { type: "advance", dts: STEP, frames: n });
      const deadline = Date.now() + 120000;
      while (Date.now() < deadline) {
        if ((await state()).pending_steps === 0) return;
        await sleep(10);
      }
      throw new Error(`${n} queued step(s) never landed`);
    },
    key: (key, down) => post("/input", { type: "key", key, down }),
    async hold(key, n) {
      await game.key(key, true);
      await game.step(n);
      await game.key(key, false);
      await game.step(1);
    },
    mouseX: 400,
    async turn(radians) {
      game.mouseX -= Math.round(radians / MOUSE_RATE);
      await post("/input", { type: "mouse_move", x: game.mouseX, y: 300 });
      await game.step(1);
    },
    async face(yaw) {
      const d = yaw - (await game.model()).yaw;
      await game.turn(Math.atan2(Math.sin(d), Math.cos(d)));
    },
    async view(n) {
      await game.hold(String(n), 1);
      await game.step(3);
    },
    model: async () => (await state()).model,
  };
  return game;
}

// Walk straight to (x, z), `chunk` steps at a time, facing it each time;
// `onStep` sees every sample. Stops early when blocked.
async function walkTo(game, [x, z], onStep, chunk = 12) {
  let m = await game.model();
  for (let leg = 0; leg < 400; leg++) {
    const [dx, dz] = [x - m.at.x, z - m.at.z];
    const left = Math.hypot(dx, dz);
    if (left < 0.05) break;
    await game.face(Math.atan2(dx, dz));
    await game.key("w", true);
    await game.step(Math.max(1, Math.min(chunk, Math.floor(left / WALK))));
    await game.key("w", false);
    await game.step(1);
    const before = m.at;
    m = await game.model();
    onStep?.(m);
    if (Math.hypot(m.at.x - before.x, m.at.z - before.z) < 0.001) break; // blocked
  }
  return m;
}

async function walk(game) {
  const snags = [];
  const go = async (x, z, label, onStep) => {
    const m = await walkTo(game, [x, z], onStep);
    if (Math.hypot(m.at.x - x, m.at.z - z) > 0.1) snags.push(`${label} (stopped at ${m.at.x.toFixed(2)}, ${m.at.y.toFixed(2)}, ${m.at.z.toFixed(2)})`);
    return m;
  };
  const cellAt = async (cell, label) => {
    const m = await game.model();
    check(m.cell === cell, `${label}: in cell ${cell} (cell ${m.cell} at ${m.at.x.toFixed(1)}, ${m.at.z.toFixed(1)})`);
  };
  await game.view(9); // the parking lot
  await cellAt(CELL.apron, "the lot");
  await go(30, -4, "the apron");
  await go(30, 2, "the entrance");
  await cellAt(CELL.lobby, "inside the entrance");
  await go(40, 9, "the lobby");
  await go(43.8, 13, "the arch");
  await go(46.75, 13, "corridor A");
  await cellAt(CELL.corridorA, "through the arch");
  await go(46.75, 19.75, "the bend");
  await cellAt(CELL.corridorB, "round the bend");
  await go(41.6, 19.75, "auditorium 3's door");
  await go(41.6, 22.6, "auditorium 3");
  await cellAt(CELL.east, "through the door");
  await go(42.25, 24.0, "the east aisle's foot");
  // Up the aisle: the rake lifts the walker a row at a time to its top.
  const up = [];
  let m = await go(42.25, 39.2, "the top row", (s) => up.push(s.at.y));
  const topY = RAKE_TOP + CENTRE;
  check(Math.abs(m.at.y - topY) < 0.05 && m.cell === CELL.east && m.grounded,
    `up the aisle to the top of the rake (y ${m.at.y.toFixed(3)} vs ${topY.toFixed(3)}, cell ${m.cell})`);
  check(up.every((y, i) => i === 0 || y >= up[i - 1] - 0.02), `the rake only rises going up (${up.map((y) => y.toFixed(2)).join(" ")})`);
  // And down the rake to the screen.
  const down = [];
  m = await go(42.25, 23.0, "down to the screen", (s) => down.push(s.at.y));
  check(Math.abs(m.at.y - CENTRE) < 0.05 && m.cell === CELL.east && m.grounded,
    `down the rake to the floor by the screen (y ${m.at.y.toFixed(3)}, cell ${m.cell})`);
  check(down.every((y, i) => i === 0 || y <= down[i - 1] + 0.02) && down[0] > down.at(-1) + RAKE_TOP - 0.5,
    `the rake only falls going down (${down.map((y) => y.toFixed(2)).join(" ")})`);
  check(snags.length === 0, `no snag on the walk${snags.length ? `: ${snags.slice(0, 3).join("; ")}` : ""}`);
}

// Static draws from `/state.cull` (tested − culled): a static model the
// walk doesn't hide is tested once, and one in the frustum submits a draw
// per group; so drawing each model 0×, 1× or once per group counts them.
const ROOM_OF = "Scene.modelOf(p.model) |> Scene.inCells([p.cell])";
const UNLIT_OF = "Scene.inCells([p.cell], Scene.modelOf(p.model))";
const times = (n) => `Scene.group(List.range(${n}) |> List.map((k) => Scene.modelOf(p.model))) |> Scene.inCells([p.cell])`;
async function staticDraws(game, label) {
  const project = await (await fetch(`${game.base}/project`)).json();
  const at = project.findIndex(([path]) => path.endsWith("/cinema.fun") || path === "cinema.fun");
  if (at < 0 || !project[at][1].includes(ROOM_OF) || !project[at][1].includes(UNLIT_OF)) {
    throw new Error("cinema.fun's roomOf/unlitOf changed: update the e2e's draw-count variants");
  }
  const tested = async (lit, unlit) => {
    const src = project[at][1].replace(ROOM_OF, lit).replace(UNLIT_OF, unlit);
    await game.post("/reload-project", project.map(([path, s], i) => [path, i === at ? src : s]));
    await game.step(2);
    const s = await game.state();
    return { live: s.cull.tested - s.cull.culled, hidden: s.cells.hidden };
  };
  const none = await tested(times("0.0"), times("0.0"));
  const groups = await tested(times("litGroups(p.cell)"), times("unlitGroups(p.cell)"));
  const once = await tested(ROOM_OF, UNLIT_OF);
  return { label, models: once.live - none.live, draws: groups.live - none.live, hidden: once.hidden };
}

async function drawCounts(game) {
  const rows = [];
  for (const [n, label] of [[1, "lobby"], [7, "lobby, toward the entrance"], [5, "corridor A, toward the bend"],
    [2, "corridor B"], [3, "auditorium 3, up the rake"], [4, "auditorium 3, down the rake"], [6, "service corridor"],
    [9, "the parking lot"]]) {
    await game.view(n);
    rows.push(await staticDraws(game, `view ${n}: ${label}`));
  }
  for (const r of rows) {
    check(r.models > 0 && r.draws >= r.models && r.draws <= 30,
      `${r.label}: ${r.models} static models, ${r.draws} static draws, at most 30 (${r.hidden} hidden)`);
  }
  return rows;
}

const frameStats = (game) => game.log().split("\n").filter((l) => l.includes('"frame_stats"')).map((l) => JSON.parse(l));
async function statsAfter(game, n) {
  const seen = frameStats(game).length;
  await game.step(n);
  const deadline = Date.now() + 30000;
  while (frameStats(game).length <= seen && Date.now() < deadline) await sleep(50);
  return frameStats(game).at(-1);
}

async function residency(game) {
  await game.hold("c", 1);
  await game.step(3);
  await game.view(0); // the lot's far corner: the cinema is its shell
  await statsAfter(game, 300);
  const far = await game.model();
  check(far.town === TOWN && far.active.length === 0, `residency: the cinema inactive from the lot's far corner (${JSON.stringify(far.active)})`);
  const from = frameStats(game).length;
  await game.view(1); // the lobby
  await statsAfter(game, 300);
  await statsAfter(game, 300);
  const misses = frameStats(game).slice(from).reduce((n, w) => n + w.gpu_cache_misses, 0);
  const entered = (await game.model()).active;
  check(entered.includes(0) && misses === 0, `entering the lobby from the shell misses no GPU cache (${misses} misses, active ${JSON.stringify(entered)})`);
  return { misses, active: entered };
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(name) {
  const dir = mkdtempSync(join(tmpdir(), "liminal-cinema-"));
  const out = join(dir, `${name}.png`);
  const script = join(DIR, `${name.replace("liminal-office-", "")}.script`);
  const run = spawnSync(BIN, ["-d", DIR, "run", "native", "--", "--capture-frame", out, "--capture-size", "1600x1200",
    "--input-script", script, "--capture-at-frame", "30"], { cwd: ROOT, encoding: "utf8" });
  if (run.status !== 0) throw new Error(`${name}: ${run.stderr}`);
  const bytes = readFileSync(out);
  rmSync(dir, { recursive: true, force: true });
  return bytes;
}

const games = [];
try {
  const game = await launch(8496);
  games.push(game);
  await game.ready();
  await game.turn(0);
  await game.hold("c", 1);
  await game.step(3);
  let m = await game.model();
  check(m.town === TOWN, `C stands the cinema on its lot (town ${m.town})`);
  await walk(game);
  m = await game.model();
  check(m.uncovered === 0, `the cinema's cells never lost the eye (uncovered ${m.uncovered} over the walk)`);
  console.log(`static draws: ${JSON.stringify(await drawCounts(game), null, 1)}`);
  check((await game.model()).uncovered === 0, "no fallback walks while measuring");

  const stats = await launch(8497, ["--json"]);
  games.push(stats);
  await stats.ready();
  await stats.turn(0);
  console.log(`residency: ${JSON.stringify(await residency(stats))}`);

  for (const name of ["lobby-day", "corridor", "auditorium", "neon"].map((n) => `liminal-office-cinema-${n}`)) {
    check(golden(name).equals(golden(name)), `${name}: byte-identical across runs`);
  }
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  for (const g of games) g.proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${games.map((g) => g.log().slice(-3000)).join("\n")}`);
  process.exit(1);
}
console.log("\nliminal-cinema e2e: all checks passed");
