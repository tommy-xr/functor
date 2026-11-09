// The armory station's contract, headless: Game.step with a stand-in world,
// a standing head, and the right hand moved by script.

let dt = 1.0 / 60.0
let start = Game.reset(Game.init, Stations.armory.id)
let pistolAt = Stations.spawn(Props.Pistol)
// A level hand on the resting pistol's grip.
let onGrip = Hold.compose(pistolAt, Gun.grip)
let moved = (p: Input.pose, x: float, y: float, z: float): Input.pose => Hold.at(Quat.add(p.position, Quat.v3(x, y, z)), p.orientation)

/// The right hand at `pose`, squeezing `squeeze`, its trigger at `trigger`.
let hand = (pose: Input.pose, squeeze: float, trigger: float): Input.controller =>
  { Game.hand(pose.position, squeeze) with grip: Option.Some(pose), trigger: trigger }

/// Every model a script of right hands passes through in `world`, the start first.
let playIn = (world: Game.World, m0: Game.Model, script: List<Input.controller>): List<Game.Model> =>
  script
    |> List.fold((acc, c) =>
         match acc with
         | [m, ..rest] =>
           let xr = { head: Option.Some(Hips.headAt(0.0, 0.0)), left: Game.away, right: c } in
           let (next, _) = Game.step({ m with xr: Option.Some(xr) }, Stations.armory, world, (p) => p, List.length(acc) * dt, dt) in
           [next, m, ..rest]
         | [] => [],
       [m0])
    |> List.reverse
let play = (script: List<Input.controller>) => playIn(Game.noWorld, start, script)

let last = (ms: List<Game.Model>): Game.Model => List.last(ms) |> Option.defaultValue(start)
let times = (n: float, c: Input.controller): List<Input.controller> => List.range(n) |> List.map((i) => c)
let pulsesOf = (ms: List<Game.Model>) => ms |> List.concatMap((m) => m.pulses)
let count = (e: Pulses.Event, ms: List<Game.Model>): float => pulsesOf(ms) |> List.filter((se) => se == (Hold.Right, e)) |> List.length
let pistol = (m: Game.Model) => Game.propOf(m, Props.Pistol)

// Taken open-triggered, held a moment: settled in the hand.
let take = List.flatten([[hand(onGrip, 0.0, 0.0), hand(onGrip, 1.0, 0.0)], times(5.0, hand(onGrip, 1.0, 0.0))])
let pull = hand(onGrip, 1.0, 1.0)
let ease = hand(onGrip, 1.0, 0.0)

expect Game.isHeld(last(play(take)), Props.Pistol)
// Only the pistol is at the armory, and the holsters are worn.
expect List.length(start.props) == 1.0 && Stations.worn(Stations.armory)

// --- the trigger ---

// Held down for 30 steps: one shot, one fire pulse.
expect (let ms = play(List.flatten([take, times(30.0, pull)])) in last(ms).gun.fired == 1.0 && count(Pulses.Fire, ms) == 1.0)
// Let out to `rearmAt` and pulled again: a second.
expect (last(play(List.flatten([take, times(3.0, pull), [hand(onGrip, 1.0, 0.3)], times(3.0, pull)]))).gun.fired == 2.0)
// Wavering between the thresholds: still one.
expect (last(play(List.flatten([take, [pull, hand(onGrip, 1.0, 0.5), pull, hand(onGrip, 1.0, 0.35), pull]]))).gun.fired == 1.0)
// Taken with the trigger already held: no shot, however long; let out and
// pulled, one.
let heldOnGrab = List.flatten([[hand(onGrip, 0.0, 1.0), hand(onGrip, 1.0, 1.0)], times(20.0, pull)])
expect last(play(heldOnGrab)).gun.fired == 0.0
expect last(play(List.flatten([heldOnGrab, [hand(onGrip, 1.0, 0.1), pull]]))).gun.fired == 1.0
// Tracking lost disarms: back with it held, nothing; let out and pulled, one.
let lostAndBack = List.flatten([take, times(3.0, Game.away), times(5.0, pull)])
expect last(play(lostAndBack)).gun.fired == 0.0 && Game.isHeld(last(play(lostAndBack)), Props.Pistol)
expect last(play(List.flatten([lostAndBack, [hand(onGrip, 1.0, 0.2), pull]]))).gun.fired == 1.0

