// vr-lab E2E: the stations driven by injected XR samples, headlessly.
//
// Plays `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab` over the debug
// server (`--headless`, pinned clock, one sample per fixed step) and checks
// `/state.model`, whose props' `pose`/`velocity` are the physics world read
// back each step (rig-local, one step behind):
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
//   4d. The resting, turned brick taken at a corner with the wrist turned
//      keeps its pose (1 mm, 0.5°); handed to the other hand mid-hold, put
//      down and taken again elsewhere, it never jumps.
//   4e. The brick tipped and pressed onto the table's near edge never
//      overlaps the table (no corner inside it, no point of its edge inside
//      the brick) and stops on it; lifted, it follows the hand again.
//   4f. The bottle taken near its neck snaps the neck into the hand; taken
//      by its base it keeps its pose.
//   4g. With the ball blocked at the wall, H switches the drawn hand
//      (`/scene`) from the tracked grip to the grip the ball implies, with a
//      ghost at the tracked grip only while they are over 2 cm apart. The
//      switch never moves the ball.
//   5. A station reset mid-hold drops the hold and respawns the ball at rest.
//   5b. Inventory (station 2): the brick let go in the right holster goes in
//      it, at its pose, and the head turning 90° past the deadzone drags the
//      hips round, the brick riding the holster every step and staying in
//      reach. A hand reaching into the holster pins the hips through a 40°
//      glance; squeezed there, it takes the brick back exactly where it sat
//      (1 mm, 0.5°). The stick (fitting no slot) and the bottle (let go in
//      the full holster) are let go and fall. The stow and turn replays to
//      an identical model.
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
import {
  add, advance, cross, dirToWorld, launch, play, playFixture, qconj, qmul, qrot, rigToWorld, sub, tap,
} from "./xr-scenario.mjs";
import {
  BALL, BOTTLE, FAST_V, FLICK_W, NECK, SLOT, STICK, TABLE, THROW_V, WALL_RIG_X, pitch, project, scenarios, slotAt, yaw,
} from "./vr-lab-scenarios.mjs";

const PORT = 8293;
const WALL_FACE = 0.6; // world x, at the table's end
const MAX_SPEED = 12;
const RADIUS = 0.06;
const FEET = [0, 0, 4];
const BRICK_SIZE = [0.1, 0.065, 0.21];
// The bottle's authored grip: a fist round the neck, its -Z up the bottle.
const NECK_GRIP = { position: [0, BOTTLE.neck, 0], orientation: pitch(90) };

const len = (v) => Math.hypot(...v);
const xyz = (p) => [p.x, p.y, p.z];
const angleDeg = (a, b) =>
  (Math.acos(Math.min(1, a.reduce((s, x, i) => s + x * b[i], 0) / (len(a) * len(b)))) * 180) / Math.PI;
const some = (opt) => (opt?.$ctor === "Option.Some" ? opt.args[0] : undefined);
const fmt = (v) => `[${v.map((x) => x.toFixed(3)).join(", ")}]`;
const quat = (q) => [q.x, q.y, q.z, q.w];
// The turn from `a` to `b`, in degrees (atan2, not acos of a dot product:
// exact near zero for read-back f32 poses).
const turnDeg = (a, b) => {
  const [x, y, z, w] = qmul(b, qconj(a));
  return (2 * Math.atan2(Math.hypot(x, y, z), Math.abs(w)) * 180) / Math.PI;
};

// Poses as { position: [x, y, z], orientation: [x, y, z, w] }.
const asPose = (p) => ({ position: xyz(p.position), orientation: quat(p.orientation) });
const compose = (a, b) => ({ position: add(a.position, qrot(a.orientation, b.position)), orientation: qmul(a.orientation, b.orientation) });
const inverse = (a) => ({ position: qrot(qconj(a.orientation), a.position.map((c) => -c)), orientation: qconj(a.orientation) });
// How far apart two poses are: metres and degrees.
const apart = (a, b) => [len(sub(a.position, b.position)), turnDeg(a.orientation, b.orientation)];
const fmtApart = ([d, a]) => `${(d * 1000).toFixed(3)} mm, ${a.toFixed(3)}°`;
const within = ([d, a], mm, deg) => d < mm / 1000 && a < deg;

