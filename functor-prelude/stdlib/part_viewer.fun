// PartViewer — the default scene for a printable part: lit, on a bed, with an
// OrbitCamera; `V` toggles a PartViews sheet of eight orthographic views.
// A click pins a PartReview note on the sheet, or on the orbit view with the
// Note tool (with `"cursor": "visible"` in `functor.json`; a captured pointer
// has no position to pick). A file that
// defines `part` and none of the names in `<stdlib>/PartViewerDefault.fun`
// (`update` and `ui` among them) runs with those ten lines; paste them into
// the file to change any of them.
//
// On the orbit view the toolbar picks what a left press does: Orbit drags
// the view, Note pins a note where the pointer meets a part.
//
// An app showing several parts keeps a `PartViewer.t` in its model, forwards
// the mouse, key, tick and `update` hooks, shows `uiWith(wrap, m)`, and passes
// its non-empty PartViews pieces (`{ name, solid, color }`) to `drawPieces` and
// `sampledInputPieces`.

type tool = | Orbit | Note

type msg =
  | Review(msg: PartReview.msg)
  | UseTool(tool: tool)
  | ShowSheet(on: bool)

// `vHeld` latches V so a held key (auto-repeat) toggles once.
type t = { orbit: OrbitCamera.t, sheet: bool, vHeld: bool, review: PartReview.t, tool: tool }

let init: t = { orbit: OrbitCamera.init, sheet: false, vHeld: false, review: PartReview.init, tool: Orbit }

// The turntable rests while the Note tool is out, so parts hold still to click.
let tick = (m, dt: float, tts: float) => if m.tool == Note then m else { m with orbit: OrbitCamera.tick(m.orbit, dt) }

let mouseMove = (m, x: float, y: float) => { m with orbit: OrbitCamera.move(m.orbit, x, y) }

let mouseButton = (m, button: Mouse.t, isDown: bool) =>
  if button == Mouse.Left && m.tool == Orbit then { m with orbit: OrbitCamera.press(m.orbit, isDown) } else m

let mouseWheel = (m, delta: float) => { m with orbit: OrbitCamera.scroll(m.orbit, delta) }

let input = (m, key: Key.t, isDown: bool) =>
  if key == Key.V then { m with sheet: if isDown && not m.vHeld then not m.sheet else m.sheet, vHeld: isDown }
  else m

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

let piecesOf = (part: Solid.t) => [{ name: "part", solid: part, color: Color.rgb(0.25, 0.5, 0.85) }]

// The orbit view's framing of `pieces`: centre and size in scene space, and
// the floor (their lowest point).
let framing = (pieces) =>
  let b = PartViews.bounds(pieces) in
  let lo = PartViews.toScene(b.min) in
  let hi = PartViews.toScene(b.max) in
  { center: Vec3.scale(0.5, Vec3.add(lo, hi)), size: Vec3.distance(lo, hi), floor: Vec3.y(lo) }

let camera = (pieces, m) =>
  let f = framing(pieces) in
  OrbitCamera.camera(m.orbit, f.center, f.size)

// A click pins a note on the sheet, or on the orbit view with the Note tool.
let sampledInputPieces = (pieces, m, s: Input.snapshot) =>
  if m.sheet then { m with review: PartReview.input(m.review, pieces, s) }
  else if m.tool == Note && s.mouse.pressed.left then
    { m with review: PartReview.pinAt(m.review, pieces, PartViews.pickThrough(camera(pieces, m), "orbit", pieces, s.mouse)) }
  else m

let sampledInput = (part: Solid.t, m, s: Input.snapshot) => sampledInputPieces(piecesOf(part), m, s)

// Switching tools ends a drag, so the view doesn't keep turning.
let update = (m, msg: msg) =>
  match msg with
  | Review(r) => { m with review: PartReview.update(m.review, r) }
  | UseTool(tool) => { m with tool: tool, orbit: OrbitCamera.press(m.orbit, false) }
  | ShowSheet(on) => { m with sheet: on }

// The toolbar, top right, and the notes panel wherever notes can be pinned;
// `wrap` lifts a `msg` into the app's message.
let uiWith = (wrap, m) =>
  let pick = (label, on, msg) => Ui.button(if on then Text.concat("> ", label) else label, wrap(msg)) in
  let toolbar =
    if m.sheet then [pick("3D view", false, ShowSheet(false))]
    else [pick("Orbit", m.tool == Orbit, UseTool(Orbit)), pick("Note", m.tool == Note, UseTool(Note)), pick("8 views", false, ShowSheet(true))] in
  let notes = if m.sheet || m.tool == Note then [PartReview.ui((r) => wrap(Review(r)), m.review)] else [] in
  Ui.column([Ui.row(toolbar) |> Ui.panel(Ui.topRight())] |> List.append(notes))

let ui = (m) => uiWith((msg) => msg, m)

let drawPieces = (pieces, m, tts: float) =>
  if m.sheet then PartReview.sheet(m.review, pieces) else
  let f = framing(pieces) in
  Frame.createLit(
    OrbitCamera.camera(m.orbit, f.center, f.size),
    Scene.group([
      Scene.plane()
        |> Scene.scale(f.size * 4.0)
        |> Scene.translate(Vec3.make(Vec3.x(f.center), f.floor, Vec3.z(f.center)))
        |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      PartViews.scene(pieces),
      PartReview.pins(m.review, pieces),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9),
      // A low fill from the other side, so walls aren't black.
      Light.directional(Vec3.make(-0.6, -0.2, -0.5), Color.rgb(0.75, 0.8, 1.0), 0.45),
    ])

let draw = (part: Solid.t, m, tts: float) => drawPieces(piecesOf(part), m, tts)
