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

import { readFileSync } from "node:fs";
import { ROOT as REPO, check } from "./mcp-rpc.mjs";
import { add, qconj, qmul, qrot, quat, some, sub, xyz } from "./xr-scenario.mjs";

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

// High above the player, looking down the table to the wall: the hand on
// either side of it.
const aboveWall = { eye: [0.65, 3.6, 3.95], target: [0.4, 1.0, 4.55], fov: 44 };

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

// The bottle: an upright hand, 2 cm off the neck.
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

// --- Armory (station 3) ---
// The rig stands at world (4, 0, 0) facing +X. Rig-local, the bench is the
// grab table's (top 0.9), the pistol stands on it upright at PISTOL facing
// down range (-Z), the wall's face is at x = -0.6 (z -1.4 to 0.2), and the
// targets' faces are at z = -5.5. Held, the pistol settles so the hand is at
// its grip: the bore runs 5 cm above the hand's -Z axis, the muzzle 15 cm
// along it (`Gun`).

export const armory = { feet: [4, 0, 0], yaw: Math.PI / 2 };
// Where the hand's root sits in each gun's layout frame (`Gun.placement`,
// `Longarm.placement`, `Pump.placement`): the handle up the fist, so the bore
// is 12° (14°) above the hand's -Z. A scenario holding a gun scripts its
// right hand as the gun's fist (`fists`; each take below switches it), so
// the geometry below is the fist's; the controller is injected at
// fist ∘ placement.
// Read from the generated `fitted.fun` (`let pistolGripPlacement: Input.pose = { position: { x: …`).
const FITTED = readFileSync(`${REPO}examples/vr-lab/fitted.fun`, "utf8");
const placement = (name) => {
  const line = FITTED.split("\n").find((l) => l.startsWith(`let ${name}:`));
  const v = [...line.matchAll(/[xyzw]: (0\.0 - )?([0-9.]+)/g)].map((m) => (m[1] ? -1 : 1) * Number(m[2]));
  return { position: v.slice(0, 3), orientation: v.slice(3, 7) };
};
export const PISTOL_FIST = placement("pistolGripPlacement");
export const RIFLE_FIST = placement("rifleGripPlacement");
export const SHOTGUN_FIST = placement("shotgunGripPlacement");
export const PISTOL = [0.15, 0.9675, -0.5];
// The resting pistol's grip, where a level hand takes it.
// The grip in the pistol's frame (`Gun.grip`): where the hand holds it.
export const GRIP_IN_PISTOL = [0, -0.0025, 0.0475];
export const PISTOL_GRIP = add(PISTOL, GRIP_IN_PISTOL);
export const BORE = { up: 0.05, ahead: 0.15, chamber: 0.02 };
export const PISTOL_HALF = [0.0175, 0.0675, 0.1025];
export const TARGETS = [[-0.7, 1.15], [0, 1.45], [0.7, 1.15]].map(([x, y]) => [x, y, -5.5]);
export const TARGET_SIZE = 0.3;
export const ARMORY_WALL_X = -0.6;

// The muzzle and the shot's way for a hand at `at` turned `q`.
export const muzzleOf = (at, q) => ({ from: add(at, qrot(q, [0, BORE.up, -BORE.ahead])), dir: qrot(q, [0, 0, -1]) });
// A hand at `at` turned (yaw, then pitch, no roll) so its shot passes through `point`.
export function aimAt(at, point) {
  let q = [0, 0, 0, 1];
  for (let i = 0; i < 8; i++) {
    const d = sub(point, muzzleOf(at, q).from);
    const n = Math.hypot(...d);
    const pitchDeg = (Math.asin(d[1] / n) * 180) / Math.PI;
    const yawDeg = (Math.atan2(-d[0], -d[2]) * 180) / Math.PI;
    q = qmul(yaw(yawDeg), pitch(pitchDeg));
  }
  return q;
}

const AIM_HAND = [0.2, 1.3, -0.35];
// At the right target, 6 cm off its centre; low between the targets, at the backstop.
const AIM_RIGHT = aimAt(AIM_HAND, add(TARGETS[2], [-0.06, 0.04, 0]));
const AIM_MIDDLE = aimAt(AIM_HAND, add(TARGETS[1], [0.03, -0.05, 0]));
const AIM_LEFT = aimAt(AIM_HAND, add(TARGETS[0], [0.05, 0.02, 0]));
const AIM_MISS = aimAt(AIM_HAND, [0.35, 0.7, -5.6]);
// Pointing left at the wall, level: through it, and back clear of it.
const LEFT = yaw(90);
const WALL_NEAR = [-0.25, 1.2, -0.5];
const THROUGH = [-0.55, 1.2, -0.5];
const CLEAR = [-0.33, 1.2, -0.5];
const ARMORY_RH = slotAt("rightHolster");

// Behind the player's right shoulder: the hand and pistol, the shot down range.
const behindGun = { eye: [2.8, 1.9, 1.25], target: [7.0, 1.2, -0.35], fov: 40 };
// Above and behind, looking at the wall at the bench's left end.
const atArmoryWall = { eye: [3.35, 2.25, 0.85], target: [4.45, 1.15, -0.45], fov: 42 };
// From the player's right, at the hip.
const atArmoryHip = { eye: [4.1, 1.45, 2.15], target: [4.0, 1.0, 0.05], fov: 40 };
// Down range from beside the targets: the hits.
const atTargets = { eye: [8.0, 1.6, 1.3], target: [9.5, 1.25, 0.0], fov: 32 };

const takePistol = [
  { frames: 15, left: { to: LEFT_OUT }, fists: { right: PISTOL_FIST } },
  { frames: 20, right: { to: [0.15, 1.05, -0.3] } },
  { frames: 15, right: { to: PISTOL_GRIP }, mark: "reachedPistol" },
  { frames: 6, right: { squeeze: 1 }, mark: "tookPistol" },
];
const fireAt = (turn, name) => [
  { frames: 25, right: { to: AIM_HAND, turn } },
  { frames: 6, right: {}, mark: `aimed${name}` },
  { frames: 1, right: { trigger: 1 }, mark: `fired${name}` },
  { frames: 6, right: {} },
  { frames: 4, right: { trigger: 0 } },
];

const armoryScenarios = {
  // Take the pistol off the bench, aim at the right target and pull: one hit
  // there. Hold the trigger down: still one shot. Let it out, aim low between
  // the targets and pull: a shot into the backstop, no hit.
  "armory-fire": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...takePistol,
      { frames: 25, right: { to: AIM_HAND, turn: AIM_RIGHT } },
      { frames: 6, right: {}, mark: "aimed" },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 30, right: {}, mark: "held" },
      { frames: 4, right: { trigger: 0 }, mark: "letOut" },
      { frames: 20, right: { turn: AIM_MISS } },
      { frames: 1, right: { trigger: 1 }, mark: "missed" },
      { frames: 4, right: { trigger: 0 }, mark: "after" },
    ],
  },

  // Point the pistol at the wall and push the hand on through it: the pistol
  // stops at the wall, and the trigger only clicks. Drawn back clear of it,
  // the trigger fires (into the wall).
  "armory-wall": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atArmoryWall,
    steps: [
      ...takePistol,
      { frames: 25, right: { to: WALL_NEAR, turn: LEFT } },
      { frames: 25, right: { to: THROUGH }, mark: "pushed" },
      { frames: 1, right: { trigger: 1 }, mark: "clicked" },
      { frames: 6, right: { trigger: 0 } },
      { frames: 20, right: { to: CLEAR }, mark: "clear" },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 6, right: { trigger: 0 } },
    ],
  },

  // Holster the pistol at the right thigh; reach back with the trigger
  // already held and draw it: no jump, no shot until the trigger is let out
  // and pulled again.
  "armory-holster": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atArmoryHip,
    steps: [
      ...takePistol,
      { frames: 30, right: { to: [0.3, 1.0, -0.1] } },
      { frames: 25, right: { to: ARMORY_RH }, mark: "atHolster" },
      { frames: 1, right: { squeeze: 0 }, mark: "holstered" },
      { frames: 15, right: { to: add(ARMORY_RH, [0.06, 0.12, -0.08]) } },
      { frames: 4, right: { trigger: 1 } },
      { frames: 15, right: { to: ARMORY_RH }, mark: "reachedHolster" },
      { frames: 1, right: { squeeze: 1 }, mark: "drawn" },
      { frames: 20, right: {}, mark: "drawnHeld" },
      { frames: 30, right: { to: AIM_HAND, turn: AIM_MIDDLE }, mark: "out" },
      { frames: 4, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 6, right: { trigger: 0 } },
    ],
  },

  // The demo: take the pistol, hit each target with a tracer, hold the
  // trigger down (one shot), push the barrel into the wall (refused), fire
  // clear of it, then holster and draw it with the trigger held.
  "armory-tour": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...takePistol,
      ...fireAt(AIM_RIGHT, "Right"),
      ...fireAt(AIM_MIDDLE, "Middle"),
      { frames: 20, right: { turn: AIM_LEFT }, spectator: behindGun },
      { frames: 1, right: { trigger: 1 }, mark: "release" },
      { frames: 45, right: {}, mark: "held" },
      { frames: 4, right: { trigger: 0 } },
      { frames: 20, right: {}, spectator: atTargets },
      { frames: 30, right: { to: WALL_NEAR, turn: LEFT }, spectator: atArmoryWall },
      { frames: 25, right: { to: THROUGH }, mark: "pushed" },
      { frames: 1, right: { trigger: 1 }, mark: "clicked" },
      { frames: 15, right: { trigger: 0 } },
      { frames: 20, right: { to: CLEAR } },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 10, right: { trigger: 0 } },
      { frames: 30, right: { to: [0.3, 1.0, -0.1], turn: [0, 0, 0, 1] }, spectator: atArmoryHip },
      { frames: 25, right: { to: ARMORY_RH }, mark: "atHolster" },
      { frames: 1, right: { squeeze: 0 }, mark: "holstered" },
      { frames: 20, right: { to: add(ARMORY_RH, [0.06, 0.12, -0.08]), trigger: 1 } },
      { frames: 15, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 1 }, mark: "drawn" },
      { frames: 20, right: {} },
      { frames: 30, right: { to: AIM_HAND, turn: AIM_MIDDLE }, spectator: behindGun },
      { frames: 6, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 } },
      { frames: 20, right: { trigger: 0 } },
    ],
  },
};

