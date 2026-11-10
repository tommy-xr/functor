// Hand: an articulated hand as data. The 26 joints of OpenXR's
// XR_EXT_hand_tracking, in its order; an average adult hand's bones; a
// finger pose (`FingerPose`: how far each finger curls) and the forward
// kinematics from it; where the hand sits on a controller; and the capsules
// it is drawn and (later) fitted with. Pure, so it is tested below with no
// world.
//
// Joint frames follow OpenXR: each joint sits at the base of its bone, -Z
// along the bone toward the fingertip, +Y out of the back of the hand (the
// nail side), +X = Y × Z (a tip's frame is its distal bone's). So hand
// tracking and the WebXR `generic-hand` model map onto these with no
// retargeting. The right hand is authored; the left is its mirror across the
// root's YZ plane.
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

/// How far the thumb curls (0 straight, 1 closed) and how far it is turned
/// across the palm (0 at rest, 1 opposed).
type Thumb = { curl: float, opposition: float }

/// How far each finger curls, 0 (straight) to 1 (a fist). What a grip
/// fitter outputs and the drawing reads.
type FingerPose = { thumb: Thumb, index: float, middle: float, ring: float, little: float }

/// Straight fingers, the thumb at rest: the skeleton's bind pose.
let flat: FingerPose = { thumb: { curl: 0.0, opposition: 0.0 }, index: 0.0, middle: 0.0, ring: 0.0, little: 0.0 }
/// A fist, the thumb across it.
let fist: FingerPose = { thumb: { curl: 1.0, opposition: 1.0 }, index: 1.0, middle: 1.0, ring: 1.0, little: 1.0 }

// An open hand's curl, relaxed; and the index on a trigger, at rest and
// pulled.
let relaxed = 0.15
let triggerRest = 0.25
let triggerPulled = 0.55

// From `a` to `b` as `t` goes 0 to 1 (clamped).
let mix = (a: float, b: float, t: float): float => Math.lerp(b, Math.clamp01(t), a)

// The presets, until a grip fitter fits the fingers to what they hold.

/// A power wrap round a handle (stick, bottle neck, foregrip, fore-end,
/// magazine): each finger curled as far as it goes without entering a
/// Touch handle, the thumb opposed round its back.
let power: FingerPose = { thumb: { curl: 0.75, opposition: 1.0 }, index: 0.70, middle: 0.75, ring: 0.72, little: 0.60 }
/// A pistol grip: the index on the trigger (`trigger` 0..1 pulls it), the
/// rest wrapped.
let onTrigger = (trigger: float): FingerPose => { power with index: mix(triggerRest, triggerPulled, trigger) }
/// A pinch, thumb to index (a round, a shell), the rest tucked.
let pinch: FingerPose = { thumb: { curl: 0.5, opposition: 0.1 }, index: 0.5, middle: 0.55, ring: 0.6, little: 0.6 }
/// An open palm: the ball, the brick.
let palmOpen: FingerPose = { thumb: { curl: 0.2, opposition: 0.3 }, index: 0.18, middle: 0.2, ring: 0.22, little: 0.25 }
/// An empty hand on its controller: the grip (`squeeze`) curls the middle,
/// ring and little fingers from relaxed to wrapped round the handle, the
/// trigger the index; the thumb rests half curled, little opposed.
let empty = (trigger: float, squeeze: float): FingerPose =>
  { thumb: { curl: 0.5, opposition: 0.25 }, index: mix(relaxed, triggerPulled, trigger),
    middle: mix(relaxed, power.middle, squeeze), ring: mix(relaxed, power.ring, squeeze), little: mix(relaxed, power.little, squeeze) }

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

let fingerChain = (f: Finger, curl: float) =>
  chain(Hold.at(f.base, Quat.yaw(f.splay)),
    [(f.metacarpal, 0.0), (f.proximal, curl * mcpFlex), (f.intermediate, curl * pipFlex), (f.distal, curl * dipFlex)])

let thumbTurn = (opposition: float): Input.quaternion =>
  let t = Math.clamp01(opposition) in
  let at = (a: float, b: float) => Math.lerp(b, t, a) in
  Quat.mul(Quat.yaw(at(thumbRest.yaw, thumbOpposed.yaw)),
    Quat.mul(flex(at(thumbRest.pitch, thumbOpposed.pitch)), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), at(thumbRest.roll, thumbOpposed.roll))))

let thumbChain = (t: Thumb) =>
  chain(Hold.at(thumbBase, thumbTurn(t.opposition)),
    [(thumbMetacarpal, t.curl * thumbCmcFlex), (thumbProximal, t.curl * thumbMcpFlex), (thumbDistal, t.curl * thumbIpFlex)])

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

/// The glove posed `posed` (in the grip's frame), painted `c`, as a subtree
/// for `Scene.atPose` at the grip: one instanced draw of the capsules'
/// cylinders and one of their caps. (`atPose`'s local +Z is the grip's -Z,
/// and its +X the grip's -X.) A chain's radii shrink toward its tip, so a
/// capsule's start cap lies inside the one before's end cap: only a chain's
/// first start cap is drawn (`starts`, then every capsule's end).
let glove = (c: Color.t, posed: Posed): Scene.t =>
  let drawn = (p: Input.point3) => Quat.v3(0.0 - p.x, p.y, 0.0 - p.z) in
  let parts = capsules |> List.map((cap) => let (a, b) = ends(posed, cap) in (drawn(a), drawn(b), cap.radius, cap.length, starts(cap))) in
  let at = (p: Input.point3) => Instance.at(Quat.toVec3(p)) in
  let firsts = parts |> List.filter((s) => let (_, _, _, _, first) = s in first) |> List.map((s) => let (a, _, r, _, _) = s in at(a) |> Instance.scale(r)) in
  let spheres = firsts |> List.append(parts |> List.map((s) => let (_, b, r, _, _) = s in at(b) |> Instance.scale(r))) in
  // A unit cylinder stands along +Y: turned about X, then Z, onto the axis.
  let cylinders = parts |> List.map((s) =>
    let (a, b, r, length, _) = s in
    let u = Quat.scale(1.0 / length, Quat.sub(b, a)) in
    at(Quat.lerp(a, b, 0.5)) |> Instance.scaleXYZ(2.0 * r, length, 2.0 * r)
      |> Instance.rotateX(Angle.radians(Math.asin(Math.clamp(0.0 - 1.0, 1.0, u.z))))
      |> Instance.rotateZ(Angle.radians(Math.atan2(0.0 - u.x, u.y)))) in
  Scene.group([
    Scene.sphere() |> Scene.lit(c) |> Scene.instanced(spheres),
    Scene.cylinder() |> Scene.lit(c) |> Scene.instanced(cylinders),
  ])

// --- checks ---

let dist = (a: Input.point3, b: Input.point3): float => Quat.length(Quat.sub(a, b))
let at = (posed: Posed, j: float): Input.point3 => (jointOf(posed, j)).position
let curled = (c: float): FingerPose => { thumb: { curl: c, opposition: c }, index: c, middle: c, ring: c, little: c }
let posesOf = [flat, fist, curled(0.4), power, pinch, palmOpen, onTrigger(0.5), empty(0.3, 0.8)]

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
    && pulled.index > open0.index && pulled.middle == open0.middle && open0.thumb == { curl: 0.5, opposition: 0.25 }
)
