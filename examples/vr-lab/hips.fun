// Hips: a body frame `{root, yaw}` for things worn on the body, from the head
// alone. Rig-local (+X right, +Y up, -Z forward); yaw is radians about +Y,
// 0 = facing -Z, as `Quat.yaw` turns it.
//
// The heading follows the head with examples/fullbody's lag (`slewYaw` and
// `nextYaw` are copied from its body.fun, unchanged): a glance inside the
// deadzone barely moves it, a real turn drags it along. `pin` freezes it,
// within the twist limit, while a hand reaches for something on the body.

type State = { head: Option.t<Input.pose>, yaw: float, root: Input.point3, pinned: bool }

/// One fixed step's input: the tracked head (None while lost), and whether a
/// hand is reaching for a slot.
type Sample = { head: Option.t<Input.pose>, pin: bool }

let init: State = { head: Option.None, yaw: 0.0, root: Quat.v3(0.0, 0.0, 0.0), pinned: false }

let degrees = (d: float): float => d * Math.pi / 180.0

// Head turns within this of the hips only drift them back, at `settleRate`.
let deadzone = degrees(25.0)
let settleRate = degrees(20.0)
// The hips never trail the head by more than this.
let maxTwist = degrees(70.0)
// How fast the hips catch up, in radians per second.
let turnRate = degrees(150.0)
// A head that moves this far in one step was recentred, not turned.
let recenterJump = 0.25
let recenterTurn = degrees(45.0)

// The pelvis is inferred, not tracked: the neck pivots 11 cm below and 9 cm
// behind the eyes, and the pelvis is under it at this fraction of its height
// (so it scales with the player and sinks when they crouch).
let eyeFromNeck = Quat.v3(0.0, 0.11, -0.09)
let pelvisRatio = 0.62

/// The heading of a head (or any frame) that looks along its -Z: exact for any
/// pitch, so looking straight up or down still has one. None when it has no
/// heading at all (rolled onto its side, looking level).
let facing = (q: Input.quaternion): Option.t<float> =>
  // Turned a half turn about Y, the frame looks along +Z, as `Quat.heading` wants.
  let t = { x: 0.0 - q.x, y: q.y, z: 0.0 - q.z, w: q.w } in
  let f = Quat.rotate(t, Quat.v3(0.0, 0.0, 1.0)) in
  let u = Quat.rotate(t, Quat.v3(0.0, 1.0, 0.0)) in
  let hx = f.x * u.y - u.x * f.y in
  let hz = f.z * u.y - u.z * f.y in
  if hx * hx + hz * hz < 0.000001 then Option.None else Option.Some(Quat.heading(t))

let headingOr = (fallback: float, q: Input.quaternion): float => facing(q) |> Option.defaultValue(fallback)

/// Where the pelvis is, from the head: under the neck it pivots on.
let rootOf = (head: Input.pose): Input.point3 =>
  let neck = Quat.sub(head.position, Quat.rotate(head.orientation, eyeFromNeck)) in
  Quat.v3(neck.x, neck.y * pelvisRatio, neck.z)

/// The hips' heading chases the head's at `turnRate` once it turns past the
/// deadzone, drifts after it slowly inside it (so a glance barely moves them
/// but they realign), and never trails it by more than `maxTwist`.
let slewYaw = (yaw: float, heading: float, dt: float): float =>
  let d = Quat.wrap(heading - yaw) in
  let rate = if Math.abs(d) > deadzone then turnRate else settleRate in
  let chased = yaw + Math.clamp(0.0 - rate * dt, rate * dt, d) in
  let behind = Quat.wrap(heading - chased) in
  if Math.abs(behind) > maxTwist then Quat.wrap(heading - Math.sign(behind) * maxTwist)
  else Quat.wrap(chased)

/// The hips' heading for a new head sample. The first one sets it; a head
/// that jumps (a recentre moves the whole tracking space) turns the hips by
/// the same amount, keeping the twist; otherwise they lag (`slewYaw`). A head
/// with no heading keeps the last.
let nextYaw = (s: State, head: Input.pose, fresh: bool, dt: float): float =>
  let heading = headingOr(s.yaw, head.orientation) in
  match s.head with
  | Option.None => heading
  | Option.Some(p) =>
    let turned = Quat.wrap(heading - headingOr(heading, p.orientation)) in
    let jumped = fresh && (Quat.length(Quat.sub(head.position, p.position)) > recenterJump
      || Math.abs(turned) > recenterTurn) in
    if jumped then Quat.wrap(s.yaw + turned) else slewYaw(s.yaw, heading, dt)

