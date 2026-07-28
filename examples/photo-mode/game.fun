// photo-mode — an asset-free composition vignette.
//
// Mistline Observatory combines three authored viewpoints, an adjustable lens,
// atmospheric fog, a 4:3 rule-of-thirds overlay, and a live alternate-camera
// monitor. The monitor's writer renders world(false), which deliberately omits
// the monitor itself and avoids render-target feedback.
//
//   functor -d examples/photo-mode run native
//
// Controls:
//   1/2/3       authored viewpoints
//   Mouse       free-look from the selected viewpoint
//   W/S, A/D    dolly and strafe
//   Up/Down     narrow and widen the lens
//   P / V       toggle the 4:3 guides and the in-world monitor
//   Space       record an exposure in the model
//
// gallery: Mistline Observatory — compose photographs across a fog-bound sculptural coast.
// gallery-controls: 1/2/3 viewpoints · Mouse look · WASD compose · Up/Down lens · P guides · V monitor · Space exposure

let monitor = RenderTarget.named("photo-mode-monitor")
  |> RenderTarget.sized(480.0, 270.0)

// Mouse sensitivity is radians per pixel. Pitch stops just short of vertical
// so free-look cannot flip the authored composition upside down.
let mouseSensitivity = 0.003
let pitchLimit = 1.45

// Mouse positions are absolute window pixels. The first sample establishes a
// baseline so entering pointer capture or selecting a view never causes a jump.
type Mouse =
  | NoMouse
  | MouseAt(x: float, y: float)

let init = {
  view: 1.0,
  x: 0.0,
  z: 0.0,
  yawOffset: 0.0,
  pitchOffset: 0.0,
  lastMouse: NoMouse,
  fov: 42.0,
  guides: true,
  monitor: true,
  shots: 0.0,
  pHeld: false,
  vHeld: false,
  spaceHeld: false,
}

let yawFor = (view) =>
  match view with
  | 2.0 => 52.0
  | 3.0 => -68.0
  | _ => 2.5

let pitchFor = (view) =>
  match view with
  | 2.0 => -5.0
  | 3.0 => 8.0
  | _ => 5.0

let radians = (degrees) => degrees * Math.pi / 180.0

let yawRadians = (model) =>
  radians(yawFor(model.view)) + model.yawOffset

let pitchRadians = (model) =>
  radians(pitchFor(model.view)) + model.pitchOffset

let clamp = (value, low, high) =>
  if value < low then low
  else if value > high then high
  else value

let nudge = (model, forward, right) =>
  let yaw = yawRadians(model) in
  // With yaw zero looking down +Z in this right-handed view, camera-right is
  // -X. Rotate that basis with yaw so A/D stay relative to the visible gaze.
  let dx = Math.sin(yaw) * forward - Math.cos(yaw) * right in
  let dz = Math.cos(yaw) * forward + Math.sin(yaw) * right in
  { model with x: model.x + dx, z: model.z + dz }

let selectView = (model, view) =>
  { model with
      view: view,
      x: 0.0,
      z: 0.0,
      yawOffset: 0.0,
      pitchOffset: 0.0,
      lastMouse: NoMouse }

let input = (model, key, isDown) =>
  match isDown with
  | false =>
    (match key with
     | Key.P => { model with pHeld: false }
     | Key.V => { model with vHeld: false }
     | Key.Space => { model with spaceHeld: false }
     | _ => model)
  | true =>
    match key with
    | Key.Num1 => selectView(model, 1.0)
    | Key.Num2 => selectView(model, 2.0)
    | Key.Num3 => selectView(model, 3.0)
    | Key.A => nudge(model, 0.0, -0.35)
    | Key.D => nudge(model, 0.0, 0.35)
    | Key.W => nudge(model, 0.35, 0.0)
    | Key.S => nudge(model, -0.35, 0.0)
    | Key.Up => { model with fov: Math.max(24.0, model.fov - 3.0) }
    | Key.Down => { model with fov: Math.min(70.0, model.fov + 3.0) }
    | Key.P =>
      if model.pHeld then model
      else { model with guides: not model.guides, pHeld: true }
    | Key.V =>
      if model.vHeld then model
      else { model with monitor: not model.monitor, vHeld: true }
    | Key.Space =>
      if model.spaceHeld then model
      else { model with shots: model.shots + 1.0, spaceHeld: true }
    | _ => model

