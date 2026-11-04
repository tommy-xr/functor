// The cinema's interior from cinemaLayout.fun: its palette (paints.fun),
// every cell's faces, and the night and day bake jobs' lights. The rooms'
// walls, door reveals and daylight are building.fun's; the soffits,
// columns, light boxes, counter and the auditoria's rake are the cinema's
// own. The seats aren't baked (cinema.fun instances them). Editing this file
// makes the cinema's committed bake stale: rebake (README).

open Layout

let v = Vec3.make
let l = CinemaLayout.layout

// ---- The palette ---------------------------------------------------------------

let paint = Building.paint
let image = Building.image
let shade = Building.shade
let speckled = Building.speckled
let solid = Building.solid

let carpetBase = paint(70.0, 34.0, 104.0)
let tealPaint = paint(38.0, 124.0, 128.0)
let orangePaint = paint(190.0, 70.0, 46.0)
let wallPaint = paint(88.0, 52.0, 116.0)
let blackPaint = paint(14.0, 12.0, 16.0)
let fabricPaint = paint(58.0, 40.0, 76.0)
let tilePaint = paint(226.0, 228.0, 222.0)
let ceilingPaint = paint(200.0, 202.0, 186.0)
let pinkPaint = paint(255.0, 70.0, 190.0)
let cyanPaint = paint(60.0, 240.0, 255.0)
let lensPaint = paint(255.0, 236.0, 200.0)
let creamPaint = paint(240.0, 226.0, 196.0)

// Purple pile with teal and orange-red waves across it, 64 texels a repeat.
let carpetImage = image(64.0, 64.0, (x, y) =>
  let w = Math.mod(y + 5.0 * Math.sin(x * Math.pi / 16.0) + 64.0, 16.0) in
  let base = if w < 2.0 then tealPaint else if w >= 8.0 && w < 9.0 then orangePaint else carpetBase in
  shade(1.0 + 0.3 * (Building.noise(x + 64.0 * y) - 0.5), base))
// Upholstery: faint vertical ribs.
let fabricImage = image(16.0, 16.0, (x, y) =>
  shade((if Math.mod(x, 4.0) == 0.0 then 0.82 else 1.0) + 0.08 * (Building.noise(x + 16.0 * y) - 0.5), fabricPaint))
// 15 cm white tiles: four to a repeat, grey grout.
let tileImage = image(32.0, 32.0, (x, y) =>
  if Math.mod(x, 8.0) == 0.0 || Math.mod(y, 8.0) == 0.0 then paint(170.0, 172.0, 168.0)
  else shade(1.0 + 0.04 * (Building.noise(x + 32.0 * y) - 0.5), tilePaint))

// ---- The glow atlas: every unlit sign, poster, lens and tube ------------------------

