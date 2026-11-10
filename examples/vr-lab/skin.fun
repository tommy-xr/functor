// Skin: the skinned glove. WebXR's `generic-hand` models (`Assets.handLeft`
// / `handRight`) posed by the capsule glove's forward kinematics (`Hand`).
// Every joint of those models hangs off its root, so each skinned joint is
// placed outright (`Anim.place`) where FK puts it, turned by a fixed
// correction: how the model's joint frame sits on its own bone. The models
// follow OpenXR's joint convention as `Hand` does, so the corrections are
// small (the modeller's joints a few degrees off their bones). Joint
// positions are FK's exactly; each bone's skin keeps its shape, so where
// FK's bone lengths differ from the model's (up to 1.4 cm, the thumb's
// metacarpal) the skin between two joints stretches or overlaps.

/// The skinned joints, as indices into `Hand.names`: the wrist, the thumb's
/// three bones and each finger's four. The palm and the tips carry no skin.
let skinned = [1.0, 2.0, 3.0, 4.0, 6.0, 7.0, 8.0, 9.0, 11.0, 12.0, 13.0, 14.0, 16.0, 17.0, 18.0, 19.0, 21.0, 22.0, 23.0, 24.0]
let thumbJoints = [2.0, 3.0, 4.0]
let fingerJoints = [6.0, 11.0, 16.0, 21.0] |> List.map((first) => [first, first + 1.0, first + 2.0, first + 3.0])

/// A model's joints, in `Hand.names` order without the palm.
let jointNames = (j: Assets.HandLeftJoints): List<string> => [
  j.wrist, j.thumb_metacarpal, j.thumb_phalanx_proximal, j.thumb_phalanx_distal, j.thumb_tip,
  j.index_finger_metacarpal, j.index_finger_phalanx_proximal, j.index_finger_phalanx_intermediate, j.index_finger_phalanx_distal, j.index_finger_tip,
  j.middle_finger_metacarpal, j.middle_finger_phalanx_proximal, j.middle_finger_phalanx_intermediate, j.middle_finger_phalanx_distal, j.middle_finger_tip,
  j.ring_finger_metacarpal, j.ring_finger_phalanx_proximal, j.ring_finger_phalanx_intermediate, j.ring_finger_phalanx_distal, j.ring_finger_tip,
  j.pinky_finger_metacarpal, j.pinky_finger_phalanx_proximal, j.pinky_finger_phalanx_intermediate, j.pinky_finger_phalanx_distal, j.pinky_finger_tip,
]

expect jointNames(Assets.handLeftJoints) == List.drop(1.0, Hand.names) && jointNames(Assets.handRightJoints) == List.drop(1.0, Hand.names)

let nameOf = (j: float): string => List.nth(j, Hand.names) |> Option.defaultValue("")

// --- the models' bind poses ---
// Each joint's bind pose in its model's space, in `jointNames` order: the
// nodes' translations and rotations in `generic-hand` 1.0.20's left.glb and
// right.glb (`Assets.handLeft` / `handRight`).
// (The rotations are rounded to six places: normalized here.)
let bind = (x: float, y: float, z: float, qx: float, qy: float, qz: float, qw: float): Input.pose =>
  let n = Math.sqrt(qx * qx + qy * qy + qz * qz + qw * qw) in
  Hold.at(Quat.v3(x, y, z), { x: qx / n, y: qy / n, z: qz / n, w: qw / n })

