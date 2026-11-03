// Layout, cell and derivation checks, run by `functor -d examples/liminal-office test`.
// (Not hashed into the bake's provenance, so editing them needs no rebake.)

open Layout

// The office, as the kit derives it.
let archetype = Archetypes.office
let l = archetype.layout
let kit = Archetypes.officeInterior
let street = archetype.street
let shell = Shells.shellAt("office")
let level = archetype.level
let rooms = l.rooms
let doors = l.doors
let flights = l.flights
let windows = l.windows
let footprint = l.footprint
let storey = l.storey
let ceiling = l.ceiling
let partition = l.partition
let facade = l.facade
let facadeHeight = l.facadeHeight
let spawn = l.spawn

let n = List.length(rooms)
let ids = List.range(n)
let interiorIds = ids |> List.filter((i) => Building.isInside(Building.roomAt(l, i)))

let overlap = (a: Rect, b: Rect): float =>
  Math.max(0.0, Math.min(a.x1, b.x1) - Math.max(a.x0, b.x0))
    * Math.max(0.0, Math.min(a.z1, b.z1) - Math.max(a.z0, b.z0))
let within = (outer: Rect, r: Rect): bool =>
  r.x0 >= outer.x0 && r.x1 <= outer.x1 && r.z0 >= outer.z0 && r.z1 <= outer.z1
let pairs = ids |> List.concatMap((i) => ids |> List.filter((j) => j > i) |> List.map((j) => (i, j)))
let near = Building.near

// Rooms on one level don't overlap; every inside room is within the
// footprint, on level 0 or 1.
expect pairs |> List.all((p) =>
  let (i, j) = p in Building.roomAt(l, i).level != Building.roomAt(l, j).level || overlap(Building.roomAt(l, i).rect, Building.roomAt(l, j).rect) < 0.0001)
expect interiorIds |> List.all((i) => within(footprint, Building.roomAt(l, i).rect))
expect rooms |> List.all((r) => r.level == 0.0 || r.level == 1.0)
// Both floors have rooms; upstairs is all inside.
expect interiorIds |> List.any((i) => Building.roomAt(l, i).level == 1.0)
expect rooms |> List.all((r) => r.level == 0.0 || Building.isInside(r))

// The stretch of wall two rooms share, along it.
let shared = (d: Door): Building.Hole =>
  let a = Building.roomAt(l, d.a).rect in
  let b = Building.roomAt(l, d.b).rect in
  let side = Building.sideBetween(a, b) in
  if side == Option.Some(North) || side == Option.Some(South)
  then { lo: Math.max(a.x0, b.x0), hi: Math.min(a.x1, b.x1), bottom: 0.0, top: 0.0 }
  else { lo: Math.max(a.z0, b.z0), hi: Math.min(a.z1, b.z1), bottom: 0.0, top: 0.0 }

// Doors join two rooms on one level across a shared wall, inside its span
// with a jamb's worth of wall on each side, and are big enough to walk through.
expect doors |> List.all((d) => Building.roomAt(l, d.a).level == Building.roomAt(l, d.b).level)
expect doors |> List.all((d) => Building.sideBetween(Building.roomAt(l, d.a).rect, Building.roomAt(l, d.b).rect) != Option.None)
expect doors |> List.all((d) =>
  let s = shared(d) in
  let h = Building.holeOf(d) in
  h.lo >= s.lo + partition / 2.0 - 0.0001 && h.hi <= s.hi - partition / 2.0 + 0.0001)
expect doors |> List.all((d) => d.width >= 0.9 && d.height >= 2.0 && d.height <= ceiling)
// Holes in one wall never overlap.
expect interiorIds |> List.all((i) => [North, South, East, West] |> List.all((s) =>
  let holes = Building.holesIn(l, i, s) in
  List.zip(holes, List.drop(1.0, holes)) |> List.all((p) => let (next, prev) = p in prev.hi < next.lo)))

// Every room is reachable from the apron: through doors, up the stairs, or
// (the street beyond them) through windows.
let neighbours = (i: float): List<float> =>
  List.flatten([
    windows |> List.concatMap((w) =>
      match Building.outsideOf(l, w) with
      | Option.Some(o) => if w.room == i then [o] else if o == i then [w.room] else []
      | Option.None => []),
    doors |> List.concatMap((d) => if d.a == i then [d.b] else if d.b == i then [d.a] else []),
    flights |> List.concatMap((f) => if f.lower == i then [f.upper] else if f.upper == i then [f.lower] else []),
  ])
let reached = ids |> List.fold((seen, k) =>
  seen |> List.concatMap(neighbours) |> List.filter((j) => not List.any((s) => s == j, seen))
    |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else acc |> List.append([j]), seen), [0.0])
expect List.length(reached) == n

// Clearances: rooms and the corridor leave room for the 0.6 m capsule.
expect interiorIds |> List.all((i) =>
  let r = Building.inner(l, Building.roomAt(l, i).rect) in Math.min(r.x1 - r.x0, r.z1 - r.z0) >= 2.0)
// Panels sit inside their room, clear of the walls.
expect rooms |> List.all((r) =>
  let c = Building.inner(l, r.rect) in
  r.panels |> List.all((p) => p.x - 0.6 > c.x0 && p.x + 0.6 < c.x1 && p.z - 0.6 > c.z0 && p.z + 0.6 < c.z1))

// ---- Stairs ----------------------------------------------------------------

// A flight's rooms are one stairwell: the same rect, a level apart.
expect flights |> List.all((f) =>
  let lo = Building.roomAt(l, f.lower) in
  let up = Building.roomAt(l, f.upper) in
  lo.kind == Stair && up.kind == Stair && lo.rect == up.rect && up.level == lo.level + 1.0)
// The flight runs along the south wall, with room to step on at its foot
// and a landing at its head, and lies inside the stairwell.
let landingWidth = (f: Flight): float =>
  Building.inner(l, Building.roomAt(l, f.lower).rect).x1 - Building.flightEnd(f) - Building.parapetThickness
expect flights |> List.all((f) =>
  let r = Building.inner(l, Building.roomAt(l, f.lower).rect) in
  near(f.z0, r.z0) && f.z1 < r.z1 && f.x0 - r.x0 >= 1.0 && landingWidth(f) >= 1.2)
// Clear width between the wall and the parapet; headroom under the landing.
expect flights |> List.all((f) => f.z1 - Building.parapetThickness - f.z0 >= 1.2)
expect storey - Building.slabDepth >= 2.4
// Comfortable steps, and a ramp the character climbs (maxSlope 40°).
let slope = (f: Flight): float => Math.atan(Building.riserOf(l, f) / f.tread) * 180.0 / Math.pi
expect flights |> List.all((f) => Building.riserOf(l, f) <= 0.19 && f.tread >= 0.28 && slope(f) <= 35.0)
// The ramp starts on the floor, passes through every step's nosing, and
// meets the landing at its edge.
expect flights |> List.all((f) =>
  near(Building.rampY(l, f, f.x0 - f.tread), 0.0)
    && (List.range(f.risers - 1.0) |> List.all((j) =>
      near(Building.rampY(l, f, f.x0 + j * f.tread), (j + 1.0) * Building.riserOf(l, f))))
    && near(Building.rampY(l, f, Building.flightEnd(f)), storey))
// The upper room's door opens onto the landing's walkway.
expect flights |> List.all((f) =>
  let r = Building.inner(l, Building.roomAt(l, f.upper).rect) in
  doors |> List.filter((d) => d.a == f.upper || d.b == f.upper) |> List.all((d) =>
    let h = Building.holeOf(d) in
    h.lo >= Building.flightEnd(f) + Building.parapetThickness && h.hi <= r.x1))
// Every face of a cell's mesh lies inside that cell, so what a cell draws
// is only ever seen from it or through its portals: the stairs split at
// the upper floor, doors and windows at their wall's centre line.
let inBox = (i: float, p: Vec3.t): bool =>
  let r = Building.roomAt(l, i) in
  let (y0, y1) = if Building.isInside(r) then (Building.baseOf(l, r), Building.baseOf(l, r) + Building.wallTop(l, i)) else (0.0, Portals.outsideTop) in
  let e = 0.0001 in
  Vec3.x(p) >= r.rect.x0 - e && Vec3.x(p) <= r.rect.x1 + e && Vec3.z(p) >= r.rect.z0 - e && Vec3.z(p) <= r.rect.z1 + e
    && Vec3.y(p) >= y0 - e && Vec3.y(p) <= y1 + e
