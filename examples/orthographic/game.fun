// examples/orthographic — two equal cubes, the right one 8 units farther
// away, through an orthographic camera: they draw the same size. (The
// camera looks down a little, so the far one sits higher on screen; it is
// set 1 lower to stay near the middle.)
//
//   functor -d examples/orthographic run native

let init = 0.0
let tick = (m, dt, tts) => m

let cube = (x: float, y: float, z: float, color) =>
  Scene.cube() |> Scene.lit(color) |> Scene.translate(Vec3.make(x, y, z))

let draw = (m, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, 6.0), Vec3.make(0.0, 0.0, 0.0))
      |> Camera3D.clip(1.0, 30.0)
      |> Camera3D.orthographic(2.0),
    Scene.group([
      cube(-1.5, 0.0, 0.0, Color.rgb(0.25, 0.5, 0.85)),
      cube(1.5, -1.0, -8.0, Color.rgb(0.9, 0.55, 0.2)),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.28)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.6), Color.rgb(1.0, 0.97, 0.92), 0.9),
    ])
    |> Frame.withClearColor(Color.rgb(0.07, 0.07, 0.08))
