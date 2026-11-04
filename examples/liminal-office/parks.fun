// The parks kit: a park plot (a lawn inside hedges, curving paths, benches
// facing nothing in particular, lamps and bins, an empty playground, a
// pavilion, a fountain), a town plaza (paving, a larger fountain with a few
// stepped jets, benches, planters, lamps), and the monolith.
//
// A park is built once in its own space (x east, z north; its gate on the
// south side) and drawn where its `Placement` puts it: the park town's
// (block.fun, R) or the island's (island.mjs, islandLayout.fun). Each draws
// once per material: a lit instanced cube for everything plain, one per
// finish (the paths and paving), an unlit cube for the lamp globes, its
// signs (streetscape.mjs's atlas: a draw by day, two by night), and the
// plaza's jets. No shadows; nothing is baked. What you'd bump into (benches,
// bins, lamp posts, hedges, basin rims, the playground, the pavilion's posts,
// the monolith) has a collider, built only while the player is near; paths
// and paving have none.

let v = Vec3.make
let rgb = Building.rgb
let noise = Building.noise
let box = Exterior.box

let lawnGreen = rgb(84.0, 98.0, 62.0)
let rimGrey = rgb(150.0, 146.0, 136.0)
let basinGrey = rgb(104.0, 102.0, 96.0)
let stain = rgb(70.0, 68.0, 62.0)
let water = rgb(38.0, 92.0, 98.0)
let leafBrown = rgb(112.0, 84.0, 52.0)
let lampGreen = rgb(46.0, 58.0, 52.0)
let mulch = rgb(104.0, 72.0, 64.0)
let edging = rgb(98.0, 78.0, 58.0)
let slideYellow = rgb(196.0, 164.0, 74.0)
let frameRed = rgb(150.0, 70.0, 60.0)
let climberBlue = rgb(70.0, 104.0, 140.0)
let chainGrey = rgb(140.0, 142.0, 144.0)
let seatBlack = rgb(40.0, 40.0, 42.0)
let gazeboWhite = rgb(206.0, 202.0, 190.0)
let roofGreen = rgb(62.0, 84.0, 70.0)
let padGrey = rgb(136.0, 134.0, 128.0)
let slabBlack = rgb(16.0, 16.0, 19.0)
let plinthGrey = rgb(60.0, 60.0, 62.0)
let pathPaving = Finishes.on(Finishes.paving, rgb(150.0, 148.0, 140.0))
let plazaPaving = Finishes.on(Finishes.paving, rgb(162.0, 156.0, 144.0))
let borderPaving = Finishes.on(Finishes.paving, rgb(112.0, 108.0, 102.0))

// ---- Turned boxes ------------------------------------------------------------------

// A box `w` × `h` × `d` (along x, y, z) about (x, y, z), tilted `tilt`
// radians about x, then turned `yaw` about y: `Instance` and `Physics`
// order.
type Turned = { x: float, y: float, z: float, w: float, h: float, d: float, tilt: float, yaw: float }
let flat = (b: Exterior.Box): Turned =>
  { x: (b.x0 + b.x1) / 2.0, y: (b.y0 + b.y1) / 2.0, z: (b.z0 + b.z1) / 2.0, w: b.x1 - b.x0, h: b.y1 - b.y0, d: b.z1 - b.z0, tilt: 0.0, yaw: 0.0 }
// A beam `w` wide and `h` thick from `a` to `b` (its length along its z).
let beam = (a: Vec3.t, b: Vec3.t, w: float, h: float): Turned =>
  let dx = Vec3.x(b) - Vec3.x(a) in
  let dy = Vec3.y(b) - Vec3.y(a) in
  let dz = Vec3.z(b) - Vec3.z(a) in
  let l = Math.sqrt(dx * dx + dz * dz) in
  { x: (Vec3.x(a) + Vec3.x(b)) / 2.0, y: (Vec3.y(a) + Vec3.y(b)) / 2.0, z: (Vec3.z(a) + Vec3.z(b)) / 2.0,
    w: w, h: h, d: Math.sqrt(l * l + dy * dy), tilt: 0.0 - Math.atan2(dy, l), yaw: if l > 0.0 then Math.atan2(dx, dz) else 0.0 }
// A box `w` × `d` turned `yaw` about (x, z), from `y0` to `y1`.
let turned = (x: float, z: float, y0: float, y1: float, w: float, d: float, yaw: float): Turned =>
  { x: x, y: (y0 + y1) / 2.0, z: z, w: w, h: y1 - y0, d: d, tilt: 0.0, yaw: yaw }
let instanceOf = (t: Turned): Instance.t =>
  Instance.at(v(t.x, t.y, t.z)) |> Instance.scaleXYZ(t.w, t.h, t.d)
    |> Instance.rotateX(Angle.radians(t.tilt)) |> Instance.rotateY(Angle.radians(t.yaw))
// Whether `t` comes within `m` of walk `p` (sampled every 10 cm), in the
// box's own frame: a turned box's footprint, not its bounds. A tilted
// beam's length stands in for its footprint, so it errs clear.
let blocking = (m: float, p: Streetscape.Path, t: Turned): bool =>
  let n = Math.ceil(Math.sqrt((p.x1 - p.x0) * (p.x1 - p.x0) + (p.z1 - p.z0) * (p.z1 - p.z0)) / 0.1) + 1.0 in
  let c = Math.cos(t.yaw) in
  let s = Math.sin(t.yaw) in
  List.range(n + 1.0) |> List.any((k) =>
    let x = p.x0 + (p.x1 - p.x0) * k / n - t.x in
    let z = p.z0 + (p.z1 - p.z0) * k / n - t.z in
    Math.abs(x * c - z * s) < t.w / 2.0 + m && Math.abs(z * c + x * s) < t.d / 2.0 + m)

// ---- Content -------------------------------------------------------------------------

