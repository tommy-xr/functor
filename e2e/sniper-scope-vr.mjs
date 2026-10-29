// Sniper-scope VR E2E: injected XR poses drive the held rifle and its scope
// camera, headlessly, with no headset and no GL window.
//
// Drives `examples/sniper-scope` over the debug server (`--headless`): each
// step injects a right-hand sample (`POST /input {"type":"xr"}`) and advances
// one pinned fixed step. The scope camera is read from `/scene`
// (`render_targets[0].frame.camera`), so what is asserted is the frame the
// renderer would get, through `Camera3D.mapTrackedPose` + `Camera3D.fromPose`:
//
//   1. A toggles the shoulder; the scope pass appears, looking where the hand
//      points.
//   2. A canted controller (rolled 90 degrees about its forward) rolls the
//      scope camera, the right way round: the 200 m plate's tall left post
//      (an asymmetric landmark) shows ABOVE the plate, not below.
//   3. A head-only move changes nothing about the aim.
//   4. The trigger fires once per pull and must be released to fire again.
//   5. Losing the hand hides the rifle, unshoulders, and a trigger held on
//      return waits for a release.
//
// Run (needs the CLI; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/sniper-scope-vr.mjs
import { spawn } from "node:child_process";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const PORT = 8189;
const BASE = `http://127.0.0.1:${PORT}`;

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

async function waitReady(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if ((await get("/state")).frame > 0) return;
    } catch {
      // not listening yet
    }
    await sleep(100);
  }
  throw new Error("runtime never reported a frame");
}

// An injected pose is level state, so each step waits for its frame to land
// before the next sample replaces it.
async function step(sample, n = 1) {
  for (let i = 0; i < n; i++) {
    await post("/input", sample);
    const before = (await get("/state")).frame;
    await post("/time", { type: "advance", dts: 1 / 60 });
    const deadline = Date.now() + 15000;
    for (;;) {
      if ((await get("/state")).frame > before) break;
      if (Date.now() > deadline) throw new Error(`frame never advanced past ${before}`);
      await sleep(5);
    }
  }
  return (await get("/state")).model;
}

// Quaternion [x, y, z, w] for `deg` about rig +Z — the controller's forward
// axis is rig -Z, so +90 turns its top toward the player's left.
const roll = (deg) => {
  const h = (deg * Math.PI) / 360;
  return [0, 0, Math.sin(h), Math.cos(h)];
};

const GRIP = [0.15, -0.2, -0.3];
const sample = ({ orientation = [0, 0, 0, 1], trigger = 0, a = false, head, tracked = true } = {}) => ({
  type: "xr",
  head: head ?? { position: [0, 0, 0] },
  left: { active: false },
  right: tracked
    ? {
        active: true,
        grip: { position: GRIP, orientation },
        aim: { position: GRIP, orientation },
        trigger,
        primary_pressed: a,
      }
    : { active: false },
});

const sub = (a, b) => a.map((v, i) => v - b[i]);
const dot = (a, b) => a.reduce((s, v, i) => s + v * b[i], 0);
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (a) => a.map((v) => v / Math.hypot(...a));

// Where a world point sits in a camera's view: (right, up) over depth, the
// look_at_rh basis (right = forward × up).
function onScreen(cam, p) {
  const f = unit(sub(cam.target, cam.eye));
  const r = unit(cross(f, cam.up));
  const u = cross(r, f);
  const d = sub(p, cam.eye);
  return { x: dot(d, r) / dot(d, f), y: dot(d, u) / dot(d, f) };
}

const scopeCamera = async () => {
  const targets = (await get("/scene")).render_targets;
  return targets.length ? targets[0].frame.camera : undefined;
};

