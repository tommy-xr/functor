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
// `C` switches the shading between the lit, layered terrain and a baked
// colormap (unlit: lighting painted into a day and a night map offline). `N`
// toggles night, cross-fading the maps and the frame's lights and fog. `L`
// toggles a flashlight: the one light the colormap terrain still receives.
//
//   node examples/terrain/generate-heightmap.mjs
//   node examples/terrain/generate-colormaps.mjs
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

let ground =
  Terrain.heightmap(Asset.texture("heightmap.png"), worldSize, worldSize, -80.0, 520.0)
  // 4 px fits the Quest vertex budget; 8 swims in stereo.
  |> Terrain.maxPixelError(4.0)

// The bands alone: what `baked` shows until its maps have streamed in.
let banded =
  ground
  |> Terrain.layered(
       Color.rgb(0.12, 0.30, 0.12),
       Color.rgb(0.34, 0.49, 0.20),
       Color.rgb(0.28, 0.25, 0.23),
       Color.rgb(0.86, 0.90, 0.91),
       340.0)

let world =
  banded
  |> Terrain.textured(
       Asset.texture("detail-low.jpg"),
       Asset.texture("detail-high.jpg"),
       Asset.texture("detail-rock.jpg"),
       Asset.texture("detail-snow.jpg"),
       8.0)
  |> Terrain.grass(2.2, 140.0, 1.1, Color.rgb(0.25, 0.46, 0.10))

// The same ground, unlit: generate-colormaps.mjs bakes the bands, sun and
// occlusion into the maps. `blend` is 0 for day, 1 for night.
let baked = (blend) =>
  banded
  |> Terrain.colormap(Asset.texture("colormap-day.png"), Asset.texture("colormap-night.png"), blend)
  |> Terrain.colormapDetail(Asset.texture("detail-high.jpg"), 8.0, 80.0)
  // Lets the flashlight light the unlit ground.
  |> Terrain.normalMap(Asset.texture("normalmap.png"))

// Held just below the eye and aimed where you look, as in night-terrain.
let flashlightCone = Angle.degrees(20.0)
let flashlightRange = 60.0
let flashlightIntensity = 2.6
let flashlightColor = Color.rgb(1.0, 0.92, 0.78)

// Seconds for a day ↔ night cross-fade.
let fadeSeconds = 2.0

type Shading =
  | Lit
  | Baked

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
  shading: Lit,
  night: false,
  blend: 0.0,
  flashlight: false,
  // Held toggle keys: a held key auto-repeats, and must flip only once.
  toggleDown: { c: false, n: false, l: false },
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


let clamp = (value: float, lo, hi) =>
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
  | Key.C =>
    let flip = isDown && not model.toggleDown.c in
    { model with
        shading: if flip then (match model.shading with | Lit => Baked | Baked => Lit) else model.shading,
        toggleDown: { model.toggleDown with c: isDown } }
  | Key.N =>
    let flip = isDown && not model.toggleDown.n in
    { model with
        night: if flip then not model.night else model.night,
        toggleDown: { model.toggleDown with n: isDown } }
  | Key.L =>
    let flip = isDown && not model.toggleDown.l in
    { model with
        flashlight: if flip then not model.flashlight else model.flashlight,
        toggleDown: { model.toggleDown with l: isDown } }
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
  let fade = dt / fadeSeconds in
  let next =
    { model with
        grounded: probe.hit,
        eye: { x: pos.x, y: pos.y + eyeOffset, z: pos.z },
        blend: clamp(if model.night then model.blend + fade else model.blend - fade, 0.0, 1.0) } in
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
  let terrain =
    match model.shading with
    | Lit => world
    | Baked => baked(model.blend) in
  // Day → night. The night lights match the colormap bake's moonlight, so
  // lit props agree with the ground under them.
  let b = model.blend in
  let sky = [
    Light.ambient(Color.rgb(0.20 |> Math.lerp(0.06, b), 0.23 |> Math.lerp(0.07, b), 0.28 |> Math.lerp(0.12, b))),
    Light.directional(
      Vec3.make(0.42, -1.0, 0.28) |> Vec3.lerp(Vec3.make(-0.35, -0.8, -0.45), b),
      Color.rgb(1.0 |> Math.lerp(0.5, b), 0.84 |> Math.lerp(0.6, b), 0.64 |> Math.lerp(0.85, b)),
      1.15 |> Math.lerp(0.5, b)),
  ] in
  let flashlight =
    Light.spot(Vec3.make(model.eye.x, model.eye.y - 0.25, model.eye.z), lookDir(model.yaw, model.pitch),
               flashlightColor, flashlightIntensity, flashlightRange, flashlightCone) in
  Frame.createLit(
    camera,
    Scene.group([Scene.terrain(terrain), water]
                |> List.append(model.balls |> List.map(ballNode))),
    if model.flashlight then sky |> List.append([flashlight]) else sky)
  |> Frame.withFog(
       Fog.linear(700.0, 4800.0,
                  Color.rgb(0.50 |> Math.lerp(0.03, b), 0.64 |> Math.lerp(0.04, b), 0.72 |> Math.lerp(0.07, b))))

let shadingLabel = (model) =>
  match model.shading with
  | Lit => ""
  | Baked => " · baked colormap"

let flashlightLabel = (model) => if model.flashlight then " · flashlight" else ""

let ui = (model) =>
  Ui.text($"4 km terrain · WASD + mouse · SPACE jump · CLICK to launch ({List.length(model.balls)}){shadingLabel(model)}{flashlightLabel(model)}")
  |> Ui.panel(Ui.topLeft())
