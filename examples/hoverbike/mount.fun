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

// With a headset, a fresh squeeze within `reach` of a grip grabs it. The grips
// ride above a standing walker's head, so the reach is generous.
let reach = 0.6 // m

let distance = (a: Input.point3, b: Input.point3) => Bars.distance(a, b)

// A fresh squeeze: squeezing now, not the step before, tracked both times.
let fresh = (before: Bars.Hand, now: Bars.Hand) =>
  now.tracked && before.tracked && now.squeezing && not before.squeezing

// Whether a mount request goes through: on foot, not fading, near the seat;
// and for a headset hand's grab (`hand` ≠ 0), calibrated, with that hand
// `toGrip` from its grip.
let eligible = (onFoot: bool, fading: bool, near: bool, hand: float, calibrated: bool, toGrip: float) =>
  onFoot && not fading && near && (hand == 0.0 || (calibrated && toGrip < reach))

// ---- The fade ------------------------------------------------------------

// What happens at the black: mount (the hand whose grab did it: 1 left, -1
// right, 0 none), or set the walker down at a capsule centre, facing `yaw`.
type Action =
  | GetOn(hand: float)
  | GetOff(x: float, y: float, z: float, yaw: float)

// `t` is the time since the fade began, negative when idle.
type Fade = { t: float, action: Action }
let clear: Fade = { t: -1.0, action: GetOn(0.0) }
let start = (a: Action): Fade => { t: 0.0, action: a }
let fading = (f: Fade) => f.t >= 0.0

// One step: the next fade, and whether its action happens on this step.
let step = (dt: float, f: Fade): (Fade, bool) =>
  if f.t < 0.0 then (f, false)
  else
    let (t, black) = Bars.fadeStep(dt, f.t) in
    ({ f with t: t }, black)

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
// reach; neither while riding or mid-fade.
expect eligible(true, false, true, 0.0, false, 9.0) && eligible(true, false, true, 1.0, true, 0.5)
expect not eligible(true, false, false, 0.0, true, 0.0) && not eligible(false, false, true, 0.0, true, 0.0)
  && not eligible(true, true, true, 0.0, true, 0.0)
expect not eligible(true, false, true, -1.0, true, 0.7) && not eligible(true, false, true, 1.0, false, 0.1)

// The fade: its action lands on the step it is fully black, six steps in at
// 60 Hz; it is clear again after 15 (to a float's width), and never black twice.
let run = (n: float, f: Fade) =>
  List.range(n) |> List.fold((acc, i) =>
    let (g, blacks, levels) = acc in
    let (h, b) = step(Ride.dt60, g) in
    (h, (if b then [i, ..blacks] else blacks), [Bars.fadeAt(h.t), ..levels]), (f, [], []))
expect (
  let (f, blacks, levels) = run(20.0, start(GetOn(1.0))) in
  blacks == [5.0] && List.nth(14.0, levels) == Option.Some(1.0) && not fading(f)
    && List.head(levels) == Option.Some(0.0) && List.length(levels |> List.filter((l) => l > 0.001)) == 14.0
)
expect (let (f, b) = step(Ride.dt60, clear) in not b && f == clear)

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
