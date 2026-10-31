// Everything specific to the Xbot rig: joint names, bind measurements, how a
// hand sits on a controller, and the rest pose the solver starts from. A
// different character swaps this file (and `skeleton.fun`); `body.fun` only
// reads it. Xbot's bind pose is a T-pose facing +Z, every joint's bind
// rotation is identity (so local axes are model axes), units are metres.

type Side =
  | Left
  | Right

let model = Assets.xbot
let j = Assets.xbotJoints

let bindAt = (name: string): Input.point3 =>
  Skeleton.bind
    |> List.find((b) => b.name == name)
    |> Option.map((b) => b.position)
    |> Option.defaultValue(Quat.v3(0.0, 0.0, 0.0))

let neck = bindAt(j.mixamorig_Neck)
let eyes = Quat.scale(0.5, Quat.add(bindAt(j.mixamorig_LeftEye), bindAt(j.mixamorig_RightEye)))

/// Eye height of the unscaled rig (1.66 m): calibration scales by the user's.
let eyeHeight = eyes.y

let head = bindAt(j.mixamorig_Head)

/// The eyes relative to the head joint, and the head relative to the neck.
let eyeInHead = Quat.sub(eyes, head)
let headInNeck = Quat.sub(head, neck)

/// The share of the head's rotation (relative to the chest) the neck takes;
/// the skull takes the rest. Nodding about two joints moves the eyes about as
/// far forward as a person's do.
let neckBend = 0.5

/// How far ahead of the avatar's eyes the camera sits, in metres. Xbot's face
/// is 4.5 cm ahead of its eye joints, so this puts the whole head within the
/// camera's 5 cm near plane, and the head turns rigidly with the headset, so
/// no head vertex is ever drawn in first person. (Xbot has no head-less mesh,
/// and nothing culls back faces, so the head can't simply be left out.)
let faceClearance = 0.02

let spine = [j.mixamorig_Spine, j.mixamorig_Spine1, j.mixamorig_Spine2]

/// The share of the head's yaw (relative to the hips) each spine joint takes;
/// the neck takes the rest.
let spineTwist = [0.15, 0.15, 0.2]

/// The neck joint's model position with the spine twisted by `twist` radians
/// (shared out by `spineTwist`): the chain the solver hangs the avatar from.
let neckUnder = (twist: float): Input.point3 =>
  let b = spine |> List.append([j.mixamorig_Neck]) |> List.map(bindAt) in
  let (_, neckAt) =
    List.zip(spineTwist, List.zip(List.drop(1.0, b), List.take(3.0, b)))
      |> List.fold((acc, step) =>
           let (angle, at) = acc in
           let (link, share) = step in
           let (from, to) = link in
           let a = angle + share * twist in
           (a, Quat.add(at, Quat.rotate(Quat.yaw(a), Quat.sub(to, from)))),
         (0.0, bindAt(j.mixamorig_Spine))) in
  neckAt

let legs = [j.mixamorig_LeftUpLeg, j.mixamorig_RightUpLeg]

/// Standing legs: the idle clip, masked so it moves nothing above the hips.
let idleLegs = (seconds: float): Anim.t =>
  Anim.mask(legs, Anim.clip(Assets.xbotClips.idle.name, seconds))

/// One arm's two-bone chain.
type Arm = { shoulder: string, elbow: string, wrist: string }

let arm = (side: Side): Arm =>
  match side with
  | Left => { shoulder: j.mixamorig_LeftArm, elbow: j.mixamorig_LeftForeArm, wrist: j.mixamorig_LeftHand }
  | Right => { shoulder: j.mixamorig_RightArm, elbow: j.mixamorig_RightForeArm, wrist: j.mixamorig_RightHand }

/// Arms down with the elbows bent forward: the neutral pose a lost hand
/// returns to, and the bend side `Anim.reach` keeps. (The left arm points +X
/// in bind, the right -X; local axes are model axes.)
let restArms = (anim: Anim.t): Anim.t =>
  anim
    |> Anim.rotate(j.mixamorig_LeftArm, 0deg, 0deg, -72deg)
    |> Anim.rotate(j.mixamorig_LeftForeArm, 0deg, -35deg, 0deg)
    |> Anim.rotate(j.mixamorig_RightArm, 0deg, 0deg, 72deg)
    |> Anim.rotate(j.mixamorig_RightForeArm, 0deg, 35deg, 0deg)

/// The wrist's rotation relative to its controller's grip: maps the bind hand
/// (fingers out along ±X, palm down, thumb +Z) to a fist around the
/// controller (fingers along the grip's forward, palm in, thumb up). Grip
/// space here is the body solver's world convention: forward +Z, up +Y.
let gripToWrist = (side: Side): Input.quaternion =>
  match side with
  | Left => { x: 0.0 - 0.5, y: 0.0 - 0.5, z: 0.0 - 0.5, w: 0.5 }
  | Right => { x: 0.0 - 0.5, y: 0.5, z: 0.5, w: 0.5 }

