// Scripted XR scenarios against a live desktop runtime: the shared driver
// behind the VR e2e checks (`e2e/vr-lab.mjs`) and the demo-video recorder
// (`e2e/xr-demo.mjs`).
//
// A scenario is plain data: a station key to press first, a spectator view,
// and a list of segments. Each segment runs `frames` fixed steps; a hand named
// in it moves in a straight line from where the previous segment left it to
// `to` (rig-local metres: +X right, +Y up from the floor, -Z forward), with
// its squeeze held at `squeeze`. `turn` (a quaternion, `[x, y, z, w]`) turns
// its grip the same way, the shorter way round, from where it was to that.
// `active: false` loses the hand's tracking. A segment's `tap` key is
// pressed on its first step and released after it.
// A segment's `mark` names its last step, so a check can find it.
//
//   { frames: 30, right: { to: [0.2, 1.0, -0.55], squeeze: 0 } }
//
// The head stands at `[0, 1.6, 0]` looking ahead; a segment's `head` moves
// and turns it the same way. A scenario's `neck: { at, eye }` instead pivots
// it: the eyes sit at `eye` in the head's frame from a neck fixed at `at`,
// so a `head: { turn }` turns it about the neck, as a real head does.
//
// A scenario may instead name a recorded `fixture` (`e2e/xr-fixture.mjs`
// format; a path from the repo root): its steps replace `station`/`steps`,
// and its `marks` name steps the same way.
//
//   { fixture: "e2e/fixtures/vr-lab/grab-throw.xr.json", spectator: { ... } }
//
// One step = one injected sample + one pinned `/time` advance, waiting for the
// frame to land before the next sample (a sample is level state, not a queue).
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { BIN, ROOT, failures, portIsBound, sleep } from "./mcp-rpc.mjs";
import { decode } from "./xr-fixture.mjs";

export const DT = 1 / 60;
const HEAD = [0, 1.6, 0];
// At the player's sides, clear of the props on the table.
const REST = { left: [-0.25, 0.95, -0.1], right: [0.25, 0.95, -0.2] };

/** A client for the debug server on `port` (a desktop runtime, or a headset's over `adb forward`). */
export function attach(port) {
  const base = `http://127.0.0.1:${port}`;
  return {
    async get(path) {
      const res = await fetch(`${base}${path}`);
      if (!res.ok) throw new Error(`GET ${path} → ${res.status}: ${await res.text()}`);
      return res.json();
    },
    async post(path, body) {
      const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
      const text = await res.text();
      if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
      return text;
    },
    async capture() {
      const res = await fetch(`${base}/capture`, { method: "POST" });
      if (!res.ok) throw new Error(`POST /capture → ${res.status}: ${await res.text()}`);
      return Buffer.from(await res.arrayBuffer());
    },
  };
}

/** Launch `functor run native` on `project` with the debug server on `port`. */
export async function launch(project, port, extra = []) {
  // A stale runtime on the port would answer in place of this one.
  if (await portIsBound(port)) throw new Error(`port ${port} is taken: stop what's listening there`);
  const proc = spawn(BIN, ["-d", project, "run", "native", "--debug-port", String(port), ...extra], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  const rt = {
    ...attach(port),
    proc,
    log: () => log,
    // Resolves once the process is gone, so the port is free for the next.
    kill: () =>
      new Promise((done) => {
        if (proc.exitCode !== null || proc.signalCode !== null) return done();
        proc.once("exit", done);
        proc.kill("SIGKILL");
      }),
  };
  const deadline = Date.now() + 60000;
  for (;;) {
    try {
      if ((await rt.get("/state")).frame > 0) break;
    } catch {
      // not listening yet
    }
    if (proc.exitCode !== null) throw new Error(`runtime exited:\n${log}`);
    if (Date.now() > deadline) throw new Error(`runtime never reported a frame:\n${log}`);
    await sleep(100);
  }
  await rt.post("/time", { type: "set", tts: 0.0 });
  return rt;
}

/**
 * Run `body` against a fresh runtime on `project`: a throw is recorded in
 * `failures` with the runtime's log, and the runtime is always stopped.
 */
export async function session(name, project, port, body, flags = ["--headless"]) {
  console.log(`\n▸ ${name}`);
  let rt;
  try {
    rt = await launch(project, port, flags);
    await body(rt);
  } catch (e) {
    failures.push(`${name}: ${e}`);
    console.error(e, `\n--- runtime log ---\n${rt?.log().slice(-3000) ?? ""}`);
  } finally {
    await rt?.kill();
  }
}

/** Advance one pinned step and wait for it; returns `/state`. */
export async function advance(rt, dts = DT) {
  const before = (await rt.get("/state")).frame;
  await rt.post("/time", { type: "advance", dts });
  const deadline = Date.now() + 15000;
  for (;;) {
    const state = await rt.get("/state");
    if (state.frame > before) return state;
    if (Date.now() > deadline) throw new Error(`frame never advanced past ${before}`);
    await sleep(2);
  }
}

/** Tap a key (its press lands on the next step). */
export async function tap(rt, key) {
  await rt.post("/input", { type: "key", key, down: true });
  await advance(rt);
  await rt.post("/input", { type: "key", key, down: false });
}

const hand = (h) =>
  h.active === false
    ? { active: false }
    : {
        active: true,
        grip: { position: h.at, orientation: h.orientation ?? [0, 0, 0, 1] },
        aim: { position: h.at, orientation: h.orientation ?? [0, 0, 0, 1] },
        squeeze: h.squeeze ?? 0,
      };

/** The XR sample for the head's and both hands' current state. */
export const sample = (hands) => ({
  type: "xr",
  head: { position: hands.head?.at ?? HEAD, orientation: hands.head?.orientation ?? [0, 0, 0, 1] },
  left: hand(hands.left),
  right: hand(hands.right),
});

// Vectors as [x, y, z] and quaternions as [x, y, z, w]: the Hamilton product
// (`b` first, then `a`), the inverse of a unit one, and a rotated vector.
export const add = (a, b) => a.map((v, i) => v + b[i]);
export const sub = (a, b) => a.map((v, i) => v - b[i]);
export const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
export const qmul = ([ax, ay, az, aw], [bx, by, bz, bw]) => [
  aw * bx + ax * bw + ay * bz - az * by,
  aw * by - ax * bz + ay * bw + az * bx,
  aw * bz + ax * by - ay * bx + az * bw,
  aw * bw - ax * bx - ay * by - az * bz,
];
export const qconj = ([x, y, z, w]) => [-x, -y, -z, w];
export const qrot = ([x, y, z, w], v) => {
  const t = cross([x, y, z], v).map((c) => 2 * c);
  const u = cross([x, y, z], t);
  return v.map((c, i) => c + w * t[i] + u[i]);
};

const lerp = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);

