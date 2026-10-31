// Everything derived from layout.fun: the interior surfaces (one mesh, baked
// together, then drawn per cell), the culling cells and portals, the wall and
// floor colliders, and the exterior shell. Editing this file also makes the
// committed bake stale (its hash covers both files).

open Layout

let v = Vec3.make

// ---- Materials -------------------------------------------------------------

// sRGB bytes, as the 1-texel or procedural texture and the bake's albedo.
type Paint = { r: float, g: float, b: float }
let paint = (r: float, g: float, b: float): Paint => { r: r, g: g, b: b }

// `tile`: metres per texture repeat, world-anchored; 0 stretches the texture
// once across each face.
type Material = { paint: Paint, tile: float, unlit: bool, pixels: Texture.pixels }

// Deterministic per-texel noise in [0, 1).
let noise = (i: float): float =>
  let s = Math.sin(i * 12.9898 + 78.233) * 43758.5453 in
  s - Math.floor(s)
let byte = (x: float): float => Math.clamp(0.0, 255.0, Math.round(x))
let image = (w: float, h: float, texel: (float, float) => Paint): Texture.pixels =>
  { width: w, height: h, format: Texture.Rgb8, colorSpace: Texture.Srgb,
    pixels: Bytes.fromU8(List.range(w * h) |> List.concatMap((i) =>
      let p = texel(Math.mod(i, w), Math.floor(i / w)) in [byte(p.r), byte(p.g), byte(p.b)])) }
let shade = (k: float, p: Paint): Paint => paint(p.r * k, p.g * k, p.b * k)
let solid = (p: Paint): Texture.pixels => image(1.0, 1.0, (x, y) => p)
// Speckled: carpet pile and terrazzo.
let speckled = (amount: float, p: Paint): Texture.pixels =>
  image(32.0, 32.0, (x, y) => shade(1.0 + amount * (noise(x + 32.0 * y) - 0.5), p))
// A square tile with a darker `line`-texel border: ceiling and floor tiles.
let tiled = (n: float, line: float, edge: Paint, p: Paint): Texture.pixels =>
  image(n, n, (x, y) =>
    if x < line || y < line then edge else shade(1.0 + 0.04 * (noise(x + n * y) - 0.5), p))

let material = (p: Paint, tile: float, unlit: bool, pixels: Texture.pixels): Material =>
  { paint: p, tile: tile, unlit: unlit, pixels: pixels }

let carpetPaint = paint(92.0, 98.0, 74.0)
let terrazzoPaint = paint(158.0, 160.0, 148.0)
let wallPaint = paint(178.0, 184.0, 160.0)
let ceilingPaint = paint(200.0, 202.0, 186.0)
let panelPaint = paint(240.0, 246.0, 216.0)
let framePaint = paint(58.0, 72.0, 62.0)

// Indices into `materials`, in draw order.
let carpet = 0.0
let terrazzo = 1.0
let wall = 2.0
let ceilingTile = 3.0
let panelOn = 4.0
let panelOff = 5.0
let frame = 6.0
let exitGreen = 7.0
let sodium = 8.0

let materials = [
  material(carpetPaint, 1.0, false, speckled(0.35, carpetPaint)),
  material(terrazzoPaint, 0.6, false, tiled(32.0, 1.0, shade(0.8, terrazzoPaint), terrazzoPaint)),
  material(wallPaint, 1.0, false, speckled(0.06, wallPaint)),
  material(ceilingPaint, 0.6, false, tiled(32.0, 1.0, shade(0.62, ceilingPaint), ceilingPaint)),
  // A lit lens in a dark frame, stretched over each panel.
  material(panelPaint, 0.0, true, image(8.0, 16.0, (x, y) =>
    if x < 1.0 || x > 6.0 || y < 1.0 || y > 14.0 then framePaint
    else if x == 3.0 || x == 4.0 then shade(0.93, panelPaint) else panelPaint)),
  material(paint(150.0, 152.0, 140.0), 0.0, false, solid(paint(150.0, 152.0, 140.0))),
  material(framePaint, 1.0, false, solid(framePaint)),
  material(paint(70.0, 205.0, 125.0), 0.0, true, image(16.0, 8.0, (x, y) =>
    if x < 1.0 || x > 14.0 || y < 1.0 || y > 6.0 then paint(30.0, 60.0, 40.0)
    else paint(70.0, 205.0, 125.0))),
  material(paint(214.0, 158.0, 78.0), 0.0, true, solid(paint(214.0, 158.0, 78.0))),
]

