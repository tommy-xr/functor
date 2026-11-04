// The streetscape kit: the ground round every building. A sidewalk ring of
// paving slabs with a curb (15 cm over the road) and two kerb ramps at
// each corner; street trees in grates, hedges along the facade, planters; street
// furniture; fences; parking lots; signs and window dressing.
//
// Everything is boxes on the street side's instanced draws (facade.fun's
// `Trim`: `plain` boxes stay on shells, `fine` ones only the street side
// draws; the paving is one more finish), and decals: quads of one atlas
// (streetscape.mjs) in a static model per archetype, signs lit, glowing
// faces unlit by night, window dressing unlit at every hour. Built once per
// archetype, so every copy draws the same. No shadows; nothing is baked.
//
// The sidewalk has no collider: the walker's feet sink 13 cm into it, and
// every walk stays as flat as the road. What you'd bump into (benches,
// bins, bollards, planters, hedges, trunks, fences) has one, and keeps off
// `Plan.paths`.

open Layout

let box = Exterior.box
let rgb = Building.rgb
let noise = Building.noise
let v = Vec3.make

// The road (the ground plane), and the sidewalk's top: a 15 cm curb.
let road = Exterior.street - 0.02
let walk = road + 0.15
// The sidewalk runs `depth` out from the facade, the outer `curb` of it a
// curb; a kerb ramp is `ramp` long.
let depth = 4.0
let curb = 0.15
let ramp = 1.2

let curbGrey = rgb(150.0, 148.0, 140.0)
let paving = Finishes.on(Finishes.paving, rgb(146.0, 144.0, 138.0))
let planterGrey = rgb(140.0, 138.0, 130.0)
let hedgeGreens = [rgb(52.0, 74.0, 46.0), rgb(60.0, 80.0, 48.0), rgb(46.0, 68.0, 44.0)]
let leafGreens = [rgb(58.0, 86.0, 50.0), rgb(72.0, 94.0, 54.0), rgb(50.0, 74.0, 46.0)]
let bark = rgb(74.0, 62.0, 52.0)
let grateDark = rgb(46.0, 46.0, 48.0)
let steel = rgb(112.0, 114.0, 118.0)
let darkSteel = rgb(58.0, 60.0, 64.0)
let wood = rgb(116.0, 86.0, 58.0)
let binGreen = rgb(38.0, 60.0, 46.0)
let bollardGrey = rgb(128.0, 128.0, 124.0)
let bandYellow = rgb(196.0, 168.0, 58.0)
let vendingBlue = rgb(54.0, 64.0, 92.0)
let granite = rgb(52.0, 52.0, 56.0)
let wire = rgb(136.0, 140.0, 140.0)
let picketWhite = rgb(212.0, 208.0, 198.0)
let dumpsterGreen = rgb(44.0, 72.0, 56.0)
let asphalt = rgb(38.0, 36.0, 42.0)
let linePaint = rgb(196.0, 196.0, 186.0)
let stopGrey = rgb(118.0, 116.0, 110.0)
let dirt = rgb(82.0, 74.0, 62.0)
let weedGreen = rgb(70.0, 82.0, 48.0)

// One of `cs` by noise `r` in [0, 1).
let pick = (r: float, cs: List<Color.t>): Color.t =>
  List.nth(Math.floor(r * List.length(cs)), cs) |> Option.defaultValue(rgb(0.0, 0.0, 0.0))

// ---- Frames -----------------------------------------------------------------------

// A frame on the ground: a line facing `side`, through (x, z). A point in it
// is `u` along the line (x on south and north, z on west and east, as
// absolute coordinates) and `d` out from it, toward `side`.
type Frame = { side: Side, x: float, z: float }
// The frame on side `s`'s facade: its line the outer face.
let sideFrame = (l: Layout, s: Side): Frame =>
  let f = l.footprint in
  let o = l.facade / 2.0 in
  match s with
  | South => { side: South, x: 0.0, z: f.z0 - o }
  | North => { side: North, x: 0.0, z: f.z1 + o }
  | West => { side: West, x: f.x0 - o, z: 0.0 }
  | East => { side: East, x: f.x1 + o, z: 0.0 }
let opposite = (s: Side): Side =>
  match s with
  | South => North
  | North => South
  | West => East
  | East => West
// The frame facing back, its line `at` out.
let flipped = (fr: Frame, at: float): Frame =>
  match fr.side with
  | South => { side: North, x: fr.x, z: fr.z - at }
  | North => { side: South, x: fr.x, z: fr.z + at }
  | West => { side: East, x: fr.x - at, z: fr.z }
  | East => { side: West, x: fr.x + at, z: fr.z }
// [u0, u1] × [y0, y1] × [d0, d1] in frame `fr`.
let boxIn = (fr: Frame, u0: float, u1: float, y0: float, y1: float, d0: float, d1: float): Exterior.Box =>
  match fr.side with
  | South => box(fr.x + u0, y0, fr.z - d1, fr.x + u1, y1, fr.z - d0)
  | North => box(fr.x + u0, y0, fr.z + d0, fr.x + u1, y1, fr.z + d1)
  | West => box(fr.x - d1, y0, fr.z + u0, fr.x - d0, y1, fr.z + u1)
  | East => box(fr.x + d0, y0, fr.z + u0, fr.x + d1, y1, fr.z + u1)
// A quad over [u0, u1] × [y0, y1] at `d`, facing `side` (`out`) or back.
let faceIn = (fr: Frame, out: bool, u0: float, u1: float, y0: float, y1: float, d: float): Building.Quad =>
  let s = if out then fr.side else opposite(fr.side) in
  let (x, z) = match fr.side with
    | South => (0.0, fr.z - d)
    | North => (0.0, fr.z + d)
    | West => (fr.x - d, 0.0)
    | East => (fr.x + d, 0.0) in
  (match s with
    | South => Building.wallZ(0.0, fr.x + u0, fr.x + u1, y0, y1, z, false)
    | North => Building.wallZ(0.0, fr.x + u0, fr.x + u1, y0, y1, z, true)
    | West => Building.wallX(0.0, fr.z + u0, fr.z + u1, y0, y1, x, false)
    | East => Building.wallX(0.0, fr.z + u0, fr.z + u1, y0, y1, x, true)).q

// ---- Decals -----------------------------------------------------------------------

// A sign (lit), a glowing face (unlit by night, lit by day) or a pane
// (window dressing: unlit, its cell by `day` or `night`).
type DecalKind =
  | Sign
  | Glow
  | Pane
type Decal = { kind: DecalKind, day: StreetAtlas.Cell, night: StreetAtlas.Cell, q: Building.Quad }
let sign = (cell: StreetAtlas.Cell, q: Building.Quad): Decal => { kind: Sign, day: cell, night: cell, q: q }
let glow = (cell: StreetAtlas.Cell, q: Building.Quad): Decal => { kind: Glow, day: cell, night: cell, q: q }

