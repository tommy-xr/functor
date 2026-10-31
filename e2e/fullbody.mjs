// Full-body E2E: injected XR samples drive `examples/fullbody` headlessly, and
// the solved joints (the game's `solved` readout, P) are checked numerically.
//
// Ground truth is the OpenXR grip convention, derived here independently of
// the solver: the grip's -Z runs along the fist, +Y up, +X to the right, and
// the hand holds it rigidly. So in the grip's frame the wrist joint sits 7 cm
// behind (+Z) and 3.5 cm toward the back of the hand (left hand -X, right
// +X), times the avatar's scale; the fingers (left joint +X, right -X) run
// along the grip's -Z, the palm (joint -Y) faces the grip's +X on the left
// and -X on the right, and the thumb (joint +Z) is the grip's +Y. The solver's
// world is tracking space turned to face +Z: (x, y, z) -> (-x, y, -z).
//
// Fixtures: calibration from a floor-locked 1.75 m head, crossed arms, an
// overhead reach, wrist roll, one-hand tracking loss and recovery, a lagging
// head turn and a recentre. The whole script runs twice, in fresh processes,
// and must produce identical models.
//
// Run (needs the CLI and examples/fullbody/Xbot.glb from `npm run fetch:assets`):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/fullbody.mjs
import { spawn } from "node:child_process";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const PORT = 8196;
const BASE = `http://127.0.0.1:${PORT}`;
const EYE = 1.75;
const XBOT_EYE = 1.6606; // Xbot's eye height (skeleton.fun's bind eyes)
const SCALE = EYE / XBOT_EYE;
const SHOULDER_Y = 1.4378 * SCALE; // Xbot's bind shoulders, scaled (the spine only twists)

async function post(path, body) {
  const res = await fetch(`${BASE}${path}`, { method: "POST", body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
}

async function get(path) {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) throw new Error(`GET ${path} → ${res.status}`);
  return res.json();
}

async function waitFrame(after, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const s = await get("/state");
      if (s.frame > after) return s;
    } catch {
      // not listening yet
    }
    if (Date.now() > deadline) throw new Error(`frame never advanced past ${after}`);
    await sleep(5);
  }
}

// One fixed step; an injected sample is level state, so wait for it to land.
async function step(n = 1) {
  let s;
  for (let i = 0; i < n; i++) {
    const before = (await get("/state")).frame;
    await post("/time", { type: "advance", dts: 1 / 60 });
    s = await waitFrame(before, 15000);
  }
  return s;
}

// --- vectors and quaternions ([x, y, z, w], OpenXR order) ---
const add = (a, b) => a.map((v, i) => v + b[i]);
const sub = (a, b) => a.map((v, i) => v - b[i]);
const scale = (k, a) => a.map((v) => v * k);
const dot = (a, b) => a.reduce((s, v, i) => s + v * b[i], 0);
const len = (a) => Math.hypot(...a);
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const axisAngle = (axis, deg) => {
  const h = (deg * Math.PI) / 360;
  return [...axis.map((v) => v * Math.sin(h)), Math.cos(h)];
};
const qmul = ([ax, ay, az, aw], [bx, by, bz, bw]) => [
  aw * bx + ax * bw + ay * bz - az * by,
  aw * by - ax * bz + ay * bw + az * bx,
  aw * bz + ax * by - ay * bx + az * bw,
  aw * bw - ax * bx - ay * by - az * bz,
];
const rotate = ([x, y, z, w], v) => {
  const q = [x, y, z];
  const t = cross(q, v).map((c) => 2 * c);
  return v.map((c, i) => c + w * t[i] + cross(q, t)[i]);
};
const yaw = (deg) => axisAngle([0, 1, 0], deg);
const pitch = (deg) => axisAngle([1, 0, 0], deg);
const roll = (deg) => axisAngle([0, 0, 1], deg);
const fmt = (v) => `[${v.map((x) => x.toFixed(3)).join(", ")}]`;

// Solver world -> tracking space (the half turn is its own inverse).
const tracked = (p) => [-p.x, p.y, -p.z];

// --- samples ---
const pose = (position, orientation = [0, 0, 0, 1]) => ({ position, orientation });
const hand = (grip, extra = {}) => (grip ? { active: true, grip, aim: grip, ...extra } : { active: false });
const sample = (head, left, right) => ({ type: "xr", head, left: hand(left), right: hand(right) });

// Turn a whole sample about the floor origin (what a recentre does to it).
const turnPose = (q, p) => pose(rotate(q, p.position), qmul(q, p.orientation));

// --- checks ---
const SIDE = {
  left: { back: -1, fingers: [1, 0, 0], palmFaces: [1, 0, 0] },
  right: { back: 1, fingers: [-1, 0, 0], palmFaces: [-1, 0, 0] },
};

