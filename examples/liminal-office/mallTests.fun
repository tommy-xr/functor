// The mall's checks (`functor test`): its cells and portals, faces in
// their cells, the galleries, the escalators, sight across the court, the
// collar, and the bake's size.

open Layout
open MallLayout

let mall = Archetypes.mall
let near = Building.near
let eps = Building.eps
let cellCount = List.length(cells)
let cellAt = MallInterior.cellAt

// 18 cells, two per open shop (8), and two more collar strips; a portal
// each way on every open-plan face, door and collar corner.
expect cellCount == 34.0 && List.length(bays) == 8.0
expect List.length(Mall.cellList) == 36.0
expect List.length(Mall.portalList) == 2.0 * (List.length(links) + List.length(doors) + 4.0)
expect (cells |> List.filter((c) => c.kind == Void) |> List.length) == 1.0
// The void is one box from the court's floor to the skylight's apex,
// over both levels.
expect Mall.spanOf(courtVoid) == (0.0, apex) && cellAt(courtVoid).rect == voidRect
// Every portal is a flat convex polygon: each corner turns the same way.
let turnsOf = (pts: List<Vec3.t>): List<Vec3.t> =>
  let n = List.length(pts) in
  let at = (k: float): Vec3.t => List.nth(Math.mod(k, n), pts) |> Option.defaultValue(Vec3.make(0.0, 0.0, 0.0)) in
  List.range(n) |> List.map((k) => at(k + 1.0) |> Vec3.sub(at(k)) |> Vec3.cross(at(k + 2.0) |> Vec3.sub(at(k + 1.0))))
expect Mall.portalList |> List.all((p) =>
  let turns = turnsOf(p.polygon) in
  let first = List.head(turns) |> Option.defaultValue(Vec3.make(0.0, 0.0, 0.0)) in
  List.length(p.polygon) >= 3.0 && Vec3.length(first) > eps && (turns |> List.all((t) => Vec3.dot(t, first) > 0.0 && Vec3.length(Vec3.cross(t, first)) < eps)))

// Every face lies in its cell's box: balustrades in their galleries, the
// escalators and fountain in the void, every bay's fit-out in its shop.
let inBox = (i: float, p: Vec3.t): bool =>
  let r = cellAt(i).rect in
  let (y0, y1) = Mall.spanOf(i) in
  Vec3.x(p) >= r.x0 - eps && Vec3.x(p) <= r.x1 + eps && Vec3.z(p) >= r.z0 - eps && Vec3.z(p) <= r.z1 + eps
    && Vec3.y(p) >= y0 - eps && Vec3.y(p) <= y1 + eps
let facesOf = (i: float): List<MallInterior.Poly> =>
  if MallInterior.isEcho(i) then MallInterior.cellFaces(i) else List.nth(i, MallInterior.interior.cellParts) |> Option.defaultValue([])
expect List.range(cellCount) |> List.all((i) => facesOf(i) |> List.all((p) => p.pts |> List.all((q) => inBox(i, q))))
// Faces are flat, and each polygon's normal is its triangles'.
expect Mall.faces |> List.all((p) =>
  let n = MallInterior.normalOf(p) in
  p.pts |> List.all((q) => near(Vec3.dot(n, q |> Vec3.sub(MallInterior.pt(0.0, p))), 0.0)))
// One model per cell with faces of its own; the apron's and the yard's are
// their halves of the doors to them. Every cell draws one.
let echoes = List.range(cellCount) |> List.filter(MallInterior.isEcho)
expect List.length(Archetypes.mallBaked.models) == cellCount - List.length(echoes)
expect List.length(Mall.placedOf(Archetypes.mallBaked)) == cellCount
expect List.length(mall.night) == cellCount + 1.0 && List.length(mall.day) == cellCount + 1.0

// ---- Shops ------------------------------------------------------------------------

