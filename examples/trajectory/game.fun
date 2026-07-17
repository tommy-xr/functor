// examples/trajectory — trajectory-preview spike (docs/time-travel.md T6, the
// "Inventing on Principle" demo).
//
// A handful of balls launched under gravity. Each frame `draw` FORWARD-SIMULATES
// the model — it re-runs the exact same pure step the live sim uses, N times,
// to get the balls' future states — and draws a dim ghost dot at every future
// position. You see where each ball WILL go before it gets there.
//
// The trick is only possible because the sim step is a PURE function of the
// model (no side effects): holding a model, anyone can roll it forward. No
// engine changes — this is all in-language.
//
// "Smart" trails: only balls that are actually MOVING contribute ghost dots
// (see `moving`), so a resting ball leaves the scene clean — trails appear
// exactly where something is in motion.
//
// Run:   functor -d examples/trajectory run native
// Live:  edit `gravity` / a ball's launch `vel` and save — hot reload keeps the
//        model and the predicted arcs update instantly (the Bret Victor moment).
//
// Deterministic still (arcs fully drawn; --fixed-time pins dt=0 so the live
// balls hold at their launch point while the preview still projects forward):
//   functor -d examples/trajectory run native \
//     --capture-frame /tmp/trajectory.png --fixed-time 0.0 --capture-time 0.5

type Vec3 = { x: float, y: float, z: float }
type Ball = { pos: Vec3, vel: Vec3 }

// --- Tunables (edit while running; the arcs re-project live on save) ---
let gravity = 14.0        // downward accel, units/s^2
let bounce = 0.55         // restitution when a ball hits the ground (y = 0)
let restVel = 0.5         // a bounce reflecting slower than this lands as at-rest
let previewSteps = 48.0   // how many steps into the future to project
let previewDt = 0.03      // seconds per preview step (~arc resolution)

let init = {
  balls: [
    { pos: { x: -4.0, y: 0.5, z: 0.0 }, vel: { x: 3.2, y: 9.0, z: 0.0 } },
    { pos: { x: -1.5, y: 0.5, z: 1.5 }, vel: { x: 2.6, y: 11.5, z: -0.7 } },
    { pos: { x: 1.5, y: 0.5, z: -1.0 }, vel: { x: -1.8, y: 8.0, z: 0.4 } },
    // A ball at rest on the ground: `stepBall` holds it exactly still, so its
    // velocity stays zero and it earns NO trail (the "smart" filter).
    { pos: { x: 4.0, y: 0.0, z: 0.0 }, vel: { x: 0.0, y: 0.0, z: 0.0 } }
  ]
}

// A ball exactly at rest on the ground stays at rest. Without this, gravity
// accumulates for a step, the discrete bounce reflects it, and a "resting"
// ball pumps a tiny perpetual hop — enough velocity to defeat the `moving`
// filter below and earn a trail it must not have.
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

// The ONE pure step. Euler integrate; bounce off the ground plane at y = 0 (a
// bounce reflecting slower than restVel PARKS the ball — all velocity zeroed,
// crude friction — so every ball converges to atRest and its trail ends when
// its motion does). Both the live `tick` and the forward-sim preview call
// this — the ghost trail is, by construction, exactly the future the game
// itself will produce.
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

let stepModel = (dt, m) =>
  { m with balls: m.balls |> List.map((b) => stepBall(dt, b)) }

let tick = (model, dt, tts) => stepModel(dt, model)

// --- Forward simulation: roll the model forward, collecting future states ---
// (List.fold, not recursion — the interpreter caps manual recursion depth.)
let futures = (m) =>
  List.range(previewSteps)
    |> List.fold(
         (acc, _) =>
           let next = stepModel(previewDt, acc.cur) in
           { cur: next, seq: [next, ..acc.seq] },
         { cur: m, seq: [] })

// --- The "smart" filter: only moving balls get a ghost trail ---
let speedSq = (b) => b.vel.x * b.vel.x + b.vel.y * b.vel.y + b.vel.z * b.vel.z
let moving = (b) => speedSq(b) > 0.05

// A dim ghost marker at a ball's (future) position.
let ghost = (b) =>
  Scene.sphere()
    |> Scene.scale(0.07)
    |> Scene.emissive(Color.rgb(0.25, 0.85, 1.0))
    |> Scene.translate(Vec3.make(b.pos.x, b.pos.y, b.pos.z))

// The moving balls of one future model, as a group of ghost dots.
let ghostsOf = (m) =>
  m.balls |> List.filter(moving) |> List.map(ghost) |> Scene.group

// All ghost dots across the whole projected future.
let trail = (model) =>
  let fs = futures(model) in
  fs.seq |> List.map(ghostsOf) |> Scene.group

// A solid, lit ball at its CURRENT position.
let ballView = (b) =>
  Scene.sphere()
    |> Scene.scale(0.3)
    |> Scene.lit(Color.rgb(1.0, 0.45, 0.2))
    |> Scene.translate(Vec3.make(b.pos.x, b.pos.y, b.pos.z))

let ground =
  Scene.plane()
    |> Scene.scale(24.0)
    |> Scene.lit(Color.rgb(0.14, 0.15, 0.2))

let draw = (model, tts) =>
  let cam = Camera.lookAt(Vec3.make(0.0, 6.0, 15.0), Vec3.make(0.0, 3.0, 0.0)) in
  let scene = Scene.group([
    ground,
    model.balls |> List.map(ballView) |> Scene.group,
    trail(model)
  ]) in
  Frame.createLit(cam, scene, [
    Light.ambient(Color.rgb(0.3, 0.3, 0.35)),
    Light.directional(Vec3.make(-0.4, -1.0, -0.3), Color.rgb(1.0, 1.0, 1.0), 1.0)
  ])
