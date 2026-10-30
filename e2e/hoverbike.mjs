// Hoverbike E2E: scripted rides on the dune course and the flat test stand,
// over the debug server with the clock paused and stepped (`/time advance`).
//
// The dunes (the default course):
//
//   a. A cold load never drops the bike: until every probe sees the
//      heightfield it holds at the spawn, then it settles at hover height.
//   b. A route: straight, over the crest, past the block, a turn, a stop.
//      Clearance over the crest stays inside [0.5, 2.0] m (no strike, no
//      launch), vertical acceleration stays bounded, the block is passed
//      without touching it, and braking stops within the analytic distance.
//   c. Straight into the block at full throttle: it never passes its face.
//   d. Full throttle out past the soft boundary: the bike stops under 25 m
//      past it, and comes back in once the throttle is released.
//   e. Two runs (one windowed but hidden, one headless) give identical
//      models at the end of b, c and d. The windowed run prints
//      `/state.terrain` LOD stats along the route.
//
// The stand (T switches to it):
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
// On foot (game.fun, walk.fun, mount.fun). Every ride above starts by walking
// up to the bike and pressing E (`board`); R puts the rider back on foot.
//
//   i.   Spawn on foot on the pad, away from the bike; walk up; E gets on.
//   ii.  Ride a short way and stop. Holding E for 0.5 s fades and sets the
//        walker down on the bike's left, on the ground, clear of the bike and
//        the props, facing it.
//   iii. Walk away and back; E gets on again.
//   iv.  Holding E at speed is refused (slow down); braking while still
//        holding gets off once below 2 m/s.
//   v.   Two runs end on an identical model.
//
// Run (needs the CLI built; the wasm bundle is not required; the hidden
// window needs a GL display):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/hoverbike.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;
const SAMPLE = "examples/hoverbike";
const DT = 1 / 60;

// Mirrors examples/hoverbike/ride.fun, game.fun, dunes.fun and dunesLayout.fun.
const HOVER = 1.2;
const THRUST = 12;
const TOP = 24;
const DRAG_LIN = 0.3;
const DRAG_QUAD = (THRUST - DRAG_LIN * TOP) / (TOP * TOP);
const BRAKE = 14;
const MAX_YAW = 1;
const MAX_LAT = 5;
const GRIP = 4;
const STAND_X = 600; // stand.fun's originX
const WALL = STAND_X + 15; // the east wall's inner face
const NORTH = 210; // the north wall's inner face
const RADIUS = 0.8; // the bike's capsule
const WALKER_RADIUS = 0.3; // walk.fun
const FOOT_Y = 0.92; // the walker's capsule centre standing on ground at 0
const HOLD = 0.5; // mount.fun's holdTime
const MAX_OFF_SPEED = 2; // mount.fun's maxSpeed
const SPAWN = { x: 100, y: 2.15, z: -380 }; // on the dunes' start pad
const PROBE_RANGE = 3;
const CREST_Z = -180;
const BLOCK = { x: 100, z: 40, w: 8, d: 3 };
const SOFT_LIMIT = 440;
// Clearance bounds over the crest: the lowest-hanging fairing parts sit
// ~0.2 m under the emitter plane, and probes reach 3 m.
const CLEAR_MIN = 0.5;
const CLEAR_MAX = 2.0;

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
const tap = async (base, k) => {
  await key(base, k, true);
  await key(base, k, false);
};

/**
 * Walk up to the bike and get on: W until near the seat (the spawn faces
 * it), then E and the fade. Returns the model, riding.
 */
async function board(base) {
  await key(base, "w", true);
  let m;
  for (let i = 0; i < 200; i++) {
    m = await step(base, 2);
    if (m.near) break;
  }
  await key(base, "w", false);
  await tap(base, "e");
  m = await step(base, 16);
  if (m.mode.$ctor !== "Riding") throw new Error(`never got on (near ${m.near}, at ${JSON.stringify(m.footPrev)})`);
  return m;
}

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

