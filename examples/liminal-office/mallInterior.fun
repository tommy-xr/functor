// The mall's baked interior, from mallLayout.fun: every cell's surfaces
// (one mesh, cut into one model per cell at load, as building.fun's), its
// own materials, and the night and day bake jobs. Faces are polygons, not
// only quads: the escalator's sides and the pendants need triangles.
// Editing this file makes the mall's committed bake stale: rebake (README).
//
// The bake runs at the kit's 12 texels/m on the mesh at HALF scale, lights
// and all, so the mall's lightmap is 6 texels/m: a 1024² atlas. Every
// light's falloff is relative to its range, so a half-scale scene lights
// the same; only the bake's ambient occlusion reach doubles, to 2 m.

open Layout
open MallLayout

let eps = Building.eps
let near = Building.near

// ---- Materials -----------------------------------------------------------------

let paint = Building.paint
let tilePaint = paint(196.0, 186.0, 160.0)
let wallPaint = paint(206.0, 192.0, 172.0)
let ceilingPaint = paint(214.0, 214.0, 204.0)
let lensPaint = paint(240.0, 246.0, 216.0)
let trimPaint = paint(72.0, 62.0, 54.0)
let shutterPaint = paint(150.0, 154.0, 152.0)
let pinkPaint = paint(232.0, 118.0, 170.0)
let tealPaint = paint(84.0, 206.0, 192.0)
let waterPaint = paint(26.0, 92.0, 98.0)
let leafPaint = paint(58.0, 92.0, 56.0)
let steelPaint = paint(128.0, 130.0, 132.0)
let enamelPaint = paint(150.0, 192.0, 172.0)
let darkPaint = paint(8.0, 8.0, 10.0)
let drumPaint = paint(255.0, 214.0, 150.0)

// The faces' materials. Each draws in one of the palette's groups (one
// draw per group per cell): the tiled floor, ceiling and treads, and two
// atlases, one lit and one unlit, of colours and textures stretched over a
// face. Few groups keep the court's draws in budget.
let tile = 0.0
let wall = 1.0
let ceilingMat = 2.0
let lens = 3.0
let trim = 4.0
let shutter = 5.0
let signPink = 6.0
let signTeal = 7.0
let water = 8.0
let leaves = 9.0
let tread = 10.0
let enamel = 11.0
let dark = 12.0
let drum = 13.0

// An atlas: 32-texel blocks in a 4 × 4 grid; `block` paints block (bx, by)
// at (x, y) within it, each 0..31.
let atlasOf = (block: (float, float, float, float) => Building.Paint): Texture.pixels =>
  Building.image(128.0, 128.0, (x, y) => block(Math.floor(x / 32.0), Math.floor(y / 32.0), Math.mod(x, 32.0), Math.mod(y, 32.0)))
let litAtlas = atlasOf((bx, by, x, y) =>
  if by == 0.0 && bx == 1.0 then trimPaint
  else if by == 0.0 && bx == 2.0 then enamelPaint
  // A roller shutter's slats, and its bottom bar.
  else if by == 0.0 && bx == 3.0 then (if y > 28.0 then trimPaint else Building.shade(if Math.mod(Math.floor(y / 2.0), 2.0) == 0.0 then 0.82 else 1.0, shutterPaint))
  else if by == 1.0 && bx == 0.0 then Building.shade(1.0 + 0.6 * (Building.noise(x + 32.0 * y) - 0.5), leafPaint)
  else wallPaint)
let glowAtlas = atlasOf((bx, by, x, y) =>
  // A lit lens in a dark frame.
  if by == 0.0 && bx == 0.0 then (if x < 4.0 || x > 27.0 || y < 2.0 || y > 29.0 then trimPaint else lensPaint)
  else if by == 0.0 && bx == 1.0 then pinkPaint
  else if by == 0.0 && bx == 2.0 then tealPaint
  else if by == 0.0 && bx == 3.0 then waterPaint
  else if by == 1.0 && bx == 1.0 then drumPaint
  else darkPaint)

// Groups, in draw order.
let tileGroup = 0.0
let ceilingGroup = 1.0
let treadGroup = 2.0
let litGroup = 3.0
let glowGroup = 4.0
let materials = [
  Building.material(tilePaint, 1.2, false, Building.tiled(32.0, 1.0, Building.shade(0.82, tilePaint), tilePaint)),
  Building.material(ceilingPaint, 1.2, false, Building.tiled(32.0, 1.0, Building.shade(0.7, ceilingPaint), ceilingPaint)),
  // Cleats: grooves 1 cm apart along the treads.
  Building.material(steelPaint, 0.08, false, Building.image(8.0, 1.0, (x, y) => Building.shade(if x < 4.0 then 0.7 else 1.0, steelPaint))),
  Building.material(wallPaint, 0.0, false, litAtlas),
  Building.material(darkPaint, 0.0, true, glowAtlas),
]

// How a material draws: in `group`, its UVs world-anchored repeats every
// `tile` metres, or (tile 0) the atlas rect [u0, u1] × [v0, v1] stretched
// over the face, a point for a plain colour.
type Slot = { group: float, tile: float, u0: float, v0: float, u1: float, v1: float }
type Palette = { groups: List<Building.Material>, slots: List<Slot> }
let tiledSlot = (group: float, t: float): Slot => { group: group, tile: t, u0: 0.0, v0: 0.0, u1: 0.0, v1: 0.0 }
// Block (bx, by): textured, a texel in from its edges; plain, its middle.
let blockSlot = (group: float, bx: float, by: float, textured: bool): Slot =>
  let inset = if textured then 1.0 else 16.0 in
  { group: group, tile: 0.0, u0: (32.0 * bx + inset) / 128.0, v0: (32.0 * by + inset) / 128.0,
    u1: (32.0 * bx + 32.0 - inset) / 128.0, v1: (32.0 * by + 32.0 - inset) / 128.0 }
