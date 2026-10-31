// A scripted 8-second XR fixture, played when no headset is attached: it goes
// through the same `Body.fromXr` path a headset's samples take. Authored in the
// solver's world (forward +Z, so +X is the player's left) for a player with
// Xbot's own proportions, then converted to tracking space.

let loop = 8.0

let v3 = Quat.v3
let pitch = (d: float) => Quat.axisAngle(v3(1.0, 0.0, 0.0), Body.degrees(d))
let roll = (d: float) => Quat.axisAngle(v3(0.0, 0.0, 1.0), Body.degrees(d))
let turn = (d: float) => Quat.yaw(Body.degrees(d))

type Hands = { left: Body.Pose, right: Body.Pose }

let pose = (p: Input.point3, q: Input.quaternion): Body.Pose => { position: p, rotation: q }

// Key poses: holding forward, arms crossed, reaching overhead.
let forward: Hands = {
  left: pose(v3(0.18, 1.12, 0.3), pitch(20.0)),
  right: pose(v3(0.0 - 0.18, 1.12, 0.3), pitch(20.0))
}
let crossed: Hands = {
  left: pose(v3(0.0 - 0.14, 1.22, 0.3), Quat.mul(turn(0.0 - 50.0), pitch(10.0))),
  right: pose(v3(0.14, 1.16, 0.32), Quat.mul(turn(50.0), pitch(10.0)))
}
let overhead: Hands = {
  left: pose(v3(0.24, 2.02, 0.12), pitch(0.0 - 90.0)),
  right: pose(v3(0.0 - 0.24, 2.02, 0.12), pitch(0.0 - 90.0))
}

let mix = (a: Body.Pose, b: Body.Pose, t: float): Body.Pose =>
  pose(Quat.lerp(a.position, b.position, t), Quat.nlerp(a.rotation, b.rotation, t))

let mixHands = (a: Hands, b: Hands, t: float): Hands =>
  { left: mix(a.left, b.left, t), right: mix(a.right, b.right, t) }

// Ease from `a` to `b` across [t0, t1].
let ease = (t: float, t0: float, t1: float): float => Math.smoothstep(t0, t1, t)

// The timeline: roll the right wrist, cross the arms, back, reach overhead,
// back; the left hand drops out of tracking near the end.
let hands = (t: float): Hands =>
  if t < 1.9 then
    let r = Math.sin(Math.pi * Math.clamp01((t - 0.3) / 1.4) * 2.0) * 70.0 in
    { forward with right: { forward.right with rotation: Quat.mul(forward.right.rotation, roll(r)) } }
  else if t < 3.6 then mixHands(forward, crossed, ease(t, 1.9, 2.5))
  else if t < 4.6 then mixHands(crossed, forward, ease(t, 3.6, 4.2))
  else if t < 6.2 then mixHands(forward, overhead, ease(t, 4.6, 5.2))
  else mixHands(overhead, forward, ease(t, 6.2, 6.8))

let leftTracked = (t: float): bool => t < 6.9 || t > 7.6

// The head looks down at the hands, then up at the inspection panel, with a
// slow sway the hips lag behind.
let head = (t: float): Body.Pose =>
  let down = 55.0 * (1.0 - ease(t, 3.8, 4.4)) + 55.0 * ease(t, 7.6, 8.0) + 6.0 in
  let sway = 30.0 * Math.sin(2.0 * Math.pi * t / loop) in
  let q = Quat.mul(turn(sway), pitch(down)) in
  let neckAt = Rig.neck in
  pose(Quat.add(neckAt, Quat.rotate(q, Quat.add(Rig.eyeInNeck, v3(0.0, 0.0, Rig.faceClearance)))), q)

// Back to tracking space (the same half turn undoes itself).
let tracked = (p: Body.Pose): Input.pose => {
  position: { x: 0.0 - p.position.x, y: p.position.y, z: 0.0 - p.position.z },
  orientation: { x: 0.0 - p.rotation.x, y: p.rotation.y, z: 0.0 - p.rotation.z, w: p.rotation.w }
}

let controller = (active: bool, grip: Body.Pose, trigger: float, squeeze: float): Input.controller => {
  active: active,
  grip: if active then Option.Some(tracked(grip)) else Option.None,
  aim: Option.None,
  trigger: trigger,
  squeeze: squeeze,
  thumbstick: { x: 0.0, y: 0.0 },
  primaryPressed: false,
  secondaryPressed: false,
  thumbstickPressed: false,
  menuPressed: false
}

/// The fixture's XR sample at `tts`.
let xr = (tts: float): Input.xr =>
  let t = Math.mod(tts, loop) in
  let h = hands(t) in
  // Fists while the arms are crossed; the right index pulls during the roll.
  let fist = ease(t, 1.9, 2.4) * (1.0 - ease(t, 3.6, 4.0)) in
  let pull = if t < 1.9 then Math.clamp01(Math.sin(Math.pi * t / 1.9) * 1.5) else 0.0 in
  {
    head: Option.Some(tracked(head(t))),
    left: controller(leftTracked(t), h.left, fist, fist),
    right: controller(true, h.right, Math.max(pull, fist), fist)
  }
