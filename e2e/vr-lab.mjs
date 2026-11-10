// vr-lab E2E: the grab table driven by injected XR samples, headlessly.
//
// Plays `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab` over the debug
// server (`--headless`, pinned clock, one sample per fixed step) and checks
// `/state.model`, whose `ballPos`/`ballVel` are the physics world read back
// each step (one step behind):
//
//   1. A steady sweep, released mid-swing, launches the ball at the sweep's
//      velocity (within 5% and 5 degrees), still holding its speed a step
//      later, and it lands in the basket. A swing past the speed cap leaves
//      at the cap, so the release's velocity command is what sets it.
//   2. A hand pushed through the wall leaves the ball at the wall; the ball
//      follows it back out; let go there, the ball drops outside the wall.
//   3. Dragged farther, the grip breaks: dropped in place, no velocity, and the
//      still-squeezing hand can't take it back.
//   4. Losing the hand mid-swing and opening it on return never throws.
//   4b. The stick, its hand held still and the wrist turned a quarter turn
//      into the wall, never passes the wall plane at any step and stops at
//      it; turned back, it follows the hand; let go, it is free.
//   4c. The held stick, swept across the table, shoves the ball away. Flicked
//      and let go mid-flick, it is thrown with the wrist's spin, and measured
//      from its read-back poses two steps later it still spins at that rate.
//      Landed and at rest, its read-back pose is the pose it is drawn at, and
//      the same throw in a fresh runtime ends bit-identically.
//   5. A station reset mid-hold drops the hold and respawns the ball at rest.
//   6. The same throw in two fresh runtimes ends bit-identically.
//   7. Recorded fixtures (`e2e/xr-fixture.mjs`): the checked-in grab-throw
//      recording lands in the basket; a scripted run recorded under
//      `--emulate-xr` and a live (wall-clock) emulated session recorded over
//      `GET /input-log` each replay in fresh runtimes to a bit-identical
//      model, twice.
//
// Run (needs the CLI; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab.mjs
import { check, failures, sleep } from "./mcp-rpc.mjs";
import { decode, encode, readLog, recordLive } from "./xr-fixture.mjs";
import { advance, dirToWorld, launch, play, playFixture, rigToWorld, tap } from "./xr-scenario.mjs";
import { BALL, FAST_V, FLICK_W, STICK, THROW_V, WALL_RIG_X, project, scenarios, yaw } from "./vr-lab-scenarios.mjs";

const PORT = 8293;
const WALL_FACE = 0.6; // world x, at the table's end
const MAX_SPEED = 12;
const RADIUS = 0.06;

const len = (v) => Math.hypot(...v);
const sub = (a, b) => a.map((x, i) => x - b[i]);
const xyz = (p) => [p.x, p.y, p.z];
const angleDeg = (a, b) =>
  (Math.acos(Math.min(1, a.reduce((s, x, i) => s + x * b[i], 0) / (len(a) * len(b)))) * 180) / Math.PI;
const some = (opt) => (opt?.$ctor === "Option.Some" ? opt.args[0] : undefined);
const fmt = (v) => `[${v.map((x) => x.toFixed(3)).join(", ")}]`;
const quat = (q) => [q.x, q.y, q.z, q.w];
// v' = v + 2w(q × v) + 2 q × (q × v)
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const rotate = ([x, y, z, w], v) => {
  const t = cross([x, y, z], v).map((c) => 2 * c);
  const u = cross([x, y, z], t);
  return v.map((c, i) => c + w * t[i] + u[i]);
};
const turnDeg = (a, b) =>
  (2 * Math.acos(Math.min(1, Math.abs(a.reduce((s, v, i) => s + v * b[i], 0)))) * 180) / Math.PI;