let palette: Palette = { groups: materials, slots: [
  tiledSlot(tileGroup, 1.2),
  blockSlot(litGroup, 0.0, 0.0, false),
  tiledSlot(ceilingGroup, 1.2),
  blockSlot(glowGroup, 0.0, 0.0, true),
  blockSlot(litGroup, 1.0, 0.0, false),
  blockSlot(litGroup, 3.0, 0.0, true),
  blockSlot(glowGroup, 1.0, 0.0, false),
  blockSlot(glowGroup, 2.0, 0.0, false),
  blockSlot(glowGroup, 3.0, 0.0, false),
  blockSlot(litGroup, 0.0, 1.0, true),
  tiledSlot(treadGroup, 0.08),
  blockSlot(litGroup, 2.0, 0.0, false),
  blockSlot(glowGroup, 0.0, 1.0, false),
  blockSlot(glowGroup, 1.0, 1.0, false),
] }

let materialOf = (mats: List<Building.Material>, i: float): Building.Material =>
  match List.nth(i, mats) with
  | Option.Some(m) => m
  | Option.None => Binary.invalid($"no material {i}")
let slotOf = (pal: Palette, mat: float): Slot =>
  match List.nth(mat, pal.slots) with
  | Option.Some(s) => s
  | Option.None => Binary.invalid($"no material {mat}")
let groupIn = (pal: Palette, mat: float): float => slotOf(pal, mat).group

// ---- Polygons ------------------------------------------------------------------

// A flat convex polygon of 3 or 4 points, counter-clockwise seen from its
// front (the side the bake lights).
type Poly = { mat: float, pts: List<Vec3.t> }
let ofFace = (f: Building.Face): Poly => { mat: f.mat, pts: Building.corners(f.q) }
let quad = (mat: float, o: Vec3.t, a: Vec3.t, b: Vec3.t): Poly => ofFace(Building.face(mat, o, a, b))
let tri = (mat: float, a: Vec3.t, b: Vec3.t, c: Vec3.t): Poly => { mat: mat, pts: [a, b, c] }
let floorAt = (mat: float, x0: float, z0: float, x1: float, z1: float, y: float): Poly => ofFace(Building.floorFace(mat, x0, z0, x1, z1, y))
let ceilingAt = (mat: float, x0: float, z0: float, x1: float, z1: float, y: float): Poly => ofFace(Building.ceilingFace(mat, x0, z0, x1, z1, y))
let wallZ = (mat: float, x0: float, x1: float, y0: float, y1: float, z: float, plus: bool): Poly => ofFace(Building.wallZ(mat, x0, x1, y0, y1, z, plus))
let wallX = (mat: float, z0: float, z1: float, y0: float, y1: float, x: float, plus: bool): Poly => ofFace(Building.wallX(mat, z0, z1, y0, y1, x, plus))
let raise = (dy: float, p: Poly): Poly => { p with pts: p.pts |> List.map((q) => q |> Vec3.add(v(0.0, dy, 0.0))) }
let pt = (i: float, p: Poly): Vec3.t => List.nth(i, p.pts) |> Option.defaultValue(v(0.0, 0.0, 0.0))
let normalOf = (p: Poly): Vec3.t =>
  pt(1.0, p) |> Vec3.sub(pt(0.0, p)) |> Vec3.cross(pt(List.length(p.pts) - 1.0, p) |> Vec3.sub(pt(0.0, p))) |> Vec3.normalize
let trianglesIn = (p: Poly): float => List.length(p.pts) - 2.0

// A box's faces, outward: its four sides, then its top and bottom; `Box3`
// is [x0, x1] × [y0, y1] × [z0, z1].
type Box3 = { x0: float, y0: float, z0: float, x1: float, y1: float, z1: float }
let box3 = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Box3 =>
  { x0: x0, y0: y0, z0: z0, x1: x1, y1: y1, z1: z1 }
let sidesOf = (mat: float, b: Box3): List<Poly> => [
  wallX(mat, b.z0, b.z1, b.y0, b.y1, b.x0, false), wallX(mat, b.z0, b.z1, b.y0, b.y1, b.x1, true),
  wallZ(mat, b.x0, b.x1, b.y0, b.y1, b.z0, false), wallZ(mat, b.x0, b.x1, b.y0, b.y1, b.z1, true)]
let topOf = (mat: float, b: Box3): Poly => floorAt(mat, b.x0, b.z0, b.x1, b.z1, b.y1)
let bottomOf = (mat: float, b: Box3): Poly => ceilingAt(mat, b.x0, b.z0, b.x1, b.z1, b.y0)
// A box standing on something: no bottom.
let standing = (sides: float, top: float, b: Box3): List<Poly> => List.append(sidesOf(sides, b), [topOf(top, b)])

// ---- Cells, walls and openings ---------------------------------------------------

let cellAt = (i: float): Cell =>
  match List.nth(i, cells) with
  | Option.Some(c) => c
  | Option.None => Binary.invalid($"no mall cell {i}")
let rectOf = (i: float): Rect => cellAt(i).rect
let baseOf = (i: float): float => cellAt(i).level * storey
let isInside = (i: float): bool => cellAt(i).kind != Lot

// Cell `i`'s open-plan neighbours, and whether its side `s` is open: the
// void's every side, and a side shared with a neighbour.
let neighbours = (i: float): List<float> =>
  links |> List.concatMap((p) => let (a, b) = p in if a == i then [b] else if b == i then [a] else [])
let isOpen = (i: float, s: Side): bool =>
  cellAt(i).kind == Void || (neighbours(i) |> List.any((o) => Building.sideBetween(rectOf(i), rectOf(o)) == Option.Some(s)))

// Half the wall along side `s` of cell `i`: none where it is open.
let halfWall = (i: float, s: Side): float =>
  if isOpen(i, s) then 0.0 else Building.halfWall(layout, s, rectOf(i))
let inner = (i: float): Rect =>
  let r = rectOf(i) in
  rect(r.x0 + halfWall(i, West), r.z0 + halfWall(i, South), r.x1 - halfWall(i, East), r.z1 - halfWall(i, North))

// The doors in cell `i`'s side `s`, as holes in increasing order.
let doorsOn = (i: float, s: Side): List<Door> =>
  doors |> List.filter((d) =>
    let other = if d.a == i then Option.Some(d.b) else if d.b == i then Option.Some(d.a) else Option.None in
    match other with
    | Option.Some(o) => Building.sideBetween(rectOf(i), rectOf(o)) == Option.Some(s)
    | Option.None => false)
