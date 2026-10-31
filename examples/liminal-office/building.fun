// Everything derived from layout.fun except the exterior (exterior.fun): the
// interior surfaces (one mesh, baked together, then drawn per cell), the bake
// job and its lights, the culling cells and portals, and the wall and floor
// colliders. Editing this file makes the committed bake stale (provenance.fun
// hashes it with layout.fun).

open Layout

let v = Vec3.make

// ---- Materials -------------------------------------------------------------

// sRGB bytes, as the 1-texel or procedural texture and the bake's albedo.
type Paint = { r: float, g: float, b: float }
let paint = (r: float, g: float, b: float): Paint => { r: r, g: g, b: b }

// `tile`: metres per texture repeat, world-anchored; 0 stretches the texture
// once across each face.
type Material = { tile: float, unlit: bool, texture: Asset.Texture, albedo: Color.t }

let rgb = (r: float, g: float, b: float): Color.t => Color.rgb(r / 255.0, g / 255.0, b / 255.0)

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
  { tile: tile, unlit: unlit, texture: Texture.fromPixels(pixels), albedo: rgb(p.r, p.g, p.b) }

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

let materialAt = (i: float): Material =>
  match List.nth(i, materials) with
  | Option.Some(m) => m
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
  match List.nth(i, rooms) with
  | Option.Some(r) => r
  | Option.None => Binary.invalid($"no room {i}")
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

// Whether that line is an outside wall, seen from either side: the apron's
// north edge is the lobby's south facade.
let onFootprint = (side: Side, r: Rect): bool =>
  let l = lineOf(side, r) in
  if side == North || side == South then near(l, footprint.z0) || near(l, footprint.z1)
  else near(l, footprint.x0) || near(l, footprint.x1)

// Half the thickness of the wall along a side: facade or partition.
let halfWall = (side: Side, r: Rect): float =>
  if onFootprint(side, r) then facade / 2.0 else partition / 2.0

// A room's inside: its rect less half of each wall.
let inner = (r: Rect): Rect =>
  rect(r.x0 + halfWall(West, r), r.z0 + halfWall(South, r),
       r.x1 - halfWall(East, r), r.z1 - halfWall(North, r))

// An opening's span along its wall, and its bottom and top.
type Hole = { lo: float, hi: float, bottom: float, top: float }
let holeOf = (d: Door): Hole => { lo: d.at - d.width / 2.0, hi: d.at + d.width / 2.0, bottom: 0.0, top: d.height }
let windowHole = (w: Window): Hole => { lo: w.at - w.width / 2.0, hi: w.at + w.width / 2.0, bottom: w.sill, top: w.head }

// The doors and windows in a room's side, as holes in increasing order.
let holesIn = (i: float, side: Side): List<Hole> =>
  let doorHoles = doors
    |> List.filter((d) =>
      let other = if d.a == i then Option.Some(d.b) else if d.b == i then Option.Some(d.a) else Option.None in
      match other with
      | Option.Some(o) => sideBetween(roomAt(i).rect, roomAt(o).rect) == Option.Some(side)
      | Option.None => false)
    |> List.map(holeOf) in
  let windowHoles = windows |> List.filter((w) => w.room == i && w.side == side) |> List.map(windowHole) in
  List.flatten([doorHoles, windowHoles]) |> List.sortBy((h) => h.lo)

// The openings through the facade on footprint side `s`: the entrance and
// the windows, in order along it.
let openingsIn = (s: Side): List<Hole> =>
  List.range(List.length(rooms))
    |> List.filter((i) => isInside(roomAt(i)) && onFootprint(s, roomAt(i).rect))
    |> List.concatMap((i) => holesIn(i, s))
    |> List.sortBy((h) => h.lo)