// Mouse right turns the camera right and mouse up looks up. Offsets are kept
// relative to each authored view, so pressing 1/2/3 restores its exact framing.
let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    let basePitch = radians(pitchFor(model.view)) in
    let nextPitch =
      clamp(
        basePitch + model.pitchOffset - (y - lastY) * mouseSensitivity,
        0.0 - pitchLimit,
        pitchLimit)
    in
    { model with
        yawOffset: model.yawOffset - (x - lastX) * mouseSensitivity,
        pitchOffset: nextPitch - basePitch,
        lastMouse: MouseAt(x, y) }

// GLFW may repeat key-down edges, so the toggles and exposure action latch
// until their matching release. Movement and lens controls intentionally keep
// repeat stepping.
expect (
  let first = input(init, Key.P, true) in
  let repeated = input(first, Key.P, true) in
  not repeated.guides && repeated.pHeld
)

expect (
  let first = input(init, Key.Space, true) in
  let released = input(first, Key.Space, false) in
  let second = input(released, Key.Space, true) in
  second.shots == 2.0 && second.spaceHeld
)

expect (
  let moved = input({ init with view: 2.0 }, Key.W, true) in
  moved.x > 0.0 && moved.z > 0.0
)

expect (
  let sampled = mouseMove(init, 400.0, 300.0) in
  sampled.yawOffset == 0.0
    && sampled.pitchOffset == 0.0
    && (match sampled.lastMouse with
        | MouseAt(x, y) => x == 400.0 && y == 300.0
        | NoMouse => false)
)

expect (
  let sampled = mouseMove(init, 400.0, 300.0) in
  let turned = mouseMove(sampled, 600.0, 200.0) in
  turned.yawOffset < -0.59
    && turned.yawOffset > -0.61
    && turned.pitchOffset > 0.29
    && turned.pitchOffset < 0.31
)

expect (
  let sampled = mouseMove(init, 400.0, 300.0) in
  let looked = mouseMove(sampled, 600.0, -1000.0) in
  pitchRadians(looked) > 1.44 && pitchRadians(looked) <= pitchLimit
)

expect (
  let sampled = mouseMove(init, 400.0, 300.0) in
  let looked = mouseMove(sampled, 600.0, 200.0) in
  let selected = input(looked, Key.Num2, true) in
  selected.view == 2.0
    && selected.yawOffset == 0.0
    && selected.pitchOffset == 0.0
    && (match selected.lastMouse with
        | NoMouse => true
        | MouseAt(_, _) => false)
)

expect (
  let looked = { init with yawOffset: Math.pi / 2.0 } in
  let moved = input(looked, Key.W, true) in
  moved.x > 0.3 && moved.z < 0.0
)

expect (
  let strafed = input(init, Key.D, true) in
  strafed.x < -0.3 && strafed.z > 0.0
)

let tick = (model, dt, tts) => model

let rock = (x, y, z, sx, sy, sz, shade) =>
  Scene.sphere()
    |> Scene.lit(shade)
    |> Scene.scaleXYZ(sx, sy, sz)
    |> Scene.translate(Vec3.make(x, y, z))