// What a park is made of, in its own space with the ground at y = 0: lit
// boxes, paved ones (by finish), lamp globes (unlit, off by day), decals,
// solid boxes, its lamps' lights, and its walks (to keep clear).
type Content = {
  lit: List<(Turned, Color.t)>,
  paved: List<(Turned, Finishes.Surface)>,
  glow: List<Turned>,
  decals: List<Streetscape.Decal>,
  solid: List<Turned>,
  lamps: List<Vec3.t>,
  walks: List<Streetscape.Path>,
}
let none: Content = { lit: [], paved: [], glow: [], decals: [], solid: [], lamps: [], walks: [] }
let merged = (cs: List<Content>): Content => {
  lit: cs |> List.concatMap((c) => c.lit),
  paved: cs |> List.concatMap((c) => c.paved),
  glow: cs |> List.concatMap((c) => c.glow),
  decals: cs |> List.concatMap((c) => c.decals),
  solid: cs |> List.concatMap((c) => c.solid),
  lamps: cs |> List.concatMap((c) => c.lamps),
  walks: cs |> List.concatMap((c) => c.walks),
}
// The streetscape kit's pieces, as content.
let ofProps = (p: Streetscape.Props): Content =>
  { none with
    lit: List.flatten([p.plain, p.fine]) |> List.map((e) => let (b, c) = e in (flat(b), c)),
    paved: p.skinned |> List.map((e) => let (b, s) = e in (flat(b), s)),
    decals: p.decals, solid: p.solid |> List.map(flat) }
let lit = (bs: List<(Exterior.Box, Color.t)>): Content => { none with lit: bs |> List.map((e) => let (b, c) = e in (flat(b), c)) }

// Every upright box standing on the ground sunk `sink` into it, so on uneven
// ground (the island) nothing floats: legs, posts and path slabs reach down.
let sunkBox = (sink: float, t: Turned): Turned =>
  if t.tilt == 0.0 && t.y - t.h / 2.0 <= 0.0001 then { t with y: t.y - sink / 2.0, h: t.h + sink } else t
let sunk = (sink: float, c: Content): Content =>
  if sink <= 0.0 then c
  else { c with
    lit: c.lit |> List.map((e) => let (t, k) = e in (sunkBox(sink, t), k)),
    paved: c.paved |> List.map((e) => let (t, s) = e in (sunkBox(sink, t), s)) }

// A piece at (x, z) facing `s`: the streetscape frame whose line runs
// through it (its back), and the piece's place along that line.
let facing = (s: Layout.Side, x: float, z: float): (Streetscape.Frame, float) =>
  match s with
  | Layout.South => ({ side: Layout.South, x: 0.0, z: z }, x)
  | Layout.North => ({ side: Layout.North, x: 0.0, z: z }, x)
  | Layout.West => ({ side: Layout.West, x: x, z: 0.0 }, z)
  | Layout.East => ({ side: Layout.East, x: x, z: 0.0 }, z)
// A street tree standing on the lawn: the kit's, without its grate.
let treeAt = (x: float, z: float, seed: float): Content =>
  let t = Streetscape.tree({ side: Layout.North, x: 0.0, z: z }, x, 0.0, 0.0, seed) in ofProps({ t with plain: List.drop(1.0, t.plain) })
let benchAt = (s: Layout.Side, x: float, z: float): Content => let (fr, u) = facing(s, x, z) in ofProps(Streetscape.bench(fr, u, 0.0, 0.0))
let binAt = (s: Layout.Side, x: float, z: float): Content => let (fr, u) = facing(s, x, z) in ofProps(Streetscape.bin(fr, u, 0.0, 0.0))
let signAt = (s: Layout.Side, x: float, z: float, cell: StreetAtlas.Cell): Content =>
  let (fr, u) = facing(s, x, z) in ofProps(Streetscape.poleSign(fr, u, 0.0, 0.0, cell))

// ---- Paths -------------------------------------------------------------------------

// Concrete paths `pathWidth` wide, their slabs 2.5 cm proud of the ground:
// paving segments along a curve, each run 15 cm into the next so the joints
// close (the finish is mapped by world position, so overlaps don't show).
let pathWidth = 1.8
let pathTop = 0.025
// Consecutive points' segments, as (x0, z0, x1, z1).
let segmentsOf = (pts: List<(float, float)>): List<(float, float, float, float)> =>
  List.zip(pts, List.drop(1.0, pts)) |> List.map((e) =>
    let (a, b) = e in
    let (x0, z0) = a in
    let (x1, z1) = b in
    (x0, z0, x1, z1))
let walkOf = (s: (float, float, float, float)): Streetscape.Path => let (x0, z0, x1, z1) = s in { x0: x0, z0: z0, x1: x1, z1: z1 }
let pathOf = (pts: List<(float, float)>): Content =>
  let segments = segmentsOf(pts) in
  { none with
    paved: segments |> List.map((e) =>
      let (x0, z0, x1, z1) = e in
      let l = Math.sqrt((x1 - x0) * (x1 - x0) + (z1 - z0) * (z1 - z0)) in
      (turned((x0 + x1) / 2.0, (z0 + z1) / 2.0, -0.04, pathTop, pathWidth, l + 0.3, Math.atan2(x1 - x0, z1 - z0)), pathPaving)),
    walks: segments |> List.map(walkOf) }
// A cubic Bézier from (x0, z0) to (x3, z3) as `n` segments.
let curve = (n: float, p0: (float, float), p1: (float, float), p2: (float, float), p3: (float, float)): List<(float, float)> =>
  let (x0, z0) = p0 in
  let (x1, z1) = p1 in
  let (x2, z2) = p2 in
  let (x3, z3) = p3 in
  List.range(n + 1.0) |> List.map((k) =>
    let t = k / n in
    let u = 1.0 - t in
    let a = u * u * u in
    let b = 3.0 * u * u * t in
    let c = 3.0 * u * t * t in
    let d = t * t * t in
    (a * x0 + b * x1 + c * x2 + d * x3, a * z0 + b * z1 + c * z2 + d * z3))