// --- the shot ---

// A stand-in world: a wall the pistol's centre stops at, x >= -0.3 (the hand
// goes on through it), and a ray that strikes 5 m along whatever it is given.
let wallAt = (k: Props.Kind, from: Input.pose, to: Input.pose): Option.t<Hold.Hit> =>
  let face = 0.0 - 0.3 in
  let d = to.position.x - from.position.x in
  if d >= 0.0 || to.position.x > face then Option.None
  else Option.Some({ fraction: Math.max(0.0, (face - from.position.x) / d), normal: Quat.v3(1.0, 0.0, 0.0) })
let struck = (tag: string, target: bool) =>
  (from: Input.point3, dir: Input.point3) => Option.Some({ point: Quat.add(from, Quat.scale(5.0, dir)), tag: tag, target: target })
let walled = { Game.noWorld with sweep: wallAt, ray: struck("backstop", false) }

// Pushed through the wall with the hand, and fired: the shot leaves from the
// muzzle of the pose the wall stopped it at, not the tracked hand's.
let through = List.range(10.0) |> List.map((i) => hand(moved(onGrip, 0.0 - 0.05 * (i + 1.0), 0.0, 0.0), 1.0, 0.0))
let shotThrough = last(playIn(walled, start, List.flatten([take, through, [hand(moved(onGrip, -0.5, 0.0, 0.0), 1.0, 1.0)]])))
expect (
  shotThrough.gun.last |> Option.bind((s) => pistol(shotThrough).held |> Option.map((h) =>
    let accepted = Gun.boreOf(h.accepted) in
    let tracked = Gun.boreOf(Gun.heldAt(moved(onGrip, -0.5, 0.0, 0.0))) in
    Hold.near3(s.from, accepted.muzzle) && Hold.near3(s.dir, accepted.dir)
      && Quat.length(Quat.sub(s.from, tracked.muzzle)) > 0.04 && h.accepted.position.x > -0.31))
    |> Option.defaultValue(false)
)
// Turned and rolled, settled in the hand: the shot leaves 5 cm above the
// hand's -Z axis and 15 cm along it.
expect (
  let q = Quat.mul(Quat.yaw(0.5), Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.2), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.7))) in
  let aimed = Hold.at(Quat.v3(0.1, 1.3, -0.4), q) in
  let m = last(playIn(walled, start, List.flatten([take, times(25.0, hand(aimed, 1.0, 0.0)), [hand(aimed, 1.0, 1.0)]]))) in
  match m.gun.last with
  | Option.Some(s) =>
    Hold.near3(s.from, Quat.add(aimed.position, Quat.rotate(q, Quat.v3(0.0, 0.05, -0.15))))
      && Hold.near3(s.dir, Quat.rotate(q, Quat.v3(0.0, 0.0, -1.0)))
  | Option.None => false
)
// A target struck scores and is recorded where it was hit; the backstop
// struck is a shot, not a hit.
expect (
  let m = last(playIn({ Game.noWorld with ray: struck("target-1", true) }, start, List.flatten([take, [pull]]))) in
  m.score == 1.0 && List.length(m.gun.hits) == 1.0
    && (m.gun.hits |> List.all((h) => h.tag == "target-1" && Hold.near3(h.point, Quat.add((Gun.boreOf(Gun.heldAt(onGrip))).muzzle, Quat.v3(0.0, 0.0, -5.0)))))
)
expect (let m = last(playIn(walled, start, List.flatten([take, [pull]]))) in m.score == 0.0 && m.gun.hits == [] && m.gun.fired == 1.0)
// The muzzle against the level: refused, a click and no shot. Let out and
// pulled clear of it, it fires.
expect (
  let ms = playIn({ walled with blocked: (a: Input.point3, b: Input.point3) => true }, start, List.flatten([take, [pull]])) in
  let m = last(ms) in
  m.gun.fired == 0.0 && m.gun.refused == 1.0 && Option.isNone(m.gun.last) && count(Pulses.Click, ms) == 1.0 && count(Pulses.Fire, ms) == 0.0
    && (last(playIn(walled, m, [ease, pull]))).gun.fired == 1.0
)
// The kick, from 1 as it fires, is gone 10 steps later.
expect (
  let ms = play(List.flatten([take, [pull], times(10.0, pull)])) in
  (List.nth(List.length(take) + 1.0, ms) |> Option.map((m) => m.gun.kick == 1.0) |> Option.defaultValue(false)) && last(ms).gun.kick == 0.0
)