let textures = materials |> List.map((m) => Texture.fromPixels(m.pixels))
let albedo = materials |> List.map((m) => Color.rgb(m.paint.r / 255.0, m.paint.g / 255.0, m.paint.b / 255.0))
let materialAt = (i: float): Material =>
  match List.nth(i, materials) with
  | Option.Some(m) => m
  | Option.None => Binary.invalid($"no material {i}")
let textureAt = (i: float) =>
  match List.nth(i, textures) with
  | Option.Some(t) => t
  | Option.None => Binary.invalid($"no material {i}")
let albedoAt = (i: float): Color.t =>
  match List.nth(i, albedo) with
  | Option.Some(c) => c
  | Option.None => Binary.invalid($"no material {i}")

// ---- Faces -----------------------------------------------------------------

// A quad from corner `o` along `a`, then `b`; it faces `a × b`.
type Quad = { o: Vec3.t, a: Vec3.t, b: Vec3.t }
type Face = { mat: float, q: Quad }

let face = (mat: float, o: Vec3.t, a: Vec3.t, b: Vec3.t): Face => { mat: mat, q: { o: o, a: a, b: b } }
// Up-facing floor and down-facing ceiling over [x0, x1] × [z0, z1] at `y`.
let floorFace = (mat: float, x0: float, z0: float, x1: float, z1: float, y: float): Face =>
  face(mat, v(x0, y, z1), v(x1 - x0, 0.0, 0.0), v(0.0, 0.0, z0 - z1))
let ceilingFace = (mat: float, x0: float, z0: float, x1: float, z1: float, y: float): Face =>
  face(mat, v(x0, y, z0), v(x1 - x0, 0.0, 0.0), v(0.0, 0.0, z1 - z0))
// A wall in the plane z = `z` over [x0, x1] × [y0, y1], facing +Z or -Z.
let wallZ = (mat: float, x0: float, x1: float, y0: float, y1: float, z: float, plus: bool): Face =>
  if plus then face(mat, v(x0, y0, z), v(x1 - x0, 0.0, 0.0), v(0.0, y1 - y0, 0.0))
  else face(mat, v(x1, y0, z), v(x0 - x1, 0.0, 0.0), v(0.0, y1 - y0, 0.0))
// A wall in the plane x = `x` over [z0, z1] × [y0, y1], facing +X or -X.
let wallX = (mat: float, z0: float, z1: float, y0: float, y1: float, x: float, plus: bool): Face =>
  if plus then face(mat, v(x, y0, z1), v(0.0, 0.0, z0 - z1), v(0.0, y1 - y0, 0.0))
  else face(mat, v(x, y0, z0), v(0.0, 0.0, z1 - z0), v(0.0, y1 - y0, 0.0))

// ---- Rooms, walls and doors --------------------------------------------------

let eps = 0.001
let near = (a: float, b: float): bool => Math.abs(a - b) < eps

let roomAt = (i: float): Room =>
  List.nth(i, rooms) |> Option.defaultValue(room("missing", Outside, rect(0.0, 0.0, 0.0, 0.0), []))
let isInside = (r: Room): bool => r.kind != Outside

// The side of `r` that `o` sits against, if they share a wall line.
let sideBetween = (r: Rect, o: Rect): Option.t<Side> =>
  if near(r.x1, o.x0) then Option.Some(East)
  else if near(r.x0, o.x1) then Option.Some(West)
  else if near(r.z1, o.z0) then Option.Some(North)
  else if near(r.z0, o.z1) then Option.Some(South)
  else Option.None

// The wall line a side of `r` runs along.
let lineOf = (side: Side, r: Rect): float =>
  match side with
  | North => r.z1
  | South => r.z0
  | East => r.x1
  | West => r.x0