// The wrist the grip convention predicts, and the solved one, in tracking space.
function expectHeld(label, solved, side, grip) {
  const f = solved[side];
  const at = tracked(f.position);
  const axes = { x: tracked(f.x), y: tracked(f.y), z: tracked(f.z) };
  const q = grip.orientation;
  const want = add(grip.position, rotate(q, [SIDE[side].back * 0.035 * SCALE, 0, 0.07 * SCALE]));
  check(len(sub(at, want)) < 0.002, `${label}: ${side} wrist on the grip ${fmt(want)} (got ${fmt(at)})`);
  // Fingers along the grip's -Z; palm toward its ±X; thumb along its +Y.
  const fingers = SIDE[side].fingers[0] > 0 ? axes.x : scale(-1, axes.x);
  const palm = scale(-1, axes.y);
  check(dot(fingers, rotate(q, [0, 0, -1])) > 0.999, `${label}: ${side} fingers along the grip's forward`);
  check(dot(palm, rotate(q, SIDE[side].palmFaces)) > 0.999, `${label}: ${side} palm faces in`);
  check(dot(axes.z, rotate(q, [0, 1, 0])) > 0.999, `${label}: ${side} thumb along the grip's up`);
}

async function solvedAfter(sampleBody, steps = 1) {
  await post("/input", sampleBody);
  const s = await step(steps);
  const solved = s.model.solved?.args?.[0];
  if (!solved) throw new Error("no `solved` readout (P toggles it)");
  return { solved, body: s.model.body, model: s.model };
}

// --- the script ---
const HEAD = pose([0, EYE, 0]);
const headDown = (deg) => pose([0, EYE - 0.05, -0.05], pitch(-deg));

