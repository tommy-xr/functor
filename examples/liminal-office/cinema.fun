// The cinema archetype's unbaked parts (archetypes.fun assembles it): its
// cells, colliders, the auditoria's seats and the street side (a blank box,
// a neon sign tower, posters under a marquee, a parking lot). Not hashed
// into the bake's provenance: the bake doesn't depend on it.

open Layout

let v = Vec3.make
let l = CinemaLayout.layout

// The collar's top, over the roofline.
let collarTop = 10.0
let level = Portals.levelOf(l, collarTop)

// ---- Seats -------------------------------------------------------------------------

// A seat, from its row's front edge on the tread, centred on x: a cushion
// and a back facing the screen (-z), a lighter cap on the back, an arm on
// its +x side; each part a box of its own shade, since the seats aren't in
// the bake and draw unlit.
let seatParts: List<(Exterior.Box, Color.t)> = [
  (Exterior.box(-0.25, 0.36, 0.3, 0.25, 0.5, 0.8), Building.rgb(176.0, 34.0, 46.0)),
  (Exterior.box(-0.25, 0.36, 0.8, 0.25, 0.96, 0.92), Building.rgb(120.0, 22.0, 34.0)),
  (Exterior.box(-0.25, 0.96, 0.79, 0.25, 1.02, 0.93), Building.rgb(190.0, 44.0, 56.0)),
  (Exterior.box(0.26, 0.0, 0.32, 0.32, 0.62, 0.9), Building.rgb(22.0, 18.0, 22.0)),
]

// Each auditorium's seats, 0.6 m apart on every row between the aisles:
// not in the bake, but one instanced cube per auditorium, drawn in its
// cell.
let seatPitch = 0.6
let seatsAt = (i: float): List<Vec3.t> =>
  let r = CinemaBuilding.innerOf(i) in
  let x0 = r.x0 + CinemaLayout.aisle in
  let n = Math.floor((r.x1 - CinemaLayout.aisle - x0) / seatPitch) in
  let pad = (r.x1 - CinemaLayout.aisle - x0 - n * seatPitch) / 2.0 in
  List.range(CinemaLayout.rows) |> List.concatMap((k) =>
    List.range(n) |> List.map((j) => v(x0 + pad + (j + 0.5) * seatPitch, CinemaBuilding.treadY(k), CinemaBuilding.rowZ(r, k))))
