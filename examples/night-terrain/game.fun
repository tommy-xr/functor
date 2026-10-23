// A 4 km × 4 km terrain at night, crossed on foot by flashlight: the
// reference scene for a dark open-world VR game.
//
// One unshadowed spot light rides the camera and points where it looks (on
// XR, where the headset looks); a dim blue moon and ambient leave only faint
// silhouettes beyond its cone, fading into dark fog. ~200 props — trees and
// rocks — are scattered from a seed and stood on the ground by raycasting the
// same heightfield the terrain draws.
//
//   node examples/night-terrain/generate-heightmap.mjs
//   functor -d examples/night-terrain run native

// ── Tunables (hot-reload: save and the scene updates in place) ──────────────

let fogDensity = 0.018
let fogColor = Color.rgb(0.010, 0.013, 0.024)
// Darker than the fog, so the sky reads as night rather than haze.
let skyColor = Color.rgb(0.004, 0.005, 0.012)

let flashlightCone = Angle.degrees(21.0)
let flashlightRange = 75.0
let flashlightIntensity = 2.6
let flashlightColor = Color.rgb(1.0, 0.92, 0.78)

let moonIntensity = 0.32
let moonColor = Color.rgb(0.55, 0.66, 1.0)
let moonDirection = Vec3.make(0.35, -0.55, 0.75)
let ambientColor = Color.rgb(0.020, 0.024, 0.040)

let propCount = 200.0
let propSeed = 7.0
// Props spread over a square this wide around the spawn: 200 across the whole
// 4 km map would be ~280 m apart, never two inside the flashlight at once.
let scatterSize = 240.0
let treeShare = 0.7

// ── World ───────────────────────────────────────────────────────────────────

let worldSize = 4000.0
let minHeight = 0.0
let maxHeight = 190.0
let walkSpeed = 6.0
let sensitivity = 0.003
let pitchLimit = 1.45
let eyeHeight = 1.7
let spawnX = 0.0 - 1260.0
let spawnZ = 0.0 - 1180.0

let world =
  Terrain.heightmap(Asset.texture("night-heightmap.png"), worldSize, worldSize, minHeight, maxHeight)
  |> Terrain.maxPixelError(2.0)
  |> Terrain.layered(
       Color.rgb(0.20, 0.30, 0.14),
       Color.rgb(0.32, 0.36, 0.20),
       Color.rgb(0.36, 0.33, 0.30),
       Color.rgb(0.80, 0.82, 0.84),
       // Above the highest ground: no snow band.
       1000.0)

let terrainBody = Physics.tag("terrain")

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0),
                [Physics.heightfield(terrainBody, world)])

// Straight down through the whole height range. Misses off the map, and
// everywhere before the heightfield's first physics step.
let groundAt = (x, z) =>
  Physics.cast(Vec3.make(x, maxHeight + 10.0, z),
               Vec3.make(0.0, -1.0, 0.0),
               maxHeight - minHeight + 20.0)

// ── Props ───────────────────────────────────────────────────────────────────

// One prop's placement, drawn from its own forked stream so changing the
// count adds or removes props without reshuffling the others.
let scatterProp = (i) =>
  let half = scatterSize / 2.0 in
  let s0 = Random.seed(propSeed) |> Random.fork(i) in
  let (dx, s1) = Random.range(0.0 - half, half, s0) in
  let (dz, s2) = Random.range(0.0 - half, half, s1) in
  let (yaw, s3) = Random.range(0.0, 2.0 * Math.pi, s2) in
  let (size, s4) = Random.range(0.7, 1.5, s3) in
  let (pick, _) = Random.step(s4) in
  { tree: pick < treeShare, x: spawnX + dx, z: spawnZ + dz, yaw: yaw, size: size }

// Top-level, so it is computed once per load — and again on every hot reload,
// which is what lets a seed or count edit re-scatter a running game.
let scattered = List.range(propCount) |> List.map(scatterProp)

expect List.length(scattered) == propCount
expect scattered |> List.all((p) =>
  Math.abs(p.x - spawnX) <= scatterSize / 2.0 && Math.abs(p.z - spawnZ) <= scatterSize / 2.0)
expect (scattered |> List.filter((p) => p.tree) |> List.length) > 100.0

// Ground heights need the physics world, so they settle in `tick` once per
// distinct scatter, as soon as the heightfield exists (the spawn probe hits).
// A prop scattered off the map edge has no ground and is dropped.
let settle = (model) =>
  let ready = groundAt(spawnX, spawnZ) in
  if model.settledFrom == scattered || not ready.hit then model
  else
    let placed =
      scattered
      |> List.map((p) =>
           let hit = groundAt(p.x, p.z) in
           { tree: p.tree, x: p.x, y: hit.y, z: p.z, yaw: p.yaw, size: p.size, hit: hit.hit })
      |> List.filter((p) => p.hit) in
    { model with
        settledFrom: scattered,
        trees: placed |> List.filter((p) => p.tree),
        rocks: placed |> List.filter((p) => not p.tree) }

// ── Input and movement ──────────────────────────────────────────────────────

type MousePos =
  | NoMouse
  | MouseAt(x: float, y: float)

