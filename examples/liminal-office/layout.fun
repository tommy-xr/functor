// The office's ONE layout authority: plain data that rendering, collision,
// culling cells and the lightmap bake all derive from (building.fun).
// Editing this file makes the committed bake stale: rebake (README).
//
// Metres, Y up, +Z north. Rects run between wall CENTRE lines, so two rooms
// sharing a wall share an edge; a cell is a room's rect, and a door is a
// portal on that shared edge.

type Rect = { x0: float, z0: float, x1: float, z1: float }

type Kind =
  | Outside
  | Lobby
  | Corridor
  | Office
  | Stair

type Side =
  | North
  | South
  | East
  | West

// A ceiling panel's centre; `on` panels are baked as lights.
type Panel = { x: float, z: float, on: bool }

// `level` 0 is the ground floor, 1 the floor above.
type Room = { name: string, kind: Kind, level: float, rect: Rect, panels: List<Panel> }

// An opening in the wall `a` and `b` share, centred at `at` along it.
type Door = { a: float, b: float, at: float, width: float, height: float }

let rect = (x0: float, z0: float, x1: float, z1: float): Rect => { x0: x0, z0: z0, x1: x1, z1: z1 }

// The building's outside walls run along this rect.
let footprint = rect(0.0, 0.0, 20.0, 29.0)
let ceiling = 2.7
// Floor to floor: the drop ceiling, then a 0.6 m plenum and slab.
let storey = 3.3
// The roofline, over both floors.
let facadeHeight = storey + 4.2
let partition = 0.2
let facade = 0.3

// Panels every `step` metres along a corridor's long axis.
let alongZ = (x: float, z0: float, n: float, step: float, on: bool): List<Panel> =>
  List.range(n) |> List.map((i) => { x: x, z: z0 + i * step, on: on })
let alongX = (z: float, x0: float, n: float, step: float, on: bool): List<Panel> =>
  List.range(n) |> List.map((i) => { x: x0 + i * step, z: z, on: on })
// Two panels down an office's middle, lit or not.
let pair = (r: Rect, on: bool): List<Panel> =>
  let x = (r.x0 + r.x1) / 2.0 in
  let dz = (r.z1 - r.z0) / 4.0 in
  [{ x: x, z: r.z0 + dz, on: on }, { x: x, z: r.z1 - dz, on: on }]

let room = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room =>
  { name: name, kind: kind, level: 0.0, rect: r, panels: panels }
let upstairs = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room =>
  { name: name, kind: kind, level: 1.0, rect: r, panels: panels }

// Room ids are list positions, and they are the cell ids. Rooms on one
// level never overlap; the stairwell is a room on each level, one above
// the other.
let rooms = [
  room("apron", Outside, rect(4.0, 0.0 - 12.0, 16.0, 0.0), []),
  room("lobby", Lobby, rect(6.0, 0.0, 14.0, 6.0),
    [{ x: 8.0, z: 2.0, on: true }, { x: 12.0, z: 2.0, on: true },
     { x: 8.0, z: 4.4, on: true }, { x: 12.0, z: 4.4, on: true }]),
  room("corridor", Corridor, rect(8.8, 6.0, 11.2, 26.6), alongZ(10.0, 7.6, 7.0, 3.0, true)),
  room("turn", Corridor, rect(8.8, 26.6, 20.0, 29.0), alongX(27.8, 10.0, 4.0, 3.0, true)),
  room("101", Office, rect(0.0, 0.0, 6.0, 6.0), pair(rect(0.0, 0.0, 6.0, 6.0), false)),
  room("102", Office, rect(14.0, 0.0, 20.0, 6.0), pair(rect(14.0, 0.0, 20.0, 6.0), true)),
  room("103", Office, rect(0.0, 6.0, 8.8, 13.0), pair(rect(0.0, 6.0, 8.8, 13.0), true)),
  room("104", Office, rect(11.2, 6.0, 20.0, 13.0), pair(rect(11.2, 6.0, 20.0, 13.0), false)),
  room("105", Office, rect(0.0, 13.0, 8.8, 20.0), pair(rect(0.0, 13.0, 8.8, 20.0), false)),
  room("106", Office, rect(11.2, 13.0, 20.0, 20.0), pair(rect(11.2, 13.0, 20.0, 20.0), true)),
  room("107", Office, rect(0.0, 20.0, 8.8, 29.0), pair(rect(0.0, 20.0, 8.8, 29.0), false)),
  room("stair B", Stair, rect(11.2, 20.0, 20.0, 26.6), []),
  upstairs("stair B 2", Stair, rect(11.2, 20.0, 20.0, 26.6), []),
  upstairs("corridor 2", Corridor, rect(8.8, 0.0, 11.2, 26.6), alongZ(10.0, 1.6, 9.0, 3.0, true)),
  upstairs("turn 2", Corridor, rect(8.8, 26.6, 20.0, 29.0), alongX(27.8, 10.0, 4.0, 3.0, true)),
  upstairs("201", Office, rect(0.0, 0.0, 8.8, 6.0), pair(rect(0.0, 0.0, 8.8, 6.0), false)),
  upstairs("202", Office, rect(11.2, 0.0, 20.0, 6.0), pair(rect(11.2, 0.0, 20.0, 6.0), true)),
  upstairs("203", Office, rect(0.0, 6.0, 8.8, 13.0), pair(rect(0.0, 6.0, 8.8, 13.0), true)),
  upstairs("204", Office, rect(11.2, 6.0, 20.0, 13.0), pair(rect(11.2, 6.0, 20.0, 13.0), false)),
  upstairs("205", Office, rect(0.0, 13.0, 8.8, 20.0), pair(rect(0.0, 13.0, 8.8, 20.0), false)),
  upstairs("206", Office, rect(11.2, 13.0, 20.0, 20.0), pair(rect(11.2, 13.0, 20.0, 20.0), true)),
  upstairs("207", Office, rect(0.0, 20.0, 8.8, 29.0), pair(rect(0.0, 20.0, 8.8, 29.0), false)),
]

