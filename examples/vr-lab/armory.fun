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
  { Game.turnedHand(pose.position, pose.orientation, squeeze) with trigger: trigger }

/// Every model a script of (left, right) hands passes through in `world`,
/// the start first.
let playBoth = (world: Game.World, m0: Game.Model, script: List<(Input.controller, Input.controller)>): List<Game.Model> =>
  script
    |> List.fold((acc, lr) =>
         match acc with
         | [m, ..rest] =>
           let (l, r) = lr in
           let xr = { head: Option.Some(Hips.headAt(0.0, 0.0)), left: l, right: r } in
           let (next, _) = Game.step({ m with xr: Option.Some(xr) }, Stations.armory, world, (p) => p, List.length(acc) * dt, dt) in
           [next, m, ..rest]
         | [] => [],
       [m0])
    |> List.reverse
/// The same, the left hand away.
let playIn = (world: Game.World, m0: Game.Model, script: List<Input.controller>): List<Game.Model> =>
  playBoth(world, m0, script |> List.map((c) => (Game.away, c)))
let play = (script: List<Input.controller>) => playIn(Game.noWorld, start, script)

let last = (ms: List<Game.Model>): Game.Model => List.last(ms) |> Option.defaultValue(start)
let times = (n: float, c: Input.controller): List<Input.controller> => List.range(n) |> List.map((i) => c)
let pulsesOf = (ms: List<Game.Model>) => ms |> List.concatMap((m) => m.pulses)
let count = (e: Pulses.Event, ms: List<Game.Model>): float => countOf(Hold.Right, e, ms)
let countOf = (side: Hold.Side, e: Pulses.Event, ms: List<Game.Model>): float =>
  pulsesOf(ms) |> List.filter((se) => se == (side, e)) |> List.length
let pistol = (m: Game.Model) => Game.propOf(m, Props.Pistol)

// Taken open-triggered, held a moment: settled in the hand.
let take = List.flatten([[hand(onGrip, 0.0, 0.0), hand(onGrip, 1.0, 0.0)], times(5.0, hand(onGrip, 1.0, 0.0))])
let pull = hand(onGrip, 1.0, 1.0)
let ease = hand(onGrip, 1.0, 0.0)

expect Game.isHeld(last(play(take)), Props.Pistol)
// The pistol, the rifle and four spare magazines are at the armory, and the
// holsters are worn.
expect List.length(start.props) == 6.0 && Stations.worn(Stations.armory)

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
  (k: Props.Kind, from: Input.point3, dir: Input.point3) => Option.Some({ point: Quat.add(from, Quat.scale(5.0, dir)), tag: tag, target: target })
let walled = { Game.noWorld with sweep: wallAt, ray: struck("backstop", false) }

