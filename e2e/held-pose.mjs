// Held-pose E2E: injected XR grips place `examples/held-pose`'s asymmetric
// marker through `Camera3D.mapTrackedPose` + `Scene.atPose`, headlessly.
//
// Ground truth is the injected quaternion's OpenXR meaning, computed here
// independently of the engine: the controller's forward is tracked -Z, its top
// +Y, so the held marker's cyan arm (+Z) follows tracked -Z, green (+Y)
// follows tracked +Y, and pink (+X, the hand's left) follows tracked -X. The
// rig looks down world +Z from (0, 1.5, 0), which maps tracked (x, y, z) to
// world (-x, y, -z). Each arm's world direction is read from `/scene` by
// composing the node matrices. A mirrored basis flips pink and fails.
//
// Run (needs the CLI; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/held-pose.mjs
import { spawn } from "node:child_process";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const PORT = 8191;
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

// An injected pose is level state: wait for its fixed step to land.
async function step() {
  const before = (await get("/state")).frame;
  await post("/time", { type: "advance", dts: 1 / 60 });
  const deadline = Date.now() + 15000;
  while ((await get("/state")).frame <= before) {
    if (Date.now() > deadline) throw new Error(`frame never advanced past ${before}`);
    await sleep(5);
  }
}

// --- vectors and quaternions ([x, y, z, w], OpenXR order) ---
const sub = (a, b) => a.map((v, i) => v - b[i]);
const dot = (a, b) => a.reduce((s, v, i) => s + v * b[i], 0);
const unit = (a) => a.map((v) => v / Math.hypot(...a));
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
// v' = v + 2w(q × v) + 2 q × (q × v)
const rotate = ([x, y, z, w], v) => {
  const q = [x, y, z];
  const t = cross(q, v).map((c) => 2 * c);
  const u = cross(q, t);
  return v.map((c, i) => c + w * t[i] + u[i]);
};
const EYE = [0, 1.5, 0];
const toWorldDir = ([x, y, z]) => [-x, y, -z];
const toWorld = (p) => toWorldDir(p).map((v, i) => v + EYE[i]);

// --- the scene: world-space centers of the marker's boxes, by color ---
const COLORS = { hub: [0.95, 0.95, 1.0], forward: [0.1, 0.85, 1.0], up: [0.3, 1.0, 0.4], left: [1.0, 0.2, 0.72] };
// Column-major 4x4s, as `/scene` serializes them.
const mmul = (a, b) =>
  b.map((col) => [0, 1, 2, 3].map((r) => a.reduce((s, ac, k) => s + ac[r] * col[k], 0)));
const IDENTITY = [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]];

function boxCenters(scene) {
  const found = {};
  const walk = (node, parent, color) => {
    const world = mmul(parent, node.xform);
    const obj = node.obj;
    if (obj.Group) obj.Group.forEach((c) => walk(c, world, color));
    else if (obj.Material) obj.Material[1].forEach((c) => walk(c, world, obj.Material[0].Color ?? color));
    else if (obj.Geometry === "Cube" && color) {
      for (const [name, rgb] of Object.entries(COLORS)) {
        if (rgb.every((v, i) => Math.abs(v - color[i]) < 1e-6)) found[name] = world[3].slice(0, 3);
      }
    }
  };
  walk(scene, IDENTITY, null);
  return found;
}

const sample = (position, orientation) => ({
  type: "xr",
  head: { position: [0, 0, 0] },
  left: { active: false },
  right: { active: true, grip: { position, orientation }, aim: { position, orientation } },
});

const near = (a, b, eps = 1e-3) => a.every((v, i) => Math.abs(v - b[i]) < eps);
const fmt = (v) => `[${v.map((x) => x.toFixed(3)).join(", ")}]`;

// Inject one grip; assert the hub sits at the mapped position and each arm
// points along its expected world axis.
async function expectPlaced(label, position, orientation) {
  await post("/input", sample(position, orientation));
  await step();
  const c = boxCenters((await get("/scene")).scene);
  const hub = toWorld(position);
  const axes = {
    forward: toWorldDir(rotate(orientation, [0, 0, -1])),
    up: toWorldDir(rotate(orientation, [0, 1, 0])),
    left: toWorldDir(rotate(orientation, [-1, 0, 0])),
  };
  check(c.hub && near(c.hub, hub), `${label}: hub at the mapped grip ${fmt(hub)} (got ${c.hub && fmt(c.hub)})`);
  for (const [arm, axis] of Object.entries(axes)) {
    const dir = c[arm] && unit(sub(c[arm], c.hub));
    check(dir && dot(dir, axis) > 0.9999, `${label}: ${arm} arm along ${fmt(axis)} (got ${dir && fmt(dir)})`);
  }
  return c;
}

const proc = spawn(
  BIN,
  ["-d", "examples/held-pose", "run", "native", "--headless", "--emulate-xr", "--debug-port", String(PORT)],
  { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
);
let log = "";
proc.stdout.on("data", (d) => (log += d));
proc.stderr.on("data", (d) => (log += d));

try {
  await waitReady(30000);
  await post("/time", { type: "set", tts: 0.0 });

  console.log("\n▸ level grip");
  await expectPlaced("level", [0.1, -0.2, -0.4], [0, 0, 0, 1]);

  console.log("\n▸ rolled 90° about the controller's forward (top to the player's left)");
  const rolled = await expectPlaced("rolled", [0.1, -0.2, -0.4], axisAngle([0, 0, 1], 90));
  // Spelled out: the top turns to world +X (left of a +Z gaze), so the hand's
  // left (pink) points world DOWN. A mirrored basis points it up.
  check(rolled.up[0] > rolled.hub[0] + 0.03, "rolled: the green arm points world +X");
  check(rolled.left[1] < rolled.hub[1] - 0.03, "rolled: the pink arm points world down");

  console.log("\n▸ yawed, pitched and rolled");
  const general = qmul(qmul(axisAngle([0, 1, 0], -50), axisAngle([1, 0, 0], 35)), axisAngle([0, 0, 1], -70));
  await expectPlaced("general", [-0.2, 0.1, -0.3], general);

  console.log("\n▸ losing the hand falls back to the demo pose");
  await post("/input", { type: "xr", right: { active: false } });
  await step();
  check((await get("/state")).model.tracked === false, "untracked once the right hand drops");
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
console.log("\nheld-pose e2e: all checks passed");