// A posed stick's eight corners (rig-local): its length along its -Z.
const corners = (pose) => {
  const [h, l] = [STICK.thickness / 2, STICK.length / 2];
  const out = [];
  for (const x of [-h, h]) for (const y of [-h, h]) for (const z of [-l, l]) {
    out.push(rotate(quat(pose.orientation), [x, y, z]).map((c, i) => c + xyz(pose.position)[i]));
  }
  return out;
};
const leftmost = (pose) => Math.min(...corners(pose).map((c) => c[0]));
const mul = ([ax, ay, az, aw], [bx, by, bz, bw]) => [
  aw * bx + ax * bw + ay * bz - az * by,
  aw * by - ax * bz + ay * bw + az * bx,
  aw * bz + ax * by - ay * bx + az * bw,
  aw * bw - ax * bx - ay * by - az * bz,
];
const conj = ([x, y, z, w]) => [-x, -y, -z, w];
// A rotation's axis × angle (radians), the shorter way round.
const log = (q) => {
  const [x, y, z, w] = q[3] < 0 ? q.map((c) => -c) : q;
  const n = Math.hypot(x, y, z);
  return n < 1e-9 ? [2 * x, 2 * y, 2 * z] : [x, y, z].map((c) => (c * 2 * Math.atan2(n, w)) / n);
};
// The spin carrying rig-local pose `a` to `b` in one step (rad/s).
const spinBetween = (a, b) => log(mul(quat(b.orientation), conj(quat(a.orientation)))).map((c) => c * 60);

async function run(name, body, flags = ["--headless"]) {
  console.log(`\n▸ ${name}`);
  let rt;
  try {
    rt = await launch(project, PORT, flags);
    await body(rt);
  } catch (e) {
    failures.push(`${name}: ${e}`);
    console.error(e, `\n--- runtime log ---\n${rt?.log().slice(-3000) ?? ""}`);
  } finally {
    await rt?.kill();
  }
}

let throwEnd;
await run("grab and throw", async (rt) => {
  const m = await play(rt, scenarios["grab-throw"]);
  const reached = m.reached.model;
  check(reached.right.hover === true, "the open hand at the ball hovers it");
  const held = some(m.grabbed.model.held);
  check(held?.side?.$ctor === "Hold.Right", `the squeeze grabs it with the right hand (${JSON.stringify(held?.side)})`);

  const release = some(m.release.model.lastRelease);
  const want = dirToWorld(THROW_V);
  check(release?.kind?.$ctor === "Throw", "opening mid-swing throws");
  const v0 = release ? xyz(release.velocity) : [0, 0, 0];
  check(len(sub(v0, want)) < 0.01 * len(want), `the estimate is the sweep's velocity: ${fmt(v0)} vs ${fmt(want)}`);

  // The world one step after release (the read lags a step, so `after1`).
  const v1 = xyz(m.after1.model.ballVel);
  const speedErr = Math.abs(len(v1) - len(want)) / len(want);
  const dir = angleDeg(v1, want);
  check(speedErr < 0.05 && dir < 5, `the ball leaves at it: ${fmt(v1)} (${(speedErr * 100).toFixed(1)}%, ${dir.toFixed(1)}°)`);
  const v2 = xyz(m.after2.model.ballVel);
  check(Math.abs(len(v2) - len(want)) / len(want) < 0.05, `and still has its speed a step later: |v| ${len(v2).toFixed(3)}`);
  check(m.landed.model.score === 1, `it lands in the basket (score ${m.landed.model.score})`);
  throwEnd = m.landed.model;
});

await run("fast throw", async (rt) => {
  const m = await play(rt, scenarios["fast-throw"]);
  const release = some(m.release.model.lastRelease);
  const want = dirToWorld(FAST_V).map((v) => (v * MAX_SPEED) / len(FAST_V));
  const v0 = release ? xyz(release.velocity) : [0, 0, 0];
  check(len(sub(v0, want)) < 0.01 * MAX_SPEED, `a 16 m/s swing estimates the 12 m/s cap: ${fmt(v0)}`);
  // The kinematic ball was moving at 16 m/s; only the release's velocity
  // command brings it to 12.
  const v1 = xyz(m.after1.model.ballVel);
  check(Math.abs(len(v1) - MAX_SPEED) / MAX_SPEED < 0.05 && angleDeg(v1, want) < 5, `the ball leaves at it: |v| ${len(v1).toFixed(3)} (${angleDeg(v1, want).toFixed(1)}°)`);
});