// The props in the model (`Props.Ball`, …), and what a hand holds.
const prop = (m, kind) => m.props.find((p) => p.kind.$ctor === `Props.${kind}`);
const heldOf = (m, kind) => some(prop(m, kind).held);
const heldIn = (m, side) => m.props.map((p) => some(p.held)).find((h) => h?.side?.$ctor === `Hold.${side}`);
const poseOf = (m, kind) => asPose(prop(m, kind).pose);
const acceptedOf = (m, kind) => asPose(heldOf(m, kind).accepted);
const worldAt = (m, kind) => rigToWorld(xyz(prop(m, kind).pose.position), FEET);
const worldVel = (m, kind) => dirToWorld(xyz(prop(m, kind).velocity));
const gripOf = (m, side) => asPose(some(some(m.xr)[side].grip));
// Where the hand is on a held prop as it is: the grip its accepted pose implies.
const gripOn = (held) => compose(asPose(held.accepted), inverse(asPose(held.offset)));
// Where the hold wants the prop: the tracked grip carrying its offset.
const wanted = (m, side, held) => compose(gripOf(m, side), asPose(held.offset));

// A posed box's eight corners (rig-local).
const corners = (pose, size) => {
  const out = [];
  for (const x of [-1, 1]) for (const y of [-1, 1]) for (const z of [-1, 1]) {
    out.push(add(xyz(pose.position), qrot(quat(pose.orientation), [x * size[0] / 2, y * size[1] / 2, z * size[2] / 2])));
  }
  return out;
};
const leftmost = (pose) => Math.min(...corners(pose, [STICK.thickness, STICK.thickness, STICK.length]).map((c) => c[0]));
// How deep a point is inside an axis-aligned box ({ min, max }), or 0.
const depthIn = (p, box) => Math.max(0, Math.min(...p.map((v, i) => Math.min(v - box.min[i], box.max[i] - v))));
const TABLE_BOX = { min: [TABLE.x[0], 0, TABLE.z[0]], max: [TABLE.x[1], TABLE.top, TABLE.z[1]] };
// How deep the brick and the table overlap: the brick's corners in the table,
// and the points of the table's near top edge inside the brick.
const overlap = (pose) => {
  const inTable = Math.max(...corners(pose, BRICK_SIZE).map((c) => depthIn(c, TABLE_BOX)));
  const q = quat(pose.orientation);
  const half = BRICK_SIZE.map((s) => s / 2);
  let inBrick = 0;
  for (let x = TABLE.x[0]; x <= TABLE.x[1]; x += 0.005) {
    const local = qrot(qconj(q), sub([x, TABLE.top, TABLE.z[1]], xyz(pose.position)));
    inBrick = Math.max(inBrick, depthIn(local, { min: half.map((h) => -h), max: half }));
  }
  return Math.max(inTable, inBrick);
};

// A rotation's axis × angle (radians), the shorter way round.
const log = (q) => {
  const [x, y, z, w] = q[3] < 0 ? q.map((c) => -c) : q;
  const n = Math.hypot(x, y, z);
  return n < 1e-9 ? [2 * x, 2 * y, 2 * z] : [x, y, z].map((c) => (c * 2 * Math.atan2(n, w)) / n);
};
// The spin carrying rig-local pose `a` to `b` in one step (rad/s).
const spinBetween = (a, b) => log(qmul(quat(b.orientation), qconj(quat(a.orientation)))).map((c) => c * 60);

// `/scene`'s top group: 3 the props (in `Props.all` order), 4/5 the left and
// right hands, 6/7 their ghosts.
const sceneKids = async (rt) => (await rt.get("/scene")).scene.obj.Group;
const IDENTITY = JSON.stringify([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]);
// The first placed transform under a node (`Scene.atPose`'s), if any.
const placed = (node) => {
  if (!node || typeof node !== "object") return undefined;
  if (Array.isArray(node.xform) && JSON.stringify(node.xform) !== IDENTITY) return node.xform;
  for (const child of Object.values(node)) {
    const found = Array.isArray(child) ? child.map(placed).find(Boolean) : placed(child);
    if (found) return found;
  }
  return undefined;
};
const placedAt = (node) => placed(node)?.[3].slice(0, 3);

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
const ballEndOf = (m) => JSON.stringify([prop(m, "Ball").pose, prop(m, "Ball").velocity]);
await run("grab and throw", async (rt) => {
  const m = await play(rt, scenarios["grab-throw"]);
  const reached = m.reached.model;
  check(some(reached.right.hover)?.$ctor === "Props.Ball", "the open hand at the ball hovers it");
  const held = heldOf(m.grabbed.model, "Ball");
  check(held?.side?.$ctor === "Hold.Right", `the squeeze grabs it with the right hand (${JSON.stringify(held?.side)})`);

  const release = some(m.release.model.lastRelease);
  const want = dirToWorld(THROW_V);
  check(release?.kind?.$ctor === "Throw" && release?.prop?.$ctor === "Props.Ball", "opening mid-swing throws");
  const v0 = release ? xyz(release.velocity) : [0, 0, 0];
  check(len(sub(v0, want)) < 0.01 * len(want), `the estimate is the sweep's velocity: ${fmt(v0)} vs ${fmt(want)}`);

  // The world one step after release (the read lags a step, so `after1`).
  const v1 = worldVel(m.after1.model, "Ball");
  const speedErr = Math.abs(len(v1) - len(want)) / len(want);
  const dir = angleDeg(v1, want);
  check(speedErr < 0.05 && dir < 5, `the ball leaves at it: ${fmt(v1)} (${(speedErr * 100).toFixed(1)}%, ${dir.toFixed(1)}°)`);
  const v2 = worldVel(m.after2.model, "Ball");
  check(Math.abs(len(v2) - len(want)) / len(want) < 0.05, `and still has its speed a step later: |v| ${len(v2).toFixed(3)}`);
  // Released from where its body was: a step on, it has moved one step of
  // the throw from there, not two.
  const from = rigToWorld(xyz(heldOf(m.sweep.model, "Ball").accepted.position), FEET);
  const moved = len(sub(worldAt(m.after1.model, "Ball"), from));
  check(Math.abs(moved - len(want) / 60) < 0.2 * (len(want) / 60), `released where it was held: ${(moved * 1000).toFixed(1)} mm a step on (one step: ${((len(want) / 60) * 1000).toFixed(1)})`);
  check(m.landed.model.score === 1, `it lands in the basket (score ${m.landed.model.score})`);
  throwEnd = ballEndOf(m.landed.model);
});

