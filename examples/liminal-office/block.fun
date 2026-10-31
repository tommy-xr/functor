// The block: the office as an archetype, built ONCE in its own space (its
// cell models and lightmaps in game.fun, portals.fun's cells, building.fun's
// and exterior.fun's colliders, exterior.fun's street side), placed as four
// copies at 0/90/180/270° yaw around a crossroads. Every copy draws the same
// `Model.static` values, so the GPU holds one office.
//
// A copy is translation plus a yaw about Y, unit scale. Its interior draws
// only within `activeOn` of its footprint (staying on out to `activeOff`),
// where portals.fun's collar holds the eye.

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
let street = 24.0
let centre = 0.0 - street / 2.0
let copyAt = (k: float): Copy =>
  let yaw = 90.0 * k in
  let (x, z) = turn(yaw, centre, centre) in
  { name: $"office-{k}", x: centre - x, z: centre - z, yaw: yaw }
let copies = List.range(4.0) |> List.map(copyAt)
let home = copyAt(0.0)
// The copies standing: the office alone, or the block.
let placed = (block: bool): List<Copy> => if block then copies else [home]

let sceneOf = (c: Copy, s: Scene.t): Scene.t =>
  s |> Scene.rotateY(Angle.degrees(c.yaw)) |> Scene.translate(v(c.x, 0.0, c.z))

// A copy's colliders: building.fun's and the lamp posts, each a fixed body
// under its own tag.
let tagOf = (c: Copy, local: string): string => $"{c.name}/{local}"
let bodiesOf = (c: Copy): List<Physics.body> =>
  List.flatten([Building.colliders, Exterior.lampColliders]) |> List.map((k) =>
    Physics.fixed(Physics.tag(tagOf(c, k.tag)), k.shape)
      |> Physics.at(toWorld(c, k.at)) |> Physics.rotateY(Angle.degrees(c.yaw)))

// ---- Activation ----------------------------------------------------------------

// A copy's interior draws from `activeOn` metres of its footprint, and stays
// on until `activeOff`, so walking the edge doesn't flicker it.
let activeOn = 30.0
let activeOff = 34.0
// How far an eye may sit from the body the radius is measured from: the
// stereo half-IPD and a lean. The collar covers `activeOff` plus this.
let eyeReach = 1.0

// How far a local point is from the footprint (0 inside it).
let fromFootprint = (p: Vec3.t): float =>
  let f = footprint in
  let dx = Math.max(0.0, Math.max(f.x0 - Vec3.x(p), Vec3.x(p) - f.x1)) in
  let dz = Math.max(0.0, Math.max(f.z0 - Vec3.z(p), Vec3.z(p) - f.z1)) in
  Math.sqrt(dx * dx + dz * dz)

// Which copies' interiors draw for an eye at `eye`, given which did last
// frame (`was`, a flag per copy in `copies`). Unplaced copies are off.
let activeAt = (block: bool, was: List<bool>, eye: Vec3.t): List<bool> =>
  copies |> List.indexedMap((k, c) =>
    let d = fromFootprint(toLocal(c, eye)) in
    let on = List.nth(k, was) |> Option.defaultValue(false) in
    (block || k == 0.0) && (d < activeOn || (on && d <= activeOff)))
let nothingActive = copies |> List.map((c) => false)

// How many active copies' cells don't hold `eye`: each would draw its whole
// interior (`Scene.cells`' fallback). Zero by construction; the e2e checks it.
let uncovered = (active: List<bool>, eye: Vec3.t): float =>
  copies |> List.zip(active) |> List.filter((pair) =>
    let (c, on) = pair in on && Cells.locate(toLocal(c, eye), Portals.level) == Option.None) |> List.length

// The placed copy nearest `eye`, and its index.
let nearest = (block: bool, eye: Vec3.t): float =>
  let (best, _) = List.range(if block then 4.0 else 1.0) |> List.fold((acc, k) =>
    let (b, d) = acc in
    let dk = fromFootprint(toLocal(copyAt(k), eye)) in
    if dk < d then (k, dk) else acc, (0.0, 1000000.0)) in
  best
