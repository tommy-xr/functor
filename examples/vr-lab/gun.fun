// Gun: what every weapon shares, pure and rig-local (`Hold`'s frame): the
// trigger and its fire modes, the bore a shot leaves along, the shot, the
// muzzle check and the recoil kick; the magazine look; and the pistol's shape
// and authored grip. A weapon is data (`Weapon`): its grip, bore, magazine
// well and readout, and whether it has a selector (the rifle's is in
// `Longarm`). Its rounds are `Ammo`'s. The world reaches in only through the
// shot's ray and the muzzle check (`Game.World`).

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

/// How a magazine (or shell) is guided into a well (`Seating`). It latches
/// when its centre is within `entry` of where it sits at the well's mouth and
/// its axis (local +Y) is within `alignDot` of the well's; keyed, its back
/// (+Z) also within `keyDot` of the well's (`anyTurn`: any roll). Latched, it holds
/// while the hand keeps it within `exit` of the well's axis (below the mouth,
/// of the mouth), its axis within `unalignDot` and its back within `unkeyDot`.
type Guide = { entry: float, exit: float, alignDot: float, unalignDot: float, keyDot: float, unkeyDot: float }

/// A guide's dot threshold that passes any turn (below -1, so a dot of
/// exactly -1 that rounds past it still passes).
let anyTurn = 0.0 - 2.0

/// The guide both magazine wells use: a 4 cm catch, 7 cm to pull out of;
/// about 32° off the axis to latch, 46° to lose it; keyed (not turned back to
/// front), any turn about its axis once latched.
let guide: Guide = { entry: 0.04, exit: 0.07, alignDot: 0.85, unalignDot: 0.7, keyDot: 0.0, unkeyDot: anyTurn }

/// A weapon's magazine well, in its frame: the magazine kind it takes, where
/// a seated one's centre sits, raked back `rake` (radians about +X), and how
/// far down that axis an ejected one drops out (clear of the collider): the
/// mouth, where an inserted one starts. And how it guides one in.
type Well = { kind: Ammo.Kind, at: Input.point3, rake: float, drop: float, guide: Guide }

/// The slide (or charging handle, or pump) a hand racks, in its weapon's
/// frame: the zone a hand takes it in (`at`, within `radius`), how far back
/// it goes (`travel`, along +Z), where it and the bolt rest while locked back
/// (`lockBack`, `boltLock`: the pistol's slide is its bolt and stays back;
/// the rifle's handle goes home, its bolt doesn't), and the port a
/// thrown-out round leaves from. A `pump` is worked by the support hand on
/// the fore-end (not taken in a zone) and stays where it is let go.
type Rack = { at: Input.point3, radius: float, travel: float, lockBack: float, boltLock: float, port: Input.point3, pump: bool }

/// Where its ammo readout sits, in its frame, readable from behind (+Z),
/// tipped `tilt` (radians about +X) toward the eye, each digit `size` wide.
type Readout = { at: Input.point3, tilt: float, size: float }

/// A weapon, in its own frame (centred on its collider): the authored grip
/// (the trigger hand's pose), the chamber and the muzzle (the bore runs
/// along the frame's -Z between them), whether a selector switches it to
/// full-auto, its magazine well, its readout and its slide; how many
/// projectiles a shot casts (`pellets`, the rest `spread` radians round the
/// first), and its recoil's size (1: the pistol's).
type Weapon = { grip: Input.pose, chamber: Input.point3, muzzle: Input.point3, selector: bool, well: Well, readout: Readout, rack: Rack, pellets: float, spread: float, recoil: float }

/// A weapon whose bore runs `bore` above the grip's fist, parallel to its
/// -Z, from `chamberAhead` to `muzzleAhead` ahead of it: one bullet a shot.
let weaponAt = (g: Input.pose, bore: float, chamberAhead: float, muzzleAhead: float, selector: bool, well: Well, readout: Readout, rack: Rack): Weapon =>
  let along = (ahead: float) => Quat.add(g.position, Quat.rotate(g.orientation, Quat.v3(0.0, bore, 0.0 - ahead))) in
  { grip: g, chamber: along(chamberAhead), muzzle: along(muzzleAhead), selector: selector, well: well, readout: readout, rack: rack,
    pellets: 1.0, spread: 0.0, recoil: 1.0 }

// The handle: its centre in the hand's frame, raked back 12 degrees.
let handleAt = Quat.v3(0.0, 0.0 - 0.015, 0.02)
let handleRake = 0.0 - 0.21