// A ring of radius `r` about (x, z), as `n` segments.
let ring = (n: float, x: float, z: float, r: float): List<(float, float)> =>
  List.range(n + 1.0) |> List.map((k) => let a = 2.0 * Math.pi * k / n in (x + r * Math.sin(a), z + r * Math.cos(a)))

// ---- Lamps -------------------------------------------------------------------------

// A park lamp: a dark green post, a globe that glows by night, a cap; its
// light at the globe. island.mjs reads `parkLight` to paint the island's
// night map.
let parkLight = { y: 3.75, r: 1.0, g: 0.9, b: 0.74, intensity: 0.8, range: 12.0 }
let lampAt = (x: float, z: float): Content =>
  let post = box(x - 0.06, 0.0, z - 0.06, x + 0.06, 3.56, z + 0.06) in
  { none with
    lit: [(flat(post), lampGreen), (flat(box(x - 0.12, 0.0, z - 0.12, x + 0.12, 0.5, z + 0.12)), lampGreen),
          (flat(box(x - 0.22, 3.94, z - 0.22, x + 0.22, 4.0, z + 0.22)), lampGreen)],
    glow: [flat(box(x - 0.18, 3.56, z - 0.18, x + 0.18, 3.94, z + 0.18))],
    solid: [flat(box(x - 0.12, 0.0, z - 0.12, x + 0.12, 3.56, z + 0.12))],
    lamps: [v(x, parkLight.y, z)] }

// ---- The fountain --------------------------------------------------------------------

// An octagonal basin about (x, z): its rim `h` tall and `t` thick, the inner
// face `a` from the middle; a floor, still water at `level` (opaque, like
// the mall's) or none (dry: stained, a few leaves); and a pedestal.
type Basin = { x: float, z: float, a: float, h: float, t: float, level: float, wet: bool, seed: float }
let tan8 = Math.tan(Math.pi / 8.0)
// The octagon's interior as four strips, each across it between opposite
// faces, a side wide, turned 45° apart: their union is the octagon, and
// their overlaps are coplanar tops in one colour, so they don't show.
let octagon = (b: Basin, y0: float, y1: float, color: Color.t): List<(Turned, Color.t)> =>
  List.range(4.0) |> List.map((k) => (turned(b.x, b.z, y0, y1, 2.0 * b.a * tan8, 2.0 * b.a, k * Math.pi / 4.0), color))
let rimOf = (b: Basin): List<Turned> =>
  List.range(8.0) |> List.map((k) =>
    let yaw = k * Math.pi / 4.0 in
    let r = b.a + b.t / 2.0 in
    turned(b.x + r * Math.sin(yaw), b.z + r * Math.cos(yaw), 0.0, b.h, 2.0 * (b.a + b.t) * tan8, b.t, yaw))
let basin = (b: Basin): Content =>
  let rim = rimOf(b) in
  { none with
    lit: List.flatten([
      rim |> List.map((t) => (t, rimGrey)),
      octagon(b, 0.0, 0.06, if b.wet then basinGrey else stain),
      if b.wet then octagon(b, b.level - 0.04, b.level, water)
      // Dry: a tide line of stain and a few leaves on the floor.
      else List.range(9.0) |> List.map((k) =>
        let a = 2.0 * Math.pi * noise(b.seed + k) in
        let r = 0.6 * b.a + 0.3 * b.a * noise(b.seed + k + 20.0) in
        (turned(b.x + r * Math.sin(a), b.z + r * Math.cos(a), 0.06, 0.068, 0.14, 0.09, a), leafBrown)),
    ]),
    solid: rim }
// A pedestal in the middle: a column, a bowl on it and a finial, sized by
// `k`. Behind the rim, so it needs no collider.
let pedestal = (x: float, z: float, k: float): Content =>
  lit([(box(x - 0.25 * k, 0.0, z - 0.25 * k, x + 0.25 * k, 1.0 * k, z + 0.25 * k), rimGrey),
       (box(x - 0.7 * k, 1.0 * k, z - 0.7 * k, x + 0.7 * k, 1.12 * k, z + 0.7 * k), rimGrey),
       (box(x - 0.12 * k, 1.12 * k, z - 0.12 * k, x + 0.12 * k, 1.5 * k, z + 0.12 * k), rimGrey)])

// ---- The playground --------------------------------------------------------------------

// Rubber mulch over [x0, x1] × [z0, z1], 4 cm deep, in a timber edging.
let mulchPatch = (r: Layout.Rect): Content =>
  let e = 0.12 in
  lit([(box(r.x0, 0.0, r.z0, r.x1, 0.04, r.z1), mulch),
       (box(r.x0 - e, 0.0, r.z0 - e, r.x1 + e, 0.14, r.z0), edging), (box(r.x0 - e, 0.0, r.z1, r.x1 + e, 0.14, r.z1 + e), edging),
       (box(r.x0 - e, 0.0, r.z0, r.x0, 0.14, r.z1), edging), (box(r.x1, 0.0, r.z0, r.x1 + e, 0.14, r.z1), edging)])
