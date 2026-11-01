// The block: the office as an archetype, built ONCE in its own space (its
// cell models and lightmaps in game.fun, portals.fun's cells, building.fun's
// and exterior.fun's colliders, exterior.fun's street side), placed as four
// copies at 0/90/180/270° yaw around a crossroads. Every copy draws the same
// `Model.static` values, so the GPU holds one office. The grids repeat the
// block, a street apart, to 16 and 64 copies.
//
// A copy is translation plus a yaw about Y, unit scale. Its interior draws
// only within `activeOn` of its footprint (staying on out to `activeOff`),
// where portals.fun's collar holds the eye, and at most `maxActive` at once.

open Layout

let v = Vec3.make

// Where a copy stands: `local` → rotate `yaw` degrees about Y, then move by
// (x, z). Plain data, so it can be a `Scene.memo`/`Physics.memo` key.
type Copy = { name: string, x: float, z: float, yaw: float }

// (x, z) turned `deg` degrees about Y, as `Scene.rotateY` turns it: +z
// swings toward +x.
let turn = (deg: float, x: float, z: float): (float, float) =>
  let a = deg * Math.pi / 180.0 in
  (x * Math.cos(a) + z * Math.sin(a), z * Math.cos(a) - x * Math.sin(a))

let toWorld = (c: Copy, p: Vec3.t): Vec3.t =>
  let (x, z) = turn(c.yaw, Vec3.x(p), Vec3.z(p)) in v(x + c.x, Vec3.y(p), z + c.z)
let toLocal = (c: Copy, p: Vec3.t): Vec3.t =>
  let (x, z) = turn(0.0 - c.yaw, Vec3.x(p) - c.x, Vec3.z(p) - c.z) in v(x, Vec3.y(p), z)
// A heading (the game's yaw: forward is (sin yaw, cos yaw)) in the world.
let headingToWorld = (c: Copy, yaw: float): float => yaw + c.yaw * Math.pi / 180.0

// The street between neighbours. The copies turn about a crossroads this far
// south-west of the office's south-west corner, so each one's entrance and
// windows face a street (the office's entrance faces south, its windows west).
let streetWidth = 24.0
let centre = 0.0 - streetWidth / 2.0
// A block of four spans this far along x and z; the grids repeat it every
// `blockPitch`, a street apart.
let blockSpan = 2.0 * Math.max(footprint.x1 - footprint.x0, footprint.z1 - footprint.z0) + streetWidth
let blockPitch = blockSpan + streetWidth

// Block `s`'s cell in a grid, in rings out from (0, 0): 0 is (0, 0); 1–3 are
// (1, 0), (1, 1), (0, 1); 4–8 are ring 2. So the first 4 blocks are 2 × 2, the
// first 16 are 4 × 4, and each grid extends the smaller ones.
let ringCell = (s: float): (float, float) =>
  let r = Math.floor(Math.sqrt(s)) in
  let o = s - r * r in
  if o <= r then (r, o) else (o - r - 1.0, r)

// Copy `id`: the block's copy `id mod 4` (yawed 90° × that about the
// crossroads), in the grid's block `id / 4`. Copies 0–3 are the block, so an
// id names the same building, tags and memo key in every town.
let copyAt = (id: float): Copy =>
  let k = Math.mod(id, 4.0) in
  let (bx, bz) = ringCell(Math.floor(id / 4.0)) in
  let yaw = 90.0 * k in
  let (x, z) = turn(yaw, centre, centre) in
  { name: $"office-{id}", x: centre - x + bx * blockPitch, z: centre - z + bz * blockPitch, yaw: yaw }
let home = copyAt(0.0)

let sceneOf = (c: Copy, s: Scene.t): Scene.t =>
  s |> Scene.rotateY(Angle.degrees(c.yaw)) |> Scene.translate(v(c.x, 0.0, c.z))

// A copy's colliders: building.fun's and the lamp posts, each a fixed body
// under its own tag.
let tagOf = (c: Copy, local: string): string => $"{c.name}/{local}"
let bodiesOf = (c: Copy): List<Physics.body> =>
  List.flatten([Building.colliders, Exterior.lampColliders]) |> List.map((k) =>
    Physics.fixed(Physics.tag(tagOf(c, k.tag)), k.shape)
      |> Physics.at(toWorld(c, k.at)) |> Physics.rotateY(Angle.degrees(c.yaw)))

// How far a local point is from the footprint (0 inside it).
let fromFootprint = (p: Vec3.t): float =>
  let f = footprint in
  let dx = Math.max(0.0, Math.max(f.x0 - Vec3.x(p), Vec3.x(p) - f.x1)) in
  let dz = Math.max(0.0, Math.max(f.z0 - Vec3.z(p), Vec3.z(p) - f.z1)) in
  Math.sqrt(dx * dx + dz * dz)