// --- Armory: the rifle (station 3) ---
// Racked level on the bench's right, pointing down range: its centre at
// RIFLE_RACK, the pistol grip's fist GRIP_IN_RIFLE from it (`Longarm.grip`).
// Held, the bore runs 6 cm above the primary fist's -Z axis, the muzzle 56 cm
// along it; the support fist rides that axis 14-40 cm ahead, and the barrel
// points from the primary fist to the support fist, rolled as the primary
// hand is (`Longarm.aimAt`).

export const RIFLE_RACK = [0.55, 1.05, -0.62];
export const GRIP_IN_RIFLE = [0, -0.0275, 0.14];
export const RIFLE_GRIP = add(RIFLE_RACK, GRIP_IN_RIFLE);
// Half the collider (from the rifle's centre), the bore, and the foregrip.
export const RIFLE = { half: [0.025, 0.0725, 0.42], bore: 0.06, muzzle: 0.56, foreNear: 0.14, foreFar: 0.4, breakOff: 0.12 };
const ID = [0, 0, 0, 1];
const scale = (v, k) => v.map((c) => c * k);
const dot = (a, b) => a.reduce((s, c, i) => s + c * b[i], 0);
const unit = (v) => scale(v, 1 / Math.hypot(...v));
// A point `d` along a fist's -Z, the fist at `at` turned `q`.
export const aheadOf = (at, q, d) => add(at, qrot(q, [0, 0, -d]));
// The rifle aimed by a primary hand at `at` turned `q` and the support fist
// at `s`: its barrel's way, its up, and its muzzle.
export function twoHandAim(at, q, s) {
  const dir = unit(sub(s, at));
  const hint = qrot(q, [0, 1, 0]);
  const up = unit(sub(hint, scale(dir, dot(hint, dir))));
  return { dir, up, muzzle: add(at, add(scale(dir, RIFLE.muzzle), scale(up, RIFLE.bore))) };
}
// Where the support fist goes, `d` ahead of a level primary hand at `at`, so
// the shot passes through `point`.
export function supportFor(at, point, d) {
  let dir = unit(sub(point, at));
  for (let i = 0; i < 8; i++) {
    const up = unit(sub([0, 1, 0], scale(dir, dir[1])));
    dir = unit(sub(point, add(at, scale(up, RIFLE.bore))));
  }
  return add(at, scale(dir, d));
}

// Off the rack: the right hand on the pistol grip, then the left on the
// foregrip 25 cm ahead (by way of above the rifle, clear of the pistol).
const takeRifle = [
  { frames: 15, left: { to: [0.3, 1.25, -0.3] }, fists: { right: RIFLE_FIST } },
  { frames: 20, right: { to: [0.5, 1.15, -0.35] } },
  { frames: 15, right: { to: RIFLE_GRIP }, mark: "reachedRifle" },
  { frames: 6, right: { squeeze: 1 }, mark: "tookRifle" },
  { frames: 15, left: { to: aheadOf(RIFLE_GRIP, ID, 0.25) }, mark: "reachedFore" },
  { frames: 6, left: { squeeze: 1 }, mark: "twoHands" },
];
// Raised to the shoulder, both hands together: the aim doesn't change.
export const RIFLE_AIM = [0.2, 1.3, -0.3];
const raise = (to = RIFLE_AIM, frames = 30) => ({ frames, right: { to }, left: { to: aheadOf(to, ID, 0.25) }, mark: "raised" });
const at = (target) => supportFor(RIFLE_AIM, target, 0.25);
// Aim with the left hand only, settle, pull once.
const shootAt = (target, name) => [
  { frames: 20, left: { to: at(target) } },
  { frames: 10, left: {}, mark: `aimed${name}` },
  { frames: 1, right: { trigger: 1 }, mark: `fired${name}` },
  { frames: 6, right: { trigger: 0 } },
];
// Pointing left at the wall (rig x -0.6), both hands: near it, pushed on
// through, and drawn back clear.
const LEFTWARD = yaw(90);
export const RIFLE_WALL = { near: [0.1, 1.2, -0.45], through: [-0.1, 1.2, -0.45], clear: [0.0, 1.2, -0.45] };
const bothTo = (to, q = LEFTWARD) => ({ right: { to, turn: q }, left: { to: aheadOf(to, q, 0.25) } });
// Swung into the wall by the support hand alone: the primary fist 30 cm off
// the wall, the support 18 cm ahead of it, turned 35° left about it.
export const RIFLE_SWING = { at: [-0.3, 1.2, -0.5], d: 0.18, deg: 35 };
const swungTo = (deg) => add(RIFLE_SWING.at, qrot(yaw(deg), [0, 0, -RIFLE_SWING.d]));
// Dragged 15 cm off the axis to the right, from the raised aim.
const DRAG = add(aheadOf(RIFLE_AIM, ID, 0.25), [0.15, 0, 0]);

// Behind the player's left shoulder: both hands on the rifle, the shot down range.
const behindRifle = { eye: [2.85, 1.9, 0.95], target: [7.0, 1.2, -0.1], fov: 40 };
// Above the bench, side on: the support sliding and breaking off.
const aboveRifle = { eye: [3.7, 2.2, 1.35], target: [4.35, 1.2, -0.15], fov: 40 };
// Down range of the bench, side on to the wall (edge-on, on the right): the
// rifle pointing into it, both hands on it.
const besideRifleWall = { eye: [5.7, 1.55, -0.25], target: [4.45, 1.2, -0.45], fov: 40 };

const rifleScenarios = {
  // Two hands on the rifle, raised; aimed at each target by the support hand
  // alone, one pull each: three hits.
  "rifle-aim": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: behindRifle,
    steps: [...takeRifle, raise(), ...shootAt(TARGETS[2], "Right"), ...shootAt(TARGETS[1], "Middle"), ...shootAt(TARGETS[0], "Left")],
  },

  // Semi with the trigger held 30 steps: one shot. A on the right hand
  // switches to full-auto: held 30 steps, 5.
  "rifle-auto": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: behindRifle,
    steps: [
      ...takeRifle,
      raise(),
      { frames: 20, left: { to: at(add(TARGETS[1], [0, -0.1, 0])) } },
      { frames: 10, left: {}, mark: "aimed" },
      { frames: 30, right: { trigger: 1 }, mark: "semiHeld" },
      { frames: 6, right: { trigger: 0 }, mark: "semiDone" },
      { frames: 1, right: { primary: true }, mark: "selected" },
      { frames: 3, right: { primary: false } },
      { frames: 30, right: { trigger: 1 }, mark: "autoHeld" },
      { frames: 6, right: { trigger: 0 }, mark: "autoDone" },
    ],
  },

  // Both hands push the barrel into the wall: the whole rifle stops at it,
  // the pull only clicks; both hands are drawn on its grips, their ghosts
  // through the wall. Drawn back clear, the pull fires.
  "rifle-wall": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: atArmoryWall,
    steps: [
      ...takeRifle,
      raise(),
      { frames: 30, ...bothTo(RIFLE_WALL.near), mark: "near" },
      { frames: 25, ...bothTo(RIFLE_WALL.through), mark: "pushed" },
      { frames: 1, right: { trigger: 1 }, mark: "clicked" },
      { frames: 6, right: { trigger: 0 } },
      { frames: 6, right: {}, mark: "stillHeld" },
      { frames: 1, right: {}, mark: "still" },
      { frames: 20, ...bothTo(RIFLE_WALL.clear), mark: "clear" },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 6, right: { trigger: 0 } },
    ],
  },

  // The primary hand held still by the wall, the support swings the barrel
  // left into it: it stops there, turned short of the support.
  "rifle-swing": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: atArmoryWall,
    steps: [
      ...takeRifle,
      raise(),
      { frames: 30, right: { to: RIFLE_SWING.at }, left: { to: swungTo(0) }, mark: "ready" },
      ...Array.from({ length: 35 }, (_, i) => ({ frames: 1, left: { to: swungTo(i + 1) }, ...(i === 34 ? { mark: "swung" } : {}) })),
      { frames: 10, left: {}, mark: "held" },
      ...Array.from({ length: 35 }, (_, i) => ({ frames: 1, left: { to: swungTo(34 - i) } })),
      { frames: 5, left: {}, mark: "back" },
    ],
  },

  // Slid along the foregrip and back: still two hands, the aim unchanged.
  // Dragged 15 cm off the axis: it breaks off at 12 cm, back to one hand.
  "rifle-break": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: aboveRifle,
    steps: [
      ...takeRifle,
      raise(),
      { frames: 15, left: { to: aheadOf(RIFLE_AIM, ID, 0.38) }, mark: "slidFar" },
      { frames: 20, left: { to: aheadOf(RIFLE_AIM, ID, 0.16) }, mark: "slidNear" },
      { frames: 10, left: { to: aheadOf(RIFLE_AIM, ID, 0.25) }, mark: "slidBack" },
      { frames: 30, left: { to: DRAG }, mark: "dragged" },
      { frames: 20, left: {}, mark: "after" },
      { frames: 4, left: { squeeze: 0 } },
    ],
  },

  // The right hand lets go while the left holds the foregrip: the left keeps
  // the rifle where it is and carries it. The right takes its grip again:
  // two hands. Then the left lets go: one hand. No pop at any of them.
  "rifle-handoff": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: behindRifle,
    steps: [
      ...takeRifle,
      raise(),
      { frames: 1, right: { squeeze: 0 }, mark: "handed" },
      { frames: 15, right: { to: add(RIFLE_AIM, [0.12, -0.15, 0.1]) } },
      { frames: 20, left: { to: add(aheadOf(RIFLE_AIM, ID, 0.25), [0, 0.06, 0]) }, mark: "carried" },
      { frames: 15, right: { to: add(RIFLE_AIM, [0, 0.06, 0]) } },
      { frames: 1, right: { squeeze: 1 }, mark: "retaken" },
      { frames: 15, right: {}, mark: "twoAgain" },
      { frames: 1, left: { squeeze: 0 }, mark: "supportOpened" },
      { frames: 15, left: { to: LEFT_OUT }, mark: "oneHand" },
    ],
  },

  // The demo: take the rifle in two hands, aim it with the support hand at
  // each target, a full-auto burst and a held semi pull, the barrel into the
  // wall, the support slid and broken off, the hand-off both ways, racked.
  "rifle-tour": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: aboveRifle,
    steps: [
      ...takeRifle,
      { ...raise(), spectator: behindRifle },
      ...shootAt(TARGETS[2], "Right"),
      ...shootAt(TARGETS[0], "Left"),
      { frames: 20, left: { to: at(TARGETS[1]) } },
      { frames: 8, left: {} },
      { frames: 1, right: { primary: true }, mark: "auto" },
      { frames: 3, right: { primary: false } },
      { frames: 30, right: { trigger: 1 }, mark: "burst" },
      { frames: 12, right: { trigger: 0 } },
      { frames: 1, right: { primary: true } },
      { frames: 3, right: { primary: false } },
      { frames: 30, right: { trigger: 1 }, mark: "semi" },
      { frames: 12, right: { trigger: 0 } },
      { frames: 30, ...bothTo(RIFLE_WALL.near), spectator: besideRifleWall },
      { frames: 25, ...bothTo(RIFLE_WALL.through), mark: "pushed" },
      { frames: 10, right: {} },
      { frames: 1, right: { trigger: 1 }, mark: "clicked" },
      { frames: 15, right: { trigger: 0 } },
      { frames: 42, right: {} },
      { frames: 20, ...bothTo(RIFLE_WALL.clear) },
      { frames: 1, right: { trigger: 1 } },
      { frames: 8, right: { trigger: 0 } },
      { frames: 30, ...bothTo(RIFLE_AIM, ID), spectator: aboveRifle },
      { frames: 15, left: { to: aheadOf(RIFLE_AIM, ID, 0.38) } },
      { frames: 20, left: { to: aheadOf(RIFLE_AIM, ID, 0.16) } },
      { frames: 10, left: { to: aheadOf(RIFLE_AIM, ID, 0.25) } },
      { frames: 45, left: { to: DRAG }, mark: "broke" },
      { frames: 20, left: {} },
      { frames: 15, left: { squeeze: 0 } },
      { frames: 15, left: { to: aheadOf(RIFLE_AIM, ID, 0.25) } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 1, right: { squeeze: 0 }, mark: "handed" },
      { frames: 20, right: { to: add(RIFLE_AIM, [0.12, -0.15, 0.1]) } },
      { frames: 30, left: { to: add(aheadOf(RIFLE_AIM, ID, 0.25), [0, 0.06, 0]) } },
      { frames: 20, right: { to: add(RIFLE_AIM, [0, 0.06, 0]) } },
      { frames: 1, right: { squeeze: 1 }, mark: "retaken" },
      { frames: 20, right: {} },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: LEFT_OUT } },
      { frames: 30, right: { to: add(RIFLE_GRIP, [0, 0.06, 0.08]) } },
      { frames: 15, right: { to: RIFLE_GRIP } },
      { frames: 1, right: { squeeze: 0 }, mark: "racked" },
      { frames: 20, right: { to: [0.35, 1.1, -0.2] } },
    ],
  },
};

