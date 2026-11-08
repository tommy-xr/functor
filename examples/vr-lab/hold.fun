// Hold: one hand grabbing, holding and releasing an item, and the throw
// estimate. Pure and rig-local (+X right, +Y up, -Z forward, as XR samples
// arrive), so it is tested below with no world, and an item held while the
// rig moves rides along with it.
//
// The world reaches in through one function, `cast(from, to)`: where a
// sweep of the item from `from` toward `to` first hits something.

type Side = | Left | Right

/// One step's sample of where the held item actually is (rig-local), and when.
type Sample = { t: float, p: Input.point3 }

/// The first hit of a sweep: its distance along the segment and the surface
/// normal (pointing back at the item).
type Hit = { hit: bool, distance: float, normal: Input.point3 }

/// A hand's grab controls. `armed` is false after tracking loss, a broken
/// grip or a station reset, until the hand is seen open again, so a squeeze
/// held through any of those never grabs.
type Hand = { tracked: bool, squeezed: bool, armed: bool, hover: bool }

/// An item in a hand: where it sits in the grip's frame, where it actually
/// is (`accepted`: the sweep can stop it short of the hand), and the
/// accepted history the throw is estimated from.
type Held = { side: Side, offset: Input.point3, accepted: Input.point3, history: List<Sample> }

/// What a held step did.
type Outcome =
  | Holding(held: Held)
  | Released(at: Input.point3, velocity: Input.point3)
  | Broke(at: Input.point3)

let pressAt = 0.6
let releaseAt = 0.4
// Reach is measured from the grip to the item's surface.
let reach = 0.12
let reachHysteresis = 0.04
// The hand this far from the item it holds (blocked by a wall) lets go.
let breakDistance = 0.25
let windowSeconds = 0.06
let maxSpeed = 12.0
// A step longer than this is a discontinuity, not motion (30 m/s at 60 Hz).
let jumpLimit = 0.5
let skin = 0.004
let historyCap = 8.0

let zero = Quat.v3(0.0, 0.0, 0.0)
let idle: Hand = { tracked: false, squeezed: false, armed: false, hover: false }

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
let stepHand = (h: Hand, tracked: bool, squeeze: float): (Hand, bool) =>
  if not tracked then ({ h with tracked: false, armed: false, hover: false }, false)
  else
    let s = squeezed(h.squeezed, squeeze) in
    ({ h with tracked: true, squeezed: s, armed: h.armed || not s }, s && not h.squeezed && h.armed)

/// Disarm a hand until it is seen open again.
let disarm = (h: Hand): Hand => { h with armed: false, hover: false }

let release = (h: Hand) => stepHand(h, true, 0.0)
let press = (h: Hand) => stepHand(h, true, 1.0)

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
/// one nearest the item (`gap`: grip to surface), the right hand on a tie.
type Reacher = { side: Side, edge: bool, hover: bool, gap: float }

let pick = (left: Reacher, right: Reacher): Option.t<Side> =>
  [right, left]
    |> List.filter((r) => r.edge && r.hover)
    |> List.fold((best, r) => (
         match best with
         | Option.Some(b) => if r.gap < b.gap then Option.Some(r) else best
         | Option.None => Option.Some(r)), Option.None)
    |> Option.map((r) => r.side)

let reacher = (side, edge, hover, gap): Reacher => { side: side, edge: edge, hover: hover, gap: gap }
expect pick(reacher(Left, true, true, 0.05), reacher(Right, true, true, 0.02)) == Option.Some(Right)
expect pick(reacher(Left, true, true, 0.01), reacher(Right, true, true, 0.02)) == Option.Some(Left)
expect pick(reacher(Left, true, true, 0.03), reacher(Right, true, true, 0.03)) == Option.Some(Right)
expect pick(reacher(Left, true, true, 0.05), reacher(Right, false, true, 0.0)) == Option.Some(Left)
expect pick(reacher(Left, true, false, 0.05), reacher(Right, false, true, 0.0)) == Option.None

// --- the throw estimate ---

/// Add a sample, starting over after a discontinuity (a jump no hand makes).
let push = (s: Sample, history: List<Sample>): List<Sample> =>
  match List.last(history) with
  | Option.Some(prev) =>
    if Quat.length(Quat.sub(s.p, prev.p)) > jumpLimit then [s]
    else
      let all = history |> List.append([s]) in
      List.drop(Math.max(0.0, List.length(all) - historyCap), all)
  | Option.None => [s]

let capped = (v: Input.point3): Input.point3 =>
  let speed = Quat.length(v) in
  if speed > maxSpeed then Quat.scale(maxSpeed / speed, v) else v

