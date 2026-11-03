// The house and the suburb, run by `functor -d examples/liminal-office test`.
// (Not hashed into the bakes' provenance.)

open Layout

let l = HouseLayout.layout
let house = Archetypes.houseOchre
let kit = Archetypes.ochreInterior
let level = house.level
let f = l.footprint
let n = List.length(l.rooms)
let ids = List.range(n)
let insideIds = ids |> List.filter((i) => Building.isInside(Building.roomAt(l, i)))
let near = Building.near
let roomAt = (i: float): Room => Building.roomAt(l, i)

// ---- The layout ----------------------------------------------------------------

let overlap = Tests.overlap
// Rooms on one level don't overlap; inside rooms are within the footprint,
// with room for the capsule.
expect ids |> List.all((i) => ids |> List.all((j) =>
  i >= j || roomAt(i).level != roomAt(j).level || overlap(roomAt(i).rect, roomAt(j).rect) < 0.0001))
expect insideIds |> List.all((i) =>
  let r = roomAt(i).rect in r.x0 >= f.x0 && r.x1 <= f.x1 && r.z0 >= f.z0 && r.z1 <= f.z1)
expect insideIds |> List.all((i) => let r = Building.inner(l, roomAt(i).rect) in Math.min(r.x1 - r.x0, r.z1 - r.z0) >= 1.4)
// Doors: one level, a shared wall, a jamb each side, walkable.
expect l.doors |> List.all((d) =>
  let a = roomAt(d.a).rect in
  let b = roomAt(d.b).rect in
  let h = Building.holeOf(d) in
  let ns = near(a.z1, b.z0) || near(a.z0, b.z1) in
  let (lo, hi) = if ns then (Math.max(a.x0, b.x0), Math.min(a.x1, b.x1)) else (Math.max(a.z0, b.z0), Math.min(a.z1, b.z1)) in
  roomAt(d.a).level == roomAt(d.b).level && Building.sideBetween(a, b) != Option.None
    && h.lo >= lo + l.partition / 2.0 - 0.0001 && h.hi <= hi - l.partition / 2.0 + 0.0001
    && d.width >= 0.9 && d.height >= 2.0 && d.height <= l.ceiling)
// Holes in one wall never overlap.
expect insideIds |> List.all((i) => [North, South, East, West] |> List.all((s) =>
  let holes = Building.holesIn(l, i, s) in
  List.zip(holes, List.drop(1.0, holes)) |> List.all((p) => let (next, prev) = p in prev.hi < next.lo)))
// Four windows downstairs and two up, each in an outside wall, clear of
// its corners, with a sill.
expect (l.windows |> List.map((w) => roomAt(w.room).level)) == [0.0, 0.0, 0.0, 0.0, 1.0, 1.0]
expect l.windows |> List.all((w) =>
  let c = Building.inner(l, roomAt(w.room).rect) in
  let (lo, hi) = if w.side == North || w.side == South then (c.x0, c.x1) else (c.z0, c.z1) in
  Building.onFootprint(l, w.side, roomAt(w.room).rect)
    && w.at - w.width / 2.0 > lo + 0.3 && w.at + w.width / 2.0 < hi - 0.3 && w.sill >= 0.8 && w.head < l.ceiling - 0.2)
// Upstairs: the landing, a corridor and three bedrooms, one with a window.
expect (ids |> List.filter((i) => roomAt(i).level == 1.0) |> List.map((i) => roomAt(i).name))
  == ["landing", "corridor", "bedroom 1", "bedroom 2", "bedroom 3"]
expect (l.windows |> List.filter((w) => Text.contains("bedroom", roomAt(w.room).name)) |> List.map((w) => w.room)) == [10.0]

// ---- The stair -------------------------------------------------------------------

let flight = List.head(l.flights) |> Option.defaultValue({ kind: Straight, lower: 0.0, upper: 0.0, x0: 0.0, z0: 0.0, z1: 0.0, risers: 0.0, tread: 0.0 })
let hall = Building.inner(l, roomAt(flight.lower).rect)
// 15 risers along the hall's south wall, from a foot clear of the front
// door to a landing; comfortable and climbable (maxSlope 40°).
expect flight.risers == 15.0 && roomAt(flight.lower).rect == roomAt(flight.upper).rect && roomAt(flight.upper).level == 1.0
expect near(flight.z0, hall.z0) && flight.z1 - Building.parapetThickness - flight.z0 >= 1.0
expect flight.x0 - Building.holeOf(List.head(l.doors) |> Option.defaultValue({ a: 0.0, b: 0.0, at: 0.0, width: 0.0, height: 0.0 })).hi >= 1.5
expect hall.x1 - Building.flightEnd(flight) - Building.parapetThickness >= 1.2
expect Building.riserOf(l, flight) <= 0.19 && Math.atan(Building.riserOf(l, flight) / flight.tread) * 180.0 / Math.pi <= 35.0
expect l.storey - Building.slabDepth >= 2.4
expect near(Building.rampY(l, flight, flight.x0 - flight.tread), 0.0) && near(Building.rampY(l, flight, Building.flightEnd(flight)), l.storey)
// The landing's door to the corridor opens off its walkway.
expect l.doors |> List.filter((d) => d.a == flight.upper || d.b == flight.upper) |> List.all((d) =>
  Building.holeOf(d).lo >= Building.flightEnd(flight) + Building.parapetThickness && Building.holeOf(d).hi <= hall.x1)
