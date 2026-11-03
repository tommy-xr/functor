// The mall archetype, from mallLayout.fun and its bake (mallInterior.fun):
// its culling cells, colliders, skylight, street side and parking lot.
// archetypes.fun holds it. Not hashed into the bake's provenance: the bake
// doesn't depend on it.

open Layout
open MallLayout

let cellAt = MallInterior.cellAt
let baseOf = MallInterior.baseOf

// ---- Cells -----------------------------------------------------------------------

// The collar's top, over every eye on the ground (nothing reaches the roof).
let collarTop = Portals.outsideTop

// A cell's height range: a storey on its level; the void's from the court's
// floor to the skylight's apex; the apron's the collar's.
let spanOf = (i: float): (float, float) =>
  match cellAt(i).kind with
  | Lot => (0.0, collarTop)
  | Void => (0.0, apex)
  | _ => (baseOf(i), baseOf(i) + storey)
let cellBox = (i: float): Cells.cell =>
  match cellAt(i).kind with
  | Lot => Portals.cellOf(layout, collarTop, i)
  | _ => let (y0, y1) = spanOf(i) in Portals.box3(cellAt(i).rect, y0, y1)

// The whole face two open-plan neighbours share, as a portal polygon.
let sharedFace = (a: float, b: float): List<Vec3.t> =>
  let ra = cellAt(a).rect in
  let rb = cellAt(b).rect in
  let (ya0, ya1) = spanOf(a) in
  let (yb0, yb1) = spanOf(b) in
  let y0 = Math.max(ya0, yb0) in
  match Building.sideBetween(ra, rb) with
  | Option.None => Binary.invalid($"mall cells {a} and {b} share no face")
  | Option.Some(side) =>
    let (lo, hi) = if side == North || side == South then (Math.max(ra.x0, rb.x0), Math.min(ra.x1, rb.x1))
      else (Math.max(ra.z0, rb.z0), Math.min(ra.z1, rb.z1)) in
    Portals.opening(side, Building.lineOf(side, ra), y0, { lo: lo, hi: hi, bottom: 0.0, top: Math.min(ya1, yb1) - y0 })

// Every cell: the mall's, then the collar strips the apron doesn't hold.
// Portals: each open-plan face, each door, the collar's corners.
let cellList = List.flatten([
  List.range(List.length(cells)) |> List.map(cellBox),
  Portals.extraSides(layout) |> List.map((s) => Portals.strip(layout, collarTop, s)),
])
let portalList = List.flatten([
  links |> List.concatMap((p) => let (a, b) = p in Portals.both(a, b, sharedFace(a, b))),
  doors |> List.concatMap((d) => Portals.both(d.a, d.b, Portals.doorway(layout, d))),
  [South, North] |> List.concatMap((ns) => [West, East] |> List.concatMap((we) =>
    Portals.both(Portals.stripId(layout, ns), Portals.stripId(layout, we), Portals.corner(layout, collarTop, ns, we)))),
])
// A walk stops at a portal under `minPortal` of the view both ways: a shop
// seen far down the concourse, a stockroom door through a far shopfront.
let minPortal = 0.06
let level = Cells.make({ cells: cellList, portals: portalList }) |> Cells.withMinPortal(minPortal)

// ---- The skylight ------------------------------------------------------------------

// A space-frame pyramid over the void, from the roof to `apex`: glazing
// panels, unlit, 1.5 m apart on a dark frame. Inside it is sky-blue by day
// and navy by night; outside (the street side's) dull glass, 5 cm proud.
// Not in the bake: daylight comes in through it (mallInterior.fun).
let frameTexture = (glass: Building.Paint): Texture.pixels =>
  Building.image(32.0, 32.0, (x, y) => if x < 2.0 || y < 2.0 then framePaint else glass)
let framePaint = Building.paint(40.0, 44.0, 52.0)
let glassOf = (p: Building.Paint): MallInterior.Palette =>
  { groups: [Building.material(p, 1.5, true, frameTexture(p)), Building.material(framePaint, 1.0, true, Building.solid(framePaint))],
    slots: [MallInterior.tiledSlot(0.0, 1.5), MallInterior.tiledSlot(1.0, 1.0)] }
