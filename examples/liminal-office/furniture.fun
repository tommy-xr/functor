// The furniture kit: layout.fun's `Furnish` entries as chunky boxes, baked
// with their rooms (building.fun, cinemaBuilding.fun and mallInterior.fun
// each paint the swatches their own way), and a collider box each. A
// piece's disorder comes from the noise at its place, so every copy of an
// archetype matches. Editing this file makes every committed bake stale:
// rebake (README).

open Layout

let v = Vec3.make

// ---- Swatches --------------------------------------------------------------------

// What a part shows. An archetype paints each from its own colours, as one
// atlas of `atlasSide` × `atlasSide` blocks (building.fun's `furnishImage`).
let laminate = 0.0
let fabric = 1.0
let dark = 2.0
let metal = 3.0
let wood = 4.0
let leaf = 5.0
let white = 6.0
let water = 7.0
let books = 8.0
let dead = 9.0
let swatches = 10.0
let atlasSide = 4.0
let blockTexels = 16.0
// Swatch `s`'s block in the atlas, two texels in from its edges.
let swatchRect = (s: float): Texture.rect =>
  let size = atlasSide * blockTexels in
  let x = Math.mod(s, atlasSide) * blockTexels in
  let y = Math.floor(s / atlasSide) * blockTexels in
  { x: x, y: y, u0: (x + 2.0) / size, v0: (y + 2.0) / size, u1: (x + blockTexels - 2.0) / size, v1: (y + blockTexels - 2.0) / size }

// ---- Boxes -----------------------------------------------------------------------