// The treads are the hall's carpet (magenta in the magenta palette).
expect Building.treadMatOf(l, flight) == Building.carpet

// ---- Cells and portals -------------------------------------------------------------

// 10 rooms and 4 outside rooms (the collar's strips: porch and yards, under
// their room ids); 12 doors, 6 windows, the stair opening and the collar's
// corners, both ways.
expect List.length(Portals.cellsOf(l, Portals.outsideTop)) == 14.0 && List.length(Portals.portalsOf(l, Portals.outsideTop)) == 46.0
expect (Portals.sides |> List.map(Portals.stripId(l))) == [0.0, 13.0, 11.0, 12.0]
// Every face lies in its cell.
let inBox = (i: float, p: Vec3.t): bool =>
  let r = roomAt(i) in
  let (y0, y1) = if Building.isInside(r) then (Building.baseOf(l, r), Building.baseOf(l, r) + Building.wallTop(l, i)) else (0.0, Portals.outsideTop) in
  let e = 0.0001 in
  Vec3.x(p) >= r.rect.x0 - e && Vec3.x(p) <= r.rect.x1 + e && Vec3.z(p) >= r.rect.z0 - e && Vec3.z(p) <= r.rect.z1 + e
    && Vec3.y(p) >= y0 - e && Vec3.y(p) <= y1 + e
expect ids |> List.all((i) => Building.cellFaces(l, i) |> List.all((face) => Building.corners(face.q) |> List.all((p) => inBox(i, p))))
// Each room's middle, at eye height, is in its own cell.
let middle = (i: float): Vec3.t =>
  let r = roomAt(i).rect in Vec3.make((r.x0 + r.x1) / 2.0, Building.baseOf(l, roomAt(i)) + 1.6, (r.z0 + r.z1) / 2.0)
expect ids |> List.all((i) => Cells.locate(middle(i), level) == Option.Some(i))
// Every room is reachable through the portals from the porch.
let linked = (i: float): List<float> => Portals.portalsOf(l, Portals.outsideTop) |> List.filter((p) => p.from == i) |> List.map((p) => p.to)
let reached = ids |> List.fold((seen, k) =>
  seen |> List.concatMap(linked) |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else acc |> List.append([j]), seen), [0.0])
expect List.length(reached) == n
// Sight: the porch through the front door into the hall; up the stair
// opening; not through the hall's ceiling.
let up = l.storey + 1.6
expect Cells.segment(Vec3.make(1.4, 1.6, -2.0), Vec3.make(2.0, 1.6, 2.4), level)
expect Cells.segment(Vec3.make(2.0, 1.6, 1.0), Vec3.make(6.0, up, 1.0), level)
expect not Cells.segment(Vec3.make(6.0, 1.6, 2.4), Vec3.make(6.0, up, 3.6), level)
// Every window portals from the yard outside it: sight runs in through it,
// not through the wall beside it.
let through = (w: Window, along: float, depth: float): Vec3.t =>
  let r = roomAt(w.room) in
  let line = Building.lineOf(w.side, r.rect) in
  let y = Building.baseOf(l, r) + (w.sill + w.head) / 2.0 in
  let out = if w.side == North || w.side == East then depth else 0.0 - depth in
  if w.side == North || w.side == South then Vec3.make(w.at + along, y, line + out) else Vec3.make(line + out, y, w.at + along)
expect l.windows |> List.all((w) => Building.outsideOf(l, w) != Option.None)
expect l.windows |> List.all((w) => Cells.segment(through(w, 0.0, 5.0), through(w, 0.0, -1.0), level))
expect l.windows |> List.all((w) => not Cells.segment(through(w, w.width, 5.0), through(w, w.width, -1.0), level))

