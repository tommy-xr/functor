// Grip: a grip fitter. Given the solids a hand holds and where it holds
// them, it closes each finger until it touches, and measures a grip (fitted
// or preset). Pure and offline: nothing in the step calls it yet.
//
// Everything is in the hand's root frame (the grip's, `Hand.local`'s). What
// is held is a few solids, each a signed distance field; the hand is its
// capsules (`Hand.capsules`). A capsule's gap is the least distance from its
// axis to a solid, less its radius (negative: how deep it is in). Every solid
// is convex, so that distance is convex along the axis, and a golden-section
// search finds its least.

// --- solids ---

/// A solid's shape about its pose's origin: a box (its half extents), a
/// sphere, and a capsule or cylinder along its local Y (radius, half length).
type Form = | Box(half: Input.point3) | Sphere(radius: float) | Capsule(radius: float, half: float) | Cylinder(radius: float, half: float)

/// A solid, posed in some frame (an item's, or the hand's root).
type Solid = { at: Input.pose, form: Form }

let solid = (at: Input.pose, form: Form): Solid => { at: at, form: form }

// The signed distance to a box from how far a point is past each face pair
// (negative: inside them).
let past = (x: float, y: float, z: float): float =>
  Quat.length(Quat.v3(Math.max(x, 0.0), Math.max(y, 0.0), Math.max(z, 0.0))) + Math.min(Math.max(x, Math.max(y, z)), 0.0)

/// The signed distance from `q` to a solid: negative inside.
let sdf = (s: Solid, q: Input.point3): float =>
  // `Hold.localPoint` without its pose: this is the fit's innermost call.
  let p = Quat.rotate(Quat.conj(s.at.orientation), Quat.sub(q, s.at.position)) in
  match s.form with
  | Box(h) => past(Math.abs(p.x) - h.x, Math.abs(p.y) - h.y, Math.abs(p.z) - h.z)
  | Sphere(r) => Quat.length(p) - r
  | Capsule(r, h) => Quat.length(Quat.v3(p.x, p.y - Math.clamp(0.0 - h, h, p.y), p.z)) - r
  | Cylinder(r, h) =>
    let radial = Math.sqrt(p.x * p.x + p.z * p.z) - r in
    let axial = Math.abs(p.y) - h in
    Quat.length(Quat.v3(Math.max(radial, 0.0), Math.max(axial, 0.0), 0.0)) + Math.min(Math.max(radial, axial), 0.0)

// From a solid's origin to its farthest point.
let bound = (f: Form): float =>
  match f with
  | Box(h) => Quat.length(h)
  | Sphere(r) => r
  | Capsule(r, h) => r + h
  | Cylinder(r, h) => Math.sqrt(r * r + h * h)

let near = (a: float, b: float): bool => Math.abs(a - b) < 0.000001
let tilted = Hold.at(Quat.v3(0.1, 0.2, 0.3), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), Math.pi / 2.0))

// A box: off a face, off an edge, off a corner, inside; turned, in its frame.
expect (
  let b = solid(Hold.origin, Box(Quat.v3(0.01, 0.02, 0.03))) in
  near(sdf(b, Quat.v3(0.05, 0.0, 0.0)), 0.04) && near(sdf(b, Quat.v3(0.04, 0.06, 0.0)), 0.05)
    && near(sdf(b, Quat.v3(0.02, 0.04, 0.05)), 0.03) && near(sdf(b, Quat.v3(0.0, 0.015, 0.0)), 0.0 - 0.005)
    && near(sdf({ b with at: tilted }, Quat.v3(0.1, 0.25, 0.3)), 0.04)
)
// A sphere: outside, inside.
expect (let s = solid(Hold.p(0.0, 1.0, 0.0), Sphere(0.06)) in near(sdf(s, Quat.v3(0.0, 1.1, 0.0)), 0.04) && near(sdf(s, Quat.v3(0.02, 1.0, 0.0)), 0.0 - 0.04))
// A capsule: off its side, past its cap, on its axis inside.
expect (
  let c = solid(Hold.origin, Capsule(0.01, 0.05)) in
  near(sdf(c, Quat.v3(0.03, 0.04, 0.0)), 0.02) && near(sdf(c, Quat.v3(0.0, 0.08, 0.0)), 0.02) && near(sdf(c, Quat.v3(0.0, 0.0 - 0.055, 0.0)), 0.0 - 0.005)
    && near(sdf({ c with at: tilted }, Quat.v3(0.1, 0.23, 0.3)), 0.02)
)
// A cylinder: off its side, past its end, off its rim, inside nearer the side.
expect (
  let c = solid(Hold.origin, Cylinder(0.02, 0.05)) in
  near(sdf(c, Quat.v3(0.0, 0.0, 0.05)), 0.03) && near(sdf(c, Quat.v3(0.01, 0.07, 0.0)), 0.02)
    && near(sdf(c, Quat.v3(0.05, 0.09, 0.0)), 0.05) && near(sdf(c, Quat.v3(0.015, 0.0, 0.0)), 0.0 - 0.005)
)

// --- a capsule's gap ---

// The least of a convex `f` on 0..1: golden-section, its bracket 0.3% of the
// way after 12 steps (a 4 cm bone's least to 0.12 mm); exact at the ends.
let golden = 0.381966
let goldenSteps = 12.0
let least = (f: (float) => float): float =>
  let (_, _, _, fc, _, fd) = List.range(goldenSteps) |> List.fold((s, _) =>
    let (lo, hi, c, fc, d, fd) = s in
    if fc < fd then let c2 = lo + golden * (d - lo) in (lo, d, c2, f(c2), c, fc)
    else let d2 = hi - golden * (hi - c) in (c, hi, d, fd, d2, f(d2)),
    (0.0, 1.0, golden, f(golden), 1.0 - golden, f(1.0 - golden))) in
  Math.min(Math.min(fc, fd), Math.min(f(0.0), f(1.0)))

let toSegment = (a: Input.point3, b: Input.point3, q: Input.point3): float =>
  let ab = Quat.sub(b, a) in
  Quat.length(Quat.sub(q, Quat.lerp(a, b, Math.clamp01(Quat.dot(Quat.sub(q, a), ab) / Quat.dot(ab, ab)))))

/// The gap from a hand capsule (its axis `a` to `b`, `radius` round) to the
/// nearest of `solids`, or `limit` if none is nearer. A solid that can't beat
/// the nearest so far is skipped: by its bounding sphere, then by its
/// distance from the axis's middle less half the axis (no point of the axis
/// is further from the middle). So a small `limit` (only asking whether it
/// touches) skips most.
let gapUnder = (limit: float, solids: List<Solid>, a: Input.point3, b: Input.point3, radius: float): float =>
  let mid = Quat.lerp(a, b, 0.5) in
  let half = Quat.length(Quat.sub(b, a)) / 2.0 in
  solids |> List.fold((best, s) =>
    if toSegment(a, b, s.at.position) - bound(s.form) - radius >= best then best
    else if sdf(s, mid) - half - radius >= best then best
    else Math.min(best, least((t) => sdf(s, Quat.lerp(a, b, t))) - radius), limit)

