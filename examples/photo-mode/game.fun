// gallery: Mistline Observatory — compose photographs across a fog-bound sculptural coast.
// gallery-controls: 1/2/3 viewpoints · A/D strafe · W/S dolly · Up/Down lens · P guides · V monitor

let monitor = RenderTarget.named("photo-mode-monitor")
  |> RenderTarget.sized(480.0, 270.0)

let init = {
  view: 1.0,
  x: 0.0,
  z: 0.0,
  fov: 42.0,
  guides: true,
  monitor: true,
  shots: 0.0,
}

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | Key.Num1 => { model with view: 1.0, x: 0.0, z: 0.0 }
    | Key.Num2 => { model with view: 2.0, x: 0.0, z: 0.0 }
    | Key.Num3 => { model with view: 3.0, x: 0.0, z: 0.0 }
    | Key.A => { model with x: model.x - 0.35 }
    | Key.D => { model with x: model.x + 0.35 }
    | Key.W => { model with z: model.z + 0.35 }
    | Key.S => { model with z: model.z - 0.35 }
    | Key.Up => { model with fov: Math.max(24.0, model.fov - 3.0) }
    | Key.Down => { model with fov: Math.min(70.0, model.fov + 3.0) }
    | Key.P => { model with guides: not model.guides }
    | Key.V => { model with monitor: not model.monitor }
    | Key.Space => { model with shots: model.shots + 1.0 }
    | _ => model

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
  match model.view with
  | 2.0 => Camera.firstPerson(
      Vec3.make(-6.8 + model.x, 3.2, 2.0 + model.z),
      Angle.degrees(30.0),
      Angle.degrees(-5.0),
      Angle.degrees(model.fov))
  | 3.0 => Camera.firstPerson(
      Vec3.make(5.6 + model.x, 1.65, 7.0 + model.z),
      Angle.degrees(-24.0),
      Angle.degrees(8.0),
      Angle.degrees(model.fov))
  | _ => Camera.firstPerson(
      Vec3.make(-0.7 + model.x, 1.55, -4.8 + model.z),
      Angle.degrees(2.5),
      Angle.degrees(5.0),
      Angle.degrees(model.fov))

let observerCamera = Camera.lookAt(
  Vec3.make(7.4, 6.0, -1.0),
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
      Sprite.line(ink, 0.025, {x: -5.33, y: 2.0}, {x: 5.33, y: 2.0}),
      Sprite.line(ink, 0.025, {x: -5.33, y: -2.0}, {x: 5.33, y: -2.0}),
      Sprite.line(ink, 0.025, {x: -2.67, y: -3.0}, {x: -2.67, y: 3.0}),
      Sprite.line(ink, 0.025, {x: 2.67, y: -3.0}, {x: 2.67, y: 3.0}),
      Sprite.line(ink, 0.04, {x: -0.22, y: 0.0}, {x: 0.22, y: 0.0}),
      Sprite.line(ink, 0.04, {x: 0.0, y: -0.22}, {x: 0.0, y: 0.22}),
    ])

let draw = (model, tts) =>
  let scene = world(model.monitor) in
  let monitorFrame = Frame.createLit(observerCamera, world(false), lights)
    |> Frame.withFog(fog)
    |> Frame.withClearColor(clear) in
  Frame.createLit(cameraFor(model), scene, lights)
    |> Frame.withFog(fog)
    |> Frame.withClearColor(clear)
    |> Frame.withRenderTarget(monitor, monitorFrame)
    |> Frame.with2D(Camera2D.create(16.0, 9.0), guides(model.guides))

let ui = (model) =>
  Ui.column([
    Ui.textColor(Color.rgb(1.0, 0.72, 0.42), "MISTLINE / PHOTO STUDY"),
    Ui.text(Text.concat("VIEW 0", Text.fixed(model.view, 0.0))),
    Ui.text(Text.concat("LENS ", Text.concat(Text.fixed(model.fov, 0.0), " DEG"))),
    Ui.text(Text.concat("EXPOSURES ", Text.fixed(model.shots, 0.0))),
    Ui.text("1 2 3 VIEW  /  WASD COMPOSE  /  P GUIDES  /  V MONITOR"),
  ]) |> Ui.panel(Ui.bottomLeft())