let onFootprint = (side: Side, r: Rect): bool =>
  let l = lineOf(side, r) in
  match side with
  | North => near(l, footprint.z1)
  | South => near(l, footprint.z0)
  | East => near(l, footprint.x1)
  | West => near(l, footprint.x0)

// Half the thickness of the wall along a side: facade or partition.
let halfWall = (side: Side, r: Rect): float =>
  if onFootprint(side, r) then facade / 2.0 else partition / 2.0

// A room's inside: its rect less half of each wall.
let inner = (r: Rect): Rect =>
  rect(r.x0 + halfWall(West, r), r.z0 + halfWall(South, r),
       r.x1 - halfWall(East, r), r.z1 - halfWall(North, r))

// An opening's span along its wall, and its top.
type Hole = { lo: float, hi: float, top: float }
let holeOf = (d: Door): Hole => { lo: d.at - d.width / 2.0, hi: d.at + d.width / 2.0, top: d.height }

// The doors in a room's side, as holes in increasing order.
let holesIn = (i: float, side: Side): List<Hole> =>
  doors
    |> List.filter((d) =>
      let other = if d.a == i then Option.Some(d.b) else if d.b == i then Option.Some(d.a) else Option.None in
      match other with
      | Option.Some(o) => sideBetween(roomAt(i).rect, roomAt(o).rect) == Option.Some(side)
      | Option.None => false)
    |> List.map(holeOf)
    |> List.sortBy((h) => h.lo)

// The solid parts of a wall run [s0, s1] × [0, top] around its holes: full
// height between holes, a header above each.
type Piece = { u0: float, u1: float, y0: float, y1: float }
let pieces = (s0: float, s1: float, top: float, holes: List<Hole>): List<Piece> =>
  let strip = (a: float, b: float): List<Piece> =>
    if b - a > eps then [{ u0: a, u1: b, y0: 0.0, y1: top }] else [] in
  let walked = holes |> List.fold((acc, h) =>
    let (cursor, out) = acc in
    let header = if top - h.top > eps then [{ u0: h.lo, u1: h.hi, y0: h.top, y1: top }] else [] in
    (h.hi, List.flatten([out, strip(Math.max(cursor, s0), Math.min(h.lo, s1)), header])),
    (s0, [])) in
  let (cursor, out) = walked in
  out |> List.append(strip(cursor, s1))

// A room's four inner walls, facing in.
let wallFaces = (i: float): List<Face> =>
  let r = inner(roomAt(i).rect) in
  let side = (s: Side, s0: float, s1: float, make: (Piece) => Face) =>
    pieces(s0, s1, ceiling, holesIn(i, s)) |> List.map(make) in
  List.flatten([
    side(South, r.x0, r.x1, (p) => wallZ(wall, p.u0, p.u1, p.y0, p.y1, r.z0, true)),
    side(North, r.x0, r.x1, (p) => wallZ(wall, p.u0, p.u1, p.y0, p.y1, r.z1, false)),
    side(West, r.z0, r.z1, (p) => wallX(wall, p.u0, p.u1, p.y0, p.y1, r.x0, true)),
    side(East, r.z0, r.z1, (p) => wallX(wall, p.u0, p.u1, p.y0, p.y1, r.x1, false)),
  ])

