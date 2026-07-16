// examples/inspector — a tiny MVU game for the paused visual-debugger
// (docs/visual-debugger). A per-second `Sub.every` timer fires a `Tick`
// message; its `update` returns an `Effect.now` command, so ONE timer firing
// drives TWO `update` calls in a single frame — the timer message, then the
// effect result — plus the per-frame `tick`. Pausing on such a frame
// (`POST /time set` past a 1s boundary, or `advance` across one) makes
// `GET /trace` show `update` with count 2 and both provenance kinds:
//
//   functor -d examples/inspector run native --headless --debug-port 8077
//   curl -s -XPOST localhost:8077/time -d '{"type":"set","tts":0.9}'
//   curl -s -XPOST localhost:8077/time -d '{"type":"advance","dts":0.2}'
//   curl -s localhost:8077/trace | jq

type Msg =
  | Tick
  | GotTime(t: Float)

type Model = {
  ticks: Float,
  lastTime: Float,
}

let init = { ticks: 0.0, lastTime: 0.0 }

let update = (model: Model, msg: Msg) =>
  match msg with
  | Tick =>
    let bumped = model.ticks + 1.0 in
    ({ model with ticks: bumped }, Effect.now((t) => GotTime(t)))
  | GotTime(t) => { model with lastTime: t }

let subscriptions = (model: Model) => Sub.every(Time.seconds(1.0), Tick)

let tick = (model: Model, dt: Float, tts: Float) => model

let draw = (model: Model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.radians(tts)),
  )