let arch = Scene.group([
  Scene.cube()
    |> Scene.lit(Color.rgb(0.24, 0.29, 0.31))
    |> Scene.scaleXYZ(0.7, 4.8, 0.8)
    |> Scene.rotateZ(Angle.degrees(-7.0))
    |> Scene.translate(Vec3.make(-1.9, 2.2, 7.2)),
  Scene.cube()
    |> Scene.lit(Color.rgb(0.20, 0.25, 0.28))
    |> Scene.scaleXYZ(0.7, 4.2, 0.8)
    |> Scene.rotateZ(Angle.degrees(9.0))
    |> Scene.translate(Vec3.make(1.8, 2.0, 7.2)),
  Scene.cube()
    |> Scene.lit(Color.rgb(0.30, 0.34, 0.34))
    |> Scene.scaleXYZ(4.3, 0.55, 0.8)
    |> Scene.rotateZ(Angle.degrees(2.0))
    |> Scene.translate(Vec3.make(0.0, 4.25, 7.2)),
])

let beacon = Scene.group([
  Scene.cylinder()
    |> Scene.lit(Color.rgb(0.15, 0.19, 0.21))
    |> Scene.scaleXYZ(0.32, 3.4, 0.32)
    |> Scene.translate(Vec3.make(0.0, 2.0, 12.5)),
  Scene.sphere()
    |> Scene.emissive(Color.rgb(1.0, 0.38, 0.16))
    |> Scene.scale(0.45)
    |> Scene.translate(Vec3.make(0.0, 5.6, 12.5)),
])

let shoreline = Scene.group([
  rock(-5.5, 0.25, 5.0, 2.8, 0.7, 2.0, Color.rgb(0.18, 0.25, 0.27)),
  rock(4.8, 0.15, 4.4, 3.4, 0.6, 2.2, Color.rgb(0.16, 0.23, 0.25)),
  rock(-4.0, 0.1, 10.0, 2.2, 0.55, 1.5, Color.rgb(0.22, 0.27, 0.28)),
  rock(3.8, 0.2, 9.5, 2.0, 0.7, 1.8, Color.rgb(0.19, 0.24, 0.26)),
  rock(-7.0, 0.6, 15.0, 4.0, 1.2, 3.0, Color.rgb(0.14, 0.21, 0.23)),
  rock(6.5, 0.5, 15.5, 3.6, 1.0, 2.8, Color.rgb(0.14, 0.20, 0.22)),
])

let world = (showMonitor) =>
  let water = Scene.plane()
    |> Scene.lit(Color.rgb(0.10, 0.24, 0.29))
    |> Scene.scaleXYZ(34.0, 1.0, 34.0)
    |> Scene.translate(Vec3.make(0.0, -0.35, 11.0)) in
  let path = Scene.cube()
    |> Scene.lit(Color.rgb(0.34, 0.31, 0.27))
    |> Scene.scaleXYZ(2.2, 0.18, 15.0)
    |> Scene.translate(Vec3.make(0.0, -0.12, 4.0)) in
  let screen = Scene.quad()
    |> Scene.screen(monitor)
    |> Scene.scaleXYZ(2.7, 1.52, 1.0)
    |> Scene.rotateY(Angle.degrees(-18.0))
    |> Scene.translate(Vec3.make(3.9, 2.25, 4.7)) in
  let screenFrame = Scene.cube()
    |> Scene.lit(Color.rgb(0.08, 0.09, 0.10))
    |> Scene.scaleXYZ(3.05, 1.85, 0.14)
    |> Scene.rotateY(Angle.degrees(-18.0))
    |> Scene.translate(Vec3.make(3.94, 2.25, 4.82)) in
  Scene.group([
    water,
    path,
    shoreline,
    arch,
    beacon,
    match showMonitor with
    | true => Scene.group([screenFrame, screen])
    | false => Scene.group([]),
  ])

