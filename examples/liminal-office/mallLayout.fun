// The mall archetype's plain data: a two-level concourse 76 × 22 m round a
// skylit atrium court, shops off it, a food court at the west end and an
// anchor store's blank wall at the east end; behind the north shops, their
// stockrooms and a service corridor on each level; the parking lot south of
// it. mallInterior.fun derives the baked surfaces and lights, mall.fun the
// cells, colliders and street side. Editing this file makes the mall's
// committed bake stale: rebake (README).
//
// Metres, Y up, +Z north. As in layout.fun, rects run between wall centre
// lines and a cell is a rect. The court's void is ONE full-height cell; a
// ring of strips round it on each level (a ring isn't convex): the lower
// concourse under the galleries, the galleries above.

open Layout

let v = Vec3.make

let footprint = rect(0.0, 0.0, 76.0, 48.0)
// Floor to floor; each level's ceiling is `ceiling` over its floor, the
// stockrooms' and service corridors' `lowCeiling`.
let storey = 4.5
let ceiling = 4.0
let lowCeiling = 2.7
// The roof slab's top, where the skylight pyramid stands, and its apex.
let roof = 2.0 * storey
let apex = 13.0
let facadeHeight = roof + 1.0
let partition = 0.2
let facade = 0.3

// `Lot`: the street (a collar strip). `Void`: the court, both levels and up
// into the skylight, open on every side. `Stock`: a shop's stockroom.
// `Service`: the corridor behind the north shops.
type CellKind =
  | Lot
  | Entry
  | Hall
  | Strip
  | Void
  | Shop
  | Stock
  | Service

type Cell = { name: string, kind: CellKind, level: float, rect: Rect, panels: List<Panel> }

let cell = (name: string, kind: CellKind, level: float, r: Rect, panels: List<Panel>): Cell =>
  { name: name, kind: kind, level: level, rect: r, panels: panels }

// Ceiling panels on a grid, lit in a checkerboard (the other half dark).
let gridOf = (xs: List<float>, zs: List<float>, parity: float): List<Panel> =>
  xs |> List.indexedMap((i, x) => zs |> List.indexedMap((j, z) => { x: x, z: z, on: Math.mod(i + j + parity, 2.0) == 0.0 })) |> List.flatten
// Panels along a strip at `z`, lit where `lit` holds.
let rowOf = (z: float, xs: List<float>, lit: (float) => bool): List<Panel> =>
  xs |> List.indexedMap((i, x) => { x: x, z: z, on: lit(i) })
let even = (i: float): bool => Math.mod(i, 2.0) == 0.0
let odd = (i: float): bool => not even(i)
let courtXs = [29.0, 35.0, 41.0, 47.0, 53.0]
let hallZs = [14.5, 21.0, 27.5]

// The court: the void, and round it the strips, 5 m along the long sides,
// 4 m across its ends.
let court = rect(26.0, 10.0, 56.0, 32.0)
let voidRect = rect(30.0, 15.0, 52.0, 27.0)

// ---- Shops ------------------------------------------------------------------------

// An open shop: a bay of the north shopfronts, 8 × 10 m behind its front at
// `at`, and its stockroom behind it; the service corridor runs behind
// those. Shops with the same fit-out are the same geometry, moved: one bake
// of each (mallInterior.fun's `homeOf`). Racks: empty clothing racks and a
// counter. Screens: a dead electronics display. Boxes: stacked cartons and
// "CLOSING" banners.
type FitOut =
  | Racks
  | Screens
  | Boxes