// A slide down +x from (x, z): a ladder, a railed platform 1.5 m up, a
// railed chute to a run-out.
let slide = (x: float, z: float): Content =>
  let rails = [z - 0.27, z + 0.27] in
  { none with
    lit: List.flatten([
      rails |> List.map((rz) => (flat(box(x - 0.54, 0.0, rz - 0.03, x - 0.48, 2.3, rz + 0.03)), slideYellow)),
      List.range(5.0) |> List.map((k) => let y = 0.3 + 0.3 * k in (flat(box(x - 0.54, y - 0.02, z - 0.27, x - 0.48, y + 0.02, z + 0.27)), chainGrey)),
      [(flat(box(x - 0.5, 1.44, z - 0.3, x + 0.3, 1.5, z + 0.3)), slideYellow)],
      rails |> List.concatMap((rz) => [(flat(box(x - 0.5, 2.24, rz - 0.03, x + 0.3, 2.3, rz + 0.03)), slideYellow),
                                       (flat(box(x + 0.24, 0.0, rz - 0.03, x + 0.3, 2.3, rz + 0.03)), slideYellow)]),
      [(beam(v(x + 0.3, 1.47, z), v(x + 2.9, 0.33, z), 0.56, 0.04), chainGrey),
       (flat(box(x + 2.9, 0.29, z - 0.28, x + 3.4, 0.33, z + 0.28)), chainGrey)],
      rails |> List.map((rz) => (beam(v(x + 0.3, 1.6, rz), v(x + 2.9, 0.46, rz), 0.04, 0.26), slideYellow)),
      rails |> List.map((rz) => (flat(box(x + 3.3, 0.0, rz - 0.03, x + 3.36, 0.29, rz + 0.03)), slideYellow)),
    ]),
    solid: [flat(box(x - 0.6, 0.0, z - 0.33, x + 0.33, 2.3, z + 0.33)), flat(box(x + 0.33, 0.0, z - 0.33, x + 3.4, 1.0, z + 0.33))] }
// Swings along x about (x, z): two A-frames, a top bar 2.3 m up, two seats
// hanging still.
let swings = (x: float, z: float): Content =>
  let ends = [x - 1.8, x + 1.8] in
  let seats = [x - 0.8, x + 0.8] in
  { none with
    lit: List.flatten([
      ends |> List.concatMap((ex) => [(beam(v(ex, 0.0, z - 1.0), v(ex, 2.3, z), 0.08, 0.08), frameRed),
                                      (beam(v(ex, 0.0, z + 1.0), v(ex, 2.3, z), 0.08, 0.08), frameRed)]),
      // Upright footings under the legs, so they reach sunk ground too.
      ends |> List.concatMap((ex) => [z - 1.0, z + 1.0] |> List.map((fz) => (flat(box(ex - 0.07, 0.0, fz - 0.07, ex + 0.07, 0.06, fz + 0.07)), frameRed))),
      [(flat(box(x - 1.86, 2.25, z - 0.05, x + 1.86, 2.35, z + 0.05)), frameRed)],
      seats |> List.concatMap((sx) => [
        (flat(box(sx - 0.24, 0.42, z - 0.09, sx + 0.24, 0.46, z + 0.09)), seatBlack),
        (flat(box(sx - 0.23, 0.46, z - 0.01, sx - 0.21, 2.25, z + 0.01)), chainGrey),
        (flat(box(sx + 0.21, 0.46, z - 0.01, sx + 0.23, 2.25, z + 0.01)), chainGrey)]),
    ]),
    solid: List.flatten([
      ends |> List.map((ex) => flat(box(ex - 0.1, 0.0, z - 1.05, ex + 0.1, 2.35, z + 1.05))),
      seats |> List.map((sx) => flat(box(sx - 0.25, 0.4, z - 0.1, sx + 0.25, 2.25, z + 0.1))),
    ]) }
// A climbing frame about (x, z): a 3 × 3 grid of posts 1.2 m apart and
// bars at three heights, 1.8 m tall.
let climber = (x: float, z: float): Content =>
  let at = [-1.2, 0.0, 1.2] in
  let t = 0.025 in
  { none with
    lit: List.flatten([
      at |> List.concatMap((i) => at |> List.map((j) => (flat(box(x + i - t, 0.0, z + j - t, x + i + t, 1.8, z + j + t)), climberBlue))),
      [0.6, 1.2, 1.8] |> List.concatMap((y) => at |> List.concatMap((k) => [
        (flat(box(x - 1.2, y - t, z + k - t, x + 1.2, y + t, z + k + t)), climberBlue),
        (flat(box(x + k - t, y - t, z - 1.2, x + k + t, y + t, z + 1.2)), climberBlue)])),
    ]),
    solid: [flat(box(x - 1.25, 0.0, z - 1.25, x + 1.25, 1.85, z + 1.25))] }

// ---- The pavilion ------------------------------------------------------------------------

// An open pavilion about (x, z), 5.6 m square: a low concrete pad, four
// white posts and beams, a gable roof (two slabs and a ridge, its ends
// open), a rail round three sides (open to the south) and a bench inside.
let pavilion = (x: float, z: float): Content =>
  let h = 2.5 in
  let posts = [(x - 2.5, z - 2.5), (x + 2.5, z - 2.5), (x - 2.5, z + 2.5), (x + 2.5, z + 2.5)] in
  let postBox = (e: (float, float)): Exterior.Box => let (px, pz) = e in box(px - 0.08, 0.1, pz - 0.08, px + 0.08, h, pz + 0.08) in
  let railBoxes = [box(x - 2.5, 0.1, z + 2.47, x + 2.5, 0.9, z + 2.53), box(x - 2.53, 0.1, z - 2.5, x - 2.47, 0.9, z + 2.5),
                   box(x + 2.47, 0.1, z - 2.5, x + 2.53, 0.9, z + 2.5)] in
  merged([
    { none with
      lit: List.flatten([
        [(flat(box(x - 2.8, 0.0, z - 2.8, x + 2.8, 0.1, z + 2.8)), padGrey)],
        posts |> List.map((e) => (flat(postBox(e)), gazeboWhite)),
        [(flat(box(x - 2.6, h - 0.15, z - 2.58, x + 2.6, h, z - 2.42)), gazeboWhite), (flat(box(x - 2.6, h - 0.15, z + 2.42, x + 2.6, h, z + 2.58)), gazeboWhite),
         (flat(box(x - 2.58, h - 0.15, z - 2.6, x - 2.42, h, z + 2.6)), gazeboWhite), (flat(box(x + 2.42, h - 0.15, z - 2.6, x + 2.58, h, z + 2.6)), gazeboWhite)],
        [(beam(v(x, h, z - 3.1), v(x, h + 1.0, z), 6.0, 0.08), roofGreen), (beam(v(x, h, z + 3.1), v(x, h + 1.0, z), 6.0, 0.08), roofGreen),
         (flat(box(x - 3.02, h + 0.96, z - 0.1, x + 3.02, h + 1.06, z + 0.1)), roofGreen)],
        railBoxes |> List.map((b) => (flat(b), gazeboWhite)),
      ]),
      solid: List.flatten([posts |> List.map((e) => flat(postBox(e))), railBoxes |> List.map(flat)]) },
    // Its bench faces the back rail.
    (let c = benchAt(Layout.North, x, z + 1.0) in { c with lit: c.lit |> List.map((e) => let (t, k) = e in ({ t with y: t.y + 0.1 }, k)),
                                                     solid: c.solid |> List.map((t) => { t with y: t.y + 0.1 }) }),
  ])