/// Where the grip's origin sits in the wrist joint's frame. Measured on the
/// bind hand: the wrist to the middle knuckle is 9.5 cm, a fist closes about
/// 7 cm along it, and the handle sits about 3.5 cm off the palm.
let gripInWrist = (side: Side): Input.point3 =>
  match side with
  | Left => Quat.v3(0.07, 0.0 - 0.035, 0.0)
  | Right => Quat.v3(0.0 - 0.07, 0.0 - 0.035, 0.0)

/// Finger joints, base to tip: the index follows the trigger, the other
/// three the grip button.
let indexFinger = (side: Side): List<string> =>
  match side with
  | Left => [j.mixamorig_LeftHandIndex1, j.mixamorig_LeftHandIndex2, j.mixamorig_LeftHandIndex3]
  | Right => [j.mixamorig_RightHandIndex1, j.mixamorig_RightHandIndex2, j.mixamorig_RightHandIndex3]

let gripFingers = (side: Side): List<string> =>
  match side with
  | Left => [
      j.mixamorig_LeftHandMiddle1, j.mixamorig_LeftHandMiddle2, j.mixamorig_LeftHandMiddle3,
      j.mixamorig_LeftHandRing1, j.mixamorig_LeftHandRing2, j.mixamorig_LeftHandRing3,
      j.mixamorig_LeftHandPinky1, j.mixamorig_LeftHandPinky2, j.mixamorig_LeftHandPinky3,
    ]
  | Right => [
      j.mixamorig_RightHandMiddle1, j.mixamorig_RightHandMiddle2, j.mixamorig_RightHandMiddle3,
      j.mixamorig_RightHandRing1, j.mixamorig_RightHandRing2, j.mixamorig_RightHandRing3,
      j.mixamorig_RightHandPinky1, j.mixamorig_RightHandPinky2, j.mixamorig_RightHandPinky3,
    ]

let thumb = (side: Side): List<string> =>
  match side with
  | Left => [j.mixamorig_LeftHandThumb2, j.mixamorig_LeftHandThumb3]
  | Right => [j.mixamorig_RightHandThumb2, j.mixamorig_RightHandThumb3]

/// Curl a list of finger joints toward the palm: `amount` 0 is a relaxed
/// hand, 1 a fist. Fingers point along ±X with the palm down, so a curl
/// turns about Z (the sign follows the side).
let curl = (side: Side, names: List<string>, amount: float, anim: Anim.t): Anim.t =>
  let angle = Angle.degrees((30.0 + 55.0 * Math.clamp01(amount)) * (match side with | Left => 0.0 - 1.0 | Right => 1.0)) in
  names |> List.fold((a, name) => Anim.rotate(name, 0deg, 0deg, angle, a), anim)

/// The thumb closes over the fingers: it points along +Z, so it turns about X.
let curlThumb = (side: Side, amount: float, anim: Anim.t): Anim.t =>
  let angle = Angle.degrees(15.0 + 30.0 * Math.clamp01(amount)) in
  thumb(side) |> List.fold((a, name) => Anim.rotate(name, angle, 0deg, 0deg, a), anim)

// The grip-to-wrist rotations put the fingers along the grip's forward, the
// palm toward the body's midline and the thumb up.
expect (
  let l = gripToWrist(Left) in
  let r = gripToWrist(Right) in
  Quat.length(Quat.sub(Quat.rotate(l, Quat.v3(1.0, 0.0, 0.0)), Quat.v3(0.0, 0.0, 1.0))) < 0.0001
    && Quat.length(Quat.sub(Quat.rotate(l, Quat.v3(0.0, 0.0 - 1.0, 0.0)), Quat.v3(0.0 - 1.0, 0.0, 0.0))) < 0.0001
    && Quat.length(Quat.sub(Quat.rotate(l, Quat.v3(0.0, 0.0, 1.0)), Quat.v3(0.0, 1.0, 0.0))) < 0.0001
    && Quat.length(Quat.sub(Quat.rotate(r, Quat.v3(0.0 - 1.0, 0.0, 0.0)), Quat.v3(0.0, 0.0, 1.0))) < 0.0001
    && Quat.length(Quat.sub(Quat.rotate(r, Quat.v3(0.0, 0.0 - 1.0, 0.0)), Quat.v3(1.0, 0.0, 0.0))) < 0.0001
    && Quat.length(Quat.sub(Quat.rotate(r, Quat.v3(0.0, 0.0, 1.0)), Quat.v3(0.0, 1.0, 0.0))) < 0.0001
)

// With no twist the chain reproduces the bind neck.
expect Quat.length(Quat.sub(neckUnder(0.0), neck)) < 0.00001
expect Math.abs(eyeHeight - 1.6606) < 0.001
expect Quat.length(Quat.sub(Quat.add(neck, Quat.add(headInNeck, eyeInHead)), eyes)) < 0.000001
