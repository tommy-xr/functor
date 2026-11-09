// Pump: the pump-action shotgun, pure and rig-local (`Hold`'s frame). Its
// shape, its grips, its loading port and its pump. Held like the rifle
// (`Longarm`): one hand rigid at the pistol grip (the trigger), a second on
// the fore-end aiming it. That second hand also works the pump: the fore-end
// slides back along the bore with it (`Gun.Rack`, `pump`), so its support
// line is the fore-end wherever the pump has it. Its tube is loaded a shell
// at a time through the port under the receiver (`Seating`). A shot is a
// spread of pellets (`Gun.pelletDirs`).

// --- the shotgun ---
// Laid out in the primary hand's frame when held (-Z forward, +Y up), like
// the rifle: the bore 6 cm above the fist, the stock's back 30 cm behind it,
// the muzzle 62 cm ahead. The tube runs under the barrel; the fore-end rides
// it, 28-42 cm ahead of the fist at home, 9 cm back pumped.

let bore = 0.06
let chamberAhead = 0.04
let muzzleAhead = 0.62
let stockBack = 0.3
let foreNear = 0.28
let foreFar = 0.42
let pumpTravel = 0.09

// The collider's bounds there: 5 cm wide, from the fist's top (the pistol
// grip and the trigger guard hang below it, so a shell at the loading port's
// mouth is clear of it) to the rib 9.5 cm above, from the stock's back to the
// muzzle.
let size = Quat.v3(0.05, 0.095, stockBack + muzzleAhead)
let center = Quat.v3(0.0, 0.0475, (stockBack - muzzleAhead) / 2.0)

/// The primary hand's pose in the shotgun's frame (centred on its
/// collider): the authored pistol grip. Like the rifle's, only taken there.
let grip: Input.pose = Hold.at(Quat.scale(0.0 - 1.0, center), Quat.identity)
let gripZone = 0.07

/// The support line: the fore-end, wherever the pump has it.
let fore: Longarm.Fore = { grip: grip, near: foreNear, far: foreFar, travel: pumpTravel }

/// How a shell is guided into the loading port: a 3 cm catch, 5 cm to pull
/// out of; lying along the bore, its head back, within ~32° (dot 0.85) to
/// latch and 46° to lose it; any turn about its own axis.
let portGuide: Gun.Guide = { entry: 0.03, exit: 0.05, alignDot: -1.0, unalignDot: -1.0, keyDot: 0.85, unkeyDot: 0.7 }

// The loading port: a seated shell lies in the receiver's floor, 12 cm ahead
// of the fist; one comes in from 6 cm below, under the receiver (clear of the
// collider and ahead of the trigger guard).
let portAt = Quat.v3(0.0, 0.045, 0.0 - 0.12)

// The readout on the receiver's top at its back, tipped toward the eye; the
// port a pumped-out shell leaves by on the right.
let weapon: Gun.Weapon =
  { Gun.weaponAt(grip, bore, chamberAhead, muzzleAhead, false,
      { kind: Ammo.Shell, at: Quat.sub(portAt, center), rake: 0.0, drop: 0.06, guide: portGuide },
      { at: Quat.sub(Quat.v3(0.0, 0.0905, 0.01), center), tilt: 0.0 - Math.pi / 6.0, size: 0.005 },
      { at: Quat.sub(Quat.v3(0.0, 0.0, 0.0 - (foreNear + foreFar) / 2.0), center), radius: 0.0, travel: pumpTravel, lockBack: 0.0, boltLock: 0.0,
        port: Quat.sub(Quat.v3(0.04, 0.06, 0.0 - 0.08), center), pump: true })
    with pellets: 8.0, spread: 1.2 * Math.pi / 180.0, recoil: 1.8 }

expect Gun.clearBelow(weapon, size)

// The support line is the fore-end where the pump has it: 30 cm ahead with
// the pump home, 21 cm pumped back; a hand stays on it anywhere it slides.
let level = Hold.at(Quat.v3(0.2, 1.1, -0.3), Quat.identity)
let held = Gun.heldAt(weapon, level)
let ahead = (d: float) => Quat.v3(0.2, 1.1, -0.3 - d)
expect Longarm.foreGap(fore, 0.0, held, ahead(0.3)) < 0.0001 && Math.abs(Longarm.foreGap(fore, 0.0, held, ahead(0.21)) - 0.07) < 0.0001
expect Longarm.foreGap(fore, pumpTravel, held, ahead(0.21)) < 0.0001
expect ([foreNear - pumpTravel - 0.07, 0.3, foreFar + 0.07] |> List.all((d) => Longarm.holds(fore, Longarm.alongOf(level, ahead(d)))))
  && not Longarm.holds(fore, Longarm.alongOf(level, ahead(foreNear - pumpTravel - 0.09)))