// Its four faces, outward, each eave its first edge (the UVs' axis), `out`
// metres proud of the opening; and a frame bar down each hip, where the
// faces' grids meet, `out` + 5 cm proud (below it inside).
let pyramid = (out: float): List<MallInterior.Poly> =>
  let w = voidRect in
  let a = v(w.x0 - out, roof, w.z0 - out) in
  let b = v(w.x1 + out, roof, w.z0 - out) in
  let c = v(w.x1 + out, roof, w.z1 + out) in
  let d = v(w.x0 - out, roof, w.z1 + out) in
  let p = v((w.x0 + w.x1) / 2.0, apex + out, (w.z0 + w.z1) / 2.0) in
  [MallInterior.tri(0.0, a, b, p), MallInterior.tri(0.0, b, c, p), MallInterior.tri(0.0, c, d, p), MallInterior.tri(0.0, d, a, p)]
let hips = (out: float, lift: float): List<MallInterior.Poly> =>
  pyramid(out) |> List.map((f) =>
    let c = MallInterior.pt(0.0, f) in
    let up = MallInterior.pt(2.0, f) |> Vec3.sub(c) in
    let across = up |> Vec3.cross(v(0.0, 1.0, 0.0)) |> Vec3.normalize |> Vec3.scale(0.25) in
    MallInterior.quad(1.0, c |> Vec3.sub(Vec3.scale(0.5, across)) |> Vec3.add(v(0.0, lift, 0.0)), up, across))
// The glazing `out` proud: inside (0) its bars hang below it, outside above.
let glazing = (p: Building.Paint, out: float): Scene.t =>
  let pal = glassOf(p) in
  let polys = List.flatten([pyramid(out), hips(out, if out > 0.0 then 0.05 else -0.05)]) in
  Scene.modelOf(Model.static(MallInterior.meshOf(pal, 1.0, polys, MallInterior.spansOf(pal, 0.0, polys))))
let skylightDay = glazing(Building.paint(196.0, 220.0, 244.0), 0.0) |> Scene.inCells([courtVoid])
let skylightNight = glazing(Building.paint(16.0, 20.0, 40.0), 0.0) |> Scene.inCells([courtVoid])
let roofGlass: Exterior.Grid = {
  night: glazing(Building.paint(30.0, 34.0, 58.0), 0.05),
  day: glazing(Building.paint(128.0, 140.0, 152.0), 0.05),
}

// ---- Drawing -----------------------------------------------------------------------

// Where a cell model draws: in `cell`, moved by `at`. `part` is the mesh's
// cell it was cut from: a bay's home cell, drawn again in every bay with its
// fit-out (the same `Model.static`, so one GPU copy, as the towers' floors).
// The bake's blockers, past the cells, don't draw.
type Placed = { model: Model.t, part: float, cell: float, at: Vec3.t }
let placedOf = (b: Provenance.Baked): List<Placed> =>
  b.models |> List.filter((p) => p.cell < List.length(cells)) |> List.concatMap((p) =>
    match MallInterior.bayOf(p.cell) with
    | Option.Some(home) =>
      List.range(List.length(bays)) |> List.filter((k) => MallInterior.homeOf(k) == home) |> List.map((k) =>
        { model: p.model, part: p.cell, cell: p.cell + 2.0 * (k - home), at: MallInterior.offsetOf(k) })
    | Option.None => [{ model: p.model, part: p.cell, cell: p.cell, at: v(0.0, 0.0, 0.0) }])
// A placed model in its cell. (`roomOf`'s body is the e2e's draw-count
// seam, as archetypes.fun's.)
let roomOf = (p: Placed): Scene.t => Scene.modelOf(p.model) |> Scene.translate(p.at) |> Scene.inCells([p.cell])
// Stand-ins for the shops and stockrooms a walk cuts (`minPortal`): their
// walls, floor and ceiling 5 cm behind the real ones, one unlit dark mesh. A
// room that draws hides its stand-in; through a cut room's opening it shows
// instead of the clear colour.
let standInPaint = Building.paint(46.0, 43.0, 38.0)
let standInPalette: MallInterior.Palette =
  { groups: [Building.material(standInPaint, 1.0, true, Building.solid(standInPaint))], slots: [MallInterior.tiledSlot(0.0, 1.0)] }