/// A capsule's gap (`gapUnder`), up to 1 m.
let gapOf = (solids: List<Solid>, a: Input.point3, b: Input.point3, radius: float): float => gapUnder(1.0, solids, a, b, radius)

/// A solid moved by `by`.
let shift = (by: Input.point3, s: Solid): Solid => { s with at: Hold.at(Quat.add(s.at.position, by), s.at.orientation) }

/// Solids posed in an item's frame, seen from a hand whose root is at `hand`
/// in that frame.
let seenFrom = (hand: Input.pose, solids: List<Solid>): List<Solid> =>
  solids |> List.map((s) => { s with at: Hold.relative(hand, s.at) })

// --- the hand's digits ---
// A digit is the thumb (0) or a finger (1-4: index to little). Each has
// three joints, each moving one capsule and every capsule past it: a
// finger's MCP, PIP and DIP (its proximal, intermediate and distal bones'),
// the thumb's CMC, MCP and IP (its metacarpal, proximal and distal's). A
// digit closes by controls, each driving some of its joints (a rig): its
// pose's (`Hand.Bend`: a finger's curl the MCP, its tip the PIP and DIP; the
// thumb's curl the CMC and MCP, its tip the IP), or one per joint.

let fingers = [Hand.index, Hand.middle, Hand.ring, Hand.little]

let capOf = (j: float): Hand.Capsule => Hand.capsules |> List.find((c) => c.joint == j) |> Option.defaultValue({ joint: j, radius: 0.0, length: 0.0 })

/// A digit's three moving capsules, proximal first.
let moving = (d: float): List<Hand.Capsule> => [capOf(5.0 * d + 2.0), capOf(5.0 * d + 3.0), capOf(5.0 * d + 4.0)]

/// The palm: the four metacarpals' capsules (they don't move).
let palmCaps = [6.0, 11.0, 16.0, 21.0] |> List.map(capOf)

/// A rig: which control drives each of a digit's three joints, proximal
/// first. A finger's pose's, the thumb's, and one control per joint.
let fingerRig = [0.0, 1.0, 1.0]
let thumbRig = [0.0, 0.0, 1.0]
let jointRig = [0.0, 1.0, 2.0]
let rigOf = (d: float): List<float> => if d == 0.0 then thumbRig else fingerRig

/// The joints' curls a rig's controls (`c`) give.
let jointsOf = (rig: List<float>, c: List<float>): List<float> => rig |> List.map((i) => List.nth(i, c) |> Option.defaultValue(0.0))

/// A digit's moving joints in the wrist's frame, each joint's curl
/// (`c`: 0 straight to 1 a fist's flexion) its own; the thumb turned
/// `opposition`. Then its tip.
let chainOf = (d: float, c: List<float>, opposition: float): List<(Input.pose, Input.quaternion)> =>
  let k = (i: float) => List.nth(i, c) |> Option.defaultValue(0.0) in
  if d == 0.0 then Hand.thumbJoints(opposition, k(0.0), k(1.0), k(2.0))
  else Hand.fingerJoints(List.nth(d - 1.0, fingers) |> Option.defaultValue(Hand.index), k(0.0), k(1.0), k(2.0)) |> List.drop(1.0)

// A pose in the wrist's frame, in the root's (`Hand.local`'s).
let inRoot = (side: Hold.Side, p: Input.pose): Input.pose =>
  let r = Hold.compose(Hand.wristOnGrip, p) in
  if side == Hold.Left then Hand.mirror(r) else r

/// A digit's three moving capsules posed: each one's axis and radius.
let segmentsOf = (side: Hold.Side, d: float, c: List<float>, opposition: float): List<(Input.point3, Input.point3, float)> =>
  List.zip(moving(d), List.take(3.0, chainOf(d, c, opposition))) |> List.map((jc) =>
    let (j, cap) = jc in
    let (p, _) = j in
    let r = inRoot(side, p) in
    (r.position, Quat.add(r.position, Quat.rotate(r.orientation, Quat.v3(0.0, 0.0, 0.0 - cap.length))), cap.radius))

// Driven by its pose's rig, a digit's capsules are `Hand.local`'s, either
// hand.
expect (
  [Hold.Left, Hold.Right] |> List.all((side) =>
    let f = { Hand.power with thumb: { Hand.power.thumb with tip: 0.3 }, index: { curl: 0.4, tip: 0.8 }, little: Hand.uniform(0.9) } in
    let p = Hand.local(side, f) in
    [(0.0, [f.thumb.curl, f.thumb.tip]), (1.0, [0.4, 0.8]), (2.0, [f.middle.curl, f.middle.tip]), (4.0, [0.9, 0.9])] |> List.all((dc) =>
      let (d, c) = dc in
      List.zip(moving(d), segmentsOf(side, d, jointsOf(rigOf(d), c), f.thumb.opposition)) |> List.all((sc) =>
        let (s, cap) = sc in
        let (a, b, _) = s in
        let (ea, eb) = Hand.ends(p, cap) in
        Hold.near3(a, ea) && Hold.near3(b, eb))))
)

// --- closing a digit ---

/// How near a capsule stops short of what it touches: 1 mm.
let contactGap = 0.001
/// A capsule this near (or in) is touching, for the counts.
let touchGap = 0.002
// A closing joint steps by its digit's clearance over `sweep` (no point of a
// capsule's axis moves further than that a unit of curl: a finger's tip
// 0.24 m, the thumb's 0.11), so it never steps past a contact; at most 1/16,
// at least 1/256 (1 mm). Then it bisects the step that touched 8 times.
let sweep = 0.25
let marchStep = 1.0 / 16.0
let minStep = 1.0 / 256.0
let reach = contactGap + sweep * marchStep
let bisections = 8.0

/// A digit closed: its rig's controls' curls, and whether it started in what
/// it holds (`buried`: left at its start).
type Closed = { curls: List<float>, buried: bool }

/// How much deeper than it starts the thenar (the thumb's metacarpal, part of
/// the palm) may press into what it holds before it stops the thumb's curl:
/// flesh, 3 mm. So a thenar resting on an item lets the MCP close on.
let thenarGive = 0.003

