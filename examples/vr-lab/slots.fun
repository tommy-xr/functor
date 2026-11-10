// Slots: places on the body a held prop can be put away in, and taken back
// from. Each sits at an authored offset in the hips' frame (`Hips`), never at
// a tracked joint, and holds one prop of the kinds it fits.

/// A slot: where it sits in the hips' frame (+X right, -Z forward), how near
/// its centre a hand must be to use it, and what it takes (one at a time):
/// these props, and magazines of these kinds.
type Slot = { id: float, name: string, offset: Input.pose, radius: float, fits: List<Props.Kind>, mags: List<Ammo.Kind> }

// The holsters hang at the sides of the thighs, just below the pelvis (the
// brick, the bottle or the pistol); the pouch sits on the belt in front (the
// ball, or a magazine).
let leftHolster: Slot = { id: 1.0, name: "left holster", offset: Hold.p(-0.2, -0.1, -0.02), radius: 0.08, fits: [Props.Brick, Props.Bottle, Props.Pistol], mags: [] }
let rightHolster: Slot = { id: 2.0, name: "right holster", offset: Hold.p(0.2, -0.1, -0.02), radius: 0.08, fits: [Props.Brick, Props.Bottle, Props.Pistol], mags: [] }
let pouch: Slot = { id: 3.0, name: "pouch", offset: Hold.p(0.0, 0.0, -0.15), radius: 0.08, fits: [Props.Ball], mags: [Ammo.PistolMag, Ammo.RifleMag] }

let all = [leftHolster, rightHolster, pouch]

/// The armory's rifle rack: not worn but fixed on the bench (`Game` places
/// it), a hand within `radius` of the racked rifle's centre using it.
let rack: Slot = { id: 4.0, name: "rifle rack", offset: Hold.origin, radius: 0.2, fits: [Props.Rifle], mags: [] }

/// The shotgun's rack beside it, the same way (its id past the cradles').
let shotgunRack: Slot = { id: 9.0, name: "shotgun rack", offset: Hold.origin, radius: 0.2, fits: [Props.Shotgun], mags: [] }

// A hand this much farther out than a slot's radius already pins the hips,
// so the slot holds still as the hand arrives.
let pinMargin = 0.1

let fits = (k: Props.Kind, s: Slot): bool =>
  match k with
  | Props.Mag(m, _) => s.mags |> List.any((f) => f == m)
  | _ => s.fits |> List.any((f) => f == k)

expect fits(Props.Brick, leftHolster) && fits(Props.Bottle, rightHolster) && fits(Props.Ball, pouch) && fits(Props.Pistol, rightHolster)
expect not fits(Props.Ball, leftHolster) && not fits(Props.Brick, pouch)
expect all |> List.all((s) => not fits(Props.Stick, s) && not fits(Props.Rifle, s))
/// A spare magazine's cradle on the armory bench (`Game` places it where the
/// spare `id` stands): a hand within `radius` of its centre uses it; it
/// takes a magazine of that kind. Like the rack, what it holds has no body.
let cradle = (id: float, kind: Ammo.Kind): Slot =>
  { id: rack.id + id, name: "magazine cradle", offset: Hold.origin, radius: 0.07, fits: [], mags: [kind] }

expect fits(Props.Rifle, rack) && not fits(Props.Pistol, rack) && fits(Props.Shotgun, shotgunRack) && not fits(Props.Rifle, shotgunRack)
expect all |> List.all((s) => not fits(Props.Shotgun, s) && not fits(Props.Round(Ammo.Shell, 1.0), s))
expect fits(Props.Mag(Ammo.RifleMag, 9.0), cradle(3.0, Ammo.RifleMag)) && not fits(Props.Mag(Ammo.PistolMag, 1.0), cradle(3.0, Ammo.RifleMag))
expect fits(Props.Mag(Ammo.PistolMag, 1.0), pouch) && fits(Props.Mag(Ammo.RifleMag, 7.0), pouch)
expect not fits(Props.Mag(Ammo.PistolMag, 1.0), rightHolster) && not fits(Props.Mag(Ammo.RifleMag, 1.0), rack)

/// A slot placed in the hips' frame (rig-local).
type Placed = { slot: Slot, pose: Input.pose }

