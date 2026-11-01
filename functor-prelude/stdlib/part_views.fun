// PartViews — a review sheet for printable parts: front, back, left, right,
// top, bottom and two three-quarter views, each an orthographic camera framed
// to the part and rendered into a labelled tile, laid out like a drawing: the
// top above the front, the left and right beside it, the bottom below.
//
// Pieces are `{ solid, color }` records in part space (mm, Z up, the front
// facing -Y), drawn together:
//
//   let draw = (m, tts) => PartViews.sheet([{ solid: part, color: Color.rgb(0.8, 0.5, 0.2) }])

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// Where each camera sits (direction from the part), its screen-up, in part
// space, and its tile's column and row (from the top left) in a 3 × 3 grid.
// Left/right are seen standing at that side; each view's edge next to the
// front's is the edge it shares, as when a drawing's views fold out.
let views = () =>
  let v = (x, y, z) => Vec3.make(x, y, z) in
  [
    { name: "ISO FRONT", dir: v(1.0, -1.4, 0.9), up: v(0.0, 0.0, 1.0), col: 0.0, row: 0.0 },
    { name: "TOP", dir: v(0.0, 0.0, 1.0), up: v(0.0, 1.0, 0.0), col: 1.0, row: 0.0 },
    { name: "ISO BACK", dir: v(-1.0, 1.4, 0.9), up: v(0.0, 0.0, 1.0), col: 2.0, row: 0.0 },
    { name: "LEFT", dir: v(-1.0, 0.0, 0.0), up: v(0.0, 0.0, 1.0), col: 0.0, row: 1.0 },
    { name: "FRONT", dir: v(0.0, -1.0, 0.0), up: v(0.0, 0.0, 1.0), col: 1.0, row: 1.0 },
    { name: "RIGHT", dir: v(1.0, 0.0, 0.0), up: v(0.0, 0.0, 1.0), col: 2.0, row: 1.0 },
    { name: "BOTTOM", dir: v(0.0, 0.0, -1.0), up: v(0.0, -1.0, 0.0), col: 1.0, row: 2.0 },
    { name: "BACK", dir: v(0.0, 1.0, 0.0), up: v(0.0, 0.0, 1.0), col: 2.0, row: 2.0 },
  ]

// Tiles: 1 × 1 world units, `gap` apart; the sheet camera shows the whole
// grid on any window at least as wide as it is tall.
let gap = 0.04
let sheetHalfHeight = 1.6
let tilePixels = 720.0

// Part space (mm, Z up) to scene space (metres, Y up), like Solid.toScene.
let toScene = (v: Vec3.t) => Vec3.make(Vec3.x(v) / 1000.0, Vec3.z(v) / 1000.0, 0.0 - Vec3.y(v) / 1000.0)

// A part-space direction in scene space.
let dir = (v: Vec3.t) => Vec3.normalize(toScene(v))

let point = (v: Vec3.t): Input.point3 => { x: Vec3.x(v), y: Vec3.y(v), z: Vec3.z(v) }

// The pieces' combined bounds, from each piece's own (cached) measurement.
let bounds = (pieces): Solid.bounds =>
  let each = List.map((p) => Solid.bounds(p.solid), pieces) in
  let lo = (f) => List.fold((acc, b) => Math.min(acc, f(b.min)), 1000000000.0, each) in
  let hi = (f) => List.fold((acc, b) => Math.max(acc, f(b.max)), -1000000000.0, each) in
  { min: Vec3.make(lo(Vec3.x), lo(Vec3.y), lo(Vec3.z)), max: Vec3.make(hi(Vec3.x), hi(Vec3.y), hi(Vec3.z)) }

// One view's camera: orthographic, just wide enough for the part's box seen
// from `view`, with its screen axes. It is all worked in part space (mm) —
// the scene conversion is a rotation and a scale, so extents carry over — and
// only the camera converted.
let framing = (b, view) =>
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
  // Far back, so the eye-relative specular lighting falls almost parallel.
  let distance = 20.0 * radius in
  let eye = center |> Vec3.add(Vec3.scale(distance, back)) |> Vec3.add(Vec3.scale(-0.08 * half, up)) in
  { forward: forward, up: up, right: right,
    camera:
      Camera3D.fromPose({ position: point(toScene(eye)), forward: point(dir(forward)), up: point(dir(up)) }, Angle.degrees(45.0))
        |> Camera3D.clip((distance - 1.1 * radius) / 1000.0, (distance + 1.1 * radius) / 1000.0)
        |> Camera3D.orthographic(half / 1000.0) }