expect ids |> List.all((i) => Building.cellFaces(l, i) |> List.all((f) => Building.corners(f.q) |> List.all((p) => inBox(i, p))))
// Stacked windows share their patch of sky-lit ground in the day bake.
expect windows |> List.all((w) => Building.stacked(l, w.side, Building.windowHole(w)) == 2.0)

// ---- The kit -----------------------------------------------------------------

// The office's Layout derives the cell graph it had as constants: 23 rooms
// plus 2 collar strips, 54 portals between the same cells in the same order,
// 558 faces and 173 colliders; and cell ids stay room positions (the e2e's).
expect List.length(Portals.cellsOf(l, Portals.outsideTop)) == 25.0 && List.length(Portals.portalsOf(l, Portals.outsideTop)) == 54.0
expect List.length(kit.allFaces) == 558.0 && List.length(kit.colliders) == 173.0
expect Text.join(" ", Portals.portalsOf(l, Portals.outsideTop) |> List.map((p) => $"{p.from}-{p.to}"))
  == "0-1 1-0 1-2 2-1 2-3 3-2 4-1 1-4 1-5 5-1 6-2 2-6 2-7 7-2 8-2 2-8 2-9 9-2 10-2 2-10 2-11 11-2 13-14 14-13 15-13 13-15 13-16 16-13 17-13 13-17 13-18 18-13 19-13 13-19 13-20 20-13 21-13 13-21 14-12 12-14 22-6 6-22 22-17 17-22 11-12 12-11 0-22 22-0 0-24 24-0 23-22 22-23 23-24 24-23"
expect ([0.0, 1.0, 2.0, 9.0, 11.0, 12.0, 13.0, 14.0, 20.0, 22.0] |> List.map((i) => Building.roomAt(l, i).name))
  == ["apron", "lobby", "corridor", "106", "stair B", "stair B 2", "corridor 2", "turn 2", "206", "west yard"]
// Every copy in every town names an archetype, and draws its cells.
expect Block.towns |> List.all((t) => t.copies |> List.all((c) => Block.archetypeOf(c).name == c.archetype))

// ---- Cells and portals -------------------------------------------------------

// Each room's middle, at eye height, is in its own cell; the spawn is on the apron.
let middle = (i: float): Vec3.t =>
  let r = Building.roomAt(l, i).rect in
  Vec3.make((r.x0 + r.x1) / 2.0, Building.baseOf(l, Building.roomAt(l, i)) + 1.6, (r.z0 + r.z1) / 2.0)
expect ids |> List.all((i) => Cells.locate(middle(i), level) == Option.Some(i))
expect Cells.locate(Vec3.make(spawn.x, 1.6, spawn.z), level) == Option.Some(0.0)
// Sight runs through doorways and stops at walls.
expect Cells.segment(Vec3.make(10.0, 1.6, -6.0), Vec3.make(10.0, 1.6, 20.0), level)
expect Cells.segment(Vec3.make(10.0, 1.6, 16.5), Vec3.make(15.6, 1.6, 16.5), level)
expect not Cells.segment(Vec3.make(10.0, 1.6, 10.0), Vec3.make(15.6, 1.6, 16.5), level)
// The stair opening is a portal between floors: sight runs up through it,
// but not through the landing, nor through any other floor.
let up = storey + 1.6
expect Cells.segment(Vec3.make(12.0, 1.6, 23.0), Vec3.make(15.0, up, 25.0), level)
expect not Cells.segment(Vec3.make(19.0, 1.6, 24.0), Vec3.make(19.0, up, 24.0), level)
expect not Cells.segment(Vec3.make(10.0, 1.6, 10.0), Vec3.make(10.0, up, 10.0), level)
// Upstairs, through doorways as below.
expect Cells.segment(Vec3.make(10.0, up, 3.0), Vec3.make(15.6, up, 3.0), level)
// Every window is a portal from the street: sight runs in through one,
// not through the wall beside it.
expect windows |> List.all((w) => Building.outsideOf(l, w) != Option.None)
expect Cells.segment(Vec3.make(-5.0, 1.6, 9.3), Vec3.make(4.0, 1.6, 9.3), level)
expect Cells.segment(Vec3.make(-5.0, up, 9.3), Vec3.make(4.0, up, 9.3), level)
expect not Cells.segment(Vec3.make(-5.0, 1.6, 16.0), Vec3.make(4.0, 1.6, 16.0), level)

// ---- The mesh and its cells ----------------------------------------------------

// Every face's material has a group, and the committed bake's mesh has
// exactly the derived triangles.
let total = 2.0 * List.length(kit.allFaces)
expect (kit.meshSpans |> List.map((s) => s.count) |> List.sum) == total
expect (kit.cellRanges |> List.map((r) => r.count) |> List.sum) == total
expect archetype.status != Provenance.Current
  || Bytes.length(Lightmap.readMesh(File.bytes(Provenance.meshPathOf(Archetypes.officeBake))).triangles) == 12.0 * total

// The split into cell meshes loses and repeats nothing: the cells' triangle
// ranges and vertex runs tile the mesh in order, each cell's triangles index
// only its own vertices, and putting the cells back together (indices plus
// each run's start; vertex arrays end to end) gives the mesh byte for byte,
// UV2 included.
let u32s = (b: Bytes.t): List<float> => Decode.run(Decode.list(Bytes.length(b) / 4.0, Decode.u32le), b) |> Binary.require
let splitsExactly = (data: Model.staticData): bool =>
  let runs = Building.runsOf(kit, data) in
  let cells = runs |> List.map((run) => (run, Building.cellData(kit, data, run))) in
  let tiled = runs |> List.fold((acc, run) =>
    let (ok, first, v) = acc in (ok && run.first == first && run.v0 == v && run.v1 > run.v0, first + run.count, run.v1),
    (true, 0.0, 0.0)) in
  let (ok, triangles, vertices) = tiled in
  let rejoined = (pick: (Model.staticData) => Bytes.t) => Bytes.concat(cells |> List.map((c) => let (run, d) = c in pick(d))) in
  ok && triangles == total && vertices == Bytes.length(data.positions) / 12.0
    && (cells |> List.all((c) => let (run, d) = c in u32s(d.triangles) |> List.all((i) => i < run.v1 - run.v0)))
    && Bytes.equals(Bytes.fromU32le(cells |> List.concatMap((c) =>
         let (run, d) = c in u32s(d.triangles) |> List.map((i) => i + run.v0))), data.triangles)
    && Bytes.equals(rejoined((d) => d.positions), data.positions)
    && Bytes.equals(rejoined((d) => d.normals), data.normals)
    && Bytes.equals(rejoined((d) => d.uvs), data.uvs)
    && Bytes.equals(rejoined((d) => d.uvs2), data.uvs2)
expect splitsExactly(kit.interiorData)
expect archetype.status != Provenance.Current
  || splitsExactly(Lightmap.readMesh(File.bytes(Provenance.meshPathOf(Archetypes.officeBake))))
// The baked mesh has lightmap UVs to keep.
expect archetype.status != Provenance.Current
  || Bytes.length(Lightmap.readMesh(File.bytes(Provenance.meshPathOf(Archetypes.officeBake))).uvs2) > 0.0

// A door's reveal spans its wall's full thickness: its two halves'
// thresholds together. Doors from outside go through the facade; the rest
// through partitions.
let depth = (d: Door, i: float): float =>
  match List.last(Building.doorHalf(l, d, i)) with
  | Option.Some(f) => Vec3.length(Vec3.cross(f.q.b, f.q.a)) / d.width
  | Option.None => 0.0
expect doors |> List.all((d) =>
  let wall = if Building.isInside(Building.roomAt(l, d.a)) then partition else facade in
  Math.abs(depth(d, d.a) + depth(d, d.b) - wall) < 0.0001 && Math.abs(depth(d, d.a) - depth(d, d.b)) < 0.0001)

// Collider tags are unique.
let tags = kit.blocks |> List.map((b) => b.tag)
expect tags |> List.indexedMap((i, t) => List.drop(i + 1.0, tags) |> List.any((u) => u == t)) |> List.all((dup) => not dup)

// Windows sit in an outside wall of an inside room, clear of its corners,
// with a sill and a head under the ceiling.
expect windows |> List.all((w) =>
  let r = Building.roomAt(l, w.room) in
  let c = Building.inner(l, r.rect) in
  let (lo, hi) = if w.side == North || w.side == South then (c.x0, c.x1) else (c.z0, c.z1) in
  Building.isInside(r) && Building.onFootprint(l, w.side, r.rect)
    && w.at - w.width / 2.0 > lo + 0.3 && w.at + w.width / 2.0 < hi - 0.3
    && w.sill >= 0.8 && w.head < ceiling - 0.2)
