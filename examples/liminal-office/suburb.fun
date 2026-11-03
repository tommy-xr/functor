// The suburb (U): twelve houses round a square green, three a side, each
// facing it across a loop road; palettes alternate. Only
// quarter-turn yaws: a mirrored copy would turn its faces inside out.
// Plain data and the ground, for block.fun's town.

// From the green's middle (the origin) to each house's front facade, and
// between neighbours' middles along a side.
let front = 30.0
let spacing = 16.0
let palettes = ["house-ochre", "house-magenta"]

// Side k's outward direction (its houses' local +z, at yaw 90k°) and the
// direction along it (their local +x).
let outward = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)]
let along = (u: (float, float)): (float, float) => let (ux, uz) = u in (uz, 0.0 - ux)

// Copy `id`'s (archetype, x, z, yaw): side id / 3, place id mod 3 along it.
// A copy maps a house's (5, 0), its front's middle, to `front` out along
// its side's direction and `t` along it.
let placements: List<(string, float, float, float)> =
  let half = (HouseLayout.footprint.x1 - HouseLayout.footprint.x0) / 2.0 in
  List.range(12.0) |> List.map((id) =>
    let k = Math.floor(id / 3.0) in
    let u = List.nth(k, outward) |> Option.defaultValue((0.0, 1.0)) in
    let (ux, uz) = u in
    let (wx, wz) = along(u) in
    let t = (Math.mod(id, 3.0) - 1.0) * spacing - half in
    (List.nth(Math.mod(id, 2.0), palettes) |> Option.defaultValue("house-ochre"), front * ux + t * wx, front * uz + t * wz, 90.0 * k))

// The road: a square loop between `inner` and `outer` from the middle,
// where every house's front path ends. The north and south runs span the
// corners; the east and west runs fit between them.
let inner = 15.0
let outer = 21.0
let asphalt = Color.rgb(0.17, 0.16, 0.19)
let width = outer - inner
let roadPieces: List<Instance.t> = outward |> List.map((u) =>
  let (ux, uz) = u in
  let mid = (inner + outer) / 2.0 in
  Instance.trs(Vec3.make(mid * ux, Exterior.street - 0.01, mid * uz), 0deg,
    if ux == 0.0 then 2.0 * outer else width, 0.02, if ux == 0.0 then width else 2.0 * inner))

// The ground reaches past the far clip from every house.
let size = 2.0 * (front + 12.0 + Hours.farthest(Hours.streetReach) + 20.0)
let ground = Scene.group([
  Scene.plane() |> Scene.scaleXYZ(size, 1.0, size) |> Scene.translate(Vec3.make(0.0, Exterior.street - 0.02, 0.0))
    |> Scene.lit(Color.rgb(0.19, 0.24, 0.16)),
  Scene.cube() |> Scene.lit(asphalt) |> Scene.instanced(roadPieces),
])
let groundBody = Exterior.groundBodyAt("suburb-ground", 0.0, 0.0, size)

// The suburb's views (game.fun), keys 1–9 then 0, as (copy, x, level, z,
// yaw, pitch) in a house's space: yaw 0 faces +z, the front door from the
// porch. 1–8 are in the nearest house: the front walk, the stair foot looking up,
// the living room, the kitchen, the landing looking down, the upstairs
// corridor, bedroom 3's window, the hall's niche. 9 is the road in front
// of copy 0 (ochre, so 1–8 jump into it), looking along past copy 1; 0 the
// stair foot in copy 1 (magenta).
let views = [
  (0.0, 1.6, 0.0, 0.0 - 9.0, 0.0, 0.05),
  (0.0, 2.6, 0.0, 0.75, 1.5708, 0.3),
  (0.0, 4.4, 0.0, 3.3, 0.0 - 1.05, 0.0 - 0.05),
  (0.0, 5.7, 0.0, 8.5, 0.6, 0.0),
  (0.0, 9.1, 1.0, 1.4, 0.0 - 1.5708, 0.0 - 0.45),
  (0.0, 9.2, 1.0, 3.6, 0.0 - 1.5708, 0.0),
  (0.0, 7.8, 1.0, 7.0, 1.5708, 0.0 - 0.05),
  (0.0, 1.3, 0.0, 0.7, 1.1, 0.1),
  (0.0, 6.0, 0.0, 0.0 - 12.0, 1.27, 0.02),
  (1.0, 2.6, 0.0, 0.75, 1.5708, 0.3),
]
