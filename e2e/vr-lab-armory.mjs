// vr-lab armory E2E (station 3): the pistol driven by injected XR samples,
// headlessly.
//
// Plays the armory scenarios in `e2e/vr-lab-scenarios.mjs` against
// `examples/vr-lab` (`--headless`, pinned clock, one sample per fixed step)
// and checks `/state.model.gun`:
//
//   1. Taken off the bench, aimed at a target and fired: one hit, within
//      1 cm of where the injected hand pose puts it (the bore 5 cm above the
//      hand's -Z axis, the muzzle 15 cm along it), and the score is 1.
//      Holding the trigger down 30 steps fires nothing more. Aimed off the
//      targets, a shot strikes the backstop: no hit, no score.
//   2. Pushed with the hand through the wall, the pistol stops at it and the
//      trigger only clicks: no shot. The hand is drawn (`/scene`) on the
//      pistol's grip at the wall, its ghost at the tracked hand. Drawn back
//      clear, it fires, from the muzzle of the pose the hold accepted.
//   2b. Clear of the wall, the left hand racks the slide 8 cm past its
//      travel: drawn on the slide at its end, its ghost at the tracked hand.
//   3. Holstered at the right thigh and drawn with the trigger already held:
//      it never jumps (at most a 2 cm, 8.6° settling step each step), and
//      fires nothing until the trigger is let out and pulled.
//   4. The aimed shot replays in a fresh runtime to an identical model.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-armory.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { add, fmt, len, placedAt, play, qrot, quat, record, rigToWorld, session, some, sub, turnOf, xyz } from "./xr-scenario.mjs";
import { ARMORY_WALL_X, GRIP_IN_PISTOL, PISTOL_FIST, PISTOL_HALF, SLIDE, TARGETS, TARGET_SIZE, armory, muzzleOf, project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8298;
const SNAP_STEP = 0.02;
const SNAP_TURN = 0.15;

const pistol = (m) => m.props.find((p) => p.kind.$ctor === "Props.Pistol");
// Where the pistol is: held, its accepted pose; else as read back.
const poseOf = (m) => {
  const p = pistol(m);
  const pose = some(p.held)?.accepted ?? p.pose;
  return { position: xyz(pose.position), orientation: quat(pose.orientation) };
};
// A point in the frame of a pistol at `pose`; rig-local to world.
const inPistol = (pose, local) => add(pose.position, qrot(pose.orientation, local));
const toWorld = (p) => rigToWorld(p, armory.feet, armory.yaw);
// The shot from a pistol at `pose`, from where its grip puts the hand.
const boreOf = (pose) => {
  const hand = inPistol(pose, GRIP_IN_PISTOL);
  return muzzleOf(hand, pose.orientation);
};
const lastShot = (m) => some(m.gun.last);

const run = (name, body) => session(name, project, PORT, body);

let fireEnd;
await run("aim and fire", async (rt) => {
  const { marks, at } = await record(rt, scenarios["armory-fire"]);
  const fired = marks.fired.model;
  check(fired.gun.fired === 1 && fired.score === 1, `one shot, one point (fired ${fired.gun.fired}, score ${fired.score})`);
  const hit = fired.gun.hits[0];
  check(fired.gun.hits.length === 1 && hit?.tag === "target-2", `one hit, on the right target (${fired.gun.hits.map((h) => h.tag)})`);
  // Analytic: the injected hand at the pull, its shot to the targets' plane.
  const hand = at("fired").right;
  const { from, dir } = muzzleOf(hand.at, hand.orientation ?? [0, 0, 0, 1]);
  const want = add(from, dir.map((c) => (c * (TARGETS[2][2] - from[2])) / dir[2]));
  const got = hit ? xyz(hit.point) : [NaN, NaN, NaN];
  check(len(sub(got, want)) < 0.01, `the hit at ${fmt(got)}, ${(len(sub(got, want)) * 1000).toFixed(2)} mm from where the hand aimed it (${fmt(want)})`);
  check(
    Math.abs(got[0] - TARGETS[2][0]) < TARGET_SIZE / 2 && Math.abs(got[1] - TARGETS[2][1]) < TARGET_SIZE / 2,
    `on the target's face (${fmt(TARGETS[2])})`,
  );
  const shot = lastShot(fired);
  check(len(sub(xyz(shot.from), from)) < 0.001, `from the muzzle (${fmt(xyz(shot.from))})`);

  const held = marks.held.model;
  check(held.gun.fired === 1 && held.score === 1, `the trigger held 30 steps: still one shot (${held.gun.fired})`);

  const missed = marks.missed.model;
  const miss = some(lastShot(missed).impact);
  check(missed.gun.fired === 2, `aimed off the targets, a second shot (${missed.gun.fired})`);
  check(missed.gun.hits.length === 1 && missed.score === 1, `no hit, no point (${missed.gun.hits.length} hits, score ${missed.score})`);
  check(miss?.tag === "backstop" && !miss.target, `it struck the backstop (${miss?.tag})`);
  fireEnd = JSON.stringify((await rt.get("/state")).model);
});

await run("barrel into the wall", async (rt) => {
  const { marks, scenes, at } = await record(rt, scenarios["armory-wall"], { scenes: true });
  const pushed = marks.pushed.model;
  const pose = poseOf(pushed);
  // Pointing along -X: its front face is `PISTOL_HALF[2]` ahead of its centre.
  const face = pose.position[0] - PISTOL_HALF[2];
  check(face >= ARMORY_WALL_X - 0.001 && face < ARMORY_WALL_X + 0.01, `the pistol stops at the wall (its muzzle face at x ${face.toFixed(4)})`);
  // The tracked hand, pointing along -X, would put the muzzle 15 cm ahead of it.
  const tracked = at("pushed").right.at[0] - 0.15;
  check(tracked < face - 0.05, `the tracked hand would put it ${((face - tracked) * 100).toFixed(1)} cm into the wall`);
  // `/scene`'s top group: 5 the right hand, 7 its ghost. Drawn (by default)
  // on the grip where the wall stopped the pistol; the ghost at the hand.
  check(pushed.handView.$ctor === "Accepted", "the hands drawn Accepted by default");
  const kids = scenes.pushed.scene.obj.Group;
  // The hand's root on the grip: the fist's placement past it.
  const grip = toWorld(inPistol(pose, add(GRIP_IN_PISTOL, PISTOL_FIST.position)));
  const drawn = placedAt(kids[5]);
  const ghost = placedAt(kids[7]);
  const hand = toWorld(at("pushed").sent.right.at);
  check(drawn && len(sub(drawn, grip)) < 1e-4, `the hand drawn on the pistol's grip at the wall (${drawn && (len(sub(drawn, grip)) * 1000).toFixed(3)} mm), ${(len(sub(grip, hand)) * 100).toFixed(1)} cm short of the tracked hand`);
  check(ghost && len(sub(ghost, hand)) < 1e-4, `its ghost at the tracked hand (${ghost && (len(sub(ghost, hand)) * 1000).toFixed(3)} mm)`);
  const clicked = marks.clicked.model;
  check(clicked.gun.fired === 0 && clicked.gun.refused === 1 && !lastShot(clicked), `the pull is refused (fired ${clicked.gun.fired}, refused ${clicked.gun.refused})`);
  const fired = marks.fired.model;
  const shot = lastShot(fired);
  const struck = some(shot?.impact);
  check(fired.gun.fired === 1, `drawn back clear of it, the pull fires (${fired.gun.fired})`);
  check(struck?.tag === "armory-wall" && Math.abs(struck.point.x - ARMORY_WALL_X) < 0.001, `into the wall (${struck?.tag} at x ${struck?.point.x.toFixed(4)})`);
  // From the accepted pose, not the tracked hand's (they agree once clear).
  const want = boreOf(poseOf(fired)).from;
  check(shot && len(sub(xyz(shot.from), want)) < 0.0001, `from the accepted pose's muzzle (${fmt(want)})`);
});

await run("racking hand drawn on the slide", async (rt) => {
  const { marks, scenes, at } = await record(rt, scenarios["hand-view-guns"], { scenes: true });
  const m = marks.rackHeld.model;
  check(some(m.gun.slide.hand)?.$ctor === "Hold.Left" && m.gun.slide.pull === SLIDE.travel, `the left hand on the slide, back its full travel (${m.gun.slide.pull})`);
  // `/scene`: 4 the left hand, 6 its ghost.
  const kids = scenes.rackHeld.scene.obj.Group;
  const pose = poseOf(m);
  const end = toWorld(inPistol(pose, add(SLIDE.at, [0, 0, SLIDE.travel])));
  const hand = toWorld(at("rackHeld").left.at);
  const drawn = placedAt(kids[4]);
  const ghost = placedAt(kids[6]);
  // At its fitted placement on the slide (`/state`'s `shown`), back with it: within 6 cm of the slide's end.
  const shownAt = m.shown.find((h) => h.side.$ctor === "Hold.Left")?.at.position;
  const shownW = shownAt && toWorld([shownAt.x, shownAt.y, shownAt.z]);
  check(drawn && shownW && len(sub(drawn, shownW)) < 1e-4 && len(sub(drawn, end)) < 0.06,
    `drawn at its fitted placement on the slide, ${drawn && (len(sub(drawn, end)) * 100).toFixed(1)} cm from its end, ${(len(sub(end, hand)) * 100).toFixed(1)} cm short of the tracked hand`);
  check(ghost && len(sub(ghost, hand)) < 1e-4, `its ghost at the tracked hand (${ghost && (len(sub(ghost, hand)) * 1000).toFixed(3)} mm)`);
});

await run("holster and draw", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["armory-holster"]);
  const holstered = marks.holstered.model;
  check(some(pistol(holstered).slot) === 2, `holstered in the right holster (${JSON.stringify(pistol(holstered).slot)})`);
  // From holstered to held out: each step it moves at most as the hand did
  // plus a settling step.
  const from = steps.findIndex((s) => s.state.frame === marks.reachedHolster.frame);
  const to = steps.findIndex((s) => s.state.frame === marks.drawnHeld.frame);
  let worst = [0, 0];
  for (let i = from + 1; i <= to; i++) {
    const [a, b] = [poseOf(steps[i - 1].state.model), poseOf(steps[i].state.model)];
    const moved = len(sub(b.position, a.position)) - len(sub(steps[i].right.at, steps[i - 1].right.at));
    const turned = turnOf(a.orientation, b.orientation);
    worst = [Math.max(worst[0], moved), Math.max(worst[1], turned)];
  }
  check(worst[0] <= SNAP_STEP + 1e-4 && worst[1] <= SNAP_TURN + 1e-4, `drawn without a jump (worst step ${(worst[0] * 1000).toFixed(1)} mm, ${((worst[1] * 180) / Math.PI).toFixed(2)}°)`);
  const drawn = marks.drawnHeld.model;
  check(some(pistol(drawn).held) && !some(pistol(drawn).slot), "drawn: in the hand, out of the slot");
  const quiet = steps.filter((s) => s.state.frame <= marks.out.frame).every((s) => s.state.model.gun.fired === 0);
  check(quiet, "no shot on the draw, the trigger held throughout");
  check(marks.fired.model.gun.fired === 1, `let out and pulled, it fires (${marks.fired.model.gun.fired})`);
});

await run("replay", async (rt) => {
  await play(rt, scenarios["armory-fire"]);
  check(fireEnd !== undefined && fireEnd === JSON.stringify((await rt.get("/state")).model), "the same shots in a fresh runtime end on an identical model");
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab armory e2e: all checks passed");