// ---- The monolith ----------------------------------------------------------------------

// A tall dark slab on a sunken plinth, a faint seam of light at its foot
// front and back, and no label. In streetscape frame `fr`, centred `u`
// along it, its back `d` out, on the ground at `y`, its face to +d; as the
// streetscape kit's Props, so a street side can take one too (a rooftop, a
// forecourt).
let monolithSize = { w: 1.2, h: 4.8, d: 0.4 }
let monolith = (fr: Streetscape.Frame, u: float, d: float, y: float): Streetscape.Props =>
  let p = Streetscape.partIn(fr, u, d, y) in
  let w = monolithSize.w / 2.0 in
  let depth = monolithSize.d in
  { plain: [(p(0.0 - w - 0.12, w + 0.12, 0.0, 0.03, -0.12, depth + 0.12), plinthGrey), (p(0.0 - w, w, 0.03, monolithSize.h, 0.0, depth), slabBlack)],
    fine: [], skinned: [],
    decals: [Streetscape.glow(StreetAtlas.seam, Streetscape.faceIn(fr, true, u - w, u + w, y + 0.03, y + 0.09, d + depth + 0.003)),
             Streetscape.glow(StreetAtlas.seam, Streetscape.faceIn(fr, false, u - w, u + w, y + 0.03, y + 0.09, d - 0.003))],
    solid: [p(0.0 - w - 0.12, w + 0.12, 0.0, monolithSize.h, -0.12, depth + 0.12)] }

// ---- The park -----------------------------------------------------------------------------

// A park `parkSize` across about its middle: hedged all round but for a gate
// in the south and east sides; a path from each gate curving to a ring round
// the fountain, and on to the pavilion (north-west) and the playground
// (north-east); the monolith alone on the lawn to the south-west.
// island.mjs reads `parkSize` and `parkLampSpots`.
let parkSize = { w: 44.0, d: 36.0 }
let parkLampSpots = [(2.4, -14.5), (-4.8, -6.4), (7.6, -2.2), (-7.0, 8.4), (3.4, 8.6), (16.0, 2.4)]
let gate = 2.2
let parkFountain: Basin = { x: 0.0, z: 0.0, a: 2.6, h: 0.45, t: 0.35, level: 0.32, wet: true, seed: 0.0 }
let playground = Layout.rect(6.0, 6.5, 18.5, 15.5)
let parkHedges = (seed: float): Content =>
  let hw = parkSize.w / 2.0 in
  let hd = parkSize.d / 2.0 in
  let i = 0.2 in
  let southFr: Streetscape.Frame = { side: Layout.North, x: 0.0, z: 0.0 - hd + i } in
  let northFr: Streetscape.Frame = { side: Layout.South, x: 0.0, z: hd - i } in
  let westFr: Streetscape.Frame = { side: Layout.East, x: 0.0 - hw + i, z: 0.0 } in
  let eastFr: Streetscape.Frame = { side: Layout.West, x: hw - i, z: 0.0 } in
  let run = (fr: Streetscape.Frame, u0: float, u1: float, k: float) => ofProps(Streetscape.hedge(fr, u0, u1, 0.0, 0.0, seed + k)) in
  merged([
    run(southFr, 0.0 - hw + i, 0.0 - gate, 1.0), run(southFr, gate, hw - i, 2.0),
    run(northFr, 0.0 - hw + i, hw - i, 3.0),
    run(westFr, 0.0 - hd + i + 0.55, hd - i - 0.55, 4.0),
    run(eastFr, 0.0 - hd + i + 0.55, 4.0 - gate, 5.0), run(eastFr, 4.0 + gate, hd - i - 0.55, 6.0),
  ])
