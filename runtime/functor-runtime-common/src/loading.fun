// The engine's loading view: what a shell draws while a project's top-level
// values load off the frame thread. A turning cube over a bar that fills with
// the load; a game draws its own with a `loading(progress, tts)` hook. Framed
// like the Quest boot scene, so it sits in front of the viewer in VR too.

let init = 0.0
let tick = (m, dt, tts) => m

let camera = Camera3D.lookAt(Vec3.make(0.0, 1.6, 0.5), Vec3.make(0.0, 1.2, -2.5))
let lights = [
  Light.ambient(Color.rgb(0.3, 0.3, 0.35)),
  Light.directional(Vec3.make(-0.4, -1.0, 0.3), Color.rgb(0.9, 0.85, 0.8), 1.0)
]

// A bar from the left edge; the fill is thicker, so it encloses the track.
let bar = (width, thick, color) =>
  Scene.cube()
  |> Scene.scaleXYZ(Math.max(width, 0.01), thick, thick)
  |> Scene.translate(Vec3.make(width / 2.0 - 0.6, 1.1, -2.5))
  |> Scene.lit(color)

// `progress` in 0..1; below 0, the load failed: the cube stops, red.
let view = (progress, tts) =>
  let failed = progress < 0.0 in
  let cube =
    Scene.cube()
    |> Scene.scale(0.3)
    |> Scene.rotateY(Angle.radians(if failed then 0.6 else tts * 2.0))
    |> Scene.translate(Vec3.make(0.0, 1.5, -2.5))
    |> Scene.lit(if failed then Color.rgb(0.9, 0.2, 0.2) else Color.rgb(0.25, 0.65, 1.0)) in
  Frame.createLit(
    camera,
    Scene.group([
      cube,
      bar(1.2, 0.04, Color.rgb(0.2, 0.2, 0.24)),
      bar(1.2 * Math.max(0.0, progress), 0.05, Color.rgb(0.9, 0.9, 0.95))
    ]),
    lights)

let draw = (m, tts) => view(m, tts)