// Shops with a fit-out share one geometry: each bay's shop and stockroom
// are its home's faces moved by its offset, so only the three homes are in
// the mesh and the bake (lightmap memory stays flat in the bay count).
let movedBy = (d: Vec3.t, a: MallInterior.Poly, b: MallInterior.Poly): bool =>
  a.mat == b.mat && List.length(a.pts) == List.length(b.pts)
    && (a.pts |> List.zip(b.pts) |> List.all((e) => let (q, p) = e in Vec3.length(p |> Vec3.add(d) |> Vec3.sub(q)) < eps))
let reuses = (i: float, d: Vec3.t): bool =>
  let home = MallInterior.cellFaces(MallInterior.homeCellOf(i)) in
  let own = MallInterior.cellFaces(i) in
  List.length(home) == List.length(own) && (own |> List.zip(home) |> List.all((e) => let (h, o) = e in movedBy(d, h, o)))
expect List.range(List.length(bays)) |> List.all((k) => reuses(shopOf(k), MallInterior.offsetOf(k)) && reuses(stockOf(k), MallInterior.offsetOf(k)))
expect (List.range(List.length(bays)) |> List.filter((k) => MallInterior.homeOf(k) == k)) == [0.0, 1.0, 2.0]
expect List.length(echoes) == 10.0 && (echoes |> List.all((i) => List.nth(i, MallInterior.interior.cellParts) == Option.Some([])))
// Each fit-out furnishes its shop: racks, screens, cartons.
expect [0.0, 1.0, 2.0] |> List.all((k) => List.length(MallInterior.itemsOf(shopOf(k))) >= 5.0)
expect List.length(MallInterior.cellFaces(shopOf(1.0)) |> List.filter((p) => p.mat == MallInterior.screen)) >= 20.0
expect List.length(MallInterior.cellFaces(shopOf(2.0)) |> List.filter((p) => p.mat == MallInterior.closing)) == 2.0
// Open bays on both levels, the rest shuttered; every shop has a stockroom
// behind it and the stockroom a door into its level's corridor, far from
// the shop's door to it.
expect List.range(2.0) |> List.all((level) => (bays |> List.filter((b) => b.level == level) |> List.length) == 4.0)
expect List.length(MallInterior.fronts |> List.filter((f) => not f.open)) >= 20.0
let doorBetween = (a: float, b: float): Option.t<Door> => doors |> List.find((d) => (d.a == a && d.b == b) || (d.a == b && d.b == a))
let doorAt = (a: float, b: float): float => doorBetween(a, b) |> Option.map((d) => d.at) |> Option.defaultValue(0.0)
expect List.range(List.length(bays)) |> List.all((k) =>
  Math.abs(doorAt(stockOf(k), serviceOf(MallInterior.bayAt(k).level)) - doorAt(shopOf(k), stockOf(k))) > 5.0)
// The corridor and stockrooms are low; the exit is the corridor's east end.
expect MallInterior.ceilingOf(service0) == lowCeiling && MallInterior.ceilingOf(stockOf(0.0)) == lowCeiling
expect doorBetween(yard, service0) != Option.None
// The facade opens at the exit: no box of the street side's in its doorway
// (the facade's hole is 1 cm inside the reveal).
expect Mall.street.facadeBoxes |> List.all((b) =>
  b.y0 >= 2.08 || b.z1 <= exitAt - 0.48 || b.z0 >= exitAt + 0.48 || b.x1 <= footprint.x1 - eps)

// Sight: from the corridor through a stockroom into its shop, but never on
// out to the concourse; the concourse never sees the corridor.
let eyeAt = (x: float, level: float, z: float): Vec3.t => Vec3.make(x, level * storey + 1.6, z)
let serviceEyes = List.range(2.0) |> List.concatMap((level) => List.range(19.0) |> List.concatMap((k) =>
  [46.0, 46.8, 47.6] |> List.map((z) => eyeAt(2.0 + 4.0 * k, level, z))))
let concourseEyes = List.range(2.0) |> List.concatMap((level) => List.range(13.0) |> List.concatMap((k) =>
  [11.0, 20.0, 31.0] |> List.map((z) => eyeAt(2.0 + 6.0 * k, level, z))))