/// Close a digit from `start` (every control's curl; 0 straight) on `solids`
/// by `rig`'s controls (`opposition`: the thumb's turn). Every control
/// closes together until a capsule comes within `contactGap`; its control and
/// every one before it stops, the controls past it close on, until each has
/// touched or reached a full curl. A phalanx already inside buries the
/// digit. The thenar stops the thumb's first control only `thenarGive`
/// deeper than it started.
let closeFrom = (start: float, solids: List<Solid>, side: Hold.Side, d: float, opposition: float, rig: List<float>): Closed =>
  let ctl = (i: float) => List.nth(i, rig) |> Option.defaultValue(0.0) in
  let controls = ctl(2.0) + 1.0 in
  let c0 = List.range(controls) |> List.map((_) => start) in
  // How far past `contactGap` the thenar's gap stops its control.
  let give =
    if d != 0.0 then 0.0
    else List.head(segmentsOf(side, d, jointsOf(rig, c0), opposition)) |> Option.map((s) => let (a, b, r) = s in
      contactGap - Math.min(contactGap, gapUnder(contactGap, solids, a, b, r)) + thenarGive) |> Option.defaultValue(0.0) in
  let off = (i: float) => if i == 0.0 then give else 0.0 in
  // The gaps (up to `limit`) of the capsules `keep` names, less their give;
  // 1 m the rest.
  let gapsOf = (limit: float, keep: (float) => bool, c: List<float>): List<float> =>
    segmentsOf(side, d, jointsOf(rig, c), opposition) |> List.indexedMap((i, s) => let (a, b, r) = s in if keep(i) then gapUnder(limit - off(i), solids, a, b, r) + off(i) else 1.0) in
  // The capsules control `j` and past it move.
  let gapsFrom = (limit: float, j: float, c: List<float>): List<float> => gapsOf(limit, (i) => ctl(i) >= j, c) in
  // The control of the farthest capsule within `contactGap`, if any.
  let within = (gaps: List<float>): Option.t<float> =>
    gaps |> List.indexedMap((i, g) => if g <= contactGap then i else 0.0 - 1.0) |> List.maximum |> Option.filter((i) => i >= 0.0) |> Option.map(ctl) in
  let touching = (j: float, c: List<float>): Option.t<float> => within(gapsFrom(touchGap, j, c)) in
  // Control `j` and past it at `s`.
  let from = (j: float, s: float, c: List<float>): List<float> => c |> List.indexedMap((i, x) => if i >= j then s else x) in
  let nearest = (gaps: List<float>) => List.minimum(gaps) |> Option.defaultValue(reach) in
  // The step that touched, if any: `(before, after)`.
  let march = (j: float, c: List<float>): Option.t<(float, float)> =>
    let (_, _, hit) = Loop.until((st) => let (s, _, hit) = st in Option.isSome(hit) || s >= 1.0,
      (st) => let (s, g, _) = st in
        let s2 = Math.min(1.0, s + Math.clamp(minStep, marchStep, (g - contactGap) / sweep)) in
        let gaps = gapsFrom(reach, j, from(j, s2, c)) in
        (s2, nearest(gaps), within(gaps) |> Option.map((_) => (s, s2))),
      (List.nth(j, c) |> Option.defaultValue(0.0), nearest(gapsFrom(reach, j, c)), Option.None)) in
    hit in
  // One phase: the joints from `j` on close together until a capsule
  // touches; the next phase starts past it (past the last, done).
  let phase = (st: (float, List<float>)): (float, List<float>) =>
    let (j, c) = st in
    match touching(j, c) with
    | Option.Some(i) => (i + 1.0, c)
    | Option.None =>
      (match march(j, c) with
       | Option.None => (controls, from(j, 1.0, c))
       | Option.Some(bracket) =>
         let (lo, hi) = List.range(bisections) |> List.fold((b, _) =>
           let (lo, hi) = b in
           let mid = (lo + hi) / 2.0 in
           if Option.isSome(touching(j, from(j, mid, c))) then (lo, mid) else (mid, hi), bracket) in
         (1.0 + (touching(j, from(j, hi, c)) |> Option.defaultValue(controls - 1.0)), from(j, lo, c))) in
  if gapsOf(touchGap, (i) => d != 0.0 || i > 0.0, c0) |> List.any((g) => g < 0.0) then { curls: c0, buried: true }
  else
    let (_, curls) = Loop.until((st) => let (j, _) = st in j >= controls, phase, (0.0, c0)) in
    { curls: curls, buried: false }

/// Close a straight digit (`closeFrom` 0).
let close = (solids: List<Solid>, side: Hold.Side, d: float, opposition: float, rig: List<float>): Closed => closeFrom(0.0, solids, side, d, opposition, rig)

// How far back past straight the thumb opens when straight is inside what it
// holds at every turn: hyperextended, a third of a fist's flexion.
let thumbOpen = 0.0 - 0.35

// The thumb's turns tried: at rest to opposed.
let oppositions = [0.0, 0.25, 0.5, 0.75, 1.0]

/// The thumb closed at its best turn: the one whose tip (its pad) ends
/// nearest the item (either side of it), the thenar's depth in it counted
/// ten times; then the one touching with the most capsules. Straight, it is
/// buried at every turn: it closes from open (`thumbOpen`); buried at every
/// turn from there too, at rest, buried.
let closeThumb = (solids: List<Solid>, side: Hold.Side): (Closed, float) =>
  match thumbFrom(0.0, solids, side) with
  | Option.Some(t) => t
  | Option.None => thumbFrom(thumbOpen, solids, side) |> Option.defaultValue(({ curls: [0.0, 0.0], buried: true }, 0.0))

// The thumb's best turn closed from `start`, if any isn't buried.
let thumbFrom = (start: float, solids: List<Solid>, side: Hold.Side): Option.t<(Closed, float)> =>
  let tried = oppositions |> List.map((o) =>
    let cl = closeFrom(start, solids, side, 0.0, o, thumbRig) in
    let gaps = segmentsOf(side, 0.0, jointsOf(thumbRig, cl.curls), o) |> List.map((s) => let (a, b, r) = s in gapOf(solids, a, b, r)) in
    let at = (i: float) => List.nth(i, gaps) |> Option.defaultValue(1.0) in
    let touches = gaps |> List.filter((g) => g <= touchGap) |> List.length in
    (cl, o, Math.abs(at(2.0)) + 10.0 * Math.max(0.0, 0.0 - at(0.0)) - 0.0001 * touches)) |> List.filter((t) => let (cl, _, _) = t in not cl.buried) in
  tried |> List.sortBy((t) => let (_, _, score) = t in score) |> List.head
    |> Option.map((t) => let (cl, o, _) = t in (cl, o))

// --- the palm ---

// Each hand flat, posed once: the palm doesn't move.
let flatLeft = Hand.local(Hold.Left, Hand.flat)
let flatRight = Hand.local(Hold.Right, Hand.flat)
let flatOf = (side: Hold.Side): Hand.Posed => if side == Hold.Left then flatLeft else flatRight

let palmJoint = (side: Hold.Side): Input.pose => Hand.jointOf(flatOf(side), Hand.palm)

/// The way the palm faces: out of it, away from the back of the hand.
let palmNormal = (side: Hold.Side): Input.point3 => Quat.rotate((palmJoint(side)).orientation, Quat.v3(0.0, 0.0 - 1.0, 0.0))

