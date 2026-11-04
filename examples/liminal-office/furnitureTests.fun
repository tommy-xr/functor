// The furniture kit and the debris, in every archetype: each piece inside
// its room, its collider clear of the room's doors and of the e2e's walks;
// the debris capped and clear, the same for a copy and different between
// copies.

open Layout

let v = Vec3.make

// An archetype's layout, its rooms' insides as its pipeline places the
// furniture in them, and the e2e's walks over its ground floor.
type Case = { name: string, l: Layout, inner: (float) => Rect, walks: List<Streetscape.Path> }
let walk = (x0: float, z0: float, x1: float, z1: float): Streetscape.Path => { x0: x0, z0: z0, x1: x1, z1: z1 }
let kit = (l: Layout) => (i: float): Rect => Building.inner(l, Building.roomAt(l, i).rect)
let cases = [
  // The route up the lobby and the corridor.
  { name: "office", l: OfficeLayout.layout, inner: kit(OfficeLayout.layout), walks: [walk(10.0, -11.5, 10.0, 26.6)] },
  // From the apron to the stair door.
  { name: "tower ends", l: TowerLayout.ends, inner: kit(TowerLayout.ends), walks: [walk(12.0, -6.0, 9.2, 4.0), walk(9.2, 4.0, 9.2, 9.6)] },
  { name: "tower typical", l: TowerLayout.typical, inner: kit(TowerLayout.typical), walks: [] },
  // From the entrance across the lobby to the arch.
  { name: "cinema", l: CinemaLayout.layout, inner: CinemaBuilding.innerOf,
    walks: [walk(30.0, -4.0, 30.0, 2.0), walk(30.0, 2.0, 40.0, 9.0), walk(40.0, 9.0, 43.8, 13.0)] },
  // Out of bay 3's shopfront into the east hall.
  { name: "mall", l: MallLayout.layout, inner: MallInterior.inner, walks: [walk(70.0, 38.0, 70.0, 31.0)] },
  // In at the front door, along the hall to the stair.
  { name: "house", l: HouseLayout.layout, inner: kit(HouseLayout.layout), walks: [walk(1.6, -9.5, 1.6, 0.7), walk(1.6, 0.7, 9.2, 0.7)] },
]

let solids = (c: Case): List<(Furnish, Furniture.Bounds)> => c.l.furniture |> List.map((f) => (f, Furniture.solidOf(c.inner(f.room), f)))
let overlaps = (b: Furniture.Bounds, r: Rect): bool => b.x0 < r.x1 && r.x0 < b.x1 && b.z0 < r.z1 && r.z0 < b.z1

// Every archetype has some; most rooms have none.
expect cases |> List.all((c) => List.length(c.l.furniture) > 0)
expect cases |> List.all((c) => List.length(Debris.roomsOf(c.l)) * 3.0 <= List.length(c.l.rooms))
// Every piece stands inside its room, under its ceiling.
expect cases |> List.all((c) => solids(c) |> List.all((e) =>
  let (f, b) = e in
  let r = c.inner(f.room) in
  b.x0 >= r.x0 && b.x1 <= r.x1 && b.z0 >= r.z0 && b.z1 <= r.z1 && b.y0 > -0.001 && b.y1 < c.l.ceiling))
// Nothing stands within a metre of a door into its room.
expect cases |> List.all((c) => solids(c) |> List.all((e) =>
  let (f, b) = e in Debris.doorZones(c.l, f.room) |> List.all((z) => not overlaps(b, z))))
// Nothing on the ground floor blocks an e2e walk (`Streetscape.clearance` off it).
expect cases |> List.all((c) => solids(c) |> List.all((e) =>
  let (f, b) = e in
  Building.roomAt(c.l, f.room).level != 0.0
    || not (c.walks |> List.any((p) => Streetscape.blocks(Exterior.box(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1), p)))))

// The bakes hold the furniture, and each piece is a collider: the office's,
// the house's (both palettes), the towers' (a floor at a time) and the
// cinema's from the kit, the mall's among its items.
let furnitureFaces = (k: Building.Interior): float => k.allFaces |> List.filter((f) => f.mat >= Building.swatchBase) |> List.length
expect furnitureFaces(Archetypes.officeInterior) > 0.0 && furnitureFaces(Archetypes.ochreInterior) > 0.0
  && furnitureFaces(Tower.typicalInterior) > 0.0 && furnitureFaces(Tower.endsInterior) > 0.0
