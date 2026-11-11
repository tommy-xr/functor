// Longarm: the two-handed rifle, pure and rig-local (`Hold`'s frame). Its
// shape, its two grips and the two-hand aim. One hand holds it at the pistol
// grip (the trigger) rigidly, like the pistol. A second on the foregrip aims
// it: the barrel from the primary fist toward the support fist, rolled as the
// primary hand is (`aimAt`). The trigger, shot and recoil are `Gun`'s.

// --- the rifle ---
// Laid out in the primary hand's frame when held (-Z forward, +Y up), like
// the pistol: the bore 6 cm above the fist, the stock's back 28 cm behind it,
// the muzzle 56 cm ahead. The support fist rides a line through the primary
// fist parallel to the bore (a vertical foregrip under the handguard), from
// `foreNear` to `foreFar` ahead of it, so the barrel points from one fist to
// the other.

let bore = 0.06
let chamberAhead = 0.04
let muzzleAhead = 0.56
let stockBack = 0.28
let foreNear = 0.14
let foreFar = 0.4

// The collider's bounds there: 5 cm wide, from the grip's and magazine's
// foot 4.5 cm below the fist to 10 cm above it, from the stock's back to the
// muzzle.
let size = Quat.v3(0.05, 0.145, stockBack + muzzleAhead)
let center = Quat.v3(0.0, 0.0275, (stockBack - muzzleAhead) / 2.0)

/// The frame the rifle is laid out in (the primary fist's, as below), in its
/// own (centred on its collider).
let layout: Input.pose = Hold.at(Quat.scale(0.0 - 1.0, center), Quat.identity)

/// Where the primary hand's root sits in `layout`: the rifle grip spec's
/// solved position (`Fitted`, generated), levelled (`Gun.levelled`).
let placement: Input.pose = Gun.levelled(Fitted.rifleGripPlacement)

/// The primary hand's pose in the rifle's frame: its pistol grip. Like the
/// pistol's, only taken there.
let grip: Input.pose = Hold.compose(layout, placement)
let gripZone = 0.07

// The charging handle: a T at the receiver's top rear, under the rear
// sight, pulled straight back `handleTravel`.
let handleAt = Quat.v3(0.0, 0.09, 0.075)
let handleTravel = 0.07

// The magazine in the well ahead of the grip; the readout on the receiver's
// top ahead of the rear sight, tipped back toward the eye; the charging
// handle (home while the bolt is locked back, 70% back), the port on the right.
let weapon: Gun.Weapon = Gun.weaponAt(layout, grip, bore, chamberAhead, muzzleAhead, true,
  { kind: Ammo.RifleMag, at: Quat.sub(Quat.v3(0.0, 0.0, 0.0 - 0.09), center), rake: 0.0, drop: 0.1, guide: Gun.guide },
  { at: Quat.sub(Quat.v3(0.0, 0.0865, 0.0 - 0.07), center), tilt: 0.0 - Math.pi / 6.0, size: 0.005 },
  { at: Quat.sub(handleAt, center), radius: 0.04, travel: handleTravel, lockBack: 0.0, boltLock: 0.7 * handleTravel,
    port: Quat.sub(Quat.v3(0.04, 0.06, 0.0 - 0.03), center), pump: false })

expect Gun.clearBelow(weapon, size)

// --- the foregrip ---

/// A two-handed weapon's support line: through the layout's fist (`layout`:
/// the layout frame, in the weapon's) parallel to the bore, from `near` to
/// `far` ahead of it (the rifle's foregrip, or a pump's fore-end home), and
/// how far that slides back (`travel`: a pump's; 0 for a fixed foregrip).
/// And where the primary hand's root sits in that frame (`hand`).
type Fore = { layout: Input.pose, hand: Input.pose, near: float, far: float, travel: float }

let fore: Fore = { layout: layout, hand: placement, near: foreNear, far: foreFar, travel: 0.0 }

/// The layout frame a primary hand at `primary` holds a weapon by: what the
/// support line and the two-hand aim are measured from.
let layoutOf = (f: Fore, primary: Input.pose): Input.pose => Hold.compose(primary, Hold.inverse(f.hand))

