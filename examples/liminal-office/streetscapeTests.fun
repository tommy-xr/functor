// The streetscape kit's checks (streetscape.fun, roads.fun).

open Layout

let office = Archetypes.office
let ol = office.layout
let tower = Archetypes.tower8
let officeProps = Streetscape.officeProps(ol, office.street)
let towerProps = Streetscape.towerProps(tower.layout, tower.street)
let cinemaProps = Cinema.streetscapeProps(Cinema.exterior)
let mallProps = Mall.streetscapeProps(Mall.street)
let houseProps = Streetscape.houseProps(HouseLayout.layout)
let dressings = [(ol, officeProps, Streetscape.officePaths), (tower.layout, towerProps, Streetscape.towerPaths),
                 (CinemaLayout.layout, cinemaProps, Cinema.paths), (MallLayout.layout, mallProps, Mall.paths),
                 (HouseLayout.layout, houseProps, Streetscape.housePaths)]
// The mall's lot is the kit's: three double rows of 32 bays, a wheel stop
// each; a house's frontage cuts round its front path.
expect List.length(Mall.lotPlan.rows) == 6.0 && Mall.lotPlan.n == 32.0
let gate = HouseExterior.gateOf(HouseLayout.layout)
expect houseProps.skinned |> List.all((e) => let (b, _) = e in b.x1 <= gate.lo || b.x0 >= gate.hi)

// Nothing solid blocks the e2e's walks (each archetype's, in its space).
expect dressings |> List.all((e) =>
  let (l, p, paths) = e in p.solid |> List.all((b) => not (paths |> List.any((q) => Streetscape.blocks(b, q)))))

// Clear of every door and real window: no solid box, and no box or decal
// the street side adds below 2.5 m, stands within a metre in front of one.
let fronts = (l: Layout): List<Exterior.Box> =>
  [South, North, West, East] |> List.concatMap((s) =>
    let fr = Streetscape.sideFrame(l, s) in
    Building.levelsOf(l) |> List.filter((level) => level == 0.0) |> List.concatMap((level) =>
      Building.openingsIn(l, level, s) |> List.map((h) => Streetscape.boxIn(fr, h.lo, h.hi, h.bottom, h.top, 0.0, 1.0))))
let meets = (a: Exterior.Box, b: Exterior.Box): bool =>
  a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1 && a.z0 < b.z1 && b.z0 < a.z1
let boundsOf = (q: Building.Quad): Exterior.Box =>
  let cs = Building.corners(q) in
  let lo = (f: (Vec3.t) => float) => cs |> List.map(f) |> List.minimum |> Option.defaultValue(0.0) in
  let hi = (f: (Vec3.t) => float) => cs |> List.map(f) |> List.maximum |> Option.defaultValue(0.0) in
  Exterior.box(lo(Vec3.x) - 0.01, lo(Vec3.y), lo(Vec3.z) - 0.01, hi(Vec3.x) + 0.01, hi(Vec3.y), hi(Vec3.z) + 0.01)
let streetLevel = (p: Streetscape.Props): List<Exterior.Box> => List.flatten([
  p.solid,
  List.flatten([p.plain, p.fine]) |> List.map((e) => let (b, _) = e in b) |> List.filter((b) => b.y1 > Streetscape.walk + 0.02 && b.y0 < 2.5),
  p.decals |> List.filter((d) => d.kind != Streetscape.Pane) |> List.map((d) => boundsOf(d.q)) |> List.filter((b) => b.y0 < 2.5 && b.y1 > Streetscape.walk + 0.02),
])
expect dressings |> List.all((e) =>
  let (l, p, _) = e in streetLevel(p) |> List.all((b) => not (fronts(l) |> List.any((o) => meets(b, o)))))

// Caps: a street side adds at most this many boxes, decals and colliders.
let boxesOf = (p: Streetscape.Props): float => List.length(p.plain) + List.length(p.fine) + List.length(p.skinned)
expect dressings |> List.all((e) => let (_, p, _) = e in boxesOf(p) <= 1600.0 && List.length(p.decals) <= 160.0 && List.length(p.solid) <= 160.0)
// What shells keep per copy (plain and paving): at most 400 boxes, so the
// grid of 64 stays under 26k shell instances of it.
expect [officeProps, towerProps] |> List.all((p) => List.length(p.plain) + List.length(p.skinned) <= 400.0)

