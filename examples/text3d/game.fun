// text3d — text in the world: a hundred labels that face the camera, and a
// sign set flat on a wall, all drawn from one signed-distance-field atlas.
//
// Every label is `Scene.text`; `Scene.facing` turns it to the camera, so the
// orbit never shows an edge. The sign is a `Scene.textBlock` wrapped to the
// panel's width and seen at a glancing angle. All of it is one draw.
//
//   functor -d examples/text3d run native
//   functor -d examples/text3d test

let side = 10.0
let spacing = 3.4

let lots = List.range(side * side)

let at = (i: float): Vec3.t =>
  Vec3.make((Math.mod(i, side) - (side - 1.0) / 2.0) * spacing, 0.0,
    (Math.floor(i / side) - (side - 1.0) / 2.0) * spacing)

// A stable pseudo-random 0–1 per lot.
let hash = (i: float): float =>
  let s = Math.sin(i * 12.9898) * 43758.5453 in
  s - Math.floor(s)

let height = (i: float): float => 0.6 + 2.4 * hash(i)

let names = ["main", "draw", "tick", "scene", "layout", "parse", "lower", "check", "eval", "render"]

let nameOf = (i: float): string =>
  match List.nth(Math.mod(i, 10.0), names) with
  | Option.Some(name) => $"{name}.fun:{Math.floor(i / 10.0) * 10.0 + 12.0}"
  | Option.None => "?"

let towers =
  Scene.cube()
  |> Scene.lit(Color.rgb(0.32, 0.4, 0.62))
  |> Scene.instanced(lots |> List.map((i) =>
      Instance.at(Vec3.add(at(i), Vec3.make(0.0, height(i) / 2.0, 0.0)))
      |> Instance.scaleXYZ(1.2, height(i), 1.2)))

let label = (i: float) =>
  Scene.text(nameOf(i))
  |> Scene.scale(0.4)
  |> Scene.facing
  |> Scene.translate(Vec3.add(at(i), Vec3.make(0.0, height(i) + 0.45, 0.0)))

let labels = Scene.group(lots |> List.map(label)) |> Scene.emissive(Color.rgb(1.0, 0.93, 0.75))

let signText = "TEXT3D\nEvery label here is Scene.text from one Inter atlas: crisp near and far, at any angle, in one draw."

let sign =
  Scene.group([
    Scene.quad() |> Scene.scaleXYZ(9.0, 4.4, 1.0) |> Scene.emissive(Color.rgb(0.06, 0.05, 0.12)),
    Scene.textBlock(19.0, 6.0, signText)
      |> Scene.scale(0.42)
      |> Scene.translate(Vec3.make(0.0, 0.0, 0.02))
      |> Scene.emissive(Color.rgb(0.55, 0.95, 1.0)),
  ])
  |> Scene.rotateY(Angle.degrees(35.0))
  |> Scene.translate(Vec3.make(-8.0, 8.0, -22.0))

let init = 0.0
let tick = (model: float, dt: float, tts: float) => tts

let draw = (model: float, tts: float) =>
  let a = tts * 0.15 in
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(Math.sin(a) * 26.0, 15.0, Math.cos(a) * 26.0 + 2.0),
      Vec3.make(0.0, 1.0, -4.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(40.0) |> Scene.lit(Color.rgb(0.08, 0.08, 0.1)),
      towers,
      labels,
      sign,
    ]),
    [
      Light.ambient(Color.rgb(0.2, 0.2, 0.26)),
      Light.directional(Vec3.make(0.3, -1.0, -0.4), Color.rgb(0.9, 0.9, 1.0), 0.7),
    ])
  |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.05))

expect nameOf(0.0) == "main.fun:12"
expect nameOf(23.0) == "scene.fun:32"