type Bay = { level: float, at: float, fit: FitOut }
let bays = [
  { level: 0.0, at: 13.0, fit: Racks }, { level: 0.0, at: 33.0, fit: Screens },
  { level: 0.0, at: 49.0, fit: Boxes }, { level: 0.0, at: 70.0, fit: Racks },
  { level: 1.0, at: 5.0, fit: Boxes }, { level: 1.0, at: 21.0, fit: Screens },
  { level: 1.0, at: 41.0, fit: Racks }, { level: 1.0, at: 62.0, fit: Boxes },
]
let shopZ0 = court.z1
let stockZ0 = shopZ0 + 10.0
let serviceZ0 = stockZ0 + 3.6
let shopRect = (b: Bay): Rect => rect(b.at - 4.0, shopZ0, b.at + 4.0, stockZ0)
let stockRect = (b: Bay): Rect => rect(b.at - 4.0, stockZ0, b.at + 4.0, serviceZ0)
let serviceRect = rect(footprint.x0, serviceZ0, footprint.x1, footprint.z1)
// A shop's four panels, and which are lit: the dead display has one left on.
let shopPanels = (b: Bay): List<Panel> =>
  let on = (match b.fit with
    | Racks => [true, false, false, true]
    | Screens => [false, false, true, false]
    | Boxes => [true, true, false, false]) in
  [b.at - 2.0, b.at + 2.0] |> List.concatMap((x) => [shopZ0 + 2.5, stockZ0 - 2.5] |> List.map((z) => (x, z)))
    |> List.zip(on) |> List.map((e) => let (p, lit) = e in let (x, z) = p in { x: x, z: z, on: lit })
// Down the corridor every 4 m, two panels lit in three.
let servicePanels = (shift: float): List<Panel> =>
  rowOf((serviceZ0 + footprint.z1) / 2.0, List.range(19.0) |> List.map((k) => 2.0 + 4.0 * k), (i) => Math.mod(i + shift, 3.0) != 2.0)

// Cell ids are list positions. The apron is the lot's strip of the collar,
// the yard the east strip (the service exit opens onto it). Each bay adds
// its shop and stockroom.
let cells = List.flatten([[
  cell("apron", Lot, 0.0, rect(30.0, 0.0 - 12.0, 52.0, 0.0), []),
  cell("entrance", Entry, 0.0, rect(38.0, 0.0, 44.0, 10.0), [{ x: 41.0, z: 3.0, on: true }, { x: 41.0, z: 7.0, on: true }]),
  // The food court is the west hall's west end, under drum pendants.
  cell("west hall", Hall, 0.0, rect(0.0, 10.0, 26.0, 32.0), gridOf([16.0, 22.0], hallZs, 0.0)),
  cell("court S", Strip, 0.0, rect(26.0, 10.0, 56.0, 15.0), rowOf(12.5, courtXs, even)),
  cell("court N", Strip, 0.0, rect(26.0, 27.0, 56.0, 32.0), rowOf(29.5, courtXs, odd)),
  cell("court W", Strip, 0.0, rect(26.0, 15.0, 30.0, 27.0), [{ x: 28.0, z: 18.0, on: true }, { x: 28.0, z: 24.0, on: false }]),
  cell("court E", Strip, 0.0, rect(52.0, 15.0, 56.0, 27.0), [{ x: 54.0, z: 18.0, on: false }, { x: 54.0, z: 24.0, on: true }]),
  cell("east hall", Hall, 0.0, rect(56.0, 10.0, 76.0, 32.0), gridOf([60.0, 66.0, 72.0], hallZs, 1.0)),
  cell("court", Void, 0.0, voidRect, []),
  cell("west hall 2", Hall, 1.0, rect(0.0, 10.0, 26.0, 32.0), gridOf([4.0, 10.0, 16.0, 22.0], hallZs, 1.0)),
  cell("gallery S", Strip, 1.0, rect(26.0, 10.0, 56.0, 15.0), rowOf(12.5, courtXs, odd)),
  cell("gallery N", Strip, 1.0, rect(26.0, 27.0, 56.0, 32.0), rowOf(29.5, courtXs, even)),
  cell("bridge W", Strip, 1.0, rect(26.0, 15.0, 30.0, 27.0), [{ x: 28.0, z: 21.0, on: true }]),
  cell("bridge E", Strip, 1.0, rect(52.0, 15.0, 56.0, 27.0), [{ x: 54.0, z: 21.0, on: false }]),
  cell("east hall 2", Hall, 1.0, rect(56.0, 10.0, 76.0, 32.0), gridOf([60.0, 66.0, 72.0], hallZs, 0.0)),
  cell("service 0", Service, 0.0, serviceRect, servicePanels(0.0)),
  cell("service 1", Service, 1.0, serviceRect, servicePanels(1.0)),
  cell("yard", Lot, 0.0, rect(footprint.x1, 40.0, footprint.x1 + 30.0, footprint.z1), []),
], bays |> List.concatMap((b) => [
  cell($"shop {b.at} {b.level}", Shop, b.level, shopRect(b), shopPanels(b)),
  cell($"stock {b.at} {b.level}", Stock, b.level, stockRect(b), [{ x: b.at, z: (stockZ0 + serviceZ0) / 2.0, on: true }]),
])])

