// Hoverbike E2E: a scripted ride on the flat test stand, headless over the
// debug server with the clock paused and stepped (`/time advance`).
//
//   1. Settled hover clearance is 1.2 m ± 5 cm, and stays there while riding.
//   2. Full throttle (W) follows the analytic thrust/drag speed curve.
//   3. Full brake (gamepad left trigger) stops within the analytic distance.
//   4. A steady turn (gamepad trigger + stick) holds the speed-dependent
//      yaw-rate cap and the analytic radius; speed × yaw rate stays under the
//      lateral-acceleration bound throughout.
//   5. Respawn (R), then full throttle into the east wall at a shallow angle:
//      the bike never passes it, slides along it near top speed, and the
//      north wall stops it.
//   6. Two runs end on an identical model.
//   7. Live at a simulated 120 Hz, the drawn rig moves every frame: `draw`
//      interpolates between the 60 Hz steps.
//
// Run (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/hoverbike.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;
const SAMPLE = "examples/hoverbike";
const DT = 1 / 60;

// Mirrors examples/hoverbike/ride.fun and game.fun.
const HOVER = 1.2;
const THRUST = 12;
const TOP = 24;
const DRAG_LIN = 0.3;
const DRAG_QUAD = (THRUST - DRAG_LIN * TOP) / (TOP * TOP);
const BRAKE = 14;
const MAX_YAW = 1;
const MAX_LAT = 5;
const GRIP = 4;
const WALL = 15; // the east wall's inner face
const NORTH = 210; // the north wall's inner face
const RADIUS = 0.8; // the bike's capsule

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function get(base, path) {
  const res = await fetch(`${base}${path}`);
  if (!res.ok) throw new Error(`GET ${path} → ${res.status}`);
  return res.json();
}

async function post(base, path, body) {
  const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
  return text;
}

async function waitFrame(base, atLeast) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    try {
      const s = await get(base, "/state");
      if (s.frame >= atLeast) return s;
    } catch {
      // not listening yet
    }
    await sleep(50);
  }
  throw new Error(`runtime never reached frame ${atLeast}`);
}

/** Run `n` paused steps and return the model once they have all landed. */
async function step(base, n) {
  const before = (await get(base, "/state")).frame;
  await post(base, "/time", { type: "advance", dts: DT, frames: n });
  const deadline = Date.now() + 30000;
  for (;;) {
    const s = await get(base, "/state");
    if (s.frame >= before + n && s.pending_steps === 0) return s.model;
    if (Date.now() > deadline) throw new Error(`${n} steps from frame ${before} never landed (frame ${s.frame}, pending ${s.pending_steps})`);
    await sleep(5);
  }
}

const key = (base, k, down) => post(base, "/input", { type: "key", key: k, down });

/** The analytic speed curve from rest at full throttle (see ride.fun). */
function throttleSpeed(t) {
  const vMinus = -DRAG_LIN / DRAG_QUAD - TOP;
  const r = (TOP / vMinus) * Math.exp(-DRAG_QUAD * (TOP - vMinus) * t);
  return (TOP - r * vMinus) / (1 - r);
}

/** Braking distance from v0 under brake + drag, integrated finely. */
function stoppingDistance(v0) {
  let v = v0;
  let d = 0;
  const h = 1e-4;
  while (v > 0) {
    v -= (BRAKE + DRAG_LIN * v + DRAG_QUAD * v * v) * h;
    d += Math.max(v, 0) * h;
  }
  return d;
}

const yawCap = (speed) => Math.min(MAX_YAW, MAX_LAT / Math.max(speed, 0.001));

/** ride.fun's steady turn at total speed v: the throttle that holds it. */
function steadyTurn(v) {
  const w = yawCap(v);
  const vF = v / Math.sqrt(1 + (w / GRIP) ** 2);
  return { w, throttle: (DRAG_LIN * vF + DRAG_QUAD * vF * vF + (w * w * vF) / GRIP) / THRUST };
}

/** The radius of the circle through three ground points. */
function circumradius(a, b, c) {
  const d = (p, q) => Math.hypot(p.x - q.x, p.z - q.z);
  const area = Math.abs((b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z)) / 2;
  return (d(a, b) * d(b, c) * d(c, a)) / (4 * area);
}

