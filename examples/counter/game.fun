// counter — the interactive-UI hello world (docs/ui-interaction.md U3).
//
// A `Ui.button` click delivers its msg (Inc) verbatim through `update`; the
// HUD echoes the count and the cube turns 15° per click, so the whole
// UI → msg → model → view loop is visible. Drive it headlessly with the
// debug server: POST /input {"type":"ui_event","slot":0,"kind":"Clicked"}
// (the button is slot 0 — the tree's first interactive widget), then read
// the count back from GET /state.

type Model = { count: float }
type Msg = | Inc

let init = { count: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Inc => { m with count: m.count + 1.0 }

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 1.5, -4.0, 0.0, 0.0, 0.0),
    Scene.cube()
      |> Scene.lit(0.3, 0.65, 0.9)
      |> Scene.rotateY(Angle.degrees(m.count * 15.0)),
    [
      Light.ambient(0.25, 0.25, 0.25),
      Light.directional(-0.5, -1.0, 0.4, 1.0, 1.0, 1.0, 0.9),
    ],
  )

let ui = (m: Model) =>
  Ui.column([
    Ui.text(Text.concat("count: ", Text.fixed(m.count, 0.0))),
    Ui.button("+1", Inc),
  ]) |> Ui.panel(Ui.topLeft())