// --- Armory: ammunition (station 3) ---
// The pistol starts 12+1, the rifle 30+1 (`Ammo`). The thumbstick pushed
// down on a gun's hand (`stick: [0, -1]`) drops its magazine below the well
// (`Gun.dropOf`): for a level pistol, 13.24 cm below the hand and 4.5 cm
// behind it. The pouch is on the belt (`slotAt("pouch")`).

export const CAPACITY = { pistol: 12, rifle: 30 };
export const PISTOL_ROUNDS = CAPACITY.pistol + 1;
export const RIFLE_ROUNDS = CAPACITY.rifle + 1;
export const SPARES = { pistol: 2, rifle: 2 };
// The shotgun starts unloaded; its shells are in the box on the bench.
export const SHELL_BOX = 12;
export const RESET_ROUNDS = PISTOL_ROUNDS + RIFLE_ROUNDS + SPARES.pistol * CAPACITY.pistol + SPARES.rifle * CAPACITY.rifle + SHELL_BOX;
// Reading the rounds from `/state.model`: a magazine's, a gun's (its
// magazine's and chamber's), every unspent one at the station (magazines,
// tube, chambers, the shell box, loose rounds and shells), and a gun's
// readout.
export const roundsIn = (mag) => some(mag)?.rounds ?? 0;
export const ammoCount = (ammo) => roundsIn(ammo.mag) + (ammo.chambered ? 1 : 0);
export const unspent = (m) =>
  m.props.reduce((n, p) => n + (p.kind.$ctor === "Props.Round" ? 1 : roundsIn(p.mag)), 0) +
  ammoCount(m.gun.ammo) + ammoCount(m.rifle.ammo) + ammoCount(m.shotgun.ammo) + m.shellBox;
export const readout = (ammo) => `${some(ammo.mag) ? roundsIn(ammo.mag) : "--"}+${ammo.chambered ? 1 : 0}`;
/** Check every recorded step keeps the station's rounds: unspent, plus shots fired and loose rounds taken away. */
export function conserves(name, steps) {
  const total = (m) => unspent(m) + m.gun.fired + m.rifle.fired + m.shotgun.fired + m.lost;
  const bad = steps.find((s) => total(s.state.model) !== RESET_ROUNDS);
  check(!bad, `${name}: every step (${steps.length}) conserves ${RESET_ROUNDS} rounds${bad ? ` — frame ${bad.state.frame}: ${total(bad.state.model)}` : ""}`);
}
// Fire `n` pulls of the right trigger, each let out over 4 steps; the last
// pull marked `mark`.
const pulls = (n, mark) =>
  Array.from({ length: n }, (_, i) => [
    { frames: 1, right: { trigger: 1 }, ...(i === n - 1 && mark ? { mark } : {}) },
    { frames: 4, right: { trigger: 0 } },
  ]).flat();
// Over the bench, level: the magazine drops onto it below the hand.
export const OVER_BENCH = [0.0, 1.2, -0.6];
export const MAG_DROP = add(OVER_BENCH, [0, -0.1324, 0.045]);
// Lower, the magazine would drop into the bench.
const LOW_OVER_BENCH = [0.0, 1.0, -0.6];
// Where the dropped magazine comes to rest (on its base, the bench top 0.9),
// and where a hand takes it: by its top, clear of its count.
const MAG_REST = [MAG_DROP[0], 0.95, MAG_DROP[2]];
const MAG_TOP = add(MAG_REST, [0, 0.065, 0]);
const POUCH = slotAt("pouch");

// (Armory rig (x, y, z) is world (4 - z, y, x).) Close behind the aimed
// pistol's slide: its readout.
const atSlide = { eye: [3.95, 1.5, 0.35], target: [4.4, 1.3, 0.2], fov: 30 };
// The same, over the bench.
const atBenchSlide = { eye: [4.15, 1.4, 0.15], target: [4.55, 1.22, 0.0], fov: 32 };
// Behind and left of the bench: the dropped magazine and the spares.
const atSpares = { eye: [3.95, 1.45, -0.6], target: [4.5, 0.95, -0.15], fov: 40 };
// From ahead and right, at the belt: the pouch (a controller's body runs
// back from its grip, so a camera behind a hand sees the controller).
const atPouch = { eye: [4.65, 1.2, 0.35], target: [4.06, 0.95, 0.0], fov: 34 };
// From ahead and left, at the left hand held out.
const atLeftHand = { eye: [4.95, 1.35, -0.45], target: [4.4, 1.12, -0.2], fov: 34 };

// Close behind the raised rifle's receiver: its readout, the targets beyond.
const atReceiver = { eye: [4.0, 1.5, 0.32], target: [4.45, 1.36, 0.18], fov: 28 };

// Shared runs. The pistol aimed at the right target, settled `settle` steps.
const aimPistol = (settle) => [
  { frames: 25, right: { to: AIM_HAND, turn: AIM_RIGHT } },
  { frames: settle, right: {}, mark: "aimed" },
];
// Level over the bench, the thumbstick pushed down: the magazine drops.
// The thumbstick pushed down on the right hand (its magazine drops), then
// let back for `fall` steps.
const dropMag = (fall, view) => [
  { frames: 1, right: { stick: [0, -1] }, mark: "ejected", ...(view ? { spectator: view } : {}) },
  { frames: fall, right: { stick: [0, 0] }, mark: "fell" },
];
const dropOverBench = (fall) => [
  { frames: 25, right: { to: OVER_BENCH, turn: [0, 0, 0, 1] }, spectator: atSpares },
  { frames: 5, right: {}, mark: "overBench" },
  ...dropMag(fall).map((seg, i) => (i === 1 ? { ...seg, mark: "landed" } : seg)),
];
// The right hand aside; the left takes the dropped magazine by its top,
// puts it in the pouch and takes it back.
const pouchTheMag = [
  { frames: 20, right: { to: [0.25, 1.2, -0.3] }, spectator: atSpares },
  { frames: 20, left: { to: add(MAG_TOP, [0, 0.1, 0]) } },
  { frames: 10, left: { to: MAG_TOP }, mark: "reachedMag" },
  { frames: 4, left: { squeeze: 1 }, mark: "tookMag" },
  { frames: 20, left: { to: add(MAG_REST, [0, 0.2, 0.15]) }, mark: "lifted" },
  { frames: 30, left: { to: POUCH }, mark: "atPouch", spectator: atPouch },
  { frames: 1, left: { squeeze: 0 }, mark: "pouched" },
  { frames: 20, left: {} },
  { frames: 4, left: { squeeze: 1 }, mark: "retrieved" },
];
// The rifle in two hands, raised, aimed at the middle target, switched to full-auto.
const autoRifle = (view = {}) => [
  { ...takeRifle[0], ...(view.take ? { spectator: view.take } : {}) },
  ...takeRifle.slice(1),
  raise(),
  { frames: 20, left: { to: at(TARGETS[1]) }, ...(view.aim ? { spectator: view.aim } : {}) },
  { frames: 10, left: {}, mark: "aimed" },
  { frames: 1, right: { primary: true }, mark: "auto" },
  { frames: 3, right: { primary: false } },
];

