// Hand: an articulated hand as data. The 26 joints of OpenXR's
// XR_EXT_hand_tracking, in its order; an average adult hand's bones; a
// finger pose (`FingerPose`: how far each finger bends, at its knuckle and
// past it) and the forward
// kinematics from it; where the hand sits on a controller; and the capsules
// it is drawn and (later) fitted with, and their drawing. Pure, so it is
// tested below with no world.
//
// Joint frames follow OpenXR: each joint sits at the base of its bone, -Z
// along the bone toward the fingertip, +Y out of the back of the hand (the
// nail side), +X = Y × Z (a tip's frame is its distal bone's). So hand
// tracking maps onto these directly, and the WebXR `generic-hand` model
// (`Skin`) with a few degrees' correction per joint. The right hand is
// authored; the left is its mirror across the root's YZ plane.
//
// Poses are in the hand's root frame: the controller's grip (+X right, +Y
// up, -Z forward), so `pose(side, grip, f)` places a hand on a tracked
// controller.

/// The joints, in OpenXR's order, by their WebXR names (plus OpenXR's palm).
let names = [
  "palm", "wrist",
  "thumb-metacarpal", "thumb-phalanx-proximal", "thumb-phalanx-distal", "thumb-tip",
  "index-finger-metacarpal", "index-finger-phalanx-proximal", "index-finger-phalanx-intermediate", "index-finger-phalanx-distal", "index-finger-tip",
  "middle-finger-metacarpal", "middle-finger-phalanx-proximal", "middle-finger-phalanx-intermediate", "middle-finger-phalanx-distal", "middle-finger-tip",
  "ring-finger-metacarpal", "ring-finger-phalanx-proximal", "ring-finger-phalanx-intermediate", "ring-finger-phalanx-distal", "ring-finger-tip",
  "pinky-finger-metacarpal", "pinky-finger-phalanx-proximal", "pinky-finger-phalanx-intermediate", "pinky-finger-phalanx-distal", "pinky-finger-tip",
]

/// Each joint's parent (an index into `names`); the wrist's is the root, -1.
let parents = [1.0, -1.0, 1.0, 2.0, 3.0, 4.0, 1.0, 6.0, 7.0, 8.0, 9.0, 1.0, 11.0, 12.0, 13.0, 14.0, 1.0, 16.0, 17.0, 18.0, 19.0, 1.0, 21.0, 22.0, 23.0, 24.0]

// Indices of the joints the checks and the drawing name.
let palm = 0.0
let wrist = 1.0
let thumbTip = 5.0
let indexTip = 10.0
let middleTip = 15.0
let ringTip = 20.0
let littleTip = 25.0
let tips = [thumbTip, indexTip, middleTip, ringTip, littleTip]

// --- the bones: an average adult right hand, in the wrist's frame ---
// Wrist crease to the middle fingertip 18.1 cm; knuckles 5.6 cm across,
// index to little. Each finger is its metacarpal's base (the CMC joint),
// how far it fans toward the thumb (radians about the wrist's +Y; negative
// fans toward the little finger), and its four bones' lengths (the distal's
// to the fingertip). The thumb has no intermediate bone.

type Finger = { base: Input.point3, splay: float, metacarpal: float, proximal: float, intermediate: float, distal: float }

let index: Finger = { base: Quat.v3(0.0 - 0.019, 0.0, 0.0 - 0.022), splay: 0.12, metacarpal: 0.066, proximal: 0.040, intermediate: 0.023, distal: 0.020 }
let middle: Finger = { base: Quat.v3(0.0 - 0.004, 0.0, 0.0 - 0.024), splay: 0.0, metacarpal: 0.064, proximal: 0.045, intermediate: 0.027, distal: 0.021 }
let ring: Finger = { base: Quat.v3(0.007, 0.0, 0.0 - 0.023), splay: 0.0 - 0.12, metacarpal: 0.058, proximal: 0.042, intermediate: 0.026, distal: 0.020 }
let little: Finger = { base: Quat.v3(0.016, 0.0, 0.0 - 0.021), splay: 0.0 - 0.24, metacarpal: 0.053, proximal: 0.033, intermediate: 0.019, distal: 0.018 }

// The thumb: its CMC joint on the palm's thumb side, toward the palm; its
// bones; and how its metacarpal turns at rest and fully opposed (across the
// palm): `yaw` toward the thumb side about the wrist's +Y, `pitch` toward
// the palm, `roll` about the bone (the nail turning to face the fingers).
let thumbBase = Quat.v3(0.0 - 0.022, 0.0 - 0.010, 0.0 - 0.020)
let thumbMetacarpal = 0.046
let thumbProximal = 0.032
let thumbDistal = 0.029
type Turn = { yaw: float, pitch: float, roll: float }
let thumbRest: Turn = { yaw: 0.70, pitch: 0.30, roll: 1.0 }
let thumbOpposed: Turn = { yaw: 0.3, pitch: 1.3, roll: 1.6 }

