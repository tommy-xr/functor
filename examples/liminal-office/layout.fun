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

type Side =
  | North
  | South
  | East
  | West

// A ceiling panel's centre; `on` panels are baked as lights.
type Panel = { x: float, z: float, on: bool }

type Room = { name: string, kind: Kind, rect: Rect, panels: List<Panel> }

// An opening in the wall `a` and `b` share, centred at `at` along it.
type Door = { a: float, b: float, at: float, width: float, height: float }

let rect = (x0: float, z0: float, x1: float, z1: float): Rect => { x0: x0, z0: z0, x1: x1, z1: z1 }

// The building's outside walls run along this rect.
let footprint = rect(0.0, 0.0, 20.0, 29.0)
let ceiling = 2.7
let facadeHeight = 4.2
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
  { name: name, kind: kind, rect: r, panels: panels }

// Room ids are list positions, and they are the cell ids.
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
  room("108", Office, rect(11.2, 20.0, 20.0, 26.6), pair(rect(11.2, 20.0, 20.0, 26.6), false)),
]

let door = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 1.0, height: 2.1 }
// A corridor's open end: its full inner width, floor to ceiling.
let throughway = (a: float, b: float, at: float): Door =>
  { a: a, b: b, at: at, width: 2.4 - partition, height: ceiling }

// Jamb faces are baked with room `b`, so the apron is always `a`.
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
]

// Signs on a room's inner wall, centred at `at` along it.
type Sign =
  | ExitSign
  | StairDoor

type Fixture = { room: float, side: Side, at: float, sign: Sign }

let fixtures = [
  { room: 3.0, side: North, at: 10.0, sign: ExitSign },
  { room: 3.0, side: East, at: 27.8, sign: StairDoor },
]

// Where the player starts: on the apron, facing the entrance (+Z).
let spawn = { x: 10.0, z: 0.0 - 11.0 }