let atlas = Asset.texture("street-atlas.png")
let rankOf = (k: DecalKind): float =>
  match k with
  | Sign => 0.0
  | Glow => 1.0
  | Pane => 2.0
let cellUvs = (c: StreetAtlas.Cell): List<float> => [c.u0, c.v1, c.u1, c.v1, c.u1, c.v0, c.u0, c.v0]
// Decals as one static model by night or day: the signs, the glowing faces
// (unlit by night), then the panes (unlit): a draw or two. `atlasModelOf`
// for another atlas's cells.
let modelOf = (ds: List<Decal>, night: bool): Scene.t => atlasModelOf(atlas, ds, night)
let atlasModelOf = (atlas: Asset.Texture, ds: List<Decal>, night: bool): Scene.t =>
  let sorted = ds |> List.sortBy((d) => rankOf(d.kind)) in
  let count = (k: DecalKind): float => 2.0 * List.length(ds |> List.filter((d) => d.kind == k)) in
  let signs = count(Sign) in
  let glows = count(Glow) in
  let panes = count(Pane) in
  let spans = if night then [(0.0, signs, false), (signs, glows + panes, true)] else [(0.0, signs + glows, false), (signs + glows, panes, true)] in
  let faces = sorted |> List.map((d) => { mat: 0.0, q: d.q }) in
  if List.isEmpty(ds) then Scene.group([])
  else Scene.modelOf(Model.static({ Building.meshOf(Building.materials, faces, []) with
    uvs: Bytes.fromF32le(sorted |> List.concatMap((d) => cellUvs(if night then d.night else d.day))),
    groups: spans |> List.filter((e) => let (first, n, unlit) = e in n > 0.0)
      |> List.map((e) => let (first, n, unlit) = e in { first: first, count: n, texture: Option.Some(atlas), unlit: unlit, emissive: 0.0 }) }))
let dressingOf = (ds: List<Decal>): Exterior.Grid => { night: modelOf(ds, true), day: modelOf(ds, false) }

// ---- Props ------------------------------------------------------------------------

// What a piece adds: boxes kept on shells (`plain`, `skinned`), boxes only
// the street side draws (`fine`: detail with no collider, so a shell never
// hides something solid), decals, and boxes you'd bump into.
type Props = {
  plain: List<(Exterior.Box, Color.t)>,
  fine: List<(Exterior.Box, Color.t)>,
  skinned: List<(Exterior.Box, Finishes.Surface)>,
  decals: List<Decal>,
  solid: List<Exterior.Box>,
}
let nothing: Props = { plain: [], fine: [], skinned: [], decals: [], solid: [] }
let joined = (ps: List<Props>): Props => {
  plain: ps |> List.concatMap((p) => p.plain),
  fine: ps |> List.concatMap((p) => p.fine),
  skinned: ps |> List.concatMap((p) => p.skinned),
  decals: ps |> List.concatMap((p) => p.decals),
  solid: ps |> List.concatMap((p) => p.solid),
}

// ---- The sidewalk -----------------------------------------------------------------

let inside = (r: Rect, x: float, z: float): bool => x > r.x0 && x < r.x1 && z > r.z0 && z < r.z1
let sorted = (xs: List<float>): List<float> =>
  xs |> List.sortBy((x) => x) |> List.fold((out, x) =>
    match List.last(out) with
    | Option.Some(y) => if x - y < 0.0001 then out else List.append([x], out)
    | Option.None => [x], [])
let valueAt = (i: float, xs: List<float>): float => List.nth(i, xs) |> Option.defaultValue(0.0)
// `r` less the `cuts`: the grid their edges cut it into, outside every cut,
// merged along z.
let less = (r: Rect, cuts: List<Rect>): List<Rect> =>
  let edges = (lo: float, hi: float, es: List<float>): List<float> =>
    sorted(List.flatten([[lo, hi], es |> List.filter((e) => e > lo && e < hi)])) in
  let xs = edges(r.x0, r.x1, cuts |> List.concatMap((c) => [c.x0, c.x1])) in
  let zs = edges(r.z0, r.z1, cuts |> List.concatMap((c) => [c.z0, c.z1])) in
  List.range(List.length(xs) - 1.0) |> List.concatMap((i) =>
    let x0 = valueAt(i, xs) in
    let x1 = valueAt(i + 1.0, xs) in
    List.range(List.length(zs) - 1.0) |> List.map((j) => (valueAt(j, zs), valueAt(j + 1.0, zs)))
      |> List.filter((e) => let (z0, z1) = e in not (cuts |> List.any((c) => inside(c, (x0 + x1) / 2.0, (z0 + z1) / 2.0))))
      |> List.fold((out, e) =>
        let (z0, z1) = e in
        match List.last(out) with
        | Option.Some(p) =>
          if Math.abs(p.z1 - z0) < 0.0001 then List.append([rect(x0, p.z0, x1, z1)], List.take(List.length(out) - 1.0, out))
          else List.append([rect(x0, z0, x1, z1)], out)
        | Option.None => [rect(x0, z0, x1, z1)], []))

// A kerb ramp: its rect, sloping down to the road on side `down`.
type Ramp = { r: Rect, down: Side }
// The ramp's surface, a tactile pad, from the road up to the sidewalk.
let rampQuad = (k: Ramp): Building.Quad =>
  let r = k.r in
  let low = Exterior.street + 0.005 in
  let h = walk - low in
  match k.down with
  | South => { o: v(r.x1, low, r.z0), a: v(r.x0 - r.x1, 0.0, 0.0), b: v(0.0, h, r.z1 - r.z0) }
  | North => { o: v(r.x0, low, r.z1), a: v(r.x1 - r.x0, 0.0, 0.0), b: v(0.0, h, r.z0 - r.z1) }
  | West => { o: v(r.x0, low, r.z0), a: v(0.0, 0.0, r.z1 - r.z0), b: v(r.x1 - r.x0, h, 0.0) }
  | East => { o: v(r.x1, low, r.z1), a: v(0.0, 0.0, r.z0 - r.z1), b: v(r.x0 - r.x1, h, 0.0) }

// The outer corners of the ring round `l`'s footprint, each with the way in.
let cornersOf = (l: Layout): List<(float, float, float, float)> =>
  let f = l.footprint in
  let a = l.facade / 2.0 + depth in
  [(f.x0 - a, f.z0 - a, 1.0, 1.0), (f.x1 + a, f.z0 - a, -1.0, 1.0), (f.x0 - a, f.z1 + a, 1.0, -1.0), (f.x1 + a, f.z1 + a, -1.0, -1.0)]