// A full curl (1): each finger's flexion at the MCP, PIP and DIP joints, a
// fist's (80°, 100°, 65°); the thumb's at its CMC, MCP and IP joints. A curl
// of `c` bends each by `c` times that: the ratios are fixed.
let mcpFlex = 80.0 * Math.pi / 180.0
let pipFlex = 100.0 * Math.pi / 180.0
let dipFlex = 65.0 * Math.pi / 180.0
let thumbCmcFlex = 0.2
let thumbMcpFlex = 0.9
let thumbIpFlex = 1.2

// --- the hand on a controller ---
// Where the right wrist sits in the grip's frame. The metacarpals point
// along the grip's -Z (the way a straightened index finger points), the
// back of the hand faces +X and the knuckles run down the handle (index on
// top): the wrist turned -90° about Z, then tipped 0.2 rad (11°) about X so
// the knuckle line slants across the palm as a handle does (the little
// finger's further forward). It sits 3.1 cm to the back of the hand's side,
// 1.5 cm down and 6.2 cm back, so a handle up the grip's Y axis through its
// origin (the fist's centroid; `handleRadius`, 1.7 cm, a Touch controller's)
// rests on the palm, and a power wrap (`power`) closes round it.
let wristOnGrip: Input.pose =
  Hold.at(Quat.v3(0.031, 0.0 - 0.015, 0.062), Quat.mul(Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.2), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.0 - Math.pi / 2.0)))
let handleRadius = 0.017

// --- the finger pose ---

/// How far a finger bends, each 0 (straight) to 1 (a fist's): `curl` at its
/// knuckle (MCP), `tip` at its two distal joints (PIP and DIP, together).
type Bend = { curl: float, tip: float }

/// How far the thumb bends, 0 straight to 1 closed (`curl` at its CMC and
/// MCP joints, `tip` at its IP), and how far it is turned across the palm
/// (`opposition`: 0 at rest, 1 opposed; below 0, to -1, abducted: swung out
/// of the palm, back and away from the index, as round a ball or along a
/// brick's side, where any turn across the palm puts its base in the item).
type Thumb = { curl: float, tip: float, opposition: float }

/// How each finger bends. What a grip fitter outputs and the drawing reads.
type FingerPose = { thumb: Thumb, index: Bend, middle: Bend, ring: Bend, little: Bend }

/// A finger bent `c` at every joint: one curl, as a fist closes.
let uniform = (c: float): Bend => { curl: c, tip: c }
/// The thumb bent `c` at every joint, turned `opposition`.
let uniformThumb = (c: float, opposition: float): Thumb => { curl: c, tip: c, opposition: opposition }

/// Every finger (and the thumb, curled and opposed) at `c`.
let curled = (c: float): FingerPose => { thumb: uniformThumb(c, c), index: uniform(c), middle: uniform(c), ring: uniform(c), little: uniform(c) }

/// Straight fingers, the thumb at rest: the skeleton's bind pose.
let flat: FingerPose = curled(0.0)
/// A fist, the thumb across it.
let fist: FingerPose = curled(1.0)

// An open hand's curl, relaxed; and the index on a trigger, at rest and
// pulled: hooked round it, its tip bent further than its knuckle.
let relaxed = 0.15
let triggerRest = 0.25
let triggerPulled = 0.55
let hookRest = 0.4
let hookPulled = 0.75

// From `a` to `b` as `t` goes 0 to 1 (clamped).
let mix = (a: float, b: float, t: float): float => Math.lerp(b, Math.clamp01(t), a)

// The presets, until a grip fitter fits the fingers to what they hold.

/// A power wrap round a handle (stick, bottle neck, foregrip, fore-end,
/// magazine): each finger curled as far as it goes without entering a
/// Touch handle, the thumb opposed round its back.
let power: FingerPose = { thumb: uniformThumb(0.75, 1.0), index: uniform(0.70), middle: uniform(0.75), ring: uniform(0.72), little: uniform(0.60) }
/// A pistol grip: the index hooked on the trigger (`trigger` 0..1 pulls it),
/// the rest wrapped.
let onTrigger = (trigger: float): FingerPose =>
  { power with index: { curl: mix(triggerRest, triggerPulled, trigger), tip: mix(hookRest, hookPulled, trigger) } }
/// A pinch, thumb to index (a round, a shell), the rest tucked.
let pinch: FingerPose = { thumb: uniformThumb(0.5, 0.1), index: uniform(0.5), middle: uniform(0.55), ring: uniform(0.6), little: uniform(0.6) }
/// An open palm: the ball, the brick.
let palmOpen: FingerPose = { thumb: uniformThumb(0.2, 0.3), index: uniform(0.18), middle: uniform(0.2), ring: uniform(0.22), little: uniform(0.25) }
/// A gun's grip `f` (its index resting on the trigger), the trigger pulled
/// `trigger` (0..1): the index curls in as far as `onTrigger`'s does, at
/// most a fist's.
let pullTrigger = (f: FingerPose, trigger: float): FingerPose =>
  let t = Math.clamp01(trigger) in
  { f with index: { curl: Math.min(1.0, f.index.curl + t * (triggerPulled - triggerRest)), tip: Math.min(1.0, f.index.tip + t * (hookPulled - hookRest)) } }

