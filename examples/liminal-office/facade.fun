// The facade kit: trim and rooftop plant on an archetype's street side. Plain
// trim joins the street side's one instanced lit draw, textured trim its
// draw per finish (finishes.fun). Each archetype builds its trim once, in
// its street side, so every copy draws the same; shells keep all but the
// fine trim. Trim stands clear of every opening (and the canopy), so no
// face lies just in front of a window's. It has no colliders: what the
// street reaches stands at most 12 cm proud, inside the player's radius.

open Layout
open Building

let box = Exterior.box

// What an archetype adds to its street side: `plain` and `skinned` boxes,
// kept on its shells, and `fine` ones only the street side draws.
type Trim = { plain: List<(Exterior.Box, Color.t)>, skinned: List<(Exterior.Box, Finishes.Surface)>, fine: List<(Exterior.Box, Color.t)> }

let trimmed = (s: Exterior.Street, t: Trim): Exterior.Street =>
  let streetBoxes = List.flatten([s.streetBoxes, t.plain, t.fine]) in
  let skinned = List.flatten([s.skinned, t.skinned]) in
  { s with streetBoxes: streetBoxes, skinned: skinned, extraBoxes: List.flatten([s.extraBoxes, t.plain]),
           extraSkinned: List.flatten([s.extraSkinned, t.skinned]), streetSide: Exterior.sideOf(streetBoxes, skinned) }

// ---- On a facade -----------------------------------------------------------------

// A box on side `s`'s outer face: [u0, u1] along it (x on south and north,
// z on west and east), [y0, y1] up, from `d0` to `d1` out from the face.
let onFace = (l: Layout, s: Side, u0: float, u1: float, y0: float, y1: float, d0: float, d1: float): Exterior.Box =>
  let f = l.footprint in
  let o = l.facade / 2.0 in
  match s with
  | South => box(u0, y0, f.z0 - o - d1, u1, y1, f.z0 - o - d0)
  | North => box(u0, y0, f.z1 + o + d0, u1, y1, f.z1 + o + d1)
  | West => box(f.x0 - o - d1, y0, u0, f.x0 - o - d0, y1, u1)
  | East => box(f.x1 + o + d0, y0, u0, f.x1 + o + d1, y1, u1)
// The outer face's span along side `s`.
let spanOf = (l: Layout, s: Side): (float, float) =>
  let f = l.footprint in
  let o = l.facade / 2.0 in
  match s with
  | South => (f.x0 - o, f.x1 + o)
  | North => (f.x0 - o, f.x1 + o)
  | West => (f.z0 - o, f.z1 + o)
  | East => (f.z0 - o, f.z1 + o)
let sides = Portals.sides

// What trim keeps clear of on side `s`: every level's openings and bands, and
// on the south the entrance under its canopy, from the street up.
let holesOn = (l: Layout, st: Exterior.Street, s: Side): List<Hole> =>
  let c = st.canopyBox in
  List.flatten([
    levelsOf(l) |> List.concatMap((level) => Exterior.facadeHoles(l, level, s) |> List.map((h) =>
      { h with bottom: level * l.storey + h.bottom, top: level * l.storey + h.top })),
    if s == South then [{ lo: c.x0, hi: c.x1, bottom: 0.0, top: c.y1 }] else [],
  ])
// Whether [u0, u1] × [y0, y1] misses every hole, by `gap`.
let isClear = (holes: List<Hole>, u0: float, u1: float, y0: float, y1: float, gap: float): bool =>
  not (holes |> List.any((h) => h.lo - gap < u1 && h.hi + gap > u0 && h.bottom - gap < y1 && h.top + gap > y0))
// [s0, s1] less the holes that reach [y0, y1], `gap` round each, in runs
// longer than `gap`.
let runs = (s0: float, s1: float, y0: float, y1: float, holes: List<Hole>, gap: float): List<(float, float)> =>
  let blocking = holes |> List.filter((h) => h.bottom - gap < y1 && h.top + gap > y0) |> List.sortBy((h) => h.lo) in
  let run = (a: float, b: float): List<(float, float)> => if b - a > gap then [(a, b)] else [] in
  let (cursor, out) = blocking |> List.fold((acc, h) =>
    let (c, o) = acc in (Math.max(c, h.hi + gap), List.flatten([o, run(c, h.lo - gap)])), (s0, [])) in
  List.flatten([out, run(cursor, s1)])

