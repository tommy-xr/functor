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

// The parking lot: asphalt either side of the apron and south of it, white
// bays in four rows (stall lines and an end line each), the middle two
// back to back across the lamps' island; the lamps are the layout's street
// lamps.
let asphalt = Building.rgb(38.0, 36.0, 42.0)
let paintWhite = Building.rgb(196.0, 196.0, 186.0)
let lotBoxes = [
  box(0.0 - 10.0, street - 0.02, 0.0 - 50.0, 70.0, street, 0.0 - 12.0),
  box(0.0 - 10.0, street - 0.02, 0.0 - 12.0, 15.0, street, front),
  box(45.0, street - 0.02, 0.0 - 12.0, 70.0, street, front),
]
// A row of bays from z0 to z1 (5 m deep), its end line at `end`.
let bayRow = (z0: float, z1: float, end: float): List<Exterior.Box> =>
  List.flatten([
    List.range(24.0) |> List.map((j) => let x = 0.0 + 2.6 * j in box(x - 0.06, street, z0, x + 0.06, street + 0.01, z1)),
    [box(0.0 - 0.06, street, end - 0.06, 59.86, street + 0.01, end + 0.06)],
  ])
let bayLines = List.flatten([
  bayRow(0.0 - 18.0, 0.0 - 13.0, 0.0 - 13.0),
  bayRow(0.0 - 28.0, 0.0 - 23.0, 0.0 - 28.0),
  bayRow(0.0 - 35.0, 0.0 - 30.0, 0.0 - 30.0),
  bayRow(0.0 - 45.0, 0.0 - 40.0, 0.0 - 45.0),
])

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
// along the marquee, the roofline and up the sign tower; the tower, lot,
// bays and poster frames among the lit boxes; the posters and pink as its
// grid. The facade's outer half (the kit's) is solid all round the
// footprint.
let exterior: Exterior.Street =
  let s = Exterior.streetWith(l, stuccoWall) in
  let extras = List.flatten([
    [(signTower, towerDark)],
    posterFrames |> List.map((b) => (b, Building.rgb(14.0, 12.0, 16.0))),
    lotBoxes |> List.map((b) => (b, asphalt)),
    bayLines |> List.map((b) => (b, paintWhite)),
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
                     grid: Option.Some({ night: glowAt(1.0), day: glowAt(0.6) }) } in
  dressed(lot)

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
