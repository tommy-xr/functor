// examples/vr-lab's scripted XR scenarios: played by its e2e checks
// (`e2e/vr-lab.mjs`) and recorded as demo videos (`npm run demo:vr-lab`).
// A new station adds its scenarios here. Format: `e2e/xr-scenario.mjs`;
// recorded fixtures: `e2e/xr-fixture.mjs`.
//
// Grab table (station 1): the rig stands at world (0, 0, 4) facing +Z, so
// rig-local x is world -x and rig-local -z is world +z. The ball rests at
// rig (0.2, 0.96, -0.55); the wall's face is at rig x = -0.6, at the table's end.
// The stick lies on the table at rig (-0.3, 0.9175, -0.6) along rig -Z, 0.7 m
// long and 3.5 cm thick.

import { add, qconj, qmul, qrot, sub } from "./xr-scenario.mjs";

export const project = "examples/vr-lab";
// Tapped before an attached recording (`npm run record:vr-lab`): station 1, reset.
export const resetKey = "1";

export const grabTable = { feet: [0, 0, 4], yaw: 0 };
export const BALL = [0.2, 0.96, -0.55];
export const STICK = { length: 0.7, thickness: 0.035 };
export const WALL_RIG_X = -0.6;
// A turn of the wrist about +Y (left, toward the wall, for positive degrees).
export const yaw = (deg) => [0, Math.sin((deg * Math.PI) / 360), 0, Math.cos((deg * Math.PI) / 360)];
// Where the stick's near end is held: 5 cm in from the end, just above it.
const HANDLE = [-0.3, 0.95, -0.3];
const LIFTED = [-0.3, 1.2, -0.3];
// A turn of the wrist about +X (the tip up, for positive degrees).
export const pitch = (deg) => [Math.sin((deg * Math.PI) / 360), 0, 0, Math.cos((deg * Math.PI) / 360)];
// A roll about -Z, the way the hand points (clockwise, for positive degrees).
export const roll = (deg) => [0, 0, -Math.sin((deg * Math.PI) / 360), Math.cos((deg * Math.PI) / 360)];

// The table (rig-local): its top, and its extent across and ahead.
export const TABLE = { top: 0.9, x: [-0.6, 0.85], z: [-0.9, -0.3] };
// The brick rests turned 35° on the table's right; the bottle stands behind
// it, its neck half way up its top 8 cm.
export const BRICK = { at: [0.6, 0.9325, -0.5], turn: yaw(35), size: [0.1, 0.065, 0.21] };
export const BOTTLE = { at: [0.62, 1.03, -0.8], neck: 0.09 };

// Where a prop grabbed at its rest pose by a hand at `grip` (a point, a
// wrist turn) is when the hand moves to `to`: the hold keeps their offset.
const carried = (prop, grip, to) => {
  const offset = qrot(qconj(grip.turn), sub(prop.at, grip.at));
  return add(to.at, qrot(to.turn, offset));
};

// The stick throw: lifted, wound back tip up, then flicked tip down at 10° a
// step while the hand moves forward, released mid-flick.
const RAISED = [-0.1, 1.3, -0.25];
const FLICK_DEG = -10;
// The flick's spin, rig-local (rad/s about +X).
export const FLICK_W = [(FLICK_DEG * 60 * Math.PI) / 180, 0, 0];
const flickAt = (n) => RAISED.map((v, i) => v + (THROW_V[i] * n) / 60);

// The throw: a straight, steady sweep forward and up, released mid-sweep.
export const THROW_V = [0, 2.0, -3.6];
const SWEEP_FROM = [0.22, 1.2, -0.25];
// 16 m/s: past the 12 m/s cap.
export const FAST_V = [0, 1.0, -15.97];
const along = (n) => SWEEP_FROM.map((v, i) => v + (THROW_V[i] * n) / 60);

// From the player's right, level with the table: the hands, the wall
// behind them and the basket down range all in frame.
const side = { eye: [-2.1, 2.2, 2.7], target: [0.05, 0.85, 5.5], fov: 55 };
// Films a recorded fixture named by path (`npm run demo:vr-lab -- rec.json …`).
export const defaultSpectator = side;