let holesOn = (i: float, s: Side): List<Building.Hole> =>
  doorsOn(i, s) |> List.map(Building.holeOf) |> List.sortBy((h) => h.lo)

// A wall side's run along its inner face: from `s0` to `s1`, at `line`.
type Run = { s0: float, s1: float, line: float }
let runOf = (i: float, s: Side): Run =>
  let r = inner(i) in
  match s with
  | South => { s0: r.x0, s1: r.x1, line: r.z0 }
  | North => { s0: r.x0, s1: r.x1, line: r.z1 }
  | West => { s0: r.z0, s1: r.z1, line: r.x0 }
  | East => { s0: r.z0, s1: r.z1, line: r.x1 }
// A rect on cell `i`'s inner wall `s`, `out` proud of it, facing in: [lo, hi]
// along the wall, [y0, y1] from the floor.
let onWall = (i: float, s: Side, mat: float, lo: float, hi: float, y0: float, y1: float, out: float): Poly =>
  let line = runOf(i, s).line in
  match s with
  | South => wallZ(mat, lo, hi, y0, y1, line + out, true)
  | North => wallZ(mat, lo, hi, y0, y1, line - out, false)
  | West => wallX(mat, lo, hi, y0, y1, line + out, true)
  | East => wallX(mat, lo, hi, y0, y1, line - out, false)

let sides = [South, North, West, East]
// Cell `i`'s inner walls, facing in, round its doors.
let wallsOf = (i: float): List<Poly> =>
  sides |> List.filter((s) => not isOpen(i, s)) |> List.concatMap((s) =>
    let run = runOf(i, s) in
    Building.pieces(run.s0, run.s1, ceiling, holesOn(i, s)) |> List.map((p) => onWall(i, s, wall, p.u0, p.u1, p.y0, p.y1, 0.0)))

// Cell `i`'s half of door `d`'s reveal (building.fun's `doorHalf`).
let doorHalf = (d: Door, i: float): List<Poly> =>
  let ra = rectOf(d.a) in
  match Building.sideBetween(ra, rectOf(d.b)) with
  | Option.None => []
  | Option.Some(side) =>
    let line = Building.lineOf(side, ra) in
    let t = Building.halfWall(layout, side, ra) in
    let h = Building.holeOf(d) in
    // Cell `a` is below the line when the line is its North or East side.
    let below = (i == d.a) == (side == North || side == East) in
    let top = if h.top < ceiling - eps then trim else ceilingMat in
    let half = if below then Building.revealFaces(side, line - t, line, h, trim, top, tile)
      else Building.revealFaces(side, line, line + t, h, trim, top, tile) in
    half |> List.map(ofFace)

// A ceiling panel's lens (building.fun's panel); panels that are off are
// plain ceiling.
let lensOf = (i: float, p: Panel): Poly => ofFace({ Building.panelFace(layout, rectOf(i), p) with mat: lens })

// ---- Shopfronts --------------------------------------------------------------------

// Every 8 m along the concourse's walls, a shopfront: a shutter, or an open
// shop's door. A sign band over each: lit over an open shop and along the
// food court's west wall (its menus), blank over a shutter.
type Front = { cell: float, side: Side, at: float, open: bool }
let frontWidth = 5.6
let isConcourse = (i: float): bool => cellAt(i).kind == Hall || cellAt(i).kind == Strip
let frontsOf = (i: float): List<Front> =>
  if not isConcourse(i) then []
  else sides |> List.filter((s) => not isOpen(i, s) && s != East) |> List.concatMap((s) =>
    let r = rectOf(i) in
    let (s0, s1) = if s == South || s == North then (r.x0, r.x1) else (r.z0, r.z1) in
    let n = Math.floor((s1 - s0) / 8.0) in
    let first = s0 + (s1 - s0 - 8.0 * n) / 2.0 + 4.0 in
    List.range(n) |> List.map((k) => first + 8.0 * k) |> List.concatMap((at) =>
      match doorsOn(i, s) |> List.find((d) => Math.abs(d.at - at) < 4.0) with
      | Option.Some(d) => if cellAt(d.a).kind == Shop || cellAt(d.b).kind == Shop then [{ cell: i, side: s, at: d.at, open: true }] else []
      | Option.None => [{ cell: i, side: s, at: at, open: false }]))
let fronts = List.range(List.length(cells)) |> List.concatMap(frontsOf)
// A lit sign's colour: alternating along the wall.
let signOf = (f: Front): float => if Math.mod(Math.floor(f.at / 8.0), 2.0) == 0.0 then signPink else signTeal
let isLitSign = (f: Front): bool => f.open || (f.cell == westHall && f.side == West)
let signBand = (f: Front): Building.Hole =>
  { lo: f.at - frontWidth / 2.0, hi: f.at + frontWidth / 2.0, bottom: if f.open then 3.35 else 3.25, top: 3.8 }
let frontFaces = (f: Front): List<Poly> =>
  let lo = f.at - frontWidth / 2.0 in
  let hi = f.at + frontWidth / 2.0 in
  let band = signBand(f) in
  List.flatten([
    if f.open then [] else [onWall(f.cell, f.side, shutter, lo, hi, 0.0, 3.0, 0.02)],
    [onWall(f.cell, f.side, if isLitSign(f) then signOf(f) else trim, band.lo, band.hi, band.bottom, band.top, 0.04)],
  ])

// The anchor store: its dark entrance in the lower east hall's end wall,
// a blank sign band over it.
let anchorFaces = (i: float): List<Poly> =>
  if cellAt(i).kind != Hall || cellAt(i).level != 0.0 || isOpen(i, East) || not Building.onFootprint(layout, East, rectOf(i)) then []
  else [onWall(i, East, dark, anchorDoor.lo, anchorDoor.hi, 0.0, anchorDoor.top, 0.02),
        onWall(i, East, trim, anchorDoor.lo - 1.0, anchorDoor.hi + 1.0, anchorDoor.top + 0.15, 3.9, 0.04)]

