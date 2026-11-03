// liminal-office's towers (H) E2E: the real physics world, a hidden window,
// the clock paused between `/time` steps.
//
//   1. the climb: in each of the four towers (one per yaw, 8, 16, 12 and 8
//      floors), from the lobby through the stair door, up every dog-leg
//      (lane, half-landing, lane, landing) to the roof's head room and out
//      onto the terrace. Every waypoint is reached (nothing snags), each
//      landing is a storey up and in that floor's stairwell cell, and the
//      eye is in a cell of every active copy at every tick (`uncovered` 0);
//   2. culling, the eye's tower alone (K = 1): down the well from floor 6's
//      landing every non-stair cell is hidden and at most 8 stairwell cells
//      draw; down a corridor at most two floors' cells draw. A variant of
//      tower.fun's `nightOf` drawing only stairwell cells tells them apart;
//   3. residency (`--json` frame stats): entering a tower after a walk out
//      of every interior misses no GPU cache;
//   4. the tower goldens render byte-identically across runs.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-tower.mjs
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`
const WALK = 2.2 / 60; // metres per step
const STOREY = 3.3;
const CENTRE = 0.92; // the capsule's centre above its feet
// towerLayout.fun: floor k's room j is cell 8k + j; the stairwell is room 2
// on the ground, 0 above; the roof's head room and terrace are 0 and 1.
const PER_FLOOR = 8;
const stairCell = (k) => (k === 0 ? 2 : PER_FLOOR * k);
// block.fun's towers: copy k turns 90k° about the crossroads (-12, -12).
const FLOORS = [8, 16, 12, 8];
const CROSS = -12;
const turnXZ = (deg, x, z) => {
  const a = (deg * Math.PI) / 180;
  return [x * Math.cos(a) + z * Math.sin(a), z * Math.cos(a) - x * Math.sin(a)];
};
const COPIES = FLOORS.map((floors, k) => {
  const [x, z] = turnXZ(90 * k, CROSS, CROSS);
  return { k, floors, yaw: 90 * k, x: CROSS - x, z: CROSS - z };
});
const toWorld = (c, x, z) => {
  const [a, b] = turnXZ(c.yaw, x, z);
  return [a + c.x, b + c.z];
};

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

// Walk straight to world (x, z), `chunk` steps at a time, facing it each
// time; `onStep` sees every sample. Stops early when blocked.
async function walkToPoint(game, [x, z], onStep, chunk = 12) {
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

// Up tower c from its lobby to its terrace, checking each landing.
async function climb(game, c) {
  const snags = [];
  const at = (x, z) => toWorld(c, x, z);
  const go = async (x, z, label) => {
    const m = await walkToPoint(game, at(x, z));
    const [wx, wz] = at(x, z);
    if (Math.hypot(m.at.x - wx, m.at.z - wz) > 0.1) snags.push(`${label} (stopped at ${m.at.x.toFixed(2)}, ${m.at.y.toFixed(2)}, ${m.at.z.toFixed(2)})`);
    return m;
  };
  // Through the lobby and the stair door onto the ground landing.
  await go(9.2, 4.0, "lobby");
  await go(9.2, 9.6, "stair door");
  let m = await go(8.3, 10.0, "ground landing");
  check(m.copy === c.k && m.cell === stairCell(0) && Math.abs(m.at.y - CENTRE) < 0.05,
    `tower ${c.k}: on the ground landing (copy ${m.copy}, cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  const wrong = [];
  for (let k = 0; k <= c.floors; k++) {
    await go(8.3, 14.1, `floor ${k} up lane`);
    await go(10.1, 14.1, `floor ${k} half-landing`);
    await go(10.1, 10.0, `floor ${k} back lane`);
    m = await go(8.3, 10.0, `floor ${k + 1} landing`);
    const y = (k + 1) * STOREY + CENTRE;
    if (m.cell !== stairCell(k + 1) || Math.abs(m.at.y - y) > 0.05 || !m.grounded) {
      wrong.push(`floor ${k + 1}: cell ${m.cell}, y ${m.at.y.toFixed(3)}, grounded ${m.grounded}`);
    }
  }
  // Out of the head room's door onto the terrace.
  await go(9.2, 10.0, "head room");
  m = await go(9.2, 4.0, "terrace");
  const roof = (c.floors + 1) * STOREY;
  check(snags.length === 0, `tower ${c.k} (${c.floors} floors, yaw ${c.yaw}): no snag on the way up${snags.length ? `: ${snags.slice(0, 3).join("; ")}` : ""}`);
  check(wrong.length === 0, `tower ${c.k}: every landing a storey up, in its floor's stairwell${wrong.length ? `: ${wrong.slice(0, 3).join("; ")}` : ""}`);
  check(m.cell === PER_FLOOR * (c.floors + 1) + 1 && Math.abs(m.at.y - (roof + CENTRE)) < 0.05,
    `tower ${c.k}: out on the roof terrace (cell ${m.cell}, y ${m.at.y.toFixed(3)} vs ${(roof + CENTRE).toFixed(3)})`);
}

// tower.fun's `nightOf`, and a variant drawing only stairwell cells.
const NIGHT_OF = "Scene.modelOf(p.model.model) |> Scene.translate(v(0.0, p.dy, 0.0)) |> Scene.inCells([p.cell])";
const STAIRS_ONLY = `(if p.cell == 2.0 || (p.cell >= 8.0 && Math.mod(p.cell, 8.0) == 0.0) then ${NIGHT_OF} else Scene.group([]))`;
// The tower-8's cell models, and how many are stairwell cells (tests.fun pins both).
const TOWER8_MODELS = 88;
const TOWER8_STAIRS = 10;