await run("fast throw", async (rt) => {
  const m = await play(rt, scenarios["fast-throw"]);
  const release = some(m.release.model.lastRelease);
  const want = dirToWorld(FAST_V).map((v) => (v * MAX_SPEED) / len(FAST_V));
  const v0 = release ? xyz(release.velocity) : [0, 0, 0];
  check(len(sub(v0, want)) < 0.01 * MAX_SPEED, `a 16 m/s swing estimates the 12 m/s cap: ${fmt(v0)}`);
  // The kinematic ball was moving at 16 m/s; only the release's velocity
  // command brings it to 12.
  const v1 = worldVel(m.after1.model, "Ball");
  check(Math.abs(len(v1) - MAX_SPEED) / MAX_SPEED < 0.05 && angleDeg(v1, want) < 5, `the ball leaves at it: |v| ${len(v1).toFixed(3)} (${angleDeg(v1, want).toFixed(1)}°)`);
});

await run("wall push", async (rt) => {
  const m = await play(rt, scenarios["wall-push"]);
  const pushed = m.pushed.model;
  check(heldOf(pushed, "Ball") !== undefined, "still held with the hand pushed through the wall");
  const ballX = worldAt(pushed, "Ball")[0];
  check(ballX < WALL_FACE - RADIUS && ballX > WALL_FACE - RADIUS - 0.02, `the ball stops at the wall (x ${ballX.toFixed(4)})`);
  const pulled = heldOf(m.pulled.model, "Ball");
  // The hand's rig x -0.4 is world 0.4; the ball follows it out.
  check(pulled && Math.abs(-pulled.accepted.position.x - 0.4) < 0.005, `pulled back, it follows the hand (rig x ${pulled?.accepted.position.x.toFixed(4)})`);
  const release = some(m.release.model.lastRelease);
  check(release?.kind?.$ctor === "Drop", `let go inside the wall: a drop (${release?.kind?.$ctor})`);
  check(release && release.at.x < WALL_FACE - RADIUS, `released outside the wall (x ${release?.at.x.toFixed(4)})`);
  const settled = worldAt(m.settled.model, "Ball");
  check(settled[0] < WALL_FACE - RADIUS + 0.005, `and stays outside it (x ${settled[0].toFixed(4)})`);
  check(Math.abs(settled[1] - BALL[1]) < 0.005, `resting on the table (y ${settled[1].toFixed(4)})`);
});

await run("break grip", async (rt) => {
  const m = await play(rt, scenarios["break-grip"]);
  const broke = m.broke.model;
  const release = some(broke.lastRelease);
  check(release?.kind?.$ctor === "Break", `dragged 25 cm past the ball, the grip breaks (${release?.kind?.$ctor})`);
  check(release && len(xyz(release.velocity)) === 0, "dropped with no velocity");
  check(release && release.at.x < WALL_FACE - RADIUS, `where the wall stopped it (x ${release?.at.x.toFixed(4)})`);
  check(heldOf(broke, "Ball") === undefined && broke.right.armed === false, "the hand is disarmed");
  check(heldOf(m.returned.model, "Ball") === undefined, "still squeezing, back at the ball, it doesn't regrab");
});

