// The file's TOP LEVEL deliberately declares no init/tick/draw: if the page
// booted the plain contract it would fail to load, so a successful load proves
// the `module Server` role resolved on wasm.
let spinRate = 1.5

module Server {
  let init = { spin: 0.0 }

  let tick = (model, dt, tts) => { spin: model.spin + dt * spinRate }

  let draw = (model, tts) =>
    Frame.create(
      Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
      Scene.cube() |> Scene.rotateY(Angle.radians(model.spin)))
}
