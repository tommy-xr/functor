// The house's street side (exterior.fun's `Street`): the facade in siding up
// to the eaves, a gable, a porch with four posts and two rocking chairs, a
// porch light, a picket fence with a gate, a front path and lawns. Not
// baked, and not hashed into the house bakes.

open Layout
open Building

// A palette's outside: siding (the facade and porch roof), trim (posts,
// fence) and the roof.
type Look = { siding: Color.t, trim: Color.t, roof: Color.t }
// Dark enough that the noon sun doesn't wash the siding out to white.
let ochre = { siding: rgb(124.0, 108.0, 76.0), trim: rgb(190.0, 186.0, 176.0), roof: rgb(70.0, 54.0, 48.0) }
let magenta = { siding: rgb(132.0, 96.0, 110.0), trim: rgb(196.0, 190.0, 186.0), roof: rgb(60.0, 54.0, 70.0) }

let lawn = rgb(72.0, 96.0, 54.0)
let paving = rgb(132.0, 126.0, 118.0)
let chairWood = rgb(70.0, 50.0, 36.0)

let street = Exterior.street
let box = Exterior.box

// The gable (exterior.fun's): its lower half sunk into the facade above the
// top storey's slab (the facade is the eaves' knee wall), its upper half
// the roof. It overhangs the side facades by `eave`; its ends stand `inset`
// inside the front and back facades' outer faces, so they hide its lower
// half and only the upper triangle closes the gable.
let eave = 0.4
let inset = 0.05
let gableRise = 1.6
let gableBoxOf = (l: Layout): Exterior.Box =>
  let f = l.footprint in
  let half = l.facade / 2.0 in
  box(f.x0 - half - eave, l.facadeHeight - gableRise, f.z0 - half + inset, f.x1 + half + eave, l.facadeHeight + gableRise, f.z1 + half - inset)
// The porch: the entrance's outside room. Its roof (the street side's
// canopy) runs the facade's width just under the upstairs sills, on four
// posts along its front edge.
let porchRectOf = (l: Layout): Rect =>
  Exterior.entranceDoorOf(l) |> Option.map((d) => roomAt(l, d.a).rect) |> Option.defaultValue(rect(0.0, 0.0, 0.0, 0.0))
let porchTop = 2.62
let porchRoofOf = (l: Layout): Exterior.Box =>
  let p = porchRectOf(l) in
  let half = l.facade / 2.0 in
  box(p.x0 - half - 0.2, porchTop, p.z0 - 0.3, p.x1 + half + 0.2, porchTop + 0.15, p.z1 - half)
let postsOf = (l: Layout): List<Exterior.Box> =>
  let p = porchRectOf(l) in
  let z = p.z0 + 0.1 in
  List.range(4.0) |> List.map((k) =>
    let x = p.x0 + 0.2 + k * (p.x1 - p.x0 - 0.4) / 3.0 in
    box(x - 0.07, street, z - 0.07, x + 0.07, porchTop, z + 0.07))
// Two rocking chairs facing the street: seat, back and two rockers.
let chairAt = (x: float, z: float): List<Exterior.Box> => [
  box(x - 0.25, 0.4, z - 0.25, x + 0.25, 0.45, z + 0.25),
  box(x - 0.25, 0.45, z + 0.2, x + 0.25, 1.05, z + 0.25),
  box(x - 0.25, street, z - 0.35, x - 0.21, 0.4, z + 0.35),
  box(x + 0.21, street, z - 0.35, x + 0.25, 0.4, z + 0.35),
]
let chairsOf = (l: Layout): List<(float, float)> =>
  let p = porchRectOf(l) in [(p.x0 + 5.4, p.z0 + 1.4), (p.x0 + 6.6, p.z0 + 1.4)]

// The front yard: a picket fence `yard` metres out from the facade, `side`
// metres past each side of the footprint, with a gate where the path from
// the front door meets it; the path runs on to the road.
let yard = 6.0
let side = 3.0
let road = 9.0
type Gate = { lo: float, hi: float }
let gateOf = (l: Layout): Gate =>
  let e = Exterior.entranceOf(l) in { lo: e.lo - 0.1, hi: e.hi + 0.1 }
let fenceZ = (l: Layout): float => l.footprint.z0 - yard
// Pickets every 0.25 m, two rails, each side of the gate.
let fenceRuns = (l: Layout): List<(float, float)> =>
  let f = l.footprint in
  let g = gateOf(l) in [(f.x0 - side, g.lo), (g.hi, f.x1 + side)]