/** Constant-rate quaternion interpolation (`[x, y, z, w]`), the shorter way. */
export function slerp(a, b, t) {
  const d = a.reduce((s, v, i) => s + v * b[i], 0);
  const sign = d < 0 ? -1 : 1;
  const half = Math.acos(Math.min(1, Math.abs(d)));
  if (half < 1e-4) return a.map((v, i) => v + (sign * b[i] - v) * t);
  const wa = Math.sin((1 - t) * half) / Math.sin(half);
  const wb = (sign * Math.sin(t * half)) / Math.sin(half);
  return a.map((v, i) => wa * v + wb * b[i]);
}

/**
 * Play `scenario` step by step. `onStep({ step, mark, state, hands })` runs
 * after every step (await it to capture a frame); returns the marks' states.
 */
export async function play(rt, scenario, onStep = async () => {}) {
  if (scenario.fixture) return playFixture(rt, scenario.fixture, onStep);
  if (scenario.station !== undefined) await tap(rt, String(scenario.station));
  const neck = scenario.neck;
  const headAt = (orientation) => (neck ? add(neck.at, qrot(orientation, neck.eye)) : HEAD);
  const hands = {
    head: { at: headAt([0, 0, 0, 1]), orientation: [0, 0, 0, 1] },
    left: { at: REST.left, squeeze: 0 },
    right: { at: REST.right, squeeze: 0 },
  };
  const marks = {};
  let step = 0;
  for (const seg of scenario.steps) {
    const from = { head: { ...hands.head }, left: { ...hands.left }, right: { ...hands.right } };
    for (let i = 0; i < seg.frames; i++) {
      for (const side of ["head", "left", "right"]) {
        const spec = seg[side];
        if (!spec) continue;
        hands[side] = {
          ...hands[side],
          ...spec,
          at: spec.to ? lerp(from[side].at, spec.to, (i + 1) / seg.frames) : hands[side].at,
          orientation: spec.turn
            ? slerp(from[side].orientation ?? [0, 0, 0, 1], spec.turn, (i + 1) / seg.frames)
            : hands[side].orientation,
        };
      }
      if (neck) hands.head.at = headAt(hands.head.orientation);
      const tapped = i === 0 && seg.tap;
      if (tapped) await rt.post("/input", { type: "key", key: seg.tap, down: true });
      await rt.post("/input", sample(hands));
      const state = await advance(rt);
      if (tapped) await rt.post("/input", { type: "key", key: seg.tap, down: false });
      step++;
      const mark = i === seg.frames - 1 ? seg.mark : undefined;
      if (mark) marks[mark] = state;
      await onStep({ step, mark, state, hands });
    }
  }
  return marks;
}

/** Load a fixture: a path from the repo root, or the parsed object. */
export const loadFixture = (fixture) =>
  typeof fixture === "string" ? JSON.parse(readFileSync(resolve(ROOT, fixture), "utf8")) : fixture;

/** Replay a recorded fixture step by step; `play`'s callback and result. */
export async function playFixture(rt, fixture, onStep = async () => {}) {
  const { dt, t0, marks: named, steps } = decode(loadFixture(fixture));
  const markAt = new Map(Object.entries(named).map(([name, i]) => [i, name]));
  await rt.post("/time", { type: "set", tts: t0 });
  const marks = {};
  for (const [i, s] of steps.entries()) {
    for (const key of s.down) await rt.post("/input", { type: "key", key, down: true });
    for (const key of s.up) await rt.post("/input", { type: "key", key, down: false });
    await rt.post("/input", s.xr ? { type: "xr", ...s.xr } : { type: "xr_clear" });
    const state = await advance(rt, dt);
    const mark = markAt.get(i);
    if (mark) marks[mark] = state;
    await onStep({ step: i + 1, mark, state });
  }
  return marks;
}

/** Rig-local → world for a station rig at `feet` facing `yaw` (radians, 0 = +Z). */
export function rigToWorld([x, y, z], feet = [0, 0, 0], yaw = 0) {
  // Tracked -Z is the station's forward: a half turn about +Y beyond its yaw.
  const a = yaw + Math.PI;
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [feet[0] + c * x + s * z, feet[1] + y, feet[2] - s * x + c * z];
}

export const dirToWorld = (v, yaw = 0) => rigToWorld(v, [0, 0, 0], yaw);