// Pellets: eight, one along the bore and seven on a 1.2° cone round it,
// evenly; the ring turns from shot to shot, the same every run.
expect (
  let dirs = Gun.pelletDirs(weapon, held, 0.0) in
  let b = Gun.boreOf(weapon, held) in
  let off = (d: Input.point3) => Math.acos(Math.clamp(-1.0, 1.0, Quat.dot(d, b.dir))) in
  List.length(dirs) == 8.0 && off(List.head(dirs) |> Option.defaultValue(Hold.zero)) < 0.0001
    && (List.drop(1.0, dirs) |> List.all((d) => Math.abs(off(d) - weapon.spread) < 0.00001))
    && Gun.pelletDirs(weapon, held, 0.0) == dirs && Gun.pelletDirs(weapon, held, 1.0) != dirs
)

// --- the look ---

let part = (sx: float, sy: float, sz: float, x: float, y: float, z: float) =>
  Scene.cube() |> Scene.scaleXYZ(sx, sy, sz) |> Scene.translate(Vec3.make(x - center.x, y - center.y, z - center.z))

/// The shotgun in its own frame, from parts placed in the primary hand's
/// (-Z the barrel's way; turned a half turn at the end, as drawn its local
/// +Z is): stock, receiver, pistol grip, trigger, the loading port under the
/// receiver, barrel over the tube and its bead; the fore-end (lighter: where
/// the support hand goes) and the bolt in the ejection port, `pump` back.
/// `paint` colours the furniture.
let scene = (paint: (float, float, float, Scene.t) => Scene.t, pump: float): Scene.t =>
  let dark = Color.rgb(0.08, 0.08, 0.09) in
  let metal = Color.rgb(0.14, 0.14, 0.16) in
  let mid = (foreNear + foreFar) / 2.0 in
  Scene.group([
    part(0.04, 0.075, 0.25, 0.0, 0.045, 0.175) |> paint(0.42, 0.26, 0.14),
    part(0.045, 0.08, 0.22, 0.0, 0.05, 0.0 - 0.06) |> Scene.lit(metal),
    Scene.cube() |> Scene.scaleXYZ(0.03, 0.09, 0.04) |> Scene.rotateX(Angle.radians(0.0 - 0.25))
      |> Scene.translate(Vec3.make(0.0 - center.x, 0.0 - center.y, 0.012 - center.z)) |> paint(0.42, 0.26, 0.14),
    part(0.008, 0.006, 0.05, 0.0, 0.0 - 0.002, 0.0 - 0.045) |> Scene.lit(dark),
    part(0.006, 0.02, 0.006, 0.0, 0.008, 0.0 - 0.035) |> Scene.lit(dark),
    // The loading port: a slot in the receiver's floor.
    part(0.024, 0.004, 0.075, 0.0, 0.009, portAt.z) |> Scene.lit(dark),
    part(0.018, 0.018, muzzleAhead - 0.17, 0.0, bore, 0.0 - (0.17 + muzzleAhead) / 2.0) |> Scene.lit(metal),
    part(0.021, 0.021, 0.4, 0.0, 0.03, 0.0 - 0.37) |> Scene.lit(dark),
    part(0.005, 0.006, 0.005, 0.0, bore + 0.012, 0.0 - muzzleAhead + 0.01) |> Scene.emissive(Color.rgb(1.0, 0.55, 0.1)),
    // The fore-end, ribbed, and the bolt in the port on the right.
    Scene.group([
      part(0.046, 0.04, foreFar - foreNear, 0.0, 0.025, 0.0 - mid) |> paint(0.58, 0.4, 0.24),
      part(0.048, 0.042, 0.006, 0.0, 0.025, 0.0 - mid + 0.04) |> Scene.lit(dark),
      part(0.048, 0.042, 0.006, 0.0, 0.025, 0.0 - mid - 0.04) |> Scene.lit(dark),
    ]) |> Scene.translate(Vec3.make(0.0, 0.0, pump)),
    part(0.004, 0.022, 0.055, 0.0235, bore, 0.0 - 0.08 + pump) |> Scene.lit(Color.rgb(0.62, 0.62, 0.66)),
  ]) |> Scene.rotateY(180deg)
