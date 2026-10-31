// `functor reads` golden fixture: see cli/tests/reads.rs.
let level = World.load("level.bin")
let reader = (p) => File.exists(Path.of(p))
let applyTo = (f, x) => f(x)
let probe = (p) => applyTo(reader, p)
let init = () => { y: level.height }
let groundAt = (x) => Physics.cast(Vec3.make(x, 10.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 20.0).hit
let tick = (model, tts) => if groundAt(0.0) then model else { y: model.y - 1.0 }
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.translate(Vec3.make(0.0, model.y + level.height, 0.0)))
let ui = (model) => Ui.column([Ui.text(level.name), Ui.button("jump", 1.0)])