// From above the wall, in its plane: the ball held on this side while the
// hand goes through to the other.
const alongWall = { eye: [0.65, 2.45, 3.75], target: [0.6, 1.05, 4.6], fov: 50 };
// Straight down over the wall: the held ball on this side, the hand (and its
// ghost) through on the other.
const overWall = { eye: [0.64, 8.0, 4.45], target: [0.64, 1.15, 4.55], fov: 8 };

// Above and behind the player's left shoulder: the stick turning into the wall.
const overStick = { eye: [0.2, 3.1, 3.75], target: [0.3, 1.1, 4.6], fov: 45 };

// From the player's right, side on: the stick sweeping the ball off the table,
// then flying spinning down range.
const besideStick = { eye: [-2.5, 1.5, 5.2], target: [0.2, 0.9, 5.5], fov: 48 };

const grab = [
  { frames: 20, right: { to: [0.2, 1.05, -0.45], squeeze: 0 } },
  { frames: 20, right: { to: BALL, squeeze: 0 }, mark: "reached" },
  { frames: 6, right: { squeeze: 1 }, mark: "grabbed" },
];

// From the player's right, low and side on to the table's near edge.
const atEdge = { eye: [-1.25, 1.15, 3.7], target: [-0.35, 0.93, 4.33], fov: 38 };
// From the player's right and above: the brick and bottle end of the table.
const rightEnd = { eye: [-1.5, 1.75, 3.45], target: [-0.45, 1.0, 4.55], fov: 44 };
// Close on the bottle, from the player's right.
const atBottle = { eye: [-1.35, 1.45, 4.05], target: [-0.55, 1.15, 4.7], fov: 34 };

// The brick, taken near a top corner with the wrist turned, lifted and turned
// again, handed to the left hand (held at its centre), put down, then taken
// again by the right at another point.
const brickGrip = { at: [0.585, 0.975, -0.6], turn: qmul(yaw(-50), roll(30)) };
const brickLift = { at: [0.4, 1.2, -0.42], turn: qmul(yaw(20), pitch(-25)) };
const brickHandOff = carried(BRICK, brickGrip, brickLift);
const brickDown = [0.42, 1.06, -0.45];

// The brick held at its centre, tipped over its long edge and lowered onto
// the table's near edge, the hand going on 4 cm below the top; then lifted.
const edgeAbove = { at: [0.35, 1.1, -0.32], turn: roll(-30) };
const edgeBelow = [0.35, TABLE.top - 0.04, -0.32];

// The bottle: a hand pointing up, 2 cm off the neck.
const neckGrip = add(BOTTLE.at, [0.02, BOTTLE.neck, 0.008]);
const bottleBase = add(BOTTLE.at, [0.05, -0.1, 0]);

// The ball pushed into the wall (rig x -0.6) with the hand through it.
const throughWall = [-0.77, 1.15, -0.55];
const atWall = [-0.545, 1.15, -0.55];

// --- Inventory (station 2) ---
// The rig stands at world (0, 0, -4) facing -Z, so rig-local is the world
// moved to the feet, and the same props lie on the same table ahead. The
// head turns about a neck fixed at NECK (`Hips.eyeFromNeck`); the pelvis is
// inferred under the neck at 0.62 of its height, and the slots sit at their
// authored offsets from it (`Slots`), turned with the hips' yaw.

export const NECK = { at: [0, 1.49, 0.09], eye: [0, 0.11, -0.09] };
export const ROOT = [0, 1.49 * 0.62, 0.09];
export const SLOT = {
  leftHolster: { id: 1, offset: [-0.2, -0.1, -0.02], radius: 0.08 },
  rightHolster: { id: 2, offset: [0.2, -0.1, -0.02], radius: 0.08 },
  pouch: { id: 3, offset: [0, 0, -0.15], radius: 0.08 },
};
// Where a slot is with the hips turned `deg` (left, positive) about ROOT.
export const slotAt = (name, deg = 0) => add(ROOT, qrot(yaw(deg), SLOT[name].offset));
const RH = slotAt("rightHolster");
const LH = slotAt("leftHolster");
// The left hand waits out of the way of the slots as they turn.
const LEFT_OUT = [-0.4, 1.15, 0];
const RIGHT_REST = [0.25, 0.95, -0.2];

// From behind the player's right shoulder, high: the bench ahead, the
// belt, the slots and the gaze arrow, and the body turning left.
const overShoulder = { eye: [0.9, 2.9, -2.3], target: [-0.05, 1.1, -4.05], fov: 31 };
// From the player's right, a little above the hips.
const atHip = { eye: [2.0, 1.45, -3.1], target: [0.0, 1.0, -4.05], fov: 36 };

