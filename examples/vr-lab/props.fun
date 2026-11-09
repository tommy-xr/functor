// Props: what can be picked up, as plain shapes. Each is one collider (a
// sphere or a box), swept and declared at its pose, drawn at the same pose,
// and optionally an authored grip. Poses are rig-local (`Hold`'s frame);
// drawn and declared, a prop's local +Y is its pose's +Y.

type Kind = | Ball | Stick | Brick | Bottle | Pistol | Rifle

let all = [Ball, Stick, Brick, Bottle, Pistol, Rifle]

let name = (k: Kind): string =>
  match k with
  | Ball => "ball"
  | Stick => "stick"
  | Brick => "brick"
  | Bottle => "bottle"
  | Pistol => "pistol"
  | Rifle => "rifle"

let ballRadius = 0.06
let stickLength = 0.7
let stickThickness = 0.035
// A bottle is a box collider round a cylinder body and a neck, upright along
// its +Y: the body from the base up `bodyHeight`, the neck above it.
let bottleRadius = 0.04
let bodyHeight = 0.18
let neckRadius = 0.015
let neckLength = 0.08

/// The collider's full size: a box's, or the ball's bounds.
let extent = (k: Kind): Input.point3 =>
  match k with
  | Ball => Quat.v3(2.0 * ballRadius, 2.0 * ballRadius, 2.0 * ballRadius)
  | Stick => Quat.v3(stickThickness, stickThickness, stickLength)
  | Brick => Quat.v3(0.1, 0.065, 0.21)
  | Bottle => Quat.v3(2.0 * bottleRadius, bodyHeight + neckLength, 2.0 * bottleRadius)
  | Pistol => Gun.size
  | Rifle => Longarm.size

let shape = (k: Kind) =>
  match k with
  | Ball => Physics.sphere(ballRadius)
  | _ => let e = extent(k) in Physics.box(e.x, e.y, e.z)

/// From the centre to the farthest point: how far a turn moves it.
let radius = (k: Kind): float =>
  match k with
  | Ball => ballRadius
  | _ => Quat.length(extent(k)) / 2.0

/// From a point to where a hand takes the prop: its surface (0 inside a box,
/// negative inside the ball), or a gun's grip (the rifle's pistol grip; its
/// foregrip is `Longarm.foreGap`).
let gap = (k: Kind, q: Input.point3, pose: Input.pose): float =>
  let local = Quat.rotate(Quat.conj(pose.orientation), Quat.sub(q, pose.position)) in
  match k with
  | Ball => Quat.length(local) - ballRadius
  | Pistol => Quat.length(Quat.sub(local, Gun.grip.position))
  | Rifle => Quat.length(Quat.sub(local, Longarm.grip.position))
  | _ =>
    let e = extent(k) in
    let out = (v: float, size: float) => Math.max(0.0, Math.abs(v) - size / 2.0) in
    Quat.length(Quat.v3(out(local.x, e.x), out(local.y, e.y), out(local.z, e.z)))

expect Math.abs(gap(Ball, Quat.v3(0.0, 0.1, 0.0), Hold.origin) - 0.04) < 0.0001
expect Math.abs(gap(Stick, Quat.v3(0.0, 0.1, 0.0), Hold.p(0.0, 0.0, -0.2)) - (0.1 - stickThickness / 2.0)) < 0.0001
expect Math.abs(gap(Stick, Quat.v3(0.0, 0.0, -0.8), Hold.p(0.0, 0.0, -0.2)) - 0.25) < 0.0001
// Turned, measured in the prop's own frame.
expect Math.abs(gap(Brick, Quat.v3(0.2, 0.0, 0.0), Hold.at(Hold.zero, Quat.yaw(Math.pi / 2.0))) - (0.2 - 0.105)) < 0.0001

/// How near a hand must be (`gap`) to take a prop: a gun only at its grip.
let reach = (k: Kind): float =>
  match k with
  | Pistol => Gun.gripZone
  | Rifle => Longarm.gripZone
  | _ => Hold.reach

// A hand on the pistol's grip reaches it; one at the muzzle, touching it,
// doesn't (even hovering it already).
expect (
  let pose = Hold.at(Quat.v3(0.2, 1.0, -0.5), Quat.yaw(0.5)) in
  let at = (p: Input.point3) => Quat.add(pose.position, Quat.rotate(pose.orientation, p)) in
  Hold.hovering(reach(Pistol), false, gap(Pistol, at(Gun.grip.position), pose))
    && not Hold.hovering(reach(Pistol), true, gap(Pistol, at(Quat.v3(0.0, 0.05, -0.1)), pose))
)

// The bottle's neck, half way up it: held there in a fist round it, the
// fist's -Z (pinky to thumb) up the bottle, so the body hangs below the hand.
let neckAt = bodyHeight + neckLength / 2.0 - extent(Bottle).y / 2.0

/// The hand's pose in the prop's frame where it is meant to be held, if
/// anywhere: a hand that grabs within `Hold.snapRadius` of it snaps there.
let grip = (k: Kind): Option.t<Input.pose> =>
  match k with
  | Bottle => Option.Some(Hold.at(Quat.v3(0.0, neckAt, 0.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), Math.pi / 2.0)))
  | Pistol => Option.Some(Gun.grip)
  | Rifle => Option.Some(Longarm.grip)
  | _ => Option.None