// Two ramps a corner, one down to each street, clear of each other: the
// crosswalks (roads.fun) land on them.
let rampSpan = (sign: float, at: float): (float, float) =>
  let a = at + sign * (ramp + 0.1) in
  let b = at + sign * (depth - 0.3) in
  (Math.min(a, b), Math.max(a, b))
let rampsOf = (l: Layout): List<Ramp> => cornersOf(l) |> List.concatMap((c) =>
  let (cx, cz, sx, sz) = c in
  let (ux0, ux1) = rampSpan(sx, cx) in
  let (uz0, uz1) = rampSpan(sz, cz) in
  [{ r: rect(ux0, Math.min(cz, cz + sz * ramp), ux1, Math.max(cz, cz + sz * ramp)), down: if sz > 0.0 then South else North },
   { r: rect(Math.min(cx, cx + sx * ramp), uz0, Math.max(cx, cx + sx * ramp), uz1), down: if sx > 0.0 then West else East }])

// The forecourt: the sidewalk cut back to the road in front of the
// entrance, 1.5 m past the door each side.
let forecourtOf = (l: Layout, st: Exterior.Street): Rect =>
  let e = st.entrance in
  let o = l.facade / 2.0 in
  rect(e.lo - 1.5, l.footprint.z0 - o - depth, e.hi + 1.5, l.footprint.z0 - o)

// The ring: paving slabs inside a curb, round the footprint (the corners in
// the south and north strips), less the forecourt and the ramps.
let sidewalkOf = (l: Layout, st: Exterior.Street): Props =>
  let f = l.footprint in
  let o = l.facade / 2.0 in
  let a = o + depth in
  let (x0, x1, z0, z1) = (f.x0 - a, f.x1 + a, f.z0 - a, f.z1 + a) in
  let c = curb in
  let ramps = rampsOf(l) in
  let holes = List.append(ramps |> List.map((k) => k.r), [forecourtOf(l, st)]) in
  let slabs = [rect(x0 + c, z0 + c, x1 - c, f.z0 - o), rect(x0 + c, f.z1 + o, x1 - c, z1 - c),
               rect(x0 + c, f.z0 - o, f.x0 - o, f.z1 + o), rect(f.x1 + o, f.z0 - o, x1 - c, f.z1 + o)] in
  let curbs = [rect(x0, z0, x1, z0 + c), rect(x0, z1 - c, x1, z1), rect(x0, z0 + c, x0 + c, z1 - c), rect(x1 - c, z0 + c, x1, z1 - c)] in
  let boxes = (rs: List<Rect>): List<Exterior.Box> =>
    rs |> List.concatMap((r) => less(r, holes)) |> List.map((r) => box(r.x0, road, r.z0, r.x1, walk, r.z1)) in
  { nothing with
    skinned: boxes(slabs) |> List.map((b) => (b, paving)),
    plain: boxes(curbs) |> List.map((b) => (b, curbGrey)),
    decals: ramps |> List.map((k) => sign(StreetAtlas.tactile, rampQuad(k))) }

// ---- Pieces -----------------------------------------------------------------------
//
// Each stands in frame `fr`, centred `u` along it, its back `d` out, on the
// ground at `y`, facing on out (+d).

// Boxes of a piece, relative to (u, d, y).
let partIn = (fr: Frame, u: float, d: float, y: float) =>
  (u0: float, u1: float, y0: float, y1: float, d0: float, d1: float): Exterior.Box =>
    boxIn(fr, u + u0, u + u1, y + y0, y + y1, d + d0, d + d1)

// A bench: steel legs, a slatted seat and back.
let bench = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.7, -0.62, 0.0, 0.84, 0.0, 0.42), darkSteel), (p(0.62, 0.7, 0.0, 0.84, 0.0, 0.42), darkSteel),
           (p(-0.85, 0.85, 0.4, 0.46, 0.04, 0.46), wood), (p(-0.85, 0.85, 0.54, 0.82, 0.02, 0.07), wood)],
    solid: [p(-0.85, 0.85, 0.0, 0.84, 0.0, 0.46)] }
// A litter bin with a lid.
let bin = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with plain: [(p(-0.25, 0.25, 0.0, 0.82, 0.0, 0.5), binGreen), (p(-0.27, 0.27, 0.82, 0.88, -0.02, 0.52), darkSteel)],
                 solid: [p(-0.27, 0.27, 0.0, 0.88, -0.02, 0.52)] }
// A concrete bollard with a reflective band.
let bollard = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with plain: [(p(-0.1, 0.1, 0.0, 0.9, 0.0, 0.2), bollardGrey), (p(-0.105, 0.105, 0.7, 0.78, -0.005, 0.205), bandYellow)],
                 solid: [p(-0.1, 0.1, 0.0, 0.9, 0.0, 0.2)] }
// A bike rack: three hoops across the frame, 0.8 m apart.
let bikeRack = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [-0.8, 0.0, 0.8] |> List.concatMap((k) => [(p(k - 0.025, k + 0.025, 0.0, 0.8, 0.0, 0.05), steel),
      (p(k - 0.025, k + 0.025, 0.0, 0.8, 0.6, 0.65), steel), (p(k - 0.025, k + 0.025, 0.75, 0.8, 0.0, 0.65), steel)]),
    solid: [p(-0.85, 0.85, 0.0, 0.8, 0.0, 0.65)] }
// A payphone on a post: a steel hood, the handset and keypad, a lit sign.
let payphone = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.05, 0.05, 0.0, 1.0, 0.1, 0.2), darkSteel), (p(-0.28, 0.28, 1.0, 1.85, 0.0, 0.36), steel),
           (p(-0.22, 0.22, 1.08, 1.7, 0.36, 0.38), darkSteel), (p(-0.16, -0.06, 1.3, 1.6, 0.38, 0.44), granite),
           (p(0.02, 0.16, 1.32, 1.5, 0.38, 0.4), steel)],
    decals: [glow(StreetAtlas.phone, faceIn(fr, true, u - 0.25, u + 0.25, y + 1.72, y + 1.84, d + 0.365))],
    solid: [p(-0.28, 0.28, 0.0, 1.85, 0.0, 0.44)] }
// A vending machine, its front lit at night.
let vendingMachine = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.48, 0.48, 0.0, 1.86, 0.0, 0.8), vendingBlue)],
    decals: [glow(StreetAtlas.vending, faceIn(fr, true, u - 0.4, u + 0.4, y + 0.12, y + 1.72, d + 0.805))],
    solid: [p(-0.48, 0.48, 0.0, 1.86, 0.0, 0.8)] }
// A tenant directory: a granite slab on a plinth, its face a decal.
let monolith = (fr: Frame, u: float, d: float, y: float, cell: StreetAtlas.Cell): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.5, 0.5, 0.0, 0.12, -0.05, 0.4), planterGrey), (p(-0.45, 0.45, 0.12, 1.95, 0.0, 0.32), granite)],
    decals: [sign(cell, faceIn(fr, true, u - 0.32, u + 0.32, y + 0.56, y + 1.84, d + 0.325))],
    solid: [p(-0.5, 0.5, 0.0, 1.95, -0.05, 0.4)] }