async function cullCounts(game, variant) {
  const project = await (await fetch(`${game.base}/project`)).json();
  const at = project.findIndex(([path]) => path.endsWith("tower.fun"));
  if (at < 0 || !project[at][1].includes(NIGHT_OF)) throw new Error("tower.fun's nightOf changed: update the e2e's variant");
  const body = project.map(([path, src], i) => [path, i === at && variant ? src.replace(NIGHT_OF, variant) : src]);
  await game.post("/reload-project", body);
  await game.step(10);
  const s = await game.state();
  return s.cells;
}

async function culling(game) {
  await game.hold("k", 1); // K = 1: only the eye's own tower draws
  await game.step(2);
  // View 3: floor 3's south corridor, looking west. (Measured before the
  // variant reloads below: a reload back doesn't rebuild an unchanged
  // `Scene.memo`'d copy.)
  await game.view(3);
  await game.step(3);
  const corridor = (await game.state()).cells;
  check(TOWER8_MODELS - corridor.hidden <= 2 * PER_FLOOR,
    `corridor view: ${TOWER8_MODELS - corridor.hidden} cells draw, at most two floors' (${2 * PER_FLOOR})`);
  // View 6: floor 6's landing in the nearest tower (the last climbed, 8
  // floors), down the well.
  await game.view(6);
  let m = await game.model();
  check(FLOORS[m.copy] === 8 && m.cell === stairCell(6) && JSON.stringify(m.active) === `[${m.copy}]`,
    `well view: on floor 6's landing, only its tower active (copy ${m.copy}, cell ${m.cell}, active ${JSON.stringify(m.active)})`);
  const all = await cullCounts(game, null);
  const stairs = await cullCounts(game, STAIRS_ONLY);
  const drawn = TOWER8_MODELS - all.hidden;
  const stairsDrawn = TOWER8_STAIRS - stairs.hidden;
  check(all.hidden - stairs.hidden === TOWER8_MODELS - TOWER8_STAIRS,
    `well view: every non-stair cell hidden (${all.hidden} hidden in all, ${stairs.hidden} stairwell cells hidden; ${drawn} drawn)`);
  check(drawn === stairsDrawn && stairsDrawn <= 8, `well view: ${stairsDrawn} stairwell cells draw, at most 8 (${all.visible} cells reached)`);
  await cullCounts(game, null);
  for (let i = 0; i < 2; i++) await game.hold("k", 1); // 1 → 2 → 4
  m = await game.model();
  check(m.k === 4 && m.uncovered === 0, `culling: back at K 4, uncovered ${m.uncovered}`);
  return { well: { hidden: all.hidden, drawn, reached: all.visible }, corridor: { hidden: corridor.hidden, drawn: TOWER8_MODELS - corridor.hidden } };
}

const frameStats = (game) => game.log().split("\n").filter((l) => l.includes('"frame_stats"')).map((l) => JSON.parse(l));
async function statsAfter(game, n) {
  const seen = frameStats(game).length;
  await game.step(n);
  const deadline = Date.now() + 30000;
  while (frameStats(game).length <= seen && Date.now() < deadline) await sleep(50);
  return frameStats(game).at(-1);
}

// Walk south out of every tower's reach, then jump into tower 0's lobby.
async function residency(game) {
  await game.hold("h", 1);
  await game.view(0); // the street south of the crossroads
  await walkToPoint(game, [CROSS, -110]);
  await statsAfter(game, 300);
  const far = await game.model();
  check(far.town === 5 && far.active.length === 0, `residency: no tower active at z ${far.at.z.toFixed(1)} (${JSON.stringify(far.active)})`);
  const from = frameStats(game).length;
  await game.view(9); // tower 0's roof terrace
  await statsAfter(game, 300);
  await statsAfter(game, 300);
  const spanned = frameStats(game).slice(from);
  const misses = spanned.reduce((n, w) => n + w.gpu_cache_misses, 0);
  const entered = (await game.model()).active;
  check(entered.includes(0) && misses === 0, `entering tower 0 after the walk out misses no GPU cache (${misses} misses, active ${JSON.stringify(entered)})`);
  return { misses, active: entered };
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(name) {
  const dir = mkdtempSync(join(tmpdir(), "liminal-tower-"));
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
  const game = launch(8194);
  games.push(game);
  await game.ready();
  await game.turn(0);
  await game.hold("h", 1);
  let m = await game.model();
  check(m.town === 5, `H turns the office into the towers (town ${m.town})`);
  for (const c of COPIES) {
    await game.view(0); // the street south of the crossroads
    await walkToPoint(game, [CROSS, CROSS]);
    await walkToPoint(game, toWorld(c, 12.0, -6.0));
    await game.view(1); // the nearest tower's lobby
    m = await game.model();
    check(m.copy === c.k, `tower ${c.k}: view 1 lands in its lobby (copy ${m.copy}, cell ${m.cell})`);
    await climb(game, c);
  }
  m = await game.model();
  check(m.uncovered === 0, `no active tower's cells ever lost the eye (uncovered ${m.uncovered} over the climbs)`);
  const cull = await culling(game);
  console.log(`culling: ${JSON.stringify(cull)}`);

  const stats = launch(8195, ["--json"]);
  games.push(stats);
  await stats.ready();
  await stats.turn(0);
  console.log(`residency: ${JSON.stringify(await residency(stats))}`);

  for (const name of ["lifts", "well-down", "well-up", "roof"].map((n) => `liminal-office-tower-${n}`)) {
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
console.log("\nliminal-tower e2e: all checks passed");
