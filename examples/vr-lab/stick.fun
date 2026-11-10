// Stick: a long prop held rigidly in one hand. Each step it moves from where
// it is toward where the hand holds it, turning on the way, and stops at what
// it hits: turned into a wall it stays at the wall, turned back it follows the
// hand again. Let go, it is thrown with the velocity and spin of its last
// accepted poses. Rig-local poses (+X right, +Y up, -Z forward); the stick
// lies along its pose's -Z, the way a controller points.
//
// The world reaches in through one function, `cast(from, to)`: the fraction
// of the way from one pose to the other at which the stick first touches
// something (`Physics.sweep`), or none.

let length = 0.7
let thickness = 0.035
// A blocked stick stops this far short of the hit, at its farthest point.
let skin = 0.004
// Held further than this off the hand, it drops: past it the shorter way
// round from the stuck pose would swing back through the wall.
let breakTurn = Math.pi / 2.0
// A throw's spin is capped here, as its speed is at `Hold.maxSpeed`.
let maxSpin = 30.0
// A turn over this (radians) in one step, 60 rad/s, is a snap (unstuck from a wall), not
// the wrist: the history starts over, as at `Hold.jumpLimit`.
let jumpTurn = 1.0

/// One step's accepted pose, and when.
type Sample = { t: float, pose: Input.pose }

/// A stick in a hand: where it sits in the grip's frame, where it actually
/// is (the sweep can stop it short of the hand), and the accepted history the
/// throw is estimated from.
type Held = { side: Hold.Side, offset: Input.pose, accepted: Input.pose, history: List<Sample> }

/// What a held step did: still held, let go with a velocity and a spin
/// (rig-local axis × rad/s), or dropped where it stopped.
type Outcome =
  | Holding(held: Held)
  | Released(pose: Input.pose, velocity: Input.point3, spin: Input.point3)
  | Broke(pose: Input.pose)

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

/// Part of the way from `a` to `b` as the sweep moves: steadily, turning at
/// a constant rate.
let interpolate = (a: Input.pose, b: Input.pose, t: float): Input.pose => {
  position: Quat.lerp(a.position, b.position, t),
  orientation: Quat.slerp(a.orientation, b.orientation, t)
}

/// The stick's (near, far) ends: half its length either side of the centre.
let ends = (p: Input.pose): (Input.point3, Input.point3) =>
  let half = Quat.rotate(p.orientation, Quat.v3(0.0, 0.0, 0.0 - length / 2.0)) in
  (Quat.sub(p.position, half), Quat.add(p.position, half))

/// From a point to the stick's surface.
let gap = (q: Input.point3, p: Input.pose): float =>
  let (a, b) = ends(p) in
  let ab = Quat.sub(b, a) in
  let t = Quat.dot(Quat.sub(q, a), ab) / Quat.dot(ab, ab) |> Math.clamp(0.0, 1.0) in
  Quat.length(Quat.sub(q, Quat.lerp(a, b, t))) - thickness / 2.0

let grab = (side: Hold.Side, grip: Input.pose, stick: Input.pose, t: float): Held =>
  { side: side, offset: relative(grip, stick), accepted: stick, history: [{ t: t, pose: stick }] }

/// Add a sample, starting over after a jump or a snap.
let push = (s: Sample, history: List<Sample>): List<Sample> =>
  Hold.pushWhere((a: Sample, b: Sample) =>
    Quat.length(Quat.sub(b.pose.position, a.pose.position)) > Hold.jumpLimit
      || Quat.angle(a.pose.orientation, b.pose.orientation) > jumpTurn, s, history)

/// The throw's velocity: `Hold.estimate` of the stick's centre.
let velocity = (history: List<Sample>): Input.point3 =>
  Hold.estimate(history |> List.map((s) => { t: s.t, p: s.pose.position }))

/// The throw's spin: the least-squares rate of each pose's turn away from the
/// newest, over the same window as the velocity, capped.
let spin = (history: List<Sample>): Input.point3 =>
  match List.last(history) with
  | Option.None => Hold.zero
  | Option.Some(newest) =>
    let turns = history |> List.map((s) =>
      { t: s.t, p: Quat.log(Quat.mul(s.pose.orientation, Quat.conj(newest.pose.orientation))) }) in
    let w = Hold.slope(turns) in
    let rate = Quat.length(w) in
    if rate > maxSpin then Quat.scale(maxSpin / rate, w) else w

