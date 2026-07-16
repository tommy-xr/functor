// hero.fun — the synthwave scene running live behind the functor.dev landing
// page (and the sandbox's opening example). A neon dot-grid rolls toward a
// glowing sun on the horizon. Animation is driven by the model's own clock
// (not tts), so a live edit in the sandbox keeps the wave rolling exactly
// where it was — hot reload preserves the model.

let gridCols = 12.0
let gridRows = 10.0
let spacing = 2.0

// The rolling wave: a diagonal sine marching toward the camera.
let waveY = (t: float, ix: float, iz: float): float =>
  Math.sin(iz * 0.8 - t * 2.2 + ix * 0.3) * 0.55

// One glowing grid dot. Near rows burn hot magenta; far rows cool toward
// violet so the grid reads as depth even without fog. The landing page mounts
// a live editor over exactly this region — edit the emissive and watch the
// running grid recolor with the wave still rolling.
// <editable>
let dot = (t, ix, iz) =>
  let depth = iz / gridRows in
  Scene.cube()
    |> Scene.emissive(Color.rgb(1.0 - 0.4 * depth, 0.15 + 0.1 * depth, 0.85 - 0.2 * depth))
    |> Scene.rotateY(Angle.radians(t * 0.7 + ix + iz))
    |> Scene.scale(0.21 + 0.07 * Math.sin(t * 3.0 + ix * 0.9 + iz * 0.6))
    |> Scene.translate(
         (ix - (gridCols - 1.0) / 2.0) * spacing,
         waveY(t, ix, iz),
         iz * spacing)
// </editable>

let row = (t, iz) =>
  Scene.group(List.range(gridCols) |> List.map((ix) => dot(t, ix, iz)))

let grid = (t) =>
  Scene.group(List.range(gridRows) |> List.map((iz) => row(t, iz)))

// The retro sun, low over the horizon, breathing slowly.
let sun = (t) =>
  Scene.sphere()
    |> Scene.emissive(Color.rgb(1.0, 0.36 + 0.08 * Math.sin(t * 1.7), 0.5))
    |> Scene.scale(6.5)
    |> Scene.translate(0.0, 3.2, 40.0)

// A flat backdrop quad well behind the sun: it fills the frame with the
// night-sky violet (the renderer has no clear-color hook). A quad's front is
// +Z and the camera looks down +Z, so rotate it to face the viewer.
let sky = () =>
  Scene.quad()
    |> Scene.emissive(Color.rgb(0.06, 0.015, 0.14))
    |> Scene.rotateY(Angle.degrees(180.0))
    |> Scene.scale(130.0)
    |> Scene.translate(0.0, 0.0, 55.0)

let ground = () =>
  Scene.plane()
    |> Scene.color(Color.rgb(0.045, 0.01, 0.1))
    |> Scene.scale(90.0)
    |> Scene.translate(0.0, -0.9, 20.0)

let init = { t: 0.0 }

let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }

let draw = (model, tts: float) =>
  Frame.create(
    Camera.lookAt(0.0, 3.4, -13.0, 0.0, 1.8, 12.0),
    Scene.group([sky(), ground(), sun(model.t), grid(model.t)]))
