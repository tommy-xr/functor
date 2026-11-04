// The house archetype's layout (layout.fun's `Layout`): two storeys, 10 × 12
// m. Downstairs a porch, a hall with a straight stair along its south wall,
// a living room, dining room, kitchen (back door) and den; upstairs the
// stair's landing, a corridor and three bedrooms. Its palettes are in
// housePalettes.fun, its street side (porch, gable, fence) in
// houseExterior.fun. Editing this file makes both house bakes stale:
// rebake (README).

open Layout

let v = Vec3.make

let footprint = rect(0.0, 0.0, 10.0, 12.0)
let ceiling = 2.5
// 15 risers of 0.19 m.
let storey = 2.85
let levels = 2.0
let partition = 0.15
let facade = 0.2
// The eaves: the facade rises past the top storey's slab to meet the gable,
// whose lower half sinks into it above the bedroom ceilings
// (houseExterior.fun).
let facadeHeight = 7.0

let room = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room => roomAtLevel(0.0, name, kind, r, panels)
let upstairs = (name: string, kind: Kind, r: Rect): Room => roomAtLevel(1.0, name, kind, r, [])

// Room ids are list positions, and they are the cell ids. The hall,
// landing, corridor and bedrooms are carpeted, the dining room and kitchen
// not (building.fun's `floorMatOf`). The hall is the stair's foot, the
// landing its head. The four outside rooms are the collar's strips: the
// porch (south) and the yards.
let rooms = [
  room("porch", Outside, rect(0.0, 0.0 - 2.4, 10.0, 0.0), []),
  room("hall", StairHall, rect(0.0, 0.0, 10.0, 2.8), []),
  room("living", Office, rect(0.0, 2.8, 5.0, 8.8), []),
  room("dining", Lobby, rect(5.0, 2.8, 10.0, 8.0), []),
  room("kitchen", Lobby, rect(5.0, 8.0, 10.0, 12.0), [{ x: 7.5, z: 10.0, on: true }]),
  room("den", Office, rect(0.0, 8.8, 5.0, 12.0), []),
  upstairs("landing", StairHall, rect(0.0, 0.0, 10.0, 2.8)),
  upstairs("corridor", Corridor, rect(0.0, 2.8, 10.0, 4.4)),
  upstairs("bedroom 1", Office, rect(0.0, 4.4, 3.4, 9.6)),
  upstairs("bedroom 2", Office, rect(3.4, 4.4, 6.6, 9.6)),
  upstairs("bedroom 3", Office, rect(6.6, 4.4, 10.0, 9.6)),
  room("west yard", Outside, rect(0.0 - 6.0, 0.0, 0.0, 12.0), []),
  room("east yard", Outside, rect(10.0, 0.0, 16.0, 12.0), []),
  room("back yard", Outside, rect(0.0, 12.0, 10.0, 18.0), []),
]

let door = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 0.9, height: 2.1 }

// The front door is first: the entrance, from the porch.
let doors = [
  { a: 0.0, b: 1.0, at: 1.6, width: 1.0, height: 2.1 },
  { a: 1.0, b: 2.0, at: 2.5, width: 1.2, height: 2.1 },
  door(1.0, 3.0, 7.0),
  { a: 2.0, b: 3.0, at: 5.4, width: 1.2, height: 2.1 },
  door(3.0, 4.0, 7.5),
  door(2.0, 5.0, 2.5),
  door(5.0, 4.0, 10.4),
  // The back door.
  door(4.0, 13.0, 8.6),
  door(6.0, 7.0, 9.1),
  door(7.0, 8.0, 1.7),
  door(7.0, 9.0, 5.0),
  door(7.0, 10.0, 8.3),
]

// Up east from the hall's front door along its south wall, onto the
// landing's east end.
let flights = [{ kind: Straight, lower: 1.0, upper: 6.0, x0: 4.4, z0: 0.1, z1: 1.35, risers: 15.0, tread: 0.28 }]

let window = (room: float, side: Side, at: float, width: float): Window =>
  { room: room, side: side, at: at, width: width, sill: 0.9, head: 2.1 }
// Four downstairs, two up: over the stairs, and bedroom 3's.
let windows = [
  window(1.0, South, 3.2, 0.9),
  window(2.0, West, 5.8, 1.6),
  window(3.0, East, 5.4, 1.4),
  window(4.0, North, 6.6, 1.2),
  window(6.0, South, 5.6, 1.2),
  window(10.0, East, 7.0, 1.2),
]

// A niche in the hall, between the living and dining room doors.
let fixtures = [{ room: 1.0, side: North, at: 4.7, sign: Niche }]

// Lamps (light in the bakes only). The porch's has no lens in the bake (an
// outside room has no faces): houseExterior.fun draws its glow.
let lamps = [
  { room: 0.0, side: North, at: 0.55, y: 2.1 },
  { room: 1.0, side: West, at: 1.6, y: 2.0 },
  { room: 2.0, side: East, at: 7.6, y: 1.5 },
  { room: 2.0, side: West, at: 3.6, y: 1.5 },
  { room: 5.0, side: North, at: 2.5, y: 2.0 },
  { room: 6.0, side: East, at: 1.4, y: 2.0 },
  { room: 7.0, side: West, at: 3.6, y: 2.0 },
  { room: 9.0, side: North, at: 5.0, y: 1.4 },
  { room: 10.0, side: North, at: 8.3, y: 1.6 },
]

// The living room: a couch under the window, a low table, a shelf, a
// plant, a chair over. Bedroom 2: a bed against its north wall, a chair
// and a chest of drawers. Every other room empty.
let piece = (room: float, p: Article, x: float, z: float, facing: Side, mess: float): Furnish =>
  { room: room, piece: p, x: x, z: z, facing: facing, mess: mess }
let furniture = [
  piece(2.0, Couch, 0.58, 5.8, East, 0.3),
  piece(2.0, CoffeeTable, 1.6, 5.8, East, 0.4),
  piece(2.0, Bookshelf, 4.35, 8.53, South, 0.6),
  piece(2.0, Plant, 0.45, 8.3, East, 1.0),
  piece(2.0, SideChair, 3.9, 3.6, West, 1.0),
  piece(9.0, Bed, 4.3, 8.44, South, 0.8),
  piece(9.0, SideChair, 6.1, 6.2, West, 0.7),
  piece(9.0, Cabinet, 6.2, 8.9, West, 0.6),
]

let layout: Layout = {
  footprint: footprint,
  ceiling: ceiling,
  storey: storey,
  levels: levels,
  facadeHeight: facadeHeight,
  partition: partition,
  facade: facade,
  rooms: rooms,
  doors: doors,
  flights: flights,
  windows: windows,
  fixtures: fixtures,
  lamps: lamps,
  furniture: furniture,
  bands: [],
  // A street lamp past the fence's east end (houseExterior.fun).
  streetLamps: [v(11.4, 4.8, 0.0 - 6.8)],
  // On the front walk, facing the door.
  spawn: { x: 1.6, z: 0.0 - 9.0 },
}