const outOfTheWay = { frames: 15, left: { to: LEFT_OUT } };
// Take the brick from the bench by its centre and put it in the right holster.
const stowBrick = [
  { frames: 20, right: { to: [0.6, 1.05, -0.45] } },
  { frames: 15, right: { to: BRICK.at }, mark: "reachedBrick" },
  { frames: 6, right: { squeeze: 1 }, mark: "tookBrick" },
  { frames: 30, right: { to: [0.3, 1.0, -0.1] } },
  { frames: 25, right: { to: RH }, mark: "atHolster" },
  { frames: 1, right: { squeeze: 0 }, mark: "stored" },
  { frames: 20, right: { to: RIGHT_REST }, mark: "clear" },
];
// The head turns 90° left at 120°/s, past the deadzone, and holds: the hips
// drag after it, then settle on it.
const turnLeft = [
  { frames: 45, head: { turn: yaw(90) }, mark: "turning" },
  { frames: 120, head: {}, mark: "turned" },
];
// The empty right hand reaches back into the holster and, there, the head
// glances 40° right: the hips hold. Squeezed, it takes the brick back out.
const reachBack = [
  { frames: 25, right: { to: RH }, mark: "inZone" },
  { frames: 30, head: { turn: yaw(-40) }, mark: "glanced" },
  { frames: 1, right: {}, mark: "beforeRetrieve" },
  { frames: 1, right: { squeeze: 1 }, mark: "retrieved" },
  { frames: 1, right: {}, mark: "afterRetrieve" },
  { frames: 6, right: {}, mark: "holding" },
  { frames: 30, right: { to: [0.3, 1.15, -0.3] }, mark: "lifted" },
];
// The stick, taken by its near end, drawn back off the bench and turned
// tip down, held still in the left holster and let go: it fits no slot, so it
// drops. The bottle, held still in the full right holster: refused.
const stickIntoHolster = [
  { frames: 20, right: { to: [-0.3, 1.05, -0.2] } },
  { frames: 20, right: { to: HANDLE } },
  { frames: 6, right: { squeeze: 1 }, mark: "tookStick" },
  { frames: 10, right: { to: [-0.3, 1.1, -0.3] } },
  { frames: 25, right: { to: [-0.3, 1.1, 0.45] } },
  { frames: 25, right: { turn: pitch(-75) } },
  { frames: 25, right: { to: LH } },
  { frames: 6, right: {}, mark: "stickAtHolster" },
  { frames: 1, right: { squeeze: 0 }, mark: "stickLetGo" },
  { frames: 30, right: { to: RIGHT_REST, turn: [0, 0, 0, 1] }, mark: "stickDropped" },
];
const bottleIntoFull = [
  { frames: 20, right: { to: [0.65, 1.0, -0.75] } },
  { frames: 15, right: { to: add(BOTTLE.at, [0.05, -0.1, 0]) } },
  { frames: 6, right: { squeeze: 1 }, mark: "tookBottle" },
  { frames: 30, right: { to: [0.35, 1.0, -0.15] } },
  { frames: 20, right: { to: RH } },
  { frames: 6, right: {}, mark: "bottleAtHolster" },
  { frames: 1, right: { squeeze: 0 }, mark: "bottleLetGo" },
  { frames: 40, right: { to: RIGHT_REST }, mark: "bottleDropped" },
];