/// The least-squares velocity of the samples inside the window ending at the
/// newest, capped. Fewer than two samples estimate zero: a drop.
let estimate = (history: List<Sample>): Input.point3 =>
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
      if stt <= 0.0 then zero else capped(Quat.scale(1.0 / stt, stp))

let near3 = (a: Input.point3, b: Input.point3): bool => Quat.length(Quat.sub(a, b)) < 0.0001
let sampleAt = (v: Input.point3, t: float): Sample => { t: t, p: Quat.scale(t, v) }
let steady = (v: Input.point3, n: float) =>
  List.range(n) |> List.fold((h, i) => push(sampleAt(v, i / 60.0), h), [])

// A steady hand estimates its velocity exactly.
expect near3(estimate(steady(Quat.v3(1.0, 2.0, -3.0), 10.0)), Quat.v3(1.0, 2.0, -3.0))
// Only the window counts: a hand that stopped dead is still.
expect (
  let moving = steady(Quat.v3(0.0, 0.0, -4.0), 10.0) in
  let last = List.last(moving) |> Option.map((s) => s.p) |> Option.defaultValue(zero) in
  let stopped = List.range(4.0) |> List.fold((h, i) => push({ t: (10.0 + i) / 60.0, p: last }, h), moving) in
  near3(estimate(stopped), zero)
)
// Too fast is capped, keeping the direction.
expect (let v = estimate(steady(Quat.v3(0.0, 0.0, -20.0), 6.0)) in Math.abs(Quat.length(v) - maxSpeed) < 0.0001 && v.z < 0.0)
// A jump starts the history over: one sample estimates nothing.
expect (
  let h = steady(Quat.v3(0.0, 1.0, 0.0), 5.0) |> push({ t: 6.0 / 60.0, p: Quat.v3(3.0, 0.0, 0.0) }) in
  List.length(h) == 1.0 && near3(estimate(h), zero)
)
expect near3(estimate([]), zero)
expect List.length(steady(Quat.v3(1.0, 0.0, 0.0), 20.0)) == historyCap

// --- moving a held item ---

// Toward `to`, stopping `skin` short of the first hit; the hit's normal when blocked.
let clampTo = (cast: (Input.point3, Input.point3) => Hit, from: Input.point3, to: Input.point3) =>
  let len = Quat.length(Quat.sub(to, from)) in
  if len < 0.000001 then (to, Option.None)
  else
    let hit = cast(from, to) in
    if hit.hit then
      (Quat.lerp(from, to, Math.max(0.0, hit.distance - skin) / len), Option.Some(hit.normal))
    else (to, Option.None)

/// Sweep from `from` toward `to`; on a hit, slide the rest of the way along
/// the surface once. Motion out of a surface is never blocked (the cast
/// ignores separating motion), so an item pushed into a wall comes back out.
let sweep = (cast: (Input.point3, Input.point3) => Hit, from: Input.point3, to: Input.point3): Input.point3 =>
  let (stop, blocked) = clampTo(cast, from, to) in
  match blocked with
  | Option.None => stop
  | Option.Some(n) =>
    let rest = Quat.sub(to, stop) in
    let into = Quat.dot(rest, n) in
    let slide = if into < 0.0 then Quat.sub(rest, Quat.scale(into, n)) else rest in
    let (slid, _) = clampTo(cast, stop, Quat.add(stop, slide)) in
    slid

// A test world: a wall filling x >= 1, swept by a ball of radius 0.06.
let wallCast = (from: Input.point3, to: Input.point3): Hit =>
  let face = 1.0 - 0.06 in
  let d = Quat.sub(to, from) in
  let len = Quat.length(d) in
  if d.x <= 0.0 || to.x < face then { hit: false, distance: 0.0, normal: zero }
  else { hit: true, distance: Math.max(0.0, (face - from.x) / d.x * len), normal: Quat.v3(-1.0, 0.0, 0.0) }
let open3 = (from: Input.point3, to: Input.point3): Hit => { hit: false, distance: 0.0, normal: zero }

expect near3(sweep(open3, zero, Quat.v3(1.0, 1.0, 0.0)), Quat.v3(1.0, 1.0, 0.0))
// Into the wall: stops at its face (less the skin), and slides along it.
expect near3(sweep(wallCast, zero, Quat.v3(2.0, 0.0, 0.0)), Quat.v3(0.94 - skin, 0.0, 0.0))
// (The skin is measured along the motion, so a slanted stop sits a little nearer.)
expect (let p = sweep(wallCast, Quat.v3(0.5, 0.0, 0.0), Quat.v3(1.5, 0.5, 0.0)) in p.x > 0.935 && p.x < 0.94 && Math.abs(p.y - 0.5) < 0.0001)
// Out of it again, unhindered.
expect near3(sweep(wallCast, Quat.v3(0.936, 0.0, 0.0), Quat.v3(0.5, 0.0, 0.0)), Quat.v3(0.5, 0.0, 0.0))

