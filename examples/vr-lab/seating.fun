// Seating: a held magazine guided into its weapon's well, pure and rig-local
// (`Hold`'s frame). Near the mouth (where an ejected one drops out), lined up
// with the well, it latches (`latches`). From then it sits on the well's
// axis, turned as the well is, at the depth the hand pushes it to
// (`depthOf`), easing in from where it was over a few steps (`lag`). Pushed
// home it seats; pulled off the axis past the guide's exit, or turned off it,
// it comes back out where it is, into the hand (`Leaves`). The weapon may be
// held, or anywhere else: it is only a pose here.

/// A latched magazine: the weapon it is going into, how deep (0 seated,
/// negative out along the axis), and how far it still is off the well's pose
/// (in that pose's frame), easing out.
type Latch = { gun: Props.Kind, depth: float, lag: Input.pose }

let up = (q: Input.quaternion): Input.point3 => Quat.rotate(q, Quat.v3(0.0, 1.0, 0.0))
let back = (q: Input.quaternion): Input.point3 => Quat.rotate(q, Quat.v3(0.0, 0.0, 1.0))

/// A pose `depth` along the well of a weapon `w` at `gun` (rig-local).
let along = (w: Gun.Weapon, gun: Input.pose, depth: float): Input.pose => Hold.compose(gun, Gun.wellAt(w, depth))

/// Whether a magazine turned `q` lines up with a well turned `well`: its
/// axis within `axisDot` of the well's, and (keyed) its back within the
/// guide's `keyDot`.
let lined = (g: Gun.Guide, axisDot: float, well: Input.quaternion, q: Input.quaternion): bool =>
  Quat.dot(up(q), up(well)) >= axisDot && Quat.dot(back(q), back(well)) >= g.keyDot

/// A point seen from the well: how deep along its axis (0 seated, negative
/// out), and how far off it.
type Place = { depth: float, off: float }

let placeOf = (w: Gun.Weapon, gun: Input.pose, p: Input.point3): Place =>
  let local = (Hold.relative(along(w, gun, 0.0), Hold.at(p, Quat.identity))).position in
  { depth: local.y, off: Math.sqrt(local.x * local.x + local.z * local.z) }

/// How far a place is from the well: off its axis from the mouth in, and
/// from the mouth below it.
let fromWell = (w: Gun.Weapon, pl: Place): float =>
  let below = Math.max(0.0, 0.0 - w.well.drop - pl.depth) in
  Math.sqrt(pl.off * pl.off + below * below)

/// Whether a magazine of `kind` at `mag` latches into the well of a weapon
/// `w` at `gun` holding `ammo`: it fits and the well is empty, its centre is
/// within the guide's `entry` of where it sits at the mouth, lined up.
let latches = (w: Gun.Weapon, gun: Input.pose, ammo: Ammo.State, kind: Ammo.Kind, mag: Input.pose): bool =>
  let g = w.well.guide in
  kind == w.well.kind && Option.isNone(ammo.mag)
    && Quat.length(Quat.sub(mag.position, (along(w, gun, 0.0 - w.well.drop)).position)) <= g.entry
    && lined(g, g.alignDot, (along(w, gun, 0.0)).orientation, mag.orientation)

/// Latched where it is, `mag`: as deep as it is (no deeper than seated), the
/// rest of the way onto the well's pose still to ease out.
let latch = (k: Props.Kind, w: Gun.Weapon, gun: Input.pose, mag: Input.pose): Latch =>
  let depth = Math.min(0.0, (placeOf(w, gun, mag.position)).depth) in
  { gun: k, depth: depth, lag: Hold.relative(along(w, gun, depth), mag) }

/// Where a latched magazine is.
let poseOf = (w: Gun.Weapon, gun: Input.pose, l: Latch): Input.pose => Hold.compose(along(w, gun, l.depth), l.lag)

/// What a latched step did: still guided, seated, or pulled out (it stays
/// where it is, now in the hand).
type Guided = | Guiding(latch: Latch) | Seats | Leaves

/// Pushed this near home (and eased onto the axis), it seats.
let seatSlack = 0.004