// Each window has a sill collider: its floor to its sill, across the
// window, in its wall.
expect windows |> List.all((w) =>
  let line = Building.lineOf(w.side, Building.roomAt(l, w.room).rect) in
  let y = Building.baseOf(l, Building.roomAt(l, w.room)) in
  kit.blocks |> List.any((b) =>
    near(b.y - b.h / 2.0, y) && near(b.y + b.h / 2.0, y + w.sill)
      && (if w.side == North || w.side == South then near(b.z, line) && near(b.x, w.at) && near(b.w, w.width)
          else near(b.x, line) && near(b.z, w.at) && near(b.d, w.width))))

// ---- Hours -------------------------------------------------------------------

// Hours → daylight: night at midnight, day at noon, crossing at dawn and dusk.
expect Hours.daylight(0.0) == 0.0 && Hours.daylight(4.0) == 0.0 && Hours.daylight(21.0) == 0.0
expect Hours.daylight(12.0) == 1.0
expect Hours.daylight(6.0) < Hours.daylight(7.0) && Hours.daylight(18.0) > Hours.daylight(19.0)
// The bake switches where daylight reaches the threshold: 6:30 and 18:30.
expect Hours.daylight(6.5) == Hours.dayThreshold && Hours.daylight(18.5) == Hours.dayThreshold
expect not Hours.isDay(6.0) && Hours.isDay(6.5) && Hours.isDay(7.0)
expect Hours.isDay(18.0) && not Hours.isDay(19.0)

// The outdoor lerp's endpoints: night exactly as before the hour existed
// (so night frames don't change), day with the outdoor sun.
let night = Hours.outdoorAt(0.0)
let noon = Hours.outdoorAt(1.0)
expect night.sky == Hours.nightSky && night.fogNear == 35.0 && night.sunIntensity == 0.6
expect night.neon == 1.0 && night.lamps == 1.0
expect Vec3.length(night.sunDirection |> Vec3.sub(Hours.moonDirection)) == 0.0
expect noon.sky == Hours.daySky && noon.fogNear == 70.0 && noon.sunIntensity == 2.6
expect noon.neon == 0.7 && noon.lamps == 0.0
expect Vec3.length(noon.sunDirection |> Vec3.sub(Hours.sunDirection)) < 0.000001
// Halfway, the sky is dusk.
expect Hours.outdoorAt(0.5).sky == Hours.duskSky

// ---- The collar ----------------------------------------------------------------

// The collar's cells: one strip per side, the apron and the west yard
// among them under their room ids.
let collarIds = Portals.sides |> List.map(Portals.stripId(l))
expect collarIds == [0.0, n, n - 1.0, n + 1.0]
// Every cell is reachable through portals from the south strip.
let portals = Portals.portalsOf(l, Portals.outsideTop)
let linked = (i: float): List<float> =>
  portals |> List.filter((p) => p.from == i) |> List.map((p) => p.to)
let cellCount = n + List.length(Portals.extraSides(l))
let reachedCells = List.range(cellCount) |> List.fold((seen, k) =>
  seen |> List.concatMap(linked) |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else acc |> List.append([j]), seen), [0.0])
expect List.length(reachedCells) == cellCount
// The street sees round the corners and in through the doors and windows:
// south strip → east strip; north strip → west strip → office 103's window.
expect Cells.segment(Vec3.make(30.0, 1.6, -5.0), Vec3.make(30.0, 1.6, 10.0), level)
expect Cells.segment(Vec3.make(-34.0, 1.6, 34.0), Vec3.make(2.0, 1.6, 8.0), level)
expect Cells.segment(Vec3.make(-34.0, up, 34.0), Vec3.make(2.0, up, 8.0), level)
expect Cells.segment(Vec3.make(10.0, 1.6, -30.0), Vec3.make(10.0, 1.6, 3.0), level)
expect not Cells.segment(Vec3.make(30.0, 1.6, 10.0), Vec3.make(15.0, 1.6, 10.0), level)

// The collar holds every eye that sees an interior: anywhere up to
// `activeOff` + `eyeReach` from the footprint (corners rounded, so the strips'
// square corners cover them), at street eye heights; the footprint itself
// is rooms on both floors.
let reach = Block.activeOff + Block.eyeReach
expect reach <= Portals.collar
let f = footprint
// The footprint grown by `d`, its outline every metre or so.
let outline = (d: float): List<(float, float)> => outlineOf(f, d)
let outlineOf = (f: Rect, d: float): List<(float, float)> =>
  let along = (a: float, b: float): List<float> => List.range(Math.ceil(b - a) + 1.0) |> List.map((k) => Math.min(b, a + k)) in
  let arc = (cx: float, cz: float, from: float): List<(float, float)> =>
    List.range(19.0) |> List.map((k) => let a = (from + 5.0 * k) * Math.pi / 180.0 in (cx + d * Math.sin(a), cz + d * Math.cos(a))) in
  List.flatten([
    along(f.x0, f.x1) |> List.concatMap((x) => [(x, f.z0 - d), (x, f.z1 + d)]),
    along(f.z0, f.z1) |> List.concatMap((z) => [(f.x0 - d, z), (f.x1 + d, z)]),
    arc(f.x1, f.z1, 0.0), arc(f.x1, f.z0, 90.0), arc(f.x0, f.z0, 180.0), arc(f.x0, f.z1, 270.0),
  ])
let held = (x: float, y: float, z: float): bool => Cells.locate(Vec3.make(x, y, z), level) != Option.None
let outside = List.range(Math.floor(reach) + 1.0) |> List.map((k) => Math.min(reach, k + 0.001)) |> List.concatMap(outline)
expect outside |> List.all((p) => let (x, z) = p in [0.3, 1.6, 2.6] |> List.all((y) => held(x, y, z)))
let inside = List.range(f.x1 - f.x0 + 1.0) |> List.concatMap((x) => List.range(f.z1 - f.z0 + 1.0) |> List.map((z) => (f.x0 + x, f.z0 + z)))
expect inside |> List.all((p) => let (x, z) = p in [1.2, 1.6, 2.2, storey + 1.2, storey + 1.6, storey + 2.2] |> List.all((y) => held(x, y, z)))
// A test that can fail: past the collar, nothing holds the eye.
expect not held(f.x0 - Portals.collar - 0.5, 1.6, 10.0)

// ---- The block -----------------------------------------------------------------

let block = Block.townAt(1.0)
let copies = block.copies
let allIds = (t: Block.Town): List<float> => List.range(List.length(t.copies))

// Four copies at 0/90/180/270°; the first is the office where it stands.
expect (copies |> List.map((c) => c.yaw)) == [0.0, 90.0, 180.0, 270.0]
expect Block.home == { name: "office-0", archetype: "office", x: 0.0, z: 0.0, yaw: 0.0 }
let close = (a: Vec3.t, b: Vec3.t): bool => Vec3.length(a |> Vec3.sub(b)) < 0.00001
// To the world and back; a heading turns with its copy: forward in the
// copy's space lands where its heading points in the world.
let probe = Vec3.make(3.0, 1.6, 7.0)
expect copies |> List.all((c) => close(Block.toLocal(c, Block.toWorld(c, probe)), probe))
expect copies |> List.all((c) =>
  let yaw = 0.4 in
  let ahead = Block.toWorld(c, probe |> Vec3.add(Vec3.make(Math.sin(yaw), 0.0, Math.cos(yaw)))) |> Vec3.sub(Block.toWorld(c, probe)) in
  let h = Block.headingToWorld(c, yaw) in
  close(ahead, Vec3.make(Math.sin(h), 0.0, Math.cos(h))))
// Quarter turns: the copy at 90° sends local +z to world +x.
expect close(Block.toWorld(Block.copyAt(1.0), Vec3.make(0.0, 0.0, 1.0)) |> Vec3.sub(Block.toWorld(Block.copyAt(1.0), Vec3.make(0.0, 0.0, 0.0))), Vec3.make(1.0, 0.0, 0.0))
// Each copy's footprint, and its outside rooms (apron, yard), in the world.
let worldRect = (c: Block.Copy, r: Rect): Rect =>
  let a = Block.toWorld(c, Vec3.make(r.x0, 0.0, r.z0)) in
  let b = Block.toWorld(c, Vec3.make(r.x1, 0.0, r.z1)) in
  rect(Math.min(Vec3.x(a), Vec3.x(b)), Math.min(Vec3.z(a), Vec3.z(b)), Math.max(Vec3.x(a), Vec3.x(b)), Math.max(Vec3.z(a), Vec3.z(b)))
