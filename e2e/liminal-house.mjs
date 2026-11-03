// liminal-office's suburb (U) E2E: houses through the real physics world,
// with a hidden window and the clock paused between `/time` steps.
//
//   1. U lands on the suburb's road, in copy 0's cells;
//   2. view 1 (copy 0's front walk) → through the gate, up the path, over
//      the porch and in at the front door → up the stair (grounded on its
//      ramp the whole way) → the landing → the upstairs corridor → bedroom
//      3, to its window: the street shows through it, brighter by day;
//   3. a walk round the loop road: the eye is in a cell of the nearest house
//      at every sample, and no active house's cells ever miss it
//      (`uncovered` stays 0);
//   4. U again leaves it for the office's apron;
//   5. the suburb's goldens render byte-identically across runs.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-house.mjs
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decodePng } from "../scripts/png.mjs";
import { BIN, ROOT, check, failures, portIsBound, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`
const WALK = 2.2 / 60; // metres per step
const CENTRE = 0.92; // game.fun's capsule centre above the feet
// block.fun's `suburbTown`.
const SUBURB = 8;
// Cell ids are houseLayout.fun's room positions.
const PORCH = 0;
const HALL = 1;
const LANDING = 6;
const CORRIDOR = 7;
const BEDROOM_3 = 10;
// houseLayout.fun's storey and flight.
const STOREY = 2.85;
const FLIGHT = { x0: 4.4, tread: 0.28, risers: 15 };
const rampY = (x) =>
  Math.min(STOREY, Math.max(0, ((x - (FLIGHT.x0 - FLIGHT.tread)) * (STOREY / FLIGHT.risers)) / FLIGHT.tread));
// suburb.fun's copy 0: yaw 0, so a house point (x, z) is (x - 21, z + 30).
const HOME = { x: -21, z: 30 };
const local = (m) => ({ x: m.at.x - HOME.x, z: m.at.z - HOME.z });

function launch(port) {
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
    log: () => log,
    state,
    model: async () => (await state()).model,
    async ready() {
      const deadline = Date.now() + 60000;
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
    // Hold W until the player passes `target` along world `axis`; `onStep`
    // sees the model after each chunk.
    async walkTo(axis, target, { chunk = 30, onStep } = {}) {
      let m = await game.model();
      const sign = Math.sign(target - m.at[axis]);
      await game.key("w", true);
      while ((target - m.at[axis]) * sign > 0.02) {
        const before = m.at[axis];
        await game.step(Math.max(1, Math.min(chunk, Math.floor(((target - m.at[axis]) * sign) / WALK))));
        m = await game.model();
        onStep?.(m);
        if (Math.abs(m.at[axis] - before) < 0.001) break; // blocked
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

// Mean luma (0–255) over the rect given as fractions of the image.
function meanLuma(png, [x0, y0, x1, y1]) {
  const { width, height, channels, pixels } = decodePng(png);
  let sum = 0;
  let n = 0;
  for (let y = Math.floor(y0 * height); y < Math.floor(y1 * height); y++) {
    for (let x = Math.floor(x0 * width); x < Math.floor(x1 * width); x++) {
      const p = pixels.subarray((y * width + x) * channels);
      sum += 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
      n++;
    }
  }
  return sum / n;
}

// Every sample of the tours: whether the eye was in a cell of the nearest
// house.
const samples = { n: 0, outside: [] };
const sample = (m) => {
  samples.n++;
  if (m.cell < 0) samples.outside.push(`(${m.at.x.toFixed(1)}, ${m.at.z.toFixed(1)}) copy ${m.copy}`);
};

async function walkIn(game) {
  await game.turn(0); // anchor the pointer
  await game.hold("u", 1);
  await game.step(3);
  let m = await game.model();
  check(m.town === SUBURB && m.copy === 0 && m.cell === PORCH && m.baked, `U: on the suburb's road, in copy 0's porch strip (town ${m.town}, copy ${m.copy}, cell ${m.cell}, baked ${m.baked})`);

  await game.view("1");
  m = await game.model();
  check(m.copy === 0 && near(local(m).x, 1.6, 0.05) && near(local(m).z, -9, 0.05), `view 1: copy 0's front walk (local ${local(m).x.toFixed(2)}, ${local(m).z.toFixed(2)})`);
  // Up the path, through the gate and over the porch, in at the front door.
  await game.face(0);
  m = await game.walkTo("z", HOME.z + 0.7, { onStep: sample });
  check(m.cell === HALL && near(m.at.y, CENTRE, 0.05), `in at the front door (cell ${m.cell}, local z ${local(m).z.toFixed(2)}, y ${m.at.y.toFixed(3)})`);

  // East up the stair, on its ramp the whole way, onto the landing.
  await game.face(Math.PI / 2);
  const misses = [];
  let onRamp = 0;
  m = await game.walkTo("x", HOME.x + 9.2, {
    chunk: 4,
    onStep: (s) => {
      sample(s);
      const x = local(s).x;
      if (x < FLIGHT.x0 || x > FLIGHT.x0 + (FLIGHT.risers - 1) * FLIGHT.tread) return;
      onRamp++;
      const above = s.at.y - (rampY(x) + CENTRE);
      if (!s.grounded || above < -0.02 || above > 0.12) misses.push(`x ${x.toFixed(2)} +${above.toFixed(3)} grounded ${s.grounded}`);
    },
  });
  check(onRamp >= 20 && misses.length === 0, `up the flight: grounded on the ramp at ${onRamp} samples${misses.length ? `; off at ${misses.slice(0, 3).join(", ")}` : ""}`);
  check(m.cell === LANDING && near(m.at.y, STOREY + CENTRE, 0.05), `on the landing (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);

  // North through the landing's door, west along the corridor, north into
  // bedroom 3, east to its window.
  await game.face(0);
  m = await game.walkTo("z", HOME.z + 3.6, { onStep: sample });
  check(m.cell === CORRIDOR, `through the landing's door into the upstairs corridor (cell ${m.cell})`);
  await game.face(-Math.PI / 2);
  await game.walkTo("x", HOME.x + 8.3, { onStep: sample });
  await game.face(0);
  m = await game.walkTo("z", HOME.z + 7.0, { onStep: sample });
  check(m.cell === BEDROOM_3 && near(m.at.y, STOREY + CENTRE, 0.05), `into bedroom 3 (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  await game.face(Math.PI / 2);
  m = await game.walkTo("x", HOME.x + 10.5, { onStep: sample });
  check(m.cell === BEDROOM_3 && local(m).x < 9.7, `the window's sill stops the walk (local x ${local(m).x.toFixed(2)}, cell ${m.cell})`);

  // Out of the window: the street (and the next house) by night, brighter
  // by day; the window is in the middle of the view.
  const WINDOW = [0.42, 0.42, 0.58, 0.58];
  const night = meanLuma(await game.capture(), WINDOW);
  await game.hold("n", 1);
  await game.step(2);
  const noon = meanLuma(await game.capture(), WINDOW);
  await game.hold("n", 1);
  check(noon > night + 40, `out of bedroom 3's window: day outside (luma ${night.toFixed(1)} → ${noon.toFixed(1)})`);
  m = await game.model();
  check(m.active.includes(0) && m.uncovered === 0, `copy 0 draws, its cells hold the eye (active ${JSON.stringify(m.active)}, uncovered ${m.uncovered})`);
}

// Round the loop road's middle (view 9 is on it): a square 18 m out from
// the green's middle, corner to corner.
async function loop(game) {
  await game.view("9");
  const R = 18;
  const missed = [];
  for (const [x, z] of [[R, R], [R, -R], [-R, -R], [-R, R], [-15, R]]) {
    let m = await game.model();
    for (let leg = 0; leg < 80; leg++) {
      const [dx, dz] = [x - m.at.x, z - m.at.z];
      const left = Math.hypot(dx, dz);
      if (left < 0.1) break;
      await game.face(Math.atan2(dx, dz));
      await game.key("w", true);
      await game.step(Math.max(1, Math.min(20, Math.floor(left / WALK))));
      await game.key("w", false);
      await game.step(1);
      m = await game.model();
      sample(m);
    }
    if (Math.hypot(x - m.at.x, z - m.at.z) > 0.2) missed.push(`(${x}, ${z})`);
  }
  check(missed.length === 0, `walked the whole loop${missed.length ? `; missed ${missed.join(", ")}` : ""}`);
  const m = await game.model();
  check(m.active.length <= 4, `at most 4 interiors on the loop (${JSON.stringify(m.active)})`);
  check(samples.n > 150 && samples.outside.length === 0, `the eye was in a cell of the nearest house at all ${samples.n} samples${samples.outside.length ? `; not at ${samples.outside.slice(0, 3).join(", ")}` : ""}`);
  check(m.uncovered === 0, `no active house's cells ever lost the eye (uncovered ${m.uncovered})`);
}

// U again leaves the suburb for the office alone's entrance (view 1).
async function leave(game) {
  await game.view("u");
  const m = await game.model();
  check(m.town === 0 && m.cell === 0 && m.grounded, `U leaves the suburb onto the office's apron (town ${m.town}, cell ${m.cell}, grounded ${m.grounded})`);
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(script) {
  const out = join(mkdtempSync(join(tmpdir(), "liminal-house-")), "golden.png");
  const run = spawnSync(BIN, ["-d", DIR, "run", "native", "--", "--capture-frame", out, "--capture-size", "1600x1200",
    "--input-script", join(DIR, script), "--capture-at-frame", "30"], { cwd: ROOT, encoding: "utf8" });
  if (run.status !== 0) throw new Error(`${script}: ${run.stderr}`);
  const bytes = readFileSync(out);
  rmSync(out, { force: true });
  return bytes;
}

let game;
try {
  // Another game on the port would answer instead of ours.
  const PORT = 8247;
  if (await portIsBound(PORT)) throw new Error(`port ${PORT} is already in use`);
  game = launch(PORT);
  await game.ready();
  await walkIn(game);
  await loop(game);
  await leave(game);
  for (const script of ["house-porch-day.script", "house-stair-day.script", "suburb-street.script"]) {
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
console.log("\nliminal-house e2e: all checks passed");