// A sign on a post, its plate facing out.
let poleSign = (fr: Frame, u: float, d: float, y: float, cell: StreetAtlas.Cell): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.03, 0.03, 0.0, 2.5, -0.03, 0.03), steel), (p(-0.25, 0.25, 1.88, 2.48, 0.03, 0.05), steel)],
    decals: [sign(cell, faceIn(fr, true, u - 0.24, u + 0.24, y + 1.9, y + 2.46, d + 0.055))],
    solid: [p(-0.05, 0.05, 0.0, 2.5, -0.05, 0.05)] }
// A concrete planter `len` long, a clipped hedge in it.
let planter = (fr: Frame, u: float, d: float, y: float, len: float, seed: float): Props =>
  let p = partIn(fr, u, d, y) in
  let g = pick(noise(seed), hedgeGreens) in
  let h = len / 2.0 in
  { nothing with
    plain: [(p(0.0 - h, h, 0.0, 0.5, 0.0, 0.6), planterGrey), (p(0.05 - h, h - 0.05, 0.5, 0.82, 0.05, 0.55), g),
            (p(0.15 - h, h - 0.15, 0.82, 0.9, 0.12, 0.48), g)],
    solid: [p(0.0 - h, h, 0.0, 0.9, 0.0, 0.6)] }
// A street tree in a grate: a trunk and a canopy of clipped boxes.
let tree = (fr: Frame, u: float, d: float, y: float, seed: float): Props =>
  let p = partIn(fr, u, d, y) in
  let g = pick(noise(seed), leafGreens) in
  let k = 0.85 + 0.3 * noise(seed + 5.0) in
  { nothing with
    plain: [(p(-0.6, 0.6, 0.0, 0.01, -0.6, 0.6), grateDark), (p(-0.09, 0.09, 0.0, 2.2 * k, -0.09, 0.09), bark),
            (p(-1.1, 1.1, 2.0 * k, 3.1 * k, -1.1, 1.1), g), (p(-0.75, 0.75, 3.1 * k, 3.8 * k, -0.75, 0.75), g),
            (p(-0.9, 0.3, 2.4 * k, 3.4 * k, 0.2, 1.3), g)],
    solid: [p(-0.12, 0.12, 0.0, 2.2, -0.12, 0.12)] }
// A hedge from `u0` to `u1`, its back `d` out.
let hedge = (fr: Frame, u0: float, u1: float, d: float, y: float, seed: float): Props =>
  let b = boxIn(fr, u0, u1, y, y + 0.72, d, d + 0.55) in
  { nothing with plain: [(b, pick(noise(seed), hedgeGreens))], solid: [b] }
// A dumpster: a green bin, its lid shut.
let dumpster = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.9, 0.9, 0.12, 1.15, 0.0, 1.0), dumpsterGreen), (p(-0.92, 0.92, 1.15, 1.22, -0.04, 1.02), rgb(34.0, 54.0, 44.0))],
    fine: [(p(-0.8, -0.6, 0.0, 0.12, 0.1, 0.3), darkSteel), (p(0.6, 0.8, 0.0, 0.12, 0.7, 0.9), darkSteel)],
    solid: [p(-0.92, 0.92, 0.0, 1.22, -0.04, 1.02)] }
// A shopping cart, abandoned: a wire basket, its handle and child seat, on
// four castors.
let cart = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  let t = 0.012 in
  { nothing with
    plain: List.flatten([
      [(-0.27, 0.0), (0.27, 0.0), (-0.27, 0.85), (0.27, 0.85)] |> List.concatMap((e) =>
        let (cu, cd) = e in [(p(cu - t, cu + t, 0.12, 0.95, cd - t, cd + t), wire), (p(cu - 0.04, cu + 0.04, 0.0, 0.1, cd - 0.04, cd + 0.04), granite)]),
      [(p(-0.28, 0.28, 0.94, 0.97, 0.0 - t, 0.85 + t), wire), (p(-0.28, 0.28, 0.45, 0.47, 0.0, 0.85), wire),
       (p(-0.28 - t, -0.28 + t, 0.45, 0.95, 0.0, 0.85), wire), (p(0.28 - t, 0.28 + t, 0.45, 0.95, 0.0, 0.85), wire),
       (p(-0.28, 0.28, 0.45, 0.95, 0.85, 0.85 + 2.0 * t), wire), (p(-0.28, 0.28, 0.15, 0.17, 0.05, 0.8), wire),
       (p(-0.3, 0.3, 1.02, 1.06, 0.95, 0.99), rgb(150.0, 40.0, 36.0)), (p(-0.29, -0.27, 0.95, 1.04, 0.85, 0.97), wire),
       (p(0.27, 0.29, 0.95, 1.04, 0.85, 0.97), wire)],
    ]),
    solid: [p(-0.3, 0.3, 0.0, 1.06, 0.0, 1.0)] }

// ---- Fences -----------------------------------------------------------------------

// Chain-link from `u0` to `u1` at `d`, `h` tall: posts every 3 m or less, a
// top rail, and a wire mesh 0.2 m square (real geometry, no cutout).
let chainLink = (fr: Frame, u0: float, u1: float, d: float, y: float, h: float): Props =>
  let n = Math.ceil((u1 - u0) / 3.0) in
  let w = 0.006 in
  let m = Math.floor((u1 - u0) / 0.2) in
  { nothing with
    plain: List.flatten([
      List.range(n + 1.0) |> List.map((k) => let u = u0 + (u1 - u0) * k / n in
        (boxIn(fr, u - 0.03, u + 0.03, y, y + h, d - 0.03, d + 0.03), wire)),
      [(boxIn(fr, u0, u1, y + h - 0.05, y + h, d - 0.025, d + 0.025), wire)],
    ]),
    fine: List.flatten([
      List.range(m) |> List.map((k) => let u = u0 + (u1 - u0) * (k + 0.5) / m in
        (boxIn(fr, u - w, u + w, y + 0.05, y + h - 0.05, d - w, d + w), wire)),
      List.range(Math.floor(h / 0.2) - 1.0) |> List.map((k) => let yy = y + 0.2 * (k + 1.0) in
        (boxIn(fr, u0, u1, yy - w, yy + w, d - w, d + w), wire)),
    ]),
    solid: [boxIn(fr, u0, u1, y, y + h, d - 0.05, d + 0.05)] }
