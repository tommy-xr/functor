// Everything derived from layout.fun except the exterior (exterior.fun): the
// interior surfaces (one mesh, baked together, then cut into one mesh per
// cell), the bake jobs and their lights, the culling cells and portals, and
// the wall, floor and stair colliders. Editing this file makes the committed
// bake stale (provenance.fun hashes it with layout.fun).

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
// Speckled: carpet pile, terrazzo, castConcrete.
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
let concretePaint = paint(148.0, 144.0, 134.0)

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
let castConcrete = 9.0

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
  material(concretePaint, 1.0, false, speckled(0.12, concretePaint)),
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
// `f` moved up by `dy`.
let lift = (dy: float, f: Face): Face =>
  { mat: f.mat, q: { o: f.q.o |> Vec3.add(v(0.0, dy, 0.0)), a: f.q.a, b: f.q.b } }

let corners = (q: Quad): List<Vec3.t> =>
  [q.o, q.o |> Vec3.add(q.a), q.o |> Vec3.add(q.a) |> Vec3.add(q.b), q.o |> Vec3.add(q.b)]
let normalOf = (q: Quad): Vec3.t => q.a |> Vec3.cross(q.b) |> Vec3.normalize
// The lowest corner's height.
let lowestY = (f: Face): float => corners(f.q) |> List.map(Vec3.y) |> List.minimum |> Option.defaultValue(0.0)

// ---- Rooms, walls and doors --------------------------------------------------

let eps = 0.001
let near = (a: float, b: float): bool => Math.abs(a - b) < eps

let roomAt = (i: float): Room =>
  match List.nth(i, rooms) with
  | Option.Some(r) => r
  | Option.None => Binary.invalid($"no room {i}")
let isInside = (r: Room): bool => r.kind != Outside
// A room's floor height, and the levels' floors.
let baseOf = (r: Room): float => r.level * storey
let levels = [0.0, 1.0]

// A stairwell is a room on each level: the foot (the lower one) has no
// ceiling and walls a full storey high; the head's floor is the landing.
let isStairFoot = (i: float): bool => flights |> List.any((f) => f.lower == i)
let isStairHead = (i: float): bool => flights |> List.any((f) => f.upper == i)
let wallTop = (i: float): float => if isStairFoot(i) then storey else ceiling
let floorMatOf = (r: Room): float => if r.kind == Office || r.kind == Corridor then carpet else terrazzo

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

// The openings through the facade on footprint side `s` at `level`: the
// entrance and the windows, in order along it, their heights from that
// level's floor.
let openingsIn = (level: float, s: Side): List<Hole> =>
  List.range(List.length(rooms))
    |> List.filter((i) => isInside(roomAt(i)) && roomAt(i).level == level && onFootprint(s, roomAt(i).rect))
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

// A room's four inner walls, facing in, from its own floor.
let wallFaces = (i: float): List<Face> =>
  let r = inner(roomAt(i).rect) in
  let side = (s: Side, s0: float, s1: float, make: (Piece) => Face) =>
    pieces(s0, s1, wallTop(i), holesIn(i, s)) |> List.map(make) in
  List.flatten([
    side(South, r.x0, r.x1, (p) => wallZ(wall, p.u0, p.u1, p.y0, p.y1, r.z0, true)),
    side(North, r.x0, r.x1, (p) => wallZ(wall, p.u0, p.u1, p.y0, p.y1, r.z1, false)),
    side(West, r.z0, r.z1, (p) => wallX(wall, p.u0, p.u1, p.y0, p.y1, r.x0, true)),
    side(East, r.z0, r.z1, (p) => wallX(wall, p.u0, p.u1, p.y0, p.y1, r.x1, false)),
  ])