/// An empty hand on its controller: the grip (`squeeze`) curls the middle,
/// ring and little fingers from relaxed to wrapped round the handle, the
/// trigger the index; the thumb rests half curled, little opposed.
let empty = (trigger: float, squeeze: float): FingerPose =>
  let wrap = (to: Bend) => uniform(mix(relaxed, to.curl, squeeze)) in
  { thumb: uniformThumb(0.5, 0.25), index: uniform(mix(relaxed, triggerPulled, trigger)),
    middle: wrap(power.middle), ring: wrap(power.ring), little: wrap(power.little) }

// --- forward kinematics ---

/// A posed hand: each joint's rotation relative to its parent's (`parents`;
/// the wrist's relative to the root), and its pose in the root's frame (or
/// the world's, given the root's world pose). Both in `names` order.
type Posed = { rotations: List<Input.quaternion>, poses: List<Input.pose> }

// A flexion `a` (radians) about a joint's +X: positive bends toward the palm.
let flex = (a: float): Input.quaternion => Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.0 - a)

/// A chain of bones from `base`, each `(length, flexion at its base)`: each
/// joint's pose and its rotation from its parent (the first's from `base`'s
/// parent), base first, then the tip.
let chain = (base: Input.pose, bones: List<(float, float)>): List<(Input.pose, Input.quaternion)> =>
  let (tip, last, joints) =
    bones |> List.fold((acc, bone) =>
      let (p, q, js) = acc in
      let (length, angle) = bone in
      let bend = flex(angle) in
      let turned = Quat.mul(q, bend) in
      let rotation = if List.isEmpty(js) then turned else bend in
      (Quat.add(p, Quat.rotate(turned, Quat.v3(0.0, 0.0, 0.0 - length))), turned, [(Hold.at(p, turned), rotation), ..js]),
      (base.position, base.orientation, [])) in
  List.reverse([(Hold.at(tip, last), Quat.identity), ..joints])

/// A finger's chain (`chain`), its MCP, PIP and DIP joints each bent its own
/// curl (0 straight, 1 a fist's flexion).
let fingerJoints = (f: Finger, mcp: float, pip: float, dip: float) =>
  chain(Hold.at(f.base, Quat.yaw(f.splay)),
    [(f.metacarpal, 0.0), (f.proximal, mcp * mcpFlex), (f.intermediate, pip * pipFlex), (f.distal, dip * dipFlex)])
let fingerChain = (f: Finger, b: Bend) => fingerJoints(f, b.curl, b.tip, b.tip)

// Abducted (below 0), the turn carries on past rest the other way.
let thumbTurn = (opposition: float): Input.quaternion =>
  let t = Math.clamp(0.0 - 1.0, 1.0, opposition) in
  let at = (a: float, b: float) => Math.lerp(b, t, a) in
  Quat.mul(Quat.yaw(at(thumbRest.yaw, thumbOpposed.yaw)),
    Quat.mul(flex(at(thumbRest.pitch, thumbOpposed.pitch)), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), at(thumbRest.roll, thumbOpposed.roll))))

/// The thumb's chain turned `opposition`, its CMC, MCP and IP joints each
/// bent its own curl.
let thumbJoints = (opposition: float, cmc: float, mcp: float, ip: float) =>
  chain(Hold.at(thumbBase, thumbTurn(opposition)),
    [(thumbMetacarpal, cmc * thumbCmcFlex), (thumbProximal, mcp * thumbMcpFlex), (thumbDistal, ip * thumbIpFlex)])
let thumbChain = (t: Thumb) => thumbJoints(t.opposition, t.curl, t.curl, t.tip)

// The palm joint: half way along the middle metacarpal.
let palmInWrist = Hold.at(Quat.add(middle.base, Quat.v3(0.0, 0.0, 0.0 - middle.metacarpal / 2.0)), Quat.identity)

/// A pose seen in the mirror across its frame's YZ plane: a right hand's
/// joint as the left hand's (x flips; the frame stays right-handed, so its
/// +X flips with it).
let mirror = (p: Input.pose): Input.pose =>
  Hold.at(Quat.v3(0.0 - p.position.x, p.position.y, p.position.z), mirrorTurn(p.orientation))
let mirrorTurn = (q: Input.quaternion): Input.quaternion => { x: q.x, y: 0.0 - q.y, z: 0.0 - q.z, w: q.w }

