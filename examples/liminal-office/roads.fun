// The towns' road paint, drawn with the ground (game.fun): a zebra
// crosswalk over each arm of every crossroads, landing on the corners' kerb
// ramps (streetscape.fun), and a dashed centre line down each street; the
// suburb's frontage. A lit instanced cube per town (and the suburb's
// paving): a draw or two.

let box = Exterior.box
let paintWhite = Building.rgb(196.0, 196.0, 186.0)
let laneYellow = Building.rgb(190.0, 156.0, 58.0)
let y0 = Streetscape.road
let y1 = Streetscape.road + 0.01

// Dashes 3 m long, 3 m apart, along x (`alongX`) or z at `at`, from `a` to `b`.
let dashes = (alongX: bool, at: float, a: float, b: float): List<(Exterior.Box, Color.t)> =>
  List.range(Math.max(0.0, Math.floor((b - a) / 6.0))) |> List.map((k) =>
    let u = a + 1.5 + 6.0 * k in
    if alongX then (box(u, y0, at - 0.06, u + 3.0, y1, at + 0.06), laneYellow) else (box(at - 0.06, y0, u, at + 0.06, y1, u + 3.0), laneYellow))

// The crossroads at (cx, cz) of streets `w` wide between buildings whose
// facades stand `o` outside their footprints: each arm's crosswalk over its
// mouth, bars 0.5 m wide about a metre apart across it, between the
// corners' ramps; its centre line out to `reach` from the crossroads. (The
// grids' streets between blocks get only their centre lines, unbroken.)
let crossroads = (cx: float, cz: float, w: float, o: float, reach: float): List<(Exterior.Box, Color.t)> =>
  let h = w / 2.0 in
  let a = o + Streetscape.depth in
  let across = h - a in
  let n = Math.floor(2.0 * across) in
  let bars = (along: float, lo: float, hi: float, alongX: bool): List<(Exterior.Box, Color.t)> =>
    List.range(n) |> List.map((k) =>
      let t = along - across + 0.25 + k * (2.0 * across - 0.5) / Math.max(1.0, n - 1.0) in
      if alongX then (box(lo, y0, t - 0.25, hi, y1, t + 0.25), paintWhite) else (box(t - 0.25, y0, lo, t + 0.25, y1, hi), paintWhite)) in
  // A crosswalk over the span of the corners' ramps (`Streetscape.rampSpan`).
  let span = (alongX: bool, along: float, r: (float, float)): List<(Exterior.Box, Color.t)> =>
    let (lo, hi) = r in bars(along, lo, hi, alongX) in
  List.flatten([
    // East and west arms (streets along x): bars along x, across z.
    span(true, cz, Streetscape.rampSpan(1.0, cx + h - a)),
    span(true, cz, Streetscape.rampSpan(-1.0, cx - h + a)),
    // North and south arms (streets along z).
    span(false, cx, Streetscape.rampSpan(1.0, cz + h - a)),
    span(false, cx, Streetscape.rampSpan(-1.0, cz - h + a)),
    dashes(true, cz, cx + h - a + Streetscape.depth + 0.7, cx + reach),
    dashes(true, cz, cx - reach, cx - h + a - Streetscape.depth - 0.7),
    dashes(false, cx, cz + h - a + Streetscape.depth + 0.7, cz + reach),
    dashes(false, cx, cz - reach, cz - h + a - Streetscape.depth - 0.7),
  ])

// A block's paint: its crossroads, arms out to the block's edge.
let blockReach = Block.blockSpan / 2.0
let blockPaint = (bx: float, bz: float): List<(Exterior.Box, Color.t)> =>
  crossroads(Block.centre + bx * Block.blockPitch, Block.centre + bz * Block.blockPitch, Block.streetWidth,
             OfficeLayout.layout.facade / 2.0, blockReach)
// A grid of `side` × `side` blocks: each block's crossroads, and the
// streets between blocks' centre lines, the whole grid long.
let gridPaint = (side: float): List<(Exterior.Box, Color.t)> =>
  let lo = Block.centre - blockReach in
  let hi = Block.centre + (side - 1.0) * Block.blockPitch + blockReach in
  List.flatten([
    List.range(side * side) |> List.concatMap((k) => blockPaint(Math.mod(k, side), Math.floor(k / side))),
    List.range(Math.max(0.0, side - 1.0)) |> List.concatMap((k) =>
      let at = Block.centre + (k + 0.5) * Block.blockPitch in
      List.flatten([dashes(true, at, lo, hi), dashes(false, at, lo, hi)])),
  ])