const inventoryScenarios = {
  // Put the brick in the right holster and turn the body a quarter turn
  // left: it rides the hips round, and the holster stays where the hand
  // finds it.
  "inventory-stow-turn": {
    station: 2,
    neck: NECK,
    spectator: overShoulder,
    steps: [outOfTheWay, ...stowBrick, ...turnLeft],
  },

  // Stow the brick, then reach back for it while glancing aside: the hips
  // hold still under the reaching hand, and it comes out exactly where it sat.
  "inventory-reach": {
    station: 2,
    neck: NECK,
    spectator: atHip,
    steps: [outOfTheWay, ...stowBrick, ...reachBack],
  },

  // Neither goes in: the stick fits no slot, and the brick fills the holster
  // the bottle is let go in. Both drop as any let-go prop does.
  "inventory-refuse": {
    station: 2,
    neck: NECK,
    spectator: atHip,
    steps: [outOfTheWay, ...stickIntoHolster, ...stowBrick, ...bottleIntoFull],
  },

  // The demo: stow, turn and turn back, reach back with a glance, take it
  // out and put it back, then the stick and the bottle refused.
  "inventory-tour": {
    station: 2,
    neck: NECK,
    spectator: overShoulder,
    steps: [
      outOfTheWay,
      ...stowBrick,
      ...turnLeft,
      { frames: 45, head: { turn: [0, 0, 0, 1] } },
      { frames: 90, head: {}, mark: "back" },
      ...reachBack,
      { frames: 30, head: { turn: [0, 0, 0, 1] } },
      { frames: 90, head: {} },
      { frames: 30, right: { to: RH } },
      { frames: 1, right: { squeeze: 0 }, mark: "restored" },
      { frames: 20, right: { to: RIGHT_REST } },
      ...stickIntoHolster,
      ...bottleIntoFull,
    ],
  },
};

