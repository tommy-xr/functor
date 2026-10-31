// `functor -d examples/fullbody test`: the solver's contract, headless.

let dt = 1.0 / 60.0
let v3 = Quat.v3
let near = (a: float, b: float, eps: float): bool => Math.abs(a - b) <= eps
let close = (a: Input.point3, b: Input.point3, eps: float): bool => Quat.length(Quat.sub(a, b)) <= eps

let noHand: Body.HandSample = { grip: Option.None, trigger: 0.0, squeeze: 0.0 }

// The headset of a person `scale` times Xbot's size standing at the origin,
// facing +Z, head turned and pitched (the chest still facing ahead).
let headAt = (scale: float, turnDeg: float, pitchDeg: float): Body.Pose =>
  Body.headFor(scale, Quat.mul(Quat.yaw(Body.degrees(turnDeg)), Quat.axisAngle(v3(1.0, 0.0, 0.0), Body.degrees(pitchDeg))))

let sample = (head: Body.Pose, left: Body.HandSample, right: Body.HandSample): Body.Sample =>
  { head: Option.Some(head), left: left, right: right, calibrate: false }

let looking = (turnDeg: float): Body.Sample => sample(headAt(1.0, turnDeg, 0.0), noHand, noHand)

// Every state a sample sequence passes through, oldest first.
let trace = (s0: Body.State, samples: List<Body.Sample>): List<Body.State> =>
  samples
    |> List.fold((acc, x) =>
         match acc with
         | [s, ..rest] => [Body.step(s, x, dt), s, ..rest]
         | [] => [],
       [s0])
    |> List.reverse

let final = (states: List<Body.State>): Body.State =>
  states |> List.last |> Option.defaultValue(Body.init)

let repeat = (n: float, x: Body.Sample): List<Body.Sample> => List.range(n) |> List.map((i) => x)

let degreesOf = (r: float): float => r * 180.0 / Math.pi

// --- torso: bounded lag and twist ---

// A glance inside the deadzone only drifts the hips: 20°/s, 5° in 0.25 s.
expect (
  let s = final(trace(Body.init, List.flatten([[looking(0.0)], repeat(15.0, looking(20.0))]))) in
  near(degreesOf(s.yaw), 5.0, 0.0001) && near(degreesOf(s.twist), 15.0, 0.0001)
)

// A 120° turn (12° a step, then held): the hips never trail by more than the
// twist limit, otherwise turn no faster than the turn rate, and realign.
expect (
  let turning = List.range(10.0) |> List.map((i) => looking(12.0 * (i + 1.0))) in
  let states = trace(Body.init, List.flatten([[looking(0.0)], turning, repeat(90.0, looking(120.0))])) in
  let steps = List.zip(List.drop(1.0, states), states) in
  let rateOk = steps |> List.all((p) =>
    let (a, b) = p in
    Math.abs(Quat.wrap(b.yaw - a.yaw)) <= Body.turnRate * dt + 0.000001
      || near(Math.abs(b.twist), Body.maxTwist, 0.000001)) in
  let twistOk = states |> List.all((s) => Math.abs(s.twist) <= Body.maxTwist + 0.000001) in
  let lagged = List.nth(6.0, states) |> Option.map((s) => degreesOf(s.twist) > 40.0) |> Option.defaultValue(false) in
  rateOk && twistOk && lagged && near(final(states).twist, 0.0, 0.000001)
)

// A recentre (the head jumps 90° in one step) turns the hips with it,
// keeping their twist, instead of swinging them.
expect (
  let states = trace(Body.init, [looking(0.0), looking(20.0), sample(headAt(1.0, 110.0, 0.0), noHand, noHand)]) in
  let before = List.nth(2.0, states) |> Option.defaultValue(Body.init) in
  let after = final(states) in
  near(degreesOf(after.yaw - before.yaw), 90.0, 0.0001) && near(after.twist, before.twist, 0.000001)
)

// --- calibration ---

// The first head calibrates: 1.75 m eyes scale Xbot's 1.66 m and stand the
// feet on the floor, level or looking down alike.
expect (
  let level = { position: v3(0.0, 1.75, 0.0), rotation: Quat.identity } in
  let a = Body.step(Body.init, sample(level, noHand, noHand), dt) in
  let k = 1.75 / Rig.eyeHeight in
  let b = Body.step(Body.init, sample(headAt(k, 0.0, 60.0), noHand, noHand), dt) in
  near(a.scale, k, 0.000001) && near(a.root.y, 0.0, 0.000001)
    && near(b.scale, k, 0.000001) && near(b.root.y, 0.0, 0.000001)
)

// Recalibration takes a rising edge: holding the button doesn't keep rescaling.
expect (
  let at = (y: float, press: bool) =>
    { head: Option.Some({ position: v3(0.0, y, 0.0), rotation: Quat.identity }), left: noHand, right: noHand, calibrate: press } in
  let s = final(trace(Body.init, [at(1.66, false), at(1.5, true), at(1.9, true)])) in
  near(s.scale, 1.5 / Rig.eyeHeight, 0.000001)
)

