// vr-lab rifle E2E (station 3): the two-handed rifle driven by injected XR
// samples, headlessly.
//
// Plays the rifle scenarios in `e2e/vr-lab-scenarios.mjs` against
// `examples/vr-lab` (`--headless`, pinned clock, one sample per fixed step)
// and checks `/state.model` and `/scene`:
//
//   1. Two-hand aim: taken by the pistol grip and the foregrip, raised, then
//      aimed by the support hand alone. Each shot leaves along support -
//      primary (within 0.5°), from the accepted rifle's muzzle, and hits its
//      target within 1 cm of where that aim puts it at 5.5 m.
//   2. Barrel into the wall, two hands: the whole rifle stops at it, its
//      grips rigid on it, the pull refused; drawn (by default) both hands sit
//      on the accepted grips, ghosts where they are tracked; the camera never
//      moves.
//      Drawn back clear, it fires.
//   3. Rotation only: the primary hand still, the support swings the barrel
//      into the wall; the swept turn stops it short of the support.
//   4. Support slide and break: slid along the foregrip the aim holds;
//      dragged 15 cm off the axis it breaks off at 12 cm, back to one hand,
//      with no pop on that step, then settles into the primary hand.
//   5. Hand-off both ways: primary opens (the support keeps it), retaken,
//      support opens: no pop at any of them.
//   6. Full-auto: semi held 30 steps fires 1; A switches; held 30, 5.
//   7. Replay: the aim scenario in a fresh runtime ends on an identical model.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-rifle.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { add, fmt, len, placedAt, play, qrot, quat, record, rigToWorld, session, some, sub, turnOf, xyz } from "./xr-scenario.mjs";
import {
  ARMORY_WALL_X, GRIP_IN_RIFLE, RIFLE_FIST, armory, RIFLE, RIFLE_SWING, TARGETS, TARGET_SIZE, project, scenarios, twoHandAim,
} from "./vr-lab-scenarios.mjs";

const PORT = 8299;
const SNAP_STEP = 0.02;
const SNAP_TURN = 0.15;

const dot = (a, b) => a.reduce((s, c, i) => s + c * b[i], 0);
const deg = (r) => (r * 180) / Math.PI;
const angleOf = (a, b) => Math.acos(Math.min(1, dot(a, b) / (len(a) * len(b))));
const rifle = (m) => m.props.find((p) => p.kind.$ctor === "Props.Rifle");
const held = (m) => some(rifle(m).held);
const supportSide = (m) => some(rifle(m).support)?.side.$ctor;
const holderSide = (m) => held(m)?.side.$ctor;
// Where the rifle is: held, its accepted pose; else as read back.
const poseOf = (m) => {
  const pose = held(m)?.accepted ?? rifle(m).pose;
  return { position: xyz(pose.position), orientation: quat(pose.orientation) };
};
const at = (pose, local) => add(pose.position, qrot(pose.orientation, local));
const gripOn = (pose) => at(pose, GRIP_IN_RIFLE);
// The muzzle face's corner nearest -X (rig), of a rifle at `pose`.
const leftmost = (pose) =>
  Math.min(...[-1, 1].flatMap((sx) => [-1, 1].map((sy) => at(pose, [sx * RIFLE.half[0], sy * RIFLE.half[1], -RIFLE.half[2]])[0])));
const shot = (m) => some(m.rifle.last);
// Each step's change of the rifle's pose: moved (m) and turned (radians).
const stepMoves = (steps, from, to) =>
  steps.slice(from + 1, to + 1).map((s, i) => {
    const [a, b] = [poseOf(steps[from + i].state.model), poseOf(s.state.model)];
    return { moved: len(sub(b.position, a.position)), turned: turnOf(a.orientation, b.orientation), i: from + i + 1 };
  });

const run = (name, body) => session(name, project, PORT, body);

// `/scene`'s top group: 4/5 the left and right hands, 6/7 their ghosts.
const toWorld = (p) => rigToWorld(p, armory.feet, armory.yaw);
// And back (the armory's rig faces world +X: rig x is world z).
const toRig = ([x, y, z]) => [z - armory.feet[2], y, armory.feet[0] - x];

