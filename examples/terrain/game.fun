// A finite 4 km × 4 km terrain rendered from a 16-bit heightmap.
//
// The runtime chooses a quadtree LOD from the stable center camera and draws
// every visible patch as an instance of one 64×64 grid. Both VR eyes therefore
// see the same geometry, and changing detail uploads only the compact patch
// list—not a world-sized mesh.
//
//   node examples/terrain/generate-heightmap.mjs
//   functor -d examples/terrain run native

let worldSize = 4000.0
let moveSpeed = 240.0
let sensitivity = 0.003
let pitchLimit = 1.45

let world =
  Terrain.heightmap(Asset.texture("heightmap.png"), worldSize, worldSize, -80.0, 520.0)
  |> Terrain.maxPixelError(2.0)
  |> Terrain.layered(
       Color.rgb(0.12, 0.30, 0.12),
       Color.rgb(0.34, 0.49, 0.20),
       Color.rgb(0.28, 0.25, 0.23),
       Color.rgb(0.86, 0.90, 0.91),
       340.0)
  |> Terrain.grass(13.0, 520.0, 5.5, Color.rgb(0.25, 0.46, 0.10))

let terrainBody = Physics.tag("terrain")

type Mouse =
  | NoMouse
  | MouseAt(x: float, y: float)

let init = {
  held: { up: false, down: false, left: false, right: false },
  eye: { x: -650.0, y: 280.0, z: -1200.0 },
  yaw: 0.15,
  pitch: 0.0 - 0.08,
  lastMouse: NoMouse,
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

let input = (model, key, isDown) =>
  { model with held: setHeld(model.held, key, isDown) }

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

let axis = (negative, positive) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

let tick = (model, dt, tts) =>
  let forward = axis(model.held.down, model.held.up) in
  let right = axis(model.held.left, model.held.right) in
  let distance = moveSpeed * dt in
  let eye = model.eye in
  { model with
      eye: {
        x: eye.x + distance * (forward * Math.sin(model.yaw) - right * Math.cos(model.yaw)),
        y: eye.y,
        z: eye.z + distance * (forward * Math.cos(model.yaw) + right * Math.sin(model.yaw)),
      } }

let physics = (model) =>
  Physics.scene(
    Vec3.make(0.0, -9.81, 0.0),
    [Physics.heightfield(terrainBody, world) |> Physics.friction(0.8)])

let draw = (model, tts) =>
  let camera =
    Camera.firstPerson(
      Vec3.make(model.eye.x, model.eye.y, model.eye.z),
      Angle.radians(model.yaw),
      Angle.radians(model.pitch),
      Angle.degrees(68.0))
    |> Camera.clip(0.5, 6000.0) in
  let water =
    Scene.plane()
    |> Scene.scale(worldSize)
    |> Scene.translate(Vec3.make(0.0, 28.0, 0.0))
    |> Scene.lit(Color.rgb(0.08, 0.24, 0.34)) in
  Frame.createLit(
    camera,
    Scene.group([Scene.terrain(world), water]),
    [
      Light.ambient(Color.rgb(0.20, 0.23, 0.28)),
      Light.directional(
        Vec3.make(0.42, -1.0, 0.28),
        Color.rgb(1.0, 0.84, 0.64),
        1.15),
    ])
  |> Frame.withFog(Fog.linear(700.0, 4800.0, Color.rgb(0.50, 0.64, 0.72)))

let ui = (model) =>
  Ui.text("4 km terrain · WASD + mouse")
  |> Ui.panel(Ui.topLeft())
