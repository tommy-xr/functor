// course.fun — the flat test stand: a walled 60 × 250 m pad, a lane of
// low crates the bike hovers over, pillars and a barrier it can't.
//
// Every piece is a box (centre and full extents, metres), drawn and declared
// as a fixed body from the same list. The open lane x ∈ [0, 15] runs north
// from the spawn along the east wall, which `e2e/hoverbike.mjs` slides along.

type Block = { x: float, y: float, z: float, w: float, h: float, d: float, kind: float }

let floorKind = 0.0
let wallKind = 1.0
let crateKind = 2.0
let pillarKind = 3.0

let block = (kind: float, x: float, y: float, z: float, w: float, h: float, d: float): Block =>
  { x: x, y: y, z: z, w: w, h: h, d: d, kind: kind }
// A box standing on the ground.
let standing = (kind: float, x: float, z: float, w: float, h: float, d: float) =>
  block(kind, x, h / 2.0, z, w, h, d)

let eastWall = 15.0 // x of the east wall's inner face

let blocks = [
  block(floorKind, -15.0, -0.5, 85.0, 70.0, 1.0, 260.0),
  standing(wallKind, eastWall + 0.5, 85.0, 1.0, 3.0, 252.0),
  standing(wallKind, -45.5, 85.0, 1.0, 3.0, 252.0),
  standing(wallKind, -15.0, -40.5, 62.0, 3.0, 1.0),
  standing(wallKind, -15.0, 210.5, 62.0, 3.0, 1.0),
  // Low crates: the probes ride up over them, the collider passes above.
  standing(crateKind, -10.0, 20.0, 2.0, 0.5, 2.0),
  standing(crateKind, -14.0, 32.0, 2.0, 0.5, 2.0),
  standing(crateKind, -10.0, 44.0, 2.0, 0.5, 2.0),
  standing(crateKind, -14.0, 56.0, 3.0, 0.7, 3.0),
  // Tall enough to stop the bike.
  standing(pillarKind, -26.0, 30.0, 1.5, 5.0, 1.5),
  standing(pillarKind, -32.0, 70.0, 1.5, 5.0, 1.5),
  standing(pillarKind, -22.0, 105.0, 1.5, 5.0, 1.5),
  standing(wallKind, -20.0, 150.0, 14.0, 2.0, 0.8),
]

// Declared every frame from this one value, so the world never changes.
let bodies = blocks |> List.indexedMap((i, b) =>
  Physics.fixed(Physics.tag($"course{i}"), Physics.box(b.w, b.h, b.d))
    |> Physics.at(Vec3.make(b.x, b.y, b.z)))

let colorOf = (kind: float) =>
  if kind == floorKind then Color.rgb(0.1, 0.08, 0.14)
  else if kind == crateKind then Color.rgb(0.45, 0.32, 0.5)
  else if kind == pillarKind then Color.rgb(0.3, 0.28, 0.4)
  else Color.rgb(0.2, 0.18, 0.28)

let solid = (b: Block) =>
  Scene.cube() |> Scene.scaleXYZ(b.w, b.h, b.d) |> Scene.lit(colorOf(b.kind))
    |> Scene.translate(Vec3.make(b.x, b.y, b.z))

// A glowing edge along the top of every wall, crate and pillar.
let trim = (b: Block) =>
  Scene.cube() |> Scene.scaleXYZ(b.w + 0.02, 0.06, b.d + 0.02) |> Scene.emissive(Color.rgb(0.35, 1.0, 0.9))
    |> Scene.translate(Vec3.make(b.x, b.y + b.h / 2.0, b.z))

// A 10 m grid of thin lines on the floor, for a sense of speed.
let gridLines =
  (List.range(7.0) |> List.map((i) =>
     Instance.at(Vec3.make(-45.0 + 10.0 * i, 0.01, 85.0)) |> Instance.scaleXYZ(0.06, 0.01, 250.0)))
  |> List.append(List.range(26.0) |> List.map((i) =>
     Instance.at(Vec3.make(-15.0, 0.01, -40.0 + 10.0 * i)) |> Instance.scaleXYZ(60.0, 0.01, 0.06)))

let scene = Scene.group([
  Scene.group(blocks |> List.map(solid)),
  Scene.group(blocks |> List.filter((b) => b.kind != floorKind) |> List.map(trim)),
  Scene.cube() |> Scene.emissive(Color.rgb(0.55, 0.25, 0.6)) |> Scene.instanced(gridLines),
])