await run("stick against the wall", async (rt) => {
  let deepest = Infinity;
  let deepestStep;
  const m = await play(rt, scenarios["stick-wall"], async ({ step, state }) => {
    const stick = heldOf(state.model, "Stick");
    if (stick && leftmost(stick.accepted) < deepest) [deepest, deepestStep] = [leftmost(stick.accepted), step];
  });
  const took = heldOf(m.tookStick.model, "Stick");
  check(took?.side?.$ctor === "Hold.Right" && heldOf(m.tookStick.model, "Ball") === undefined, "squeezed at its near end, the stick is held (not the ball)");
  check(deepest >= WALL_RIG_X - 0.002, `it never passes the wall plane (deepest rig x ${deepest.toFixed(4)}, step ${deepestStep})`);
  const pressed = heldOf(m.pressed.model, "Stick");
  const far = pressed && leftmost(pressed.accepted);
  check(far !== undefined && far < WALL_RIG_X + 0.01, `turned a quarter turn, it stops at the wall (rig x ${far?.toFixed(4)})`);
  const lag = pressed && turnDeg(quat(pressed.accepted.orientation), yaw(90));
  check(lag > 45, `well short of the hand's turn (${lag?.toFixed(1)}° behind)`);
  const back = heldOf(m.turnedBack.model, "Stick");
  // Where it was taken from the table, lifted with the hand.
  const home = sub(xyz(took.accepted.position), [0, 0.95 - 1.2, 0]);
  const off = back && len(sub(xyz(back.accepted.position), home));
  const turn = back && turnDeg(quat(back.accepted.orientation), quat(took.accepted.orientation));
  check(off < 1e-3 && turn < 0.1, `turned back, it follows the hand (${off?.toFixed(5)} m, ${turn?.toFixed(3)}°)`);
  check(heldOf(m.letGo.model, "Stick") === undefined, "let go, it is free");
  const ball = worldAt(m.letGo.model, "Ball");
  check(len(sub(ball, rigToWorld(BALL, FEET))) < 0.005, `the ball sat still throughout (${fmt(ball)})`);
});

let stickEnd;
const stickEndOf = (m) => JSON.stringify([prop(m.rest.model, "Stick").pose, prop(m.rest.model, "Ball").pose, m.rest.model.lastRelease]);
await run("stick shove and throw", async (rt) => {
  const m = await play(rt, scenarios["stick-throw"]);
  check(heldOf(m.tookStick.model, "Stick") !== undefined, "the stick is taken");
  const ballBefore = worldAt(m.low.model, "Ball");
  const swept = m.swept.model;
  const ballAfter = worldAt(swept, "Ball");
  check(heldOf(swept, "Stick") !== undefined, "still held after the sweep");
  // Rig +X is world -X: the sweep turns the stick's far end that way.
  check(ballBefore[0] - ballAfter[0] > 0.1, `the held stick shoves the ball away: ${fmt(ballBefore)} → ${fmt(ballAfter)}`);

  const release = some(m.release.model.lastRelease);
  const want = dirToWorld(FLICK_W);
  check(release?.kind?.$ctor === "Throw" && release?.prop?.$ctor === "Props.Stick", `opening mid-flick throws it (${release?.kind?.$ctor})`);
  const w0 = release ? xyz(release.spin) : [0, 0, 0];
  check(len(sub(w0, want)) < 0.01 * len(want), `the estimate is the wrist's spin: ${fmt(w0)} vs ${fmt(want)}`);
  // Read back a step behind: after2 → after3 is the second step after release.
  const measured = spinBetween(prop(m.after2.model, "Stick").pose, prop(m.after3.model, "Stick").pose);
  const err = Math.abs(len(measured) - len(FLICK_W)) / len(FLICK_W);
  const dir = angleDeg(measured, FLICK_W);
  check(err < 0.05 && dir < 5, `two steps on it spins at it: ${fmt(measured)} rad/s (${(err * 100).toFixed(1)}%, ${dir.toFixed(1)}°)`);

  const [landed, rest] = [prop(m.landed.model, "Stick").pose, prop(m.rest.model, "Stick").pose];
  const still = len(sub(xyz(landed.position), xyz(rest.position))) + turnDeg(quat(landed.orientation), quat(rest.orientation));
  check(still < 1e-4, `it comes to rest (${still.toExponential(1)})`);
  // Drawn: `Scene.atPose` of the world pose, columns left, up, forward
  // (rig -Z) and position. The rig faces world +Z: a half turn about +Y.
  const xform = (await sceneKids(rt))[3].obj.Group[1].xform;
  const toWorld = (v) => [-v[0], v[1], -v[2]];
  const q = quat(rest.orientation);
  const drawn = xform.map((c) => c.slice(0, 3));
  const [up, forward] = [toWorld(qrot(q, [0, 1, 0])), toWorld(qrot(q, [0, 0, -1]))];
  const read = [cross(up, forward), up, forward, rigToWorld(xyz(rest.position), FEET)];
  const off = Math.max(...drawn.map((c, i) => len(sub(c, read[i]))));
  check(off < 1e-4, `at rest it is drawn at its read-back pose (${off.toExponential(1)})`);
  stickEnd = stickEndOf(m);
});