// How far the pistol's slide goes back.
let slideTravel = 0.04

// The magazine in the handle, raked with it; its readout on the slide's back
// face; the slide taken at its rear serrations, the port on its right.
let pistol = weaponAt(grip, bore, chamberAhead, muzzleAhead, false,
  { kind: Ammo.PistolMag, at: Quat.sub(handleAt, center), rake: handleRake, drop: 0.12, guide: guide },
  { at: Quat.sub(Quat.v3(0.0, bore, 0.056), center), tilt: 0.0, size: 0.0045 },
  { at: Quat.sub(Quat.v3(0.0, bore, 0.04), center), radius: 0.035, travel: slideTravel, lockBack: 0.7 * slideTravel, boltLock: 0.7 * slideTravel,
    port: Quat.sub(Quat.v3(0.03, bore, 0.0 - 0.01), center), pump: false })

/// A seated magazine's pose in its weapon's frame.
let seatOf = (w: Weapon): Input.pose => Hold.at(w.well.at, Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), w.well.rake))

/// A magazine `depth` along its well's axis from seated (0; negative, out
/// of it), in its weapon's frame.
let wellAt = (w: Weapon, depth: float): Input.pose => Hold.compose(seatOf(w), Hold.p(0.0, depth, 0.0))

/// Where an ejected magazine appears, in its weapon's frame: dropped down
/// the well's axis, below the weapon's collider. An inserted one starts
/// there, at the mouth.
let dropOf = (w: Weapon): Input.pose => wellAt(w, 0.0 - w.well.drop)

// --- magazines ---

/// A magazine's bounds (its collider as a prop): the pistol's, a body
/// inside the handle and a wider base plate; the rifle's, a box. A shell (a
/// 12 gauge, 20 mm by 70) goes in by itself, lying along its +Z: its brass
/// head back.
let magSize = (k: Ammo.Kind): Input.point3 =>
  match k with
  | Ammo.PistolMag => Quat.v3(0.032, 0.1, 0.047)
  | Ammo.RifleMag => Quat.v3(0.025, 0.09, 0.05)
  | Ammo.Shell => Quat.v3(0.02, 0.02, 0.07)

/// How far a magazine's back is from its centre (where its count goes,
/// facing back at the shooter).
let magFace = (k: Ammo.Kind): float =>
  match k with
  | Ammo.PistolMag => 0.016
  | Ammo.RifleMag => 0.025
  | Ammo.Shell => 0.035

/// A magazine in its own frame (centred on its bounds), `paint` colouring
/// its body: the pistol's base plate tan, so a seated one shows under the
/// handle.
let magazine = (k: Ammo.Kind, paint: (float, float, float, Scene.t) => Scene.t): Scene.t =>
  let e = magSize(k) in
  match k with
  | Ammo.PistolMag =>
    Scene.group([
      Scene.cube() |> Scene.scaleXYZ(0.026, e.y - 0.008, 2.0 * magFace(k)) |> Scene.translate(Vec3.make(0.0, 0.004, 0.0)) |> paint(0.12, 0.12, 0.13),
      Scene.cube() |> Scene.scaleXYZ(e.x, 0.008, e.z) |> Scene.translate(Vec3.make(0.0, 0.004 - e.y / 2.0, 0.0)) |> paint(0.75, 0.6, 0.3),
    ])
  | Ammo.RifleMag => Scene.cube() |> Scene.scaleXYZ(e.x, e.y, e.z) |> paint(0.16, 0.16, 0.18)
  | Ammo.Shell => shell(paint, false)

/// A shell in its own frame: a red hull, its brass head at the back (+Z;
/// drawn, local -Z). Spent (a fired hull), its hull dull and open.
let shell = (paint: (float, float, float, Scene.t) => Scene.t, spent: bool): Scene.t =>
  let e = magSize(Ammo.Shell) in
  let (r, g, b) = if spent then (0.35, 0.14, 0.12) else (0.8, 0.12, 0.1) in
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(0.9 * e.x, 0.9 * e.y, 0.8 * e.z) |> Scene.translate(Vec3.make(0.0, 0.0, 0.1 * e.z)) |> paint(r, g, b),
    Scene.cube() |> Scene.scaleXYZ(e.x, e.y, 0.2 * e.z) |> Scene.translate(Vec3.make(0.0, 0.0, 0.0 - 0.4 * e.z)) |> paint(0.85, 0.65, 0.25),
  ])