/// How near the foregrip line a hand must be to take it: off it by at most
/// this, between its ends.
let foreZone = 0.06
/// A support hand this far off the primary hand's axis breaks off, or past
/// either end of the foregrip by `foreSlack` (a pump's, anywhere it slides).
let breakOff = 0.12
let foreSlack = 0.08
/// Once supporting, it is forgiving: measured from the two-hand aim line (not
/// the primary hand's axis, which a bent wrist or a recoil ease swings away),
/// it breaks only this far off that line, this far past either end, or with
/// the aim swung this far from the primary hand. A quick pull or a shot's
/// kick keeps the grip; a deliberate pull-off still breaks it.
let releaseOff = 0.6
let releaseSlack = 0.25
let releaseSwing = 150.0 * Math.pi / 180.0

/// A point seen from a primary fist at `fist`: how far ahead along its -Z,
/// and how far off that axis.
type Along = { along: float, off: float }

let alongOf = (fist: Input.pose, q: Input.point3): Along =>
  let local = Hold.localPoint(fist, q) in
  { along: 0.0 - local.z, off: Math.sqrt(local.x * local.x + local.y * local.y) }

/// From a point to the foregrip of a weapon at `pose`, slid back `pull`: 0
/// on its line.
let foreGap = (f: Fore, pull: float, pose: Input.pose, q: Input.point3): float =>
  let a = alongOf(Hold.compose(pose, f.layout), q) in
  let past = Math.max(0.0, Math.max(f.near - pull - a.along, a.along - (f.far - pull))) in
  Math.sqrt(a.off * a.off + past * past)

/// Whether a support hand at `a` (seen from the primary hand) still holds.
let holds = (f: Fore, a: Along): bool =>
  a.off <= breakOff && a.along >= f.near - f.travel - foreSlack && a.along <= f.far + foreSlack

/// Whether a supporting hand at `a` (seen along the two-hand aim) keeps it.
let keeps = (f: Fore, a: Along): bool =>
  a.off <= releaseOff && a.along >= f.near - f.travel - releaseSlack && a.along <= f.far + releaseSlack

// The layout's fist level, and the hand that holds it there.
let level = Hold.at(Quat.v3(0.2, 1.1, -0.3), Quat.identity)
let levelHand = Hold.compose(level, placement)
// Held level, the bore points along the hand's -Z.
expect Hold.near3((Gun.boreOf(weapon, Gun.heldAt(weapon, Hold.origin))).dir, Quat.v3(0.0, 0.0, 0.0 - 1.0))
expect foreGap(fore, 0.0, Gun.heldAt(weapon, levelHand), Quat.v3(0.2, 1.1, -0.55)) < 0.0001
expect Math.abs(foreGap(fore, 0.0, Gun.heldAt(weapon, levelHand), Quat.v3(0.2, 1.15, -0.5)) - 0.05) < 0.0001
// Past the near end by 4 cm, on the line: 4 cm away.
expect Math.abs(foreGap(fore, 0.0, Gun.heldAt(weapon, levelHand), Quat.v3(0.2, 1.1, -0.4)) - 0.04) < 0.0001
// It slides freely along the foregrip, and a little past its ends; off the
// axis by 11 cm it still holds, by 15 cm it breaks; and past an end by more
// than `foreSlack`.
expect [foreNear, 0.25, foreFar, foreNear - 0.07, foreFar + 0.07] |> List.all((d) => holds(fore, alongOf(level, Quat.v3(0.2, 1.1, -0.3 - d))))
expect holds(fore, alongOf(level, Quat.v3(0.31, 1.1, -0.55))) && not holds(fore, alongOf(level, Quat.v3(0.35, 1.1, -0.55)))
expect not holds(fore, alongOf(level, Quat.v3(0.2, 1.1, -0.3 - foreNear + 0.09))) && not holds(fore, alongOf(level, Quat.v3(0.2, 1.1, -0.3 - foreFar - 0.09)))

// --- the two-hand aim ---