const ammoScenarios = {
  // Aimed at the right target: thirteen pulls, thirteen hits; the fourteenth
  // clicks dry.
  "ammo-pistol": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...takePistol,
      ...aimPistol(6),
      ...pulls(PISTOL_ROUNDS, "lastShot"),
      { frames: 1, right: { trigger: 1 }, mark: "dry" },
      { frames: 6, right: { trigger: 0 }, mark: "after" },
    ],
  },

  // Two hands, full-auto, the trigger held 200 steps: 31 shots, then a dry
  // click. Let out and pulled again: another.
  "ammo-rifle": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: behindRifle,
    steps: [
      ...autoRifle(),
      { frames: 200, right: { trigger: 1 }, mark: "held" },
      { frames: 6, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 }, mark: "dryAgain" },
      { frames: 6, right: { trigger: 0 }, mark: "after" },
    ],
  },

  // Three shots, then over the bench the thumbstick drops the magazine (9)
  // onto it. The left hand takes it, puts it in the pouch and takes it back
  // out. The chambered round still fires; then dry.
  "ammo-mag": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atSpares,
    steps: [
      ...takePistol,
      ...aimPistol(4),
      ...pulls(3, "fired3"),
      // Low over the bench, the magazine has no room to drop: it stays in.
      { frames: 25, right: { to: LOW_OVER_BENCH, turn: [0, 0, 0, 1] } },
      { frames: 5, right: {} },
      { frames: 1, right: { stick: [0, -1] }, mark: "lowPush" },
      { frames: 5, right: { stick: [0, 0] } },
      ...dropOverBench(40),
      ...pouchTheMag,
      { frames: 30, left: { to: [-0.15, 1.15, -0.35] }, mark: "out" },
      ...aimPistol(4).map((seg) => ({ ...seg, spectator: behindGun })),
      { frames: 1, right: { trigger: 1 }, mark: "lastRound" },
      { frames: 4, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 }, mark: "dry" },
      { frames: 6, right: { trigger: 0 }, mark: "after" },
    ],
  },

  // The demo: the pistol counting down (12+1 to 8+1), its magazine dropped
  // over the bench (8 on it), the chambered round fired (--+0) and a dry
  // click (the readout red); the left hand takes the magazine, pouches it,
  // takes it back (still 8) and sets it down; the pistol holstered. Then the
  // rifle on full-auto to empty (30+1 to 0+0), and a dry click.
  "ammo-tour": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...takePistol,
      ...aimPistol(10).map((seg, i) => (i === 1 ? { ...seg, spectator: atSlide } : seg)),
      ...pulls(4).map((seg) => (seg.frames === 4 ? { ...seg, frames: 12 } : seg)),
      ...dropOverBench(30),
      { frames: 1, right: { trigger: 1 }, spectator: atBenchSlide },
      { frames: 14, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 }, mark: "dry" },
      { frames: 14, right: { trigger: 0 } },
      ...pouchTheMag,
      { frames: 30, left: { to: [-0.2, 1.15, -0.4] }, spectator: atLeftHand },
      { frames: 20, left: {} },
      { frames: 20, left: { to: [-0.3, 1.03, -0.55] }, spectator: atSpares },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: LEFT_OUT } },
      { frames: 30, right: { to: [0.3, 1.0, -0.1] }, spectator: atArmoryHip },
      { frames: 25, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 10, right: {} },
      ...autoRifle({ take: behindRifle, aim: atReceiver }),
      { frames: 120, right: { trigger: 1 } },
      { frames: 80, right: { trigger: 1 }, spectator: behindRifle, mark: "emptied" },
      { frames: 10, right: { trigger: 0 }, spectator: atReceiver },
      { frames: 1, right: { trigger: 1 } },
      { frames: 20, right: { trigger: 0 } },
    ],
  },
};

// --- Armory: reloading (station 3) ---
// A magazine brought to a well's mouth (where an ejected one drops out),
// lined up within ~32° (`Gun.guide`), latches, and is guided up the well's
// axis at the depth the hand pushes it to; home, it seats. The other hand
// racks the slide (or the rifle's charging handle) back along the bore past
// 80% of its travel, then lets go: it chambers. The last shot locks the
// pistol's slide (and the rifle's bolt) back on the empty magazine.
//
// The pistol held level at RELOAD_HAND: its centre is the hand plus
// PISTOL_FROM_GRIP; its well WELL from that centre, raked back 12°
// (`Gun.pistol.well`); its slide taken at SLIDE (`Gun.pistol.rack`). A
// magazine is taken by its base, 4 cm below its centre, so the hand
// pushing it sits that far down the well's axis from it.

export const RELOAD_HAND = [0.05, 1.15, -0.4];
const PISTOL_FROM_GRIP = [0, 0.0025, -0.0475];
export const WELL = { at: [0, -0.0175, 0.0675], rake: (-0.21 * 180) / Math.PI, drop: 0.12, entry: 0.04, exit: 0.07 };
export const SLIDE = { at: [0, 0.0475, 0.0875], travel: 0.04, lockBack: 0.028 };
const MAG_BASE = 0.04;
export const LOOSE_CAP = 6;
const RELOAD_PISTOL = add(RELOAD_HAND, PISTOL_FROM_GRIP);
export const WELL_TURN = pitch(WELL.rake);
// A point `d` along the held pistol's well (0 seated, -drop the mouth).
export const wellPoint = (d) => add(RELOAD_PISTOL, add(WELL.at, qrot(WELL_TURN, [0, d, 0])));
// Where the hand holding a magazine by its base is, the magazine `d` along
// the well turned `q` (by default, lined up with it).
const magHand = (d, q = WELL_TURN) => add(wellPoint(d), qrot(q, [0, -MAG_BASE, 0]));
const tipped = (deg) => qmul(WELL_TURN, pitch(deg));
// The pistol's slide, `back` behind home.
const slideAt = (back) => add(RELOAD_PISTOL, add(SLIDE.at, [0, 0, back]));
// The spares on the bench: by their base.
export const SPARE = { pistol: [-0.42, 0.95, -0.5], rifle: [-0.24, 0.945, -0.5] };
const spareBase = (at, half) => add(at, [0, -half, 0]);

// The rifle held one-handed, level, at RIFLE_HAND: its centre the hand plus
// RIFLE_FROM_GRIP; its well RIFLE_WELL from that (no rake), its mouth 10 cm
// down; its charging handle at CHARGING (`Longarm.weapon.rack`).
const RIFLE_HAND = [0.15, 1.15, -0.35];
const RIFLE_FROM_GRIP = [0, 0.0275, -0.14];
const RIFLE_CENTRE = add(RIFLE_HAND, RIFLE_FROM_GRIP);
export const RIFLE_WELL = { at: [0, -0.0275, 0.05], drop: 0.1 };
export const CHARGING = { at: [0, 0.0625, 0.215], travel: 0.07 };
const RIFLE_BASE = 0.035;
const rifleWell = (d) => add(RIFLE_CENTRE, add(RIFLE_WELL.at, [0, d, 0]));
const rifleMagHand = (d) => add(rifleWell(d), [0, -RIFLE_BASE, 0]);
const handleAt = (back) => add(RIFLE_CENTRE, add(CHARGING.at, [0, 0, back]));

// On the well from ahead and left of the gun, at its height: the
// magazine's approach and seat. (Armory rig (x, y, z) is world (4 - z, y, x).)
const atWell = { eye: [4.85, 1.12, -0.3], target: [4.38, 1.06, 0.05], fov: 30 };
// On the slide from behind the left shoulder, above.
const atSlideSide = { eye: [4.25, 1.32, -0.3], target: [4.4, 1.18, 0.05], fov: 30 };
// Above and left of the rifle's well and handle.
const atRifleWell = { eye: [4.75, 1.22, -0.3], target: [4.38, 1.1, 0.12], fov: 34 };
const atRifleHandle = { eye: [4.55, 1.45, -0.25], target: [4.3, 1.22, 0.15], fov: 32 };
// Over the bench from the left: the round thrown out to the right.
const atPort = { eye: [4.25, 1.38, -0.32], target: [4.42, 1.12, 0.1], fov: 40 };

// The pistol taken, held at RELOAD_HAND.
const pistolUp = [
  ...takePistol,
  { frames: 25, right: { to: RELOAD_HAND } },
  { frames: 6, right: {}, mark: "up" },
];
// The left hand takes a spare by its base out of its cradle.
const takeSpare = (at, half) => [
  { frames: 20, left: { to: add(spareBase(at, half), [0, 0.1, 0.1]) } },
  { frames: 10, left: { to: spareBase(at, half) } },
  { frames: 4, left: { squeeze: 1 }, mark: "tookSpare" },
  { frames: 20, left: { to: add(spareBase(at, half), [0, 0.15, 0.05]) } },
];
// To the mouth tipped 45° (no latch), held; turned lined up there (it
// latches), held; pushed home along the axis, past it by 1 cm (it seats).
const insert = [
  { frames: 25, left: { to: magHand(-WELL.drop - 0.05, tipped(45)), turn: tipped(45) } },
  { frames: 10, left: { to: magHand(-WELL.drop, tipped(45)) } },
  { frames: 10, left: {}, mark: "misaligned" },
  { frames: 10, left: { to: magHand(-WELL.drop), turn: WELL_TURN }, mark: "aligned" },
  { frames: 6, left: {}, mark: "latchedHeld" },
  { frames: 20, left: { to: magHand(0.01) }, mark: "seated" },
];
// The left hand off the seated magazine, to the slide where it rests (`rest`
// back), takes it, pulls it `to` back, lets go.
const rack = (rest, to, view) => [
  { frames: 1, left: { squeeze: 0 }, mark: "letGo" },
  { frames: 15, left: { to: add(magHand(0), [-0.06, 0.0, 0.03]), turn: [0, 0, 0, 1] }, ...(view ? { spectator: view } : {}) },
  { frames: 15, left: { to: slideAt(rest) } },
  { frames: 4, left: { squeeze: 1 }, mark: "onSlide" },
  { frames: 8, left: { to: slideAt(to) }, mark: "pulled" },
  { frames: 1, left: { squeeze: 0 }, mark: "racked" },
  { frames: 15, left: { to: add(slideAt(to), [-0.1, 0.05, 0.1]) } },
];

