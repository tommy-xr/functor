// The cinema's layout, cell, rake and bake checks, run by `functor -d
// examples/liminal-office test`. (Not hashed into the bake's provenance.)

open Layout

let a = Archetypes.cinema
let l = a.layout
let k = CinemaBuilding.interior
let level = a.level
let n = List.length(l.rooms)
let ids = List.range(n)
let room = (i: float): Room => Building.roomAt(l, i)
let insideIds = ids |> List.filter((i) => Building.isInside(room(i)))
let pairs = ids |> List.concatMap((i) => ids |> List.filter((j) => j > i) |> List.map((j) => (i, j)))

// ---- The layout --------------------------------------------------------------------

// Rooms don't overlap; inside rooms fill part of the footprint; one height
// per room, the auditoria the tallest, within their cells.
expect pairs |> List.all((p) => let (i, j) = p in Tests.overlap(room(i).rect, room(j).rect) == 0.0)
expect insideIds |> List.all((i) => Tests.within(l.footprint, room(i).rect))
expect List.length(CinemaLayout.heights) == n
expect insideIds |> List.all((i) => CinemaBuilding.heightOf(i) > 2.5 && CinemaBuilding.heightOf(i) <= l.ceiling)
// The lobby is 30 × 18; the numbered corridor 3.5 wide and about 40 long
// round its bend; the auditoria hold a 12-row rake.
let wide = (i: float): float => room(i).rect.x1 - room(i).rect.x0
let deep = (i: float): float => room(i).rect.z1 - room(i).rect.z0
expect wide(CinemaLayout.lobby) == 30.0 && deep(CinemaLayout.lobby) == 18.0
expect wide(CinemaLayout.corridorA) == 3.5 && deep(CinemaLayout.corridorB) == 3.5
let corridorLength = deep(CinemaLayout.corridorA) + wide(CinemaLayout.corridorB) - 3.5 / 2.0
expect corridorLength > 38.0 && corridorLength < 44.0
// Doors join rooms across a shared wall, inside its span with a jamb's
// worth each side (a throughway fills it), big enough to walk through and
// under their rooms' heights.
let span = (d: Door): (float, float) =>
  let ra = room(d.a).rect in
  let rb = room(d.b).rect in
  let side = Building.sideBetween(ra, rb) in
  if side == Option.Some(North) || side == Option.Some(South) then (Math.max(ra.x0, rb.x0), Math.min(ra.x1, rb.x1))
  else (Math.max(ra.z0, rb.z0), Math.min(ra.z1, rb.z1))
expect l.doors |> List.all((d) => Building.sideBetween(room(d.a).rect, room(d.b).rect) != Option.None)
expect l.doors |> List.all((d) => let (lo, hi) = span(d) in d.at - d.width / 2.0 >= lo + 0.1 - 0.0001 && d.at + d.width / 2.0 <= hi - 0.1 + 0.0001)
expect l.doors |> List.all((d) => d.width >= 0.9 && d.height >= 2.0
  && ([d.a, d.b] |> List.all((i) => not Building.isInside(room(i)) || d.height <= CinemaBuilding.heightOf(i))))
// The entrance is the street side's: the first door, from the apron.
expect Exterior.entranceDoorOf(l) == List.head(l.doors)
// Every room is reachable from the apron through doors.
let reached = ids |> List.fold((seen, step) =>
  l.doors |> List.fold((acc, d) =>
    let has = (i: float) => List.any((s) => s == i, acc) in
    if has(d.a) && not has(d.b) then List.append([d.b], acc)
    else if has(d.b) && not has(d.a) then List.append([d.a], acc) else acc, seen), [CinemaLayout.apron])
expect List.length(reached) == n

// ---- Cells -------------------------------------------------------------------------