/// The primary hand turned so its -Z points from its fist at the support
/// fist `at`: forward along `at` - fist, up the primary hand's up made
/// perpendicular to it, so the roll is the primary hand's. (Its up along the
/// barrel: its back instead.)
let aimAt = (primary: Input.pose, at: Input.point3): Input.quaternion =>
  let toward = Quat.sub(at, primary.position) in
  let ahead = Quat.scale(1.0 / Quat.length(toward), toward) in
  let perpendicular = (v: Input.point3) => Quat.sub(v, Quat.scale(Quat.dot(v, ahead), ahead)) in
  let up = perpendicular(Quat.rotate(primary.orientation, Quat.v3(0.0, 1.0, 0.0))) in
  let upright = if Quat.length(up) > 0.001 then up else perpendicular(Quat.rotate(primary.orientation, Quat.v3(0.0, 0.0, 1.0))) in
  let y = Quat.scale(1.0 / Quat.length(upright), upright) in
  let z = Quat.scale(0.0 - 1.0, ahead) in
  Quat.fromBasis(Quat.cross(y, z), y, z)

let fwd = (q: Input.quaternion) => Quat.rotate(q, Quat.v3(0.0, 0.0, -1.0))
let upOf = (q: Input.quaternion) => Quat.rotate(q, Quat.v3(0.0, 1.0, 0.0))
let unit = (v: Input.point3) => Quat.scale(1.0 / Quat.length(v), v)
// Hands turned every way, the support ahead of the primary along its -Z:
// the aim is the primary hand's own turn.
let turns = [
  Quat.identity, Quat.yaw(0.8), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.5),
  Quat.axisAngle(Quat.v3(0.0, 0.0, -1.0), 1.2), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 2.8),
  Quat.mul(Quat.yaw(-2.0), Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), -0.7), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.9))),
]
expect turns |> List.all((q) =>
  let p = Hold.at(Quat.v3(0.1, 1.2, -0.3), q) in
  Quat.angle(aimAt(p, Quat.add(p.position, Quat.scale(0.3, fwd(q)))), q) < 0.0001)
// The support moved off that line: forward follows support - primary, the
// up stays perpendicular to it and in the plane of the primary hand's up
// (the roll is the primary hand's), and the basis is orthonormal.
expect turns |> List.all((q) =>
  let p = Hold.at(Quat.v3(0.1, 1.2, -0.3), q) in
  let s = Quat.add(p.position, Quat.rotate(q, Quat.v3(0.06, -0.04, -0.28))) in
  let a = aimAt(p, s) in
  let f = fwd(a) in
  let u = upOf(a) in
  let x = Quat.rotate(a, Quat.v3(1.0, 0.0, 0.0)) in
  let primaryUp = upOf(q) in
  Hold.near3(f, unit(Quat.sub(s, p.position)))
    && Math.abs(Quat.dot(u, f)) < 0.0001 && Math.abs(Quat.dot(x, f)) < 0.0001 && Math.abs(Quat.dot(x, u)) < 0.0001
    && Math.abs(Quat.length(u) - 1.0) < 0.0001
    && Math.abs(Quat.dot(x, primaryUp)) < 0.0001 && Quat.dot(u, primaryUp) > 0.0)
// Rolled a quarter turn, aimed 20 degrees left by the support: still rolled
// a quarter turn about the new forward.
expect (
  let rolled = Quat.axisAngle(Quat.v3(0.0, 0.0, -1.0), Math.pi / 2.0) in
  let p = Hold.at(Quat.v3(0.0, 1.2, 0.0), rolled) in
  let left = Quat.yaw(20.0 * Math.pi / 180.0) in
  let a = aimAt(p, Quat.add(p.position, Quat.scale(0.3, fwd(left)))) in
  Quat.angle(a, Quat.mul(left, rolled)) < 0.0001
)
// The primary hand's up along the barrel (pointing straight down): its back
// stands in, and the aim is still a rotation.
expect (
  let down = Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.0 - Math.pi / 2.0) in
  let p = Hold.at(Quat.v3(0.0, 1.2, 0.0), Quat.identity) in
  let a = aimAt(p, Quat.add(p.position, Quat.scale(0.3, fwd(down)))) in
  Hold.near3(fwd(a), Quat.v3(0.0, -1.0, 0.0)) && Math.abs(Quat.length(upOf(a)) - 1.0) < 0.0001
)