let leftBind: List<Input.pose> = [
  bind(-0.037324, 0.055986, 0.008114, -0.500000, 0.500000, 0.500000, 0.500000),
  bind(-0.018166, 0.020028, -0.019915, -0.470325, -0.128226, 0.241300, 0.839123),
  bind(-0.003017, -0.003232, -0.036842, -0.418900, -0.030170, 0.255136, 0.870930),
  bind(0.006388, -0.027137, -0.058799, -0.341943, -0.108182, 0.211520, 0.909193),
  bind(0.011588, -0.035692, -0.069785, -0.344979, -0.112945, 0.208098, 0.908255),
  bind(-0.031836, 0.019583, -0.001378, -0.453265, 0.443597, 0.567030, 0.525594),
  bind(-0.030008, -0.036010, -0.015437, -0.544424, 0.472279, 0.495467, 0.484839),
  bind(-0.026774, -0.077768, -0.013908, -0.538231, 0.489196, 0.505236, 0.464469),
  bind(-0.024555, -0.101934, -0.012578, -0.513785, 0.540051, 0.461053, 0.481456),
  bind(-0.025165, -0.113431, -0.011311, -0.507647, 0.541658, 0.462514, 0.484750),
  bind(-0.031618, 0.026880, 0.004855, -0.475765, 0.517914, 0.536281, 0.466707),
  bind(-0.034781, -0.035660, 0.006388, -0.542233, 0.500071, 0.505933, 0.447152),
  bind(-0.030303, -0.082129, 0.010754, -0.539428, 0.510322, 0.505816, 0.439020),
  bind(-0.027531, -0.109369, 0.013800, -0.492846, 0.586054, 0.454965, 0.454589),
  bind(-0.028561, -0.121421, 0.015416, -0.483768, 0.579913, 0.466028, 0.460964),
  bind(-0.031618, 0.028996, 0.016380, -0.490272, 0.599352, 0.518919, 0.362123),
  bind(-0.030795, -0.028708, 0.025579, -0.553353, 0.557948, 0.486226, 0.382204),
  bind(-0.026439, -0.070328, 0.034875, -0.541336, 0.576459, 0.496761, 0.357601),
  bind(-0.023156, -0.095902, 0.041306, -0.515743, 0.580167, 0.502349, 0.380869),
  bind(-0.022382, -0.107586, 0.043637, -0.511453, 0.573781, 0.503820, 0.394152),
  bind(-0.027904, 0.021913, 0.031113, -0.472767, 0.638673, 0.533652, 0.289489),
  bind(-0.023633, -0.017910, 0.043168, -0.523980, 0.587587, 0.516337, 0.337020),
  bind(-0.019207, -0.051417, 0.050481, -0.518079, 0.627426, 0.497943, 0.299973),
  bind(-0.016352, -0.070417, 0.057069, -0.482037, 0.621573, 0.510483, 0.347410),
  bind(-0.015562, -0.081921, 0.060045, -0.478420, 0.619371, 0.515121, 0.349491),
]
let rightBind: List<Input.pose> = [
  bind(0.039126, 0.055775, 0.009157, -0.500000, -0.500000, -0.500000, 0.500000),
  bind(0.019968, 0.019817, -0.018871, -0.464330, 0.158257, -0.209365, 0.845883),
  bind(0.004818, -0.003443, -0.035799, -0.418267, 0.035194, -0.255945, 0.870808),
  bind(-0.004586, -0.027348, -0.057755, -0.344972, 0.108748, -0.208582, 0.908659),
  bind(-0.009786, -0.035903, -0.068741, -0.343486, 0.114166, -0.214832, 0.907100),
  bind(0.032062, 0.026563, -0.001006, -0.480212, -0.454859, -0.521946, 0.538584),
  bind(0.031810, -0.032773, -0.014394, -0.542580, -0.463695, -0.502931, 0.487499),
  bind(0.028576, -0.077979, -0.012865, -0.537272, -0.486956, -0.507620, 0.465333),
  bind(0.026357, -0.102145, -0.011534, -0.507597, -0.542286, -0.463131, 0.483510),
  bind(0.026967, -0.113642, -0.010268, -0.512512, -0.547705, -0.458035, 0.477026),
  bind(0.033072, 0.026669, 0.005730, -0.462260, -0.513635, -0.541776, 0.478512),
  bind(0.036583, -0.035871, 0.007431, -0.545212, -0.501496, -0.505165, 0.442780),
  bind(0.032105, -0.082340, 0.011797, -0.547738, -0.518615, -0.496014, 0.430108),
  bind(0.029333, -0.109580, 0.014843, -0.484146, -0.578653, -0.464074, 0.464111),
  bind(0.030363, -0.121632, 0.016460, -0.480411, -0.576434, -0.468972, 0.465827),
  bind(0.033072, 0.028785, 0.017255, -0.470717, -0.585441, -0.530684, 0.392504),
  bind(0.032597, -0.028918, 0.026622, -0.554501, -0.553794, -0.488066, 0.384229),
  bind(0.028241, -0.070539, 0.035918, -0.545993, -0.578631, -0.491282, 0.354569),
  bind(0.024957, -0.096112, 0.042349, -0.512239, -0.576574, -0.503119, 0.389929),
  bind(0.024184, -0.107797, 0.044680, -0.510669, -0.577271, -0.505771, 0.387519),
  bind(0.029706, 0.021702, 0.032156, -0.459375, -0.642968, -0.534767, 0.299319),
  bind(0.025435, -0.018120, 0.044211, -0.518028, -0.580733, -0.524518, 0.345365),
  bind(0.021009, -0.051627, 0.051525, -0.515963, -0.626419, -0.499638, 0.302893),
  bind(0.018154, -0.070628, 0.058112, -0.484381, -0.621213, -0.509596, 0.346094),
  bind(0.017364, -0.082131, 0.061088, -0.498105, -0.631593, -0.496324, 0.326564),
]