// Every face (baked or unlit) lies inside its cell's box, so what a cell
// draws is only ever seen from it or through its portals.
let inBox = (i: float, p: Vec3.t): bool =>
  let r = room(i).rect in
  let top = if Building.isInside(room(i)) then l.ceiling else Cinema.collarTop in
  let e = 0.0001 in
  Vec3.x(p) >= r.x0 - e && Vec3.x(p) <= r.x1 + e && Vec3.z(p) >= r.z0 - e && Vec3.z(p) <= r.z1 + e
    && Vec3.y(p) >= 0.0 - e && Vec3.y(p) <= top + e
expect ids |> List.all((i) => CinemaBuilding.cellFaces(i) |> List.all((f) => Building.corners(f.q) |> List.all((p) => inBox(i, p))))
// Each room's middle, at eye height, is in its own cell; so are the eyes at
// the top of the rake and the spawn, on the lot (the south strip, the
// apron's).
let middle = (i: float): Vec3.t => let r = room(i).rect in Vec3.make((r.x0 + r.x1) / 2.0, 1.6, (r.z0 + r.z1) / 2.0)
expect ids |> List.all((i) => Cells.locate(middle(i), level) == Option.Some(i))
let top = CinemaBuilding.rakeTop + 1.65
expect CinemaLayout.auditoria |> List.all((i) => Cells.locate(Vec3.make(Vec3.x(middle(i)), top, 39.0), level) == Option.Some(i))
expect Cells.locate(Vec3.make(l.spawn.x, 1.6, l.spawn.z), level) == Option.Some(CinemaLayout.apron)
// Sight runs in through the entrance, the arch, the bend and an auditorium
// door; not through the lobby's back wall into the corridor behind it.
expect Cells.segment(Vec3.make(30.0, 1.6, -20.0), Vec3.make(30.0, 1.6, 12.0), level)
expect Cells.segment(Vec3.make(40.0, 1.6, 13.0), Vec3.make(47.5, 1.6, 13.0), level)
expect Cells.segment(Vec3.make(46.75, 1.6, 9.0), Vec3.make(46.75, 1.6, 21.0), level)
expect Cells.segment(Vec3.make(41.6, 1.6, 19.0), Vec3.make(41.6, 1.6, 30.0), level)
expect not Cells.segment(Vec3.make(30.0, 1.6, 15.0), Vec3.make(30.0, 1.6, 20.0), level)
// Every cell is reachable through portals from the south strip.
let portals = Portals.portalsOf(l, Cinema.collarTop)
let cellCount = n + List.length(Portals.extraSides(l))
let reachedCells = List.range(cellCount) |> List.fold((seen, step) =>
  seen |> List.concatMap((i) => portals |> List.filter((p) => p.from == i) |> List.map((p) => p.to))
    |> List.fold((acc, j) => if List.any((s) => s == j, acc) then acc else List.append([j], acc), seen), [CinemaLayout.apron])
expect List.length(reachedCells) == cellCount
// The collar holds every eye within an active copy's reach, as the
// office's does.
let reach = Block.activeOff + Block.eyeReach
let held = (x: float, y: float, z: float): bool => Cells.locate(Vec3.make(x, y, z), level) != Option.None
expect Tests.outlineOf(l.footprint, reach) |> List.all((p) => let (x, z) = p in [0.3, 1.6, 2.6] |> List.all((y) => held(x, y, z)))

// ---- The rake and the seats ----------------------------------------------------------

// The rake's ramp starts on the floor a row before the first riser, passes
// through every row's nosing, and is a slope the character climbs (maxSlope
// 40°).
let slope = Math.atan(CinemaLayout.rise / CinemaLayout.rowDepth) * 180.0 / Math.pi
expect slope < 35.0
let ramp = Cinema.rampFaces(CinemaLayout.east)
let r3 = CinemaBuilding.innerOf(CinemaLayout.east)
let rampAt = (z: float): float =>
  let z0 = CinemaBuilding.rakeStart(r3) - CinemaLayout.rowDepth in
  Math.clamp(0.0, CinemaBuilding.rakeTop, (z - z0) * CinemaLayout.rise / CinemaLayout.rowDepth)