let init = {
  held: { up: false, down: false, left: false, right: false },
  yaw: 0.35,
  pitch: 0.0 - 0.14,
  lastMouse: NoMouse,
  eye: { x: spawnX, y: 12.0, z: spawnZ },
  head: Option.None,
  settledFrom: [],
  trees: [],
  rocks: [],
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

let input = (model, key, isDown) => { model with held: setHeld(model.held, key, isDown) }

let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: Math.clamp(0.0 - pitchLimit, pitchLimit, model.pitch - (y - lastY) * sensitivity),
        lastMouse: MouseAt(x, y) }

// The headset's rig-local head pose (None off XR). On XR the camera composes
// head motion itself; the flashlight needs the pose to follow it.
let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => { model with head: Option.None }
  | Option.Some(xr) => { model with head: xr.head }

// Where the camera points: yaw 0 / pitch 0 looks down +Z, positive pitch up.
let lookDir = (yaw, pitch) =>
  let flat = Math.cos(pitch) in
  Vec3.make(Math.sin(yaw) * flat, Math.sin(pitch), Math.cos(yaw) * flat)

let axis = (negative, positive) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

// A kinematic walker: steer in XZ, then snap the eye to the ground below.
let tick = (model, dt, tts) =>
  let forward = axis(model.held.down, model.held.up) in
  let right = axis(model.held.left, model.held.right) in
  let step =
    Vec3.make(forward * Math.sin(model.yaw) - right * Math.cos(model.yaw),
              0.0,
              forward * Math.cos(model.yaw) + right * Math.sin(model.yaw))
    |> Vec3.normalize
    |> Vec3.scale(walkSpeed * dt) in
  // Stop a metre short of the map edge, where the ground probe would miss.
  let edge = worldSize / 2.0 - 1.0 in
  let x = Math.clamp(0.0 - edge, edge, model.eye.x + Vec3.x(step)) in
  let z = Math.clamp(0.0 - edge, edge, model.eye.z + Vec3.z(step)) in
  let ground = groundAt(x, z) in
  let y = if ground.hit then ground.y + eyeHeight else model.eye.y in
  settle({ model with eye: { x: x, y: y, z: z } })

// ── Rendering ───────────────────────────────────────────────────────────────

let trunk = Scene.cylinder() |> Scene.lit(Color.rgb(0.15, 0.11, 0.08))
let canopy = Scene.sphere() |> Scene.lit(Color.rgb(0.10, 0.22, 0.12))
let rock = Scene.sphere() |> Scene.lit(Color.rgb(0.42, 0.41, 0.40))

// Unit cylinder: 1 tall, centred; unit sphere: radius 1. Each sits slightly
// sunk so slopes never show a gap under it.
let trunkAt = (p) =>
  Instance.trs(Vec3.make(p.x, p.y + 2.0 * p.size, p.z), Angle.radians(p.yaw),
               0.35 * p.size, 4.8 * p.size, 0.35 * p.size)

let canopyAt = (p) =>
  Instance.trs(Vec3.make(p.x, p.y + 6.2 * p.size, p.z), Angle.radians(p.yaw),
               1.7 * p.size, 3.6 * p.size, 1.7 * p.size)

let rockAt = (p) =>
  Instance.trs(Vec3.make(p.x, p.y + 0.2 * p.size, p.z), Angle.radians(p.yaw),
               2.0 * p.size, 1.0 * p.size, 1.4 * p.size)

let draw = (model, tts) =>
  let eye = Vec3.make(model.eye.x, model.eye.y, model.eye.z) in
  // Past ~300 m the fog is opaque, so a farther plane only draws hidden terrain.
  let camera =
    Camera3D.firstPerson(eye, Angle.radians(model.yaw), Angle.radians(model.pitch), Angle.degrees(68.0))
    |> Camera3D.clip(0.3, 1000.0) in
  // On XR the beam follows the tracked head, mapped through the same camera.
  let (from, dir) =
    match model.head with
    | Option.None => (eye, lookDir(model.yaw, model.pitch))
    | Option.Some(pose) =>
      let m = Camera3D.mapTrackedPose(camera, pose) in
      (Vec3.make(m.position.x, m.position.y, m.position.z),
       Vec3.make(m.forward.x, m.forward.y, m.forward.z)) in
  // Held just below the eye, so the beam's falloff reads across the ground.
  let flashlight =
    Light.spot(from |> Vec3.add(Vec3.make(0.0, -0.25, 0.0)), dir,
               flashlightColor, flashlightIntensity, flashlightRange, flashlightCone) in
  Frame.createLit(
    camera,
    Scene.group([
      Scene.terrain(world),
      trunk |> Scene.instanced(model.trees |> List.map(trunkAt)),
      canopy |> Scene.instanced(model.trees |> List.map(canopyAt)),
      rock |> Scene.instanced(model.rocks |> List.map(rockAt)),
    ]),
    [
      Light.ambient(ambientColor),
      Light.directional(moonDirection, moonColor, moonIntensity),
      flashlight,
    ])
  |> Frame.withFog(Fog.exp(fogDensity, fogColor))
  |> Frame.withClearColor(skyColor)

let ui = (model) =>
  Ui.text("Night terrain · WASD + mouse")
  |> Ui.panel(Ui.topLeft())