let standInPolys = List.range(List.length(cells)) |> List.filter((i) => cellAt(i).kind == Shop || cellAt(i).kind == Stock) |> List.concatMap((i) =>
  let r = MallInterior.inner(i) in
  List.flatten([[MallInterior.floorAt(0.0, r.x0, r.z0, r.x1, r.z1, 0.0), MallInterior.ceilingAt(0.0, r.x0, r.z0, r.x1, r.z1, MallInterior.ceilingOf(i))], MallInterior.wallsOf(i)])
    |> List.map((p) => { mat: 0.0, pts: p.pts |> List.map((q) => q |> Vec3.sub(Vec3.scale(0.05, MallInterior.normalOf(p))) |> Vec3.add(v(0.0, baseOf(i), 0.0))) }))
let standIn = Scene.modelOf(Model.static(MallInterior.meshOf(standInPalette, 1.0, standInPolys, MallInterior.spansOf(standInPalette, 0.0, standInPolys))))
  |> Scene.inCells(List.range(List.length(cells)) |> List.filter(MallInterior.isInside))
// The cell models, the skylight and the stand-ins by night, and under the
// day bake.
let nightOf = (b: Provenance.Baked): List<Scene.t> => List.flatten([placedOf(b) |> List.map(roomOf), [skylightNight, standIn]])
let dayOf = (b: Provenance.Baked): List<Scene.t> =>
  List.flatten([placedOf(b) |> List.map((p) => Provenance.underDay(b.day, roomOf(p))), [skylightDay, standIn]])

// ---- The street side and the lot -------------------------------------------------

let box = Exterior.box
let street0 = Exterior.street
let asphalt = Building.rgb(34.0, 33.0, 38.0)
let linePaint = Building.rgb(200.0, 200.0, 190.0)
let stopGrey = Building.rgb(110.0, 108.0, 104.0)

// The roof slab, open over the void (the skylight stands in the opening),
// its edges 2 cm behind the void's fascia so they don't z-fight it; a
// glazed ridge over each upper hall.
let roofSlabs: List<Exterior.Box> =
  let f = footprint in
  let g = 0.02 in
  let w = rect(voidRect.x0 - g, voidRect.z0 - g, voidRect.x1 + g, voidRect.z1 + g) in
  let o = facade / 4.0 in
  let y0 = roof - 0.3 in
  [box(f.x0 - o, y0, f.z0 - o, f.x1 + o, roof, w.z0), box(f.x0 - o, y0, w.z1, f.x1 + o, roof, f.z1 + o),
   box(f.x0 - o, y0, w.z0, w.x0, roof, w.z1), box(w.x1, y0, w.z0, f.x1 + o, roof, w.z1)]
let ridges = [box(3.0, roof, 19.0, 23.0, roof + 0.7, 23.0), box(59.0, roof, 19.0, 73.0, roof + 0.7, 23.0)]
// The anchor store: a lower box off the east end, a dark sign plate on it.
let anchor = box(footprint.x1 + facade / 2.0, street0, 3.0, 94.0, 7.5, 39.0)
let anchorSign = box(80.0, 4.6, 2.9, 90.0, 6.2, 3.0)
let anchorNeon = box(80.0, 6.3, 2.88, 90.0, 6.38, 3.0)

