// Hold: one hand grabbing, holding and releasing a prop, and the throw
// estimate. Pure and rig-local (+X right, +Y up, -Z forward, as XR samples
// arrive), so it is tested below with no world, and a prop held while the
// rig moves rides along with it.
//
// A prop is a pose. The world reaches in through one function,
// `cast(from, to)`: where a sweep of the prop from one pose toward another
// first touches something (`Physics.sweep`), or none.

type Side = | Left | Right

/// A sweep's first touch: the fraction of the way along it, and the surface
/// normal (pointing back at the prop).
type Hit = { fraction: float, normal: Input.point3 }

/// A hand's grab controls. `armed` is false after tracking loss, a broken
/// grip or a station reset, until the hand is seen open again, so a squeeze
/// held through any of those never grabs. `hover` is the prop in its reach.
type Hand<'prop> = { tracked: bool, squeezed: bool, armed: bool, hover: Option.t<'prop> }

/// One step's accepted pose, and when.
type Sample = { t: float, pose: Input.pose }

/// A prop in a hand: where it sits in the grip's frame, where it actually is
/// (`accepted`: the sweep can stop it short of the hand), and the accepted
/// history the throw is estimated from.
type Held = { side: Side, offset: Input.pose, accepted: Input.pose, history: List<Sample> }

/// What a held step did: still held, let go with a velocity and a spin
/// (axis × rad/s), or dropped where it was.
type Outcome =
  | Holding(held: Held)
  | Released(pose: Input.pose, velocity: Input.point3, spin: Input.point3)
  | Broke(pose: Input.pose)

let pressAt = 0.6
let releaseAt = 0.4
// Reach is measured from the grip to the prop's surface.
let reach = 0.12
let reachHysteresis = 0.04
// A hand this near a prop's authored grip, pointing within this of the way
// it does, snaps to it.
let snapRadius = 0.05
let snapTurn = Math.pi / 4.0
// The hand this far off where it holds the prop (blocked by a wall) lets
// go; or turned this far: past a quarter turn the shorter way round from the
// stuck pose would swing back through the wall.
let breakDistance = 0.25
let breakTurn = Math.pi / 2.0
let windowSeconds = 0.06
let maxSpeed = 12.0
let maxSpin = 30.0
// A step longer (or a turn larger) than this is a discontinuity, not motion:
// 30 m/s, 60 rad/s at 60 Hz.
let jumpLimit = 0.5
let jumpTurn = 1.0
// A blocked prop stops this far short of the hit, at its farthest point.
let skin = 0.004
let historyCap = 8.0

let zero = Quat.v3(0.0, 0.0, 0.0)
let origin: Input.pose = { position: zero, orientation: Quat.identity }
let idle = { tracked: false, squeezed: false, armed: false, hover: Option.None }

let squeezed = (was: bool, value: float): bool =>
  if was then value > releaseAt else value >= pressAt

expect squeezed(false, 0.6) && not squeezed(false, 0.59)
expect squeezed(true, 0.41) && not squeezed(true, 0.4)

let hovering = (was: bool, gap: float): bool =>
  if was then gap <= reach + reachHysteresis else gap <= reach

expect hovering(false, 0.12) && not hovering(false, 0.13)
expect hovering(true, 0.15) && not hovering(true, 0.17)

/// Advance a hand by one sample: its new state, and whether this step is a
/// fresh squeeze edge that may grab.
let stepHand = (h: Hand<'p>, tracked: bool, squeeze: float): (Hand<'p>, bool) =>
  if not tracked then ({ h with tracked: false, armed: false, hover: Option.None }, false)
  else
    let s = squeezed(h.squeezed, squeeze) in
    ({ h with tracked: true, squeezed: s, armed: h.armed || not s }, s && not h.squeezed && h.armed)