/// The lag one step further out: at most `Hold.snapStep` and
/// `Hold.snapTurnStep`, as a grip settles.
let ease = (lag: Input.pose): Input.pose =>
  let steps = Math.max(Quat.length(lag.position) / Hold.snapStep, Quat.angle(lag.orientation, Quat.identity) / Hold.snapTurnStep) in
  if steps <= 1.0 then Hold.origin else Hold.interpolate(lag, Hold.origin, 1.0 / steps)

/// One step of a latched magazine, the hand holding it as if at `want` (its
/// grip composed with the hold's offset). Off the well past the guide's
/// `exit`, or its axis past `unalignDot`: it leaves. Else it follows the
/// hand's depth along the axis (no deeper than seated), and seats there.
let step = (w: Gun.Weapon, gun: Input.pose, l: Latch, want: Input.pose): Guided =>
  let g = w.well.guide in
  let pl = placeOf(w, gun, want.position) in
  if fromWell(w, pl) > g.exit || Quat.dot(up(want.orientation), up((along(w, gun, 0.0)).orientation)) < g.unalignDot then Leaves
  else
    let next = { l with depth: Math.min(0.0, pl.depth), lag: ease(l.lag) } in
    if next.depth >= 0.0 - seatSlack && next.lag == Hold.origin then Seats else Guiding(next)

// --- the truth tables, at the pistol's well, the pistol held turned ---

let w = Gun.pistol
let gun = Hold.at(Quat.v3(0.1, 1.2, -0.35), Quat.mul(Quat.yaw(0.4), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.3)))
let empty: Ammo.State = { Ammo.loaded(Ammo.PistolMag) with mag: Option.None, chambered: false, locked: true }
let mouth = along(w, gun, 0.0 - w.well.drop)
/// The mouth's pose moved `d` in the well's frame, turned `q` there.
let near = (d: Input.point3, q: Input.quaternion): Input.pose => Hold.compose(mouth, Hold.at(d, q))
let degrees = (a: float): float => a * Math.pi / 180.0
let about = (axis: Input.point3, deg: float): Input.quaternion => Quat.axisAngle(axis, degrees(deg))
let x = Quat.v3(1.0, 0.0, 0.0)
let y = Quat.v3(0.0, 1.0, 0.0)
let latchesAt = (p: Input.pose): bool => latches(w, gun, empty, Ammo.PistolMag, p)

// At the mouth, lined up: latches. Within `entry` (3.9 cm off) yes; past it
// (4.1 cm, any way) no.
expect latchesAt(mouth)
expect latchesAt(near(Quat.v3(0.039, 0.0, 0.0), Quat.identity)) && latchesAt(near(Quat.v3(0.0, 0.0 - 0.039, 0.0), Quat.identity))
expect not latchesAt(near(Quat.v3(0.041, 0.0, 0.0), Quat.identity)) && not latchesAt(near(Quat.v3(0.0, 0.0 - 0.041, 0.0), Quat.identity))
  && not latchesAt(near(Quat.v3(0.0, 0.0, 0.041), Quat.identity))
// Tipped off the axis: 31° (dot 0.857) latches; 33° (0.839) doesn't.
expect latchesAt(near(Hold.zero, about(x, 31.0))) && not latchesAt(near(Hold.zero, about(x, 33.0)))
expect latchesAt(near(Hold.zero, about(Quat.v3(0.0, 0.0, 1.0), -31.0))) && not latchesAt(near(Hold.zero, about(Quat.v3(0.0, 0.0, 1.0), -33.0)))
// Upside down (reversed): never.
expect not latchesAt(near(Hold.zero, about(x, 180.0)))
// Keyed: turned about its own axis up to 90° it latches; back to front, no.
expect latchesAt(near(Hold.zero, about(y, 80.0))) && not latchesAt(near(Hold.zero, about(y, 100.0))) && not latchesAt(near(Hold.zero, about(y, 180.0)))
// A rifle magazine, or a well already full: never.
expect not latches(w, gun, empty, Ammo.RifleMag, mouth)
expect not latches(w, gun, { empty with mag: Option.Some(Ammo.full(Ammo.PistolMag)) }, Ammo.PistolMag, mouth)
// The rifle's well takes its own, at its own mouth.
expect latches(Longarm.weapon, gun, { empty with well: Ammo.RifleMag }, Ammo.RifleMag, along(Longarm.weapon, gun, 0.0 - Longarm.weapon.well.drop))

// --- guided ---