await run("brick regrab and hand-off", async (rt) => {
  let worstStep = 0;
  let prev;
  const m = await play(rt, scenarios["brick-regrab"], async ({ state }) => {
    // While held, the brick never jumps more than the hand moves in a step.
    const h = heldOf(state.model, "Brick");
    if (h && prev) worstStep = Math.max(worstStep, len(sub(xyz(h.accepted.position), xyz(prev.position))));
    prev = h?.accepted;
  });
  const rested = poseOf(m.reachedBrick.model, "Brick");
  const took = heldOf(m.tookBrick.model, "Brick");
  const grabGap = apart(acceptedOf(m.tookBrick.model, "Brick"), rested);
  check(took?.side?.$ctor === "Hold.Right" && within(grabGap, 1, 0.5), `taken at a corner, turned, it keeps its pose (${fmtApart(grabGap)})`);
  const bodyGap = apart(poseOf(m.holdingBrick.model, "Brick"), rested);
  check(within(bodyGap, 1, 0.5), `and its body stays there (${fmtApart(bodyGap)})`);
  const liftedGap = apart(acceptedOf(m.lifted.model, "Brick"), wanted(m.lifted.model, "right", heldOf(m.lifted.model, "Brick")));
  check(within(liftedGap, 0.1, 0.01), `lifted and turned, it keeps its place in the hand (${fmtApart(liftedGap)})`);

  const handOff = apart(acceptedOf(m.handedOff.model, "Brick"), acceptedOf(m.beforeHandOff.model, "Brick"));
  check(heldOf(m.handedOff.model, "Brick")?.side?.$ctor === "Hold.Left" && heldIn(m.handedOff.model, "Right") === undefined, "the left hand's squeeze takes it from the right");
  check(within(handOff, 0.1, 0.01), `without moving it (${fmtApart(handOff)})`);
  const opened = apart(acceptedOf(m.rightOpen.model, "Brick"), acceptedOf(m.handedOff.model, "Brick"));
  check(some(m.rightOpen.model.lastRelease) === undefined && within(opened, 0.1, 0.01), `the right opening lets nothing go (${fmtApart(opened)})`);
  const lowered = heldOf(m.lowered.model, "Brick");
  const follows = lowered && apart(asPose(lowered.accepted), wanted(m.lowered.model, "left", lowered));
  check(follows && within(follows, 0.1, 0.01), `it follows the left hand (${follows && fmtApart(follows)})`);
  const put = some(m.putDown.model.lastRelease);
  check(put?.prop?.$ctor === "Props.Brick" && put?.kind?.$ctor === "Drop", `put down: a drop (${put?.kind?.$ctor})`);

  const settled = poseOf(m.reachedAgain.model, "Brick");
  const again = apart(acceptedOf(m.tookAgain.model, "Brick"), settled);
  check(heldOf(m.tookAgain.model, "Brick")?.side?.$ctor === "Hold.Right" && within(again, 1, 0.5), `taken again by the right elsewhere, it keeps its pose (${fmtApart(again)})`);
  const againBody = apart(poseOf(m.holdingAgain.model, "Brick"), settled);
  check(within(againBody, 1, 0.5), `and its body stays there (${fmtApart(againBody)})`);
  check(worstStep < 0.03, `held, it never jumps (largest step ${(worstStep * 1000).toFixed(1)} mm)`);
});

await run("brick pressed onto the table's edge", async (rt) => {
  let deepest = 0;
  let deepestStep;
  // From the lift on: resting, a body sits a millimetre or so into the table.
  let lifted = false;
  const m = await play(rt, scenarios["brick-edge"], async ({ step, mark, state }) => {
    const h = heldOf(state.model, "Brick");
    const d = lifted && h ? overlap(h.accepted) : 0;
    lifted ||= mark === "above";
    if (d > deepest) [deepest, deepestStep] = [d, step];
  });
  check(deepest < 0.001, `it never overlaps the table (deepest ${(deepest * 1000).toFixed(2)} mm${deepestStep ? `, step ${deepestStep}` : ""})`);
  const pressed = heldOf(m.held.model, "Brick");
  const short = pressed && len(sub(xyz(pressed.accepted.position), wanted(m.held.model, "right", pressed).position));
  check(short > 0.05, `pressed 4 cm below the top, it is held back (${(short * 100).toFixed(1)} cm short of the hand)`);
  const lowest = pressed && Math.min(...corners(pressed.accepted, BRICK_SIZE).map((c) => c[1]));
  check(lowest !== undefined && lowest > TABLE.top - 0.001 && lowest < TABLE.top + 0.01, `resting on the top (lowest corner y ${lowest?.toFixed(4)})`);
  const raised = heldOf(m.lifted.model, "Brick");
  const follows = raised && apart(asPose(raised.accepted), wanted(m.lifted.model, "right", raised));
  check(follows && within(follows, 1, 0.5), `lifted, it follows the hand again (${follows && fmtApart(follows)})`);
});