let gap = (a: Rect, b: Rect): float =>
  Math.max(Math.max(a.x0 - b.x1, b.x0 - a.x1), Math.max(a.z0 - b.z1, b.z0 - a.z1))
let pairsOf = (cs: List<Block.Copy>): List<(Block.Copy, Block.Copy)> =>
  cs |> List.indexedMap((i, a) => List.drop(i + 1.0, cs) |> List.map((b) => (a, b))) |> List.flatten
let yards = rooms |> List.filter((r) => not Building.isInside(r))
// A street at least `streetWidth` wide between any two buildings, and no
// apron or yard on another building: in the block, and in the 64-copy grid.
let streetsBetween = (cs: List<Block.Copy>): bool =>
  pairsOf(cs) |> List.all((p) =>
    let (a, b) = p in
    gap(worldRect(a, f), worldRect(b, f)) >= Block.streetWidth - 0.001
      && (yards |> List.all((r) => gap(worldRect(a, r.rect), worldRect(b, f)) >= -0.001 && gap(worldRect(b, r.rect), worldRect(a, f)) >= -0.001)))
expect streetsBetween(copies)
// Every copy's entrance and windows face the crossroads' streets: their
// outside rooms lie in the cross between the buildings.
let inCross = (r: Rect): bool =>
  (r.x0 >= 0.0 - Block.streetWidth - 0.001 && r.x1 <= 0.001) || (r.z0 >= 0.0 - Block.streetWidth - 0.001 && r.z1 <= 0.001)
expect copies |> List.all((c) => yards |> List.all((r) => inCross(worldRect(c, r.rect))))

// Collider tags are unique across the copies, the grounds and the player.
let tagsOf = (cs: List<Block.Copy>): List<string> =>
  cs |> List.concatMap((c) => Block.collidersOf(c) |> List.map((k) => Block.tagOf(c, k.tag)))
let unique = (xs: List<string>): bool => Map.size(Map.fromList(xs |> List.map((x) => (x, true)))) == List.length(xs)
expect unique(List.flatten([
  ["ground", "block-ground", "island", "island-pad", "player"],
  IslandLayout.trees |> List.map((t) => $"island-tree-{t.id}"),
  IslandLayout.rocks |> List.map((r) => $"island-rock-{r.id}"),
  tagsOf(copies),
]))

// Activation: on inside `activeOn`, held to `activeOff`, off past it; only
// the home copy stands outside the block.
let office = Block.townAt(0.0)
let at = (d: float): Vec3.t => Vec3.make(f.x0 - d, 1.6, 10.0)
let homeOn = (was: bool, d: float): bool =>
  Block.isIn(0.0, Block.activeAt(office, Block.maxActive, if was then [0.0] else [], at(d)))
expect homeOn(false, Block.activeOn - 0.1) && not homeOn(false, Block.activeOn + 0.1)
expect homeOn(true, Block.activeOff - 0.1) && not homeOn(true, Block.activeOff + 0.1)
expect Block.activeAt(office, Block.maxActive, [0.0, 1.0, 2.0, 3.0], Block.toWorld(Block.copyAt(2.0), probe)) == []
expect Block.isIn(2.0, Block.activeAt(block, Block.maxActive, [], Block.toWorld(Block.copyAt(2.0), probe)))
// At the far edge of every copy's activation, its cells hold the eye.
expect copies |> List.all((c) =>
  outline(Block.activeOff) |> List.all((p) =>
    let (x, z) = p in
    let eye = Block.toWorld(c, Vec3.make(x, 1.6, z)) in
    Block.uncovered(block, Block.activeAt(block, Block.maxActive, [0.0, 1.0, 2.0, 3.0], eye), eye) == 0.0))
// From the crossroads every interior draws; from deep inside a copy only
// its own does.
let crossroads = Vec3.make(Block.centre, 1.6, Block.centre)
expect Block.activeAt(block, Block.maxActive, [], crossroads) == [0.0, 1.0, 2.0, 3.0]
expect Block.activeAt(block, Block.maxActive, [], Block.toWorld(Block.copyAt(1.0), Vec3.make(14.0, 1.6, 20.0))) == [1.0]

// Every copy draws the home copy's scene (one archetype: the same cell
// models and lightmaps) under its own transform.
let look = (c: Block.Copy): Game.CopyLook => { copy: c, neon: 1.0, lamps: 1.0, day: false, active: true }
let local = Scene.group([Exterior.exteriorAt(street, 1.0, 1.0, false), Scene.cells(level, Scene.group(archetype.night))])
expect copies |> List.all((c) => Scene.equals(Game.copyScene(look(c)), Block.sceneOf(c, local)))

// ---- The grids -------------------------------------------------------------------

let grid64 = Block.townAt(4.0)
expect (Block.towns |> List.map((t) => List.length(t.copies))) == [1.0, 4.0, 4.0, 16.0, 64.0, 4.0, 1.0, 1.0, 12.0]
expect (Block.towns |> List.map((t) => t.shells)) == [false, false, true, true, true, true, true, true, true]
// An id is the same office in every office town: each extends the smaller
// ones, and the grid of 4 stands where the block does.
expect List.take(5.0, Block.towns) |> List.all((t) => t.copies == List.take(List.length(t.copies), grid64.copies))
expect Block.townAt(2.0).copies == copies
// The blocks fill 2 × 2, then 4 × 4.
let cells = (n: float): List<(float, float)> => List.range(n) |> List.map(Block.ringCell)
expect cells(4.0) |> List.all((c) => let (x, z) = c in x <= 1.0 && z <= 1.0)
expect Map.size(Map.fromList(cells(16.0) |> List.map((c) => let (x, z) = c in (x * 4.0 + z, true)))) == 16.0
expect cells(16.0) |> List.all((c) => let (x, z) = c in x <= 3.0 && z <= 3.0)
// Streets between all 64; unique names (memo keys) and collider tags.
expect streetsBetween(grid64.copies)
expect unique(grid64.copies |> List.map((c) => c.name))
expect unique(List.flatten([["player", "ground", "block-ground", "grid-4-ground", "grid-16-ground", "grid-64-ground", "island", "island-pad"], tagsOf(grid64.copies)]))

// The index finds exactly the copies a scan of all 64 would, out to every
// radius it's asked for, from street level all over the grid and past it.
let scan = (t: Block.Town, r: float, eye: Vec3.t): List<(float, float)> =>
  t.copies |> List.indexedMap((id, c) => (id, Block.fromFootprint(Block.footprintOf(c), Block.toLocal(c, eye)))) |> List.filter((e) => let (id, d) = e in d <= r)
let probes = List.range(24.0) |> List.concatMap((i) => List.range(24.0) |> List.map((j) => Vec3.make(-90.0 + 23.0 * i, 1.6, -90.0 + 23.0 * j)))
expect probes |> List.all((p) =>
  [Block.activeOff, Block.lightReach, Block.solidOn + Block.solidMargin] |> List.all((r) => Block.near(grid64, r, p) == scan(grid64, r, p)))

// A tighter layout than any street allows: 64 copies 6 m apart.
let tight = (n: float, gapX: float, gapZ: float): Block.Town =>
  let side = Math.sqrt(n) in
  let cs = List.range(n) |> List.map((id) =>
    { name: $"tight-{id}", archetype: "office", x: Math.mod(id, side) * (f.x1 + gapX), z: Math.floor(id / side) * (f.z1 + gapZ), yaw: 0.0 }) in
  { name: "tight", copies: cs, index: Block.indexOf(cs), shells: true, solidOn: Block.solidOn, lightReach: Block.lightReach, ground: Exterior.ground, groundBody: Exterior.groundBody, reach: Hours.streetReach }
let tight64 = tight(64.0, 6.0, 6.0)
let tight16 = tight(16.0, 6.0, 6.0)
expect Block.near(tight64, Block.activeOff, Vec3.make(83.0, 1.6, 101.5)) == scan(tight64, Block.activeOff, Vec3.make(83.0, 1.6, 101.5))

// The selector folded along `eyes`: each eye's active set, from the last.
let walk = (t: Block.Town, k: float, eyes: List<Vec3.t>): List<List<float>> =>
  let (_, sets) = eyes |> List.fold((acc, eye) =>
    let (was, out) = acc in
    let now = Block.activeAt(t, k, was, eye) in
    (now, List.append([now], out)), ([], [])) in
  sets