// The lot: asphalt, a line between every two bays across each double row
// and one down its median, and a wheel stop at the head of every bay.
let lotLine = 0.06
let bayXs = List.range(Math.floor((lotRect.x1 - lotRect.x0 - 4.0) / bay) + 1.0) |> List.map((k) => lotRect.x0 + 2.0 + k * bay)
let lotBoxes: List<(Exterior.Box, Color.t)> =
  let y0 = street0 in
  let y1 = street0 + 0.02 in
  let first = List.head(bayXs) |> Option.defaultValue(0.0) in
  let last = List.head(List.reverse(bayXs)) |> Option.defaultValue(0.0) in
  List.flatten([
    [(box(lotRect.x0, y0 - 0.02, lotRect.z0, lotRect.x1, y0, lotRect.z1), asphalt)],
    medians |> List.concatMap((m) => List.flatten([
      [(box(first, y0, m - lotLine, last, y1, m + lotLine), linePaint)],
      bayXs |> List.map((x) => (box(x - lotLine, y0, m - bayDepth, x + lotLine, y1, m + bayDepth), linePaint)),
      List.take(List.length(bayXs) - 1.0, bayXs) |> List.concatMap((x) =>
        let c = x + bay / 2.0 in
        [(box(c - 0.8, y0, m + 0.5, c + 0.8, y0 + 0.12, m + 0.65), stopGrey),
         (box(c - 0.8, y0, m - 0.65, c + 0.8, y0 + 0.12, m - 0.5), stopGrey)]),
    ])),
  ])

// The kit's street side (facade, canopy, neon roofline, lamps, apron), the
// roof opened over the void, the ridges, the anchor and the lot; the
// skylight's glazing in the window grid's place.
let kitStreet = Exterior.streetOf(layout)
let street: Exterior.Street =
  let s = kitStreet in
  let extras = List.flatten([
    ridges |> List.map((b) => (b, Exterior.glass)),
    [(anchor, Exterior.concrete), (anchorSign, Exterior.glass)],
    lotBoxes,
  ]) in
  let boxes = List.flatten([
    s.streetBoxes |> List.filter((e) => let (b, _) = e in b != s.roofBox),
    roofSlabs |> List.map((b) => (b, Exterior.roofGrey)),
    extras,
  ]) in
  let neon = List.append(s.neonBoxes, [anchorNeon]) in
  { s with streetBoxes: boxes, streetSide: Exterior.instancedOf(boxes), extraBoxes: extras,
           neonBoxes: neon, neonStrips: Scene.group(neon |> List.map(Exterior.slabOf)), grid: Option.Some(roofGlass) }

// ---- Collision ---------------------------------------------------------------------

let block = Building.block
let fromBox = (tag: string, dy: float, b: MallInterior.Box3): Building.Block =>
  block(tag, b.x0, b.y0 + dy, b.z0, b.x1, b.y1 + dy, b.z1)

// Cell `i`'s walls, between wall centre lines round its doors, a storey
// high (the upper facade on to the roofline).
let wallBlocks = (i: float): List<Building.Block> =>
  let c = cellAt(i) in
  let r = c.rect in
  let y = baseOf(i) in
  if not MallInterior.isInside(i) || c.kind == Void then []
  else MallInterior.sides |> List.filter((s) => not MallInterior.isOpen(i, s)) |> List.concatMap((s) =>
    let line = Building.lineOf(s, r) in
    let t = Building.halfWall(layout, s, r) in
    let onFacade = Building.onFootprint(layout, s, r) in
    let top = if onFacade && c.level == 1.0 then facadeHeight - storey else storey in
    let (s0, s1) = if s == North || s == South then (r.x0, r.x1) else (r.z0, r.z1) in
    Building.pieces(s0, s1, top, MallInterior.holesOn(i, s)) |> List.indexedMap((k, p) =>
      let tag = $"wall-{i}-{s}-{k}" in
      if s == North || s == South then block(tag, p.u0, y + p.y0, line - t, p.u1, y + p.y1, line + t)
      else block(tag, line - t, y + p.y0, p.u0, line + t, y + p.y1, p.u1)))

// The escalators: each lane's ramp through its nosings, from the court's
// floor to the top plate; boxes along the balustrades from the floor to
// the handrails, the body's end under the top plate, the barrier.
let steps = MallInterior.stepsOf(escalator)
let rampOf = (lane: (float, float)): MallInterior.Poly =>
  let (za, zb) = lane in
  MallInterior.quad(0.0, v(steps.xp, storey, za), v(0.0, 0.0, zb - za), v(steps.xq - steps.xp, 0.0 - storey, 0.0))
