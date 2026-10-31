// Hoverbike headset E2E: the handlebars and comfort (examples/hoverbike/bars.fun)
// driven by injected XR samples over the debug server, clock paused and
// stepped (`/time advance`), on the flat test stand. No headset.
//
//   1. Calibration: an offset, turned head held still becomes the eye.
//   2. Grab both grips: latched and armed. Bars turned left yaw left, right
//      yaw right; inside the dead zone nothing.
//   3. A full-trigger ride follows the analytic speed curve, straight.
//   4. Releasing one grip at speed cuts the throttle and brakes gently; a
//      re-grab with the trigger held stays unpowered until it is released.
//   5. Losing a grip's tracking does the same; tracking back with the squeeze
//      held doesn't relatch, a fresh grab does.
//   6. Right B respawns. Stepped turning: one 30° snap per push of the bars,
//      at the faded step.
//   7. Two runs end on an identical model.
//
// On foot (walk.fun, mount.fun), on the stand. Every ride above starts on
// foot: the left stick walks up to the bike and a grab at the left grip gets
// on (`board`).
//
//   a. The parked bike's grips sit between a standing player's hip and
//      shoulder. A squeeze out of reach of the grip does nothing; within
//      reach it fades, and on the bike that hand is latched; grabbing the
//      right arms.
//   b. Releasing the grips brakes but never gets off.
//   c. Holding right B at speed is refused, and its release (short or long)
//      doesn't respawn.
//   d. Stopped, holding right B for 0.5 s sets the walker down on the bike's
//      left, on the floor, facing it, hands unlatched.
//   e. The right stick snap turns the walker 30°; a tap of right B respawns.
//   f. Two runs end on an identical model.
//   8. Hidden-window captures: the cockpit at idle and with the vignette match
//      their goldens and repeat byte for byte; against the same ride with the
//      vignette off, the vignette's alpha is ~0 at the centre and equal around
//      a ring (it is centred on the view).
//
// Run (needs the CLI built; the hidden window needs a GL display):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/hoverbike-xr.mjs              # UPDATE_GOLDENS=1 rewrites the two PNGs
import { spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { decodePng } from "../scripts/png.mjs";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;
const SAMPLE = "examples/hoverbike";
const GOLDEN = `${ROOT}${SAMPLE}/golden`;
const DT = 1 / 60;

// Mirrors examples/hoverbike/ride.fun and bars.fun.
const THRUST = 12;
const TOP = 24;
const DRAG_LIN = 0.3;
const DRAG_QUAD = (THRUST - DRAG_LIN * TOP) / (TOP * TOP);
const BRAKE = 14;
const GRIP_LOSS_BRAKE = 0.4;
const SNAP = 0.5236;
const GRIP = { span: 0.31, height: 1.0, forward: 0.62 };
const SEATED_EYE = { x: 0, y: 1.4, z: 0.15 };
const CAPTURE_SIZE = "840x880"; // half a Quest 3 eye buffer: the comfort layer's aspect
// Mirrors walk.fun, mount.fun and game.fun.
const WALKER_EYE = { x: 0, y: 0.75, z: 0 }; // from the walker's capsule centre
const FOOT_Y = 0.92; // the walker's capsule centre standing on the stand's floor
const COLLIDER_LIFT = 0.95; // the bike's capsule centre above its emitter plane
const REACH = 0.3;
const IDLE = 0.3; // the parked hover height (ride.fun)
const HOLD = 0.5;
const STAND_FOOT = { x: 609, z: 0 };

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function get(base, path) {
  const res = await fetch(`${base}${path}`);
  if (!res.ok) throw new Error(`GET ${path} → ${res.status}`);
  return res.json();
}

async function post(base, path, body) {
  const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${await res.text()}`);
  return res;
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
async function step(base, n = 1) {
  const before = (await get(base, "/state")).frame;
  await post(base, "/time", { type: "advance", dts: DT, frames: n });
  const deadline = Date.now() + 30000;
  for (;;) {
    const s = await get(base, "/state");
    if (s.frame >= before + n && s.pending_steps === 0) return s.model;
    if (Date.now() > deadline) throw new Error(`${n} steps from frame ${before} never landed`);
    await sleep(5);
  }
}

const key = async (base, k) => {
  await post(base, "/input", { type: "key", key: k, down: true });
  await post(base, "/input", { type: "key", key: k, down: false });
};

/** bars.fun's `toRig`: a bike-frame point as rig-local, through a calibration. */
function toRig(cal, eye, b) {
  const c = Math.cos(cal.yaw);
  const s = Math.sin(cal.yaw);
  const lx = eye.x - b.x;
  const lz = eye.z - b.z;
  return [cal.x + lx * c + lz * s, cal.y + b.y - eye.y, cal.z - lx * s + lz * c];
}

const yawQuat = (a) => [0, Math.sin(a / 2), 0, Math.cos(a / 2)];

// The head the rides calibrate on: off the reference and turned 0.2 rad
// left. The captures use the reference itself, so the desktop view (the
// authored camera, with no head composed on it) is the eye's.
const HEAD = { x: 0.04, y: -0.06, z: 0.05, yaw: 0.2 };
let head = HEAD;

/**
 * One XR sample: both hands on the bars turned `angle` (left positive), with
 * each hand's squeeze and trigger; `rightGrip: false` drops the right grip's
 * pose (tracking loss). Buttons by name: `{ left: ["secondary"] }`. On foot:
 * `leftAt` places the left grip rig-local, `stick` is the left thumbstick
 * and `turn` the right.
 */
function xr({ angle = 0, ls = 0, rs = 0, lt = 0, rt = 0, rightGrip = true, buttons = {}, leftAt = null, stick = [0, 0], turn = [0, 0] } = {}) {
  const at = (side) => toRig(head, SEATED_EYE, {
    x: side * GRIP.span * Math.cos(angle),
    y: GRIP.height,
    z: GRIP.forward - side * GRIP.span * Math.sin(angle),
  });
  const hand = (side, squeeze, trigger, tracked, pressed = []) => ({
    active: true,
    grip: tracked ? { position: side === 1 && leftAt ? leftAt : at(side), orientation: [0, 0, 0, 1] } : null,
    trigger,
    squeeze,
    thumbstick: side === 1 ? stick : turn,
    primary_pressed: pressed.includes("primary"),
    secondary_pressed: pressed.includes("secondary"),
    menu_pressed: pressed.includes("menu"),
    thumbstick_pressed: pressed.includes("stick"),
  });
  return {
    type: "xr",
    head: { position: [head.x, head.y, head.z], orientation: yawQuat(head.yaw) },
    left: hand(1, ls, lt, true, buttons.left),
    right: hand(-1, rs, rt, rightGrip, buttons.right),
  };
}

/** game.fun's `rigPoint`: a point in a yaw-only frame (+x left, +z forward) in the world. */
const rigPoint = (o, yaw, p) => ({
  x: o.x + p.x * Math.cos(yaw) + p.z * Math.sin(yaw),
  y: o.y + p.y,
  z: o.z - p.x * Math.sin(yaw) + p.z * Math.cos(yaw),
});
/** Its inverse: a world point in the frame. */
function inFrame(o, yaw, w) {
  const dx = w.x - o.x;
  const dz = w.z - o.z;
  return { x: dx * Math.cos(yaw) - dz * Math.sin(yaw), y: w.y - o.y, z: dx * Math.sin(yaw) + dz * Math.cos(yaw) };
}
const toward = (from, to) => Math.atan2(to.x - from.x, to.z - from.z);
const angleDiff = (a, b) => Math.abs(Math.atan2(Math.sin(a - b), Math.cos(a - b)));

/**
 * On foot: the rig-local left-hand position `off` metres from the left grip,
 * toward the walker's eye.
 */
function reachFor(m, off) {
  const bike = { x: m.last.x, y: m.last.y - COLLIDER_LIFT, z: m.last.z };
  const grip = rigPoint(bike, m.ride.yaw, { x: GRIP.span, y: GRIP.height, z: GRIP.forward });
  const eye = rigPoint(m.footPrev, m.walker.yaw, WALKER_EYE);
  const d = Math.hypot(eye.x - grip.x, eye.y - grip.y, eye.z - grip.z);
  const at = { x: grip.x + ((eye.x - grip.x) * off) / d, y: grip.y + ((eye.y - grip.y) * off) / d, z: grip.z + ((eye.z - grip.z) * off) / d };
  return toRig(head, WALKER_EYE, inFrame(m.footPrev, m.walker.yaw, at));
}

/** Hold an XR sample for `n` steps. */
async function hold(base, sample, n = 1) {
  await post(base, "/input", sample);
  return step(base, n);
}

function throttleSpeed(t) {
  const vMinus = -DRAG_LIN / DRAG_QUAD - TOP;
  const r = (TOP / vMinus) * Math.exp(-DRAG_QUAD * (TOP - vMinus) * t);
  return (TOP - r * vMinus) / (1 - r);
}

function launch(port, args) {
  const proc = spawn(BIN, ["-d", SAMPLE, "run", "native", ...args, "--display-hz", "60", "--debug-port", String(port)], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  return { proc, log: () => log };
}

/** Pause, switch to the stand with the hands open on the grips, and calibrate. */
async function onStand(base) {
  await waitFrame(base, 12);
  await step(base, 1);
  await post(base, "/rewind", { frame: 10 });
  await post(base, "/input", xr());
  await key(base, "t");
  return step(base, 60);
}

/** On foot: the left stick walks toward the bike (the spawn faces it) until near the seat. */
async function walkUp(base) {
  let m;
  for (let i = 0; i < 100; i++) {
    m = await hold(base, xr({ stick: [0, 1] }), 2);
    if (m.near) break;
  }
  return hold(base, xr(), 4);
}

/**
 * Walk up and get on: the left hand squeezes 0.15 m from its grip, the fade
 * runs, then the right grabs its grip (both latched and armed). Pads to the
 * same step phase every time (`steps` mod 6: the gauge's refresh).
 */
async function board(base) {
  let m = await walkUp(base);
  const at = reachFor(m, 0.15);
  await hold(base, xr({ leftAt: at }));
  await hold(base, xr({ leftAt: at, ls: 1 }));
  m = await hold(base, xr({ ls: 1 }), 16);
  if (m.mode.$ctor !== "Riding" || !m.vr.left.latched) throw new Error(`never got on (near ${m.near}, mode ${m.mode.$ctor})`);
  m = await hold(base, xr({ ls: 1 }), 1 + (((5 - ((m.steps + 2) % 6)) % 6) + 6) % 6);
  return hold(base, xr({ ls: 1, rs: 1 }));
}

async function respawn(base, sample) {
  await post(base, "/input", sample);
  await key(base, "r");
  await step(base, 2);
  await step(base, 30);
  return board(base);
}

async function ride(port, check) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, ["--headless"]);
  try {
    // 1. Calibration.
    let m = await onStand(base);
    const cal = m.vr.calibration;
    check(
      m.course.$ctor === "OnStand" && m.vr.present && !m.vr.pending
        && Math.abs(cal.x - HEAD.x) < 1e-9 && Math.abs(cal.z - HEAD.z) < 1e-9 && Math.abs(cal.yaw - HEAD.yaw) < 1e-6,
      `calibrated on the still head (yaw ${cal.yaw.toFixed(4)})`,
    );
    check(m.mode.$ctor === "OnFoot" && !m.vr.left.latched && !m.vr.armed, `on foot: open hands, not latched`);

    // 2. Get on, then steer.
    m = await board(base);
    check(m.vr.left.latched && m.vr.right.latched && m.vr.armed && Math.abs(m.vr.neutral) < 1e-6, `both grips latched and armed, neutral ${m.vr.neutral.toExponential(2)}`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: 0.03 }), 30);
    check(m.vr.steer === 0 && m.ride.yawRate === 0, `inside the dead zone: steer ${m.vr.steer}`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: 0.2 }), 30);
    check(m.vr.steer > 0.4 && m.ride.yawRate > 0 && m.ride.yaw > 0, `bars left: steer ${m.vr.steer.toFixed(3)}, yaw rate ${m.ride.yawRate.toFixed(3)} (left)`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: -0.2 }), 60);
    check(m.vr.steer < -0.4 && m.ride.yawRate < 0, `bars right: steer ${m.vr.steer.toFixed(3)}, yaw rate ${m.ride.yawRate.toFixed(3)} (right)`);

    // 3. A full-trigger ride, bars at neutral.
    m = await respawn(base, xr({ ls: 1, rs: 1 }));
    check(m.vr.armed && m.speed === 0, `respawn, walk up and get on again: armed`);
    const start = m.last;
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 180);
    const expect = throttleSpeed(3);
    check(Math.abs(m.speed - expect) < 0.03 * expect, `3 s of trigger: ${m.speed.toFixed(3)} m/s (analytic ${expect.toFixed(3)})`);
    check(m.ride.yaw === 0 && Math.abs(m.last.x - start.x) < 1e-6 && m.last.z - start.z > 20, `straight north: ${(m.last.z - start.z).toFixed(2)} m, yaw ${m.ride.yaw}`);

    // 4. Release the right grip at speed, trigger still held.
    const v0 = m.speed;
    m = await hold(base, xr({ ls: 1, rs: 0, rt: 1 }), 1);
    check(!m.vr.armed && m.controls.throttle === 0 && m.controls.brake === GRIP_LOSS_BRAKE, `one hand off: throttle ${m.controls.throttle}, brake ${m.controls.brake}`);
    const before = m.speed;
    m = await step(base, 30);
    const decel = (before - m.speed) / (30 * DT);
    check(decel > GRIP_LOSS_BRAKE * BRAKE && decel < 0.5 * BRAKE + DRAG_LIN * v0 + DRAG_QUAD * v0 * v0, `brakes gently: ${decel.toFixed(2)} m/s² with drag (grip-loss brake ${(GRIP_LOSS_BRAKE * BRAKE).toFixed(1)}, full ${BRAKE})`);
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 2);
    check(m.vr.right.latched && !m.vr.armed && m.controls.throttle === 0, `re-grab with the trigger held: latched, unpowered`);
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 0 }), 1);
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 1);
    check(m.vr.armed && m.controls.throttle === 1, `trigger released, then pressed: powered again`);

    // 5. Tracking loss and reacquire.
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1, rightGrip: false }), 2);
    check(!m.vr.right.tracked && !m.vr.right.latched && !m.vr.armed && m.controls.brake === GRIP_LOSS_BRAKE, `right tracking lost: unlatched, braking`);
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 0 }), 20);
    check(m.vr.right.tracked && !m.vr.right.latched && !m.vr.armed, `tracking back, squeeze still held: still loose`);
    m = await hold(base, xr({ ls: 1, rs: 0 }), 1);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 2);
    check(m.vr.right.latched && m.vr.armed, `a fresh grab re-arms`);

    // 6. Stepped turning, from rest. Stopped, a tap of right B respawns (on
    // foot); at speed it wouldn't (c below).
    await hold(base, xr({ ls: 1, rs: 1, lt: 1 }), 90);
    await hold(base, xr({ ls: 1, rs: 1, buttons: { right: ["secondary"] } }), 1);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 30);
    check(m.speed === 0 && m.steps === 29 && m.mode.$ctor === "OnFoot" && Math.abs(m.last.z - start.z) < 1e-6, `a tap of right B respawns, on foot`);
    m = await board(base);
    m = await hold(base, xr({ ls: 1, rs: 1, buttons: { left: ["secondary"] } }), 1);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 1);
    check(m.vr.stepped && m.ride.yaw === 0, `left Y: stepped turning`);
    await post(base, "/input", xr({ ls: 1, rs: 1, angle: 0.4 }));
    const yaws = [];
    for (let i = 0; i < 60; i++) {
      m = await step(base, 1);
      yaws.push(m.ride.yaw);
      check.quiet(Math.abs(m.prev.yaw - m.ride.yaw) < 1e-9, `step ${i}: the snap moves the last pose too`);
    }
    const jumps = yaws.map((y, i) => y - (i ? yaws[i - 1] : 0)).filter((d) => d !== 0);
    check(jumps.length === 1 && Math.abs(jumps[0] - SNAP) < 1e-9, `held left: one snap of ${jumps.map((d) => d.toFixed(4)).join(", ")} rad`);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 15);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: 0.4 }), 40);
    check(Math.abs(m.ride.yaw - 2 * SNAP) < 1e-9 && m.vr.snap.t < 0 && m.speed < 0.01, `back to centre and left again: a second snap (yaw ${m.ride.yaw.toFixed(4)})`);

    return JSON.stringify((await get(base, "/state")).model);
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

/**
 * A hidden-window ride on the stand: grab, settle, capture the cockpit at
 * idle, then 20 steps of full trigger and capture again. `vignetteOff`
 * presses left X twice first, turning the default Medium off; either way
 * the same steps run, so the two rides match.
 */
async function captures(port, vignetteOff = false) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, ["--hidden", "--capture-size", CAPTURE_SIZE]);
  const capture = async () => Buffer.from(await (await post(base, "/capture", {})).arrayBuffer());
  try {
    await onStand(base);
    for (let i = 0; i < 2; i++) {
      await hold(base, xr({ buttons: { left: vignetteOff ? ["primary"] : [] } }));
      await hold(base, xr());
    }
    await board(base);
    let m = await step(base, 30);
    const idle = await capture();
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 20);
    const moving = await capture();
    return { idle, moving, model: m };
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

async function onFoot(port, check) {
  const base = `http://127.0.0.1:${port}`;
  const { proc, log } = launch(port, ["--headless"]);
  try {
    let m = await onStand(base);
    check(m.mode.$ctor === "OnFoot" && !m.vr.mounted && !m.vr.pending, `on foot on the stand, calibrated`);
    m = await walkUp(base);
    check(m.near && m.mode.$ctor === "OnFoot", `the left stick walks up to the seat`);
    // Calibration puts the head at the walker's eye, so the grip's drop below
    // it is what a real player reaches. By stature, eye ≈ 0.936, shoulder
    // ≈ 0.818 and hip ≈ 0.53: the shoulder is ≈ 0.126 × eye height under the
    // eye, the hip ≈ 0.43 ×. For eye heights 1.6–1.75 m the grip must sit
    // below the tallest's shoulder and above the shortest's hip.
    const gripY = m.last.y - COLLIDER_LIFT + GRIP.height;
    const drop = m.footPrev.y + WALKER_EYE.y - gripY;
    check(Math.abs(m.clearance - IDLE) < 0.01 && drop > 0.126 * 1.75 && drop < 0.43 * 1.6,
      `parked grips ${gripY.toFixed(3)} m up, ${drop.toFixed(3)} m below the eye: under the shoulder (≥ ${(0.126 * 1.75).toFixed(3)}), over the hip (≤ ${(0.43 * 1.6).toFixed(3)})`);

    // a. Out of reach, then within.
    const far = reachFor(m, REACH + 0.3);
    await hold(base, xr({ leftAt: far }));
    m = await hold(base, xr({ leftAt: far, ls: 1 }), 10);
    check(m.mode.$ctor === "OnFoot" && m.fade.t < 0, `a squeeze ${(REACH + 0.3).toFixed(1)} m from the grip does nothing`);
    const at = reachFor(m, 0.15);
    await hold(base, xr({ leftAt: at }));
    m = await hold(base, xr({ leftAt: at, ls: 1 }), 1);
    check(m.fade.t >= 0 && m.fade.action.$ctor === "Mount.GetOn" && m.fade.action.args[0] === 1, `a squeeze 0.15 m from it: the fade starts`);
    m = await hold(base, xr({ ls: 1 }), 6);
    check(m.mode.$ctor === "Riding" && m.vr.mounted && m.vr.left.latched && !m.vr.right.latched && !m.vr.armed, `at the black: on the bike, the left hand latched`);
    m = await hold(base, xr({ ls: 1 }), 1);
    check(Math.abs(m.clearance - 1.2) < 0.015 && Math.abs(m.ride.vy) < 0.01 && m.vr.vignette === 0,
      `snapped to ride height while black (clearance ${m.clearance.toFixed(4)} m, vy ${m.ride.vy.toExponential(2)}), no vignette`);
    m = await hold(base, xr({ ls: 1 }), 11);
    m = await hold(base, xr({ ls: 1, rs: 1 }));
    check(m.vr.armed && m.fade.t < 0, `grabbing the right arms`);

    // b. Grips released: braking, still riding.
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 60);
    const fast = m.speed;
    m = await hold(base, xr(), 30);
    check(m.mode.$ctor === "Riding" && m.fade.t < 0 && m.controls.brake === GRIP_LOSS_BRAKE && m.speed < fast, `grips released: braking (${fast.toFixed(2)} → ${m.speed.toFixed(2)} m/s), still riding`);

    // c. Right B held at speed.
    await hold(base, xr({ ls: 1, rs: 0 }));
    await hold(base, xr({ ls: 1, rs: 1 }));
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 0 }));
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 30);
    let steps = m.steps;
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1, buttons: { right: ["secondary"] } }), 10);
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1 }), 2);
    check(m.mode.$ctor === "Riding" && m.steps === steps + 12, `a short press of right B at speed, let go: no respawn`);
    steps = m.steps;
    m = await hold(base, xr({ ls: 1, rs: 1, rt: 1, buttons: { right: ["secondary"] } }), 40);
    check(m.mode.$ctor === "Riding" && m.fade.t < 0 && m.off >= HOLD && m.speed > 2, `right B held at ${m.speed.toFixed(2)} m/s: refused`);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 2);
    check(m.mode.$ctor === "Riding" && m.steps === steps + 42, `its release doesn't respawn`);

    // d. Stop, hold right B.
    m = await hold(base, xr({ ls: 1, rs: 1, lt: 1 }), 60);
    check(m.speed === 0, `the left trigger stops it`);
    m = await hold(base, xr({ ls: 1, rs: 1, buttons: { right: ["secondary"] } }), 30);
    for (let i = 0; i < 3 && m.fade.t < 0; i++) m = await step(base, 1);
    check(m.fade.t >= 0 && m.fade.action.$ctor === "Mount.GetOff" && Math.abs(m.off - HOLD) < 1e-6, `right B held ${m.off.toFixed(3)} s, stopped: the fade starts`);
    m = await hold(base, xr(), 30);
    const w = m.footPrev;
    const bike = m.last;
    const left = inFrame(bike, m.ride.yaw, w).x;
    check(m.mode.$ctor === "OnFoot" && !m.vr.mounted && !m.vr.left.latched && !m.vr.right.latched, `on foot, hands let go`);
    check(Math.abs(w.y - FOOT_Y) < 0.01 && left > 1.5, `set down on the bike's left (${left.toFixed(3)} m), on the floor (centre y ${w.y.toFixed(4)})`);
    check(angleDiff(m.walker.yaw, toward(w, bike)) < 0.01, `facing the bike`);

    // e. Snap turn, then a tap of right B.
    const facing = m.walker.yaw;
    m = await hold(base, xr({ turn: [1, 0] }), 20);
    m = await hold(base, xr(), 2);
    check(Math.abs(m.walker.yaw - (facing - SNAP)) < 1e-9, `the right stick snap turns 30° right`);
    await hold(base, xr({ buttons: { right: ["secondary"] } }));
    m = await hold(base, xr(), 2);
    check(m.mode.$ctor === "OnFoot" && Math.abs(m.footPrev.x - STAND_FOOT.x) < 0.05 && Math.abs(m.footPrev.z - STAND_FOOT.z) < 0.05,
      `a tap of right B respawns at the stand's start`);

    return JSON.stringify((await get(base, "/state")).model);
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log ---\n${log()}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