// Horizontal courses round the building, `d` proud, at each of `bands`
// ([y0, y1]), broken round openings: string courses and spandrels.
let courses = (l: Layout, st: Exterior.Street, bands: List<(float, float)>, d: float): List<Exterior.Box> =>
  sides |> List.concatMap((s) =>
    let (s0, s1) = spanOf(l, s) in
    let holes = holesOn(l, st, s) in
    // South and north wrap the corners.
    let (a, b) = if s == South || s == North then (s0 - d, s1 + d) else (s0, s1) in
    bands |> List.concatMap((band) =>
      let (y0, y1) = band in
      runs(a, b, y0, y1, holes, 0.1) |> List.map((r) => let (u0, u1) = r in onFace(l, s, u0, u1, y0, y1, 0.0, d))))

// Vertical piers `w` wide, `d` proud, from the street to `top`: at each
// corner (L-shaped, wrapping it: both halves or neither) and every `every`
// metres between (0: corners only), wherever they miss the openings.
let pilasters = (l: Layout, st: Exterior.Street, every: float, w: float, d: float, top: float): List<Exterior.Box> =>
  let pier = (s: Side, u0: float, u1: float): Option.t<Exterior.Box> =>
    if isClear(holesOn(l, st, s), u0, u1, Exterior.street, top, 0.05) then Option.Some(onFace(l, s, u0, u1, Exterior.street, top, 0.0, d))
    else Option.None in
  let (x0, x1) = spanOf(l, South) in
  let (z0, z1) = spanOf(l, West) in
  // Each corner: its south or north half (wrapping it) and its west or east half.
  let corners = [
    (pier(South, x0 - d, x0 + w), pier(West, z0, z0 + w)),
    (pier(South, x1 - w, x1 + d), pier(East, z0, z0 + w)),
    (pier(North, x0 - d, x0 + w), pier(West, z1 - w, z1)),
    (pier(North, x1 - w, x1 + d), pier(East, z1 - w, z1)),
  ] |> List.concatMap((e) =>
    let (a, b) = e in
    if Option.isSome(a) && Option.isSome(b) then List.flatten([Option.toList(a), Option.toList(b)]) else []) in
  let between = sides |> List.concatMap((s) =>
    let (s0, s1) = spanOf(l, s) in
    let n = if every > 0.0 then Math.floor((s1 - s0) / every) else 0.0 in
    List.range(Math.max(0.0, n - 1.0)) |> List.concatMap((k) =>
      let u = s0 + (s1 - s0) * (k + 1.0) / n in Option.toList(pier(s, u - w / 2.0, u + w / 2.0)))) in
  List.flatten([corners, between])

// Vertical fins `w` wide, `d` proud, at each of `us` along every side, from
// `y0` to `y1`, wherever they miss the openings.
let fins = (l: Layout, st: Exterior.Street, us: List<float>, y0: float, y1: float, w: float, d: float): List<Exterior.Box> =>
  sides |> List.concatMap((s) =>
    let holes = holesOn(l, st, s) in
    us |> List.filter((u) => isClear(holes, u - w / 2.0, u + w / 2.0, y0, y1, 0.05))
      |> List.map((u) => onFace(l, s, u - w / 2.0, u + w / 2.0, y0, y1, 0.0, d)))

// A cap along the parapet's top, `over` past its outer face and `h` deep:
// south and north whole, west and east between them.
let cornice = (l: Layout, over: float, h: float): List<Exterior.Box> =>
  let f = l.footprint in
  let o = l.facade / 2.0 in
  let y0 = l.facadeHeight - 0.05 in
  let y1 = l.facadeHeight + h in
  [
    box(f.x0 - o - over, y0, f.z0 - o - over, f.x1 + o + over, y1, f.z0 + 0.05),
    box(f.x0 - o - over, y0, f.z1 - 0.05, f.x1 + o + over, y1, f.z1 + o + over),
    box(f.x0 - o - over, y0, f.z0 + 0.05, f.x0 + 0.05, y1, f.z1 - 0.05),
    box(f.x1 - 0.05, y0, f.z0 + 0.05, f.x1 + o + over, y1, f.z1 - 0.05),
  ]