/// The palm's gap: its nearest metacarpal's.
let palmGap = (solids: List<Solid>, side: Hold.Side): float =>
  palmCaps |> List.map((c) => let (a, b) = Hand.ends(flatOf(side), c) in gapOf(solids, a, b, c.radius)) |> List.minimum |> Option.defaultValue(1.0)

// How far a palm push looks, either way.
let pushRange = 0.1

/// How far the hand would move along its palm's normal for the palm to come
/// within `contactGap` (negative: back out of the item), if it can within
/// `pushRange`. Sphere-traced: a gap is never more than the way to the
/// surface along the normal, so stepping by it never overshoots (backing
/// out of an item too), and converges on the first touch.
let palmPush = (solids: List<Solid>, side: Hold.Side): Option.t<float> =>
  let n = palmNormal(side) in
  let gapAt = (d: float) => palmGap(solids |> List.map((s) => shift(Quat.scale(0.0 - d, n), s)), side) - contactGap in
  let (d, g, _) = Loop.until((st) => let (d, g, k) = st in Math.abs(g) < 0.0002 || Math.abs(d) > pushRange || k >= 24.0,
    (st) => let (d, g, k) = st in (d + g, gapAt(d + g), k + 1.0), (0.0, gapAt(0.0), 0.0)) in
  if Math.abs(d) <= pushRange && Math.abs(g) < 0.0002 then Option.Some(d) else Option.None

// --- a fit ---

/// A fit: the finger pose (each digit closed by its curl and tip), each
/// digit's joints' curls (thumb first), which digits are buried, whether the
/// palm starts inside the item, and the palm push (`palmPush`).
type Fitted = { pose: Hand.FingerPose, curls: List<List<float>>, buried: List<bool>, palmBuried: bool, push: Option.t<float> }

// A finger stopped this near straight was stopped by a graze: a straight
// finger along an item's side, short of where it wraps. It closes from a
// curl instead (a real finger is already curling as it gets there): the
// first of `curledStarts` that doesn't start it inside.
let stuck = 0.1
let curledStarts = [Hand.relaxed, 0.3, 0.45]

/// A finger closed from straight, or from a curl (`curledStarts`) if
/// straight left it stuck (`stuck`).
let closeFinger = (solids: List<Solid>, side: Hold.Side, d: float): Closed =>
  let straight = close(solids, side, d, 0.0, fingerRig) in
  if straight.buried || (straight.curls |> List.any((c) => c >= stuck)) then straight
  else
    curledStarts |> List.map((s) => closeFrom(s, solids, side, d, 0.0, fingerRig)) |> List.find((c) => not c.buried)
      |> Option.defaultValue(straight)

/// Fit a hand to `solids` (in its root frame): each finger closed
/// (`closeFinger`), the thumb at its best turn.
let fit = (side: Hold.Side, solids: List<Solid>): Fitted => fitOn(side, solids, Option.None)

/// `fit`, the index closing on `index` alone if given (a trigger: it passes
/// the guard and frame it would graze on the way).
let fitOn = (side: Hold.Side, solids: List<Solid>, index: Option.t<List<Solid>>): Fitted =>
  let (thumb, opposition) = closeThumb(solids, side) in
  let on = (d: float) => if d == 1.0 then index |> Option.defaultValue(solids) else solids in
  let closed = [thumb, ..([1.0, 2.0, 3.0, 4.0] |> List.map((d) => closeFinger(on(d), side, d)))] in
  let k = (d: float, i: float) => List.nth(d, closed) |> Option.bind((c) => List.nth(i, c.curls)) |> Option.defaultValue(0.0) in
  let bend = (d: float): Hand.Bend => { curl: k(d, 0.0), tip: k(d, 1.0) } in
  { pose: { thumb: { curl: k(0.0, 0.0), tip: k(0.0, 1.0), opposition: opposition }, index: bend(1.0), middle: bend(2.0), ring: bend(3.0), little: bend(4.0) },
    curls: closed |> List.indexedMap((d, c) => jointsOf(rigOf(d), c.curls)),
    buried: closed |> List.map((c) => c.buried),
    palmBuried: palmGap(solids, side) < 0.0,
    push: palmPush(solids, side) }

// --- measuring a grip ---

/// A grip measured: each digit's nearest gap (thumb first, its phalanges;
/// negative, how deep it is in), the palm's and the thenar's, how many
/// capsules touch (`touchGap`), and its strain (`strain`).
type Metrics = { digits: List<float>, palm: float, thenar: float, contacts: float, strain: float }

/// How strained a pose is, 0 relaxed: the mean of how far each curl and tip
/// is out of 0.15-0.85, plus the mean of how far the ring finger's curl and
/// tip are each more than 0.3 from the middle's and the little's (they share
/// tendons; the index and thumb move freely).
let strain = (f: Hand.FingerPose): float =>
  let out = (c: float) => Math.max(0.0, 0.15 - c) + Math.max(0.0, c - 0.85) in
  let apart = (a: Hand.Bend, b: Hand.Bend) => Math.max(0.0, Math.abs(a.curl - b.curl) - 0.3) + Math.max(0.0, Math.abs(a.tip - b.tip) - 0.3) in
  let bends = [f.index, f.middle, f.ring, f.little] |> List.concatMap((b) => [b.curl, b.tip]) in
  ([f.thumb.curl, f.thumb.tip, ..bends] |> List.map(out) |> List.sum) / 10.0
    + (apart(f.ring, f.middle) + apart(f.ring, f.little)) / 4.0

/// Measure the hand posed `f` on `solids`.
let measure = (side: Hold.Side, solids: List<Solid>, f: Hand.FingerPose): Metrics =>
  let p = Hand.local(side, f) in
  let gaps = Hand.capsules |> List.map((c) => let (a, b) = Hand.ends(p, c) in (c.joint, gapOf(solids, a, b, c.radius))) in
  let nearest = (keep: (float) => bool) => gaps |> List.filter((g) => let (j, _) = g in keep(j)) |> List.map((g) => let (_, x) = g in x) |> List.minimum |> Option.defaultValue(1.0) in
  let digit = (d: float) => nearest((j) => j >= 5.0 * d + (if d == 0.0 then 3.0 else 2.0) && j <= 5.0 * d + 4.0) in
  { digits: [0.0, 1.0, 2.0, 3.0, 4.0] |> List.map(digit),
    palm: palmGap(solids, side),
    thenar: nearest((j) => j == 2.0),
    contacts: gaps |> List.filter((g) => let (_, x) = g in x <= touchGap) |> List.length,
    strain: strain(f) }

/// How far each digit's tip is, posed by the fit, from where closing each
/// joint on its own (`jointRig`; the thumb at the fit's turn) puts it: the
/// pose's error.
let tipError = (side: Hold.Side, solids: List<Solid>, fitted: Fitted): List<float> =>
  let tip = (d: float, c: List<float>, o: float) => List.last(chainOf(d, c, o)) |> Option.map((j) => let (p, _) = j in (inRoot(side, p)).position) |> Option.defaultValue(Hold.zero) in
  fitted.curls |> List.indexedMap((d, c) =>
    let o = if d == 0.0 then fitted.pose.thumb.opposition else 0.0 in
    Quat.length(Quat.sub(tip(d, c, o), tip(d, (close(solids, side, d, o, jointRig)).curls, o))))