/// The hand posed by `f` in its root's frame (the grip's).
let local = (side: Hold.Side, f: FingerPose): Posed =>
  let inWrist = List.flatten([
    [(palmInWrist, Quat.identity), (Hold.origin, Quat.identity)],
    thumbChain(f.thumb),
    fingerChain(index, f.index),
    fingerChain(middle, f.middle),
    fingerChain(ring, f.ring),
    fingerChain(little, f.little),
  ]) in
  // The wrist's rotation is from the root; the rest are from their parents.
  let rotations = inWrist |> List.indexedMap((i, j) => let (_, r) = j in if i == wrist then wristOnGrip.orientation else r) in
  let poses = inWrist |> List.map((j) => let (p, _) = j in Hold.compose(wristOnGrip, p)) in
  match side with
  | Hold.Right => { rotations: rotations, poses: poses }
  | Hold.Left => { rotations: rotations |> List.map(mirrorTurn), poses: poses |> List.map(mirror) }

/// The hand posed by `f` with its root (the grip) at `root`.
let pose = (side: Hold.Side, root: Input.pose, f: FingerPose): Posed =>
  let l = local(side, f) in
  { l with poses: l.poses |> List.map((p) => Hold.compose(root, p)) }

let jointOf = (posed: Posed, j: float): Input.pose => List.nth(j, posed.poses) |> Option.defaultValue(Hold.origin)

/// The frame a finger's distal two bones hang from: its proximal bone's,
/// at that bone's end (`first`: the finger's metacarpal joint). Seen from
/// it, they move with the finger's tip alone.
let hangFrom = (posed: Posed, first: float): Input.pose =>
  Hold.at((jointOf(posed, first + 2.0)).position, (jointOf(posed, first + 1.0)).orientation)

// --- the glove ---

/// One of the glove's capsules: from `joint`'s origin `length` along its -Z,
/// `radius` round (and capped by half spheres, so a distal bone's ends at
/// its fingertip). What is drawn, and what a grip fitter tests for contact.
type Capsule = { joint: float, radius: float, length: float }

// The palm is the four fat metacarpals; the thenar is the thumb's.
let fingerCapsules = (first: float, f: Finger, radii: (float, float, float)): List<Capsule> =>
  let (rp, ri, rd) = radii in
  [{ joint: first, radius: 0.014, length: f.metacarpal },
   { joint: first + 1.0, radius: rp, length: f.proximal },
   { joint: first + 2.0, radius: ri, length: f.intermediate },
   { joint: first + 3.0, radius: rd, length: f.distal - rd }]

let capsules: List<Capsule> = List.flatten([
  [{ joint: 2.0, radius: 0.014, length: thumbMetacarpal },
   { joint: 3.0, radius: 0.0105, length: thumbProximal },
   { joint: 4.0, radius: 0.0095, length: thumbDistal - 0.0095 }],
  fingerCapsules(6.0, index, (0.0095, 0.0085, 0.0075)),
  fingerCapsules(11.0, middle, (0.0098, 0.0088, 0.0078)),
  fingerCapsules(16.0, ring, (0.0092, 0.0082, 0.0073)),
  fingerCapsules(21.0, little, (0.0082, 0.0072, 0.0065)),
])

/// Whether a capsule starts its chain (a metacarpal's, off the wrist).
let starts = (c: Capsule): bool => List.nth(c.joint, parents) == Option.Some(wrist)

/// A capsule's axis in `posed`'s frame: its two ends.
let ends = (posed: Posed, c: Capsule): (Input.point3, Input.point3) =>
  let j = jointOf(posed, c.joint) in
  (j.position, Quat.add(j.position, Quat.rotate(j.orientation, Quat.v3(0.0, 0.0, 0.0 - c.length))))

// --- drawing the glove ---
// The glove is drawn in two parts, each one instanced draw of its capsules'
// caps and one of their cylinders: the palm and thumb, posed as given; and
// the four fingers, each bent to the nearest 1/64 (`drawLevels`) and read
// from tables built at load, so a finger moving every step (a trigger
// pulled) costs no forward kinematics. A finger's proximal bone hangs on its
// curl alone, and its distal two, seen from the proximal's end, on its tip
// alone: one table of each by level, composed as drawn. Each part is a
// subtree for `Scene.atPose` at the grip (whose local +Z is the grip's -Z,
// and +X the grip's -X).

/// A part's draws: its capsules' caps and cylinders, as instances.
type Part = { caps: List<Instance.t>, cylinders: List<Instance.t> }

/// A capsule's axis (its two ends), radius and length, in some frame.
type Seg = { a: Input.point3, b: Input.point3, radius: float, length: float }

// A point in the grip's frame as drawn.
let drawn = (p: Input.point3) => Quat.v3(0.0 - p.x, p.y, 0.0 - p.z)
let segOf = (posed: Posed, cap: Capsule): Seg => let (a, b) = ends(posed, cap) in { a: a, b: b, radius: cap.radius, length: cap.length }
let capAt = (p: Input.point3, r: float) => Instance.at(Quat.toVec3(p)) |> Instance.scale(r)
// A unit cylinder stands along +Y: turned about X, then Z, onto the axis.
let cylinderOf = (s: Seg) =>
  let u = Quat.scale(1.0 / s.length, Quat.sub(s.b, s.a)) in
  Instance.at(Quat.toVec3(Quat.lerp(s.a, s.b, 0.5))) |> Instance.scaleXYZ(2.0 * s.radius, s.length, 2.0 * s.radius)
    |> Instance.rotateX(Angle.radians(Math.asin(Math.clamp(0.0 - 1.0, 1.0, u.z))))
    |> Instance.rotateZ(Angle.radians(Math.atan2(0.0 - u.x, u.y)))

