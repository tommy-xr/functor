// liminal-office's mall (P) E2E: the real physics world, a hidden window,
// the clock paused between `/time` steps.
//
//   1. the walk: from the lot through the entrance into the court, up the
//      stopped escalator's open lane onto the west bridge (a storey up, in
//      its cell), along the south gallery; the eye is in a cell of the
//      active mall at every tick (`uncovered` 0) and nothing snags;
//   2. looking down into the court from the gallery, the court's cell draws;
//   3. draws: from the court, the gallery and looking down, the static draws
//      (one per material group) stay at most 80. Variants of mall.fun's
//      `roomOf` draw each cell model 0×, once, or once per group
//      (liminal-office.mjs's method);
//   4. the mall goldens render byte-identically across runs.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-mall.mjs
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`
const WALK = 2.2 / 60; // metres per step
const STOREY = 4.5;
const CENTRE = 0.92; // the capsule's centre above its feet
// mallLayout.fun's cells.
const COURT = 8;
const BRIDGE_W = 14;
const GALLERY_S = 12;
const MALL_TOWN = 6;
const MAX_DRAWS = 80;

// Another session on the port would take this run's input: refuse it.
const PORT = 8661;
const isFree = (port) => new Promise((resolve) => {
  const s = createServer().once("error", () => resolve(false)).once("listening", () => s.close(() => resolve(true)));
  s.listen(port, "127.0.0.1");
});

function launch(port, flags = []) {
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
    mouse: { x: 400, y: 300 },
    async look(dyaw, dpitch) {
      game.mouse.x -= Math.round(dyaw / MOUSE_RATE);
      game.mouse.y -= Math.round(dpitch / MOUSE_RATE);
      await post("/input", { type: "mouse_move", x: game.mouse.x, y: game.mouse.y });
      await game.step(1);
    },
    async face(yaw, pitch) {
      const m = await game.model();
      const d = yaw - m.yaw;
      await game.look(Math.atan2(Math.sin(d), Math.cos(d)), pitch === undefined ? 0 : pitch - m.pitch);
    },
    async view(n) {
      await game.hold(String(n), 1);
      await game.step(3);
    },
    model: async () => (await state()).model,
  };
  return game;
}

// Walk straight to (x, z), `chunk` steps at a time, facing it level each
// time; `onStep` sees every sample. Stops early when blocked.
async function walkToPoint(game, [x, z], onStep, chunk = 12) {
  let m = await game.model();
  for (let leg = 0; leg < 600; leg++) {
    const [dx, dz] = [x - m.at.x, z - m.at.z];
    const left = Math.hypot(dx, dz);
    if (left < 0.05) break;
    await game.face(Math.atan2(dx, dz), 0);
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

// The lot → the entrance → the court → up the escalator's open lane → the
// west bridge → along the south gallery. Returns the cells the eye passed.
async function walk(game) {
  const snags = [];
  const seen = [];
  const go = async (x, z, label) => {
    const m = await walkToPoint(game, [x, z], (s) => seen.push(s.cell));
    if (Math.hypot(m.at.x - x, m.at.z - z) > 0.1) snags.push(`${label} (stopped at ${m.at.x.toFixed(2)}, ${m.at.y.toFixed(2)}, ${m.at.z.toFixed(2)})`);
    return m;
  };
  await go(41, -6, "apron");
  await go(41, 5, "entrance");
  let m = await go(41, 12.5, "court S");
  check(m.cell === 3 && Math.abs(m.at.y - CENTRE) < 0.05, `in the lower court strip (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  m = await go(42.5, 16.4, "escalator foot");
  check(m.cell === COURT, `at the escalator's foot, in the court's void (cell ${m.cell})`);
  // Up lane 1 (lane 2 is barred) to the top plate, then off onto the bridge.
  m = await go(31.0, 16.4, "escalator head");
  check(m.cell === COURT && Math.abs(m.at.y - (STOREY + CENTRE)) < 0.08,
    `rode (walked) the escalator up: on its top plate a storey up (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  m = await go(28.0, 16.4, "west bridge");
  check(m.cell === BRIDGE_W && Math.abs(m.at.y - (STOREY + CENTRE)) < 0.05 && m.grounded,
    `off the escalator onto the west bridge (cell ${m.cell}, y ${m.at.y.toFixed(3)}, grounded ${m.grounded})`);
  await go(28.0, 13.0, "gallery S west end");
  m = await go(50.0, 13.0, "gallery S east end");
  check(m.cell === GALLERY_S && Math.abs(m.at.y - (STOREY + CENTRE)) < 0.05, `walked the south gallery (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  // The balustrade holds: walking north into the void stops at it.
  m = await walkToPoint(game, [50.0, 20.0]);
  check(m.at.z < 15 && Math.abs(m.at.y - (STOREY + CENTRE)) < 0.05, `the balustrade stops a walk into the void (z ${m.at.z.toFixed(2)}, y ${m.at.y.toFixed(3)})`);
  check(snags.length === 0, `no snag from the lot to the gallery${snags.length ? `: ${snags.join("; ")}` : ""}`);
  const cells = [...new Set(seen)];
  for (const c of [3, COURT, BRIDGE_W, GALLERY_S]) check(cells.includes(c), `the walk passed cell ${c} (${JSON.stringify(cells)})`);
  m = await game.model();
  check(m.uncovered === 0 && JSON.stringify(m.active) === "[0]", `the eye never left the active mall's cells (uncovered ${m.uncovered}, active ${JSON.stringify(m.active)})`);
}

// Static draws from `/state.cull` (tested − culled), as liminal-office.mjs:
// variants of mall.fun's `roomOf` draw each cell model 0×, once, or once per
// material group.
const ROOM_OF = "Scene.modelOf(p.model) |> Scene.inCells([p.cell])";
const GROUPS = "List.length(MallInterior.spansOf(MallInterior.palette, 0.0, List.nth(p.cell, MallInterior.interior.cellParts) |> Option.defaultValue([])))";
const times = (n) => `Scene.group(List.range(${n}) |> List.map((k) => Scene.modelOf(p.model))) |> Scene.inCells([p.cell])`;
const COURT_ONLY = `(if p.cell == ${COURT}.0 then ${ROOM_OF} else Scene.group([]))`;

async function staticDraws(game, label) {
  const project = await (await fetch(`${game.base}/project`)).json();
  const at = project.findIndex(([path]) => path === "mall.fun" || path.endsWith("/mall.fun"));
  if (at < 0 || !project[at][1].includes(ROOM_OF)) throw new Error("mall.fun's roomOf changed: update the e2e's draw-count variants");
  const tested = async (variant) => {
    const body = project.map(([path, src], i) => [path, i === at ? src.replace(ROOM_OF, variant) : src]);
    await game.post("/reload-project", body);
    // The reload is adopted off the frame thread: sample once it runs.
    const want = body[at][1];
    for (let k = 0; ; k++) {
      await game.step(1);
      const running = await (await fetch(`${game.base}/project`)).json();
      if (running[at][1] === want) break;
      if (k > 600) throw new Error(`the ${label} variant never loaded`);
      await sleep(50);
    }
    await game.step(3);
    const s = await game.state();
    return { live: s.cull.tested - s.cull.culled, hidden: s.cells.hidden, visible: s.cells.visible };
  };
  const none = await tested(times("0.0"));
  const court = await tested(COURT_ONLY);
  const groups = await tested(times(GROUPS));
  const once = await tested(ROOM_OF);
  return { label, models: once.live - none.live, draws: groups.live - none.live, court: court.live - none.live, hidden: once.hidden, visible: once.visible };
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(name) {
  const dir = mkdtempSync(join(tmpdir(), "liminal-mall-"));
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
  if (!(await isFree(PORT))) throw new Error(`port ${PORT} is taken: another session would share this run's input`);
  const game = launch(PORT);
  games.push(game);
  await game.ready();
  await game.look(0, 0);
  await game.hold("p", 1);
  await game.step(3);
  let m = await game.model();
  check(m.town === MALL_TOWN && Math.hypot(m.at.x - 41, m.at.z + 26) < 0.5, `P goes to the mall's lot (town ${m.town}, at ${m.at.x.toFixed(1)}, ${m.at.z.toFixed(1)})`);
  await walk(game);

  // Back on the gallery's balustrade, looking down into the court.
  await walkToPoint(game, [41.0, 14.2]);
  await game.face(0, -0.9);
  m = await game.model();
  const down = await staticDraws(game, "gallery S, looking down into the court");
  check(m.cell === GALLERY_S && down.court > 0, `looking down from the gallery, the court's cell draws (cell ${m.cell}, ${down.court} court draws)`);
  const rows = [down];
  await game.view(2);
  rows.push(await staticDraws(game, "the court (view 2)"));
  await game.view(3);
  rows.push(await staticDraws(game, "the south gallery (view 3)"));
  await game.view(5);
  rows.push(await staticDraws(game, "down into the court from the north gallery (view 5)"));
  await game.view(4);
  rows.push(await staticDraws(game, "the escalators' foot (view 4)"));
  for (const r of rows) {
    check(r.models > 0 && r.draws >= r.models && r.draws <= MAX_DRAWS,
      `${r.label}: ${r.models} cell models, ${r.draws} static draws (at most ${MAX_DRAWS}), ${r.hidden} hidden`);
  }
  console.log(`draws: ${JSON.stringify(rows)}`);
  m = await game.model();
  check(m.uncovered === 0, `no fallback walks while measuring (uncovered ${m.uncovered})`);

  for (const name of ["court-day", "gallery", "escalator", "lot-dusk"].map((n) => `liminal-office-mall-${n}`)) {
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
console.log("\nliminal-mall e2e: all checks passed");
