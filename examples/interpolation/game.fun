// interpolation — smooth motion on displays faster than the 60 Hz sim.
//
// `tick` runs at a fixed 60 Hz. At 120 Hz every other frame runs no step, so
// a pose read straight from the model holds for two frames and judders. A
// three-parameter `draw` also receives `alpha` in [0, 1): how far this frame
// sits between the last step and the next. Keep the previous and current pose
// in the model and draw between them.
//
// The two ghosts above the rig mark the stepped poses; the rig is drawn between them, and
// the chase camera follows the drawn rig. Simulate a 120 Hz display with
// `functor -d examples/interpolation run native -- --display-hz 120`.
// `e2e/interpolation.mjs` checks it headlessly over the debug server.

type Model = { angle: float, prev: Vec3.t, cur: Vec3.t }

let radius = 5.0
let speed = 2.0 // radians per second around the loop

let poseAt = (angle: float) =>
  Vec3.make(Math.cos(angle) * radius, 0.5, Math.sin(angle) * radius)

let init = { angle: 0.0, prev: poseAt(0.0), cur: poseAt(0.0) }

// The step moves `cur` and remembers where it was.
let tick = (m: Model, dt: float, tts) =>
  let angle = m.angle + speed * dt in
  { angle: angle, prev: m.cur, cur: poseAt(angle) }

// The drawn pose: `alpha` of the way from the previous step to the current.
let drawnPose = (m: Model, alpha: float) => m.prev |> Vec3.lerp(m.cur, alpha)

expect (
  let stepped = tick(init, 1.0 / 60.0, 0.0) in
  Vec3.x(drawnPose(stepped, 0.0)) == Vec3.x(stepped.prev)
)
expect (
  let stepped = tick(init, 1.0 / 60.0, 0.0) in
  Vec3.z(drawnPose(stepped, 0.5)) == (Vec3.z(stepped.prev) + Vec3.z(stepped.cur)) / 2.0
)

let ghost = (pos: Vec3.t, color: Color.t) =>
  Scene.sphere()
    |> Scene.scaleXYZ(0.25, 0.25, 0.25)
    |> Scene.color(color)
    |> Scene.translate(pos |> Vec3.add(Vec3.make(0.0, 1.2, 0.0)))

// Group order is fixed — rig, previous, current — for the e2e check.
let draw = (m: Model, tts, alpha: float) =>
  let rig = drawnPose(m, alpha) in
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(Vec3.x(rig) * 2.2, 3.5, Vec3.z(rig) * 2.2), rig),
    Scene.group([
      Scene.cube() |> Scene.lit(Color.rgb(0.3, 0.65, 0.9)) |> Scene.translate(rig),
      ghost(m.prev, Color.rgb(1.0, 0.35, 0.65)),
      ghost(m.cur, Color.rgb(1.0, 0.85, 0.2)),
      Scene.cylinder()
        |> Scene.scaleXYZ(0.6, 3.0, 0.6)
        |> Scene.lit(Color.rgb(0.6, 0.6, 0.65)),
      Scene.plane()
        |> Scene.scaleXYZ(20.0, 1.0, 20.0)
        |> Scene.lit(Color.rgb(0.2, 0.22, 0.28)),
    ]),
    [
      Light.ambient(Color.rgb(0.3, 0.3, 0.3)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 1.0, 1.0), 0.9),
    ],
  )
