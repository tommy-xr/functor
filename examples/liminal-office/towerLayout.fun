// The tower archetype's layout (layout.fun's `Layout`): a 24 m square
// high-rise round a core, a dog-leg stair and a lift lobby on the same rect
// on every floor. A lobby floor, typical floors stacked over it, and a roof
// with the stair's head room and a terrace. The floors make three layouts:
// the whole tower of `n` typical floors (`towerOf`: cells, street side), and
// two bake layouts, the typical floor between the stairs of the floors above
// and below (`typical`), and the ground and roof beside the stairs next to
// them (`ends`). Editing this file makes the tower's committed bakes stale:
// rebake (README).

open Layout

let v = Vec3.make

let footprint = rect(0.0, 0.0, 24.0, 24.0)
let ceiling = 2.7
let storey = 3.3
let partition = 0.2
let facade = 0.3
// The parapet round the roof, above its slab.
let parapet = 1.0

// The core, the same rects on every floor.
let stairRect = rect(7.6, 8.8, 10.8, 15.2)
let liftRect = rect(10.8, 8.8, 16.4, 15.2)

// One floor's plain data, its room ids counted from 0 within it.
type Floor = { rooms: List<Room>, doors: List<Door>, windows: List<Window>, fixtures: List<Fixture>, lamps: List<Lamp>, furniture: List<Furnish> }

let room = (name: string, kind: Kind, r: Rect, panels: List<Panel>): Room => roomAtLevel(0.0, name, kind, r, panels)
let door = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 1.0, height: 2.1 }
// A corridor's open end, and the lift lobby's mouth.
let throughway = (a: float, b: float, at: float): Door =>
  { a: a, b: b, at: at, width: 2.4 - partition, height: ceiling }
let mouth = (a: float, b: float, at: float): Door => { a: a, b: b, at: at, width: 3.0, height: 2.4 }
let window = (r: float, side: Side, at: float): Window => { room: r, side: side, at: at, width: 1.2, sill: 0.9, head: 2.3 }

// The lift lobby: three brushed steel doors on its north wall, a grid of
// panels. The stairwell: a sodium lamp by the landing's door and one over
// the half-landing.
let liftDoors = (r: float): List<Fixture> =>
  [12.0, 13.6, 15.2] |> List.map((x) => { room: r, side: North, at: x, sign: LiftDoor })
let liftPanels = [{ x: 12.4, z: 10.6, on: true }, { x: 14.8, z: 10.6, on: true },
                  { x: 12.4, z: 13.4, on: true }, { x: 14.8, z: 13.4, on: true }]
let landingLamp = (r: float): Lamp => { room: r, side: South, at: 10.2, y: 2.3 }
let stairLamps = (r: float): List<Lamp> => [landingLamp(r), { room: r, side: North, at: 9.2, y: 3.0 }]

// Ground: a lobby across the south, the entrance and two windows in its
// facade; the core; two side rooms and a back room. The apron and the west
// yard are the street.
let apron = room("apron", Outside, rect(6.0, 0.0 - 12.0, 18.0, 0.0), [])
// The street west of the tower, which sees in through its windows.
let westYard = room("west yard", Outside, rect(0.0 - 12.0, 0.0, 0.0, 24.0), [])
let ground: Floor = {
  rooms: [
    apron,
    room("lobby", Lobby, rect(0.0, 0.0, 24.0, 8.8),
      [4.0, 8.0, 12.0, 16.0, 20.0] |> List.concatMap((x) => [{ x: x, z: 3.0, on: true }, { x: x, z: 6.0, on: true }])),
    room("stair 0", Stair, stairRect, []),
    room("lifts 0", LiftLobby, liftRect, liftPanels),
    room("west wing", Office, rect(0.0, 8.8, 7.6, 24.0), pair(rect(0.0, 8.8, 7.6, 24.0), true)),
    room("east wing", Office, rect(16.4, 8.8, 24.0, 24.0), pair(rect(16.4, 8.8, 24.0, 24.0), false)),
    westYard,
    room("back room", Office, rect(7.6, 15.2, 16.4, 24.0), pair(rect(7.6, 15.2, 16.4, 24.0), false)),
  ],
  // The entrance first, from the apron.
  doors: [
    { a: 0.0, b: 1.0, at: 12.0, width: 2.0, height: 2.4 },
    door(1.0, 2.0, 9.2),
    mouth(1.0, 3.0, 13.6),
    door(1.0, 4.0, 4.0),
    door(1.0, 5.0, 20.2),
    door(4.0, 7.0, 19.6),
  ],
  windows: [{ room: 1.0, side: South, at: 4.0, width: 3.0, sill: 0.9, head: 2.3 },
            { room: 1.0, side: South, at: 20.0, width: 3.0, sill: 0.9, head: 2.3 }],
  fixtures: List.flatten([
    [{ room: 1.0, side: South, at: 12.0, sign: ExitSign }, { room: 1.0, side: North, at: 9.2, sign: StairSign }],
    liftDoors(3.0),
  ]),
  lamps: stairLamps(2.0),
  // A waiting area by the west windows, a reception counter on the east,
  // clear of the walk from the entrance to the stair door.
  furniture: [
    piece(1.0, Couch, 0.62, 4.2, East, 0.3),
    piece(1.0, CoffeeTable, 1.9, 4.2, East, 0.5),
    piece(1.0, SideChair, 3.1, 3.5, West, 0.4),
    piece(1.0, SideChair, 3.1, 4.9, West, 1.0),
    piece(1.0, Plant, 0.5, 7.4, East, 0.6),
    piece(1.0, Counter, 17.0, 4.0, West, 0.0),
    piece(1.0, Chair, 17.9, 4.1, West, 0.8),
    piece(1.0, Cooler, 23.5, 7.9, West, 0.3),
    piece(1.0, Plant, 23.4, 0.6, West, 0.3),
  ],
}