async function script() {
  const models = [];
  const keep = (r) => (models.push(r.model), r);

  // Floor-locked calibration: the emulated head stands at --xr-eye-height.
  let s = await step();
  check(Math.abs(s.input.xr.head.position[1] - EYE) < 1e-6, `emulated head at ${EYE} m`);
  check(Math.abs(s.model.body.scale - SCALE) < 1e-4, `calibrated scale ${SCALE.toFixed(4)} (got ${s.model.body.scale})`);
  check(Math.abs(s.model.body.root.y) < 1e-6, `feet on the floor (root.y ${s.model.body.root.y})`);

  console.log("\n▸ holding forward");
  const fwdL = pose([-0.2, 1.15, -0.3], pitch(-20));
  const fwdR = pose([0.2, 1.15, -0.3], pitch(-20));
  let r = keep(await solvedAfter(sample(HEAD, fwdL, fwdR)));
  expectHeld("forward", r.solved, "left", fwdL);
  expectHeld("forward", r.solved, "right", fwdR);

  console.log("\n▸ crossed arms");
  const crossL = pose([0.12, 1.28, -0.2], qmul(yaw(-60), pitch(-10)));
  const crossR = pose([-0.12, 1.24, -0.22], qmul(yaw(60), pitch(-10)));
  r = keep(await solvedAfter(sample(headDown(30), crossL, crossR)));
  expectHeld("crossed", r.solved, "left", crossL);
  expectHeld("crossed", r.solved, "right", crossR);
  for (const side of ["left", "right"]) {
    const elbow = tracked(r.solved[`${side}Elbow`].position);
    const wrist = tracked(r.solved[side].position);
    // Bent down, not flipped up: the elbow sags below the shoulder-wrist chord.
    const sag = (SHOULDER_Y + wrist[1]) / 2;
    check(elbow[1] < sag, `crossed: ${side} elbow sags below the chord (${elbow[1].toFixed(3)} < ${sag.toFixed(3)})`);
    check(Math.sign(elbow[0]) === (side === "left" ? -1 : 1), `crossed: ${side} elbow stays on its own side (x ${elbow[0].toFixed(3)})`);
  }

  console.log("\n▸ overhead reach");
  const upL = pose([-0.25, 2.0, -0.12], pitch(90));
  const upR = pose([0.25, 2.0, -0.12], pitch(90));
  r = keep(await solvedAfter(sample(HEAD, upL, upR)));
  expectHeld("overhead", r.solved, "left", upL);
  expectHeld("overhead", r.solved, "right", upR);
  const headY = tracked(r.solved.head.position)[1];
  for (const side of ["left", "right"]) {
    const elbow = tracked(r.solved[`${side}Elbow`].position);
    check(elbow[1] > headY - 0.1, `overhead: ${side} elbow raised (${elbow[1].toFixed(3)})`);
    check(Math.abs(elbow[0]) > 0.15, `overhead: ${side} elbow out to the side (x ${elbow[0].toFixed(3)})`);
  }

  console.log("\n▸ wrist roll");
  for (const deg of [-120, -60, 60, 120, 180]) {
    const rolled = pose(fwdR.position, qmul(fwdR.orientation, roll(deg)));
    r = keep(await solvedAfter(sample(HEAD, fwdL, rolled)));
    expectHeld(`roll ${deg}°`, r.solved, "right", rolled);
  }

  console.log("\n▸ left hand lost, then found");
  r = keep(await solvedAfter(sample(HEAD, fwdL, fwdR)));
  const held = tracked(r.solved.left.position);
  let previous = held;
  let maxStep = 0;
  for (let i = 1; i <= 18; i++) {
    r = keep(await solvedAfter(sample(HEAD, null, fwdR)));
    const at = tracked(r.solved.left.position);
    maxStep = Math.max(maxStep, len(sub(at, previous)));
    previous = at;
    if (i === 9) check(Math.abs(r.body.left.blend - 0.5) < 1e-6, `loss: half blended at 0.15 s (${r.body.left.blend})`);
  }
  check(r.body.left.blend === 0, "loss: neutral after 0.3 s");
  const neutral = previous;
  check(neutral[1] < 1.15 && len(sub(neutral, held)) > 0.1, `loss: the arm settled down at the side ${fmt(neutral)}`);
  check(len([neutral[0], neutral[2]]) < 0.5 && neutral[1] > 0.6, `loss: never snapped toward the origin ${fmt(neutral)}`);
  check(maxStep < 0.05, `loss: moved at most ${(maxStep * 100).toFixed(1)} cm a step`);
  expectHeld("loss", r.solved, "right", fwdR);
  const found = pose([-0.15, 1.3, -0.35], pitch(-30));
  maxStep = 0;
  previous = neutral;
  for (let i = 1; i <= 12; i++) {
    r = keep(await solvedAfter(sample(HEAD, found, fwdR)));
    const at = tracked(r.solved.left.position);
    maxStep = Math.max(maxStep, len(sub(at, previous)));
    previous = at;
  }
  check(r.body.left.blend === 1, "recovery: tracked again after 0.2 s");
  check(maxStep < 0.07, `recovery: moved at most ${(maxStep * 100).toFixed(1)} cm a step`);
  expectHeld("recovery", r.solved, "left", found);

  console.log("\n▸ turning 60° to the right: the hips lag");
  // The player turns on the spot, hands and all.
  const turning = (deg) => sample(pose(HEAD.position, yaw(-deg)), turnPose(yaw(-deg), fwdL), turnPose(yaw(-deg), fwdR));
  let twistMax = 0;
  for (let i = 1; i <= 12; i++) {
    r = keep(await solvedAfter(turning(5 * i)));
    twistMax = Math.max(twistMax, Math.abs(r.body.twist));
  }
  const deg = (rad) => ((rad * 180) / Math.PI).toFixed(1);
  check(twistMax > (40 * Math.PI) / 180, `turn: the hips trailed the head (max twist ${deg(twistMax)}°)`);
  r = keep(await solvedAfter(turning(60), 60));
  check(Math.abs(Math.abs(r.body.twist) - (25 * Math.PI) / 180) < 1e-6, `turn: settled at the 25° deadzone (${deg(r.body.twist)}°)`);
  expectHeld("turned", r.solved, "left", turnPose(yaw(-60), fwdL));
  expectHeld("turned", r.solved, "right", turnPose(yaw(-60), fwdR));

  console.log("\n▸ recentre: the tracking space turns 90° at once");
  const before = r.body;
  const q = yaw(90);
  const recentred = turning(60);
  const recL = turnPose(q, recentred.left.grip);
  const recR = turnPose(q, recentred.right.grip);
  r = keep(await solvedAfter(sample(turnPose(q, recentred.head), recL, recR)));
  const turned = r.body.yaw - before.yaw;
  check(Math.abs(turned - Math.PI / 2) < 1e-6, `recentre: the hips turned with it (${deg(turned)}°)`);
  check(Math.abs(r.body.twist - before.twist) < 1e-6, `recentre: the twist is kept (${deg(r.body.twist)}°)`);
  expectHeld("recentre", r.solved, "left", recL);
  expectHeld("recentre", r.solved, "right", recR);

  return models;
}

async function run() {
  const proc = spawn(
    BIN,
    ["-d", "examples/fullbody", "run", "native", "--headless", "--emulate-xr", "--xr-eye-height", String(EYE), "--debug-port", String(PORT)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  let exited = false;
  const exit = new Promise((r) => proc.once("close", () => r((exited = true))));
  try {
    await waitFrame(0);
    await post("/time", { type: "set", tts: 0 });
    // Steps before the pause ran on the wall clock; start both runs from the
    // first one.
    await post("/rewind", { frame: 1 });
    await post("/input", { type: "key", key: "p", down: true });
    await post("/input", { type: "key", key: "p", down: false });
    return await script();
  } catch (e) {
    failures.push(String(e));
    console.error(e, `\n--- runtime log ---\n${log.slice(-3000)}`);
    return [];
  } finally {
    if (!exited) proc.kill("SIGKILL");
    await exit;
  }
}

console.log("── run 1");
const first = await run();
console.log("\n── run 2 (replay)");
const second = await run();
const differs = first.findIndex((m, i) => JSON.stringify(m) !== JSON.stringify(second[i]));
if (differs >= 0) console.error(`first difference at model ${differs}:\n${JSON.stringify(first[differs])}\n${JSON.stringify(second[differs])}`);
check(first.length > 0 && first.length === second.length && differs < 0, `replay: ${first.length} models identical across runs`);

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nfullbody e2e: all checks passed");