/// Whether the prop is only ever held at its grip: taken anywhere in reach of
/// it (`reach`), it settles into it (`Hold.settleInto`) rather than snapping.
let gripRequired = (k: Kind): bool => k == Pistol || k == Rifle

/// The weapon a prop is, if it is one.
let weapon = (k: Kind): Option.t<Gun.Weapon> =>
  match k with
  | Pistol => Option.Some(Gun.pistol)
  | Rifle => Option.Some(Longarm.weapon)
  | _ => Option.None

// A hand pointing up, gripping the neck: snapped, the bottle stands upright
// with its neck in the fist.
expect (
  match grip(Bottle) with
  | Option.Some(g) =>
    let fist = Hold.at(Quat.v3(0.3, 1.2, -0.5), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), Math.pi / 2.0)) in
    let held = Hold.grab(Hold.Right, fist, Hold.at(Quat.add(fist.position, Quat.v3(0.0, 0.01 - neckAt, 0.02)), Quat.identity), grip(Bottle)) in
    Hold.near(Hold.compose(fist, held.offset), Hold.at(Quat.add(fist.position, Quat.v3(0.0, 0.0 - neckAt, 0.0)), Quat.identity))
  | Option.None => false
)

/// A free dynamic body's material.
let dynamics = (k: Kind, body: Physics.body): Physics.body =>
  let (friction, restitution, spinDamping) =
    match k with
    | Ball => (0.6, 0.45, 1.5)
    | Stick => (0.6, 0.2, 0.5)
    | Brick => (0.7, 0.1, 0.5)
    | Bottle => (0.5, 0.25, 0.5)
    | Pistol => (0.7, 0.1, 0.8)
    | Rifle => (0.7, 0.1, 0.8) in
  body
    |> Physics.friction(friction)
    |> Physics.restitution(restitution)
    |> Physics.linearDamping(0.1)
    |> Physics.angularDamping(spinDamping)
    |> Physics.continuous

/// How a prop is drawn: free, hovered by a hand in reach, or held.
type Look = | Resting | Hovered | Taken

let paint = (look: Look, r: float, g: float, b: float, s: Scene.t): Scene.t =>
  match look with
  | Resting => s |> Scene.lit(Color.rgb(r, g, b))
  | Hovered => s |> Scene.emissive(Color.rgb(0.4 + 0.6 * r, 0.4 + 0.6 * g, 0.4 + 0.6 * b))
  | Taken => s |> Scene.lit(Color.rgb(0.25 + 0.75 * r, 0.25 + 0.75 * g, 0.25 + 0.75 * b))

let cylinder = (r: float, h: float, y: float) =>
  Scene.cylinder() |> Scene.scaleXYZ(2.0 * r, h, 2.0 * r) |> Scene.translate(Vec3.make(0.0, y, 0.0))

/// A prop's look in its own frame (centred on its collider).
let scene = (k: Kind, look: Look): Scene.t =>
  let e = extent(k) in
  match k with
  | Ball => Scene.sphere() |> Scene.scale(ballRadius) |> paint(look, 0.95, 0.3, 0.2)
  | Stick =>
    // A grip band at its near end (the pose's +Z; drawn, local -Z).
    Scene.group([
      Scene.cube() |> Scene.scaleXYZ(e.x, e.y, e.z) |> paint(look, 0.25, 0.6, 0.58),
      Scene.cube()
        |> Scene.scaleXYZ(e.x * 1.3, e.y * 1.3, 0.12)
        |> Scene.translate(Vec3.make(0.0, 0.0, 0.07 - e.z / 2.0))
        |> Scene.lit(Color.rgb(0.12, 0.12, 0.14)),
    ])
  | Brick =>
    // A stripe round its long middle shows how it is turned.
    Scene.group([
      Scene.cube() |> Scene.scaleXYZ(e.x, e.y, e.z) |> paint(look, 0.35, 0.45, 0.9),
      Scene.cube() |> Scene.scaleXYZ(e.x * 1.02, e.y * 1.02, 0.03) |> Scene.translate(Vec3.make(0.0, 0.0, e.z / 4.0))
        |> Scene.lit(Color.rgb(0.95, 0.9, 0.8)),
    ])
  | Bottle =>
    let base = 0.0 - e.y / 2.0 in
    Scene.group([
      cylinder(bottleRadius, bodyHeight - 0.03, base + (bodyHeight - 0.03) / 2.0) |> paint(look, 0.2, 0.62, 0.35),
      // The shoulder, then the neck and its lip.
      cylinder(0.6 * bottleRadius + 0.4 * neckRadius, 0.03, base + bodyHeight - 0.015) |> paint(look, 0.2, 0.62, 0.35),
      cylinder(neckRadius, neckLength, base + bodyHeight + neckLength / 2.0) |> paint(look, 0.2, 0.62, 0.35),
      cylinder(neckRadius * 1.25, 0.012, base + bodyHeight + neckLength - 0.006) |> Scene.lit(Color.rgb(0.85, 0.8, 0.35)),
    ])
  | Pistol => Gun.scene(paint(look))
  | Rifle => Longarm.scene(paint(look))