// --- the corrections ---
// A joint's bone frame: -Z along its bone to the joint it leads to (the
// wrist's, the middle finger's metacarpal), +Y its own +Y made square to
// that. FK's joints and the models' both mean -Z down the bone and +Y the
// nail, so a joint's frame is its bone frame turned by an offset: none for
// FK's fingers and thumb, a few degrees for the models'.

/// The joint `j`'s bone leads to.
let childOf = (j: float): float => if j == Hand.wrist then 11.0 else j + 1.0

let normalize = (v: Input.point3): Input.point3 => Quat.scale(1.0 / Quat.length(v), v)

/// The bone frame of joint `p`, whose bone ends at `child`.
let boneFrame = (p: Input.pose, child: Input.point3): Input.quaternion =>
  let z = normalize(Quat.sub(p.position, child)) in
  let up = Quat.rotate(p.orientation, Quat.v3(0.0, 1.0, 0.0)) in
  let y = normalize(Quat.sub(up, Quat.scale(Quat.dot(up, z), z))) in
  Quat.fromBasis(Quat.cross(y, z), y, z)

/// Joint `j`'s frame in its bone frame, its skeleton's joints by `Hand`
/// index (`at`).
let offsetOf = (at: (float) => Input.pose, j: float): Input.quaternion =>
  let p = at(j) in
  Quat.mul(Quat.conj(boneFrame(p, (at(childOf(j))).position)), p.orientation)

let bindOf = (side: Hold.Side) => if side == Hold.Left then leftBind else rightBind
let bindAt = (side: Hold.Side, j: float): Input.pose => List.nth(j - 1.0, bindOf(side)) |> Option.defaultValue(Hold.origin)

// FK's straight hand, and a model's bind, each by `Hand` index.
let modelOffset = (side: Hold.Side, j: float) => offsetOf((k) => bindAt(side, k), j)
let fkOffset = (side: Hold.Side, j: float) => offsetOf((k) => Hand.jointOf(Hand.local(side, Hand.flat), k), j)

/// Each skinned joint's correction (in `skinned` order): the model's joint
/// frame in FK's joint frame, once both stand on the same bone. A skinned
/// joint's frame is FK's turned by it.
let corrections = (side: Hold.Side): List<Input.quaternion> =>
  skinned |> List.map((j) => Quat.mul(Quat.conj(fkOffset(side, j)), modelOffset(side, j)))
let leftCorrections = corrections(Hold.Left)
let rightCorrections = corrections(Hold.Right)

// --- driving the models ---

// A model's space is the hand's subtree under `Scene.atPose` at the grip:
// the grip's frame turned half round +Y (as `Hand.partOf` draws).
let toModel = Hold.at(Quat.v3(0.0, 0.0, 0.0), Quat.yaw(Math.pi))

/// FK's joint `j` (`posed`, in the grip's frame) as the model's: where FK
/// puts it, turned by its correction `c`.
let placed = (posed: Hand.Posed, j: float, c: Input.quaternion): (string, Input.pose) =>
  let p = Hand.jointOf(posed, j) in
  (nameOf(j), Hold.compose(toModel, Hold.at(p.position, Quat.mul(p.orientation, c))))

let correctionOf = (side: Hold.Side, j: float): Input.quaternion =>
  let cs = if side == Hold.Left then leftCorrections else rightCorrections in
  List.range(List.length(skinned)) |> List.find((i) => List.nth(i, skinned) == Option.Some(j))
    |> Option.bind((i) => List.nth(i, cs)) |> Option.defaultValue(Quat.identity)