/// Disarm a hand until it is seen open again.
let disarm = (h: Hand<'p>): Hand<'p> => { h with armed: false, hover: Option.None }

let release = (h: Hand<float>) => stepHand(h, true, 0.0)
let press = (h: Hand<float>) => stepHand(h, true, 1.0)

// A press is an edge once; holding it is not.
expect (let (h, _) = release(idle) in let (h2, edge) = press(h) in edge && not (let (_, again) = press(h2) in again))
// Tracking lost while open, back already squeezing: no grab until it opens.
expect (
  let (h, _) = release(idle) in
  let (lost, _) = stepHand(h, false, 0.0) in
  let (back, edge) = press(lost) in
  let (h2, _) = release(back) in
  let (_, edge2) = press(h2) in
  not edge && edge2
)
// A disarmed (broken or reset) hand still squeezing never re-grabs.
expect (let (h, _) = release(idle) in let (held, _) = press(h) in not (let (_, e) = press(disarm(held)) in e))

/// The hand to grab with: of the hands that just pressed while in reach, the
/// one nearest the prop (`gap`: grip to surface), the right hand on a tie.
type Reacher = { side: Side, edge: bool, hover: bool, gap: float }

let pick = (left: Reacher, right: Reacher): Option.t<Side> =>
  // `sortBy` is stable: listing the right hand first wins it the tie.
  [right, left]
    |> List.filter((r) => r.edge && r.hover)
    |> List.sortBy((r) => r.gap)
    |> List.head
    |> Option.map((r) => r.side)

let reacher = (side, edge, hover, gap): Reacher => { side: side, edge: edge, hover: hover, gap: gap }
expect pick(reacher(Left, true, true, 0.05), reacher(Right, true, true, 0.02)) == Option.Some(Right)
expect pick(reacher(Left, true, true, 0.01), reacher(Right, true, true, 0.02)) == Option.Some(Left)
expect pick(reacher(Left, true, true, 0.03), reacher(Right, true, true, 0.03)) == Option.Some(Right)
expect pick(reacher(Left, true, true, 0.05), reacher(Right, false, true, 0.0)) == Option.Some(Left)
expect pick(reacher(Left, true, false, 0.05), reacher(Right, false, true, 0.0)) == Option.None

// --- poses ---

/// `b` placed in `a`'s frame.
let compose = (a: Input.pose, b: Input.pose): Input.pose => {
  position: Quat.add(a.position, Quat.rotate(a.orientation, b.position)),
  orientation: Quat.mul(a.orientation, b.orientation)
}

/// `b` seen from `a`'s frame: `compose(a, relative(a, b))` is `b`.
let relative = (a: Input.pose, b: Input.pose): Input.pose => {
  position: Quat.rotate(Quat.conj(a.orientation), Quat.sub(b.position, a.position)),
  orientation: Quat.mul(Quat.conj(a.orientation), b.orientation)
}

let inverse = (a: Input.pose): Input.pose => relative(a, origin)

/// Part of the way from `a` to `b` as the sweep moves: steadily, turning at
/// a constant rate.
let interpolate = (a: Input.pose, b: Input.pose, t: float): Input.pose => {
  position: Quat.lerp(a.position, b.position, t),
  orientation: Quat.slerp(a.orientation, b.orientation, t)
}

let at = (p: Input.point3, q: Input.quaternion): Input.pose => { position: p, orientation: q }
let near3 = (a: Input.point3, b: Input.point3): bool => Quat.length(Quat.sub(a, b)) < 0.0001
let near = (a: Input.pose, b: Input.pose): bool =>
  near3(a.position, b.position) && Quat.angle(a.orientation, b.orientation) < 0.0001

let tilted = at(Quat.v3(1.0, 2.0, 3.0), Quat.mul(Quat.yaw(0.7), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.4)))
let elsewhere = at(Quat.v3(-1.0, 0.5, 2.0), Quat.mul(Quat.yaw(-2.2), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 1.1)))
// An offset composes back to the pose it was taken from, and inverts.
expect near(compose(tilted, relative(tilted, elsewhere)), elsewhere)
expect near(compose(inverse(tilted), tilted), origin) && near(compose(tilted, inverse(tilted)), origin)

// --- the throw estimate ---

/// A sample of something's rate of change: a point, and when.
type Timed = { t: float, p: Input.point3 }

/// Add a sample, starting over after a jump or a sharp turn; at most `historyCap`.
let push = (s: Sample, history: List<Sample>): List<Sample> =>
  match List.last(history) with
  | Option.Some(prev) =>
    if Quat.length(Quat.sub(s.pose.position, prev.pose.position)) > jumpLimit
      || Quat.angle(prev.pose.orientation, s.pose.orientation) > jumpTurn then [s]
    else
      let all = history |> List.append([s]) in
      List.drop(Math.max(0.0, List.length(all) - historyCap), all)
  | Option.None => [s]