let apron = 0.0
let entrance = 1.0
let westHall = 2.0
let courtS = 3.0
let courtN = 4.0
let courtW = 5.0
let courtE = 6.0
let eastHall = 7.0
let courtVoid = 8.0
let westHall2 = 9.0
let galleryS = 10.0
let galleryN = 11.0
let bridgeW = 12.0
let bridgeE = 13.0
let eastHall2 = 14.0
let service0 = 15.0
let yard = 17.0
let serviceOf = (level: float): float => service0 + level
// Bay `k`'s shop and stockroom.
let firstBay = 18.0
let shopOf = (k: float): float => firstBay + 2.0 * k
let stockOf = (k: float): float => firstBay + 2.0 * k + 1.0

// Open-plan neighbours: no wall between them, and a portal over their whole
// shared face. The concourse is one hall on each level; the void opens onto
// every strip, the lower ones and the galleries.
let lower = [(westHall, courtS), (westHall, courtW), (westHall, courtN), (eastHall, courtS), (eastHall, courtE), (eastHall, courtN),
             (courtS, courtW), (courtS, courtE), (courtN, courtW), (courtN, courtE)]
let upper = [(westHall2, galleryS), (westHall2, bridgeW), (westHall2, galleryN), (eastHall2, galleryS), (eastHall2, bridgeE), (eastHall2, galleryN),
             (galleryS, bridgeW), (galleryS, bridgeE), (galleryN, bridgeW), (galleryN, bridgeE)]
let links = List.flatten([lower, upper, [courtS, courtN, courtW, courtE, galleryS, galleryN, bridgeW, bridgeE] |> List.map((s) => (courtVoid, s))])

// The north concourse cell on `level` that a shopfront at `at` opens onto.
let hallOf = (level: float, at: float): float =>
  if level == 0.0 then (if at < court.x0 then westHall else if at < court.x1 then courtN else eastHall)
  else if at < court.x0 then westHall2 else if at < court.x1 then galleryN else eastHall2

// Openings in walls: the entrance first, from the apron; the entrance hall
// into the court; the service exit, from the yard; and each bay's
// shopfront, its stockroom's door at the back's west end, and the
// stockroom's door into the service corridor at its east end. The two
// doors don't line up, so the corridor never sees the concourse.
let shopfront = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 6.0, height: 3.2 }
let staffDoor = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 0.9, height: 2.1 }
let staffOffset = 2.6
let exitAt = (serviceZ0 + footprint.z1) / 2.0
let doors = List.flatten([
  [{ a: apron, b: entrance, at: 41.0, width: 4.0, height: 3.2 },
   { a: entrance, b: courtS, at: 41.0, width: 6.0 - partition, height: ceiling },
   { a: yard, b: service0, at: exitAt, width: 1.0, height: 2.1 }],
  bays |> List.indexedMap((k, b) => [
    shopfront(shopOf(k), hallOf(b.level, b.at), b.at),
    staffDoor(shopOf(k), stockOf(k), b.at - staffOffset),
    staffDoor(stockOf(k), serviceOf(b.level), b.at + staffOffset),
  ]) |> List.flatten,
])

// The escalator pair, both lanes stopped: from the void's floor up west
// onto the west bridge. Each lane is `risers` steps of 30° between plates
// `plate` long; the pair's body spans [z0, z1], its lanes split by a
// balustrade down the middle. Lane 2 is barred off ("out of service").
type Escalator = { xTop: float, z0: float, z1: float, plate: float, risers: float, balustrade: float, rail: float }
let escalator = { xTop: voidRect.x0, z0: 15.6, z1: 18.6, plate: 1.2, risers: 23.0, balustrade: 0.15, rail: 1.0 }

