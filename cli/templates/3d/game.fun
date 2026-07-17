// A small Functor scene. Run it with:
//
//   functor run native
//
// Saving this file while the game runs hot-reloads it immediately.

let init = {}

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(6.0, 4.0, -8.0), Vec3.make(0.0, 0.5, 0.0)),
    Scene.group([
      Scene.plane()
        |> Scene.scale(20.0)
        |> Scene.lit(Color.rgb(0.35, 0.38, 0.42)),
      Scene.cube()
        |> Scene.rotateY(Angle.radians(tts * 0.5))
        |> Scene.translate(Vec3.make(0.0, 0.75, 0.0))
        |> Scene.lit(Color.rgb(0.25, 0.65, 1.0)),
      Scene.sphere()
        |> Scene.scale(0.55)
        |> Scene.translate(Vec3.make(-2.0, 0.55, 1.0))
        |> Scene.lit(Color.rgb(1.0, 0.35, 0.25)),
    ]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.16)),
      Light.directional(Vec3.make(0.5, -1.0, 0.35), Color.rgb(1.0, 0.96, 0.9), 1.0)
        |> Light.castShadows,
    ])