// --- what the lab holds ---
// Each held item and site as solids in its own frame (its collider's: a
// prop's pose). A prop's collider is one bounding box, which for a gun holds
// the fist, so the solids are its drawn parts near a hand.

let boxAt = (at: Input.pose, size: Input.point3): Solid => solid(at, Box(Quat.scale(0.5, size)))

// A gun's part as drawn: its size and centre in the trigger hand's frame
// (`center`: the gun's collider's centre there), turned `rake` about X.
let part = (center: Input.point3, sx: float, sy: float, sz: float, x: float, y: float, z: float, rake: float): Solid =>
  boxAt(Hold.at(Quat.sub(Quat.v3(x, y, z), center), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), rake)), Quat.v3(sx, sy, sz))

/// A magazine's solids in its frame: the pistol's body and base plate, the
/// rifle's box; a shell's hull and brass head (its head back, +Z).
let magSolids = (k: Ammo.Kind): List<Solid> =>
  let e = Gun.magSize(k) in
  match k with
  | Ammo.PistolMag => [boxAt(Hold.p(0.0, 0.004, 0.0), Quat.v3(0.026, e.y - 0.008, 2.0 * Gun.magFace(k))), boxAt(Hold.p(0.0, 0.004 - e.y / 2.0, 0.0), Quat.v3(e.x, 0.008, e.z))]
  | Ammo.RifleMag => [boxAt(Hold.origin, e)]
  | Ammo.Shell => [boxAt(Hold.p(0.0, 0.0, 0.0 - 0.1 * e.z), Quat.v3(0.9 * e.x, 0.9 * e.y, 0.8 * e.z)), boxAt(Hold.p(0.0, 0.0, 0.4 * e.z), Quat.v3(e.x, e.y, 0.2 * e.z))]

/// A weapon's seated magazine, in the weapon's frame.
let seatedIn = (w: Gun.Weapon): List<Solid> => magSolids(w.well.kind) |> List.map((s) => { s with at: Hold.compose(Gun.seatOf(w), s.at) })

let pistolSolids =
  let c = Gun.center in
  List.append(seatedIn(Gun.pistol), [
    part(c, 0.03, 0.04, 0.205, 0.0, Gun.bore, 0.0 - 0.0475, 0.0),
    part(c, 0.03, 0.1, 0.045, Gun.handleAt.x, Gun.handleAt.y, Gun.handleAt.z, Gun.handleRake),
    part(c, 0.008, 0.006, 0.045, 0.0, 0.0 - 0.002, 0.0 - 0.028, 0.0),
    part(c, 0.006, 0.022, 0.006, 0.0, 0.012, 0.0 - 0.016, 0.0),
  ])

let rifleSolids =
  let c = Longarm.center in
  let mid = 0.0 - (Longarm.foreNear + Longarm.foreFar) / 2.0 in
  let length = Longarm.foreFar - Longarm.foreNear in
  List.append(seatedIn(Longarm.weapon), [
    part(c, 0.04, 0.08, 0.2, 0.0, 0.04, 0.18, 0.0),
    part(c, 0.045, 0.05, 0.22, 0.0, 0.06, 0.0 - 0.03, 0.0),
    part(c, 0.03, 0.09, 0.04, 0.0, 0.0, 0.012, 0.0 - 0.25),
    part(c, 0.008, 0.006, 0.05, 0.0, 0.016, 0.0 - 0.045, 0.0),
    part(c, 0.006, 0.02, 0.006, 0.0, 0.026, 0.0 - 0.035, 0.0),
    part(c, 0.045, 0.05, length, 0.0, Longarm.bore, mid, 0.0),
    part(c, 0.03, 0.03, length, 0.0, 0.02, mid, 0.0),
    part(c, 0.045, 0.008, 0.014, Longarm.handleAt.x, Longarm.handleAt.y, Longarm.handleAt.z, 0.0),
  ])

let shotgunSolids =
  let c = Pump.center in
  let mid = 0.0 - (Pump.foreNear + Pump.foreFar) / 2.0 in
  [part(c, 0.04, 0.075, 0.25, 0.0, 0.045, 0.175, 0.0),
   part(c, 0.045, 0.08, 0.22, 0.0, 0.05, 0.0 - 0.06, 0.0),
   part(c, 0.03, 0.09, 0.04, 0.0, 0.0, 0.012, 0.0 - 0.25),
   part(c, 0.008, 0.006, 0.05, 0.0, 0.0 - 0.002, 0.0 - 0.045, 0.0),
   part(c, 0.006, 0.02, 0.006, 0.0, 0.008, 0.0 - 0.035, 0.0),
   part(c, 0.018, 0.018, Pump.muzzleAhead - 0.17, 0.0, Pump.bore, 0.0 - (0.17 + Pump.muzzleAhead) / 2.0, 0.0),
   part(c, 0.021, 0.021, 0.4, 0.0, 0.03, 0.0 - 0.37, 0.0),
   part(c, 0.046, 0.04, Pump.foreFar - Pump.foreNear, 0.0, 0.025, mid, 0.0),
   part(c, 0.048, 0.042, 0.006, 0.0, 0.025, mid + 0.04, 0.0),
   part(c, 0.048, 0.042, 0.006, 0.0, 0.025, mid - 0.04, 0.0)]

// The bottle: its body, shoulder, neck and lip, up its +Y.
let bottleSolids =
  let base = 0.0 - (Props.extent(Props.Bottle)).y / 2.0 in
  let r = Props.bottleRadius in
  let n = Props.neckRadius in
  let cyl = (radius: float, h: float, y: float) => solid(Hold.p(0.0, y, 0.0), Cylinder(radius, h / 2.0)) in
  [cyl(r, Props.bodyHeight - 0.03, base + (Props.bodyHeight - 0.03) / 2.0),
   cyl(0.6 * r + 0.4 * n, 0.03, base + Props.bodyHeight - 0.015),
   cyl(n, Props.neckLength, base + Props.bodyHeight + Props.neckLength / 2.0),
   cyl(n * 1.25, 0.012, base + Props.bodyHeight + Props.neckLength - 0.006)]

// --- intents ---
// What a hand does with an item at a site, and the item-space target that
// needs. The placement solve (`proposal`) turns it into where the item sits
// in the hand's root frame; the hand stays where it is on the controller.