/// A magazine scene placed in its weapon's well, drawn with the weapon.
let seated = (w: Weapon, mag: Scene.t): Scene.t =>
  mag |> Scene.rotateX(Angle.radians(w.well.rake)) |> Scene.translate(Quat.toVec3(w.well.at)) |> Scene.rotateY(180deg)

// Dropped, a magazine's bounds (turned with the well) are wholly below its
// weapon's collider, so it never starts inside it.
let clearBelow = (w: Weapon, collider: Input.point3): bool =>
  let d = dropOf(w) in
  let e = magSize(w.well.kind) in
  let top = d.position.y + Math.abs(e.y / 2.0 * Math.cos(w.well.rake)) + Math.abs(e.z / 2.0 * Math.sin(w.well.rake)) in
  top < 0.0 - collider.y / 2.0
expect clearBelow(pistol, size)

/// A shot's line from a weapon at `pose`: the chamber, the muzzle, and the
/// barrel's way (unit).
type Bore = { chamber: Input.point3, muzzle: Input.point3, dir: Input.point3 }

let boreOf = (w: Weapon, pose: Input.pose): Bore =>
  let at = (p: Input.point3) => Quat.add(pose.position, Quat.rotate(pose.orientation, p)) in
  { chamber: at(w.chamber), muzzle: at(w.muzzle), dir: Quat.rotate(pose.orientation, Quat.v3(0.0, 0.0, 0.0 - 1.0)) }

/// A weapon held settled in a hand at `hand`: where it is.
let heldAt = (w: Weapon, hand: Input.pose): Input.pose => Hold.compose(hand, Hold.inverse(w.grip))

// Level, yawed, pitched and rolled hands: the muzzle 5 cm above the hand's
// -Z axis and 15 cm along it, and the shot along that axis.
expect (
  let b = boreOf(pistol, heldAt(pistol, Hold.p(0.1, 1.2, -0.3))) in
  Hold.near3(b.muzzle, Quat.v3(0.1, 1.25, -0.45)) && Hold.near3(b.chamber, Quat.v3(0.1, 1.25, -0.32)) && Hold.near3(b.dir, Quat.v3(0.0, 0.0, -1.0))
)
expect (
  // A quarter turn left: the barrel points along -X.
  let b = boreOf(pistol, heldAt(pistol, Hold.at(Quat.v3(0.1, 1.2, -0.3), Quat.yaw(Math.pi / 2.0)))) in
  Hold.near3(b.muzzle, Quat.v3(-0.05, 1.25, -0.3)) && Hold.near3(b.dir, Quat.v3(-1.0, 0.0, 0.0))
)
expect (
  // Tipped 30 degrees up.
  let a = Math.pi / 6.0 in
  let b = boreOf(pistol, heldAt(pistol, Hold.at(Quat.v3(0.0, 1.0, 0.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), a)))) in
  Hold.near3(b.dir, Quat.v3(0.0, Math.sin(a), 0.0 - Math.cos(a)))
    && Hold.near3(b.muzzle, Quat.v3(0.0, 1.0 + 0.05 * Math.cos(a) + 0.15 * Math.sin(a), 0.05 * Math.sin(a) - 0.15 * Math.cos(a)))
)
expect (
  // Rolled a quarter turn clockwise (about -Z): the bore is beside the hand,
  // on its right, still parallel to its -Z.
  let b = boreOf(pistol, heldAt(pistol, Hold.at(Quat.v3(0.0, 1.0, 0.0), Quat.axisAngle(Quat.v3(0.0, 0.0, -1.0), Math.pi / 2.0)))) in
  Hold.near3(b.muzzle, Quat.v3(0.05, 1.0, -0.15)) && Hold.near3(b.dir, Quat.v3(0.0, 0.0, -1.0))
)
expect (
  // Yawed, pitched and rolled at once: the muzzle is the hand's (0, 5, -15) cm.
  let q = Quat.mul(Quat.yaw(0.6), Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), -0.3), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 1.1))) in
  let hand = Hold.at(Quat.v3(0.2, 1.3, -0.4), q) in
  let b = boreOf(pistol, heldAt(pistol, hand)) in
  Hold.near3(b.muzzle, Quat.add(hand.position, Quat.rotate(q, Quat.v3(0.0, 0.05, -0.15))))
    && Hold.near3(b.dir, Quat.rotate(q, Quat.v3(0.0, 0.0, -1.0)))
)

