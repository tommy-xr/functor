// The parks kit's checks (parks.fun; island.mjs places the island's).

let v = Vec3.make
let island = Parks.islandParks
let town = Parks.townParks
let plazasOf = (ps: List<Parks.Placement>): List<Parks.Placement> => ps |> List.filter(Parks.isPlaza)
let count = (ps: List<Parks.Placement>, f: (Parks.Placement) => bool): float => List.length(ps |> List.filter(f))

// The island: one plaza and 1–2 parks; the town: one of each. One monolith
// in each.
expect List.length(plazasOf(island)) == 1.0 && List.length(island) >= 2.0 && List.length(island) <= 3.0
expect List.length(plazasOf(town)) == 1.0 && List.length(town) == 2.0
expect count(island, (p) => p.monolith) == 1.0 && count(town, (p) => p.monolith) == 1.0
expect List.length(Parks.all |> List.map((p) => p.name) |> List.fold((m, n) => Map.insert(n, true, m), Map.empty()) |> Map.keys) == List.length(Parks.all)

// Points 2 m apart over a placement's rect, in the world.
let samplesOf = (p: Parks.Placement): List<(float, float)> =>
  let s = Parks.sizeOf(p) in
  List.range(s.w / 2.0 + 1.0) |> List.concatMap((i) => List.range(s.d / 2.0 + 1.0) |> List.map((j) =>
    let w = Parks.toWorld(p, v(2.0 * i - s.w / 2.0, 0.0, 2.0 * j - s.d / 2.0)) in (Vec3.x(w), Vec3.z(w))))
// How far (x, z) is from the island's pad (0 on it).
let fromPad = Tests.fromPad
// Within 450 m of the town's middle, every point past the pad's collars.
let padMiddle = ((IslandLayout.pad.x0 + IslandLayout.pad.x1) / 2.0, (IslandLayout.pad.z0 + IslandLayout.pad.z1) / 2.0)
expect island |> List.all((p) =>
  let (mx, mz) = padMiddle in
  Math.sqrt((p.x - mx) * (p.x - mx) + (p.z - mz) * (p.z - mz)) <= 450.0
  && (samplesOf(p) |> List.all((e) => let (x, z) = e in fromPad(x, z) >= 35.0)))
// Flat enough: nothing sinks more than 30 cm, and a quarter turn each.
expect island |> List.all((p) => p.sink > 0.0 && p.sink <= 0.3 && Math.mod(p.yaw, 90.0) == 0.0)
// Clear of the trees and rocks, and of the island e2e's walk (down the
// south track from the street), its hillside and its grove edge.
expect island |> List.all((p) =>
  List.flatten([IslandLayout.trees |> List.map((t) => (t.x, t.z)), IslandLayout.rocks |> List.map((r) => (r.x, r.z))])
    |> List.all((e) => let (x, z) = e in Parks.fromPark(p, x, z) >= 8.0))
let e2eSpots = List.flatten([List.range(11.0) |> List.map((k) => (-12.0, -50.0 - 10.0 * k)), [(142.0, 374.0), (-64.0, -270.0)]])
expect island |> List.all((p) => e2eSpots |> List.all((e) => let (x, z) = e in Parks.fromPark(p, x, z) >= 10.0))
// The night map's park pools are game.fun's park lights, in order.
let islandLamps = island |> List.concatMap(Parks.lampsOf)
let toMm = Tests.toMm
expect List.length(IslandLayout.parkLamps) == List.length(islandLamps)
expect IslandLayout.parkLamps |> List.zip(islandLamps) |> List.all((pair) =>
  let (l, at) = pair in
  toMm(l.x, Vec3.x(at)) && toMm(l.y, Vec3.y(at)) && toMm(l.z, Vec3.z(at)) && l.r == Parks.parkLight.r && l.g == Parks.parkLight.g
  && l.b == Parks.parkLight.b && l.intensity == Parks.parkLight.intensity && l.range == Parks.parkLight.range)

// The town's stand on the block's ground, 10 m or more off its pad (the
// copies, their sidewalks and apron), and clear of the block's streets'
// e2e views (9, 0's copy) but for the plaza's approach.
expect town |> List.all((p) => samplesOf(p) |> List.all((e) =>
  let (x, z) = e in fromPad(x, z) >= 10.0 && Math.abs(x - Block.centre) < Block.groundSize / 2.0 && Math.abs(z - Block.centre) < Block.groundSize / 2.0))

// Every park's walks stay clear: nothing solid within a path's half width
// (and 10 cm) of a park path's centre line, nor within the streetscape's
// clearance of a plaza's walk: (park, walk, solid's middle) for each that
// does, none.
let blocked = Parks.all |> List.concatMap((p) =>
  let c = Parks.contentAt(p) in
  let m = if Parks.isPlaza(p) then Streetscape.clearance else Parks.pathWidth / 2.0 + 0.1 in
  c.walks |> List.indexedMap((i, w) => c.solid |> List.filter((t) => Parks.blocking(m, w, t)) |> List.map((t) => (p.name, i, t.x, t.z))) |> List.flatten)
expect blocked == []
// The box's footprint corners, for clearance checks.
let cornersOf = (t: Parks.Turned): List<(float, float)> =>
  let c = Math.cos(t.yaw) in
  let s = Math.sin(t.yaw) in
  [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] |> List.map((e) =>
    let (a, b) = e in
    let lx = a * t.w / 2.0 in
    let lz = b * t.d / 2.0 in
    (t.x + lx * c + lz * s, t.z + lz * c - lx * s))
