// A layout's culling cells and portals, in its local space: a cell per
// room, and the collar, four street strips around the footprint. Not hashed
// into the bake's provenance (the bake doesn't depend on them).
//
// `Scene.cells` draws every cell when the eye is outside them all, so the
// collar must hold every eye that sees the interior: block.fun's activation
// radius plus an eye's reach (tests.fun checks it). A ring isn't convex, so
// it is four boxes: the south and north strips run the full width, corners
// included; the west and east strips fill between them.

open Layout
open Building

// ---- The collar ----------------------------------------------------------------

// How far the collar reaches out from the footprint's wall lines.
let collar = 35.0
// The outside cells' top: over the roofline.
let outsideTop = 8.0

let box3 = (r: Rect, y0: float, y1: float): Cells.cell => { planes: [
  { n: v(1.0, 0.0, 0.0), d: 0.0 - r.x0 }, { n: v(-1.0, 0.0, 0.0), d: r.x1 },
  { n: v(0.0, 1.0, 0.0), d: 0.0 - y0 },   { n: v(0.0, -1.0, 0.0), d: y1 },
  { n: v(0.0, 0.0, 1.0), d: 0.0 - r.z0 }, { n: v(0.0, 0.0, -1.0), d: r.z1 },
] }

let sides = [South, North, West, East]

// The collar's strip along footprint side `s`.
let stripOf = (l: Layout, s: Side): Rect =>
  let f = l.footprint in
  match s with
  | South => rect(f.x0 - collar, f.z0 - collar, f.x1 + collar, f.z0)
  | North => rect(f.x0 - collar, f.z1, f.x1 + collar, f.z1 + collar)
  | West => rect(f.x0 - collar, f.z0, f.x0, f.z1)
  | East => rect(f.x1, f.z0, f.x1 + collar, f.z1)

let within = (outer: Rect, r: Rect): bool =>
  r.x0 >= outer.x0 - eps && r.x1 <= outer.x1 + eps && r.z0 >= outer.z0 - eps && r.z1 <= outer.z1 + eps

// An outside room (the apron, the west yard) is the strip it lies in: its
// cell id stays its room id. Sides with no outside room get ids after the
// rooms', in `sides` order.
let roomOn = (l: Layout, s: Side): Option.t<float> =>
  List.range(List.length(l.rooms)) |> List.find((i) => not isInside(roomAt(l, i)) && within(stripOf(l, s), roomAt(l, i).rect))
let extraSides = (l: Layout): List<Side> => sides |> List.filter((s) => roomOn(l, s) == Option.None)
let stripId = (l: Layout, s: Side): float =>
  match roomOn(l, s) with
  | Option.Some(i) => i
  | Option.None =>
    let extra = extraSides(l) in
    List.length(l.rooms) + (List.range(List.length(extra)) |> List.find((k) => List.nth(k, extra) == Option.Some(s)) |> Option.defaultValue(0.0))
let sideOfRoom = (l: Layout, i: float): Side =>
  sides |> List.find((s) => roomOn(l, s) == Option.Some(i)) |> Option.defaultValue(South)

let strip = (l: Layout, s: Side): Cells.cell => box3(stripOf(l, s), 0.0, outsideTop)

// Where a west or east strip meets the south or north one: the face they
// share, on the footprint's south or north line.
let corner = (l: Layout, ns: Side, we: Side): List<Vec3.t> =>
  let r = stripOf(l, we) in
  let z = if ns == South then l.footprint.z0 else l.footprint.z1 in
  [v(r.x0, 0.0, z), v(r.x1, 0.0, z), v(r.x1, outsideTop, z), v(r.x0, outsideTop, z)]

// ---- Rooms ---------------------------------------------------------------------

// Room `i`'s cell: its rect, from its floor up to its walls' top; an
// outside room's is its strip.
let cellOf = (l: Layout, i: float): Cells.cell =>
  let r = roomAt(l, i) in
  if isInside(r) then box3(r.rect, baseOf(l, r), baseOf(l, r) + wallTop(l, i)) else strip(l, sideOfRoom(l, i))

// Hole `h` in the wall on line `line` along `side`, from a floor at `y`, as
// a polygon on that line.
let opening = (side: Side, line: float, y: float, h: Hole): List<Vec3.t> =>
  let y0 = y + h.bottom in
  let y1 = y + h.top in
  if side == North || side == South then [v(h.lo, y0, line), v(h.hi, y0, line), v(h.hi, y1, line), v(h.lo, y1, line)]
  else [v(line, y0, h.lo), v(line, y0, h.hi), v(line, y1, h.hi), v(line, y1, h.lo)]

let doorway = (l: Layout, d: Door): List<Vec3.t> =>
  let ra = roomAt(l, d.a).rect in
  match sideBetween(ra, roomAt(l, d.b).rect) with
  | Option.None => []
  | Option.Some(side) => opening(side, lineOf(side, ra), baseOf(l, roomAt(l, d.a)), holeOf(d))

// A flight's stair opening: the floor of its upper room less the landing.
let stairOpening = (l: Layout, f: Flight): List<Vec3.t> =>
  match f.kind with
  | Straight =>
    let r = inner(l, roomAt(l, f.lower).rect) in
    let y = baseOf(l, roomAt(l, f.upper)) in
    let x1 = flightEnd(f) in
    [v(r.x0, y, r.z0), v(x1, y, r.z0), v(x1, y, r.z1), v(r.x0, y, r.z1)]

let windowOpening = (l: Layout, w: Window): List<Vec3.t> =>
  let r = roomAt(l, w.room) in
  opening(w.side, lineOf(w.side, r.rect), baseOf(l, r), windowHole(w))

// Both directions of a portal from `a` to `b`.
let both = (a: float, b: float, polygon: List<Vec3.t>): List<Cells.portal> =>
  [{ from: a, to: b, polygon: polygon }, { from: b, to: a, polygon: List.reverse(polygon) }]

// Portals: each door, each window onto an outside room (the street sees in),
// a horizontal one at each stair opening, and the collar's four corners.
let portalsOf = (l: Layout): List<Cells.portal> => List.flatten([
  l.doors |> List.concatMap((d) => both(d.a, d.b, doorway(l, d))),
  l.windows |> List.concatMap((w) =>
    match outsideOf(l, w) with
    | Option.Some(o) => both(o, w.room, windowOpening(l, w))
    | Option.None => []),
  l.flights |> List.concatMap((f) => both(f.lower, f.upper, stairOpening(l, f))),
  [South, North] |> List.concatMap((ns) => [West, East] |> List.concatMap((we) =>
    both(stripId(l, ns), stripId(l, we), corner(l, ns, we)))),
])

// Every cell: the rooms', then the collar strips no outside room holds.
let cellsOf = (l: Layout): List<Cells.cell> =>
  List.flatten([List.range(List.length(l.rooms)) |> List.map((i) => cellOf(l, i)), extraSides(l) |> List.map((s) => strip(l, s))])

let levelOf = (l: Layout): Cells.t => Cells.make({ cells: cellsOf(l), portals: portalsOf(l) })