// --- grab, hold, release ---

/// Where a grip holds an item at `offset` (grip frame).
let target = (grip: Input.pose, offset: Input.point3): Input.point3 =>
  Quat.add(grip.position, Quat.rotate(grip.orientation, offset))

let grab = (side: Side, grip: Input.pose, item: Input.point3, t: float): Held => {
  side: side,
  offset: Quat.rotate(Quat.conj(grip.orientation), Quat.sub(item, grip.position)),
  accepted: item,
  history: [{ t: t, p: item }]
}

/// One step of a held item. Tracking lost (`grip` None): it freezes where it
/// is and forgets its history, so the throw restarts from the hand's return.
/// Opened: released at the swept pose with the estimated velocity. Held
/// farther than `breakDistance` from the hand: dropped where it stopped.
let step = (cast: (Input.point3, Input.point3) => Hit, held: Held, grip: Option.t<Input.pose>, hand: Hand, t: float): Outcome =>
  match grip with
  | Option.None => Holding({ held with history: [] })
  | Option.Some(g) =>
    let wanted = target(g, held.offset) in
    let accepted = sweep(cast, held.accepted, wanted) in
    let history = push({ t: t, p: accepted }, held.history) in
    if not hand.squeezed then Released(accepted, estimate(history))
    else if Quat.length(Quat.sub(wanted, accepted)) > breakDistance then Broke(accepted)
    else Holding({ held with accepted: accepted, history: history })

let pose = (p: Input.point3): Input.pose => { position: p, orientation: Quat.identity }
let closed: Hand = { tracked: true, squeezed: true, armed: true, hover: false }
let opened: Hand = { closed with squeezed: false }

// Swing a held item at `v` for `n` steps, then let go on the next.
let swing = (cast, v: Input.point3, n: float, hand: Hand, grip: Option.t<Input.pose>) =>
  let held = grab(Right, pose(zero), Quat.v3(0.0, 0.0, -0.05), 0.0) in
  let moved = List.range(n) |> List.fold((h, i) => (
    match step(cast, h, Option.Some(pose(Quat.scale((i + 1.0) / 60.0, v))), closed, (i + 1.0) / 60.0) with
    | Holding(next) => next
    | _ => h), held) in
  step(cast, moved, grip, hand, (n + 1.0) / 60.0)

let throwAt = (v: Input.point3) => Option.Some(pose(Quat.scale(11.0 / 60.0, v)))

// A swing releases at the hand's velocity, from where the item is.
expect (
  match swing(open3, Quat.v3(0.0, 1.0, -3.0), 10.0, opened, throwAt(Quat.v3(0.0, 1.0, -3.0))) with
  | Released(at, v) => near3(v, Quat.v3(0.0, 1.0, -3.0)) && near3(at, Quat.v3(0.0, 11.0 / 60.0, -0.05 - 0.55))
  | _ => false
)
// Tracking lost mid-swing never throws: the item freezes and forgets its history...
expect (
  match swing(open3, Quat.v3(0.0, 0.0, -3.0), 10.0, closed, Option.None) with
  | Holding(h) => List.length(h.history) == 0.0
  | _ => false
)
// ...so opening the hand on its return drops it.
expect (
  match swing(open3, Quat.v3(0.0, 0.0, -3.0), 10.0, closed, Option.None) with
  | Holding(h) =>
    (match step(open3, h, throwAt(Quat.v3(0.0, 0.0, -3.0)), opened, 1.0) with
     | Released(_, v) => near3(v, zero)
     | _ => false)
  | _ => false
)
// A hand pushed through a wall leaves the item at the wall, and breaks its
// grip once it is farther than `breakDistance` beyond.
expect (
  let held = grab(Right, pose(Quat.v3(0.9, 0.0, 0.0)), Quat.v3(0.9, 0.0, 0.0), 0.0) in
  match step(wallCast, held, Option.Some(pose(Quat.v3(1.1, 0.0, 0.0))), closed, 1.0 / 60.0) with
  | Holding(h) =>
    near3(h.accepted, Quat.v3(0.94 - skin, 0.0, 0.0))
      && (match step(wallCast, h, Option.Some(pose(Quat.v3(1.3, 0.0, 0.0))), closed, 2.0 / 60.0) with
          | Broke(at) => near3(at, h.accepted)
          | _ => false)
  | _ => false
)