let parkPaths = merged([
  pathOf(ring(28.0, 0.0, 0.0, 5.2)),
  pathOf(curve(10.0, (0.0, -18.3), (0.0, -13.5), (-5.0, -10.0), (0.0, -5.2))),
  pathOf(curve(12.0, (22.3, 4.0), (15.0, 7.5), (10.5, -1.0), (5.0, 1.4))),
  pathOf(curve(8.0, (-3.7, 3.7), (-6.5, 8.0), (-9.5, 3.5), (-12.0, 6.6))),
  pathOf(curve(6.0, (3.7, 3.7), (5.0, 6.6), (6.4, 6.4), (8.5, 6.2))),
])
// What the park holds, by `seed`: `wet` a fountain of still water (or a dry
// basin), `monolith` the slab on the lawn, `lawn` a green ground patch
// (where no colormap paints one: the flat ground).
let parkOf = (seed: float, wet: bool, withMonolith: bool, withLawn: bool): Content =>
  let hw = parkSize.w / 2.0 in
  let hd = parkSize.d / 2.0 in
  merged(List.flatten([
    if withLawn then [lit([(box(0.0 - hw, -0.04, 0.0 - hd, hw, 0.0, hd), lawnGreen)])] else [],
    [parkHedges(seed), parkPaths,
     basin({ parkFountain with wet: wet, seed: seed }), pedestal(0.0, 0.0, 1.0),
     mulchPatch(playground), slide(7.8, 13.4), swings(14.6, 8.9), climber(15.6, 13.2),
     pavilion(-12.0, 10.0)],
    parkLampSpots |> List.map((e) => let (x, z) = e in lampAt(x, z)),
    // Benches facing nothing in particular: the hedge, the empty lawn, the
    // north hedge past the fountain; one at the playground, its back to it.
    [benchAt(Layout.South, 8.5, -9.6), benchAt(Layout.West, -18.6, -2.0), benchAt(Layout.East, 3.6, -12.4),
     benchAt(Layout.West, -6.6, -1.0), benchAt(Layout.North, 0.0, 7.2), benchAt(Layout.South, 12.0, 5.6)],
    [treeAt(-17.5, -13.0, seed), treeAt(-18.0, 13.5, seed + 1.0), treeAt(-4.0, 14.5, seed + 2.0), treeAt(17.5, -6.0, seed + 3.0),
     treeAt(12.0, -14.0, seed + 4.0), treeAt(-8.5, -15.0, seed + 5.0)],
    [binAt(Layout.East, 3.6, -14.0), binAt(Layout.North, 1.6, 7.2), binAt(Layout.South, 10.2, 5.6)],
    [signAt(Layout.South, 2.6, -17.0, StreetAtlas.parkCloses), signAt(Layout.West, 5.4, 9.0, StreetAtlas.noAdults)],
    if withMonolith then [ofProps(monolith({ side: Layout.South, x: 0.0, z: -8.0 }, -12.0, 0.0, 0.0))] else [],
  ]))

// ---- The plaza ----------------------------------------------------------------------------

// A town square `plazaSize` across, paved with a darker border: a large
// fountain in the middle (still water round a tiered pedestal, with jets),
// benches round it facing out, trees and planters at its corners, lamps on
// the diagonals, a ring walk round the basin and an approach from each side.
// island.mjs reads `plazaSize` and `plazaLampSpots`.
let plazaSize = { w: 30.0, d: 26.0 }
let plazaLampSpots = [(-9.0, -8.0), (9.0, -8.0), (-9.0, 8.0), (9.0, 8.0)]
let plazaFountain: Basin = { x: 0.0, z: 0.0, a: 5.0, h: 0.5, t: 0.45, level: 0.36, wet: true, seed: 0.0 }
let border = 0.8
let plazaOf = (seed: float): Content =>
  let hw = plazaSize.w / 2.0 in
  let hd = plazaSize.d / 2.0 in
  let b = border in
  let paved = (r: Layout.Rect, s: Finishes.Surface): (Turned, Finishes.Surface) => (flat(box(r.x0, -0.04, r.z0, r.x1, 0.03, r.z1)), s) in
  merged(List.flatten([
    [{ none with
       paved: [paved(Layout.rect(b - hw, b - hd, hw - b, hd - b), plazaPaving),
               paved(Layout.rect(0.0 - hw, 0.0 - hd, hw, b - hd), borderPaving), paved(Layout.rect(0.0 - hw, hd - b, hw, hd), borderPaving),
               paved(Layout.rect(0.0 - hw, b - hd, b - hw, hd - b), borderPaving), paved(Layout.rect(hw - b, b - hd, hw, hd - b), borderPaving)],
       walks: List.flatten([
         segmentsOf(ring(32.0, 0.0, 0.0, 6.5)) |> List.map(walkOf),
         [{ x0: 0.0, z0: 0.0 - hd, x1: 0.0, z1: -6.5 }, { x0: 0.0, z0: 6.5, x1: 0.0, z1: hd },
          { x0: 0.0 - hw, z0: 0.0, x1: -6.5, z1: 0.0 }, { x0: 6.5, z0: 0.0, x1: hw, z1: 0.0 }],
       ]) }],
    [basin({ plazaFountain with seed: seed }),
     lit([(box(-0.9, 0.0, -0.9, 0.9, 0.7, 0.9), rimGrey), (box(-1.3, 1.6, -1.3, 1.3, 1.75, 1.3), rimGrey),
          (box(-0.25, 0.7, -0.25, 0.25, 2.4, 0.25), rimGrey), (box(-0.65, 2.4, -0.65, 0.65, 2.5, 0.65), rimGrey),
          (box(-0.14, 2.5, -0.14, 0.14, 2.9, 0.14), rimGrey)])],
    plazaLampSpots |> List.map((e) => let (x, z) = e in lampAt(x, z)),
    // Benches round the basin, backs to it, facing out over the paving.
    [-2.4, 2.4] |> List.concatMap((u) => [benchAt(Layout.North, u, 7.4), benchAt(Layout.South, u, -7.4), benchAt(Layout.East, 7.4, u), benchAt(Layout.West, -7.4, u)]),
    [binAt(Layout.North, 4.4, 7.4), binAt(Layout.South, -4.4, -7.4)],
    [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] |> List.concatMap((e) =>
      let (sx, sz) = e in
      [ofProps(Streetscape.tree({ side: Layout.North, x: 0.0, z: 0.0 }, sx * 12.0, sz * 10.0, 0.0, seed + sx + 3.0 * sz)),
       ofProps(Streetscape.planter({ side: Layout.North, x: 0.0, z: sz * 11.6 - 0.3 }, sx * 5.4, 0.0, 0.0, 1.6, seed + 7.0 * sx + sz))]),
  ]))