let changes = (sets: List<List<float>>): float =>
  sets |> List.indexedMap((i, s) => if i > 0.0 && List.nth(i - 1.0, sets) != Option.Some(s) then 1.0 else 0.0) |> List.sum
let ks = [1.0, 2.0, 4.0]

// Dense: from an alley crossing more than k qualify, and k is the bound;
// with nothing on, the k nearest win.
let alley = Vec3.make(3.0 * (f.x1 + 6.0) - 3.0, 1.6, 3.0 * (f.z1 + 6.0) - 3.0)
expect List.length(Block.near(tight64, Block.activeOn, alley)) == 8.0
expect ks |> List.all((k) =>
  Block.activeAt(tight64, k, [], alley)
    == (Block.near(tight64, Block.activeOn, alley) |> List.sortBy((e) => let (id, d) = e in d) |> List.take(k)
          |> List.map((e) => let (id, d) = e in id) |> List.sortBy((id) => id)))
// Whatever was on, never more than k: everywhere over the grids.
expect [grid64, tight64] |> List.all((t) => ks |> List.all((k) => probes |> List.all((p) =>
  List.length(Block.activeAt(t, k, [], p)) <= k && List.length(Block.activeAt(t, k, allIds(t), p)) <= k)))

// Retention: an active copy stays on while the eye is in its band, though
// nearer ones qualify; with nothing on, the nearest wins.
let quad = Block.near(grid64, Block.activeOn, crossroads) |> List.sortBy((e) => let (id, d) = e in d) |> List.map((e) => let (id, d) = e in id)
expect List.length(quad) == 4.0
let farthest = List.last(quad) |> Option.defaultValue(-1.0)
expect Block.activeAt(grid64, 1.0, [farthest], crossroads) == [farthest]
expect Block.activeAt(grid64, 1.0, [], crossroads) == List.take(1.0, quad)

// The eye's own building always wins, whatever holds the k slots.
let lobbyOf = (c: Block.Copy): Vec3.t => Block.toWorld(c, Vec3.make(10.0, 1.6, 3.0))
let thresholdOf = (c: Block.Copy): Vec3.t => Block.toWorld(c, Vec3.make(10.0, 1.6, 0.0 - 0.9))
expect [grid64, tight16] |> List.all((t) => ks |> List.all((k) => t.copies |> List.indexedMap((id, c) =>
  [lobbyOf(c), thresholdOf(c)] |> List.all((eye) =>
    let others = Block.near(t, Block.activeOff, eye) |> List.filter((e) => let (j, d) = e in j != id)
      |> List.map((e) => let (j, d) = e in j) |> List.take(k) in
    Block.isIn(id, Block.activeAt(t, k, others, eye)))) |> List.all((ok) => ok)))

// Boundary oscillation, the office alone: jittering across `activeOn` turns
// it on once; across `activeOff`, off once; neither flickers.
let jitter = (a: float, b: float): List<Vec3.t> => List.range(20.0) |> List.map((i) => at(if Math.mod(i, 2.0) == 0.0 then a else b))
expect changes(walk(office, 1.0, jitter(Block.activeOn + 0.1, Block.activeOn - 0.1))) == 1.0
expect changes(walk(office, 1.0, List.append(jitter(Block.activeOff - 0.1, Block.activeOff + 0.1), [at(10.0)]))) == 1.0
// Departure and return: on, off past `activeOff`, on again inside `activeOn`.
let out = List.range(80.0) |> List.map((i) => 0.5 * i)
let away = List.append(List.reverse(out), out) |> List.map(at)
let awaySets = walk(office, 1.0, away)
expect changes(awaySets) == 2.0 && List.head(awaySets) == Option.Some([0.0]) && List.last(awaySets) == Option.Some([0.0])
expect List.range(160.0) |> List.all((i) =>
  let d = List.nth(i, List.append(List.reverse(out), out)) |> Option.defaultValue(0.0) in
  let on = List.nth(i, awaySets) == Option.Some([0.0]) in
  if i < 80.0 then on == (d <= Block.activeOff) else on == (d < Block.activeOn))

// Down the grid's streets and into a lobby, every metre: at most k on, the
// eye always in an active copy's cells, and no flicker — stepping 20 cm on
// and back changes nothing the step itself didn't.
let line = (x0: float, z0: float, x1: float, z1: float, n: float): List<Vec3.t> =>
  List.range(n + 1.0) |> List.map((i) => Vec3.make(x0 + (x1 - x0) * i / n, 1.6, z0 + (z1 - z0) * i / n))
let far = Block.centre + 3.0 * Block.blockPitch + 70.0
let route = List.flatten([
  line(Block.centre, -70.0, Block.centre, far, 480.0),
  line(-70.0, Block.centre + Block.blockPitch, far, Block.centre + Block.blockPitch, 480.0),
  line(10.0, -8.0, 10.0, 9.0, 17.0),
])
let nudge = Vec3.make(0.2, 0.0, 0.2)
expect ks |> List.all((k) =>
  let sets = walk(grid64, k, route) in
  sets |> List.indexedMap((i, s) =>
    let eye = List.nth(i, route) |> Option.defaultValue(crossroads) in
    let stepped = Block.activeAt(grid64, k, s, eye |> Vec3.add(nudge)) in
    List.length(s) <= k && Block.uncovered(grid64, s, eye) == 0.0
      && Block.activeAt(grid64, k, stepped, eye) == stepped) |> List.all((ok) => ok))
// Down a street at k = 1, each copy turns on at most once: no A, B, A.
let onsets = (sets: List<List<float>>): List<float> =>
  sets |> List.indexedMap((i, s) => s |> List.filter((id) => i == 0.0 || not Block.isIn(id, List.nth(i - 1.0, sets) |> Option.defaultValue([]))))
    |> List.flatten
expect ks |> List.all((k) =>
  let street = walk(grid64, k, line(Block.centre, -70.0, Block.centre, far, 480.0)) |> onsets in
  unique(street |> List.map((id) => $"{id}")))

// Colliders: the block's are always on; in a grid, only near the player.
expect Block.solidAt(block, [], Vec3.make(500.0, 0.0, 500.0)) == [0.0, 1.0, 2.0, 3.0]
expect Block.solidAt(grid64, [], lobbyOf(Block.copyAt(37.0))) == [37.0]
expect Block.solidAt(grid64, [], crossroads) == []
let south = (d: float): Vec3.t => Block.toWorld(Block.copyAt(5.0), Vec3.make(10.0, 0.0, 0.0 - d))
expect not Block.isIn(5.0, Block.solidAt(grid64, [], south(Block.solidOn + 1.0)))
expect Block.isIn(5.0, Block.solidAt(grid64, [5.0], south(Block.solidOn + 1.0)))
expect not Block.isIn(5.0, Block.solidAt(grid64, [5.0], south(Block.solidOn + Block.solidMargin + 1.0)))

// ---- The street side -------------------------------------------------------------

let outer = street.half
let eps = Building.eps
let roof = street.roofBox
// The roof covers the footprint at the roofline, run into the facade's outer
// half but not out through it, above every room's ceiling.
expect roof.x0 < f.x0 && roof.x1 > f.x1 && roof.z0 < f.z0 && roof.z1 > f.z1
expect roof.x0 > f.x0 - outer && roof.x1 < f.x1 + outer && roof.z0 > f.z0 - outer && roof.z1 < f.z1 + outer
expect near(roof.y1, List.length(Building.levelsOf(l)) * storey)
expect interiorIds |> List.all((i) => Building.baseOf(l, Building.roomAt(l, i)) + Building.wallTop(l, i) < roof.y0)
// It draws with the street side, and on shells: their massing stops under
// it, inside a parapet like the facade's.
expect street.skinned |> List.any((e) => let (b, _) = e in b == roof)
expect shell.skinned |> List.any((e) => let (b, _) = e in b == roof)
expect near(shell.massing.y1, roof.y0) && (shell.parapet |> List.all((b) => near(b.y0, roof.y0) && near(b.y1, facadeHeight)))
// The facade rises on all round as a parapet, unbroken, at least 0.8 m.
expect facadeHeight - roof.y1 >= 0.8
// Whether intervals `spans` cover [a, b] without a gap.
let covers = (spans: List<(float, float)>, a: float, b: float): bool =>
  let reach = spans |> List.sortBy((e) => let (lo, _) = e in lo) |> List.fold((at, e) =>
    let (lo, hi) = e in if lo <= at + eps then Math.max(at, hi) else at, a) in
  reach >= b - eps