/// The instances drawing `caps` posed `posed`. A chain's radii shrink toward
/// its tip, so a capsule's start cap lies inside the one before's end cap:
/// only a chain's first start cap is drawn (`starts`, then every end).
let partOf = (posed: Posed, caps: List<Capsule>): Part =>
  let segs = caps |> List.map((cap) => let s = segOf(posed, cap) in { s with a: drawn(s.a), b: drawn(s.b) }) in
  let firsts = caps |> List.zip(segs) |> List.filter((cs) => let (cap, _) = cs in starts(cap)) |> List.map((cs) => let (_, s) = cs in capAt(s.a, s.radius)) in
  { caps: firsts |> List.append(segs |> List.map((s) => capAt(s.b, s.radius))), cylinders: segs |> List.map(cylinderOf) }

/// A part painted `c`.
let partScene = (c: Color.t, part: Part): Scene.t =>
  Scene.group([
    Scene.sphere() |> Scene.lit(c) |> Scene.instanced(part.caps),
    Scene.cylinder() |> Scene.lit(c) |> Scene.instanced(part.cylinders),
  ])

// The palm and thumb's capsules (the metacarpals' and the thumb's); each
// finger's first joint (index, middle, ring, little), and its distal two
// phalanges' capsules.
let inPalm = (c: Capsule): bool => c.joint < 6.0 || starts(c)
let fingerFirsts = [6.0, 11.0, 16.0, 21.0]
let proximalCap = (first: float): Capsule =>
  capsules |> List.find((c) => c.joint == first + 1.0) |> Option.defaultValue({ joint: first + 1.0, radius: 0.0, length: 0.0 })
let distalCaps = fingerFirsts |> List.map((first) => capsules |> List.filter((c) => c.joint > first + 1.0 && c.joint < first + 4.0))

/// The palm and thumb's part, painted `c`, the thumb posed `t`.
let palmAndThumb = (c: Color.t, side: Hold.Side, t: Thumb): Scene.t =>
  partScene(c, partOf(local(side, { flat with thumb: t }), capsules |> List.filter(inPalm)))

// A level's step moves a straight finger's tip about 3 mm.
let drawLevels = 64.0
let level = (curl: float): float => Math.round(Math.clamp01(curl) * drawLevels)

/// The fingers' levels as drawn (index, middle, ring, little), each its curl
/// and tip to the nearest 1/64.
let levels = (f: FingerPose): List<(float, float)> =>
  [f.index, f.middle, f.ring, f.little] |> List.map((b) => (level(b.curl), level(b.tip)))

/// A finger's proximal bone drawn at a curl: its part, and the frame its
/// distal two hang from (`hangFrom`).
type Proximal = { part: Part, frame: Input.pose }

/// Each hand curled to every level (`curled(k / drawLevels)`): what the
/// drawing tables (here and `Skin`'s) are read from.
let levelPoses = (side: Hold.Side): List<Posed> => List.range(drawLevels + 1.0) |> List.map((k) => local(side, curled(k / drawLevels)))
let leftLevels = levelPoses(Hold.Left)
let rightLevels = levelPoses(Hold.Right)
let levelsOf = (side: Hold.Side): List<Posed> => if side == Hold.Left then leftLevels else rightLevels

/// Per level, each finger's proximal bone at that curl (`proximal`), its
/// distal two's capsules at that tip in the frame they hang from (`distal`),
/// and the whole finger bent evenly to it (`even`: curl and tip alike, as
/// most poses bend a finger, drawn without composing).
let fingerTables = (side: Hold.Side): (List<List<Proximal>>, List<List<List<Seg>>>, List<List<Part>>) =>
  let posed = levelsOf(side) in
  (posed |> List.map((p) => fingerFirsts |> List.map((first) =>
     { part: partOf(p, [proximalCap(first)]), frame: hangFrom(p, first) })),
   posed |> List.map((p) => fingerFirsts |> List.zip(distalCaps) |> List.map((fc) =>
     let (first, caps) = fc in
     let at = hangFrom(p, first) in
     caps |> List.map((cap) => let s = segOf(p, cap) in { s with a: Hold.localPoint(at, s.a), b: Hold.localPoint(at, s.b) }))),
   posed |> List.map((p) => fingerFirsts |> List.zip(distalCaps) |> List.map((fc) => let (first, caps) = fc in partOf(p, [proximalCap(first), ..caps]))))
let leftTables = fingerTables(Hold.Left)
let rightTables = fingerTables(Hold.Right)

