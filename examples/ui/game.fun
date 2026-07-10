// ui — the widget showcase (docs/ui-interaction.md U5).
//
// Every interactive widget in one panel — buttons, a slider, a text input —
// each wired to model state that is echoed straight back as text, so the
// whole UI → msg → model → view loop is visible with no 3D scene in the way.
// The display vocabulary (row/column, corner anchors, colored text) frames
// it; a second panel pinned to the opposite corner echoes the full model.
//
// Drive it headlessly (docs/debug-runtime.md): interactive widgets number by
// SLOT in construction order — here 0 = "+1", 1 = "Reset", 2 = the slider,
// 3 = the text input:
//   POST /input {"type":"ui_event","slot":0,"kind":"Clicked"}
//   POST /input {"type":"ui_event","slot":2,"kind":{"SliderChanged":7.5}}
//   POST /input {"type":"ui_event","slot":3,"kind":{"TextChanged":"shark"}}
// then read the model back from GET /state.

type Model = { count: float, speed: float, name: string }
type Msg =
  | Inc
  | Reset
  | SetSpeed(v: float)
  | SetName(s: string)

let init = { count: 0.0, speed: 1.0, name: "functor" }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Inc => { m with count: m.count + 1.0 }
  | Reset => init
  | SetSpeed(v) => { m with speed: v }
  | SetName(s) => { m with name: s }

let tick = (m: Model, dt, tts) => m

// No scene — the showcase is the UI itself. `draw` is still the required
// frame source, so give it a camera over an empty group.
let draw = (m: Model, tts) =>
  Frame.create(
    Camera.lookAt(0.0, 1.5, -4.0, 0.0, 0.0, 0.0),
    Scene.group([])
  )

let counterRow = (m: Model) =>
  Ui.row([
    Ui.button("+1", Inc),
    Ui.button("Reset", Reset),
    Ui.text(Text.concat("count: ", Text.fixed(m.count, 0.0))),
  ])

let speedRow = (m: Model) =>
  Ui.row([
    Ui.slider(0.0, 10.0, m.speed, SetSpeed),
    Ui.text(Text.concat("speed: ", Text.fixed(m.speed, 1.0))),
  ])

let nameRow = (m: Model) =>
  Ui.row([
    Ui.textInput(m.name, SetName),
    Ui.text(Text.concat("name: ", m.name)),
  ])

// The full model echoed in one line, pinned to the opposite corner — the
// other anchors and colored text.
let echoPanel = (m: Model) =>
  Ui.textColor(
    1.0, 0.85, 0.4,
    Text.join("", [
      "model = { count: ", Text.fixed(m.count, 0.0),
      ", speed: ", Text.fixed(m.speed, 1.0),
      ", name: \"", m.name, "\" }",
    ])
  ) |> Ui.panel(Ui.bottomRight())

let ui = (m: Model) =>
  Ui.column([
    Ui.column([
      Ui.text("functor · ui showcase"),
      counterRow(m),
      speedRow(m),
      nameRow(m),
    ]) |> Ui.panel(Ui.topLeft()),
    echoPanel(m),
  ])