// ---- Towns ---------------------------------------------------------------------

// A spatial index over a town's copies: each copy is filed under the square
// tile holding its footprint's middle, so a query reads only the tiles near
// the eye, however many copies there are. `i0`..`j1` bound the tiles in use.
let tile = blockPitch / 2.0
let halfDiagonal =
  let w = footprint.x1 - footprint.x0 in
  let d = footprint.z1 - footprint.z0 in
  Math.sqrt(w * w + d * d) / 2.0
let tileOf = (x: float): float => Math.floor(x / tile)
let tileKey = (i: float, j: float): float => i * 4096.0 + j
let middleOf = (c: Copy): Vec3.t =>
  toWorld(c, v((footprint.x0 + footprint.x1) / 2.0, 0.0, (footprint.z0 + footprint.z1) / 2.0))
type Index = { tiles: Map<float, List<(float, Copy)>>, i0: float, i1: float, j0: float, j1: float }
let indexOf = (copies: List<Copy>): Index =>
  let filed = copies |> List.indexedMap((id, c) => (tileOf(Vec3.x(middleOf(c))), tileOf(Vec3.z(middleOf(c))), id, c)) in
  let is = filed |> List.map((e) => let (i, j, id, c) = e in i) in
  let js = filed |> List.map((e) => let (i, j, id, c) = e in j) in
  { tiles: filed |> List.fold((m, e) =>
      let (i, j, id, c) = e in
      m |> Map.update(tileKey(i, j), (was) => Option.Some(List.append([(id, c)], Option.defaultValue([], was)))), Map.empty()),
    i0: List.minimum(is) |> Option.defaultValue(0.0), i1: List.maximum(is) |> Option.defaultValue(0.0),
    j0: List.minimum(js) |> Option.defaultValue(0.0), j1: List.maximum(js) |> Option.defaultValue(0.0) }

let everywhere = 1000000.0
// In a grid, copies within this of the player are solid.
let solidOn = 12.0
// In a grid, the frame's 6 nearest lights come from copies this near.
let lightReach = 60.0

// A placement of the first `n` copies. `shells`: inactive copies draw as
// shells.fun's stand-ins, not as their street side. Copies within `solidOn`
// of the player get colliders; copies within `lightReach` of the eye lend
// their lights (and may be the nearest).
type Town = { name: string, copies: List<Copy>, index: Index, shells: bool, solidOn: float, lightReach: float, ground: Scene.t, groundBody: Physics.body }
// Grids have shells and nearby-only colliders and lights; the office alone
// and the block keep every copy solid and lit (views 9 and 0 teleport
// between copies).
let townOf = (name: string, n: float, shells: bool, ground: Scene.t, groundBody: Physics.body): Town =>
  let copies = List.range(n) |> List.map(copyAt) in
  { name: name, copies: copies, index: indexOf(copies), shells: shells, solidOn: if shells then solidOn else everywhere,
    lightReach: if shells then lightReach else everywhere, ground: ground, groundBody: groundBody }

// The block's ground and its collider, past every collar.
let groundSize = 400.0
let ground = Exterior.groundAt(centre, centre, groundSize)
let groundBody = Exterior.groundBodyAt("block-ground", centre, centre, groundSize)

// A grid of `n` copies (4, 16, 64: 1, 2 × 2, 4 × 4 blocks), on a ground that
// reaches past the far clip from every street.
let gridOf = (n: float): Town =>
  let side = Math.sqrt(n / 4.0) * blockPitch in
  let mid = centre + (side - blockPitch) / 2.0 in
  let size = side + 2.0 * 160.0 in
  townOf($"grid-{n}", n, true,
    Exterior.groundAt(mid, mid, size), Exterior.groundBodyAt($"grid-{n}-ground", mid, mid, size))

// The towns, by the model's `town`: the office alone, the block, then the
// grids of 4, 16 and 64. B toggles the office alone and the block; G cycles
// the grids.
let office = townOf("office", 1.0, false, Exterior.ground, Exterior.groundBody)
let towns = [office, townOf("block", 4.0, false, ground, groundBody), gridOf(4.0), gridOf(16.0), gridOf(64.0)]
let townAt = (t: float): Town => List.nth(t, towns) |> Option.defaultValue(office)

