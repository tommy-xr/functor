// Layout, cell and derivation checks, run by `functor -d examples/liminal-office test`.
// (Not hashed into the bake's provenance, so editing them needs no rebake.)

open Layout

let n = List.length(rooms)
let ids = List.range(n)
let interiorIds = ids |> List.filter((i) => Building.isInside(Building.roomAt(i)))
let roomOf = Building.roomAt

let overlap = (a: Rect, b: Rect): float =>
  Math.max(0.0, Math.min(a.x1, b.x1) - Math.max(a.x0, b.x0))
    * Math.max(0.0, Math.min(a.z1, b.z1) - Math.max(a.z0, b.z0))
let within = (outer: Rect, r: Rect): bool =>
  r.x0 >= outer.x0 && r.x1 <= outer.x1 && r.z0 >= outer.z0 && r.z1 <= outer.z1
let pairs = ids |> List.concatMap((i) => ids |> List.filter((j) => j > i) |> List.map((j) => (i, j)))
let near = (a: float, b: float): bool => Math.abs(a - b) < 0.001

// Rooms on one level don't overlap; every inside room is within the
// footprint, on level 0 or 1.
expect pairs |> List.all((p) =>
  let (i, j) = p in roomOf(i).level != roomOf(j).level || overlap(roomOf(i).rect, roomOf(j).rect) < 0.0001)
expect interiorIds |> List.all((i) => within(footprint, roomOf(i).rect))
expect rooms |> List.all((r) => r.level == 0.0 || r.level == 1.0)
// Both floors have rooms; upstairs is all inside.
expect interiorIds |> List.any((i) => roomOf(i).level == 1.0)
expect rooms |> List.all((r) => r.level == 0.0 || Building.isInside(r))

// The stretch of wall two rooms share, along it.
let shared = (d: Door): Building.Hole =>
  let a = roomOf(d.a).rect in
  let b = roomOf(d.b).rect in
  let side = Building.sideBetween(a, b) in
  if side == Option.Some(North) || side == Option.Some(South)
  then { lo: Math.max(a.x0, b.x0), hi: Math.min(a.x1, b.x1), bottom: 0.0, top: 0.0 }
  else { lo: Math.max(a.z0, b.z0), hi: Math.min(a.z1, b.z1), bottom: 0.0, top: 0.0 }

// Doors join two rooms on one level across a shared wall, inside its span
// with a jamb's worth of wall on each side, and are big enough to walk through.
expect doors |> List.all((d) => roomOf(d.a).level == roomOf(d.b).level)
expect doors |> List.all((d) => Building.sideBetween(roomOf(d.a).rect, roomOf(d.b).rect) != Option.None)
expect doors |> List.all((d) =>
  let s = shared(d) in
  let h = Building.holeOf(d) in
  h.lo >= s.lo + partition / 2.0 - 0.0001 && h.hi <= s.hi - partition / 2.0 + 0.0001)
expect doors |> List.all((d) => d.width >= 0.9 && d.height >= 2.0 && d.height <= ceiling)
// Holes in one wall never overlap.
expect interiorIds |> List.all((i) => [North, South, East, West] |> List.all((s) =>
  let holes = Building.holesIn(i, s) in
  List.zip(holes, List.drop(1.0, holes)) |> List.all((p) => let (next, prev) = p in prev.hi < next.lo)))

// Every room is reachable from the apron, through doors and up the stairs.
let neighbours = (i: float): List<float> =>
  List.flatten([
    doors |> List.concatMap((d) => if d.a == i then [d.b] else if d.b == i then [d.a] else []),
    flights |> List.concatMap((f) => if f.lower == i then [f.upper] else if f.upper == i then [f.lower] else []),
  ])
let reached = ids |> List.fold((seen, k) =>
  seen |> List.concatMap(neighbours) |> List.filter((j) => not List.any((s) => s == j, seen))
    |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else acc |> List.append([j]), seen), [0.0])
expect List.length(reached) == n

// Clearances: rooms and the corridor leave room for the 0.6 m capsule.
expect interiorIds |> List.all((i) =>
  let r = Building.inner(roomOf(i).rect) in Math.min(r.x1 - r.x0, r.z1 - r.z0) >= 2.0)
// Panels sit inside their room, clear of the walls.
expect rooms |> List.all((r) =>
  let c = Building.inner(r.rect) in
  r.panels |> List.all((p) => p.x - 0.6 > c.x0 && p.x + 0.6 < c.x1 && p.z - 0.6 > c.z0 && p.z + 0.6 < c.z1))