let capped = (cap: float, v: Input.point3): Input.point3 =>
  let rate = Quat.length(v) in
  if rate > cap then Quat.scale(cap / rate, v) else v

/// The least-squares rate of change of the samples inside the window ending
/// at the newest. Fewer than two samples estimate zero.
let slope = (history: List<Timed>): Input.point3 =>
  match List.last(history) with
  | Option.None => zero
  | Option.Some(newest) =>
    let recent = history |> List.filter((s) => newest.t - s.t <= windowSeconds + 0.000001) in
    let n = List.length(recent) in
    if n < 2.0 then zero
    else
      let tm = (recent |> List.fold((a, s) => a + s.t, 0.0)) / n in
      let pm = Quat.scale(1.0 / n, recent |> List.fold((a, s) => Quat.add(a, s.p), zero)) in
      let stt = recent |> List.fold((a, s) => a + (s.t - tm) * (s.t - tm), 0.0) in
      let stp = recent |> List.fold((a, s) => Quat.add(a, Quat.scale(s.t - tm, Quat.sub(s.p, pm))), zero) in
      if stt <= 0.0 then zero else Quat.scale(1.0 / stt, stp)

/// The throw's velocity: the windowed slope of the prop's centre, capped.
let velocity = (history: List<Sample>): Input.point3 =>
  capped(maxSpeed, slope(history |> List.map((s) => { t: s.t, p: s.pose.position })))

/// The throw's spin: the windowed slope of each pose's turn away from the
/// newest, capped.
let spin = (history: List<Sample>): Input.point3 =>
  match List.last(history) with
  | Option.None => zero
  | Option.Some(newest) =>
    capped(maxSpin, slope(history |> List.map((s) =>
      { t: s.t, p: Quat.log(Quat.mul(s.pose.orientation, Quat.conj(newest.pose.orientation))) })))

let sampleAt = (v: Input.point3, t: float): Sample => { t: t, pose: at(Quat.scale(t, v), Quat.identity) }
let steady = (v: Input.point3, n: float) =>
  List.range(n) |> List.fold((h, i) => push(sampleAt(v, i / 60.0), h), [])

// A steady hand estimates its velocity exactly.
expect near3(velocity(steady(Quat.v3(1.0, 2.0, -3.0), 10.0)), Quat.v3(1.0, 2.0, -3.0))
// Only the window counts: a hand that stopped dead is still.
expect (
  let moving = steady(Quat.v3(0.0, 0.0, -4.0), 10.0) in
  let last = List.last(moving) |> Option.map((s) => s.pose) |> Option.defaultValue(origin) in
  let stopped = List.range(4.0) |> List.fold((h, i) => push({ t: (10.0 + i) / 60.0, pose: last }, h), moving) in
  near3(velocity(stopped), zero)
)
// Too fast is capped, keeping the direction.
expect (let v = velocity(steady(Quat.v3(0.0, 0.0, -20.0), 6.0)) in Math.abs(Quat.length(v) - maxSpeed) < 0.0001 && v.z < 0.0)
// A jump or a sharp turn starts the history over: one sample estimates nothing.
expect (
  let h = steady(Quat.v3(0.0, 1.0, 0.0), 5.0) |> push(sampleAt(Quat.v3(18.0, 0.0, 0.0), 6.0 / 60.0)) in
  List.length(h) == 1.0 && near3(velocity(h), zero)
)
expect (
  let a = { t: 0.0, pose: origin } in
  let b = { t: 1.0 / 60.0, pose: at(zero, Quat.yaw(1.2)) } in
  List.length(push(b, [a])) == 1.0 && List.length(push(b, [{ a with pose: at(zero, Quat.yaw(0.4)) }])) == 2.0
)
expect near3(velocity([]), zero) && near3(spin([]), zero)
expect List.length(steady(Quat.v3(1.0, 0.0, 0.0), 20.0)) == historyCap

// --- moving a held prop ---

