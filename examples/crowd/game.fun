// Crowd benchmark: `count` Xbots in a grid, each on its own playhead and
// walk/run blend weight, so every figure is posed independently every frame.
// Shadows are on, as in a normal scene. The whole crowd stays in view.
//
// Set `count` (1 / 25 / 50 / 100) and save: hot reload applies it live, and
// `functor run vr` re-pushes it to the headset. `GET /state` reports the
// frame's `skinned` draws / pose evaluations and the shell's `stages` timings.

let count = 100.0

let spacing = 1.2

let columns = Math.ceil(Math.sqrt(count))

let rows = Math.ceil(count / columns)

// A deterministic per-figure fraction in [0, 1): the golden-ratio sequence.
let fraction = (i: float): float => Math.mod(i * 0.618034, 1.0)

let init = {}

let tick = (model, dt: float, tts: float) => model

// Both clips are sampled for every figure: the run weight stays in
// [0.25, 0.75], so neither blend entry is dropped.
let figure = (i: float, tts: float): Scene.t =>
  let row = Math.floor(i / columns) in
  let column = i - row * columns in
  let playhead = tts + fraction(i + 0.5) * 2.0 in
  let run = 0.25 + 0.5 * fraction(i) in
  Scene.model(Assets.xbot)
    |> Scene.animate(Anim.blend([
         (Anim.clip(Assets.xbotClips.walk.name, playhead), 1.0 - run),
         (Anim.clip(Assets.xbotClips.run.name, playhead), run),
       ]))
    // glTF forward is +Z; turn each figure to face the camera.
    |> Scene.rotateY(Angle.degrees(180.0))
    |> Scene.translate(Vec3.make((column - (columns - 1.0) / 2.0) * spacing, 0.0, row * spacing))

// Back the camera off with the grid's size so every figure is in frame.
let extent = Math.max(columns, rows) * spacing

let camera =
  Camera3D.lookAt(
    Vec3.make(0.0, 1.5 + extent * 0.5, 0.0 - (2.5 + extent)),
    Vec3.make(0.0, 0.9, (rows - 1.0) * spacing / 2.0))

let draw = (model, tts: float) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(extent + 6.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      Scene.group(List.range(count) |> List.map((i) => figure(i, tts))),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0) |> Light.castShadows,
    ])