async function ride(port, check) {
  const base = `http://127.0.0.1:${port}`;
  const proc = spawn(
    BIN,
    ["-d", SAMPLE, "run", "native", "--headless", "--display-hz", "60", "--debug-port", String(port)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  try {
    // Pause, then rewind to a fixed early frame so both runs start alike.
    await waitFrame(base, 12);
    await step(base, 1);
    await post(base, "/rewind", { frame: 10 });

    // 1. Settle.
    let m = await step(base, 120);
    check(Math.abs(m.clearance - HOVER) < 0.05, `settled clearance ${m.clearance.toFixed(4)} m`);
    const start = m.last;

    // 2. Full throttle for 3 s, straight ahead.
    await key(base, "w", true);
    let clearance = [m.clearance, m.clearance];
    for (let i = 0; i < 18; i++) {
      m = await step(base, 10);
      clearance = [Math.min(clearance[0], m.clearance), Math.max(clearance[1], m.clearance)];
    }
    await key(base, "w", false);
    const expect = throttleSpeed(3);
    check(Math.abs(m.speed - expect) < 0.03 * expect, `3 s of throttle: ${m.speed.toFixed(3)} m/s (analytic ${expect.toFixed(3)})`);
    check(m.ride.yaw === 0 && Math.abs(m.last.x - start.x) < 1e-3, `straight: yaw ${m.ride.yaw}, x ${m.last.x}`);

    // 3. Brake on the gamepad's left trigger until stopped.
    const v0 = m.speed;
    await post(base, "/input", { type: "gamepad", left_trigger: 1 });
    // The first braked step reads where the braking starts.
    m = await step(base, 1);
    const from = m.last;
    let reversed = false;
    for (let i = 0; i < 60 && m.speed > 0; i++) {
      m = await step(base, 2);
      reversed ||= m.ride.vz < 0;
      clearance = [Math.min(clearance[0], m.clearance), Math.max(clearance[1], m.clearance)];
    }
    await post(base, "/input", { type: "gamepad_clear" });
    m = await step(base, 2);
    const stopped = m.last.z - from.z;
    const analytic = stoppingDistance(v0);
    check(m.speed === 0 && !reversed, `brake stops without reversing (speed ${m.speed})`);
    check(Math.abs(stopped - analytic) < 0.1 * analytic, `stopping distance ${stopped.toFixed(2)} m (analytic ${analytic.toFixed(2)} from ${v0.toFixed(2)} m/s)`);
    check(clearance[0] > HOVER - 0.05 && clearance[1] < HOVER + 0.05, `clearance while riding ${clearance.map((c) => c.toFixed(4)).join("..")} m`);

    // 4. A steady right turn on the gamepad: the trigger holds 8 m/s against
    // drag and the slip the turn scrubs off (ride.fun's steady-turn balance).
    const turn = steadyTurn(8);
    await post(base, "/input", { type: "gamepad", right_trigger: turn.throttle, left_stick: [1, 0] });
    let worstYaw = 0;
    let worstLat = 0;
    const turning = [];
    for (let i = 0; i < 60; i++) {
      m = await step(base, 10);
      worstYaw = Math.max(worstYaw, Math.abs(m.ride.yawRate));
      worstLat = Math.max(worstLat, m.speed * Math.abs(m.ride.yawRate));
      turning.push(m);
    }
    await post(base, "/input", { type: "gamepad_clear" });
    const settled = turning.slice(-12);
    const capError = Math.max(...settled.map((s) => Math.abs(Math.abs(s.ride.yawRate) - yawCap(s.speed)) / yawCap(s.speed)));
    const [a, b, c] = [settled[0], settled[6], settled[11]].map((s) => s.last);
    const radius = circumradius(a, b, c);
    const expectRadius = m.speed / yawCap(m.speed);
    check(Math.abs(m.speed - 8) < 0.4, `steady turn holds ${m.speed.toFixed(3)} m/s`);
    check(m.ride.yawRate < 0, `right stick turns right (yaw rate ${m.ride.yawRate.toFixed(4)})`);
    check(capError < 0.03, `yaw rate holds its speed cap within 3% (worst ${(capError * 100).toFixed(2)}%)`);
    check(Math.abs(radius - expectRadius) < 0.1 * expectRadius, `turn radius ${radius.toFixed(2)} m (analytic ${expectRadius.toFixed(2)})`);

    // 5. Respawn, aim a few degrees toward the east wall, and hold full
    // throttle: the bike meets the wall near top speed, slides along it, and
    // stops against the north wall.
    await key(base, "r", true);
    await key(base, "r", false);
    m = await step(base, 2);
    check(m.steps <= 1 && Math.abs(m.last.x - start.x) < 1e-3 && Math.abs(m.last.z - start.z) < 1e-3, `R respawns at the start`);
    await key(base, "a", true);
    await step(base, 8);
    await key(base, "a", false);
    m = await step(base, 60);
    const heading = m.ride.yaw;
    await key(base, "w", true);
    let maxX = -Infinity;
    let maxZ = -Infinity;
    let contact = null;
    const slide = [];
    for (let i = 0; i < 400; i++) {
      const before = m;
      m = await step(base, 2);
      worstYaw = Math.max(worstYaw, Math.abs(m.ride.yawRate));
      worstLat = Math.max(worstLat, m.speed * Math.abs(m.ride.yawRate));
      maxX = Math.max(maxX, m.last.x);
      maxZ = Math.max(maxZ, m.last.z);
      contact ??= m.last.x > WALL - RADIUS - 0.05 ? before : null;
      if (contact && m.last.z < NORTH - 20) slide.push(m);
      if (m.last.z > NORTH - 2 && m.speed < 0.5) break;
    }
    await key(base, "w", false);
    check(worstYaw <= MAX_YAW, `yaw rate ≤ ${MAX_YAW} rad/s (worst ${worstYaw.toFixed(4)})`);
    check(worstLat <= MAX_LAT + 1e-6, `speed × yaw rate ≤ ${MAX_LAT} m/s² (worst ${worstLat.toFixed(4)})`);
    check(contact && contact.speed > 0.8 * TOP, `meets the east wall at ${contact?.speed.toFixed(2)} m/s, heading ${((heading * 180) / Math.PI).toFixed(1)}° into it`);
    check(maxX < WALL - RADIUS + 0.05, `never passes the east wall: max x ${maxX.toFixed(4)} (face ${WALL}, radius ${RADIUS})`);
    check(slide.length > 20 && slide.every((s) => s.last.x > WALL - RADIUS - 0.3), `stays against it, sliding for ${slide.length * 2} frames (x ${Math.min(...slide.map((s) => s.last.x)).toFixed(4)}..${Math.max(...slide.map((s) => s.last.x)).toFixed(4)})`);
    const slideSpeed = Math.min(...slide.map((s) => s.speed));
    check(slideSpeed > 0.8 * TOP, `keeps its speed sliding (≥ ${slideSpeed.toFixed(2)} m/s)`);
    check(maxZ < NORTH - RADIUS + 0.05 && m.speed < 0.5, `the north wall stops it: max z ${maxZ.toFixed(4)}, speed ${m.speed.toFixed(3)}`);

    return JSON.stringify((await get(base, "/state")).model);
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

/**
 * Live at 120 Hz with the throttle held, sampling `/scene` back to back (the
 * debug server answers one request per frame). A rig drawn at the stepped
 * pose alternates still and full-step frames; an interpolated one moves
 * evenly every frame.
 */
async function interpolates(port) {
  const base = `http://127.0.0.1:${port}`;
  const proc = spawn(
    BIN,
    ["-d", SAMPLE, "run", "native", "--headless", "--display-hz", "120", "--debug-port", String(port)],
    { cwd: ROOT, stdio: ["ignore", "ignore", "ignore"] },
  );
  try {
    await waitFrame(base, 5);
    await key(base, "w", true);
    await waitFrame(base, (await get(base, "/state")).frame + 120);
    const zs = [];
    for (let i = 0; i < 16; i++) zs.push((await get(base, "/scene")).scene.obj.Group[1].xform[3][2]);
    const deltas = zs.slice(1).map((z, i) => z - zs[i]);
    const even = Math.min(...deltas) > 0 && Math.max(...deltas) < 1.5 * Math.min(...deltas);
    check(even, `120 Hz: the rig moves evenly every frame (Δz ${deltas.map((d) => d.toFixed(3)).join(" ")})`);
  } finally {
    proc.kill("SIGKILL");
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};

const first = await ride(8951, check);
const second = await ride(8952, () => {});
check(first === second, `two runs end on an identical model`);
await interpolates(8953);

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed`);
  process.exit(1);
}
console.log("\nhoverbike e2e passed");