// --- the support hand ---

/// A hand on the foregrip: which, the turn still easing out (`ease`, so
/// taking the foregrip doesn't jump the barrel to it), and this step's aim
/// (the primary hand's turn, aimed at the support).
type Support = { side: Hold.Side, ease: Input.quaternion, aim: Input.quaternion }

/// A support hand at `at` takes foregrip `f` of a rifle the primary hand at
/// `primary` holds: aimed exactly as it was, the rest easing out. The aim is
/// the layout frame's turn (`layoutOf`), the bore's.
let support = (f: Fore, side: Hold.Side, primary: Input.pose, at: Input.point3): Support =>
  let l = layoutOf(f, primary) in
  { side: side, ease: Quat.mul(l.orientation, Quat.conj(aimAt(l, at))), aim: l.orientation }

/// What a support step did: still on the foregrip, let go, or broken off.
type Grasp = | Supporting(support: Support) | LetGo | Broke

/// One step of a support hand at `at` (None: untracked) on foregrip `f`:
/// too far off the aim line or past the foregrip's ends (`keeps`), it
/// breaks off (even if it opened); opened or lost, it lets go; else it aims
/// the weapon, the ease turning out by at most `Hold.snapTurnStep`.
let grasp = (f: Fore, s: Support, primary: Input.pose, at: Option.t<Input.point3>, squeezed: bool): Grasp =>
  let l = layoutOf(f, primary) in
  match at with
  | Option.None => LetGo
  | Option.Some(q) =>
    if not keeps(f, alongOf(Hold.at(l.position, s.aim), q)) || Quat.angle(s.aim, l.orientation) > releaseSwing then Broke
    else if not squeezed then LetGo
    else
      let left = Quat.angle(s.ease, Quat.identity) in
      let ease = if left <= Hold.snapTurnStep then Quat.identity else Quat.slerp(s.ease, Quat.identity, Hold.snapTurnStep / left) in
      Supporting({ s with ease: ease, aim: Quat.mul(ease, aimAt(l, q)) })

/// The primary grip a two-hand hold steers the rifle by: the layout's fist
/// where the primary hand has it, turned to the aim, and the hand on it.
let steer = (f: Fore, primary: Input.pose, s: Support): Input.pose =>
  Hold.compose(Hold.at((layoutOf(f, primary)).position, s.aim), f.hand)

/// The primary hand's hold once its support lets go: the rifle kept where
/// the aim had it, relative to the hand (then eased back into the grip).
let alone = (f: Fore, held: Hold.Held, s: Support, primary: Input.pose): Hold.Held =>
  { held with offset: Hold.compose(Hold.relative(primary, steer(f, primary, s)), held.offset), snapTo: Option.None }

let ahead = (p: Input.pose, d: float) => Quat.add(p.position, Quat.scale(d, fwd(p.orientation)))
let grasped = (g: Grasp): Option.t<Support> =>
  match g with
  | Supporting(s) => Option.Some(s)
  | _ => Option.None

