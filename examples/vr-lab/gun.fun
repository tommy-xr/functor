// Gun: the pistol's working parts, pure and rig-local (`Hold`'s frame): its
// shape and authored grip, the trigger, the bore a shot leaves along, the
// shot, and the recoil kick. The world reaches in only through the shot's
// ray and the muzzle check (`Game.World`).

// --- the pistol ---
// Laid out in the hand's frame when held (the grip pose: -Z forward, +Y up):
// the bore runs 5 cm above the fist, parallel to its -Z, from the chamber
// 2 cm ahead of it to the muzzle 15 cm ahead.

let bore = 0.05
let chamberAhead = 0.02
let muzzleAhead = 0.15
// The collider's bounds there: 3.5 cm wide, from the handle's foot 6.5 cm
// below the fist to the slide's top 7 cm above it, and from the muzzle to the
// slide's back 5.5 cm behind it.
let size = Quat.v3(0.035, 0.135, 0.205)
let center = Quat.v3(0.0, 0.0025, 0.0 - 0.0475)

/// The hand's pose in the pistol's frame (centred on its collider): the
/// authored grip. A pistol is only taken there (`gripZone`), and always
/// settles into it.
let grip: Input.pose = Hold.at(Quat.scale(0.0 - 1.0, center), Quat.identity)

/// How near the grip a hand must be to take the pistol: the front of the
/// barrel is out of reach, so it is never taken by the muzzle end.
let gripZone = 0.07

/// A shot's line from a pistol at `pose`: the chamber, the muzzle, and the
/// barrel's way (unit).
type Bore = { chamber: Input.point3, muzzle: Input.point3, dir: Input.point3 }

let boreOf = (pose: Input.pose): Bore =>
  let hand = Hold.compose(pose, grip) in
  let along = (ahead: float) => Quat.add(hand.position, Quat.rotate(hand.orientation, Quat.v3(0.0, bore, 0.0 - ahead))) in
  { chamber: along(chamberAhead), muzzle: along(muzzleAhead), dir: Quat.rotate(hand.orientation, Quat.v3(0.0, 0.0, 0.0 - 1.0)) }

/// The pistol held settled in a hand at `hand`: where it is.
let heldAt = (hand: Input.pose): Input.pose => Hold.compose(hand, Hold.inverse(grip))

let close3 = (a: Input.point3, b: Input.point3): bool => Quat.length(Quat.sub(a, b)) < 0.000001
// Level, yawed, pitched and rolled hands: the muzzle 5 cm above the hand's
// -Z axis and 15 cm along it, and the shot along that axis.
expect (
  let b = boreOf(heldAt(Hold.p(0.1, 1.2, -0.3))) in
  close3(b.muzzle, Quat.v3(0.1, 1.25, -0.45)) && close3(b.chamber, Quat.v3(0.1, 1.25, -0.32)) && close3(b.dir, Quat.v3(0.0, 0.0, -1.0))
)
expect (
  // A quarter turn left: the barrel points along -X.
  let b = boreOf(heldAt(Hold.at(Quat.v3(0.1, 1.2, -0.3), Quat.yaw(Math.pi / 2.0)))) in
  close3(b.muzzle, Quat.v3(-0.05, 1.25, -0.3)) && close3(b.dir, Quat.v3(-1.0, 0.0, 0.0))
)
expect (
  // Tipped 30 degrees up.
  let a = Math.pi / 6.0 in
  let b = boreOf(heldAt(Hold.at(Quat.v3(0.0, 1.0, 0.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), a)))) in
  close3(b.dir, Quat.v3(0.0, Math.sin(a), 0.0 - Math.cos(a)))
    && close3(b.muzzle, Quat.v3(0.0, 1.0 + 0.05 * Math.cos(a) + 0.15 * Math.sin(a), 0.05 * Math.sin(a) - 0.15 * Math.cos(a)))
)
expect (
  // Rolled a quarter turn clockwise (about -Z): the bore is beside the hand,
  // on its right, still parallel to its -Z.
  let b = boreOf(heldAt(Hold.at(Quat.v3(0.0, 1.0, 0.0), Quat.axisAngle(Quat.v3(0.0, 0.0, -1.0), Math.pi / 2.0)))) in
  close3(b.muzzle, Quat.v3(0.05, 1.0, -0.15)) && close3(b.dir, Quat.v3(0.0, 0.0, -1.0))
)
expect (
  // Yawed, pitched and rolled at once: the muzzle is the hand's (0, 5, -15) cm.
  let q = Quat.mul(Quat.yaw(0.6), Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), -0.3), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 1.1))) in
  let hand = Hold.at(Quat.v3(0.2, 1.3, -0.4), q) in
  let b = boreOf(heldAt(hand)) in
  close3(b.muzzle, Quat.add(hand.position, Quat.rotate(q, Quat.v3(0.0, 0.05, -0.15))))
    && close3(b.dir, Quat.rotate(q, Quat.v3(0.0, 0.0, -1.0)))
)