let escalatorBlocks = List.flatten([
  steps.rails |> List.indexedMap((k, rail) =>
    let (za, zb) = rail in block($"escalator-rail-{k}", escalator.xTop, 0.0, za, steps.xEnd, storey + escalator.rail, zb)),
  [block("escalator-end", escalator.xTop, 0.0, escalator.z0, steps.xp, storey - 0.05, escalator.z1)],
  MallInterior.barrierBoxes(escalator, steps) |> List.indexedMap((k, b) => fromBox($"barrier-{k}", 0.0, b)),
])

// Furniture: tables, the shops' fit-outs and the stockrooms' shelving, the
// fountain, the galleries' balustrades and planters.
let furnitureBlocks = List.flatten([
  tables |> List.indexedMap((k, t) => let top = MallInterior.tableBox(t) in fromBox($"table-{k}", 0.0, { top with y0: 0.0 })),
  [fromBox("fountain", 0.0, MallInterior.box3(basin.x0, 0.0, basin.z0, basin.x1, MallInterior.rimHeight, basin.z1))],
  List.range(List.length(cells)) |> List.concatMap((i) => List.flatten([
    MallInterior.itemsOf(i) |> List.concatMap((t) => t.solid) |> List.indexedMap((k, b) => fromBox($"item-{i}-{k}", baseOf(i), b)),
    MallInterior.balustradeBoxes(i) |> List.indexedMap((k, b) => fromBox($"balustrade-{i}-{k}", baseOf(i), b)),
    MallInterior.planterBoxes(i) |> List.indexedMap((k, b) => fromBox($"planter-{i}-{k}", baseOf(i), b)),
  ])),
])

// Every cell's faces, a bay's own though the mesh has only its home's.
let faces = List.range(List.length(cells)) |> List.concatMap(MallInterior.facesOf)
// Floors: up-facing faces level with a storey's floor (thresholds, plates).
let floorPolys = faces |> List.filter((p) =>
  Vec3.y(MallInterior.normalOf(p)) > 0.99 && (p.mat == MallInterior.tile || p.mat == MallInterior.trim || p.mat == MallInterior.concrete)
    && (Building.near(Vec3.y(MallInterior.pt(0.0, p)), 0.0) || Building.near(Vec3.y(MallInterior.pt(0.0, p)), storey)))
let trimeshOf = (polys: List<MallInterior.Poly>): Physics.shape =>
  Physics.trimesh({ positions: Bytes.fromF32le(polys |> List.concatMap((p) => p.pts |> List.concatMap((q) => MallInterior.xyz(1.0, q)))),
                    triangles: Bytes.fromU32le(MallInterior.indicesOf(polys)) })

// Every collider in the mall's space: the facade's outer half (the shop
// bands behind the shutters have no cells), every wall, the floors, the
// escalators' ramps, the furniture, and the anchor store's massing.
let colliders: List<Building.Collider> =
  let origin = v(0.0, 0.0, 0.0) in
  let boxes = List.flatten([
    street.facadeBoxes |> List.indexedMap((k, b) => fromBox($"facade-{k}", 0.0, MallInterior.box3(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1))),
    List.range(List.length(cells)) |> List.concatMap(wallBlocks),
    escalatorBlocks,
    furnitureBlocks,
    [block("anchor", anchor.x0, anchor.y0, anchor.z0, anchor.x1, anchor.y1, anchor.z1)],
  ]) in
  List.flatten([
    boxes |> List.map((b) => { tag: b.tag, shape: Physics.box(b.w, b.h, b.d), at: v(b.x, b.y, b.z) }),
    [{ tag: "floors", shape: trimeshOf(floorPolys), at: origin }],
    steps.lanes |> List.indexedMap((k, lane) => { tag: $"ramp-{k}", shape: trimeshOf([rampOf(lane)]), at: origin }),
  ])