let latched = latch(Props.Pistol, w, gun, near(Quat.v3(0.03, 0.0, 0.0), about(x, 25.0)))
let stepped = (want: Input.pose) => step(w, gun, latched, want)
let leaves = (g: Guided): bool => g == Leaves
let guidedDepth = (g: Guided): float =>
  match g with
  | Guiding(l) => l.depth
  | _ => 1.0

// Latched where it is: no pop.
expect Hold.near(poseOf(w, gun, latched), near(Quat.v3(0.03, 0.0, 0.0), about(x, 25.0))) && Math.abs(latched.depth + w.well.drop) < 0.0001
// Hysteresis: 5 cm off the mouth never latches, but latched it holds there;
// out to 6.9 cm off the axis (at any depth) or below the mouth it holds, at
// 7.1 cm it leaves. Tipped 44° (dot 0.72) it holds; 47° (0.68), it leaves.
expect not latchesAt(near(Quat.v3(0.05, 0.0, 0.0), Quat.identity)) && not leaves(stepped(near(Quat.v3(0.05, 0.0, 0.0), Quat.identity)))
expect not leaves(stepped(near(Quat.v3(0.069, 0.06, 0.0), Quat.identity))) && leaves(stepped(near(Quat.v3(0.071, 0.06, 0.0), Quat.identity)))
expect not leaves(stepped(near(Quat.v3(0.0, 0.0 - 0.069, 0.0), Quat.identity))) && leaves(stepped(near(Quat.v3(0.0, 0.0 - 0.071, 0.0), Quat.identity)))
expect not leaves(stepped(near(Hold.zero, about(x, 44.0)))) && leaves(stepped(near(Hold.zero, about(x, 47.0))))
// Depth is the hand's projected onto the axis, whatever its offset or turn
// (5 cm in, 3 cm aside, turned 20°: 5 cm in); pushed past home, home.
expect Math.abs(guidedDepth(stepped(near(Quat.v3(0.03, 0.05, 0.01), about(y, 20.0)))) - (0.05 - w.well.drop)) < 0.0001
expect Math.abs(guidedDepth(stepped(near(Quat.v3(0.0, 0.2, 0.0), Quat.identity))) - 0.0) < 0.0001

/// The poses a latched magazine is guided through as the hand goes `wants`.
let guided = (l0: Latch, wants: List<Input.pose>): (List<Input.pose>, Guided) =>
  wants |> List.fold((acc, want) =>
    let (ps, last) = acc in
    match last with
    | Guiding(l) =>
      let next = step(w, gun, l, want) in
      (match next with
       | Guiding(l2) => (ps |> List.append([poseOf(w, gun, l2)]), next)
       | _ => (ps, next))
    | _ => acc, ([poseOf(w, gun, l0)], Guiding(l0)))

// Held 3 cm aside and turned 25° at the mouth: it eases onto the axis, at
// most `Hold.snapStep` and `Hold.snapTurnStep` a step, then sits turned as
// the well is; pushed home along the axis, it seats.
expect (
  let hold = List.range(6.0) |> List.map((_) => near(Quat.v3(0.03, 0.0, 0.0), about(x, 25.0))) in
  let (ps, last) = guided(latched, hold) in
  let moves = List.zip(List.drop(1.0, ps), ps) in
  (moves |> List.all((ab) => let (a, b) = ab in
     Quat.length(Quat.sub(a.position, b.position)) <= Hold.snapStep + 0.000001 && Quat.angle(a.orientation, b.orientation) <= Hold.snapTurnStep + 0.000001))
    && (List.last(ps) |> Option.map((p) => Hold.near(p, mouth)) |> Option.defaultValue(false))
    && (match last with | Guiding(_) => true | _ => false)
)
expect (
  let push = List.range(12.0) |> List.map((i) => near(Quat.v3(0.03, w.well.drop * (i + 1.0) / 10.0, 0.0), about(x, 25.0))) in
  let (ps, last) = guided(latched, push) in
  last == Seats && (List.last(ps) |> Option.map((p) => Quat.length(Quat.sub(p.position, (along(w, gun, 0.0)).position)) < 0.02) |> Option.defaultValue(false))
)
// Pushed home before it has eased onto the axis, it waits for it.
expect (match stepped(near(Quat.v3(0.03, w.well.drop, 0.0), about(x, 25.0))) with | Guiding(l) => Math.abs(l.depth) < 0.000001 | _ => false)