// A highway guardrail: posts every 2 m, a galvanised beam on blocks.
let guardrail = (fr: Frame, u0: float, u1: float, d: float, y: float): Props =>
  let n = Math.ceil((u1 - u0) / 2.0) in
  { nothing with
    plain: List.flatten([
      List.range(n + 1.0) |> List.map((k) => let u = u0 + (u1 - u0) * k / n in
        (boxIn(fr, u - 0.05, u + 0.05, y, y + 0.72, d, d + 0.1), darkSteel)),
      [(boxIn(fr, u0, u1, y + 0.44, y + 0.76, d - 0.06, d - 0.02), steel),
       (boxIn(fr, u0, u1, y + 0.56, y + 0.64, d - 0.08, d - 0.06), steel)],
    ]),
    solid: [boxIn(fr, u0, u1, y, y + 0.76, d - 0.08, d + 0.1)] }
// A white picket fence, 1 m tall: pickets every 0.15 m on two rails.
let picket = (fr: Frame, u0: float, u1: float, d: float, y: float): Props =>
  let n = Math.floor((u1 - u0) / 0.15) in
  { nothing with
    plain: List.flatten([
      List.range(n) |> List.concatMap((k) => let u = u0 + (u1 - u0) * (k + 0.5) / n in
        [(boxIn(fr, u - 0.04, u + 0.04, y, y + 0.94, d, d + 0.025), picketWhite),
         (boxIn(fr, u - 0.02, u + 0.02, y + 0.94, y + 1.0, d, d + 0.025), picketWhite)]),
      [0.25, 0.75] |> List.map((h) => (boxIn(fr, u0, u1, y + h, y + h + 0.07, d - 0.03, d), picketWhite)),
    ]),
    solid: [boxIn(fr, u0, u1, y, y + 1.0, d - 0.03, d + 0.025)] }

// ---- Window dressing ---------------------------------------------------------------

// Noise for a window: `kind` picks what it's for, `k` the window.
let windowNoise = (kind: float, k: float): float => noise(k * 0.731 + kind * 1013.0)
// What hangs in a window by day, by its noise: blinds at one of three
// heights, vertical blinds, a sheet of paper taped up, or nothing; FOR
// LEASE when `lease`. Each in a band's cells or a bay's.
type Panes = { dark: StreetAtlas.Cell, blinds1: StreetAtlas.Cell, blinds2: StreetAtlas.Cell, blinds3: StreetAtlas.Cell, verticals: StreetAtlas.Cell,
               paper: StreetAtlas.Cell, lease: StreetAtlas.Cell, darkNight: StreetAtlas.Cell, blindsNight: StreetAtlas.Cell,
               verticalsNight: StreetAtlas.Cell, leaseNight: StreetAtlas.Cell, litWarm: StreetAtlas.Cell, litCool: StreetAtlas.Cell }
let bandPanes: Panes = { dark: StreetAtlas.bandDark, blinds1: StreetAtlas.bandBlinds1, blinds2: StreetAtlas.bandBlinds2, blinds3: StreetAtlas.bandBlinds3,
  verticals: StreetAtlas.bandVerticals, paper: StreetAtlas.bandPaper, lease: StreetAtlas.bandLease, darkNight: StreetAtlas.bandDarkNight,
  blindsNight: StreetAtlas.bandBlindsNight, verticalsNight: StreetAtlas.bandVerticalsNight, leaseNight: StreetAtlas.bandLeaseNight,
  litWarm: StreetAtlas.bandLitWarm, litCool: StreetAtlas.bandLitCool }
let bayPanes: Panes = { dark: StreetAtlas.bayDark, blinds1: StreetAtlas.bayBlinds1, blinds2: StreetAtlas.bayBlinds2, blinds3: StreetAtlas.bayBlinds3,
  verticals: StreetAtlas.bayVerticals, paper: StreetAtlas.bayPaper, lease: StreetAtlas.bayLease, darkNight: StreetAtlas.bayDarkNight,
  blindsNight: StreetAtlas.bayBlindsNight, verticalsNight: StreetAtlas.bayVerticalsNight, leaseNight: StreetAtlas.bayLeaseNight,
  litWarm: StreetAtlas.bayLitWarm, litCool: StreetAtlas.bayLitCool }
// The share of windows lit at night: sparse, a late office or two.
let litShare = 0.07
let isLit = (k: float): bool => windowNoise(3.0, k) < litShare
// Window `k`'s cells by day and by night; FOR LEASE if `lease`.
let panesOf = (p: Panes, k: float, lease: bool): (StreetAtlas.Cell, StreetAtlas.Cell) =>
  let r = windowNoise(1.0, k) in
  let (day, night) =
    if lease then (p.lease, p.leaseNight)
    else if r < 0.2 then (p.dark, p.darkNight)
    else if r < 0.4 then (p.blinds1, p.blindsNight)
    else if r < 0.58 then (p.blinds2, p.blindsNight)
    else if r < 0.72 then (p.blinds3, p.blindsNight)
    else if r < 0.88 then (p.verticals, p.verticalsNight)
    else (p.paper, p.blindsNight) in
  // A vacant suite stays dark.
  (day, if isLit(k) && not lease then (if windowNoise(4.0, k) < 0.5 then p.litWarm else p.litCool) else night)
let pane = (p: Panes, k: float, lease: bool, q: Building.Quad): Decal =>
  let (day, night) = panesOf(p, k, lease) in { kind: Pane, day: day, night: night, q: q }

// A band's id, for its noise: unique per level, side and place.
let sideId = (s: Side): float =>
  match s with
  | South => 0.0
  | West => 1.0
  | North => 2.0
  | East => 3.0
let bandId = (level: float, s: Side, at: float): float => level * 97.0 + sideId(s) * 23.0 + at * 3.1
// Whether a band is lit at night: the shells light the same ones.
let isLitBand = (level: float, s: Side, at: float): bool => isLit(bandId(level, s, at))
// FOR LEASE in some ground-floor bands.
let isLeased = (level: float, s: Side, at: float): bool => level == 0.0 && windowNoise(2.0, bandId(level, s, at)) < 0.15
// Every dark band's dressing, 2 cm before its recessed panel (6 cm in from
// the facade's face, behind the mullions).
let bandPanesOf = (l: Layout): List<Decal> =>
  Building.levelsOf(l) |> List.concatMap((level) => [South, North, West, East] |> List.concatMap((s) =>
    let fr = sideFrame(l, s) in
    let y = level * l.storey in
    Exterior.bandHoles(l, level, s) |> List.map((h) =>
      let at = (h.lo + h.hi) / 2.0 in
      pane(bandPanes, bandId(level, s, at), isLeased(level, s, at),
           faceIn(fr, true, h.lo, h.hi, y + h.bottom, y + h.top, 0.02 - Exterior.recess)))))

// ---- Parking lots ------------------------------------------------------------------