await run("bottle neck snap", async (rt) => {
  const m = await play(rt, scenarios["bottle-neck"]);
  const before = poseOf(m.atNeck.model, "Bottle");
  const off = len(sub(compose(before, NECK_GRIP).position, gripOf(m.atNeck.model, "right").position));
  const held = heldOf(m.liftedNeck.model, "Bottle");
  const offset = held && apart(asPose(held.offset), inverse(NECK_GRIP));
  check(held && within(offset, 0.1, 0.01), `taken ${(off * 100).toFixed(1)} cm off the neck, it snaps to it (${offset && fmtApart(offset)} from the authored grip)`);
  const inHand = held && apart(gripOn(held), gripOf(m.liftedNeck.model, "right"));
  check(inHand && within(inHand, 1, 0.5), `lifted, the neck is in the hand (${inHand && fmtApart(inHand)})`);
});

await run("bottle base, no snap", async (rt) => {
  const m = await play(rt, scenarios["bottle-base"]);
  const before = poseOf(m.atBase.model, "Bottle");
  const kept = apart(acceptedOf(m.holdingBase.model, "Bottle"), before);
  check(within(kept, 1, 0.5), `taken by its base, it keeps its pose (${fmtApart(kept)})`);
  const held = heldOf(m.liftedBase.model, "Bottle");
  const fromNeck = held && apart(asPose(held.offset), inverse(NECK_GRIP))[0];
  check(fromNeck > 0.1, `held where it was taken, not at the neck (${(fromNeck * 100).toFixed(1)} cm off it)`);
  const follows = held && apart(asPose(held.accepted), wanted(m.liftedBase.model, "right", held));
  check(follows && within(follows, 1, 0.5), `and lifted with the hand (${follows && fmtApart(follows)})`);
});

await run("hand presentation at the wall", async (rt) => {
  const snaps = {};
  await play(rt, scenarios["hand-view"], async ({ mark, state }) => {
    if (mark) snaps[mark] = { model: state.model, kids: await sceneKids(rt) };
  });
  const view = (mark) => snaps[mark].model.handView.$ctor;
  const tracked = (mark) => rigToWorld(gripOf(snaps[mark].model, "right").position, FEET);
  const implied = (mark) => rigToWorld(gripOn(heldOf(snaps[mark].model, "Ball")).position, FEET);
  const drawn = (mark) => placedAt(snaps[mark].kids[5]);
  const ghost = (mark) => placedAt(snaps[mark].kids[7]);
  const gap = (a, b) => (a && b ? len(sub(a, b)) : Infinity);

  check(view("blockedTracked") === "Tracked", "Tracked by default");
  const sep = gap(tracked("blockedTracked"), implied("blockedTracked"));
  check(sep > 0.15, `the ball is blocked at the wall, the hand through it (${(sep * 100).toFixed(1)} cm apart)`);
  check(gap(drawn("blockedTracked"), tracked("blockedTracked")) < 1e-4 && ghost("blockedTracked") === undefined, "Tracked: drawn at the tracked grip, no ghost");
  check(view("blockedAccepted") === "Accepted", "H switches to Accepted");
  check(gap(drawn("blockedAccepted"), implied("blockedAccepted")) < 1e-4, `Accepted: drawn at the grip the ball implies (${fmt(drawn("blockedAccepted"))})`);
  check(gap(ghost("blockedAccepted"), tracked("blockedAccepted")) < 1e-4, "with a ghost at the tracked grip");
  const same = JSON.stringify(heldOf(snaps.blockedAccepted.model, "Ball").accepted) === JSON.stringify(heldOf(snaps.blockedTracked.model, "Ball").accepted);
  check(same, "switching never moves the ball");
  const near = gap(tracked("nearWall"), implied("nearWall"));
  check(near < 0.02 && ghost("nearWall") === undefined && gap(drawn("nearWall"), implied("nearWall")) < 1e-4, `${(near * 1000).toFixed(1)} mm apart: no ghost`);
  check(gap(drawn("out"), tracked("out")) < 1e-3 && ghost("out") === undefined, "out of the wall, drawn at the hand, no ghost");
  check(gap(ghost("blockedAgain"), tracked("blockedAgain")) < 1e-4, "pushed through again, the ghost is back");
  check(view("trackedAgain") === "Tracked" && gap(drawn("trackedAgain"), tracked("trackedAgain")) < 1e-4 && ghost("trackedAgain") === undefined, "H again: Tracked, no ghost");
});

