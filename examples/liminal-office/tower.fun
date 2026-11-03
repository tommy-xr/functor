// The tower archetype, derived from towerLayout.fun: its two bakes' cell
// models stacked into floors, its colliders a floor at a time, and its
// street side (massing, a window grid on every facade, the plant room and
// terrace on the roof, a beacon). archetypes.fun holds one tower per height,
// every one drawing the same models and lightmaps. Not hashed into the
// bakes' provenance: the bakes don't depend on it.

open Layout

let v = Vec3.make
let storey = TowerLayout.storey

// The bakes' interiors: the typical floor's and the ground and roof's.
let typicalInterior = Building.interiorOf("tower-typical", TowerLayout.typical)
let endsInterior = Building.interiorOf("tower-ends", TowerLayout.ends)

// The tallest tower's typical floors. Every tower's collar rises over the
// eyes on that tower's roof: `Scene.cells` draws everything for an eye
// outside every cell, and a neighbour's roof is within its reach.
let maxFloors = 16.0
let collarTop = (maxFloors + 1.0) * storey + 4.0

// Floor `k`'s room `j`'s cell, in a tower (ground k = 0, roof k = n + 1).
let cellOf = (k: float, j: float): float => k * TowerLayout.cellsPerFloor + j
// The bake layouts' rooms, by id: in `typical`, the floor's rooms follow
// the shaft below (1..8), the apron and west yard the shaft above (10, 11);
// in `ends`, the ground's are 0..7, the roof's follow two shafts (10, 11).
let typicalFirst = 1.0
let typicalApron = 10.0
let typicalYard = 11.0
let endsRoofFirst = 10.0
// The ground floor's apron and west yard.
let apron = 0.0
let westYard = 6.0

// ---- Drawing ---------------------------------------------------------------------

// Where a bake's cell model draws in a tower: its cell, and how far up.
type Placed = { model: Building.CellModel, cell: float, dy: float, day: Option.t<Asset.Texture> }

// A tower of `n` typical floors: the ground and roof from the ends bake,
// and the typical floor's models once per floor, `storey` apart, the same
// `Model.static` values every time (one GPU copy).
let placedOf = (n: float, typical: Provenance.Baked, ends: Provenance.Baked): List<Placed> =>
  let place = (b: Provenance.Baked, p: Building.CellModel, cell: float, dy: float): Placed => { model: p, cell: cell, dy: dy, day: b.day } in
  let roofLift = (n + 1.0 - TowerLayout.endsRoof) * storey in
  List.flatten([
    ends.models |> List.filter((p) => p.cell < TowerLayout.cellsPerFloor) |> List.map((p) => place(ends, p, p.cell, 0.0)),
    List.range(n) |> List.concatMap((k) =>
      typical.models |> List.concatMap((p) =>
        let dy = k * storey in
        if p.cell >= typicalFirst && p.cell < typicalFirst + TowerLayout.cellsPerFloor then [place(typical, p, cellOf(k + 1.0, p.cell - typicalFirst), dy)]
        else if p.cell == typicalApron then [place(typical, p, apron, dy)]
        else if p.cell == typicalYard then [place(typical, p, westYard, dy)]
        else [])),
    ends.models |> List.filter((p) => p.cell >= endsRoofFirst) |> List.map((p) => place(ends, p, cellOf(n + 1.0, p.cell - endsRoofFirst), roofLift)),
  ])

// A placed model in its cell, by night, and under its day lightmap.
let nightOf = (p: Placed): Scene.t =>
  Scene.modelOf(p.model.model) |> Scene.translate(v(0.0, p.dy, 0.0)) |> Scene.inCells([p.cell])
let dayOf = (p: Placed): Scene.t => Provenance.underDay(p.day, nightOf(p))
// A bake's models as they are, under both lightmaps, to keep resident.
let residentOf = (b: Provenance.Baked): List<Scene.t> =>
  b.models |> List.concatMap((p) => let s = Scene.modelOf(p.model) in [s, Provenance.underDay(b.day, s)])

