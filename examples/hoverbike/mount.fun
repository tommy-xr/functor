// mount.fun — getting on and off the bike, as pure functions: who is near
// enough to mount, the fade both ways, the dismount hold and speed gate, and
// where to set the walker down.
//
// Both ways go through a fade to black (`Bars.fadeStep`, the stepped turn's
// timeline): whatever moves the view happens on the step it is black, never
// as a sweep. `game.fun` reads the world (positions, casts) and runs these.

// ---- Near the bike -------------------------------------------------------

// Mounting needs the walker within `range` of the seat, horizontally, and
// looking at it within 60°.
let range = 2.0 // m
let facing = 0.5 // cos 60°
// The seat, below the eye, in the bike's frame.
let seat: Input.point3 = { Bars.seatedEye with y: 0.66 }

// Whether a walker at (x, z), looking along `look` (yaw: forward is
// (sin, cos)), is near the seat at (sx, sz).
let near = (x: float, z: float, look: float, sx: float, sz: float): bool =>
  let dx = sx - x in
  let dz = sz - z in
  let d = Math.sqrt(dx * dx + dz * dz) in
  d < range && (d < 0.001 || (dx * Math.sin(look) + dz * Math.cos(look)) / d > facing)

// With a headset, a fresh squeeze within `reach` of a grip grabs it: a hand on
// the grip, give or take the controller's size and the calibration.
let reach = 0.3 // m

// Parked, the grips sit 0.37 m below the walker's eye. Calibration puts any
// player's head at that eye, so for real eye heights of 1.6–1.75 m that is
// below the shoulder (≈ 0.126 × eye height under the eye) and above the hip
// (≈ 0.43 × eye height under it): a grab without reaching up or stooping.
let gripDrop = Walk.centreHeight + Walk.eye.y - (Ride.idleHeight + Bars.gripHeight)
expect gripDrop > 0.126 * 1.75 && gripDrop < 0.43 * 1.6

// A fresh squeeze: squeezing now, not the step before, tracked both times.
let fresh = (before: Bars.Hand, now: Bars.Hand) =>
  now.tracked && before.tracked && now.squeezing && not before.squeezing

// Whether a mount request (on foot, not fading) goes through: near the seat;
// and for a headset hand's grab (`hand` ≠ 0), calibrated, with that hand
// `toGrip` from its grip.
let eligible = (near: bool, hand: float, calibrated: bool, toGrip: float) =>
  near && (hand == 0.0 || (calibrated && toGrip < reach))

// ---- The fade ------------------------------------------------------------

// What happens at the black: mount (the hand whose grab did it: 1 left, -1
// right, 0 none), or set the walker down at a capsule centre, facing `yaw`.
type Action =
  | GetOn(hand: float)
  | GetOff(x: float, y: float, z: float, yaw: float)

// Getting on stays black for `rise` after the black step, while the parked
// bike lifts to ride height (`height`): the spring takes about 1.3 s to carry
// it the 0.9 m to within 1 cm, so the view never rises once it fades in.
let rise = 1.3 // s

// `t` is the time on `Bars.fadeStep`'s timeline, negative when idle; it
// stops at the black while `hold` counts down.
type Fade = { t: float, hold: float, action: Action }
let clear: Fade = { t: -1.0, hold: 0.0, action: GetOn(0.0) }
let gettingOn = (a: Action) =>
  match a with
  | GetOn(_) => true
  | GetOff(_, _, _, _) => false
let start = (a: Action): Fade => { t: 0.0, hold: if gettingOn(a) then rise else 0.0, action: a }
let fading = (f: Fade) => f.t >= 0.0

// One step: the next fade, and whether its action happens on this step.
// (The tolerance absorbs the float sum of the held steps.)
let step = (dt: float, f: Fade): (Fade, bool) =>
  if f.t < 0.0 then (f, false)
  else if Bars.black(f.t) && f.hold > 0.000001 then ({ f with hold: f.hold - dt }, false)
  else
    let (t, black) = Bars.fadeStep(dt, f.t) in
    ({ f with t: t }, black)

// The hover target: ride height while ridden, and through getting off's
// fade; else the idle height. So the bike rises from getting on's black step
// (unseen) and sinks once getting off has faded back in.
let height = (riding: bool, f: Fade) =>
  if riding || (fading(f) && not gettingOn(f.action)) then Ride.hoverHeight else Ride.idleHeight

// ---- Getting off ---------------------------------------------------------

// Hold the dismount button for `holdTime`, below `maxSpeed`. Holding faster
// asks the rider to slow down, and getting off waits for it.
let holdTime = 0.5 // s
let maxSpeed = 2.0 // m/s