/// A grip's intent: wrap a handle (`Power`: a point on its axis, the axis's
/// way up the fist, the handle's radius toward the palm); a pistol grip
/// (`Pistol`: Power, and the trigger's face the index presses); a pinch
/// between the thumb's and index's pads (`Pinch`: the point, and the way
/// across it pad to pad); the palm on a surface (`Palm`: a point on it and
/// its outward normal); or hand-authored, not solved (`Free`: the hand's root
/// in the item's frame).
type Intent =
  | Power(at: Input.point3, axis: Input.point3, radius: float)
  | Pistol(at: Input.point3, axis: Input.point3, radius: float, trigger: Input.point3)
  | Pinch(at: Input.point3, across: Input.point3)
  | Palm(at: Input.point3, normal: Input.point3)
  | Free(hand: Input.pose)

/// One held item at one site: what it is (for its preset), its solids in its
/// frame, the intent, the item's way the fingers point (`forward`: to the
/// root's -Z, squared to the intent's axis), and a hand edit (`nudge`: the
/// item moved in the hand's root frame after the solve; `Hold.origin` for
/// none).
type Spec = { name: string, kind: Props.Kind, site: Props.Site, solids: List<Solid>, intent: Intent, forward: Input.point3, nudge: Input.pose }

let spec = (name: string, k: Props.Kind, site: Props.Site, solids: List<Solid>, intent: Intent, forward: Input.point3): Spec =>
  { name: name, kind: k, site: site, solids: solids, intent: intent, forward: forward, nudge: Hold.origin }

let up = Quat.v3(0.0, 1.0, 0.0)
let ahead = Quat.v3(0.0, 0.0, 0.0 - 1.0)
let raked = (rake: float): Input.point3 => Quat.rotate(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), rake), up)

// A gun's point in its trigger hand's layout frame, in its collider's.
let gunAt = (center: Input.point3, x: float, y: float, z: float): Input.point3 => Quat.sub(Quat.v3(x, y, z), center)

/// Every held item and site.
let specs: List<Spec> = [
  spec("ball", Props.Ball, Props.AtGrip, [solid(Hold.origin, Sphere(Props.ballRadius))], Palm(Quat.v3(0.0, Props.ballRadius, 0.0), up), ahead),
  // On its top, across the palm, the fingers over its 10 cm width.
  spec("brick", Props.Brick, Props.AtGrip, [boxAt(Hold.origin, Props.extent(Props.Brick))], Palm(Quat.v3(0.0, (Props.extent(Props.Brick)).y / 2.0, 0.0), up), Quat.v3(1.0, 0.0, 0.0)),
  // The band at its near end (+Z), the stick out past the index.
  spec("stick (band)", Props.Stick, Props.AtGrip,
    [boxAt(Hold.p(0.0, 0.0, Props.stickLength / 2.0 - 0.07), Quat.v3(1.3 * Props.stickThickness, 1.3 * Props.stickThickness, 0.12)), boxAt(Hold.origin, Props.extent(Props.Stick))],
    Power(Quat.v3(0.0, 0.0, Props.stickLength / 2.0 - 0.07), ahead, 0.65 * Props.stickThickness), Quat.v3(0.0, 0.0 - 1.0, 0.0)),
  spec("bottle neck", Props.Bottle, Props.AtGrip, bottleSolids, Power(Quat.v3(0.0, Props.neckAt, 0.0), up, Props.neckRadius), ahead),
  spec("pistol grip", Props.Pistol, Props.AtGrip, pistolSolids,
    Pistol(Quat.sub(Gun.handleAt, Gun.center), raked(Gun.handleRake), 0.015, gunAt(Gun.center, 0.0, 0.006, 0.0 - 0.019)), ahead),
  spec("pistol slide", Props.Pistol, Props.AtSlide, pistolSolids, Free(Hold.at(Gun.pistol.rack.at, Quat.identity)), ahead),
  spec("rifle grip", Props.Rifle, Props.AtGrip, rifleSolids,
    Pistol(gunAt(Longarm.center, 0.0, 0.0, 0.012), raked(0.0 - 0.25), 0.015, gunAt(Longarm.center, 0.0, 0.018, 0.0 - 0.038)), ahead),
  // The support hands: the fore-end's axis up the fist, the palm under it.
  spec("rifle foregrip", Props.Rifle, Props.AtFore, rifleSolids,
    Power(gunAt(Longarm.center, 0.0, 0.02, 0.0 - (Longarm.foreNear + Longarm.foreFar) / 2.0), ahead, 0.015), Quat.v3(0.0 - 1.0, 0.0, 0.0)),
  spec("shotgun grip", Props.Shotgun, Props.AtGrip, shotgunSolids,
    Pistol(gunAt(Pump.center, 0.0, 0.0, 0.012), raked(0.0 - 0.25), 0.015, gunAt(Pump.center, 0.0, 0.0, 0.0 - 0.038)), ahead),
  spec("shotgun fore-end", Props.Shotgun, Props.AtFore, shotgunSolids,
    Power(gunAt(Pump.center, 0.0, 0.025, 0.0 - (Pump.foreNear + Pump.foreFar) / 2.0), ahead, 0.02), Quat.v3(0.0 - 1.0, 0.0, 0.0)),
  spec("pistol mag", Props.Mag(Ammo.PistolMag, 0.0), Props.AtGrip, magSolids(Ammo.PistolMag), Power(Quat.v3(0.0, 0.004, 0.0), up, 0.013), ahead),
  spec("rifle mag", Props.Mag(Ammo.RifleMag, 0.0), Props.AtGrip, magSolids(Ammo.RifleMag), Power(Hold.zero, up, 0.0125), ahead),
  // Pinched across, its nose (-Z) along the fingers.
  spec("shell (from the box)", Props.Round(Ammo.Shell, 0.0), Props.AtGrip, magSolids(Ammo.Shell), Pinch(Hold.zero, Quat.v3(1.0, 0.0, 0.0)), ahead),
]

// --- the placement solve ---
// Right hand, in its root frame; the left's is its mirror (`held`).

let unitOf = (v: Input.point3): Input.point3 => Quat.scale(1.0 / Quat.length(v), v)
let squareTo = (v: Input.point3, n: Input.point3): Input.point3 => unitOf(Quat.sub(v, Quat.scale(Quat.dot(v, n), n)))

/// The turn taking item direction `a` onto root direction `ra`, and `b`
/// (squared to `a`) onto `rb` squared to `ra`.
let alignTurn = (a: Input.point3, ra: Input.point3, b: Input.point3, rb: Input.point3): Input.quaternion =>
  let basis = (u: Input.point3, v: Input.point3) => let x = unitOf(u) in let y = squareTo(v, x) in Quat.fromBasis(x, y, Quat.cross(x, y)) in
  Quat.mul(basis(ra, rb), Quat.conj(basis(a, b)))

/// An item turned `turn` in the root with its point `from` at `to`.
let itemAt = (turn: Input.quaternion, from: Input.point3, to: Input.point3): Input.pose => Hold.at(Quat.sub(to, Quat.rotate(turn, from)), turn)