// ---- Furniture -----------------------------------------------------------------------

// The food court's tables: a top on a pedestal.
let tableTop = 0.76
let tableBox = (t: Spot): Box3 => box3(t.x - 0.5, tableTop - 0.04, t.z - 0.5, t.x + 0.5, tableTop, t.z + 0.5)
let tableFaces = (t: Spot): List<Poly> =>
  let top = tableBox(t) in
  List.flatten([
    sidesOf(trim, top),
    [topOf(enamel, top), bottomOf(trim, top)],
    sidesOf(trim, box3(t.x - 0.06, 0.0, t.z - 0.06, t.x + 0.06, top.y0, t.z + 0.06)),
  ])

// A drum pendant: an octagonal shade with a glowing lens under it, on a rod
// from the ceiling.
let drumRadius = 0.45
let drumY0 = 2.9
let drumY1 = 3.35
let octagon = (c: Spot, y: float): List<Vec3.t> =>
  List.range(8.0) |> List.map((k) =>
    let a = (k + 0.5) * Math.pi / 4.0 in v(c.x + drumRadius * Math.cos(a), y, c.z + drumRadius * Math.sin(a)))
let drumFaces = (c: Spot): List<Poly> =>
  let ring = octagon(c, drumY0) in
  let at = (k: float): Vec3.t => List.nth(Math.mod(k, 8.0), ring) |> Option.defaultValue(v(0.0, 0.0, 0.0)) in
  let up = v(0.0, drumY1 - drumY0, 0.0) in
  List.flatten([
    // Each side from corner k + 1 back to k faces out.
    List.range(8.0) |> List.map((k) => quad(trim, at(k + 1.0), at(k) |> Vec3.sub(at(k + 1.0)), up)),
    List.range(8.0) |> List.map((k) => tri(drum, v(c.x, drumY0, c.z), at(k), at(k + 1.0))),
    sidesOf(trim, box3(c.x - 0.02, drumY1, c.z - 0.02, c.x + 0.02, ceiling, c.z + 0.02)),
  ])

// The open shops: a counter along the back wall, two display plinths.
let shopBoxes = (i: float): List<Box3> =>
  let r = inner(i) in
  let back = if holesOn(i, North) == [] then r.z1 - 1.6 else r.z0 + 1.0 in
  let mid = (r.x0 + r.x1) / 2.0 in
  let midZ = (r.z0 + r.z1) / 2.0 in
  [box3(r.x0 + 1.5, 0.0, back, r.x1 - 1.5, 1.0, back + 0.6),
   box3(mid - 2.3, 0.0, midZ - 0.5, mid - 1.3, 0.6, midZ + 0.5),
   box3(mid + 1.3, 0.0, midZ - 0.5, mid + 2.3, 0.6, midZ + 0.5)]
let shopFaces = (i: float): List<Poly> =>
  shopBoxes(i) |> List.indexedMap((k, b) => if k == 0.0 then standing(trim, enamel, b) else standing(enamel, trim, b)) |> List.flatten

// ---- The galleries -------------------------------------------------------------------

// Round the void on each gallery, an opaque balustrade, `balustrade` thick
// and `railHeight` high, inside its strip; the west bridge's opens where the
// escalators land. A planter against it every `planterXs` on the long sides.
let balustrade = 0.15
let railHeight = 1.1
// The void's side gallery `i` sits on, if it borders the void.
let voidSide = (i: float): Option.t<Side> =>
  if cellAt(i).level != 1.0 || not (neighbours(i) |> List.any((o) => o == courtVoid)) then Option.None
  else Building.sideBetween(rectOf(i), voidRect)
// The balustrade's boxes in gallery `i`, in its local space (floor 0).
let balustradeBoxes = (i: float): List<Box3> =>
  let b = balustrade in
  let h = railHeight in
  let e = escalator in
  let w = voidRect in
  match voidSide(i) with
  | Option.None => []
  | Option.Some(s) => (match s with
    | North => [box3(w.x0 - b, 0.0, w.z0 - b, w.x1 + b, h, w.z0)]
    | South => [box3(w.x0 - b, 0.0, w.z1, w.x1 + b, h, w.z1 + b)]
    | East => [box3(w.x0 - b, 0.0, w.z0, w.x0, h, e.z0), box3(w.x0 - b, 0.0, e.z1, w.x0, h, w.z1)]
    | West => [box3(w.x1, 0.0, w.z0, w.x1 + b, h, w.z1)])
// Its faces: both long faces, the handrail on top, and the ends that stand
// free (the long sides' corners; the escalators' opening).
let balustradeFaces = (i: float): List<Poly> =>
  let s = voidSide(i) |> Option.defaultValue(South) in
  balustradeBoxes(i) |> List.concatMap((bx) =>
    let all = sidesOf(enamel, bx) in
    let long = if s == North || s == South then [2.0, 3.0] else [0.0, 1.0] in
    let ends = if s == North || s == South then [0.0, 1.0] else [2.0, 3.0] in
    let freeEnd = (k: float): bool =>
      if s == North || s == South then true
      else (k == 3.0 && near(bx.z1, escalator.z0)) || (k == 2.0 && near(bx.z0, escalator.z1)) in
    List.flatten([
      long |> List.concatMap((k) => List.nth(k, all) |> Option.toList),
      ends |> List.filter(freeEnd) |> List.concatMap((k) => List.nth(k, all) |> Option.toList),
      [topOf(trim, bx)],
    ]))
let planterBoxes = (i: float): List<Box3> =>
  let w = voidRect in
  let b = balustrade in
  if voidSide(i) == Option.Some(North) then planterXs |> List.map((x) => box3(x - 0.8, 0.0, w.z0 - b - 0.6, x + 0.8, 0.55, w.z0 - b))
  else if voidSide(i) == Option.Some(South) then planterXs |> List.map((x) => box3(x - 0.8, 0.0, w.z1 + b, x + 0.8, 0.55, w.z1 + b + 0.6))
  else []