/** golden.rs's rule: under 1% of pixels may differ by more than 16 in a channel. */
function matchesGolden(name, png) {
  const path = `${GOLDEN}/${name}.png`;
  if (process.env.UPDATE_GOLDENS || !existsSync(path)) {
    writeFileSync(path, png);
    return { ok: true, note: "written" };
  }
  const a = decodePng(readFileSync(path));
  const b = decodePng(png);
  if (a.width !== b.width || a.height !== b.height) return { ok: false, note: "size differs" };
  let differing = 0;
  for (let p = 0; p < a.width * a.height; p++) {
    for (let c = 0; c < 3; c++) {
      if (Math.abs(a.pixels[p * a.channels + c] - b.pixels[p * b.channels + c]) > 16) {
        differing++;
        break;
      }
    }
  }
  const fraction = differing / (a.width * a.height);
  return { ok: fraction <= 0.01, note: `${(fraction * 100).toFixed(3)}% of pixels differ` };
}

/**
 * The vignette's alpha at a point, from the same frame with it on and off:
 * on = off × (1 − alpha), so alpha = 1 − on / off, read from the brightest
 * channel; `null` where the frame is too dark to tell.
 */
function alphaAt(on, off, x, y) {
  const i = (Math.round(y) * off.width + Math.round(x)) * off.channels;
  const c = [0, 1, 2].reduce((best, k) => (off.pixels[i + k] > off.pixels[i + best] ? k : best), 0);
  return off.pixels[i + c] < 30 ? null : 1 - on.pixels[i + c] / off.pixels[i + c];
}

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};
check.quiet = (ok, what) => ok || check(ok, what);
const silent = () => {};
silent.quiet = () => {};