let parapetCovers = (onSide: (Exterior.Box) => bool, along: (Exterior.Box) => (float, float), a: float, b: float): bool =>
  covers(street.facadeBoxes |> List.filter((b) => onSide(b) && b.y0 <= roof.y1 && near(b.y1, facadeHeight)) |> List.map(along), a, b)
let spanX = (b: Exterior.Box): (float, float) => (b.x0, b.x1)
let spanZ = (b: Exterior.Box): (float, float) => (b.z0, b.z1)
expect parapetCovers((b) => near(b.z1, f.z0), spanX, f.x0 - outer, f.x1 + outer)
expect parapetCovers((b) => near(b.z0, f.z1), spanX, f.x0 - outer, f.x1 + outer)
expect parapetCovers((b) => near(b.x1, f.x0) && b.z0 > f.z0 - eps, spanZ, f.z0, f.z1)
expect parapetCovers((b) => near(b.x0, f.x1) && b.z0 > f.z0 - eps, spanZ, f.z0, f.z1)
// Each window band is a hole through the facade's outer half with its panel
// `recess` back: no facade face lies just in front of a panel.
let solidIn = (a: Exterior.Box, b: Exterior.Box): bool =>
  a.x0 < b.x1 - eps && b.x0 < a.x1 - eps && a.y0 < b.y1 - eps && b.y0 < a.y1 - eps && a.z0 < b.z1 - eps && b.z0 < a.z1 - eps
// The part of band `h`'s hole from `d0` to `d1` in from the facade's outer face.
let bandSlice = (level: float, s: Side, h: Building.Hole, d0: float, d1: float): Exterior.Box =>
  let y0 = level * storey + h.bottom in
  let y1 = level * storey + h.top in
  match s with
  | South => Exterior.box(h.lo, y0, f.z0 - outer + d0, h.hi, y1, f.z0 - outer + d1)
  | North => Exterior.box(h.lo, y0, f.z1 + outer - d1, h.hi, y1, f.z1 + outer - d0)
  | West => Exterior.box(f.x0 - outer + d0, y0, h.lo, f.x0 - outer + d1, y1, h.hi)
  | East => Exterior.box(f.x1 + outer - d1, y0, h.lo, f.x1 + outer - d0, y1, h.hi)
let bandSlices = (d0: float, d1: float): List<Exterior.Box> => Building.levelsOf(l) |> List.concatMap((level) =>
  [South, North, West, East] |> List.concatMap((s) => Exterior.bandHoles(l, level, s) |> List.map((h) => bandSlice(level, s, h, d0, d1))))
let r = Exterior.recess
expect List.length(bandSlices(0.0, r)) == List.length(street.bandBoxes)
expect bandSlices(0.0, r) |> List.all((o) => not (List.any((b) => solidIn(o, b), List.append(street.bandBoxes, street.facadeBoxes))))
// Behind that, a panel fills the hole.
let contains = (b: Exterior.Box, a: Exterior.Box): bool =>
  b.x0 <= a.x0 && b.x1 >= a.x1 && b.y0 <= a.y0 && b.y1 >= a.y1 && b.z0 <= a.z0 && b.z1 >= a.z1
expect bandSlices(r, r + 0.01) |> List.all((a) => street.bandBoxes |> List.any((b) => contains(b, a)))
// Every town's fog ends past where it starts.
expect Block.towns |> List.all((t) => Math.min(t.reach.night, t.reach.day) > Hours.outdoorAt(1.0).fogNear)
expect Hours.reachAt(Hours.streetReach, 0.0) == Hours.streetReach.night && Hours.reachAt(Hours.streetReach, 1.0) == Hours.streetReach.day

// ---- Shells --------------------------------------------------------------------

// An inactive copy is a shell, an active one isn't.
let shellLook = (active: List<float>): Shells.Look => { town: 4.0, active: active, neon: 1.0, lamps: 1.0, day: false }
expect List.length(Shells.idleOf(shellLook([0.0, 37.0]))) == 62.0
expect Shells.idleOf(shellLook([0.0, 37.0])) |> List.all((id) => id != 0.0 && id != 37.0)
// The shells build, by night and day, with every copy idle or none.
expect [shellLook([]), { shellLook(Block.townAt(4.0).copies |> List.indexedMap((id, c) => id)) with day: true }] |> List.all((l) =>
  Scene.equals(Shells.shellsOf(l), Shells.shellsOf(l)))
// Every window glows where layout.fun cuts it, in the wall's outer face, and
// a panel lights the room behind it (so glowing is right); the entrance
// comes first.
let half = street.half
expect List.length(shell.openings) == List.length(windows) + 1.0
expect windows |> List.indexedMap((i, w) =>
  match List.nth(i + 1.0, shell.openings) with
  | Option.Some(b) =>
    let y = Building.baseOf(l, Building.roomAt(l, w.room)) in
    w.side == West && near(b.x1, f.x0 - half)
      && near(b.z0, w.at - w.width / 2.0) && near(b.z1, w.at + w.width / 2.0) && near(b.y0, y + w.sill) && near(b.y1, y + w.head)
  | Option.None => false) |> List.all((ok) => ok)
expect windows |> List.all((w) => Building.roomAt(l, w.room).panels |> List.any((p) => p.on))
// Each copy's parts are prebuilt, per town.
expect (Shells.parts |> List.map(List.length)) == (Block.towns |> List.map((t) => List.length(t.copies)))


// ---- The island ----------------------------------------------------------------

let island = IslandLayout.pad
let padRect = rect(island.x0, island.z0, island.x1, island.z1)
let grown = (r: Rect, d: float): Rect => rect(r.x0 - d, r.z0 - d, r.x1 + d, r.z1 + d)
// 2ⁿ + 1 samples, a metre apart, so every terrain LOD vertex lands on one.
expect IslandLayout.samples == IslandLayout.size + 1.0
expect List.range(16.0) |> List.any((k) => Math.pow(2.0, k) == IslandLayout.size)
// The pad lies on the terrain, its edges on the collider's 2 m samples.
let onSample = (along: float, centre: float): bool =>
  let i = along - centre + IslandLayout.size / 2.0 in Math.mod(i, 2.0) == 0.0 && i > 0.0 && i < IslandLayout.size
expect onSample(island.x0, IslandLayout.centre.x) && onSample(island.x1, IslandLayout.centre.x)
  && onSample(island.z0, IslandLayout.centre.z) && onSample(island.z1, IslandLayout.centre.z)
// The pad holds every copy with a 6 m apron, and the aprons and yards.
expect copies |> List.all((c) => within(padRect, grown(worldRect(c, f), 6.0)))
expect copies |> List.all((c) => yards |> List.all((r) => within(padRect, worldRect(c, r.rect))))
// Every eye within the activation radius of a footprint is within
// `band.reach` of the pad's edge: each footprint, grown by the rest of the
// radius, lies on the pad.
expect copies |> List.all((c) => within(padRect, grown(worldRect(c, f), reach - IslandLayout.band.reach)))
// There the ground is never below the pad, and stays low enough that the
// collar holds the eye at every street eye height, all the way round.
let rise = (y: float): float => Island.groundY + y - island.y
expect IslandLayout.band.yMin >= island.y && IslandLayout.band.yMax <= island.y + 3.0
expect outside |> List.all((p) =>
  let (x, z) = p in [0.3 + rise(IslandLayout.band.yMin), 2.6 + rise(IslandLayout.band.yMax)] |> List.all((y) => held(x, y, z)))
// Toggling the island off the pad sends the walker to view 9: on the pad.
expect (List.nth(8.0, Game.views) |> Option.map((w) => Island.onPad(w.x, w.z))) == Option.Some(true)
// The sea is below the pad, the seabed below the sea.
expect IslandLayout.seaLevel < island.y && IslandLayout.minHeight < IslandLayout.seaLevel
// The 16-bit samples step at most 1 cm over the range.
expect (IslandLayout.maxHeight - IslandLayout.minHeight) / 65535.0 <= 0.01

// ---- The island's props --------------------------------------------------------

let props = IslandLayout.props
let trees = IslandLayout.trees
let rocks = IslandLayout.rocks
// Distance from (x, z) to the pad's rect (0 on it).
let fromPad = (x: float, z: float): float =>
  let dx = Math.max(0.0, Math.max(island.x0 - x, x - island.x1)) in
  let dz = Math.max(0.0, Math.max(island.z0 - z, z - island.z1)) in
  Math.sqrt(dx * dx + dz * dz)