let bushOf = (b: Box3): Box3 => box3(b.x0 + 0.1, b.y1, b.z0 + 0.05, b.x1 - 0.1, b.y1 + 0.5, b.z1 - 0.05)
let planterFaces = (i: float): List<Poly> =>
  planterBoxes(i) |> List.concatMap((b) => List.append(standing(trim, leaves, b), standing(leaves, leaves, bushOf(b))))

// ---- The void ------------------------------------------------------------------------

// The court's floor; the galleries' slab edges and the roof's fascia round
// it (the west bridge's slab edge stops at the escalators' body); the
// escalators and the fountain.
let fascia = (s: Side, lo: float, hi: float, y0: float, y1: float): Poly =>
  let w = voidRect in
  match s with
  | South => wallZ(wall, lo, hi, y0, y1, w.z0, true)
  | North => wallZ(wall, lo, hi, y0, y1, w.z1, false)
  | West => wallX(wall, lo, hi, y0, y1, w.x0, true)
  | East => wallX(wall, lo, hi, y0, y1, w.x1, false)
let edgesOf = (s: Side, lo: float, hi: float, gaps: List<Building.Hole>): List<Poly> =>
  List.flatten([
    Building.pieces(lo, hi, storey - ceiling, gaps) |> List.map((p) => fascia(s, p.u0, p.u1, ceiling + p.y0, ceiling + p.y1)),
    [fascia(s, lo, hi, storey + ceiling, roof)],
  ])
let voidEdges =
  let w = voidRect in
  let e = escalator in
  List.flatten([
    edgesOf(South, w.x0, w.x1, []), edgesOf(North, w.x0, w.x1, []),
    edgesOf(West, w.z0, w.z1, [{ lo: e.z0, hi: e.z1, bottom: 0.0, top: storey - ceiling }]),
    edgesOf(East, w.z0, w.z1, []),
  ])

// The escalators: 30° from the court's floor up west to the west bridge.
// `rise` and `run` per step; the treads' nosings lie on one 30° line from
// the top plate's edge `xp` down to the floor at `xq`; the bottom plate
// ends at `xEnd`. Lanes between the outer balustrades and the middle one.
type Steps = { rise: float, run: float, xp: float, xq: float, xEnd: float, lanes: List<(float, float)>, rails: List<(float, float)> }
let stepsOf = (e: Escalator): Steps =>
  let rise = storey / e.risers in
  let run = rise * Math.sqrt(3.0) in
  let xp = e.xTop + e.plate in
  let zm = (e.z0 + e.z1) / 2.0 in
  let b = e.balustrade in
  { rise: rise, run: run, xp: xp, xq: xp + e.risers * run, xEnd: xp + (e.risers - 1.0) * run + e.plate,
    lanes: [(e.z0 + b, zm - b / 2.0), (zm + b / 2.0, e.z1 - b)],
    rails: [(e.z0, e.z0 + b), (zm - b / 2.0, zm + b / 2.0), (e.z1 - b, e.z1)] }
// The nosing line's height at `x`.
let nosingY = (g: Steps, x: float): float => Math.clamp(0.0, storey, storey - (x - g.xp) / Math.sqrt(3.0))

// One lane's top plate, treads, risers and bottom plate.
let laneFaces = (e: Escalator, g: Steps, za: float, zb: float): List<Poly> =>
  let n = e.risers in
  List.flatten([
    [floorAt(trim, e.xTop, za, g.xp, zb, storey)],
    List.range(n - 1.0) |> List.map((j) => floorAt(tread, g.xp + j * g.run, za, g.xp + (j + 1.0) * g.run, zb, storey - (j + 1.0) * g.rise)),
    List.range(n) |> List.map((j) => wallX(tread, za, zb, storey - (j + 1.0) * g.rise, storey - j * g.rise, g.xp + j * g.run, true)),
    [floorAt(trim, g.xp + (n - 1.0) * g.run, za, g.xEnd, zb, 0.02)],
  ])
// A balustrade's face in the plane z = `z`, facing +Z (`plus`) or -Z: flat
// at the top, along the nosing line, flat at the foot, `rail` high; with
// `skirt`, the small triangles between each tread and the nosing line.
let railFace = (e: Escalator, g: Steps, z: float, plus: bool, skirt: bool): List<Poly> =>
  let r = e.rail in
  let flip = (p: Poly): Poly => if plus then p else { p with pts: List.reverse(p.pts) } in
  let along = v(g.xq - g.xp, 0.0 - storey, 0.0) in
  List.flatten([
    [wallZ(enamel, e.xTop, g.xp, storey, storey + r, z, plus), wallZ(enamel, g.xq, g.xEnd, 0.0, r, z, plus),
     flip(quad(enamel, v(g.xp, storey, z), along, v(0.0, r, 0.0)))],
    if not skirt then []
    else List.range(e.risers - 1.0) |> List.map((j) =>
      let xa = g.xp + j * g.run in
      let y = storey - (j + 1.0) * g.rise in
      flip(tri(enamel, v(xa, y, z), v(xa + g.run, y, z), v(xa, y + g.rise, z)))),
  ])
// A balustrade's handrail cap and its ends.
let railCap = (e: Escalator, g: Steps, za: float, zb: float): List<Poly> =>
  let r = e.rail in
  [floorAt(trim, e.xTop, za, g.xp, zb, storey + r),
   quad(trim, v(g.xp, storey + r, zb), v(g.xq - g.xp, 0.0 - storey, 0.0), v(0.0, 0.0, za - zb)),
   floorAt(trim, g.xq, za, g.xEnd, zb, r),
   wallX(trim, za, zb, storey, storey + r, e.xTop, false),
   wallX(trim, za, zb, 0.0, r, g.xEnd, true)]
// The body's outer side in the plane z = `z`, under the nosing line down to
// the floor: under the top plate, and the wedge under the steps.
let cladding = (e: Escalator, g: Steps, z: float, plus: bool): List<Poly> =>
  let flip = (p: Poly): Poly => if plus then p else { p with pts: List.reverse(p.pts) } in
  [wallZ(enamel, e.xTop, g.xp, 0.0, storey, z, plus),
   flip(tri(enamel, v(g.xp, 0.0, z), v(g.xq, 0.0, z), v(g.xp, storey, z)))]