await run("wall push", async (rt) => {
  const m = await play(rt, scenarios["wall-push"]);
  const pushed = m.pushed.model;
  const accepted = some(pushed.held)?.accepted;
  check(accepted !== undefined, "still held with the hand pushed through the wall");
  const ballX = pushed.ballPos.x;
  check(ballX < WALL_FACE - RADIUS && ballX > WALL_FACE - RADIUS - 0.02, `the ball stops at the wall (x ${ballX.toFixed(4)})`);
  const pulled = some(m.pulled.model.held);
  // The hand's rig x -0.4 is world 0.4; the ball follows it out.
  check(pulled && Math.abs(-pulled.accepted.x - 0.4) < 0.005, `pulled back, it follows the hand (rig x ${pulled?.accepted.x.toFixed(4)})`);
  const release = some(m.release.model.lastRelease);
  check(release?.kind?.$ctor === "Drop", `let go inside the wall: a drop (${release?.kind?.$ctor})`);
  check(release && release.at.x < WALL_FACE - RADIUS, `released outside the wall (x ${release?.at.x.toFixed(4)})`);
  const settled = m.settled.model.ballPos;
  check(settled.x < WALL_FACE - RADIUS + 0.005, `and stays outside it (x ${settled.x.toFixed(4)})`);
  check(Math.abs(settled.y - BALL[1]) < 0.005, `resting on the table (y ${settled.y.toFixed(4)})`);
});

await run("break grip", async (rt) => {
  const m = await play(rt, scenarios["break-grip"]);
  const broke = m.broke.model;
  const release = some(broke.lastRelease);
  check(release?.kind?.$ctor === "Break", `dragged 25 cm past the ball, the grip breaks (${release?.kind?.$ctor})`);
  check(release && len(xyz(release.velocity)) === 0, "dropped with no velocity");
  check(release && release.at.x < WALL_FACE - RADIUS, `where the wall stopped it (x ${release?.at.x.toFixed(4)})`);
  check(some(broke.held) === undefined && broke.right.armed === false, "the hand is disarmed");
  check(some(m.returned.model.held) === undefined, "still squeezing, back at the ball, it doesn't regrab");
});

await run("stick against the wall", async (rt) => {
  let deepest = Infinity;
  let deepestStep;
  const m = await play(rt, scenarios["stick-wall"], async ({ step, state }) => {
    const stick = some(state.model.stick);
    if (stick && leftmost(stick.accepted) < deepest) [deepest, deepestStep] = [leftmost(stick.accepted), step];
  });
  const took = some(m.tookStick.model.stick);
  check(took?.side?.$ctor === "Hold.Right" && some(m.tookStick.model.held) === undefined, "squeezed at its near end, the stick is held (not the ball)");
  check(deepest >= WALL_RIG_X - 0.002, `it never passes the wall plane (deepest rig x ${deepest.toFixed(4)}, step ${deepestStep})`);
  const pressed = some(m.pressed.model.stick);
  const far = pressed && leftmost(pressed.accepted);
  check(far !== undefined && far < WALL_RIG_X + 0.01, `turned a quarter turn, it stops at the wall (rig x ${far?.toFixed(4)})`);
  const lag = pressed && turnDeg(quat(pressed.accepted.orientation), yaw(90));
  check(lag > 45, `well short of the hand's turn (${lag?.toFixed(1)}° behind)`);
  const back = some(m.turnedBack.model.stick);
  // Where it was taken from the table, lifted with the hand.
  const home = sub(xyz(took.accepted.position), [0, 0.95 - 1.2, 0]);
  const off = back && len(sub(xyz(back.accepted.position), home));
  const turn = back && turnDeg(quat(back.accepted.orientation), [0, 0, 0, 1]);
  check(off < 1e-3 && turn < 0.1, `turned back, it follows the hand (${off?.toFixed(5)} m, ${turn?.toFixed(3)}°)`);
  check(some(m.letGo.model.stick) === undefined, "let go, it is free");
  const ball = xyz(m.letGo.model.ballPos);
  check(len(sub(ball, [-BALL[0], BALL[1], 4 - BALL[2]])) < 0.005, `the ball sat still throughout (${fmt(ball)})`);
});