// One view of `scene`, lit by a key light from over the camera's shoulder.
let tileFrame = (scene, b, view) =>
  let f = framing(b, view) in
  let key = f.forward |> Vec3.add(Vec3.scale(-0.6, f.up)) |> Vec3.add(Vec3.scale(-0.4, f.right)) in
  let fill = Vec3.scale(-1.0, key) |> Vec3.add(Vec3.scale(-1.5, f.up)) in
  Frame.createLit(
    f.camera,
    scene,
    [
      Light.ambient(Color.rgb(0.22, 0.22, 0.25)),
      Light.directional(dir(key), Color.rgb(1.0, 0.97, 0.92), 0.85),
      Light.directional(dir(fill), Color.rgb(0.7, 0.75, 0.9), 0.3),
    ])
    |> Frame.withClearColor(Color.rgb(0.16, 0.17, 0.19))
    // The view's name in the tile's bottom-left corner.
    |> Frame.with2D(
      Camera2D.create(1.0, 1.0),
      Sprite.text(Color.rgb(0.85, 0.87, 0.9), 0.04, view.name)
        |> Sprite.nearest
        |> Sprite.move(Sprite.measure(0.04, view.name).width / 2.0 - 0.46, -0.46))

// A tile's centre on the sheet, in sheet units.
let tileX = (view) => (view.col - 1.0) * (1.0 + gap)
let tileY = (view) => (1.0 - view.row) * (1.0 + gap)

let sheetCamera = () =>
  Camera3D.lookAt(Vec3.make(0.0, 0.0, 5.0), Vec3.make(0.0, 0.0, 0.0)) |> Camera3D.orthographic(sheetHalfHeight)

// The sheet: every view of `pieces` (a non-empty list of `{ solid, color }`
// in part space), framed to their combined bounds. Its render targets are
// named `part-views-0` … `part-views-7`, so draw one sheet per frame.
let sheet = (pieces) => sheetWith(Scene.group([]), pieces)

// A sheet with `overlay` — scene space, like `Solid.toScene`'s — drawn with
// the pieces in every view (e.g. `PartReview`'s pins), but not framed.
let sheetWith = (overlay, pieces) =>
  let scene = Scene.group(List.map((p) => Solid.toScene(p.solid) |> Scene.lit(p.color), pieces) |> List.append([overlay])) in
  let b = bounds(pieces) in
  let tiles = List.indexedMap((i, view) =>
    { view: view,
      target: RenderTarget.named(Text.concat("part-views-", Text.fixed(i, 0.0))) |> RenderTarget.sized(tilePixels, tilePixels) },
    views()) in
  List.fold((frame, t) => frame |> Frame.withRenderTarget(t.target, tileFrame(scene, b, t.view)),
    Frame.create(
      sheetCamera(),
      Scene.group(List.map((t) => Scene.quad() |> Scene.screen(t.target) |> Scene.translate(Vec3.make(tileX(t.view), tileY(t.view), 0.0)), tiles)))
      |> Frame.withClearColor(Color.rgb(0.07, 0.07, 0.08)),
    tiles)

// Scene space (metres, Y up) back to part space (mm, Z up).
let fromScene = (v: Vec3.t) => Vec3.make(Vec3.x(v) * 1000.0, 0.0 - Vec3.z(v) * 1000.0, Vec3.y(v) * 1000.0)

// What the pointer is over on a sheet of `pieces`: the piece's index in the
// list, the view's name, and the hit in part space (mm, distance from the
// view's eye plane).
type hit = { piece: float, view: string, point: Vec3.t, normal: Vec3.t, distance: float }

// The hit under `mouse` (the `sampledInput` snapshot's) on the sheet that
// `sheet(pieces)` draws, nearest first, or `Option.None` off the parts. The
// pointer maps to a tile, then through that view's camera onto each piece.
let pick = (pieces, mouse: Input.mouse): Option.t<hit> =>
  let b = bounds(pieces) in
  Camera3D.toWorldRay(mouse, sheetCamera()) |> Option.bind((onSheet) =>
    let sx = Vec3.x(onSheet.origin) in
    let sy = Vec3.y(onSheet.origin) in
    views()
      |> List.find((v) => Math.abs(sx - tileX(v)) < 0.5 && Math.abs(sy - tileY(v)) < 0.5)
      |> Option.bind((view) =>
        // The tile's pixel, top-left origin, like a pointer on its own surface.
        let inTile = { mouse with
          x: (sx - tileX(view) + 0.5) * tilePixels,
          y: (tileY(view) + 0.5 - sy) * tilePixels,
          surfaceWidth: tilePixels,
          surfaceHeight: tilePixels } in
        Camera3D.toWorldRay(inTile, framing(b, view).camera) |> Option.bind((ray) =>
          let inPart: Camera3D.ray = { origin: fromScene(ray.origin), direction: fromScene(ray.direction) } in
          List.indexedMap((i, p) => Solid.raycast(inPart, p.solid)
              |> Option.map((h) => { piece: i, view: view.name, point: h.point, normal: h.normal, distance: h.distance }), pieces)
            |> List.concatMap(Option.toList)
            |> List.sortBy((h) => h.distance)
            |> List.head)))