expect (CinemaBuilding.interior.allFaces |> List.any((f) => f.mat >= CinemaBuilding.furnishBase))
expect List.length(MallInterior.itemsOf(MallLayout.eastHall)) == List.length(MallLayout.layout.furniture)
let tagged = (cs: List<Building.Collider>): float => cs |> List.filter((k) => Text.contains("furniture-", k.tag)) |> List.length
expect tagged(Archetypes.office.colliders) == List.length(OfficeLayout.layout.furniture)
expect tagged(Archetypes.houseOchre.colliders) == List.length(HouseLayout.layout.furniture)
expect tagged(Archetypes.cinema.colliders) == List.length(CinemaLayout.layout.furniture)
// The tower's ground floor, and every typical floor of the tallest.
expect tagged(Archetypes.tower16.colliders) == 9.0 + 16.0 * 2.0

// The office's mess, pinned: two chairs over in 202 and one in 102, a
// drawer out in each, and every desk turned off square.
let office = OfficeLayout.layout
let inner = kit(office)
let height = (f: Furnish): float => let b = Furniture.solidOf(inner(f.room), f) in b.y1 - b.y0
let tippedIn = (i: float): float => office.furniture |> List.filter((f) => f.room == i && f.piece == Chair && height(f) < 0.9) |> List.length
expect tippedIn(16.0) == 2.0 && tippedIn(5.0) == 1.0
let depth = (f: Furnish): float => let b = Furniture.solidOf(inner(f.room), f) in Math.max(b.x1 - b.x0, b.z1 - b.z0)
expect [5.0, 16.0] |> List.all((i) => office.furniture |> List.any((f) => f.room == i && f.piece == Cabinet && depth(f) > 0.8))

// ---- Debris --------------------------------------------------------------------

let seeds = List.range(4.0) |> List.map((k) => Debris.seedOf(Block.copyAt(k)))
let strewnIn = (c: Case, seed: float): List<Debris.Candidate> => Debris.candidatesOf(seed, c.l)
let numbers = (cs: List<Debris.Candidate>): List<float> =>
  cs |> List.concatMap((d) => [d.cell, Vec3.x(d.at), Vec3.y(d.at), Vec3.z(d.at), d.yaw])
// At most `cap` a room, only in furnished rooms, every one inside its room
// and clear of the furniture and the doors, its body's box no lower than
// the floor.
expect cases |> List.all((c) => seeds |> List.all((s) =>
  let ds = strewnIn(c, s) in
  (Debris.roomsOf(c.l) |> List.all((i) => (ds |> List.filter((d) => d.cell == i) |> List.length) <= Debris.cap))
    && (ds |> List.all((d) =>
      let r = Building.inner(c.l, Building.roomAt(c.l, d.cell).rect) in
      let rad = Math.max(Vec3.x(d.shape), Vec3.z(d.shape)) / 2.0 in
      List.length(Furniture.inRoom(c.l, d.cell)) > 0.0
        && Vec3.x(d.at) - rad >= r.x0 && Vec3.x(d.at) + rad <= r.x1 && Vec3.z(d.at) - rad >= r.z0 && Vec3.z(d.at) + rad <= r.z1
        && Debris.clearOf(Debris.keepsOf(c.l, d.cell), Vec3.x(d.at), Vec3.z(d.at), rad)
        && Vec3.y(d.at) - Vec3.y(d.shape) / 2.0 >= Building.baseOf(c.l, Building.roomAt(c.l, d.cell)) - 0.001))))
// Every room gets some, every tag is its own.
expect cases |> List.all((c) => Debris.roomsOf(c.l) |> List.all((i) => strewnIn(c, 1.0) |> List.any((d) => d.cell == i)))
expect cases |> List.all((c) =>
  let tags = strewnIn(c, 1.0) |> List.map((d) => d.tag) in
  tags |> List.all((t) => (tags |> List.filter((u) => u == t) |> List.length) == 1.0))
// A copy's debris is the same every time; no two of the block's copies match.
let ofCopy = (k: float): List<float> => numbers(Debris.candidatesOf(Debris.seedOf(Block.copyAt(k)), office))
expect ofCopy(1.0) == ofCopy(1.0)
expect List.range(4.0) |> List.all((a) => List.range(4.0) |> List.all((b) =>
  a == b || ofCopy(a) != ofCopy(b)))
