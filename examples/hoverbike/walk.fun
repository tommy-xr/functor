// walk.fun — on foot: the walker's capsule, walking, gravity and looking, as
// pure functions.
//
// `game.fun` makes the walker a `Physics.character` and feeds `step` what the
// world says (grounded, ground in reach); `Physics.move` carries out the
// velocity. Points in the walker's frame follow the bike's convention (+x
// left, +y up, +z forward), from the capsule centre.

let halfHeight = 0.6
let radius = 0.3
// Capsule centre to feet: 0.9 m, plus the character's 2 cm offset.
let centreHeight = halfHeight + radius + 0.02
// The eye, 1.65 m above the feet.
let eye: Input.point3 = { x: 0.0, y: 0.75, z: 0.0 }
let speed = 1.4 // m/s, a stroll; no head bob
let gravity = Ride.gravity
// The ground counts as in reach within this of the capsule centre: walking
// waits for it after every spawn, as the bike waits for its probes.
let senseRange = 1.4
let lookLimit = 1.4 // rad, up or down
let mouseSensitivity = 0.003 // rad per pixel

type State = { yaw: float, pitch: float, vy: float, ready: bool }
let standing: State = { yaw: 0.0, pitch: 0.0, vy: 0.0, ready: false }

// What the walker asks for this step: forward and right, -1..1, along the
// look (`heading` from the walker's yaw: the head's turn, with a headset),
// and the right stick's turn (left positive), for snap turns.
type Wish = { forward: float, right: float, heading: float, turn: float }
let still: Wish = { forward: 0.0, right: 0.0, heading: 0.0, turn: 0.0 }

// One step: the next state and the velocity to move by. Until the ground is
// in reach the walker holds still; then gravity pulls it down whenever the
// last move didn't end on the ground.
let step = (grounded: bool, sensed: bool, w: Wish, dt: float, s: State): (State, Input.point3) =>
  let ready = s.ready || sensed in
  if not ready then (s, { x: 0.0, y: 0.0, z: 0.0 })
  else
    let vy = (if grounded && s.vy <= 0.0 then 0.0 else s.vy) - gravity * dt in
    // A diagonal is no faster than straight.
    let len = Math.max(1.0, Math.sqrt(w.forward * w.forward + w.right * w.right)) in
    let f = w.forward / len in
    let r = w.right / len in
    let a = s.yaw + w.heading in
    // Forward (sin, cos), right (-cos, sin).
    ({ s with vy: vy, ready: true },
     { x: speed * (f * Math.sin(a) - r * Math.cos(a)), y: vy, z: speed * (f * Math.cos(a) + r * Math.sin(a)) })

// Mouse look: right turns right, up looks up.
let look = (dx: float, dy: float, s: State): State =>
  { s with yaw: s.yaw - dx * mouseSensitivity,
           pitch: Math.clamp(-lookLimit, lookLimit, s.pitch - dy * mouseSensitivity) }

// ---- Tests ---------------------------------------------------------------

let flatSpeed = (v: Input.point3) => Math.sqrt(v.x * v.x + v.z * v.z)
let walking = (w: Wish) => let (_, v) = step(true, true, w, Ride.dt60, { standing with ready: true }) in v
// Nothing moves until the ground is in reach, whatever is asked.
expect (
  let (s, v) = step(false, false, { still with forward: 1.0 }, Ride.dt60, standing) in
  not s.ready && v == { x: 0.0, y: 0.0, z: 0.0 }
)
// Forward at yaw 0 is +z; right is -x (+x is left); a diagonal is no faster.
expect Math.abs(walking({ still with forward: 1.0 }).z - speed) < 0.000001
expect walking({ still with right: 1.0 }).x < -1.39
expect Math.abs(flatSpeed(walking({ still with forward: 1.0, right: 1.0 })) - speed) < 0.000001
// With a headset the look turns the walk: the head turned 90° left walks +x.
expect Math.abs(walking({ still with forward: 1.0, heading: Math.pi / 2.0 }).x - speed) < 0.000001
// Grounded, gravity never builds up; in the air it does.
expect (
  let (s, _) = step(true, true, still, Ride.dt60, { standing with ready: true, vy: -3.0 }) in
  Math.abs(s.vy + gravity * Ride.dt60) < 0.000001
)
expect (let (s, _) = step(false, true, still, 0.5, { standing with ready: true }) in Math.abs(s.vy + gravity * 0.5) < 0.000001)
// Looking: the pitch stops at the limit.
expect look(0.0, -10000.0, standing).pitch == lookLimit && look(100.0, 0.0, standing).yaw < 0.0