/// The placements of joints `js` of a hand posed `posed`.
let placements = (side: Hold.Side, posed: Hand.Posed, js: List<float>): List<(string, Input.pose)> =>
  js |> List.map((j) => placed(posed, j, correctionOf(side, j)))

/// Every skinned joint of a hand posed `f` (what the drawing's quantized
/// parts add up to, at the fingers' drawn levels).
let pose = (side: Hold.Side, f: Hand.FingerPose): List<(string, Input.pose)> =>
  placements(side, Hand.local(side, f), skinned)

// --- drawing ---
// As the capsule glove is: the wrist fixed, the thumb posed as given (three
// joints from its chain), and each finger at its drawn level, read from a
// table built at load.

let wristOf = (side: Hold.Side) => placements(side, Hand.local(side, Hand.flat), [Hand.wrist])
let leftWrist = wristOf(Hold.Left)
let rightWrist = wristOf(Hold.Right)

/// The thumb's placements, curled and opposed by `t`: only its chain.
let thumbOf = (side: Hold.Side, t: Hand.Thumb): List<(string, Input.pose)> =>
  let inGrip = Hand.thumbChain(t) |> List.map((jr) => let (p, _) = jr in Hold.compose(Hand.wristOnGrip, p)) in
  let mirrored = if side == Hold.Left then inGrip |> List.map(Hand.mirror) else inGrip in
  thumbJoints |> List.indexedMap((i, j) =>
    let p = List.nth(i, mirrored) |> Option.defaultValue(Hold.origin) in
    (nameOf(j), Hold.compose(toModel, Hold.at(p.position, Quat.mul(p.orientation, correctionOf(side, j))))))

// The thumbs `Hand`'s presets pose, placed at load: a thumb's chain costs
// about 85 µs a redraw, and the thumb only ever takes these (any other is
// placed as drawn).
let thumbPresets = [Hand.flat.thumb, Hand.fist.thumb, Hand.power.thumb, Hand.pinch.thumb, Hand.palmOpen.thumb, (Hand.empty(0.0, 0.0)).thumb]
let presetThumbs = (side: Hold.Side) => thumbPresets |> List.map((t) => (t, thumbOf(side, t)))
let leftThumbs = presetThumbs(Hold.Left)
let rightThumbs = presetThumbs(Hold.Right)

/// The thumb's placements, curled and opposed by `t`.
let thumbAt = (side: Hold.Side, t: Hand.Thumb): List<(string, Input.pose)> =>
  match (if side == Hold.Left then leftThumbs else rightThumbs) |> List.find((tp) => let (u, _) = tp in u == t) with
  | Option.Some(tp) => let (_, ps) = tp in ps
  | Option.None => thumbOf(side, t)

/// Each hand's finger placements at every level: per level, the four
/// fingers' four joints.
let fingerTable = (side: Hold.Side): List<List<List<(string, Input.pose)>>> =>
  List.range(Hand.drawLevels + 1.0) |> List.map((k) =>
    let posed = Hand.local(side, Hand.curled(k / Hand.drawLevels)) in
    fingerJoints |> List.map((js) => placements(side, posed, js)))
let leftFingers = fingerTable(Hold.Left)
let rightFingers = fingerTable(Hold.Right)

/// A hand's placements: the thumb posed `t`, the fingers at `levels`.
let drawn = (side: Hold.Side, t: Hand.Thumb, levels: (float, float, float, float)): List<(string, Input.pose)> =>
  let (li, lm, lr, ll) = levels in
  let table = if side == Hold.Left then leftFingers else rightFingers in
  let pick = (finger: float, k: float) =>
    List.nth(k, table) |> Option.bind((parts) => List.nth(finger, parts)) |> Option.defaultValue([]) in
  List.flatten([if side == Hold.Left then leftWrist else rightWrist, thumbAt(side, t), pick(0.0, li), pick(1.0, lm), pick(2.0, lr), pick(3.0, ll)])

/// The skinned glove, in its grip's frame (as `Scene.atPose` places it).
let scene = (side: Hold.Side, t: Hand.Thumb, levels: (float, float, float, float)): Scene.t =>
  Scene.model(if side == Hold.Left then Assets.handLeft else Assets.handRight)
    |> Scene.animate(Anim.rest() |> Anim.place(drawn(side, t, levels)))

// --- checks ---

