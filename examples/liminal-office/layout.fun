// The building kit's vocabulary: a `Layout` is one archetype's plain data,
// which rendering, collision, culling cells and the lightmap bake all derive
// from (building.fun, portals.fun, exterior.fun, shells.fun). Each archetype
// has its own `<name>Layout.fun` (officeLayout.fun). Editing this file makes
// every committed bake stale: rebake (README).
//
// Metres, Y up, +Z north. Rects run between wall CENTRE lines, so two rooms
// sharing a wall share an edge; a cell is a room's rect, and a door is a
// portal on that shared edge.

type Rect = { x0: float, z0: float, x1: float, z1: float }

// `Roof`: open to the sky on a roof; the street side draws it. `StairHall`: a
// carpeted stairwell (the house's).
type Kind =
  | Outside
  | Lobby
  | Corridor
  | Office
  | Stair
  | LiftLobby
  | Roof
  | StairHall

type Side =
  | North
  | South
  | East
  | West

// A ceiling panel's centre; `on` panels are baked as lights.
type Panel = { x: float, z: float, on: bool }

// `level` 0 is the ground floor, 1 the floor above, and so on.
type Room = { name: string, kind: Kind, level: float, rect: Rect, panels: List<Panel> }

// An opening in the wall rooms `a` and `b` share, centred at `at` along it.
type Door = { a: float, b: float, at: float, width: float, height: float }

// How a flight climbs. Straight: risers in one run, onto a landing.
// DogLeg: half the risers north up the west lane from a floor landing
// `landing` deep, a half-landing across the north end, then the rest south
// down the east lane onto the next floor's landing; the lanes are split by a
// `well` wide gap down the room's middle.
type FlightKind =
  | Straight
  | DogLeg(well: float, landing: float)

// A flight of `risers` steps up one storey, from room `lower` to `upper`
// (the same rect, a level up), one `tread` per step. Straight: it runs east
// from `x0` along the lower room's south wall, between `z0` and `z1` (its
// parapet included), onto a landing that fills the rest of the east end
// (DogLeg ignores `x0`, `z0` and `z1`).
type Flight = { kind: FlightKind, lower: float, upper: float, x0: float, z0: float, z1: float, risers: float, tread: float }

// An opening in room `room`'s outside wall on `side`, centred at `at` along
// it, from `sill` up to `head`. Open (no glass): daylight enters the day bake
// only through these and the entrance.
type Window = { room: float, side: Side, at: float, width: float, sill: float, head: float }

// Signs on a room's inner wall, centred at `at` along it.
// A stair sign is a lit amber plate over a stairwell door; a lift door and a
// shut door are plates on the floor. A skylight is a slot in the room's
// ceiling (`side` unused), centred `at` along its z. A niche is a panel
// floor to head height, shaded at its edges like a recess.
type Sign =
  | ExitSign
  | StairSign
  | LiftDoor
  | ShutDoor
  | Skylight
  | Niche

type Fixture = { room: float, side: Side, at: float, sign: Sign }

// Wall lamps, `y` above the room's floor: an amber lens on the wall, and a
// light in the bakes only.
type Lamp = { room: float, side: Side, at: float, y: float }

// A dark window band in the facade on footprint side `side`, on `level`,
// centred `at` along it (exterior.fun; not baked).
type Band = { level: float, side: Side, at: float }

// Where the player starts, on the ground.
type Spot = { x: float, z: float }

// Box-built furniture, baked into its room (furniture.fun). `Stack`: four
// side chairs stacked in a corner.
type Article =
  | Desk
  | Chair
  | SideChair
  | Stack
  | Couch
  | CoffeeTable
  | Cabinet
  | Counter
  | Bookshelf
  | Cooler
  | Plant
  | Bed
  | Bench
  | Kiosk

// A piece in room `room`, centred (x, z), its front toward `facing`.
// `mess` (0 to 1) scales its disorder: turned or pushed off its place, a
// chair tipped, a drawer pulled out; which, from the noise at (x, z).
type Furnish = { room: float, piece: Article, x: float, z: float, facing: Side, mess: float }

// One archetype. `levels` storeys, `storey` floor to floor; `facadeHeight`
// is the roofline. Room ids are list positions, and they are the cell ids.
// `bands` and `streetLamps` (lamp heads, in its space) are the street side's.
// The player starts at `spawn`. Most rooms have no `furniture`.
type Layout = {
  footprint: Rect,
  ceiling: float,
  storey: float,
  levels: float,
  facadeHeight: float,
  partition: float,
  facade: float,
  rooms: List<Room>,
  doors: List<Door>,
  flights: List<Flight>,
  windows: List<Window>,
  fixtures: List<Fixture>,
  lamps: List<Lamp>,
  furniture: List<Furnish>,
  bands: List<Band>,
  streetLamps: List<Vec3.t>,
  spawn: Spot,
}

let rect = (x0: float, z0: float, x1: float, z1: float): Rect => { x0: x0, z0: z0, x1: x1, z1: z1 }

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

let roomAtLevel = (level: float, name: string, kind: Kind, r: Rect, panels: List<Panel>): Room =>
  { name: name, kind: kind, level: level, rect: r, panels: panels }