let stickEnd;
const stickEndOf = (m) => JSON.stringify([m.rest.model.stickPose, m.rest.model.ballPos, m.rest.model.lastStickRelease]);
await run("stick shove and throw", async (rt) => {
  const m = await play(rt, scenarios["stick-throw"]);
  check(some(m.tookStick.model.stick) !== undefined, "the stick is taken");
  const ballBefore = xyz(m.low.model.ballPos);
  const swept = m.swept.model;
  const ballAfter = xyz(swept.ballPos);
  check(some(swept.stick) !== undefined, "still held after the sweep");
  // Rig +X is world -X: the sweep turns the stick's far end that way.
  check(ballBefore[0] - ballAfter[0] > 0.1, `the held stick shoves the ball away: ${fmt(ballBefore)} → ${fmt(ballAfter)}`);

  const release = some(m.release.model.lastStickRelease);
  const want = dirToWorld(FLICK_W);
  check(release?.kind?.$ctor === "Throw", `opening mid-flick throws it (${release?.kind?.$ctor})`);
  const w0 = release ? xyz(release.spin) : [0, 0, 0];
  check(len(sub(w0, want)) < 0.01 * len(want), `the estimate is the wrist's spin: ${fmt(w0)} vs ${fmt(want)}`);
  // Read back a step behind: after2 → after3 is the second step after release.
  const measured = spinBetween(m.after2.model.stickPose, m.after3.model.stickPose);
  const err = Math.abs(len(measured) - len(FLICK_W)) / len(FLICK_W);
  const dir = angleDeg(measured, FLICK_W);
  check(err < 0.05 && dir < 5, `two steps on it spins at it: ${fmt(measured)} rad/s (${(err * 100).toFixed(1)}%, ${dir.toFixed(1)}°)`);

  const [landed, rest] = [m.landed.model.stickPose, m.rest.model.stickPose];
  const still = len(sub(xyz(landed.position), xyz(rest.position))) + turnDeg(quat(landed.orientation), quat(rest.orientation));
  check(still < 1e-4, `it comes to rest (${still.toExponential(1)})`);
  // Drawn: `Scene.atPose` of the world pose, columns left, up, forward
  // (rig -Z) and position. The rig faces world +Z: a half turn about +Y.
  const xform = (await rt.get("/scene")).scene.obj.Group[4].xform;
  const toWorld = (v) => [-v[0], v[1], -v[2]];
  const q = quat(rest.orientation);
  const drawn = xform.map((c) => c.slice(0, 3));
  const [up, forward] = [toWorld(rotate(q, [0, 1, 0])), toWorld(rotate(q, [0, 0, -1]))];
  const read = [cross(up, forward), up, forward, rigToWorld(xyz(rest.position), [0, 0, 4])];
  const off = Math.max(...drawn.map((c, i) => len(sub(c, read[i]))));
  check(off < 1e-4, `at rest it is drawn at its read-back pose (${off.toExponential(1)})`);
  stickEnd = stickEndOf(m);
});

await run("tracking loss", async (rt) => {
  const m = await play(rt, scenarios["tracking-loss"]);
  const swinging = some(m.swinging.model.held);
  const lost = some(m.lost.model.held);
  check(lost && JSON.stringify(lost.accepted) === JSON.stringify(swinging.accepted), "the held ball freezes while the hand is lost");
  check(lost && lost.history.length === 0, "and forgets its history");
  const release = some(m.release.model.lastRelease);
  check(release?.kind?.$ctor === "Drop" && len(xyz(release.velocity)) === 0, `opened on return: a drop, not a throw (${release?.kind?.$ctor})`);
});

