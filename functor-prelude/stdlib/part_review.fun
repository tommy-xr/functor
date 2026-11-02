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
//   let webview = (m) => Html.div([Attr.class("pv")], [Html.style(PartViewer.css), PartReview.webview(Review, m.review)])
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

let rounded = (perUnit: float, v: Vec3.t) =>
  // Dividing keeps decimals exact (19.4, not 19.400000000000002); `+ 0.0`
  // turns -0 into 0.
  let r = (x) => Math.round(x * perUnit) / perUnit + 0.0 in
  let p: Input.point3 = { x: r(Vec3.x(v)), y: r(Vec3.y(v)), z: r(Vec3.z(v)) } in
  p

let selectedNote = (r: t) => List.find((n) => n.id == r.selected, r.notes)

// Pin a note where `mouse` meets `pieces` on their sheet, and select it to
// type into.
let click = (r: t, pieces, mouse: Input.mouse): t => pinAt(r, pieces, PartViews.pick(pieces, mouse))

// Pin a note at `hit` (a `PartViews` pick), and select it to type into. A
// selected note still empty moves there instead, so a misclick doesn't leave
// a blank pin behind. Off the parts (`Option.None`), nothing changes.
let pinAt = (r: t, pieces, hit: Option.t<PartViews.hit>): t =>
  match hit with
  | Option.None => r
  | Option.Some(h) =>
    let at = (id) =>
      { id: id,
        part: List.nth(h.piece, pieces) |> Option.map((p) => p.name) |> Option.defaultValue(""),
        view: h.view,
        point: rounded(100.0, h.point),
        normal: rounded(1000.0, h.normal),
        text: "" } in
    match selectedNote(r) |> Option.filter((n) => n.text == "") with
    | Option.Some(blank) => { r with notes: List.map((n) => if n.id == blank.id then at(n.id) else n, r.notes) }
    | Option.None => { notes: r.notes |> List.append([at(r.next)]), selected: r.next, next: r.next + 1.0 }

// A left click this step, from `sampledInput`'s snapshot.
let input = (r: t, pieces, snapshot: Input.snapshot): t =>
  if snapshot.mouse.pressed.left then click(r, pieces, snapshot.mouse) else r

let update = (r: t, m: msg): t =>
  match m with
  | Edit(text) => { r with notes: List.map((n) => if n.id == r.selected then { n with text: text } else n, r.notes) }
  | Select(id) => { r with selected: id }
  | Delete(id) =>
    { r with notes: List.filter((n) => n.id != id, r.notes), selected: if r.selected == id then 0.0 else r.selected }

// How many notes the panel lists, so it stays clear of the view.
let shown = 8.0

// A note's text cut to fit a list row.
let short = (text: string) =>
  if Text.length(text) <= 32.0 then text else Text.concat(Text.join("", List.take(31.0, Text.chars(text))), "…")

// Where a note sits, to the whole mm (the list row's hint; `/state` has it exact).
let at3 = (p: Input.point3) =>
  Text.join(", ", [Text.fixed(p.x, 0.0), Text.fixed(p.y, 0.0), Text.fixed(p.z, 0.0)])

// The notes card (HTML, for a `webview`), bottom left: the selected note's
// text field, then the latest `shown` notes — click one to select it, `×` to
// delete it. `wrap` lifts a `msg` into the app's own message (a constructor,
// or `(m) => m` when they are the same). Styled by `PartViewer.css`.
let webview = (wrap, r: t) =>
  let editing = selectedNote(r) |> Option.map((n) => [
      Html.div([Attr.class("pv-label")], [Html.text(Text.concat(Text.concat("Note ", Text.fixed(n.id, 0.0)), Text.concat(" · ", n.part)))]),
      Html.input([Attr.value(n.text), Attr.placeholder("What should change?"), Attr.onInput((text) => wrap(Edit(text)))]),
    ]) |> Option.defaultValue([Html.div([Attr.class("pv-hint")], [Html.text("Click a part to pin a note")])]) in
  let row = (n) => Html.div([Attr.class("pv-row")], [
      Html.button(
        [Attr.class(if n.id == r.selected then "pv-note pv-selected" else "pv-note"), Attr.onClick(wrap(Select(n.id)))],
        [Html.span([Attr.class("pv-id")], [Html.text(Text.fixed(n.id, 0.0))]),
         Html.span([Attr.class(if n.text == "" then "pv-hint" else "")], [Html.text(if n.text == "" then "(no text)" else short(n.text))]),
         Html.span([Attr.class("pv-at")], [Html.text(Text.concat(Text.concat(n.part, " "), at3(n.point)))])]),
      Html.button([Attr.class("pv-x"), Attr.onClick(wrap(Delete(n.id)))], [Html.text("×")]),
    ]) in
  let earlier = List.length(r.notes) - shown in
  let more = if earlier > 0 then [Html.div([Attr.class("pv-hint")], [Html.text(Text.concat(Text.fixed(earlier, 0.0), " earlier notes"))])] else [] in
  // The card's own click (on its background) keeps the selection; it stops
  // the press pinning a note on the part behind the card.
  Html.div([Attr.class("pv-card pv-notes"), Attr.onClick(wrap(Select(r.selected)))],
    [Html.div([Attr.class("pv-title")], [Html.text("NOTES")])]
      |> List.append(editing) |> List.append(more) |> List.append(List.map(row, List.drop(earlier, r.notes))))

// A marker at each note on one of `pieces` (by name), in scene space, sized
// to the pieces: the selected one yellow, the rest red.
let pins = (r: t, pieces) =>
  let b = PartViews.bounds(pieces) in
  let size = Vec3.distance(b.min, b.max) in
  Scene.group(List.map((n) =>
      Scene.sphere()
        |> Scene.scale(size * 0.02 / 1000.0)
        |> Scene.translate(PartViews.toScene(Vec3.make(n.point.x, n.point.y, n.point.z)))
        |> Scene.emissive(if n.id == r.selected then Color.rgb(1.0, 0.85, 0.1) else Color.rgb(0.95, 0.15, 0.2)),
    List.filter((n) => List.any((p) => p.name == n.part, pieces), r.notes)))

// `PartViews.sheet` of `pieces` with the pins.
let sheet = (r: t, pieces) => PartViews.sheetWith(pins(r, pieces), pieces)