const reloadScenarios = {
  // The pistol emptied (12+1) and dry: locked back. The magazine dropped; a
  // spare from its cradle brought up tipped (refused), lined up (latched),
  // pushed home (seated, 12+0, still locked). The slide, nudged back from
  // locked and let go: home, chambered (11+1). Then 12 shots at the right
  // target, and the 13th pull dry: locked back again.
  "reload-pistol": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...pistolUp,
      ...pulls(PISTOL_ROUNDS),
      { frames: 1, right: { trigger: 1 }, mark: "dryLocked" },
      { frames: 6, right: { trigger: 0 } },
      ...dropMag(30, atWell),
      ...takeSpare(SPARE.pistol, MAG_BASE),
      ...insert,
      ...rack(SLIDE.lockBack, SLIDE.travel, atSlideSide),
      { frames: 15, left: { to: LEFT_OUT } },
      ...aimPistol(6).map((seg, i) => (i === 0 ? { ...seg, spectator: behindGun } : seg)),
      ...pulls(CAPACITY.pistol, "fired12"),
      { frames: 1, right: { trigger: 1 }, mark: "dryAgain" },
      { frames: 6, right: { trigger: 0 }, mark: "after" },
    ],
  },

  // A spare upside down at the mouth, pushed up the well: never latches.
  "reload-reversed": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atWell,
    steps: [
      ...pistolUp,
      ...dropMag(30),
      ...takeSpare(SPARE.pistol, MAG_BASE),
      { frames: 30, left: { to: magHand(-WELL.drop, tipped(180)), turn: tipped(180) }, mark: "atMouth" },
      { frames: 20, left: { to: magHand(0.01, tipped(180)) }, mark: "pushed" },
      { frames: 20, left: { to: magHand(-WELL.drop - 0.1, tipped(180)) }, mark: "out" },
    ],
  },

  // A rifle magazine lined up at the pistol's mouth, pushed: never latches.
  "reload-wrong": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atWell,
    steps: [
      ...pistolUp,
      ...dropMag(30),
      ...takeSpare(SPARE.rifle, RIFLE_BASE),
      { frames: 30, left: { to: add(wellPoint(-WELL.drop), qrot(WELL_TURN, [0, -RIFLE_BASE, 0])), turn: WELL_TURN }, mark: "atMouth" },
      { frames: 20, left: { to: add(wellPoint(0.01), qrot(WELL_TURN, [0, -RIFLE_BASE, 0])) }, mark: "pushed" },
    ],
  },

  // Latched, pushed half way in, then pulled out sideways (off the axis,
  // 1 cm a step): it stays latched to 7 cm, then leaves into the hand where
  // it is, easing back to how the hand holds it.
  "reload-cancel": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atWell,
    steps: [
      ...pistolUp,
      ...dropMag(30),
      ...takeSpare(SPARE.pistol, MAG_BASE),
      { frames: 25, left: { to: magHand(-WELL.drop - 0.03), turn: WELL_TURN } },
      { frames: 10, left: { to: magHand(-WELL.drop) }, mark: "latched" },
      { frames: 10, left: { to: magHand(-WELL.drop / 2) }, mark: "halfIn" },
      { frames: 12, left: { to: add(magHand(-WELL.drop / 2), qrot(WELL_TURN, [-0.12, 0, 0])) }, mark: "pulledOut" },
      { frames: 15, left: {}, mark: "settled" },
    ],
  },

  // Loaded (12+1): the slide pulled 70% of the way and let go: nothing.
  // Then all the way: the chambered round thrown out loose (a prop that
  // falls), the next chambered (11+1).
  "reload-rack": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atPort,
    steps: [
      ...pistolUp,
      { frames: 20, left: { to: add(slideAt(0), [-0.05, 0.05, 0.05]) } },
      { frames: 10, left: { to: slideAt(0) } },
      { frames: 4, left: { squeeze: 1 }, mark: "onSlide" },
      { frames: 8, left: { to: slideAt(0.7 * SLIDE.travel) }, mark: "partial" },
      { frames: 1, left: { squeeze: 0 }, mark: "partialLetGo" },
      { frames: 10, left: { to: slideAt(0) } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 8, left: { to: slideAt(SLIDE.travel) }, mark: "back" },
      { frames: 1, left: { squeeze: 0 }, mark: "racked" },
      { frames: 60, left: { to: LEFT_OUT }, mark: "landed" },
    ],
  },

  // Loaded: the slide racked eight times, back then home in the hand, and
  // let go: eight rounds thrown out, the oldest two taken away past the cap
  // of six; the six come to rest.
  "reload-rounds": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: atPort,
    steps: [
      ...pistolUp,
      { frames: 20, left: { to: add(slideAt(0), [-0.05, 0.05, 0.05]) } },
      { frames: 10, left: { to: slideAt(0) } },
      { frames: 4, left: { squeeze: 1 } },
      ...Array.from({ length: 8 }, () => [
        { frames: 6, left: { to: slideAt(SLIDE.travel) } },
        { frames: 6, left: { to: slideAt(0) } },
      ]).flat(),
      { frames: 1, left: { squeeze: 0 }, mark: "done" },
      { frames: 90, left: { to: LEFT_OUT }, mark: "rest" },
    ],
  },

  // The demo, close on the well and the slide: the pistol emptied (locked
  // back), its magazine dropped; a spare brought up upside down (refused),
  // turned the right way but tipped 45° (refused), lined up (latched),
  // pushed home; the slide racked from locked; three shots; racked again
  // with a round chambered (thrown out loose); holstered. Then the rifle:
  // its magazine dropped, a spare seated, the charging handle racked (its
  // chambered round thrown out), a shot.
  "reload-tour": {
    station: 3,
    neck: NECK,
    spectator: atSlideSide,
    steps: [
      ...pistolUp,
      ...pulls(PISTOL_ROUNDS).map((seg) => (seg.frames === 4 ? { ...seg, frames: 6 } : seg)),
      { frames: 1, right: { trigger: 1 } },
      { frames: 20, right: { trigger: 0 }, mark: "lockedBack" },
      ...dropMag(30, atWell),
      ...takeSpare(SPARE.pistol, MAG_BASE).map((seg, i) => (i === 0 ? { ...seg, spectator: atSpares } : seg)),
      { frames: 30, left: { to: magHand(-WELL.drop, tipped(180)), turn: tipped(180) }, spectator: atWell },
      { frames: 20, left: { to: magHand(-WELL.drop + 0.04, tipped(180)) }, mark: "reversed" },
      { frames: 20, left: { to: magHand(-WELL.drop - 0.06, tipped(180)) } },
      { frames: 30, left: { to: magHand(-WELL.drop - 0.05, tipped(45)), turn: tipped(45) } },
      { frames: 15, left: { to: magHand(-WELL.drop, tipped(45)) } },
      { frames: 15, left: {}, mark: "tipped" },
      { frames: 15, left: { to: magHand(-WELL.drop), turn: WELL_TURN } },
      { frames: 15, left: {}, mark: "latched" },
      { frames: 30, left: { to: magHand(0.01) }, mark: "seated" },
      { frames: 10, left: {} },
      ...rack(SLIDE.lockBack, SLIDE.travel, atSlideSide).map((seg) => (seg.frames === 8 ? { ...seg, frames: 12 } : seg)),
      { frames: 15, left: { to: LEFT_OUT } },
      ...aimPistol(6).map((seg, i) => (i === 0 ? { ...seg, spectator: behindGun } : seg)),
      ...pulls(3).map((seg) => (seg.frames === 4 ? { ...seg, frames: 10 } : seg)),
      { frames: 25, right: { to: RELOAD_HAND, turn: [0, 0, 0, 1] }, spectator: atPort },
      { frames: 20, left: { to: add(slideAt(0), [-0.05, 0.05, 0.05]) } },
      { frames: 10, left: { to: slideAt(0) } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 12, left: { to: slideAt(SLIDE.travel) }, mark: "thrown" },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 50, left: { to: LEFT_OUT } },
      { frames: 30, right: { to: [0.3, 1.0, -0.1] }, spectator: atArmoryHip },
      { frames: 25, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 10, right: {} },
      { frames: 20, right: { to: [0.5, 1.15, -0.35] }, spectator: atRifleWell },
      { frames: 15, right: { to: RIFLE_GRIP }, fists: { right: RIFLE_FIST } },
      { frames: 6, right: { squeeze: 1 } },
      { frames: 30, right: { to: RIFLE_HAND } },
      { frames: 6, right: {} },
      ...dropMag(30),
      ...takeSpare(SPARE.rifle, RIFLE_BASE),
      { frames: 25, left: { to: rifleMagHand(-RIFLE_WELL.drop - 0.04) } },
      { frames: 15, left: { to: rifleMagHand(-RIFLE_WELL.drop) } },
      { frames: 30, left: { to: rifleMagHand(0.01) }, mark: "rifleSeated" },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: add(rifleMagHand(0), [-0.08, 0.05, 0]) }, spectator: atRifleHandle },
      { frames: 20, left: { to: handleAt(0) } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 15, left: { to: handleAt(CHARGING.travel) }, mark: "charged" },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 40, left: { to: LEFT_OUT } },
      { frames: 20, right: { to: AIM_HAND, turn: AIM_RIGHT }, spectator: behindRifle },
      { frames: 6, right: {} },
      { frames: 1, right: { trigger: 1 } },
      { frames: 20, right: { trigger: 0 } },
    ],
  },

  // The rifle off its rack in the right hand alone: the thumbstick drops its
  // magazine (one chambered); a rifle spare pushed into its well (seated);
  // the charging handle pulled all the way back and let go: the chambered
  // round thrown out, the next chambered (29+1). Then it fires.
  "reload-rifle": {
    fists: { right: RIFLE_FIST },
    station: 3,
    neck: NECK,
    spectator: atRifleWell,
    steps: [
      { frames: 15, left: { to: LEFT_OUT } },
      { frames: 20, right: { to: [0.5, 1.15, -0.35] } },
      { frames: 15, right: { to: RIFLE_GRIP } },
      { frames: 6, right: { squeeze: 1 }, mark: "tookRifle" },
      { frames: 30, right: { to: RIFLE_HAND } },
      { frames: 6, right: {} },
      ...dropMag(30),
      ...takeSpare(SPARE.rifle, RIFLE_BASE),
      { frames: 25, left: { to: rifleMagHand(-RIFLE_WELL.drop - 0.04) } },
      { frames: 10, left: { to: rifleMagHand(-RIFLE_WELL.drop) }, mark: "aligned" },
      { frames: 20, left: { to: rifleMagHand(0.01) }, mark: "seated" },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: add(rifleMagHand(0), [-0.08, 0.05, 0]) }, spectator: atRifleHandle },
      { frames: 20, left: { to: handleAt(0) } },
      { frames: 4, left: { squeeze: 1 }, mark: "onHandle" },
      { frames: 10, left: { to: handleAt(CHARGING.travel) }, mark: "pulled" },
      { frames: 1, left: { squeeze: 0 }, mark: "racked" },
      { frames: 20, left: { to: LEFT_OUT } },
      { frames: 20, right: { to: AIM_HAND, turn: AIM_RIGHT }, spectator: behindRifle },
      { frames: 6, right: {} },
      { frames: 1, right: { trigger: 1 }, mark: "fired" },
      { frames: 6, right: { trigger: 0 } },
    ],
  },
};