/// Every slot where the hips put it; none before the hips have a head.
let placed = (h: Hips.State): List<Placed> =>
  match h.head with
  | Option.None => []
  | Option.Some(_) =>
    // `Hips.place`'s frame, turned once for every slot.
    let frame = Hips.frame(h) in
    all |> List.map((s) => { slot: s, pose: Hold.compose(frame, s.offset) })

let distance = (p: Placed, q: Input.point3): float => Quat.length(Quat.sub(q, p.pose.position))

/// The slot a hand at `q` is in, if any: the nearest whose radius it is inside.
let target = (slots: List<Placed>, q: Input.point3): Option.t<Placed> =>
  slots
    |> List.filter((p) => distance(p, q) <= p.slot.radius)
    |> List.sortBy((p) => distance(p, q))
    |> List.head

/// Whether a hand at `q` is near enough a slot to pin the hips.
let pins = (slots: List<Placed>, q: Input.point3): bool =>
  slots |> List.any((p) => distance(p, q) <= p.slot.radius + pinMargin)

/// How a slot is drawn: idle, offered a prop it would take, or offered one it
/// won't (it doesn't fit, or the slot is full).
type Look = | Idle | Ready | Refused

/// A slot's look, for the props the holding hands in it offer: ready if it
/// would take one of them.
let look = (s: Slot, occupied: bool, offered: List<Props.Kind>): Look =>
  if List.isEmpty(offered) then Idle
  else if not occupied && (offered |> List.any((k) => fits(k, s))) then Ready
  else Refused

expect look(leftHolster, false, []) == Idle
expect look(leftHolster, false, [Props.Brick]) == Ready
expect look(leftHolster, true, [Props.Brick]) == Refused
expect look(pouch, false, [Props.Stick]) == Refused
expect look(leftHolster, false, [Props.Stick, Props.Brick]) == Ready

// The slots ride the hips: placed at the authored offset from the root,
// turned with them.
expect (
  let h = { Hips.init with head: Option.Some(Hold.origin), yaw: Math.pi / 2.0, root: Quat.v3(1.0, 0.9, 2.0) } in
  match placed(h) with
  | [l, r, p] =>
    // Turned a quarter turn left, the hips' right is rig -Z.
    Hold.near3(r.pose.position, Quat.v3(0.98, 0.8, 1.8)) && Hold.near3(p.pose.position, Quat.v3(0.85, 0.9, 2.0))
      && target(placed(h), Quat.v3(0.98, 0.85, 1.8)) == Option.Some(r) && Option.isNone(target(placed(h), Quat.v3(0.98, 0.7, 1.8)))
      && pins(placed(h), Quat.v3(0.98, 0.65, 1.8)) && not pins(placed(h), Quat.v3(0.98, 0.6, 1.8))
  | _ => false
)
expect placed(Hips.init) == []

// --- drawing ---

// A dashed ring of radius 1, level: one instanced cube, its copies a
// constant list (uploaded once).
let dashes =
  List.range(12.0) |> List.map((i) =>
    let a = i * Math.pi / 6.0 in
    Instance.trs(Vec3.make(Math.sin(a), 0.0, Math.cos(a)), Angle.radians(a), 0.32, 0.1, 0.12))

/// A slot in its own frame: a ring round its reach and a ghost of it, the pouch's amber and
/// the holsters' steel when idle, green when offered what it takes, red when
/// offered what it won't.
let scene = (s: Slot, l: Look): Scene.t =>
  let idle = if fits(Props.Ball, s) then Color.rgb(0.95, 0.7, 0.3) else Color.rgb(0.55, 0.65, 0.9) in
  let (paint, ghost) =
    match l with
    | Idle => ((x: Scene.t) => x |> Scene.lit(idle), 0.12)
    | Ready => ((x: Scene.t) => x |> Scene.emissive(Color.rgb(0.35, 1.0, 0.45)), 0.25)
    | Refused => ((x: Scene.t) => x |> Scene.emissive(Color.rgb(1.0, 0.25, 0.2)), 0.25) in
  Scene.group([
    Scene.cube() |> paint |> Scene.instanced(dashes) |> Scene.scale(s.radius),
    Scene.sphere() |> Scene.scale(s.radius) |> paint |> Scene.opacity(ghost),
  ])
