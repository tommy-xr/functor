// The street side, not lit by the bake and not hashed into its provenance:
// the facade's outer half, window bands, neon, the canopy, sodium lamps, the
// apron and the ground, and their colliders.

open Layout
open Building

let concrete = rgb(72.0, 72.0, 82.0)
let glass = rgb(26.0, 32.0, 44.0)
let cyan = rgb(110.0, 240.0, 230.0)

let slab = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float) =>
  Scene.cube() |> Scene.scaleXYZ(x1 - x0, y1 - y0, z1 - z0) |> Scene.translate(v((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0))

// Doors through the facade along one footprint side: the holes in the
// inside rooms' walls on that side.
let facadeHoles = (s: Side): List<Hole> =>
  List.range(List.length(rooms))
    |> List.filter((i) => isInside(roomAt(i)) && onFootprint(s, roomAt(i).rect))
    |> List.concatMap((i) => holesIn(i, s))
    // 1 cm back from the reveal, so the baked jambs and head cover the joint.
    |> List.map((h) => { lo: h.lo - 0.01, hi: h.hi + 0.01, top: h.top + 0.01 })
    |> List.sortBy((h) => h.lo)

// The outer half of the facade (the inner half is the baked interior wall).
let half = facade / 2.0
let facadeSlabs =
  let f = footprint in
  let run = (s: Side, s0: float, s1: float, make: (Piece) => Scene.t) =>
    pieces(s0, s1, facadeHeight, facadeHoles(s)) |> List.map(make) in
  List.flatten([
    run(South, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z0 - half, p.u1, p.y1, f.z0)),
    run(North, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z1, p.u1, p.y1, f.z1 + half)),
    run(West, f.z0, f.z1, (p) => slab(f.x0 - half, p.y0, p.u0, f.x0, p.y1, p.u1)),
    run(East, f.z0, f.z1, (p) => slab(f.x1, p.y0, p.u0, f.x1 + half, p.y1, p.u1)),
  ]) |> Scene.group |> Scene.lit(concrete)

// Dark window bands on the outside of the walls (no apertures: open windows
// would need portals, and glass would bypass culling).
let windows =
  let f = footprint in
  let o = half + 0.02 in
  List.flatten([
    [2.0, 4.6, 15.4, 18.0] |> List.map((x) => slab(x - 1.1, 1.0, f.z0 - o, x + 1.1, 2.3, f.z0 - half)),
    List.range(8.0) |> List.concatMap((k) =>
      let z = 2.5 + 3.4 * k in
      [slab(f.x0 - o, 1.0, z - 1.1, f.x0 - half, 2.3, z + 1.1),
       slab(f.x1 + half, 1.0, z - 1.1, f.x1 + o, 2.3, z + 1.1)]),
  ]) |> Scene.group |> Scene.lit(glass)

let entrance = doors |> List.find((d) => not isInside(roomAt(d.a))) |> Option.map(holeOf)
  |> Option.defaultValue({ lo: 0.0, hi: 0.0, top: 0.0 })
let canopy =
  let x0 = entrance.lo - 1.0 in
  let x1 = entrance.hi + 1.0 in
  let z0 = footprint.z0 - half - 1.6 in
  Scene.group([
    slab(x0, entrance.top + 0.3, z0, x1, entrance.top + 0.45, footprint.z0 - half) |> Scene.lit(concrete),
    slab(x0, entrance.top + 0.33, z0 - 0.05, x1, entrance.top + 0.39, z0) |> Scene.color(cyan),
  ])

// Cyan along the roofline.
let roofline =
  let f = footprint in
  let y0 = facadeHeight - 0.12 in
  let y1 = facadeHeight - 0.06 in
  let o = half + 0.05 in
  Scene.group([
    slab(f.x0 - o, y0, f.z0 - o, f.x1 + o, y1, f.z0 - half),
    slab(f.x0 - o, y0, f.z1 + half, f.x1 + o, y1, f.z1 + o),
    slab(f.x0 - o, y0, f.z0 - o, f.x0 - half, y1, f.z1 + o),
    slab(f.x1 + half, y0, f.z0 - o, f.x1 + o, y1, f.z1 + o),
  ]) |> Scene.color(cyan)

// Sodium street lamps by the apron.
let lampHeads = [v(4.4, 4.8, -4.0), v(15.6, 4.8, -4.0)]
let lampPosts = lampHeads |> List.map((p) =>
  let x = Vec3.x(p) in
  let z = Vec3.z(p) in
  Scene.group([
    slab(x - 0.06, 0.0, z - 0.06, x + 0.06, 4.9, z + 0.06) |> Scene.lit(rgb(60.0, 60.0, 66.0)),
    slab(x - 0.25, 4.75, z - 0.12, x + 0.25, 4.85, z + 0.12) |> Scene.color(rgb(255.0, 190.0, 110.0)),
  ])) |> Scene.group

let apron = roomAt(0.0).rect
// The street sits 3 cm below the interior floors, so indoors the floor
// trimesh, not the ground box, carries the player.
let street = 0.0 - 0.03
let ground = Scene.group([
  // Below the interior floors (y = 0), which would otherwise z-fight it.
  Scene.plane() |> Scene.scaleXYZ(240.0, 1.0, 240.0) |> Scene.translate(v(10.0, street - 0.02, 10.0)) |> Scene.lit(rgb(40.0, 36.0, 46.0)),
  slab(apron.x0, street - 0.02, apron.z0, apron.x1, street, apron.z1 - half) |> Scene.lit(rgb(96.0, 94.0, 102.0)),
])

let exterior = Scene.group([ground, facadeSlabs, windows, canopy, roofline, lampPosts])

let streetBodies = (rev: float): List<Physics.body> =>
  List.flatten([
    [Physics.fixed(Physics.tag("ground"), Physics.box(240.0, 1.0, 240.0)) |> Physics.at(v(10.0, street - 0.5, 10.0))],
    lampHeads |> List.indexedMap((i, p) =>
      Physics.fixed(Physics.tag($"lamp-{i}"), Physics.box(0.12, 4.9, 0.12)) |> Physics.at(v(Vec3.x(p), 2.45, Vec3.z(p)))),
  ])
