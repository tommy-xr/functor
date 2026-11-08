// vr-lab E2E: the grab table driven by injected XR samples, headlessly.
//
// Plays `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab` over the debug
// server (`--headless`, pinned clock, one sample per fixed step) and checks
// `/state.model`, whose `ballPos`/`ballVel` are the physics world read back
// each step (one step behind):
//
//   1. A steady sweep, released mid-swing, launches the ball at the sweep's
//      velocity (within 5% and 5 degrees), still holding its speed a step
//      later, and it lands in the basket.
//   2. A hand pushed through the wall leaves the ball at the wall; the ball
//      follows it back out; let go there, the ball drops outside the wall.
//   3. Dragged farther, the grip breaks: dropped in place, no velocity, and the
//      still-squeezing hand can't take it back.
//   4. Losing the hand mid-swing and opening it on return never throws.
//   5. A station reset mid-hold drops the hold and respawns the ball at rest.
//   6. The same throw in two fresh runtimes ends bit-identically.
//
// Run (needs the CLI; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { advance, dirToWorld, launch, play, tap } from "./xr-scenario.mjs";
import { BALL, THROW_V, project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8193;
const WALL_FACE = 0.7; // world x
const RADIUS = 0.06;

const len = (v) => Math.hypot(...v);
const sub = (a, b) => a.map((x, i) => x - b[i]);
const xyz = (p) => [p.x, p.y, p.z];
const angleDeg = (a, b) =>
  (Math.acos(Math.min(1, a.reduce((s, x, i) => s + x * b[i], 0) / (len(a) * len(b)))) * 180) / Math.PI;
const some = (opt) => (opt?.$ctor === "Option.Some" ? opt.args[0] : undefined);
const fmt = (v) => `[${v.map((x) => x.toFixed(3)).join(", ")}]`;

async function run(name, body) {
  console.log(`\n▸ ${name}`);
  const rt = await launch(project, PORT, ["--headless"]);
  try {
    await body(rt);
  } catch (e) {
    failures.push(`${name}: ${e}`);
    console.error(e, `\n--- runtime log ---\n${rt.log().slice(-3000)}`);
  } finally {
    rt.kill();
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

await run("wall push", async (rt) => {
  const m = await play(rt, scenarios["wall-push"]);
  const pushed = m.pushed.model;
  const accepted = some(pushed.held)?.accepted;
  check(accepted !== undefined, "still held with the hand 15 cm inside the wall");
  const ballX = pushed.ballPos.x;
  check(ballX < WALL_FACE - RADIUS && ballX > WALL_FACE - RADIUS - 0.02, `the ball stops at the wall (x ${ballX.toFixed(4)})`);
  const pulled = some(m.pulled.model.held);
  // The hand's rig x -0.4 is world 0.4; the ball follows it out.
  check(pulled && Math.abs(-pulled.accepted.x - 0.4) < 0.005, `pulled back, it follows the hand (rig x ${pulled?.accepted.x.toFixed(4)})`);
  const release = some(m.release.model.lastRelease);
  check(release?.kind?.$ctor === "Drop", `let go inside the wall: a drop (${release?.kind?.$ctor})`);
  check(release && release.at.x < WALL_FACE - RADIUS, `released outside the wall (x ${release?.at.x.toFixed(4)})`);
  const settled = m.settled.model.ballPos;
  check(settled.x < WALL_FACE - RADIUS + 0.005, `and stays outside it (x ${settled.x.toFixed(4)}, y ${settled.y.toFixed(3)})`);
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

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab e2e: all checks passed");
