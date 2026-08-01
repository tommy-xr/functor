// A finite 4 km × 4 km terrain rendered from a 16-bit heightmap, walked on
// foot as a real physics body.
//
// The runtime chooses a quadtree LOD from the stable center camera and draws
// every visible patch as an instance of one 64×64 grid. Both VR eyes therefore
// see the same geometry, and changing detail uploads only the compact patch
// list—not a world-sized mesh.
//
// The walker is a dynamic capsule steered from `tick`, standing on the SAME
// heightfield the renderer draws: `Scene.terrain` and `Physics.heightfield`
// take one descriptor, so the ground you see is the ground you collide with.
//
//   node examples/terrain/generate-heightmap.mjs
//   functor -d examples/terrain run native

let worldSize = 4000.0
let walkSpeed = 7.5
let jumpSpeed = 7.0
let sensitivity = 0.003
let pitchLimit = 1.45

// Capsule: a 3.4 m body, tall enough to read against 4 m heightmap cells.
let capsuleHalfHeight = 1.2
let capsuleRadius = 0.5
// Centre-to-feet, and how far past the feet the ground probe reaches.
let feetOffset = capsuleHalfHeight + capsuleRadius
let probeSkin = 0.35
// The camera rides near the top of the capsule rather than at its centre.
let eyeOffset = 0.9

// Just above the surface at this XZ (the heightmap reads ~189 m there). The
// drop is deliberately short: the sample should start standing rather than
// falling for five seconds, and a golden capture should be of a settled body,
// not of one still in the air — where the exact height would depend on how
// many physics steps the capture window happened to fit.
let spawn = Vec3.make(0.0 - 650.0, 192.0, 0.0 - 1200.0)

// Launched spheres. The cap keeps the declared body list bounded: physics
// reconciles against exactly what `physics` returns each frame, so a ball that
// falls out of the list is despawned.
let ballRadius = 1.1
let ballSpeed = 55.0
let maxBalls = 24.0

let world =
  Terrain.heightmap(Asset.texture("heightmap.png"), worldSize, worldSize, -80.0, 520.0)
  |> Terrain.maxPixelError(2.0)
  |> Terrain.layered(
       Color.rgb(0.12, 0.30, 0.12),
       Color.rgb(0.34, 0.49, 0.20),
       Color.rgb(0.28, 0.25, 0.23),
       Color.rgb(0.86, 0.90, 0.91),
       340.0)
  |> Terrain.textured(
       Asset.texture("detail-low.jpg"),
       Asset.texture("detail-high.jpg"),
       Asset.texture("detail-rock.jpg"),
       Asset.texture("detail-snow.jpg"),
       8.0)
  |> Terrain.grass(2.2, 140.0, 1.1, Color.rgb(0.25, 0.46, 0.10))

let terrainBody = Physics.tag("terrain")
let walkerTag = Physics.tag("walker")

type Mouse =
  | NoMouse
  | MouseAt(x: float, y: float)

let init = {
  held: { up: false, down: false, left: false, right: false },
  jump: false,
  yaw: 0.15,
  pitch: 0.0 - 0.08,
  lastMouse: NoMouse,
  eye: { x: 0.0 - 650.0, y: 192.0, z: 0.0 - 1200.0 },
  grounded: false,
  balls: [],
  nextBall: 0.0,
}

let setHeld = (held, key, isDown) =>
  match key with
  | Key.W => { held with up: isDown }
  | Key.Up => { held with up: isDown }
  | Key.S => { held with down: isDown }
  | Key.Down => { held with down: isDown }
  | Key.A => { held with left: isDown }
  | Key.Left => { held with left: isDown }
  | Key.D => { held with right: isDown }
  | Key.Right => { held with right: isDown }
  | _ => held


let clamp = (value, lo, hi) =>
  if value < lo then lo else if hi < value then hi else value

let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: clamp(model.pitch - (y - lastY) * sensitivity, 0.0 - pitchLimit, pitchLimit),
        lastMouse: MouseAt(x, y) }

// Where the camera is pointing. `Camera3D.firstPerson` treats yaw 0 / pitch 0 as
// looking down +Z, with positive pitch looking up.
let lookDir = (yaw, pitch) =>
  let flat = Math.cos(pitch) in
  Vec3.make(Math.sin(yaw) * flat, Math.sin(pitch), Math.cos(yaw) * flat)

// Spawned clear of the capsule so the new body does not start interpenetrating
// the walker and get shoved sideways on its first step.
let launch = (model) =>
  let dir = lookDir(model.yaw, model.pitch) in
  let eye = Vec3.make(model.eye.x, model.eye.y, model.eye.z) in
  let ball = {
    id: model.nextBall,
    at: eye |> Vec3.add(dir |> Vec3.scale(2.5)),
    velocity: dir |> Vec3.scale(ballSpeed),
  } in
  let kept =
    model.balls
    |> List.filter((b) => b.id > model.nextBall - maxBalls) in
  { model with
      balls: kept |> List.append([ball]),
      nextBall: model.nextBall + 1.0 }

