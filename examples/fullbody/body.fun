// The upper-body solver, split in two:
//
// - `step(state, sample, dt)` keeps the history: calibration, the hips' lagging
//   heading, and each hand's tracking-loss blend.
// - `pose(state)` turns a state into an `Anim.t` and is pure, so one state
//   draws the local avatar and its inspection twin alike.
//
// Space: the solver's world is the rig's tracking space turned to face +Z
// (the rig camera looks down +Z from the floor origin), so tracked (x, y, z)
// is world (-x, y, -z). "Model" space is the avatar's own: origin at its
// feet, facing +Z, unscaled.

/// A position and rotation in the solver's world. A rotation is relative to a
/// frame facing +Z with +Y up.
type Pose = { position: Input.point3, rotation: Input.quaternion }

type HandSample = { grip: Option.t<Pose>, trigger: float, squeeze: float }

/// One fixed step's input. `calibrate` is a level; a rising edge recalibrates.
type Sample = { head: Option.t<Pose>, left: HandSample, right: HandSample, calibrate: bool }

/// A hand: its last wrist target in model space (kept through tracking loss,
/// so the arm never reaches for the origin), how much of it the arm follows
/// (0 = the neutral arm, 1 = tracked), and its finger curls.
type Hand = { blend: float, wrist: Option.t<Pose>, index: float, grip: float }

type State = {
  scale: float,
  calibrated: bool,
  calibrateHeld: bool,
  head: Option.t<Pose>,
  yaw: float,
  twist: float,
  root: Input.point3,
  neckRotation: Input.quaternion,
  headRotation: Input.quaternion,
  left: Hand,
  right: Hand,
  clock: float
}

let restHand: Hand = { blend: 0.0, wrist: Option.None, index: 0.0, grip: 0.0 }

let init: State = {
  scale: 1.0,
  calibrated: false,
  calibrateHeld: false,
  head: Option.None,
  yaw: 0.0,
  twist: 0.0,
  root: Quat.v3(0.0, 0.0, 0.0),
  neckRotation: Quat.identity,
  headRotation: Quat.identity,
  left: restHand,
  right: restHand,
  clock: 0.0
}

let degrees = (d: float): float => d * Math.pi / 180.0

// Head turns within this of the hips don't move them.
let deadzone = degrees(25.0)
// The hips never trail the head by more than this.
let maxTwist = degrees(70.0)
// How fast the hips catch up, in radians per second.
let turnRate = degrees(150.0)
// Seconds for a lost hand to settle into the neutral arm, and to come back.
let lossFade = 0.3
let recoverFade = 0.15
// A head that moves this far in one step was recentred, not turned.
let recenterJump = 0.25
let recenterTurn = degrees(45.0)

// --- input ---

/// Tracked (rig-local, -Z forward) to the solver's world: a half turn about Y.
let toWorld = (p: Input.pose): Pose => {
  position: Quat.v3(0.0 - p.position.x, p.position.y, 0.0 - p.position.z),
  rotation: { x: 0.0 - p.orientation.x, y: p.orientation.y, z: 0.0 - p.orientation.z, w: p.orientation.w }
}

let handSample = (c: Input.controller): HandSample => {
  grip: if c.active then c.grip |> Option.map(toWorld) else Option.None,
  trigger: c.trigger,
  squeeze: c.squeeze
}

/// An XR sample. Holding A and X together (or `calibrate`) recalibrates.
let fromXr = (calibrate: bool, xr: Input.xr): Sample => {
  head: xr.head |> Option.map(toWorld),
  left: handSample(xr.left),
  right: handSample(xr.right),
  calibrate: calibrate || (xr.left.primaryPressed && xr.right.primaryPressed)
}

// --- step ---

/// The neck's world rotation: `Rig.neckBend` of the way from the chest
/// (which takes the spine's share of the twist) to the head.
let neckRotation = (yaw: float, twist: float, head: Input.quaternion): Input.quaternion =>
  Quat.nlerp(Quat.yaw(yaw + List.sum(Rig.spineTwist) * twist), head, Rig.neckBend)

/// The camera relative to the neck joint, for the head's and neck's world
/// rotations: the forward kinematics the solver inverts to hang the body.
let eyeOffset = (scale: float, neck: Input.quaternion, head: Input.quaternion): Input.point3 =>
  Quat.add(
    Quat.rotate(neck, Quat.scale(scale, Rig.headInNeck)),
    Quat.rotate(head, Quat.add(Quat.scale(scale, Rig.eyeInHead), Quat.v3(0.0, 0.0, Rig.faceClearance))))

/// Scale the rig so this head stands its feet on the floor. It accounts for
/// the head's pitch, so looking down calibrates the same as looking ahead.
let scaleFor = (head: Pose, neck: Input.quaternion): float =>
  let fixed = eyeOffset(0.0, neck, head.rotation) in
  let perScale = Quat.sub(eyeOffset(1.0, neck, head.rotation), fixed) in
  Math.clamp(0.5, 1.5, (head.position.y - fixed.y) / (Rig.neck.y + perScale.y))

/// The hips' heading chases the head's once it turns past the deadzone, at a
/// bounded rate, and never trails it by more than `maxTwist`.
let slewYaw = (yaw: float, heading: float, dt: float): float =>
  let d = Quat.wrap(heading - yaw) in
  let chased =
    if Math.abs(d) <= deadzone then yaw
    else yaw + Math.clamp(0.0 - turnRate * dt, turnRate * dt, d - Math.sign(d) * deadzone) in
  let behind = Quat.wrap(heading - chased) in
  if Math.abs(behind) > maxTwist then Quat.wrap(heading - Math.sign(behind) * maxTwist)
  else Quat.wrap(chased)

