// Stand-ins for the grid's inactive copies: each archetype's massing,
// parapet, canopy, apron, window bands, lamp posts and own boxes (a tower's
// plant room) as lit boxes, and its neon, lamp shades, beacon, entrance and
// real windows as glowing ones — five `Scene.instanced` cubes for the whole
// town, plus a tower's window grid. The windows and entrance glow like the
// lit rooms behind them, so a distant copy reads as the building it becomes
// at `Block.activeOn`.

open Layout
open Building

// An opening on side `s`'s outer face, `proud` of it: far enough that the
// depth buffer keeps it in front of the massing at a distance.
let proud = 0.1

// One archetype's shell boxes, in its own space.
type Shell = {
  massing: Exterior.Box,
  parapet: List<Exterior.Box>,
  openings: List<Exterior.Box>,
  bands: List<Exterior.Box>,
  litBoxes: List<(Exterior.Box, Color.t)>,
}
let shellOf = (a: Archetypes.Archetype): Shell =>
  let l = a.layout in
  let s = a.street in
  let f = l.footprint in
  let half = s.half in
  // The massing: the facade's outer faces, up to the roof's underside; then
  // the street side's roof, and the parapet round it, the facade's outer
  // half on up to the roofline.
  let massing = Exterior.box(f.x0 - half, Exterior.street, f.z0 - half, f.x1 + half, s.roofBox.y0, f.z1 + half) in
  let y0 = s.roofBox.y0 in
  let parapet = [
    Exterior.box(f.x0 - half, y0, f.z0 - half, f.x1 + half, l.facadeHeight, f.z0),
    Exterior.box(f.x0 - half, y0, f.z1, f.x1 + half, l.facadeHeight, f.z1 + half),
    Exterior.box(f.x0 - half, y0, f.z0, f.x0, l.facadeHeight, f.z1),
    Exterior.box(f.x1, y0, f.z0, f.x1 + half, l.facadeHeight, f.z1),
  ] in
  let opening = (side: Side, level: float, h: Hole): Exterior.Box =>
    let o = half + proud in
    let ya = level * l.storey + h.bottom in
    let yb = level * l.storey + h.top in
    match side with
    | South => Exterior.box(h.lo, ya, f.z0 - o, h.hi, yb, f.z0 - half)
    | North => Exterior.box(h.lo, ya, f.z1 + half, h.hi, yb, f.z1 + o)
    | West => Exterior.box(f.x0 - o, ya, h.lo, f.x0 - half, yb, h.hi)
    | East => Exterior.box(f.x1 + half, ya, h.lo, f.x1 + o, yb, h.hi) in
  // The entrance and the layout's windows (tests.fun checks a panel lights
  // every window's room; the lobby is lit).
  let openings = List.flatten([
    [opening(South, 0.0, s.entrance)],
    l.windows |> List.map((w) => opening(w.side, roomAt(l, w.room).level, windowHole(w))),
  ]) in
  // The window bands, where the street side cuts them, proud like the openings.
  let bands = levelsOf(l) |> List.concatMap((level) =>
    [South, North, West, East] |> List.concatMap((side) => Exterior.bandHoles(l, level, side) |> List.map((h) => opening(side, level, h)))) in
  // The lit boxes and their colours, the same at every hour.
  { massing: massing, parapet: parapet, openings: openings, bands: bands,
    litBoxes: List.flatten([
      [(massing, Exterior.concrete), (s.roofBox, Exterior.roofGrey), (s.canopyBox, Exterior.concrete), (s.apronBox, Exterior.apronGrey)],
      parapet |> List.map((b) => (b, Exterior.concrete)),
      bands |> List.map((b) => (b, Exterior.glass)),
      s.postBoxes |> List.map((b) => (b, Exterior.postGrey)),
      s.extraBoxes,
    ]) }

