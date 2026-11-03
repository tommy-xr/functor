// The office archetype's layout (layout.fun's `Layout`): two floors, a
// lobby, corridors, offices and a stairwell. Editing this file makes the
// office's committed bake stale: rebake (README).

open Layout

let v = Vec3.make

// The building's outside walls run along this rect.
let footprint = rect(0.0, 0.0, 20.0, 29.0)
let ceiling = 2.7
// Floor to floor: the drop ceiling, then a 0.6 m plenum and slab.
let storey = 3.3
// The roofline, over both floors.
let facadeHeight = storey + 4.2
let partition = 0.2
let levels = 2.0

let room = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room => roomAtLevel(0.0, name, kind, r, panels)
let upstairs = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room => roomAtLevel(1.0, name, kind, r, panels)

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
  // The street west of the building, which sees in through the windows.
  room("west yard", Outside, rect(0.0 - 12.0, 0.0, 0.0, 29.0), []),
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

let flights = [{ kind: Straight, lower: 11.0, upper: 12.0, x0: 13.0, z0: 20.1, z1: 21.6, risers: 18.0, tread: 0.3 }]

// A ribbon window in office 103, between the west facade's window bands,
// and the same one in 203 above it.
let windows = [
  { room: 6.0, side: West, at: 9.3, width: 3.6, sill: 0.9, head: 2.3 },
  { room: 17.0, side: West, at: 9.3, width: 3.6, sill: 0.9, head: 2.3 },
]

let fixtures = [
  { room: 3.0, side: North, at: 10.0, sign: ExitSign },
  { room: 2.0, side: East, at: 23.3, sign: StairSign },
  { room: 14.0, side: North, at: 10.0, sign: ExitSign },
  { room: 14.0, side: South, at: 19.2, sign: StairSign },
]

// Sodium wall lamps in the stairwell.
let lamps = [
  { room: 11.0, side: West, at: 21.4, y: 2.6 },
  { room: 11.0, side: North, at: 15.6, y: 2.9 },
  { room: 12.0, side: East, at: 23.3, y: 2.2 },
  { room: 12.0, side: South, at: 14.0, y: 2.2 },
]

// Window bands, a row per storey: the south row leaves the entrance clear
// on the ground floor, the north side has none. exterior.fun leaves out any
// that overlaps a real window.
let bandsAlong = (level: float, s: Side): List<float> =>
  match s with
  | South => if level == 0.0 then [2.0, 4.6, 15.4, 18.0] else [2.0, 4.6, 7.2, 12.8, 15.4, 18.0]
  | North => []
  | _ => List.range(8.0) |> List.map((k) => 2.5 + 3.4 * k)
let bands = List.range(levels) |> List.concatMap((level) => [South, North, West, East] |> List.concatMap((s) =>
  bandsAlong(level, s) |> List.map((at) => { level: level, side: s, at: at })))

// Sodium street lamps by the apron.
let lampHeads = [v(4.4, 4.8, -4.0), v(15.6, 4.8, -4.0)]

let layout: Layout = {
  footprint: footprint,
  ceiling: ceiling,
  storey: storey,
  levels: levels,
  facadeHeight: facadeHeight,
  partition: partition,
  facade: 0.3,
  rooms: rooms,
  doors: doors,
  flights: flights,
  windows: windows,
  fixtures: fixtures,
  lamps: lamps,
  bands: bands,
  streetLamps: lampHeads,
  // On the apron, facing the entrance (+Z).
  spawn: { x: 10.0, z: 0.0 - 11.0 },
}