expect serviceEyes |> List.all((a) => concourseEyes |> List.all((b) => not Cells.segment(a, b, mall.level)))
expect List.range(List.length(bays)) |> List.all((k) =>
  let b = MallInterior.bayAt(k) in
  Cells.segment(eyeAt(b.at + staffOffset, b.level, 47.0), eyeAt(b.at + staffOffset - 0.5, b.level, stockZ0 + 0.5), mall.level)
    && Cells.segment(eyeAt(b.at - staffOffset, b.level, stockZ0 + 1.0), eyeAt(b.at - staffOffset + 0.5, b.level, stockZ0 - 2.0), mall.level)
    && Cells.segment(eyeAt(b.at, b.level, shopZ0 - 2.0), eyeAt(b.at, b.level, shopZ0 + 5.0), mall.level))
// The walk: shop, stockroom, corridor, out of the exit into the yard.
expect [(13.0, 36.0, shopOf(0.0)), (12.0, 43.8, stockOf(0.0)), (40.0, 46.8, service0), (80.0, 46.8, yard)] |> List.all((e) =>
  let (x, z, c) = e in Cells.locate(eyeAt(x, 0.0, z), mall.level) == Option.Some(c))

// The galleries: a balustrade 1.1 m high along every edge of the void,
// open only where the escalators land; the lower concourse has none.
let galleries = List.range(cellCount) |> List.filter((i) => Option.isSome(MallInterior.voidSide(i)))
expect galleries == [galleryS, galleryN, bridgeW, bridgeE]
expect List.range(cellCount) |> List.all((i) => (galleries |> List.any((g) => g == i)) == (MallInterior.balustradeBoxes(i) != []))
let railLength = (i: float): float => MallInterior.balustradeBoxes(i) |> List.map((b) =>
  if Option.Some(North) == MallInterior.voidSide(i) || Option.Some(South) == MallInterior.voidSide(i) then b.x1 - b.x0 else b.z1 - b.z0) |> List.sum
expect near(railLength(galleryS), voidRect.x1 - voidRect.x0 + 2.0 * MallInterior.balustrade)
expect near(railLength(bridgeW), voidRect.z1 - voidRect.z0 - (escalator.z1 - escalator.z0))
expect galleries |> List.all((i) => MallInterior.balustradeBoxes(i) |> List.all((b) => near(b.y1 - b.y0, 1.1)))

// The escalators: 30°, the ramp through every nosing from the court's
// floor to the top plate, the head a storey up on the west bridge.
let steps = Mall.steps
expect near(Math.atan(steps.rise / steps.run) * 180.0 / Math.pi, 30.0) && steps.rise < 0.2
let rampAt = (q: MallInterior.Poly, x: float): float =>
  let o = MallInterior.pt(0.0, q) in
  let far = MallInterior.pt(3.0, q) in
  Vec3.y(o) + (Vec3.y(far) - Vec3.y(o)) * (x - Vec3.x(o)) / (Vec3.x(far) - Vec3.x(o))
expect steps.lanes |> List.all((lane) =>
  let q = Mall.rampOf(lane) in
  (List.range(escalator.risers) |> List.all((j) => near(rampAt(q, steps.xp + j * steps.run), storey - j * steps.rise)))
    && near(rampAt(q, steps.xq), 0.0) && near(rampAt(q, steps.xp), storey))
expect Cells.locate(Vec3.make(escalator.xTop - 0.5, storey + 1.6, (escalator.z0 + escalator.z1) / 2.0), mall.level) == Option.Some(bridgeW)
expect Cells.locate(Vec3.make(steps.xEnd + 1.0, 1.6, 16.4), mall.level) == Option.Some(courtVoid)
// The eye stays in the void all the way up a lane.
expect List.range(20.0) |> List.all((k) =>
  let x = steps.xq - k * (steps.xq - steps.xp) / 19.0 in
  Cells.locate(Vec3.make(x, MallInterior.nosingY(steps, x) + 1.65, 16.4), mall.level) == Option.Some(courtVoid))