// Taken off the primary's axis: the aim doesn't move on the grab, and eases
// to aim at the support, at most `Hold.snapTurnStep` a step.
expect (
  let hand = Quat.add(ahead(level, 0.25), Quat.v3(0.05, 0.0, 0.0)) in
  let s0 = support(fore, Hold.Left, levelHand, hand) in
  let steps = List.range(6.0) |> List.fold((acc, i) =>
    let s = List.last(acc) |> Option.defaultValue(s0) in
    acc |> List.append([grasp(fore, s, levelHand, Option.Some(hand), true) |> grasped |> Option.defaultValue(s)]), [s0]) in
  let aims = steps |> List.map((s) => s.aim) in
  Quat.angle(s0.aim, level.orientation) < 0.0001
    && (List.zip(List.drop(1.0, aims), aims) |> List.all((ab) => let (a, b) = ab in Quat.angle(a, b) <= Hold.snapTurnStep + 0.000001))
    && (List.last(aims) |> Option.map((a) => Hold.near3(fwd(a), unit(Quat.sub(hand, level.position)))) |> Option.defaultValue(false))
)
// Opened, it lets go; jerked 15 cm off the aim it holds; dragged 65 cm off
// it breaks off, opened or not.
expect (
  let s = support(fore, Hold.Left, levelHand, ahead(level, 0.25)) in
  let jerk = Quat.add(ahead(level, 0.25), Quat.v3(-0.15, 0.0, 0.0)) in
  let off = Quat.add(ahead(level, 0.25), Quat.v3(-0.65, 0.0, 0.0)) in
  grasp(fore, s, levelHand, Option.Some(ahead(level, 0.25)), false) == LetGo
    && grasp(fore, s, levelHand, Option.Some(jerk), true) != Broke
    && grasp(fore, s, levelHand, Option.Some(off), true) == Broke && grasp(fore, s, levelHand, Option.Some(off), false) == Broke
    && grasp(fore, s, levelHand, Option.None, true) == LetGo
)
// Overshooting the far end by 20 cm (a hard pump or a fast slide) holds.
expect grasp(fore, support(fore, Hold.Left, levelHand, ahead(level, 0.25)), levelHand, Option.Some(ahead(level, foreFar + 0.2)), true) != Broke
// Let go after aiming 20 degrees left: alone, the primary keeps the rifle
// exactly where the aim had it, then it settles back.
expect (
  let left = Quat.yaw(20.0 * Math.pi / 180.0) in
  let s = { support(fore, Hold.Left, levelHand, ahead(level, 0.25)) with ease: Quat.identity, aim: left } in
  let held = Hold.grab(Hold.Right, levelHand, Gun.heldAt(weapon, levelHand), Option.None) in
  Hold.near(Hold.compose(levelHand, (alone(fore, held, s, levelHand)).offset), Hold.compose(steer(fore, levelHand, s), held.offset))
)
// The rifle steered by the aim: its barrel along support - the layout's
// fist, the primary hand on its grip, the support fist on its foregrip line.
expect (
  let fist = Hold.at(Quat.v3(0.1, 1.3, -0.3), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.6)) in
  let primary = Hold.compose(fist, placement) in
  let hand = Quat.add(fist.position, Quat.v3(-0.08, 0.05, -0.28)) in
  let s = { support(fore, Hold.Left, primary, hand) with ease: Quat.identity } in
  let aimed = (grasp(fore, s, primary, Option.Some(hand), true) |> grasped |> Option.defaultValue(s)) in
  let rifle = Gun.heldAt(weapon, steer(fore, primary, aimed)) in
  let b = Gun.boreOf(weapon, rifle) in
  Hold.near3(b.dir, unit(Quat.sub(hand, fist.position)))
    && Hold.near3((Hold.compose(rifle, layout)).position, fist.position) && foreGap(fore, 0.0, rifle, hand) < 0.0001
)

// --- what the support hand is drawn at ---

/// Where the support hand at `hand` is drawn on foregrip `f` (slid back
/// `pull`) of a weapon accepted at `accepted` and wanted at `wanted`: carried
/// with it where the level stopped it short, its fist on the foregrip line,
/// between its ends.
let supportOn = (f: Fore, pull: float, accepted: Input.pose, wanted: Input.pose, hand: Input.pose): Input.pose =>
  let moved = Hold.compose(accepted, Hold.relative(wanted, hand)) in
  let local = (Hold.relative(accepted, moved)).position in
  let z = Math.clamp(f.layout.position.z - f.far + pull, f.layout.position.z - f.near + pull, local.z) in
  Hold.at(Quat.add(accepted.position, Quat.rotate(accepted.orientation, Quat.v3(f.layout.position.x, f.layout.position.y, z))), moved.orientation)

