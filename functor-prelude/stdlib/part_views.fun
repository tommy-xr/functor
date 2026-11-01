// PartViews — a review sheet for printable parts: front, back, left, right,
// top, bottom and two isometric views, each an orthographic camera framed to
// the part and rendered into a labelled tile, four by two.
//
// Pieces are `{ solid, color }` records in part space (mm, Z up, the front
// facing -Y), drawn together:
//
//   let draw = (m, tts) => PartViews.sheet([{ solid: part, color: Color.rgb(0.8, 0.5, 0.2) }])

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// Where each camera sits (direction from the part) and its screen-up, in part
// space. Left/right are seen standing at that side; bottom has the front at
// the top of the tile, like a drawing's views folded out around the front.
let views = () =>
  let v = (x, y, z) => Vec3.make(x, y, z) in
  [
    { name: "FRONT", dir: v(0.0, -1.0, 0.0), up: v(0.0, 0.0, 1.0) },
    { name: "BACK", dir: v(0.0, 1.0, 0.0), up: v(0.0, 0.0, 1.0) },
    { name: "LEFT", dir: v(-1.0, 0.0, 0.0), up: v(0.0, 0.0, 1.0) },
    { name: "RIGHT", dir: v(1.0, 0.0, 0.0), up: v(0.0, 0.0, 1.0) },
    { name: "TOP", dir: v(0.0, 0.0, 1.0), up: v(0.0, 1.0, 0.0) },
    { name: "BOTTOM", dir: v(0.0, 0.0, -1.0), up: v(0.0, -1.0, 0.0) },
    { name: "ISO FRONT", dir: v(1.0, -1.4, 0.9), up: v(0.0, 0.0, 1.0) },
    { name: "ISO BACK", dir: v(-1.0, 1.4, 0.9), up: v(0.0, 0.0, 1.0) },
  ]

// Tiles: 1 × 1 world units, `gap` apart, in a grid the sheet camera shows
// whole on a 4:3 or wider window (narrower ones crop the outer columns).
let columns = 4.0
let gap = 0.04
let sheetHalfHeight = 1.6
let tilePixels = 720.0

// Part space (mm, Z up) to scene space (metres, Y up), like Solid.toScene.
let toScene = (v: Vec3.t) => Vec3.make(Vec3.x(v) / 1000.0, Vec3.z(v) / 1000.0, 0.0 - Vec3.y(v) / 1000.0)

let point = (v: Vec3.t): Input.point3 => { x: Vec3.x(v), y: Vec3.y(v), z: Vec3.z(v) }

// The pieces' combined bounds, from each piece's own (cached) measurement.
let bounds = (pieces): Solid.bounds =>
  let each = List.map((p) => Solid.bounds(p.solid), pieces) in
  let lo = (f) => List.fold((acc, b) => Math.min(acc, f(b.min)), 1000000000.0, each) in
  let hi = (f) => List.fold((acc, b) => Math.max(acc, f(b.max)), -1000000000.0, each) in
  { min: Vec3.make(lo(Vec3.x), lo(Vec3.y), lo(Vec3.z)), max: Vec3.make(hi(Vec3.x), hi(Vec3.y), hi(Vec3.z)) }

// One view of `scene`: an orthographic camera just wide enough for the
// part's box seen from `view`, and a key light from over its shoulder. It is
// all worked in part space (mm) — the scene conversion is a rotation and a
// scale, so extents carry over — and only the camera and lights converted.
let tileFrame = (scene, b, view) =>
  let center = Vec3.scale(0.5, Vec3.add(b.min, b.max)) in
  let size = b.max |> Vec3.sub(b.min) in
  let back = Vec3.normalize(view.dir) in
  let forward = Vec3.scale(-1.0, back) in
  // Screen right is forward × up; screen up, right × forward.
  let right = Vec3.normalize(forward |> Vec3.cross(view.up)) in
  let up = right |> Vec3.cross(forward) in
  // The box's half extent along an axis: half its size, projected.
  let reach = (axis) =>
    0.5 * (Math.abs(Vec3.x(axis)) * Vec3.x(size) + Math.abs(Vec3.y(axis)) * Vec3.y(size) + Math.abs(Vec3.z(axis)) * Vec3.z(size)) in
  let half = Math.max(0.5, Math.max(reach(right), reach(up))) * 1.25 in
  let radius = Math.max(0.5, Vec3.length(size) / 2.0) in
  // Lowered a little (the part rides up 4% of the tile) to clear the label.
  let eye = center |> Vec3.add(Vec3.scale(2.0 * radius, back)) |> Vec3.add(Vec3.scale(-0.08 * half, up)) in
  let key = forward |> Vec3.add(Vec3.scale(-0.6, up)) |> Vec3.add(Vec3.scale(-0.4, right)) in
  let fill = Vec3.scale(-1.0, key) |> Vec3.add(Vec3.scale(-1.5, up)) in
  let light = (v) => Vec3.normalize(toScene(v)) in
  Frame.createLit(
    Camera3D.fromPose({ position: point(toScene(eye)), forward: point(light(forward)), up: point(light(up)) }, Angle.degrees(45.0))
      |> Camera3D.clip(radius * 0.9 / 1000.0, radius * 3.1 / 1000.0)
      |> Camera3D.orthographic(half / 1000.0),
    scene,
    [
      Light.ambient(Color.rgb(0.22, 0.22, 0.25)),
      Light.directional(light(key), Color.rgb(1.0, 0.97, 0.92), 0.85),
      Light.directional(light(fill), Color.rgb(0.7, 0.75, 0.9), 0.3),
    ])
    |> Frame.withClearColor(Color.rgb(0.16, 0.17, 0.19))
    // The view's name in the tile's bottom-left corner.
    |> Frame.with2D(
      Camera2D.create(1.0, 1.0),
      Sprite.text(Color.rgb(0.85, 0.87, 0.9), 0.04, view.name)
        |> Sprite.nearest
        |> Sprite.move(Sprite.measure(0.04, view.name).width / 2.0 - 0.46, -0.46))

// The sheet: every view of `pieces` (a non-empty list of `{ solid, color }`
// in part space), framed to their combined bounds.
let sheet = (pieces) =>
  let scene = Scene.group(List.map((p) => Solid.toScene(p.solid) |> Scene.lit(p.color), pieces)) in
  let b = bounds(pieces) in
  let pitch = 1.0 + gap in
  let all = views() in
  let rows = Math.ceil(List.length(all) / columns) in
  let tiles = List.indexedMap((i, view) =>
    let col = Math.mod(i, columns) in
    let row = Math.floor(i / columns) in
    { view: view,
      at: Vec3.make((col - (columns - 1.0) / 2.0) * pitch, ((rows - 1.0) / 2.0 - row) * pitch, 0.0),
      target: RenderTarget.named(Text.concat("part-views-", Text.fixed(i, 0.0))) |> RenderTarget.sized(tilePixels, tilePixels) },
    all) in
  List.fold((frame, t) => frame |> Frame.withRenderTarget(t.target, tileFrame(scene, b, t.view)),
    Frame.create(
      Camera3D.lookAt(Vec3.make(0.0, 0.0, 5.0), Vec3.make(0.0, 0.0, 0.0)) |> Camera3D.orthographic(sheetHalfHeight),
      Scene.group(List.map((t) => Scene.quad() |> Scene.screen(t.target) |> Scene.translate(t.at), tiles)))
      |> Frame.withClearColor(Color.rgb(0.07, 0.07, 0.08)),
    tiles)