// A box by its centre and half-axes; `loose` ones (a fallen cushion, books
// on the floor) are no part of the piece's collider.
type Solid = { swatch: float, c: Vec3.t, ax: Vec3.t, ay: Vec3.t, az: Vec3.t, loose: bool }
// [x0, x1] × [y0, y1] × [z0, z1] in a piece's frame: x across it, y up from
// the floor, its front toward +z.
let part = (swatch: float, x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Solid =>
  { swatch: swatch, c: v((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0),
    ax: v(Math.abs(x1 - x0) / 2.0, 0.0, 0.0), ay: v(0.0, Math.abs(y1 - y0) / 2.0, 0.0), az: v(0.0, 0.0, Math.abs(z1 - z0) / 2.0), loose: false }
let loosen = (s: Solid): Solid => { s with loose: true }

// Turns about the frame's origin: about y (+z swings toward +x, as
// block.fun's `turn`), about x (+y toward +z) and about z (+x toward +y).
let yawed = (deg: float, p: Vec3.t): Vec3.t =>
  let a = deg * Math.pi / 180.0 in
  v(Vec3.x(p) * Math.cos(a) + Vec3.z(p) * Math.sin(a), Vec3.y(p), Vec3.z(p) * Math.cos(a) - Vec3.x(p) * Math.sin(a))
let tilted = (deg: float, p: Vec3.t): Vec3.t =>
  let a = deg * Math.pi / 180.0 in
  v(Vec3.x(p), Vec3.y(p) * Math.cos(a) - Vec3.z(p) * Math.sin(a), Vec3.y(p) * Math.sin(a) + Vec3.z(p) * Math.cos(a))
let rolled = (deg: float, p: Vec3.t): Vec3.t =>
  let a = deg * Math.pi / 180.0 in
  v(Vec3.x(p) * Math.cos(a) - Vec3.y(p) * Math.sin(a), Vec3.x(p) * Math.sin(a) + Vec3.y(p) * Math.cos(a), Vec3.z(p))
// `parts` turned by `turn` about the origin, then moved by `by`.
let moved = (turn: (Vec3.t) => Vec3.t, by: Vec3.t, parts: List<Solid>): List<Solid> =>
  parts |> List.map((s) => { s with c: turn(s.c) |> Vec3.add(by), ax: turn(s.ax), ay: turn(s.ay), az: turn(s.az) })
let shifted = (x: float, y: float, z: float, parts: List<Solid>): List<Solid> => moved((p) => p, v(x, y, z), parts)

let cornersOf = (s: Solid): List<Vec3.t> =>
  [-1.0, 1.0] |> List.concatMap((i) => [-1.0, 1.0] |> List.concatMap((j) => [-1.0, 1.0] |> List.map((k) =>
    s.c |> Vec3.add(Vec3.scale(i, s.ax)) |> Vec3.add(Vec3.scale(j, s.ay)) |> Vec3.add(Vec3.scale(k, s.az)))))
let lowest = (parts: List<Solid>): float =>
  parts |> List.concatMap(cornersOf) |> List.map(Vec3.y) |> List.minimum |> Option.defaultValue(0.0)
// `parts` set down on the floor: lifted (or lowered) till the lowest touches it.
let grounded = (parts: List<Solid>): List<Solid> => shifted(0.0, 0.0 - lowest(parts), 0.0, parts)

// An axis-aligned box: a collider, or a piece's extent.
type Bounds = { x0: float, y0: float, z0: float, x1: float, y1: float, z1: float }
let boundsOf = (parts: List<Solid>): Bounds =>
  let ps = parts |> List.concatMap(cornersOf) in
  let lo = (f: (Vec3.t) => float) => ps |> List.map(f) |> List.minimum |> Option.defaultValue(0.0) in
  let hi = (f: (Vec3.t) => float) => ps |> List.map(f) |> List.maximum |> Option.defaultValue(0.0) in
  { x0: lo(Vec3.x), y0: lo(Vec3.y), z0: lo(Vec3.z), x1: hi(Vec3.x), y1: hi(Vec3.y), z1: hi(Vec3.z) }

// A face of a box: a quad from corner `o` along `a`, then `b`, facing
// `a × b` (building.fun's `Quad`).
type Slab = { swatch: float, o: Vec3.t, a: Vec3.t, b: Vec3.t }
// A box's six outward faces, less any lying on the floor (unseen).
let slabsOf = (s: Solid): List<Slab> =>
  let side = (n: Vec3.t, p: Vec3.t, q: Vec3.t): Slab =>
    { swatch: s.swatch, o: s.c |> Vec3.add(n) |> Vec3.sub(p) |> Vec3.sub(q), a: Vec3.scale(2.0, p), b: Vec3.scale(2.0, q) } in
  let neg = (p: Vec3.t): Vec3.t => Vec3.scale(-1.0, p) in
  [side(s.ax, s.ay, s.az), side(neg(s.ax), s.az, s.ay), side(s.ay, s.az, s.ax), side(neg(s.ay), s.ax, s.az),
   side(s.az, s.ax, s.ay), side(neg(s.az), s.ay, s.ax)]
    |> List.filter((f) => [f.o, f.o |> Vec3.add(f.a), f.o |> Vec3.add(f.b), f.o |> Vec3.add(f.a) |> Vec3.add(f.b)]
      |> List.any((p) => Vec3.y(p) > 0.005))

// Whether `p` is inside box `s` (its faces included).
let holds = (s: Solid, p: Vec3.t): bool =>
  let d = p |> Vec3.sub(s.c) in
  [s.ax, s.ay, s.az] |> List.all((h) => Math.abs(Vec3.dot(d, h)) <= Vec3.dot(h, h) + 0.0001)
// Every box's faces less those flush against or buried in another box of
// the same piece (a drawer's back on its pedestal, a leg's top under a
// table top): unseen, they'd only take lightmap texels.
let unseen = (parts: List<Solid>): List<Slab> =>
  parts |> List.indexedMap((k, s) => slabsOf(s) |> List.filter((f) =>
    let n = Vec3.scale(0.002, Vec3.normalize(f.a |> Vec3.cross(f.b))) in
    let at = (u: float, w: float): Vec3.t => f.o |> Vec3.add(Vec3.scale(u, f.a)) |> Vec3.add(Vec3.scale(w, f.b)) |> Vec3.add(n) in
    let probes = [at(0.5, 0.5), at(0.05, 0.05), at(0.95, 0.05), at(0.05, 0.95), at(0.95, 0.95)] in
    not (probes |> List.all((p) => parts |> List.indexedMap((j, o) => j != k && holds(o, p)) |> List.any((x) => x))))) |> List.flatten

// ---- Disorder --------------------------------------------------------------------

// Deterministic noise in [0, 1) (building.fun's textures use it too), and
// the `k`th draw for a piece: from its place, so it never changes with the list around it.
let noise = (i: float): float =>
  let s = Math.sin(i * 12.9898 + 78.233) * 43758.5453 in
  s - Math.floor(s)
let draw = (f: Furnish, k: float): float => noise(f.x * 37.13 + f.z * 11.71 + k * 3.7)
// A swing in [-1, 1), scaled by the mess.
let swing = (f: Furnish, k: float): float => f.mess * (2.0 * draw(f, k) - 1.0)
let happens = (f: Furnish, k: float, odds: float): bool => draw(f, k) < f.mess * odds

// The way a side faces, as a yaw: north (+z) is 0.
let yawOf = (s: Side): float =>
  match s with
  | North => 0.0
  | East => 90.0
  | South => 180.0
  | West => 270.0

// How far a piece turns and slides off its place at full mess.
type Drift = { spin: float, slide: float }
let driftOf = (p: Article): Drift =>
  match p with
  | Desk => { spin: 14.0, slide: 0.25 }
  | Chair => { spin: 180.0, slide: 0.5 }
  | SideChair => { spin: 60.0, slide: 0.4 }
  | Couch => { spin: 9.0, slide: 0.3 }
  | CoffeeTable => { spin: 25.0, slide: 0.35 }
  | Cabinet => { spin: 5.0, slide: 0.1 }
  | Bookshelf => { spin: 3.0, slide: 0.05 }
  | Cooler => { spin: 10.0, slide: 0.15 }
  | Plant => { spin: 40.0, slide: 0.15 }
  | Bed => { spin: 4.0, slide: 0.1 }
  | _ => { spin: 0.0, slide: 0.0 }

// ---- Pieces ----------------------------------------------------------------------

// A drawer `w` wide and `h` tall whose front is at `z` from `x0`, `y0`,
// pulled `pull` out: shut, a plate and a handle; open, a tray 0.4 deep.
let drawer = (x0: float, y0: float, w: float, h: float, z: float, pull: float): List<Solid> =>
  let x1 = x0 + w in
  let f = z + pull in
  let handle = part(dark, x0 + w / 2.0 - 0.07, y0 + h - 0.07, f + 0.02, x0 + w / 2.0 + 0.07, y0 + h - 0.05, f + 0.04) in
  if pull <= 0.0 then [part(metal, x0, y0, z, x1, y0 + h, z + 0.02), handle]
  else [part(metal, x0, y0, f, x1, y0 + h, f + 0.02), handle,
        part(metal, x0 + 0.01, y0 + 0.02, f - 0.4, x0 + 0.02, y0 + h - 0.04, f),
        part(metal, x1 - 0.02, y0 + 0.02, f - 0.4, x1 - 0.01, y0 + h - 0.04, f),
        part(dark, x0 + 0.02, y0 + 0.02, f - 0.4, x1 - 0.02, y0 + 0.03, f)]

// A beige monitor and keyboard on a desk top at `y`, turned `deg`.
let monitor = (y: float, deg: float): List<Solid> =>
  moved((p) => yawed(deg, p), v(0.0 - 0.15, y, 0.0 - 0.05), [
    part(white, -0.2, 0.0, -0.2, 0.2, 0.36, 0.16),
    part(dark, -0.16, 0.04, 0.16, 0.16, 0.31, 0.17),
    part(white, -0.22, 0.0, 0.22, 0.22, 0.03, 0.38),
  ])

// A desk 1.5 × 0.75: a laminate top, a side panel, a drawer pedestal on
// its right (the top drawer maybe pulled out), a modesty panel; maybe a
// monitor.
let desk = (f: Furnish): List<Solid> =>
  let pull = if happens(f, 3.0, 1.0) then 0.2 + 0.12 * draw(f, 4.0) else 0.0 in
  List.flatten([
    [part(laminate, -0.75, 0.71, -0.375, 0.75, 0.74, 0.375),
     part(metal, -0.74, 0.0, -0.35, -0.71, 0.71, 0.35),
     part(metal, 0.33, 0.0, -0.34, 0.72, 0.71, 0.32),
     part(metal, -0.71, 0.25, -0.35, 0.33, 0.71, -0.32)],
    drawer(0.35, 0.5, 0.35, 0.18, 0.32, pull),
    drawer(0.35, 0.27, 0.35, 0.2, 0.32, 0.0),
    drawer(0.35, 0.04, 0.35, 0.2, 0.32, 0.0),
    if draw(f, 5.0) < 0.65 then monitor(0.74, 40.0 * swing(f, 6.0)) else [],
  ])

// A swivel chair: a cross of feet, a column, a seat and a back.
let chairParts = [
  part(dark, -0.3, 0.0, -0.035, 0.3, 0.06, 0.035), part(dark, -0.035, 0.0, -0.3, 0.035, 0.06, 0.3),
  part(dark, -0.03, 0.06, -0.03, 0.03, 0.42, 0.03),
  part(fabric, -0.25, 0.42, -0.23, 0.25, 0.5, 0.25),
  part(dark, -0.03, 0.44, -0.27, 0.03, 0.6, -0.23),
  part(fabric, -0.23, 0.56, -0.29, 0.23, 0.96, -0.21),
]
// Knocked over on its back or on its side, by the noise.
let chair = (f: Furnish): List<Solid> =>
  if not happens(f, 3.0, 0.45) then chairParts
  else if draw(f, 4.0) < 0.5 then grounded(moved((p) => tilted(-90.0, p), v(0.0, 0.0, 0.0), chairParts))
  else grounded(moved((p) => rolled(90.0, p), v(0.0, 0.0, 0.0), chairParts))

// A stacking chair: four legs, a seat, a back on two posts.
let sideChairParts = List.flatten([
  [(-0.2, -0.2), (0.175, -0.2), (-0.2, 0.2), (0.175, 0.2)] |> List.map((e) =>
    let (x, z) = e in part(dark, x, 0.0, z, x + 0.025, 0.44, z + 0.025)),
  [part(fabric, -0.22, 0.44, -0.22, 0.22, 0.48, 0.24),
   part(dark, -0.2, 0.48, -0.24, -0.18, 0.8, -0.22), part(dark, 0.18, 0.48, -0.24, 0.2, 0.8, -0.22),
   part(fabric, -0.21, 0.58, -0.25, 0.21, 0.82, -0.22)],
])
let sideChair = (f: Furnish): List<Solid> =>
  if happens(f, 3.0, 0.5) then grounded(moved((p) => tilted(-90.0, p), v(0.0, 0.0, 0.0), sideChairParts)) else sideChairParts
// Four stacked, nested 9 cm apart, each a little off; the top one maybe
// upside down on the rest.
let stack = (f: Furnish): List<Solid> =>
  let n = 4.0 in
  List.flatten([
    List.range(n - 1.0) |> List.concatMap((k) =>
      moved((p) => yawed(4.0 * swing(f, 10.0 + k), p), v(0.03 * swing(f, 20.0 + k), 0.09 * k, 0.03 * swing(f, 30.0 + k)), sideChairParts)),
    // Flipped front to back too, so its back hangs clear in front of the stack.
    if happens(f, 3.0, 0.6) then moved((p) => tilted(180.0, p), v(0.0, 0.09 * (n - 2.0) + 0.96, 0.04), sideChairParts)
    else moved((p) => yawed(6.0 * swing(f, 13.0), p), v(0.0, 0.09 * (n - 1.0), 0.0), sideChairParts),
  ])

// A couch 2 m long: a base on dark feet, a back, two arms, two seat
// cushions; one maybe pulled off onto the floor in front of it.
let couch = (f: Furnish): List<Solid> =>
  let fallen = happens(f, 3.0, 0.8) in
  let cushion = part(fabric, 0.01, 0.42, -0.22, 0.82, 0.54, 0.4) in
  List.flatten([
    [(-0.95, -0.38), (0.89, -0.38), (-0.95, 0.32), (0.89, 0.32)] |> List.map((e) =>
      let (x, z) = e in part(dark, x, 0.0, z, x + 0.06, 0.08, z + 0.06)),
    [part(fabric, -1.0, 0.08, -0.42, 1.0, 0.42, 0.42),
     part(fabric, -1.0, 0.42, -0.42, 1.0, 0.84, -0.22),
     part(fabric, -1.0, 0.42, -0.22, -0.82, 0.64, 0.42), part(fabric, 0.82, 0.42, -0.22, 1.0, 0.64, 0.42),
     part(fabric, -0.82, 0.42, -0.22, -0.01, 0.54, 0.4)],
    if fallen then moved((p) => yawed(60.0 * draw(f, 4.0) - 30.0, p), v(0.35, 0.0 - 0.42, 0.95), [loosen(cushion)])
    else [cushion],
  ])

// A low table on four legs.
let coffeeTable = (f: Furnish): List<Solid> =>
  List.append([part(wood, -0.5, 0.38, -0.275, 0.5, 0.42, 0.275)],
    [(-0.46, -0.235), (0.42, -0.235), (-0.46, 0.195), (0.42, 0.195)] |> List.map((e) =>
      let (x, z) = e in part(dark, x, 0.0, z, x + 0.04, 0.38, z + 0.04)))

// A four-drawer filing cabinet; drawers maybe pulled out.
let cabinet = (f: Furnish): List<Solid> =>
  List.append([part(metal, -0.235, 0.0, -0.31, 0.235, 1.32, 0.29)],
    List.range(4.0) |> List.concatMap((k) =>
      let pull = if happens(f, 3.0 + k, 0.4) then 0.25 + 0.2 * draw(f, 7.0 + k) else 0.0 in
      drawer(-0.215, 0.03 + 0.32 * k, 0.43, 0.29, 0.29, pull)))

// A reception counter 2.4 m long: a wood front to a ledge, the staff's desk
// top behind it, a dead monitor on that.
let counter = (f: Furnish): List<Solid> => List.flatten([
  [part(wood, -1.2, 0.0, 0.3, 1.2, 1.05, 0.375),
   part(laminate, -1.22, 1.05, 0.17, 1.22, 1.1, 0.4),
   part(laminate, -1.2, 0.72, -0.375, 1.2, 0.75, 0.3),
   part(wood, -1.2, 0.0, -0.375, -1.17, 0.72, 0.3), part(wood, 1.17, 0.0, -0.375, 1.2, 0.72, 0.3)],
  moved((p) => yawed(180.0, p), v(0.3, 0.0, 0.0), monitor(0.75, 0.0)),
])

// Shelving 0.9 m wide: sides, a back, five boards, rows of books with
// gaps; with mess, some on the floor in front.
let bookshelf = (f: Furnish): List<Solid> =>
  let boards = [0.0, 0.43, 0.86, 1.29, 1.77] in
  let rows = List.range(3.0) |> List.concatMap((k) =>
    let y = 0.03 + 0.43 * k in
    let gap = 0.1 + 0.35 * draw(f, 10.0 + k) in
    let w = 0.1 + 0.15 * draw(f, 20.0 + k) in
    [part(books, -0.42, y, -0.14, gap - 0.42, y + 0.27, 0.08), part(books, gap - 0.42 + w, y, -0.14, 0.42, y + 0.25, 0.08)]) in
  let floor = List.range(3.0) |> List.filter((k) => happens(f, 30.0 + k, 0.8)) |> List.concatMap((k) =>
    moved((p) => yawed(90.0 * draw(f, 40.0 + k), p), v(0.4 * (2.0 * draw(f, 50.0 + k) - 1.0), 0.0, 0.35 + 0.2 * draw(f, 60.0 + k)),
      [loosen(part(books, -0.12, 0.0, -0.1, 0.12, 0.05 + 0.03 * k, 0.1))])) in
  List.flatten([
    [part(wood, -0.45, 0.0, -0.16, -0.42, 1.8, 0.16), part(wood, 0.42, 0.0, -0.16, 0.45, 1.8, 0.16),
     part(wood, -0.42, 0.0, -0.16, 0.42, 1.8, -0.14)],
    boards |> List.map((y) => part(wood, -0.42, y, -0.14, 0.42, y + 0.03, 0.16)),
    rows,
    floor,
  ])

// A water cooler: a white body, a tap and tray, its bottle on top, or
// lying on the floor beside it.
let cooler = (f: Furnish): List<Solid> =>
  let bottle = part(water, -0.13, 0.98, -0.13, 0.13, 1.42, 0.13) in
  List.flatten([
    [part(white, -0.16, 0.0, -0.16, 0.16, 0.98, 0.16),
     part(dark, -0.04, 0.74, 0.16, 0.04, 0.8, 0.21), part(dark, -0.1, 0.55, 0.16, 0.1, 0.58, 0.23)],
    if happens(f, 3.0, 0.7) then [loosen({ bottle with c: v(0.45, 0.13, 0.1), ay: v(0.0, 0.0, 0.22), az: v(0.0, -0.13, 0.0) })]
    else [bottle],
  ])

// A potted plant: a dark pot, blades of leaves fanned out of it (dead and
// brown, with mess), maybe knocked over.
let plant = (f: Furnish): List<Solid> =>
  let colour = if happens(f, 3.0, 0.9) then dead else leaf in
  let blades = List.range(5.0) |> List.concatMap((k) =>
    moved((p) => yawed(72.0 * k + 30.0 * draw(f, 10.0 + k), tilted(20.0 + 20.0 * draw(f, 20.0 + k), p)), v(0.0, 0.38, 0.0),
      [part(colour, -0.07, 0.0, -0.015, 0.07, 0.6 + 0.25 * draw(f, 30.0 + k), 0.015)])) in
  let pot = part(dark, -0.19, 0.0, -0.19, 0.19, 0.4, 0.19) in
  let whole = List.append([pot], blades) in
  // Over, it lies on its pot, less the blades that would go through the
  // floor.
  let over = (p: Vec3.t): Vec3.t => tilted(85.0, p) in
  if happens(f, 4.0, 0.25) then
    shifted(0.0, 0.0 - lowest(moved(over, v(0.0, 0.0, 0.0), [pot])), 0.0, moved(over, v(0.0, 0.0, 0.0), whole))
      |> List.filter((s) => lowest([s]) > -0.001)
  else whole

// A bed 2 m long down its z (its foot to the front): a frame, a
// headboard, the mattress (askew, with mess), a pillow and a blanket.
let bed = (f: Furnish): List<Solid> =>
  let askew = (parts: List<Solid>) => moved((p) => yawed(8.0 * swing(f, 3.0), p), v(0.15 * swing(f, 4.0), 0.0, 0.1 * swing(f, 5.0)), parts) in
  List.flatten([
    [part(wood, -0.7, 0.1, -1.0, 0.7, 0.3, 1.0), part(wood, -0.72, 0.0, -1.06, 0.72, 1.0, -1.0)],
    [(-0.7, 0.94), (0.64, 0.94)] |> List.map((e) => let (x, z) = e in part(dark, x, 0.0, z, x + 0.06, 0.1, z + 0.06)),
    askew([part(white, -0.68, 0.3, -0.98, 0.68, 0.5, 0.98), part(white, -0.4, 0.5, -0.95, 0.4, 0.6, -0.62),
           part(fabric, -0.7, 0.5, -0.4, 0.7, 0.53, 0.98)]),
  ])

// A backless bench on two slab legs.
let bench = (f: Furnish): List<Solid> =>
  [part(wood, -0.8, 0.4, -0.2, 0.8, 0.46, 0.2), part(metal, -0.7, 0.0, -0.18, -0.64, 0.4, 0.18), part(metal, 0.64, 0.0, -0.18, 0.7, 0.4, 0.18)]

// A shut kiosk: a counter round its stock, posts holding up a canopy, a
// blank sign band on its front.
let kiosk = (f: Furnish): List<Solid> => List.flatten([
  [part(white, -1.0, 0.0, -0.6, 1.0, 1.0, 0.6), part(laminate, -1.04, 1.0, -0.64, 1.04, 1.04, 0.64)],
  [(-1.0, -0.6), (0.94, -0.6), (-1.0, 0.54), (0.94, 0.54)] |> List.map((e) =>
    let (x, z) = e in part(metal, x, 1.04, z, x + 0.06, 2.3, z + 0.06)),
  [part(wood, -1.15, 2.3, -0.75, 1.15, 2.45, 0.75), part(dark, -1.1, 2.02, 0.6, 1.1, 2.3, 0.63)],
])

let partsOf = (f: Furnish): List<Solid> =>
  match f.piece with
  | Desk => desk(f)
  | Chair => chair(f)
  | SideChair => sideChair(f)
  | Stack => stack(f)
  | Couch => couch(f)
  | CoffeeTable => coffeeTable(f)
  | Cabinet => cabinet(f)
  | Counter => counter(f)
  | Bookshelf => bookshelf(f)
  | Cooler => cooler(f)
  | Plant => plant(f)
  | Bed => bed(f)
  | Bench => bench(f)
  | Kiosk => kiosk(f)

// ---- In the room -----------------------------------------------------------------

// Where `f` goes in its room's space, from the room's floor: turned to
// face its side and knocked off its place by its mess, then nudged inside
// `r` (the room inside its walls), 2 cm clear, where that pushed it into a
// wall. As (turn, offset) for `moved`.
let gap = 0.02
let into = (lo: float, hi: float, b0: float, b1: float): float =>
  if b0 < lo + gap then lo + gap - b0 else if b1 > hi - gap then hi - gap - b1 else 0.0
let poseIn = (r: Rect, f: Furnish): ((Vec3.t) => Vec3.t, Vec3.t) =>
  let d = driftOf(f.piece) in
  let turn = (p: Vec3.t): Vec3.t => yawed(yawOf(f.facing) + d.spin * swing(f, 0.0), p) in
  let by = v(f.x + d.slide * swing(f, 1.0), 0.0, f.z + d.slide * swing(f, 2.0)) in
  let b = boundsOf(moved(turn, by, partsOf(f))) in
  (turn, by |> Vec3.add(v(into(r.x0, r.x1, b.x0, b.x1), 0.0, into(r.z0, r.z1, b.z0, b.z1))))
let placedIn = (r: Rect, f: Furnish): List<Solid> =>
  let (turn, by) = poseIn(r, f) in moved(turn, by, partsOf(f))
// Its footprint: the box round all but its loose boxes, turned with it.
let footprintIn = (r: Rect, f: Furnish): Solid =>
  let (turn, by) = poseIn(r, f) in
  let b = boundsOf(partsOf(f) |> List.filter((s) => not s.loose)) in
  moved(turn, by, [part(0.0, b.x0, b.y0, b.z0, b.x1, b.y1, b.z1)]) |> List.head |> Option.defaultValue(part(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0))
// Its faces, and its collider: the bounds of all but its loose boxes.
let facesOf = (r: Rect, f: Furnish): List<Slab> => unseen(placedIn(r, f))
let solidOf = (r: Rect, f: Furnish): Bounds => boundsOf(placedIn(r, f) |> List.filter((s) => not s.loose))

// Room `i`'s pieces.
let inRoom = (l: Layout, i: float): List<Furnish> => l.furniture |> List.filter((f) => f.room == i)

// ---- Tests -----------------------------------------------------------------------

let anywhere = rect(-100.0, -100.0, 100.0, 100.0)
let tidy = (p: Article): Furnish => { room: 0.0, piece: p, x: 0.0, z: 0.0, facing: North, mess: 0.0 }
let messy = (p: Article, x: float): Furnish => { tidy(p) with x: x, mess: 1.0 }
let pieces = [Desk, Chair, SideChair, Stack, Couch, CoffeeTable, Cabinet, Counter, Bookshelf, Cooler, Plant, Bed, Bench, Kiosk]
let dot = (a: Vec3.t, b: Vec3.t): float => Vec3.dot(a, b)
// Every face faces out of its box.
expect pieces |> List.all((p) => partsOf(tidy(p)) |> List.all((s) => slabsOf(s) |> List.all((f) =>
  dot(f.a |> Vec3.cross(f.b), f.o |> Vec3.add(Vec3.scale(0.5, f.a)) |> Vec3.add(Vec3.scale(0.5, f.b)) |> Vec3.sub(s.c)) > 0.0)))
// Tidy, every piece stands on the floor where it was put, square to it.
expect pieces |> List.all((p) => let b = solidOf(anywhere, tidy(p)) in Math.abs(b.y0) < 0.001 && b.x0 < 0.0 && b.x1 > 0.0 && b.z0 < 0.0 && b.z1 > 0.0)
expect solidOf(anywhere, tidy(Desk)) == boundsOf(desk(tidy(Desk)))
// Messy, nothing sinks into the floor, and the same entry always comes out
// the same.
expect pieces |> List.all((p) => List.range(8.0) |> List.all((k) => lowest(placedIn(anywhere, messy(p, k))) > -0.001))
let numbersOf = (fs: List<Slab>): List<float> =>
  fs |> List.concatMap((f) => List.append([f.swatch], [f.o, f.a, f.b] |> List.concatMap((p) => [Vec3.x(p), Vec3.y(p), Vec3.z(p)])))
expect numbersOf(facesOf(anywhere, messy(Chair, 3.0))) == numbersOf(facesOf(anywhere, messy(Chair, 3.0)))
// Over a few places, full mess tips a chair, pulls a drawer and turns a
// desk; none of it at no mess.
let tipped = (f: Furnish): bool => (solidOf(anywhere, f).y1 - solidOf(anywhere, f).y0) < 0.9
expect List.range(8.0) |> List.any((k) => tipped(messy(Chair, k)))
expect not tipped(tidy(Chair))
expect List.range(8.0) |> List.any((k) => solidOf(anywhere, messy(Cabinet, k)).z1 > 0.5)
expect solidOf(anywhere, tidy(Cabinet)).z1 < 0.4
