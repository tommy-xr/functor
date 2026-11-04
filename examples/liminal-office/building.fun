// Everything derived from a `Layout` except the exterior (exterior.fun) and
// the culling cells (portals.fun): the interior surfaces (one mesh, baked
// together, then cut into one mesh per cell), the bake jobs and their lights,
// and the wall, floor and stair colliders. `interiorOf` derives them once
// per archetype. Editing this file makes every committed bake stale
// (provenance.fun hashes it with the layouts).

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

// Deterministic per-texel noise in [0, 1) (furniture.fun's).
let noise = Furniture.noise
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
let mustardPaint = paint(176.0, 138.0, 48.0)
let steelPaint = paint(170.0, 174.0, 178.0)
let doorPaint = paint(122.0, 110.0, 92.0)

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
let mustard = 10.0
let steel = 11.0
let doorPanel = 12.0
// A niche's panel (`Niche` fixtures).
let niche = 13.0
// The furniture's swatch atlas (furniture.fun). A furniture face shows
// swatch `s` of it: its `mat` is `swatchBase + s`.
let furnish = 14.0
let swatchBase = 100.0

// The atlas: each swatch's block from its paint, textured by kind (grain,
// weave, book spines), as furniture.fun lays them out.
let furnishImage = (paints: List<Paint>): Texture.pixels =>
  let n = Furniture.blockTexels in
  let side = Furniture.atlasSide * n in
  image(side, side, (x, y) =>
    let s = Math.floor(y / n) * Furniture.atlasSide + Math.floor(x / n) in
    let p = List.nth(s, paints) |> Option.defaultValue(paint(128.0, 128.0, 128.0)) in
    let u = Math.mod(x, n) in
    let t = Math.mod(y, n) in
    let grain = noise(x + 977.0 * y) in
    if s == Furniture.books then
      (let spine = Math.floor(u / 2.0) + 8.0 * Math.floor(y / n) in
       let k = noise(spine * 7.0 + 3.0) in
       let hue = if k < 0.33 then paint(p.r, p.g * 0.5, p.b * 0.4) else if k < 0.66 then paint(p.r * 0.45, p.g * 0.6, p.b) else p in
       shade(0.7 + 0.5 * noise(spine) + (if t < 2.0 then 0.25 else 0.0), hue))
    else if s == Furniture.laminate || s == Furniture.wood then shade(0.92 + 0.12 * noise(Math.floor(t / 2.0) * 13.0 + s) + 0.04 * grain, p)
    else if s == Furniture.fabric then shade(0.88 + 0.24 * grain + (if Math.mod(u + t, 4.0) == 0.0 then -0.06 else 0.0), p)
    else if s == Furniture.leaf || s == Furniture.dead then shade(0.8 + 0.4 * grain, p)
    else shade(0.96 + 0.08 * grain, p))
let swatchAverage = (paints: List<Paint>): Paint =>
  let n = Math.max(1.0, List.length(paints)) in
  paint((paints |> List.map((p) => p.r) |> List.sum) / n, (paints |> List.map((p) => p.g) |> List.sum) / n,
        (paints |> List.map((p) => p.b) |> List.sum) / n)
// The furnish material over swatch colours, in furniture.fun's order.
let furnishMaterial = (paints: List<Paint>): Material => material(swatchAverage(paints), 0.0, false, furnishImage(paints))
// The office's: pale laminate, blue-grey upholstery, near-black plastic,
// putty steel, brown wood, leaf green, off-white plastic, bottle blue,
// book spines, dead leaves.
let officeSwatches = [
  paint(196.0, 178.0, 140.0), paint(84.0, 98.0, 118.0), paint(34.0, 34.0, 36.0), paint(168.0, 164.0, 150.0),
  paint(112.0, 78.0, 50.0), paint(62.0, 104.0, 56.0), paint(214.0, 208.0, 192.0), paint(150.0, 196.0, 216.0),
  paint(150.0, 120.0, 90.0), paint(124.0, 96.0, 58.0),
]

// A palette: a `Material` per index above, in that order. This is the
// office's; another archetype may repaint it (housePalettes.fun).
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
  material(mustardPaint, 1.0, false, speckled(0.35, mustardPaint)),
  // Brushed: faint vertical streaks, stretched over each lift door.
  material(steelPaint, 0.0, false, image(16.0, 4.0, (x, y) => shade(1.0 + 0.12 * (noise(x) - 0.5), steelPaint))),
  material(doorPaint, 0.0, false, image(8.0, 16.0, (x, y) =>
    if x < 1.0 || x > 6.0 || y < 1.0 then shade(0.8, doorPaint) else doorPaint)),
  material(ceilingPaint, 0.0, false, solid(ceilingPaint)),
  furnishMaterial(officeSwatches),
]

let materialIn = (p: List<Material>, i: float): Material =>
  match List.nth(if i >= swatchBase then furnish else i, p) with
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

let roomAt = (l: Layout, i: float): Room =>
  match List.nth(i, l.rooms) with
  | Option.Some(r) => r
  | Option.None => Binary.invalid($"no room {i}")
// Whether a room has its own inner surfaces: not the street, not a roof.
let isInside = (r: Room): bool => r.kind != Outside && r.kind != Roof
// A room's floor height, and the levels' floors.
let baseOf = (l: Layout, r: Room): float => r.level * l.storey
let levelsOf = (l: Layout): List<float> => List.range(l.levels)
// How high the facade rises from `level`'s floor: a storey, or to the
// roofline from the top one.
let facadeTop = (l: Layout, level: float): float =>
  if level == l.levels - 1.0 then l.facadeHeight - level * l.storey else l.storey

// A stairwell is a room on each level: the foot (the lower one) has no
// ceiling and walls a full storey high; the head's floor is the landing. A
// room between two flights is both.
let isStairFoot = (l: Layout, i: float): bool => l.flights |> List.any((f) => f.lower == i)
let isStairHead = (l: Layout, i: float): bool => l.flights |> List.any((f) => f.upper == i)
let wallTop = (l: Layout, i: float): float => if isStairFoot(l, i) then l.storey else l.ceiling
let floorMatOf = (r: Room): float =>
  if r.kind == Office || r.kind == Corridor || r.kind == StairHall then carpet else if r.kind == LiftLobby then mustard else terrazzo
// A flight's treads and landing: its stairwell's floor.
let treadMatOf = (l: Layout, f: Flight): float => floorMatOf(roomAt(l, f.lower))

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
let onFootprint = (l: Layout, side: Side, r: Rect): bool =>
  let line = lineOf(side, r) in
  let f = l.footprint in
  if side == North || side == South then near(line, f.z0) || near(line, f.z1)
  else near(line, f.x0) || near(line, f.x1)