/// Advance the frame by one fixed step. Pinned, the heading slews for no
/// time: it holds, except that it still never trails the head by more than
/// `maxTwist`. A lost head keeps the last frame.
let step = (s: State, x: Sample, dt: float): State =>
  let head = match x.head with | Option.Some(h) => Option.Some(h) | Option.None => s.head in
  match head with
  | Option.None => { s with pinned: x.pin }
  | Option.Some(h) =>
    { head: Option.Some(h),
      yaw: nextYaw(s, h, Option.isSome(x.head), if x.pin then 0.0 else dt),
      root: rootOf(h),
      pinned: x.pin }

/// A pose in the hips' frame, rig-local; None before the first head.
let place = (s: State, local: Input.pose): Option.t<Input.pose> =>
  s.head |> Option.map((h) =>
    let turn = Quat.yaw(s.yaw) in
    { position: Quat.add(s.root, Quat.rotate(turn, local.position)), orientation: Quat.mul(turn, local.orientation) })

/// How far the head is turned from the hips.
let twist = (s: State): float =>
  match s.head with
  | Option.Some(h) => Quat.wrap(headingOr(s.yaw, h.orientation) - s.yaw)
  | Option.None => 0.0

// --- the contract (fullbody's torso checks, ported) ---

let dt = 1.0 / 60.0
let near = (a: float, b: float, eps: float): bool => Math.abs(a - b) <= eps
let degreesOf = (r: float): float => r * 180.0 / Math.pi
// A standing head whose neck stays at `neck`, turned and pitched about it.
let neck = Quat.v3(0.0, 1.49, 0.09)
let headAt = (turnDeg: float, pitchDeg: float): Input.pose =>
  let q = Quat.mul(Quat.yaw(degrees(turnDeg)), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), degrees(pitchDeg))) in
  { position: Quat.add(neck, Quat.rotate(q, eyeFromNeck)), orientation: q }
let looking = (turnDeg: float): Sample => { head: Option.Some(headAt(turnDeg, 0.0)), pin: false }
let pinnedAt = (turnDeg: float): Sample => { head: Option.Some(headAt(turnDeg, 0.0)), pin: true }
let trace = (s0: State, samples: List<Sample>): List<State> =>
  samples
    |> List.fold((acc, x) =>
         match acc with
         | [s, ..rest] => [step(s, x, dt), s, ..rest]
         | [] => [],
       [s0])
    |> List.reverse
let final = (states: List<State>): State => states |> List.last |> Option.defaultValue(init)
let repeat = (n: float, x: Sample): List<Sample> => List.range(n) |> List.map((i) => x)

// The head model: identity at 1.6 m puts the neck 9 cm behind, 11 cm under.
expect Hold.near3(rootOf({ position: Quat.v3(0.0, 1.6, 0.0), orientation: Quat.identity }), Quat.v3(0.0, 1.49 * pelvisRatio, 0.09))
// Turned left a quarter turn, the head faces -X; yaw 0 faces -Z.
expect near(headingOr(9.0, Quat.yaw(Math.pi / 2.0)), Math.pi / 2.0, 0.000001) && near(headingOr(9.0, Quat.identity), 0.0, 0.000001)

// A glance inside the deadzone only drifts the hips: 20°/s, 5° in 0.25 s.
expect (
  let s = final(trace(init, List.flatten([[looking(0.0)], repeat(15.0, looking(20.0))]))) in
  near(degreesOf(s.yaw), 5.0, 0.0001) && near(degreesOf(twist(s)), 15.0, 0.0001)
)