// Unblocked, a hand on the line is drawn where it is; stopped 10 cm short,
// it is drawn 10 cm back with the rifle; past the end, at the end.
expect (
  let rifle = Gun.heldAt(weapon, levelHand) in
  let hand = Hold.at(ahead(level, 0.3), Quat.yaw(0.3)) in
  let back = Hold.at(Quat.add(rifle.position, Quat.v3(0.0, 0.0, 0.1)), rifle.orientation) in
  Hold.near(supportOn(fore, 0.0, rifle, rifle, hand), hand)
    && Hold.near(supportOn(fore, 0.0, back, rifle, hand), Hold.at(Quat.add(hand.position, Quat.v3(0.0, 0.0, 0.1)), hand.orientation))
    && Hold.near3((supportOn(fore, 0.0, rifle, rifle, Hold.at(ahead(level, 0.6), Quat.identity))).position, ahead(level, foreFar))
)

// --- the look ---

let part = (sx: float, sy: float, sz: float, x: float, y: float, z: float) =>
  Scene.cube() |> Scene.scaleXYZ(sx, sy, sz) |> Scene.translate(Vec3.make(x - center.x, y - center.y, z - center.z))

/// The rifle in its own frame, from parts placed in the primary hand's (-Z
/// the barrel's way; turned a half turn at the end, as drawn its local +Z
/// is): stock, receiver, pistol grip, trigger, the handguard with
/// the foregrip rail under it (lighter: where the support hand goes), barrel
/// and sights (the magazine is `Gun.seated`); the charging handle `handle`
/// back and the bolt in the port on the right `bolt` back. `paint` colours
/// the furniture.
let scene = (paint: (float, float, float, Scene.t) => Scene.t, handle: float, bolt: float): Scene.t =>
  let dark = Color.rgb(0.08, 0.08, 0.09) in
  let metal = Color.rgb(0.16, 0.16, 0.18) in
  Scene.group([
    part(0.04, 0.08, 0.2, 0.0, 0.04, 0.18) |> paint(0.36, 0.32, 0.22),
    part(0.045, 0.05, 0.22, 0.0, 0.06, 0.0 - 0.03) |> Scene.lit(metal),
    Scene.cube() |> Scene.scaleXYZ(0.03, 0.09, 0.04) |> Scene.rotateX(Angle.radians(0.0 - 0.25))
      |> Scene.translate(Vec3.make(0.0 - center.x, 0.0 - center.y, 0.012 - center.z)) |> paint(0.36, 0.32, 0.22),
    part(0.008, 0.006, 0.05, 0.0, 0.016, 0.0 - 0.045) |> Scene.lit(dark),
    part(0.006, 0.02, 0.006, 0.0, 0.026, 0.0 - 0.035) |> Scene.lit(dark),
    part(0.045, 0.05, foreFar - foreNear, 0.0, bore, 0.0 - (foreNear + foreFar) / 2.0) |> paint(0.36, 0.32, 0.22),
    part(0.03, 0.03, foreFar - foreNear, 0.0, 0.02, 0.0 - (foreNear + foreFar) / 2.0) |> paint(0.6, 0.55, 0.42),
    part(0.016, 0.016, muzzleAhead - foreFar, 0.0, bore, 0.0 - (foreFar + muzzleAhead) / 2.0) |> Scene.lit(dark),
    part(0.005, 0.03, 0.008, 0.0, bore + 0.025, 0.0 - 0.5) |> Scene.emissive(Color.rgb(1.0, 0.55, 0.1)),
    part(0.02, 0.02, 0.01, 0.0, bore + 0.035, 0.06) |> Scene.lit(dark),
    // The charging handle's T, and the bolt (bright steel) in the port.
    part(0.045, 0.008, 0.014, handleAt.x, handleAt.y, handleAt.z + handle) |> Scene.lit(dark),
    part(0.004, 0.016, 0.05, 0.0235, 0.06, 0.0 - 0.03 + bolt) |> Scene.lit(Color.rgb(0.62, 0.62, 0.66)),
  ]) |> Scene.rotateY(180deg)

/// The selector's light on the receiver's left: green on semi, red on full-auto.
let selector = (mode: Gun.Mode): Scene.t =>
  let c = match mode with
    | Gun.Semi => Color.rgb(0.3, 1.0, 0.4)
    | Gun.Auto => Color.rgb(1.0, 0.25, 0.15) in
  part(0.006, 0.014, 0.02, 0.0 - 0.025, 0.06, 0.03) |> Scene.emissive(c) |> Scene.rotateY(180deg)