export const scenarios = {
  // Pick the ball up and throw it into the basket.
  "grab-throw": {
    station: 1,
    spectator: side,
    steps: [
      ...grab,
      { frames: 30, right: { to: SWEEP_FROM }, mark: "wound" },
      { frames: 6, right: { to: along(6) }, mark: "sweep" },
      // Still moving at the sweep's velocity as the hand opens.
      { frames: 1, right: { to: along(7), squeeze: 0 }, mark: "release" },
      { frames: 1, right: { to: along(8) }, mark: "after1" },
      { frames: 1, right: { to: along(9) }, mark: "after2" },
      { frames: 80, right: { to: [0.25, 1.2, -0.45] }, mark: "landed" },
    ],
  },

  // Push the held ball into the wall: it stops there while the hand goes on,
  // follows the hand back out, and when let go inside the wall drops outside.
  "wall-push": {
    station: 1,
    spectator: alongWall,
    steps: [
      ...grab,
      { frames: 20, right: { to: [-0.3, 1.15, -0.55] } },
      { frames: 25, right: { to: [-0.72, 1.15, -0.55] }, mark: "pushed" },
      { frames: 20, right: { to: [-0.4, 1.15, -0.55] }, mark: "pulled" },
      { frames: 20, right: { to: [-0.72, 1.15, -0.55] }, mark: "pushedAgain" },
      { frames: 1, right: { squeeze: 0 }, mark: "release" },
      { frames: 40, right: { to: [0.0, 1.1, -0.35] }, mark: "settled" },
    ],
  },

  // Drag the hand far through the wall: the grip breaks and the ball drops
  // where it stopped. Still squeezing, the hand can't take it back; it has to
  // open and squeeze again.
  "break-grip": {
    station: 1,
    spectator: alongWall,
    steps: [
      ...grab,
      { frames: 20, right: { to: [-0.3, 1.1, -0.55] } },
      { frames: 30, right: { to: [-1.0, 1.1, -0.55] }, mark: "broke" },
      { frames: 30, right: { to: [-0.5, 1.02, -0.55] }, mark: "returned" },
    ],
  },

  // A swing faster than the cap: the ball leaves at the capped estimate, not
  // the kinematic body's own speed, so the release's setVelocity is what counts.
  "fast-throw": {
    station: 1,
    spectator: side,
    steps: [
      ...grab,
      { frames: 20, right: { to: [0.22, 1.1, -0.1] } },
      { frames: 4, right: { to: [0.22, 1.1 + (FAST_V[1] * 4) / 60, -0.1 + (FAST_V[2] * 4) / 60] } },
      { frames: 1, right: { to: [0.22, 1.1 + (FAST_V[1] * 5) / 60, -0.1 + (FAST_V[2] * 5) / 60], squeeze: 0 }, mark: "release" },
      { frames: 1, right: { to: [0.22, 1.2, -0.6] }, mark: "after1" },
    ],
  },

  // Take the stick by its near end and, the hand held still, turn the wrist a
  // quarter turn toward the wall: the far end stops at the wall. Turned back,
  // it follows the hand again; let go, it returns to its rack.
  "stick-wall": {
    station: 1,
    spectator: overStick,
    steps: [
      { frames: 20, right: { to: [-0.3, 1.05, -0.2], squeeze: 0 } },
      { frames: 20, right: { to: HANDLE }, mark: "reachedStick" },
      { frames: 6, right: { squeeze: 1 }, mark: "tookStick" },
      { frames: 30, right: { to: LIFTED }, mark: "lifted" },
      { frames: 45, right: { turn: yaw(90) }, mark: "turned" },
      { frames: 15, right: {}, mark: "pressed" },
      { frames: 45, right: { turn: yaw(0) }, mark: "turnedBack" },
      { frames: 6, right: { squeeze: 0 }, mark: "letGo" },
      { frames: 20, right: { to: [0.25, 1.0, -0.2] } },
    ],
  },

  // Take the stick and, held low, sweep it across the table into the ball,
  // which it shoves away; then lift it, wind it back and flick it, letting
  // go mid-flick: it flies on spinning at the wrist's rate.
  "stick-throw": {
    station: 1,
    spectator: besideStick,
    steps: [
      { frames: 20, right: { to: [-0.3, 1.05, -0.2], squeeze: 0 } },
      { frames: 20, right: { to: HANDLE }, mark: "reachedStick" },
      { frames: 6, right: { squeeze: 1 }, mark: "tookStick" },
      { frames: 10, right: { to: [-0.3, 0.99, -0.3] }, mark: "low" },
      { frames: 30, right: { turn: yaw(-100) }, mark: "swept" },
      { frames: 30, right: { to: RAISED, turn: yaw(0) }, mark: "raised" },
      { frames: 20, right: { turn: pitch(50) }, mark: "wound" },
      { frames: 6, right: { to: flickAt(6), turn: pitch(50 + 6 * FLICK_DEG) }, mark: "flick" },
      { frames: 1, right: { to: flickAt(7), turn: pitch(50 + 7 * FLICK_DEG), squeeze: 0 }, mark: "release" },
      { frames: 1, right: { to: flickAt(8) }, mark: "after1" },
      { frames: 1, right: { to: flickAt(9) }, mark: "after2" },
      { frames: 1, right: { to: flickAt(10) }, mark: "after3" },
      { frames: 180, right: { to: [0.25, 1.0, -0.2], turn: [0, 0, 0, 1] }, mark: "landed" },
      { frames: 1, right: {}, mark: "rest" },
    ],
  },

  // Take the resting, turned brick near a corner with the wrist turned: it
  // doesn't move. Lift it, hand it to the left hand, put it down, and take it
  // again with the right somewhere else: it never jumps.
  "brick-regrab": {
    station: 1,
    spectator: rightEnd,
    steps: [
      { frames: 20, right: { to: [0.62, 1.1, -0.45], turn: brickGrip.turn } },
      { frames: 20, right: { to: brickGrip.at }, mark: "reachedBrick" },
      { frames: 6, right: { squeeze: 1 }, mark: "tookBrick" },
      { frames: 6, right: {}, mark: "holdingBrick" },
      { frames: 30, right: { to: brickLift.at, turn: brickLift.turn }, mark: "lifted" },
      { frames: 30, left: { to: brickHandOff } },
      { frames: 1, left: {}, mark: "beforeHandOff" },
      { frames: 1, left: { squeeze: 1 }, mark: "handedOff" },
      { frames: 6, left: {}, right: { squeeze: 0 }, mark: "rightOpen" },
      { frames: 20, right: { to: [0.25, 0.95, -0.2] } },
      { frames: 30, left: { to: brickDown } },
      { frames: 6, left: {}, mark: "lowered" },
      { frames: 1, left: { squeeze: 0 }, mark: "putDown" },
      { frames: 60, left: { to: [-0.25, 0.95, -0.1] }, mark: "settled" },
      { frames: 25, right: { to: [0.45, 1.05, -0.4], turn: qmul(yaw(70), pitch(15)) } },
      { frames: 15, right: { to: [0.45, 0.955, -0.43] }, mark: "reachedAgain" },
      { frames: 6, right: { squeeze: 1 }, mark: "tookAgain" },
      { frames: 6, right: {}, mark: "holdingAgain" },
      { frames: 30, right: { to: [0.4, 1.15, -0.4] }, mark: "liftedAgain" },
    ],
  },

  // Take the brick by its centre, tip it over its long edge and press it
  // down onto the table's near edge, the hand going on below the top: it
  // stops on the edge. Lifted, it follows the hand again.
  "brick-edge": {
    station: 1,
    spectator: atEdge,
    steps: [
      { frames: 20, right: { to: [0.6, 1.05, -0.45] } },
      { frames: 15, right: { to: BRICK.at }, mark: "reachedBrick" },
      { frames: 6, right: { squeeze: 1 }, mark: "tookBrick" },
      { frames: 30, right: { to: edgeAbove.at, turn: edgeAbove.turn }, mark: "above" },
      { frames: 30, right: { to: edgeBelow }, mark: "pressed" },
      { frames: 15, right: {}, mark: "held" },
      { frames: 30, right: { to: edgeAbove.at }, mark: "lifted" },
      { frames: 1, right: { squeeze: 0 }, mark: "letGo" },
      { frames: 40, right: { to: [0.25, 0.95, -0.2], turn: [0, 0, 0, 1] } },
    ],
  },

  // A hand pointing up takes the bottle 2 cm off its neck: it snaps there.
  "bottle-neck": {
    station: 1,
    spectator: atBottle,
    steps: [
      { frames: 20, right: { to: [0.6, 1.25, -0.6], turn: pitch(90) } },
      { frames: 20, right: { to: neckGrip }, mark: "atNeck" },
      { frames: 1, right: { squeeze: 1 }, mark: "tookNeck" },
      { frames: 30, right: { to: [0.45, 1.35, -0.55] }, mark: "liftedNeck" },
      { frames: 30, right: { to: [0.45, 1.15, -0.55] }, mark: "lowered" },
      { frames: 1, right: { squeeze: 0 }, mark: "letGo" },
      { frames: 40, right: { to: [0.25, 1.05, -0.3], turn: [0, 0, 0, 1] } },
    ],
  },

  // The bottle taken by its base, the wrist turned: it doesn't snap.
  "bottle-base": {
    station: 1,
    spectator: rightEnd,
    steps: [
      { frames: 20, right: { to: [0.7, 1.0, -0.7], turn: yaw(60) } },
      { frames: 15, right: { to: bottleBase }, mark: "atBase" },
      { frames: 1, right: { squeeze: 1 }, mark: "tookBase" },
      { frames: 6, right: {}, mark: "holdingBase" },
      { frames: 30, right: { to: [0.55, 1.2, -0.6] }, mark: "liftedBase" },
    ],
  },

  // Push the held ball into the wall, the hand going on through it, and
  // switch where the hand is drawn (H): tracked, through the wall; then at
  // the ball, with a ghost where it really is. Brought back to within a
  // centimetre of the ball the ghost goes; pushed through again it returns.
  "hand-view": {
    station: 1,
    spectator: overWall,
    steps: [
      ...grab,
      { frames: 20, right: { to: [-0.3, 1.15, -0.55] } },
      { frames: 30, right: { to: throughWall }, mark: "blockedTracked" },
      { frames: 20, right: {}, tap: "h", mark: "blockedAccepted" },
      { frames: 20, right: { to: atWall }, mark: "nearWall" },
      { frames: 20, right: { to: [-0.4, 1.15, -0.55] }, mark: "out" },
      { frames: 25, right: { to: throughWall }, mark: "blockedAgain" },
      { frames: 20, right: {}, tap: "h", mark: "trackedAgain" },
      { frames: 1, right: { squeeze: 0 }, mark: "letGo" },
      { frames: 30, right: { to: [0.0, 1.1, -0.35] } },
    ],
  },

  // `grab-throw` recorded under `--emulate-xr` (`npm run record:vr-lab --
  // e2e/fixtures/vr-lab/grab-throw.xr.json --scenario grab-throw`).
  "grab-throw-recorded": {
    fixture: "e2e/fixtures/vr-lab/grab-throw.xr.json",
    spectator: side,
  },

  ...inventoryScenarios,

  // Lose the hand mid-swing, then let go as it comes back: no throw.
  "tracking-loss": {
    station: 1,
    spectator: side,
    steps: [
      ...grab,
      { frames: 20, right: { to: SWEEP_FROM } },
      { frames: 4, right: { to: along(4) }, mark: "swinging" },
      { frames: 12, right: { active: false }, mark: "lost" },
      { frames: 1, right: { active: true, to: along(10), squeeze: 0 }, mark: "release" },
      { frames: 40, right: { to: [0.25, 1.1, -0.35] }, mark: "settled" },
    ],
  },
};
