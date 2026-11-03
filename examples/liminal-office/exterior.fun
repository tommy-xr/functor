// A layout's street side, not lit by the bake and not hashed into its
// provenance: the facade's outer half, window bands, the roof, neon, the
// canopy, sodium lamps and the apron, and their colliders; and the ground.
// `streetOf` derives them once per archetype.

open Layout
open Building

let concrete = rgb(72.0, 72.0, 82.0)
let glass = rgb(26.0, 32.0, 44.0)
let roofGrey = rgb(60.0, 58.0, 62.0)
let postGrey = rgb(60.0, 60.0, 66.0)
let apronGrey = rgb(96.0, 94.0, 102.0)

let slab = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float) =>
  Scene.cube() |> Scene.scaleXYZ(x1 - x0, y1 - y0, z1 - z0) |> Scene.translate(v((x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0))

// A slab as data, for shells.fun to instance: [x0, x1] × [y0, y1] × [z0, z1].
type Box = { x0: float, y0: float, z0: float, x1: float, y1: float, z1: float }
let box = (x0: float, y0: float, z0: float, x1: float, y1: float, z1: float): Box =>
  { x0: x0, y0: y0, z0: z0, x1: x1, y1: y1, z1: z1 }
let slabOf = (b: Box): Scene.t => slab(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1)

// Whether the 2.2 m band centred `at` along side `s` overlaps a real window
// on `level`.
let isOpen = (l: Layout, level: float, s: Side, at: float): bool =>
  l.windows |> List.any((w) =>
    roomAt(l, w.room).level == level && w.side == s && Math.abs(w.at - at) < (w.width + 2.2) / 2.0)
// The layout's dark window bands on `level` along side `s`, except where it
// opens a real window (open, no glass: glass would bypass culling). Each is
// a hole through the facade's outer half with a panel set back in it, so no
// face lies just behind another: two faces a few cm apart z-fight at a
// distance.
let bandHoles = (l: Layout, level: float, s: Side): List<Hole> =>
  l.bands |> List.filter((b) => b.level == level && b.side == s) |> List.map((b) => b.at)
    |> List.filter((at) => not isOpen(l, level, s, at)) |> List.map((at) => { lo: at - 1.1, hi: at + 1.1, bottom: 1.0, top: 2.3 })

// A level's facade holes on one side: the bands, and the real openings 1 cm
// smaller, so the facade's outer half overlaps the baked reveal's edges: no
// seam to see through, and no coplanar faces.
let facadeHoles = (l: Layout, level: float, s: Side): List<Hole> =>
  List.flatten([
    openingsIn(l, level, s)
      |> List.map((h) => { lo: h.lo + 0.01, hi: h.hi - 0.01, bottom: if h.bottom > 0.0 then h.bottom + 0.01 else 0.0, top: h.top - 0.01 }),
    bandHoles(l, level, s),
  ]) |> List.sortBy((h) => h.lo)

// The outer half of the facade (the inner half is the baked interior wall),
// a band per storey; the top one rises to the roofline.
let halfOf = (l: Layout): float => l.facade / 2.0
let facadeBoxesOf = (l: Layout): List<Box> =>
  let f = l.footprint in
  let half = halfOf(l) in
  levelsOf(l) |> List.concatMap((level) =>
    let y = level * l.storey in
    let top = facadeTop(l, level) in
    let run = (s: Side, s0: float, s1: float, make: (Piece) => Box) =>
      pieces(s0, s1, top, facadeHoles(l, level, s)) |> List.map((p) => make({ p with y0: y + p.y0, y1: y + p.y1 })) in
    List.flatten([
      run(South, f.x0 - half, f.x1 + half, (p) => box(p.u0, p.y0, f.z0 - half, p.u1, p.y1, f.z0)),
      run(North, f.x0 - half, f.x1 + half, (p) => box(p.u0, p.y0, f.z1, p.u1, p.y1, f.z1 + half)),
      run(West, f.z0, f.z1, (p) => box(f.x0 - half, p.y0, p.u0, f.x0, p.y1, p.u1)),
      run(East, f.z0, f.z1, (p) => box(f.x1, p.y0, p.u0, f.x1 + half, p.y1, p.u1)),
    ]))

// A band's panel, `recess` behind the facade's outer face, filling its hole
// back to the wall line and 2 cm past its edges (hidden in the facade).
let recess = 0.08
let bandBoxesOf = (l: Layout): List<Box> =>
  let f = l.footprint in
  let half = halfOf(l) in
  let e = 0.02 in
  levelsOf(l) |> List.concatMap((level) =>
    let y0 = level * l.storey + 1.0 - e in
    let y1 = level * l.storey + 2.3 + e in
    [South, North, West, East] |> List.concatMap((s) => bandHoles(l, level, s) |> List.map((h) =>
      match s with
      | South => box(h.lo - e, y0, f.z0 - half + recess, h.hi + e, y1, f.z0)
      | North => box(h.lo - e, y0, f.z1, h.hi + e, y1, f.z1 + half - recess)
      | West => box(f.x0 - half + recess, y0, h.lo - e, f.x0, y1, h.hi + e)
      | East => box(f.x1, y0, h.lo - e, f.x1 + half - recess, y1, h.hi + e))))

// The roof: a slab over the footprint, its top level with the top storey's
// slab, run into the facade's outer half so no seam shows. The facade rising
// on to the roofline is its parapet.
let roofBoxOf = (l: Layout): Box =>
  let f = l.footprint in
  let half = halfOf(l) in
  let top = l.levels * l.storey in
  box(f.x0 - half / 2.0, top - 0.3, f.z0 - half / 2.0, f.x1 + half / 2.0, top, f.z1 + half / 2.0)

// The entrance: the first door from an outside room (the apron).
let entranceDoorOf = (l: Layout): Option.t<Door> => l.doors |> List.find((d) => roomAt(l, d.a).kind == Outside)
let entranceOf = (l: Layout): Hole =>
  entranceDoorOf(l) |> Option.map(holeOf) |> Option.defaultValue({ lo: 0.0, hi: 0.0, bottom: 0.0, top: 0.0 })
// The canopy over the entrance, in the south facade.
let canopyBoxOf = (l: Layout): Box =>
  let entrance = entranceOf(l) in
  let half = halfOf(l) in
  box(entrance.lo - 1.0, entrance.top + 0.3, l.footprint.z0 - half - 1.6, entrance.hi + 1.0, entrance.top + 0.45, l.footprint.z0 - half)

// Cyan along the roofline and the canopy's edge.
let neonBoxesOf = (l: Layout): List<Box> =>
  let f = l.footprint in
  let half = halfOf(l) in
  let canopy = canopyBoxOf(l) in
  let entrance = entranceOf(l) in
  let y0 = l.facadeHeight - 0.12 in
  let y1 = l.facadeHeight - 0.06 in
  let o = half + 0.05 in
  [
    box(canopy.x0, entrance.top + 0.33, canopy.z0 - 0.05, canopy.x1, entrance.top + 0.39, canopy.z0),
    box(f.x0 - o, y0, f.z0 - o, f.x1 + o, y1, f.z0 - half),
    box(f.x0 - o, y0, f.z1 + half, f.x1 + o, y1, f.z1 + o),
    box(f.x0 - o, y0, f.z0 - o, f.x0 - half, y1, f.z1 + o),
    box(f.x1 + half, y0, f.z0 - o, f.x1 + o, y1, f.z1 + o),
  ]

// Sodium street lamps: a post under each of the layout's lamp heads.
let postBoxesOf = (l: Layout): List<Box> => l.streetLamps |> List.map((p) =>
  box(Vec3.x(p) - 0.06, 0.0, Vec3.z(p) - 0.06, Vec3.x(p) + 0.06, 4.9, Vec3.z(p) + 0.06))
// The street's point lights (game.fun): the lobby's light spilling onto the
// apron, and a sodium lamp `drop` under each head, off by day. island.mjs
// parses these lines to paint them into the island's night map.
let lobbyLight = { x: 10.0, y: 2.0, z: 0.9, r: 0.85, g: 0.95, b: 0.8, intensity: 1.2, range: 9.0 }
let sodiumLight = { drop: 0.2, r: 1.0, g: 0.68, b: 0.36, intensity: 0.9, range: 14.0 }
let shadeBoxesOf = (l: Layout): List<Box> => l.streetLamps |> List.map((p) =>
  box(Vec3.x(p) - 0.25, 4.75, Vec3.z(p) - 0.12, Vec3.x(p) + 0.25, 4.85, Vec3.z(p) + 0.12))

// The street sits 3 cm below the interior floors, so indoors the floor
// trimesh, not the ground box, carries the player.
let street = 0.0 - 0.03
// The apron: the entrance's outside room, paved up to the facade.
let apronBoxOf = (l: Layout): Box =>
  let apron = entranceDoorOf(l) |> Option.map((d) => roomAt(l, d.a).rect) |> Option.defaultValue(rect(0.0, 0.0, 0.0, 0.0)) in
  box(apron.x0, street - 0.02, apron.z0, apron.x1, street, apron.z1 - halfOf(l))

// A window grid on a facade, in a building's space: by night some bays lit,
// by day all dark glass.
type Grid = { night: Scene.t, day: Scene.t }

// The street side's lit boxes, by colour, as instances of one white cube: a
// single draw; and the neon and the shades, which glow. `extraBoxes` (lit,
// among `streetBoxes`) and `beaconBoxes` (red) are an archetype's own, on
// top of the kit's; `lobbyLight` is where the lobby's light spills out.
type Street = {
  half: float,
  facadeBoxes: List<Box>,
  bandBoxes: List<Box>,
  roofBox: Box,
  entrance: Hole,
  canopyBox: Box,
  apronBox: Box,
  postBoxes: List<Box>,
  neonBoxes: List<Box>,
  shadeBoxes: List<Box>,
  streetBoxes: List<(Box, Color.t)>,
  streetSide: Scene.t,
  neonStrips: Scene.t,
  lampShades: Scene.t,
  lampHeads: List<Vec3.t>,
  lampColliders: List<Collider>,
  lobbyLight: Vec3.t,
  extraBoxes: List<(Box, Color.t)>,
  beaconBoxes: List<Box>,
  grid: Option.t<Grid>,
}
// Lit boxes as instances of one white cube, tinted.
let instancedOf = (boxes: List<(Box, Color.t)>): Scene.t =>
  Scene.cube() |> Scene.lit(rgb(255.0, 255.0, 255.0)) |> Scene.instanced(boxes |> List.map((e) =>
    let (b, color) = e in
    Instance.trs(v((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0), 0deg, b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0)
      |> Instance.tint(color)))
let streetOf = (l: Layout): Street =>
  let facadeBoxes = facadeBoxesOf(l) in
  let bandBoxes = bandBoxesOf(l) in
  let roofBox = roofBoxOf(l) in
  let canopyBox = canopyBoxOf(l) in
  let apronBox = apronBoxOf(l) in
  let postBoxes = postBoxesOf(l) in
  let neonBoxes = neonBoxesOf(l) in
  let shadeBoxes = shadeBoxesOf(l) in
  let streetBoxes = List.flatten([
    facadeBoxes |> List.map((b) => (b, concrete)),
    bandBoxes |> List.map((b) => (b, glass)),
    [(roofBox, roofGrey), (canopyBox, concrete), (apronBox, apronGrey)],
    postBoxes |> List.map((b) => (b, postGrey)),
  ]) in
  { half: halfOf(l), facadeBoxes: facadeBoxes, bandBoxes: bandBoxes, roofBox: roofBox, entrance: entranceOf(l),
    canopyBox: canopyBox, apronBox: apronBox, postBoxes: postBoxes, neonBoxes: neonBoxes, shadeBoxes: shadeBoxes,
    streetBoxes: streetBoxes,
    streetSide: instancedOf(streetBoxes),
    neonStrips: Scene.group(neonBoxes |> List.map(slabOf)),
    lampShades: shadeBoxes |> List.map(slabOf) |> Scene.group,
    lampHeads: l.streetLamps,
    // The lamp posts' colliders, in the building's space.
    lampColliders: l.streetLamps |> List.indexedMap((i, p) =>
      { tag: $"lamp-{i}", shape: Physics.box(0.12, 4.9, 0.12), at: v(Vec3.x(p), 2.45, Vec3.z(p)) }),
    // Inside the entrance, at head height.
    lobbyLight: (let e = entranceOf(l) in v((e.lo + e.hi) / 2.0, 2.0, l.footprint.z0 + 0.9)),
    extraBoxes: [], beaconBoxes: [], grid: Option.None }

// The building's street side, in its own space, with the neon and the lamp
// heads glowing at `neon` and `lamps` (1 at night), and its window grid by
// `day` or night: only those change with the hour. The ground isn't part of
// it.
let exteriorAt = (s: Street, neon: float, lamps: float, day: bool): Scene.t => Scene.group([
  s.streetSide,
  s.neonStrips |> Scene.color(neonColor(neon)),
  s.lampShades |> Scene.color(shadeColor(lamps)),
  Scene.group(s.beaconBoxes |> List.map(slabOf)) |> Scene.color(beaconRed),
  s.grid |> Option.map((g) => gridAt(day, g)) |> Option.defaultValue(Scene.group([])),
])
let gridAt = (day: bool, g: Grid): Scene.t => if day then g.day else g.night
// An aircraft warning light: red at every hour.
let beaconRed = rgb(255.0, 40.0, 30.0)
// The neon and the sodium shades at `neon` and `lamps` (1 at night).
let neonColor = (neon: float): Color.t => rgb(110.0 * neon, 240.0 * neon, 230.0 * neon)
let shadeColor = (lamps: float): Color.t => Hours.color(Hours.mix(shadeByDay, sodiumGlow, lamps))
// Off by day: about the grey the lit posts show at noon.
let shadeByDay: Hours.Rgb = { r: 110.0 / 255.0, g: 110.0 / 255.0, b: 116.0 / 255.0 }
let sodiumGlow: Hours.Rgb = { r: 255.0 / 255.0, g: 190.0 / 255.0, b: 110.0 / 255.0 }

// ---- The ground ------------------------------------------------------------------

// The ground, shared by every building on it, `size` metres square about
// (x, z): below the interior floors (y = 0), which would otherwise z-fight it.
let groundAt = (x: float, z: float, size: float): Scene.t =>
  Scene.plane() |> Scene.scaleXYZ(size, 1.0, size) |> Scene.translate(v(x, street - 0.02, z)) |> Scene.lit(rgb(40.0, 36.0, 46.0))
let ground = groundAt(10.0, 10.0, 240.0)

let groundBodyAt = (tag: string, x: float, z: float, size: float): Physics.body =>
  Physics.fixed(Physics.tag(tag), Physics.box(size, 1.0, size)) |> Physics.at(v(x, street - 0.5, z))
let groundBody = groundBodyAt("ground", 10.0, 10.0, 240.0)