// --- the trigger ---
// Analog, with hysteresis: a pull to `fireAt` fires; it must fall back to
// `rearmAt` before the next pull counts. Semi-automatic, one shot per pull;
// full-auto, one on the pull and one every `stepsPerShot` steps it is held.

let fireAt = 0.8
let rearmAt = 0.3
/// Full-auto's rate: 10 rounds a second at the 60 Hz fixed step, counted in
/// steps so it is the same on every run.
let stepsPerShot = 6.0

type Mode = | Semi | Auto

/// `pulled`: past `fireAt` and not yet back to `rearmAt`. `armed`: seen at or
/// under `rearmAt` since the weapon was taken or its hand was last tracked,
/// so a trigger already held then never fires. `since`: steps held since the
/// last shot.
type Trigger = { pulled: bool, armed: bool, since: float }

let disarmed: Trigger = { pulled: false, armed: false, since: 0.0 }

/// The trigger at `value`: its new state, and whether this step fires.
let pull = (mode: Mode, t: Trigger, value: float): (Trigger, bool) =>
  let pulled = if t.pulled then value > rearmAt else value >= fireAt in
  let edge = t.armed && pulled && not t.pulled in
  let again = mode == Auto && t.armed && pulled && t.pulled && t.since + 1.0 >= stepsPerShot in
  let fires = edge || again in
  ({ pulled: pulled, armed: t.armed || value <= rearmAt, since: if fires then 0.0 else t.since + 1.0 }, fires)

/// Pull through `values` from `t`: how many shots.
let shotsFrom = (mode: Mode, t: Trigger, values: List<float>): float =>
  let (_, n) = values |> List.fold((acc, v) =>
    let (s, k) = acc in
    let (next, fires) = pull(mode, s, v) in
    (next, if fires then k + 1.0 else k), (t, 0.0)) in
  n

let ready = { pulled: false, armed: true, since: 0.0 }
let held = (n: float): List<float> => List.range(n) |> List.map((_) => 1.0)
expect shotsFrom(Semi, ready, [0.79, 0.5, 0.79]) == 0.0
expect shotsFrom(Semi, ready, [0.8]) == 1.0
// Held, or wavering above `rearmAt`, it is one shot; released to it, two.
expect shotsFrom(Semi, ready, held(30.0)) == 1.0
expect shotsFrom(Semi, ready, [0.9, 0.31, 0.9, 0.5, 1.0]) == 1.0
expect shotsFrom(Semi, ready, [0.9, 0.3, 0.9]) == 2.0
// Taken already pulled: no shot until it is let out and pulled again.
expect shotsFrom(Semi, disarmed, [1.0, 1.0, 1.0]) == 0.0
expect shotsFrom(Semi, disarmed, [1.0, 0.5, 0.9]) == 0.0
expect shotsFrom(Semi, disarmed, [1.0, 0.2, 0.9]) == 1.0
// Full-auto: pulled, then held N more steps, fires floor(N / stepsPerShot) + 1.
expect [0.0, 1.0, 5.0, 6.0, 11.0, 12.0, 29.0, 30.0, 59.0] |> List.all((n) =>
  shotsFrom(Auto, ready, held(n + 1.0)) == Math.floor(n / stepsPerShot) + 1.0)
// Held between the thresholds it keeps cycling; let out and pulled again, the
// count starts over from the new pull.
expect shotsFrom(Auto, ready, [1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]) == 2.0
expect shotsFrom(Auto, ready, List.flatten([held(4.0), [0.2], held(1.0)])) == 2.0
// Taken already pulled, full-auto fires nothing either until it is let out.
expect shotsFrom(Auto, disarmed, held(30.0)) == 0.0

// --- the slide ---
// A hand takes the slide (or charging handle) in its zone and pulls it back
// along the bore (+Z), clamped to its travel. Past `backAt` of the way: back,
// a chambered round thrown out (`Ammo.extract`). Then let go, or returned
// under `homeAt`, it goes home and feeds (`Ammo.close`). Short of `backAt`,
// nothing happens and it springs home.

let backAt = 0.8
let homeAt = 0.2

/// The hand on the slide (none: it rests, home or locked back), where along
/// the bore (+Z, the weapon's frame) the hand would have it home, how far
/// back it is, and whether this stroke has reached `backAt`.
type Slide = { hand: Option.t<Hold.Side>, from: float, pull: float, back: bool }

