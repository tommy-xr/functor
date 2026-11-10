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

// The distance outside a box (or cylinder), from how far a point is past
// each face pair (negative: inside them).
let past = (x: float, y: float, z: float): float =>
  Quat.length(Quat.v3(Math.max(x, 0.0), Math.max(y, 0.0), Math.max(z, 0.0))) + Math.min(Math.max(x, Math.max(y, z)), 0.0)

/// The signed distance from `q` to a solid: negative inside.
let sdf = (s: Solid, q: Input.point3): float =>
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
// way after 12 steps (a 4 cm bone's least to 0.12 mm).
let golden = 0.381966
let goldenSteps = 12.0
let least = (f: (float) => float): float =>
  let (_, _, _, fc, _, fd) = List.range(goldenSteps) |> List.fold((s, _) =>
    let (lo, hi, c, fc, d, fd) = s in
    if fc < fd then let c2 = lo + golden * (d - lo) in (lo, d, c2, f(c2), c, fc)
    else let d2 = hi - golden * (hi - c) in (c, hi, d, fd, d2, f(d2)),
    (0.0, 1.0, golden, f(golden), 1.0 - golden, f(1.0 - golden))) in
  Math.min(fc, fd)

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
// A digit is the thumb (0) or a finger (1-4: index to little). Each closes
// by three joints, each moving one capsule and every capsule past it: a
// finger's MCP, PIP and DIP (its proximal, intermediate and distal bones'),
// the thumb's CMC, MCP and IP (its metacarpal, proximal and distal's).

let fingers = [Hand.index, Hand.middle, Hand.ring, Hand.little]

let capOf = (j: float): Hand.Capsule => Hand.capsules |> List.find((c) => c.joint == j) |> Option.defaultValue({ joint: j, radius: 0.0, length: 0.0 })

/// A digit's three moving capsules, proximal first.
let moving = (d: float): List<Hand.Capsule> => [capOf(5.0 * d + 2.0), capOf(5.0 * d + 3.0), capOf(5.0 * d + 4.0)]

/// The palm: the four metacarpals' capsules (they don't move).
let palmCaps = [6.0, 11.0, 16.0, 21.0] |> List.map(capOf)

/// A digit's moving joints in the wrist's frame, each joint's curl
/// (`c`: 0 straight to 1 a fist's flexion) its own; the thumb turned
/// `opposition`. Then its tip.
let chainOf = (d: float, c: List<float>, opposition: float): List<(Input.pose, Input.quaternion)> =>
  let k = (i: float) => List.nth(i, c) |> Option.defaultValue(0.0) in
  if d == 0.0 then
    Hand.chain(Hold.at(Hand.thumbBase, Hand.thumbTurn(opposition)),
      [(Hand.thumbMetacarpal, k(0.0) * Hand.thumbCmcFlex), (Hand.thumbProximal, k(1.0) * Hand.thumbMcpFlex), (Hand.thumbDistal, k(2.0) * Hand.thumbIpFlex)])
  else
    let f = List.nth(d - 1.0, fingers) |> Option.defaultValue(Hand.index) in
    Hand.chain(Hold.at(f.base, Quat.yaw(f.splay)),
      [(f.metacarpal, 0.0), (f.proximal, k(0.0) * Hand.mcpFlex), (f.intermediate, k(1.0) * Hand.pipFlex), (f.distal, k(2.0) * Hand.dipFlex)])
      |> List.drop(1.0)

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

