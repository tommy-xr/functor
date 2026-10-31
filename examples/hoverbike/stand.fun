// stand.fun — the flat test stand: a walled 60 × 250 m pad, a lane of
// low crates the bike hovers over, pillars and a barrier it can't.
//
// Every piece is a box (centre and full extents, metres), drawn and declared
// as a fixed body from the same list. Coordinates are local: the stand sits
// `originX` along +x, clear of the dune course. The open lane x ∈ [0, 15] runs
// north from the spawn along the east wall, which `e2e/hoverbike.mjs` slides
// along; x ∈ [-20, 15] south of z = 100 is clear for its test turn.

let originX = 600.0

type Kind =
  | Floor
  | Wall
  | Crate
  | Pillar

type Block = { x: float, y: float, z: float, w: float, h: float, d: float, kind: Kind }

let block = (kind: Kind, x: float, y: float, z: float, w: float, h: float, d: float): Block =>
  { x: x, y: y, z: z, w: w, h: h, d: d, kind: kind }
// A box standing on the ground.
let standing = (kind: Kind, x: float, z: float, w: float, h: float, d: float) =>
  block(kind, x, h / 2.0, z, w, h, d)

let eastWall = 15.0 // x of the east wall's inner face

let blocks = [
  block(Floor, -15.0, -0.5, 85.0, 70.0, 1.0, 260.0),
  standing(Wall, eastWall + 0.5, 85.0, 1.0, 3.0, 252.0),
  standing(Wall, -45.5, 85.0, 1.0, 3.0, 252.0),
  standing(Wall, -15.0, -40.5, 62.0, 3.0, 1.0),
  standing(Wall, -15.0, 210.5, 62.0, 3.0, 1.0),
  // Low crates: the probes ride up over them, the collider passes above.
  standing(Crate, -26.0, 20.0, 2.0, 0.5, 2.0),
  standing(Crate, -30.0, 32.0, 2.0, 0.5, 2.0),
  standing(Crate, -26.0, 44.0, 2.0, 0.5, 2.0),
  standing(Crate, -30.0, 56.0, 3.0, 0.7, 3.0),
  // Tall enough to stop the bike.
  standing(Pillar, -38.0, 26.0, 1.5, 5.0, 1.5),
  standing(Pillar, -36.0, 70.0, 1.5, 5.0, 1.5),
  standing(Pillar, -24.0, 105.0, 1.5, 5.0, 1.5),
  standing(Wall, -28.0, 150.0, 14.0, 2.0, 0.8),
]

// The floor (the first block)'s tag: a dismount lands only on it, and only
// inside the walls.
let floorTag = Physics.tag("stand0")
let inside = (x: float, z: float) => x > originX - 45.0 && x < originX + eastWall && z > -40.0 && z < 210.0

// Declared every frame from this one value, so the world never changes.
let bodies = blocks |> List.indexedMap((i, b) =>
  Physics.fixed(Physics.tag($"stand{i}"), Physics.box(b.w, b.h, b.d))
    |> Physics.at(Vec3.make(originX + b.x, b.y, b.z)))

let colorOf = (kind: Kind) =>
  match kind with
  | Floor => Color.rgb(0.1, 0.08, 0.14)
  | Crate => Color.rgb(0.45, 0.32, 0.5)
  | Pillar => Color.rgb(0.3, 0.28, 0.4)
  | Wall => Color.rgb(0.2, 0.18, 0.28)

let solid = (b: Block) =>
  Scene.cube() |> Scene.scaleXYZ(b.w, b.h, b.d) |> Scene.lit(colorOf(b.kind))
    |> Scene.translate(Vec3.make(b.x, b.y, b.z))

// A glowing edge along the top of every wall, crate and pillar.
let trim = (b: Block) =>
  Scene.cube() |> Scene.scaleXYZ(b.w + 0.02, 0.06, b.d + 0.02) |> Scene.emissive(Bike.ion)
    |> Scene.translate(Vec3.make(b.x, b.y + b.h / 2.0, b.z))

// A 10 m grid of thin lines on the floor, for a sense of speed.
let gridLines =
  (List.range(7.0) |> List.map((i) =>
     Instance.at(Vec3.make(-45.0 + 10.0 * i, 0.01, 85.0)) |> Instance.scaleXYZ(0.06, 0.01, 250.0)))
  |> List.append(List.range(26.0) |> List.map((i) =>
     Instance.at(Vec3.make(-15.0, 0.01, -40.0 + 10.0 * i)) |> Instance.scaleXYZ(60.0, 0.01, 0.06)))

let scene = Scene.group([
  Scene.group(blocks |> List.map(solid)),
  Scene.group(blocks |> List.filter((b) => b.kind != Floor) |> List.map(trim)),
  Scene.cube() |> Scene.emissive(Color.rgb(0.55, 0.25, 0.6)) |> Scene.instanced(gridLines),
])
  |> Scene.translate(Vec3.make(originX, 0.0, 0.0))