let sides = [Hold.Left, Hold.Right]
let posesOf = [Hand.flat, Hand.fist, Hand.power, Hand.pinch, Hand.onTrigger(0.5), Hand.empty(0.3, 0.8)]
let poseAt = (ps: List<(string, Input.pose)>, j: float): Input.pose =>
  ps |> List.find((np) => let (n, _) = np in n == nameOf(j)) |> Option.map((np) => let (_, p) = np in p) |> Option.defaultValue(Hold.origin)

// The models keep OpenXR's convention: a finger's or the thumb's correction
// is at most 5.6°. The wrist's is 12°: FK's wrist and the model's lean off
// the middle metacarpal by different amounts (9.5° and 13.5°), and the wrist
// is turned to keep the model's skin square to the palm.
let degrees = (q: Input.quaternion): float => Quat.angle(q, Quat.identity) * 180.0 / Math.pi
expect (
  sides |> List.all((side) => (skinned |> List.zip(corrections(side))) |> List.all((jc) =>
    let (j, c) = jc in degrees(c) < (if j == Hand.wrist then 12.5 else 6.0)))
)

// Retargeted, the skinned bind pose is FK's straight hand: driven at curl 0,
// each skinned joint sits on FK's joint and its bone frame (its frame less
// the model's offset) is FK's, within 1e-6 m and 1e-5 rad.
expect (
  sides |> List.all((side) =>
    let fk = Hand.local(side, Hand.flat) in
    let ps = pose(side, Hand.flat) in
    skinned |> List.all((j) =>
      let got = Hold.relative(toModel, poseAt(ps, j)) in
      let want = Hand.jointOf(fk, j) in
      Quat.length(Quat.sub(got.position, want.position)) < 0.000001
        && Quat.angle(Quat.mul(got.orientation, Quat.conj(modelOffset(side, j))), Quat.mul(want.orientation, Quat.conj(fkOffset(side, j)))) < 0.00001))
)
// In any pose, each skinned bone runs at FK's next joint: the model's -Z
// down each bone (its frame less its offset) points at the joint FK puts
// there, within 1e-5 rad.
expect (
  sides |> List.all((side) => posesOf |> List.all((f) =>
    let fk = Hand.local(side, f) in
    let ps = pose(side, f) in
    skinned |> List.all((j) =>
      let got = Hold.relative(toModel, poseAt(ps, j)) in
      let down = Quat.rotate(Quat.mul(got.orientation, Quat.conj(modelOffset(side, j))), Quat.v3(0.0, 0.0, -1.0)) in
      let bone = normalize(Quat.sub((Hand.jointOf(fk, childOf(j))).position, got.position)) in
      Quat.length(Quat.sub(down, bone)) < 0.00001)))
)
// The left hand is the right's mirror: its placements, posed any way, the
// right's across the model's YZ plane; and its corrections the right's
// mirrored, within 5° (the two models are not quite mirror images: 4.6° at
// most, the ring finger's metacarpal).
expect (
  posesOf |> List.all((f) =>
    (pose(Hold.Left, f) |> List.zip(pose(Hold.Right, f))) |> List.all((lr) =>
      let (left, right) = lr in
      let (ln, l) = left in
      let (rn, r) = right in
      ln == rn && Quat.length(Quat.sub(l.position, Quat.v3(0.0 - r.position.x, r.position.y, r.position.z))) < 0.000001))
)
expect ((leftCorrections |> List.zip(rightCorrections)) |> List.all((lr) => let (l, r) = lr in degrees(Quat.mul(Quat.conj(l), Hand.mirrorTurn(r))) < 5.0))
// The drawn parts add up to `pose` at the fingers' drawn levels.
expect (
  sides |> List.all((side) => [Hand.power, Hand.pinch, Hand.empty(0.3, 0.8)] |> List.all((f) =>
    let k = (c: float) => Hand.level(c) / Hand.drawLevels in
    let at = { f with index: k(f.index), middle: k(f.middle), ring: k(f.ring), little: k(f.little) } in
    (drawn(side, f.thumb, Hand.levels(f)) |> List.zip(pose(side, at))) |> List.all((ab) =>
      let (x, y) = ab in
      let (an, a) = x in
      let (bn, b) = y in
      an == bn && Hold.near(a, b))))
)
expect List.length(drawn(Hold.Left, Hand.power.thumb, Hand.levels(Hand.power))) == 20.0
