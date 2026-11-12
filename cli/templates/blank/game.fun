// The smallest Functor game: an empty scene and a camera. Run it with:
//
//   functor run native
//
// Saving this file while the game runs hot-reloads it immediately.

let init = {}

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([]))
    |> Frame.withClearColor(Color.rgb(0.1, 0.12, 0.18))

// `functor test` runs every `expect` in the project.
expect tick(init, 0.016, 0.0) == init