// --- the trigger ---
// Analog, with hysteresis: a pull to `fireAt` fires once; it must fall back
// to `rearmAt` before the next counts. Semi-automatic: one shot per pull.

let fireAt = 0.8
let rearmAt = 0.3

/// `pulled`: past `fireAt` and not yet back to `rearmAt`. `armed`: seen at or
/// under `rearmAt` since the pistol was taken or its hand was last tracked,
/// so a trigger already held then never fires.
type Trigger = { pulled: bool, armed: bool }

let disarmed: Trigger = { pulled: false, armed: false }

/// The trigger at `value`: its new state, and whether this step fires.
let pull = (t: Trigger, value: float): (Trigger, bool) =>
  let pulled = if t.pulled then value > rearmAt else value >= fireAt in
  ({ pulled: pulled, armed: t.armed || value <= rearmAt }, t.armed && pulled && not t.pulled)

/// Pull through `values` from `t`: how many shots.
let shotsFrom = (t: Trigger, values: List<float>): float =>
  let (_, n) = values |> List.fold((acc, v) =>
    let (s, k) = acc in
    let (next, fires) = pull(s, v) in
    (next, if fires then k + 1.0 else k), (t, 0.0)) in
  n

let ready = { pulled: false, armed: true }
expect shotsFrom(ready, [0.79, 0.5, 0.79]) == 0.0
expect shotsFrom(ready, [0.8]) == 1.0
// Held, or wavering above `rearmAt`, it is one shot; released to it, two.
expect shotsFrom(ready, List.range(30.0) |> List.map((_) => 1.0)) == 1.0
expect shotsFrom(ready, [0.9, 0.31, 0.9, 0.5, 1.0]) == 1.0
expect shotsFrom(ready, [0.9, 0.3, 0.9]) == 2.0
// Taken already pulled: no shot until it is let out and pulled again.
expect shotsFrom(disarmed, [1.0, 1.0, 1.0]) == 0.0
expect shotsFrom(disarmed, [1.0, 0.5, 0.9]) == 0.0
expect shotsFrom(disarmed, [1.0, 0.2, 0.9]) == 1.0

// --- the shot ---

/// How far a shot reaches.
let range = 30.0
/// Clear of the level by this much past the muzzle, or the shot is refused:
/// more than the 4 mm a held prop stops short of what it hits (`Hold.skin`),
/// so a barrel held against a wall never fires.
let clearance = 0.01

/// What a shot struck: where (rig-local), the body's tag, and whether that is
/// a target.
type Impact = { point: Input.point3, tag: string, target: bool }

/// A shot fired at `t` from the muzzle along `dir`, and what it struck
/// within `range`, if anything.
type Shot = { t: float, from: Input.point3, dir: Input.point3, impact: Option.t<Impact> }

/// Where a shot's line ends: where it struck, or at `range`.
let endOf = (s: Shot): Input.point3 =>
  s.impact |> Option.map((i) => i.point) |> Option.defaultValue(Quat.add(s.from, Quat.scale(range, s.dir)))

