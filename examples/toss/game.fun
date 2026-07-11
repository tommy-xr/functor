// examples/toss — bouncing balls under gravity. NOTE: this game has NO
// trajectory / trail logic at all. `draw` renders only the balls at their
// CURRENT positions. The predicted arcs come entirely from the runtime's
// scene-diff trajectory preview (`functor -d examples/toss run native
// --trajectory`), which forward-simulates the model and traces the scene
// nodes that move — proving the runtime derives the trails, not the game.
//
//   functor -d examples/toss run native --trajectory \
//     --capture-frame /tmp/toss.png --fixed-time 0.0 --capture-time 0.5

type Vec3 = { x: float, y: float, z: float }
type Ball = { pos: Vec3, vel: Vec3 }

let gravity = 14.0
let bounce = 0.55

let init = {
  balls: [
    { pos: { x: -4.0, y: 0.5, z: 0.0 }, vel: { x: 3.2, y: 9.0, z: 0.0 } },
    { pos: { x: -1.5, y: 0.5, z: 1.5 }, vel: { x: 2.6, y: 11.5, z: -0.7 } },
    { pos: { x: 1.5, y: 0.5, z: -1.0 }, vel: { x: -1.8, y: 8.0, z: 0.4 } },
    // Already at rest on the ground (y = 0, zero velocity) — it never moves in
    // the forward-sim, so the runtime gives it NO trail.
    { pos: { x: 4.0, y: 0.0, z: 0.0 }, vel: { x: 0.0, y: 0.0, z: 0.0 } }
  ]
}

let stepBall = (dt, b) =>
  let nx = b.pos.x + b.vel.x * dt in
  let ny = b.pos.y + b.vel.y * dt in
  let nz = b.pos.z + b.vel.z * dt in
  let nvy = b.vel.y - gravity * dt in
  match ny < 0.0 with
  | true => { pos: { x: nx, y: 0.0, z: nz }, vel: { x: b.vel.x, y: 0.0 - nvy * bounce, z: b.vel.z } }
  | false => { pos: { x: nx, y: ny, z: nz }, vel: { x: b.vel.x, y: nvy, z: b.vel.z } }

let tick = (model, dt, tts) =>
  { model with balls: model.balls |> List.map((b) => stepBall(dt, b)) }

let ballView = (b) =>
  Scene.sphere()
    |> Scene.scale(0.3)
    |> Scene.lit(1.0, 0.45, 0.2)
    |> Scene.translate(b.pos.x, b.pos.y, b.pos.z)

let ground =
  Scene.plane()
    |> Scene.scale(24.0)
    |> Scene.lit(0.14, 0.15, 0.2)

let draw = (model, tts) =>
  let cam = Camera.lookAt(0.0, 6.0, 15.0, 0.0, 3.0, 0.0) in
  let scene = Scene.group([
    ground,
    model.balls |> List.map(ballView) |> Scene.group
  ]) in
  Frame.createLit(cam, scene, [
    Light.ambient(0.3, 0.3, 0.35),
    Light.directional(-0.4, -1.0, -0.3, 1.0, 1.0, 1.0, 1.0)
  ])