// Toward `to`, stopping `skin` short of the first hit at the prop's farthest
// point (`radius` from its centre); the hit's normal when blocked.
let clampTo = (cast: (Input.pose, Input.pose) => Option.t<Hit>, radius: float, from: Input.pose, to: Input.pose) =>
  let travel = Quat.length(Quat.sub(to.position, from.position)) + Quat.angle(from.orientation, to.orientation) * radius in
  if travel < 0.000001 then (to, Option.None)
  else
    match cast(from, to) with
    | Option.None => (to, Option.None)
    | Option.Some(h) => (interpolate(from, to, Math.max(0.0, h.fraction - skin / travel)), Option.Some(h.normal))

/// Sweep from `from` toward `to`; on a hit, slide the rest of the way along
/// the surface once, still turning toward `to`. Motion out of a surface is
/// never blocked (the sweep ignores separating motion), so a prop pushed into
/// a wall comes back out.
let move = (cast: (Input.pose, Input.pose) => Option.t<Hit>, radius: float, from: Input.pose, to: Input.pose): Input.pose =>
  let (stop, blocked) = clampTo(cast, radius, from, to) in
  match blocked with
  | Option.None => stop
  | Option.Some(n) =>
    let rest = Quat.sub(to.position, stop.position) in
    let into = Quat.dot(rest, n) in
    let slide = if into < 0.0 then Quat.sub(rest, Quat.scale(into, n)) else rest in
    let (slid, _) = clampTo(cast, radius, stop, at(Quat.add(stop.position, slide), to.orientation)) in
    slid

// A test world: a wall filling x >= 1, swept by a ball of radius 0.06.
let ballCast = (from: Input.pose, to: Input.pose): Option.t<Hit> =>
  let face = 1.0 - 0.06 in
  let d = to.position.x - from.position.x in
  if d <= 0.0 || to.position.x < face then Option.None
  else Option.Some({ fraction: Math.max(0.0, (face - from.position.x) / d), normal: Quat.v3(-1.0, 0.0, 0.0) })
let open3 = (from: Input.pose, to: Input.pose): Option.t<Hit> => Option.None
let p = (x: float, y: float, z: float): Input.pose => at(Quat.v3(x, y, z), Quat.identity)

expect near(move(open3, 0.06, origin, p(1.0, 1.0, 0.0)), p(1.0, 1.0, 0.0))
// Into the wall: stops at its face (less the skin), and slides along it.
expect near(move(ballCast, 0.06, origin, p(2.0, 0.0, 0.0)), p(0.94 - skin, 0.0, 0.0))
// (The skin is measured along the motion, so a slanted stop sits a little nearer.)
expect (let q = move(ballCast, 0.06, p(0.5, 0.0, 0.0), p(1.5, 0.5, 0.0)).position in q.x > 0.935 && q.x < 0.94 && Math.abs(q.y - 0.5) < 0.0001)
// Out of it again, unhindered.
expect near(move(ballCast, 0.06, p(0.936, 0.0, 0.0), p(0.5, 0.0, 0.0)), p(0.5, 0.0, 0.0))

// --- grab, hold, release ---

/// Whether a hand snaps to an authored grip placed at `at`: within
/// `snapRadius` of it, pointing within `snapTurn` of the way it does.
let snaps = (hand: Input.pose, at: Input.pose): bool =>
  let ahead = (q: Input.quaternion) => Quat.rotate(q, Quat.v3(0.0, 0.0, -1.0)) in
  Quat.length(Quat.sub(hand.position, at.position)) <= snapRadius
    && Quat.dot(ahead(hand.orientation), ahead(at.orientation)) >= Math.cos(snapTurn)

/// Take a prop at `item` in `grip`: held exactly where it is, unless the hand
/// `snaps` to the prop's authored grip (the hand's pose in the prop's frame).
/// Its history starts with the first held step, so a snap is no throw.
let grab = (side: Side, grip: Input.pose, item: Input.pose, authored: Option.t<Input.pose>): Held =>
  let snapped = authored |> Option.filter((a) => snaps(grip, compose(item, a))) in
  { side: side,
    offset: snapped |> Option.map(inverse) |> Option.defaultValue(relative(grip, item)),
    accepted: item,
    history: [] }

/// The other hand takes a held prop where it is (no snap), keeping its history.
let handOff = (side: Side, grip: Input.pose, held: Held): Held =>
  { held with side: side, offset: relative(grip, held.accepted) }

/// Where the hand is on the prop as it is: the grip the accepted pose implies.
let gripOn = (held: Held): Input.pose => compose(held.accepted, inverse(held.offset))

