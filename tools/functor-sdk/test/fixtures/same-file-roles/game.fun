// Fixture for the SDK's entry resolution: TWO roles in ONE file, each an
// inline module — the object-shaped `entries` form
// (`{ "file": "game.fun", "module": "Client" }`) that `examples/orbs` uses.
// The unit tests read the sibling functor.json; this file exists so the
// fixture is a real, runnable project rather than a dangling manifest.

let spin = (tts: float) => Angle.degrees(tts * 30.0)

module Client {
  let init = { role: "client" }
  let tick = (m, dt: float, tts: float) => m
  let draw = (m, tts: float) =>
    Frame.create(
      Camera3D.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
      Scene.cube() |> Scene.rotateY(spin(tts)),
    )
}

module Server {
  let init = { role: "server" }
  let tick = (m, dt: float, tts: float) => m
  let draw = (m, tts: float) =>
    Frame.create(
      Camera3D.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
      Scene.sphere() |> Scene.rotateY(spin(tts)),
    )
}