let home: Slide = { hand: Option.None, from: 0.0, pull: 0.0, back: false }

/// What a slide step did: it reached the back, or went home from there.
type Stroke = | Back | Home

/// Where the slide (or handle) rests untouched: back while locked, if it
/// stays there (`lockBack`).
let rest = (r: Rack, locked: bool): float => if locked then r.lockBack else 0.0

/// A hand at `z` (along the bore, the weapon's frame) takes the slide where
/// it rests.
let take = (r: Rack, locked: bool, side: Hold.Side, z: float): Slide =>
  { hand: Option.Some(side), from: z - rest(r, locked), pull: rest(r, locked), back: false }

/// A hand at `z` takes a pump where it was let go, mid-stroke or not.
let takePump = (s: Slide, side: Hold.Side, z: float): Slide => { s with hand: Option.Some(side), from: z - s.pull }

/// A pump let go: it stays where it is.
let letGoPump = (s: Slide): Slide => { s with hand: Option.None }

/// Whether a slide is closed, so the gun can fire: home (or locked back,
/// which `Ammo` keeps from chambering). A pump closes once it is home from
/// any stroke, and within `homeAt` of it, so a hand back where it took the
/// fore-end needn't find the exact spot.
let inBattery = (r: Rack, s: Slide): bool =>
  if r.pump then not s.back && s.pull <= homeAt * r.travel else s.pull <= 0.0

/// One step of a held slide, its hand at `z` (None: let go, lost, or off the
/// rail): the slide, and its strokes.
let pullTo = (r: Rack, s: Slide, z: Option.t<float>): (Slide, List<Stroke>) =>
  match z with
  | Option.None => (home, if s.back then [Home] else [])
  | Option.Some(at) =>
    let pull = Math.clamp(0.0, r.travel, at - s.from) in
    if not s.back && pull >= backAt * r.travel then ({ s with pull: pull, back: true }, [Back])
    else if s.back && pull <= homeAt * r.travel then ({ s with pull: pull, back: false }, [Home])
    else ({ s with pull: pull }, [])

/// The strokes of a hand taking the slide at 0, moved through `zs`, let go.
let strokes = (r: Rack, locked: bool, zs: List<float>): List<Stroke> =>
  let (s, all) = zs |> List.fold((acc, z) =>
    let (s, out) = acc in
    let (next, now) = pullTo(r, s, Option.Some(z)) in
    (next, out |> List.append(now)), (take(r, locked, Hold.Left, 0.0), [])) in
  let (_, last) = pullTo(r, s, Option.None) in
  all |> List.append(last)

let ramp = (to: float, n: float): List<float> => List.range(n) |> List.map((i) => to * (i + 1.0) / n)
// Pulled all the way and let go: back, then home. Short of `backAt` and let
// go, or returned: nothing.
expect strokes(pistol.rack, false, ramp(slideTravel, 8.0)) == [Back, Home]
expect strokes(pistol.rack, false, ramp(0.79 * slideTravel, 8.0)) == []
expect strokes(pistol.rack, false, ramp(0.7 * slideTravel, 6.0) |> List.append(List.reverse(ramp(0.7 * slideTravel, 6.0)))) == []
// Back, then returned under `homeAt` by the hand: home there; pulled again, a
// second stroke.
expect strokes(pistol.rack, false, [0.04, 0.02, 0.007, 0.0, 0.04]) == [Back, Home, Back, Home]
// Clamped to its travel.
expect (let (s, _) = pullTo(pistol.rack, take(pistol.rack, false, Hold.Left, 0.0), Option.Some(0.2)) in s.pull == slideTravel && s.back)
// Locked back, it is taken where it rests (70% back): nudged past 80% and let
// go, a stroke; let go untouched, none.
expect (let s = take(pistol.rack, true, Hold.Left, 0.5) in s.pull == pistol.rack.lockBack && s.from == 0.5 - pistol.rack.lockBack)
expect strokes(pistol.rack, true, [0.0, 0.006]) == [Back, Home] && strokes(pistol.rack, true, [0.0]) == []

/// Where the hand on slide `s` (at `hand`) is drawn on weapon `w` accepted
/// at `accepted` and wanted at `wanted`: turned as it is, carried with the
/// weapon where the level stopped it short, its fist on the slide's line
/// where the slide has it (the hand's place along it, within the travel).
let rackOn = (w: Weapon, s: Slide, accepted: Input.pose, wanted: Input.pose, hand: Input.pose): Input.pose =>
  let turned = (Hold.compose(accepted, Hold.relative(wanted, hand))).orientation in
  Hold.at(Quat.add(accepted.position, Quat.rotate(accepted.orientation, Quat.v3(w.rack.at.x, w.rack.at.y, s.from + s.pull))), turned)

