// The island (I): the block on a 2 km island instead of the flat ground.
// island.mjs generates the heightmap and islandLayout.fun; this file places
// them. The terrain sits so its flat pad lands where the flat ground was,
// under the block, which stands on it unchanged.
//
//   node examples/liminal-office/island.mjs   # checked in; to regenerate

let v = Vec3.make

// The flat ground's height (`Exterior.groundAt`), where the pad lands.
let groundY = Exterior.street - 0.02
// The terrain's translation: its pad level to `groundY`.
let offset = v(IslandLayout.centre.x, groundY - IslandLayout.pad.y, IslandLayout.centre.z)

// Meadow low down, scree toward the peaks (the bands blend by height over
// the declared range), rock on steep faces, snow on the highest crests
// (190 m: the committed island's peak is 222 m).
let terrain =
  Terrain.heightmap(Asset.texture("island.png"), IslandLayout.size, IslandLayout.size,
                    IslandLayout.minHeight, IslandLayout.maxHeight)
  // 4 px fits the Quest vertex budget.
  |> Terrain.maxPixelError(4.0)
  |> Terrain.layered(Color.rgb(0.36, 0.38, 0.22), Color.rgb(0.40, 0.38, 0.33),
                     Color.rgb(0.33, 0.31, 0.29), Color.rgb(0.9, 0.9, 0.9), 190.0)

let sea = Scene.plane() |> Scene.scale(2.0 * IslandLayout.size)
  |> Scene.translate(v(IslandLayout.centre.x, IslandLayout.seaLevel + Vec3.y(offset), IslandLayout.centre.z))
  |> Scene.lit(Color.rgb(0.16, 0.23, 0.29))

// Drawn after the block (game.fun), so early-Z rejects terrain behind the
// buildings; the sea after the terrain, so the land rejects the sea under it.
let ground = Scene.group([Scene.terrain(terrain) |> Scene.translate(offset), sea])

// Whether (x, z) is on the pad, where the island's ground is the flat
// ground's height.
let onPad = (x: float, z: float): bool =>
  x >= IslandLayout.pad.x0 && x <= IslandLayout.pad.x1 && z >= IslandLayout.pad.z0 && z <= IslandLayout.pad.z1

// The heightfield, and the pad as a box flush with it: the heightfield's
// collider appears only once the heightmap decodes, and until then the box
// holds the walker.
let bodies = [
  Physics.heightfield(Physics.tag("island"), terrain) |> Physics.at(offset),
  Physics.fixed(Physics.tag("island-pad"),
                Physics.box(IslandLayout.pad.x1 - IslandLayout.pad.x0, 1.0, IslandLayout.pad.z1 - IslandLayout.pad.z0))
    |> Physics.at(v((IslandLayout.pad.x0 + IslandLayout.pad.x1) / 2.0, groundY - 0.5, (IslandLayout.pad.z0 + IslandLayout.pad.z1) / 2.0)),
]

// How far the island is seen (`Hours.Reach`): 500 m by day, 200 m at
// night. The renderer skips terrain past it.
let reach: Hours.Reach = { night: 200.0, day: 500.0 }

// ---- Props ---------------------------------------------------------------------
//
// islandLayout.fun places the trees and rocks. A tree is a fir card whose
// silhouette is cut out in geometry (no `discard`, so early depth rejection
// stays on): three planes within `meshDistance`, two beyond, both copied from
// night-terrain. A rock is a few lit boxes turned every way, a third buried,
// drawn within `rockDistance`. One `Scene.instanced` node per model: at
// most five draws. Nothing casts a shadow: the frame has no shadow light.

let firs = [
  { near: Asset.model("impostor-fir-a-near.glb"), far: Asset.model("impostor-fir-a-far.glb") },
  { near: Asset.model("impostor-fir-b-near.glb"), far: Asset.model("impostor-fir-b-far.glb") },
]
let rockColor = Color.rgb(0.42, 0.40, 0.37)

// The props drawn and solid from where the eye stood (x, z). Splitting walks
// every prop, so `regroup` redoes it only once the eye moves `regroupStep`
// from there, or its key changes: a near tree turns to a card, and a rock
// at `rockDistance` shows or goes, up to that much late.
let regroupStep = 4.0
// Props within this of the eye have colliders: trunks and rocks.
let solidReach = 15.0
// No prop reaches farther than this from its origin.
let propRadius = 5.0
type Tree = { id: float, x: float, y: float, z: float, yaw: float, size: float, variant: float }
type Rock = { id: float, x: float, y: float, z: float, yaw: float, tilt: float, roll: float, sx: float, sy: float, sz: float }
type Solid = { trees: List<Tree>, rocks: List<Rock> }
// What a split depends on besides the eye: the fog's end, and the tunables
// and layout, so a hot reload that edits them re-splits.
type SplitKey = { fog: float, mesh: float, rockDistance: float, reach: float, trees: float, rocks: float }
let keyOf = (fog: float): SplitKey =>
  { fog: fog, mesh: IslandLayout.props.meshDistance, rockDistance: IslandLayout.props.rockDistance,
    reach: solidReach, trees: List.length(IslandLayout.trees), rocks: List.length(IslandLayout.rocks) }