let aimEnd;
await run("two-hand aim", async (rt) => {
  const { marks, at: hands } = await record(rt, scenarios["rifle-aim"]);
  const two = marks.twoHands.model;
  check(holderSide(two) === "Hold.Right" && supportSide(two) === "Hold.Left", `two hands: the right holds, the left supports (${holderSide(two)}, ${supportSide(two)})`);
  const raised = marks.raised.model;
  check(turnOf(poseOf(raised).orientation, [0, 0, 0, 1]) < 1e-4, "raised with both hands, the aim unchanged");
  for (const [name, target, tag] of [["Right", TARGETS[2], "target-2"], ["Middle", TARGETS[1], "target-1"], ["Left", TARGETS[0], "target-0"]]) {
    const m = marks[`fired${name}`].model;
    const h = hands(`fired${name}`);
    const s = shot(m);
    const aim = twoHandAim(h.right.at, h.right.orientation ?? [0, 0, 0, 1], h.left.at);
    const off = deg(angleOf(xyz(s.dir), sub(h.left.at, h.right.at)));
    check(off < 0.5, `${name}: the shot along support - primary (${off.toFixed(3)}°)`);
    const muzzle = len(sub(xyz(s.from), at(poseOf(m), add(GRIP_IN_RIFLE, [0, RIFLE.bore, -RIFLE.muzzle]))));
    check(muzzle < 1e-4, `${name}: from the accepted rifle's muzzle (${(muzzle * 1000).toFixed(3)} mm)`);
    const want = add(aim.muzzle, aim.dir.map((c) => (c * (target[2] - aim.muzzle[2])) / aim.dir[2]));
    const hit = some(s.impact);
    const got = hit ? xyz(hit.point) : [NaN, NaN, NaN];
    check(hit?.tag === tag && len(sub(got, want)) < 0.01, `${name}: hit ${hit?.tag} at ${fmt(got)}, ${(len(sub(got, want)) * 1000).toFixed(2)} mm from the analytic ${fmt(want)}`);
    check(Math.abs(got[0] - target[0]) < TARGET_SIZE / 2 && Math.abs(got[1] - target[1]) < TARGET_SIZE / 2, `${name}: on the target's face`);
  }
  const end = marks.firedLeft.model;
  check(end.rifle.fired === 3 && end.score === 3, `three shots, three points (${end.rifle.fired}, ${end.score})`);
  aimEnd = JSON.stringify((await rt.get("/state")).model);
});