// A 120° turn (12° a step, then held): the hips never trail by more than the
// twist limit, otherwise turn no faster than the turn rate, and realign.
expect (
  let turning = List.range(10.0) |> List.map((i) => looking(12.0 * (i + 1.0))) in
  let states = trace(init, List.flatten([[looking(0.0)], turning, repeat(90.0, looking(120.0))])) in
  let steps = List.zip(List.drop(1.0, states), states) in
  let rateOk = steps |> List.all((p) =>
    let (a, b) = p in
    Math.abs(Quat.wrap(b.yaw - a.yaw)) <= turnRate * dt + 0.000001
      || near(Math.abs(twist(b)), maxTwist, 0.000001)) in
  let twistOk = states |> List.all((s) => Math.abs(twist(s)) <= maxTwist + 0.000001) in
  let lagged = List.nth(6.0, states) |> Option.map((s) => degreesOf(twist(s)) > 40.0) |> Option.defaultValue(false) in
  rateOk && twistOk && lagged && near(twist(final(states)), 0.0, 0.000001)
)

// A recentre (the head jumps 90° in one step) turns the hips with it,
// keeping their twist, instead of swinging them.
expect (
  let states = trace(init, [looking(0.0), looking(20.0), looking(110.0)]) in
  let before = List.nth(2.0, states) |> Option.defaultValue(init) in
  let after = final(states) in
  near(degreesOf(after.yaw - before.yaw), 90.0, 0.0001) && near(twist(after), twist(before), 0.000001)
)

// --- pin ---

// Pinned, a head turned past the deadzone leaves the hips where they were;
// unpinned, they chase it again.
expect (
  let states = trace(init, List.flatten([[looking(0.0)], List.range(20.0) |> List.map((i) => pinnedAt(2.5 * (i + 1.0))), repeat(10.0, pinnedAt(50.0))])) in
  let unpinned = step(final(states), looking(50.0), dt) in
  (states |> List.all((s) => s.yaw == 0.0)) && final(states).pinned
    && near(degreesOf(unpinned.yaw), 2.5, 0.0001) && not unpinned.pinned
)
// Pinned, the twist limit still holds: a head turned on past it drags the
// hips at the limit, so unpinning never snaps them.
expect (
  let s = final(trace(init, [looking(0.0), pinnedAt(30.0), pinnedAt(60.0), pinnedAt(90.0)])) in
  near(degreesOf(s.yaw), 20.0, 0.0001)
)

// --- what a worn slot sees ---

let slot: Input.pose = { position: Quat.v3(0.2, -0.1, -0.02), orientation: Quat.identity }
let slotAt = (s: State): Input.point3 => place(s, slot) |> Option.map((p) => p.position) |> Option.defaultValue(Quat.v3(9.0, 9.0, 9.0))

// No head, no frame.
expect Option.isNone(place(init, slot))
// Looking 89° (and 90°) up or down, about the neck, moves no slot.
expect (
  let level = step(init, looking(0.0), dt) in
  [89.0, -89.0, 90.0, -90.0] |> List.all((p) =>
    let tipped = step(level, { head: Option.Some(headAt(0.0, p)), pin: false }, dt) in
    Hold.near3(slotAt(tipped), slotAt(level)) && near(tipped.yaw, level.yaw, 0.000000001))
)
// A head rolled onto its side has no heading: the hips keep theirs.
expect (
  let level = step(init, looking(30.0), dt) in
  let rolled = Quat.mul(Quat.yaw(degrees(30.0)), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), Math.pi / 2.0)) in
  let lying = step(level, { head: Option.Some({ position: Quat.add(neck, Quat.rotate(rolled, eyeFromNeck)), orientation: rolled }), pin: false }, dt) in
  Option.isNone(facing(rolled)) && near(lying.yaw, level.yaw, 0.000000001)
)
// Across ±180°: a head at 178° turning to -178° turns the hips 4° the short
// way, and the slots move with them continuously.
expect (
  let a = final(trace(init, [looking(178.0), looking(-178.0)])) in
  let b = final(trace(init, [looking(-178.0)])) in
  near(degreesOf(a.yaw), 178.0 + 20.0 * dt, 0.0001) && Quat.length(Quat.sub(slotAt(a), slotAt(b))) < 0.02
)
expect (
  let at = (yaw: float) => slotAt({ (step(init, looking(180.0), dt)) with yaw: yaw }) in
  Hold.near3(at(Math.pi), at(0.0 - Math.pi))
)
// A lost head keeps the last frame.
expect (
  let s = step(init, looking(40.0), dt) in
  let lost = step(s, { head: Option.None, pin: false }, dt) in
  near(lost.yaw, s.yaw, 0.000000001) && lost.root == s.root
)
