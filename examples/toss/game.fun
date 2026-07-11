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
let restVel = 0.5    // a bounce reflecting slower than this lands as at-rest

let init = {
  balls: [
    { pos: { x: -4.0, y: 0.5, z: 0.0 }, vel: { x: 3.2, y: 9.0, z: 0.0 } },
    { pos: { x: -1.5, y: 0.5, z: 1.5 }, vel: { x: 2.6, y: 11.5, z: -0.7 } },
    { pos: { x: 1.5, y: 0.5, z: -1.0 }, vel: { x: -1.8, y: 8.0, z: 0.4 } },
    // At rest on the ground (y = 0, zero velocity). `stepBall` holds a resting
    // ball exactly still, so it never moves in the forward-sim and the runtime
    // gives it NO trail.
    { pos: { x: 4.0, y: 0.0, z: 0.0 }, vel: { x: 0.0, y: 0.0, z: 0.0 } }
  ]
}

// A ball exactly at rest on the ground stays at rest. Without this, gravity
// accumulates for a step, the discrete bounce reflects it, and a "resting"
// ball pumps a tiny perpetual hop — enough motion to earn the trail it is
// here to prove it must NOT get.
let atRest = (b) =>
  match b.pos.y == 0.0 with
  | false => false
  | true =>
    match b.vel.x == 0.0 with
    | false => false
    | true =>
      match b.vel.y == 0.0 with
      | false => false
      | true => b.vel.z == 0.0

// One Euler step; bounce off the ground at y = 0. A bounce that would reflect
// slower than restVel PARKS the ball (all velocity zeroed — crude friction), so
// every ball converges to atRest and its trail ends when its motion does.
let stepBall = (dt, b) =>
  match atRest(b) with
  | true => b
  | false =>
    let nx = b.pos.x + b.vel.x * dt in
    let ny = b.pos.y + b.vel.y * dt in
    let nz = b.pos.z + b.vel.z * dt in
    let nvy = b.vel.y - gravity * dt in
    match ny < 0.0 with
    | true =>
      let ry = 0.0 - nvy * bounce in
      (match ry < restVel with
       | true => { pos: { x: nx, y: 0.0, z: nz }, vel: { x: 0.0, y: 0.0, z: 0.0 } }
       | false => { pos: { x: nx, y: 0.0, z: nz }, vel: { x: b.vel.x, y: ry, z: b.vel.z } })
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