// ---- Collision -------------------------------------------------------------------

// Bake layout `l`'s colliders on `level`, from its interior `k`: its rooms'
// walls, its flights' parapets and ramps, the balustrade of a flight ending
// on it, and its floors.
let levelColliders = (l: Layout, k: Building.Interior, level: float): List<Building.Collider> =>
  let flights = l.flights |> List.indexedMap((j, f) => (j, f)) in
  let from = flights |> List.filter((e) => let (j, f) = e in Building.roomAt(l, f.lower).level == level) in
  let blocks = List.flatten([
    List.range(List.length(l.rooms)) |> List.filter((i) => Building.roomAt(l, i).level == level) |> List.concatMap((i) => Building.wallBlocks(l, i)),
    from |> List.concatMap((e) => let (j, f) = e in Building.flightBlocks(l, j, f)),
    flights |> List.concatMap((e) => let (j, f) = e in if Building.roomAt(l, f.upper).level == level then Building.balustradeBlocks(l, j, f) else []),
  ]) in
  let floors = Building.floorFacesOf(l, k.allFaces) |> List.filter((f) => Building.near(Vec3.y(f.q.o), level * l.storey)) in
  Building.collidersOf({ l with flights: from |> List.map((e) => let (j, f) = e in f) }, blocks, floors)
// `cs` moved up `dy`, as floor `k`'s.
let onFloor = (k: float, dy: float, cs: List<Building.Collider>): List<Building.Collider> =>
  cs |> List.map((c) => { c with tag: $"{c.tag}@{k}", at: c.at |> Vec3.add(v(0.0, dy, 0.0)) })

// Each level's colliders built once, in its bake's space.
let groundColliders = levelColliders(TowerLayout.ends, endsInterior, 0.0)
let typicalColliders = levelColliders(TowerLayout.typical, typicalInterior, 1.0)
let roofColliders = levelColliders(TowerLayout.ends, endsInterior, TowerLayout.endsRoof)