// A row of bays across the lot, from `d0` to `d1`, the cars' noses (and
// wheel stops) at `head`.
type Row = { d0: float, d1: float, head: float }
// A lot in frame `fr` (u along its rows): asphalt over `areas`, `n` bays
// `bay` wide from `first` in each row, raised planted `islands`, footings
// under the lamp posts at `poles` ((u, d)), and a cart left at `cart`.
type Lot = { fr: Frame, areas: List<Rect>, first: float, n: float, bay: float, rows: List<Row>, islands: List<Rect>,
             poles: List<(float, float)>, cart: Option.t<(float, float)>, seed: float }
// Rects in lot frames are u (x) by d (z).
let lotBox = (fr: Frame, r: Rect, y0: float, y1: float): Exterior.Box => boxIn(fr, r.x0, r.x1, y0, y1, r.z0, r.z1)
let lotOf = (lot: Lot): Props =>
  let fr = lot.fr in
  let line = 0.06 in
  let top = Exterior.street in
  let paint = top + 0.01 in
  let last = lot.first + lot.n * lot.bay in
  joined(List.flatten([
    [{ nothing with plain: lot.areas |> List.map((r) => (lotBox(fr, r, road, top), asphalt)) }],
    lot.rows |> List.indexedMap((k, row) =>
      let stop = if row.head == row.d0 then (row.d0 + 0.5, row.d0 + 0.65) else (row.d1 - 0.65, row.d1 - 0.5) in
      let (s0, s1) = stop in
      { nothing with plain: List.flatten([
        List.range(lot.n + 1.0) |> List.map((j) => let u = lot.first + j * lot.bay in
          (boxIn(fr, u - line, u + line, top, paint, row.d0, row.d1), linePaint)),
        // Back-to-back rows share their head line: drawn once.
        if List.take(k, lot.rows) |> List.any((r) => r.head == row.head) then []
        else [(boxIn(fr, lot.first - line, last + line, top, paint, row.head - line, row.head + line), linePaint)],
        List.range(lot.n) |> List.map((j) => let u = lot.first + (j + 0.5) * lot.bay in
          (boxIn(fr, u - 0.8, u + 0.8, top, top + 0.12, s0, s1), stopGrey)),
      ]) }),
    // An island: a curb round a bed of hedge.
    lot.islands |> List.indexedMap((k, r) =>
      let g = pick(noise(lot.seed + k * 7.0), hedgeGreens) in
      let bed = boxIn(fr, r.x0 + 0.25, r.x1 - 0.25, walk, walk + 0.55, r.z0 + 0.25, r.z1 - 0.25) in
      { nothing with plain: [(lotBox(fr, r, road, walk), curbGrey), (bed, g)], solid: [bed] }),
    lot.poles |> List.map((e) => let (u, d) = e in
      { nothing with plain: [(boxIn(fr, u - 0.22, u + 0.22, road, road + 0.5, d - 0.22, d + 0.22), curbGrey)] }),
    lot.cart |> Option.map((e) => let (u, d) = e in cart(fr, u, d, top)) |> Option.toList,
  ]))

// ---- Along the sidewalk ------------------------------------------------------------

// A walk that must stay clear, (x0, z0) to (x1, z1) in the building's space:
// the e2e's routes. Nothing solid comes within `clearance` of one.
type Path = { x0: float, z0: float, x1: float, z1: float }
let clearance = 0.45
let blocks = (b: Exterior.Box, p: Path): bool =>
  let n = Math.ceil(Math.sqrt((p.x1 - p.x0) * (p.x1 - p.x0) + (p.z1 - p.z0) * (p.z1 - p.z0)) / 0.1) + 1.0 in
  List.range(n + 1.0) |> List.any((k) =>
    let t = k / n in
    let x = p.x0 + (p.x1 - p.x0) * t in
    let z = p.z0 + (p.z1 - p.z0) * t in
    x > b.x0 - clearance && x < b.x1 + clearance && z > b.z0 - clearance && z < b.z1 + clearance)
let overlaps = (a: Exterior.Box, b: Exterior.Box): bool => a.x0 < b.x1 && b.x0 < a.x1 && a.z0 < b.z1 && b.z0 < a.z1
// `b` grown by `m` round its footprint.
let grown = (m: float, b: Exterior.Box): Exterior.Box => { b with x0: b.x0 - m, z0: b.z0 - m, x1: b.x1 + m, z1: b.z1 + m }
// A piece, if nothing solid of it blocks `paths` or comes within `m` of
// `taken`.
let fits = (paths: List<Path>, taken: List<Exterior.Box>, m: float, p: Props): bool =>
  p.solid |> List.all((b) => not (paths |> List.any((q) => blocks(b, q))) && not (taken |> List.any((t) => overlaps(grown(m, b), t))))

// An archetype's dressing: `seed` for its noise, the walks to keep clear,
// its own pieces (placed first, as given), and boxes to keep off that
// aren't the dressing's (another building's massing).
type Plan = { seed: float, paths: List<Path>, own: List<Props>, avoid: List<Exterior.Box> }

// Along side `s`: a tree in a grate every 8 m in the curbside strip, 2.8 m
// out (clear of the forecourt and the lamp posts), and between two trees a
// bench facing the building or a bin, by noise.
let curbside = (l: Layout, st: Exterior.Street, plan: Plan, s: Side): List<Props> =>
  let fr = sideFrame(l, s) in
  let (s0, s1) = Facade.spanOf(l, s) in
  let n = Math.floor((s1 - s0 - 8.0) / 8.0) + 1.0 in
  let start = (s0 + s1) / 2.0 - 4.0 * (n - 1.0) in
  let forecourt = forecourtOf(l, st) in
  // The lamp posts standing on this side's sidewalk, by u.
  let lampUs = l.streetLamps |> List.map((p) => (Vec3.x(p), Vec3.z(p)))
    |> List.filter((e) => let (x, z) = e in overlaps(box(x, 0.0, z, x, 1.0, z), boxIn(fr, s0, s1, 0.0, 1.0, 0.0, depth)))
    |> List.map((e) => let (x, z) = e in if s == South || s == North then x else z) in
  let nearLamp = (u: float): bool => lampUs |> List.any((a) => Math.abs(a - u) < 1.8) in
  // Nor under the canopy.
  let k = st.canopyBox in
  let atForecourt = (u: float): bool => s == South && ((u > forecourt.x0 - 2.0 && u < forecourt.x1 + 2.0) || (u > k.x0 - 1.5 && u < k.x1 + 1.5)) in
  let seed = plan.seed * 31.0 + sideId(s) * 7.0 in
  List.flatten([
    List.range(n) |> List.map((k) => start + 8.0 * k) |> List.filter((u) => not (nearLamp(u) || atForecourt(u)))
      |> List.map((u) => tree(fr, u, 2.8, walk, seed + u)),
    List.range(Math.max(0.0, n - 1.0)) |> List.map((k) => start + 8.0 * k + 4.0) |> List.filter((u) => not (nearLamp(u) || atForecourt(u)))
      |> List.map((u) =>
        let r = noise(seed + u * 3.0) in
        if r < 0.35 then bench(flipped(fr, 3.25), u, 0.0, walk)
        else if r < 0.6 then bin(flipped(fr, 3.25), u, 0.0, walk)
        else nothing),
  ])