/// Toward `to`, stopping `skin` short of the first hit.
let move = (cast: (Input.pose, Input.pose) => Option.t<float>, from: Input.pose, to: Input.pose): Input.pose =>
  // How far the stick's farthest point travels.
  let travel = Quat.length(Quat.sub(to.position, from.position)) + Quat.angle(from.orientation, to.orientation) * length / 2.0 in
  if travel < 0.000001 then to
  else
    match cast(from, to) with
    | Option.None => to
    | Option.Some(f) => interpolate(from, to, Math.max(0.0, f - skin / travel))

/// One step of a held stick. Tracking lost (`grip` None): it stays where it
/// is and forgets its history. Blocked `breakTurn` or `Hold.breakDistance`
/// off the hand: dropped where it stopped, even if the hand opened. Opened:
/// thrown with the estimated velocity and spin from where it last was held,
/// where its body is: the release step then carries it on as the hand did,
/// rather than a step past it.
let step = (cast: (Input.pose, Input.pose) => Option.t<float>, held: Held, grip: Option.t<Input.pose>, squeezed: bool, t: float): Outcome =>
  match grip with
  | Option.None => Holding({ held with history: [] })
  | Option.Some(g) =>
    let wanted = compose(g, held.offset) in
    let accepted = move(cast, held.accepted, wanted) in
    let history = push({ t: t, pose: accepted }, held.history) in
    // Where the hand would be on the stick as it is: a turn about the grip
    // moves this not at all.
    let gripOn = compose(accepted, relative(held.offset, { position: Hold.zero, orientation: Quat.identity })) in
    let off = Quat.length(Quat.sub(g.position, gripOn.position)) > Hold.breakDistance
      || Quat.angle(wanted.orientation, accepted.orientation) > breakTurn in
    if off then Broke(accepted)
    else if not squeezed then Released(held.accepted, velocity(history), spin(history))
    else Holding({ held with accepted: accepted, history: history })

// --- tests, against a wall filling x <= -0.6 ---

let wallX = 0.0 - 0.6
let farX = (p: Input.pose) => let (_, far) = ends(p) in far.x

// The first touch of the far end, by bisection (a stand-in for the sweep).
let wallCast = (from: Input.pose, to: Input.pose): Option.t<float> =>
  let into = (t: float) => farX(interpolate(from, to, t)) < wallX in
  if not into(1.0) then Option.None
  else
    let (lo, _) = List.range(30.0) |> List.fold((r, i) =>
      let (lo, hi) = r in
      let mid = (lo + hi) / 2.0 in
      if into(mid) then (lo, mid) else (mid, hi), (0.0, 1.0)) in
    Option.Some(lo)

let at = (p: Input.point3, q: Input.quaternion): Input.pose => { position: p, orientation: q }
let near = (a: Input.pose, b: Input.pose): bool =>
  Hold.near3(a.position, b.position) && Quat.angle(a.orientation, b.orientation) < 0.0001

expect (
  let a = at(Quat.v3(1.0, 2.0, 3.0), Quat.yaw(0.7)) in
  let b = at(Quat.v3(-1.0, 0.5, 2.0), Quat.mul(Quat.yaw(0.2), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.4))) in
  near(compose(a, relative(a, b)), b)
)
expect Math.abs(gap(Quat.v3(0.0, 0.1, 0.0), at(Quat.v3(0.0, 0.0, -0.2), Quat.identity)) - (0.1 - thickness / 2.0)) < 0.0001
expect Math.abs(gap(Quat.v3(0.0, 0.0, -0.8), at(Quat.v3(0.0, 0.0, -0.2), Quat.identity)) - (0.25 - thickness / 2.0)) < 0.0001

