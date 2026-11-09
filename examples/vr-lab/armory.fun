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
let count = (e: Pulses.Event, ms: List<Game.Model>): float => pulsesOf(ms) |> List.filter((se) => se == (Hold.Right, e)) |> List.length
let pistol = (m: Game.Model) => Game.propOf(m, Props.Pistol)

// Taken open-triggered, held a moment: settled in the hand.
let take = List.flatten([[hand(onGrip, 0.0, 0.0), hand(onGrip, 1.0, 0.0)], times(5.0, hand(onGrip, 1.0, 0.0))])
let pull = hand(onGrip, 1.0, 1.0)
let ease = hand(onGrip, 1.0, 0.0)

expect Game.isHeld(last(play(take)), Props.Pistol)
// The pistol and the rifle are at the armory, and the holsters are worn.
expect List.length(start.props) == 2.0 && Stations.worn(Stations.armory)

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
let countOf = (side: Hold.Side, e: Pulses.Event, ms: List<Game.Model>): float =>
  pulsesOf(ms) |> List.filter((se) => se == (side, e)) |> List.length
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
  let snap = (pressed: List<Key.t>, l: Input.controller, r: Input.controller): Input.snapshot =>
    { heldKeys: [], pressedKeys: pressed, releasedKeys: [],
      mouse: { x: 0.0, y: 0.0, surfaceWidth: 1.0, surfaceHeight: 1.0,
               buttons: { left: false, right: false, middle: false },
               pressed: { left: false, right: false, middle: false },
               released: { left: false, right: false, middle: false } },
      xr: Option.Some({ head: Option.None, left: l, right: r }), gamepad: Option.None, touch: Option.None } in
  let a = { still with primaryPressed: true } in
  let pressed = Game.sampledInput(Game.sampledInput(ready, snap([], fore25, still)), snap([], fore25, a)) in
  let kept = Game.sampledInput(pressed, snap([], fore25, a)) in
  let byLeft = Game.sampledInput(Game.sampledInput(ready, snap([], fore25, still)), snap([], { fore25 with primaryPressed: true }, still)) in
  ready.rifle.mode == Gun.Semi && pressed.rifle.mode == Gun.Auto && kept.rifle.mode == Gun.Auto && byLeft.rifle.mode == Gun.Semi
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