// A door's reveal, baked with room `b`: the two jambs, the head (or, for a
// floor-to-ceiling opening, a ceiling strip) and the threshold.
let jambFaces = (d: Door): List<Face> =>
  let ra = roomAt(d.a).rect in
  match sideBetween(ra, roomAt(d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let l = lineOf(side, ra) in
    let t = halfWall(side, ra) in
    let h = holeOf(d) in
    let top = if h.top < ceiling - eps then frame else ceilingTile in
    let sill = if roomAt(d.b).kind == Lobby then terrazzo else carpet in
    match side == North || side == South with
    | true => [
        wallX(frame, l - t, l + t, 0.0, h.top, h.lo, true),
        wallX(frame, l - t, l + t, 0.0, h.top, h.hi, false),
        ceilingFace(top, h.lo, l - t, h.hi, l + t, h.top),
        floorFace(sill, h.lo, l - t, h.hi, l + t, 0.0),
      ]
    | false => [
        wallZ(frame, l - t, l + t, 0.0, h.top, h.lo, true),
        wallZ(frame, l - t, l + t, 0.0, h.top, h.hi, false),
        ceilingFace(top, l - t, h.lo, l + t, h.hi, h.top),
        floorFace(sill, l - t, h.lo, l + t, h.hi, 0.0),
      ]

let panelW = 0.6
let panelL = 1.2
// A ceiling panel, its long side along the room's long axis.
let panelFace = (r: Rect, p: Panel): Face =>
  let alongZ = r.z1 - r.z0 >= r.x1 - r.x0 in
  let hx = (if alongZ then panelW else panelL) / 2.0 in
  let hz = (if alongZ then panelL else panelW) / 2.0 in
  ceilingFace(if p.on then panelOn else panelOff, p.x - hx, p.z - hz, p.x + hx, p.z + hz, ceiling - 0.01)

// A sign 2 cm proud of a room's inner wall.
let signFace = (f: Fixture): Face =>
  let r = inner(roomAt(f.room).rect) in
  let (mat, w, y0, y1) = (match f.sign with
    | ExitSign => (exitGreen, 0.5, 2.2, 2.45)
    | StairDoor => (sodium, 0.95, 0.0, 2.1)) in
  let lo = f.at - w / 2.0 in
  let hi = f.at + w / 2.0 in
  match f.side with
  | North => wallZ(mat, lo, hi, y0, y1, r.z1 - 0.02, false)
  | South => wallZ(mat, lo, hi, y0, y1, r.z0 + 0.02, true)
  | East => wallX(mat, lo, hi, y0, y1, r.x1 - 0.02, false)
  | West => wallX(mat, lo, hi, y0, y1, r.x0 + 0.02, true)

// Every interior face of room `i`, sorted by material (one group each).
let roomFaces = (i: float): List<Face> =>
  let room = roomAt(i) in
  if not isInside(room) then []
  else
    let r = inner(room.rect) in
    let floorMat = if room.kind == Lobby then terrazzo else carpet in
    List.flatten([
      [floorFace(floorMat, r.x0, r.z0, r.x1, r.z1, 0.0), ceilingFace(ceilingTile, r.x0, r.z0, r.x1, r.z1, ceiling)],
      wallFaces(i),
      doors |> List.filter((d) => d.b == i) |> List.concatMap(jambFaces),
      room.panels |> List.map((p) => panelFace(room.rect, p)),
      fixtures |> List.filter((f) => f.room == i) |> List.map(signFace),
    ]) |> List.sortBy((f) => f.mat)

let cellFaces = List.range(List.length(rooms)) |> List.map(roomFaces)
let allFaces = List.flatten(cellFaces)

// ---- The interior mesh -----------------------------------------------------

let corners = (q: Quad): List<Vec3.t> =>
  [q.o, q.o |> Vec3.add(q.a), q.o |> Vec3.add(q.a) |> Vec3.add(q.b), q.o |> Vec3.add(q.b)]
let xyz = (p: Vec3.t): List<float> => [Vec3.x(p), Vec3.y(p), Vec3.z(p)]
let normalOf = (q: Quad): Vec3.t => q.a |> Vec3.cross(q.b) |> Vec3.normalize
let dot = (a: Vec3.t, b: Vec3.t): float => Vec3.x(a) * Vec3.x(b) + Vec3.y(a) * Vec3.y(b) + Vec3.z(a) * Vec3.z(b)
// Texture UVs: world-anchored repeats along the face's axes, so tiles line
// up across neighbouring faces; tile 0 stretches the texture over the face.
let uvsOf = (f: Face): List<float> =>
  let t = materialAt(f.mat).tile in
  if t == 0.0 then [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
  else
    let au = Vec3.normalize(f.q.a) in
    let bu = Vec3.normalize(f.q.b) in
    corners(f.q) |> List.concatMap((p) => [dot(p, au) / t, 0.0 - dot(p, bu) / t])

// Each cell's triangles are one contiguous range of the mesh, in cell order.
type Range = { first: float, count: float }
let cellRanges: List<Range> =
  let (ranges, total) = cellFaces |> List.fold((acc, faces) =>
    let (out, first) = acc in
    let count = 2.0 * List.length(faces) in
    (out |> List.append([{ first: first, count: count }]), first + count), ([], 0.0)) in
  ranges

// One span per material present in a cell, `first` counted from `base`.
type Span = { mat: float, first: float, count: float }
let spansOf = (base: float, faces: List<Face>): List<Span> =>
  let (spans, next) = List.range(List.length(materials)) |> List.fold((acc, m) =>
    let (out, first) = acc in
    let n = 2.0 * List.length(faces |> List.filter((f) => f.mat == m)) in
    if n == 0.0 then acc
    else (out |> List.append([{ mat: m, first: first, count: n }]), first + n), ([], base)) in
  spans
let groupOf = (s: Span): Model.group =>
  { first: s.first, count: s.count, texture: Option.Some(textureAt(s.mat)), unlit: materialAt(s.mat).unlit }

// Per cell (its triangles counted from 0), and for the whole mesh.
let cellSpans = cellFaces |> List.map((faces) => spansOf(0.0, faces))
let meshSpans = List.zip(cellRanges, cellFaces) |> List.concatMap((pair) =>
  let (faces, range) = pair in spansOf(range.first, faces))
let meshAlbedo = meshSpans |> List.map((s) => albedoAt(s.mat))

let interiorData: Model.staticData = {
  positions: Bytes.fromF32le(allFaces |> List.concatMap((f) => corners(f.q) |> List.concatMap(xyz))),
  normals: Bytes.fromF32le(allFaces |> List.concatMap((f) =>
    let n = xyz(normalOf(f.q)) in List.flatten([n, n, n, n]))),
  uvs: Bytes.fromF32le(allFaces |> List.concatMap(uvsOf)),
  uvs2: Bytes.fromF32le([]),
  triangles: Bytes.fromU32le(List.range(List.length(allFaces)) |> List.concatMap((k) =>
    [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i))),
  groups: meshSpans |> List.map(groupOf),
  lightmap: Option.None
}

// One static model per cell over the baked (or unbaked) mesh: all of its
// vertices, the cell's triangle range, and that cell's groups.
let cellModels = (data: Model.staticData): List<Option.t<Model.t>> =>
  List.zip(cellRanges, cellSpans) |> List.map((pair) =>
    let (spans, range) = pair in
    if range.count == 0.0 then Option.None
    else Option.Some(Model.static({ data with
      triangles: data.triangles |> Bytes.slice(range.first * 12.0, range.count * 12.0) |> Binary.require,
      groups: spans |> List.map(groupOf) })))

// ---- Cells and portals -------------------------------------------------------

let box3 = (r: Rect, top: float): Cells.cell => { planes: [
  { n: v(1.0, 0.0, 0.0), d: 0.0 - r.x0 }, { n: v(-1.0, 0.0, 0.0), d: r.x1 },
  { n: v(0.0, 1.0, 0.0), d: 0.0 },        { n: v(0.0, -1.0, 0.0), d: top },
  { n: v(0.0, 0.0, 1.0), d: 0.0 - r.z0 }, { n: v(0.0, 0.0, -1.0), d: r.z1 },
] }
let outsideTop = 8.0

// A door's opening as a polygon on its wall line.
let doorway = (d: Door): List<Vec3.t> =>
  let ra = roomAt(d.a).rect in
  let h = holeOf(d) in
  match sideBetween(ra, roomAt(d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let l = lineOf(side, ra) in
    if side == North || side == South then
      [v(h.lo, 0.0, l), v(h.hi, 0.0, l), v(h.hi, h.top, l), v(h.lo, h.top, l)]
    else [v(l, 0.0, h.lo), v(l, 0.0, h.hi), v(l, h.top, h.hi), v(l, h.top, h.lo)]

let level = Cells.make({
  cells: rooms |> List.map((r) => box3(r.rect, if isInside(r) then ceiling else outsideTop)),
  portals: doors |> List.concatMap((d) => [
    { from: d.a, to: d.b, polygon: doorway(d) },
    { from: d.b, to: d.a, polygon: List.reverse(doorway(d)) },
  ]),
})

// ---- Collision ---------------------------------------------------------------

// Solid boxes along each side of every room, between wall centre lines, with
// the door holes left open. Facade walls are boxes, not trimesh faces: a
// face only blocks from its front, so an inward-facing wall would let a
// capsule walk in from outside. Shared walls get a box from each room.
type Block = { tag: string, x: float, y: float, z: float, w: float, h: float, d: float }

let wallBlocks = (i: float): List<Block> =>
  let r = roomAt(i).rect in
  let side = (s: Side, s0: float, s1: float) =>
    let l = lineOf(s, r) in
    let t = 2.0 * halfWall(s, r) in
    let top = if onFootprint(s, r) then facadeHeight else ceiling in
    pieces(s0, s1, top, holesIn(i, s)) |> List.indexedMap((k, p) =>
      let tag = $"wall-{i}-{s}-{k}" in
      if s == North || s == South then
        { tag: tag, x: (p.u0 + p.u1) / 2.0, y: (p.y0 + p.y1) / 2.0, z: l, w: p.u1 - p.u0, h: p.y1 - p.y0, d: t }
      else { tag: tag, x: l, y: (p.y0 + p.y1) / 2.0, z: (p.u0 + p.u1) / 2.0, w: t, h: p.y1 - p.y0, d: p.u1 - p.u0 }) in
  if not isInside(roomAt(i)) then []
  else List.flatten([side(South, r.x0, r.x1), side(North, r.x0, r.x1), side(West, r.z0, r.z1), side(East, r.z0, r.z1)])

let blocks = List.range(List.length(rooms)) |> List.concatMap(wallBlocks)

// Interior floors and door thresholds, from the same faces the mesh draws.
let floorFaces = allFaces |> List.filter((f) => (f.mat == carpet || f.mat == terrazzo) && Vec3.y(f.q.o) == 0.0)
let floorShape = Physics.trimesh({
  positions: Bytes.fromF32le(floorFaces |> List.concatMap((f) => corners(f.q) |> List.concatMap(xyz))),
  triangles: Bytes.fromU32le(List.range(List.length(floorFaces)) |> List.concatMap((k) =>
    [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i))),
})

let buildingBodies = (rev: float): List<Physics.body> =>
  List.flatten([
    blocks |> List.map((b) => Physics.fixed(Physics.tag(b.tag), Physics.box(b.w, b.h, b.d)) |> Physics.at(v(b.x, b.y, b.z))),
    [Physics.fixed(Physics.tag("floors"), floorShape),
     // The street, under everything.
     Physics.fixed(Physics.tag("ground"), Physics.box(200.0, 1.0, 200.0)) |> Physics.at(v(10.0, -0.5, 10.0))],
  ])

// ---- Exterior ------------------------------------------------------------------

let rgb = (r: float, g: float, b: float): Color.t => Color.rgb(r / 255.0, g / 255.0, b / 255.0)
let concrete = rgb(72.0, 72.0, 82.0)
let glass = rgb(26.0, 32.0, 44.0)
let cyan = rgb(110.0, 240.0, 230.0)

let slab = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float) =>
  Scene.cube() |> Scene.scaleXYZ(x1 - x0, y1 - y0, z1 - z0) |> Scene.translate(v((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0))

// Doors through the facade along one footprint side.
let facadeHoles = (s: Side): List<Hole> =>
  doors
    |> List.filter((d) => not isInside(roomAt(d.a)) || not isInside(roomAt(d.b)))
    |> List.filter((d) =>
      let inside = if isInside(roomAt(d.a)) then roomAt(d.a) else roomAt(d.b) in
      let outside = if isInside(roomAt(d.a)) then roomAt(d.b) else roomAt(d.a) in
      sideBetween(inside.rect, outside.rect) == Option.Some(s) && onFootprint(s, inside.rect))
    |> List.map(holeOf)
    // 1 cm back from the reveal, so the baked jambs and head cover the joint.
    |> List.map((h) => { lo: h.lo - 0.01, hi: h.hi + 0.01, top: h.top + 0.01 })
    |> List.sortBy((h) => h.lo)

// The outer half of the facade (the inner half is the baked interior wall).
let half = facade / 2.0
let facadeSlabs =
  let f = footprint in
  let run = (s: Side, s0: float, s1: float, make: (Piece) => Scene.t) =>
    pieces(s0, s1, facadeHeight, facadeHoles(s)) |> List.map(make) in
  List.flatten([
    run(South, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z0 - half, p.u1, p.y1, f.z0)),
    run(North, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z1, p.u1, p.y1, f.z1 + half)),
    run(West, f.z0, f.z1, (p) => slab(f.x0 - half, p.y0, p.u0, f.x0, p.y1, p.u1)),
    run(East, f.z0, f.z1, (p) => slab(f.x1, p.y0, p.u0, f.x1 + half, p.y1, p.u1)),
  ]) |> Scene.group |> Scene.lit(concrete)

// Dark window bands on the outside of the walls (no apertures: open windows
// would need portals, and glass would bypass culling).
let windows =
  let f = footprint in
  let o = half + 0.02 in
  List.flatten([
    [2.0, 4.6, 15.4, 18.0] |> List.map((x) => slab(x - 1.1, 1.0, f.z0 - o, x + 1.1, 2.3, f.z0 - half)),
    List.range(8.0) |> List.concatMap((k) =>
      let z = 2.5 + 3.4 * k in
      [slab(f.x0 - o, 1.0, z - 1.1, f.x0 - half, 2.3, z + 1.1),
       slab(f.x1 + half, 1.0, z - 1.1, f.x1 + o, 2.3, z + 1.1)]),
  ]) |> Scene.group |> Scene.lit(glass)

let entrance = doors |> List.find((d) => not isInside(roomAt(d.a))) |> Option.map(holeOf)
  |> Option.defaultValue({ lo: 0.0, hi: 0.0, top: 0.0 })
let canopy =
  let x0 = entrance.lo - 1.0 in
  let x1 = entrance.hi + 1.0 in
  let z0 = footprint.z0 - half - 1.6 in
  Scene.group([
    slab(x0, entrance.top + 0.3, z0, x1, entrance.top + 0.45, footprint.z0 - half) |> Scene.lit(concrete),
    slab(x0, entrance.top + 0.33, z0 - 0.05, x1, entrance.top + 0.39, z0) |> Scene.color(cyan),
  ])

// Cyan along the roofline.
let roofline =
  let f = footprint in
  let y0 = facadeHeight - 0.12 in
  let y1 = facadeHeight - 0.06 in
  let o = half + 0.05 in
  Scene.group([
    slab(f.x0 - o, y0, f.z0 - o, f.x1 + o, y1, f.z0 - half),
    slab(f.x0 - o, y0, f.z1 + half, f.x1 + o, y1, f.z1 + o),
    slab(f.x0 - o, y0, f.z0 - o, f.x0 - half, y1, f.z1 + o),
    slab(f.x1 + half, y0, f.z0 - o, f.x1 + o, y1, f.z1 + o),
  ]) |> Scene.color(cyan)

// Sodium street lamps by the apron.
let lampHeads = [v(4.4, 4.8, -4.0), v(15.6, 4.8, -4.0)]
let lampPosts = lampHeads |> List.map((p) =>
  let x = Vec3.x(p) in
  let z = Vec3.z(p) in
  Scene.group([
    slab(x - 0.06, 0.0, z - 0.06, x + 0.06, 4.9, z + 0.06) |> Scene.lit(rgb(60.0, 60.0, 66.0)),
    slab(x - 0.25, 4.75, z - 0.12, x + 0.25, 4.85, z + 0.12) |> Scene.color(rgb(255.0, 190.0, 110.0)),
  ])) |> Scene.group

let apron = roomAt(0.0).rect
let ground = Scene.group([
  // Below the interior floors (y = 0), which would otherwise z-fight it.
  Scene.plane() |> Scene.scaleXYZ(240.0, 1.0, 240.0) |> Scene.translate(v(10.0, -0.03, 10.0)) |> Scene.lit(rgb(40.0, 36.0, 46.0)),
  slab(apron.x0, -0.03, apron.z0, apron.x1, 0.0, apron.z1 - half) |> Scene.lit(rgb(96.0, 94.0, 102.0)),
])

let exterior = Scene.group([ground, facadeSlabs, windows, canopy, roofline, lampPosts])
