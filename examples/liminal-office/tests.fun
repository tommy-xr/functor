// Layout, cell and derivation checks, run by `functor -d examples/liminal-office test`.
// (Not hashed into the bake's provenance, so editing them needs no rebake.)

open Layout

let n = List.length(rooms)
let ids = List.range(n)
let interiorIds = ids |> List.filter((i) => Building.isInside(Building.roomAt(i)))

let overlap = (a: Rect, b: Rect): float =>
  Math.max(0.0, Math.min(a.x1, b.x1) - Math.max(a.x0, b.x0))
    * Math.max(0.0, Math.min(a.z1, b.z1) - Math.max(a.z0, b.z0))
let within = (outer: Rect, r: Rect): bool =>
  r.x0 >= outer.x0 && r.x1 <= outer.x1 && r.z0 >= outer.z0 && r.z1 <= outer.z1
let pairs = ids |> List.concatMap((i) => ids |> List.filter((j) => j > i) |> List.map((j) => (i, j)))

// No two rooms overlap; every inside room is within the footprint.
expect pairs |> List.all((p) =>
  let (i, j) = p in overlap(Building.roomAt(i).rect, Building.roomAt(j).rect) < 0.0001)
expect interiorIds |> List.all((i) => within(footprint, Building.roomAt(i).rect))

// The stretch of wall two rooms share, along it.
let shared = (d: Door): Building.Hole =>
  let a = Building.roomAt(d.a).rect in
  let b = Building.roomAt(d.b).rect in
  let side = Building.sideBetween(a, b) in
  if side == Option.Some(North) || side == Option.Some(South)
  then { lo: Math.max(a.x0, b.x0), hi: Math.min(a.x1, b.x1), bottom: 0.0, top: 0.0 }
  else { lo: Math.max(a.z0, b.z0), hi: Math.min(a.z1, b.z1), bottom: 0.0, top: 0.0 }

// Doors sit on a shared wall, inside its span with a jamb's worth of wall
// on each side, and are big enough to walk through.
expect doors |> List.all((d) => Building.sideBetween(Building.roomAt(d.a).rect, Building.roomAt(d.b).rect) != Option.None)
expect doors |> List.all((d) =>
  let s = shared(d) in
  let h = Building.holeOf(d) in
  h.lo >= s.lo + partition / 2.0 - 0.0001 && h.hi <= s.hi - partition / 2.0 + 0.0001)
expect doors |> List.all((d) => d.width >= 0.9 && d.height >= 2.0 && d.height <= ceiling)
// Holes in one wall never overlap.
expect interiorIds |> List.all((i) => [North, South, East, West] |> List.all((s) =>
  let holes = Building.holesIn(i, s) in
  List.zip(holes, List.drop(1.0, holes)) |> List.all((p) => let (next, prev) = p in prev.hi < next.lo)))

// Every room is reachable from the apron.
let neighbours = (i: float): List<float> =>
  doors |> List.concatMap((d) => if d.a == i then [d.b] else if d.b == i then [d.a] else [])
let reached = ids |> List.fold((seen, k) =>
  seen |> List.concatMap(neighbours) |> List.filter((j) => not List.any((s) => s == j, seen))
    |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else acc |> List.append([j]), seen), [0.0])
expect List.length(reached) == n

// Clearances: rooms and the corridor leave room for the 0.6 m capsule.
expect interiorIds |> List.all((i) =>
  let r = Building.inner(Building.roomAt(i).rect) in Math.min(r.x1 - r.x0, r.z1 - r.z0) >= 2.0)
// Panels sit inside their room, clear of the walls.
expect rooms |> List.all((r) =>
  let c = Building.inner(r.rect) in
  r.panels |> List.all((p) => p.x - 0.6 > c.x0 && p.x + 0.6 < c.x1 && p.z - 0.6 > c.z0 && p.z + 0.6 < c.z1))

// Cells: each room's centre is in its own cell; the spawn is on the apron.
let centre = (r: Rect): Vec3.t => Vec3.make((r.x0 + r.x1) / 2.0, 1.6, (r.z0 + r.z1) / 2.0)
expect ids |> List.all((i) => Cells.locate(centre(Building.roomAt(i).rect), Building.level) == Option.Some(i))
expect Cells.locate(Vec3.make(spawn.x, 1.6, spawn.z), Building.level) == Option.Some(0.0)
// Sight runs through doorways and stops at walls.
expect Cells.segment(Vec3.make(10.0, 1.6, -6.0), Vec3.make(10.0, 1.6, 20.0), Building.level)
expect Cells.segment(Vec3.make(10.0, 1.6, 16.5), Vec3.make(15.6, 1.6, 16.5), Building.level)
expect not Cells.segment(Vec3.make(10.0, 1.6, 10.0), Vec3.make(15.6, 1.6, 16.5), Building.level)

// Every face's material has a group, and the committed bake's mesh has
// exactly the derived triangles, so per-part ranges index it correctly.
let total = 2.0 * List.length(Building.allFaces)
expect (Building.meshSpans |> List.map((s) => s.count) |> List.sum) == total
expect (Building.partRanges |> List.map((r) => r.count) |> List.sum) == total
expect Provenance.status != Provenance.Current
  || Bytes.length(Lightmap.readMesh(File.bytes(Provenance.meshPath)).triangles) == 12.0 * total

// A door's reveal spans its wall's full thickness: its threshold's depth.
let depth = (d: Door): float =>
  match List.last(Building.jambFaces(d)) with
  | Option.Some(f) => Vec3.length(Vec3.cross(f.q.b, f.q.a)) / d.width
  | Option.None => 0.0
// Doors from outside go through the facade; the rest through partitions.
expect doors |> List.all((d) =>
  let wall = if Building.isInside(Building.roomAt(d.a)) then partition else facade in
  Math.abs(depth(d) - wall) < 0.0001)

// Collider tags are unique.
let tags = Building.blocks |> List.map((b) => b.tag)
expect tags |> List.indexedMap((i, t) => List.drop(i + 1.0, tags) |> List.any((u) => u == t)) |> List.all((dup) => not dup)

// Windows sit in an outside wall of an inside room, clear of its corners,
// with a sill and a head under the ceiling; the wall keeps its colliders.
expect windows |> List.all((w) =>
  let r = Building.roomAt(w.room) in
  let c = Building.inner(r.rect) in
  let (lo, hi) = if w.side == North || w.side == South then (c.x0, c.x1) else (c.z0, c.z1) in
  Building.isInside(r) && Building.onFootprint(w.side, r.rect)
    && w.at - w.width / 2.0 > lo + 0.3 && w.at + w.width / 2.0 < hi - 0.3
    && w.sill >= 0.8 && w.head < ceiling - 0.2)
expect windows |> List.all((w) =>
  Building.blocks |> List.any((b) => Math.abs(b.y + b.h / 2.0 - w.sill) < 0.001 && b.y - b.h / 2.0 == 0.0))

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