// Half the thickness of the wall along a side: facade or partition.
let halfWall = (l: Layout, side: Side, r: Rect): float =>
  if onFootprint(l, side, r) then l.facade / 2.0 else l.partition / 2.0

// A room's inside: its rect less half of each wall.
let inner = (l: Layout, r: Rect): Rect =>
  rect(r.x0 + halfWall(l, West, r), r.z0 + halfWall(l, South, r),
       r.x1 - halfWall(l, East, r), r.z1 - halfWall(l, North, r))

// An opening's span along its wall, and its bottom and top.
type Hole = { lo: float, hi: float, bottom: float, top: float }
let holeOf = (d: Door): Hole => { lo: d.at - d.width / 2.0, hi: d.at + d.width / 2.0, bottom: 0.0, top: d.height }
let windowHole = (w: Window): Hole => { lo: w.at - w.width / 2.0, hi: w.at + w.width / 2.0, bottom: w.sill, top: w.head }

// The doors in a room's side, as holes.
let doorHolesIn = (l: Layout, i: float, side: Side): List<Hole> =>
  l.doors
    |> List.filter((d) =>
      let other = if d.a == i then Option.Some(d.b) else if d.b == i then Option.Some(d.a) else Option.None in
      match other with
      | Option.Some(o) => sideBetween(roomAt(l, i).rect, roomAt(l, o).rect) == Option.Some(side)
      | Option.None => false)
    |> List.map(holeOf)
// The doors and windows in a room's side, as holes in increasing order.
let holesIn = (l: Layout, i: float, side: Side): List<Hole> =>
  let windowHoles = l.windows |> List.filter((w) => w.room == i && w.side == side) |> List.map(windowHole) in
  List.flatten([doorHolesIn(l, i, side), windowHoles]) |> List.sortBy((h) => h.lo)