await run("barrel into the wall, two hands", async (rt) => {
  const { steps, marks, scenes, at: hands, index } = await record(rt, scenarios["rifle-wall"], { scenes: true });
  const pushed = marks.pushed.model;
  const pose = poseOf(pushed);
  const face = leftmost(pose);
  check(face >= ARMORY_WALL_X - 0.001 && face < ARMORY_WALL_X + 0.01, `the rifle stops at the wall (its muzzle face at x ${face.toFixed(4)})`);
  const h = hands("pushed");
  const tracked = h.right.at[0] - RIFLE.muzzle;
  check(tracked < face - 0.05, `the tracked hands would put it ${((face - tracked) * 100).toFixed(1)} cm in`);
  check(supportSide(pushed) === "Hold.Left", "both hands still on it");
  const grip = gripOn(pose);
  check(len(sub(grip, h.right.at)) > 0.05, `the accepted grip ${(len(sub(grip, h.right.at)) * 100).toFixed(1)} cm short of the tracked hand`);
  const clicked = marks.clicked.model;
  check(clicked.rifle.fired === 0 && clicked.rifle.refused === 1 && !shot(clicked), `the pull is refused (fired ${clicked.rifle.fired}, refused ${clicked.rifle.refused})`);
  // Both hands drawn on the accepted rifle (the default), ghosts at the tracked hands.
  const kids = scenes.stillHeld.scene.obj.Group;
  const acc = marks.stillHeld.model;
  check(acc.handView.$ctor === "Accepted", "Accepted by default");
  const drawnRight = placedAt(kids[5]);
  const drawnLeft = placedAt(kids[4]);
  const accPose = poseOf(acc);
  // The hand's root on the grip: the fist's placement past it.
  const gripW = toWorld(at(accPose, add(GRIP_IN_RIFLE, RIFLE_FIST.position)));
  check(drawnRight && len(sub(drawnRight, gripW)) < 1e-4, `the right hand drawn on the accepted grip (${drawnRight && (len(sub(drawnRight, gripW)) * 1000).toFixed(3)} mm)`);
  // The left drawn on the foregrip line: along the barrel from the grip, between its ends.
  const leftRig = drawnLeft && toRig(drawnLeft);
  const rel = leftRig && sub(leftRig, gripOn(accPose));
  const along = rel && dot(rel, qrot(accPose.orientation, [0, 0, -1]));
  const offLine = rel && len(sub(rel, qrot(accPose.orientation, [0, 0, -along])));
  check(rel && offLine < 1e-4 && along >= RIFLE.foreNear - 1e-6 && along <= RIFLE.foreFar + 1e-6, `the left hand drawn on the foregrip (${along?.toFixed(3)} m ahead, ${offLine?.toExponential(1)} off it)`);
  const ha = hands("stillHeld");
  const ghostR = placedAt(kids[7]);
  const ghostL = placedAt(kids[6]);
  check(ghostR && len(sub(ghostR, toWorld(ha.sent.right.at))) < 1e-4 && ghostL && len(sub(ghostL, toWorld(ha.left.at))) < 1e-4, "ghosts at both tracked hands");
  // Held there, the hands still, it stays at the wall: the hold's slide along
  // the contact normal trembles it a little (`Hold.move`), never turns it.
  const pressed = stepMoves(steps, index("pushed"), index("still"));
  const tremble = pressed.reduce((w, s) => [Math.max(w[0], s.moved), Math.max(w[1], s.turned)], [0, 0]);
  check(tremble[0] < 0.003 && tremble[1] < 1e-4, `pressed there, it holds (worst step ${(tremble[0] * 1000).toFixed(2)} mm, ${deg(tremble[1]).toFixed(4)}°)`);
  const cams = ["near", "pushed", "stillHeld", "fired"].map((k) => JSON.stringify(scenes[k].camera));
  check(cams.every((c) => c === cams[0]), `the camera never moves (${cams[0]})`);
  const fired = marks.fired.model;
  const s = shot(fired);
  const struck = some(s?.impact);
  check(fired.rifle.fired === 1 && struck?.tag === "armory-wall" && Math.abs(struck.point.x - ARMORY_WALL_X) < 0.001, `drawn back clear, it fires into the wall (${struck?.tag} at x ${struck?.point.x.toFixed(4)})`);
});

await run("rotation only into the wall", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["rifle-swing"]);
  const yawOf = (m) => {
    const f = qrot(poseOf(m).orientation, [0, 0, -1]);
    return deg(Math.atan2(-f[0], -f[2]));
  };
  const swung = marks.swung.model;
  const worst = Math.min(...steps.slice(index("ready"), index("back") + 1).map((s) => leftmost(poseOf(s.state.model))));
  check(worst >= ARMORY_WALL_X - 0.001, `the barrel never passes the wall (nearest x ${worst.toFixed(4)})`);
  check(yawOf(swung) > 15 && yawOf(swung) < RIFLE_SWING.deg - 4, `turned ${yawOf(swung).toFixed(1)}°, stopped short of the support's ${RIFLE_SWING.deg}°`);
  check(leftmost(poseOf(swung)) < ARMORY_WALL_X + 0.01, `against the wall (x ${leftmost(poseOf(swung)).toFixed(4)})`);
  const pivot = len(sub(gripOn(poseOf(swung)), RIFLE_SWING.at));
  check(pivot < 0.01, `turned about the still primary fist (${(pivot * 1000).toFixed(1)} mm off it)`);
  check(supportSide(swung) === "Hold.Left" && supportSide(marks.back.model) === "Hold.Left", "the support holds throughout");
  check(Math.abs(yawOf(marks.back.model)) < 0.01, `swung back, aimed straight again (${yawOf(marks.back.model).toFixed(3)}°)`);
});