await run("station reset", async (rt) => {
  const s = scenarios["grab-throw"];
  await play(rt, { ...s, steps: s.steps.slice(0, 4) });
  const before = (await rt.get("/state")).model;
  check(some(before.held) !== undefined, "holding the ball");
  await tap(rt, "1");
  const after = (await advance(rt)).model;
  check(some(after.held) === undefined && after.ballGen === before.ballGen + 1, "1 drops the hold and respawns the ball");
  const spawn = after.ballPos;
  check(Math.abs(spawn.x + BALL[0]) < 1e-4 && Math.abs(spawn.y - BALL[1]) < 1e-4, `at rest at its spawn (${JSON.stringify(spawn)})`);
  check(len(xyz(after.ballVel)) < 1e-6, "with no velocity");
  check(after.right.armed === false && after.lastRelease.$ctor === "Option.None", "the still-squeezing hand is disarmed and the history gone");
});

await run("replay", async (rt) => {
  const m = await play(rt, scenarios["grab-throw"]);
  const a = throwEnd && JSON.stringify([throwEnd.ballPos, throwEnd.ballVel]);
  const b = JSON.stringify([m.landed.model.ballPos, m.landed.model.ballVel]);
  check(a === b, `the same throw in a fresh runtime ends identically (${b})`);
});

await run("stick replay", async (rt) => {
  const m = await play(rt, scenarios["stick-throw"]);
  const b = stickEndOf(m);
  check(stickEnd !== undefined && stickEnd === b, `the same stick throw in a fresh runtime ends identically (${b})`);
});

await run("recorded fixture", async (rt) => {
  const m = await play(rt, scenarios["grab-throw-recorded"]);
  check(some(m.release.model.lastRelease)?.kind?.$ctor === "Throw", "the recording's release throws");
  check(m.landed.model.score === 1, `and lands in the basket (score ${m.landed.model.score})`);
});

// The fixture of `steps` and the model they ended on.
async function recording(rt, steps) {
  const state = await rt.get("/state");
  check(state.frame === steps.at(-1).frame, `the log reaches the state shown (frame ${state.frame})`);
  return { fixture: encode(steps), model: JSON.stringify(state.model) };
}

async function replays(what, { fixture, model }) {
  let first;
  for (const n of [1, 2]) {
    await run(`replay ${what} (${n})`, async (rt) => {
      await playFixture(rt, fixture);
      const replayed = JSON.stringify((await rt.get("/state")).model);
      check(replayed === model, `${fixture.steps.length} steps replay to the recorded model`);
      if (first !== undefined) check(replayed === first, "the same as the first replay");
      first = replayed;
    });
  }
}

let scripted;
await run(
  "record a scripted run under --emulate-xr",
  async (rt) => {
    const from = (await rt.get("/state")).frame + 1;
    await play(rt, scenarios["grab-throw"]);
    scripted = await recording(rt, (await readLog(rt, from)).steps);
    check(JSON.parse(scripted.model).score === 1, "it scored");
  },
  ["--headless", "--emulate-xr"],
);
if (scripted) await replays("the scripted run", scripted);

// A live session, recorded as a worn one is: the wall clock runs, the
// emulator's hand follows the mouse and Enter squeezes.
let live;
await run(
  "record a live emulated session",
  async (rt) => {
    await rt.post("/time", { type: "resume" });
    // Resets before recording: a fresh launch never saw them.
    for (const down of [true, false, true, false]) {
      await rt.post("/input", { type: "key", key: "1", down });
      await sleep(100);
    }
    check((await rt.get("/state")).model.ballGen === 3, "reset twice before recording");
    let stop;
    const stopped = new Promise((done) => (stop = done));
    const recorded = recordLive(rt, { resetKey: "1", stopped });
    for (let i = 0; i < 90; i++) {
      await rt.post("/input", { type: "mouse_move", x: 400 + 3 * i, y: 300 - i });
      if (i === 30 || i === 60) await rt.post("/input", { type: "key", key: "enter", down: i === 30 });
      await sleep(16);
    }
    await rt.post("/time", { type: "cancel" });
    stop();
    live = await recording(rt, await recorded);
    check(live.fixture.t0 > 0, `the live clock had run (t0 ${live.fixture.t0})`);
    const squeeze = decode(live.fixture).steps.map((s) => s.xr.right.squeeze);
    check(squeeze.includes(1) && squeeze.at(-1) === 0, "Enter squeezed and let go");
  },
  ["--headless", "--emulate-xr"],
);
if (live) await replays("the live session", live);

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab e2e: all checks passed");