type Split = { x: float, z: float, key: SplitKey }
type Props = { at: Option.t<Split>, near: List<Tree>, far: List<Tree>, rocks: List<Rock>, solid: Solid }
let noProps: Props = { at: Option.None, near: [], far: [], rocks: [], solid: { trees: [], rocks: [] } }

// Whether a prop stands within d of (x, z).
let within = (x: float, z: float, d: float, p): bool =>
  (p.x - x) * (p.x - x) + (p.z - z) * (p.z - z) < d * d

// Off the island, nothing. Props past the fog (from anywhere within
// `regroupStep`) are dropped.
let regroup = (props: Props, on: bool, x: float, z: float, fog: float): Props =>
  if not on then noProps
  else
    let key = keyOf(fog) in
    let fresh = match props.at with
      | Option.Some(at) => at.key == key && within(x, z, regroupStep, at)
      | Option.None => false in
    if fresh then props
    else
      let mesh = IslandLayout.props.meshDistance in
      let shown = IslandLayout.trees |> List.filter((t) => within(x, z, fog + regroupStep + propRadius, t)) in
      let solid = solidReach + regroupStep + propRadius in
      { at: Option.Some({ x: x, z: z, key: key }),
        near: shown |> List.filter((t) => within(x, z, mesh, t)),
        far: shown |> List.filter((t) => not within(x, z, mesh, t)),
        rocks: IslandLayout.rocks |> List.filter((r) => within(x, z, IslandLayout.props.rockDistance, r)),
        solid: { trees: IslandLayout.trees |> List.filter((t) => within(x, z, solid, t)),
                 rocks: IslandLayout.rocks |> List.filter((r) => within(x, z, solid, r)) } }

// The layout's heights are the terrain's own; `offset` lifts them.
let lift = Vec3.y(offset)
// Trees sink a little so slopes never show a gap under the trunk.
let treeAt = (t: Tree) =>
  Instance.trs(v(t.x, t.y + lift - 0.3, t.z), Angle.radians(t.yaw), t.size, t.size, t.size)
let shade = (g: float) => Color.rgb(g, g, g)
// A rock's centre: 15% of its height over the ground, so about a third of
// the box is buried.
let rockCentre = (r: Rock) => v(r.x, r.y + lift + 0.15 * r.sy, r.z)
// Instances scale, then rotate (tilt, roll, then yaw), then move.
let rockAt = (r: Rock) =>
  Instance.at(rockCentre(r)) |> Instance.scaleXYZ(r.sx, r.sy, r.sz)
    |> Instance.rotateX(Angle.radians(r.tilt)) |> Instance.rotateZ(Angle.radians(r.roll))
    |> Instance.rotateY(Angle.radians(r.yaw))
    // A shade lighter or darker each, by id.
    |> Instance.tint(shade(0.8 + 0.25 * Math.mod(r.id * 0.618, 1.0)))

let ofVariant = (i: float, trees: List<Tree>): List<Tree> => trees |> List.filter((t) => t.variant == i)
let propsScene = (props: Props): Scene.t =>
  Scene.group(List.flatten([
    firs |> List.indexedMap((i, fir) => [
      Scene.model(fir.near) |> Scene.instanced(props.near |> ofVariant(i) |> List.map(treeAt)),
      Scene.model(fir.far) |> Scene.instanced(props.far |> ofVariant(i) |> List.map(treeAt)),
    ]) |> List.flatten,
    [Scene.cube() |> Scene.lit(rockColor) |> Scene.instanced(props.rocks |> List.map(rockAt))],
  ]))

// A trunk: a capsule 0.35 m × size round (wider than the painted trunk, so
// the eye stays out of the card's planes), 3 m × size tall. A rock: its box.
let trunk = (t: Tree) =>
  Physics.fixed(Physics.tag($"island-tree-{t.id}"), Physics.capsule(1.5 * t.size, 0.35 * t.size))
    |> Physics.at(v(t.x, t.y + lift + 1.5 * t.size, t.z))
let boulder = (r: Rock) =>
  Physics.fixed(Physics.tag($"island-rock-{r.id}"), Physics.box(r.sx, r.sy, r.sz))
    |> Physics.rotateX(Angle.radians(r.tilt)) |> Physics.rotateZ(Angle.radians(r.roll))
    |> Physics.rotateY(Angle.radians(r.yaw))
    |> Physics.at(rockCentre(r))
let propBodies = (solid: Solid) => List.flatten([solid.trees |> List.map(trunk), solid.rocks |> List.map(boulder)])