// Hedges along side `s`'s facade, 0.15 m out, low under the windows: broken
// round every opening that starts below their top (doors, real windows; not
// the bands), round the forecourt, `taken` and the walks.
let hedgesOn = (l: Layout, st: Exterior.Street, plan: Plan, taken: List<Exterior.Box>, s: Side): List<Props> =>
  let fr = sideFrame(l, s) in
  let (s0, s1) = Facade.spanOf(l, s) in
  let forecourt = forecourtOf(l, st) in
  let zone = boxIn(fr, s0, s1, 0.0, 1.0, 0.1, 0.75) in
  let asHole = (b: Exterior.Box): Building.Hole =>
    if s == South || s == North then { lo: b.x0, hi: b.x1, bottom: -1.0, top: 9.0 } else { lo: b.z0, hi: b.z1, bottom: -1.0, top: 9.0 } in
  let walks = plan.paths |> List.map((p) =>
    box(Math.min(p.x0, p.x1) - clearance, 0.0, Math.min(p.z0, p.z1) - clearance, Math.max(p.x0, p.x1) + clearance, 1.0, Math.max(p.z0, p.z1) + clearance)) in
  let holes = List.flatten([
    Facade.holesOn(l, st, s) |> List.filter((h) => h.bottom < 0.95),
    if s == South then [{ lo: forecourt.x0, hi: forecourt.x1, bottom: -1.0, top: 9.0 }] else [],
    List.flatten([taken, walks]) |> List.filter((b) => overlaps(b, zone)) |> List.map(asHole),
  ]) in
  Facade.runs(s0 + 0.3, s1 - 0.3, Exterior.street, 0.95, holes, 0.4)
    |> List.map((r) => let (u0, u1) = r in hedge(fr, u0, u1, 0.15, walk, plan.seed + u0))

// The archetype's own pieces, then hedges, trees and furniture round its
// sides where they fit, then the sidewalk under them all.
let along = (l: Layout, st: Exterior.Street, plan: Plan): Props =>
  let own = joined(plan.own) in
  let taken = List.append(plan.avoid, own.solid) in
  // A tree keeps half a metre (its grate) from the pieces; hedges are
  // already broken round them.
  let trees = Portals.sides |> List.concatMap((s) => curbside(l, st, plan, s)) |> List.filter((p) => fits(plan.paths, taken, 0.5, p)) in
  let hedges = Portals.sides |> List.concatMap((s) => hedgesOn(l, st, plan, taken, s)) |> List.filter((p) => fits(plan.paths, taken, 0.0, p)) in
  joined(List.flatten([[own, sidewalkOf(l, st)], trees, hedges]))

// The street side dressed by `p`: its boxes on the street side's draws, its
// decals one static model, its solid boxes colliders beside the lamp
// posts' (each copy's, block.fun).
let collidersOf = (p: Props): List<Building.Collider> => p.solid |> List.indexedMap((k, b) =>
  { tag: $"prop-{k}", shape: Physics.box(b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0), at: v((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) })
let dressed = (st: Exterior.Street, p: Props): Exterior.Street =>
  let t = Facade.trimmed(st, { plain: p.plain, skinned: p.skinned, fine: p.fine }) in
  // A house's gables draw beside its side.
  let side = if List.isEmpty(st.gables) then t.streetSide else Scene.group([t.streetSide, Exterior.gablesOf(st.gables)]) in
  { t with streetSide: side, dressing: dressingOf(p.decals), lampColliders: List.flatten([t.lampColliders, collidersOf(p)]) }

// ---- Shared dressing -----------------------------------------------------------------

// A lit sign box on a facade: a dark cabinet `d` deep over [u0, u1] × [y0, y1],
// its face glowing at night (some letters dead).
let boxSign = (fr: Frame, u0: float, u1: float, y0: float, y1: float, d: float, cell: StreetAtlas.Cell): Props =>
  { nothing with plain: [(boxIn(fr, u0, u1, y0, y1, 0.0, d), darkSteel)],
                 decals: [glow(cell, faceIn(fr, true, u0 + 0.04, u1 - 0.04, y0 + 0.04, y1 - 0.04, d + 0.005))] }
// A sign flat on a facade (a plaque, a number), 5 mm proud of `d`.
let plaqueOn = (fr: Frame, u0: float, u1: float, y0: float, y1: float, d: float, cell: StreetAtlas.Cell): Props =>
  { nothing with decals: [sign(cell, faceIn(fr, true, u0, u1, y0, y1, d + 0.005))] }
// The street number on the canopy's fascia, centred over the door.
let canopyNumber = (l: Layout, st: Exterior.Street, cell: StreetAtlas.Cell): Props =>
  let k = st.canopyBox in
  let u = (st.entrance.lo + st.entrance.hi) / 2.0 in
  plaqueOn(sideFrame(l, South), u - 0.15, u + 0.15, k.y0 - 0.13, k.y0 - 0.02, (l.footprint.z0 - l.facade / 2.0) - k.z0, cell)
// Row markers on a lamp post at (u, d): `front` facing out, `back` in.
let postMarkers = (fr: Frame, u: float, d: float, y: float, front: StreetAtlas.Cell, back: StreetAtlas.Cell): List<Decal> =>
  [sign(front, faceIn(fr, true, u - 0.15, u + 0.15, y + 2.8, y + 3.1, d + 0.065)),
   sign(back, faceIn(fr, false, u - 0.15, u + 0.15, y + 2.8, y + 3.1, d - 0.065))]
// A lamp post whose lamp is out: a post, a dark head and a row marker.
let deadLamp = (fr: Frame, u: float, d: float, y: float, cell: StreetAtlas.Cell): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.06, 0.06, 0.0, 4.9, -0.06, 0.06), Exterior.postGrey), (p(-0.12, 0.12, 4.75, 4.85, -0.25, 0.25), darkSteel)],
    decals: postMarkers(fr, u, d, y, cell, cell),
    solid: [p(-0.06, 0.06, 0.0, 4.9, -0.06, 0.06)] }

// ---- The office ----------------------------------------------------------------------