expect List.range(CinemaLayout.rows) |> List.all((j) => Math.abs(rampAt(CinemaBuilding.rowZ(r3, j)) - CinemaBuilding.treadY(j)) < 0.0001)
expect ramp |> List.all((f) => Vec3.y(Building.normalOf(f.q)) > 0.5)
expect Building.lowestY(List.head(ramp) |> Option.defaultValue(Building.floorFace(0.0, 0.0, 0.0, 1.0, 1.0, 9.0))) == 0.0
// 18 seats a row, 12 rows; the aisles each side leave the capsule room.
expect List.length(Cinema.seatsAt(CinemaLayout.east)) == 216.0 && List.length(Cinema.seatsAt(CinemaLayout.west)) == 216.0
expect CinemaLayout.aisle >= 0.6 + 0.4
// The screen is clear of the door beside it.
let door3 = l.doors |> List.find((d) => d.b == CinemaLayout.east) |> Option.map((d) => d.at - d.width / 2.0) |> Option.defaultValue(0.0)
expect CinemaBuilding.screenCentre(r3) + CinemaLayout.screenWidth / 2.0 < door3 - 0.2

// ---- The mesh and its bake -------------------------------------------------------------

// The bake holds only lit faces; the unlit ones (the black, the glow
// atlas) draw from their own models, one per cell that has any.
expect k.allFaces |> List.all((f) => not Paints.isUnlit(CinemaBuilding.palette, f))
let unlitFaces = CinemaBuilding.cellParts |> List.flatten |> List.filter((f) => Paints.isUnlit(CinemaBuilding.palette, f))
expect List.length(k.allFaces) + List.length(unlitFaces) == List.length(List.flatten(CinemaBuilding.cellParts))
// The lit surface fits a 1024² lightmap at the bake's 12 texels a metre.
let area = (f: Building.Face): float => Vec3.length(Vec3.cross(f.q.a, f.q.b))
expect (k.allFaces |> List.map(area) |> List.sum) < 4000.0
// The committed mesh has exactly the derived triangles, and its cut
// tiles it: each cell's run follows the last.
let total = 2.0 * List.length(k.allFaces)
expect (k.meshSpans |> List.map((s) => s.count) |> List.sum) == total
// (Read only when current: before the first bake there is none.)
let committed = (): Model.staticData => Lightmap.readMesh(File.bytes(Provenance.meshPathOf(Archetypes.cinemaBake)))
expect a.status != Provenance.Current || Bytes.length(committed().triangles) == 12.0 * total
let tiles = (data: Model.staticData): bool =>
  let (ok, first, v) = Building.runsOf(k, data) |> List.fold((acc, run) =>
    let (ok, first, v) = acc in (ok && run.first == first && run.v0 == v && run.v1 > run.v0, first + run.count, run.v1), (true, 0.0, 0.0)) in
  ok && first == total && v == Bytes.length(data.positions) / 12.0
expect tiles(k.interiorData)
expect a.status != Provenance.Current || tiles(committed())

// ---- Draws ------------------------------------------------------------------------------

// A cell draws a group per look in it, lit (its baked model's) or unlit.
// The busiest cell stays at 7, so a view through the lobby, both corridor
// cells and an auditorium stays under 30.
let groupsOf = (i: float): float => Cinema.litGroups(i) + Cinema.unlitGroups(i)
expect insideIds |> List.all((i) => groupsOf(i) <= 7.0)
expect List.length(CinemaBuilding.unlit) == List.length(ids |> List.filter((i) => Cinema.unlitGroups(i) > 0.0))
expect groupsOf(CinemaLayout.lobby) + groupsOf(CinemaLayout.corridorA) + groupsOf(CinemaLayout.corridorB) + groupsOf(CinemaLayout.east) <= 30.0

// ---- Colliders ----------------------------------------------------------------------------

// Tags are unique; the lot's lamp posts are solid when the player is beside
// them.
expect Tests.unique(a.colliders |> List.map((c) => c.tag))
let town = Block.townAt(Block.cinemaTown)
expect l.streetLamps |> List.all((p) => Block.solidAt(town, [], p) == [0.0])