/// The muzzle check's segment: from the chamber to `clearance` past the
/// muzzle. Anything fixed on it (the barrel poked into a wall) refuses the shot.
let checked = (b: Bore): (Input.point3, Input.point3) => (b.chamber, Quat.add(b.muzzle, Quat.scale(clearance, b.dir)))

// --- the pistol's state ---

// The recoil kick: 1 as it fires, falling to 0 over `recoilSeconds`. Drawing
// only (`kicked`): the body, the muzzle check and the next shot all use the
// accepted pose.
let recoilSeconds = 0.12
let recoilPitch = 8.0 * Math.pi / 180.0
let recoilBack = 0.02
let hitCap = 12.0

/// The pistol: the hand that held it last step, its trigger, the kick, shots
/// fired and refused, the last shot, and the target hits, newest last (at
/// most `hitCap`).
type State = {
  holder: Option.t<Hold.Side>,
  trigger: Trigger,
  kick: float,
  fired: float,
  refused: float,
  last: Option.t<Shot>,
  hits: List<Impact>
}

let idle: State = { holder: Option.None, trigger: disarmed, kick: 0.0, fired: 0.0, refused: 0.0, last: Option.None, hits: [] }

/// One step of the pistol held by `holder` (None: nobody), with that hand's
/// trigger at `value`: the new state, and whether the trigger fired. A new
/// holder, or an untracked one, starts disarmed, so a trigger held as the
/// pistol is taken (off the bench, out of a holster or from the other hand)
/// or as tracking comes back fires nothing until it is let out.
let step = (g: State, holder: Option.t<Hold.Side>, tracked: bool, value: float, dt: float): (State, bool) =>
  let kick = Math.max(0.0, g.kick - dt / recoilSeconds) in
  if Option.isNone(holder) || not tracked then ({ g with holder: holder, trigger: disarmed, kick: kick }, false)
  else
    let (trigger, fires) = pull(if holder == g.holder then g.trigger else disarmed, value) in
    ({ g with holder: holder, trigger: trigger, kick: kick }, fires)

// --- ammunition: infinite here; a magazine plugs in at these two ---

/// Whether a pulled trigger has a round to fire.
let canFire = (g: State): bool => true

/// After a shot: the round spent.
let onFired = (g: State): State => g

/// The pistol after firing `s`: kicked, counted, the hit recorded.
let fire = (g: State, s: Shot): State =>
  let hit = s.impact |> Option.filter((i) => i.target) in
  let hits = match hit with
    | Option.Some(i) => g.hits |> List.append([i])
    | Option.None => g.hits in
  { onFired(g) with
      kick: 1.0,
      fired: g.fired + 1.0,
      last: Option.Some(s),
      hits: List.drop(Math.max(0.0, List.length(hits) - hitCap), hits) }

/// The pistol after a refused pull: a click.
let refuse = (g: State): State => { g with refused: g.refused + 1.0 }

let dt = 1.0 / 60.0
let run = (g: State, holder: Option.t<Hold.Side>, tracked: bool, values: List<float>): (State, float) =>
  values |> List.fold((acc, v) =>
    let (s, n) = acc in
    let (next, fires) = step(s, holder, tracked, v, dt) in
    (next, if fires then n + 1.0 else n), (g, 0.0))
let right = Option.Some(Hold.Right)

// Taken with the trigger held: nothing until it is let out.
expect (let (_, n) = run(idle, right, true, [1.0, 1.0, 1.0]) in n == 0.0)
expect (let (_, n) = run(idle, right, true, [1.0, 0.0, 1.0]) in n == 1.0)
// Handed to the other hand mid-pull: that hand starts disarmed.
expect (
  let (g, _) = run(idle, right, true, [0.0]) in
  let (_, n) = run(g, Option.Some(Hold.Left), true, [1.0]) in
  n == 0.0
)
// Tracking lost disarms: back with it held, no shot; let out and pulled, one.
expect (
  let (armed, _) = run(idle, right, true, [0.0]) in
  let (lost, _) = run(armed, right, false, [0.0]) in
  let (_, held) = run(lost, right, true, [1.0, 1.0]) in
  let (_, again) = run(lost, right, true, [1.0, 0.1, 1.0]) in
  not lost.trigger.armed && held == 0.0 && again == 1.0
)
// The kick falls to exactly 0 within `recoilSeconds`, and never below.
expect (
  let kicked1 = { idle with kick: 1.0 } in
  let (later, _) = run(kicked1, right, true, List.range(8.0) |> List.map((_) => 0.0)) in
  let (mid, _) = run(kicked1, right, true, [0.0, 0.0, 0.0]) in
  later.kick == 0.0 && mid.kick > 0.0 && mid.kick < 1.0
)