// The e2e's walks round an office copy, in its space: the apron into the
// lobby, to the facade beside the entrance, and up the west yard to office
// 103's window.
let officePaths: List<Path> = [
  { x0: 10.0, z0: -11.5, x1: 10.0, z1: 3.0 },
  { x0: 10.0, z0: -6.0, x1: 6.5, z1: -3.0 },
  { x0: 6.5, z0: -3.0, x1: 6.5, z1: -0.4 },
  { x0: -4.0, z0: -6.0, x1: -4.0, z1: 9.3 },
  { x0: -4.0, z0: 9.3, x1: -0.3, z1: 9.3 },
]
// The office: by the entrance a directory, planters, a bin, a bench, a
// bike rack, suite plaques, a lit sign over the canopy and the street
// number on it; a side lot east of it (head-in bays off the street, a dead
// lamp), with a vending machine, a payphone and bollards along its
// sidewalk; a fenced dumpster at the back; and the window bands dressed.
let officeProps = (l: Layout, st: Exterior.Street): Props =>
  let south = sideFrame(l, South) in
  let east = sideFrame(l, East) in
  let north = sideFrame(l, North) in
  let back = l.footprint.z1 + l.facade / 2.0 in
  let own = [
    monolith(south, 14.9, 1.05, walk, StreetAtlas.directory),
    planter(south, 4.0, 0.2, walk, 1.6, 1.0),
    planter(south, 13.4, 0.2, walk, 1.6, 2.0),
    bin(flipped(south, 3.25), 12.9, 0.0, walk),
    bench(flipped(south, 3.25), 17.8, 0.0, walk),
    bikeRack(south, 1.6, 1.2, walk),
    plaqueOn(south, 12.3, 13.5, 1.55, 1.71, 0.0, StreetAtlas.suite100),
    plaqueOn(south, 12.3, 13.5, 1.33, 1.49, 0.0, StreetAtlas.suite200),
    boxSign(south, 8.4, 11.6, 3.75, 4.25, 0.22, StreetAtlas.boxOffice),
    canopyNumber(l, st, StreetAtlas.number1200),
    vendingMachine(east, 4.2, 0.15, walk),
    payphone(east, 7.6, 0.15, walk),
    bin(east, 5.4, 0.2, walk),
    joined([3.0, 5.6, 8.2] |> List.map((u) => bollard(east, u, 3.55, walk))),
    poleSign(east, 1.0, 3.5, walk, StreetAtlas.visitor),
    poleSign(east, 28.0, 3.5, walk, StreetAtlas.fireLane),
    lotOf({ fr: east, areas: [rect(0.0, depth, 29.0, 8.85)], first: 1.5, n: 10.0, bay: 2.6,
            rows: [{ d0: depth, d1: 8.85, head: depth }], islands: [], poles: [(14.5, 8.45)], cart: Option.None, seed: 3.0 }),
    deadLamp(east, 14.5, 8.45, road, StreetAtlas.rowP),
    dumpster(north, 14.5, 0.5, walk),
    chainLink(north, 12.5, 13.8, 2.9, walk, 1.8),
    chainLink(north, 15.4, 16.5, 2.9, walk, 1.8),
    chainLink({ side: East, x: 12.5, z: 0.0 }, back + 0.2, back + 2.9, 0.0, walk, 1.8),
    chainLink({ side: East, x: 16.5, z: 0.0 }, back + 0.2, back + 2.9, 0.0, walk, 1.8),
    { nothing with decals: bandPanesOf(l) },
  ] in
  along(l, st, { seed: 1.0, paths: officePaths, own: own, avoid: [] })
let office = (l: Layout, st: Exterior.Street): Exterior.Street => dressed(st, officeProps(l, st))

// ---- The towers -----------------------------------------------------------------------

// A tower: by the entrance a floor directory, planters, a bench and a bin,
// a bike rack, bollards across the forecourt, a lobby plaque and the street
// number; a vending machine, a payphone and a bin along its west yard.
// The towers e2e's walk from the crossroads to the apron, in a tower's space.
let towerPaths: List<Path> = [{ x0: -12.0, z0: -12.0, x1: 12.0, z1: -6.0 }]
let towerProps = (l: Layout, st: Exterior.Street): Props =>
  let south = sideFrame(l, South) in
  let west = sideFrame(l, West) in
  let own = [
    monolith(south, 17.0, 1.05, walk, StreetAtlas.directoryTower),
    planter(south, 8.4, 0.2, walk, 1.6, 4.0),
    planter(south, 15.4, 0.2, walk, 1.6, 5.0),
    bench(flipped(south, 3.25), 20.0, 0.0, walk),
    bin(flipped(south, 3.25), 4.6, 0.0, walk),
    bikeRack(south, 1.4, 1.2, walk),
    // On the forecourt, cut down to the road.
    joined([10.0, 14.0] |> List.map((u) => bollard(south, u, 3.55, Exterior.street))),
    plaqueOn(south, 13.7, 14.3, 1.55, 1.71, 0.0, StreetAtlas.lobby),
    canopyNumber(l, st, StreetAtlas.number400),
    vendingMachine(west, 4.0, 0.15, walk),
    payphone(west, 6.0, 0.15, walk),
    bin(west, 7.3, 0.2, walk),
  ] in
  along(l, st, { seed: 2.0, paths: towerPaths, own: own, avoid: [] })
let tower = (l: Layout, st: Exterior.Street): Exterior.Street => dressed(st, towerProps(l, st))

// ---- The houses --------------------------------------------------------------------

// A mailbox on a post, its door to the road (+d), its red flag up.
let mailbox = (fr: Frame, u: float, d: float, y: float): Props =>
  let p = partIn(fr, u, d, y) in
  { nothing with
    plain: [(p(-0.04, 0.04, 0.0, 1.0, -0.04, 0.04), wood), (p(-0.12, 0.12, 1.0, 1.24, -0.28, 0.24), darkSteel),
            (p(0.12, 0.14, 1.12, 1.4, -0.2, -0.16), rgb(170.0, 40.0, 34.0))],
    solid: [p(-0.12, 0.14, 0.0, 1.4, -0.28, 0.24)] }

// The e2e's walk from the road up the front path, in a house's space.
let housePaths: List<Path> = [{ x0: 1.6, z0: -9.5, x1: 1.6, z1: 0.7 }]
// By a house's front path, on the lawn strip along the road (roads.fun lays
// the strip for the whole suburb): a street tree and a mailbox; and a picket
// fence down its east side to the back yard, the one the next house along
// shares.
let houseProps = (l: Layout): Props =>
  let f = l.footprint in
  let g = HouseExterior.gateOf(l) in
  let edge = f.z0 - HouseExterior.road in
  let y = Exterior.street in
  let front: Frame = { side: South, x: 0.0, z: 0.0 } in
  joined([
    tree(front, f.x0 + 7.0, 0.0 - edge - 0.8, y, f.x1),
    mailbox(front, g.hi + 0.7, 0.0 - edge - 0.45, y),
    picket({ side: East, x: f.x1 + HouseExterior.side, z: 0.0 }, HouseExterior.fenceZ(l), f.z1 + 2.0, 0.0, y),
  ])
let house = (l: Layout, st: Exterior.Street): Exterior.Street => dressed(st, houseProps(l))