let cameraFor = (model) =>
  let yaw = Angle.radians(yawRadians(model)) in
  let pitch = Angle.radians(pitchRadians(model)) in
  match model.view with
  | 2.0 => Camera.firstPerson(
      Vec3.make(-6.8 + model.x, 3.2, 2.0 + model.z),
      yaw,
      pitch,
      Angle.degrees(model.fov))
  | 3.0 => Camera.firstPerson(
      Vec3.make(5.6 + model.x, 1.65, 7.0 + model.z),
      yaw,
      pitch,
      Angle.degrees(model.fov))
  | _ => Camera.firstPerson(
      Vec3.make(-0.7 + model.x, 1.55, -4.8 + model.z),
      yaw,
      pitch,
      Angle.degrees(model.fov))

// The observer drifts across the fixed composition. Only the monitor sees this
// camera, so its motion makes the offscreen pass visibly live without moving
// the player's authored photograph.
let observerCamera = (tts) =>
  Camera.lookAt(
    Vec3.make(7.4 + Math.sin(tts * 0.8) * 2.0, 6.0, -1.0),
    Vec3.make(0.0, 2.0, 8.0))

let lights = [
  Light.ambient(Color.rgb(0.18, 0.24, 0.28)),
  Light.directional(
    Vec3.make(-0.45, -0.7, 0.35),
    Color.rgb(1.0, 0.52, 0.32),
    1.7) |> Light.castShadows,
  Light.point(Vec3.make(0.0, 5.6, 12.5), Color.rgb(1.0, 0.25, 0.08), 5.0, 14.0),
]

let fog = Fog.linear(8.0, 28.0, Color.rgb(0.08, 0.16, 0.21))
let clear = Color.rgb(0.08, 0.16, 0.21)

let guides = (enabled) =>
  match enabled with
  | false => Sprite.blank()
  | true =>
    let ink = Color.rgb(0.88, 0.82, 0.65) in
    Sprite.group([
      Sprite.line(ink, 0.025, {x: -8.0, y: 2.0}, {x: 8.0, y: 2.0}),
      Sprite.line(ink, 0.025, {x: -8.0, y: -2.0}, {x: 8.0, y: -2.0}),
      Sprite.line(ink, 0.025, {x: -2.67, y: -6.0}, {x: -2.67, y: 6.0}),
      Sprite.line(ink, 0.025, {x: 2.67, y: -6.0}, {x: 2.67, y: 6.0}),
      Sprite.line(ink, 0.04, {x: -0.22, y: 0.0}, {x: 0.22, y: 0.0}),
      Sprite.line(ink, 0.04, {x: 0.0, y: -0.22}, {x: 0.0, y: 0.22}),
    ])

let draw = (model, tts) =>
  let scene = world(model.monitor) in
  // Excluding the monitor from its own feed prevents recursive self-sampling.
  let monitorFrame = Frame.createLit(observerCamera(tts), world(false), lights)
    |> Frame.withFog(fog)
    |> Frame.withClearColor(clear) in
  let baseFrame = Frame.createLit(cameraFor(model), scene, lights)
    |> Frame.withFog(fog)
    |> Frame.withClearColor(clear) in
  let withMonitor =
    if model.monitor
    then baseFrame |> Frame.withRenderTarget(monitor, monitorFrame)
    else baseFrame
  in
  withMonitor
    |> Frame.with2D(Camera2D.create(16.0, 12.0), guides(model.guides))

let ui = (model) =>
  Ui.column([
    Ui.textColor(Color.rgb(1.0, 0.72, 0.42), "MISTLINE / PHOTO STUDY"),
    Ui.text(Text.concat("VIEW 0", Text.fixed(model.view, 0.0))),
    Ui.text(Text.concat("LENS ", Text.concat(Text.fixed(model.fov, 0.0), " DEG"))),
    Ui.text(Text.concat("EXPOSURES ", Text.fixed(model.shots, 0.0))),
    Ui.text("1 2 3 VIEW  /  MOUSE LOOK  /  WASD COMPOSE"),
    Ui.text("UP DOWN LENS"),
    Ui.text("P 4:3 GUIDES  /  V MONITOR  /  SPACE EXPOSURE"),
  ]) |> Ui.panel(Ui.bottomLeft())