// The sidewalk: every slab and curb from the road to the walk's top, the
// curb 15 cm above the road.
expect Math.abs(Streetscape.walk - Streetscape.road - 0.15) < 0.0001
expect officeProps.skinned |> List.all((e) => let (b, _) = e in b.y0 == Streetscape.road && b.y1 == Streetscape.walk)
// `less` keeps the area outside its cuts: a 10 × 10 less a 2 × 2 inside it.
let area = (rs: List<Rect>): float => rs |> List.map((r) => (r.x1 - r.x0) * (r.z1 - r.z0)) |> List.sum
expect Math.abs(area(Streetscape.less(rect(0.0, 0.0, 10.0, 10.0), [rect(4.0, 4.0, 6.0, 6.0)])) - 96.0) < 0.0001
// Every ramp lies in the ring, outside the slabs (it cuts them).
let inRect = (r: Rect, x: float, z: float): bool => x >= r.x0 && x <= r.x1 && z >= r.z0 && z <= r.z1
expect Streetscape.rampsOf(ol) |> List.all((k) => officeProps.skinned |> List.all((e) =>
  let (b, _) = e in not inRect(rect(b.x0 + 0.01, b.z0 + 0.01, b.x1 - 0.01, b.z1 - 0.01), (k.r.x0 + k.r.x1) / 2.0, (k.r.z0 + k.r.z1) / 2.0)))
// The block's crosswalks land on the copies' ramps: across the arm east of
// the crossroads, from copy 0's south-west corner to copy 1's.
let crosswalk = Roads.crossroads(Block.centre, Block.centre, Block.streetWidth, ol.facade / 2.0, Roads.blockReach) |> List.head
  |> Option.map((e) => let (b, _) = e in (b.x0, b.x1)) |> Option.defaultValue((0.0, 0.0))
let rampsIn = (c: Block.Copy): List<(float, float)> => Streetscape.rampsOf(ol) |> List.map((k) =>
  let a = Block.toWorld(c, Vec3.make(k.r.x0, 0.0, k.r.z0)) in
  let b = Block.toWorld(c, Vec3.make(k.r.x1, 0.0, k.r.z1)) in
  (Math.min(Vec3.x(a), Vec3.x(b)), Math.max(Vec3.x(a), Vec3.x(b))))
let near = (a: (float, float), b: (float, float)): bool =>
  let (a0, a1) = a in
  let (b0, b1) = b in
  Math.abs(a0 - b0) < 0.001 && Math.abs(a1 - b1) < 0.001
expect [0.0, 1.0] |> List.all((id) => rampsIn(Block.copyAt(id)) |> List.any((r) => near(r, crosswalk)))

// Windows by night: sparse, but at least one lit, on the office's bands
// (the shells light the same ones) and on a tower's grid.
let bandIds = Building.levelsOf(ol) |> List.concatMap((level) => [South, North, West, East] |> List.concatMap((s) =>
  Exterior.bandHoles(ol, level, s) |> List.map((h) => Streetscape.isLitBand(level, s, (h.lo + h.hi) / 2.0))))
let litOf = (lits: List<bool>): float => List.length(lits |> List.filter((x) => x))
expect litOf(bandIds) >= 1.0 && litOf(bandIds) <= 0.15 * List.length(bandIds)
expect List.length(Shells.shellAt("office").litBands) == litOf(bandIds)
let bays = Tower.gridFaces(tower.layout, 8.0) |> List.map((e) => let (_, id) = e in Streetscape.isLit(id))
expect litOf(bays) >= 1.0 && litOf(bays) <= 0.15 * List.length(bays)
// Some ground-floor bands are FOR LEASE.
expect List.length(Streetscape.bandPanesOf(ol) |> List.filter((d) => d.day == StreetAtlas.bandLease)) >= 1.0

// The picket fence the houses will stand behind: pickets 0.15 m apart, a
// metre tall, one collider.
let pickets = Streetscape.picket(Streetscape.sideFrame(ol, North), 0.0, 3.0, 1.0, Streetscape.walk)
expect List.length(pickets.plain) == 2.0 * 20.0 + 2.0 && List.length(pickets.solid) == 1.0