// --- Armory: the shotgun (station 3) ---
// A pump shotgun racked between the pistol and the rifle at SHOTGUN.rack,
// its pistol grip's fist SHOTGUN.grip from its centre. Held like the rifle:
// the bore 6 cm above the primary fist's -Z axis, the support fist on that
// axis, on the fore-end 28-42 cm ahead with the pump home; it slides 9 cm
// back with the pump. Loaded empty: shells come from the box on the bench,
// into the loading port 12 cm ahead of the fist under the receiver (seated
// 4.5 cm above the fist's axis, the mouth 6 cm below that). Eight pellets a
// shot: one along the bore, seven on a 1.2° cone round it, the ring turned
// by the golden angle a shot (`Gun.pelletDirs`).

export const SHOTGUN = {
  rack: [0.35, 1.01, -0.62], grip: [0, -0.0475, 0.16], fore: 0.3, travel: 0.09, port: { at: [0, 0.045, -0.12], drop: 0.06 },
  box: [0.0, 0.95, -0.7], tube: 6, pellets: 8, spread: (1.2 * Math.PI) / 180, turn: Math.PI * (3 - Math.sqrt(5)),
};
const SHOTGUN_GRIP = add(SHOTGUN.rack, SHOTGUN.grip);
// Held level for loading; raised to aim.
export const SG_HAND = [0.15, 1.15, -0.35];
export const SG_AIM = [0.2, 1.3, -0.3];
// A point `d` along the level shotgun's loading port (0 seated).
const sgPort = (d) => add(SG_HAND, [0, SHOTGUN.port.at[1] + d, SHOTGUN.port.at[2]]);
// The shells a run reads from `/state`: live ones (loose or held) and hulls.
export const shellsOf = (m) => m.props.filter((p) => p.kind.$ctor === "Props.Round" && p.kind.args[0].$ctor === "Ammo.Shell");
export const hullsOf = (m) => m.props.filter((p) => p.kind.$ctor === "Props.Hull");
export const tubeOf = (m) => roundsIn(m.shotgun.ammo.mag);

// (Armory rig (x, y, z) is world (4 - z, y, x).) Ahead, left and low: the
// loading port under the receiver, and the shell box.
const atLoadingPort = { eye: [4.95, 1.02, -0.3], target: [4.45, 1.1, 0.1], fov: 36 };
// Left of the fore-end, a little above: the pump, level.
const atPump = { eye: [4.6, 1.38, -0.38], target: [4.62, 1.17, 0.13], fov: 34 };
// The same, the shotgun aimed.
const atPumpAimed = { eye: [4.5, 1.5, -0.32], target: [4.62, 1.33, 0.18], fov: 34 };
// Down range, close on the middle target: the pellets' marks.
const atMiddleTarget = { eye: [8.4, 1.55, 0.55], target: [9.5, 1.45, 0.0], fov: 18 };

const takeShotgun = [
  { frames: 15, left: { to: LEFT_OUT }, fists: { right: SHOTGUN_FIST } },
  { frames: 20, right: { to: [0.35, 1.1, -0.3] } },
  { frames: 15, right: { to: SHOTGUN_GRIP }, mark: "reachedShotgun" },
  { frames: 6, right: { squeeze: 1 }, mark: "tookShotgun" },
  { frames: 30, right: { to: SG_HAND } },
  { frames: 6, right: {}, mark: "up" },
];
// A shell out of the box, brought up under the port to its mouth (it
// latches), pushed home (into the tube), the hand opened and lowered.
const sgLoad = (mark, view) => [
  { frames: 20, left: { to: add(SHOTGUN.box, [0, 0.08, 0.06]), turn: [0, 0, 0, 1] }, ...(view ? { spectator: view } : {}) },
  { frames: 10, left: { to: SHOTGUN.box } },
  { frames: 4, left: { squeeze: 1 } },
  { frames: 15, left: { to: sgPort(-SHOTGUN.port.drop - 0.08) } },
  { frames: 10, left: { to: sgPort(-SHOTGUN.port.drop) } },
  { frames: 12, left: { to: sgPort(0.01) }, ...(mark ? { mark } : {}) },
  { frames: 1, left: { squeeze: 0 } },
  { frames: 8, left: { to: sgPort(-SHOTGUN.port.drop - 0.06) } },
];
// The support fist `d` ahead of a level primary fist at `at`, or of one
// aimed (by the support) so the shot passes through `target`.
const sgFore = (d, at = SG_HAND, target) => (target ? supportFor(at, target, d) : aheadOf(at, ID, d));
// The fore-end taken (the pump home), level.
const takeFore = [
  { frames: 15, left: { to: sgFore(SHOTGUN.fore - 0.02) } },
  { frames: 6, left: { to: sgFore(SHOTGUN.fore) } },
  { frames: 4, left: { squeeze: 1 }, mark: "onFore" },
];
// A stroke: back the whole travel and home, along the aim.
const sgPump = (mark, at = SG_HAND, target, frames = 6) => [
  { frames, left: { to: sgFore(SHOTGUN.fore - SHOTGUN.travel, at, target) }, ...(mark ? { mark: `${mark}Back` } : {}) },
  { frames, left: { to: sgFore(SHOTGUN.fore, at, target) }, ...(mark ? { mark } : {}) },
];
// Raised, both hands, aimed at the middle target by the support hand.
const sgRaise = [
  { frames: 30, right: { to: SG_AIM }, left: { to: aheadOf(SG_AIM, ID, SHOTGUN.fore) } },
  { frames: 20, left: { to: sgFore(SHOTGUN.fore, SG_AIM, TARGETS[1]) } },
  { frames: 10, left: {}, mark: "aimed" },
];

