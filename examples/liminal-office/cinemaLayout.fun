// The cinema archetype's plain data: a 60 × 40 m multiplex on one tall
// level. A lobby with a concession counter and a ticket desk; a numbered
// corridor that bends once (two convex cells) past four auditorium doors,
// two of them open onto raked auditoria, two shut; a white-tiled service
// corridor from the west auditorium round to a rear exit. The rooms are a
// layout.fun `Layout` (cells, portals, walls, the street side); the rest
// (heights, fittings, the rake) is the cinema's own. Editing this file makes
// the cinema's committed bake stale: rebake (README).

open Layout

let v = Vec3.make

let footprint = rect(0.0, 0.0, 60.0, 40.0)
let partition = 0.2
let facade = 0.3
// The roof slab, and the blank facade's top.
let roofline = 8.5
let facadeHeight = 9.5
// Every room's cell is this tall: the auditoria's height, over the eye at
// the top of the rake. Lower rooms are lower inside it.
let cellHeight = 8.0

let room = (name: string, kind: Kind, r: Rect): Room => roomAtLevel(0.0, name, kind, r, [])

// Room ids, as in `rooms`.
let apron = 0.0
let lobby = 1.0
let corridorA = 2.0
let corridorB = 3.0
let east = 4.0
let west = 5.0
let serviceN = 6.0
let serviceW = 7.0
let rearYard = 8.0

// The kit has no auditorium kind: `Office` stands in (an inside room).
let rooms = [
  room("apron", Outside, rect(15.0, 0.0 - 12.0, 45.0, 0.0)),
  room("lobby", Lobby, rect(15.0, 0.0, 45.0, 18.0)),
  room("corridor A", Corridor, rect(45.0, 8.0, 48.5, 18.0)),
  room("corridor B", Corridor, rect(15.0, 18.0, 48.5, 21.5)),
  room("auditorium 3", Office, rect(29.0, 21.5, 43.0, 40.0)),
  room("auditorium 4", Office, rect(15.0, 21.5, 29.0, 40.0)),
  room("service N", Corridor, rect(12.6, 18.0, 15.0, 40.0)),
  room("service W", Corridor, rect(0.0, 37.6, 12.6, 40.0)),
  room("rear yard", Outside, rect(0.0 - 12.0, 28.0, 0.0, 40.0)),
]

let door = (a: float, b: float, at: float, width: float, height: float): Door =>
  { a: a, b: b, at: at, width: width, height: height }
// The entrance first (the street side's), from the apron.
let doors = [
  door(apron, lobby, 30.0, 3.2, 2.7),
  // The corridor's arch off the lobby, and its bend: open full width and height.
  door(lobby, corridorA, 13.0, 4.0, 3.2),
  door(corridorA, corridorB, 46.75, 3.5 - partition, 5.0),
  // The auditoria's doors, by the screen.
  door(corridorB, east, 41.6, 1.6, 2.4),
  door(corridorB, west, 27.6, 1.6, 2.4),
  // Staff only: the corridor's west end, the west auditorium's front, the
  // service corridor's turn, and the rear exit.
  door(corridorB, serviceN, 19.75, 1.0, 2.1),
  door(serviceN, west, 23.0, 1.0, 2.1),
  door(serviceN, serviceW, 38.8, 2.4 - partition, 3.0),
  door(rearYard, serviceW, 38.8, 1.0, 2.1),
]

// Sodium lamps over the parking lot, south of the apron: along its near
// edge and down its middle island (z -30 to -28).
let lampHeads = List.flatten([
  [8.0, 22.0, 38.0, 52.0] |> List.map((x) => v(x, 4.8, 0.0 - 12.6)),
  [5.0, 20.0, 40.0, 55.0] |> List.map((x) => v(x, 4.8, 0.0 - 29.0)),
])

// The lobby's waiting corner: a couch under the west posters, a bench and
// a plant.
let furniture = [
  { room: lobby, piece: Couch, x: 15.6, z: 8.0, facing: East, mess: 0.3 },
  { room: lobby, piece: Bench, x: 18.6, z: 5.0, facing: West, mess: 0.0 },
  { room: lobby, piece: Plant, x: 15.5, z: 10.0, facing: East, mess: 0.7 },
]

let layout: Layout = {
  footprint: footprint,
  ceiling: cellHeight,
  storey: roofline,
  levels: 1.0,
  facadeHeight: facadeHeight,
  partition: partition,
  facade: facade,
  rooms: rooms,
  doors: doors,
  flights: [],
  windows: [],
  fixtures: [],
  lamps: [],
  furniture: furniture,
  bands: [],
  streetLamps: lampHeads,
  // In the parking lot, facing the entrance (+Z).
  spawn: { x: 30.0, z: 0.0 - 30.0 },
}

// ---- The cinema's own ----------------------------------------------------------------

// Each room's height inside its cell (outside rooms: none).
let heights = [0.0, 5.0, 5.0, 5.0, 8.0, 8.0, 3.0, 3.0, 0.0]
// The dado: walls are coloured below it, black above (the lobby's soffit
// ring hangs at it); the auditoria's fabric runs to the ceiling.
let lobbyDado = 4.0
let corridorDado = 3.3