let door = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 1.0, height: 2.1 }
// A corridor's open end: its full inner width, floor to ceiling.
let throughway = (a: float, b: float, at: float): Door =>
  { a: a, b: b, at: at, width: 2.4 - partition, height: ceiling }

// Doors join rooms on one level. The entrance is first, from the apron.
let doors = [
  { a: 0.0, b: 1.0, at: 10.0, width: 2.0, height: 2.4 },
  throughway(1.0, 2.0, 10.0),
  throughway(2.0, 3.0, 10.0),
  door(4.0, 1.0, 4.2),
  door(1.0, 5.0, 4.2),
  door(6.0, 2.0, 9.5),
  door(2.0, 7.0, 9.5),
  door(8.0, 2.0, 16.5),
  door(2.0, 9.0, 16.5),
  door(10.0, 2.0, 23.3),
  door(2.0, 11.0, 23.3),
  throughway(13.0, 14.0, 10.0),
  door(15.0, 13.0, 3.0),
  door(13.0, 16.0, 3.0),
  door(17.0, 13.0, 9.5),
  door(13.0, 18.0, 9.5),
  door(19.0, 13.0, 16.5),
  door(13.0, 20.0, 16.5),
  door(21.0, 13.0, 23.3),
  door(14.0, 12.0, 19.2),
]

// A straight flight of `risers` steps up one storey, from room `lower` to
// `upper` (the same rect, a level up). It runs east from `x0` along the
// lower room's south wall, between `z0` and `z1` (its parapet included),
// one `tread` per step, onto a landing that fills the rest of the east end.
type Flight = { lower: float, upper: float, x0: float, z0: float, z1: float, risers: float, tread: float }

let flights = [{ lower: 11.0, upper: 12.0, x0: 13.0, z0: 20.1, z1: 21.6, risers: 18.0, tread: 0.3 }]

// An opening in room `room`'s outside wall on `side`, centred at `at` along
// it, from `sill` up to `head`. Open (no glass): daylight enters the day bake
// only through these and the entrance.
type Window = { room: float, side: Side, at: float, width: float, sill: float, head: float }

// A ribbon window in office 103, between the west facade's window bands,
// and the same one in 203 above it.
let windows = [
  { room: 6.0, side: West, at: 9.3, width: 3.6, sill: 0.9, head: 2.3 },
  { room: 17.0, side: West, at: 9.3, width: 3.6, sill: 0.9, head: 2.3 },
]

// Signs on a room's inner wall, centred at `at` along it.
// A stair sign is a lit amber plate over a stairwell door.
type Sign =
  | ExitSign
  | StairSign

type Fixture = { room: float, side: Side, at: float, sign: Sign }

let fixtures = [
  { room: 3.0, side: North, at: 10.0, sign: ExitSign },
  { room: 2.0, side: East, at: 23.3, sign: StairSign },
  { room: 14.0, side: North, at: 10.0, sign: ExitSign },
  { room: 14.0, side: South, at: 19.2, sign: StairSign },
]

// Sodium wall lamps in the stairwell, `y` above the room's floor: an amber
// lens on the wall, and a light in the bakes only.
type Lamp = { room: float, side: Side, at: float, y: float }

let lamps = [
  { room: 11.0, side: West, at: 21.4, y: 2.6 },
  { room: 11.0, side: North, at: 15.6, y: 2.9 },
  { room: 12.0, side: East, at: 23.3, y: 2.2 },
  { room: 12.0, side: South, at: 14.0, y: 2.2 },
]

// Where the player starts: on the apron, facing the entrance (+Z).
let spawn = { x: 10.0, z: 0.0 - 11.0 }