const proc = spawn(
  BIN,
  ["-d", "examples/sniper-scope", "run", "native", "--headless", "--debug-port", String(PORT)],
  { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
);
let log = "";
proc.stdout.on("data", (d) => (log += d));
proc.stderr.on("data", (d) => (log += d));

try {
  await waitReady(30000);
  await post("/time", { type: "set", tts: 0.0 });

  console.log("\n▸ shoulder toggle");
  let m = await step(sample());
  check(m.xr === true && m.tracked === true, "an injected right hand switches to VR");
  check((await scopeCamera()) === undefined, "no scope pass at the hip");
  await step(sample({ a: true }), 3);
  m = await step(sample(), 20);
  check(m.aiming === true && m.shoulder === 1, `A toggled the shoulder once (aiming ${m.aiming}, shoulder ${m.shoulder})`);
  const level = await scopeCamera();
  check(level !== undefined, "the scope pass is declared once shouldered");
  const fwd = unit(sub(level.target, level.eye));
  check(fwd[2] > 0.999 && level.up[1] > 0.999, `an identity grip aims down +Z, up +Y (fwd ${fwd}, up ${level.up})`);

  console.log("\n▸ canted aim rolls the scope camera");
  await step(sample({ orientation: roll(90) }));
  const canted = await scopeCamera();
  check(canted.up[0] > 0.999, `rolled +90°, the camera's up is world +X (shooter's left): ${canted.up}`);
  check(unit(sub(canted.target, canted.eye))[2] > 0.999, "rolling about forward keeps the aim");
  // The 200 m plate's frame: a tall cyan-capped post on the shooter's left
  // (+X) and a short one on the right. Level, the cap is left of the plate;
  // canted top-left, it must sit ABOVE it. A mirrored basis puts it below.
  const plate = [0, 1.0, 200];
  const cap = [0.75, 1.8, 200];
  const lv = { cap: onScreen(level, cap), plate: onScreen(level, plate) };
  const cv = { cap: onScreen(canted, cap), plate: onScreen(canted, plate) };
  check(lv.cap.x < lv.plate.x, "level: the tall post is screen-left of the plate");
  // Rolled so world +X is screen-up, world +Y (the post's top) is screen-right.
  check(cv.cap.y > cv.plate.y + 0.002, `canted: the tall post is above the plate (Δy ${(cv.cap.y - cv.plate.y).toFixed(4)})`);
  check(cv.cap.x > cv.plate.x + 0.002, `canted: the post's cap is right of the plate (Δx ${(cv.cap.x - cv.plate.x).toFixed(4)})`);

  console.log("\n▸ a head move does not change the aim");
  await step(sample({ orientation: roll(30) }));
  const before = await scopeCamera();
  await step(sample({ orientation: roll(30), head: { position: [0.2, 0.1, -0.15], orientation: [0.1, 0.3, 0.05, 0.95] } }), 2);
  const after = await scopeCamera();
  const head = (await get("/state")).input.xr?.head?.position;
  check(head?.[0] === 0.2, `the head move reached the runtime (head ${head})`);
  check(JSON.stringify(after) === JSON.stringify(before), "the scope camera is identical after the head moved");
  await step(sample(), 1);

  console.log("\n▸ trigger re-arm");
  m = await step(sample({ trigger: 1 }));
  check(m.shots === 1, `a pull fires (shots ${m.shots})`);
  m = await step(sample({ trigger: 1 }), 70);
  check(m.shots === 1 && m.cooldown === 0, `holding past the bolt does not refire (shots ${m.shots})`);
  m = await step(sample({ trigger: 0.4 }));
  m = await step(sample({ trigger: 1 }));
  check(m.shots === 1, "a partial release does not re-arm");
  m = await step(sample({ trigger: 0 }));
  m = await step(sample({ trigger: 1 }), 3);
  check(m.shots === 2, `release, then pull: fires again (shots ${m.shots})`);

  console.log("\n▸ tracking loss");
  m = await step(sample(), 70);
  const grip = JSON.stringify(m.grip);
  // Only the rifle (turrets, barrel) uses cylinders.
  const rifleShown = async () => JSON.stringify((await get("/scene")).scene).includes('"Cylinder"');
  check(await rifleShown(), "the rifle is drawn while tracked");
  m = await step(sample({ tracked: false }));
  check(m.tracked === false && m.aiming === false && m.triggerArmed === false, "losing the hand unshoulders and disarms");
  check(JSON.stringify(m.grip) === grip, "the weapon pose freezes");
  m = await step(sample({ tracked: false }), 20);
  check(m.shoulder === 0 && (await scopeCamera()) === undefined, "the scope pass is gone once unshouldered");
  check(!(await rifleShown()), "the rifle is hidden");
  await step(sample({ trigger: 1 }));
  await step(sample({ trigger: 1, a: true }));
  m = await step(sample({ trigger: 1 }), 20);
  check(m.shoulder === 1 && m.shots === 2, `a trigger held through the return does not fire (shots ${m.shots})`);
  await step(sample({ trigger: 0 }));
  m = await step(sample({ trigger: 1 }), 3);
  check(m.shots === 3, `after a release it fires (shots ${m.shots})`);
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${log.slice(-4000)}`);
  process.exit(1);
}
console.log("\nsniper-scope VR e2e: all checks passed");