// The suburb's frontage along each side of its loop road, the corners
// clear: a curb at the road's edge, a lawn strip, then a concrete sidewalk
// up to the houses' fences, cut round each front path. Laid once for the
// town, so the corner houses' frontages don't overlap.
let houseCut = HouseExterior.gateOf(HouseLayout.layout)
let frontage: (List<(Exterior.Box, Color.t)>, List<(Exterior.Box, Finishes.Surface)>) =
  let r = Suburb.outer in
  let walkTop = Exterior.street + 0.005 in
  let fence = Suburb.front - HouseExterior.yard in
  let half = (HouseLayout.footprint.x1 - HouseLayout.footprint.x0) / 2.0 in
  let paths = [0.0 - 1.0, 0.0, 1.0] |> List.map((k) => k * Suburb.spacing - half) in
  // Along [a0, a1], from d0 to d1 out, on the side facing `u`.
  let boxOn = (u: (float, float), a0: float, a1: float, d0: float, d1: float, y0: float, y1: float): Exterior.Box =>
    let (ux, uz) = u in
    let (wx, wz) = Suburb.along(u) in
    let xs = [d0 * ux + a0 * wx, d0 * ux + a1 * wx, d1 * ux + a0 * wx, d1 * ux + a1 * wx] in
    let zs = [d0 * uz + a0 * wz, d0 * uz + a1 * wz, d1 * uz + a0 * wz, d1 * uz + a1 * wz] in
    let lo = (vs: List<float>) => List.minimum(vs) |> Option.defaultValue(0.0) in
    let hi = (vs: List<float>) => List.maximum(vs) |> Option.defaultValue(0.0) in
    box(lo(xs), y0, lo(zs), hi(xs), y1, hi(zs)) in
  // [-r, r] less each house's path.
  let runs = List.flatten([[0.0 - r], paths |> List.concatMap((t) => [t + houseCut.lo, t + houseCut.hi]), [r]]) in
  let spans = List.range(List.length(runs) / 2.0) |> List.map((k) =>
    (List.nth(2.0 * k, runs) |> Option.defaultValue(0.0), List.nth(2.0 * k + 1.0, runs) |> Option.defaultValue(0.0))) in
  let layer = (d0: float, d1: float, y1: float): List<Exterior.Box> =>
    Suburb.outward |> List.concatMap((u) => spans |> List.map((e) => let (a0, a1) = e in boxOn(u, a0, a1, d0, d1, Streetscape.road, y1))) in
  (List.flatten([
     layer(r, r + Streetscape.curb, Exterior.street + 0.12) |> List.map((b) => (b, Streetscape.curbGrey)),
     layer(r + Streetscape.curb, r + 1.4, Exterior.street) |> List.map((b) => (b, HouseExterior.lawn)),
   ]),
   layer(r + 1.4, fence - 0.3, walkTop) |> List.map((b) => (b, Streetscape.paving)))

let paintOf = (boxes: List<(Exterior.Box, Color.t)>): Scene.t => Exterior.instancedOf(boxes)
let suburbPaint: Scene.t =
  let (plain, skinned) = frontage in
  Scene.group([Exterior.instancedOf(plain), Finishes.instancedBy(skinned |> List.map((e) => let (b, f) = e in (Exterior.instanceOf(b), f)))])
// Each town's paint, by `Block.towns`' order: the office alone, the block
// and the park town share the block's crossroads (so does the island); the towers
// theirs, arms to the towers' edge; the cinema and mall have their lots;
// the suburb its frontage.
let towersReach = (2.0 * (TowerLayout.footprint.x1 - TowerLayout.footprint.x0) + Block.streetWidth) / 2.0
let paints: List<Scene.t> = Block.towns |> List.map((t) =>
  if t.name == "suburb" then suburbPaint else paintOf(
    if t.name == "office" || t.name == "block" || t.name == "park" then blockPaint(0.0, 0.0)
    else if t.name == "grid-4" then gridPaint(1.0)
    else if t.name == "grid-16" then gridPaint(2.0)
    else if t.name == "grid-64" then gridPaint(4.0)
    else if t.name == "towers" then crossroads(Block.centre, Block.centre, Block.streetWidth, TowerLayout.facade / 2.0, towersReach)
    else []))
let paintAt = (town: float): Scene.t => List.nth(town, paints) |> Option.defaultValue(Scene.group([]))