const shotgunScenarios = {
  // Out of its rack, six shells loaded (6+0), the pump worked (5+1), a
  // seventh loaded (6+1); an eighth is refused (the tube full) and set down
  // on the bench. Raised, aimed at the middle target: seven shots, each
  // pumped (the hull out, the next fed); the next pull clicks dry.
  "shotgun-loop": {
    fists: { right: SHOTGUN_FIST },
    station: 3,
    neck: NECK,
    spectator: atLoadingPort,
    steps: [
      ...takeShotgun,
      ...Array.from({ length: SHOTGUN.tube }, (_, i) => sgLoad(`loaded${i + 1}`)).flat(),
      ...takeFore,
      ...sgPump("pumped"),
      { frames: 1, left: { squeeze: 0 }, mark: "letGoFore" },
      ...sgLoad("loaded7"),
      // The eighth: to the mouth and held there (never latching), then set down.
      ...sgLoad(undefined).slice(0, 5),
      { frames: 10, left: {}, mark: "refused" },
      { frames: 20, left: { to: [-0.1, 0.93, -0.82] } },
      { frames: 1, left: { squeeze: 0 }, mark: "setDown" },
      { frames: 15, left: { to: [-0.1, 1.05, -0.82] } },
      ...takeFore,
      ...sgRaise,
      ...Array.from({ length: SHOTGUN.tube + 1 }, (_, i) => [...pulls(1, `fired${i + 1}`), ...sgPump(`pumped${i + 1}`, SG_AIM, TARGETS[1])]).flat(),
      { frames: 1, right: { trigger: 1 }, mark: "dry" },
      { frames: 6, right: { trigger: 0 }, mark: "after" },
      { frames: 120, right: {}, mark: "rested" },
    ],
  },

  // Two shells in; pumped (1+1); pumped again with the shell chambered: it is
  // thrown out live, the next fed (0+1). The left hand picks it up where it
  // came to rest, turns it to lie along the bore, and loads it (1+1).
  "shotgun-eject": {
    fists: { right: SHOTGUN_FIST },
    station: 3,
    neck: NECK,
    spectator: atLoadingPort,
    steps: [
      ...takeShotgun,
      ...sgLoad("loaded1"),
      ...sgLoad("loaded2"),
      ...takeFore,
      ...sgPump("pumped"),
      ...sgPump("thrown"),
      { frames: 1, left: { squeeze: 0 } },
      { frames: 60, left: { to: [0.0, 1.1, -0.3] }, mark: "landed" },
      // To where it lies, from above, taken; lifted and turned so it lies
      // along the bore, head back, the way the port takes it.
      { frames: 20, left: { to: (s) => add(xyz(shellsOf(s.model)[0].pose.position), [0, 0.08, 0]) } },
      { frames: 10, left: { to: (s) => xyz(shellsOf(s.model)[0].pose.position) } },
      { frames: 4, left: { squeeze: 1 }, mark: "pickedUp" },
      // Straight up first, clear of the bench's edge.
      { frames: 25, left: { to: (s) => { const [x, , z] = xyz(shellsOf(s.model)[0].pose.position); return [x, 1.0, z]; } } },
      { frames: 25, left: { to: sgPort(-SHOTGUN.port.drop - 0.08), turn: (s) => qconj(quat(some(shellsOf(s.model)[0].held).accepted.orientation)) } },
      { frames: 10, left: { to: sgPort(-SHOTGUN.port.drop) } },
      { frames: 12, left: { to: sgPort(0.01) }, mark: "reloaded" },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 10, left: { to: LEFT_OUT } },
    ],
  },

  // The demo, close on the loading port and the pump: out of its rack,
  // three shells loaded; the fore-end taken and pumped (2+1); raised and
  // aimed at the middle target: a shot (the pellets' marks on the target),
  // pumped (the hull out), a second shot (the pattern turned), pumped, a
  // third; then the next pull dry.
  "shotgun-tour": {
    fists: { right: SHOTGUN_FIST },
    station: 3,
    neck: NECK,
    spectator: atArmoryHip,
    steps: [
      ...takeShotgun.map((seg, i) => (i === 4 ? { ...seg, spectator: atLoadingPort } : seg)),
      ...sgLoad("loaded1"),
      ...sgLoad("loaded2"),
      ...sgLoad("loaded3"),
      ...takeFore.map((seg, i) => (i === 0 ? { ...seg, spectator: atPump } : seg)),
      { frames: 10, left: {} },
      ...sgPump("pumped", SG_HAND, undefined, 14),
      { frames: 15, left: {} },
      ...sgRaise.map((seg, i) => (i === 0 ? { ...seg, spectator: behindRifle } : seg)),
      { frames: 1, right: { trigger: 1 }, spectator: atMiddleTarget },
      { frames: 50, right: { trigger: 0 }, mark: "spread" },
      { frames: 10, left: {}, spectator: atPumpAimed },
      ...sgPump(undefined, SG_AIM, TARGETS[1], 12),
      { frames: 20, left: {} },
      { frames: 1, right: { trigger: 1 }, spectator: atMiddleTarget },
      { frames: 50, right: { trigger: 0 } },
      { frames: 10, left: {}, spectator: behindRifle },
      ...sgPump(undefined, SG_AIM, TARGETS[1], 12),
      { frames: 10, left: {} },
      { frames: 1, right: { trigger: 1 } },
      { frames: 30, right: { trigger: 0 } },
      { frames: 1, right: { trigger: 1 }, mark: "dry" },
      { frames: 30, right: { trigger: 0 } },
    ],
  },
};

// --- Hands drawn on what they hold (station 3) ---
// A point in the frame of the pistol held by a hand at `at` turned `q`
// (`Gun`'s, centred on its collider), and its slide `back` behind home.
const onPistol = (at, q, local) => add(at, qrot(q, add(PISTOL_FROM_GRIP, local)));
const slideOn = (at, q, back) => onPistol(at, q, add(SLIDE.at, [0, 0, back]));
// The hands 20 cm through the wall (the pistol's), and the rifle's 21 cm.
const PISTOL_DEEP = [-0.65, 1.2, -0.5];
const RIFLE_DEEP = [-0.25, 1.2, -0.45];
// Down range, side on to the wall (edge-on, on the right): the guns in it.
const besideWall = { eye: [5.2, 1.35, -0.3], target: [4.5, 1.2, -0.5], fov: 34 };
// Down range of the pistol held clear of the wall: the slide racked.
const overSlide = { eye: [5.1, 1.5, -0.3], target: [4.5, 1.22, -0.33], fov: 40 };

const handViewScenarios = {
  // The pistol pushed into the wall, the hand 20 cm on: the hand drawn on
  // its grip at the wall, its ghost through it. Clear of it, the left hand
  // racks the slide 8 cm past its travel: drawn at the slide's end, its ghost
  // behind. Holstered; the rifle taken in two hands and pushed into the
  // wall: both hands drawn on its grips, their ghosts through it.
  "hand-view-guns": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: besideWall,
    steps: [
      ...takePistol,
      { frames: 25, right: { to: WALL_NEAR, turn: LEFT } },
      { frames: 30, right: { to: PISTOL_DEEP } },
      { frames: 30, right: {}, mark: "pistolHeld" },
      { frames: 20, right: { to: CLEAR }, spectator: overSlide },
      { frames: 20, left: { to: add(slideOn(CLEAR, LEFT, 0), [0, 0.06, 0.06]), turn: LEFT } },
      { frames: 10, left: { to: slideOn(CLEAR, LEFT, 0) } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 12, left: { to: slideOn(CLEAR, LEFT, SLIDE.travel + 0.08) } },
      { frames: 30, left: {}, mark: "rackHeld" },
      { frames: 12, left: { to: slideOn(CLEAR, LEFT, 0) } },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 20, left: { to: LEFT_OUT, turn: ID } },
      { frames: 30, right: { to: [0.3, 1.0, -0.1], turn: ID }, spectator: besideRifleWall },
      { frames: 25, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 0 } },
      ...takeRifle,
      raise(),
      { frames: 30, ...bothTo(RIFLE_WALL.near) },
      { frames: 30, ...bothTo(RIFLE_DEEP) },
      { frames: 40, right: {}, mark: "rifleHeld" },
      { frames: 20, ...bothTo(RIFLE_WALL.clear) },
    ],
  },
};

// --- Hands (station 1, and the armory's pistol) ---
// Both hands up in front of the player, palms down (each turned a quarter
// turn about the way it points) and a little yawed in, 20 cm apart.
const HANDS = {
  right: { at: [0.1, 1.3, -0.32], turn: qmul(yaw(10), roll(-90)) },
  left: { at: [-0.1, 1.3, -0.32], turn: qmul(yaw(-10), roll(90)) },
};
// Ahead of the player, to the right and above: both hands' curl.
const atHands = { eye: [-0.42, 1.55, 4.88], target: [0.0, 1.29, 4.36], fov: 27 };
// Both hands' `field` (squeeze or trigger) stepped from `from` to `to` over
// `n` steps; `mark` names the last.
const both = (field, from, to, n = 12, mark) =>
  Array.from({ length: n }, (_, i) => {
    const v = from + ((to - from) * (i + 1)) / n;
    return { frames: 1, right: { [field]: v }, left: { [field]: v }, ...(mark && i === n - 1 ? { mark } : {}) };
  });

const handScenarios = {
  // Both hands raised, open; squeezed (the middle, ring and little fingers
  // close round the handle); let open and the trigger pulled (the index
  // closes); let out.
  "hand-curl": {
    station: 1,
    spectator: atHands,
    steps: [
      { frames: 20, right: { to: HANDS.right.at, turn: HANDS.right.turn }, left: { to: HANDS.left.at, turn: HANDS.left.turn } },
      { frames: 4, right: {}, left: {}, mark: "open" },
      ...both("squeeze", 0, 1, 12, "squeezed"),
      ...both("squeeze", 1, 0, 12),
      ...both("trigger", 0, 1, 12, "pulled"),
      ...both("trigger", 1, 0, 12, "released"),
    ],
  },

  // Both hands raised, open; squeezed with the trigger pulled (a fist); the
  // squeeze let open (the index alone closed). Then G: the capsule glove;
  // and G again: the skinned one.
  "skin-curl": {
    station: 1,
    spectator: atHands,
    steps: [
      { frames: 20, right: { to: HANDS.right.at, turn: HANDS.right.turn }, left: { to: HANDS.left.at, turn: HANDS.left.turn } },
      { frames: 4, right: {}, left: {}, mark: "open" },
      ...Array.from({ length: 12 }, (_, i) => {
        const v = (i + 1) / 12;
        return { frames: 1, right: { squeeze: v, trigger: v }, left: { squeeze: v, trigger: v }, ...(i === 11 ? { mark: "fist" } : {}) };
      }),
      ...both("squeeze", 1, 0, 12, "trigger"),
      { frames: 1, right: {}, left: {}, tap: "G", mark: "capsule" },
      { frames: 1, right: {}, left: {}, tap: "G", mark: "skinned" },
    ],
  },

  // The pistol taken and aimed low, the trigger let in by steps: the index
  // follows it.
  "pistol-trigger": {
    fists: { right: PISTOL_FIST },
    station: 3,
    neck: NECK,
    spectator: behindGun,
    steps: [
      ...takePistol,
      { frames: 25, right: { to: AIM_HAND, turn: AIM_MISS } },
      { frames: 6, right: {}, mark: "trigger0" },
      { frames: 3, right: { trigger: 0.5 }, mark: "trigger50" },
      { frames: 3, right: { trigger: 1 }, mark: "trigger100" },
      { frames: 3, right: { trigger: 0 }, mark: "triggerOut" },
    ],
  },
};