let apart = (a, b): float => Math.sqrt((a.x - b.x) * (a.x - b.x) + (a.z - b.z) * (a.z - b.z))
// Off the pad, its ramp and the band (so out of every activation radius and
// collar), with room for the widest prop.
expect props.clearing >= IslandLayout.band.reach + Island.propRadius && props.clearing >= IslandLayout.ramp
expect trees |> List.all((t) => fromPad(t.x, t.z) >= props.clearing)
expect rocks |> List.all((r) => fromPad(r.x, r.z) >= props.clearing)
// The caps: trees (so cards) and rocks in all, and the near tier: any
// `meshDistance` disc holding a tree lies within twice that of it.
expect List.length(trees) <= props.treeCap && List.length(rocks) <= props.rockCap
expect trees |> List.all((t) => (trees |> List.filter((u) => apart(t, u) <= 2.0 * props.meshDistance) |> List.length) <= props.nearCap)
// On land: above the sea, inside the heightmap's range. (island.mjs checks
// each height against its samples; the e2e against `ground`.)
expect List.flatten([trees |> List.map((t) => t.y), rocks |> List.map((r) => r.y)])
  |> List.all((y) => y > IslandLayout.seaLevel && y <= IslandLayout.maxHeight)
// No tree above the treeline: the peaks are bare.
expect (trees |> List.all((t) => t.y <= props.treeline)) && props.treeline < IslandLayout.maxHeight
// Each prop has a distinct id, its collider's tag.
expect (trees |> List.indexedMap((i, t) => t.id == i) |> List.all((same) => same)) && (rocks |> List.indexedMap((i, r) => r.id == i) |> List.all((same) => same))
// The split from view 9 by day, and from the pad's far corner by night: each
// tier within its distance and under its cap; off the island, nothing.
let eyes = [{ x: -12.0, z: -50.0, fog: 500.0 }, { x: island.x1, z: island.z1, fog: 200.0 }]
let splits = eyes |> List.map((e) => Island.regroup(Island.noProps, true, e.x, e.z, e.fog))
expect List.zip(splits, eyes) |> List.all((pair) =>
  let (e, s) = pair in
  List.length(s.near) <= props.nearCap && List.length(s.near) + List.length(s.far) <= props.treeCap
  && (s.near |> List.all((t) => Island.within(e.x, e.z, props.meshDistance, t)))
  && (s.rocks |> List.all((r) => Island.within(e.x, e.z, props.rockDistance, r)))
  && (s.solid.trees |> List.all((t) => Island.within(e.x, e.z, Island.solidReach + Island.regroupStep + Island.propRadius, t))))
expect (List.nth(0.0, splits) |> Option.map((s) => List.length(s.far) > 0)) == Option.Some(true)
expect Island.regroup(List.nth(0.0, splits) |> Option.defaultValue(Island.noProps), false, 0.0, 0.0, 500.0) == Island.noProps

// ---- The island's colormaps ----------------------------------------------------

// island.mjs bakes the night map's lamp pools at the pad's height in the
// world, the street lights game.fun lights every copy with.
expect IslandLayout.groundY == Island.groundY
let streetLights = copies |> List.concatMap(Game.streetLights)
let toMm = (a: float, b: float): bool => Math.abs(a - b) < 0.001
expect List.length(IslandLayout.lamps) == List.length(streetLights)
expect IslandLayout.lamps |> List.zip(streetLights) |> List.all((pair) =>
  let (l, s) = pair in
  toMm(l.x, Vec3.x(s.at)) && toMm(l.y, Vec3.y(s.at)) && toMm(l.z, Vec3.z(s.at)) && l.r == s.r && l.g == s.g && l.b == s.b
  && l.intensity == s.intensity && l.range == s.range)
// The day map's sun is high (over 55°), so slopes facing away from it still
// show under the flashlight; the island's frame lights at noon and midnight
// are the ones each map was baked with.
let sunUp = (d) => (0.0 - d.y) / Math.sqrt(d.x * d.x + d.y * d.y + d.z * d.z)
expect sunUp(IslandLayout.light.day.direction) > Math.sin(55.0 * Math.pi / 180.0)
let islandNoon = Hours.islandOutdoorAt(1.0)
let islandMidnight = Hours.islandOutdoorAt(0.0)
expect islandNoon.ambient == IslandLayout.light.day.ambient && islandNoon.sun == IslandLayout.light.day.sun && islandNoon.sunIntensity == IslandLayout.light.day.intensity
expect islandMidnight.ambient == IslandLayout.light.night.ambient && islandMidnight.sun == IslandLayout.light.night.sun
  && islandMidnight.sunIntensity == IslandLayout.light.night.intensity
// Every biome's detail tiles at 2–3 m and fades out by 80 m.
expect (Island.biomes |> List.all((b) => b.tile >= 2.0 && b.tile <= 3.0)) && Island.detailFade <= 80.0
// The flashlight takes a point light's slot: with or without it the frame
// holds 8 lights (the lit shader's cap) among the block's 12 point lights.
let beam = Light.spot(Vec3.make(0.0, 1.5, 0.0), Vec3.make(0.0, 0.0, 1.0), Game.flashlightColor, 1.0, 60.0, Game.flashlightCone)
let streetEye = Vec3.make(-12.0, 1.6, -50.0)
let lit = Game.frameLights(Hours.islandOutdoorAt(0.0), copies, streetEye, [beam])
expect List.length(lit) == 8.0
expect List.length(Game.frameLights(Hours.islandOutdoorAt(0.0), copies, streetEye, [])) == 8.0

// ---- The towers ------------------------------------------------------------------

let tower = Archetypes.tower8
let tl = tower.layout
let roomOf = (lay: Layout, i: float): Room => Building.roomAt(lay, i)
let stairsOf = (lay: Layout): List<float> => List.range(List.length(lay.rooms)) |> List.filter((i) => roomOf(lay, i).kind == Stair)
// The stairwell is one core rect on every floor, ground to roof, a level
// apart, in the tower and both bake layouts; a flight joins each pair.
expect [tl, TowerLayout.typical, TowerLayout.ends] |> List.all((lay) => stairsOf(lay) |> List.all((i) => roomOf(lay, i).rect == TowerLayout.stairRect))
expect (stairsOf(tl) |> List.map((i) => roomOf(tl, i).level)) == List.range(10.0)
expect (tl.flights |> List.map((fl) => [fl.lower, fl.upper]))
  == (List.zip(List.drop(1.0, stairsOf(tl)), List.take(9.0, stairsOf(tl))) |> List.map((p) => let (a, b) = p in [a, b]))
// Floor k's room j is cell 8k + j: the typical floor's rooms, then the
// roof's, each named and levelled as in its bake.
expect List.range(8.0) |> List.all((k0) => List.range(TowerLayout.cellsPerFloor) |> List.all((j) =>
  roomOf(tl, Tower.cellOf(k0 + 1.0, j)).name == roomOf(TowerLayout.typical, Tower.typicalFirst + j).name
    && roomOf(tl, Tower.cellOf(k0 + 1.0, j)).level == k0 + 1.0))
expect List.range(2.0) |> List.all((j) => roomOf(tl, Tower.cellOf(9.0, j)).name == roomOf(TowerLayout.ends, Tower.endsRoofFirst + j).name)
expect List.range(TowerLayout.cellsPerFloor) |> List.all((j) => roomOf(tl, j).name == roomOf(TowerLayout.ends, j).name)
expect [roomOf(tl, Tower.apron).name, roomOf(tl, Tower.westYard).name, roomOf(TowerLayout.typical, Tower.typicalApron).name, roomOf(TowerLayout.typical, Tower.typicalYard).name]
  == ["apron", "west yard", "apron", "west yard"]