// The collar holds every eye within reach of the footprint, and the rooms
// every eye inside it on both floors (upstairs, the bedrooms reach z 9.6;
// past them is solid).
let reach = Block.activeOff + Block.eyeReach
let held = (x: float, y: float, z: float): bool => Cells.locate(Vec3.make(x, y, z), level) != Option.None
let ring = (d: float): List<(float, float)> => List.range(72.0) |> List.map((k) =>
  let a = k * 5.0 * Math.pi / 180.0 in
  (Math.clamp(f.x0, f.x1, 5.0 + 9.0 * Math.sin(a)) + d * Math.sin(a), Math.clamp(f.z0, f.z1, 6.0 + 9.0 * Math.cos(a)) + d * Math.cos(a)))
expect List.range(Math.floor(reach) + 1.0) |> List.all((k) => ring(Math.min(reach, k + 0.001)) |> List.all((p) =>
  let (x, z) = p in [0.3, 1.6, 2.6] |> List.all((y) => held(x, y, z))))
expect List.range(10.0) |> List.all((x) => List.range(12.0) |> List.all((z) =>
  ([1.6, 2.4] |> List.all((y) => held(x + 0.5, y, z + 0.5)))
    && (z + 0.5 > 9.6 || ([1.6, 2.2] |> List.all((y) => held(x + 0.5, l.storey + y, z + 0.5))))))

// ---- Mesh, palettes, bakes ---------------------------------------------------------------

// The cut into cell meshes loses nothing, unbaked and baked.
let total = 2.0 * List.length(kit.allFaces)
let cutTotal = (data: Model.staticData): float =>
  Building.runsOf(kit, data) |> List.map((run) => Bytes.length(Building.cellData(kit, data, run).triangles) / 12.0) |> List.sum
expect cutTotal(kit.interiorData) == total
expect [(Archetypes.ochreStatus, Archetypes.ochreBake), (Archetypes.magentaStatus, Archetypes.magentaBake)] |> List.all((e) =>
  let (status, bake) = e in status != Provenance.Current || cutTotal(Lightmap.readMesh(File.bytes(Provenance.meshPathOf(bake)))) == total)
// One layout, two palettes: the same faces; bakes named per palette.
let magentaKit = Archetypes.magentaInterior
expect List.length(magentaKit.allFaces) == List.length(kit.allFaces) && magentaKit.meshSpans == kit.meshSpans
expect kit.night.name == "house-ochre-night" && magentaKit.day.name == "house-magenta-day"
// The niche and the stair's parapet are their own materials.
expect [Building.niche, Building.castConcrete] |> List.all((m) => kit.meshSpans |> List.any((s) => s.mat == m))
// Collider tags are unique.
let tagsOf = (cs: List<Building.Collider>): List<string> => cs |> List.map((k) => k.tag)
let unique = Tests.unique
expect unique(List.flatten([tagsOf(house.colliders), tagsOf(house.street.lampColliders)]))

// ---- The street side ---------------------------------------------------------------------

let street = house.street
let gable = HouseExterior.gableBoxOf(l)
// The gable's lower half sinks into the facade above the bedrooms'
// ceilings and the roof slab's underside, and overhangs the side facades;
// its ends stand inside the front and back facades, which hide its lower
// half.
expect gable.y0 >= l.storey + l.ceiling && gable.y0 >= street.roofBox.y0 - 0.0001 && near((gable.y0 + gable.y1) / 2.0, l.facadeHeight)
expect gable.x0 < f.x0 - street.half && gable.x1 > f.x1 + street.half
  && gable.z0 > f.z0 - street.half && gable.z0 < f.z0 && gable.z1 < f.z1 + street.half && gable.z1 > f.z1
// The porch roof clears the front door and the upstairs sills.
let porchRoof = street.canopyBox
expect porchRoof.y0 > 2.1 && porchRoof.y1 < l.storey + 0.9
// No neon on a house; its porch light glows.
expect street.neonBoxes == [] && List.length(street.shadeBoxes) == List.length(l.streetLamps) + 1.0
// The front path meets the gate in the fence.
let path = HouseExterior.pathOf(l)
expect HouseExterior.fenceRuns(l) |> List.all((run) => let (x0, x1) = run in x1 <= path.x0 + 0.0001 || x0 >= path.x1 - 0.0001)

// ---- The suburb --------------------------------------------------------------------------

let suburb = Block.suburb
let copies = suburb.copies
// Twelve houses at quarter turns only (never mirrored), both palettes,
// unique names (memo keys) and collider tags.
expect List.length(copies) == 12.0 && Block.townAt(Block.suburbTown).name == "suburb" && suburb.shells
expect copies |> List.all((c) => Math.mod(c.yaw, 90.0) == 0.0 && c.yaw >= 0.0 && c.yaw < 360.0)
expect [0.0, 90.0, 180.0, 270.0] |> List.all((y) => copies |> List.any((c) => c.yaw == y))
expect copies |> List.all((c) => Block.archetypeOf(c).name == c.archetype && Text.contains("house-", c.archetype))
expect ["house-ochre", "house-magenta"] |> List.all((a) => List.length(copies |> List.filter((c) => c.archetype == a)) == 6.0)
expect unique(copies |> List.map((c) => c.name))
expect unique(List.flatten([["player", "suburb-ground"], copies |> List.concatMap((c) => Block.collidersOf(c) |> List.map((k) => Block.tagOf(c, k.tag)))]))
// Every porch faces the green: a house's front middle is nearer the
// middle than its back.
expect copies |> List.all((c) =>
  Vec3.length(Block.toWorld(c, Vec3.make(5.0, 0.0, 0.0))) < Vec3.length(Block.toWorld(c, Vec3.make(5.0, 0.0, 12.0))))