// Stepped jets: `jetCount` arcs leaning in from the rim to the pedestal's
// bowl, and a plume off its top; `jetSteps` heights a cycle, `jetRate`
// steps a second. Thin unlit boxes, a draw for all of them.
let jetCount = 8.0
let jetSteps = 12.0
let jetRate = 6.0
let jetStepAt = (tts: float): float => Math.mod(Math.floor(tts * jetRate), jetSteps)
// Jet `k`'s height at step `s`: 1.3 to 1.9 m, each a step behind the last.
let jetHeight = (k: float, s: float): float =>
  let phase = Math.mod(s + 3.0 * k, jetSteps) / jetSteps in
  1.3 + 0.6 * (0.5 - 0.5 * Math.cos(2.0 * Math.pi * phase))
let jetsOf = (s: float): List<Turned> =>
  let level = plazaFountain.level in
  List.flatten([
    List.range(jetCount) |> List.concatMap((k) =>
      let a = 2.0 * Math.pi * (k + 0.5) / jetCount in
      let at = (r: float, y: float): Vec3.t => v(r * Math.sin(a), y, r * Math.cos(a)) in
      let h = jetHeight(k, s) in
      [beam(at(3.6, level), at(2.6, level + h), 0.05, 0.05), beam(at(2.6, level + h), at(1.5, 1.75 + 0.2 * h), 0.05, 0.05),
       turned(3.6 * Math.sin(a), 3.6 * Math.cos(a), level, level + 0.012, 0.3 + 0.1 * h, 0.3 + 0.1 * h, a)]),
    [turned(0.0, 0.0, 2.9, 3.3 + 0.4 * (jetHeight(0.0, s) - 1.3), 0.06, 0.06, 0.0)],
  ])
let jetColor = (lamps: float): Color.t => Hours.color(Hours.mix({ r: 0.86, g: 0.9, b: 0.92 }, { r: 0.46, g: 0.6, b: 0.66 }, lamps))

// ---- Placements ---------------------------------------------------------------------------

// Where a park (`kind` "park") or plaza ("plaza") stands: its middle at
// (x, y, z), turned `yaw` degrees about y (as `Scene.rotateY`), each upright
// box `sink` into the ground. `wet`, `monolith` and `lawn` as `parkOf`
// takes them. Plain data, for memo keys.
type Placement = { name: string, kind: string, x: float, y: float, z: float, yaw: float, sink: float, wet: bool, monolith: bool, lawn: bool, seed: float }
let nowhere: Placement = { name: "", kind: "park", x: 0.0, y: 0.0, z: 0.0, yaw: 0.0, sink: 0.0, wet: false, monolith: false, lawn: false, seed: 0.0 }
let isPlaza = (p: Placement): bool => p.kind == "plaza"
let sizeOf = (p: Placement) => if isPlaza(p) then plazaSize else parkSize
let contentOf = (p: Placement): Content =>
  sunk(p.sink, if isPlaza(p) then plazaOf(p.seed) else parkOf(p.seed, p.wet, p.monolith, p.lawn))

// The park town (R): the block, a park west of it, the plaza south of it at
// the end of the street through the crossroads. Its ground is the block's.
let townParks: List<Placement> = [
  { name: "park-west", kind: "park", x: -100.0, y: Exterior.street, z: -12.0, yaw: 0.0, sink: 0.0, wet: true, monolith: true, lawn: true, seed: 1.0 },
  { name: "plaza-south", kind: "plaza", x: -12.0, y: Exterior.street, z: -84.0, yaw: 0.0, sink: 0.0, wet: true, monolith: false, lawn: false, seed: 2.0 },
]
// The island's (islandLayout.fun), its heights the terrain's own: lifted
// as island.fun lifts the terrain. The colormaps paint their lawns.
let islandParks: List<Placement> = IslandLayout.parks |> List.map((p) =>
  { name: p.name, kind: p.kind, x: p.x, y: p.y + Island.lift, z: p.z, yaw: p.yaw, sink: p.sink, wet: p.wet, monolith: p.monolith, lawn: false, seed: p.seed })
let all = List.append(townParks, islandParks)
// Each placement's content, built once at load.
let contents: Map<string, Content> = Map.fromList(all |> List.map((p) => (p.name, contentOf(p))))
let contentAt = (p: Placement): Content => Map.get(p.name, contents) |> Option.defaultValue(none)

// A local point of `p` in the world.
let toWorld = (p: Placement, at: Vec3.t): Vec3.t =>
  let (x, z) = Block.turn(p.yaw, Vec3.x(at), Vec3.z(at)) in v(x + p.x, Vec3.y(at) + p.y, z + p.z)
// How far (x, z) is from `p`'s rect (0 inside it).
let fromPark = (p: Placement, x: float, z: float): float =>
  let (lx, lz) = Block.turn(0.0 - p.yaw, x - p.x, z - p.z) in
  let s = sizeOf(p) in
  Block.fromFootprint(Layout.rect(0.0 - s.w / 2.0, 0.0 - s.d / 2.0, s.w / 2.0, s.d / 2.0), v(lx, 0.0, lz))

// ---- Drawing --------------------------------------------------------------------------------

// What a placement draws that only its own data decides: the lit boxes, a
// draw per finish, and the signs by `day` or night.
type Look = { park: Placement, day: bool }
let staticScene = (look: Look): Scene.t =>
  let c = contentAt(look.park) in
  Scene.group([
    Scene.cube() |> Scene.lit(rgb(255.0, 255.0, 255.0)) |> Scene.instanced(c.lit |> List.map((e) => let (t, k) = e in instanceOf(t) |> Instance.tint(k))),
    Finishes.instancedBy(c.paved |> List.map((e) => let (t, s) = e in (instanceOf(t), s))),
    Streetscape.modelOf(c.decals, not look.day),
  ]) |> placed(look.park)