// "Out of service": a barrier across lane 2's foot.
let barrierBoxes = (e: Escalator, g: Steps): List<Box3> =>
  match List.nth(1.0, g.lanes) with
  | Option.Some(lane) =>
    let (za, zb) = lane in
    let x = g.xEnd + 0.4 in
    [box3(x - 0.03, 0.0, za - 0.03, x + 0.03, 1.0, za + 0.03), box3(x - 0.03, 0.0, zb - 0.03, x + 0.03, 1.0, zb + 0.03),
     box3(x - 0.02, 0.85, za, x + 0.02, 0.97, zb)]
  | Option.None => []
let escalatorFaces = (e: Escalator): List<Poly> =>
  let g = stepsOf(e) in
  let rails = g.rails in
  List.flatten([
    g.lanes |> List.concatMap((lane) => let (za, zb) = lane in laneFaces(e, g, za, zb)),
    // Outer balustrades: skirts toward the lanes, cladding outside.
    railFace(e, g, e.z0 + e.balustrade, true, true), railFace(e, g, e.z0, false, false), cladding(e, g, e.z0, false),
    railFace(e, g, e.z1 - e.balustrade, false, true), railFace(e, g, e.z1, true, false), cladding(e, g, e.z1, true),
    // The middle balustrade, between the lanes.
    (match List.nth(1.0, rails) with
      | Option.Some(rail) => let (za, zb) = rail in List.append(railFace(e, g, za, false, true), railFace(e, g, zb, true, true))
      | Option.None => []),
    rails |> List.concatMap((rail) => let (za, zb) = rail in railCap(e, g, za, zb)),
    // The body's end under the top plate, facing the west strip.
    [wallX(enamel, e.z0, e.z1, 0.0, storey, e.xTop, false)],
    List.flatten(barrierBoxes(e, g) |> List.indexedMap((k, b) => standing(if k == 2.0 then signPink else trim, if k == 2.0 then signPink else trim, b))),
  ])

// The fountain: a basin with a rim, still water round a planter island.
let rimHeight = 0.45
let rimWidth = 0.3
let waterLevel = 0.3
let fountainFaces =
  let b = basin in
  let w = rimWidth in
  let i = rect(b.x0 + w, b.z0 + w, b.x1 - w, b.z1 - w) in
  let p = island in
  List.flatten([
    sidesOf(tile, box3(b.x0, 0.0, b.z0, b.x1, rimHeight, b.z1)),
    [floorAt(tile, b.x0, b.z0, b.x1, i.z0, rimHeight), floorAt(tile, b.x0, i.z1, b.x1, b.z1, rimHeight),
     floorAt(tile, b.x0, i.z0, i.x0, i.z1, rimHeight), floorAt(tile, i.x1, i.z0, b.x1, i.z1, rimHeight),
     wallZ(tile, i.x0, i.x1, waterLevel, rimHeight, i.z0, true), wallZ(tile, i.x0, i.x1, waterLevel, rimHeight, i.z1, false),
     wallX(tile, i.z0, i.z1, waterLevel, rimHeight, i.x0, true), wallX(tile, i.z0, i.z1, waterLevel, rimHeight, i.x1, false),
     floorAt(water, i.x0, i.z0, i.x1, p.z0, waterLevel), floorAt(water, i.x0, p.z1, i.x1, i.z1, waterLevel),
     floorAt(water, i.x0, p.z0, p.x0, p.z1, waterLevel), floorAt(water, p.x1, p.z0, i.x1, p.z1, waterLevel)],
    standing(tile, leaves, box3(p.x0, waterLevel, p.z0, p.x1, 0.9, p.z1)),
    standing(leaves, leaves, box3(p.x0 + 0.2, 0.9, p.z0 + 0.2, p.x1 - 0.2, 1.7, p.z1 - 0.2)),
  ])

// ---- Every cell's faces ------------------------------------------------------------

// Cell `i`'s faces, from its floor, sorted by group.
let cellFaces = (i: float): List<Poly> =>
  let c = cellAt(i) in
  let r = inner(i) in
  let own = if not isInside(i) then []
    else List.flatten([
      [floorAt(tile, r.x0, r.z0, r.x1, r.z1, 0.0)],
      if c.kind == Void then List.flatten([voidEdges, escalatorFaces(escalator), fountainFaces])
      else List.flatten([
        [ceilingAt(ceilingMat, r.x0, r.z0, r.x1, r.z1, ceiling)],
        wallsOf(i),
        c.panels |> List.filter((p) => p.on) |> List.map((p) => lensOf(i, p)),
        fronts |> List.filter((f) => f.cell == i) |> List.concatMap(frontFaces),
        anchorFaces(i),
        balustradeFaces(i),
        planterFaces(i),
        if c.kind == Shop then shopFaces(i) else [],
        if i == westHall then List.flatten([tables |> List.concatMap(tableFaces), drums |> List.concatMap(drumFaces)]) else [],
      ]),
    ]) in
  List.flatten([
    own,
    doors |> List.filter((d) => d.a == i || d.b == i) |> List.concatMap((d) => doorHalf(d, i)),
  ]) |> List.map((p) => raise(baseOf(i), p)) |> List.sortBy((p) => groupIn(palette, p.mat))

// ---- The mesh ------------------------------------------------------------------------