// A typical floor: a ring of corridors (four strips: a ring isn't convex)
// round the core, two open offices off it with a window each, and four shut
// doors. A sodium sconce by every door.
let typicalFloor: Floor = {
  rooms: [
    room("stair", Stair, stairRect, []),
    room("lifts", LiftLobby, liftRect, liftPanels),
    room("corridor S", Corridor, rect(5.2, 6.4, 18.8, 8.8), alongX(7.6, 6.8, 4.0, 3.0, true)),
    room("corridor W", Corridor, rect(5.2, 8.8, 7.6, 15.2), alongZ(6.4, 10.4, 2.0, 3.0, true)),
    room("corridor N", Corridor, rect(5.2, 15.2, 18.8, 17.6), alongX(16.4, 6.8, 4.0, 3.0, true)),
    room("corridor E", Corridor, rect(16.4, 8.8, 18.8, 15.2), alongZ(17.6, 10.4, 2.0, 3.0, true)),
    room("west office", Office, rect(0.0, 8.8, 5.2, 15.2), pair(rect(0.0, 8.8, 5.2, 15.2), true)),
    room("south office", Office, rect(5.2, 0.0, 12.0, 6.4), pair(rect(5.2, 0.0, 12.0, 6.4), true)),
  ],
  doors: [
    door(2.0, 0.0, 9.2),
    mouth(2.0, 1.0, 13.6),
    throughway(2.0, 3.0, 6.4),
    throughway(2.0, 5.0, 17.6),
    throughway(4.0, 3.0, 6.4),
    throughway(4.0, 5.0, 17.6),
    door(3.0, 6.0, 12.0),
    door(2.0, 7.0, 8.6),
  ],
  windows: [window(6.0, West, 12.0), window(7.0, South, 8.8)],
  fixtures: List.flatten([
    [{ room: 2.0, side: North, at: 9.2, sign: StairSign },
     { room: 2.0, side: South, at: 15.4, sign: ShutDoor },
     { room: 5.0, side: East, at: 12.0, sign: ShutDoor },
     { room: 4.0, side: North, at: 8.6, sign: ShutDoor },
     { room: 4.0, side: North, at: 15.4, sign: ShutDoor }],
    liftDoors(1.0),
  ]),
  lamps: List.flatten([
    stairLamps(0.0),
    [{ room: 2.0, side: North, at: 10.1, y: 2.0 }, { room: 2.0, side: South, at: 7.6, y: 2.0 },
     { room: 2.0, side: South, at: 16.3, y: 2.0 }, { room: 3.0, side: West, at: 13.0, y: 2.0 },
     { room: 5.0, side: East, at: 13.0, y: 2.0 }, { room: 4.0, side: North, at: 9.5, y: 2.0 },
     { room: 4.0, side: North, at: 16.3, y: 2.0 }],
  ]),
  // The south office, abandoned: a desk pushed off its place, its chair
  // over. (More would outgrow the floor's 512² bake.)
  furniture: [
    piece(7.0, Desk, 10.9, 2.2, West, 0.9),
    piece(7.0, Chair, 10.0, 2.4, East, 0.8),
  ],
}
let cellsPerFloor = 8.0

// The roof: the stairwell's head room, a skylight slot over the well, its
// door onto a terrace along the south parapet (the rest is plant room).
let roof: Floor = {
  rooms: [room("stair head", Stair, stairRect, []), room("terrace", Roof, rect(0.0, 0.0, 24.0, 8.8), [])],
  doors: [door(0.0, 1.0, 9.2)],
  windows: [],
  fixtures: [{ room: 0.0, side: North, at: 12.0, sign: Skylight }],
  lamps: [landingLamp(0.0)],
  furniture: [],
}

