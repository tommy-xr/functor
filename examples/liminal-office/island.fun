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

// Sand at the shore, dry grass on the pad, greener up the hills. No snow:
// its band starts above the highest sample.
let terrain =
  Terrain.heightmap(Asset.texture("island.png"), IslandLayout.size, IslandLayout.size,
                    IslandLayout.minHeight, IslandLayout.maxHeight)
  // 4 px fits the Quest vertex budget.
  |> Terrain.maxPixelError(4.0)
  |> Terrain.layered(Color.rgb(0.50, 0.47, 0.38), Color.rgb(0.16, 0.24, 0.11),
                     Color.rgb(0.40, 0.38, 0.36), Color.rgb(0.9, 0.9, 0.9), IslandLayout.maxHeight + 1.0)

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

// The fog's far end, which is also the camera's far plane: 500 m by day,
// 200 m at night. The renderer skips terrain past it.
let fogFar = (daylight: float): float => 200.0 |> Math.lerp(500.0, daylight)