// The copies whose footprints lie within `r` of `eye`, as (id, distance), by
// id. Reads only the index tiles within `r` plus a footprint's reach.
let near = (town: Town, r: float, eye: Vec3.t): List<(float, float)> =>
  let x = town.index in
  let span = (lo: float, hi: float, at: float): List<float> =>
    let from = Math.max(lo, tileOf(at - r - halfDiagonal)) in
    let to = Math.min(hi, tileOf(at + r + halfDiagonal)) in
    List.range(Math.max(0.0, to - from + 1.0)) |> List.map((k) => from + k) in
  span(x.i0, x.i1, Vec3.x(eye)) |> List.concatMap((i) =>
    span(x.j0, x.j1, Vec3.z(eye)) |> List.concatMap((j) =>
      Map.get(tileKey(i, j), x.tiles) |> Option.defaultValue([])))
    |> List.map((e) => let (id, c) = e in (id, fromFootprint(toLocal(c, eye))))
    |> List.filter((e) => let (id, d) = e in d <= r)
    |> List.sortBy((e) => let (id, d) = e in id)

// ---- Activation ----------------------------------------------------------------

// A copy's interior draws from `activeOn` metres of its footprint, and stays
// on until `activeOff`, so walking the edge doesn't flicker it.
let activeOn = 30.0
let activeOff = 34.0
// How far an eye may sit from the body the radius is measured from: the
// stereo half-IPD and a lean. The collar covers `activeOff` plus this.
let eyeReach = 1.0
// How many interiors draw at once, unless the model asks for fewer (K).
let maxActive = 4.0

let isIn = (id: float, ids: List<float>): bool => ids |> List.any((i) => i == id)

// The copies within `off` of `eye` that are inside `on`, or were on (`was`):
// hysteresis, as (id, distance) by id.
let qualifying = (town: Town, on: float, off: float, was: List<float>, eye: Vec3.t): List<(float, float)> =>
  near(town, off, eye) |> List.filter((e) => let (id, d) = e in d < on || isIn(id, was))

// Which copies' interiors draw for an eye at `eye`, given those that did last
// frame (`was`): at most `k`, by id. A copy qualifies inside `activeOn`, or
// inside `activeOff` if it was on. When more than `k` qualify they rank:
//   1. the eye's own building (within `eyeReach` of its footprint);
//   2. those already on, nearest first. Inside `activeOff` the eye is in that
//      copy's collar and may see in through its door or windows (the portal
//      walk that would know runs in the renderer), so a copy that is on is
//      never swapped out for a nearer one;
//   3. the rest, nearest first.
// So a copy turns off before `activeOff` only when the eye walks into
// another building and all `k` are taken.
let activeAt = (town: Town, k: float, was: List<float>, eye: Vec3.t): List<float> =>
  qualifying(town, activeOn, activeOff, was, eye)
    |> List.sortBy((e) =>
      let (id, d) = e in
      (if d <= eyeReach then 0.0 else if isIn(id, was) then 1.0 else 2.0) * 1000.0 + d)
    |> List.take(k)
    |> List.map((e) => let (id, d) = e in id)
    |> List.sortBy((id) => id)

// How many active copies' cells don't hold `eye`: each would draw its whole
// interior (`Scene.cells`' fallback). Zero by construction; the e2e checks it.
let uncovered = (town: Town, active: List<float>, eye: Vec3.t): float =>
  active |> List.filter((id) => Cells.locate(toLocal(copyAt(id), eye), Portals.level) == Option.None) |> List.length

// Copies with colliders: within the town's `solidOn` of the player, staying
// solid to `solidMargin` past it. Not capped: the streets bound how many
// stand that near.
let solidMargin = 4.0
let solidAt = (town: Town, was: List<float>, eye: Vec3.t): List<float> =>
  qualifying(town, town.solidOn, town.solidOn + solidMargin, was, eye)
    |> List.map((e) => let (id, d) = e in id)

// The id of the copy nearest `eye` (the first, on a tie); copy 0 when none
// is within the town's `lightReach`.
let nearest = (town: Town, eye: Vec3.t): float =>
  let (best, _) = near(town, town.lightReach, eye)
    |> List.fold((acc, kd) =>
      let (_, d) = acc in
      let (_, dk) = kd in
      if dk < d then kd else acc, (0.0, 1000000.0)) in
  best

// ---- Lights --------------------------------------------------------------------

// The 8 forward-light slots left after the sun and ambient: the frame takes
// the point lights nearest the eye, from the copies within `lightReach`.
let pointSlots = 6.0
// Of `lights` (each with its world position), the `pointSlots` nearest `eye`,
// in their original order (so the office alone lights exactly as before).
let nearestLights = (eye: Vec3.t, lights: List<(Vec3.t, Light.t)>): List<Light.t> =>
  lights |> List.indexedMap((i, l) => let (at, light) = l in (i, Vec3.length(at |> Vec3.sub(eye)), light))
    |> List.sortBy((e) => let (i, d, light) = e in d) |> List.take(pointSlots)
    |> List.sortBy((e) => let (i, d, light) = e in i) |> List.map((e) => let (i, d, light) = e in light)