let picketsOf = (l: Layout): List<Exterior.Box> =>
  let z = fenceZ(l) in
  fenceRuns(l) |> List.concatMap((run) =>
    let (x0, x1) = run in
    let n = Math.floor((x1 - x0) / 0.25) in
    List.flatten([
      List.range(n) |> List.map((k) =>
        let x = x0 + 0.125 + k * (x1 - x0) / n in box(x - 0.035, street, z - 0.02, x + 0.035, 0.95, z + 0.02)),
      [box(x0, 0.3, z + 0.02, x1, 0.36, z + 0.05), box(x0, 0.7, z + 0.02, x1, 0.76, z + 0.05)],
    ]))
// The path, front door to road; lawns round the house, inside the fence.
let pathOf = (l: Layout): Exterior.Box =>
  let g = gateOf(l) in
  let p = porchRectOf(l) in
  box(g.lo, street - 0.02, l.footprint.z0 - road, g.hi, street, p.z0)
let lawnsOf = (l: Layout): List<Exterior.Box> =>
  let f = l.footprint in
  let g = gateOf(l) in
  let p = porchRectOf(l) in
  let z = fenceZ(l) in
  let y0 = street - 0.02 in
  [
    box(f.x0 - side, y0, z, g.lo, street, p.z0),
    box(g.hi, y0, z, f.x1 + side, street, p.z0),
    box(f.x0 - side, y0, p.z0, f.x0, street, f.z1 + 2.0),
    box(f.x1, y0, p.z0, f.x1 + side, street, f.z1 + 2.0),
    box(f.x0, y0, f.z1, f.x1, street, f.z1 + 2.0),
  ]

// The porch light's glow, round its lamp (building.fun lights the bake from
// the same lamp).
let porchLightsOf = (l: Layout): List<Exterior.Box> =>
  l.lamps |> List.filter((lamp) => not isInside(roomAt(l, lamp.room))) |> List.map((lamp) =>
    let c = lampFace(l, lamp).q in
    let mid = c.o |> Vec3.add(Vec3.scale(0.5, c.a)) |> Vec3.add(Vec3.scale(0.5, c.b)) in
    box(Vec3.x(mid) - 0.08, Vec3.y(mid) - 0.1, Vec3.z(mid) - 0.1, Vec3.x(mid) + 0.08, Vec3.y(mid) + 0.1, Vec3.z(mid) + 0.04))

let solidBox = (tag: string, b: Exterior.Box): Collider =>
  { tag: tag, shape: Physics.box(b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0), at: Vec3.make((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) }
// The fence's runs, the porch posts and the chairs, in the house's space.
let ownCollidersOf = (l: Layout): List<Collider> =>
  let z = fenceZ(l) in
  List.flatten([
    fenceRuns(l) |> List.indexedMap((k, run) => let (x0, x1) = run in solidBox($"fence-{k}", box(x0, street, z - 0.05, x1, 1.0, z + 0.05))),
    postsOf(l) |> List.indexedMap((k, b) => solidBox($"porch-post-{k}", b)),
    chairsOf(l) |> List.indexedMap((k, c) => let (x, cz) = c in solidBox($"chair-{k}", box(x - 0.25, street, cz - 0.35, x + 0.25, 1.05, cz + 0.35))),
  ])

// The house's street side, in its own space.
let streetOf = (l: Layout, look: Look): Exterior.Street =>
  let base = Exterior.streetOf(l) in
  let porchRoof = porchRoofOf(l) in
  let posts = postsOf(l) in
  let chairs = chairsOf(l) in
  let extra = List.flatten([
    lawnsOf(l) |> List.map((b) => (b, lawn)),
    [(pathOf(l), paving)],
    picketsOf(l) |> List.map((b) => (b, look.trim)),
    posts |> List.map((b) => (b, look.trim)),
    chairs |> List.concatMap((c) => let (x, cz) = c in chairAt(x, cz)) |> List.map((b) => (b, chairWood)),
  ]) in
  let lit = List.flatten([
    base.facadeBoxes |> List.map((b) => (b, look.siding)),
    [(porchRoof, look.siding), (base.apronBox, Exterior.apronGrey)],
    base.postBoxes |> List.map((b) => (b, Exterior.postGrey)),
    extra,
  ]) in
  let gable = gableBoxOf(l) in
  let shades = List.append(base.shadeBoxes, porchLightsOf(l)) in
  { base with
    canopyBox: porchRoof,
    neonBoxes: [],
    shadeBoxes: shades,
    streetBoxes: lit,
    streetSide: Scene.group([Exterior.instancedOf(lit), Exterior.gableOf(gable, look.roof)]),
    neonStrips: Scene.group([]),
    lampShades: shades |> List.map(Exterior.slabOf) |> Scene.group,
    siding: look.siding,
    extraBoxes: extra,
    gables: [(gable, look.roof)] }
