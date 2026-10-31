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
 * pose (tracking loss). Buttons by name: `{ left: ["secondary"] }`.
 */
function xr({ angle = 0, ls = 0, rs = 0, lt = 0, rt = 0, rightGrip = true, buttons = {} } = {}) {
  const at = (side) => toRig(head, SEATED_EYE, {
    x: side * GRIP.span * Math.cos(angle),
    y: GRIP.height,
    z: GRIP.forward - side * GRIP.span * Math.sin(angle),
  });
  const hand = (side, squeeze, trigger, tracked, pressed = []) => ({
    active: true,
    grip: tracked ? { position: at(side), orientation: [0, 0, 0, 1] } : null,
    trigger,
    squeeze,
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

const grab = async (base, extra = {}) => {
  await hold(base, xr({ ls: 1, rs: 1, ...extra }));
  return step(base, 1);
};

async function respawn(base, sample) {
  await post(base, "/input", sample);
  await key(base, "r");
  await step(base, 2);
  return step(base, 30);
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
    check(!m.vr.left.latched && !m.vr.armed, `open hands on the grips: not latched`);

    // 2. Grab, then steer.
    m = await grab(base);
    check(m.vr.left.latched && m.vr.right.latched && m.vr.armed && Math.abs(m.vr.neutral) < 1e-6, `both grips latched and armed, neutral ${m.vr.neutral.toExponential(2)}`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: 0.03 }), 30);
    check(m.vr.steer === 0 && m.ride.yawRate === 0, `inside the dead zone: steer ${m.vr.steer}`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: 0.2 }), 30);
    check(m.vr.steer > 0.4 && m.ride.yawRate > 0 && m.ride.yaw > 0, `bars left: steer ${m.vr.steer.toFixed(3)}, yaw rate ${m.ride.yawRate.toFixed(3)} (left)`);
    m = await hold(base, xr({ ls: 1, rs: 1, angle: -0.2 }), 60);
    check(m.vr.steer < -0.4 && m.ride.yawRate < 0, `bars right: steer ${m.vr.steer.toFixed(3)}, yaw rate ${m.ride.yawRate.toFixed(3)} (right)`);

    // 3. A full-trigger ride, bars at neutral.
    m = await respawn(base, xr({ ls: 1, rs: 1 }));
    check(m.vr.armed && m.speed === 0, `respawn keeps the grips`);
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

    // 6. Stepped turning, from rest. Right B respawns.
    await hold(base, xr({ ls: 1, rs: 1, buttons: { right: ["secondary"] } }), 1);
    m = await hold(base, xr({ ls: 1, rs: 1 }), 30);
    check(m.speed === 0 && m.steps === 30 && Math.abs(m.last.z - start.z) < 1e-6, `right B respawns`);
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
    await grab(base);
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