// The solid parts of a wall run [s0, s1] × [0, top] around its holes: full
// height between holes, a header above and a sill below each.
type Piece = { u0: float, u1: float, y0: float, y1: float }
let pieces = (s0: float, s1: float, top: float, holes: List<Hole>): List<Piece> =>
  let strip = (a: float, b: float): List<Piece> =>
    if b - a > eps then [{ u0: a, u1: b, y0: 0.0, y1: top }] else [] in
  let walked = holes |> List.fold((acc, h) =>
    let (cursor, out) = acc in
    let header = if top - h.top > eps then [{ u0: h.lo, u1: h.hi, y0: h.top, y1: top }] else [] in
    let sill = if h.bottom > eps then [{ u0: h.lo, u1: h.hi, y0: 0.0, y1: h.bottom }] else [] in
    (h.hi, List.flatten([out, strip(Math.max(cursor, s0), Math.min(h.lo, s1)), header, sill])),
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

// An opening's reveal through the wall on line `l`, `t` either side of it:
// the two jambs, the head and the sill (a door's threshold).
let revealFaces = (side: Side, l: float, t: float, h: Hole, jamb: float, top: float, sill: float): List<Face> =>
  if side == North || side == South then [
      wallX(jamb, l - t, l + t, h.bottom, h.top, h.lo, true),
      wallX(jamb, l - t, l + t, h.bottom, h.top, h.hi, false),
      ceilingFace(top, h.lo, l - t, h.hi, l + t, h.top),
      floorFace(sill, h.lo, l - t, h.hi, l + t, h.bottom),
    ]
  else [
      wallZ(jamb, l - t, l + t, h.bottom, h.top, h.lo, true),
      wallZ(jamb, l - t, l + t, h.bottom, h.top, h.hi, false),
      ceilingFace(top, l - t, h.lo, l + t, h.hi, h.top),
      floorFace(sill, l - t, h.lo, l + t, h.hi, h.bottom),
    ]

// A door's reveal; a floor-to-ceiling opening's head is a ceiling strip.
let jambFaces = (d: Door): List<Face> =>
  let ra = roomAt(d.a).rect in
  match sideBetween(ra, roomAt(d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let h = holeOf(d) in
    let top = if h.top < ceiling - eps then frame else ceilingTile in
    let sill = if roomAt(d.b).kind == Lobby then terrazzo else carpet in
    revealFaces(side, lineOf(side, ra), halfWall(side, ra), h, frame, top, sill)

// A window's reveal, painted like the walls.
let windowFaces = (w: Window): List<Face> =>
  let r = roomAt(w.room).rect in
  revealFaces(w.side, lineOf(w.side, r), halfWall(w.side, r), windowHole(w), wall, wall, wall)

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
      room.panels |> List.map((p) => panelFace(room.rect, p)),
      fixtures |> List.filter((f) => f.room == i) |> List.map(signFace),
      windows |> List.filter((w) => w.room == i) |> List.concatMap(windowFaces),
    ]) |> List.sortBy((f) => f.mat)

// The mesh in parts, each drawn as one model in the cells it shows from:
// every room, then every door's reveal, which straddles the doorway and so
// shows from both rooms. Each part is a contiguous triangle range.
type Part = { cells: List<float>, faces: List<Face> }
let parts: List<Part> = List.flatten([
  List.range(List.length(rooms)) |> List.map((i) => { cells: [i], faces: roomFaces(i) }),
  doors |> List.map((d) => { cells: [d.a, d.b], faces: jambFaces(d) |> List.sortBy((f) => f.mat) }),
])
let allFaces = parts |> List.concatMap((p) => p.faces)

// ---- The interior mesh -----------------------------------------------------

let corners = (q: Quad): List<Vec3.t> =>
  [q.o, q.o |> Vec3.add(q.a), q.o |> Vec3.add(q.a) |> Vec3.add(q.b), q.o |> Vec3.add(q.b)]
let xyz = (p: Vec3.t): List<float> => [Vec3.x(p), Vec3.y(p), Vec3.z(p)]
let normalOf = (q: Quad): Vec3.t => q.a |> Vec3.cross(q.b) |> Vec3.normalize
let positionsOf = (faces: List<Face>): Bytes.t =>
  Bytes.fromF32le(faces |> List.concatMap((f) => corners(f.q) |> List.concatMap(xyz)))
// Two triangles per quad, over `n` quads' corners.
let quadTriangles = (n: float): Bytes.t =>
  Bytes.fromU32le(List.range(n) |> List.concatMap((k) => [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i)))
// Texture UVs: world-anchored repeats along the face's axes, so tiles line
// up across neighbouring faces; tile 0 stretches the texture over the face.
let uvsOf = (f: Face): List<float> =>
  let t = materialAt(f.mat).tile in
  if t == 0.0 then [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
  else
    let au = Vec3.normalize(f.q.a) in
    let bu = Vec3.normalize(f.q.b) in
    corners(f.q) |> List.concatMap((p) => [Vec3.dot(au, p) / t, 0.0 - Vec3.dot(bu, p) / t])

// Each part's triangle range, in part order.
type Range = { first: float, count: float }
let partRanges: List<Range> =
  let (ranges, total) = parts |> List.fold((acc, p) =>
    let (out, first) = acc in
    let count = 2.0 * List.length(p.faces) in
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
  { first: s.first, count: s.count, texture: Option.Some(materialAt(s.mat).texture), unlit: materialAt(s.mat).unlit }

// For the whole mesh, and per part (its triangles counted from 0).
let meshSpans = parts |> List.zip(partRanges) |> List.concatMap((pair) =>
  let (p, range) = pair in spansOf(range.first, p.faces))
let meshAlbedo = meshSpans |> List.map((s) => materialAt(s.mat).albedo)

let interiorData: Model.staticData = {
  positions: positionsOf(allFaces),
  normals: Bytes.fromF32le(allFaces |> List.concatMap((f) =>
    let n = xyz(normalOf(f.q)) in List.flatten([n, n, n, n]))),
  uvs: Bytes.fromF32le(allFaces |> List.concatMap(uvsOf)),
  uvs2: Bytes.fromF32le([]),
  triangles: quadTriangles(List.length(allFaces)),
  groups: meshSpans |> List.map(groupOf),
  lightmap: Option.None
}

// One static model per non-empty part over the baked (or unbaked) mesh: all
// of its vertices, the part's triangle range and groups, and the cells it
// shows from.
type PartModel = { cells: List<float>, model: Model.t }
let partModels = (data: Model.staticData): List<PartModel> =>
  parts |> List.zip(partRanges) |> List.filter((pair) => let (p, range) = pair in range.count > 0.0)
    |> List.map((pair) =>
      let (p, range) = pair in
      { cells: p.cells, model: Model.static({ data with
        triangles: data.triangles |> Bytes.slice(range.first * 12.0, range.count * 12.0) |> Binary.require,
        groups: spansOf(0.0, p.faces) |> List.map(groupOf) }) })

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
let floorShape = Physics.trimesh({ positions: positionsOf(floorFaces), triangles: quadTriangles(List.length(floorFaces)) })

let buildingBodies = (rev: float): List<Physics.body> =>
  List.flatten([
    blocks |> List.map((b) => Physics.fixed(Physics.tag(b.tag), Physics.box(b.w, b.h, b.d)) |> Physics.at(v(b.x, b.y, b.z))),
    [Physics.fixed(Physics.tag("floors"), floorShape)],
  ])

// ---- The bake job --------------------------------------------------------------

// Fluorescent panels: in the bake job ONLY. Submitting them to the frame
// would spend the 8 forward-light slots on light the lightmap already holds.
let panelLight = (p: Panel): Light.t =>
  Light.point(v(p.x, ceiling - 0.12, p.z), Color.rgb(0.93, 0.97, 0.86), 0.9, 5.5) |> Light.castShadows

// The lightmap stores light ÷ `bakeScale`; the game loads both bakes with it.
let bakeScale = 2.0
let night: Lightmap.bakeJob = {
  name: "night",
  mesh: interiorData,
  lights: List.flatten([
    [Light.ambient(Color.rgb(0.035, 0.04, 0.035))],
    rooms |> List.concatMap((r) => r.panels |> List.filter((p) => p.on) |> List.map(panelLight)),
  ]),
  albedo: meshAlbedo,
  scale: bakeScale,
}

// ---- Daylight: the day bake only -----------------------------------------------

// The sun's fixed direction (travelling east, down and north; 33° up): the
// day bake's shadows and the frame's sun agree.
let sunDirection = v(0.7, -0.55, 0.45)

// A point `u` along footprint side `s`, `y` up, `out` metres outside the
// wall's centre line.
let outsideFacade = (s: Side, u: float, y: float, out: float): Vec3.t =>
  match s with
  | North => v(u, y, footprint.z1 + out)
  | South => v(u, y, footprint.z0 - out)
  | East => v(footprint.x1 + out, y, u)
  | West => v(footprint.x0 - out, y, u)

// What each facade opening sees: a patch of sky above its head and of
// sunlit ground below, each a grid of shadowed spots about 0.5 m apart, 1 m
// wider than the opening each side, aimed into the room. The facade's own
// walls shadow them, so the sky lights a soft, window-shaped patch on the
// floor and lower walls inside, and the ground the ceiling by the opening.
// Ambient can't stand in: its occlusion reaches only 1 m, so it would light
// windowless rooms too.
// A patch `out` m outside the wall's centre line, `height` m tall from the
// opening's head (sky) or the street (ground), aimed at the floor or the
// ceiling 2 m inside.
type Patch = { out: float, aboveHead: bool, height: float, aimY: float, color: Color.t, perSquareMetre: float }
let skyPatch = { out: 1.0, aboveHead: true, height: 2.0, aimY: 0.0, color: Color.rgb(0.72, 0.84, 1.0), perSquareMetre: 0.6 }
let groundPatch = { out: 2.0, aboveHead: false, height: 0.5, aimY: ceiling, color: Color.rgb(0.75, 0.72, 0.64), perSquareMetre: 0.3 }

// `p`'s spots for opening `h` in side `s`.
let patchLights = (s: Side, h: Hole, p: Patch): List<Light.t> =>
  let y0 = if p.aboveHead then h.top else 0.0 in
  let lo = h.lo - 1.0 in
  let width = h.hi - h.lo + 2.0 in
  let nu = Math.ceil(width / 0.5) in
  let nv = Math.ceil(p.height / 0.5) in
  let du = width / nu in
  let dv = p.height / nv in
  let target = outsideFacade(s, (h.lo + h.hi) / 2.0, p.aimY, -2.0) in
  List.range(nu * nv) |> List.map((k) =>
    let at = outsideFacade(s, lo + (Math.mod(k, nu) + 0.5) * du, y0 + (Math.floor(k / nu) + 0.5) * dv, p.out) in
    Light.spot(at, Vec3.normalize(target |> Vec3.sub(at)), p.color, p.perSquareMetre * du * dv, 12.0, 80deg)
      |> Light.castShadows)
let skyLights = (s: Side, h: Hole): List<Light.t> =>
  List.flatten([patchLights(s, h, skyPatch), patchLights(s, h, groundPatch)])

let daylightLights = List.flatten([
  [Light.directional(sunDirection, Color.rgb(1.0, 0.93, 0.8), 3.0) |> Light.castShadows],
  [North, South, East, West] |> List.concatMap((s) => openingsIn(s) |> List.concatMap((h) => skyLights(s, h))),
])

// Day: the same mesh, so the same charts; the panels stay on.
let day: Lightmap.bakeJob = { night with name: "day", lights: List.flatten([night.lights, daylightLights]) }

// `functor bake` reads these through game.fun.
let bakes = [night, day]