// Held by its near end, 0.3 m from the wall and pointing ahead.
let hand = Quat.v3(-0.3, 1.2, -0.3)
let rack = at(Quat.v3(-0.3, 1.2, -0.6), Quat.identity)
let gripAt = (turn: float) => Option.Some(at(hand, Quat.yaw(turn)))
let turned = (held: Held, turns: List<float>): List<Held> =>
  turns |> List.fold((hs, a) =>
    let h = List.last(hs) |> Option.defaultValue(held) in
    (match step(wallCast, h, gripAt(a), true, 0.0) with
     | Holding(next) => hs |> List.append([next])
     | _ => hs), [])
let start = grab(Hold.Right, at(hand, Quat.identity), rack, 0.0)
// Turning the wrist left a quarter turn, then back.
let swing = turned(start, List.range(31.0) |> List.map((i) => i * Math.pi / 60.0))
let back = turned(List.last(swing) |> Option.defaultValue(start), List.range(31.0) |> List.map((i) => (30.0 - i) * Math.pi / 60.0))

// Open, it would reach x = -0.3 - 0.65: it never passes the wall...
expect swing |> List.all((h) => farX(h.accepted) >= wallX - 0.0001)
// ...and ends up stopped against it.
expect (List.last(swing) |> Option.map((h) => farX(h.accepted) - wallX < 0.01) |> Option.defaultValue(false))
// Turned back, it follows the hand again.
expect (List.last(back) |> Option.map((h) => near(h.accepted, compose(at(hand, Quat.identity), h.offset))) |> Option.defaultValue(false))
// Stopped by the wall, the hand turned on but still on the grip: still held.
expect List.length(swing) == 31.0
// Turned on past a quarter turn beyond where it stopped: it drops.
expect (
  let stuck = List.last(swing) |> Option.defaultValue(start) in
  match step(wallCast, stuck, gripAt(Math.pi * 0.75), true, 0.0) with
  | Broke(_) => true
  | _ => false
)
// Open: let go where it is, still. Lost: stays put and forgets its history.
expect (
  match step(wallCast, start, gripAt(0.0), false, 1.0 / 60.0) with
  | Released(p, v, w) => near(p, rack) && Hold.near3(v, Hold.zero) && Hold.near3(w, Hold.zero)
  | _ => false
)
expect (match step(wallCast, start, Option.None, true, 0.0) with | Holding(h) => near(h.accepted, rack) && List.length(h.history) == 0.0 | _ => false)

// --- the throw ---

// A wrist flicked about +X at 6 rad/s, the stick held by its near end: it
// leaves with that spin, its centre at the speed it swings at (|w| |r|).
let flick = (rate: float, n: float): Outcome =>
  let open3 = (from: Input.pose, to: Input.pose): Option.t<float> => Option.None in
  let gripAt = (i: float) => Option.Some(at(hand, Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), rate * i / 60.0))) in
  let held = List.range(n) |> List.fold((h, i) =>
    (match step(open3, h, gripAt(i + 1.0), true, (i + 1.0) / 60.0) with
     | Holding(next) => next
     | _ => h), start) in
  step(open3, held, gripAt(n + 1.0), false, (n + 1.0) / 60.0)
expect (
  match flick(6.0, 8.0) with
  | Released(p, v, w) =>
    let r = Quat.sub(p.position, hand) in
    Hold.near3(w, Quat.v3(6.0, 0.0, 0.0)) && Math.abs(Quat.length(v) / (6.0 * Quat.length(r)) - 1.0) < 0.02
  | _ => false
)
// Too fast is capped, about the same axis.
expect (match flick(50.0, 8.0) with | Released(_, _, w) => Math.abs(w.x - maxSpin) < 0.0001 | _ => false)
// A snap starts the history over.
expect (
  let a = { t: 0.0, pose: rack } in
  let b = { t: 1.0 / 60.0, pose: at(rack.position, Quat.yaw(1.2)) } in
  List.length(push(b, [a])) == 1.0 && List.length(push(b, [{ a with pose: at(rack.position, Quat.yaw(0.4)) }])) == 2.0
)
// One sample (the history forgotten on tracking loss) is no throw: a drop.
expect (
  match step(wallCast, { start with history: [] }, gripAt(0.2), false, 1.0 / 60.0) with
  | Released(_, v, w) => Hold.near3(v, Hold.zero) && Hold.near3(w, Hold.zero)
  | _ => false
)