/// A head that jumped (a recentre, or the first sample) snaps the body.
let jumped = (previous: Option.t<Pose>, head: Pose): bool =>
  match previous with
  | Option.None => true
  | Option.Some(p) =>
    Quat.length(Quat.sub(head.position, p.position)) > recenterJump
      || Math.abs(Quat.wrap(Quat.heading(head.rotation) - Quat.heading(p.rotation))) > recenterTurn

let stepHand = (side: Rig.Side, toModel: (Pose) => Pose, scale: float, dt: float, x: HandSample, h: Hand): Hand =>
  match x.grip with
  | Option.Some(g) =>
    let rotation = Quat.mul(g.rotation, Rig.gripToWrist(side)) in
    let wrist = {
      position: Quat.sub(g.position, Quat.rotate(rotation, Quat.scale(scale, Rig.gripInWrist(side)))),
      rotation: rotation
    } in
    {
      // A hand seen for the first time is followed at once; a returning one
      // fades back in.
      blend: if Option.isNone(h.wrist) then 1.0 else Math.min(1.0, h.blend + dt / recoverFade),
      wrist: Option.Some(toModel(wrist)),
      index: x.trigger,
      grip: x.squeeze
    }
  | Option.None => { h with blend: Math.max(0.0, h.blend - dt / lossFade) }

/// Advance the body by one fixed step.
let step = (s: State, x: Sample, dt: float): State =>
  let head = match x.head with | Option.Some(h) => Option.Some(h) | Option.None => s.head in
  match head with
  | Option.None => { s with calibrateHeld: x.calibrate, clock: s.clock + dt }
  | Option.Some(h) =>
    let snap = match x.head with | Option.Some(fresh) => jumped(s.head, fresh) | Option.None => false in
    let heading = Quat.heading(h.rotation) in
    let yaw = if snap then heading else slewYaw(s.yaw, heading, dt) in
    let twist = Quat.wrap(heading - yaw) in
    let neck = neckRotation(yaw, twist, h.rotation) in
    let calibrate = (x.calibrate && not s.calibrateHeld) || not s.calibrated in
    let scale = if calibrate then scaleFor(h, neck) else s.scale in
    let body = Quat.yaw(yaw) in
    let unturn = Quat.conj(body) in
    // Hang the body from the neck joint behind the eyes.
    let neckAt = Quat.sub(h.position, eyeOffset(scale, neck, h.rotation)) in
    let root = Quat.sub(neckAt, Quat.rotate(body, Quat.scale(scale, Rig.neckUnder(twist)))) in
    let toModel = (p: Pose): Pose => {
      position: Quat.scale(1.0 / scale, Quat.rotate(unturn, Quat.sub(p.position, root))),
      rotation: Quat.mul(unturn, p.rotation)
    } in
    {
      scale: scale,
      calibrated: true,
      calibrateHeld: x.calibrate,
      head: Option.Some(h),
      yaw: yaw,
      twist: twist,
      root: root,
      neckRotation: Quat.mul(unturn, neck),
      headRotation: Quat.mul(unturn, h.rotation),
      left: stepHand(Rig.Left, toModel, scale, dt, x.left, s.left),
      right: stepHand(Rig.Right, toModel, scale, dt, x.right, s.right),
      clock: s.clock + dt
    }

// --- pose ---

let twistSpine = (twist: float, anim: Anim.t): Anim.t =>
  List.zip(Rig.spineTwist, Rig.spine)
    |> List.fold((a, joint) =>
         let (name, share) = joint in
         Anim.rotate(name, 0deg, Angle.radians(share * twist), 0deg, a),
       anim)

let reachHand = (side: Rig.Side, h: Hand, anim: Anim.t): Anim.t =>
  let curled =
    anim
      |> Rig.curl(side, Rig.indexFinger(side), h.index)
      |> Rig.curl(side, Rig.gripFingers(side), h.grip)
      |> Rig.curlThumb(side, h.grip) in
  match h.wrist with
  | Option.None => curled
  | Option.Some(w) =>
    let weight = Math.smoothstep(0.0, 1.0, h.blend) in
    let arm = Rig.arm(side) in
    curled
      |> Anim.reach(arm.shoulder, arm.elbow, arm.wrist, Quat.toVec3(w.position), weight)
      |> Anim.orient(arm.wrist, w.rotation, weight)

/// The avatar's pose. Pure: the same state always gives the same pose.
let pose = (s: State): Anim.t =>
  Rig.idleLegs(s.clock)
    |> twistSpine(s.twist)
    |> Rig.restArms
    |> Anim.orient(Rig.j.mixamorig_Neck, s.neckRotation, 1.0)
    |> Anim.orient(Rig.j.mixamorig_Head, s.headRotation, 1.0)
    |> reachHand(Rig.Left, s.left)
    |> reachHand(Rig.Right, s.right)

/// Model space to the world.
let toWorldPoint = (s: State, p: Input.point3): Input.point3 =>
  Quat.add(s.root, Quat.rotate(Quat.yaw(s.yaw), Quat.scale(s.scale, p)))

/// Place a model-space subtree (the avatar) in the world.
let place = (s: State, scene: Scene.t): Scene.t =>
  scene
    |> Scene.scale(s.scale)
    |> Scene.rotateY(Angle.radians(s.yaw))
    |> Scene.translate(Quat.toVec3(s.root))