await run("tracking loss", async (rt) => {
  const m = await play(rt, scenarios["tracking-loss"]);
  const swinging = heldOf(m.swinging.model, "Ball");
  const lost = heldOf(m.lost.model, "Ball");
  check(lost && JSON.stringify(lost.accepted) === JSON.stringify(swinging.accepted), "the held ball freezes while the hand is lost");
  check(lost && lost.history.length === 0, "and forgets its history");
  const release = some(m.release.model.lastRelease);
  check(release?.kind?.$ctor === "Drop" && len(xyz(release.velocity)) === 0, `opened on return: a drop, not a throw (${release?.kind?.$ctor})`);
});

await run("station reset", async (rt) => {
  const s = scenarios["grab-throw"];
  await play(rt, { ...s, steps: s.steps.slice(0, 4) });
  const before = (await rt.get("/state")).model;
  check(heldOf(before, "Ball") !== undefined, "holding the ball");
  await tap(rt, "1");
  const after = (await advance(rt)).model;
  check(heldOf(after, "Ball") === undefined && after.gen === before.gen + 1, "1 drops the hold and respawns the ball");
  const spawn = worldAt(after, "Ball");
  check(len(sub(spawn, rigToWorld(BALL, FEET))) < 1e-3, `at rest at its spawn (${fmt(spawn)})`);
  check(len(xyz(prop(after, "Ball").velocity)) < 1e-6, "with no velocity");
  check(after.right.armed === false && after.lastRelease.$ctor === "Option.None", "the still-squeezing hand is disarmed and the history gone");
});

// --- inventory (station 2): rig-local is the world moved to its feet ---

const deg = (rad) => (rad * 180) / Math.PI;
const slotOf = (m, kind) => some(prop(m, kind).slot);
// Where the model's hips put a slot (rig-local).
const hipsPose = (m) => ({ position: xyz(m.hips.root), orientation: yaw(deg(m.hips.yaw)) });
const slotPose = (m, name) => compose(hipsPose(m), { position: SLOT[name].offset, orientation: [0, 0, 0, 1] });
const fmtDeg = (rad) => `${deg(rad).toFixed(2)}°`;

let stowEnd;
await run("inventory: stow and turn", async (rt) => {
  // Every step a prop is slotted, it sits exactly at its slot.
  let worst = 0;
  const m = await play(rt, scenarios["inventory-stow-turn"], async ({ state }) => {
    if (slotOf(state.model, "Brick") === SLOT.rightHolster.id) {
      worst = Math.max(worst, apart(poseOf(state.model, "Brick"), slotPose(state.model, "rightHolster"))[0]);
    }
  });
  check(m.atHolster.model.station === 2 && heldOf(m.atHolster.model, "Brick") !== undefined, "at the inventory, the brick is carried to the right holster");
  const stored = m.stored.model;
  check(slotOf(stored, "Brick") === SLOT.rightHolster.id && heldOf(stored, "Brick") === undefined, "let go there, it goes in the holster");
  const seat = apart(poseOf(stored, "Brick"), { position: slotAt("rightHolster"), orientation: [0, 0, 0, 1] });
  check(within(seat, 0.1, 0.01), `at the holster's pose (${fmtApart(seat)})`);
  check(some(stored.lastRelease) === undefined, "nothing is let go");
  const lag = m.turning.model.hips.yaw;
  check(deg(lag) > 30 && deg(lag) < 75, `as the head turns 90°, the hips trail it (${fmtDeg(lag)})`);
  const turned = m.turned.model;
  check(Math.abs(turned.hips.yaw - Math.PI / 2) < 0.01, `held there, they settle on it (${fmtDeg(turned.hips.yaw)})`);
  check(slotOf(turned, "Brick") === SLOT.rightHolster.id && worst < 1e-6, `the brick rides the holster round (off it by at most ${worst.toExponential(1)} m)`);
  const round = len(sub(poseOf(turned, "Brick").position, slotAt("rightHolster", 90)));
  check(round < 0.001, `a quarter turn round the pelvis (${(round * 1000).toFixed(2)} mm from where the turned holster is)`);
  // From the right shoulder, which turns with the torso: 18 cm out, 14 cm under the neck.
  const shoulder = (x) => add(NECK.at, qrot(yaw(deg(x.hips.yaw)), [0.18, -0.14, 0]));
  const reach = [stored, turned].map((x) => len(sub(poseOf(x, "Brick").position, shoulder(x))));
  check(Math.abs(reach[0] - reach[1]) < 0.001 && reach[1] < 0.6, `and stays in reach: ${reach.map((r) => (r * 100).toFixed(1)).join(" → ")} cm from the right shoulder`);
  stowEnd = JSON.stringify(turned);
});