const first = await ride(8961, check);
const second = await ride(8962, silent);
check(first === second, `two runs end on an identical model`);

const footFirst = await onFoot(8966, check);
const footSecond = await onFoot(8967, silent);
check(footFirst === footSecond, `on foot: two runs end on an identical model`);

head = { x: 0, y: 0, z: 0, yaw: 0 };
const on = await captures(8963);
const again = await captures(8964);
const off = await captures(8965, true);
check(on.model.vr.vignette > 0.9 && on.model.vr.strength === 2 && off.model.vr.strength === 0, `vignette level ${on.model.vr.vignette.toFixed(3)} after 20 steps of trigger`);
check(on.idle.equals(again.idle) && on.moving.equals(again.moving), `captures repeat byte for byte`);
for (const [name, png] of [["hoverbike-xr-idle", on.idle], ["hoverbike-xr-vignette", on.moving]]) {
  const r = matchesGolden(name, png);
  check(r.ok, `${name} golden: ${r.note}`);
}
const vig = decodePng(on.moving);
const plain = decodePng(off.moving);
const cx = vig.width / 2;
const cy = vig.height / 2;
const centre = alphaAt(vig, plain, cx, cy);
check(centre !== null && Math.abs(centre) < 0.02, `vignette clear at the centre (alpha ${centre?.toFixed(3)})`);
// Around a ring at 0.9 of the half-height, where the frame is bright enough
// to read: alpha the same in every direction, left and right included.
const ring = [];
for (let k = 0; k < 32; k++) {
  const a = (k / 32) * 2 * Math.PI;
  const alpha = alphaAt(vig, plain, cx + Math.cos(a) * 0.9 * cy, cy + Math.sin(a) * 0.9 * cy);
  if (alpha !== null) ring.push({ side: Math.cos(a), alpha });
}
const alphas = ring.map((p) => p.alpha);
const spread = Math.max(...alphas) - Math.min(...alphas);
check(
  ring.some((p) => p.side < -0.5) && ring.some((p) => p.side > 0.5) && Math.min(...alphas) > 0.2 && spread < 0.08,
  `vignette centred: alpha ${Math.min(...alphas).toFixed(3)}..${Math.max(...alphas).toFixed(3)} around the ring (${ring.length} readable points)`,
);
check(on.idle.equals(off.idle), `at idle the vignette draws nothing`);

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed`);
  process.exit(1);
}
console.log("\nhoverbike xr e2e passed");