// The lobby's soffit ring: `ringWidth` in from its walls, `ringDepth`
// deep, its underside on the dado line.
let ringWidth = 1.6
let ringDepth = 0.5
// The concession counter and the ticket desk (x0, z0, x1, z1), 1.1 m high.
let counterHeight = 1.1
let counter = rect(20.0, 14.4, 36.0, 15.9)
let desk = rect(37.0, 4.0, 42.5, 5.0)

// Poster light boxes, 1 × 1.5 m: on a room's inner wall, centred `at`
// along it, showing poster `n` (0–3).
type Poster = { room: float, side: Side, at: float, n: float }
let poster = (r: float, side: Side, at: float, n: float): Poster => { room: r, side: side, at: at, n: n }
let posters = [
  poster(lobby, West, 4.0, 0.0), poster(lobby, West, 8.0, 1.0), poster(lobby, West, 12.0, 2.0),
  poster(lobby, South, 22.0, 3.0), poster(lobby, South, 25.0, 0.0), poster(lobby, South, 35.0, 1.0), poster(lobby, South, 38.0, 2.0),
  poster(corridorA, South, 46.75, 3.0),
  poster(corridorB, North, 21.5, 1.0), poster(corridorB, North, 33.5, 2.0), poster(corridorB, North, 37.5, 3.0),
  poster(corridorB, South, 21.5, 0.0), poster(corridorB, South, 29.5, 2.0), poster(corridorB, South, 37.5, 1.0),
]

// A screen number over door or shut door `at` on a wall; `open`: an open
// door's (in the layout), else a shut door's plate.
type Screen = { room: float, side: Side, at: float, n: float, open: bool }
let screens = [
  { room: corridorA, side: East, at: 11.0, n: 1.0, open: false },
  { room: corridorA, side: East, at: 15.5, n: 2.0, open: false },
  { room: corridorB, side: North, at: 41.6, n: 3.0, open: true },
  { room: corridorB, side: North, at: 27.6, n: 4.0, open: true },
]

// Neon-edged columns along the corridors' walls, centred `at`; `pink` or
// cyan tubes on their edges.
type Column = { room: float, side: Side, at: float, pink: bool }
let column = (r: float, side: Side, at: float, pink: bool): Column => { room: r, side: side, at: at, pink: pink }
let columns = List.flatten([
  [19.5, 23.5, 31.5, 35.5, 39.5, 46.0] |> List.map((x) => column(corridorB, North, x, true)),
  [19.5, 23.5, 27.5, 31.5, 35.5, 39.5, 43.5] |> List.map((x) => column(corridorB, South, x, false)),
  [9.5, 13.25, 17.0] |> List.map((z) => column(corridorA, East, z, true)),
  [9.5, 17.0] |> List.map((z) => column(corridorA, West, z, false)),
])
let columnWidth = 0.5
let columnDepth = 0.25

// The corridors' soffits: segments `soffitStep` long down each corridor's
// length, `soffitMargin` clear of its walls, their undersides on a wave
// round `soffitY` (a curve, approximated in steps).
let soffitStep = 1.6
let soffitMargin = 0.55
let soffitY = 3.15
let soffitDepth = 0.35
let soffitWave = (k: float): float => soffitY + 0.3 * Math.sin(k * 0.9)

// The auditoria: the screen on the corridor wall, then the rake rising away
// from it, `rows` rows `rowDepth` deep, `rise` each, from `rakeFrom` metres
// in from the screen wall. Aisles `aisle` wide down both sides.
let rakeFrom = 4.4
let rows = 12.0
let rowDepth = 1.0
let rise = 0.3
let aisle = 1.3
let screenWidth = 9.0
let screenBottom = 1.4
let screenTop = 5.6
let auditoria = [east, west]

// The playset marooned in auditorium 4's front, by the screen: coloured
// boxes (colour 0 red, 1 yellow, 2 blue, 3 green), and a slide from the
// deck's east edge down to the floor.
type Toy = { x0: float, y0: float, z0: float, x1: float, y1: float, z1: float, c: float }
let toy = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float, c: float): Toy =>
  { x0: x0, y0: y0, z0: z0, x1: x1, y1: y1, z1: z1, c: c }
let playset = [
  // A deck on four yellow posts under a red roof; a green crawl tube.
  toy(18.0, 1.2, 23.6, 19.6, 1.35, 25.0, 0.0),
  toy(18.0, 0.0, 23.6, 18.12, 2.4, 23.72, 1.0), toy(19.48, 0.0, 23.6, 19.6, 2.4, 23.72, 1.0),
  toy(18.0, 0.0, 24.88, 18.12, 2.4, 25.0, 1.0), toy(19.48, 0.0, 24.88, 19.6, 2.4, 25.0, 1.0),
  toy(17.9, 2.4, 23.5, 19.7, 2.6, 25.1, 0.0),
  toy(16.6, 0.0, 23.9, 18.0, 0.8, 24.7, 3.0),
]
// The slide: from (x0, top) on the deck down to (x1, 0), between z0 and z1.
let slide = { x0: 19.6, x1: 21.8, top: 1.2, z0: 24.0, z1: 24.6 }