// An opening's reveal between `l0` and `l1` across its wall: the two jambs,
// the head and the sill (a door's threshold).
let revealFaces = (side: Side, l0: float, l1: float, h: Hole, jamb: float, top: float, sill: float): List<Face> =>
  if side == North || side == South then [
      wallX(jamb, l0, l1, h.bottom, h.top, h.lo, true),
      wallX(jamb, l0, l1, h.bottom, h.top, h.hi, false),
      ceilingFace(top, h.lo, l0, h.hi, l1, h.top),
      floorFace(sill, h.lo, l0, h.hi, l1, h.bottom),
    ]
  else [
      wallZ(jamb, l0, l1, h.bottom, h.top, h.lo, true),
      wallZ(jamb, l0, l1, h.bottom, h.top, h.hi, false),
      ceilingFace(top, l0, h.lo, l1, h.hi, h.top),
      floorFace(sill, l0, h.lo, l1, h.hi, h.bottom),
    ]

// Room `i`'s half of door `d`'s reveal, from the wall's centre line to its
// face on `i`'s side: each half is in its own room's cell, so a half seen
// from the other room is always seen through the doorway's portal. A
// floor-to-ceiling opening's head is a ceiling strip.
let doorHalf = (d: Door, i: float): List<Face> =>
  let ra = roomAt(d.a).rect in
  match sideBetween(ra, roomAt(d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let l = lineOf(side, ra) in
    let t = halfWall(side, ra) in
    let h = holeOf(d) in
    let room = roomAt(i) in
    // Room `a` is below the line when the line is its North or East side.
    let below = (i == d.a) == (side == North || side == East) in
    let top = if h.top < ceiling - eps then frame else ceilingTile in
    let half = if below then revealFaces(side, l - t, l, h, frame, top, floorMatOf(room))
      else revealFaces(side, l, l + t, h, frame, top, floorMatOf(room)) in
    half |> List.map((f) => lift(baseOf(room), f))

// A window's reveal through the whole facade, painted like the walls.
let windowFaces = (w: Window): List<Face> =>
  let r = roomAt(w.room).rect in
  let l = lineOf(w.side, r) in
  let t = halfWall(w.side, r) in
  revealFaces(w.side, l - t, l + t, windowHole(w), wall, wall, wall)

let panelW = 0.6
let panelL = 1.2
// A ceiling panel, its long side along the room's long axis.
let panelFace = (r: Rect, p: Panel): Face =>
  let alongZ = r.z1 - r.z0 >= r.x1 - r.x0 in
  let hx = (if alongZ then panelW else panelL) / 2.0 in
  let hz = (if alongZ then panelL else panelW) / 2.0 in
  ceilingFace(if p.on then panelOn else panelOff, p.x - hx, p.z - hz, p.x + hx, p.z + hz, ceiling - 0.01)

// A plate `w` wide over [y0, y1], `out` proud of room `room`'s inner wall
// on `side`, centred `at` along it.
let plate = (mat: float, room: float, side: Side, at: float, w: float, y0: float, y1: float, out: float): Face =>
  let r = inner(roomAt(room).rect) in
  let lo = at - w / 2.0 in
  let hi = at + w / 2.0 in
  match side with
  | North => wallZ(mat, lo, hi, y0, y1, r.z1 - out, false)
  | South => wallZ(mat, lo, hi, y0, y1, r.z0 + out, true)
  | East => wallX(mat, lo, hi, y0, y1, r.x1 - out, false)
  | West => wallX(mat, lo, hi, y0, y1, r.x0 + out, true)

let signFace = (f: Fixture): Face =>
  match f.sign with
  | ExitSign => plate(exitGreen, f.room, f.side, f.at, 0.5, 2.2, 2.45, 0.02)
  | StairSign => plate(sodium, f.room, f.side, f.at, 0.7, 2.2, 2.42, 0.02)
let lampFace = (l: Lamp): Face => plate(sodium, l.room, l.side, l.at, 0.5, l.y - 0.1, l.y + 0.1, 0.03)

// Room `i`'s own faces: floor, ceiling, walls, panels, signs, lamps and
// window reveals.
let roomFaces = (i: float): List<Face> =>
  let room = roomAt(i) in
  if not isInside(room) then []
  else
    let r = inner(room.rect) in
    let mat = floorMatOf(room) in
    // A stairwell's foot has no floor under its flight and landing (solid).
    let floors = (match flights |> List.find((f) => f.lower == i) with
      | Option.Some(f) => [floorFace(mat, r.x0, r.z0, f.x0, r.z1, 0.0), floorFace(mat, f.x0, f.z1, r.x1, r.z1, 0.0)]
      | Option.None => if isStairHead(i) then [] else [floorFace(mat, r.x0, r.z0, r.x1, r.z1, 0.0)]) in
    List.flatten([
      floors,
      if isStairFoot(i) then [] else [ceilingFace(ceilingTile, r.x0, r.z0, r.x1, r.z1, ceiling)],
      wallFaces(i),
      room.panels |> List.map((p) => panelFace(room.rect, p)),
      fixtures |> List.filter((f) => f.room == i) |> List.map(signFace),
      lamps |> List.filter((l) => l.room == i) |> List.map(lampFace),
      windows |> List.filter((w) => w.room == i) |> List.concatMap(windowFaces),
    ]) |> List.map((f) => lift(baseOf(room), f))

// ---- Stairs ------------------------------------------------------------------

let parapetHeight = 1.0
let parapetThickness = 0.15
let slabDepth = 0.3
let riserOf = (f: Flight): float => storey / f.risers
// Where the last riser rises to the landing.
let flightEnd = (f: Flight): float => f.x0 + (f.risers - 1.0) * f.tread

// [y0, y1], cut where the upper room starts, so each piece lies in one cell.
let cut = (y0: float, y1: float): List<(float, float)> =>
  if y0 < storey - eps && y1 > storey + eps then [(y0, storey), (storey, y1)] else [(y0, y1)]
let wallZs = (mat: float, x0: float, x1: float, y0: float, y1: float, z: float, plus: bool): List<Face> =>
  cut(y0, y1) |> List.map((ys) => let (a, b) = ys in wallZ(mat, x0, x1, a, b, z, plus))
let wallXs = (mat: float, z0: float, z1: float, y0: float, y1: float, x: float, plus: bool): List<Face> =>
  cut(y0, y1) |> List.map((ys) => let (a, b) = ys in wallX(mat, z0, z1, a, b, x, plus))

// A flight's treads and risers, its stepped parapet (1 m above each tread,
// solid down to the floor), the solid under the landing, and the landing
// with its parapet; from the lower room's floor.
let flightFaces = (f: Flight): List<Face> =>
  let r = inner(roomAt(f.lower).rect) in
  let rise = riserOf(f) in
  let x1 = flightEnd(f) in
  let zp = f.z1 - parapetThickness in
  let steps = List.range(f.risers - 1.0) |> List.concatMap((j) =>
    let xa = f.x0 + j * f.tread in
    let xb = xa + f.tread in
    let y = (j + 1.0) * rise in
    let top = y + parapetHeight in
    List.flatten([
      [floorFace(terrazzo, xa, f.z0, xb, zp, y), wallX(castConcrete, f.z0, zp, y - rise, y, xa, false)],
      wallZs(castConcrete, xa, xb, 0.0, top, f.z1, true),
      wallZs(castConcrete, xa, xb, y, top, zp, false),
      [floorFace(castConcrete, xa, zp, xb, f.z1, top)],
      // Up to the next step's parapet, or the landing's.
      wallXs(castConcrete, zp, f.z1, top, top + rise, xb, false),
    ])) in
  List.flatten([
    steps,
    [wallX(castConcrete, zp, f.z1, 0.0, rise + parapetHeight, f.x0, false),
     wallX(castConcrete, f.z0, zp, storey - rise, storey, x1, false),
     wallZ(castConcrete, x1, r.x1, 0.0, storey - slabDepth, f.z1, true),
     floorFace(terrazzo, x1, r.z0, r.x1, r.z1, storey),
     ceilingFace(castConcrete, x1, f.z1, r.x1, r.z1, storey - slabDepth),
     floorFace(castConcrete, x1, zp, x1 + parapetThickness, r.z1, storey + parapetHeight),
     wallZ(castConcrete, x1, x1 + parapetThickness, storey, storey + parapetHeight, zp, false)],
    // The landing's edge and its parapet's face over the stair opening.
    wallXs(castConcrete, f.z1, r.z1, storey - slabDepth, storey + parapetHeight, x1, false),
    wallXs(castConcrete, zp, r.z1, storey, storey + parapetHeight, x1 + parapetThickness, true),
  ]) |> List.map((face) => lift(baseOf(roomAt(f.lower)), face))

// Room `i`'s share of the stairs: below its flight's upper floor for the
// foot, at or above it for the head.
let stairFaces = (i: float): List<Face> =>
  flights |> List.concatMap((f) =>
    let split = baseOf(roomAt(f.upper)) - eps in
    if f.lower == i then flightFaces(f) |> List.filter((face) => lowestY(face) < split)
    else if f.upper == i then flightFaces(f) |> List.filter((face) => lowestY(face) >= split)
    else [])

// ---- The interior mesh -----------------------------------------------------

// Every face in cell `i` (room `i`), sorted by material (one group each):
// the room's own, its halves of its doors, and its share of the stairs.
let cellFaces = (i: float): List<Face> =>
  List.flatten([
    roomFaces(i),
    doors |> List.filter((d) => d.a == i || d.b == i) |> List.concatMap((d) => doorHalf(d, i)),
    stairFaces(i),
  ]) |> List.sortBy((f) => f.mat)

// The mesh is every cell's faces, in cell order: each cell is a contiguous
// triangle range.
let cellParts: List<List<Face>> = List.range(List.length(rooms)) |> List.map(cellFaces)
let allFaces = List.flatten(cellParts)

let xyz = (p: Vec3.t): List<float> => [Vec3.x(p), Vec3.y(p), Vec3.z(p)]
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

// Each cell's triangle range, in cell order.
type Range = { first: float, count: float }
let cellRanges: List<Range> =
  let (ranges, total) = cellParts |> List.fold((acc, faces) =>
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
  { first: s.first, count: s.count, texture: Option.Some(materialAt(s.mat).texture), unlit: materialAt(s.mat).unlit }

// For the whole mesh, and per cell (its triangles counted from 0).
let meshSpans = cellParts |> List.zip(cellRanges) |> List.concatMap((pair) =>
  let (faces, range) = pair in spansOf(range.first, faces))
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

// ---- One mesh per cell -------------------------------------------------------

// The bake keeps triangle order, numbers its vertices by first use, and no
// two faces share a vertex; so each cell's vertices are one run, from the
// first index of its first triangle up to the next cell's. A cell's mesh
// is that run of every vertex array (UV2 byte for byte) and its triangles
// less the run's start. The unbaked mesh is laid out the same way.
type Run = { cell: float, first: float, count: float, v0: float, v1: float }
let runsOf = (data: Model.staticData): List<Run> =>
  let cells = cellRanges
    |> List.indexedMap((i, r) => { cell: i, first: r.first, count: r.count, v0: 0.0, v1: 0.0 })
    |> List.filter((run) => run.count > 0.0) in
  let starts = cells |> List.map((run) => Bytes.u32le(12.0 * run.first, data.triangles) |> Binary.require) in
  let ends = List.drop(1.0, starts) |> List.append([Bytes.length(data.positions) / 12.0]) in
  cells |> List.zip(starts) |> List.zip(ends) |> List.map((pair) =>
    let (started, v1) = pair in
    let (run, v0) = started in
    { run with v0: v0, v1: v1 })

let sliceRun = (stride: float, run: Run, b: Bytes.t): Bytes.t =>
  if Bytes.length(b) == 0.0 then b
  else b |> Bytes.slice(stride * run.v0, stride * (run.v1 - run.v0)) |> Binary.require
// `run`'s cell's mesh, cut from `data` (keeping its lightmap).
let cellData = (data: Model.staticData, run: Run): Model.staticData =>
  { data with
    positions: sliceRun(12.0, run, data.positions),
    normals: sliceRun(12.0, run, data.normals),
    uvs: sliceRun(8.0, run, data.uvs),
    uvs2: sliceRun(8.0, run, data.uvs2),
    triangles: Bytes.fromU32le(List.range(3.0 * run.count) |> List.map((k) =>
      (Bytes.u32le(4.0 * (3.0 * run.first + k), data.triangles) |> Binary.require) - run.v0)),
    groups: spansOf(0.0, List.nth(run.cell, cellParts) |> Option.defaultValue([])) |> List.map(groupOf) }

// A static model per non-empty cell, drawn `Scene.inCells` it.
type CellModel = { cell: float, model: Model.t }
let cellModels = (data: Model.staticData): List<CellModel> =>
  runsOf(data) |> List.map((run) => { cell: run.cell, model: Model.static(cellData(data, run)) })

// ---- Cells and portals -------------------------------------------------------

let box3 = (r: Rect, y0: float, y1: float): Cells.cell => { planes: [
  { n: v(1.0, 0.0, 0.0), d: 0.0 - r.x0 }, { n: v(-1.0, 0.0, 0.0), d: r.x1 },
  { n: v(0.0, 1.0, 0.0), d: 0.0 - y0 },   { n: v(0.0, -1.0, 0.0), d: y1 },
  { n: v(0.0, 0.0, 1.0), d: 0.0 - r.z0 }, { n: v(0.0, 0.0, -1.0), d: r.z1 },
] }
let outsideTop = 8.0

// Room `i`'s cell: its rect, from its floor up to its walls' top.
let cellOf = (i: float): Cells.cell =>
  let r = roomAt(i) in
  if isInside(r) then box3(r.rect, baseOf(r), baseOf(r) + wallTop(i)) else box3(r.rect, 0.0, outsideTop)

// A door's opening as a polygon on its wall line.
let doorway = (d: Door): List<Vec3.t> =>
  let ra = roomAt(d.a).rect in
  let h = holeOf(d) in
  let y = baseOf(roomAt(d.a)) in
  match sideBetween(ra, roomAt(d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let l = lineOf(side, ra) in
    if side == North || side == South then
      [v(h.lo, y, l), v(h.hi, y, l), v(h.hi, y + h.top, l), v(h.lo, y + h.top, l)]
    else [v(l, y, h.lo), v(l, y, h.hi), v(l, y + h.top, h.hi), v(l, y + h.top, h.lo)]

// A flight's stair opening: the floor of its upper room less the landing.
let stairOpening = (f: Flight): List<Vec3.t> =>
  let r = inner(roomAt(f.lower).rect) in
  let y = baseOf(roomAt(f.upper)) in
  let x1 = flightEnd(f) in
  [v(r.x0, y, r.z0), v(x1, y, r.z0), v(x1, y, r.z1), v(r.x0, y, r.z1)]

// The doors' portals, and a horizontal one at each stair opening.
let level = Cells.make({
  cells: List.range(List.length(rooms)) |> List.map(cellOf),
  portals: List.flatten([
    doors |> List.concatMap((d) => [
      { from: d.a, to: d.b, polygon: doorway(d) },
      { from: d.b, to: d.a, polygon: List.reverse(doorway(d)) },
    ]),
    flights |> List.concatMap((f) => [
      { from: f.lower, to: f.upper, polygon: stairOpening(f) },
      { from: f.upper, to: f.lower, polygon: List.reverse(stairOpening(f)) },
    ]),
  ]),
})

// ---- Collision ---------------------------------------------------------------

// Solid boxes along each side of every room, between wall centre lines, with
// the door holes left open. Facade walls are boxes, not trimesh faces: a
// face only blocks from its front, so an inward-facing wall would let a
// capsule walk in from outside. Shared walls get a box from each room.
// Walls are a storey high, so the plenum leaves no gap; the top storey's
// facade rises to the roofline.
type Block = { tag: string, x: float, y: float, z: float, w: float, h: float, d: float }

let wallBlocks = (i: float): List<Block> =>
  let room = roomAt(i) in
  let r = room.rect in
  let y = baseOf(room) in
  let side = (s: Side, s0: float, s1: float) =>
    let l = lineOf(s, r) in
    let t = 2.0 * halfWall(s, r) in
    let top = if onFootprint(s, r) && room.level > 0.0 then facadeHeight - y else storey in
    pieces(s0, s1, top, holesIn(i, s)) |> List.indexedMap((k, p) =>
      let tag = $"wall-{i}-{s}-{k}" in
      if s == North || s == South then
        { tag: tag, x: (p.u0 + p.u1) / 2.0, y: y + (p.y0 + p.y1) / 2.0, z: l, w: p.u1 - p.u0, h: p.y1 - p.y0, d: t }
      else { tag: tag, x: l, y: y + (p.y0 + p.y1) / 2.0, z: (p.u0 + p.u1) / 2.0, w: t, h: p.y1 - p.y0, d: p.u1 - p.u0 }) in
  if not isInside(room) then []
  else List.flatten([side(South, r.x0, r.x1), side(North, r.x0, r.x1), side(West, r.z0, r.z1), side(East, r.z0, r.z1)])

// A box over [x0, x1] × [y0, y1] × [z0, z1].
let block = (tag: string, x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Block =>
  { tag: tag, x: (x0 + x1) / 2.0, y: (y0 + y1) / 2.0, z: (z0 + z1) / 2.0, w: x1 - x0, h: y1 - y0, d: z1 - z0 }
// A flight's parapet, from the floor to its top, so nothing walks under or
// off the steps; the solid under its landing; the landing's parapet.
let flightBlocks = (k: float, f: Flight): List<Block> =>
  let r = inner(roomAt(f.lower).rect) in
  let y = baseOf(roomAt(f.lower)) in
  let x1 = flightEnd(f) in
  let zp = f.z1 - parapetThickness in
  [block($"parapet-{k}", f.x0, y, zp, x1, y + storey + parapetHeight, f.z1),
   block($"under-landing-{k}", x1, y, zp, r.x1, y + storey - slabDepth, f.z1),
   block($"landing-parapet-{k}", x1, y + storey - slabDepth, zp, x1 + parapetThickness, y + storey + parapetHeight, r.z1)]

let blocks = List.flatten([
  List.range(List.length(rooms)) |> List.concatMap(wallBlocks),
  flights |> List.indexedMap(flightBlocks) |> List.flatten,
])

// Interior floors, door thresholds and landings, from the faces the mesh
// draws: up-facing floor surfaces level with a storey's floor (not treads).
let isFloorLevel = (y: float): bool => levels |> List.any((l) => near(y, l * storey))
let floorFaces = allFaces |> List.filter((f) =>
  (f.mat == carpet || f.mat == terrazzo) && Vec3.y(normalOf(f.q)) > 0.5 && isFloorLevel(Vec3.y(f.q.o)))
let floorShape = Physics.trimesh({ positions: positionsOf(floorFaces), triangles: quadTriangles(List.length(floorFaces)) })

// A flight's ramp: one smooth slope through its steps' nosings, from a
// tread before the first riser up to the landing's edge, so the character
// walks it instead of hopping each step. It faces up (trimesh faces block
// only from the front). `rampY` is its height at `x`, from the lower floor.
let rampY = (f: Flight, x: float): float =>
  Math.clamp(0.0, storey, (x - (f.x0 - f.tread)) * riserOf(f) / f.tread)
let rampFace = (f: Flight): Face =>
  let start = f.x0 - f.tread in
  let zp = f.z1 - parapetThickness in
  face(carpet, v(start, baseOf(roomAt(f.lower)), zp), v(flightEnd(f) - start, storey, 0.0), v(0.0, 0.0, f.z0 - zp))
let rampShape = (f: Flight): Physics.shape =>
  Physics.trimesh({ positions: positionsOf([rampFace(f)]), triangles: quadTriangles(1.0) })

let buildingBodies = (rev: float): List<Physics.body> =>
  List.flatten([
    blocks |> List.map((b) => Physics.fixed(Physics.tag(b.tag), Physics.box(b.w, b.h, b.d)) |> Physics.at(v(b.x, b.y, b.z))),
    [Physics.fixed(Physics.tag("floors"), floorShape)],
    flights |> List.indexedMap((k, f) => Physics.fixed(Physics.tag($"ramp-{k}"), rampShape(f))),
  ])

// ---- The bake job --------------------------------------------------------------

// Fluorescent panels and sodium lamps: in the bake jobs ONLY. Submitting
// them to the frame would spend the 8 forward-light slots on light the
// lightmap already holds.
let panelLight = (base: float, p: Panel): Light.t =>
  Light.point(v(p.x, base + ceiling - 0.12, p.z), Color.rgb(0.93, 0.97, 0.86), 0.9, 5.5) |> Light.castShadows
// 0.3 m out from its lens, a little below it.
let lampLight = (l: Lamp): Light.t =>
  let lens = lampFace(l) in
  let centre = lens.q.o |> Vec3.add(Vec3.scale(0.5, lens.q.a)) |> Vec3.add(Vec3.scale(0.5, lens.q.b)) in
  let at = centre |> Vec3.add(Vec3.scale(0.3, normalOf(lens.q))) |> Vec3.add(v(0.0, baseOf(roomAt(l.room)) - 0.15, 0.0)) in
  Light.point(at, Color.rgb(1.0, 0.62, 0.3), 1.3, 8.0) |> Light.castShadows

// The lightmap stores light ÷ `bakeScale`; the game loads both bakes with it.
let bakeScale = 2.0
let night: Lightmap.bakeJob = {
  name: "night",
  mesh: interiorData,
  lights: List.flatten([
    [Light.ambient(Color.rgb(0.035, 0.04, 0.035))],
    rooms |> List.concatMap((r) => r.panels |> List.filter((p) => p.on) |> List.map((p) => panelLight(baseOf(r), p))),
    lamps |> List.map(lampLight),
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

// `p`'s spots for opening `h` in side `s`, on the storey whose floor is
// `base` up. The ground is always the street.
let patchLights = (s: Side, base: float, h: Hole, p: Patch): List<Light.t> =>
  let y0 = if p.aboveHead then base + h.top else 0.0 in
  let lo = h.lo - 1.0 in
  let width = h.hi - h.lo + 2.0 in
  let nu = Math.ceil(width / 0.5) in
  let nv = Math.ceil(p.height / 0.5) in
  let du = width / nu in
  let dv = p.height / nv in
  let target = outsideFacade(s, (h.lo + h.hi) / 2.0, base + p.aimY, -2.0) in
  List.range(nu * nv) |> List.map((k) =>
    let at = outsideFacade(s, lo + (Math.mod(k, nu) + 0.5) * du, y0 + (Math.floor(k / nu) + 0.5) * dv, p.out) in
    Light.spot(at, Vec3.normalize(target |> Vec3.sub(at)), p.color, p.perSquareMetre * du * dv, 12.0, 80deg)
      |> Light.castShadows)
let openingLights = (s: Side, base: float, h: Hole): List<Light.t> =>
  List.flatten([patchLights(s, base, h, skyPatch), patchLights(s, base, h, groundPatch)])

let daylightLights = List.flatten([
  [Light.directional(sunDirection, Color.rgb(1.0, 0.93, 0.8), 3.0) |> Light.castShadows],
  levels |> List.concatMap((l) => [North, South, East, West] |> List.concatMap((s) =>
    openingsIn(l, s) |> List.concatMap((h) => openingLights(s, l * storey, h)))),
])

// Day: the same mesh, so the same charts; the panels stay on.
let day: Lightmap.bakeJob = { night with name: "day", lights: List.flatten([night.lights, daylightLights]) }

// `functor bake` reads these through game.fun.
let bakes = [night, day]