/// Solids in their item's frame, the item at `item` in the root.
let posedIn = (item: Input.pose, solids: List<Solid>): List<Solid> => solids |> List.map((s) => { s with at: Hold.compose(item, s.at) })

/// The item moved along the palm's normal until the palm touches it
/// (`palmPush`), if it can.
let onPalm = (solids: List<Solid>, item: Input.pose): Input.pose =>
  match palmPush(posedIn(item, solids), Hold.Right) with
  | Option.Some(d) => Hold.at(Quat.sub(item.position, Quat.scale(d, palmNormal(Hold.Right))), item.orientation)
  | Option.None => item

// The pads a natural pinch brings together: the thumb's and the index's tips.
let pinchPads = let p = Hand.local(Hold.Right, Hand.pinch) in (Hand.at(p, Hand.thumbTip), Hand.at(p, Hand.indexTip))
// The index's knuckle (MCP), straight: it closes about its +X.
let indexKnuckle = Hand.jointOf(flatRight, 7.0)

/// Where a spec's intent puts its item in the right hand's root frame,
/// before its nudge. Power: its axis up the fist's channel (the grip's Y
/// through its origin), `forward` ahead, then onto the palm. Pistol: that,
/// slid along the axis until the trigger is in the plane the index closes
/// in. Pinch: the point between the pads. Palm: the surface on the palm.
let proposal = (s: Spec): Input.pose =>
  match s.intent with
  | Power(at, axis, _) => onPalm(s.solids, itemAt(alignTurn(axis, up, s.forward, ahead), at, Hold.zero))
  | Pistol(at, axis, _, trigger) =>
    let item = itemAt(alignTurn(axis, up, s.forward, ahead), at, Hold.zero) in
    let hinge = Quat.rotate(indexKnuckle.orientation, Quat.v3(1.0, 0.0, 0.0)) in
    let off = Quat.dot(Quat.sub((Hold.compose(item, Hold.at(trigger, Quat.identity))).position, indexKnuckle.position), hinge) / Quat.dot(up, hinge) in
    onPalm(s.solids, Hold.at(Quat.sub(item.position, Quat.scale(off, up)), item.orientation))
  | Pinch(at, across) =>
    let (thumb, index) = pinchPads in
    itemAt(alignTurn(across, Quat.sub(index, thumb), s.forward, ahead), at, Quat.lerp(thumb, index, 0.5))
  | Palm(at, normal) =>
    let n = palmNormal(Hold.Right) in
    onPalm(s.solids, itemAt(alignTurn(normal, Quat.scale(0.0 - 1.0, n), s.forward, ahead), at, Quat.add((palmJoint(Hold.Right)).position, Quat.scale(0.015, n))))
  | Free(hand) => Hold.inverse(hand)

/// Where a spec's item sits in the right hand's root frame: the proposal,
/// nudged.
let placed = (s: Spec): Input.pose => Hold.compose(s.nudge, proposal(s))

/// The hand's root in the item's frame (what a prop's grip is).
let handOn = (s: Spec): Input.pose => Hold.inverse(placed(s))

/// A held item at a site, placed: its spec, solids, the hand's root in its
/// frame, and its preset (`Props.fingers`).
type Item = { name: string, spec: Spec, solids: List<Solid>, hand: Input.pose, preset: Hand.FingerPose }

let items: List<Item> = specs |> List.map((s) => { name: s.name, spec: s, solids: s.solids, hand: handOn(s), preset: Props.fingers(s.kind, s.site, 0.0) })

/// An item's solids in a hand's root frame (the left's mirrored).
let held = (side: Hold.Side, it: Item): List<Solid> =>
  seenFrom(it.hand, it.solids) |> List.map((s) => if side == Hold.Left then { s with at: Hand.mirror(s.at) } else s)

/// An item's preset and its fit, measured, with the fit's pose error.
type Compared = { name: string, preset: Metrics, fitted: Fitted, fit: Metrics, error: List<float> }

let compare = (side: Hold.Side, it: Item): Compared =>
  let solids = held(side, it) in
  let f = fitOn(side, solids, triggerOf(side, it)) in
  { name: it.name, preset: measure(side, solids, it.preset), fitted: f, fit: measure(side, solids, f.pose), error: tipError(side, solids, f) }

/// A pistol grip's trigger, as the index closes on it: a 1 mm bead at its
/// face, in a hand's root frame.
let triggerOf = (side: Hold.Side, it: Item): Option.t<List<Solid>> =>
  match it.spec.intent with
  | Pistol(_, _, _, t) => Option.Some([solid(Hold.at(inRootOf(side, it, t), Quat.identity), Sphere(0.001))])
  | _ => Option.None

let intentName = (i: Intent): string =>
  match i with
  | Power(_, _, _) => "power"
  | Pistol(_, _, _, _) => "pistol"
  | Pinch(_, _) => "pinch"
  | Palm(_, _) => "palm"
  | Free(_) => "free"

/// An intent's targets (its point; a pistol's trigger too) in a hand's root
/// frame.
let targetsOf = (side: Hold.Side, it: Item): List<Input.point3> =>
  let points =
    match it.spec.intent with
    | Power(at, _, _) => [at]
    | Pistol(at, _, _, trigger) => [at, trigger]
    | Pinch(at, _) => [at]
    | Palm(at, _) => [at]
    | Free(_) => [] in
  points |> List.map((p) => (inRootOf(side, it, p)))

// An item's point in a hand's root frame (the left's mirrored).
let inRootOf = (side: Hold.Side, it: Item, p: Input.point3): Input.point3 =>
  let r = Hold.compose(Hold.inverse(it.hand), Hold.at(p, Quat.identity)) in
  (if side == Hold.Left then Hand.mirror(r) else r).position

/// A fit's intent checks: the palm's gap; the index pad's to the trigger
/// (a pistol grip's; else 0); how far the pinch point is off the line
/// between the thumb's and index's pads (a pinch's; else 0).
type Checks = { palm: float, trigger: float, pinch: float }

let padOf = (posed: Hand.Posed, j: float): (Input.point3, Input.point3, float) =>
  let c = capOf(j) in let (a, b) = Hand.ends(posed, c) in (a, b, c.radius)

let checksOf = (side: Hold.Side, it: Item, f: Fitted): Checks =>
  let posed = Hand.local(side, f.pose) in
  let point = (p: Input.point3) => inRootOf(side, it, p) in
  let (_, thumb, _) = padOf(posed, 4.0) in
  let (ia, ib, ir) = padOf(posed, 9.0) in
  { palm: palmGap(held(side, it), side),
    trigger: (match it.spec.intent with
      | Pistol(_, _, _, t) => toSegment(ia, ib, point(t)) - ir
      | _ => 0.0),
    pinch: (match it.spec.intent with
      | Pinch(at, _) => toSegment(thumb, ib, point(at))
      | _ => 0.0) }

// --- checks ---

// A Touch-sized handle up the grip's Y through its origin (`Hand.handleRadius`).
let handle = [solid(Hold.origin, Cylinder(Hand.handleRadius, 0.1))]