/** Launch the sample at 60 Hz under the debug server; `log()` is its output so far. */
function launch(port, mode) {
  const proc = spawn(
    BIN,
    ["-d", SAMPLE, "run", "native", mode, "--display-hz", "60", "--debug-port", String(port)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  return { proc, log: () => log };
}

async function ride(port, check) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, "--headless");
  try {
    // Pause, then rewind to a fixed early frame so both runs start alike.
    await waitFrame(base, 12);
    await step(base, 1);
    await post(base, "/rewind", { frame: 10 });

    // 1. Switch to the stand and settle.
    await key(base, "t", true);
    await key(base, "t", false);
    let m = await step(base, 2);
    check(m.course.$ctor === "OnStand" && Math.abs(m.last.x - (STAND_X + 5)) < 1e-3, `T switches to the stand`);
    m = await step(base, 120);
    check(Math.abs(m.clearance - HOVER) < 0.05, `settled clearance ${m.clearance.toFixed(4)} m`);
    const start = m.last;
    m = await board(base);

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
    check(m.steps <= 1 && m.course.$ctor === "OnStand" && m.mode.$ctor === "OnFoot" && Math.abs(m.last.x - start.x) < 1e-3 && Math.abs(m.last.z - start.z) < 1e-3, `R respawns at the stand's start, on foot`);
    m = await board(base);
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
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

/** Hold a gamepad state (throttle, stick x: right positive) for the next steps. */
const pad = (base, throttle, stick, brake = 0) =>
  post(base, "/input", { type: "gamepad", right_trigger: throttle, left_trigger: brake, left_stick: [stick, 0] });
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
/** Stick input that turns the bike toward `yaw` (left is positive yaw; the stick's right turns right). */
const steerTo = (m, yaw) => clamp((m.ride.yaw - yaw) * 4, -1, 1);

/** Respawn (R) and settle (the bike waits for ground, then rests at hover height), then get on. */
async function respawn(base) {
  await key(base, "r", true);
  await key(base, "r", false);
  await step(base, 2);
  await step(base, 30);
  return board(base);
}

async function dunes(port, check, mode) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, mode);
  const lod = [];
  // The model at the end of each scenario, compared across runs.
  const snapshots = [];
  const sampleLod = async (where) => {
    if (mode !== "--hidden") return;
    const s = await get(base, "/state");
    lod.push({ where, ...s.terrain });
  };
  try {
    // a. Cold load: watch every frame we can from the first, live.
    const seen = [];
    const deadline = Date.now() + 30000;
    for (;;) {
      if (Date.now() > deadline) throw new Error("the bike never sensed the dunes");
      try {
        const s = await get(base, "/state");
        if (seen.at(-1)?.frame !== s.frame) seen.push({ frame: s.frame, ...s.model });
        if (s.model.ready && s.frame > 30) break;
      } catch {
        // not listening yet
      }
      await sleep(5);
    }
    const waiting = seen.filter((m) => !m.ready);
    const firstReady = seen.find((m) => m.ready);
    check(
      seen.every((m) => Math.abs(m.last.y - SPAWN.y) < 0.01 && Math.abs(m.last.x - SPAWN.x) < 1e-3 && Math.abs(m.last.z - SPAWN.z) < 1e-3),
      `cold load: held at the spawn through ${seen.length} observed frames (${waiting.length} waiting for ground, ready from frame ${firstReady?.frame})`,
    );
    check(
      waiting.every((m) => m.ride.vy === 0 && m.speed === 0),
      waiting.length
        ? `cold load: no motion before the probes see ground`
        : `cold load: the heightmap beat the first poll; the gate is covered by game.fun's expects`,
    );
    check(Math.abs(seen.at(-1).clearance - HOVER) < 0.01, `cold load: rests at hover height (${seen.at(-1).clearance.toFixed(4)} m)`);

    // Pause, and respawn so both runs start from the same state whenever the
    // heightmap happened to land.
    await step(base, 1);
    let m = await respawn(base);
    check(m.ready && m.course.$ctor === "OnDunes" && Math.abs(m.clearance - HOVER) < 0.01, `respawn: ready on the pad, clearance ${m.clearance.toFixed(4)} m`);
    await sampleLod("pad");

    // b. The route. Hold the lane (x = 100) at full throttle over the crest,
    // swing 9 m left to pass the block, then turn right and brake to a stop.
    let crest = [Infinity, -Infinity];
    let farthest = 0;
    let worstAy = 0;
    let passing = null;
    let stage = "lane";
    let brakeFrom = null;
    let v0 = 0;
    const marks = new Set();
    for (let i = 0; i < 1500; i++) {
      const p = m.last;
      let throttle = 1;
      let stick = 0;
      let brake = 0;
      if (stage === "lane") {
        const lane = p.z < -40 ? 100 : 109;
        stick = steerTo(m, Math.atan2(lane - p.x, 30));
        if (p.z > 70) stage = "turn";
      } else if (stage === "turn") {
        throttle = 0.5;
        stick = 1;
        if (m.ride.yaw < -Math.PI / 2 + 0.1) stage = "brake";
      }
      if (stage === "brake") {
        throttle = 0;
        stick = steerTo(m, -Math.PI / 2);
        brake = 1;
      }
      await pad(base, throttle, stick, brake);
      const before = m;
      m = await step(base, 2);
      if (stage === "brake" && !brakeFrom) {
        // The first braked step reads where the braking starts.
        brakeFrom = before.last;
        v0 = before.speed;
      }
      farthest = Math.max(farthest, ...m.probes);
      if (Math.abs(m.last.z - CREST_Z) < 80 && stage === "lane") {
        crest = [Math.min(crest[0], ...m.probes), Math.max(crest[1], ...m.probes)];
        worstAy = Math.max(worstAy, Math.abs(m.ride.vy - before.ride.vy) / (2 * DT));
      }
      if (Math.abs(m.last.z - BLOCK.z) < BLOCK.d / 2 + RADIUS) passing ??= m;
      for (const [name, z] of [["crest -80 m", CREST_Z - 80], ["crest", CREST_Z], ["crest +80 m", CREST_Z + 80], ["block", BLOCK.z]]) {
        if (!marks.has(name) && m.last.z >= z) {
          marks.add(name);
          await sampleLod(name);
        }
      }
      if (stage === "brake" && m.speed < 0.01) break;
    }
    await post(base, "/input", { type: "gamepad_clear" });
    await sampleLod("stop");
    check(farthest < PROBE_RANGE, `every probe keeps the ground in range along the route (highest ${farthest.toFixed(3)} m)`);
    check(crest[0] > CLEAR_MIN && crest[1] < CLEAR_MAX, `over the crest at ${TOP} m/s, every probe's clearance ${crest.map((c) => c.toFixed(3)).join("..")} m (bounds ${CLEAR_MIN}..${CLEAR_MAX})`);
    check(worstAy < 3, `vertical acceleration over the crest ≤ 3 m/s² (worst ${worstAy.toFixed(3)})`);
    check(passing && passing.last.x > BLOCK.x + BLOCK.w / 2 + RADIUS && passing.speed > 0.95 * TOP, `passes the block clear at ${passing?.speed.toFixed(2)} m/s (x ${passing?.last.x.toFixed(2)})`);
    check(m.speed < 0.01 && Math.abs(m.ride.yaw + Math.PI / 2) < 0.15, `turns right and stops (yaw ${m.ride.yaw.toFixed(3)})`);
    const stopped = Math.hypot(m.last.x - brakeFrom.x, m.last.z - brakeFrom.z);
    const analytic = stoppingDistance(v0);
    check(Math.abs(stopped - analytic) < 0.1 * analytic, `stopping distance ${stopped.toFixed(2)} m (analytic ${analytic.toFixed(2)} from ${v0.toFixed(2)} m/s)`);
    snapshots.push(m);

    // c. Straight into the block at full throttle.
    m = await respawn(base);
    await key(base, "w", true);
    let maxZ = -Infinity;
    let contact = null;
    for (let i = 0; i < 700; i++) {
      const before = m;
      m = await step(base, 2);
      maxZ = Math.max(maxZ, m.last.z);
      contact ??= m.speed < 0.5 * before.speed ? before : null;
      if (contact && m.speed < 0.5) break;
    }
    await key(base, "w", false);
    const face = BLOCK.z - BLOCK.d / 2;
    check(contact && contact.speed > 0.9 * TOP, `meets the block at ${contact?.speed.toFixed(2)} m/s`);
    check(maxZ < face - RADIUS + 0.05 && m.speed < 0.5, `the block stops it: max z ${maxZ.toFixed(4)} (face ${face}, radius ${RADIUS})`);
    snapshots.push(m);

    // d. Turn on the pad to face +x, then full throttle out past the limit.
    m = await respawn(base);
    for (let i = 0; i < 60; i++) {
      await pad(base, 0, steerTo(m, Math.PI / 2));
      m = await step(base, 2);
    }
    check(Math.abs(m.ride.yaw - Math.PI / 2) < 0.06 && m.speed < 0.1, `turns on the spot to face +x (yaw ${m.ride.yaw.toFixed(4)})`);
    let maxX = -Infinity;
    let low = Infinity;
    let high = -Infinity;
    let entry = 0;
    for (let i = 0; i < 1200; i++) {
      await pad(base, 1, steerTo(m, Math.PI / 2));
      const before = m;
      m = await step(base, 2);
      maxX = Math.max(maxX, m.last.x);
      low = Math.min(low, ...m.probes);
      high = Math.max(high, ...m.probes);
      if (m.last.x > SOFT_LIMIT && !entry) entry = before.speed;
      if (m.last.x > SOFT_LIMIT && m.speed < 0.2) break;
    }
    // Throttle released: back inside.
    for (let i = 0; i < 300 && m.last.x > SOFT_LIMIT; i++) {
      await pad(base, 0, steerTo(m, Math.PI / 2));
      m = await step(base, 2);
    }
    await post(base, "/input", { type: "gamepad_clear" });
    check(entry > 0.9 * TOP && maxX < SOFT_LIMIT + 25, `boundary: crossed at ${entry.toFixed(2)} m/s, stopped ${(maxX - SOFT_LIMIT).toFixed(2)} m past it`);
    check(m.last.x <= SOFT_LIMIT, `boundary: back inside once released (x ${m.last.x.toFixed(3)})`);
    check(low > CLEAR_MIN && high < CLEAR_MAX, `boundary run over open dunes: clearance ${low.toFixed(3)}..${high.toFixed(3)} m`);

    if (lod.length) {
      console.log("\n/state.terrain along the route (hidden window, desktop):");
      for (const s of lod) console.log(`  ${s.where.padEnd(12)} patches ${String(s.patches).padStart(4)}  vertices ${s.vertices}`);
      console.log();
    }
    snapshots.push((await get(base, "/state")).model);
    return JSON.stringify(snapshots);
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
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
    await step(base, 1);
    await board(base);
    await post(base, "/time", { type: "resume" });
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

/** The walker's yaw toward a point, as mount.fun's `toward`. */
const toward = (from, to) => Math.atan2(to.x - from.x, to.z - from.z);
const angleDiff = (a, b) => Math.abs(Math.atan2(Math.sin(a - b), Math.cos(a - b)));

async function onFoot(port, check) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, "--headless");
  try {
    await waitFrame(base, 12);
    await step(base, 1);
    await post(base, "/rewind", { frame: 10 });
    let m = await step(base, 60);

    // i. On foot, then on.
    const spawn = m.footPrev;
    check(m.mode.$ctor === "OnFoot" && !m.near && m.walker.ready && Math.abs(spawn.y - FOOT_Y) < 0.01 && Math.hypot(spawn.x - SPAWN.x, spawn.z - SPAWN.z) > 3,
      `spawns on foot on the pad, ${Math.hypot(spawn.x - SPAWN.x, spawn.z - SPAWN.z).toFixed(2)} m from the bike (centre y ${spawn.y.toFixed(4)})`);
    await tap(base, "e");
    m = await step(base, 10);
    check(m.mode.$ctor === "OnFoot" && m.fade.t < 0, `E away from the bike does nothing`);
    m = await board(base);
    check(m.mode.$ctor === "Riding" && m.speed === 0, `walks up and gets on with E`);

    // ii. Ride a short way, stop, hold E.
    await key(base, "w", true);
    m = await step(base, 60);
    await key(base, "w", false);
    await key(base, "s", true);
    for (let i = 0; i < 60 && m.speed > 0; i++) m = await step(base, 2);
    await key(base, "s", false);
    check(m.speed === 0 && m.last.z - SPAWN.z > 3, `rides ${(m.last.z - SPAWN.z).toFixed(2)} m and stops`);
    await key(base, "e", true);
    m = await step(base, 30);
    check(m.mode.$ctor === "Riding" && m.fade.t < 0, `held under ${HOLD} s: still riding`);
    for (let i = 0; i < 3 && m.fade.t < 0; i++) m = await step(base, 1);
    check(m.fade.t >= 0 && m.fade.action.$ctor === "GetOff" && Math.abs(m.off - HOLD) < 1e-6, `held ${m.off.toFixed(3)} s: the fade starts`);
    await key(base, "e", false);
    m = await step(base, 30);
    const w = m.footPrev;
    const bike = m.last;
    const apart = Math.hypot(w.x - bike.x, w.z - bike.z);
    const left = (w.x - bike.x) * Math.cos(m.ride.yaw) - (w.z - bike.z) * Math.sin(m.ride.yaw);
    check(m.mode.$ctor === "OnFoot" && Math.abs(w.y - FOOT_Y) < 0.01, `set down on the ground (centre y ${w.y.toFixed(4)})`);
    check(apart > RADIUS + WALKER_RADIUS + 0.4 && left > 1.5, `on the bike's left, ${apart.toFixed(3)} m from it, clear of its capsule`);
    const props = [BLOCK, ...[-360, -320].flatMap((z) => [{ x: 112, z }, { x: 88, z }])];
    const nearest = Math.min(...props.map((p) => Math.hypot(w.x - p.x, w.z - p.z)));
    check(nearest > 3, `clear of the props (nearest ${nearest.toFixed(2)} m)`);
    check(angleDiff(m.walker.yaw, toward(w, bike)) < 0.01, `facing the bike (yaw ${m.walker.yaw.toFixed(4)})`);
    check(m.speed === 0 && Math.abs(m.last.z - bike.z) < 1e-9, `the bike stays where it stopped`);

    // iii. Walk away, then back on.
    await key(base, "s", true);
    m = await step(base, 90);
    await key(base, "s", false);
    m = await step(base, 2);
    check(!m.near && Math.hypot(m.footPrev.x - bike.x, m.footPrev.z - bike.z) > 3, `walks away (${Math.hypot(m.footPrev.x - bike.x, m.footPrev.z - bike.z).toFixed(2)} m)`);
    m = await board(base);
    check(m.mode.$ctor === "Riding", `walks back and gets on again`);

    // iv. Refused at speed; braking while holding gets off.
    await key(base, "w", true);
    m = await step(base, 60);
    await key(base, "e", true);
    m = await step(base, 40);
    check(m.mode.$ctor === "Riding" && m.fade.t < 0 && m.off >= HOLD && m.speed > MAX_OFF_SPEED, `held at ${m.speed.toFixed(2)} m/s: refused (asks to slow down)`);
    await key(base, "w", false);
    await key(base, "s", true);
    let before = m;
    for (let i = 0; i < 120 && m.fade.t < 0; i++) {
      before = m;
      m = await step(base, 1);
    }
    await key(base, "s", false);
    await key(base, "e", false);
    check(m.fade.t >= 0 && m.speed < MAX_OFF_SPEED && before.speed >= MAX_OFF_SPEED,
      `still holding, gets off on the first step below ${MAX_OFF_SPEED} m/s (${before.speed.toFixed(3)} → ${m.speed.toFixed(3)})`);
    m = await step(base, 20);
    check(m.mode.$ctor === "OnFoot", `and lands on foot`);

    return JSON.stringify((await get(base, "/state")).model);
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};

const footFirst = await onFoot(8956, check);
const footSecond = await onFoot(8957, () => {});
check(footFirst === footSecond, `on foot: two runs end on an identical model`);
const dunesFirst = await dunes(8954, check, "--hidden");
const dunesSecond = await dunes(8955, () => {}, "--headless");
check(dunesFirst === dunesSecond, `dunes: two runs give identical models after the route, the block and the boundary`);
const first = await ride(8951, check);
const second = await ride(8952, () => {});
check(first === second, `stand: two runs end on an identical model`);
await interpolates(8953);

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed`);
  process.exit(1);
}
console.log("\nhoverbike e2e passed");