// Uniformly curled, a digit's capsules are `Hand.local`'s, either hand.
expect (
  [Hold.Left, Hold.Right] |> List.all((side) =>
    let p = Hand.local(side, { Hand.power with index: 0.4, little: 0.9 }) in
    [(0.0, Hand.power.thumb.curl), (1.0, 0.4), (2.0, Hand.power.middle), (4.0, 0.9)] |> List.all((dc) =>
      let (d, c) = dc in
      List.zip(moving(d), segmentsOf(side, d, [c, c, c], Hand.power.thumb.opposition)) |> List.all((sc) =>
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
// A closing joint steps 1/16 of its flexion (a fingertip ~6 mm: never past a
// capsule's thickness), then bisects the step that touched 8 times.
let marchStep = 1.0 / 16.0
let bisections = 8.0

/// A digit closed: its joints' curls (proximal first), and whether it started
/// in what it holds (`buried`: left where it started).
type Closed = { curls: List<float>, buried: bool }

/// Close a digit from `start` on `solids` (`opposition`: the thumb's turn).
/// Every joint closes together until a capsule comes within `contactGap`;
/// that joint and every one before it stops, the joints past it close on,
/// until each has touched or reached a full curl. A capsule already inside
/// at `start` buries the digit.
let close = (solids: List<Solid>, side: Hold.Side, d: float, opposition: float, start: float): Closed =>
  let gapsFrom = (j: float, c: List<float>): List<float> =>
    segmentsOf(side, d, c, opposition) |> List.indexedMap((i, s) => let (a, b, r) = s in if i < j then 1.0 else gapUnder(touchGap, solids, a, b, r)) in
  // The farthest capsule from joint `j` on within `contactGap`, if any.
  let touching = (j: float, c: List<float>): Option.t<float> =>
    gapsFrom(j, c) |> List.indexedMap((i, g) => if g <= contactGap then i else 0.0 - 1.0) |> List.maximum |> Option.filter((i) => i >= 0.0) in
  // Joint `j` and past it at `s`.
  let from = (j: float, s: float, c: List<float>): List<float> => c |> List.indexedMap((i, x) => if i >= j then s else x) in
  let march = (j: float, c: List<float>) =>
    Loop.until((st) => let (s, hit) = st in Option.isSome(hit) || s >= 1.0,
      (st) => let (s, _) = st in
        let s2 = Math.min(1.0, s + marchStep) in
        (s2, touching(j, from(j, s2, c)) |> Option.map((_) => (s, s2))),
      (List.nth(j, c) |> Option.defaultValue(0.0), Option.None)) in
  // One phase: the joints from `j` on close together until a capsule
  // touches; the next phase starts past it (past the last, done).
  let phase = (st: (float, List<float>)): (float, List<float>) =>
    let (j, c) = st in
    match touching(j, c) with
    | Option.Some(i) => (i + 1.0, c)
    | Option.None =>
      let (_, hit) = march(j, c) in
      (match hit with
       | Option.None => (3.0, from(j, 1.0, c))
       | Option.Some(bracket) =>
         let (lo, hi) = List.range(bisections) |> List.fold((b, _) =>
           let (lo, hi) = b in
           let mid = (lo + hi) / 2.0 in
           if Option.isSome(touching(j, from(j, mid, c))) then (lo, mid) else (mid, hi), bracket) in
         (1.0 + (touching(j, from(j, hi, c)) |> Option.defaultValue(2.0)), from(j, lo, c))) in
  let c0 = [start, start, start] in
  if gapsFrom(0.0, c0) |> List.any((g) => g < 0.0) then { curls: c0, buried: true }
  else
    let (_, curls) = Loop.until((st) => let (j, _) = st in j > 2.0, phase, (0.0, c0)) in
    { curls: curls, buried: false }

// The thumb's turns tried: at rest to opposed.
let oppositions = [0.0, 0.25, 0.5, 0.75, 1.0]

/// The thumb closed at its best turn: the one whose tip (its pad) comes
/// nearest, then the one touching with the most of its capsules. Buried at
/// every turn: at rest, buried.
let closeThumb = (solids: List<Solid>, side: Hold.Side): (Closed, float) =>
  let tried = oppositions |> List.map((o) =>
    let cl = close(solids, side, 0.0, o, 0.0) in
    let gaps = segmentsOf(side, 0.0, cl.curls, o) |> List.map((s) => let (a, b, r) = s in gapOf(solids, a, b, r)) in
    let touches = gaps |> List.filter((g) => g <= touchGap) |> List.length in
    (cl, o, (List.last(gaps) |> Option.defaultValue(1.0)) - 0.0001 * touches)) |> List.filter((t) => let (cl, _, _) = t in not cl.buried) in
  match tried |> List.sortBy((t) => let (_, _, score) = t in score) with
  | [best, ..rest] => let (cl, o, _) = best in (cl, o)
  | [] => ({ curls: [0.0, 0.0, 0.0], buried: true }, 0.0)

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
  let d = List.range(24.0) |> List.fold((d, _) => if Math.abs(d) > pushRange then d else d + gapAt(d), 0.0) in
  if Math.abs(d) <= pushRange && Math.abs(gapAt(d)) < 0.0002 then Option.Some(d) else Option.None

// --- a fit ---

/// A fit: the finger pose (a finger's curl is its first joint's: the most a
/// uniform curl closes before it touches), each digit's per-joint curls
/// (thumb first), which digits are buried, whether the palm starts inside
/// the item, and the palm push (`palmPush`).
type Fitted = { pose: Hand.FingerPose, curls: List<List<float>>, buried: List<bool>, palmBuried: bool, push: Option.t<float> }

/// Fit a hand to `solids` (in its root frame): each finger closed from
/// straight, the thumb at its best turn.
let fit = (side: Hold.Side, solids: List<Solid>): Fitted =>
  let (thumb, opposition) = closeThumb(solids, side) in
  let closed = [thumb, ..([1.0, 2.0, 3.0, 4.0] |> List.map((d) => close(solids, side, d, 0.0, 0.0)))] in
  let first = (i: float) => List.nth(i, closed) |> Option.bind((c) => List.head(c.curls)) |> Option.defaultValue(0.0) in
  { pose: { thumb: { curl: first(0.0), opposition: opposition }, index: first(1.0), middle: first(2.0), ring: first(3.0), little: first(4.0) },
    curls: closed |> List.map((c) => c.curls),
    buried: closed |> List.map((c) => c.buried),
    palmBuried: palmGap(solids, side) < 0.0,
    push: palmPush(solids, side) }

// --- measuring a grip ---

/// A grip measured: each digit's nearest gap (thumb first; negative, how
/// deep it is in), the palm's, how many capsules touch (`touchGap`), and its
/// strain (`strain`).
type Metrics = { digits: List<float>, palm: float, contacts: float, strain: float }

/// How strained a pose is, 0 relaxed: the mean of how far each curl is out
/// of 0.15-0.85, plus the mean of how far the ring finger's curl is more than
/// 0.3 from the middle's and the little's (they share tendons; the index and
/// thumb move freely).
let strain = (f: Hand.FingerPose): float =>
  let out = (c: float) => Math.max(0.0, 0.15 - c) + Math.max(0.0, c - 0.85) in
  let apart = (a: float, b: float) => Math.max(0.0, Math.abs(a - b) - 0.3) in
  ([f.thumb.curl, f.index, f.middle, f.ring, f.little] |> List.map(out) |> List.sum) / 5.0
    + (apart(f.ring, f.middle) + apart(f.ring, f.little)) / 2.0

/// Measure the hand posed `f` on `solids`.
let measure = (side: Hold.Side, solids: List<Solid>, f: Hand.FingerPose): Metrics =>
  let p = Hand.local(side, f) in
  let gaps = Hand.capsules |> List.map((c) => let (a, b) = Hand.ends(p, c) in (c.joint, gapOf(solids, a, b, c.radius))) in
  let nearest = (keep: (float) => bool) => gaps |> List.filter((g) => let (j, _) = g in keep(j)) |> List.map((g) => let (_, x) = g in x) |> List.minimum |> Option.defaultValue(1.0) in
  let digit = (d: float) => nearest((j) => j >= 5.0 * d + 2.0 && j <= 5.0 * d + 4.0) in
  { digits: [0.0, 1.0, 2.0, 3.0, 4.0] |> List.map(digit),
    palm: palmGap(solids, side),
    contacts: gaps |> List.filter((g) => let (_, x) = g in x <= touchGap) |> List.length,
    strain: strain(f) }

/// How far each digit's tip is from where its per-joint fit puts it, posed
/// by the fit's `FingerPose` (one curl a finger): the pose's error.
let tipError = (side: Hold.Side, fitted: Fitted): List<float> =>
  let tip = (d: float, c: List<float>, o: float) => List.last(chainOf(d, c, o)) |> Option.map((j) => let (p, _) = j in (inRoot(side, p)).position) |> Option.defaultValue(Hold.zero) in
  let f = fitted.pose in
  let o = f.thumb.opposition in
  List.zip([f.thumb.curl, f.index, f.middle, f.ring, f.little], fitted.curls) |> List.indexedMap((d, cu) =>
    let (c, u) = cu in
    Quat.length(Quat.sub(tip(d, c, o), tip(d, [u, u, u], o))))

// --- what the lab holds ---
// Each held item and site as solids in its own frame (its collider's: a
// prop's pose), and where the hand's root sits on it. A prop's collider is
// one bounding box, which for a gun holds the fist, so the solids are its
// drawn parts near a hand.

/// A held item at a site: its solids, the hand's root in its frame, whether
/// it is held anywhere (`free`: put on the palm, `palmPush`, before the fit),
/// and its preset (`Props.fingers`).
type Item = { name: string, solids: List<Solid>, hand: Input.pose, free: bool, preset: Hand.FingerPose }

let item = (name: string, solids: List<Solid>, hand: Input.pose, free: bool, k: Props.Kind, site: Props.Site): Item =>
  { name: name, solids: solids, hand: hand, free: free, preset: Props.fingers(k, site, 0.0) }

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

// The support hand on a long gun's foregrip line, half way along it, turned
// as the gun (`Longarm.supportOn` keeps the hand's turn: an upright hand's).
let onFore = (f: Longarm.Fore): Input.pose => Hold.compose(f.grip, Hold.p(0.0, 0.0, 0.0 - (f.near + f.far) / 2.0))

// Free props held at a representative place: the item's centre at the fist
// (a stick's grip band), then put on the palm. The brick lies across the
// palm, its top on it, the fingers over its 10 cm width.
let brickTurn = Quat.fromBasis(Quat.v3(0.0, 0.0, 1.0), Quat.v3(1.0, 0.0, 0.0), Quat.v3(0.0, 1.0, 0.0))
let stickHand = Hold.inverse(Hold.at(Quat.v3(0.0, Props.stickLength / 2.0 - 0.07, 0.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), Math.pi / 2.0)))

/// Every held item and site, at its current placement.
let items: List<Item> = [
  item("ball", [solid(Hold.origin, Sphere(Props.ballRadius))], Hold.origin, true, Props.Ball, Props.AtGrip),
  item("brick", [boxAt(Hold.origin, Props.extent(Props.Brick))], Hold.inverse(Hold.at(Hold.zero, brickTurn)), true, Props.Brick, Props.AtGrip),
  item("stick (band)", [boxAt(Hold.p(0.0, 0.0, Props.stickLength / 2.0 - 0.07), Quat.v3(1.3 * Props.stickThickness, 1.3 * Props.stickThickness, 0.12)), boxAt(Hold.origin, Props.extent(Props.Stick))],
    stickHand, true, Props.Stick, Props.AtGrip),
  item("bottle neck", bottleSolids, Props.grip(Props.Bottle) |> Option.defaultValue(Hold.origin), false, Props.Bottle, Props.AtGrip),
  item("pistol grip", pistolSolids, Gun.grip, false, Props.Pistol, Props.AtGrip),
  item("pistol slide", pistolSolids, Hold.at(Gun.pistol.rack.at, Quat.identity), false, Props.Pistol, Props.AtSlide),
  item("rifle grip", rifleSolids, Longarm.grip, false, Props.Rifle, Props.AtGrip),
  item("rifle foregrip", rifleSolids, onFore(Longarm.fore), false, Props.Rifle, Props.AtFore),
  item("shotgun grip", shotgunSolids, Pump.grip, false, Props.Shotgun, Props.AtGrip),
  item("shotgun fore-end", shotgunSolids, onFore(Pump.fore), false, Props.Shotgun, Props.AtFore),
  item("pistol mag", magSolids(Ammo.PistolMag), Hold.origin, true, Props.Mag(Ammo.PistolMag, 0.0), Props.AtGrip),
  item("rifle mag", magSolids(Ammo.RifleMag), Hold.origin, true, Props.Mag(Ammo.RifleMag, 0.0), Props.AtGrip),
  item("shell (from the box)", magSolids(Ammo.Shell), Hold.origin, false, Props.Round(Ammo.Shell, 0.0), Props.AtGrip),
]

/// An item's solids in a hand's root frame (the left's mirrored), a free
/// one put on the palm.
let held = (side: Hold.Side, it: Item): List<Solid> =>
  let seen = seenFrom(it.hand, it.solids) |> List.map((s) => if side == Hold.Left then { s with at: Hand.mirror(s.at) } else s) in
  if not it.free then seen
  else
    match palmPush(seen, side) with
    | Option.Some(d) => seen |> List.map((s) => shift(Quat.scale(0.0 - d, palmNormal(side)), s))
    | Option.None => seen

/// An item's preset and its fit, measured, with the fit's pose error.
type Compared = { name: string, preset: Metrics, fitted: Fitted, fit: Metrics, error: List<float> }

let compare = (side: Hold.Side, it: Item): Compared =>
  let solids = held(side, it) in
  let f = fit(side, solids) in
  { name: it.name, preset: measure(side, solids, it.preset), fitted: f, fit: measure(side, solids, f.pose), error: tipError(side, f) }

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
  let cl = close(solids, Hold.Right, 1.0, 0.0, 0.0) in
  let gaps = segmentsOf(Hold.Right, 1.0, cl.curls, 0.0) |> List.map((s) => let (a, b, r) = s in gapOf(solids, a, b, r)) in
  let ok = not cl.buried && (gaps |> List.all((g) => g >= 0.0)) && (gaps |> List.any((g) => g <= contactGap + 0.0002)) in
  (ok, cl.curls)
expect (
  let (onHandle, h) = curlsOn(handle) in
  let (onBar, b) = curlsOn(bar) in
  let k = (c: List<float>, i: float) => List.nth(i, c) |> Option.defaultValue(0.0) in
  onHandle && onBar && k(h, 0.0) == k(h, 2.0) && k(h, 0.0) > 0.5 && k(b, 0.0) < k(b, 1.0) && k(b, 1.0) <= k(b, 2.0)
)
// A capsule starting inside buries its digit, which stays straight: a ball
// round the index's middle joint.
expect (
  let (a, _, _) = List.nth(1.0, segmentsOf(Hold.Right, 1.0, [0.0, 0.0, 0.0], 0.0)) |> Option.defaultValue((Hold.zero, Hold.zero, 0.0)) in
  let cl = close([solid(Hold.at(a, Quat.identity), Sphere(0.01))], Hold.Right, 1.0, 0.0, 0.0) in
  cl.buried && cl.curls == [0.0, 0.0, 0.0] && not (close(handle, Hold.Right, 1.0, 0.0, 0.0)).buried
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
  let close6 = (a: float, b: float) => Math.abs(a - b) < 0.000001 in
  (List.zip(List.flatten(l.curls), List.flatten(r.curls)) |> List.all((ab) => let (a, b) = ab in close6(a, b)))
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


