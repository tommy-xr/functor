// The mall archetype's plain data: a two-level concourse 76 × 22 m round a
// skylit atrium court, shops off it, a food court at the west end and an
// anchor store's blank wall at the east end; the parking lot south of it.
// mallInterior.fun derives the baked surfaces and lights, mall.fun the
// cells, colliders and street side. Editing this file makes the mall's
// committed bake stale: rebake (README).
//
// Metres, Y up, +Z north. As in layout.fun, rects run between wall centre
// lines and a cell is a rect. The court's void is ONE full-height cell; a
// ring of strips round it on each level (a ring isn't convex): the lower
// concourse under the galleries, the galleries above.

open Layout

let v = Vec3.make

let footprint = rect(0.0, 0.0, 76.0, 42.0)
// Floor to floor; each level's ceiling is `ceiling` over its floor.
let storey = 4.5
let ceiling = 4.0
// The roof slab's top, where the skylight pyramid stands, and its apex.
let roof = 2.0 * storey
let apex = 13.0
let facadeHeight = roof + 1.0
let partition = 0.2
let facade = 0.3

// `Lot`: the street (a collar strip). `Void`: the court, both levels and up
// into the skylight, open on every side.
type CellKind =
  | Lot
  | Entry
  | Hall
  | Strip
  | Void
  | Shop

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

// Cell ids are list positions. The apron is the lot's strip of the collar.
let cells = [
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
  cell("shop A", Shop, 0.0, rect(17.0, 0.0, 25.0, 10.0), pair(rect(17.0, 0.0, 25.0, 10.0), true)),
  cell("shop B", Shop, 0.0, rect(66.0, 32.0, 74.0, 42.0), pair(rect(66.0, 32.0, 74.0, 42.0), false)),
  cell("west hall 2", Hall, 1.0, rect(0.0, 10.0, 26.0, 32.0), gridOf([4.0, 10.0, 16.0, 22.0], hallZs, 1.0)),
  cell("gallery S", Strip, 1.0, rect(26.0, 10.0, 56.0, 15.0), rowOf(12.5, courtXs, odd)),
  cell("gallery N", Strip, 1.0, rect(26.0, 27.0, 56.0, 32.0), rowOf(29.5, courtXs, even)),
  cell("bridge W", Strip, 1.0, rect(26.0, 15.0, 30.0, 27.0), [{ x: 28.0, z: 21.0, on: true }]),
  cell("bridge E", Strip, 1.0, rect(52.0, 15.0, 56.0, 27.0), [{ x: 54.0, z: 21.0, on: false }]),
  cell("east hall 2", Hall, 1.0, rect(56.0, 10.0, 76.0, 32.0), gridOf([60.0, 66.0, 72.0], hallZs, 0.0)),
  cell("shop C", Shop, 1.0, rect(29.0, 0.0, 37.0, 10.0), pair(rect(29.0, 0.0, 37.0, 10.0), true)),
  cell("shop D", Shop, 1.0, rect(45.0, 32.0, 53.0, 42.0), pair(rect(45.0, 32.0, 53.0, 42.0), true)),
]

let apron = 0.0
let entrance = 1.0
let westHall = 2.0
let courtVoid = 8.0
let bridgeW = 14.0
let galleryS = 12.0
let galleryN = 13.0

// Open-plan neighbours: no wall between them, and a portal over their whole
// shared face. The concourse is one hall on each level; the void opens onto
// every strip, the lower ones and the galleries.
let lower = [(2.0, 3.0), (2.0, 5.0), (2.0, 4.0), (7.0, 3.0), (7.0, 6.0), (7.0, 4.0),
             (3.0, 5.0), (3.0, 6.0), (4.0, 5.0), (4.0, 6.0)]
let upper = [(11.0, 12.0), (11.0, 14.0), (11.0, 13.0), (16.0, 12.0), (16.0, 15.0), (16.0, 13.0),
             (12.0, 14.0), (12.0, 15.0), (13.0, 14.0), (13.0, 15.0)]
let links = List.flatten([lower, upper, [3.0, 4.0, 5.0, 6.0, 12.0, 13.0, 14.0, 15.0] |> List.map((s) => (courtVoid, s))])

// Openings in walls: the entrance first, from the apron; the entrance hall
// into the court; a shopfront into each open shop.
let shopfront = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 6.0, height: 3.2 }
let doors = [
  { a: apron, b: entrance, at: 41.0, width: 4.0, height: 3.2 },
  { a: entrance, b: 3.0, at: 41.0, width: 6.0 - partition, height: ceiling },
  shopfront(9.0, westHall, 21.0),
  shopfront(10.0, 7.0, 70.0),
  shopfront(17.0, galleryS, 33.0),
  shopfront(18.0, galleryN, 49.0),
]

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
// every cell as a room, the entrance as its one door, the lot's lamps.
// (Its kinds only steer exterior.fun: `Outside` marks the apron.)
let kindOf = (c: Cell): Kind =>
  match c.kind with
  | Lot => Outside
  | Entry => Lobby
  | Shop => Office
  | _ => Corridor
let layout: Layout = {
  footprint: footprint,
  ceiling: ceiling,
  storey: storey,
  levels: 2.0,
  facadeHeight: facadeHeight,
  partition: partition,
  facade: facade,
  rooms: cells |> List.map((c) => roomAtLevel(c.level, c.name, kindOf(c), c.rect, c.panels)),
  doors: List.take(1.0, doors),
  flights: [],
  windows: [],
  fixtures: [],
  lamps: [],
  bands: [],
  streetLamps: lampHeads,
  // In the lot, facing the entrance.
  spawn: { x: 41.0, z: 0.0 - 30.0 },
}