// Absurd heights clamp.
expect Body.scaleFor({ position: v3(0.0, 4.0, 0.0), rotation: Quat.identity }, Quat.identity) == 1.5

// --- tracking loss ---

let grip = (x: float, y: float, z: float): Body.HandSample =>
  { grip: Option.Some({ position: v3(x, y, z), rotation: Quat.identity }), trigger: 0.0, squeeze: 0.0 }

let lossTrace =
  trace(Body.init, List.flatten([
    repeat(5.0, sample(headAt(1.0, 0.0, 0.0), grip(0.2, 1.1, 0.3), noHand)),
    repeat(18.0, sample(headAt(1.0, 0.0, 0.0), noHand, noHand)),
    repeat(12.0, sample(headAt(1.0, 0.0, 0.0), grip(0.2, 1.1, 0.3), noHand)),
  ]))

let at = (i: float): Body.State => List.nth(i, lossTrace) |> Option.defaultValue(Body.init)

// A first sighting is followed at once; loss fades out over 0.3 s, holding the
// last target (never the origin); recovery fades back in over 0.2 s.
expect at(1.0).left.blend == 1.0
expect near(at(14.0).left.blend, 0.5, 0.000001)
expect at(23.0).left.blend == 0.0
expect at(23.0).left.wrist == at(5.0).left.wrist
expect near(at(29.0).left.blend, 0.5, 0.000001)
expect at(35.0).left.blend == 1.0
// A one-step dropout while the hand moves 30 cm: the target slides there over
// 0.2 s instead of jumping.
expect (
  let states = trace(Body.init, List.flatten([
    repeat(3.0, sample(headAt(1.0, 0.0, 0.0), grip(0.2, 1.1, 0.3), noHand)),
    [sample(headAt(1.0, 0.0, 0.0), noHand, noHand)],
    repeat(12.0, sample(headAt(1.0, 0.0, 0.0), grip(0.2, 1.4, 0.3), noHand)),
  ])) in
  let y = (i: float) =>
    List.nth(i, states) |> Option.bind((s) => s.left.wrist) |> Option.map((w) => w.position.y) |> Option.defaultValue(0.0) in
  let steps = List.range(12.0) |> List.map((i) => y(i + 5.0) - y(i + 4.0)) in
  (steps |> List.all((d) => d > 0.0 && d < 0.04)) && near(y(16.0) - y(4.0), 0.3, 0.000001)
)

expect (
  match at(23.0).left.wrist with
  | Option.Some(w) => w.position.y > 0.8
  | Option.None => false
)

// --- determinism and purity ---

let script =
  List.flatten([
    repeat(4.0, sample(headAt(1.05, 0.0, 30.0), grip(0.25, 1.2, 0.35), grip(0.0 - 0.25, 1.2, 0.35))),
    List.range(20.0) |> List.map((i) => sample(headAt(1.05, i * 4.0, 30.0), grip(0.1, 1.3, 0.3), noHand)),
  ])

// The same samples from the same state always step to the same state.
expect trace(Body.init, script) == trace(Body.init, script)

// `pose` reads only its state: the same state solves to identical joints.
expect (
  let s = final(trace(Body.init, script)) in
  Bytes.equals(Probe.pose(Body.pose(s)), Probe.pose(Body.pose(s)))
)

// --- the solve itself, read back through Probe ---

let solvedIn = (s: Body.State, name: string): Probe.Frame =>
  Probe.frame(Probe.pose(Body.pose(s)), name)

// A reachable wrist lands on its target, axes and all.
expect (
  let s = final(trace(Body.init, repeat(2.0, sample(headAt(1.0, 0.0, 20.0), grip(0.2, 1.15, 0.3), noHand)))) in
  match s.left.wrist with
  | Option.None => false
  | Option.Some(w) =>
    let f = solvedIn(s, Rig.j.mixamorig_LeftHand) in
    close(f.position, w.position, 0.001)
      && close(f.x, Quat.rotate(w.rotation, v3(1.0, 0.0, 0.0)), 0.001)
      && close(f.y, Quat.rotate(w.rotation, v3(0.0, 1.0, 0.0)), 0.001)
)

// The avatar's eyes sit `faceClearance` behind the camera, along its gaze.
expect (
  let head = headAt(1.1, 35.0, 50.0) in
  let s = Body.step(Body.init, sample(head, noHand, noHand), dt) in
  let f = solvedIn(s, Rig.j.mixamorig_Head) in
  let eyeModel = Quat.add(f.position, Quat.rotate(s.headRotation, Rig.eyeInHead)) in
  let wanted = Quat.sub(head.position, Quat.rotate(head.rotation, v3(0.0, 0.0, Rig.faceClearance))) in
  close(Body.toWorldPoint(s, eyeModel), wanted, 0.002)
)
