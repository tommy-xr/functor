// liminal-office's parks (R, and on the island) E2E: through the real
// physics world, with a hidden window and the clock paused between `/time`
// steps.
//
//   1. R lands at the park's south gate, beside the block;
//   2. up the path from the gate: no collider on it, and the fountain's rim
//      stops the walk; along the paths to the playground, where the slide's
//      ladder stops it; and the monolith, which stops it too;
//   3. a park and the plaza draw once per material (lit boxes, paving,
//      globes, signs; the plaza's jets), by day and by night;
//   4. the plaza's jets move with the clock;
//   5. R again leaves for the office's apron;
//   6. on the island (I from the park town): the plaza stands on the
//      ground, its rim solid, its draws once per material;
//   7. the park goldens render byte-identically across runs.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-park.mjs
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { GROUND_Y, ground, padY, toTerrain } from "../examples/liminal-office/island.mjs";
import { decodePng } from "../scripts/png.mjs";
import { BIN, ROOT, check, failures, portIsBound, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`
const WALK = 2.2 / 60; // metres per step
const STOREY = 3.3; // officeLayout.fun's: a view's level is in storeys
// block.fun's `parkTown`; parks.fun's town park and plaza middles (both
// unturned), its fountain's rim (outer face) and the plaza's.
const PARK_TOWN = 9;
const PARK = { x: -100, z: -12 };
const PLAZA = { x: -12, z: -84 };
const RIM = 2.6 + 0.35;
const PLAZA_RIM = 5.0 + 0.45;
const CAPSULE = 0.3;
const local = (m) => ({ x: m.at.x - PARK.x, z: m.at.z - PARK.z });
// islandLayout.fun's plaza.
const ISLAND_PLAZA = (() => {
  const src = readFileSync(join(DIR, "islandLayout.fun"), "utf8");
  const m = src.match(/name: "island-plaza", kind: "plaza", x: (-?[\d.]+), y: -?[\d.]+, z: (-?[\d.]+), yaw: (-?[\d.]+)/);
  if (!m) throw new Error("islandLayout.fun has no island-plaza: update the e2e");
  return { x: Number(m[1]), z: Number(m[2]), yaw: Number(m[3]) };
})();
const islandGround = (x, z) => ground(...toTerrain(x, z)) - padY + GROUND_Y;
const PARK_SCENE = "let parkScene = Parks.scenesAt(parks, eye, far, o.lamps, day, tts) in";
const VIEW_9 = "{ copy: 0.0, x: -12.0, level: 0.0, z: -50.0, yaw: 0.0, pitch: 0.04 },";

async function launch(port) {
  // Another game on the port would answer instead of ours.
  if (await portIsBound(port)) throw new Error(`port ${port} is taken: stop what's listening there`);
  const proc = spawn(BIN, ["-d", DIR, "run", "native", "--hidden", "--debug-port", String(port)], {
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
    proc,
    base,
    post,
    log: () => log,
    state,
    model: async () => (await state()).model,
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
    async view(key) {
      await game.hold(key, 1);
      await game.step(3);
    },
    // Face `yaw` (0 is +z, π/2 is +x).
    async face(yaw) {
      const d = yaw - (await game.model()).yaw;
      await game.turn(Math.atan2(Math.sin(d), Math.cos(d)));
    },
    // Hold W toward (x, z) until there or blocked; `onStep` sees each chunk.
    async walkToward(x, z, { chunk = 10, onStep } = {}) {
      let m = await game.model();
      await game.face(Math.atan2(x - m.at.x, z - m.at.z));
      await game.key("w", true);
      for (let k = 0; k < 400; k++) {
        const left = Math.hypot(x - m.at.x, z - m.at.z);
        if (left < 0.05) break;
        const before = m.at;
        await game.step(Math.max(1, Math.min(chunk, Math.floor(left / WALK))));
        m = await game.model();
        onStep?.(m);
        if (Math.hypot(m.at.x - before.x, m.at.z - before.z) < 0.002) break; // blocked
      }
      await game.key("w", false);
      await game.step(1);
      return game.model();
    },
    async capture() {
      return Buffer.from(await (await post("/capture", {})).arrayBuffer());
    },
  };
  return game;
}

const near = (a, b, tol) => Math.abs(a - b) <= tol;
const pushVariant = (game, project, from, to) =>
  game.post("/reload-project", project.map(([path, src], i) => [path, i === 0 ? src.replace(from, to) : src]));

// The parks' draws from where the eye stands: what the frame submits (past
// frustum culling) with the parks, less without them.
async function parkDraws(game, label) {
  const project = await (await fetch(`${game.base}/project`)).json();
  if (!project[0][1].includes(PARK_SCENE)) throw new Error("game.fun's park scene changed: update the e2e");
  const live = async (variant) => {
    await pushVariant(game, project, PARK_SCENE, variant);
    await game.step(2);
    const s = await game.state();
    return s.cull.tested - s.cull.culled;
  };
  const without = await live("let parkScene = Scene.group([]) in");
  const draws = (await live(PARK_SCENE)) - without;
  console.log(`  ${label}: ${draws} park draws`);
  return draws;
}

async function walk(game) {
  await game.turn(0); // anchor the pointer
  await game.view("r");
  let m = await game.model();
  check(m.town === PARK_TOWN && near(local(m).x, 0, 0.05) && near(local(m).z, -19.5, 0.05) && m.grounded,
    `R: at the park's south gate (town ${m.town}, local ${local(m).x.toFixed(2)}, ${local(m).z.toFixed(2)}, grounded ${m.grounded})`);
  // North up the path: nothing solid on it; the fountain's rim stops it.
  const off = [];
  m = await game.walkToward(PARK.x, PARK.z, { onStep: (s) => { if (!s.grounded) off.push(local(s).z.toFixed(1)); } });
  check(near(local(m).z, -RIM - CAPSULE, 0.1) && off.length === 0,
    `up the path from the gate to the fountain: its rim stops the walk (local z ${local(m).z.toFixed(2)}, rim at ${-RIM}${off.length ? `; off the ground at ${off.join(", ")}` : ""})`);
  // Round the ring to the playground path, along it to the playground, and
  // at the slide's ladder.
  for (const [x, z] of [[3.7, -3.7], [5.2, 0], [3.7, 3.7], [5.8, 6.1], [8.5, 6.2]]) await game.walkToward(PARK.x + x, PARK.z + z);
  m = await game.model();
  check(Math.hypot(local(m).x - 8.5, local(m).z - 6.2) < 0.2, `along the paths to the playground (local ${local(m).x.toFixed(2)}, ${local(m).z.toFixed(2)})`);
  // West of the slide, then square at its ladder (its collider's west face
  // at local x 7.2), so the capsule can't slide round it.
  await game.walkToward(PARK.x + 5.0, PARK.z + 13.4);
  m = await game.walkToward(PARK.x + 9.0, PARK.z + 13.4);
  check(near(local(m).x, 7.2 - CAPSULE, 0.1), `the slide's ladder stops the walk (local x ${local(m).x.toFixed(2)})`);
  // View 0 is by the monolith: square at its face (its plinth's front at
  // local z -8.52), it stops the walk.
  await game.view("0");
  await game.walkToward(PARK.x - 12, PARK.z - 12);
  m = await game.walkToward(PARK.x - 12, PARK.z - 8.2);
  check(near(local(m).z, -8.52 - CAPSULE, 0.1), `the monolith stops the walk (local z ${local(m).z.toFixed(2)})`);
  check(m.uncovered === 0, `no active copy's cells lost the eye (uncovered ${m.uncovered})`);
}

async function draws(game) {
  await game.view("1");
  const gateNight = await parkDraws(game, "park gate (view 1), midnight");
  await game.view("6");
  const plazaNight = await parkDraws(game, "plaza (view 6), midnight");
  await game.hold("n", 1);
  await game.step(2);
  const plazaNoon = await parkDraws(game, "plaza (view 6), noon");
  await game.view("1");
  const gateNoon = await parkDraws(game, "park gate (view 1), noon");
  await game.hold("n", 1);
  // From the gate the park (the plaza is behind): its lit cube, paving,
  // globes and signs. From the plaza both: its lit cube, paving, globes and
  // jets, and the park's four.
  check(gateNight <= 5 && gateNoon <= 5 && plazaNight <= 9 && plazaNoon <= 9 && gateNoon >= 3 && plazaNoon >= 6,
    `the parks draw once per material: from the gate ${gateNight} by night, ${gateNoon} by noon (the park); from the plaza ${plazaNight} and ${plazaNoon} (it and the park)`);
}

// How many pixels differ between two captures within the rect given as
// fractions of the image.
function changed(a, b, [x0, y0, x1, y1]) {
  const A = decodePng(a);
  const B = decodePng(b);
  let n = 0;
  for (let y = Math.floor(y0 * A.height); y < Math.floor(y1 * A.height); y++) {
    for (let x = Math.floor(x0 * A.width); x < Math.floor(x1 * A.width); x++) {
      const o = (y * A.width + x) * A.channels;
      if (A.pixels[o] !== B.pixels[o] || A.pixels[o + 1] !== B.pixels[o + 1] || A.pixels[o + 2] !== B.pixels[o + 2]) n++;
    }
  }
  return n;
}
// The jets step 6 times a second (every 10th frame at 60). Frame to frame
// the paving shimmers a little (the standing capsule settles), so over 12
// consecutive frames the fountain's box changes most, by far, at a jet step.
async function jets(game) {
  await game.view("6");
  await game.step(30);
  const BOX = [0.34, 0.45, 0.66, 0.66];
  let last = await game.capture();
  const counts = [];
  for (let k = 0; k < 12; k++) {
    await game.step(1);
    const next = await game.capture();
    counts.push(changed(last, next, BOX));
    last = next;
  }
  const sorted = [...counts].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  check(sorted[sorted.length - 1] > 2 * median + 100, `the plaza's jets step with the clock (pixels changed frame to frame: ${counts.join(", ")})`);
}

async function island(game) {
  await game.hold("i", 1);
  for (let k = 0; k < 100 && !((await game.state()).terrain?.patches > 0); k++) {
    await sleep(100);
    await game.step(1);
  }
  await game.step(30);
  let m = await game.model();
  check(m.island && m.town === 1, `I from the park town: the block on the island (island ${m.island}, town ${m.town})`);
  // West of the island plaza (its gate faces the town), facing its fountain.
  const project = await (await fetch(`${game.base}/project`)).json();
  if (!project[0][1].includes(VIEW_9)) throw new Error("game.fun's view 9 changed: update the e2e");
  const a = (ISLAND_PLAZA.yaw * Math.PI) / 180;
  // The plaza's local south, in the world: `Block.turn` of (0, -1).
  const [sx, sz] = [-Math.sin(a), -Math.cos(a)];
  const [x, z] = [ISLAND_PLAZA.x + 16 * sx, ISLAND_PLAZA.z + 16 * sz];
  await pushVariant(game, project, VIEW_9, `{ copy: 0.0, x: ${x}, level: ${(islandGround(x, z) + 0.3) / STOREY}, z: ${z}, yaw: 0.0, pitch: 0.0 },`);
  await game.view("9");
  await game.step(30);
  m = await game.model();
  check(m.grounded, `stands outside the island plaza (${m.at.x.toFixed(1)}, ${m.at.z.toFixed(1)}, grounded ${m.grounded})`);
  m = await game.walkToward(ISLAND_PLAZA.x, ISLAND_PLAZA.z);
  const d = Math.hypot(m.at.x - ISLAND_PLAZA.x, m.at.z - ISLAND_PLAZA.z);
  check(m.grounded && near(d, PLAZA_RIM + CAPSULE, 0.15), `across the island plaza to its fountain: the rim stops the walk (${d.toFixed(2)} m from its middle)`);
  const n = await parkDraws(game, "island plaza, midnight");
  check(n >= 3 && n <= 16, `island parks in view draw once per material (${n} draws)`);
  await game.post("/reload-project", project);
  await game.step(2);
}

async function leave(game) {
  await game.view("r");
  const m = await game.model();
  check(m.town === 0 && m.cell === 0 && m.grounded && !m.island, `R again: the office's apron (town ${m.town}, cell ${m.cell}, grounded ${m.grounded})`);
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(script) {
  const out = join(mkdtempSync(join(tmpdir(), "liminal-park-")), "golden.png");
  const run = spawnSync(BIN, ["-d", DIR, "run", "native", "--", "--capture-frame", out, "--capture-size", "1600x1200",
    "--input-script", join(DIR, script), "--capture-at-frame", "30"], { cwd: ROOT, encoding: "utf8" });
  if (run.status !== 0) throw new Error(`${script}: ${run.stderr}`);
  const bytes = readFileSync(out);
  rmSync(out, { force: true });
  return bytes;
}

const PORT = Number(process.env.LIMINAL_PARK_PORT ?? 8263);
let game;
try {
  game = await launch(PORT);
  await game.ready();
  await walk(game);
  await draws(game);
  await jets(game);
  await leave(game);
  await game.view("r");
  await island(game);
  for (const script of ["park-day.script", "plaza-night.script"]) {
    check(golden(script).equals(golden(script)), `${script}: byte-identical across runs`);
  }
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  game?.proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${game?.log().slice(-3000)}`);
  process.exit(1);
}
console.log("\nliminal-park e2e: all checks passed");
