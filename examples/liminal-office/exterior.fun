// The street side, not lit by the bake and not hashed into its provenance:
// the facade's outer half, window bands, neon, the canopy, sodium lamps, the
// apron and the ground, and their colliders.

open Layout
open Building

let concrete = rgb(72.0, 72.0, 82.0)
let glass = rgb(26.0, 32.0, 44.0)

let slab = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float) =>
  Scene.cube() |> Scene.scaleXYZ(x1 - x0, y1 - y0, z1 - z0) |> Scene.translate(v((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0))

// A level's facade openings on one side, 1 cm smaller, so the facade's
// outer half overlaps the baked reveal's edges: no seam to see through, and
// no coplanar faces.
let facadeHoles = (level: float, s: Side): List<Hole> =>
  openingsIn(level, s)
    |> List.map((h) => { lo: h.lo + 0.01, hi: h.hi - 0.01, bottom: if h.bottom > 0.0 then h.bottom + 0.01 else 0.0, top: h.top - 0.01 })

// The outer half of the facade (the inner half is the baked interior wall),
// a band per storey; the top one rises to the roofline.
let half = facade / 2.0
let facadeSlabs =
  let f = footprint in
  levels |> List.concatMap((level) =>
    let y = level * storey in
    let top = facadeTop(level) in
    let run = (s: Side, s0: float, s1: float, make: (Piece) => Scene.t) =>
      pieces(s0, s1, top, facadeHoles(level, s)) |> List.map((p) => make({ p with y0: y + p.y0, y1: y + p.y1 })) in
    List.flatten([
      run(South, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z0 - half, p.u1, p.y1, f.z0)),
      run(North, f.x0 - half, f.x1 + half, (p) => slab(p.u0, p.y0, f.z1, p.u1, p.y1, f.z1 + half)),
      run(West, f.z0, f.z1, (p) => slab(f.x0 - half, p.y0, p.u0, f.x0, p.y1, p.u1)),
      run(East, f.z0, f.z1, (p) => slab(f.x1, p.y0, p.u0, f.x1 + half, p.y1, p.u1)),
    ])) |> Scene.group |> Scene.lit(concrete)

// Whether the 2.2 m band centred `at` along side `s` overlaps a real window
// on `level`.
let isOpen = (level: float, s: Side, at: float): bool =>
  Layout.windows |> List.any((w) =>
    roomAt(w.room).level == level && w.side == s && Math.abs(w.at - at) < (w.width + 2.2) / 2.0)
// Dark window bands on the outside of the walls, a row per storey, except
// where layout.fun opens a real window (open, no glass: glass would bypass
// culling). The ground floor's south row leaves the entrance clear.
let windowBands =
  let f = footprint in
  let o = half + 0.02 in
  levels |> List.concatMap((level) =>
    let y0 = level * storey + 1.0 in
    let y1 = level * storey + 2.3 in
    let south = if level == 0.0 then [2.0, 4.6, 15.4, 18.0] else [2.0, 4.6, 7.2, 12.8, 15.4, 18.0] in
    List.flatten([
      south |> List.filter((x) => not isOpen(level, South, x))
        |> List.map((x) => slab(x - 1.1, y0, f.z0 - o, x + 1.1, y1, f.z0 - half)),
      List.range(8.0) |> List.concatMap((k) =>
        let z = 2.5 + 3.4 * k in
        List.flatten([
          if isOpen(level, West, z) then [] else [slab(f.x0 - o, y0, z - 1.1, f.x0 - half, y1, z + 1.1)],
          if isOpen(level, East, z) then [] else [slab(f.x1 + half, y0, z - 1.1, f.x1 + o, y1, z + 1.1)],
        ])),
    ])) |> Scene.group |> Scene.lit(glass)

let entrance = doors |> List.find((d) => not isInside(roomAt(d.a))) |> Option.map(holeOf)
  |> Option.defaultValue({ lo: 0.0, hi: 0.0, bottom: 0.0, top: 0.0 })