// Its footprint's bounds, axis-aligned.
let boundsOf = (t: Parks.Turned): Layout.Rect =>
  let cs = cornersOf(t) in
  let xs = cs |> List.map((e) => let (x, _) = e in x) in
  let zs = cs |> List.map((e) => let (_, z) = e in z) in
  Layout.rect(List.minimum(xs) |> Option.defaultValue(t.x), List.minimum(zs) |> Option.defaultValue(t.z),
       List.maximum(xs) |> Option.defaultValue(t.x), List.maximum(zs) |> Option.defaultValue(t.z))
// Everything solid stands inside its rect, so a park is solid only near it.
expect Parks.all |> List.all((p) =>
  let s = Parks.sizeOf(p) in
  Parks.contentAt(p).solid |> List.all((t) => let b = boundsOf(t) in
    b.x0 >= 0.0 - s.w / 2.0 - 0.01 && b.x1 <= s.w / 2.0 + 0.01 && b.z0 >= 0.0 - s.d / 2.0 - 0.01 && b.z1 <= s.d / 2.0 + 0.01))
// The park town's views stand clear of every solid (a capsule's radius).
let viewClear = (e: (float, float, float, float)): bool =>
  let (x, z, _, _) = e in
  town |> List.all((p) =>
    let (lx, lz) = Block.turn(0.0 - p.yaw, x - p.x, z - p.z) in
    not (Parks.contentAt(p).solid |> List.any((t) => Parks.blocking(0.35, { x0: lx, z0: lz, x1: lx, z1: lz }, t))))
expect List.length(Parks.views) == 10.0 && (Parks.views |> List.all(viewClear))

// Caps, per park: instanced boxes, colliders, lamps (the frame lights the
// six nearest), and decals.
expect Parks.all |> List.all((p) =>
  let c = Parks.contentAt(p) in
  List.length(c.lit) + List.length(c.paved) + List.length(c.glow) <= 700.0 && List.length(c.solid) <= 80.0
  && List.length(c.lamps) <= 8.0 && List.length(c.decals) <= 8.0)
// A draw per material: every paved box is the paving finish, so a park draws
// its lit cube, one finish, its globes and its signs (and a plaza its jets).
expect Parks.all |> List.all((p) => Parks.contentAt(p).paved |> List.all((e) => let (_, s) = e in s.finish.name == Finishes.paving.name))
// On the island every upright box on the ground reaches `sink` below it;
// on the flat ground nothing is sunk.
let bottomOf = (t: Parks.Turned): float => t.y - t.h / 2.0
expect island |> List.all((p) => Parks.contentAt(p).lit |> List.all((e) =>
  let (t, _) = e in t.tilt != 0.0 || bottomOf(t) > 0.0001 || bottomOf(t) <= 0.0001 - p.sink))
expect town |> List.all((p) => p.sink == 0.0)

// The octagon's strips cover it: a point just inside each face and corner
// lies in one of them.
let fountain = Parks.parkFountain
let inStrip = (x: float, z: float): bool =>
  Parks.octagon(fountain, 0.0, 0.1, Parks.water) |> List.any((e) =>
    let (t, _) = e in Parks.blocking(0.0, { x0: x, z0: z, x1: x, z1: z }, t))
let r8 = fountain.a / Math.cos(Math.pi / 8.0)
expect List.range(16.0) |> List.all((k) =>
  let a = k * Math.pi / 8.0 in
  let r = (if Math.mod(k, 2.0) == 0.0 then fountain.a else r8) - 0.02 in
  inStrip(r * Math.sin(a), r * Math.cos(a)))
// ... and stay inside the rim: no strip reaches past its outer face.
expect Parks.octagon(fountain, 0.0, 0.1, Parks.water) |> List.all((e) =>
  let (t, _) = e in cornersOf(t) |> List.all((c) => let (x, z) = c in Math.sqrt(x * x + z * z) <= r8 + 0.001))

// Jets: 1.3–1.9 m, a cycle of `jetSteps`, the same after one.
let cycle = Parks.jetSteps / Parks.jetRate
expect List.range(Parks.jetSteps) |> List.all((s) => List.range(Parks.jetCount) |> List.all((k) =>
  let h = Parks.jetHeight(k, s) in h >= 1.3 && h <= 1.9))
expect [0.0, 0.4, 1.3, 5.7] |> List.all((t) => Parks.jetStepAt(t) == Parks.jetStepAt(t + cycle))

// Colliders only near: none from the middle of the block, the park's from
// inside it.
let at = (x: float, z: float): Physics.position => { x: x, y: 1.0, z: z }
expect List.length(Parks.bodiesNear(town, at(-12.0, -12.0))) == 0.0
expect List.length(Parks.bodiesNear(town, at(-100.0, -12.0))) == 1.0

// The monolith, reusable on any street side: a slab, its plinth, two seams
// of light and one collider.
let slab = Parks.monolith({ side: Layout.South, x: 0.0, z: 0.0 }, 0.0, 0.0, 0.0)
expect List.length(slab.plain) == 2.0 && List.length(slab.decals) == 2.0 && List.length(slab.solid) == 1.0
