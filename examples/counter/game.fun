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

// Inline tests: `expect` runs under `functor-lang test game.fun` (and live
// in the editor gutter — red/green as you type), never in the game loop.
// The functional core makes MVU logic directly testable: update is just a
// function of (model, msg).
expect update(init, Inc).count == 1.0
expect update(update(init, Inc), Inc).count == 2.0
expect (
  let clicked = update(init, Inc) in    // a let-in chain is the setup block
  clicked.count > init.count
)

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube()
      |> Scene.lit(Color.rgb(0.3, 0.65, 0.9))
      |> Scene.rotateY(Angle.degrees(m.count * 15.0)),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.25)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 1.0, 1.0), 0.9),
    ],
  )

let ui = (m: Model) =>
  Ui.column([
    Ui.text(Text.concat("count: ", Text.fixed(m.count, 0.0))),
    Ui.button("+1", Inc),
  ]) |> Ui.panel(Ui.topLeft())