// Sight: down into the court from each gallery, across it between the
// galleries and the lower concourse, into the entrance from the lot; not
// through a gallery's floor, nor into a shuttered shop band.
let eye = (x: float, level: float, z: float): Vec3.t => Vec3.make(x, level * storey + 1.6, z)
expect Cells.segment(eye(41.0, 1.0, 13.0), Vec3.make(41.0, 0.3, 21.0), mall.level)
expect Cells.segment(eye(41.0, 1.0, 29.0), eye(41.0, 0.0, 12.0), mall.level)
expect Cells.segment(eye(41.0, 1.0, 13.0), eye(41.0, 1.0, 29.0), mall.level)
expect Cells.segment(eye(20.0, 0.0, 20.0), eye(70.0, 0.0, 20.0), mall.level)
expect Cells.segment(eye(41.0, 0.0, -20.0), eye(41.0, 0.0, 12.0), mall.level)
expect Cells.segment(eye(41.0, 1.0, 13.0), Vec3.make(41.0, apex - 0.5, 21.0), mall.level)
expect not Cells.segment(eye(28.0, 1.0, 12.0), eye(28.0, 0.0, 12.0), mall.level)
expect Cells.locate(eye(10.0, 0.0, 5.0), mall.level) == Option.None

// The collar holds every eye near the mall (tests.fun's outline), and the
// footprint's cells every eye inside: each level's concourse, the shops
// and the entrance.
let reach = Tests.reach
let outside = List.range(Math.floor(reach) + 1.0) |> List.map((k) => Math.min(reach, k + 0.001)) |> List.concatMap((d) => Tests.outlineOf(footprint, d))
expect outside |> List.all((p) => let (x, z) = p in [0.3, 1.6, 2.6] |> List.all((y) => Cells.locate(Vec3.make(x, y, z), mall.level) != Option.None))
expect List.range(cellCount) |> List.filter((i) => cellAt(i).kind != Lot) |> List.all((i) =>
  let r = cellAt(i).rect in
  Cells.locate(Vec3.make((r.x0 + r.x1) / 2.0, MallInterior.baseOf(i) + 1.65, (r.z0 + r.z1) / 2.0), mall.level) == Option.Some(i))

// The street side: the roof is open over the void, the skylight sits in it.
expect Mall.roofSlabs |> List.all((b) => b.x1 <= voidRect.x0 + eps || b.x0 >= voidRect.x1 - eps || b.z1 <= voidRect.z0 + eps || b.z0 >= voidRect.z1 - eps)
expect List.length(lampHeads) == 12.0
// The lot's lamp posts and the anchor store are solid wherever you reach them.
let mallSolid = Block.townAt(Block.mallTown).solidOn
expect lampHeads |> List.all((p) => Block.fromFootprint(footprint, Vec3.make(Vec3.x(p), 0.0, Vec3.z(p))) < mallSolid)
expect Mall.anchor.x1 - footprint.x1 < mallSolid
// The mall town: the mall alone; its views in its cells, but the far lot's
// (past the collar: the mall draws as its shell there).
expect (Block.townAt(Block.mallTown).copies |> List.map((c) => c.archetype)) == ["mall"]
expect Game.mallViews |> List.all((w) =>
  let p = Vec3.make(w.x, w.level * storey + 0.92 + 0.73, w.z) in
  Block.fromFootprint(footprint, p) > reach || Cells.locate(p, mall.level) != Option.None)
expect Game.mallViews |> List.any((w) => Block.fromFootprint(footprint, Vec3.make(w.x, 0.0, w.z)) > Block.activeOff)

// The bake: one 1024² lightmap per hour (4 MiB each, RGBA8), a texel per
// 1/6 m. Absent or stale, nothing to check.
let pngSide = (name: string): float =>
  Bytes.u32be(16.0, File.bytes(Provenance.lightPathOf(Archetypes.mallBake, name))) |> Binary.require
expect Archetypes.mallStatus != Provenance.Current || (pngSide("night") == 1024.0 && pngSide("day") == 1024.0)