// A stairwell with no doors, for a bake layout's neighbouring floor: under
// a floor, or over one (no flight above, so no half-landing lamp).
let shaft: Floor = { rooms: [room("shaft", Stair, stairRect, [])], doors: [], windows: [], fixtures: [], lamps: stairLamps(0.0), furniture: [] }
let shaftTop: Floor = { shaft with lamps: [landingLamp(0.0)] }
// The street: the ground floor's apron and west yard.
let street: Floor = { rooms: [apron, westYard], doors: [], windows: [], fixtures: [], lamps: [], furniture: [] }

// `f` on `level`, its ids moved up by `base`.
let shift = (base: float, level: float, f: Floor): Floor => {
  rooms: f.rooms |> List.map((r) => { r with level: level }),
  doors: f.doors |> List.map((d) => { d with a: d.a + base, b: d.b + base }),
  windows: f.windows |> List.map((w) => { w with room: w.room + base }),
  fixtures: f.fixtures |> List.map((x) => { x with room: x.room + base }),
  lamps: f.lamps |> List.map((x) => { x with room: x.room + base }),
  furniture: f.furniture |> List.map((x) => { x with room: x.room + base }),
}
// Floors in order, each on its level, ids running on.
let stack = (floors: List<(float, Floor)>): Floor =>
  let (out, _) = floors |> List.fold((acc, e) =>
    let (sum, base) = acc in
    let (level, f) = e in
    let s = shift(base, level, f) in
    ({ rooms: List.append(s.rooms, sum.rooms), doors: List.append(s.doors, sum.doors), windows: List.append(s.windows, sum.windows),
       fixtures: List.append(s.fixtures, sum.fixtures), lamps: List.append(s.lamps, sum.lamps), furniture: List.append(s.furniture, sum.furniture) },
     base + List.length(f.rooms)),
    ({ rooms: [], doors: [], windows: [], fixtures: [], lamps: [], furniture: [] }, 0.0)) in
  out

// The dog-leg between stairwell rooms `lower` and `upper`: 9 risers of
// 0.183 m a lane, 0.28 m treads, round a 0.3 m well, from a 2 m landing.
let flight = (lower: float, upper: float): Flight =>
  { kind: DogLeg(0.3, 2.0), lower: lower, upper: upper, x0: 0.0, z0: 0.0, z1: 0.0, risers: 18.0, tread: 0.28 }
// A flight up from each stairwell room to the next one in `f`, a level up.
let flightsOf = (f: Floor): List<Flight> =>
  let levelOf = (i: float): float => List.nth(i, f.rooms) |> Option.map((r) => r.level) |> Option.defaultValue(-1.0) in
  let stairs = List.range(List.length(f.rooms)) |> List.filter((i) => (List.nth(i, f.rooms) |> Option.map((r) => r.kind == Stair)) == Option.Some(true)) in
  List.zip(List.drop(1.0, stairs), List.take(List.length(stairs) - 1.0, stairs))
    |> List.filter((p) => let (lo, up) = p in levelOf(up) == levelOf(lo) + 1.0)
    |> List.map((p) => let (lo, up) = p in flight(lo, up))

// Sodium street lamps by the apron.
let lampHeads = [v(7.0, 4.8, -4.0), v(17.0, 4.8, -4.0)]

// `f` as a layout of `levels` storeys under a roofline at `top`.
let layoutOf = (levels: float, top: float, f: Floor): Layout => {
  footprint: footprint,
  ceiling: ceiling,
  storey: storey,
  levels: levels,
  facadeHeight: top,
  partition: partition,
  facade: facade,
  rooms: f.rooms,
  doors: f.doors,
  flights: flightsOf(f),
  windows: f.windows,
  fixtures: f.fixtures,
  lamps: f.lamps,
  furniture: f.furniture,
  bands: [],
  streetLamps: lampHeads,
  // On the apron, facing the entrance (+Z).
  spawn: { x: 12.0, z: 0.0 - 11.0 },
}

// The whole tower of `n` typical floors: ground, floors 1..n, the roof on
// level n + 1. Floor k's room j is cell k × `cellsPerFloor` + j.
let towerOf = (n: float): Layout =>
  layoutOf(n + 1.0, (n + 1.0) * storey + parapet, stack(List.flatten([
    [(0.0, ground)],
    List.range(n) |> List.map((k) => (k + 1.0, typicalFloor)),
    [(n + 1.0, roof)],
  ])))

// The typical floor's bake: it on level 1, between the stairwells of the
// floors below and above, so light and occlusion cross the shaft; the
// street it sees.
let typical = layoutOf(3.0, 3.0 * storey, stack([(0.0, shaft), (1.0, typicalFloor), (2.0, shaftTop), (0.0, street)]))
// The ground and roof's bake: the ground floor under the first floor's
// stairwell, and the roof (level 4) over the top floor's, a level clear.
let ends = layoutOf(5.0, 5.0 * storey, stack([(0.0, ground), (1.0, shaftTop), (3.0, shaft), (4.0, roof)]))
// The roof's level in `ends`.
let endsRoof = 4.0