// --- the grip ---

/// No pop: from step to step the pistol moves at most a settling step
/// (`Hold.snapStep`, `Hold.snapTurnStep`), the hands held still.
let smooth = (ms: List<Game.Model>): bool =>
  let poses = ms |> List.map((m) => Game.poseOf(pistol(m))) in
  List.zip(List.drop(1.0, poses), poses) |> List.all((ab) =>
    let (a, b) = ab in
    Quat.length(Quat.sub(a.position, b.position)) <= Hold.snapStep + 0.000001
      && Quat.angle(a.orientation, b.orientation) <= Hold.snapTurnStep + 0.000001)

// Taken anywhere in reach of the grip, turned: it never jumps, and settles
// into the grip.
expect (
  let turned = Hold.at(Quat.add(onGrip.position, Quat.v3(0.02, 0.01, 0.03)), Quat.yaw(0.8)) in
  let ms = play(List.flatten([[hand(turned, 0.0, 0.0), hand(turned, 1.0, 0.0)], times(12.0, hand(turned, 1.0, 0.0))])) in
  smooth(ms)
    && (pistol(last(ms)).held |> Option.map((h) => Option.isNone(h.snapTo) && Hold.near(h.accepted, Gun.heldAt(turned))) |> Option.defaultValue(false))
)
// Not by the barrel: a hand at the muzzle takes nothing.
expect (
  let muzzle = Hold.at((Gun.boreOf(pistolAt)).muzzle, Quat.identity) in
  not Game.isHeld(last(play([hand(muzzle, 0.0, 0.0), hand(muzzle, 1.0, 0.0)])), Props.Pistol)
)

// --- the holster ---

// Into the right holster, then drawn with the trigger held: it comes out
// of where it sat without a jump, and doesn't fire until the trigger is let
// out.
let holster = Hold.at(Quat.add(Hips.rootOf(Hips.headAt(0.0, 0.0)), Slots.rightHolster.offset.position), Quat.identity)
let stowed = play(List.flatten([take, times(10.0, hand(holster, 1.0, 0.0)), [hand(holster, 0.0, 0.0)]]))
expect pistol(last(stowed)).slot == Option.Some(Slots.rightHolster.id)
expect (
  let drawn = playIn(Game.noWorld, last(stowed), List.flatten([[hand(holster, 0.0, 1.0), hand(holster, 1.0, 1.0)], times(20.0, hand(holster, 1.0, 1.0))])) in
  smooth(drawn) && Game.isHeld(last(drawn), Props.Pistol) && Option.isNone(pistol(last(drawn)).slot)
    && last(drawn).gun.fired == 0.0
    && (last(playIn(Game.noWorld, last(drawn), [hand(holster, 1.0, 0.0), hand(holster, 1.0, 1.0)]))).gun.fired == 1.0
)

