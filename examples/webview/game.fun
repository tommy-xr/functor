// webview — the HTML/CSS overlay hello world.
//
// `webview(model)` returns an Elm-style `Html.*` tree styled with real CSS
// (flexbox, gradients, border-radius, :hover). Natively it renders through
// blitz (Stylo + Taffy + Parley) composited over the 3D frame; on wasm it is
// a real DOM overlay above the canvas. An `Attr.onClick` click delivers its
// msg verbatim through `update` — the `Ui.button` loop, with CSS styling.
//
// Styling is split two ways: per-element INLINE styles are typed
// (`Attr.styles([Style.fontSizePx(40.0), …])` — a typo is a check error),
// while the cascade half (selectors, :hover) stays a CSS string in
// `Html.style`.

type Model = { count: float, spin: float, name: string }
type Msg =
  | Inc
  | Dec
  | Reset
  | SetName(name: string)

let init = { count: 0.0, spin: 0.0, name: "" }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Inc => { m with count: m.count + 1.0 }
  | Dec => { m with count: m.count - 1.0 }
  | Reset => { m with count: 0.0 }
  | SetName(name) => { m with name: name }

let tick = (m: Model, dt, tts) => { m with spin: m.spin + dt * 20.0 }

let draw = (m: Model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube()
      |> Scene.lit(Color.rgb(0.35, 0.75, 0.55))
      |> Scene.rotateY(Angle.degrees(m.spin + m.count * 15.0)),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.25)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 1.0, 1.0), 0.9),
    ],
  )

// The stylesheet keeps the cascade half of CSS — selectors, :hover, the
// gradient — themeable without touching the tree. .hud is a fixed-width card
// pinned by its margin; buttons get :hover.
let css = "
  .hud { display: flex; flex-direction: column; gap: 12px; width: 300px;
         margin: 24px; padding: 20px;
         background: linear-gradient(135deg, rgba(24, 26, 44, 0.92), rgba(52, 24, 64, 0.92));
         border: 2px solid #8be9fd; border-radius: 14px;
         font-family: sans-serif; color: #f8f8f2; }
  .hud h1 { margin: 0; font-size: 22px; color: #8be9fd; }
  button { padding: 8px 18px; font-size: 18px; font-weight: bold;
           background: #50fa7b; color: #1e1e3c; border: none; border-radius: 8px; }
  button:hover { background: #f1fa8c; }
  button.ghost { background: transparent; color: #8be9fd; border: 1px solid #8be9fd; }
  input { padding: 8px 10px; font-size: 16px; font-family: sans-serif;
          background: rgba(10, 12, 28, 0.9); color: #f8f8f2;
          border: 1px solid #8be9fd; border-radius: 8px; }
"

// A CONTROLLED text input (the Ui.textInput loop, HTML flavor): the field
// shows the MODEL's text, each keystroke delivers SetName(newText) through
// `update`, and the greeting proves the round-trip. Natively, keys route
// into the blitz document while the field is focused (the game's `input`
// hook is suppressed; Escape defocuses first, releases the cursor second).
let greeting = (m: Model) =>
  match m.name == "" with
  | true => "Type your name above."
  | false => Text.concat("Hello, ", Text.concat(m.name, "!"))

let webview = (m: Model) =>
  Html.div([], [
    Html.style(css),
    Html.div([Attr.class("hud")], [
      Html.h1([], [Html.text("Functor webview")]),
      // Typed inline styles: `Style.fontSizPx(40.0)` would be a check error.
      Html.div(
        [Attr.styles([Style.fontSizePx(40.0), Style.bold(), Style.textCenter()])],
        [Html.text(Text.fixed(m.count, 0.0))],
      ),
      Html.div([Attr.styles([Style.flexRow(), Style.gapPx(10.0), Style.justifyCenter()])], [
        Html.button([Attr.onClick(Dec)], [Html.text("-")]),
        Html.button([Attr.onClick(Inc)], [Html.text("+")]),
        Html.button([Attr.class("ghost"), Attr.onClick(Reset)], [Html.text("Reset")]),
      ]),
      Html.input([Attr.value(m.name), Attr.placeholder("your name"), Attr.onInput(SetName)]),
      Html.p([], [Html.text(greeting(m))]),
    ]),
  ])