expect levels(power) == [(45.0, 45.0), (48.0, 48.0), (46.0, 46.0), (38.0, 38.0)] && levels(fist) == [(64.0, 64.0), (64.0, 64.0), (64.0, 64.0), (64.0, 64.0)]
  && levels({ flat with index: { curl: 0.0 - 1.0, tip: 2.0 } }) == [(0.0, 64.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]

/// A table's entry at level `k` for finger `finger` (0 the index).
let cell = (table: List<List<'a>>, k: float, finger: float): Option.t<'a> => List.nth(k, table) |> Option.bind((fingers) => List.nth(finger, fingers))

/// A finger (0 the index) at its levels (`lv`, its curl's and tip's): its
/// proximal bone at its curl's, and its distal two at its tip's, as drawn
/// (hung from the proximal).
let composed = (side: Hold.Side, finger: float, lv: (float, float)): Option.t<(Proximal, List<Seg>)> =>
  let (proximal, distal, _) = if side == Hold.Left then leftTables else rightTables in
  let (lc, lt) = lv in
  cell(proximal, lc, finger) |> Option.bind((p) => cell(distal, lt, finger) |> Option.map((segs) =>
    let f = p.frame in
    let place = (q: Input.point3) => drawn(Quat.add(f.position, Quat.rotate(f.orientation, q))) in
    (p, segs |> List.map((s) => { s with a: place(s.a), b: place(s.b) }))))

/// The fingers' part, painted `c`, at their levels (`levels`): a finger bent
/// evenly from `even`, any other `composed`.
let fingersAt = (c: Color.t, side: Hold.Side, at: List<(float, float)>): Scene.t =>
  let (_, _, even) = if side == Hold.Left then leftTables else rightTables in
  let none: Part = { caps: [], cylinders: [] } in
  let parts = at |> List.indexedMap((finger, lv) =>
    let (lc, lt) = lv in
    if lc == lt then cell(even, lc, finger) |> Option.defaultValue(none)
    else composed(side, finger, lv) |> Option.map((pd) =>
      let (p, segs) = pd in
      { caps: p.part.caps |> List.append(segs |> List.map((s) => capAt(s.b, s.radius))),
        cylinders: p.part.cylinders |> List.append(segs |> List.map(cylinderOf)) }) |> Option.defaultValue(none)) in
  partScene(c, { caps: parts |> List.concatMap((p) => p.caps), cylinders: parts |> List.concatMap((p) => p.cylinders) })

// --- checks ---

let dist = (a: Input.point3, b: Input.point3): float => Quat.length(Quat.sub(a, b))
let at = (posed: Posed, j: float): Input.point3 => (jointOf(posed, j)).position
/// A pose bending every digit's curl and tip apart, for checks.
let uneven: FingerPose = { thumb: { curl: 0.3, tip: 0.9, opposition: 0.6 }, index: { curl: 0.2, tip: 0.9 }, middle: { curl: 0.8, tip: 0.1 }, ring: { curl: 0.5, tip: 1.0 }, little: { curl: 0.0, tip: 0.6 } }
let posesOf = [flat, fist, curled(0.4), power, pinch, palmOpen, onTrigger(0.5), empty(0.3, 0.8), uneven]

/// `f` with each finger's curl and tip at its drawn level (`levels`).
let atLevels = (f: FingerPose): FingerPose =>
  let k = (b: Bend): Bend => { curl: level(b.curl) / drawLevels, tip: level(b.tip) / drawLevels } in
  { f with index: k(f.index), middle: k(f.middle), ring: k(f.ring), little: k(f.little) }

// One curl a digit (`uniform`, `uniformThumb`) is the hand as it was posed
// by one curl: each finger's MCP, PIP and DIP and the thumb's CMC, MCP and
// IP bent that times a fist's, exactly.
expect (
  [Hold.Left, Hold.Right] |> List.all((side) => [0.0, 0.3, 0.75, 1.0] |> List.all((c) =>
    let p = local(side, curled(c)) in
    let inRoot = (q: Input.pose) => let r = Hold.compose(wristOnGrip, q) in if side == Hold.Left then mirror(r) else r in
    let joints = (bones: List<(Input.pose, Input.quaternion)>) => bones |> List.map((jr) => let (q, _) = jr in inRoot(q)) in
    let thumb = chain(Hold.at(thumbBase, thumbTurn(c)), [(thumbMetacarpal, c * thumbCmcFlex), (thumbProximal, c * thumbMcpFlex), (thumbDistal, c * thumbIpFlex)]) in
    let finger = (f: Finger) => chain(Hold.at(f.base, Quat.yaw(f.splay)), [(f.metacarpal, 0.0), (f.proximal, c * mcpFlex), (f.intermediate, c * pipFlex), (f.distal, c * dipFlex)]) in
    List.drop(2.0, p.poses) == List.flatten([joints(thumb), joints(finger(index)), joints(finger(middle)), joints(finger(ring)), joints(finger(little))])))
)
// A tip bends only its own joints past the knuckle: an index's tip moves
// its intermediate bone's turn (not its place) and the joints past it; the
// thumb's, its distal's and its tip. Nothing else moves.
expect (
  let moved = (a: FingerPose, b: FingerPose) =>
    let (pa, pb) = (local(Hold.Right, a), local(Hold.Right, b)) in
    List.range(26.0) |> List.filter((j) => not (Hold.near(jointOf(pa, j), jointOf(pb, j)))) in
  let turnedOnly = (a: FingerPose, b: FingerPose, j: float) => Hold.near3(at(local(Hold.Right, a), j), at(local(Hold.Right, b), j)) in
  moved(power, { power with index: { curl: 0.70, tip: 0.2 } }) == [8.0, 9.0, 10.0]
    && turnedOnly(power, { power with index: { curl: 0.70, tip: 0.2 } }, 8.0)
    && moved(power, { power with thumb: { power.thumb with tip: 0.2 } }) == [4.0, 5.0]
    && turnedOnly(power, { power with thumb: { power.thumb with tip: 0.2 } }, 4.0)
    && moved(power, { power with index: { curl: 0.2, tip: 0.70 } }) == [7.0, 8.0, 9.0, 10.0]
)
// The drawn fingers' distal two (the tip's table, hung from the curl's)
// are each capsule FK puts there at those levels, either hand.
expect (
  [Hold.Left, Hold.Right] |> List.all((side) => [power, uneven, onTrigger(0.3), fist, flat] |> List.all((f) =>
    let p = local(side, atLevels(f)) in
    let want = List.flatten(distalCaps) |> List.map((cap) => let s = segOf(p, cap) in (drawn(s.a), drawn(s.b))) in
    let got = levels(f) |> List.indexedMap((finger, lv) => composed(side, finger, lv)) |> List.concatMap((o) => o |> Option.map((pd) => let (_, segs) = pd in segs) |> Option.defaultValue([])) |> List.map((s) => (s.a, s.b)) in
    List.length(got) == 8.0 && (List.zip(want, got) |> List.all((gw) => let (g, w) = gw in let (ga, gb) = g in let (wa, wb) = w in Hold.near3(ga, wa) && Hold.near3(gb, wb)))))
)

expect List.length(names) == 26.0 && List.length(parents) == 26.0
expect (let p = local(Hold.Right, power) in List.length(p.poses) == 26.0 && List.length(p.rotations) == 26.0)
// Every joint's parent comes before it, but the palm's (OpenXR lists it
// first); the wrist's is the root.
expect (parents |> List.indexedMap((i, p) => if i == palm then p == wrist else if i == wrist then p < 0.0 else p >= 0.0 && p < i) |> List.all((ok) => ok))

// FK keeps every bone's length, whatever the pose, either hand, on any root:
// each joint is its parent's bone length from it.
let boneLengths = List.flatten([
  [(palm, wrist, Quat.length(palmInWrist.position))],
  [(2.0, wrist, Quat.length(thumbBase)), (3.0, 2.0, thumbMetacarpal), (4.0, 3.0, thumbProximal), (5.0, 4.0, thumbDistal)],
  [(index, 6.0), (middle, 11.0), (ring, 16.0), (little, 21.0)] |> List.concatMap((fj) =>
    let (f, j) = fj in
    [(j, wrist, Quat.length(f.base)), (j + 1.0, j, f.metacarpal), (j + 2.0, j + 1.0, f.proximal), (j + 3.0, j + 2.0, f.intermediate), (j + 4.0, j + 3.0, f.distal)]),
])
let somewhere = Hold.at(Quat.v3(0.3, 1.1, -0.4), Quat.mul(Quat.yaw(0.8), Quat.axisAngle(Quat.v3(0.6, 0.0, 0.8), 0.5)))
expect List.length(boneLengths) == 25.0
expect (
  [Hold.Left, Hold.Right] |> List.all((side) => posesOf |> List.all((f) =>
    let p = pose(side, somewhere, f) in
    boneLengths |> List.all((b) => let (j, parent, length) = b in Math.abs(dist(at(p, j), at(p, parent)) - length) < 0.000001)))
)
// Each joint's rotation is its parent's turned by its own.
expect (
  let p = pose(Hold.Left, somewhere, onTrigger(0.7)) in
  List.range(26.0) |> List.all((j) =>
    let parent = List.nth(j, parents) |> Option.defaultValue(0.0 - 1.0) in
    let r = List.nth(j, p.rotations) |> Option.defaultValue(Quat.identity) in
    let from = if parent < 0.0 then somewhere.orientation else (jointOf(p, parent)).orientation in
    Quat.angle(Quat.mul(from, r), (jointOf(p, j)).orientation) < 0.00001)
)

// Flat, a finger runs straight from its base along its splay; the middle
// fingertip is 18.1 cm ahead of the wrist (15.7 past its CMC).
expect (
  let p = local(Hold.Right, flat) in
  let tipOf = (f: Finger) => Hold.compose(wristOnGrip, Hold.at(Quat.add(f.base, Quat.rotate(Quat.yaw(f.splay), Quat.v3(0.0, 0.0, 0.0 - (f.metacarpal + f.proximal + f.intermediate + f.distal)))), Quat.identity)) in
  Hold.near3(at(p, indexTip), (tipOf(index)).position) && Hold.near3(at(p, middleTip), (tipOf(middle)).position)
    && Hold.near3(at(p, ringTip), (tipOf(ring)).position) && Hold.near3(at(p, littleTip), (tipOf(little)).position)
    && Math.abs((Hold.relative(wristOnGrip, jointOf(p, middleTip))).position.z + 0.181) < 0.0000001
)
// Flat, every joint of a finger points the way its splay does, nail up.
expect (
  let p = local(Hold.Right, flat) in
  List.range(5.0) |> List.all((k) => Quat.angle((jointOf(p, 11.0 + k)).orientation, wristOnGrip.orientation) < 0.000001)
)
// A fist: every fingertip within 3.5 cm of the palm; the thumb's, across
// the curled fingers, within 6.5.
expect (
  let p = local(Hold.Right, fist) in
  ([indexTip, middleTip, ringTip, littleTip] |> List.all((t) => dist(at(p, t), at(p, palm)) < 0.035))
    && dist(at(p, thumbTip), at(p, palm)) < 0.065
)
// Curling only ever closes a finger: each fingertip nears the palm as its
// curl grows, the thumb's too as it curls and opposes.
expect (
  let reach = (c: float, t: float) => dist(at(local(Hold.Right, curled(c)), t), at(local(Hold.Right, curled(c)), palm)) in
  tips |> List.all((t) => List.range(10.0) |> List.all((i) => reach((i + 1.0) / 10.0, t) < reach(i / 10.0, t)))
)
// The left hand is the right's mirror: every joint, posed any way, on the
// mirrored root.
expect (
  posesOf |> List.all((f) =>
    let (l, r) = (pose(Hold.Left, mirror(somewhere), f), pose(Hold.Right, somewhere, f)) in
    List.zip(r.poses, l.poses) |> List.all((lr) => let (a, b) = lr in Hold.near(a, mirror(b))))
)
// …and on one root, the left's joints sit across the grip's YZ plane.
expect (let (l, r) = (local(Hold.Left, pinch), local(Hold.Right, pinch)) in Hold.near3(at(l, indexTip), (mirror(jointOf(r, indexTip))).position))

// The presets curl as meant.
// A power wrap closes round a Touch handle (up the grip's Y through its
// origin): no capsule inside it, the palm on it, and each finger within
// 1.2 cm of it.
let fromHandle = (p: Input.point3): float => Math.sqrt(p.x * p.x + p.z * p.z)
let clearance = (posed: Posed, c: Capsule): float =>
  let (a, b) = ends(posed, c) in
  (List.range(9.0) |> List.map((i) => fromHandle(Quat.lerp(a, b, i / 8.0))) |> List.minimum |> Option.defaultValue(0.0)) - c.radius - handleRadius
let fingerOf = (c: Capsule): float => Math.floor((c.joint - 6.0) / 5.0)
expect (
  let p = local(Hold.Right, power) in
  let fingers = capsules |> List.filter((c) => c.joint >= 6.0) in
  (capsules |> List.all((c) => clearance(p, c) > 0.0 - 0.001))
    && (capsules |> List.filter((c) => c.joint == 11.0) |> List.all((c) => clearance(p, c) < 0.003))
    && (List.range(4.0) |> List.all((f) => fingers |> List.filter((c) => fingerOf(c) == f) |> List.any((c) => clearance(p, c) < 0.012)))
)
// The trigger pulls the index in and nothing else.
expect (
  let (a, b) = (local(Hold.Right, onTrigger(0.0)), local(Hold.Right, onTrigger(1.0))) in
  dist(at(b, indexTip), at(b, palm)) < dist(at(a, indexTip), at(a, palm)) - 0.01
    && Hold.near3(at(a, middleTip), at(b, middleTip)) && Hold.near3(at(a, thumbTip), at(b, thumbTip))
)
// A pinch brings the thumb's tip to the index's: their pads touch.
expect (let p = local(Hold.Right, pinch) in dist(at(p, thumbTip), at(p, indexTip)) < 0.01)
// An open palm keeps the fingertips out.
expect (let p = local(Hold.Right, palmOpen) in [indexTip, middleTip, ringTip, littleTip] |> List.all((t) => dist(at(p, t), at(p, palm)) > 0.08))
// Empty: the squeeze curls the middle, ring and little fingers round the
// handle (no further), the trigger the index; neither moves the other.
expect (
  let open0 = empty(0.0, 0.0) in
  let squeezed = empty(0.0, 1.0) in
  let pulled = empty(1.0, 0.0) in
  squeezed.middle == power.middle && squeezed.ring == power.ring && squeezed.little == power.little && squeezed.index == open0.index
    && pulled.index.curl > open0.index.curl && pulled.middle == open0.middle && open0.thumb == { curl: 0.5, tip: 0.5, opposition: 0.25 }
)