/// Where the pistol is drawn: `pose` kicked up about the grip and back by
/// `kick`, easing out.
let kicked = (kick: float, pose: Input.pose): Input.pose =>
  if kick <= 0.0 then pose
  else
    let k = kick * kick in
    let hand = Hold.compose(pose, grip) in
    let jolt = Hold.at(Quat.v3(0.0, 0.0, recoilBack * k), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), recoilPitch * k)) in
    Hold.compose(Hold.compose(hand, jolt), Hold.inverse(grip))

expect (let p = Hold.at(Quat.v3(0.1, 1.2, -0.3), Quat.yaw(0.4)) in Hold.near(kicked(0.0, p), p))
// Fully kicked, the muzzle is up and back of where it aims; the bore it
// fires along is the accepted pose's.
expect (
  let p = heldAt(Hold.p(0.0, 1.2, -0.3)) in
  let up = boreOf(kicked(1.0, p)) in
  up.dir.y > 0.13 && up.muzzle.z > (boreOf(p)).muzzle.z
)

// --- the look ---

let part = (sx: float, sy: float, sz: float, x: float, y: float, z: float) =>
  Scene.cube() |> Scene.scaleXYZ(sx, sy, sz) |> Scene.translate(Vec3.make(x - center.x, y - center.y, z - center.z))

/// The pistol in its own frame, from parts placed in the hand's (-Z the
/// barrel's way; turned a half turn at the end, as drawn its local +Z is):
/// slide and barrel, a raked handle, the trigger and its guard, and sights
/// along the bore. `paint` colours the slide and handle.
let scene = (paint: (float, float, float, Scene.t) => Scene.t): Scene.t =>
  let dark = Color.rgb(0.08, 0.08, 0.09) in
  Scene.group([
    // The slide, from the muzzle to its back, round the bore.
    part(0.03, 0.04, 0.205, 0.0, bore, 0.0 - 0.0475) |> paint(0.2, 0.21, 0.24),
    // The handle, raked back 12 degrees below the slide.
    Scene.cube() |> Scene.scaleXYZ(0.03, 0.1, 0.045) |> Scene.rotateX(Angle.radians(0.0 - 0.21))
      |> Scene.translate(Vec3.make(0.0 - center.x, 0.0 - 0.015 - center.y, 0.02 - center.z)) |> paint(0.3, 0.22, 0.16),
    // The trigger guard and the trigger.
    part(0.008, 0.006, 0.045, 0.0, 0.0 - 0.002, 0.0 - 0.028) |> Scene.lit(dark),
    part(0.006, 0.022, 0.006, 0.0, 0.012, 0.0 - 0.016) |> Scene.lit(dark),
    // The muzzle's bore, then a bright front post and the rear notch.
    part(0.012, 0.012, 0.004, 0.0, bore, 0.0 - muzzleAhead - 0.001) |> Scene.lit(dark),
    part(0.004, 0.008, 0.006, 0.0, bore + 0.024, 0.0 - 0.14) |> Scene.emissive(Color.rgb(1.0, 0.55, 0.1)),
    part(0.006, 0.008, 0.006, 0.0 - 0.008, bore + 0.024, 0.048) |> Scene.lit(dark),
    part(0.006, 0.008, 0.006, 0.008, bore + 0.024, 0.048) |> Scene.lit(dark),
  ]) |> Scene.rotateY(180deg)
