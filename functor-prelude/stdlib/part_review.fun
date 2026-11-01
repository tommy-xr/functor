// PartReview — review notes pinned to parts on a PartViews sheet: click a
// part to drop a pin where the pointer meets its surface, type a note about
// it. The notes are plain model data (`notes`), so a tool reading the model
// (the debug server's `/state`, the MCP server) sees exactly where each is.
//
// Pieces are PartViews pieces with a `name`: `{ name, solid, color }`. An
// app keeps a `PartReview.t` in its model and wires five hooks:
//
//   type Msg = | Review(msg: PartReview.msg)
//   let init = { review: PartReview.init }
//   let sampledInput = (m, s: Input.snapshot) => { m with review: PartReview.input(m.review, pieces, s) }
//   let update = (m, msg: Msg) => match msg with | Review(r) => { m with review: PartReview.update(m.review, r) }
//   let ui = (m) => PartReview.ui(Review, m.review)
//   let draw = (m, tts) => PartReview.sheet(m.review, pieces)
//
// Clicks need a free pointer: `"cursor": "visible"` in `functor.json`.

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// One note: where it is pinned (part space, mm, rounded to 0.01 mm; the
// surface's outward normal), on which piece, clicked in which view.
type note = { id: float, part: string, view: string, point: Input.point3, normal: Input.point3, text: string }

// `selected` is the id of the note being typed into, 0 for none.
type t = { notes: List<note>, selected: float, next: float }

type msg =
  | Edit(text: string)
  | Select(id: float)
  | Delete(id: float)

let init: t = { notes: [], selected: 0.0, next: 1.0 }

let rounded = (step: float, v: Vec3.t) =>
  // `+ 0.0` turns -0 into 0.
  let r = (x) => Math.round(x / step) * step + 0.0 in
  PartViews.point(Vec3.make(r(Vec3.x(v)), r(Vec3.y(v)), r(Vec3.z(v))))

let selectedNote = (r: t) => List.find((n) => n.id == r.selected, r.notes)

// Pin a note where `mouse` meets `pieces` on their sheet, and select it to
// type into. A selected note still empty moves there instead, so a misclick
// leaves no blank pin. Off the parts, nothing changes.
let click = (r: t, pieces, mouse: Input.mouse): t =>
  match PartViews.pick(pieces, mouse) with
  | Option.None => r
  | Option.Some(h) =>
    let at = (id) =>
      { id: id,
        part: List.nth(h.piece, pieces) |> Option.map((p) => p.name) |> Option.defaultValue(""),
        view: h.view,
        point: rounded(0.01, h.point),
        normal: rounded(0.001, h.normal),
        text: "" } in
    match selectedNote(r) |> Option.filter((n) => n.text == "") with
    | Option.Some(blank) => { r with notes: List.map((n) => if n.id == blank.id then at(n.id) else n, r.notes) }
    | Option.None => { notes: List.append(r.notes, [at(r.next)]), selected: r.next, next: r.next + 1.0 }

// A left click this step, from `sampledInput`'s snapshot.
let input = (r: t, pieces, snapshot: Input.snapshot): t =>
  if snapshot.mouse.pressed.left then click(r, pieces, snapshot.mouse) else r

let update = (r: t, m: msg): t =>
  match m with
  | Edit(text) => { r with notes: List.map((n) => if n.id == r.selected then { n with text: text } else n, r.notes) }
  | Select(id) => { r with selected: id }
  | Delete(id) =>
    { r with notes: List.filter((n) => n.id != id, r.notes), selected: if r.selected == id then 0.0 else r.selected }

let at3 = (p: Input.point3) =>
  Text.join(", ", [Text.fixed(p.x, 1.0), Text.fixed(p.y, 1.0), Text.fixed(p.z, 1.0)])

// The notes panel (top right): the selected note's text field first, then
// every note — select it, or `x` to delete. `wrap` lifts a `msg` into the
// app's own message (a constructor, or `(m) => m` when they are the same).
let ui = (wrap, r: t) =>
  let editing = selectedNote(r) |> Option.map((n) => [
      Ui.text(Text.concat(Text.concat("Note ", Text.fixed(n.id, 0.0)), Text.concat(" on ", n.part))),
      Ui.textInput(n.text, (text) => wrap(Edit(text))),
    ]) |> Option.defaultValue([Ui.text("Click a part to pin a note")]) in
  let row = (n) => Ui.row([
      Ui.button(Text.join(" ", [Text.fixed(n.id, 0.0), n.part, Text.concat(Text.concat("(", at3(n.point)), ")"), n.text]), wrap(Select(n.id))),
      Ui.button("x", wrap(Delete(n.id))),
    ]) in
  Ui.column(List.append(editing, List.map(row, r.notes))) |> Ui.panel(Ui.topRight())

// A marker at each note, in scene space, sized to the pieces: the selected
// one yellow, the rest red.
let pins = (r: t, pieces) =>
  let b = PartViews.bounds(pieces) in
  let size = Vec3.length(b.max |> Vec3.sub(b.min)) in
  Scene.group(List.map((n) =>
      Scene.sphere()
        |> Scene.scale(size * 0.025 / 1000.0)
        |> Scene.translate(PartViews.toScene(Vec3.make(n.point.x, n.point.y, n.point.z)))
        |> Scene.emissive(if n.id == r.selected then Color.rgb(1.0, 0.85, 0.1) else Color.rgb(0.95, 0.15, 0.2)),
    r.notes))

// `PartViews.sheet` of `pieces` with the pins.
let sheet = (r: t, pieces) => PartViews.sheetWith(pins(r, pieces), pieces)