let seatsOf = (i: float): Scene.t =>
  Scene.cube() |> Scene.color(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(seatsAt(i) |> List.concatMap((p) =>
    seatParts |> List.map((e) =>
      let (b, c) = e in
      Instance.trs(p |> Vec3.add(v((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0)), 0deg, b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0)
        |> Instance.tint(c)))) |> Scene.inCells([i])
let seats = CinemaLayout.auditoria |> List.map(seatsOf)

// ---- Drawing -------------------------------------------------------------------------

// A baked cell model in its cell, and an unlit one in its cell. (The e2e's
// draw-count variants patch these bodies.)
let roomOf = (p: Building.CellModel): Scene.t => Scene.modelOf(p.model) |> Scene.inCells([p.cell])
let unlitOf = (p: Building.CellModel): Scene.t => Scene.inCells([p.cell], Scene.modelOf(p.model))
// The groups each draws: one per look among its cell's lit faces, or its
// unlit ones.
let litGroups = (cell: float): float =>
  List.length(Paints.spansOf(CinemaBuilding.palette, 0.0, List.nth(cell, CinemaBuilding.interior.cellParts) |> Option.defaultValue([])))
let unlitGroups = (cell: float): float =>
  let faces = List.nth(cell, CinemaBuilding.cellParts) |> Option.defaultValue([]) |> List.filter((f) => Paints.isUnlit(CinemaBuilding.palette, f)) in
  List.length(Paints.spansOf(CinemaBuilding.palette, 0.0, faces))
// The seats, the cells' unlit models, and their baked ones, by night and
// under the day bake.
let fixed = List.append(seats, CinemaBuilding.unlit |> List.map(unlitOf))
let nightOf = (b: Provenance.Baked): List<Scene.t> => List.append(fixed, b.models |> List.map(roomOf))
let dayOf = (b: Provenance.Baked): List<Scene.t> =>
  List.append(fixed, b.models |> List.map((p) => Provenance.underDay(b.day, roomOf(p))))

// ---- The street side --------------------------------------------------------------

let box = Exterior.box
let street = Exterior.street
// The facade's south face.
let front = l.footprint.z0 - l.facade / 2.0

// The marquee over the entrance, and the posters under it: four light
// boxes on the facade, 10 cm proud (glowing faces in the glow atlas), in
// shallow black frames.
let marquee = box(21.0, 3.1, front - 4.5, 39.0, 3.6, front)
let posterSpans = [(22.4, 23.8), (24.8, 26.2), (33.8, 35.2), (36.2, 37.6)]
let posterFrames = posterSpans |> List.map((e) => let (x0, x1) = e in box(x0 - 0.06, 0.44, front - 0.07, x1 + 0.06, 2.56, front))
let posterMesh: Scene.t =
  let faces = posterSpans |> List.indexedMap((k, e) =>
    let (x0, x1) = e in Building.wallZ(CinemaBuilding.posterPaint(k), x0, x1, 0.5, 2.5, front - 0.1, false)) in
  let p = CinemaBuilding.palette in
  Scene.modelOf(Model.static(Paints.meshOf(p, faces, Paints.spansOf(p, 0.0, faces))))

// A pink band along the facade under the roofline's cyan.
let facadeBand = box(l.footprint.x0 - 0.15, 7.6, front - 0.06, l.footprint.x1 + 0.15, 7.8, front)

// The sign tower by the lot's entrance: a dark shaft, cyan tubes up its
// corners, pink bands across its south and west faces.
let signTower = box(50.5, 0.0, 0.0 - 9.0, 53.5, 18.0, 0.0 - 6.0)
let towerDark = Building.rgb(30.0, 24.0, 40.0)
let towerTubes: List<Exterior.Box> =
  let t = signTower in
  [(t.x0 - 0.12, t.z0 - 0.12), (t.x1, t.z0 - 0.12), (t.x0 - 0.12, t.z1), (t.x1, t.z1)]
    |> List.map((e) => let (x, z) = e in box(x, 1.5, z, x + 0.12, 17.6, z + 0.12))
let pinkBands: List<Exterior.Box> =
  let t = signTower in
  List.range(9.0) |> List.concatMap((k) =>
    let y = 4.0 + 1.5 * k in
    [box(t.x0, y, t.z0 - 0.06, t.x1, y + 0.3, t.z0), box(t.x0 - 0.06, y, t.z0, t.x0, y + 0.3, t.z1)])

// The parking lot (the streetscape kit's): asphalt either side of the apron
// and south of it, white bays in four rows (stall lines, an end line and a
// wheel stop each), the middle two back to back across the lamps' planted
// island, more islands at the rows' ends with a tree each, footings and
// row letters on the lamp posts (the layout's street lamps), a cart left in
// a bay; guardrails along its far edges, a lit pylon by its west entry, and
// a fenced vacant lot east of it. Its frame is the cinema's origin facing
// south: u is x, d is -z.
let lotFrame: Streetscape.Frame = { side: South, x: 0.0, z: 0.0 }
let rowEnds = [(13.0, 18.0), (23.0, 35.0), (40.0, 45.0)]
let lotPlan: Streetscape.Lot = {
  fr: lotFrame,
  // Either side of the apron, from the sidewalk's curb (its ramps meet the lot).
  areas: [rect(0.0 - 10.0, 12.0, 70.0, 50.0), rect(0.0 - 10.0, Streetscape.depth - front, 15.0, 12.0), rect(45.0, Streetscape.depth - front, 70.0, 12.0)],
  first: 0.0, n: 23.0, bay: 2.6,
  rows: [{ d0: 13.0, d1: 18.0, head: 13.0 }, { d0: 23.0, d1: 28.0, head: 28.0 }, { d0: 30.0, d1: 35.0, head: 30.0 }, { d0: 40.0, d1: 45.0, head: 45.0 }],
  islands: List.flatten([
    [rect(0.0 - 0.3, 28.05, 60.1, 29.95)],
    rowEnds |> List.concatMap((e) => let (d0, d1) = e in [rect(0.0 - 2.4, d0, 0.0 - 0.3, d1), rect(60.1, d0, 62.2, d1)]),
  ]),
  poles: l.streetLamps |> List.map((p) => (Vec3.x(p), 0.0 - Vec3.z(p))),
  cart: Option.Some((12.4, 42.6)),
  seed: 6.0,
}
// Row letters on the lamp posts: A by the apron; B and C either side of
// the island.
let rowLetters: List<Streetscape.Decal> = l.streetLamps |> List.concatMap((p) =>
  let d = 0.0 - Vec3.z(p) in
  let (front, back) = if d < 20.0 then (StreetAtlas.rowA, StreetAtlas.rowA) else (StreetAtlas.rowC, StreetAtlas.rowB) in
  Streetscape.postMarkers(lotFrame, Vec3.x(p), d, 0.0, front, back))
// The pylon: two posts and a lit box, "FREE PARKING", facing both ways.
let pylon: Streetscape.Props =
  let fr: Streetscape.Frame = { side: South, x: 0.0, z: 0.0 - 12.4 } in
  let face = (out: bool, d: float) => Streetscape.glow(StreetAtlas.boxLot, Streetscape.faceIn(fr, out, 0.0 - 8.56, 0.0 - 4.44, 2.44, 3.06, d)) in
  { Streetscape.nothing with
    plain: [(Streetscape.boxIn(fr, 0.0 - 8.6, 0.0 - 4.4, 2.4, 3.1, 0.0, 0.25), Streetscape.darkSteel),
            (Streetscape.boxIn(fr, 0.0 - 8.2, 0.0 - 8.0, Streetscape.road, 2.4, 0.03, 0.22), Streetscape.steel),
            (Streetscape.boxIn(fr, 0.0 - 5.0, 0.0 - 4.8, Streetscape.road, 2.4, 0.03, 0.22), Streetscape.steel)],
    decals: [face(true, 0.255), face(false, 0.0 - 0.005)],
    solid: [Streetscape.boxIn(fr, 0.0 - 8.2, 0.0 - 8.0, Streetscape.road, 2.4, 0.03, 0.22), Streetscape.boxIn(fr, 0.0 - 5.0, 0.0 - 4.8, Streetscape.road, 2.4, 0.03, 0.22)] }
// The vacant lot: chain-link round a dirt patch (a gate gap on the west),
// weeds and an old slab in it, and its board facing the lot.
let vacant = rect(74.0, 0.0 - 46.0, 100.0, 0.0 - 8.0)
let vacantLot: Streetscape.Props =
  let v0 = vacant in
  let h = 1.8 in
  let y = Streetscape.road in
  let weeds = List.range(16.0) |> List.map((k) =>
    let x = v0.x0 + 1.0 + (v0.x1 - v0.x0 - 2.0) * Building.noise(k * 3.7 + 1.0) in
    let z = v0.z0 + 1.0 + (v0.z1 - v0.z0 - 2.0) * Building.noise(k * 5.3 + 2.0) in
    let r = 0.2 + 0.4 * Building.noise(k * 7.1) in
    (box(x - r, y, z - r, x + r, y + 0.15 + 0.5 * Building.noise(k * 2.9), z + r), Streetscape.weedGreen)) in
  let board: Streetscape.Frame = { side: West, x: v0.x0 - 0.6, z: 0.0 } in
  Streetscape.joined([
    Streetscape.chainLink({ side: South, x: 0.0, z: v0.z0 }, v0.x0, v0.x1, 0.0, y, h),
    Streetscape.chainLink({ side: North, x: 0.0, z: v0.z1 }, v0.x0, v0.x1, 0.0, y, h),
    Streetscape.chainLink({ side: West, x: v0.x0, z: 0.0 }, v0.z0, 0.0 - 30.0, 0.0, y, h),
    Streetscape.chainLink({ side: West, x: v0.x0, z: 0.0 }, 0.0 - 26.0, v0.z1, 0.0, y, h),
    Streetscape.chainLink({ side: East, x: v0.x1, z: 0.0 }, v0.z0, v0.z1, 0.0, y, h),
    { Streetscape.nothing with
      plain: List.flatten([
        [(box(v0.x0 + 0.1, y, v0.z0 + 0.1, v0.x1 - 0.1, y + 0.012, v0.z1 - 0.1), Streetscape.dirt),
         (box(82.0, y, 0.0 - 31.0, 90.0, y + 0.08, 0.0 - 23.0), Streetscape.planterGrey),
         (Streetscape.boxIn(board, 0.0 - 27.5, 0.0 - 25.7, 1.1, 2.3, 0.0, 0.04), Streetscape.steel),
         (Streetscape.boxIn(board, 0.0 - 27.3, 0.0 - 27.2, y, 1.1, 0.04, 0.12), Streetscape.steel),
         (Streetscape.boxIn(board, 0.0 - 26.0, 0.0 - 25.9, y, 1.1, 0.04, 0.12), Streetscape.steel)],
        weeds,
      ]),
      decals: [Streetscape.sign(StreetAtlas.available, Streetscape.faceIn(board, true, 0.0 - 27.4, 0.0 - 25.8, 1.2, 2.2, 0.045))] },
  ])
// The lot and what stands round it: all of it seen from across the lot, so
// its decals draw with the shell too (the grid).
let lotProps: Streetscape.Props = Streetscape.joined(List.flatten([
  [Streetscape.lotOf(lotPlan), pylon, vacantLot, { Streetscape.nothing with decals: rowLetters }],
  rowEnds |> List.concatMap((e) => let (d0, d1) = e in
    [Streetscape.tree(lotFrame, 0.0 - 1.35, (d0 + d1) / 2.0, Streetscape.walk, d0), Streetscape.tree(lotFrame, 61.15, (d0 + d1) / 2.0, Streetscape.walk, d1)]),
  [Streetscape.guardrail({ side: South, x: 0.0, z: 0.0 - 50.6 }, 0.0 - 10.6, 70.0, 0.0, Streetscape.road),
   Streetscape.guardrail({ side: West, x: 0.0 - 10.6, z: 0.0 }, 0.0 - 50.6, 0.0 - 12.0, 0.0, Streetscape.road),
   Streetscape.poleSign(lotFrame, 15.3, 11.6, Streetscape.road, StreetAtlas.noParking),
   Streetscape.poleSign(lotFrame, 44.7, 11.6, Streetscape.road, StreetAtlas.noParking)],
]))
let lotDressing = Streetscape.dressingOf(lotProps.decals)

// The e2e's walk: from the lot (view 9) to the apron and in.
let paths: List<Streetscape.Path> = [{ x0: 40.0, z0: 0.0 - 26.0, x1: 30.0, z1: 0.0 - 4.0 }, { x0: 30.0, z0: 0.0 - 4.0, x1: 30.0, z1: 2.0 }]
// The sidewalk: a vending machine, a payphone and a bin west of the
// marquee, a bike rack, benches under it, bollards across the forecourt,
// fire lane signs either side; and the lot (its decals in the grid).
let streetscapeProps = (s: Exterior.Street): Streetscape.Props =>
  let south = Streetscape.sideFrame(l, South) in
  let curbside = Streetscape.flipped(south, 3.25) in
  let walk = Streetscape.walk in
  let own = [
    { lotProps with decals: [] },
    Streetscape.vendingMachine(south, 18.5, 0.15, walk),
    Streetscape.payphone(south, 16.6, 0.15, walk),
    Streetscape.bin(south, 19.4, 0.2, walk),
    Streetscape.bikeRack(south, 12.5, 1.2, walk),
    Streetscape.bench(curbside, 24.0, 0.0, walk),
    Streetscape.bench(curbside, 36.0, 0.0, walk),
    Streetscape.joined([27.4, 32.6] |> List.map((u) => Streetscape.bollard(south, u, 3.55, Exterior.street))),
    Streetscape.poleSign(south, 14.5, 3.5, walk, StreetAtlas.fireLane),
    Streetscape.poleSign(south, 45.5, 3.5, walk, StreetAtlas.fireLane),
  ] in
  Streetscape.along(l, s, { seed: 3.0, paths: paths, own: own })
let streetscape = (s: Exterior.Street): Exterior.Street => Streetscape.dressed(s, streetscapeProps(s))

let pinkColor = (glow: float): Color.t => Building.rgb(255.0 * glow, 60.0 * glow, 190.0 * glow)
// What glows besides the kit's neon: the posters, the sign tower's pink.
let glowAt = (glow: float): Scene.t =>
  Scene.group([posterMesh, Scene.group(List.append([facadeBand], pinkBands) |> List.map(Exterior.slabOf)) |> Scene.color(pinkColor(glow))])

// Painted stucco over a tiled base, in piers and a cap of a paler coat.
let stuccoWall = Finishes.on(Finishes.stucco, Building.rgb(122.0, 104.0, 132.0))
let stuccoPale = Finishes.on(Finishes.stucco, Building.rgb(146.0, 132.0, 154.0))
let baseTiles = Finishes.on(Finishes.tiles, Building.rgb(64.0, 118.0, 124.0))

// The facade kit's trim: the tiled base, piers every 10 m (under the pink
// band) and the cap, a
// frame round the doors and under the marquee, and plant on the roof by a
// stair's head house.
let dressed = (s: Exterior.Street): Exterior.Street =>
  let roof = l.levels * l.storey in
  let head = rect(4.0, 30.0, 8.0, 34.0) in
  Facade.trimmed(s, {
    plain: List.flatten([
      Facade.painted(Facade.frameDark, List.flatten([Facade.portal(l, s, 0.4, 0.12), Facade.canopyEdge(s, 0.2)])),
      Facade.stairHead(head, roof, 2.7, Facade.headGrey),
      Facade.rooftop(3.0, rect(2.0, 2.0, 58.0, 38.0), roof, [head]),
    ]),
    skinned: List.flatten([
      Facade.clad(baseTiles, Facade.courses(l, s, [(street, 1.1)], 0.04)),
      Facade.clad(stuccoPale, List.flatten([Facade.pilasters(l, s, 10.0, 0.9, 0.12, facadeBand.y0 - 0.1), Facade.cornice(l, 0.2, 0.35)])),
    ]),
    fine: [],
  })

// The kit's street side in stucco with the marquee for its canopy, cyan
// along the marquee, the roofline and up the sign tower; the tower and
// poster frames among the lit boxes; the posters, pink and the lot's signs
// as its grid; then the streetscape. The facade's outer half (the kit's) is solid all round the
// footprint.
let exterior: Exterior.Street =
  let s = Exterior.streetWith(l, stuccoWall) in
  let extras = List.flatten([
    [(signTower, towerDark)],
    posterFrames |> List.map((b) => (b, Building.rgb(14.0, 12.0, 16.0))),
  ]) in
  let boxes = List.flatten([s.streetBoxes |> List.map((e) => let (b, c) = e in if b == s.canopyBox then (marquee, c) else e), extras]) in
  let m = marquee in
  // The kit's neon past its canopy's strip: the roofline.
  let neon = List.flatten([
    Exterior.neonBoxesOf(l) |> List.drop(1.0),
    [box(m.x0, m.y0 + 0.1, m.z0 - 0.06, m.x1, m.y0 + 0.2, m.z0)],
    towerTubes,
  ]) in
  let lot = { s with canopyBox: marquee, streetBoxes: boxes, extraBoxes: extras,
                     neonBoxes: neon, neonStrips: Scene.group(neon |> List.map(Exterior.slabOf)),
                     grid: Option.Some({ night: Scene.group([glowAt(1.0), lotDressing.night]), day: Scene.group([glowAt(0.6), lotDressing.day]) }) } in
  streetscape(dressed(lot))

// ---- Collision -----------------------------------------------------------------------

let block = Building.block
let boxBlock = (tag: string, b: Exterior.Box): Building.Block => block(tag, b.x0, b.y0, b.z0, b.x1, b.y1, b.z1)
// The fittings that stand on the floor: the counter and desk, the columns,
// the seat rows, the playset. And the facade's outer half all round, since
// rooms wall only part of the footprint.
let fittingBlocks: List<Building.Block> =
  let h = CinemaLayout.counterHeight in
  let c = CinemaLayout.counter in
  let d = CinemaLayout.desk in
  List.flatten([
    [block("counter", c.x0, 0.0, c.z0, c.x1, h, c.z1), block("desk", d.x0, 0.0, d.z0, d.x1, h, d.z1)],
    CinemaLayout.columns |> List.indexedMap((k, col) =>
      let r = CinemaBuilding.innerOf(col.room) in
      let w = CinemaLayout.columnWidth / 2.0 in
      let dp = CinemaLayout.columnDepth in
      let tag = $"column-{k}" in
      match col.side with
      | South => block(tag, col.at - w, 0.0, r.z0, col.at + w, 3.0, r.z0 + dp)
      | North => block(tag, col.at - w, 0.0, r.z1 - dp, col.at + w, 3.0, r.z1)
      | West => block(tag, r.x0, 0.0, col.at - w, r.x0 + dp, 3.0, col.at + w)
      | East => block(tag, r.x1 - dp, 0.0, col.at - w, r.x1, 3.0, col.at + w)),
    CinemaLayout.auditoria |> List.concatMap((i) =>
      let r = CinemaBuilding.innerOf(i) in
      List.range(CinemaLayout.rows) |> List.map((k) =>
        let z = CinemaBuilding.rowZ(r, k) in
        let y = CinemaBuilding.treadY(k) in
        block($"seats-{i}-{k}", r.x0 + CinemaLayout.aisle, y, z + 0.3, r.x1 - CinemaLayout.aisle, y + 1.0, z + 0.92))),
    CinemaLayout.playset |> List.indexedMap((k, t) => block($"toy-{k}", t.x0, t.y0, t.z0, t.x1, t.y1, t.z1)),
    exterior.facadeBoxes |> List.indexedMap((k, b) => boxBlock($"facade-{k}", b)),
  ])

// The rake's ramp, one per auditorium: a slope through the rows' nosings
// from a row's depth before the first up to the top row's, then flat to the
// back wall, so the character walks it rather than hop row to row. Faces
// up (trimesh faces block only from the front).
let rampFaces = (i: float): List<Building.Face> =>
  let r = CinemaBuilding.innerOf(i) in
  let z0 = CinemaBuilding.rakeStart(r) - CinemaLayout.rowDepth in
  let z1 = CinemaBuilding.rowZ(r, CinemaLayout.rows - 1.0) in
  let top = CinemaBuilding.rakeTop in
  [Building.face(CinemaBuilding.carpet, v(r.x0, top, z1), v(r.x1 - r.x0, 0.0, 0.0), v(0.0, 0.0 - top, z0 - z1)),
   Building.floorFace(CinemaBuilding.carpet, r.x0, z1, r.x1, r.z1, top)]
let rampOf = (i: float): Building.Collider =>
  let faces = rampFaces(i) in
  { tag: $"rake-{i}", shape: Physics.trimesh({ positions: Building.positionsOf(faces), triangles: Building.quadTriangles(List.length(faces)) }),
    at: v(0.0, 0.0, 0.0) }

// The floors the character stands on: every up-facing carpet or tile face
// at floor level (the rake is its ramp).
let floorFaces = CinemaBuilding.interior.allFaces |> List.filter((f) =>
  (f.mat == CinemaBuilding.carpet || f.mat == CinemaBuilding.tile) && Vec3.y(Building.normalOf(f.q)) > 0.5 && Building.near(Vec3.y(f.q.o), 0.0))

// The playset's slide, its top face (as the ramp's).
let slideOf: Building.Collider =
  let s = CinemaLayout.slide in
  let faces = [Building.face(CinemaBuilding.carpet, v(s.x0, s.top, s.z1), v(s.x1 - s.x0, 0.05 - s.top, 0.0), v(0.0, 0.0, s.z0 - s.z1))] in
  { tag: "slide", shape: Physics.trimesh({ positions: Building.positionsOf(faces), triangles: Building.quadTriangles(1.0) }), at: v(0.0, 0.0, 0.0) }

// Every collider, in the cinema's space: the rooms' walls and the facade,
// the fittings, the floors, the rakes, the slide and the sign tower.
let colliders: List<Building.Collider> = List.flatten([
  Building.collidersOf(l, List.flatten([Building.blocksOf(l), fittingBlocks, [boxBlock("sign-tower", signTower)]]), floorFaces),
  CinemaLayout.auditoria |> List.map(rampOf),
  [slideOf],
])
