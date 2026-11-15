// city-lights — a grid of instanced towers lit by waves of light.
//
// The towers are ONE `Scene.instancedPaint` node over a lit cube. Their
// placements are a top-level list built once; each frame rebuilds only the
// paint bytes (16 per tower: tint r, g, b and glow), so the GPU re-uploads
// 16 bytes per tower and never the transforms.
//
// Space freezes the light: the model keeps that frame's paint bytes and
// hands the same value back every frame, which costs nothing to convert or
// upload.
//
//   functor -d examples/city-lights run native
//   functor -d examples/city-lights test

// Towers per side: 64 is 4,096 towers; 100 is 10,000.
let side = 64.0
let spacing = 1.2

// ---- The city (built once) ---------------------------------------------------

let cells = List.range(side * side)

let column = (i: float): float => Math.mod(i, side)
let row = (i: float): float => Math.floor(i / side)

// A stable pseudo-random 0–1 per tower.
let hash = (i: float): float =>
  let s = Math.sin(i * 12.9898) * 43758.5453 in
  s - Math.floor(s)

// Taller toward the middle, with noise.
let distance = (i: float): float =>
  let x = column(i) - (side - 1.0) / 2.0 in
  let z = row(i) - (side - 1.0) / 2.0 in
  Math.sqrt(x * x + z * z)

let height = (i: float): float =>
  let centre = Math.max(0.0, 1.0 - distance(i) / (side * 0.6)) in
  0.3 + (0.4 + 3.6 * centre * centre) * (0.35 + 0.65 * hash(i))

let towers = cells |> List.map((i) =>
  let h = height(i) in
  Instance.trs(
    Vec3.make((column(i) - (side - 1.0) / 2.0) * spacing, h / 2.0,
      (row(i) - (side - 1.0) / 2.0) * spacing),
    Angle.degrees(0.0), 0.9, h, 0.9))

let distances = cells |> List.map(distance)

// ---- The light (per frame) ---------------------------------------------------

// Two rings run outward from the middle; each tower glows as one passes.
let glowAt = (d: float, tts: float): float =>
  let wave = 0.5 + 0.5 * Math.sin(d * 0.45 - tts * 2.4) in
  wave * wave * wave * wave

// A glowing tower turns from cool blue to amber and stops taking the light.
let paintFor = (d: float, tts: float): List<float> =>
  let g = glowAt(d, tts) in
  [Math.lerp(1.0, g, 0.45), Math.lerp(0.72, g, 0.55), Math.lerp(0.3, g, 1.0), g]

let paintsAt = (tts: float): Bytes.t =>
  Bytes.fromF32le(distances |> List.concatMap((d) => paintFor(d, tts)))

// ---- The game ----------------------------------------------------------------

type Model = { now: float, frozen: Option.t<Bytes.t> }

let init: Model = { now: 0.0, frozen: Option.None }

let tick = (model: Model, dt: float, tts: float) => { model with now: tts }

let input = (model: Model, key, isDown: bool) =>
  if isDown && key == Key.Space then
    { model with frozen: match model.frozen with
      | Option.Some(_) => Option.None
      | Option.None => Option.Some(paintsAt(model.now)) }
  else model

let draw = (model: Model, tts: float) =>
  let paints = match model.frozen with
    | Option.Some(still) => still
    | Option.None => paintsAt(tts) in
  let city =
    Scene.cube()
    |> Scene.lit(Color.rgb(0.85, 0.85, 0.9))
    |> Scene.instancedPaint(towers, paints) in
  let ground =
    Scene.plane()
    |> Scene.scaleXYZ(side * spacing + 4.0, 1.0, side * spacing + 4.0)
    |> Scene.lit(Color.rgb(0.05, 0.05, 0.08)) in
  let a = tts * 0.08 in
  let r = side * spacing * 0.85 in
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(Math.sin(a) * r, side * spacing * 0.5, Math.cos(a) * r),
      Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([ground, city]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.16)),
      Light.directional(Vec3.make(0.4, -1.0, 0.25), Color.rgb(0.8, 0.85, 1.0), 0.6)
        |> Light.castShadows,
    ])
  |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.05))

// ---- Tests -------------------------------------------------------------------

expect List.length(towers) == side * side
expect Bytes.length(paintsAt(1.5)) == 16.0 * side * side
expect glowAt(0.0, 0.0) > 0.0
expect List.all((d) => glowAt(d, 0.7) >= 0.0 && glowAt(d, 0.7) <= 1.0, distances)
