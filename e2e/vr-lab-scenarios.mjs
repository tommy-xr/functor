// examples/vr-lab's scripted XR scenarios: played by its e2e checks
// (`e2e/vr-lab.mjs`) and recorded as demo videos (`npm run demo:vr-lab`).
// A new station adds its scenarios here. Format: `e2e/xr-scenario.mjs`.
//
// Grab table (station 1): the rig stands at world (0, 0, 4) facing +Z, so
// rig-local x is world -x and rig-local -z is world +z. The ball rests at
// rig (0.2, 0.96, -0.55); the wall's face is at rig x = -0.7.

export const project = "examples/vr-lab";

export const grabTable = { feet: [0, 0, 4], yaw: 0 };
export const BALL = [0.2, 0.96, -0.55];

// The throw: a straight, steady sweep forward and up, released mid-sweep.
export const THROW_V = [0, 2.0, -3.6];
const SWEEP_FROM = [0.22, 1.2, -0.25];
const along = (n) => SWEEP_FROM.map((v, i) => v + (THROW_V[i] * n) / 60);

// From the player's right, level with the table: the hands, the wall
// behind them and the basket down range all in frame.
const side = { eye: [-2.1, 2.2, 2.7], target: [0.05, 0.85, 5.5], fov: 55 };

// From above the wall, in its plane: the ball held on this side while the
// hand goes through to the other.
const alongWall = { eye: [0.75, 2.45, 3.75], target: [0.7, 1.05, 4.6], fov: 50 };

const grab = [
  { frames: 20, right: { to: [0.2, 1.05, -0.45], squeeze: 0 } },
  { frames: 20, right: { to: BALL, squeeze: 0 }, mark: "reached" },
  { frames: 6, right: { squeeze: 1 }, mark: "grabbed" },
];

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
      { frames: 25, right: { to: [-0.85, 1.15, -0.55] }, mark: "pushed" },
      { frames: 20, right: { to: [-0.4, 1.15, -0.55] }, mark: "pulled" },
      { frames: 20, right: { to: [-0.85, 1.15, -0.55] }, mark: "pushedAgain" },
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
      { frames: 30, right: { to: [-1.1, 1.1, -0.55] }, mark: "broke" },
      { frames: 30, right: { to: [-0.55, 1.0, -0.5] }, mark: "returned" },
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
};