await run("inventory: reach pins the hips; retrieve", async (rt) => {
  const yaws = [];
  let glancing = false;
  const m = await play(rt, scenarios["inventory-reach"], async ({ mark, state }) => {
    if (glancing) yaws.push(state.model.hips.yaw);
    if (mark === "inZone") glancing = true;
    if (mark === "glanced") glancing = false;
  });
  const inZone = m.inZone.model;
  check(inZone.hips.pinned, "the hand at the holster pins the hips");
  const held = yaws.every((y) => y === inZone.hips.yaw);
  const head = quat(some(some(m.glanced.model.xr).head).orientation);
  check(held && yaws.length === 30, `the head glancing ${turnDeg([0, 0, 0, 1], head).toFixed(1)}° aside, the hips hold (${yaws.length} steps at ${fmtDeg(inZone.hips.yaw)})`);
  const before = poseOf(m.beforeRetrieve.model, "Brick");
  const still = apart(before, poseOf(inZone, "Brick"));
  check(within(still, 0.001, 0.0001), `and the holster with them (${fmtApart(still)})`);
  const took = m.retrieved.model;
  check(heldOf(took, "Brick")?.side?.$ctor === "Hold.Right" && slotOf(took, "Brick") === undefined, "squeezed there, the hand takes the brick back");
  const pop = apart(acceptedOf(took, "Brick"), before);
  check(within(pop, 1, 0.5), `exactly where it sat (${fmtApart(pop)})`);
  const body = apart(poseOf(m.holding.model, "Brick"), before);
  check(within(body, 1, 0.5), `and its body appears there (${fmtApart(body)})`);
  const lifted = m.lifted.model;
  const follows = apart(acceptedOf(lifted, "Brick"), wanted(lifted, "right", heldOf(lifted, "Brick")));
  check(within(follows, 1, 0.5), `lifted out, it follows the hand (${fmtApart(follows)})`);
  check(!lifted.hips.pinned && lifted.hips.yaw < inZone.hips.yaw, `the hand gone, the hips follow the glance again (${fmtDeg(lifted.hips.yaw)})`);
});

await run("inventory: refusals", async (rt) => {
  const m = await play(rt, scenarios["inventory-refuse"]);
  const stick = m.stickLetGo.model;
  const stickRelease = some(stick.lastRelease);
  check(slotOf(stick, "Stick") === undefined && stickRelease?.prop?.$ctor === "Props.Stick", `the stick fits no slot: let go in the holster, it is let go (${stickRelease?.kind?.$ctor})`);
  const fell = poseOf(m.stickLetGo.model, "Stick").position[1] - poseOf(m.stickDropped.model, "Stick").position[1];
  check(fell > 0.2, `and falls (${(fell * 100).toFixed(1)} cm)`);
  check(slotOf(m.stored.model, "Brick") === SLOT.rightHolster.id, "the brick goes in the right holster");
  const bottle = m.bottleLetGo.model;
  const bottleRelease = some(bottle.lastRelease);
  check(slotOf(bottle, "Bottle") === undefined && bottleRelease?.prop?.$ctor === "Props.Bottle", `the bottle let go in the full holster is let go (${bottleRelease?.kind?.$ctor})`);
  const dropped = m.bottleDropped.model;
  const bottleFell = poseOf(bottle, "Bottle").position[1] - poseOf(dropped, "Bottle").position[1];
  check(bottleFell > 0.2 && slotOf(dropped, "Brick") === SLOT.rightHolster.id, `it falls (${(bottleFell * 100).toFixed(1)} cm), and the brick stays in`);
});

await run("inventory replay", async (rt) => {
  const m = await play(rt, scenarios["inventory-stow-turn"]);
  check(stowEnd !== undefined && stowEnd === JSON.stringify(m.turned.model), "the same stow and turn in a fresh runtime ends on an identical model");
});

await run("replay", async (rt) => {
  const m = await play(rt, scenarios["grab-throw"]);
  const b = ballEndOf(m.landed.model);
  check(throwEnd !== undefined && throwEnd === b, `the same throw in a fresh runtime ends identically (${b})`);
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
    check((await rt.get("/state")).model.gen === 3, "reset twice before recording");
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