// How long a button has been held, or -1 when it is up.
let held = (down: bool, dt: float, t: float) => if not down then -1.0 else if t < 0.0 then 0.0 else t + dt

// The dismount hold: like `held`, but only counting a press that began while
// riding, so the press that mounted can't carry over and get straight off.
let hold = (riding: bool, pressed: bool, down: bool, dt: float, t: float) =>
  if not riding || not down then -1.0 else if pressed then 0.0 else if t < 0.0 then -1.0 else t + dt

// (The tolerance absorbs the float sum of the steps.)
let long = (t: float) => t >= holdTime - 0.000001
let wantsOff = (t: float, speed: float) => long(t) && speed < maxSpeed
let tooFast = (t: float, speed: float) => t >= 0.0 && speed >= maxSpeed

// Where to set the walker down, in the bike's frame: its left, its right,
// then behind. Each clears the bike's capsule (0.8 m) and the walker's
// (0.3 m) by half a metre.
let side = 1.6
let behind = 2.2
let spots: List<Input.point3> = [
  { x: side, y: 0.0, z: 0.0 },
  { x: -side, y: 0.0, z: 0.0 },
  { x: 0.0, y: 0.0, z: -behind },
]

// The first spot `probe` finds room at (it answers the capsule centre
// there), or None: no room, and the rider stays on.
let landing = (probe: (Input.point3) => Option.t<Input.point3>): Option.t<Input.point3> =>
  spots |> List.fold((found, spot) =>
    match found with
    | Option.Some(_) => found
    | Option.None => probe(spot), Option.None)

// The yaw that looks from (x, z) at (tx, tz).
let toward = (x: float, z: float, tx: float, tz: float) => Math.atan2(tx - x, tz - z)

// ---- Tests ---------------------------------------------------------------

// Near and facing: inside 2 m, within 60°; behind or past 2 m, not.
expect near(0.0, -1.5, 0.0, 0.0, 0.0) && near(0.0, -1.5, 0.9, 0.0, 0.0)
expect not near(0.0, -1.5, 1.2, 0.0, 0.0) && not near(0.0, -1.5, Math.pi, 0.0, 0.0)
expect not near(0.0, -2.1, 0.0, 0.0, 0.0) && near(1.4, 1.4, toward(1.4, 1.4, 0.0, 0.0), 0.0, 0.0)
// A fresh squeeze, not a held or untracked one.
expect (
  let open = { Bars.loose with tracked: true } in
  let squeezing = { open with squeezing: true } in
  fresh(open, squeezing) && not fresh(squeezing, squeezing) && not fresh(Bars.loose, squeezing)
)

// Eligibility: E needs only to be near; a grab needs the calibration and the
// reach.
expect eligible(true, 0.0, false, 9.0) && eligible(true, 1.0, true, 0.25) && not eligible(false, 0.0, true, 0.0)
expect not eligible(true, -1.0, true, 0.35) && not eligible(true, 1.0, false, 0.1)

// The fade: its action lands on the step it is fully black, six steps in at
// 60 Hz; getting off is clear again after 15 (to a float's width), getting on
// after 15 plus the 78 held black; never black twice.
let run = (n: float, f: Fade) =>
  List.range(n) |> List.fold((acc, i) =>
    let (g, blacks, levels) = acc in
    let (h, b) = step(Ride.dt60, g) in
    (h, (if b then [i, ..blacks] else blacks), [Bars.fadeAt(h.t), ..levels]), (f, [], []))
expect (
  let (f, blacks, levels) = run(20.0, start(GetOff(0.0, 0.0, 0.0, 0.0))) in
  blacks == [5.0] && List.nth(14.0, levels) == Option.Some(1.0) && not fading(f)
    && List.head(levels) == Option.Some(0.0) && List.length(levels |> List.filter((l) => l > 0.001)) == 14.0
)
expect (
  let (f, blacks, levels) = run(100.0, start(GetOn(1.0))) in
  let held = levels |> List.filter((l) => l > 0.999) in
  blacks == [5.0] && not fading(f) && List.length(held) == 79.0
    && List.length(levels |> List.filter((l) => l > 0.001)) == 92.0
)
expect (let (f, b) = step(Ride.dt60, clear) in not b && f == clear)

// The bike's height through a fade, as `tick` steps it over flat ground: the
// target from the mode and fade before the step, then the fade, whose black
// step switches the mode. Oldest first.
type Lift = { h: float, s: Ride.State, f: Fade, riding: bool, level: float }
let lifting = (n: float, riding: bool, h0: float, f0: Fade): List<Lift> =>
  List.range(n) |> List.fold((acc, i) =>
    let l = acc |> List.head |> Option.defaultValue({ h: h0, s: Ride.rest, f: f0, riding: riding, level: 0.0 }) in
    let s = Ride.step(Ride.idle, height(l.riding, l.f), Ride.flatProbes(l.h), Ride.dt60, l.s) in
    let (f, black) = step(Ride.dt60, l.f) in
    [{ h: l.h + s.vy * Ride.dt60, s: s, f: f, riding: if black then gettingOn(f.action) else l.riding, level: Bars.fadeAt(f.t) }, ..acc],
    []) |> List.reverse
