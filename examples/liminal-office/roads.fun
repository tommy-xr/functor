// The towns' road paint, drawn with the ground (game.fun): a zebra
// crosswalk over each arm of every crossroads, landing on the corners' kerb
// ramps (streetscape.fun), and a dashed centre line down each street. A
// lit instanced cube per town: one draw.

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

let paintOf = (boxes: List<(Exterior.Box, Color.t)>): Scene.t => Exterior.instancedOf(boxes)
// Each town's paint, by `Block.towns`' order: the office alone and the
// block share the block's crossroads (so does the island); the towers
// theirs, arms to the towers' edge; the cinema has its lot.
let towersReach = (2.0 * (TowerLayout.footprint.x1 - TowerLayout.footprint.x0) + Block.streetWidth) / 2.0
let paints: List<Scene.t> = Block.towns |> List.map((t) =>
  paintOf(
    if t.name == "office" || t.name == "block" then blockPaint(0.0, 0.0)
    else if t.name == "grid-4" then gridPaint(1.0)
    else if t.name == "grid-16" then gridPaint(2.0)
    else if t.name == "grid-64" then gridPaint(4.0)
    else if t.name == "towers" then crossroads(Block.centre, Block.centre, Block.streetWidth, TowerLayout.facade / 2.0, towersReach)
    else []))
let paintAt = (town: float): Scene.t => List.nth(town, paints) |> Option.defaultValue(Scene.group([]))