// ---- Stairs ----------------------------------------------------------------

// A flight's rooms are one stairwell: the same rect, a level apart.
expect flights |> List.all((f) =>
  let lo = roomOf(f.lower) in
  let up = roomOf(f.upper) in
  lo.kind == Stair && up.kind == Stair && lo.rect == up.rect && up.level == lo.level + 1.0)
// The flight runs along the south wall, with room to step on at its foot
// and a landing at its head, and lies inside the stairwell.
let landingWidth = (f: Flight): float =>
  Building.inner(roomOf(f.lower).rect).x1 - Building.flightEnd(f) - Building.parapetThickness
expect flights |> List.all((f) =>
  let r = Building.inner(roomOf(f.lower).rect) in
  near(f.z0, r.z0) && f.z1 < r.z1 && f.x0 - r.x0 >= 1.0 && landingWidth(f) >= 1.2)
// Clear width between the wall and the parapet; headroom under the landing.
expect flights |> List.all((f) => f.z1 - Building.parapetThickness - f.z0 >= 1.2)
expect storey - Building.slabDepth >= 2.4
// Comfortable steps, and a ramp the character climbs (maxSlope 40°).
let slope = (f: Flight): float => Math.atan(Building.riserOf(f) / f.tread) * 180.0 / Math.pi
expect flights |> List.all((f) => Building.riserOf(f) <= 0.19 && f.tread >= 0.28 && slope(f) <= 35.0)
// The ramp starts on the floor, passes through every step's nosing, and
// meets the landing at its edge.
expect flights |> List.all((f) =>
  near(Building.rampY(f, f.x0 - f.tread), 0.0)
    && (List.range(f.risers - 1.0) |> List.all((j) =>
      near(Building.rampY(f, f.x0 + j * f.tread), (j + 1.0) * Building.riserOf(f))))
    && near(Building.rampY(f, Building.flightEnd(f)), storey))
// The upper room's door opens onto the landing's walkway.
expect flights |> List.all((f) =>
  let r = Building.inner(roomOf(f.upper).rect) in
  doors |> List.filter((d) => d.a == f.upper || d.b == f.upper) |> List.all((d) =>
    let h = Building.holeOf(d) in
    h.lo >= Building.flightEnd(f) + Building.parapetThickness && h.hi <= r.x1))
// Each stair face lies in exactly one of the two rooms' cells.
expect flights |> List.all((f) =>
  List.length(Building.stairFaces(f.lower)) + List.length(Building.stairFaces(f.upper))
    == List.length(Building.flightFaces(f)))

// ---- Cells and portals -------------------------------------------------------

// Each room's middle, at eye height, is in its own cell; the spawn is on the apron.
let middle = (i: float): Vec3.t =>
  let r = roomOf(i).rect in
  Vec3.make((r.x0 + r.x1) / 2.0, Building.baseOf(roomOf(i)) + 1.6, (r.z0 + r.z1) / 2.0)
expect ids |> List.all((i) => Cells.locate(middle(i), Building.level) == Option.Some(i))
expect Cells.locate(Vec3.make(spawn.x, 1.6, spawn.z), Building.level) == Option.Some(0.0)
// Sight runs through doorways and stops at walls.
expect Cells.segment(Vec3.make(10.0, 1.6, -6.0), Vec3.make(10.0, 1.6, 20.0), Building.level)
expect Cells.segment(Vec3.make(10.0, 1.6, 16.5), Vec3.make(15.6, 1.6, 16.5), Building.level)
expect not Cells.segment(Vec3.make(10.0, 1.6, 10.0), Vec3.make(15.6, 1.6, 16.5), Building.level)
// The stair opening is a portal between floors: sight runs up through it,
// but not through the landing, nor through any other floor.
let up = storey + 1.6
expect Cells.segment(Vec3.make(12.0, 1.6, 23.0), Vec3.make(15.0, up, 25.0), Building.level)
expect not Cells.segment(Vec3.make(19.0, 1.6, 24.0), Vec3.make(19.0, up, 24.0), Building.level)
expect not Cells.segment(Vec3.make(10.0, 1.6, 10.0), Vec3.make(10.0, up, 10.0), Building.level)
// Upstairs, through doorways as below.
expect Cells.segment(Vec3.make(10.0, up, 3.0), Vec3.make(15.6, up, 3.0), Building.level)

// ---- The mesh and its cells ----------------------------------------------------