let within = (to: float, tolerance: float, l: Lift) => Math.abs(l.h - to) < tolerance

// Parked, it stays at the idle height.
expect lifting(120.0, false, Ride.idleHeight, clear) |> List.all((l) => within(Ride.idleHeight, 0.000001, l))
// Getting on: nothing moves before the black; the rise happens while it is
// black; from the first fade-in step on, the bike is at ride height within
// 1.5 cm, nearly still, and only settles from there.
expect (
  let run = lifting(240.0, false, Ride.idleHeight, start(GetOn(1.0))) in
  let before = run |> List.take(6.0) in
  let after = run |> List.drop(6.0) |> List.filter((l) => l.level < 0.999) in
  let first = after |> List.head |> Option.defaultValue({ h: 0.0, s: Ride.rest, f: clear, riding: false, level: 0.0 }) in
  (before |> List.all((l) => within(Ride.idleHeight, 0.000001, l)))
    && first.level > 0.8 && first.riding && within(Ride.hoverHeight, 0.015, first) && Math.abs(first.s.vy) < 0.1
    && (after |> List.all((l) => within(Ride.hoverHeight, 0.015, l)))
)
// Getting off: it holds ride height through the fade, then sinks to the idle
// height and settles there, its error never growing.
expect (
  let run = lifting(300.0, true, Ride.hoverHeight, start(GetOff(0.0, 0.0, 0.0, 0.0))) in
  let fade = run |> List.take(15.0) in
  let parked = run |> List.drop(15.0) in
  let error = (from: float, n: float) =>
    parked |> List.drop(from) |> List.take(n) |> List.map((l) => Math.abs(l.h - Ride.idleHeight)) |> List.maximum |> Option.defaultValue(9.0) in
  (fade |> List.all((l) => within(Ride.hoverHeight, 0.000001, l)))
    && (parked |> List.all((l) => not l.riding && not fading(l.f)))
    && error(180.0, 105.0) < 0.01 && error(225.0, 60.0) <= error(165.0, 60.0)
)

// The dismount hold: counts only from a press while riding; fires at 0.5 s
// below 2 m/s; faster, it asks to slow down and fires once slow.
let holding = (n: float, riding: bool, t0: float) =>
  List.range(n) |> List.fold((t, i) => hold(riding, i == 0.0 && t0 < 0.0, true, Ride.dt60, t), t0)
expect not wantsOff(holding(30.0, true, -1.0), 1.0) && wantsOff(holding(31.0, true, -1.0), 1.0)
expect not wantsOff(holding(31.0, true, -1.0), 2.0) && tooFast(holding(31.0, true, -1.0), 2.0)
  && tooFast(holding(1.0, true, -1.0), 5.0) && not tooFast(holding(1.0, true, -1.0), 1.0)
// Held through the mount (no press while riding): never counts.
expect hold(true, false, true, Ride.dt60, -1.0) == -1.0
// Released, or no longer riding: it resets.
expect hold(true, false, false, Ride.dt60, 0.4) == -1.0 && hold(false, false, true, Ride.dt60, 0.4) == -1.0
// A tap: `held` counts up while down and resets on release.
expect held(true, Ride.dt60, held(true, Ride.dt60, -1.0)) == Ride.dt60 && held(false, Ride.dt60, 0.3) == -1.0

// Landing: left first; a blocked left falls to the right, then behind; with
// no room anywhere, None.
let roomAt = (open: List<Input.point3>) => (spot: Input.point3) =>
  if open |> List.any((o) => o == spot) then Option.Some({ spot with y: 1.0 }) else Option.None
let left = { x: side, y: 0.0, z: 0.0 }
let right = { x: -side, y: 0.0, z: 0.0 }
let back = { x: 0.0, y: 0.0, z: -behind }
expect landing(roomAt([left, right, back])) == Option.Some({ left with y: 1.0 })
expect landing(roomAt([right, back])) == Option.Some({ right with y: 1.0 })
expect landing(roomAt([back])) == Option.Some({ back with y: 1.0 })
expect landing(roomAt([])) == Option.None
// Facing the bike from each spot.
expect Math.abs(toward(side, 0.0, 0.0, 0.0) + Math.pi / 2.0) < 0.000001 && toward(0.0, -behind, 0.0, 0.0) == 0.0