// A frame round the entrance, `w` wide and `d` proud: jambs, and a head up
// to the canopy.
let portal = (l: Layout, st: Exterior.Street, w: float, d: float): List<Exterior.Box> =>
  let e = st.entrance in
  let head = Math.min(e.top + w, st.canopyBox.y0) in
  [
    onFace(l, South, e.lo - w, e.lo, Exterior.street, head, 0.0, d),
    onFace(l, South, e.hi, e.hi + w, Exterior.street, head, 0.0, d),
    onFace(l, South, e.lo, e.hi, e.top, head, 0.0, d),
  ]

// A fascia lip under the canopy's open edges, `h` deep.
let canopyEdge = (st: Exterior.Street, h: float): List<Exterior.Box> =>
  let k = st.canopyBox in
  let t = 0.08 in
  [
    box(k.x0, k.y0 - h, k.z0, k.x1, k.y0, k.z0 + t),
    box(k.x0, k.y0 - h, k.z0 + t, k.x0 + t, k.y0, k.z1),
    box(k.x1 - t, k.y0 - h, k.z0 + t, k.x1, k.y0, k.z1),
  ]

// Mullions splitting each dark window band in three, from its panel to just
// inside the facade's face.
let mullions = (l: Layout, w: float): List<Exterior.Box> =>
  levelsOf(l) |> List.concatMap((level) => sides |> List.concatMap((s) =>
    Exterior.bandHoles(l, level, s) |> List.concatMap((h) => [1.0, 2.0] |> List.map((k) =>
      let u = h.lo + (h.hi - h.lo) * k / 3.0 in
      let y = level * l.storey in
      onFace(l, s, u - w / 2.0, u + w / 2.0, y + h.bottom, y + h.top, 0.0 - Exterior.recess, 0.0 - 0.01)))))

// ---- On a roof -----------------------------------------------------------------

let plantGrey = rgb(124.0, 126.0, 124.0)
let fanDark = rgb(58.0, 60.0, 62.0)
let ventGrey = rgb(118.0, 118.0, 122.0)
let dishWhite = rgb(160.0, 160.0, 156.0)
let mastDark = rgb(52.0, 52.0, 56.0)
let tankRust = rgb(112.0, 96.0, 82.0)

// A packaged air handler centred on (x, z) on a roof at `y`, its fan on top;
// `turn`: its long side along z.
let hvac = (x: float, z: float, y: float, turn: bool): List<(Exterior.Box, Color.t)> =>
  let (a, b) = if turn then (0.65, 1.2) else (1.2, 0.65) in
  [(box(x - a, y, z - b, x + a, y + 1.4, z + b), plantGrey),
   (box(x - 0.45, y + 1.4, z - 0.45, x + 0.45, y + 1.55, z + 0.45), fanDark)]
// Two vent stacks.
let vents = (x: float, z: float, y: float): List<(Exterior.Box, Color.t)> =>
  [(box(x - 0.2, y, z - 0.2, x + 0.2, y + 0.7, z + 0.2), ventGrey),
   (box(x + 0.6, y, z + 0.3, x + 0.85, y + 1.0, z + 0.55), ventGrey)]
// A dish on a post, facing south.
let dish = (x: float, z: float, y: float): List<(Exterior.Box, Color.t)> =>
  [(box(x - 0.06, y, z - 0.06, x + 0.06, y + 1.1, z + 0.06), mastDark),
   (box(x - 0.6, y + 0.8, z - 0.16, x + 0.6, y + 2.0, z - 0.06), dishWhite)]
// A stair's head house over `r`, `h` tall, with a door on its south face.
let stairHead = (r: Rect, y: float, h: float, c: Color.t): List<(Exterior.Box, Color.t)> =>
  let mid = (r.x0 + r.x1) / 2.0 in
  [(box(r.x0, y, r.z0, r.x1, y + h, r.z1), c),
   (box(mid - 0.45, y, r.z0 - 0.04, mid + 0.45, y + 2.1, r.z0), fanDark)]
// A mast `h` tall with two cross arms.
let antenna = (x: float, z: float, y: float, h: float): List<(Exterior.Box, Color.t)> =>
  [(box(x - 0.08, y, z - 0.08, x + 0.08, y + h, z + 0.08), mastDark),
   (box(x - 0.9, y + 0.6 * h, z - 0.04, x + 0.9, y + 0.6 * h + 0.08, z + 0.04), mastDark),
   (box(x - 0.04, y + 0.8 * h, z - 0.6, x + 0.04, y + 0.8 * h + 0.08, z + 0.6), mastDark)]