// Pushed through the wall with the hand, and fired: the shot leaves from the
// muzzle of the pose the wall stopped it at, not the tracked hand's.
let through = List.range(10.0) |> List.map((i) => hand(moved(onGrip, 0.0 - 0.05 * (i + 1.0), 0.0, 0.0), 1.0, 0.0))
let shotThrough = last(playIn(walled, start, List.flatten([take, through, [hand(moved(onGrip, -0.5, 0.0, 0.0), 1.0, 1.0)]])))
expect (
  shotThrough.gun.last |> Option.bind((s) => pistol(shotThrough).held |> Option.map((h) =>
    let accepted = Gun.boreOf(Gun.pistol, h.accepted) in
    let tracked = Gun.boreOf(Gun.pistol, Gun.heldAt(Gun.pistol, moved(onGrip, -0.5, 0.0, 0.0))) in
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
    && (m.gun.hits |> List.all((h) => h.tag == "target-1" && Hold.near3(h.point, Quat.add((Gun.boreOf(Gun.pistol, Gun.heldAt(Gun.pistol, onGrip))).muzzle, Quat.v3(0.0, 0.0, -5.0)))))
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
    && (pistol(last(ms)).held |> Option.map((h) => Option.isNone(h.snapTo) && Hold.near(h.accepted, Gun.heldAt(Gun.pistol, turned))) |> Option.defaultValue(false))
)
// Not by the barrel: a hand at the muzzle takes nothing.
expect (
  let muzzle = Hold.at((Gun.boreOf(Gun.pistol, pistolAt)).muzzle, Quat.identity) in
  not Game.isHeld(last(play([hand(muzzle, 0.0, 0.0), hand(muzzle, 1.0, 0.0)])), Props.Pistol)
)

// --- the holster ---

// Into the right holster, then drawn with the trigger held: it comes out
// of where it sat without a jump, and doesn't fire until the trigger is let
// out.
let holster = Hold.at(Inventory.rightHolster, Quat.identity)
let stowed = play(List.flatten([take, times(10.0, hand(holster, 1.0, 0.0)), [hand(holster, 0.0, 0.0)]]))
expect pistol(last(stowed)).slot == Option.Some(Slots.rightHolster.id)
expect (
  let drawn = playIn(Game.noWorld, last(stowed), List.flatten([[hand(holster, 0.0, 1.0), hand(holster, 1.0, 1.0)], times(20.0, hand(holster, 1.0, 1.0))])) in
  smooth(drawn) && Game.isHeld(last(drawn), Props.Pistol) && Option.isNone(pistol(last(drawn)).slot)
    && last(drawn).gun.fired == 0.0
    && (last(playIn(Game.noWorld, last(drawn), [hand(holster, 1.0, 0.0), hand(holster, 1.0, 1.0)]))).gun.fired == 1.0
)


// --- the rifle ---

let rifleAt = Stations.spawn(Props.Rifle)
let rifle = (m: Game.Model) => Game.propOf(m, Props.Rifle)
let rifleAccepted = (m: Game.Model): Input.pose => Game.poseOf(rifle(m))
// The right fist on the racked rifle's pistol grip, and a point `d` ahead of
// a fist along its foregrip line, off it by (x, y).
let onStock = Hold.compose(rifleAt, Longarm.grip)
let foreOf = (fist: Input.pose, d: float, x: float, y: float): Input.pose => Hold.compose(fist, Hold.at(Quat.v3(x, y, 0.0 - d), Quat.identity))
let both = (n: float, l: Input.controller, r: Input.controller) => List.range(n) |> List.map((_) => (l, r))
let grabbed = (p: Input.pose) => hand(p, 1.0, 0.0)
let still = grabbed(onStock)
let fore25 = grabbed(foreOf(onStock, 0.25, 0.0, 0.0))
let play2 = (m0: Game.Model, script: List<(Input.controller, Input.controller)>) => playBoth(Game.noWorld, m0, script)
let supportSide = (m: Game.Model) => Game.supportOf(rifle(m))

// Out of the rack by the pistol grip (an unstow), then the left hand on the foregrip 25 cm ahead.
let twoHanded = play2(start, List.flatten([
  [(Game.away, hand(onStock, 0.0, 0.0))], both(4.0, Game.away, still),
  [(hand(foreOf(onStock, 0.25, 0.0, 0.0), 0.0, 0.0), still)], both(3.0, fore25, still),
]))
let ready = last(twoHanded)

expect (
  Game.isHeld(ready, Props.Rifle) && supportSide(ready) == Option.Some(Hold.Left) && not rifle(ready).fore
    && Option.isNone(rifle(ready).slot) && Hold.near(rifleAccepted(ready), rifleAt)
    && countOf(Hold.Right, Pulses.Unstow, twoHanded) == 1.0 && countOf(Hold.Left, Pulses.Grab, twoHanded) == 1.0
)
// The left hand alone can't take it from the rack by the foregrip, nor a hand
// at its muzzle.
expect (
  let at = (p: Input.pose) => last(play2(start, [(hand(p, 0.0, 0.0), Game.away), (grabbed(p), Game.away)])) in
  not Game.isHeld(at(foreOf(onStock, 0.25, 0.0, 0.0)), Props.Rifle) && not Game.isHeld(at(foreOf(onStock, 0.55, 0.0, 0.0)), Props.Rifle)
)

// Aimed by the support hand alone, the right hand still: the barrel along
// support - primary, level (the right hand's roll), the right fist on the
// grip, and a shot leaves the accepted rifle's muzzle that way.
let aimedBy = (dx: float, dy: float) =>
  let aim = grabbed(foreOf(onStock, 0.25, dx, dy)) in
  play2(ready, List.flatten([both(8.0, aim, still), [(aim, hand(onStock, 1.0, 1.0))]]))
expect [(0.05, 0.0), (0.0 - 0.04, 0.03), (0.02, 0.0 - 0.05)] |> List.all((d) =>
  let (dx, dy) = d in
  let m = last(aimedBy(dx, dy)) in
  let q = (rifleAccepted(m)).orientation in
  let b = Gun.boreOf(Longarm.weapon, rifleAccepted(m)) in
  let want = Longarm.unit(Quat.sub((foreOf(onStock, 0.25, dx, dy)).position, onStock.position)) in
  Hold.near3(b.dir, want) && Math.abs((Quat.rotate(q, Quat.v3(1.0, 0.0, 0.0))).y) < 0.0001 && (Quat.rotate(q, Quat.v3(0.0, 1.0, 0.0))).y > 0.0
    && Hold.near3((Hold.compose(rifleAccepted(m), Longarm.grip)).position, onStock.position)
    && (m.rifle.last |> Option.map((s) => Hold.near3(s.from, b.muzzle) && Hold.near3(s.dir, want)) |> Option.defaultValue(false))
    && m.rifle.fired == 1.0)
// Rolled 30 degrees, the rifle has the right hand's roll whichever way the
// support aims it.
expect (
  let rolled = Hold.at(onStock.position, Quat.axisAngle(Quat.v3(0.0, 0.0, -1.0), Math.pi / 6.0)) in
  let m = last(play2(ready, both(10.0, grabbed(foreOf(rolled, 0.25, 0.05, 0.0)), grabbed(rolled)))) in
  Quat.angle((rifleAccepted(m)).orientation, Longarm.aimAt(rolled, (foreOf(rolled, 0.25, 0.05, 0.0)).position)) < 0.0001
)
// The support hand slides along the foregrip: the aim doesn't change.
expect (
  let ms = play2(ready, List.range(20.0) |> List.map((i) => (grabbed(foreOf(onStock, 0.16 + 0.011 * i, 0.0, 0.0)), still))) in
  supportSide(last(ms)) == Option.Some(Hold.Left) && (ms |> List.all((m) => Quat.angle((rifleAccepted(m)).orientation, onStock.orientation) < 0.0001))
)

// Dragged 15 cm off the axis, 1 cm a step: past 12 cm it breaks off, a break
// pulse in the left hand (disarmed until it opens). The rifle doesn't move on
// that step, then settles back into the right hand.
let dragged = play2(ready, List.flatten([
  List.range(15.0) |> List.map((i) => (grabbed(foreOf(onStock, 0.25, 0.01 * (i + 1.0), 0.0)), still)),
  both(10.0, grabbed(foreOf(onStock, 0.25, 0.15, 0.0)), still)]))
let brokeAt = dragged |> List.indexedMap((i, m) => (i, m)) |> List.find((im) => let (_, m) = im in Option.isNone(supportSide(m)))
  |> Option.map((im) => let (i, _) = im in i) |> Option.defaultValue(0.0)
expect (
  let at = (i: float) => rifleAccepted(List.nth(i, dragged) |> Option.defaultValue(start)) in
  brokeAt == 13.0 && countOf(Hold.Left, Pulses.Break, dragged) == 1.0 && Game.isHeld(last(dragged), Props.Rifle)
    && Hold.near(at(brokeAt), at(brokeAt - 1.0))
    && Quat.angle((at(brokeAt - 1.0)).orientation, onStock.orientation) > 0.18
    && Hold.near(rifleAccepted(last(dragged)), rifleAt)
    && not (last(dragged)).left.armed
)
// Past the foregrip's far end, it breaks off too.
expect (
  let ms = play2(ready, List.range(20.0) |> List.map((i) => (grabbed(foreOf(onStock, 0.25 + 0.015 * (i + 1.0), 0.0, 0.0)), still))) in
  Option.isNone(supportSide(last(ms))) && countOf(Hold.Left, Pulses.Break, ms) == 1.0
)
// Opened, the support lets go: a release pulse, the rifle where the aim had
// it on that step, then back in the right hand.
expect (
  let aimed = last(aimedBy(0.05, 0.0)) in
  let ms = play2(aimed, List.flatten([[(hand(foreOf(onStock, 0.25, 0.05, 0.0), 0.0, 0.0), still)], both(8.0, Game.away, still)])) in
  let first = List.nth(1.0, ms) |> Option.defaultValue(start) in
  Option.isNone(supportSide(first)) && countOf(Hold.Left, Pulses.Release, ms) == 1.0
    && Hold.near(rifleAccepted(first), rifleAccepted(aimed)) && Hold.near(rifleAccepted(last(ms)), rifleAt)
)

// The right hand opening while the left supports: the left holds it by the
// foregrip where it is, its trigger dead. The right squeezing its grip again
// takes it back, the left still on the foregrip, and the trigger is the
// right's again (once let out).
let handedOver = play2(ready, List.flatten([
  [(fore25, hand(onStock, 0.0, 0.0))],
  both(3.0, { fore25 with trigger: 1.0 }, hand(onStock, 0.0, 1.0)),
  [(fore25, hand(onStock, 0.0, 0.0))],
]))
let takenBack = play2(last(handedOver), List.flatten([both(4.0, fore25, still), [(fore25, hand(onStock, 1.0, 1.0))]]))
expect (
  let m = last(handedOver) in
  (rifle(m).held |> Option.map((h) => h.side == Hold.Left) |> Option.defaultValue(false)) && rifle(m).fore && Option.isNone(supportSide(m))
    && (handedOver |> List.all((n) => Hold.near(rifleAccepted(n), rifleAt))) && m.rifle.fired == 0.0
    // Silent, but for the rack ready to take it from the hand that now holds it.
    && Option.isNone(rifle(m).slot) && pulsesOf(List.drop(1.0, handedOver)) == [(Hold.Left, Pulses.Ready)]
)
expect (
  let m = last(takenBack) in
  (rifle(m).held |> Option.map((h) => h.side == Hold.Right) |> Option.defaultValue(false)) && not rifle(m).fore && supportSide(m) == Option.Some(Hold.Left)
    && (takenBack |> List.all((n) => Hold.near(rifleAccepted(n), rifleAt))) && m.rifle.fired == 1.0
    && countOf(Hold.Right, Pulses.Grab, takenBack) == 1.0 && countOf(Hold.Left, Pulses.Grab, takenBack) == 0.0
)
// Held by the foregrip, the left hand moves it, rigidly.
expect (
  let m = last(play2(last(handedOver), both(3.0, grabbed(foreOf(onStock, 0.25, 0.0, 0.1)), Game.away))) in
  Hold.near3((rifleAccepted(m)).position, Quat.add(rifleAt.position, Quat.v3(0.0, 0.1, 0.0)))
)

// --- firing the rifle ---

let auto = (m: Game.Model): Game.Model => { m with rifle: Gun.select(Longarm.weapon, m.rifle) }
/// The right trigger held `n` steps, two hands on the rifle.
let holding = (m0: Game.Model, n: float) => play2(m0, both(n, fore25, hand(onStock, 1.0, 1.0)))
// Semi: one shot however long the trigger is held. Full-auto: the pull and
// one every `Gun.stepsPerShot` steps after it, so 30 steps held fire 5.
expect (last(holding(ready, 30.0))).rifle.fired == 1.0
expect [1.0, 6.0, 7.0, 30.0, 31.0] |> List.all((n) => (last(holding(auto(ready), n))).rifle.fired == Math.floor((n - 1.0) / Gun.stepsPerShot) + 1.0)
// Each shot buzzes both hands on it.
expect (let ms = holding(auto(ready), 13.0) in countOf(Hold.Right, Pulses.Fire, ms) == 3.0 && countOf(Hold.Left, Pulses.Fire, ms) == 3.0)
// The muzzle against the level: refused, one click in the trigger hand (not
// one at full-auto's rate), no shot; let out and pulled clear, it fires.
expect (
  let ms = playBoth({ Game.noWorld with blocked: (a: Input.point3, b: Input.point3) => true }, auto(ready), both(20.0, fore25, hand(onStock, 1.0, 1.0))) in
  let m = last(ms) in
  m.rifle.fired == 0.0 && m.rifle.refused == 1.0 && countOf(Hold.Right, Pulses.Click, ms) == 1.0 && countOf(Hold.Left, Pulses.Click, ms) == 0.0
    && (last(play2(m, [(fore25, still), (fore25, hand(onStock, 1.0, 1.0))]))).rifle.fired == 1.0
)
// Pushed through a stand-in wall with both hands, the shot leaves the muzzle
// of the pose the wall stopped the rifle at, the support still on it.
expect (
  let pushed = (dx: float) => Hold.at(Quat.add(onStock.position, Quat.v3(dx, 0.0, 0.0)), onStock.orientation) in
  let rifleWall = { walled with sweep: (k: Props.Kind, from: Input.pose, to: Input.pose) => if k == Props.Rifle then wallAt(k, from, to) else Option.None } in
  let path = List.range(20.0) |> List.map((i) => (grabbed(foreOf(pushed(0.0 - 0.05 * (i + 1.0)), 0.25, 0.0, 0.0)), grabbed(pushed(0.0 - 0.05 * (i + 1.0))))) in
  let m = last(playBoth(rifleWall, ready, List.flatten([path, [(grabbed(foreOf(pushed(-1.0), 0.25, 0.0, 0.0)), hand(pushed(-1.0), 1.0, 1.0))]]))) in
  let accepted = rifleAccepted(m) in
  supportSide(m) == Option.Some(Hold.Left) && accepted.position.x > -0.31
    && (m.rifle.last |> Option.map((s) => Hold.near3(s.from, (Gun.boreOf(Longarm.weapon, accepted)).muzzle)) |> Option.defaultValue(false))
)

// The selector: A on the right hand holding the rifle switches it, once a
// press; the support hand's doesn't; F does, held or not.
expect (
  let snap = (pressed: List<Key.t>, l: Input.controller, r: Input.controller) => Game.snapshotOf(pressed, Option.Some({ head: Option.None, left: l, right: r })) in
  let a = { still with primaryPressed: true } in
  let pressed = Game.sampledInput(Game.sampledInput(ready, snap([], fore25, still)), snap([], fore25, a)) in
  let kept = Game.sampledInput(pressed, snap([], fore25, a)) in
  let byLeft = Game.sampledInput(Game.sampledInput(ready, snap([], fore25, still)), snap([], { fore25 with primaryPressed: true }, still)) in
  // A held as the rifle is taken (or retaken) is no press.
  let aHeld = Game.sampledInput(start, snap([], Game.away, { hand(onStock, 0.0, 0.0) with primaryPressed: true })) in
  let taken = last(play2(aHeld, [(Game.away, { hand(onStock, 0.0, 0.0) with primaryPressed: true }), (Game.away, { still with primaryPressed: true })])) in
  let heldThrough = Game.sampledInput(taken, snap([], Game.away, { still with primaryPressed: true })) in
  ready.rifle.mode == Gun.Semi && pressed.rifle.mode == Gun.Auto && kept.rifle.mode == Gun.Auto && byLeft.rifle.mode == Gun.Semi
    && Game.isHeld(taken, Props.Rifle) && heldThrough.rifle.mode == Gun.Semi
    && (Game.sampledInput(start, snap([Key.F], Game.away, Game.away))).rifle.mode == Gun.Auto
    && (Game.sampledInput(pressed, snap([Key.F], fore25, still))).rifle.mode == Gun.Semi
)

// --- the rack ---

// Let go in the rack by the hand holding it alone, it goes back in the rack.
expect (
  let m = last(play2(ready, [(hand(foreOf(onStock, 0.25, 0.0, 0.0), 0.0, 0.0), still), (Game.away, still), (Game.away, hand(onStock, 0.0, 0.0))])) in
  rifle(m).slot == Option.Some(Slots.rack.id) && not Game.isHeld(m, Props.Rifle) && Hold.near(rifle(m).pose, rifleAt)
)
// It doesn't go in a thigh holster: opened there, it drops.
expect (
  let holster = Hold.at(Inventory.rightHolster, Quat.identity) in
  let m = last(play2(ready, List.flatten([[(Game.away, still)], both(10.0, Game.away, grabbed(holster)), [(Game.away, hand(holster, 0.0, 0.0))]]))) in
  Option.isNone(rifle(m).slot) && not Game.isHeld(m, Props.Rifle)
)
// The primary hand lost, the support opening lets go: the rifle stays where
// it was, still in the (lost) right hand.
expect (
  let ms = play2(ready, [(fore25, Game.away), (hand(foreOf(onStock, 0.25, 0.0, 0.0), 0.0, 0.0), Game.away)]) in
  let m = last(ms) in
  Option.isNone(supportSide(m)) && Game.isHeld(m, Props.Rifle) && Hold.near(rifleAccepted(m), rifleAt)
    && countOf(Hold.Left, Pulses.Release, ms) == 1.0 && Option.isNone(Game.inHand(m, Hold.Left))
)
// Retaken with the wrist turned 30 degrees off the bore, the foregrip hand
// 12.5 cm off the new primary's axis: it doesn't stay on as the support (nor
// buzz a break); the right holds it alone.
expect (
  let yawed = Hold.at(onStock.position, Quat.yaw(30.0 * Math.pi / 180.0)) in
  let ms = play2(last(handedOver), [(fore25, hand(yawed, 0.0, 0.0)), (fore25, grabbed(yawed))]) in
  let m = last(ms) in
  (rifle(m).held |> Option.map((h) => h.side == Hold.Right) |> Option.defaultValue(false)) && Option.isNone(supportSide(m))
    && countOf(Hold.Left, Pulses.Break, ms) == 0.0
)
// Nor is the foregrip taken while the rifle is still settling into a wrist
// turned 30 degrees: off the primary's axis, it would break off at once.
expect (
  let yawed = Hold.at(onStock.position, Quat.yaw(30.0 * Math.pi / 180.0)) in
  let m = last(play2(start, [(Game.away, hand(yawed, 0.0, 0.0)), (hand(foreOf(onStock, 0.25, 0.0, 0.0), 0.0, 0.0), grabbed(yawed)), (fore25, grabbed(yawed))])) in
  Game.isHeld(m, Props.Rifle) && Option.isNone(supportSide(m))
)
// Both hands opening at once in the rack: racked, one pulse each.
expect (
  let ms = play2(ready, [(hand(foreOf(onStock, 0.25, 0.0, 0.0), 0.0, 0.0), hand(onStock, 0.0, 0.0))]) in
  rifle(last(ms)).slot == Option.Some(Slots.rack.id)
    && pulsesOf(List.drop(1.0, ms)) == [(Hold.Right, Pulses.Stow), (Hold.Left, Pulses.Release)]
)

// --- ammunition ---

let initialRounds = Game.unspent(start)
/// Every model keeps the station's rounds: unspent plus both guns' shots,
/// plus the loose rounds taken away.
let conserved = (ms: List<Game.Model>): bool => ms |> List.all((m) => Game.unspent(m) + m.gun.fired + m.rifle.fired + m.lost == initialRounds)
expect initialRounds == 13.0 + 31.0 + 2.0 * 12.0 + 2.0 * 30.0

/// `n` pulls of the pistol's trigger, each let out.
let pulls = (n: float) => List.range(n) |> List.concatMap((_) => [pull, ease])

// Loaded 12+1: thirteen pulls fire thirteen; the fourteenth clicks dry (a
// click pulse, no shot, no kick), leaving 0+0.
let emptied = play(List.flatten([take, pulls(14.0)]))
expect (
  let m = last(emptied) in
  m.gun.fired == 13.0 && m.gun.dry == 1.0 && count(Pulses.Fire, emptied) == 13.0 && count(Pulses.Click, emptied) == 1.0
    && Ammo.roundsIn(m.gun.ammo.mag) == 0.0 && Option.isSome(m.gun.ammo.mag) && not m.gun.ammo.chambered && m.gun.kick < 1.0
    && conserved(emptied)
)
// Full-auto on a full magazine: exactly 31 shots, then it stops with one
// dry click (not one at its rate); let out and pulled again, another.
expect (
  let ms = holding(auto(ready), 200.0) in
  let m = last(ms) in
  m.rifle.fired == 31.0 && m.rifle.dry == 1.0 && countOf(Hold.Right, Pulses.Fire, ms) == 31.0 && countOf(Hold.Right, Pulses.Click, ms) == 1.0
    && Ammo.count(m.rifle.ammo) == 0.0 && conserved(ms)
    && (last(play2(m, [(fore25, still), (fore25, hand(onStock, 1.0, 1.0))]))).rifle.dry == 2.0
)

/// A step through `sampledInput` (keys pressed, the two hands), then the step.
let sampledIn = (world: Game.World, m: Game.Model, keys: List<Key.t>, l: Input.controller, r: Input.controller): Game.Model =>
  let xr = { head: Option.Some(Hips.headAt(0.0, 0.0)), left: l, right: r } in
  let (next, _) = Game.step(Game.sampledInput(m, Game.snapshotOf(keys, Option.Some(xr))), Stations.armory, world, (p) => p, 0.0, dt) in
  next
let sampled = (m: Game.Model, keys: List<Key.t>, l: Input.controller, r: Input.controller): Game.Model => sampledIn(Game.noWorld, m, keys, l, r)
let stickDown = (c: Input.controller): Input.controller => { c with thumbstick: { x: 0.0, y: -1.0 } }
let dropped = (m: Game.Model) => Game.propOf(m, Props.Mag(Ammo.PistolMag, List.length(Stations.spares) + 1.0))

// Three shots, then the thumbstick down on the pistol's hand: its magazine
// (9) drops out below it as a new prop; the chambered round stays.
let fired3 = last(play(List.flatten([take, pulls(3.0)])))
let ejected = sampled(fired3, [], Game.away, stickDown(ease))
expect (
  let mag = dropped(ejected) in
  mag.mag == Option.Some({ kind: Ammo.PistolMag, rounds: 9.0, capacity: 12.0 }) && Option.isNone(ejected.gun.ammo.mag) && ejected.gun.ammo.chambered
    && List.length(ejected.props) == 7.0 && Option.isNone(mag.held) && Option.isNone(mag.slot)
    && mag.at.position.y < (Game.poseOf(pistol(ejected))).position.y - 0.1 && conserved([fired3, ejected])
)
// M drops it too; the other hand's stick doesn't, nor M with no gun held.
expect Option.isNone((sampled(fired3, [Key.M], Game.away, ease)).gun.ammo.mag)
expect Option.isSome((sampled(fired3, [], stickDown(Game.away), ease)).gun.ammo.mag)
expect List.length((sampled(start, [Key.M], Game.away, Game.away)).props) == 6.0
// Its way out blocked by the level (the pistol resting on the bench): it
// stays in, and nothing drops.
expect (
  let floored = { Game.noWorld with sweep: (k: Props.Kind, a: Input.pose, b: Input.pose) =>
    match k with
    | Props.Mag(_, _) => Option.Some({ fraction: 0.0, normal: Quat.v3(0.0, 1.0, 0.0) })
    | _ => Option.None } in
  let m = sampledIn(floored, fired3, [], Game.away, stickDown(ease)) in
  Option.isSome(m.gun.ammo.mag) && List.length(m.props) == 6.0 && m.nextMag == fired3.nextMag && conserved([m])
)
// The rifle's, by M, held in two hands: 30 rounds out, one chambered.
expect (
  let m = sampled(ready, [Key.M], fore25, still) in
  let mag = Game.propOf(m, Props.Mag(Ammo.RifleMag, List.length(Stations.spares) + 1.0)) in
  Ammo.roundsIn(mag.mag) == 30.0 && Option.isNone(m.rifle.ammo.mag) && m.rifle.ammo.chambered && conserved([m])
)
// With the magazine out, the chambered round still fires once; then dry.
expect (
  let ms = playIn(Game.noWorld, ejected, pulls(2.0)) in
  (last(ms)).gun.fired == 4.0 && (last(ms)).gun.dry == 1.0 && conserved(ms)
)
// The left hand takes the dropped magazine, puts it in the pouch and takes it
// back out: the same 9 rounds throughout.
let magAt = (dropped(ejected)).pose.position
let pouched = playBoth(Game.noWorld, ejected, List.flatten([
  [(Game.hand(magAt, 0.0), ease), (Game.hand(magAt, 1.0), ease)],
  both(10.0, Game.hand(Inventory.pouch, 1.0), ease),
  [(Game.hand(Inventory.pouch, 0.0), ease)],
]))
let retrieved = playBoth(Game.noWorld, last(pouched), [(Game.hand(Inventory.pouch, 0.0), ease), (Game.hand(Inventory.pouch, 1.0), ease)])
expect (
  let nine = (m: Game.Model) => Ammo.roundsIn((dropped(m)).mag) == 9.0 in
  (List.nth(2.0, pouched) |> Option.map((m) => Option.isSome((dropped(m)).held)) |> Option.defaultValue(false))
    && (dropped(last(pouched))).slot == Option.Some(Slots.pouch.id)
    && ((dropped(last(retrieved))).held |> Option.map((h) => h.side == Hold.Left) |> Option.defaultValue(false))
    && Option.isNone((dropped(last(retrieved))).slot)
    && (pouched |> List.all(nine)) && (retrieved |> List.all(nine)) && conserved(List.append(pouched, retrieved))
)
// A spare taken out of its cradle (an unstow), and put back: in its cradle,
// bodiless, still full. A pistol magazine doesn't go in a rifle cradle.
expect (
  let spare = Props.Mag(Ammo.PistolMag, 1.0) in
  let at = (Stations.spawn(spare)).position in
  let rifleCradle = (Stations.spawn(Props.Mag(Ammo.RifleMag, 3.0))).position in
  let taken = playBoth(Game.noWorld, start, [(Game.hand(at, 0.0), Game.away), (Game.hand(at, 1.0), Game.away)]) in
  let back = playBoth(Game.noWorld, last(taken), [(Game.hand(at, 0.0), Game.away)]) in
  let moved = playBoth(Game.noWorld, last(taken), [(Game.hand(rifleCradle, 1.0), Game.away), (Game.hand(rifleCradle, 0.0), Game.away)]) in
  (start.props |> List.all((p) => Option.isNone(p.mag) || Option.isSome(p.slot)))
    && (Game.propOf(last(taken), spare).held |> Option.map((h) => h.side == Hold.Left) |> Option.defaultValue(false))
    && countOf(Hold.Left, Pulses.Unstow, taken) == 1.0
    && Game.propOf(last(back), spare).slot == Option.Some(Slots.rack.id + 1.0) && Ammo.roundsIn(Game.propOf(last(back), spare).mag) == 12.0
    && Option.isNone(Game.propOf(last(moved), spare).slot) && not Game.isHeld(last(moved), spare)
)

// --- reloading: the well, the slide, the lock ---

// The pistol taken and raised 30 cm (clear of the bench), held level.
let up = moved(onGrip, 0.0, 0.3, 0.0)
let upHold = hand(up, 1.0, 0.0)
let upPull = hand(up, 1.0, 1.0)
let gunUp = Gun.heldAt(Gun.pistol, up)
/// A pose `d` along the raised pistol's well (0 seated), and its mouth.
let wellUp = (d: float): Input.pose => Hold.compose(gunUp, Gun.wellAt(Gun.pistol, d))
let mouthUp = wellUp(0.0 - Gun.pistol.well.drop)
/// `n` steps pushing up the well from its mouth, past home.
let pushUp = (well: (float) => Input.pose, drop: float, other: Input.controller) =>
  List.range(12.0) |> List.map((i) => (lefty(well(drop * ((i + 1.0) / 10.0 - 1.0)), 1.0), other))
/// The left hand at `p` (a magazine taken by its centre is where the hand is, turned with it).
let lefty = (p: Input.pose, squeeze: float): Input.controller => Game.turnedHand(p.position, p.orientation, squeeze)
let turnedBy = (p: Input.pose, axis: Input.point3, deg: float): Input.pose => Hold.compose(p, Hold.at(Hold.zero, Quat.axisAngle(axis, deg * Math.pi / 180.0)))
let xAxis = Quat.v3(1.0, 0.0, 0.0)
let upPulls = (n: float) => List.range(n) |> List.concatMap((_) => [(Game.away, upPull), (Game.away, upHold)])
let latchOf = (m: Game.Model, k: Props.Kind) => Game.propOf(m, k).latch
let present = (m: Game.Model, k: Props.Kind): bool => m.props |> List.any((p) => p.kind == k)
let spare = Props.Mag(Ammo.PistolMag, 1.0)
let spareAt = Stations.spawn(spare)
let rounds = (m: Game.Model): List<Game.Prop> => m.props |> List.filter((p) => Game.isRound(p.kind))
/// The left hand on the raised pistol's slide, `back` behind home.
let onSlideUp = (back: float): Input.pose => Hold.compose(gunUp, Hold.at(Quat.add(Gun.pistol.rack.at, Quat.v3(0.0, 0.0, back)), Quat.identity))
/// Take the slide where it rests (`rest` back), pull it `to` back, let go.
let rackUp = (rest: float, to: float) => List.flatten([
  [(lefty(onSlideUp(rest), 0.0), upHold), (lefty(onSlideUp(rest), 1.0), upHold)],
  List.range(4.0) |> List.map((i) => (lefty(onSlideUp(rest + (to - rest) * (i + 1.0) / 4.0), 1.0), upHold)),
  [(lefty(onSlideUp(to), 0.0), upHold)],
])

// Emptied: 13 shots, then a dry pull. The slide locked back on the empty
// magazine, on the last shot, drawn back (`lockBack`).
let emptiedUp = playBoth(Game.noWorld, start, List.flatten([take |> List.map((c) => (Game.away, c)), both(6.0, Game.away, upHold), upPulls(14.0)]))
let lockedOpen = last(emptiedUp)
expect (
  lockedOpen.gun.fired == 13.0 && lockedOpen.gun.dry == 1.0 && lockedOpen.gun.ammo.locked
    && Ammo.roundsIn(lockedOpen.gun.ammo.mag) == 0.0 && Option.isSome(lockedOpen.gun.ammo.mag)
    && Gun.slideBack(Gun.pistol, lockedOpen.gun) == Gun.pistol.rack.lockBack && conserved(emptiedUp)
    && (emptiedUp |> List.all((m) => not m.gun.ammo.locked || m.gun.fired == 13.0))
)
// The magazine released: it drops; still locked.
let releasedMag = sampled(lockedOpen, [], Game.away, stickDown(upHold))
expect Option.isNone(releasedMag.gun.ammo.mag) && releasedMag.gun.ammo.locked && List.length(releasedMag.props) == 7.0
// A spare out of its cradle, in the left hand.
let tookSpare = playBoth(Game.noWorld, releasedMag, [(lefty(spareAt, 0.0), upHold), (lefty(spareAt, 1.0), upHold)])
expect (Game.propOf(last(tookSpare), spare).held |> Option.map((h) => h.side == Hold.Left) |> Option.defaultValue(false))

// Brought to the mouth tipped 45° (dot 0.71): no latch. Lined up there: it
// latches (a pulse). Pushed up the well over 10 steps: it seats (a pulse in
// each hand), the magazine in the gun (12), still locked, nothing chambered.
let misaligned = playBoth(Game.noWorld, last(tookSpare), both(10.0, lefty(turnedBy(mouthUp, xAxis, 45.0), 1.0), upHold))
let aligned = playBoth(Game.noWorld, last(misaligned), [(lefty(mouthUp, 1.0), upHold)])
let pushed = playBoth(Game.noWorld, last(aligned), pushUp(wellUp, Gun.pistol.well.drop, upHold))
expect (misaligned |> List.all((m) => Option.isNone(latchOf(m, spare)))) && countOf(Hold.Left, Pulses.Latch, misaligned) == 0.0
expect Option.isSome(latchOf(last(aligned), spare)) && countOf(Hold.Left, Pulses.Latch, aligned) == 1.0
expect (
  let m = last(pushed) in
  not present(m, spare) && Ammo.roundsIn(m.gun.ammo.mag) == 12.0 && m.gun.ammo.locked && not m.gun.ammo.chambered
    && countOf(Hold.Left, Pulses.Seat, pushed) == 1.0 && countOf(Hold.Right, Pulses.Seat, pushed) == 1.0
    && countOf(Hold.Left, Pulses.Release, pushed) == 0.0 && countOf(Hold.Left, Pulses.Stow, pushed) == 0.0
    && Option.isNone(Game.inHand(m, Hold.Left)) && conserved(List.flatten([tookSpare, misaligned, aligned, pushed]))
)
// Guided at the depth the hand pushes it to, on the axis, turned as the well is.
expect (
  let poses = List.append(pushed |> List.filter((m) => present(m, spare)), aligned) |> List.map((m) => Game.poseOf(Game.propOf(m, spare))) in
  (List.zip(List.drop(1.0, poses), poses) |> List.all((ab) => let (a, b) = ab in Quat.length(Quat.sub(a.position, b.position)) <= Gun.pistol.well.drop / 10.0 + 0.000001))
    && (List.nth(4.0, pushed) |> Option.map((m) => Hold.near(Game.poseOf(Game.propOf(m, spare)), wellUp(Gun.pistol.well.drop * (0.4 - 1.0)))) |> Option.defaultValue(false))
)
// A dry pull with the slide locked: a click, no shot.
expect (let ms = playBoth(Game.noWorld, last(pushed), [(Game.away, upHold), (Game.away, upPull)]) in last(ms).gun.dry == 2.0 && last(ms).gun.fired == 13.0 && countOf(Hold.Right, Pulses.Click, ms) == 1.0)

// Racked from locked: the left takes the slide where it rests back (a grab),
// nudges it to the back (a pulse; nothing chambered, nothing thrown out),
// lets go: home (a pulse in each hand), the lock released, chambered (11+1).
let racked = playBoth(Game.noWorld, last(pushed), List.append([(Game.away, upHold)], rackUp(Gun.pistol.rack.lockBack, Gun.slideTravel)))
expect (
  let m = last(racked) in
  not m.gun.ammo.locked && m.gun.ammo.chambered && Ammo.roundsIn(m.gun.ammo.mag) == 11.0 && List.isEmpty(rounds(m))
    && countOf(Hold.Left, Pulses.Grab, racked) == 1.0 && countOf(Hold.Left, Pulses.SlideBack, racked) == 1.0
    && countOf(Hold.Left, Pulses.SlideHome, racked) == 1.0 && countOf(Hold.Right, Pulses.SlideHome, racked) == 1.0
    && Option.isNone(m.gun.slide.hand) && Gun.slideBack(Gun.pistol, m.gun) == 0.0 && conserved(racked)
)
// Then 12 shots, and the 13th pull dry: locked back again.
expect (
  let ms = playBoth(Game.noWorld, last(racked), upPulls(13.0)) in
  last(ms).gun.fired == 25.0 && last(ms).gun.ammo.locked && last(ms).gun.dry == 2.0 && conserved(ms)
)
// The slide release (A on the pistol's hand) does what the rack does.
expect (
  let m = sampled(last(pushed), [], Game.away, { upHold with primaryPressed: true }) in
  not m.gun.ammo.locked && m.gun.ammo.chambered && Ammo.roundsIn(m.gun.ammo.mag) == 11.0 && m.pulses == [(Hold.Right, Pulses.SlideHome)]
)
// Pulled 70% of the way and let go: nothing. All the way with a round
// chambered: it is thrown out loose (a prop), and the next chambered.
let partial = playBoth(Game.noWorld, last(racked), rackUp(0.0, 0.7 * Gun.slideTravel))
let full = playBoth(Game.noWorld, last(racked), rackUp(0.0, Gun.slideTravel))
expect (
  let m = last(partial) in
  m.gun.ammo == (last(racked)).gun.ammo && List.isEmpty(rounds(m)) && countOf(Hold.Left, Pulses.SlideBack, partial) == 0.0
    && countOf(Hold.Left, Pulses.SlideHome, partial) == 0.0
)
expect (
  let m = last(full) in
  m.gun.ammo.chambered && Ammo.roundsIn(m.gun.ammo.mag) == 10.0 && List.length(rounds(m)) == 1.0
    && (rounds(m) |> List.all((p) => p.kind == Props.Round(Ammo.PistolMag, 1.0) && Option.isNone(p.held)))
    && conserved(full)
)
// The round leaves the port with a way out (right, up, back) to fly: an
// `Eject`, not a hand's release.
expect (
  let before = List.nth(4.0, full) |> Option.defaultValue(start) in
  let (_, out) = Game.step({ before with xr: Option.Some({ head: Option.Some(Hips.headAt(0.0, 0.0)), left: lefty(onSlideUp(Gun.slideTravel), 1.0), right: upHold }) }, Stations.armory, Game.noWorld, (p) => p, 0.0, dt) in
  match out with
  | [r] => r.kind == Game.Eject && r.prop == Props.Round(Ammo.PistolMag, 1.0) && Quat.length(r.velocity) > 1.5
  | _ => false
)
// Racked eight times (back, then home in the hand): eight thrown out, the
// oldest two taken away past the cap of six, counted lost.
expect (
  let stroke = [(lefty(onSlideUp(Gun.slideTravel), 1.0), upHold), (lefty(onSlideUp(0.0), 1.0), upHold)] in
  let ms = playBoth(Game.noWorld, last(racked), List.flatten([
    [(lefty(onSlideUp(0.0), 0.0), upHold), (lefty(onSlideUp(0.0), 1.0), upHold)], List.range(8.0) |> List.concatMap((_) => stroke)])) in
  let m = last(ms) in
  List.length(rounds(m)) == Game.looseCap && m.lost == 2.0 && Ammo.roundsIn(m.gun.ammo.mag) == 3.0
    && (rounds(m) |> List.map((p) => p.kind)) == (List.range(6.0) |> List.map((i) => Props.Round(Ammo.PistolMag, i + 3.0)))
    && conserved(ms)
)

// Reversed (upside down) at the mouth, and pushed up the well: never
// latches. A rifle magazine lined up at the pistol's mouth: never either.
expect (
  let flipped = (p: Input.pose) => turnedBy(p, xAxis, 180.0) in
  let ms = playBoth(Game.noWorld, last(tookSpare), List.append(both(10.0, lefty(flipped(mouthUp), 1.0), upHold),
    pushUp((d) => flipped(wellUp(d)), Gun.pistol.well.drop, upHold))) in
  (ms |> List.all((m) => Option.isNone(latchOf(m, spare)))) && countOf(Hold.Left, Pulses.Latch, ms) == 0.0 && present(last(ms), spare)
)
expect (
  let rifleSpare = Props.Mag(Ammo.RifleMag, 3.0) in
  let at = Stations.spawn(rifleSpare) in
  let ms = playBoth(Game.noWorld, releasedMag, List.flatten([[(lefty(at, 0.0), upHold), (lefty(at, 1.0), upHold)],
    both(10.0, lefty(mouthUp, 1.0), upHold), pushUp(wellUp, Gun.pistol.well.drop, upHold)])) in
  (Game.propOf(last(ms), rifleSpare).held |> Option.isSome) && (ms |> List.all((m) => Option.isNone(latchOf(m, rifleSpare)))) && Option.isNone((last(ms)).gun.ammo.mag)
)

// Pulled out half way in: it stays latched to 7 cm off the axis, then
// leaves where it is (no jump), back in the hand, easing to where the hand
// holds it (a settling step a step at most); the gun stays empty.
let halfIn = playBoth(Game.noWorld, last(aligned), List.range(5.0) |> List.map((i) => (lefty(wellUp(0.0 - Gun.pistol.well.drop + 0.012 * (i + 1.0)), 1.0), upHold)))
let pullHand = (i: float) => Hold.compose(wellUp(0.0 - Gun.pistol.well.drop + 0.06), Hold.p(0.01 * i, 0.0, 0.0))
let pulledOut = playBoth(Game.noWorld, last(halfIn), List.range(12.0) |> List.map((i) => (lefty(pullHand(i + 1.0), 1.0), upHold)))
let leftAt = pulledOut |> List.indexedMap((i, m) => (i, m)) |> List.find((im) => let (_, m) = im in Option.isNone(latchOf(m, spare)))
  |> Option.map((im) => let (i, _) = im in i) |> Option.defaultValue(0.0)
expect (
  let magAt = (i: float) => Game.poseOf(Game.propOf(List.nth(i, pulledOut) |> Option.defaultValue(start), spare)) in
  leftAt == 8.0 && Hold.near(magAt(leftAt), magAt(leftAt - 1.0))
    && (List.range(4.0) |> List.all((i) => Quat.length(Quat.sub((magAt(leftAt + i + 1.0)).position, (magAt(leftAt + i)).position)) <= Hold.snapStep + 0.01 + 0.000001))
    && Hold.near(magAt(12.0), pullHand(12.0)) && Option.isNone((last(pulledOut)).gun.ammo.mag)
    && (Game.propOf(last(pulledOut), spare).held |> Option.map((h) => h.side == Hold.Left) |> Option.defaultValue(false))
)
// Let go latched, it falls out of the mouth, free and clear of the gun.
expect (
  let m = last(playBoth(Game.noWorld, last(halfIn), [(lefty(wellUp(-0.06), 0.0), upHold)])) in
  let p = Game.propOf(m, spare) in
  Option.isNone(p.held) && Option.isNone(p.latch) && Hold.near(p.at, mouthUp) && Option.isNone(m.gun.ammo.mag)
)

// The bench: the pistol lying on its right side, nobody holding it, its
// well empty. A spare pushed into it there seats (a pulse in that hand only).
expect (
  let lying = Hold.at(Quat.add(pistolAt.position, Quat.v3(0.0, 0.02 - 0.05, 0.0)), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.0 - Math.pi / 2.0)) in
  let onBench = { Game.withProp({ pistol(start) with pose: lying, at: lying }, start) with gun: { start.gun with ammo: { start.gun.ammo with mag: Option.None } } } in
  let well = (d: float) => Hold.compose(lying, Gun.wellAt(Gun.pistol, d)) in
  let ms = playBoth(Game.noWorld, onBench, List.flatten([[(lefty(spareAt, 0.0), Game.away), (lefty(spareAt, 1.0), Game.away)],
    [(lefty(well(0.0 - Gun.pistol.well.drop), 1.0), Game.away)], pushUp(well, Gun.pistol.well.drop, Game.away)])) in
  let m = last(ms) in
  not present(m, spare) && Ammo.roundsIn(m.gun.ammo.mag) == 12.0 && m.gun.ammo.chambered
    && countOf(Hold.Left, Pulses.Seat, ms) == 1.0 && countOf(Hold.Right, Pulses.Seat, ms) == 0.0
)

// The rifle, one-handed off the rack: M drops its magazine (one chambered);
// a rifle spare pushed into its well seats; the charging handle pulled all
// the way back throws the chambered round out, and home it chambers from
// the new magazine (29+1).
expect (
  let rifleSpare = Props.Mag(Ammo.RifleMag, 3.0) in
  let held = last(play2(start, List.flatten([[(Game.away, hand(onStock, 0.0, 0.0))], both(4.0, Game.away, still)]))) in
  let out = sampled(held, [Key.M], Game.away, still) in
  let at = Stations.spawn(rifleSpare) in
  let well = (d: float) => Hold.compose(rifleAt, Gun.wellAt(Longarm.weapon, d)) in
  let handle = (back: float) => Hold.compose(rifleAt, Hold.at(Quat.add(Longarm.weapon.rack.at, Quat.v3(0.0, 0.0, back)), Quat.identity)) in
  let ms = play2(out, List.flatten([
    [(lefty(at, 0.0), still), (lefty(at, 1.0), still), (lefty(well(0.0 - Longarm.weapon.well.drop), 1.0), still)],
    pushUp(well, Longarm.weapon.well.drop, still),
    [(lefty(handle(0.0), 0.0), still), (lefty(handle(0.0), 1.0), still)],
    List.range(6.0) |> List.map((i) => (lefty(handle(Longarm.handleTravel * (i + 1.0) / 6.0), 1.0), still)),
    [(lefty(handle(Longarm.handleTravel), 0.0), still)]])) in
  let m = last(ms) in
  Option.isNone(out.rifle.ammo.mag) && out.rifle.ammo.chambered
    && not present(m, rifleSpare) && Ammo.roundsIn(m.rifle.ammo.mag) == 29.0 && m.rifle.ammo.chambered && not m.rifle.ammo.locked
    && (rounds(m) |> List.map((p) => p.kind)) == [Props.Round(Ammo.RifleMag, 1.0)]
    && countOf(Hold.Left, Pulses.SlideBack, ms) == 1.0 && countOf(Hold.Right, Pulses.SlideHome, ms) == 1.0
    && conserved(List.append([out], ms))
)
// Fired empty, the rifle's bolt locks back (drawn back), its handle home.
expect (
  let m = last(holding(auto(ready), 200.0)) in
  m.rifle.ammo.locked && Gun.slideBack(Longarm.weapon, m.rifle) == 0.0 && Longarm.boltBack(m.rifle) > 0.04
)
// A loose round come to rest lies bodiless where it is; the left hand takes
// it from there, and it is a held prop again.
expect (
  let round = Props.Round(Ammo.PistolMag, 1.0) in
  let at = Hold.p(-0.2, 0.905, -0.6) in
  let lying = Game.withProp({ Game.propOf(last(full), round) with pose: at, lying: true }, last(full)) in
  let m = last(playBoth(Game.noWorld, lying, [(lefty(at, 0.0), upHold), (lefty(at, 1.0), upHold)])) in
  let p = Game.propOf(m, round) in
  not p.lying && (p.held |> Option.map((h) => h.side == Hold.Left && Hold.near(h.accepted, at)) |> Option.defaultValue(false))
)