// A 3 × 5 digit, rows top first.
let glyphs = [
  [0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0],
  [1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0],
  [1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0],
  [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
]
// Whether digit `n` (1–4) covers texel (x, y), drawn 3× from (x0, y0).
let inGlyph = (n: float, x0: float, y0: float, x: float, y: float): bool =>
  let gx = Math.floor((x - x0) / 3.0) in
  let gy = Math.floor((y - y0) / 3.0) in
  gx >= 0.0 && gx < 3.0 && gy >= 0.0 && gy < 5.0
    && (List.nth(n - 1.0, glyphs) |> Option.map((g) => List.nth(gy * 3.0 + gx, g) == Option.Some(1.0)) |> Option.defaultValue(false))
// A screen number, the sign's shape (0.7 × 0.45 m): a warm digit on black,
// framed in cyan.
let digitImage = (n: float): Texture.pixels => image(32.0, 21.0, (x, y) =>
  if x < 1.0 || x > 30.0 || y < 1.0 || y > 19.0 then cyanPaint
  else if inGlyph(n, 11.0, 3.0, x, y) then lensPaint else blackPaint)
// "Text": light bars `h` texels tall on `rows` lines from `y0`, each line's
// length from the noise.
let textBar = (seed: float, x0: float, x1: float, y0: float, x: float, y: float): bool =>
  let line = Math.floor((y - y0) / 3.0) in
  let len = x0 + (x1 - x0) * (0.45 + 0.5 * Building.noise(seed + line * 7.0)) in
  y >= y0 && Math.mod(y - y0, 3.0) < 1.5 && x >= x0 && x < len
// The menu: three columns of dim lines under orange headings, cyan prices.
let menuImage = image(64.0, 24.0, (x, y) =>
  let col = Math.floor(x / 21.333) in
  let cx = x - col * 21.333 in
  if y < 1.0 || y > 22.0 || x < 1.0 || x > 62.0 then blackPaint
  else if y >= 3.0 && y < 5.0 && cx >= 2.0 && cx < 13.0 then orangePaint
  else if cx >= 17.0 && cx < 19.0 && y >= 8.0 && Math.mod(y - 8.0, 3.0) < 1.5 then cyanPaint
  else if textBar(col * 31.0, 2.0, 15.0, 8.0, cx, y) && y < 21.0 then creamPaint else blackPaint)
// Four posters, 32 × 48, framed: no titles but light bars.
let posterImage = (n: float): Texture.pixels => image(32.0, 48.0, (x, y) =>
  let u = x / 32.0 in
  let t = y / 48.0 in
  let title = y >= 37.0 && y < 45.0 && textBar(n * 13.0, 5.0, 27.0, 37.0, x, y) in
  let field =
    if n == 0.0 then
      (if (x - 16.0) * (x - 16.0) + (y - 18.0) * (y - 18.0) < 49.0 then paint(255.0, 230.0, 190.0)
       else Building.paint(120.0 + 120.0 * t, 40.0 + 30.0 * t, 140.0 - 90.0 * t))
    else if n == 1.0 then
      (if y > 14.0 + 2.0 * Math.abs(x - 16.0) && y < 34.0 then pinkPaint else shade(1.0 - 0.4 * t, tealPaint))
    else if n == 2.0 then
      (if Math.mod(x, 6.0) < 2.0 && y > 8.0 + 14.0 * Building.noise(Math.floor(x / 6.0)) && y < 34.0 then cyanPaint else paint(20.0, 16.0, 40.0))
    else
      (if (x - 16.0) * (x - 16.0) + (y - 12.0) * (y - 12.0) < 12.0 || (Math.abs(x - 16.0) < 4.0 && y > 15.0 && y < 34.0) then paint(30.0, 20.0, 34.0)
       else shade(1.0 - 0.3 * u, orangePaint)) in
  if x < 1.0 || x > 30.0 || y < 1.0 || y > 46.0 then blackPaint
  else if y >= 35.0 then (if title then creamPaint else blackPaint)
  else field)
// A lens: warm white in a dark rim. A fluorescent panel, as the office's.
let lensImage = image(8.0, 8.0, (x, y) => if x < 1.0 || x > 6.0 || y < 1.0 || y > 6.0 then paint(90.0, 80.0, 70.0) else lensPaint)
let panelImage = image(8.0, 16.0, (x, y) =>
  if x < 1.0 || x > 6.0 || y < 1.0 || y > 14.0 then paint(58.0, 72.0, 62.0)
  else if x == 3.0 || x == 4.0 then shade(0.93, paint(240.0, 246.0, 216.0)) else paint(240.0, 246.0, 216.0))
// The screen: dark, a hair lighter than black.
let screenImage = image(8.0, 8.0, (x, y) => paint(34.0, 36.0, 44.0))
let exitImage = image(24.0, 12.0, (x, y) =>
  if x < 1.0 || x > 22.0 || y < 1.0 || y > 10.0 then paint(30.0, 60.0, 40.0)
  else if y >= 4.0 && y < 8.0 && Math.mod(x, 5.0) < 3.0 && x > 3.0 && x < 20.0 then paint(220.0, 255.0, 230.0)
  else paint(70.0, 205.0, 125.0))
// The way to the screens: an arrow and four dots, on black.
let arrowImage = image(32.0, 16.0, (x, y) =>
  let head = x >= 18.0 && x < 26.0 && Math.abs(y - 8.0) < 26.0 - x in
  let shaft = x >= 6.0 && x < 18.0 && y >= 7.0 && y < 9.0 in
  let dots = y >= 12.0 && y < 14.0 && Math.mod(x, 6.0) < 2.0 && x >= 4.0 && x < 28.0 in
  if head || shaft then lensPaint else if dots then cyanPaint else blackPaint)

// The atlas's images, in paint order (`glowBase` on).
let glowImages = List.flatten([
  [solid(pinkPaint), solid(cyanPaint), lensImage, panelImage, screenImage, menuImage],
  List.range(4.0) |> List.map(posterImage),
  [1.0, 2.0, 3.0, 4.0] |> List.map(digitImage),
  [exitImage, arrowImage],
])
let glowAtlas = Texture.atlas({ images: glowImages |> List.map((p) => { pixels: p, layers: [] }), padding: 2.0, maxSize: 256.0 })
let toyImages = [paint(200.0, 40.0, 40.0), paint(230.0, 190.0, 40.0), paint(40.0, 90.0, 200.0), paint(50.0, 160.0, 70.0)]
  |> List.map((p) => image(4.0, 4.0, (x, y) => p))
let toyAtlas = Texture.atlas({ images: toyImages |> List.map((p) => { pixels: p, layers: [] }), padding: 2.0, maxSize: 64.0 })

let rgb = (p: Building.Paint): Color.t => Building.rgb(p.r, p.g, p.b)
let look = (p: Building.Paint, unlit: bool, pixels: Texture.pixels): Paints.Look =>
  { texture: Texture.fromPixels(pixels), unlit: unlit, albedo: rgb(p) }
// The furniture's swatches (furniture.fun): the office's, upholstered teal.
let furnishSwatches = Building.officeSwatches |> List.indexedMap((k, p) => if k == Furniture.fabric then paint(40.0, 112.0, 118.0) else p)
let looks: List<Paints.Look> = [
  look(carpetBase, false, carpetImage),
  look(wallPaint, false, speckled(0.06, wallPaint)),
  // The exposed ceilings and everything black: unlit, out of the bake.
  look(blackPaint, true, solid(blackPaint)),
  look(tealPaint, false, speckled(0.08, tealPaint)),
  look(orangePaint, false, speckled(0.08, orangePaint)),
  look(fabricPaint, false, fabricImage),
  look(tilePaint, false, tileImage),
  look(ceilingPaint, false, Building.tiled(32.0, 1.0, shade(0.62, ceilingPaint), ceilingPaint)),
  { texture: glowAtlas.texture, unlit: true, albedo: rgb(paint(60.0, 50.0, 60.0)) },
  { texture: toyAtlas.texture, unlit: false, albedo: rgb(paint(150.0, 110.0, 80.0)) },
  { texture: Texture.fromPixels(Building.furnishImage(furnishSwatches)), unlit: false, albedo: rgb(Building.swatchAverage(furnishSwatches)) },
]
let glowLook = 8.0
let toyLook = 9.0
let furnishLook = 10.0

let plain = (lk: float, tile: float): Paints.Paint => { look: lk, tile: tile, cell: Option.None }
let cellOf = (lk: float, rects: List<Texture.rect>, i: float): Paints.Paint => { look: lk, tile: 0.0, cell: List.nth(i, rects) }

// Paint ids: the plain ones, then the glow atlas's cells, the toys', the
// furniture's swatches.
let carpet = 0.0
let wall = 1.0
let black = 2.0
let teal = 3.0
let orange = 4.0
let fabric = 5.0
let tile = 6.0
let ceilingTile = 7.0
let glowBase = 8.0
let pink = glowBase
let cyan = glowBase + 1.0
let lens = glowBase + 2.0
let panel = glowBase + 3.0
let screen = glowBase + 4.0
let menu = glowBase + 5.0
let posterPaint = (n: float): float => glowBase + 6.0 + n
let digitPaint = (n: float): float => glowBase + 9.0 + n
let exit = glowBase + 14.0
let arrow = glowBase + 15.0
let toyBase = glowBase + 16.0
let toyPaint = (c: float): float => toyBase + c
let furnishBase = toyBase + List.length(toyImages)

let palette: Paints.Palette = { looks: looks, paints: List.flatten([
  [plain(0.0, 2.0), plain(1.0, 1.0), plain(2.0, 1.0), plain(3.0, 1.0), plain(4.0, 1.0), plain(5.0, 1.0),
   plain(6.0, 0.6), plain(7.0, 0.6)],
  List.range(List.length(glowImages)) |> List.map((i) => cellOf(glowLook, glowAtlas.rects, i)),
  List.range(List.length(toyImages)) |> List.map((i) => cellOf(toyLook, toyAtlas.rects, i)),
  List.range(Furniture.swatches) |> List.map((s) => { look: furnishLook, tile: 0.0, cell: Option.Some(Furniture.swatchRect(s)) }),
]) }

// ---- Faces --------------------------------------------------------------------------

let face = Building.face
let wallZ = Building.wallZ
let wallX = Building.wallX
let floorFace = Building.floorFace
let ceilingFace = Building.ceilingFace

let heightOf = (i: float): float => List.nth(i, CinemaLayout.heights) |> Option.defaultValue(0.0)
let innerOf = (i: float): Rect => Building.inner(l, Building.roomAt(l, i).rect)
let isService = (i: float): bool => i == CinemaLayout.serviceN || i == CinemaLayout.serviceW
let isAuditorium = (i: float): bool => i == CinemaLayout.east || i == CinemaLayout.west
let floorPaint = (i: float): float => if isService(i) then tile else carpet
let jambPaint = (i: float): float => if isService(i) then tile else black

// A face over [u0, u1] × [y0, y1] on side `s` of inner rect `r`, `out`
// metres into the room, facing in.
let onSide = (r: Rect, s: Side, mat: float, u0: float, u1: float, y0: float, y1: float, out: float): Building.Face =>
  match s with
  | South => wallZ(mat, u0, u1, y0, y1, r.z0 + out, true)
  | North => wallZ(mat, u0, u1, y0, y1, r.z1 - out, false)
  | West => wallX(mat, u0, u1, y0, y1, r.x0 + out, true)
  | East => wallX(mat, u0, u1, y0, y1, r.x1 - out, false)

// [x0, x1] × [y0, y1] × [z0, z1] (exterior.fun's, which the bake doesn't hash).
type Box = { x0: float, y0: float, z0: float, x1: float, y1: float, z1: float }
let box = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Box =>
  { x0: x0, y0: y0, z0: z0, x1: x1, y1: y1, z1: z1 }

// The outward faces of box `b`, each with its paint; -1 leaves one out.
type Sides = { top: float, bottom: float, n: float, s: float, e: float, w: float }
let boxFaces = (b: Box, p: Sides): List<Building.Face> =>
  [(p.top, floorFace(p.top, b.x0, b.z0, b.x1, b.z1, b.y1)),
   (p.bottom, ceilingFace(p.bottom, b.x0, b.z0, b.x1, b.z1, b.y0)),
   (p.n, wallZ(p.n, b.x0, b.x1, b.y0, b.y1, b.z1, true)),
   (p.s, wallZ(p.s, b.x0, b.x1, b.y0, b.y1, b.z0, false)),
   (p.e, wallX(p.e, b.z0, b.z1, b.y0, b.y1, b.x1, true)),
   (p.w, wallX(p.w, b.z0, b.z1, b.y0, b.y1, b.x0, false))]
    |> List.filter((e) => let (m, f) = e in m >= 0.0) |> List.map((e) => let (m, f) = e in f)
let allSides = (m: float): Sides => { top: m, bottom: m, n: m, s: m, e: m, w: m }

// A box on side `s`'s inner face of `r`, over [u0, u1] × [y0, y1], `depth`
// out: its front `front`, its edges `edge`; no back, nor bottom on the floor.
let wallBox = (r: Rect, s: Side, u0: float, u1: float, y0: float, y1: float, depth: float, front: float, edge: float): List<Building.Face> =>
  let e = { allSides(edge) with bottom: if y0 > Building.eps then edge else -1.0 } in
  match s with
  | South => boxFaces(box(u0, y0, r.z0, u1, y1, r.z0 + depth), { e with n: front, s: -1.0 })
  | North => boxFaces(box(u0, y0, r.z1 - depth, u1, y1, r.z1), { e with s: front, n: -1.0 })
  | West => boxFaces(box(r.x0, y0, u0, r.x0 + depth, y1, u1), { e with e: front, w: -1.0 })
  | East => boxFaces(box(r.x1 - depth, y0, u0, r.x1, y1, u1), { e with w: front, e: -1.0 })

// Room `i`'s inner walls round its openings, `lo` below `dado` and `hi`
// above, up to its height from `floor(side, u0, u1)`: the floor under each
// stretch of wall, as (u0, u1, y) runs.
let wallsOf = (i: float, dado: float, lo: float, hi: float, floor: (Side, float, float) => List<(float, float, float)>): List<Building.Face> =>
  let r = innerOf(i) in
  let h = heightOf(i) in
  let run = (s: Side, s0: float, s1: float) =>
    Building.pieces(s0, s1, h, Building.holesIn(l, i, s)) |> List.concatMap((p) =>
      floor(s, p.u0, p.u1) |> List.concatMap((st) =>
        let (u0, u1, y) = st in
        let y0 = Math.max(p.y0, y) in
        List.flatten([
          if Math.min(p.y1, dado) - y0 > Building.eps then [onSide(r, s, lo, u0, u1, y0, Math.min(p.y1, dado), 0.0)] else [],
          if p.y1 - Math.max(y0, dado) > Building.eps then [onSide(r, s, hi, u0, u1, Math.max(y0, dado), p.y1, 0.0)] else [],
        ]))) in
  List.flatten([run(South, r.x0, r.x1), run(North, r.x0, r.x1), run(West, r.z0, r.z1), run(East, r.z0, r.z1)])
let flatFloor = (s: Side, u0: float, u1: float): List<(float, float, float)> => [(u0, u1, 0.0)]

// Room `i`'s half of door `d`'s reveal, as building.fun's `doorHalf`: from
// the wall's centre line to its face on `i`'s side, so each half is in its
// own room's cell. An outside room's half takes the inside room's paints.
let doorHalf = (d: Door, i: float): List<Building.Face> =>
  let ra = Building.roomAt(l, d.a).rect in
  match Building.sideBetween(ra, Building.roomAt(l, d.b).rect) with
  | Option.None => []
  | Option.Some(side) =>
    let line = Building.lineOf(side, ra) in
    let t = Building.halfWall(l, side, ra) in
    let h = Building.holeOf(d) in
    let inside = if Building.isInside(Building.roomAt(l, i)) then i else if i == d.a then d.b else d.a in
    let below = (i == d.a) == (side == North || side == East) in
    let head = if isService(inside) && h.top >= heightOf(inside) - Building.eps then ceilingTile else jambPaint(inside) in
    if below then Building.revealFaces(side, line - t, line, h, jambPaint(inside), head, floorPaint(inside))
    else Building.revealFaces(side, line, line + t, h, jambPaint(inside), head, floorPaint(inside))

// ---- Fittings ---------------------------------------------------------------------

// A poster light box: 1 × 1.5 m, 8 cm deep, black edges.
let posterFaces = (p: CinemaLayout.Poster): List<Building.Face> =>
  wallBox(innerOf(p.room), p.side, p.at - 0.5, p.at + 0.5, 1.1, 2.6, 0.08, posterPaint(p.n), black)
// A screen number over its door, and a shut door's plate under it.
let screenFaces = (s: CinemaLayout.Screen): List<Building.Face> =>
  let r = innerOf(s.room) in
  List.flatten([
    wallBox(r, s.side, s.at - 0.35, s.at + 0.35, 2.6, 3.05, 0.06, digitPaint(s.n), black),
    if s.open then [] else wallBox(r, s.side, s.at - 0.8, s.at + 0.8, 0.0, 2.4, 0.04, black, black),
  ])
// A column, `columnDepth` proud and as tall as its room, a neon tube up
// each front edge.
let columnFaces = (c: CinemaLayout.Column): List<Building.Face> =>
  let r = innerOf(c.room) in
  let w = CinemaLayout.columnWidth / 2.0 in
  let d = CinemaLayout.columnDepth in
  let tube = if c.pink then pink else cyan in
  List.flatten([
    wallBox(r, c.side, c.at - w, c.at + w, 0.0, heightOf(c.room), d, orange, orange)
      |> List.filter((f) => Math.abs(Vec3.y(Building.normalOf(f.q))) < 0.5),
    [onSide(r, c.side, tube, c.at - w + 0.03, c.at - w + 0.08, 0.3, 3.0, d + 0.01),
     onSide(r, c.side, tube, c.at + w - 0.08, c.at + w - 0.03, 0.3, 3.0, d + 0.01)],
  ])

// A corridor's soffits: `n` segments down its long axis, each a box hung
// on the wave, teal underneath and at its ends, orange-red along its sides;
// a lens in every third one's underside.
type Soffit = { box: Box, lit: bool }
let soffitsOf = (i: float): List<Soffit> =>
  let r = innerOf(i) in
  let m = CinemaLayout.soffitMargin in
  let alongX = r.x1 - r.x0 > r.z1 - r.z0 in
  let (a0, a1) = if alongX then (r.x0, r.x1) else (r.z0, r.z1) in
  let n = Math.round((a1 - a0) / CinemaLayout.soffitStep) in
  let step = (a1 - a0) / n in
  List.range(n) |> List.map((k) =>
    let y0 = CinemaLayout.soffitWave(k) in
    let y1 = y0 + CinemaLayout.soffitDepth in
    let s0 = a0 + k * step in
    { box: if alongX then box(s0, y0, r.z0 + m, s0 + step, y1, r.z1 - m) else box(r.x0 + m, y0, s0, r.x1 - m, y1, s0 + step),
      lit: Math.mod(k, 3.0) == 1.0 })
let soffitFaces = (i: float): List<Building.Face> =>
  let r = innerOf(i) in
  let alongX = r.x1 - r.x0 > r.z1 - r.z0 in
  soffitsOf(i) |> List.concatMap((s) =>
    let b = s.box in
    let ends = if alongX then { allSides(teal) with top: -1.0, n: orange, s: orange } else { allSides(teal) with top: -1.0, e: orange, w: orange } in
    List.flatten([
      boxFaces(b, ends),
      if s.lit then [ceilingFace(lens, (b.x0 + b.x1) / 2.0 - 0.15, (b.z0 + b.z1) / 2.0 - 0.15, (b.x0 + b.x1) / 2.0 + 0.15, (b.z0 + b.z1) / 2.0 + 0.15, b.y0 - 0.005)] else [],
    ]))

// The lobby's soffit ring: four bands `ringWidth` wide round its walls,
// their undersides at the dado, their inner faces up `ringDepth`, a pink
// tube along the inner edge; a lens every 3 m round its middle.
let ringInner = (r: Rect): Rect =>
  let w = CinemaLayout.ringWidth in rect(r.x0 + w, r.z0 + w, r.x1 - w, r.z1 - w)
let ringLenses = (r: Rect): List<Vec3.t> =>
  let w = CinemaLayout.ringWidth / 2.0 in
  let y = CinemaLayout.lobbyDado - 0.005 in
  let run = (a: float, b: float): List<float> =>
    let n = Math.round((b - a) / 3.0) in List.range(n) |> List.map((k) => a + (k + 0.5) * (b - a) / n) in
  List.flatten([
    run(r.x0, r.x1) |> List.concatMap((x) => [v(x, y, r.z0 + w), v(x, y, r.z1 - w)]),
    run(r.z0 + 2.0 * w, r.z1 - 2.0 * w) |> List.concatMap((z) => [v(r.x0 + w, y, z), v(r.x1 - w, y, z)]),
  ])
let ringFaces = (r: Rect): List<Building.Face> =>
  let y0 = CinemaLayout.lobbyDado in
  let y1 = y0 + CinemaLayout.ringDepth in
  let c = ringInner(r) in
  List.flatten([
    [ceilingFace(teal, r.x0, r.z0, r.x1, c.z0, y0), ceilingFace(teal, r.x0, c.z1, r.x1, r.z1, y0),
     ceilingFace(teal, r.x0, c.z0, c.x0, c.z1, y0), ceilingFace(teal, c.x1, c.z0, r.x1, c.z1, y0),
     wallZ(teal, c.x0, c.x1, y0, y1, c.z0, true), wallZ(teal, c.x0, c.x1, y0, y1, c.z1, false),
     wallX(teal, c.z0, c.z1, y0, y1, c.x0, true), wallX(teal, c.z0, c.z1, y0, y1, c.x1, false),
     wallZ(pink, c.x0, c.x1, y0, y0 + 0.07, c.z0 + 0.01, true), wallZ(pink, c.x0, c.x1, y0, y0 + 0.07, c.z1 - 0.01, false),
     wallX(pink, c.z0, c.z1, y0, y0 + 0.07, c.x0 + 0.01, true), wallX(pink, c.z0, c.z1, y0, y0 + 0.07, c.x1 - 0.01, false)],
    ringLenses(r) |> List.map((p) => ceilingFace(lens, Vec3.x(p) - 0.15, Vec3.z(p) - 0.15, Vec3.x(p) + 0.15, Vec3.z(p) + 0.15, Vec3.y(p))),
  ])

// The concession counter and the ticket desk: orange-red boxes, black
// tops, a neon line along each front.
let counterFaces = (b: Rect, tube: float): List<Building.Face> =>
  let h = CinemaLayout.counterHeight in
  List.flatten([
    boxFaces(box(b.x0, 0.0, b.z0, b.x1, h, b.z1), { allSides(orange) with top: black, bottom: -1.0 }),
    [wallZ(tube, b.x0 + 0.1, b.x1 - 0.1, h - 0.12, h - 0.07, b.z0 - 0.01, false)],
  ])
// The menu: three 5 m light boxes on the wall behind the counter, from these x.
let menuXs = [20.4, 25.6, 30.8]
let menuFaces = (r: Rect): List<Building.Face> =>
  menuXs |> List.concatMap((x) => wallBox(r, North, x, x + 5.0, 2.3, 3.7, 0.15, menu, black))

// ---- Rooms ------------------------------------------------------------------------

// A grid of downlights in the lobby's black ceiling, inside the ring.
let lobbyDownlights = (r: Rect): List<Vec3.t> =>
  let c = ringInner(r) in
  List.range(10.0) |> List.map((k) =>
    let col = Math.mod(k, 5.0) in
    let row = Math.floor(k / 5.0) in
    v(c.x0 + (c.x1 - c.x0) * (col + 0.5) / 5.0, heightOf(CinemaLayout.lobby) - 0.005, c.z0 + (c.z1 - c.z0) * (row + 0.5) / 2.0))

let lobbyFaces = (): List<Building.Face> =>
  let i = CinemaLayout.lobby in
  let r = innerOf(i) in
  let h = heightOf(i) in
  List.flatten([
    [floorFace(carpet, r.x0, r.z0, r.x1, r.z1, 0.0), ceilingFace(black, r.x0, r.z0, r.x1, r.z1, h)],
    wallsOf(i, CinemaLayout.lobbyDado, wall, black, flatFloor),
    ringFaces(r),
    lobbyDownlights(r) |> List.map((p) => ceilingFace(lens, Vec3.x(p) - 0.25, Vec3.z(p) - 0.25, Vec3.x(p) + 0.25, Vec3.z(p) + 0.25, Vec3.y(p))),
    counterFaces(CinemaLayout.counter, cyan),
    counterFaces(CinemaLayout.desk, pink),
    menuFaces(r),
    // The way to the screens, over the arch.
    wallBox(r, East, 12.4, 13.6, 3.4, 3.9, 0.05, arrow, black),
  ])

let corridorFaces = (i: float): List<Building.Face> =>
  let r = innerOf(i) in
  List.flatten([
    [floorFace(carpet, r.x0, r.z0, r.x1, r.z1, 0.0), ceilingFace(black, r.x0, r.z0, r.x1, r.z1, heightOf(i))],
    wallsOf(i, CinemaLayout.corridorDado, wall, black, flatFloor),
    soffitFaces(i),
    CinemaLayout.columns |> List.filter((c) => c.room == i) |> List.concatMap(columnFaces),
    CinemaLayout.screens |> List.filter((s) => s.room == i) |> List.concatMap(screenFaces),
  ])

// The rake: rows rising from `rakeFrom` in from the screen wall; the floor
// under a stretch of wall is the highest tread beside it.
let rakeStart = (r: Rect): float => r.z0 + CinemaLayout.rakeFrom
let rakeTop = CinemaLayout.rows * CinemaLayout.rise
let rowZ = (r: Rect, k: float): float => rakeStart(r) + k * CinemaLayout.rowDepth
let treadY = (k: float): float => (k + 1.0) * CinemaLayout.rise
// The floor under a stretch of auditorium wall: the screen wall's is flat,
// the back wall stands on the top row, a side wall splits at the rows.
let rakeFloor = (r: Rect, s: Side, u0: float, u1: float): List<(float, float, float)> =>
  if s == South then [(u0, u1, 0.0)]
  else if s == North then [(u0, u1, rakeTop)]
  else
  let cuts = List.range(CinemaLayout.rows) |> List.map((k) => rowZ(r, k)) |> List.filter((z) => z > u0 + Building.eps && z < u1 - Building.eps) in
  let edges = List.flatten([[u0], cuts, [u1]]) in
  List.take(List.length(edges) - 1.0, edges) |> List.zip(List.drop(1.0, edges)) |> List.map((e) =>
    let (a, b) = e in
    let k = Math.floor(((a + b) / 2.0 - rakeStart(r)) / CinemaLayout.rowDepth) in
    (a, b, if k < 0.0 then 0.0 else treadY(Math.min(k, CinemaLayout.rows - 1.0))))

let screenCentre = (r: Rect): float => (r.x0 + r.x1) / 2.0
let sconceZs = [24.0, 28.0, 32.0, 36.0]
let downlights = (r: Rect): List<Vec3.t> =>
  List.range(12.0) |> List.map((k) =>
    let col = Math.mod(k, 3.0) in
    let row = Math.floor(k / 3.0) in
    v(r.x0 + (r.x1 - r.x0) * (col + 0.5) / 3.0, heightOf(CinemaLayout.east) - 0.005, r.z0 + 3.0 + row * 4.4))

let auditoriumFaces = (i: float): List<Building.Face> =>
  let r = innerOf(i) in
  let h = heightOf(i) in
  let z0 = rakeStart(r) in
  let last = CinemaLayout.rows - 1.0 in
  let c = screenCentre(r) in
  let w = CinemaLayout.screenWidth / 2.0 in
  let entrance = l.doors |> List.find((d) => d.a == CinemaLayout.corridorB && d.b == i) in
  List.flatten([
    [floorFace(carpet, r.x0, r.z0, r.x1, z0, 0.0), ceilingFace(black, r.x0, r.z0, r.x1, r.z1, h)],
    List.range(CinemaLayout.rows) |> List.concatMap((k) =>
      let za = rowZ(r, k) in
      [floorFace(carpet, r.x0, za, r.x1, if k == last then r.z1 else za + CinemaLayout.rowDepth, treadY(k)),
       wallZ(black, r.x0, r.x1, treadY(k) - CinemaLayout.rise, treadY(k), za, false)]),
    wallsOf(i, h, fabric, fabric, (s, u0, u1) => rakeFloor(r, s, u0, u1)),
    [onSide(r, South, screen, c - w, c + w, CinemaLayout.screenBottom, CinemaLayout.screenTop, 0.02)],
    sconceZs |> List.concatMap((z) => List.flatten([
      wallBox(r, West, z - 0.12, z + 0.12, 4.9, 5.4, 0.06, lens, black),
      wallBox(r, East, z - 0.12, z + 0.12, 4.9, 5.4, 0.06, lens, black)])),
    downlights(r) |> List.map((p) => ceilingFace(lens, Vec3.x(p) - 0.2, Vec3.z(p) - 0.2, Vec3.x(p) + 0.2, Vec3.z(p) + 0.2, Vec3.y(p))),
    entrance |> Option.map((d) => wallBox(r, South, d.at - 0.25, d.at + 0.25, 2.55, 2.8, 0.04, exit, black)) |> Option.defaultValue([]),
    l.doors |> List.filter((d) => d.a == CinemaLayout.serviceN && d.b == i)
      |> List.concatMap((d) => wallBox(r, West, d.at - 0.25, d.at + 0.25, 2.3, 2.55, 0.04, exit, black)),
    if i == CinemaLayout.west then playsetFaces() else [],
  ])

// The playset's boxes (no undersides) and its slide, both sides.
let playsetFaces = (): List<Building.Face> =>
  let s = CinemaLayout.slide in
  let top = (t: CinemaLayout.Toy): Box => box(t.x0, t.y0, t.z0, t.x1, t.y1, t.z1) in
  List.flatten([
    CinemaLayout.playset |> List.concatMap((t) => boxFaces(top(t), { allSides(toyPaint(t.c)) with bottom: if t.y0 > 0.0 then toyPaint(t.c) else -1.0 })),
    [face(toyPaint(2.0), v(s.x0, s.top, s.z1), v(s.x1 - s.x0, 0.05 - s.top, 0.0), v(0.0, 0.0, s.z0 - s.z1)),
     face(toyPaint(2.0), v(s.x0, s.top - 0.05, s.z0), v(s.x1 - s.x0, 0.05 - s.top, 0.0), v(0.0, 0.0, s.z1 - s.z0))],
  ])

let serviceFaces = (i: float): List<Building.Face> =>
  let r = innerOf(i) in
  let h = heightOf(i) in
  List.flatten([
    [floorFace(tile, r.x0, r.z0, r.x1, r.z1, 0.0), ceilingFace(ceilingTile, r.x0, r.z0, r.x1, r.z1, h)],
    wallsOf(i, h, tile, tile, flatFloor),
    servicePanels(i) |> List.map((p) => ceilingFace(panel, Vec3.x(p) - 0.3, Vec3.z(p) - 0.6, Vec3.x(p) + 0.3, Vec3.z(p) + 0.6, h - 0.01)),
    l.doors |> List.filter((d) => d.b == i && Building.roomAt(l, d.a).kind == Outside)
      |> List.concatMap((d) => wallBox(r, West, d.at - 0.25, d.at + 0.25, 2.3, 2.55, 0.04, exit, black)),
  ])
// Fluorescent panels every 3 m down a service corridor's middle.
let servicePanels = (i: float): List<Vec3.t> =>
  let r = innerOf(i) in
  let alongX = r.x1 - r.x0 > r.z1 - r.z0 in
  let (a0, a1) = if alongX then (r.x0, r.x1) else (r.z0, r.z1) in
  let n = Math.floor((a1 - a0) / 3.0) in
  let pad = (a1 - a0 - 3.0 * n) / 2.0 + 1.5 in
  List.range(n) |> List.map((k) =>
    let a = a0 + pad + 3.0 * k in
    if alongX then v(a, heightOf(i), (r.z0 + r.z1) / 2.0) else v((r.x0 + r.x1) / 2.0, heightOf(i), a))

// Cell `i`'s faces: its room's own, its door halves, its posters, its
// furniture.
let cellFaces = (i: float): List<Building.Face> =>
  let room = Building.roomAt(l, i) in
  List.flatten([
    Furniture.inRoom(l, i) |> List.concatMap((f) => Furniture.facesOf(innerOf(i), f)) |> List.map((s) => face(furnishBase + s.swatch, s.o, s.a, s.b)),
    if not Building.isInside(room) then []
    else if i == CinemaLayout.lobby then lobbyFaces()
    else if isAuditorium(i) then auditoriumFaces(i)
    else if isService(i) then serviceFaces(i)
    else corridorFaces(i),
    l.doors |> List.filter((d) => d.a == i || d.b == i) |> List.concatMap((d) => doorHalf(d, i)),
    CinemaLayout.posters |> List.filter((p) => p.room == i) |> List.concatMap(posterFaces),
  ])

// ---- The bake's lights ----------------------------------------------------------------

let warm = Color.rgb(1.0, 0.82, 0.62)
let pinkLight = Color.rgb(1.0, 0.3, 0.75)
let cyanLight = Color.rgb(0.25, 0.9, 1.0)
let down = v(0.0, -1.0, 0.0)
let spot = (at: Vec3.t, color: Color.t, intensity: float, range: float, cone: Angle.t): Light.t =>
  Light.spot(at, down, color, intensity, range, cone) |> Light.castShadows
let point = (at: Vec3.t, color: Color.t, intensity: float, range: float): Light.t =>
  Light.point(at, color, intensity, range) |> Light.castShadows
// A point `out` metres from side `s`'s inner face of `r`, `at` along it.
let offSide = (r: Rect, s: Side, at: float, y: float, out: float): Vec3.t =>
  match s with
  | South => v(at, y, r.z0 + out)
  | North => v(at, y, r.z1 - out)
  | West => v(r.x0 + out, y, at)
  | East => v(r.x1 - out, y, at)

let lobbyLights = (): List<Light.t> =>
  let r = innerOf(CinemaLayout.lobby) in
  let c = ringInner(r) in
  let y = CinemaLayout.lobbyDado - 0.1 in
  List.flatten([
    ringLenses(r) |> List.map((p) => spot(v(Vec3.x(p), y, Vec3.z(p)), warm, 0.55, 7.0, 55deg)),
    // The pink tube round the ring.
    [0.2, 0.5, 0.8] |> List.concatMap((t) =>
      [point(v(c.x0 + (c.x1 - c.x0) * t, y - 0.1, c.z0 + 0.3), pinkLight, 0.3, 6.0),
       point(v(c.x0 + (c.x1 - c.x0) * t, y - 0.1, c.z1 - 0.3), pinkLight, 0.3, 6.0)]),
    [point(v(c.x0 + 0.3, y - 0.1, (c.z0 + c.z1) / 2.0), pinkLight, 0.3, 6.0), point(v(c.x1 - 0.3, y - 0.1, (c.z0 + c.z1) / 2.0), pinkLight, 0.3, 6.0)],
    // The menu's glow, the neon along the counter and desk.
    menuXs |> List.map((x) => point(v(x + 2.5, 3.0, r.z1 - 0.8), warm, 0.45, 6.0)),
    [21.0, 28.0, 35.0] |> List.map((x) => point(v(x, 0.8, CinemaLayout.counter.z0 - 0.6), cyanLight, 0.3, 4.0)),
    [point(v((CinemaLayout.desk.x0 + CinemaLayout.desk.x1) / 2.0, 0.8, CinemaLayout.desk.z0 - 0.6), pinkLight, 0.3, 4.0)],
    // Downlights in the black ceiling over the middle.
    lobbyDownlights(r) |> List.map((p) => spot(v(Vec3.x(p), Vec3.y(p) - 0.1, Vec3.z(p)), warm, 0.8, 9.0, 50deg)),
  ])

let corridorLights = (i: float): List<Light.t> =>
  let r = innerOf(i) in
  List.flatten([
    soffitsOf(i) |> List.filter((s) => s.lit) |> List.map((s) =>
      spot(v((s.box.x0 + s.box.x1) / 2.0, s.box.y0 - 0.1, (s.box.z0 + s.box.z1) / 2.0), warm, 0.5, 6.0, 50deg)),
    CinemaLayout.columns |> List.filter((c) => c.room == i) |> List.map((c) =>
      point(offSide(r, c.side, c.at, 1.6, CinemaLayout.columnDepth + 0.35), if c.pink then pinkLight else cyanLight, 0.3, 4.5)),
  ])

let auditoriumLights = (i: float): List<Light.t> =>
  let r = innerOf(i) in
  List.flatten([
    sconceZs |> List.concatMap((z) => [point(offSide(r, West, z, 5.0, 0.35), warm, 0.6, 9.0), point(offSide(r, East, z, 5.0, 0.35), warm, 0.6, 9.0)]),
    downlights(r) |> List.map((p) => spot(v(Vec3.x(p), Vec3.y(p) - 0.1, Vec3.z(p)), warm, 0.9, 12.0, 45deg)),
  ])

let serviceLights = (i: float): List<Light.t> =>
  servicePanels(i) |> List.map((p) => point(v(Vec3.x(p), Vec3.y(p) - 0.12, Vec3.z(p)), Color.rgb(0.93, 0.97, 0.86), 0.9, 5.5))

let nightLights: List<Light.t> = List.flatten([
  [Light.ambient(Color.rgb(0.03, 0.025, 0.04))],
  lobbyLights(),
  [CinemaLayout.corridorA, CinemaLayout.corridorB] |> List.concatMap(corridorLights),
  CinemaLayout.auditoria |> List.concatMap(auditoriumLights),
  [CinemaLayout.serviceN, CinemaLayout.serviceW] |> List.concatMap(serviceLights),
])
// By day the same, and daylight through the entrance and the rear exit.
let dayLights = List.flatten([nightLights, Building.daylightLights(l)])

// ---- The interior ---------------------------------------------------------------------

let cellParts = List.range(List.length(l.rooms)) |> List.map(cellFaces)
let interior = Paints.interiorOf(palette, "cinema", cellParts, nightLights, dayLights)
let unlit = Paints.unlitModels(palette, cellParts)