// A 1 cm rod across the index's first bone, 2 cm out and 2 mm under it.
let bar =
  let mcp = Hand.jointOf(Hand.local(Hold.Right, Hand.flat), 7.0) in
  let at = Hold.compose(mcp, Hold.p(0.0, 0.0 - (0.0095 + 0.002 + 0.005), 0.0 - 0.02)) in
  [solid(Hold.at(at.position, Quat.mul(mcp.orientation, Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), Math.pi / 2.0))), Cylinder(0.005, 0.05))]

// The index closing on either stops within the contact gap, none of it
// inside. On the handle its tip touches first, so its joints stop together;
// on the rod its first bone does, and the joints past it close on, its tip
// further than one uniform curl could.
let curlsOn = (solids: List<Solid>) =>
  let cl = close(solids, Hold.Right, 1.0, 0.0, jointRig) in
  let gaps = segmentsOf(Hold.Right, 1.0, cl.curls, 0.0) |> List.map((s) => let (a, b, r) = s in gapOf(solids, a, b, r)) in
  let ok = not cl.buried && (gaps |> List.all((g) => g >= 0.0)) && (gaps |> List.any((g) => g <= contactGap + 0.0002)) in
  (ok, cl.curls)
expect (
  let (onHandle, h) = curlsOn(handle) in
  let (onBar, b) = curlsOn(bar) in
  let k = (c: List<float>, i: float) => List.nth(i, c) |> Option.defaultValue(0.0) in
  onHandle && onBar && k(h, 0.0) == k(h, 2.0) && k(h, 0.0) > 0.5 && k(b, 0.0) < k(b, 1.0) && k(b, 1.0) <= k(b, 2.0)
)
// A grazing touch between two of a fixed step's samples still stops it: a
// 1 mm bead just off the index's sweep.
expect (
  let mcp = Hand.jointOf(Hand.local(Hold.Right, Hand.flat), 7.0) in
  let bead = [solid(Hold.compose(mcp, Hold.p(0.0, 0.0 - 0.0184486122, 0.0 - 0.0812138931)), Sphere(0.001))] in
  let cl = close(bead, Hold.Right, 1.0, 0.0, jointRig) in
  let gaps = segmentsOf(Hold.Right, 1.0, cl.curls, 0.0) |> List.map((s) => let (a, b, r) = s in gapOf(bead, a, b, r)) in
  cl.curls != [1.0, 1.0, 1.0] && (gaps |> List.all((g) => g >= 0.0)) && (gaps |> List.any((g) => g <= contactGap + 0.0002))
)
// A capsule starting inside buries its digit, which stays straight: a ball
// round the index's middle joint.
expect (
  let (a, _, _) = List.nth(1.0, segmentsOf(Hold.Right, 1.0, [0.0, 0.0, 0.0], 0.0)) |> Option.defaultValue((Hold.zero, Hold.zero, 0.0)) in
  let cl = close([solid(Hold.at(a, Quat.identity), Sphere(0.01))], Hold.Right, 1.0, 0.0, fingerRig) in
  cl.buried && cl.curls == [0.0, 0.0] && not (close(handle, Hold.Right, 1.0, 0.0, fingerRig)).buried
)
// On the handle: every finger closes round it, the thumb opposed, the palm
// on it; the same every time; and the left hand's fit on the mirrored
// handle is the right's.
expect (
  let r = fit(Hold.Right, handle) in
  let m = measure(Hold.Right, handle, r.pose) in
  not (r.buried |> List.any((b) => b)) && not r.palmBuried && r.pose.thumb.opposition >= 0.5
    && (m.digits |> List.all((g) => g >= 0.0 && g < 0.012)) && m.contacts >= 5.0 && fit(Hold.Right, handle) == r
)
expect (
  let mirrored = (s: Solid) => { s with at: Hand.mirror(s.at) } in
  let solids = [solid(Hold.at(Quat.v3(0.0 - 0.01, 0.0, 0.01), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.0 - 0.25)), Box(Quat.v3(0.015, 0.045, 0.02)))] in
  let (r, l) = (fit(Hold.Right, solids), fit(Hold.Left, solids |> List.map(mirrored))) in
  (List.zip(List.flatten(l.curls), List.flatten(r.curls)) |> List.all((ab) => let (a, b) = ab in near(a, b)))
    && r.pose.thumb.opposition == l.pose.thumb.opposition && r.buried == l.buried && r.push == l.push
)
// The palm push: a palm 1 cm short of a box comes 9 mm in; one 1 cm in
// backs 11 mm out; one on the handle stays.
expect (
  let n = palmNormal(Hold.Right) in
  let palmAt = (gap: float) =>
    let at = Quat.add((palmJoint(Hold.Right)).position, Quat.scale(0.014 + gap + 0.05, n)) in
    [solid(Hold.at(at, (palmJoint(Hold.Right)).orientation), Box(Quat.v3(0.1, 0.05, 0.1)))] in
  let pushed = (gap: float) => palmPush(palmAt(gap), Hold.Right) |> Option.defaultValue(1.0) in
  Math.abs(pushed(0.01) - 0.009) < 0.0005 && Math.abs(pushed(0.0 - 0.01) + 0.011) < 0.0005
    && (palmPush(handle, Hold.Right) |> Option.map((d) => Math.abs(d + 0.001) < 0.0005) |> Option.defaultValue(false))
)


// The thenar inside a held ball or stick doesn't bury the thumb: it stops
// only its curl (its CMC and MCP), and its tip closes on.
expect (
  items |> List.filter((it) => it.name == "ball" || it.name == "stick (band)") |> List.all((it) =>
    let f = fit(Hold.Right, held(Hold.Right, it)) in
    List.head(f.buried) == Option.Some(false) && (List.head(f.curls) |> Option.bind(List.last) |> Option.defaultValue(0.0)) > 0.0)
)
// A fitted finger's tip is where closing each joint on its own puts it,
// within 2 mm, where its first bone touches first: every finger on the
// stick and the rifle magazine, the brick's middle and ring. (The brick's
// index wraps its DIP past the pose's PIP:DIP; elsewhere a lone DIP
// closes on air.)
expect (
  let errorOn = (name: string) =>
    items |> List.find((it) => it.name == name) |> Option.map((it) => let solids = held(Hold.Right, it) in tipError(Hold.Right, solids, fit(Hold.Right, solids)))
      |> Option.defaultValue([]) in
  let under = (es: List<float>, ds: List<float>) => List.length(es) == 5.0 && (ds |> List.all((d) => (List.nth(d, es) |> Option.defaultValue(1.0)) <= 0.002)) in
  under(errorOn("stick (band)"), [1.0, 2.0, 3.0, 4.0]) && under(errorOn("rifle mag"), [1.0, 2.0, 3.0, 4.0]) && under(errorOn("brick"), [2.0, 3.0])
)