// The openings through the facade on footprint side `s` at `level`: the
// entrance and the windows, in order along it, their heights from that
// level's floor.
let openingsIn = (l: Layout, level: float, s: Side): List<Hole> =>
  List.range(List.length(l.rooms))
    |> List.filter((i) => isInside(roomAt(l, i)) && roomAt(l, i).level == level && onFootprint(l, s, roomAt(l, i).rect))
    |> List.concatMap((i) => holesIn(l, i, s))
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
let wallFaces = (l: Layout, i: float): List<Face> =>
  let r = inner(l, roomAt(l, i).rect) in
  let side = (s: Side, s0: float, s1: float, make: (Piece) => Face) =>
    pieces(s0, s1, wallTop(l, i), holesIn(l, i, s)) |> List.map(make) in
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
let doorHalf = (l: Layout, d: Door, i: float): List<Face> =>
  let ra = roomAt(l, d.a).rect in
  match sideBetween(ra, roomAt(l, d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let line = lineOf(side, ra) in
    let t = halfWall(l, side, ra) in
    let h = holeOf(d) in
    let room = roomAt(l, i) in
    // Room `a` is below the line when the line is its North or East side.
    let below = (i == d.a) == (side == North || side == East) in
    let top = if h.top < l.ceiling - eps then frame else ceilingTile in
    let half = if below then revealFaces(side, line - t, line, h, frame, top, floorMatOf(room))
      else revealFaces(side, line, line + t, h, frame, top, floorMatOf(room)) in
    half |> List.map((f) => lift(baseOf(l, room), f))

// The outside room a window looks onto, if the layout has one there.
let outsideOf = (l: Layout, w: Window): Option.t<float> =>
  List.range(List.length(l.rooms)) |> List.find((o) =>
    roomAt(l, o).kind == Outside && sideBetween(roomAt(l, w.room).rect, roomAt(l, o).rect) == Option.Some(w.side))

// Cell `i`'s share of the window reveals, painted like the walls: the inner
// half of those in its walls, the outer half of those looking onto it (an
// outside room). With no outside room there, the room has the whole reveal.
let windowHalves = (l: Layout, i: float): List<Face> =>
  l.windows |> List.concatMap((w) =>
    let room = roomAt(l, w.room) in
    let line = lineOf(w.side, room.rect) in
    let t = halfWall(l, w.side, room.rect) in
    // The room is below the line when the line is its North or East side.
    let below = w.side == North || w.side == East in
    let half = (l0: float, l1: float) =>
      revealFaces(w.side, l0, l1, windowHole(w), wall, wall, wall) |> List.map((f) => lift(baseOf(l, room), f)) in
    let outside = outsideOf(l, w) in
    if w.room == i then
      (if outside == Option.None then half(line - t, line + t) else if below then half(line - t, line) else half(line, line + t))
    else if outside == Option.Some(i) then (if below then half(line, line + t) else half(line - t, line))
    else [])

let panelW = 0.6
let panelL = 1.2
// A ceiling panel, its long side along the room's long axis.
let panelFace = (l: Layout, r: Rect, p: Panel): Face =>
  let alongZ = r.z1 - r.z0 >= r.x1 - r.x0 in
  let hx = (if alongZ then panelW else panelL) / 2.0 in
  let hz = (if alongZ then panelL else panelW) / 2.0 in
  ceilingFace(if p.on then panelOn else panelOff, p.x - hx, p.z - hz, p.x + hx, p.z + hz, l.ceiling - 0.01)

// A plate `w` wide over [y0, y1], `out` proud of room `room`'s inner wall
// on `side`, centred `at` along it.
let plate = (l: Layout, mat: float, room: float, side: Side, at: float, w: float, y0: float, y1: float, out: float): Face =>
  let r = inner(l, roomAt(l, room).rect) in
  let lo = at - w / 2.0 in
  let hi = at + w / 2.0 in
  match side with
  | North => wallZ(mat, lo, hi, y0, y1, r.z1 - out, false)
  | South => wallZ(mat, lo, hi, y0, y1, r.z0 + out, true)
  | East => wallX(mat, lo, hi, y0, y1, r.x1 - out, false)
  | West => wallX(mat, lo, hi, y0, y1, r.x0 + out, true)

let signFace = (l: Layout, f: Fixture): Face =>
  match f.sign with
  | ExitSign => plate(l, exitGreen, f.room, f.side, f.at, 0.5, 2.2, 2.45, 0.02)
  | StairSign => plate(l, sodium, f.room, f.side, f.at, 0.7, 2.2, 2.42, 0.02)
  | LiftDoor => plate(l, steel, f.room, f.side, f.at, 1.1, 0.0, 2.2, 0.02)
  | ShutDoor => plate(l, doorPanel, f.room, f.side, f.at, 0.9, 0.0, 2.05, 0.02)
  | Skylight => Binary.invalid("a skylight is a slot, not a plate")
  | Niche => plate(l, niche, f.room, f.side, f.at, 1.4, 0.3, 2.2, 0.02)
let isPlate = (f: Fixture): bool => f.sign != Skylight

// A skylight's slot in its room's ceiling: 1 m across the room's middle,
// 3 m along z, centred `at`.
let slotOf = (l: Layout, f: Fixture): Rect =>
  let r = inner(l, roomAt(l, f.room).rect) in
  let x = (r.x0 + r.x1) / 2.0 in
  rect(x - 0.5, f.at - 1.5, x + 0.5, f.at + 1.5)
// Room `i`'s ceiling, less any skylight slot in it (open to the sky).
let ceilingFaces = (l: Layout, i: float): List<Face> =>
  let r = inner(l, roomAt(l, i).rect) in
  let y = l.ceiling in
  match l.fixtures |> List.find((f) => f.room == i && f.sign == Skylight) with
  | Option.None => [ceilingFace(ceilingTile, r.x0, r.z0, r.x1, r.z1, y)]
  | Option.Some(f) =>
    let s = slotOf(l, f) in
    [ceilingFace(ceilingTile, r.x0, r.z0, r.x1, s.z0, y), ceilingFace(ceilingTile, r.x0, s.z1, r.x1, r.z1, y),
     ceilingFace(ceilingTile, r.x0, s.z0, s.x0, s.z1, y), ceilingFace(ceilingTile, s.x1, s.z0, r.x1, s.z1, y)]
let lampFace = (l: Layout, lamp: Lamp): Face => plate(l, sodium, lamp.room, lamp.side, lamp.at, 0.5, lamp.y - 0.1, lamp.y + 0.1, 0.03)

// Room `i`'s own faces: floor, ceiling, walls, panels, signs and lamps.
let roomFaces = (l: Layout, i: float): List<Face> =>
  let room = roomAt(l, i) in
  if not isInside(room) then []
  else
    let r = inner(l, room.rect) in
    let mat = floorMatOf(room) in
    // A stairwell's head floors with the flight's landing; a straight
    // flight's foot has no floor under its flight and landing (solid).
    let whole = [floorFace(mat, r.x0, r.z0, r.x1, r.z1, 0.0)] in
    let floors = if isStairHead(l, i) then [] else (match l.flights |> List.find((f) => f.lower == i) with
      | Option.Some(f) => (match f.kind with
        | Straight => [floorFace(mat, r.x0, r.z0, f.x0, r.z1, 0.0), floorFace(mat, f.x0, f.z1, r.x1, r.z1, 0.0)]
        | DogLeg(_, _) => whole)
      | Option.None => whole) in
    List.flatten([
      floors,
      if isStairFoot(l, i) then [] else ceilingFaces(l, i),
      wallFaces(l, i),
      room.panels |> List.map((p) => panelFace(l, room.rect, p)),
      l.fixtures |> List.filter((f) => f.room == i && isPlate(f)) |> List.map((f) => signFace(l, f)),
      l.lamps |> List.filter((lamp) => lamp.room == i) |> List.map((lamp) => lampFace(l, lamp)),
    ]) |> List.map((f) => lift(baseOf(l, room), f))

// ---- Stairs ------------------------------------------------------------------

let parapetHeight = 1.0
let parapetThickness = 0.15
let slabDepth = 0.3
let riserOf = (l: Layout, f: Flight): float => l.storey / f.risers
// Where the last riser rises to the landing.
let flightEnd = (f: Flight): float => f.x0 + (f.risers - 1.0) * f.tread

// [y0, y1] from a flight's lower floor, cut at every floor it crosses, so
// each piece lies in one cell.
let cut = (l: Layout, y0: float, y1: float): List<(float, float)> =>
  let first = Math.floor((y0 + eps) / l.storey) + 1.0 in
  let floors = List.range(Math.max(0.0, Math.ceil((y1 - eps) / l.storey) - first)) |> List.map((k) => (first + k) * l.storey) in
  let edges = List.flatten([[y0], floors, [y1]]) in
  List.take(List.length(edges) - 1.0, edges) |> List.zip(List.drop(1.0, edges))
let wallZs = (l: Layout, mat: float, x0: float, x1: float, y0: float, y1: float, z: float, plus: bool): List<Face> =>
  cut(l, y0, y1) |> List.map((ys) => let (a, b) = ys in wallZ(mat, x0, x1, a, b, z, plus))
let wallXs = (l: Layout, mat: float, z0: float, z1: float, y0: float, y1: float, x: float, plus: bool): List<Face> =>
  cut(l, y0, y1) |> List.map((ys) => let (a, b) = ys in wallX(mat, z0, z1, a, b, x, plus))

// A flight's treads and risers, its stepped parapet (1 m above each tread,
// solid down to the floor), the solid under the landing, and the landing
// with its parapet; from the lower room's floor.
let straightFaces = (l: Layout, f: Flight): List<Face> =>
  let r = inner(l, roomAt(l, f.lower).rect) in
  let storey = l.storey in
  let rise = riserOf(l, f) in
  let x1 = flightEnd(f) in
  let zp = f.z1 - parapetThickness in
  let tread = treadMatOf(l, f) in
  let steps = List.range(f.risers - 1.0) |> List.concatMap((j) =>
    let xa = f.x0 + j * f.tread in
    let xb = xa + f.tread in
    let y = (j + 1.0) * rise in
    let top = y + parapetHeight in
    List.flatten([
      [floorFace(tread, xa, f.z0, xb, zp, y), wallX(castConcrete, f.z0, zp, y - rise, y, xa, false)],
      wallZs(l, castConcrete, xa, xb, 0.0, top, f.z1, true),
      wallZs(l, castConcrete, xa, xb, y, top, zp, false),
      [floorFace(castConcrete, xa, zp, xb, f.z1, top)],
      // Up to the next step's parapet, or the landing's.
      wallXs(l, castConcrete, zp, f.z1, top, top + rise, xb, false),
    ])) in
  List.flatten([
    steps,
    [wallX(castConcrete, zp, f.z1, 0.0, rise + parapetHeight, f.x0, false),
     wallX(castConcrete, f.z0, zp, storey - rise, storey, x1, false),
     wallZ(castConcrete, x1, r.x1, 0.0, storey - slabDepth, f.z1, true),
     floorFace(tread, x1, r.z0, r.x1, r.z1, storey),
     ceilingFace(castConcrete, x1, f.z1, r.x1, r.z1, storey - slabDepth),
     floorFace(castConcrete, x1, zp, x1 + parapetThickness, r.z1, storey + parapetHeight),
     wallZ(castConcrete, x1, x1 + parapetThickness, storey, storey + parapetHeight, zp, false)],
    // The landing's edge and its parapet's face over the stair opening.
    wallXs(l, castConcrete, f.z1, r.z1, storey - slabDepth, storey + parapetHeight, x1, false),
    wallXs(l, castConcrete, zp, r.z1, storey, storey + parapetHeight, x1 + parapetThickness, true),
  ])

// A dog-leg's plan in its lower room `r`: the floor landing's edge `zL`
// (the first riser), the half-landing's edge `zH`, the up lane
// [r.x0, ax1] and the back lane [bx0, r.x1] either side of the well, `n`
// risers each, `half` the half-landing's height.
type Leg = { r: Rect, zL: float, zH: float, ax1: float, bx0: float, n: float, rise: float, half: float }
let legOf = (l: Layout, f: Flight, well: float, landing: float): Leg =>
  let r = inner(l, roomAt(l, f.lower).rect) in
  let mid = (r.x0 + r.x1) / 2.0 in
  let n = f.risers / 2.0 in
  { r: r, zL: r.z0 + landing, zH: r.z0 + landing + (n - 1.0) * f.tread, ax1: mid - well / 2.0, bx0: mid + well / 2.0,
    n: n, rise: riserOf(l, f), half: n * riserOf(l, f) }
// A dog-leg step's underside is this far below its tread, a storey under
// the flight above's: a stepped soffit, level with the landings' undersides.
let soffitDepth = (l: Layout, f: Flight): float => riserOf(l, f) + slabDepth
// Whether a dog-leg stands on its floor (no flight comes up under it), and
// whether it is the top one (its landing closes the up lane).
let isSolid = (l: Layout, f: Flight): bool => not isStairHead(l, f.lower)
let isTop = (l: Layout, f: Flight): bool => not isStairFoot(l, f.upper)

// A dog-leg's steps, each lane with a stepped parapet (1 m above each tread)
// on its well side and a stepped soffit, or solid to the floor; the
// half-landing; and the next floor's landing, with a balustrade across the
// up lane and the well when it is the top. Faces a middle storey's room
// draws never depend on whether a flight continues above.
let dogLegFaces = (l: Layout, f: Flight, well: float, landing: float): List<Face> =>
  let g = legOf(l, f, well, landing) in
  let r = g.r in
  let t = f.tread in
  let pt = parapetThickness in
  let ph = parapetHeight in
  let storey = l.storey in
  let solid = isSolid(l, f) in
  // Never below its own floor, so a storey's room holds all of its flight
  // (a typical floor's bake has no flight below to lend those faces).
  let under = (y: float): float => if solid then 0.0 else Math.max(0.0, y - soffitDepth(l, f)) in
  // A step in the soffit, if there is one.
  let soffitStep = (x0: float, x1: float, y0: float, y1: float, z: float, plus: bool): List<Face> =>
    if y1 - y0 > eps then wallZs(l, castConcrete, x0, x1, y0, y1, z, plus) else [] in
  let steps = List.range(g.n - 1.0) in
  let up = steps |> List.concatMap((j) =>
    let za = g.zL + j * t in
    let zb = za + t in
    let y = (j + 1.0) * g.rise in
    let last = j == g.n - 2.0 in
    let u = under(y) in
    List.flatten([
      [wallZ(castConcrete, r.x0, g.ax1 - pt, y - g.rise, y, za, false),
       floorFace(terrazzo, r.x0, za, g.ax1 - pt, zb, y),
       floorFace(castConcrete, g.ax1 - pt, za, g.ax1, zb, y + ph)],
      if solid then [] else List.flatten([
        [ceilingFace(castConcrete, r.x0, za, g.ax1, zb, u)],
        soffitStep(r.x0, g.ax1, u, if last then g.half - slabDepth else under(y + g.rise), zb, true)]),
      wallXs(l, castConcrete, za, zb, y, y + ph, g.ax1 - pt, false),
      wallXs(l, castConcrete, za, zb, u, y + ph, g.ax1, true),
      if last then wallZs(l, castConcrete, g.ax1 - pt, g.ax1, g.half, y + ph, zb, true)
      else wallZs(l, castConcrete, g.ax1 - pt, g.ax1, y + ph, y + ph + g.rise, zb, false),
    ])) in
  let back = steps |> List.concatMap((j) =>
    let zb = g.zH - j * t in
    let za = zb - t in
    let y = g.half + (j + 1.0) * g.rise in
    let last = j == g.n - 2.0 in
    let u = under(y) in
    List.flatten([
      [wallZ(castConcrete, g.bx0 + pt, r.x1, y - g.rise, y, zb, true),
       floorFace(terrazzo, g.bx0 + pt, za, r.x1, zb, y),
       floorFace(castConcrete, g.bx0, za, g.bx0 + pt, zb, y + ph)],
      if solid then [] else List.flatten([
        [ceilingFace(castConcrete, g.bx0, za, r.x1, zb, u)],
        soffitStep(g.bx0, r.x1, u, if last then storey - slabDepth else under(y + g.rise), za, false)]),
      wallXs(l, castConcrete, za, zb, y, y + ph, g.bx0 + pt, true),
      wallXs(l, castConcrete, za, zb, u, y + ph, g.bx0, false),
      if last then wallZs(l, castConcrete, g.bx0, g.bx0 + pt, storey, y + ph, za, false)
      else wallZs(l, castConcrete, g.bx0, g.bx0 + pt, y + ph, y + ph + g.rise, za, true),
    ])) in
  List.flatten([
    // Each lane's first and last risers, and its parapet's ends.
    [wallZ(castConcrete, r.x0, g.ax1 - pt, g.half - g.rise, g.half, g.zH, false),
     wallZ(castConcrete, g.bx0 + pt, r.x1, storey - g.rise, storey, g.zL, true)],
    wallZs(l, castConcrete, g.ax1 - pt, g.ax1, under(g.rise), g.rise + ph, g.zL, false),
    wallZs(l, castConcrete, g.bx0, g.bx0 + pt, g.half, g.half + g.rise + ph, g.zH, true),
    up,
    back,
    // The half-landing, its edge over the well; solid, the back lane's end.
    [floorFace(terrazzo, r.x0, g.zH, r.x1, r.z1, g.half),
     wallZ(castConcrete, g.ax1, g.bx0, if solid then 0.0 else g.half - slabDepth, g.half, g.zH, false)],
    if solid then wallZs(l, castConcrete, g.bx0, r.x1, 0.0, storey - slabDepth, g.zL, false)
    else [ceilingFace(castConcrete, r.x0, g.zH, r.x1, r.z1, g.half - slabDepth)],
    // The next floor's landing, its underside, and its edge over the up
    // lane (under the next flight's first steps) and the well.
    [floorFace(terrazzo, r.x0, r.z0, r.x1, g.zL, storey),
     ceilingFace(castConcrete, r.x0, r.z0, r.x1, g.zL, storey - slabDepth),
     wallZ(castConcrete, r.x0, g.bx0, storey - slabDepth, storey, g.zL, true)],
    if isTop(l, f) then
      [wallZ(castConcrete, r.x0, g.bx0, storey, storey + ph, g.zL, false),
       floorFace(castConcrete, r.x0, g.zL, g.bx0, g.zL + pt, storey + ph),
       wallZ(castConcrete, r.x0, g.bx0, storey, storey + ph, g.zL + pt, true)]
    else [],
  ])

// A flight's faces, from the floor up, in the building's space.
let flightFaces = (l: Layout, f: Flight): List<Face> =>
  (match f.kind with
    | Straight => straightFaces(l, f)
    | DogLeg(well, landing) => dogLegFaces(l, f, well, landing))
    |> List.map((face) => lift(baseOf(l, roomAt(l, f.lower)), face))

// The level whose storey holds height `y`.
let levelAtY = (l: Layout, y: float): float => Math.floor((y + eps) / l.storey)
// Stair room `i`'s share of the stairs in its stairwell (the rooms over its
// rect): every face whose lowest corner is in its storey. A foot gets its
// flight below the upper floor, a head the rest; a room between two flights
// gets both shares, and a flight's soffit under the floor its floor.
let stairFaces = (l: Layout, i: float): List<Face> =>
  let room = roomAt(l, i) in
  if room.kind != Stair && room.kind != StairHall then []
  else l.flights |> List.concatMap((f) =>
    if roomAt(l, f.lower).rect != room.rect then []
    else flightFaces(l, f) |> List.filter((face) => levelAtY(l, lowestY(face)) == room.level))

// ---- Furniture -----------------------------------------------------------------

// Room `i`'s furniture (furniture.fun), its faces from its floor up.
let furnitureFaces = (l: Layout, i: float): List<Face> =>
  let y = baseOf(l, roomAt(l, i)) in
  let r = inner(l, roomAt(l, i).rect) in
  Furniture.inRoom(l, i) |> List.concatMap((f) => Furniture.facesOf(r, f)) |> List.map((s) => lift(y, face(swatchBase + s.swatch, s.o, s.a, s.b)))
// A collider box per piece, in the building's space.
let furnitureBlocks = (l: Layout): List<Block> =>
  l.furniture |> List.indexedMap((k, f) =>
    let b = Furniture.solidOf(inner(l, roomAt(l, f.room).rect), f) in
    let y = baseOf(l, roomAt(l, f.room)) in
    block($"furniture-{k}", b.x0, y + b.y0, b.z0, b.x1, y + b.y1, b.z1))

// ---- The interior mesh -----------------------------------------------------

// Every face in cell `i` (room `i`), sorted by material (one group each):
// the room's own, its halves of its doors and windows, its share of the
// stairs, and its furniture. A roof has none: the street side draws it.
let cellFaces = (l: Layout, i: float): List<Face> =>
  List.flatten([
    roomFaces(l, i),
    l.doors |> List.filter((d) => (d.a == i || d.b == i) && roomAt(l, i).kind != Roof) |> List.concatMap((d) => doorHalf(l, d, i)),
    windowHalves(l, i),
    stairFaces(l, i),
    furnitureFaces(l, i),
  ]) |> List.sortBy((f) => f.mat)

let xyz = (p: Vec3.t): List<float> => [Vec3.x(p), Vec3.y(p), Vec3.z(p)]
let positionsOf = (faces: List<Face>): Bytes.t =>
  Bytes.fromF32le(faces |> List.concatMap((f) => corners(f.q) |> List.concatMap(xyz)))
// Two triangles per quad, over `n` quads' corners.
let quadTriangles = (n: float): Bytes.t =>
  Bytes.fromU32le(List.range(n) |> List.concatMap((k) => [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i)))
// Texture UVs: world-anchored repeats along the face's axes, so tiles line
// up across neighbouring faces; tile 0 stretches the texture over the face.
let uvsOf = (p: List<Material>, f: Face): List<float> =>
  let t = materialIn(p, f.mat).tile in
  if f.mat >= swatchBase then
    (let r = Furniture.swatchRect(f.mat - swatchBase) in [r.u0, r.v1, r.u1, r.v1, r.u1, r.v0, r.u0, r.v0])
  else if t == 0.0 then [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
  else
    let au = Vec3.normalize(f.q.a) in
    let bu = Vec3.normalize(f.q.b) in
    corners(f.q) |> List.concatMap((p) => [Vec3.dot(au, p) / t, 0.0 - Vec3.dot(bu, p) / t])

// Each cell's triangle range, in cell order.
type Range = { first: float, count: float }
let rangesOf = (cellParts: List<List<Face>>): List<Range> =>
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
    let n = 2.0 * List.length(faces |> List.filter((f) => groupMat(f) == m)) in
    if n == 0.0 then acc
    else (out |> List.append([{ mat: m, first: first, count: n }]), first + n), ([], base)) in
  spans
// The material a face draws in: a furniture face's swatch is the furnish
// material's.
let groupMat = (f: Face): float => if f.mat >= swatchBase then furnish else f.mat
let groupOf = (p: List<Material>, s: Span): Model.group =>
  { first: s.first, count: s.count, texture: Option.Some(materialIn(p, s.mat).texture), unlit: materialIn(p, s.mat).unlit, emissive: 0.0 }

// The mesh of every cell's faces, in cell order: each cell is a contiguous
// triangle range.
let meshOf = (p: List<Material>, allFaces: List<Face>, spans: List<Span>): Model.staticData => {
  positions: positionsOf(allFaces),
  normals: Bytes.fromF32le(allFaces |> List.concatMap((f) =>
    let n = xyz(normalOf(f.q)) in List.flatten([n, n, n, n]))),
  uvs: Bytes.fromF32le(allFaces |> List.concatMap((f) => uvsOf(p, f))),
  uvs2: Bytes.fromF32le([]),
  triangles: quadTriangles(List.length(allFaces)),
  groups: spans |> List.map((s) => groupOf(p, s)),
  lightmap: Option.None
}

// ---- Collision ---------------------------------------------------------------

// Solid boxes along each side of every room, between wall centre lines, with
// the door holes left open. Facade walls are boxes, not trimesh faces: a
// face only blocks from its front, so an inward-facing wall would let a
// capsule walk in from outside. Shared walls get a box from each room.
// Walls are a storey high, so the plenum leaves no gap; the top storey's
// facade rises to the roofline.
type Block = { tag: string, x: float, y: float, z: float, w: float, h: float, d: float }

// A box over [x0, x1] × [y0, y1] × [z0, z1].
let block = (tag: string, x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Block =>
  { tag: tag, x: (x0 + x1) / 2.0, y: (y0 + y1) / 2.0, z: (z0 + z1) / 2.0, w: x1 - x0, h: y1 - y0, d: z1 - z0 }

let wallBlocks = (l: Layout, i: float): List<Block> =>
  let room = roomAt(l, i) in
  let r = room.rect in
  let y = baseOf(l, room) in
  let side = (s: Side, s0: float, s1: float) =>
    let line = lineOf(s, r) in
    let t = halfWall(l, s, r) in
    let top = if onFootprint(l, s, r) then facadeTop(l, room.level) else l.storey in
    pieces(s0, s1, top, holesIn(l, i, s)) |> List.indexedMap((k, p) =>
      let tag = $"wall-{i}-{s}-{k}" in
      if s == North || s == South then block(tag, p.u0, y + p.y0, line - t, p.u1, y + p.y1, line + t)
      else block(tag, line - t, y + p.y0, p.u0, line + t, y + p.y1, p.u1)) in
  if not isInside(room) then []
  else List.flatten([side(South, r.x0, r.x1), side(North, r.x0, r.x1), side(West, r.z0, r.z1), side(East, r.z0, r.z1)])
// A flight's parapet, from the floor to its top, so nothing walks under or
// off the steps; the solid under its landing; the landing's parapet. A
// dog-leg's lane parapets are a storey high (stacked, they wall off the
// well), and solid, it has boxes under its back lane and half-landing.
let flightBlocks = (l: Layout, k: float, f: Flight): List<Block> =>
  let y = baseOf(l, roomAt(l, f.lower)) in
  match f.kind with
  | Straight =>
    let r = inner(l, roomAt(l, f.lower).rect) in
    let x1 = flightEnd(f) in
    let zp = f.z1 - parapetThickness in
    [block($"parapet-{k}", f.x0, y, zp, x1, y + l.storey + parapetHeight, f.z1),
     block($"under-landing-{k}", x1, y, zp, r.x1, y + l.storey - slabDepth, f.z1),
     block($"landing-parapet-{k}", x1, y + l.storey - slabDepth, zp, x1 + parapetThickness, y + l.storey + parapetHeight, r.z1)]
  | DogLeg(well, landing) =>
    let g = legOf(l, f, well, landing) in
    let pt = parapetThickness in
    List.flatten([
      [block($"parapet-up-{k}", g.ax1 - pt, y, g.zL, g.ax1, y + l.storey, g.zH),
       block($"parapet-back-{k}", g.bx0, y, g.zL, g.bx0 + pt, y + l.storey, g.zH)],
      if isSolid(l, f) then
        [block($"under-back-{k}", g.bx0, y, g.zL, g.r.x1, y + g.half, g.zH),
         block($"under-half-{k}", g.r.x0, y, g.zH, g.r.x1, y + g.half - slabDepth, g.r.z1)]
      else [],
    ])
// The top dog-leg's balustrade across its up lane and well, on the upper
// floor (its upper room's colliders).
let balustradeBlocks = (l: Layout, k: float, f: Flight): List<Block> =>
  match f.kind with
  | Straight => []
  | DogLeg(well, landing) =>
    if not isTop(l, f) then []
    else
      let g = legOf(l, f, well, landing) in
      let y = baseOf(l, roomAt(l, f.upper)) in
      [block($"balustrade-{k}", g.r.x0, y, g.zL, g.bx0, y + parapetHeight, g.zL + parapetThickness)]

let blocksOf = (l: Layout): List<Block> => List.flatten([
  List.range(List.length(l.rooms)) |> List.concatMap((i) => wallBlocks(l, i)),
  l.flights |> List.indexedMap((k, f) => List.append(balustradeBlocks(l, k, f), flightBlocks(l, k, f))) |> List.flatten,
  furnitureBlocks(l),
])

// Interior floors, door thresholds and landings, from the faces the mesh
// draws: up-facing floor surfaces level with a storey's floor (not treads).
let isFloorLevel = (l: Layout, y: float): bool => levelsOf(l) |> List.any((level) => near(y, level * l.storey))
let floorFacesOf = (l: Layout, allFaces: List<Face>): List<Face> => allFaces |> List.filter((f) =>
  (f.mat == carpet || f.mat == terrazzo || f.mat == mustard) && Vec3.y(normalOf(f.q)) > 0.5 && isFloorLevel(l, Vec3.y(f.q.o)))

// A flight's ramp: one smooth slope through its steps' nosings, from a
// tread before the first riser up to the landing's edge, so the character
// walks it instead of hopping each step. It faces up (trimesh faces block
// only from the front). `rampY` is its height at `x`, from the lower floor.
let rampY = (l: Layout, f: Flight, x: float): float =>
  Math.clamp(0.0, l.storey, (x - (f.x0 - f.tread)) * riserOf(l, f) / f.tread)
// A dog-leg's: each lane's slope, a tread before its first riser to its
// landing, and the half-landing between them.
let rampFaces = (l: Layout, f: Flight): List<Face> =>
  let y = baseOf(l, roomAt(l, f.lower)) in
  match f.kind with
  | Straight =>
    let start = f.x0 - f.tread in
    let end = flightEnd(f) in
    let zp = f.z1 - parapetThickness in
    [face(carpet, v(start, y + rampY(l, f, start), zp),
      v(end - start, rampY(l, f, end) - rampY(l, f, start), 0.0), v(0.0, 0.0, f.z0 - zp))]
  | DogLeg(well, landing) =>
    let g = legOf(l, f, well, landing) in
    let pt = parapetThickness in
    // Up-facing, rising from (za, ya) to (zb, yb) or falling, za < zb.
    let slope = (x0: float, x1: float, za: float, ya: float, zb: float, yb: float): Face =>
      face(carpet, v(x0, y + yb, zb), v(x1 - x0, 0.0, 0.0), v(0.0, ya - yb, za - zb)) in
    [slope(g.r.x0, g.ax1 - pt, g.zL - f.tread, 0.0, g.zH, g.half),
     floorFace(carpet, g.r.x0, g.zH, g.r.x1, g.r.z1, y + g.half),
     slope(g.bx0 + pt, g.r.x1, g.zL, l.storey, g.zH + f.tread, g.half)]
let rampShape = (l: Layout, f: Flight): Physics.shape =>
  let faces = rampFaces(l, f) in
  Physics.trimesh({ positions: positionsOf(faces), triangles: quadTriangles(List.length(faces)) })

// Every collider, in the building's space, for the caller to place as a
// fixed body: the wall boxes, the floors and each flight's ramp.
type Collider = { tag: string, shape: Physics.shape, at: Vec3.t }
let collidersOf = (l: Layout, blocks: List<Block>, floorFaces: List<Face>): List<Collider> =>
  let origin = v(0.0, 0.0, 0.0) in
  List.flatten([
    blocks |> List.map((b) => { tag: b.tag, shape: Physics.box(b.w, b.h, b.d), at: v(b.x, b.y, b.z) }),
    [{ tag: "floors", shape: Physics.trimesh({ positions: positionsOf(floorFaces), triangles: quadTriangles(List.length(floorFaces)) }), at: origin }],
    l.flights |> List.indexedMap((k, f) => { tag: $"ramp-{k}", shape: rampShape(l, f), at: origin }),
  ])

// ---- The bake job --------------------------------------------------------------

// Fluorescent panels and sodium lamps: in the bake jobs ONLY. Submitting
// them to the frame would spend the 8 forward-light slots on light the
// lightmap already holds.
// Each kind's colour, intensity and range (debris.fun reads them too).
type Glow = { r: float, g: float, b: float, intensity: float, range: float }
let panelGlow = { r: 0.93, g: 0.97, b: 0.86, intensity: 0.9, range: 5.5 }
let lampGlow = { r: 1.0, g: 0.62, b: 0.3, intensity: 0.5, range: 7.0 }
let pointOf = (at: Vec3.t, g: Glow): Light.t => Light.point(at, Color.rgb(g.r, g.g, g.b), g.intensity, g.range) |> Light.castShadows
// Just under its panel.
let panelAt = (l: Layout, base: float, p: Panel): Vec3.t => v(p.x, base + l.ceiling - 0.12, p.z)
// 0.3 m out from its lens, a little below it.
let lampAt = (l: Layout, lamp: Lamp): Vec3.t =>
  let lens = lampFace(l, lamp) in
  let centre = lens.q.o |> Vec3.add(Vec3.scale(0.5, lens.q.a)) |> Vec3.add(Vec3.scale(0.5, lens.q.b)) in
  centre |> Vec3.add(Vec3.scale(0.3, normalOf(lens.q))) |> Vec3.add(v(0.0, baseOf(l, roomAt(l, lamp.room)) - 0.15, 0.0))
let panelLight = (l: Layout, base: float, p: Panel): Light.t => pointOf(panelAt(l, base, p), panelGlow)
let lampLight = (l: Layout, lamp: Lamp): Light.t => pointOf(lampAt(l, lamp), lampGlow)

// The lightmap stores light ÷ `bakeScale`; the game loads both bakes with it.
let bakeScale = 2.0
let nightLights = (l: Layout): List<Light.t> => List.flatten([
  [Light.ambient(Color.rgb(0.035, 0.04, 0.035))],
  l.rooms |> List.concatMap((r) => r.panels |> List.filter((p) => p.on) |> List.map((p) => panelLight(l, baseOf(l, r), p))),
  l.lamps |> List.map((lamp) => lampLight(l, lamp)),
])

// ---- Daylight: the day bake only -----------------------------------------------

// A point `u` along footprint side `s`, `y` up, `out` metres outside the
// wall's centre line.
let outsideFacade = (l: Layout, s: Side, u: float, y: float, out: float): Vec3.t =>
  let f = l.footprint in
  match s with
  | North => v(u, y, f.z1 + out)
  | South => v(u, y, f.z0 - out)
  | East => v(f.x1 + out, y, u)
  | West => v(f.x0 - out, y, u)

// An overcast day: no sun, so the bake holds at any yaw of the building
// (every daylight below is placed relative to its opening). Each facade
// opening sees a patch of sky above its head and of sky-lit ground below,
// each a grid of shadowed spots about 0.5 m apart, 1 m wider than the
// opening each side, aimed into the room. The facade's own walls shadow
// them, so the sky lights a soft, window-shaped patch on the floor and lower
// walls inside, and the ground the ceiling by the opening. Ambient can't
// stand in: its occlusion reaches only 1 m, so it would light windowless
// rooms too.
// A patch `out` m outside the wall's centre line, `height` m tall from the
// opening's head (sky) or the street (ground), aimed at the floor or the
// ceiling 2 m inside (`aimCeiling`).
type Patch = { out: float, aboveHead: bool, height: float, aimCeiling: bool, color: Color.t, perSquareMetre: float }
let skyPatch = { out: 1.0, aboveHead: true, height: 2.0, aimCeiling: false, color: Color.rgb(0.72, 0.84, 1.0), perSquareMetre: 0.6 }
// The ground reflects a quarter of the sky's light per square metre, in the
// sky's colour (grey pavement under an overcast sky).
let groundAlbedo = 0.25
let groundPatch = { out: 2.0, aboveHead: false, height: 0.5, aimCeiling: true, color: skyPatch.color, perSquareMetre: groundAlbedo * skyPatch.perSquareMetre }

// `p`'s spots for opening `h` in side `s`, on the storey whose floor is
// `base` up. The ground is always the street.
let patchLights = (l: Layout, s: Side, base: float, h: Hole, p: Patch): List<Light.t> =>
  let y0 = if p.aboveHead then base + h.top else 0.0 in
  let lo = h.lo - 1.0 in
  let width = h.hi - h.lo + 2.0 in
  let nu = Math.ceil(width / 0.5) in
  let nv = Math.ceil(p.height / 0.5) in
  let du = width / nu in
  let dv = p.height / nv in
  let target = outsideFacade(l, s, (h.lo + h.hi) / 2.0, base + (if p.aimCeiling then l.ceiling else 0.0), -2.0) in
  List.range(nu * nv) |> List.map((k) =>
    let at = outsideFacade(l, s, lo + (Math.mod(k, nu) + 0.5) * du, y0 + (Math.floor(k / nu) + 0.5) * dv, p.out) in
    Light.spot(at, Vec3.normalize(target |> Vec3.sub(at)), p.color, p.perSquareMetre * du * dv, 12.0, 80deg)
      |> Light.castShadows)
// Openings stacked over each other on `s` see one patch of ground, so they
// share its light: each emits its share.
let stacked = (l: Layout, s: Side, h: Hole): float =>
  levelsOf(l) |> List.map((level) => openingsIn(l, level, s) |> List.filter((o) => near(o.lo, h.lo) && near(o.hi, h.hi)) |> List.length)
    |> List.sum
let openingLights = (l: Layout, s: Side, base: float, h: Hole): List<Light.t> =>
  let ground = { groundPatch with perSquareMetre: groundPatch.perSquareMetre / stacked(l, s, h) } in
  List.flatten([patchLights(l, s, base, h, skyPatch), patchLights(l, s, base, h, ground)])

// Over each skylight's slot, a patch of sky 1 m wider each side, 1 m above
// the ceiling, aimed straight down.
let skylightLights = (l: Layout): List<Light.t> =>
  l.fixtures |> List.filter((f) => f.sign == Skylight) |> List.concatMap((f) =>
    let s = slotOf(l, f) in
    let y = baseOf(l, roomAt(l, f.room)) + l.ceiling + 1.0 in
    let nu = Math.ceil((s.x1 - s.x0 + 2.0) / 0.5) in
    let nv = Math.ceil((s.z1 - s.z0 + 2.0) / 0.5) in
    let du = (s.x1 - s.x0 + 2.0) / nu in
    let dv = (s.z1 - s.z0 + 2.0) / nv in
    List.range(nu * nv) |> List.map((k) =>
      Light.spot(v(s.x0 - 1.0 + (Math.mod(k, nu) + 0.5) * du, y, s.z0 - 1.0 + (Math.floor(k / nu) + 0.5) * dv),
        v(0.0, -1.0, 0.0), skyPatch.color, skyPatch.perSquareMetre * du * dv, 12.0, 80deg) |> Light.castShadows))

let daylightLights = (l: Layout): List<Light.t> => List.flatten([
  levelsOf(l) |> List.concatMap((level) => [North, South, East, West] |> List.concatMap((s) =>
    openingsIn(l, level, s) |> List.concatMap((h) => openingLights(l, s, level * l.storey, h)))),
  skylightLights(l),
])

// ---- One archetype's interior ------------------------------------------------

// Everything above for one layout in one palette, derived once
// (archetypes.fun holds one per archetype): the cells' faces and the mesh,
// the colliders, and the `night` and `day` bake jobs, named `<name>-night`
// and `<name>-day`. Day: the same mesh, so the same charts; the panels stay
// on.
type Interior = {
  palette: List<Material>,
  cellParts: List<List<Face>>,
  allFaces: List<Face>,
  cellRanges: List<Range>,
  meshSpans: List<Span>,
  interiorData: Model.staticData,
  blocks: List<Block>,
  colliders: List<Collider>,
  night: Lightmap.bakeJob,
  day: Lightmap.bakeJob,
}
let interiorOf = (name: string, l: Layout): Interior => interiorIn(name, l, materials)
let interiorIn = (name: string, l: Layout, palette: List<Material>): Interior =>
  let cellParts = List.range(List.length(l.rooms)) |> List.map((i) => cellFaces(l, i)) in
  let allFaces = List.flatten(cellParts) in
  let cellRanges = rangesOf(cellParts) in
  let meshSpans = cellParts |> List.zip(cellRanges) |> List.concatMap((pair) =>
    let (faces, range) = pair in spansOf(range.first, faces)) in
  let interiorData = meshOf(palette, allFaces, meshSpans) in
  let blocks = blocksOf(l) in
  let night: Lightmap.bakeJob = {
    name: $"{name}-night",
    mesh: interiorData,
    lights: nightLights(l),
    albedo: meshSpans |> List.map((s) => materialIn(palette, s.mat).albedo),
    scale: bakeScale,
  } in
  { palette: palette, cellParts: cellParts, allFaces: allFaces, cellRanges: cellRanges, meshSpans: meshSpans,
    interiorData: interiorData, blocks: blocks,
    colliders: collidersOf(l, blocks, floorFacesOf(l, allFaces)),
    night: night,
    day: { night with name: $"{name}-day", lights: List.flatten([night.lights, daylightLights(l)]) } }

// ---- One mesh per cell -------------------------------------------------------

// The bake keeps triangle order, numbers its vertices by first use, and no
// two faces share a vertex; so each cell's vertices are one run, from the
// first index of its first triangle up to the next cell's. A cell's mesh
// is that run of every vertex array (UV2 byte for byte) and its triangles
// less the run's start. The unbaked mesh is laid out the same way.
type Run = { cell: float, first: float, count: float, v0: float, v1: float }
let runsOf = (k: Interior, data: Model.staticData): List<Run> =>
  let cells = k.cellRanges
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
let cellData = (k: Interior, data: Model.staticData, run: Run): Model.staticData =>
  { data with
    positions: sliceRun(12.0, run, data.positions),
    normals: sliceRun(12.0, run, data.normals),
    uvs: sliceRun(8.0, run, data.uvs),
    uvs2: sliceRun(8.0, run, data.uvs2),
    triangles: Bytes.fromU32le(List.range(3.0 * run.count) |> List.map((j) =>
      (Bytes.u32le(4.0 * (3.0 * run.first + j), data.triangles) |> Binary.require) - run.v0)),
    groups: spansOf(0.0, List.nth(run.cell, k.cellParts) |> Option.defaultValue([])) |> List.map((s) => groupOf(k.palette, s)) }

// A static model per non-empty cell, drawn `Scene.inCells` it.
type CellModel = { cell: float, model: Model.t }
let cellModels = (k: Interior, data: Model.staticData): List<CellModel> =>
  runsOf(k, data) |> List.map((run) => { cell: run.cell, model: Model.static(cellData(k, data, run)) })