// Footprints and fences never overlap another house's lot.
let lot = (c: Block.Copy): Rect => Tests.worldRect(c, rect(f.x0 - HouseExterior.side, HouseExterior.fenceZ(l), f.x1 + HouseExterior.side, f.z1 + 2.0))
expect copies |> List.indexedMap((i, a) => List.drop(i + 1.0, copies) |> List.all((b) => overlap(lot(a), lot(b)) < 0.0001)) |> List.all((ok) => ok)
// At the edge of every house's activation, its cells hold the eye.
expect copies |> List.indexedMap((id, c) => ring(Block.activeOff) |> List.all((p) =>
  let (x, z) = p in
  let eye = Block.toWorld(c, Vec3.make(x, 1.6, z)) in
  Block.uncovered(suburb, Block.activeAt(suburb, Block.maxActive, [id], eye), eye) == 0.0)) |> List.all((ok) => ok)
// Every front path runs into the loop road: a square band `inner` to `outer`
// out from the middle.
let onRoad = (p: Vec3.t): bool =>
  let d = Math.max(Math.abs(Vec3.x(p)), Math.abs(Vec3.z(p))) in d >= Suburb.inner && d <= Suburb.outer
expect copies |> List.all((c) =>
  let g = HouseExterior.gateOf(l) in
  [g.lo + 0.01, g.hi - 0.01] |> List.all((x) => onRoad(Block.toWorld(c, Vec3.make(x, 0.0, path.z0 - 0.01)))))
// Round the loop road (its middle, every metre) and into a house: at most
// 4 on, the eye always in an active copy's cells.
let mid = (Suburb.inner + Suburb.outer) / 2.0
let loop = List.range(4.0) |> List.concatMap((k) =>
  let u = List.nth(k, Suburb.outward) |> Option.defaultValue((0.0, 1.0)) in
  let (ux, uz) = u in
  let (wx, wz) = Suburb.along(u) in
  List.range(36.0) |> List.map((j) => Vec3.make(mid * ux + (j - 18.0) * wx, 1.6, mid * uz + (j - 18.0) * wz)))
let walked = List.append(loop, [Block.toWorld(Block.copyIn(suburb, 1.0), Vec3.make(1.6, 1.6, -4.0)), Block.toWorld(Block.copyIn(suburb, 1.0), Vec3.make(2.0, 1.6, 1.4))])
let sets =
  let (_, out) = walked |> List.fold((acc, eye) =>
    let (was, seen) = acc in
    let now = Block.activeAt(suburb, Block.maxActive, was, eye) in (now, List.append(seen, [(eye, now)])), ([], [])) in
  out
expect sets |> List.all((e) => let (eye, s) = e in List.length(s) <= Block.maxActive && Block.uncovered(suburb, s, eye) == 0.0)
expect (List.last(sets) |> Option.map((e) => let (eye, s) = e in Block.isIn(1.0, s))) == Option.Some(true)

// Idle houses draw as shells with their gables; the shells build (a
// derivation error would fail this).
let look = (active: List<float>): Shells.Look => { town: Block.suburbTown, active: active, neon: 1.0, lamps: 1.0, day: false }
expect List.length(Shells.idleOf(look([1.0]))) == 11.0
let suburbParts = List.nth(Block.suburbTown, Shells.parts) |> Option.defaultValue([])
expect List.length(suburbParts) == 12.0 && (suburbParts |> List.all((p) => List.length(p.gables) == 1.0))
expect Scene.equals(Shells.shellsOf(look([])), Shells.shellsOf(look([])))
// Each of the suburb's views stands in the room it names: the front walk
// (the porch's strip), the hall, living room, kitchen, landing, corridor,
// bedroom 3, the hall; the road; the hall.
expect (Suburb.views |> List.map((e) =>
  let (copy, x, lv, z, yaw, pitch) = e in
  Cells.locate(Vec3.make(x, lv * l.storey + 1.65, z), level) |> Option.defaultValue(-1.0)))
  == [0.0, 1.0, 2.0, 4.0, 6.0, 7.0, 10.0, 1.0, 0.0, 1.0]