// A pistol at `gp`; a hand turned on its slide's line `z` back of the zone,
// and the slide taken there and pulled by a hand at `z`.
let gp = Hold.at(Quat.v3(0.1, 1.2, -0.4), Quat.yaw(0.5))
let onLine = (z: float): Input.pose =>
  Hold.compose(gp, Hold.at(Quat.add(pistol.rack.at, Quat.v3(0.0, 0.0, z)), Quat.mul(Quat.yaw(0.3), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.2))))
let pulledTo = (z: float): Slide =>
  let (s, _) = pullTo(pistol.rack, take(pistol.rack, false, Hold.Left, pistol.rack.at.z), Option.Some(pistol.rack.at.z + z)) in s
// On the line, unblocked: where the hand is. Pulled past the travel: at its
// end (3 cm short of the hand). 3 cm off the line: on it.
expect Hold.near(rackOn(pistol, pulledTo(0.02), gp, gp, onLine(0.02)), onLine(0.02))
expect Hold.near(rackOn(pistol, pulledTo(0.07), gp, gp, onLine(0.07)), Hold.at((onLine(slideTravel)).position, (onLine(0.07)).orientation))
expect Hold.near(rackOn(pistol, pulledTo(0.02), gp, gp, Hold.compose(onLine(0.02), Hold.p(0.0, 0.03, 0.0))), onLine(0.02))
// The pistol stopped short of (and turned from) where the hand wants it:
// the hand drawn with it, on the slide.
expect (
  let wanted = Hold.at(Quat.add(gp.position, Quat.v3(-0.1, 0.02, 0.0)), Quat.mul(gp.orientation, Quat.yaw(0.2))) in
  Hold.near(rackOn(pistol, pulledTo(0.02), gp, wanted, Hold.compose(wanted, Hold.relative(gp, onLine(0.02)))), onLine(0.02))
)

/// How far a weapon's slide (or handle, or pump) is drawn back: where the
/// hand has it, or a pump where it was let go; else where it rests.
let slideBack = (w: Weapon, g: State): float =>
  if Option.isSome(g.slide.hand) || w.rack.pump then g.slide.pull else rest(w.rack, g.ammo.locked)

/// How far its bolt is drawn back: with the handle (or pump) while it is
/// racked, else locked back or home.
let boltBack = (w: Weapon, g: State): float =>
  if Option.isSome(g.slide.hand) || w.rack.pump then g.slide.pull else if g.ammo.locked then w.rack.boltLock else 0.0

// --- the shot ---

/// The golden angle: each shot's pellet pattern turns this much from the
/// last, so the pattern varies from shot to shot, the same on every run.
let patternTurn = Math.pi * (3.0 - Math.sqrt(5.0))

/// The ways a shot's pellets leave a weapon at `pose`, the `n`th shot it
/// has fired: along the bore, and the other `pellets - 1` evenly round it,
/// `spread` off it, the ring turned `patternTurn` a shot. One pellet: the
/// bore's way.
let pelletDirs = (w: Weapon, pose: Input.pose, n: float): List<Input.point3> =>
  let ring = w.pellets - 1.0 in
  List.range(w.pellets) |> List.map((i) =>
    let local =
      if i == 0.0 then Quat.v3(0.0, 0.0, 0.0 - 1.0)
      else
        let a = n * patternTurn + 2.0 * Math.pi * (i - 1.0) / ring in
        Quat.v3(Math.sin(w.spread) * Math.cos(a), Math.sin(w.spread) * Math.sin(a), 0.0 - Math.cos(w.spread)) in
    Quat.rotate(pose.orientation, local))

// The pistol: one pellet, along the bore.
expect (let p = heldAt(pistol, Hold.at(Quat.v3(0.1, 1.2, -0.3), Quat.yaw(0.4))) in pelletDirs(pistol, p, 3.0) == [(boreOf(pistol, p)).dir])

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

// --- a weapon's state ---

// The recoil kick: 1 as it fires, falling to 0 over `recoilSeconds`. Drawing
// only (`kicked`): the body, the muzzle check and the next shot all use the
// accepted pose.
let recoilSeconds = 0.12
let recoilPitch = 8.0 * Math.pi / 180.0
let recoilBack = 0.02
let hitCap = 12.0