let xyz = (k: float, p: Vec3.t): List<float> => [k * Vec3.x(p), k * Vec3.y(p), k * Vec3.z(p)]
// World-anchored UVs along the polygon's first edge and across it in its
// plane (a quad's two edges); or its atlas rect, stretched over it.
let stretched = [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
let uvsOf = (pal: Palette, p: Poly): List<float> =>
  let s = slotOf(pal, p.mat) in
  if s.tile == 0.0 then
    List.take(2.0 * List.length(p.pts), stretched) |> List.indexedMap((k, t) =>
      if Math.mod(k, 2.0) == 0.0 then s.u0 + t * (s.u1 - s.u0) else s.v0 + t * (s.v1 - s.v0))
  else
    let au = pt(1.0, p) |> Vec3.sub(pt(0.0, p)) |> Vec3.normalize in
    let bu = normalOf(p) |> Vec3.cross(au) |> Vec3.normalize in
    p.pts |> List.concatMap((q) => [Vec3.dot(au, q) / s.tile, 0.0 - Vec3.dot(bu, q) / s.tile])
// A fan from each polygon's first point, its indices from `base`.
let indicesOf = (polys: List<Poly>): List<float> =>
  let (out, _) = polys |> List.fold((acc, p) =>
    let (out, base) = acc in
    let tris = List.range(trianglesIn(p)) |> List.concatMap((k) => [base, base + k + 1.0, base + k + 2.0]) in
    (List.append(tris, out), base + List.length(p.pts)), ([], 0.0)) in
  out

// Each cell's triangle range, and its spans per material (`first` from `base`).
let rangesOf = (parts: List<List<Poly>>): List<Building.Range> =>
  let (ranges, _) = parts |> List.fold((acc, polys) =>
    let (out, first) = acc in
    let count = polys |> List.map(trianglesIn) |> List.sum in
    (out |> List.append([{ first: first, count: count }]), first + count), ([], 0.0)) in
  ranges
// One span per group present, `first` counted from `base`.
let spansOf = (pal: Palette, base: float, polys: List<Poly>): List<Building.Span> =>
  let (spans, _) = List.range(List.length(pal.groups)) |> List.fold((acc, g) =>
    let (out, first) = acc in
    let n = polys |> List.filter((p) => groupIn(pal, p.mat) == g) |> List.map(trianglesIn) |> List.sum in
    if n == 0.0 then acc else (out |> List.append([{ mat: g, first: first, count: n }]), first + n), ([], base)) in
  spans
let groupOf = (pal: Palette, s: Building.Span): Model.group =>
  { first: s.first, count: s.count, texture: Option.Some(materialOf(pal.groups, s.mat).texture), unlit: materialOf(pal.groups, s.mat).unlit, emissive: 0.0 }

// Polygons, in group order, as static model data, positions scaled by `k`.
let meshOf = (pal: Palette, k: float, polys: List<Poly>, spans: List<Building.Span>): Model.staticData => {
  positions: Bytes.fromF32le(polys |> List.concatMap((p) => p.pts |> List.concatMap((q) => xyz(k, q)))),
  normals: Bytes.fromF32le(polys |> List.concatMap((p) =>
    let n = xyz(1.0, normalOf(p)) in p.pts |> List.concatMap((q) => n))),
  uvs: Bytes.fromF32le(polys |> List.concatMap((p) => uvsOf(pal, p))),
  uvs2: Bytes.fromF32le([]),
  triangles: Bytes.fromU32le(indicesOf(polys)),
  groups: spans |> List.map((s) => groupOf(pal, s)),
  lightmap: Option.None
}

// ---- Lights: the bake jobs only ------------------------------------------------------

// The bake's space: the mall at half scale.
let bakeSpace = 0.5
let pointAt = (p: Vec3.t, color: Color.t, intensity: float, range: float): Light.t =>
  Light.point(Vec3.scale(bakeSpace, p), color, intensity, bakeSpace * range) |> Light.castShadows
let spotAt = (p: Vec3.t, dir: Vec3.t, color: Color.t, intensity: float, range: float): Light.t =>
  Light.spot(Vec3.scale(bakeSpace, p), dir, color, intensity, bakeSpace * range, 80deg) |> Light.castShadows

let panelColor = Color.rgb(0.93, 0.97, 0.86)
let warm = Color.rgb(1.0, 0.72, 0.42)
let pinkGlow = Color.rgb(1.0, 0.45, 0.7)
let tealGlow = Color.rgb(0.35, 0.95, 0.85)
let sky = Color.rgb(0.72, 0.84, 1.0)

// The panels that are on; a warm light under each drum; a glow before each
// lit sign; four floods in the skylight's corners and the fountain's glow.
let nightLights: List<Light.t> =
  let w = voidRect in
  List.flatten([
    [Light.ambient(Color.rgb(0.03, 0.032, 0.036))],
    List.range(List.length(cells)) |> List.concatMap((i) =>
      cellAt(i).panels |> List.filter((p) => p.on) |> List.map((p) => pointAt(v(p.x, baseOf(i) + ceiling - 0.12, p.z), panelColor, 1.0, 8.0))),
    drums |> List.map((c) => pointAt(v(c.x, drumY0 - 0.25, c.z), warm, 0.9, 7.0)),
    fronts |> List.filter(isLitSign) |> List.map((f) =>
      let band = signBand(f) in
      let face = onWall(f.cell, f.side, trim, band.lo, band.hi, band.bottom, band.top, 0.5) in
      let centre = pt(0.0, face) |> Vec3.add(pt(2.0, face)) |> Vec3.scale(0.5) in
      pointAt(centre |> Vec3.add(v(0.0, baseOf(f.cell) - 0.3, 0.0)), if signOf(f) == signPink then pinkGlow else tealGlow, 0.35, 4.0)),
    [v(w.x0 + 2.0, 8.0, w.z0 + 2.0), v(w.x1 - 2.0, 8.0, w.z0 + 2.0), v(w.x0 + 2.0, 8.0, w.z1 - 2.0), v(w.x1 - 2.0, 8.0, w.z1 - 2.0)]
      |> List.map((p) => pointAt(p, Color.rgb(0.8, 0.86, 0.95), 0.45, 14.0)),
    [pointAt(v((island.x0 + island.x1) / 2.0, 0.7, (island.z0 + island.z1) / 2.0), tealGlow, 0.5, 5.0)],
  ])

// An overcast day, no sun (any yaw holds), as in building.fun: a patch of
// sky as a grid of shadowed spots about `step` apart over the skylight's
// opening, aimed down; and a patch of sky over the entrance and of sky-lit
// ground before it, aimed in.
let skyPerSquareMetre = 0.014
let gridSpots = (x0: float, x1: float, z0: float, z1: float, step: float, place: (float, float, float) => Light.t): List<Light.t> =>
  let nu = Math.ceil((x1 - x0) / step) in
  let nv = Math.ceil((z1 - z0) / step) in
  let du = (x1 - x0) / nu in
  let dv = (z1 - z0) / nv in
  List.range(nu * nv) |> List.map((k) => place(x0 + (Math.mod(k, nu) + 0.5) * du, z0 + (Math.floor(k / nu) + 0.5) * dv, du * dv))
let skylightLights =
  let w = voidRect in
  gridSpots(w.x0, w.x1, w.z0, w.z1, 1.5, (x, z, area) =>
    spotAt(v(x, roof + 0.5, z), v(0.0, -1.0, 0.0), sky, skyPerSquareMetre * area, 24.0))
// The kit's patches over and before the entrance (building.fun's
// `patchLights`), placed in the bake's space.
let entranceLights =
  let h = Building.holeOf(List.head(doors) |> Option.defaultValue(shopfront(0.0, 1.0, 0.0))) in
  let patch = (p: Building.Patch): List<Light.t> =>
    let y0 = if p.aboveHead then h.top else 0.0 in
    let width = h.hi - h.lo + 2.0 in
    let nu = Math.ceil(width / 0.5) in
    let nv = Math.ceil(p.height / 0.5) in
    let du = width / nu in
    let dv = p.height / nv in
    let target = v((h.lo + h.hi) / 2.0, if p.aimCeiling then ceiling else 0.0, 2.0) in
    List.range(nu * nv) |> List.map((k) =>
      let at = v(h.lo - 1.0 + (Math.mod(k, nu) + 0.5) * du, y0 + (Math.floor(k / nu) + 0.5) * dv, 0.0 - p.out) in
      spotAt(at, Vec3.normalize(target |> Vec3.sub(at)), p.color, p.perSquareMetre * du * dv, 12.0)) in
  List.flatten([patch(Building.skyPatch), patch(Building.groundPatch)])
let dayLights = List.flatten([nightLights, skylightLights, entranceLights])

// ---- The interior --------------------------------------------------------------------

// The cells' faces and the mesh, at full scale; the bake jobs, at half.
type Interior = { cellParts: List<List<Poly>>, cellRanges: List<Building.Range>, data: Model.staticData,
                  night: Lightmap.bakeJob, day: Lightmap.bakeJob }
let interiorOf = (name: string): Interior =>
  let cellParts = List.range(List.length(cells)) |> List.map(cellFaces) in
  let all = List.flatten(cellParts) in
  let cellRanges = rangesOf(cellParts) in
  let spans = cellParts |> List.zip(cellRanges) |> List.concatMap((pair) => let (polys, range) = pair in spansOf(palette, range.first, polys)) in
  let night: Lightmap.bakeJob = {
    name: $"{name}-night",
    mesh: meshOf(palette, bakeSpace, all, spans),
    lights: nightLights,
    albedo: spans |> List.map((s) => materialOf(materials, s.mat).albedo),
    scale: Building.bakeScale,
  } in
  { cellParts: cellParts, cellRanges: cellRanges, data: meshOf(palette, 1.0, all, spans),
    night: night, day: { night with name: $"{name}-day", lights: dayLights } }
let interior = interiorOf("mall")

// ---- One model per cell ----------------------------------------------------------------

// As building.fun's `cellModels`: the bake keeps triangle order and numbers
// vertices by first use, so each cell's vertices are one run.
let runsOf = (k: Interior, data: Model.staticData): List<Building.Run> =>
  let cellRuns = k.cellRanges
    |> List.indexedMap((i, r) => { cell: i, first: r.first, count: r.count, v0: 0.0, v1: 0.0 })
    |> List.filter((run) => run.count > 0.0) in
  let starts = cellRuns |> List.map((run) => Bytes.u32le(12.0 * run.first, data.triangles) |> Binary.require) in
  let ends = List.drop(1.0, starts) |> List.append([Bytes.length(data.positions) / 12.0]) in
  cellRuns |> List.zip(starts) |> List.zip(ends) |> List.map((pair) =>
    let (started, v1) = pair in
    let (run, v0) = started in
    { run with v0: v0, v1: v1 })
let cellData = (k: Interior, data: Model.staticData, run: Building.Run): Model.staticData =>
  { data with
    positions: Building.sliceRun(12.0, run, data.positions),
    normals: Building.sliceRun(12.0, run, data.normals),
    uvs: Building.sliceRun(8.0, run, data.uvs),
    uvs2: Building.sliceRun(8.0, run, data.uvs2),
    triangles: Bytes.fromU32le(List.range(3.0 * run.count) |> List.map((j) =>
      (Bytes.u32le(4.0 * (3.0 * run.first + j), data.triangles) |> Binary.require) - run.v0)),
    groups: spansOf(palette, 0.0, List.nth(run.cell, k.cellParts) |> Option.defaultValue([])) |> List.map((s) => groupOf(palette, s)) }
let cellModels = (k: Interior, data: Model.staticData): List<Building.CellModel> =>
  runsOf(k, data) |> List.map((run) => { cell: run.cell, model: Model.static(cellData(k, data, run)) })

// The baked mesh back at full scale: every position doubled (exact in f32).
let unscaled = (b: Bytes.t): Bytes.t =>
  Bytes.fromF32le(List.range(Bytes.length(b) / 4.0) |> List.map((i) => (Bytes.f32le(4.0 * i, b) |> Binary.require) / bakeSpace))

// The cell models under the night lightmap, and the day lightmap; flat-lit
// without a current bake (provenance.fun's `bakedOf`).
let bakedOf = (s: Provenance.Spec, status: Provenance.Status): Provenance.Baked =>
  match status with
  | Provenance.Current =>
    let mesh = Provenance.meshPathOf(s) |> File.bytes |> Lightmap.readMesh in
    { models: cellModels(interior, { mesh with positions: unscaled(mesh.positions), lightmap: Option.Some({
        texture: Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "night")), Texture.Srgb),
        scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) }),
      day: Option.Some(Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "day")), Texture.Srgb)) }
  | _ => { models: cellModels(interior, interior.data), day: Option.None }