// --- The hands tour (the demo) ---
// Station 1 (world = (-x, y, 4 - z)): close on the ball, from the player's
// right; on the stick, from the left; the bottle, `atBottle`.
const atBall = { eye: [-0.72, 1.32, 4.08], target: [-0.2, 1.05, 4.5], fov: 34 };
const atStick = { eye: [-0.3, 1.5, 3.85], target: [0.3, 1.02, 4.38], fov: 36 };
// The armory (world = (4 - z, y, x)): the pistol hand from its right, behind;
// the shell box from the left; the rifle in two hands from its right.
const atPistolHand = { eye: [4.05, 1.48, 0.62], target: [4.38, 1.28, 0.2], fov: 34 };
const atShellBox = { eye: [4.2, 1.45, -0.5], target: [4.62, 1.12, 0.0], fov: 44 };
const atRifleHands = { eye: [4.3, 1.5, 0.75], target: [4.45, 1.28, 0.18], fov: 40 };
// The ball's right side, the palm on it; the stick's axis under its grip
// band, the hand pointing down along it; the upright bottle's neck.
const BALL_SIDE = add(BALL, [0.045, 0.02, 0]);
const STICK_AXIS = [-0.3, 0.9175, -0.32];
const DOWN_ALONG = pitch(-90);
const hold = (frames, mark) => ({ frames, right: {}, left: {}, ...(mark ? { mark } : {}) });

const tourScenarios = {
  // Both hands close up: open, squeezed shut and open again, the trigger
  // pulled and let out. Then held props, each drawn on what it holds: the
  // ball in an open palm, the stick and the bottle's neck wrapped. At the
  // armory: the pistol, its index following the trigger; a shell pinched
  // from the box; the shotgun and the rifle in two hands (the support hand
  // wrapped round the fore-end and foregrip); the rifle pushed into the wall,
  // the hands' ghosts at the tracked hands.
  "hands-tour": {
    station: 1,
    neck: NECK,
    spectator: atHands,
    steps: [
      { frames: 20, right: { to: HANDS.right.at, turn: HANDS.right.turn }, left: { to: HANDS.left.at, turn: HANDS.left.turn } },
      hold(10, "open"),
      ...both("squeeze", 0, 1, 20, "squeezed"),
      hold(15),
      ...both("squeeze", 1, 0, 20),
      ...both("trigger", 0, 1, 20, "pulled"),
      hold(15),
      ...both("trigger", 1, 0, 20),
      { frames: 25, right: { to: add(BALL_SIDE, [0.06, 0.08, 0.05]), turn: ID }, left: { to: [-0.25, 0.95, -0.1], turn: ID }, spectator: atBall },
      { frames: 12, right: { to: BALL_SIDE } },
      { frames: 4, right: { squeeze: 1 } },
      { frames: 20, right: { to: add(BALL_SIDE, [0, 0.15, 0.05]) }, mark: "ball" },
      { frames: 15, right: {} },
      { frames: 20, right: { to: add(BALL_SIDE, [0, 0.003, 0]) } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 20, right: { to: add(STICK_AXIS, [0.05, 0.12, 0]), turn: DOWN_ALONG }, spectator: atStick },
      { frames: 12, right: { to: STICK_AXIS } },
      { frames: 4, right: { squeeze: 1 } },
      { frames: 20, right: { to: add(STICK_AXIS, [0, 0.2, 0]) }, mark: "stick" },
      { frames: 15, right: {} },
      { frames: 20, right: { to: add(STICK_AXIS, [0, 0.003, 0]) } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 30, right: { to: [0.6, 1.25, -0.6], turn: ID }, spectator: atBottle },
      { frames: 15, right: { to: neckGrip } },
      { frames: 4, right: { squeeze: 1 } },
      { frames: 20, right: { to: add(neckGrip, [-0.05, 0.12, 0.05]) }, mark: "bottle" },
      { frames: 15, right: {} },
      { frames: 20, right: { to: neckGrip } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 1, right: { to: RIGHT_REST }, tap: "3", spectator: atPistolHand },
      ...takePistol,
      { frames: 25, right: { to: AIM_HAND, turn: AIM_MIDDLE } },
      { frames: 20, right: { trigger: 0.4 }, mark: "pistol" },
      { frames: 1, right: { trigger: 1 } },
      { frames: 15, right: {} },
      { frames: 10, right: { trigger: 0 } },
      { frames: 30, right: { to: [0.3, 1.0, -0.1], turn: ID } },
      { frames: 25, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 20, right: { to: RIGHT_REST }, left: { to: add(SHOTGUN.box, [0, 0.08, 0.06]), turn: ID }, spectator: atShellBox },
      { frames: 10, left: { to: SHOTGUN.box } },
      { frames: 4, left: { squeeze: 1 } },
      { frames: 20, left: { to: add(SHOTGUN.box, [-0.05, 0.2, 0.15]) }, mark: "shell" },
      { frames: 15, left: {} },
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: LEFT_OUT }, spectator: atPump },
      ...takeShotgun,
      ...takeFore,
      { frames: 10, left: {} },
      ...sgPump(undefined, SG_HAND, undefined, 14),
      hold(15, "shotgun"),
      { frames: 1, left: { squeeze: 0 } },
      { frames: 15, left: { to: LEFT_OUT } },
      { frames: 30, right: { to: add(SHOTGUN_GRIP, [0, 0.06, 0.08]) } },
      { frames: 15, right: { to: SHOTGUN_GRIP } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 15, right: { to: [0.45, 1.15, -0.3] }, spectator: atRifleHands },
      ...takeRifle,
      raise(),
      hold(20, "rifle"),
      { frames: 30, ...bothTo(RIFLE_WALL.near), spectator: besideWall },
      { frames: 30, ...bothTo(RIFLE_DEEP) },
      hold(40, "release"),
      { frames: 20, ...bothTo(RIFLE_WALL.clear) },
      hold(10),
    ],
  },

  // The skinned glove (the default) and the capsule glove, G switching
  // between them: both hands open, closed and the trigger pulled; then the
  // pistol held and fired, and the rifle in two hands.
  "skin-tour": {
    station: 1,
    neck: NECK,
    spectator: atHands,
    steps: [
      { frames: 20, right: { to: HANDS.right.at, turn: HANDS.right.turn }, left: { to: HANDS.left.at, turn: HANDS.left.turn } },
      hold(15, "open"),
      ...both("squeeze", 0, 1, 16, "squeezed"),
      hold(10),
      ...both("squeeze", 1, 0, 16),
      ...both("trigger", 0, 1, 16, "pulled"),
      hold(10),
      ...both("trigger", 1, 0, 16),
      { ...hold(15, "openCapsule"), tap: "G" },
      ...both("squeeze", 0, 1, 16, "squeezedCapsule"),
      hold(10),
      ...both("squeeze", 1, 0, 16),
      { ...hold(10), tap: "G" },
      { frames: 1, right: { to: RIGHT_REST }, tap: "3", spectator: atPistolHand },
      ...takePistol,
      { frames: 25, right: { to: AIM_HAND, turn: AIM_MIDDLE } },
      { frames: 20, right: { trigger: 0.4 }, mark: "pistol" },
      { frames: 1, right: { trigger: 1 } },
      { frames: 15, right: {} },
      { frames: 10, right: { trigger: 0 } },
      { ...hold(20, "pistolCapsule"), tap: "G" },
      { ...hold(15), tap: "G" },
      { frames: 30, right: { to: [0.3, 1.0, -0.1], turn: ID } },
      { frames: 25, right: { to: ARMORY_RH } },
      { frames: 1, right: { squeeze: 0 } },
      { frames: 15, right: { to: [0.45, 1.15, -0.3] }, spectator: atRifleHands },
      ...takeRifle,
      raise(),
      hold(20, "rifle"),
      { ...hold(20, "rifleCapsule"), tap: "G" },
      { ...hold(20), tap: "G" },
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

  // An upright hand takes the bottle 2 cm off its neck: it snaps there.
  "bottle-neck": {
    station: 1,
    spectator: atBottle,
    steps: [
      { frames: 20, right: { to: [0.6, 1.25, -0.6] } },
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

  // Push the held ball into the wall, the hand going on through it: the
  // hand is drawn at the ball, with a ghost where it really is. Brought back
  // to within a centimetre of the ball the ghost goes; pushed through again
  // it returns. H draws it where it is tracked, through the wall; H again,
  // back at the ball.
  "hand-view": {
    station: 1,
    spectator: overWall,
    steps: [
      ...grab,
      { frames: 20, right: { to: [-0.3, 1.15, -0.55] } },
      { frames: 30, right: { to: throughWall }, mark: "blockedAccepted" },
      { frames: 20, right: { to: atWall }, mark: "nearWall" },
      { frames: 20, right: { to: [-0.4, 1.15, -0.55] }, mark: "out" },
      { frames: 25, right: { to: throughWall }, mark: "blockedAgain" },
      { frames: 20, right: {}, tap: "h", mark: "blockedTracked" },
      { frames: 20, right: {}, tap: "h", mark: "acceptedAgain" },
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

  // The haptics demo at the grab table: grab, a stop at the wall, the grip
  // broken by dragging on, then the ball taken again and thrown. Each pulse
  // draws a ring at the hand (`Pulses.cue`).
  "haptics-table": {
    station: 1,
    spectator: aboveWall,
    steps: [
      ...grab,
      { frames: 20, right: { to: [-0.3, 1.15, -0.55] } },
      { frames: 25, right: { to: [-0.72, 1.15, -0.55] }, mark: "blocked" },
      { frames: 10, right: {} },
      { frames: 20, right: { to: [-1.0, 1.1, -0.55] }, mark: "broke" },
      { frames: 6, right: { squeeze: 0 } },
      { frames: 25, right: { to: [-0.45, 1.05, -0.55] } },
      { frames: 15, right: { to: [-0.536, 0.96, -0.55] } },
      { frames: 6, right: { squeeze: 1 }, mark: "regrabbed" },
      { frames: 30, right: { to: SWEEP_FROM } },
      { frames: 6, right: { to: along(6) } },
      { frames: 1, right: { to: along(7), squeeze: 0 }, mark: "release" },
      { frames: 50, right: { to: [0.25, 1.2, -0.45] } },
    ],
  },

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
  ...armoryScenarios,
  ...rifleScenarios,
  ...ammoScenarios,
  ...reloadScenarios,
  ...shotgunScenarios,
  ...handViewScenarios,
  ...handScenarios,
  ...tourScenarios,
};