// `F` launches as well as the mouse: input scripts carry keys but not mouse
// buttons, so a mouse-only launcher could not be driven headlessly — and the
// deterministic capture path is exactly what proves the balls roll.
let input = (model, key, isDown) =>
  match key with
  | Key.Space => { model with jump: isDown }
  | Key.F => if isDown then launch(model) else model
  | _ => { model with held: setHeld(model.held, key, isDown) }

let mouseButton = (model, button, isDown) =>
  match button with
  | Mouse.Left => if isDown then launch(model) else model
  | _ => model

let axis = (negative, positive) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

// Straight down from the capsule's centre, past the feet by `probeSkin`, and
// ignoring the walker's own collider — a ray starting inside a capsule would
// otherwise hit it at distance 0 and report standing on itself.
let groundProbe = (pos) =>
  Physics.castExcluding(walkerTag,
                        Vec3.make(pos.x, pos.y, pos.z),
                        Vec3.make(0.0, -1.0, 0.0),
                        feetOffset + probeSkin)

// read -> decide -> write, all in one frame. The reads see the previous step
// (physics runs after `tick`) and the command applies at the next one.
let tick = (model, dt, tts) =>
  let pos = Physics.position(walkerTag) in
  let probe = groundProbe(pos) in
  let forward = axis(model.held.down, model.held.up) in
  let right = axis(model.held.left, model.held.right) in
  // Flat on purpose: steering is horizontal, and `Vec3.normalize` maps the
  // zero vector to zero, so standing still needs no special case.
  let wish =
    Vec3.make(forward * Math.sin(model.yaw) - right * Math.cos(model.yaw),
              0.0,
              forward * Math.cos(model.yaw) + right * Math.sin(model.yaw))
    |> Vec3.normalize
    |> Vec3.scale(walkSpeed) in
  let next =
    { model with
        grounded: probe.hit,
        eye: { x: pos.x, y: pos.y + eyeOffset, z: pos.z } } in
  // Steer the horizontal plane and leave the vertical axis to the solver,
  // which owns the ground contact, the landing impulse and gravity. The only
  // frame that writes vy is the one a jump fires on; the two writes touch
  // disjoint axes, so the pair applies as one whole-vector write.
  //
  // Writing the whole vector instead — echoing the previous frame's
  // `linearVelocity` as vy — is what made this walker floaty: that read is one
  // step stale (physics runs after `tick`), so every frame overwrote the
  // solver's in-progress contact resolution and a crest launched you off the
  // surface.
  let steer = Physics.setVelocityXZ(walkerTag, Vec3.x(wish), Vec3.z(wish)) in
  (next,
   if probe.hit && model.jump then
     Effect.batch([steer, Physics.setVelocityY(walkerTag, jumpSpeed)])
   else steer)

let ballTag = (ball) => Physics.tag($"ball-{ball.id}")

// A ball's declaration never changes after it is spawned — position and
// velocity are the launch values — so reconcile hands it to the simulation and
// leaves it there. Mutating these per frame would teleport it back instead.
let ballBody = (ball) =>
  Physics.dynamic(ballTag(ball), Physics.sphere(ballRadius))
  |> Physics.at(ball.at)
  |> Physics.velocity(ball.velocity)
  |> Physics.friction(0.55)
  |> Physics.restitution(0.35)

let physics = (model) =>
  Physics.scene(
    Vec3.make(0.0, -9.81, 0.0),
    [ Physics.heightfield(terrainBody, world) |> Physics.friction(0.8),
      Physics.dynamic(walkerTag, Physics.capsule(capsuleHalfHeight, capsuleRadius))
      |> Physics.at(spawn)
      // Without this the capsule tips on the first slope it lands on, and a
      // toppled body invalidates the fixed standing height the camera assumes.
      |> Physics.upright
      |> Physics.friction(0.4) ]
    |> List.append(model.balls |> List.map(ballBody)))

let draw = (model, tts) =>
  let camera =
    Camera3D.firstPerson(
      Vec3.make(model.eye.x, model.eye.y, model.eye.z),
      Angle.radians(model.yaw),
      Angle.radians(model.pitch),
      Angle.degrees(68.0))
    |> Camera3D.clip(0.5, 6000.0) in
  let water =
    Scene.plane()
    |> Scene.scale(worldSize)
    |> Scene.translate(Vec3.make(0.0, 28.0, 0.0))
    |> Scene.lit(Color.rgb(0.08, 0.24, 0.34)) in
  let ballNode = (ball) =>
    Scene.sphere()
    |> Scene.scale(ballRadius)
    |> Scene.lit(Color.rgb(0.97, 0.97, 0.95))
    |> Physics.transformed(ballTag(ball)) in
  Frame.createLit(
    camera,
    Scene.group([Scene.terrain(world), water]
                |> List.append(model.balls |> List.map(ballNode))),
    [
      Light.ambient(Color.rgb(0.20, 0.23, 0.28)),
      Light.directional(
        Vec3.make(0.42, -1.0, 0.28),
        Color.rgb(1.0, 0.84, 0.64),
        1.15),
    ])
  |> Frame.withFog(Fog.linear(700.0, 4800.0, Color.rgb(0.50, 0.64, 0.72)))

let ui = (model) =>
  Ui.text($"4 km terrain · WASD + mouse · SPACE jump · CLICK to launch ({List.length(model.balls)})")
  |> Ui.panel(Ui.topLeft())