// A water tank on four legs, `r` from its centre to its sides.
let waterTank = (x: float, z: float, y: float, r: float): List<(Exterior.Box, Color.t)> =>
  let leg = (lx: float, lz: float) => (box(lx - 0.1, y, lz - 0.1, lx + 0.1, y + 1.6, lz + 0.1), mastDark) in
  List.flatten([
    [leg(x - r + 0.3, z - r + 0.3), leg(x + r - 0.3, z - r + 0.3), leg(x - r + 0.3, z + r - 0.3), leg(x + r - 0.3, z + r - 0.3)],
    [(box(x - r, y + 1.6, z - r, x + r, y + 4.4, z + r), tankRust),
     (box(x - r - 0.05, y + 4.4, z - r - 0.05, x + r + 0.05, y + 4.6, z + r + 0.05), mastDark)],
  ])

// Plant over `area`, on a roof at `y`, the same for every copy of an
// archetype (`seed`): a grid of 6 m cells, each holding an air handler,
// vents, a dish or nothing, by noise; none in a cell meeting `keepOut`.
let rooftop = (seed: float, area: Rect, y: float, keepOut: List<Rect>): List<(Exterior.Box, Color.t)> =>
  let cell = 6.0 in
  let nx = Math.floor((area.x1 - area.x0) / cell) in
  let nz = Math.floor((area.z1 - area.z0) / cell) in
  List.range(nx * nz) |> List.concatMap((i) =>
    let x0 = area.x0 + cell * Math.mod(i, nx) in
    let z0 = area.z0 + cell * Math.floor(i / nx) in
    let x = x0 + cell / 2.0 in
    let z = z0 + cell / 2.0 in
    let r = noise(seed * 131.0 + i * 17.0) in
    let blocked = keepOut |> List.any((k) => k.x0 < x0 + cell && k.x1 > x0 && k.z0 < z0 + cell && k.z1 > z0) in
    if blocked then []
    else if r < 0.4 then hvac(x, z, y, noise(seed * 71.0 + i * 5.0) < 0.5)
    else if r < 0.62 then vents(x, z, y)
    else if r < 0.7 then dish(x, z, y)
    else [])

// ---- Dressing -----------------------------------------------------------------------

// Boxes in a colour, or a surface.
let painted = (c: Color.t, bs: List<Exterior.Box>): List<(Exterior.Box, Color.t)> => bs |> List.map((b) => (b, c))
let clad = (f: Finishes.Surface, bs: List<Exterior.Box>): List<(Exterior.Box, Finishes.Surface)> => bs |> List.map((b) => (b, f))

// The office: precast panels, a cap and corner piers in a paler mix, a
// string course at the upper floor, mullions in the bands, a dark frame
// round the entrance and under the canopy, and plant on the roof by a
// stair's head house.
let capPale = rgb(134.0, 134.0, 130.0)
let pierPale = rgb(126.0, 126.0, 122.0)
let frameDark = rgb(54.0, 56.0, 60.0)
let mullionDark = rgb(40.0, 44.0, 50.0)
let headGrey = rgb(112.0, 112.0, 108.0)
let office = (l: Layout): Exterior.Street =>
  let s = Exterior.streetOf(l) in
  let f = l.footprint in
  let roof = l.levels * l.storey in
  let head = rect(f.x1 - 5.5, f.z1 - 6.5, f.x1 - 2.0, f.z1 - 2.0) in
  trimmed(s, {
    plain: List.flatten([
      painted(capPale, cornice(l, 0.15, 0.25)),
      painted(pierPale, pilasters(l, s, 0.0, 0.6, 0.12, l.facadeHeight - 0.15)),
      painted(capPale, courses(l, s, List.range(l.levels - 1.0) |> List.map((k) =>
        let y = (k + 1.0) * l.storey in (y - 0.25, y + 0.35)), 0.08)),
      painted(frameDark, List.flatten([portal(l, s, 0.35, 0.12), canopyEdge(s, 0.15)])),
      stairHead(head, roof, 2.7, headGrey),
      rooftop(1.0, rect(f.x0 + 1.5, f.z0 + 1.5, f.x1 - 1.5, f.z1 - 1.5), roof, [head]),
    ]),
    skinned: [],
    fine: painted(mullionDark, mullions(l, 0.06)),
  })
