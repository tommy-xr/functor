// Stick: a long prop held rigidly in one hand. Each step it moves from where
// it is toward where the hand holds it, turning on the way, and stops at what
// it hits: turned into a wall it stays at the wall, turned back it follows the
// hand again. Rig-local poses (+X right, +Y up, -Z forward); the stick lies
// along its pose's -Z, the way a controller points.
//
// The world reaches in through one function, `cast(from, to)`: the fraction
// of the way from one pose to the other at which the stick first touches
// something (`Physics.sweep`), or none.
//
// Deliberately minimal: held where it was grabbed, no break, and let go it
// goes back to its rack.

let length = 0.7
let thickness = 0.035
// A blocked stick stops this far short of the hit, at its farthest point.
let skin = 0.004

/// A stick in a hand: where it sits in the grip's frame, and where it
/// actually is (the sweep can stop it short of the hand).
type Held = { side: Hold.Side, offset: Input.pose, accepted: Input.pose }

type Outcome =
  | Holding(held: Held)
  | Released

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

let grab = (side: Hold.Side, grip: Input.pose, stick: Input.pose): Held =>
  { side: side, offset: relative(grip, stick), accepted: stick }

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
/// is. Opened: let go.
let step = (cast: (Input.pose, Input.pose) => Option.t<float>, held: Held, grip: Option.t<Input.pose>, squeezed: bool): Outcome =>
  match grip with
  | Option.None => Holding(held)
  | Option.Some(g) =>
    if not squeezed then Released
    else Holding({ held with accepted: move(cast, held.accepted, compose(g, held.offset)) })

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
    (match step(wallCast, h, gripAt(a), true) with
     | Holding(next) => hs |> List.append([next])
     | Released => hs), [])
let start = grab(Hold.Right, at(hand, Quat.identity), rack)
// Turning the wrist left a quarter turn, then back.
let swing = turned(start, List.range(31.0) |> List.map((i) => i * Math.pi / 60.0))
let back = turned(List.last(swing) |> Option.defaultValue(start), List.range(31.0) |> List.map((i) => (30.0 - i) * Math.pi / 60.0))

// Open, it would reach x = -0.3 - 0.65: it never passes the wall...
expect swing |> List.all((h) => farX(h.accepted) >= wallX - 0.0001)
// ...and ends up stopped against it.
expect (List.last(swing) |> Option.map((h) => farX(h.accepted) - wallX < 0.01) |> Option.defaultValue(false))
// Turned back, it follows the hand again.
expect (List.last(back) |> Option.map((h) => near(h.accepted, compose(at(hand, Quat.identity), h.offset))) |> Option.defaultValue(false))
// Open: let go. Lost: stays put.
expect (match step(wallCast, start, gripAt(0.0), false) with | Released => true | _ => false)
expect (match step(wallCast, start, Option.None, true) with | Holding(h) => near(h.accepted, rack) | _ => false)