/// A weapon: the hand on its trigger last step, the trigger, the fire mode,
/// the kick, its rounds, its slide, shots fired (each spent a round), pulls
/// refused (the muzzle blocked) and dry (nothing chambered) and when it last
/// was, the last shot, and the target hits, newest last (at most `hitCap`).
type State = {
  holder: Option.t<Hold.Side>,
  trigger: Trigger,
  mode: Mode,
  kick: float,
  ammo: Ammo.State,
  slide: Slide,
  fired: float,
  refused: float,
  dry: float,
  dryAt: float,
  last: Option.t<Shot>,
  hits: List<Impact>
}

let idle: State = {
  holder: Option.None, trigger: disarmed, mode: Semi, kick: 0.0, ammo: Ammo.loaded(pistol.well.kind), slide: home,
  fired: 0.0, refused: 0.0, dry: 0.0, dryAt: 0.0 - 1000.0, last: Option.None, hits: []
}

/// A weapon at rest, loaded and chambered.
let stocked = (w: Weapon): State => { idle with ammo: Ammo.loaded(w.well.kind) }

/// One step of a weapon whose trigger `holder` has (None: nobody), at
/// `value`: the new state, and whether the trigger fired. A new holder, or
/// an untracked one, starts disarmed, so a trigger held as the weapon is
/// taken (off the bench, out of a holster or rack, from the other hand) or as
/// tracking comes back fires nothing until it is let out.
let step = (g: State, holder: Option.t<Hold.Side>, tracked: bool, value: float, dt: float): (State, bool) =>
  let kick = Math.max(0.0, g.kick - dt / recoilSeconds) in
  if Option.isNone(holder) || not tracked then ({ g with holder: holder, trigger: disarmed, kick: kick }, false)
  else
    let (trigger, fires) = pull(g.mode, if holder == g.holder then g.trigger else disarmed, value) in
    ({ g with holder: holder, trigger: trigger, kick: kick }, fires)

// --- ammunition: `Ammo`'s, at these two ---

/// Whether a pulled trigger has a round to fire: one chambered, the slide
/// closed (not held back by a racking hand; a pump home).
let canFire = (w: Weapon, g: State): bool => g.ammo.chambered && inBattery(w.rack, g.slide)

/// After a shot: the round spent, the next fed from the magazine (a pump's
/// hull left in the chamber).
let onFired = (g: State): State => let (a, _) = Ammo.fire(g.ammo) in { g with ammo: a }

/// The weapon after firing `s`, its pellets striking `struck`: kicked,
/// counted, the target hits recorded (the last two shots' at least).
let fire = (w: Weapon, g: State, s: Shot, struck: List<Impact>): State =>
  let hits = g.hits |> List.append(struck |> List.filter((i) => i.target)) in
  let cap = Math.max(hitCap, 2.0 * w.pellets) in
  { onFired(g) with
      kick: 1.0,
      fired: g.fired + 1.0,
      last: Option.Some(s),
      hits: List.drop(Math.max(0.0, List.length(hits) - cap), hits) }

/// The weapon after a refused pull: a click, and the trigger disarmed until
/// it is let out, so full-auto doesn't click on at its rate.
let refuse = (g: State): State => { g with refused: g.refused + 1.0, trigger: { g.trigger with armed: false } }

/// The weapon after a dry pull at `t` (nothing chambered): a click, and the
/// trigger disarmed like `refuse`, so full-auto stops at the last round.
let dryPull = (g: State, t: float): State => { g with dry: g.dry + 1.0, dryAt: t, trigger: { g.trigger with armed: false } }

/// The selector switched: semi to full-auto and back, on a weapon that has one.
let select = (w: Weapon, g: State): State =>
  if not w.selector then g
  else { g with mode: match g.mode with | Semi => Auto | Auto => Semi }

expect (select(pistol, idle)).mode == Semi