// Every face's material has a group, and the committed bake's mesh has
// exactly the derived triangles.
let total = 2.0 * List.length(Building.allFaces)
expect (Building.meshSpans |> List.map((s) => s.count) |> List.sum) == total
expect (Building.cellRanges |> List.map((r) => r.count) |> List.sum) == total
expect Provenance.status != Provenance.Current
  || Bytes.length(Lightmap.readMesh(File.bytes(Provenance.meshPath)).triangles) == 12.0 * total

// The split into cell meshes loses and repeats nothing: the cells' triangle
// ranges and vertex runs tile the mesh in order, each cell's triangles index
// only its own vertices, and putting the cells back together (indices plus
// each run's start; vertex arrays end to end) gives the mesh byte for byte,
// UV2 included.
let u32s = (b: Bytes.t): List<float> =>
  List.range(Bytes.length(b) / 4.0) |> List.map((k) => Bytes.u32le(4.0 * k, b) |> Binary.require)
let splitsExactly = (data: Model.staticData): bool =>
  let runs = Building.runsOf(data) in
  let cells = runs |> List.map((run) => (run, Building.cellData(data, run))) in
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
expect splitsExactly(Building.interiorData)
expect Provenance.status != Provenance.Current
  || splitsExactly(Lightmap.readMesh(File.bytes(Provenance.meshPath)))
// The baked mesh has lightmap UVs to keep.
expect Provenance.status != Provenance.Current
  || Bytes.length(Lightmap.readMesh(File.bytes(Provenance.meshPath)).uvs2) > 0.0

// A door's reveal spans its wall's full thickness: its two halves'
// thresholds together. Doors from outside go through the facade; the rest
// through partitions.
let depth = (d: Door, i: float): float =>
  match List.last(Building.doorHalf(d, i)) with
  | Option.Some(f) => Vec3.length(Vec3.cross(f.q.b, f.q.a)) / d.width
  | Option.None => 0.0
expect doors |> List.all((d) =>
  let wall = if Building.isInside(roomOf(d.a)) then partition else facade in
  Math.abs(depth(d, d.a) + depth(d, d.b) - wall) < 0.0001 && Math.abs(depth(d, d.a) - depth(d, d.b)) < 0.0001)

// Collider tags are unique.
let tags = Building.blocks |> List.map((b) => b.tag)
expect tags |> List.indexedMap((i, t) => List.drop(i + 1.0, tags) |> List.any((u) => u == t)) |> List.all((dup) => not dup)

// Windows sit in an outside wall of an inside room, clear of its corners,
// with a sill and a head under the ceiling.
expect windows |> List.all((w) =>
  let r = roomOf(w.room) in
  let c = Building.inner(r.rect) in
  let (lo, hi) = if w.side == North || w.side == South then (c.x0, c.x1) else (c.z0, c.z1) in
  Building.isInside(r) && Building.onFootprint(w.side, r.rect)
    && w.at - w.width / 2.0 > lo + 0.3 && w.at + w.width / 2.0 < hi - 0.3
    && w.sill >= 0.8 && w.head < ceiling - 0.2)
// Each window has a sill collider: its floor to its sill, across the
// window, in its wall.
expect windows |> List.all((w) =>
  let l = Building.lineOf(w.side, roomOf(w.room).rect) in
  let y = Building.baseOf(roomOf(w.room)) in
  Building.blocks |> List.any((b) =>
    near(b.y - b.h / 2.0, y) && near(b.y + b.h / 2.0, y + w.sill)
      && (if w.side == North || w.side == South then near(b.z, l) && near(b.x, w.at) && near(b.w, w.width)
          else near(b.x, l) && near(b.z, w.at) && near(b.d, w.width))))

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
// (so night frames don't change), day with the bake's sun.
let night = Hours.outdoorAt(0.0)
let noon = Hours.outdoorAt(1.0)
expect night.sky == Hours.nightSky && night.fogNear == 35.0 && night.sunIntensity == 0.6
expect night.neon == 1.0 && night.lamps == 1.0
expect Vec3.length(night.sunDirection |> Vec3.sub(Hours.moonDirection)) == 0.0
expect noon.sky == Hours.daySky && noon.fogNear == 70.0 && noon.sunIntensity == 2.6
expect noon.neon == 0.7 && noon.lamps == 0.0
expect Vec3.length(noon.sunDirection |> Vec3.sub(Building.sunDirection)) < 0.000001
// Halfway, the sky is dusk.
expect Hours.outdoorAt(0.5).sky == Hours.duskSky