// 74 rooms and 2 more collar strips, 210 portals; 88 cell models (the
// e2e's), 10 of them stairwells; one model per inside cell.
expect List.length(Portals.cellsOf(tl, Tower.collarTop)) == 76.0 && List.length(Portals.portalsOf(tl, Tower.collarTop)) == 210.0
let towerPlaced = Tower.placedOf(8.0, Archetypes.typicalBaked, Archetypes.endsBaked)
expect List.length(tower.night) == 88.0 && List.length(towerPlaced |> List.filter((p) => roomOf(tl, p.cell).kind == Stair)) == 10.0
expect towerPlaced |> List.all((p) => roomOf(tl, p.cell).kind == Outside || List.length(towerPlaced |> List.filter((q) => q.cell == p.cell)) == 1.0)
// Every bake layout's cell's faces lie in its cell (a stairwell room
// between two flights gets both shares), and a placed model is lifted from
// its bake's level to its cell's.
let inCell = (lay: Layout, i: float, p: Vec3.t): bool =>
  let r = roomOf(lay, i) in
  let e = 0.0001 in
  not Building.isInside(r) || (Vec3.x(p) >= r.rect.x0 - e && Vec3.x(p) <= r.rect.x1 + e && Vec3.z(p) >= r.rect.z0 - e && Vec3.z(p) <= r.rect.z1 + e
    && Vec3.y(p) >= Building.baseOf(lay, r) - e && Vec3.y(p) <= Building.baseOf(lay, r) + Building.wallTop(lay, i) + e)
expect [TowerLayout.typical, TowerLayout.ends] |> List.all((lay) => List.range(List.length(lay.rooms)) |> List.all((i) =>
  Building.cellFaces(lay, i) |> List.all((fc) => Building.corners(fc.q) |> List.all((p) => inCell(lay, i, p)))))
let bakeLevel = (p: Tower.Placed): float =>
  if p.cell >= Tower.cellOf(9.0, 0.0) then TowerLayout.endsRoof else if p.cell < TowerLayout.cellsPerFloor then 0.0 else 1.0
expect towerPlaced |> List.all((p) => roomOf(tl, p.cell).kind == Outside
  || near(Building.baseOf(tl, roomOf(tl, p.cell)), bakeLevel(p) * storey + p.dy))
// No flight reaches below its own floor: the typical floor's model holds
// its whole flight, with no flight below in its bake to lend the rest.
expect [tl, TowerLayout.typical, TowerLayout.ends] |> List.all((lay) => lay.flights |> List.all((fl) =>
  Building.flightFaces(lay, fl) |> List.all((fc) => Building.lowestY(fc) >= Building.baseOf(lay, roomOf(lay, fl.lower)) - eps)))
// The dog-leg: comfortable steps and a ramp the character climbs, 1.2 m
// clear in each lane, headroom under the flight above, landings to step
// onto, its half-landing halfway up.
let leg = (fl: Flight): Building.Leg =>
  match fl.kind with
  | DogLeg(well, landing) => Building.legOf(tl, fl, well, landing)
  | Straight => Binary.invalid("a straight flight in the tower")
expect tl.flights |> List.all((fl) =>
  let g = leg(fl) in
  Building.riserOf(tl, fl) <= 0.19 && fl.tread >= 0.28 && Math.atan(Building.riserOf(tl, fl) / fl.tread) * 180.0 / Math.pi <= 35.0
    && g.ax1 - Building.parapetThickness - g.r.x0 >= 1.2 - eps && g.r.x1 - g.bx0 - Building.parapetThickness >= 1.2 - eps
    && tl.storey - Building.soffitDepth(tl, fl) >= 2.4 && g.zL - g.r.z0 >= 1.5 && g.r.z1 - g.zH >= 1.5 && near(g.half, tl.storey / 2.0))
// Each lane's ramp runs from a tread before its first riser through every
// nosing; the half-landing between them.
let rampAt = (q: Building.Quad, z: float): float => Vec3.y(q.o) + Vec3.y(q.b) * (z - Vec3.z(q.o)) / Vec3.z(q.b)
expect tl.flights |> List.all((fl) =>
  let g = leg(fl) in
  let rise = Building.riserOf(tl, fl) in
  let y = Building.baseOf(tl, roomOf(tl, fl.lower)) in
  match Building.rampFaces(tl, fl) with
  | [up, half, back] =>
    (List.range(g.n) |> List.all((j) =>
      near(rampAt(up.q, g.zL + j * fl.tread), y + (j + 1.0) * rise) && near(rampAt(back.q, g.zH - j * fl.tread), y + g.half + (j + 1.0) * rise)))
      && near(rampAt(up.q, g.zL - fl.tread), y) && near(Vec3.y(half.q.o), y + g.half)
  | _ => false)
// Sight: down the well from floor 6's landing to the ground, and up it to
// the roof's head room; not from that landing into a corridor below, nor
// up through a floor.
let wellAt = (y: float): Vec3.t => Vec3.make(9.2, y, 12.0)
expect Cells.segment(Vec3.make(9.2, 6.0 * storey + 1.6, 11.0), wellAt(1.0), tower.level)
expect Cells.segment(wellAt(1.0), wellAt(9.0 * storey + 1.0), tower.level)
expect not Cells.segment(Vec3.make(9.2, 6.0 * storey + 1.6, 10.6), Vec3.make(12.0, 5.0 * storey + 1.6, 7.6), tower.level)
expect not Cells.segment(Vec3.make(12.0, 3.0 * storey + 1.6, 7.6), Vec3.make(12.0, 4.0 * storey + 1.6, 7.6), tower.level)
// The street sees in through each floor's west window, and the terrace
// out over its parapet.
expect List.range(8.0) |> List.all((k) =>
  Cells.segment(Vec3.make(-5.0, (k + 1.0) * storey + 1.6, 12.0), Vec3.make(2.5, (k + 1.0) * storey + 1.6, 12.0), tower.level))
expect Cells.segment(Vec3.make(12.0, 9.0 * storey + 1.6, 4.0), Vec3.make(12.0, 25.0, -20.0), tower.level)
// Collider tags are unique in every archetype and in the towers' town.
expect Archetypes.all |> List.all((a) => unique(a.colliders |> List.map((c) => c.tag)))
expect unique(List.flatten([["player", "block-ground"], tagsOf(Block.towers.copies)]))
// Every tower is at most the tallest, and its collar holds every eye up to
// that one's roof terrace, out to `activeOff` + `eyeReach`; the terrace is
// a cell.
expect [Archetypes.tower8, Archetypes.tower12, Archetypes.tower16] |> List.all((a) => a.layout.levels - 1.0 <= Tower.maxFloors)
expect Block.townAt(Block.towersTown).name == "towers"
let roofEye = (Tower.maxFloors + 1.0) * storey + 0.92 + 0.73 + Block.eyeReach
expect roofEye < Tower.collarTop
let tf = tl.footprint
let towerOutline = List.range(Math.floor(reach) + 1.0) |> List.map((k) => Math.min(reach, k + 0.001)) |> List.concatMap((d) => outlineOf(tf, d))
expect [Archetypes.tower8, Archetypes.tower12, Archetypes.tower16] |> List.all((a) => towerOutline |> List.all((p) =>
  let (x, z) = p in [0.3, 1.6, 20.0, roofEye] |> List.all((y) => Cells.locate(Vec3.make(x, y, z), a.level) != Option.None)))
expect Cells.locate(Vec3.make(12.0, 9.0 * storey + 1.6, 4.0), tower.level) == Option.Some(Tower.cellOf(9.0, 1.0))
// The towers stand a street apart.
expect (Block.towers.copies |> List.map((c) => Block.archetypeOf(c).name)) == ["tower-8", "tower-16", "tower-12", "tower-8"]
expect pairsOf(Block.towers.copies) |> List.all((p) => let (a, b) = p in gap(worldRect(a, tf), worldRect(b, tf)) >= Block.streetWidth - 0.001)

// The facade kit's trim (and every archetype's own boxes) stands clear of
// the shells' glowing windows, entrance and bands, so no face lies in front
// of one's.
let clearOfOpenings = (a: Archetypes.Archetype): bool =>
  let sh = Shells.shellAt(a.name) in
  let glow = List.flatten([sh.openings, sh.bands]) in
  List.flatten([a.street.extraBoxes |> List.map((e) => let (b, _) = e in b), a.street.extraSkinned |> List.map((e) => let (b, _) = e in b)])
    |> List.all((b) => not (glow |> List.any((o) => solidIn(b, o))))
expect Archetypes.all |> List.all(clearOfOpenings)
// A street side draws its lit boxes once and once per finish it uses (its
// shells, once per finish across the town): two for the office, three for a
// tower, the cinema and the mall, and a house's clapboard (its gable's
// shingles draw with the gables).
let finishesOf = (a: Archetypes.Archetype): float =>
  List.length(Finishes.all |> List.filter((f) => a.street.skinned |> List.any((e) => let (_, s) = e in s.finish.name == f.name)))
expect (Archetypes.all |> List.map(finishesOf)) == [2.0, 3.0, 3.0, 3.0, 3.0, 3.0, 1.0, 1.0]
