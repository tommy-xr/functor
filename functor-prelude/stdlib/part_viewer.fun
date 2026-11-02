// PartViewer — the default scene for a printable part: lit, on a bed, with an
// OrbitCamera; `V` toggles a PartViews sheet of eight orthographic views,
// where a click pins a PartReview note (with `"cursor": "visible"` in
// `functor.json`; a captured pointer has no position to pick). A file that
// defines `part` and none of the names in `<stdlib>/PartViewerDefault.fun`
// (`update` and `ui` among them) runs with those ten lines; paste them into
// the file to change any of them.
//
// An app showing several parts keeps a `PartViewer.t` in its model, forwards
// the mouse, key and tick hooks, and passes its PartViews pieces
// (`{ name, solid, color }`) to `drawPieces` and `sampledInputPieces`.

// `vHeld` latches V so a held key (auto-repeat) toggles once.
type t = { orbit: OrbitCamera.t, sheet: bool, vHeld: bool, review: PartReview.t }

let init: t = { orbit: OrbitCamera.init, sheet: false, vHeld: false, review: PartReview.init }

let tick = (m, dt: float, tts: float) => { m with orbit: OrbitCamera.tick(m.orbit, dt) }

let mouseMove = (m, x: float, y: float) => { m with orbit: OrbitCamera.move(m.orbit, x, y) }

let mouseButton = (m, button: Mouse.t, isDown: bool) =>
  if button == Mouse.Left then { m with orbit: OrbitCamera.press(m.orbit, isDown) } else m

let mouseWheel = (m, delta: float) => { m with orbit: OrbitCamera.scroll(m.orbit, delta) }

let input = (m, key: Key.t, isDown: bool) =>
  if key == Key.V then { m with sheet: if isDown && not m.vHeld then not m.sheet else m.sheet, vHeld: isDown }
  else m

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

let pieces = (part: Solid.t) => [{ name: "part", solid: part, color: Color.rgb(0.25, 0.5, 0.85) }]

// Clicks pin notes on the sheet only: on the orbit view a press drags.
let sampledInputPieces = (pieces, m, s: Input.snapshot) =>
  if m.sheet then { m with review: PartReview.input(m.review, pieces, s) } else m

let sampledInput = (part: Solid.t, m, s: Input.snapshot) => sampledInputPieces(pieces(part), m, s)

let update = (m, msg: PartReview.msg) => { m with review: PartReview.update(m.review, msg) }

// The notes panel on the sheet; `wrap` lifts a `PartReview.msg` into the
// app's message.
let uiWith = (wrap, m) => if m.sheet then PartReview.ui(wrap, m.review) else Ui.column([])

let ui = (m) => uiWith((msg) => msg, m)

let drawPieces = (pieces, m, tts: float) =>
  if m.sheet then PartReview.sheet(m.review, pieces) else
  let b = PartViews.bounds(pieces) in
  let lo = PartViews.toScene(b.min) in
  let hi = PartViews.toScene(b.max) in
  let center = Vec3.scale(0.5, Vec3.add(lo, hi)) in
  let size = Vec3.distance(lo, hi) in
  Frame.createLit(
    OrbitCamera.camera(m.orbit, center, size),
    Scene.group([
      Scene.plane()
        |> Scene.scale(size * 4.0)
        |> Scene.translate(Vec3.make(Vec3.x(center), Vec3.y(lo), Vec3.z(center)))
        |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      Scene.group(List.map((p) => Solid.toScene(p.solid) |> Scene.lit(p.color), pieces)),
      PartReview.pins(m.review, pieces),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9),
      // A low fill from the other side, so walls aren't black.
      Light.directional(Vec3.make(-0.6, -0.2, -0.5), Color.rgb(0.75, 0.8, 1.0), 0.45),
    ])

let draw = (part: Solid.t, m, tts: float) => drawPieces(pieces(part), m, tts)