// The fountain island on the court's floor: a basin, still water, a planter.
let basin = rect(43.0, 18.5, 50.0, 24.5)
let island = rect(45.6, 20.6, 47.4, 22.4)
// Planters against the galleries' balustrades, at these x.
let planterXs = [36.0, 42.0, 48.0]
// The food court: fixed tables, and drum pendants over them.
let tables = [3.0, 7.0, 11.0] |> List.concatMap((x) => [14.0, 18.5, 23.5, 28.0] |> List.map((z) => { x: x, z: z }))
let drums = [4.0, 10.0] |> List.concatMap((x) => [15.0, 21.0, 27.0] |> List.map((z) => { x: x, z: z }))
// The anchor store's dark entrance in the lower east hall's end wall: its
// span along the wall, and its height.
let anchorDoor = { lo: 17.0, hi: 25.0, bottom: 0.0, top: 3.6 }

// In the lower east hall, a shut kiosk under its canopy, benches round it
// and two plants.
let furniture = [
  { room: eastHall, piece: Kiosk, x: 66.0, z: 21.0, facing: South, mess: 0.0 },
  { room: eastHall, piece: Bench, x: 62.6, z: 21.0, facing: East, mess: 0.0 },
  { room: eastHall, piece: Bench, x: 69.4, z: 21.0, facing: West, mess: 0.0 },
  { room: eastHall, piece: Bench, x: 66.0, z: 17.6, facing: North, mess: 0.0 },
  { room: eastHall, piece: Plant, x: 63.2, z: 18.2, facing: North, mess: 0.4 },
  { room: eastHall, piece: Plant, x: 68.8, z: 23.8, facing: North, mess: 0.8 },
]

// ---- The lot ---------------------------------------------------------------------

// South of the mall: three double rows of bays, lamp posts on the first
// two medians, two by the apron. The bays run along x, 2.6 m wide, 5 m deep, in double rows with a
// median between and 7 m aisles.
let lotRect = rect(0.0 - 6.0, 0.0 - 65.0, 82.0, 0.0 - 12.0)
let bay = 2.6
let bayDepth = 5.0
// Each double row's median line; the bays are 5 m either side of it.
let medians = [0.0 - 24.0, 0.0 - 41.0, 0.0 - 58.0]
let lampHeads = List.flatten([
  [v(30.0, 4.8, 0.0 - 11.0), v(52.0, 4.8, 0.0 - 11.0)],
  [0.0 - 24.0, 0.0 - 41.0] |> List.concatMap((z) => [4.0, 20.0, 36.0, 52.0, 68.0] |> List.map((x) => v(x, 4.8, z))),
])

// The kit's view of the mall (layout.fun), for its street side and shells:
// every cell as a room, the doors through the facade (the entrance first),
// the lot's lamps. (Its kinds only steer exterior.fun: `Outside` marks the
// apron and the yard.)
let kindOf = (c: Cell): Kind =>
  match c.kind with
  | Lot => Outside
  | Entry => Lobby
  | Shop => Office
  | _ => Corridor
let isLot = (i: float): bool => List.nth(i, cells) |> Option.map((c) => c.kind == Lot) |> Option.defaultValue(false)
let layout: Layout = {
  footprint: footprint,
  ceiling: ceiling,
  storey: storey,
  levels: 2.0,
  facadeHeight: facadeHeight,
  partition: partition,
  facade: facade,
  rooms: cells |> List.map((c) => roomAtLevel(c.level, c.name, kindOf(c), c.rect, c.panels)),
  doors: doors |> List.filter((d) => isLot(d.a) || isLot(d.b)),
  flights: [],
  windows: [],
  fixtures: [],
  lamps: [],
  furniture: furniture,
  bands: [],
  streetLamps: lampHeads,
  // In the lot, facing the entrance.
  spawn: { x: 41.0, z: 0.0 - 30.0 },
}