let canopyX0 = entrance.lo - 1.0
let canopyX1 = entrance.hi + 1.0
let canopyZ0 = footprint.z0 - half - 1.6
let canopy = slab(canopyX0, entrance.top + 0.3, canopyZ0, canopyX1, entrance.top + 0.45, footprint.z0 - half) |> Scene.lit(concrete)

// Cyan along the roofline and the canopy's edge.
let neonStrips =
  let f = footprint in
  let y0 = facadeHeight - 0.12 in
  let y1 = facadeHeight - 0.06 in
  let o = half + 0.05 in
  Scene.group([
    slab(canopyX0, entrance.top + 0.33, canopyZ0 - 0.05, canopyX1, entrance.top + 0.39, canopyZ0),
    slab(f.x0 - o, y0, f.z0 - o, f.x1 + o, y1, f.z0 - half),
    slab(f.x0 - o, y0, f.z1 + half, f.x1 + o, y1, f.z1 + o),
    slab(f.x0 - o, y0, f.z0 - o, f.x0 - half, y1, f.z1 + o),
    slab(f.x1 + half, y0, f.z0 - o, f.x1 + o, y1, f.z1 + o),
  ])

// Sodium street lamps by the apron.
let lampHeads = [v(4.4, 4.8, -4.0), v(15.6, 4.8, -4.0)]
let lampPosts = lampHeads |> List.map((p) =>
  slab(Vec3.x(p) - 0.06, 0.0, Vec3.z(p) - 0.06, Vec3.x(p) + 0.06, 4.9, Vec3.z(p) + 0.06)) |> Scene.group |> Scene.lit(rgb(60.0, 60.0, 66.0))
let lampShades = lampHeads |> List.map((p) =>
  slab(Vec3.x(p) - 0.25, 4.75, Vec3.z(p) - 0.12, Vec3.x(p) + 0.25, 4.85, Vec3.z(p) + 0.12)) |> Scene.group

let apron = roomAt(0.0).rect
// The street sits 3 cm below the interior floors, so indoors the floor
// trimesh, not the ground box, carries the player.
let street = 0.0 - 0.03
// The ground, shared by every building on it: below the interior floors
// (y = 0), which would otherwise z-fight it.
let ground = Scene.plane() |> Scene.scaleXYZ(240.0, 1.0, 240.0) |> Scene.translate(v(10.0, street - 0.02, 10.0)) |> Scene.lit(rgb(40.0, 36.0, 46.0))
let apronSlab = slab(apron.x0, street - 0.02, apron.z0, apron.x1, street, apron.z1 - half) |> Scene.lit(rgb(96.0, 94.0, 102.0))

// The building's street side, in its own space, with the neon and the lamp
// heads glowing at `neon` and `lamps` (1 at night): only those two colours
// change with the hour. The ground isn't part of it.
let exteriorAt = (neon: float, lamps: float): Scene.t => Scene.group([
  apronSlab, facadeSlabs, windowBands, canopy, lampPosts,
  neonStrips |> Scene.color(rgb(110.0 * neon, 240.0 * neon, 230.0 * neon)),
  lampShades |> Scene.color(Hours.color(Hours.mix(shadeByDay, sodiumGlow, lamps))),
])
// Off by day: about the grey the lit posts show at noon.
let shadeByDay: Hours.Rgb = { r: 110.0 / 255.0, g: 110.0 / 255.0, b: 116.0 / 255.0 }
let sodiumGlow: Hours.Rgb = { r: 255.0 / 255.0, g: 190.0 / 255.0, b: 110.0 / 255.0 }

let groundBody = Physics.fixed(Physics.tag("ground"), Physics.box(240.0, 1.0, 240.0)) |> Physics.at(v(10.0, street - 0.5, 10.0))
// The lamp posts' colliders, in the building's space.
let lampColliders: List<Collider> = lampHeads |> List.indexedMap((i, p) =>
  { tag: $"lamp-{i}", shape: Physics.box(0.12, 4.9, 0.12), at: v(Vec3.x(p), 2.45, Vec3.z(p)) })