// Every archetype's shell, by name, built once.
let shells: Map<string, Shell> = Map.fromList(Archetypes.all |> List.map((a) => (a.name, shellOf(a))))
let shellAt = (name: string): Shell =>
  match Map.get(name, shells) with
  | Option.Some(s) => s
  | Option.None => Binary.invalid($"no archetype {name}")

// Box `b` of copy `c` as an instance of the unit cube.
let place = (c: Block.Copy, b: Exterior.Box): Instance.t =>
  let mid = Vec3.make((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) in
  Instance.trs(Block.toWorld(c, mid), Angle.degrees(c.yaw), b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0)

// A copy's shell as instances: the lit boxes (tinted), and what glows, by
// colour, so the hour changes a template's colour, not the instances; and
// its window grid, placed.
type Parts = { lit: List<Instance.t>, neon: List<Instance.t>, shades: List<Instance.t>, openings: List<Instance.t>,
               beacons: List<Instance.t>, grid: Option.t<Exterior.Grid> }
let partsOf = (c: Block.Copy): Parts =>
  let shell = shellAt(c.archetype) in
  let street = Block.archetypeOf(c).street in
  {
    lit: shell.litBoxes |> List.map((e) => let (b, color) = e in place(c, b) |> Instance.tint(color)),
    neon: street.neonBoxes |> List.map((b) => place(c, b)),
    shades: street.shadeBoxes |> List.map((b) => place(c, b)),
    openings: shell.openings |> List.map((b) => place(c, b)),
    beacons: street.beaconBoxes |> List.map((b) => place(c, b)),
    grid: street.grid |> Option.map((g) => { night: Block.sceneOf(c, g.night), day: Block.sceneOf(c, g.day) }),
  }
// Every town's copies' parts, by id, built once at load.
let parts: List<List<Parts>> = Block.towns |> List.map((t) => t.copies |> List.map(partsOf))
let none = { lit: [], neon: [], shades: [], openings: [], beacons: [], grid: Option.None }

// The openings glow like the lit rooms behind them by night, neutral glass
// by day; the neon and shades as on the street side.
type Look = { town: float, active: List<float>, neon: float, lamps: float, day: bool }
let roomGlow = Color.rgb(0.62, 0.66, 0.5)
let dayGlass = Color.rgb(0.36, 0.39, 0.42)

// The ids of the town's copies with no interior drawing.
let idleOf = (look: Look): List<float> =>
  List.range(List.length(Block.townAt(look.town).copies)) |> List.filter((id) => not Block.isIn(id, look.active))

// Every idle copy's shell: four instanced cubes for the town. A change of
// the active set only gathers the prebuilt instances (and re-uploads them);
// a change of hour recolours the templates.
let white = Color.rgb(1.0, 1.0, 1.0)
let shellsOf = (look: Look): Scene.t =>
  let town = List.nth(look.town, parts) |> Option.defaultValue([]) in
  let idle = idleOf(look) |> List.map((id) => List.nth(id, town) |> Option.defaultValue(none)) in
  let cubes = (material: (Scene.t) => Scene.t, of: (Parts) => List<Instance.t>): Scene.t =>
    Scene.cube() |> material |> Scene.instanced(idle |> List.concatMap(of)) in
  Scene.group(List.flatten([
    [cubes((s) => s |> Scene.lit(white), (p) => p.lit),
     cubes((s) => s |> Scene.color(Exterior.neonColor(look.neon)), (p) => p.neon),
     cubes((s) => s |> Scene.color(Exterior.shadeColor(look.lamps)), (p) => p.shades),
     cubes((s) => s |> Scene.color(if look.day then dayGlass else roomGlow), (p) => p.openings)],
    // Towers only: an office town draws exactly the four cubes.
    if idle |> List.any((p) => Option.isSome(p.grid)) then
      List.flatten([[cubes((s) => s |> Scene.color(Exterior.beaconRed), (p) => p.beacons)],
        idle |> List.concatMap((p) => p.grid |> Option.map((g) => Exterior.gridAt(look.day, g)) |> Option.toList)])
    else [],
  ]))