// Loaded 12+1: each shot spends the chambered round and feeds the next; 13
// shots, then nothing to fire.
let shot0 = { t: 0.0, from: Hold.zero, dir: Hold.zero, impact: Option.None }
expect Ammo.count((stocked(pistol)).ammo) == 13.0 && canFire(pistol, stocked(pistol))
expect (
  let after = List.range(13.0) |> List.fold((g, _) => fire(pistol, g, shot0, []), stocked(pistol)) in
  after.fired == 13.0 && not canFire(pistol, after) && Ammo.count(after.ammo) == 0.0 && Option.isSome(after.ammo.mag)
)
// A dry pull is no shot: counted apart, the trigger disarmed.
expect (
  let g = dryPull({ stocked(pistol) with trigger: { pulled: true, armed: true, since: 0.0 } }, 2.0) in
  g.dry == 1.0 && g.fired == 0.0 && g.dryAt == 2.0 && not g.trigger.armed && Ammo.count(g.ammo) == 13.0
)

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

/// Where a weapon is drawn: `pose` kicked up about its grip and back by
/// `kick` (scaled by its `recoil`), easing out.
let kicked = (w: Weapon, kick: float, pose: Input.pose): Input.pose =>
  if kick <= 0.0 then pose
  else
    let k = w.recoil * kick * kick in
    let hand = Hold.compose(pose, w.grip) in
    let jolt = Hold.at(Quat.v3(0.0, 0.0, recoilBack * k), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), recoilPitch * k)) in
    Hold.compose(Hold.compose(hand, jolt), Hold.inverse(w.grip))

expect (let p = Hold.at(Quat.v3(0.1, 1.2, -0.3), Quat.yaw(0.4)) in Hold.near(kicked(pistol, 0.0, p), p))
// Fully kicked, the muzzle is up and back of where it aims; the bore it
// fires along is the accepted pose's.
expect (
  let p = heldAt(pistol, Hold.p(0.0, 1.2, -0.3)) in
  let up = boreOf(pistol, kicked(pistol, 1.0, p)) in
  up.dir.y > 0.13 && up.muzzle.z > (boreOf(pistol, p)).muzzle.z
)

// --- the look ---

let part = (sx: float, sy: float, sz: float, x: float, y: float, z: float) =>
  Scene.cube() |> Scene.scaleXYZ(sx, sy, sz) |> Scene.translate(Vec3.make(x - center.x, y - center.y, z - center.z))

/// The pistol in its own frame, from parts placed in the hand's (-Z the
/// barrel's way; turned a half turn at the end, as drawn its local +Z is):
/// the slide `back` metres back (its bore and sights with it), the barrel
/// that shows ahead of it then, a raked handle, the trigger and its guard.
/// `paint` colours the slide and handle.
let scene = (paint: (float, float, float, Scene.t) => Scene.t, back: float): Scene.t =>
  let dark = Color.rgb(0.08, 0.08, 0.09) in
  Scene.group([
    // The slide, from the muzzle to its back, round the bore; the muzzle's
    // bore, then a bright front post and the rear notch.
    Scene.group([
      part(0.03, 0.04, 0.205, 0.0, bore, 0.0 - 0.0475) |> paint(0.2, 0.21, 0.24),
      part(0.012, 0.012, 0.004, 0.0, bore, 0.0 - muzzleAhead - 0.001) |> Scene.lit(dark),
      part(0.004, 0.008, 0.006, 0.0, bore + 0.024, 0.0 - 0.14) |> Scene.emissive(Color.rgb(1.0, 0.55, 0.1)),
      part(0.006, 0.008, 0.006, 0.0 - 0.008, bore + 0.024, 0.048) |> Scene.lit(dark),
      part(0.006, 0.008, 0.006, 0.008, bore + 0.024, 0.048) |> Scene.lit(dark),
    ]) |> Scene.translate(Vec3.make(0.0, 0.0, back)),
    // The barrel, inside the slide until it goes back.
    part(0.012, 0.012, 0.05, 0.0, bore, 0.0 - muzzleAhead + 0.026) |> Scene.lit(Color.rgb(0.34, 0.34, 0.36)),
    // The handle, raked back 12 degrees below the slide.
    Scene.cube() |> Scene.scaleXYZ(0.03, 0.1, 0.045) |> Scene.rotateX(Angle.radians(handleRake))
      |> Scene.translate(Quat.toVec3(Quat.sub(handleAt, center))) |> paint(0.3, 0.22, 0.16),
    // The trigger guard and the trigger.
    part(0.008, 0.006, 0.045, 0.0, 0.0 - 0.002, 0.0 - 0.028) |> Scene.lit(dark),
    part(0.006, 0.022, 0.006, 0.0, 0.012, 0.0 - 0.016) |> Scene.lit(dark),
  ]) |> Scene.rotateY(180deg)
