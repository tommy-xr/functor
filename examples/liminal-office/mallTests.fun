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

// 19 cells and three more collar strips; a portal each way on every
// open-plan face, door and collar corner.
expect List.length(Mall.cellList) == 22.0
expect List.length(Mall.portalList) == 2.0 * (List.length(links) + List.length(doors) + 4.0)
expect (cells |> List.filter((c) => c.kind == Void) |> List.length) == 1.0
// The void is one box from the court's floor to the skylight's apex,
// over both levels.
expect Mall.spanOf(courtVoid) == (0.0, apex) && cellAt(courtVoid).rect == voidRect

// Every face lies in its cell's box: balustrades in their galleries, the
// escalators and fountain in the void.
let inBox = (i: float, p: Vec3.t): bool =>
  let r = cellAt(i).rect in
  let (y0, y1) = Mall.spanOf(i) in
  Vec3.x(p) >= r.x0 - eps && Vec3.x(p) <= r.x1 + eps && Vec3.z(p) >= r.z0 - eps && Vec3.z(p) <= r.z1 + eps
    && Vec3.y(p) >= y0 - eps && Vec3.y(p) <= y1 + eps
expect List.range(cellCount) |> List.all((i) =>
  List.nth(i, MallInterior.interior.cellParts) |> Option.defaultValue([]) |> List.all((p) => p.pts |> List.all((q) => inBox(i, q))))
// Faces are flat, and each polygon's normal is its triangles'.
expect MallInterior.interior.cellParts |> List.flatten |> List.all((p) =>
  let n = MallInterior.normalOf(p) in
  p.pts |> List.all((q) => near(Vec3.dot(n, q |> Vec3.sub(MallInterior.pt(0.0, p))), 0.0)))
// One model per cell with faces; the apron's is its half of the entrance.
expect List.length(Archetypes.mallBaked.models) == cellCount
expect List.length(mall.night) == cellCount + 1.0 && List.length(mall.day) == cellCount + 1.0

// The galleries: a balustrade 1.1 m high along every edge of the void,
// open only where the escalators land; the lower concourse has none.
let galleries = List.range(cellCount) |> List.filter((i) => Option.isSome(MallInterior.voidSide(i)))
expect galleries == [galleryS, galleryN, bridgeW, 15.0]
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