await run("support slide and break", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["rifle-break"]);
  const straight = (m) => turnOf(poseOf(m).orientation, [0, 0, 0, 1]);
  const slid = steps.slice(index("raised"), index("slidBack") + 1);
  check(slid.every((s) => supportSide(s.state.model) === "Hold.Left" && straight(s.state.model) < 1e-4), "slid 16-38 cm along the foregrip: still two hands, the aim unchanged");
  const broke = steps.findIndex((s, i) => i > index("slidBack") && supportSide(s.state.model) === undefined);
  const offAt = (i) => steps[i].left.at[0] - steps[index("slidBack")].left.at[0];
  check(broke > 0 && offAt(broke) > RIFLE.breakOff && offAt(broke - 1) <= RIFLE.breakOff + 1e-9, `breaks off ${(offAt(broke) * 100).toFixed(1)} cm off the axis`);
  check(holderSide(steps[broke].state.model) === "Hold.Right", "back to one hand: the right");
  const [m] = stepMoves(steps, broke - 1, broke);
  check(m.moved <= 0.001 && m.turned <= (0.5 * Math.PI) / 180, `no pop on the break step (${(m.moved * 1000).toFixed(3)} mm, ${deg(m.turned).toFixed(3)}°)`);
  check(deg(straight(steps[broke - 1].state.model)) > 10, `it was aimed ${deg(straight(steps[broke - 1].state.model)).toFixed(1)}° off by the support`);
  const settling = stepMoves(steps, broke, index("after"));
  const worst = settling.reduce((w, s) => [Math.max(w[0], s.moved), Math.max(w[1], s.turned)], [0, 0]);
  check(worst[0] <= SNAP_STEP + 1e-4 && worst[1] <= SNAP_TURN + 1e-4, `settles back without a jump (worst ${(worst[0] * 1000).toFixed(1)} mm, ${deg(worst[1]).toFixed(2)}° a step)`);
  check(straight(marks.after.model) < 1e-4, "settled straight in the right hand");
  const left = marks.after.model.left;
  check(!left.armed && some(left.hover) === undefined, "the left must open before it takes the foregrip again");
});

await run("hand-off both ways", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["rifle-handoff"]);
  const handed = marks.handed.model;
  check(holderSide(handed) === "Hold.Left" && rifle(handed).fore && supportSide(handed) === undefined, "primary opened: the left holds it by the foregrip");
  const [h] = stepMoves(steps, index("handed") - 1, index("handed"));
  check(h.moved <= 1e-6 && h.turned <= 1e-6, `no pop as it is handed on (${(h.moved * 1000).toFixed(4)} mm)`);
  const carried = len(sub(poseOf(marks.carried.model).position, poseOf(handed).position));
  check(Math.abs(carried - 0.06) < 1e-4, `the left carries it (${(carried * 100).toFixed(2)} cm)`);
  const retaken = marks.retaken.model;
  check(holderSide(retaken) === "Hold.Right" && supportSide(retaken) === "Hold.Left" && !rifle(retaken).fore, "retaken by the right, the left back on the foregrip");
  const moves = stepMoves(steps, index("carried"), index("oneHand"));
  const worst = moves.reduce((w, s) => [Math.max(w[0], s.moved), Math.max(w[1], s.turned)], [0, 0]);
  check(worst[0] <= 1e-6 && worst[1] <= 1e-6, `retaken and let go by the support: still (worst ${(worst[0] * 1000).toFixed(4)} mm, ${deg(worst[1]).toFixed(4)}°)`);
  const one = marks.oneHand.model;
  check(holderSide(one) === "Hold.Right" && supportSide(one) === undefined, "the support opened: one hand");
});

await run("full-auto", async (rt) => {
  const { marks } = await record(rt, scenarios["rifle-auto"]);
  check(marks.semiDone.model.rifle.fired === 1, `semi, held 30 steps: one shot (${marks.semiDone.model.rifle.fired})`);
  check(marks.selected.model.rifle.mode.$ctor === "Gun.Auto", `A: full-auto (${marks.selected.model.rifle.mode.$ctor})`);
  const burst = marks.autoDone.model.rifle.fired - marks.semiDone.model.rifle.fired;
  // The pull and one every 6 steps of the 29 after it.
  const want = Math.floor(29 / 6) + 1;
  check(burst === want, `full-auto, held 30 steps: ${burst} shots (want ${want})`);
});

await run("replay", async (rt) => {
  await play(rt, scenarios["rifle-aim"]);
  check(aimEnd !== undefined && aimEnd === JSON.stringify((await rt.get("/state")).model), "the same aim in a fresh runtime ends on an identical model");
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab rifle e2e: all checks passed");