/// One step of a held prop. Tracking lost (`grip` None): it stays where it is
/// and forgets its history. The hand `breakDistance` or `breakTurn` off its
/// place on the blocked prop: dropped where it was, even if the hand opened.
/// Opened: thrown with the estimated velocity and spin from where it last
/// was held, where its body is: the release step then carries it on as the
/// hand did, rather than a step past it.
let step = (cast: (Input.pose, Input.pose) => Option.t<Hit>, radius: float, held: Held, grip: Option.t<Input.pose>, squeezed: bool, t: float): Outcome =>
  match grip with
  | Option.None => Holding({ held with history: [] })
  | Option.Some(g) =>
    let wanted = compose(g, held.offset) in
    let accepted = move(cast, radius, held.accepted, wanted) in
    let history = push({ t: t, pose: accepted }, held.history) in
    let next = { held with accepted: accepted, history: history } in
    let off = Quat.length(Quat.sub(g.position, gripOn(next).position)) > breakDistance
      || Quat.angle(wanted.orientation, accepted.orientation) > breakTurn in
    if off then Broke(held.accepted)
    else if not squeezed then Released(held.accepted, velocity(history), spin(history))
    else Holding(next)

// Taken anywhere, turned any way: the prop stays exactly where it was, and
// the hand held still holds it there.
expect (
  let h = grab(Right, elsewhere, tilted, Option.None) in
  near(compose(elsewhere, h.offset), tilted)
    && (match step(open3, 0.1, h, Option.Some(elsewhere), true, 1.0 / 60.0) with
        | Holding(next) => near(next.accepted, tilted)
        | _ => false)
)
// An authored grip snaps within `snapRadius` of it, and only there.
// Rolled about the way it points, it still snaps; pointing the other way, not.
let neck = at(Quat.v3(0.0, 0.1, 0.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), Math.pi / 2.0))
let toNeck = (dy: float, q: Input.quaternion) => at(Quat.v3(0.02, 0.1 + dy, 0.0), q)
let rolled = Quat.mul(neck.orientation, Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 2.0))
expect (
  near(grab(Right, toNeck(0.0, rolled), origin, Option.Some(neck)).offset, inverse(neck))
    && near(grab(Right, toNeck(snapRadius - 0.025, rolled), origin, Option.Some(neck)).offset, inverse(neck))
    && near(grab(Right, toNeck(snapRadius, rolled), origin, Option.Some(neck)).offset, relative(toNeck(snapRadius, rolled), origin))
)
expect (
  let tipped = (a: float) => Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), a), neck.orientation) in
  let down = toNeck(0.0, Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.0 - Math.pi / 2.0)) in
  near(grab(Right, toNeck(0.0, tipped(0.7)), origin, Option.Some(neck)).offset, inverse(neck))
    && near(grab(Right, toNeck(0.0, tipped(0.9)), origin, Option.Some(neck)).offset, relative(toNeck(0.0, tipped(0.9)), origin))
    && near(grab(Right, down, origin, Option.Some(neck)).offset, relative(down, origin))
)
// Snapped by a still hand and let go at once: dropped, not thrown.
expect (
  let hand = toNeck(0.02, rolled) in
  let h = grab(Right, hand, origin, Option.Some(neck)) in
  match step(open3, 0.1, h, Option.Some(hand), true, 1.0 / 60.0) with
  | Holding(snapped) =>
    near(gripOn(snapped), hand)
      && (match step(open3, 0.1, snapped, Option.Some(hand), false, 2.0 / 60.0) with
          | Released(_, v, w) => near3(v, zero) && near3(w, zero)
          | _ => false)
  | _ => false
)
// Handed to the other hand mid-hold, it stays where it was, history and all,
// and follows the new hand.
expect (
  let h = grab(Right, elsewhere, tilted, Option.None) in
  let lefty = at(Quat.v3(0.3, 2.1, 2.9), Quat.yaw(1.4)) in
  let given = handOff(Left, lefty, h) in
  given.side == Left && near(compose(lefty, given.offset), tilted) && given.history == h.history
    && near(gripOn(given), lefty)
    && (match step(open3, 0.1, given, Option.Some(at(Quat.add(lefty.position, Quat.v3(0.0, 0.1, 0.0)), lefty.orientation)), true, 1.0 / 60.0) with
        | Holding(next) => near3(next.accepted.position, Quat.add(tilted.position, Quat.v3(0.0, 0.1, 0.0)))
        | _ => false)
)