let placed = (p: Placement, s: Scene.t): Scene.t =>
  s |> Scene.rotateY(Angle.degrees(p.yaw)) |> Scene.translate(v(p.x, p.y, p.z))
// The globes, lit up by night (`lamps`, 1 at night), off by day.
type GlowLook = { park: Placement, lamps: float }
let glowScene = (look: GlowLook): Scene.t =>
  let c = contentAt(look.park) in
  Scene.cube() |> Scene.color(Exterior.shadeColor(look.lamps)) |> Scene.instanced(c.glow |> List.map(instanceOf))
    |> placed(look.park)
// A plaza's jets at step `step`.
type JetLook = { park: Placement, step: float, lamps: float }
let jetScene = (look: JetLook): Scene.t =>
  Scene.cube() |> Scene.color(jetColor(look.lamps)) |> Scene.instanced(jetsOf(look.step) |> List.map(instanceOf)) |> placed(look.park)

// Parks within `reach` (the far clip) of the eye.
let shown = (parks: List<Placement>, eye: Vec3.t, reach: float): List<Placement> =>
  parks |> List.filter((p) => fromPark(p, Vec3.x(eye), Vec3.z(eye)) < reach)
// Every shown park: its static scene (rebuilt only when day flips), its
// globes (when the lamps change), a plaza's jets (each step).
let scenesAt = (parks: List<Placement>, eye: Vec3.t, reach: float, lamps: float, day: bool, tts: float): Scene.t =>
  Scene.group(shown(parks, eye, reach) |> List.concatMap((p) => List.flatten([
    [Scene.memo(p.name, { park: p, day: day }, staticScene),
     Scene.memo($"{p.name}/glow", { park: p, lamps: lamps }, glowScene)],
    if isPlaza(p) then [Scene.memo($"{p.name}/jets", { park: p, step: jetStepAt(tts), lamps: lamps }, jetScene)] else [],
  ])))

// ---- Lights and colliders --------------------------------------------------------------------

// Every lamp of `parks` in the world, at `lamps` (1 at night), for the
// frame's nearest-lights pick (game.fun).
let lampsOf = (p: Placement): List<Vec3.t> => contentAt(p).lamps |> List.map((l) => toWorld(p, l))
// A plaza's fountain is lit by night, teal, from just over its water south
// of the pedestal (its gate's side): the drum, the bowl and the jets. Its
// pool falls on the plaza's own paving, so the island's night map doesn't
// paint it.
let fountainLight = { r: 0.35, g: 0.95, b: 0.85, intensity: 1.0, range: 8.0 }
let lightsOf = (lamps: float, parks: List<Placement>): List<(Vec3.t, Light.t)> =>
  let point = (l, at: Vec3.t) => (at, Light.point(at, Color.rgb(l.r, l.g, l.b), l.intensity * lamps, l.range) |> Light.baked) in
  List.flatten([
    parks |> List.concatMap(lampsOf) |> List.map((at) => point(parkLight, at)),
    parks |> List.filter(isPlaza) |> List.map((p) => point(fountainLight, toWorld(p, v(0.0, plazaFountain.level + 0.45, -1.9)))),
  ])

// A park's colliders: each solid box, a fixed body under the park's tags.
let bodiesOf = (p: Placement): List<Physics.body> =>
  contentAt(p).solid |> List.indexedMap((k, t) =>
    Physics.fixed(Physics.tag($"{p.name}/{k}"), Physics.box(t.w, t.h, t.d))
      |> Physics.rotateX(Angle.radians(t.tilt)) |> Physics.rotateY(Angle.radians(t.yaw))
      |> Physics.rotateY(Angle.degrees(p.yaw)) |> Physics.at(toWorld(p, v(t.x, t.y, t.z))))
// Parks within this of the player are solid.
let solidReach = 20.0
let bodiesNear = (parks: List<Placement>, at: Physics.position): List<Physics.body> =>
  parks |> List.filter((p) => fromPark(p, at.x, at.z) < solidReach) |> List.map((p) => Physics.memo(p.name, p, bodiesOf))

// ---- The park town's views (game.fun) --------------------------------------------------------

// R's views, in the world (copy 0 stands at the origin, unturned), as
// (x, z, yaw, pitch): 1 the park's south gate, looking in; 2 the playground;
// 3 the pavilion; 4 the fountain; 5 the monolith across the lawn; 6 the
// plaza's fountain from the south; 7 close to it, between it and the
// benches; 8 the park from its east gate; 9 the street, looking down it to the
// plaza; 0 the monolith close up. Each is placed in its park's space.
let viewIn = (p: Placement, x: float, z: float, yaw: float, pitch: float): (float, float, float, float) =>
  let w = toWorld(p, v(x, 0.0, z)) in (Vec3.x(w), Vec3.z(w), yaw + p.yaw * Math.pi / 180.0, pitch)
let townPark = List.nth(0.0, townParks) |> Option.defaultValue(nowhere)
let townPlaza = List.nth(1.0, townParks) |> Option.defaultValue(nowhere)
let views = [
  viewIn(townPark, 0.0, -19.5, 0.0, 0.05), viewIn(townPark, 4.5, 1.5, 0.68, 0.0), viewIn(townPark, -6.5, 0.5, -0.5, 0.05),
  viewIn(townPark, 0.0, -8.4, 0.0, -0.08), viewIn(townPark, 0.0, -11.0, -1.4, 0.1), viewIn(townPlaza, 0.0, -14.0, 0.0, 0.08),
  viewIn(townPlaza, -2.4, -6.6, 0.0, 0.1), viewIn(townPark, 23.5, 4.0, -1.5708, 0.0), (-12.0, -50.0, 3.1416, 0.02),
  viewIn(townPark, -10.4, -12.6, -0.35, 0.28),
]