// A box collider over [x0, x1] × [y0, y1] × [z0, z1].
let boxCollider = (tag: string, b: Exterior.Box): Building.Collider =>
  { tag: tag, shape: Physics.box(b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0), at: v((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) }

// ---- The roof ----------------------------------------------------------------------

// On a tower of `n` floors, whose roof is at `y`: the roof slab, open over
// the stairwell; the plant room over the rest of the core and north of it,
// its wall round the stair's head room with that room's door; the head
// room's roof, open over its skylight; a planter on the terrace; the
// beacon. The plant room stands flush with the parapet on three sides.
let roofTop = (n: float): float => (n + 1.0) * storey
let plantHeight = storey
let slabOf = (y: float, x0: float, z0: float, x1: float, z1: float): Exterior.Box => Exterior.box(x0, y - 0.3, z0, x1, y, z1)
let roofSlabs = (n: float): List<Exterior.Box> =>
  let y = roofTop(n) in
  let s = TowerLayout.stairRect in
  let f = TowerLayout.footprint in
  let o = TowerLayout.facade / 4.0 in
  [slabOf(y, f.x0 - o, f.z0 - o, f.x1 + o, s.z0),
   slabOf(y, f.x0 - o, s.z1, f.x1 + o, f.z1 + o),
   slabOf(y, f.x0 - o, s.z0, s.x0, s.z1),
   slabOf(y, s.x1, s.z0, f.x1 + o, s.z1)]
// The plant room: three blocks round the stair's head room.
let plantRoom = (n: float): List<Exterior.Box> =>
  let y = roofTop(n) in
  let s = TowerLayout.stairRect in
  let f = TowerLayout.footprint in
  [Exterior.box(f.x0, y, s.z0, s.x0, y + plantHeight, f.z1),
   Exterior.box(s.x1, y, s.z0, f.x1, y + plantHeight, f.z1),
   Exterior.box(s.x0, y, s.z1, s.x1, y + plantHeight, f.z1)]
let plantBoxes = (n: float): List<Exterior.Box> =>
  let y = roofTop(n) in
  let top = y + plantHeight in
  let s = TowerLayout.stairRect in
  let skylight = skylightSlot in
  // The head room's south wall's outer half, round its door.
  let doors = TowerLayout.roof.doors |> List.map(Building.holeOf) in
  let wall = Building.pieces(s.x0, s.x1, plantHeight, doors) |> List.map((p) => Exterior.box(p.u0, y + p.y0, s.z0 - TowerLayout.partition / 2.0, p.u1, y + p.y1, s.z0)) in
  // Its roof, 5 cm over its ceiling, round the slot.
  let h0 = y + TowerLayout.ceiling + 0.05 in
  List.flatten([
    plantRoom(n),
    wall,
    [Exterior.box(s.x0, h0, s.z0, s.x1, top, skylight.z0),
     Exterior.box(s.x0, h0, skylight.z1, s.x1, top, s.z1),
     Exterior.box(s.x0, h0, skylight.z0, skylight.x0, top, skylight.z1),
     Exterior.box(skylight.x1, h0, skylight.z0, s.x1, top, skylight.z1)],
  ])
// The head room's skylight slot, in the ends bake's space (x and z are the tower's).
let skylightSlot =
  let l = TowerLayout.ends in
  match l.fixtures |> List.find((f) => f.sign == Skylight) with
  | Option.Some(f) => Building.slotOf(l, f)
  | Option.None => Binary.invalid("the tower's roof has no skylight")
let planter = (n: float): Exterior.Box => Exterior.box(16.0, roofTop(n), 2.0, 22.0, roofTop(n) + 0.6, 4.0)
let beacon = (n: float): Exterior.Box =>
  Exterior.box(19.85, roofTop(n) + plantHeight, 19.85, 20.15, roofTop(n) + plantHeight + 0.3, 20.15)

// The roof's own colliders: the terrace's floor and parapet, the plant room
// and the planter (the head room's walls are the ends bake's).
let roofExtras = (n: float): List<Building.Collider> =>
  let y = roofTop(n) in
  let f = TowerLayout.footprint in
  let terrace = TowerLayout.roof.rooms |> List.find((r) => r.kind == Roof) |> Option.map((r) => r.rect) |> Option.defaultValue(f) in
  let t = TowerLayout.facade / 2.0 in
  let top = y + TowerLayout.parapet in
  List.flatten([
    [boxCollider("terrace", Exterior.box(terrace.x0, y - 0.3, terrace.z0, terrace.x1, y, terrace.z1)),
     boxCollider("parapet-s", Exterior.box(f.x0 - t, y, f.z0 - t, f.x1 + t, top, f.z0 + t)),
     boxCollider("parapet-w", Exterior.box(f.x0 - t, y, f.z0, f.x0 + t, top, terrace.z1)),
     boxCollider("parapet-e", Exterior.box(f.x1 - t, y, f.z0, f.x1 + t, top, terrace.z1)),
     boxCollider("planter", planter(n))],
    plantRoom(n) |> List.indexedMap((i, b) => boxCollider($"plant-{i}", b)),
  ])

// Every collider of a tower of `n` floors, in its space.
let collidersOf = (n: float): List<Building.Collider> => List.flatten([
  onFloor(0.0, 0.0, groundColliders),
  List.range(n) |> List.concatMap((k) => onFloor(k + 1.0, k * storey, typicalColliders)),
  onFloor(n + 1.0, (n + 1.0 - TowerLayout.endsRoof) * storey, roofColliders),
  roofExtras(n),
])

// ---- The street side ---------------------------------------------------------------

// The window grid's bays along every facade, and how far it stands proud.
let bays = List.range(15.0) |> List.map((k) => 0.8 + 1.6 * k)
let gridProud = 0.1
// Whether bay `u` on side `s` of floor `k` is lit, at night.
let isLit = (s: Side, k: float, u: float): bool =>
  let side = (match s with
    | South => 0.0
    | West => 1.0
    | North => 2.0
    | East => 3.0) in
  Building.noise(side * 977.0 + k * 61.0 + u * 7.0) < 0.4
// A window in every bay of floors 1..n, but where a real one is: a glass
// quad on the facade's outer face, `gridProud` out, sill to head; lit ones
// first.
let gridFaces = (l: Layout, n: float): List<(Building.Face, bool)> =>
  let f = l.footprint in
  let o = l.facade / 2.0 + gridProud in
  let isReal = (s: Side, k: float, u: float): bool =>
    l.windows |> List.any((w) => w.side == s && Building.roomAt(l, w.room).level == k && Math.abs(w.at - u) < 0.1) in
  let quad = (s: Side, y: float, u: float): Building.Face =>
    let y0 = y + 0.9 in
    let y1 = y + 2.3 in
    (match s with
      | South => Building.wallZ(0.0, u - 0.6, u + 0.6, y0, y1, f.z0 - o, false)
      | North => Building.wallZ(0.0, u - 0.6, u + 0.6, y0, y1, f.z1 + o, true)
      | West => Building.wallX(0.0, u - 0.6, u + 0.6, y0, y1, f.x0 - o, false)
      | East => Building.wallX(0.0, u - 0.6, u + 0.6, y0, y1, f.x1 + o, true)) in
  let all = List.range(n) |> List.concatMap((k0) =>
    let k = k0 + 1.0 in
    [South, West, North, East] |> List.concatMap((s) =>
      bays |> List.filter((u) => not isReal(s, k, u)) |> List.map((u) => (quad(s, k * l.storey, u), isLit(s, k, u))))) in
  List.flatten([all |> List.filter((e) => let (q, lit) = e in lit), all |> List.filter((e) => let (q, lit) = e in not lit)])

let glassLit = Building.paint(158.0, 168.0, 128.0)
let glassDark = Building.paint(16.0, 16.0, 24.0)
let glassDay = Building.paint(76.0, 84.0, 96.0)
let unlitGroup = (first: float, count: float, p: Building.Paint): Model.group =>
  { first: first, count: count, texture: Option.Some(Texture.fromPixels(Building.solid(p))), unlit: true }
// The grid as one mesh, two groups by night and one by day.
let gridOf = (l: Layout, n: float): Exterior.Grid =>
  let quads = gridFaces(l, n) in
  let faces = quads |> List.map((e) => let (q, lit) = e in q) in
  let lit = 2.0 * List.length(quads |> List.filter((e) => let (q, on) = e in on)) in
  let all = 2.0 * List.length(faces) in
  let data = { Building.meshOf(faces, []) with groups: [unlitGroup(0.0, lit, glassLit), unlitGroup(lit, all - lit, glassDark)] } in
  { night: Scene.modelOf(Model.static(data)), day: Scene.modelOf(Model.static({ data with groups: [unlitGroup(0.0, all, glassDay)] })) }

let planterGreen = Building.rgb(58.0, 70.0, 56.0)
// A tower's street side: the kit's (facade, roof, entrance canopy, neon,
// lamps), the roof slab opened over the stairwell, the plant room and
// planter, the beacon and the window grid.
let streetOf = (l: Layout, n: float): Exterior.Street =>
  let s = Exterior.streetOf(l) in
  let extras = List.flatten([plantBoxes(n) |> List.map((b) => (b, Exterior.concrete)), [(planter(n), planterGreen)]]) in
  let boxes = List.flatten([
    s.streetBoxes |> List.filter((e) => let (b, _) = e in b != s.roofBox),
    roofSlabs(n) |> List.map((b) => (b, Exterior.roofGrey)),
    extras,
  ]) in
  { s with streetBoxes: boxes, streetSide: Exterior.instancedOf(boxes), extraBoxes: extras,
           beaconBoxes: [beacon(n)], grid: Option.Some(gridOf(l, n)) }