// Swing a held ball at `v` for `n` steps, then let go on the next.
let swing = (cast, v: Input.point3, n: float, squeezed: bool, grip: Option.t<Input.pose>) =>
  let held = grab(Right, origin, p(0.0, 0.0, -0.05), Option.None) in
  let moved = List.range(n) |> List.fold((h, i) => (
    match step(cast, 0.06, h, Option.Some(at(Quat.scale((i + 1.0) / 60.0, v), Quat.identity)), true, (i + 1.0) / 60.0) with
    | Holding(next) => next
    | _ => h), held) in
  step(cast, 0.06, moved, grip, squeezed, (n + 1.0) / 60.0)

let throwAt = (v: Input.point3) => Option.Some(at(Quat.scale(11.0 / 60.0, v), Quat.identity))

// A swing releases at the hand's velocity, from where it was last held.
expect (
  match swing(open3, Quat.v3(0.0, 1.0, -3.0), 10.0, false, throwAt(Quat.v3(0.0, 1.0, -3.0))) with
  | Released(at, v, w) => near3(v, Quat.v3(0.0, 1.0, -3.0)) && near3(at.position, Quat.v3(0.0, 10.0 / 60.0, -0.05 - 0.5)) && near3(w, zero)
  | _ => false
)
// Tracking lost mid-swing never throws: the prop freezes and forgets its history...
expect (
  match swing(open3, Quat.v3(0.0, 0.0, -3.0), 10.0, true, Option.None) with
  | Holding(h) => List.length(h.history) == 0.0
  | _ => false
)
// ...so opening the hand on its return drops it.
expect (
  match swing(open3, Quat.v3(0.0, 0.0, -3.0), 10.0, true, Option.None) with
  | Holding(h) =>
    (match step(open3, 0.06, h, throwAt(Quat.v3(0.0, 0.0, -3.0)), false, 1.0) with
     | Released(_, v, _) => near3(v, zero)
     | _ => false)
  | _ => false
)
// A hand pushed through a wall leaves the ball at the wall, and breaks its
// grip once it is farther than `breakDistance` beyond.
expect (
  let held = grab(Right, p(0.9, 0.0, 0.0), p(0.9, 0.0, 0.0), Option.None) in
  match step(ballCast, 0.06, held, Option.Some(p(1.1, 0.0, 0.0)), true, 1.0 / 60.0) with
  | Holding(h) =>
    near3(h.accepted.position, Quat.v3(0.94 - skin, 0.0, 0.0))
      && (match step(ballCast, 0.06, h, Option.Some(p(1.3, 0.0, 0.0)), true, 2.0 / 60.0) with
          | Broke(at) => near(at, h.accepted)
          | _ => false)
  | _ => false
)
// Opening the hand on the same step it passes `breakDistance` is a break,
// not a throw.
expect (
  let held = grab(Right, p(0.9, 0.0, 0.0), p(0.9, 0.0, 0.0), Option.None) in
  match step(ballCast, 0.06, held, Option.Some(p(1.3, 0.0, 0.0)), false, 1.0 / 60.0) with
  | Broke(_) => true
  | _ => false
)

// --- a long prop turned into a wall filling x <= -0.6 ---
// A 0.7 m stick along its pose's -Z, held by its near end.

let stickLength = 0.7
let wallX = 0.0 - 0.6
let farX = (s: Input.pose) => Quat.add(s.position, Quat.rotate(s.orientation, Quat.v3(0.0, 0.0, 0.0 - stickLength / 2.0))).x

// The first touch of the far end, by bisection (a stand-in for the sweep).
let stickCast = (from: Input.pose, to: Input.pose): Option.t<Hit> =>
  let into = (t: float) => farX(interpolate(from, to, t)) < wallX in
  if not into(1.0) then Option.None
  else
    let (lo, _) = List.range(30.0) |> List.fold((r, i) =>
      let (lo, hi) = r in
      let mid = (lo + hi) / 2.0 in
      if into(mid) then (lo, mid) else (mid, hi), (0.0, 1.0)) in
    Option.Some({ fraction: lo, normal: Quat.v3(1.0, 0.0, 0.0) })

let hand = Quat.v3(-0.3, 1.2, -0.3)
let rack = p(-0.3, 1.2, -0.6)
let gripAt = (turn: float) => Option.Some(at(hand, Quat.yaw(turn)))
let turned = (held: Held, turns: List<float>): List<Held> =>
  turns |> List.fold((hs, a) =>
    let h = List.last(hs) |> Option.defaultValue(held) in
    (match step(stickCast, stickLength / 2.0, h, gripAt(a), true, 0.0) with
     | Holding(next) => hs |> List.append([next])
     | _ => hs), [])
let start = grab(Right, at(hand, Quat.identity), rack, Option.None)
// Turning the wrist left a quarter turn, then back.
let turnedIn = turned(start, List.range(31.0) |> List.map((i) => i * Math.pi / 60.0))
let back = turned(List.last(turnedIn) |> Option.defaultValue(start), List.range(31.0) |> List.map((i) => (30.0 - i) * Math.pi / 60.0))

// Open, it would reach x = -0.3 - 0.65: it never passes the wall...
expect turnedIn |> List.all((h) => farX(h.accepted) >= wallX - 0.0001)
// ...and ends up stopped against it.
expect (List.last(turnedIn) |> Option.map((h) => farX(h.accepted) - wallX < 0.01) |> Option.defaultValue(false))
// Turned back, it follows the hand again.
expect (List.last(back) |> Option.map((h) => near(h.accepted, compose(at(hand, Quat.identity), h.offset))) |> Option.defaultValue(false))
// Stopped by the wall, the hand turned on but still on the grip: still held.
expect List.length(turnedIn) == 31.0
// Turned on past a quarter turn beyond where it stopped: it drops.
expect (
  let stuck = List.last(turnedIn) |> Option.defaultValue(start) in
  match step(stickCast, stickLength / 2.0, stuck, gripAt(Math.pi * 0.75), true, 0.0) with
  | Broke(_) => true
  | _ => false
)

// A wrist flicked about +X, the stick held by its near end: it leaves with
// that spin, its centre at the speed it swings at (|w| |r|).
let flick = (rate: float, n: float): Outcome =>
  let gripAt = (i: float) => Option.Some(at(hand, Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), rate * i / 60.0))) in
  let held = List.range(n) |> List.fold((h, i) =>
    (match step(open3, stickLength / 2.0, h, gripAt(i + 1.0), true, (i + 1.0) / 60.0) with
     | Holding(next) => next
     | _ => h), start) in
  step(open3, stickLength / 2.0, held, gripAt(n + 1.0), false, (n + 1.0) / 60.0)
expect (
  match flick(6.0, 8.0) with
  | Released(s, v, w) =>
    let r = Quat.sub(s.position, hand) in
    near3(w, Quat.v3(6.0, 0.0, 0.0)) && Math.abs(Quat.length(v) / (6.0 * Quat.length(r)) - 1.0) < 0.02
  | _ => false
)
// Too fast is capped, about the same axis.
expect (match flick(50.0, 8.0) with | Released(_, _, w) => Math.abs(w.x - maxSpin) < 0.0001 | _ => false)

// --- what the hand is drawn at ---

/// Far enough off the real hand to show it: 2 cm, or 10 degrees.
let ghostDistance = 0.02
let ghostTurn = 10.0 * Math.pi / 180.0
let ghostedBy = (scale: float, tracked: Input.pose, drawn: Input.pose): bool =>
  Quat.length(Quat.sub(tracked.position, drawn.position)) > scale * ghostDistance
    || Quat.angle(tracked.orientation, drawn.orientation) > scale * ghostTurn
let ghosted = (tracked: Input.pose, drawn: Input.pose): bool => ghostedBy(1.0, tracked, drawn)

expect not ghosted(p(0.0, 0.0, 0.0), p(0.019, 0.0, 0.0)) && ghosted(p(0.0, 0.0, 0.0), p(0.0, 0.021, 0.0))
expect not ghosted(origin, at(zero, Quat.yaw(0.17))) && ghosted(origin, at(zero, Quat.yaw(0.18)))
// The wall-stopped stick puts the drawn hand off the real one.
expect (List.last(turnedIn) |> Option.map((h) => ghosted(at(hand, Quat.yaw(Math.pi / 2.0)), gripOn(h))) |> Option.defaultValue(false))
