// Stand-ins for the grid's inactive copies: each archetype's massing,
// parapet and roof in their finishes (one `Scene.instanced` cube per
// finish), its canopy, apron, window bands, lamp posts and own boxes (a
// tower's plant room, the facade kit's trim) as lit boxes or in their
// finishes, and its neon, lamp shades, beacon, entrance and real windows as
// glowing ones — five `Scene.instanced` cubes for the whole town, plus one
// per finish and a tower's window grid. The windows and entrance glow like the
// lit rooms behind them, so a distant copy reads as the building it becomes
// at `Block.activeOn`. Its bands, windows and grid are glazed (glass.fun):
// two draws more for the town.

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
  litBands: List<Exterior.Box>,
  litBoxes: List<(Exterior.Box, Color.t)>,
  skinned: List<(Exterior.Box, Finishes.Surface)>,
  gables: List<(Exterior.Box, Finishes.Surface)>,
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
  // The bands the street side lights at night (streetscape.fun), glowing
  // like the openings, 2 cm prouder and wider than the band they cover.
  let litBands = levelsOf(l) |> List.concatMap((level) =>
    [South, North, West, East] |> List.concatMap((side) => Exterior.bandHoles(l, level, side)
      |> List.filter((h) => Streetscape.isLitBand(level, side, (h.lo + h.hi) / 2.0))
      |> List.map((h) => let b = opening(side, level, { h with lo: h.lo - 0.02, hi: h.hi + 0.02 }) in
        match side with
        | South => { b with z0: b.z0 - 0.02 }
        | North => { b with z1: b.z1 + 0.02 }
        | West => { b with x0: b.x0 - 0.02 }
        | East => { b with x1: b.x1 + 0.02 }))) in
  // The lit boxes and their colours, the same at every hour.
  { massing: massing, parapet: parapet, openings: openings, bands: bands, litBands: litBands,
    litBoxes: List.flatten([
      [(s.canopyBox, s.siding), (s.apronBox, Exterior.apronGrey)],
      bands |> List.map((b) => (b, Exterior.glass)),
      s.postBoxes |> List.map((b) => (b, Exterior.postGrey)),
      s.extraBoxes,
    ]),
    skinned: List.flatten([
      [(massing, s.wall), (s.roofBox, Exterior.roofing)],
      parapet |> List.map((b) => (b, s.wall)),
      s.extraSkinned,
    ]),
    gables: s.gables }

// Every archetype's shell, by name, built once.
let shells: Map<string, Shell> = Map.fromList(Archetypes.all |> List.map((a) => (a.name, shellOf(a))))
let shellAt = (name: string): Shell =>
  match Map.get(name, shells) with
  | Option.Some(s) => s
  | Option.None => Binary.invalid($"no archetype {name}")

// Box `b` of copy `c`, centred on it and scaled by `s`.
let placeScaled = (c: Block.Copy, b: Exterior.Box, s: Vec3.t): Instance.t =>
  let mid = Vec3.make((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) in
  Instance.trs(Block.toWorld(c, mid), Angle.degrees(c.yaw), Vec3.x(s), Vec3.y(s), Vec3.z(s))
// As an instance of the unit cube.
let place = (c: Block.Copy, b: Exterior.Box): Instance.t => placeScaled(c, b, Vec3.make(b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0))
// As an instance of `Exterior.gableTemplate`.
let placeGable = (c: Block.Copy, b: Exterior.Box): Instance.t => placeScaled(c, b, Exterior.gableScale(b))

// A copy's shell as instances: the lit boxes (tinted), its finished boxes
// and gables (by finish), and what glows, by colour, so the hour changes a
// template's colour, not the instances; and its window grid, placed.
type Parts = { lit: List<Instance.t>, skinned: List<List<Instance.t>>, gables: List<List<Instance.t>>, neon: List<Instance.t>, shades: List<Instance.t>, openings: List<Instance.t>,
               litBands: List<Instance.t>, beacons: List<Instance.t>, grid: Option.t<Exterior.Grid>, panes: List<Instance.t> }
let partsOf = (c: Block.Copy): Parts =>
  let shell = shellAt(c.archetype) in
  let street = Block.archetypeOf(c).street in
  {
    lit: shell.litBoxes |> List.map((e) => let (b, color) = e in place(c, b) |> Instance.tint(color)),
    skinned: Finishes.groupedBy(shell.skinned |> List.map((e) => let (b, f) = e in (place(c, b), f))),
    gables: Finishes.groupedBy(shell.gables |> List.map((e) => let (b, f) = e in (placeGable(c, b), f))),
    neon: street.neonBoxes |> List.map((b) => place(c, b)),
    shades: street.shadeBoxes |> List.map((b) => place(c, b)),
    openings: shell.openings |> List.map((b) => place(c, b)),
    litBands: shell.litBands |> List.map((b) => place(c, b)),
    beacons: street.beaconBoxes |> List.map((b) => place(c, b)),
    grid: street.grid |> Option.map((g) => { night: Block.sceneOf(c, g.night), day: Block.sceneOf(c, g.day) }),
    panes: Glass.shellOf(c.archetype, Block.archetypeOf(c).layout, street.half + proud + Glass.shellGap) |> List.map((p) => Glass.place(c, p)),
  }
// Every town's copies' parts, by id, built once at load.
let parts: List<List<Parts>> = Block.towns |> List.map((t) => t.copies |> List.map(partsOf))
let none = { lit: [], skinned: [], gables: [], neon: [], shades: [], openings: [], litBands: [], beacons: [], grid: Option.None, panes: [] }

// The openings glow like the lit rooms behind them by night, neutral glass
// by day; the neon and shades as on the street side. `broken`: (copy name,
// window) of the real windows that have lost their panes.
type Look = { town: float, active: List<float>, neon: float, lamps: float, day: bool, broken: List<(string, float)> }
let roomGlow = Color.rgb(0.62, 0.66, 0.5)
let dayGlass = Color.rgb(0.36, 0.39, 0.42)

// The ids of the town's copies with no interior drawing.
let idleOf = (look: Look): List<float> =>
  List.range(List.length(Block.townAt(look.town).copies)) |> List.filter((id) => not Block.isIn(id, look.active))

// Every idle copy's shell: four instanced cubes for the town, and one per
// finish (and per gable finish, for houses). A change of the active set
// only gathers the prebuilt instances (and re-uploads them); a change of
// hour recolours the templates.
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
     cubes((s) => s |> Scene.color(if look.day then dayGlass else roomGlow), (p) => if look.day then p.openings else List.flatten([p.openings, p.litBands])),
     Finishes.drawn(List.range(List.length(Finishes.all)) |> List.map((k) =>
       idle |> List.concatMap((p) => List.nth(k, p.skinned) |> Option.defaultValue([]))))],
    // Towers and the mall only: an office town draws exactly the four cubes,
    // its finishes and its glass.
    if idle |> List.any((p) => Option.isSome(p.grid)) then
      List.flatten([[cubes((s) => s |> Scene.color(Exterior.beaconRed), (p) => p.beacons)],
        idle |> List.concatMap((p) => p.grid |> Option.map((g) => Exterior.gridAt(look.day, g)) |> Option.toList)])
    else [],
    // Houses only: their gables.
    [Finishes.drawnAs(Exterior.gableTemplate, List.range(List.length(Finishes.all)) |> List.map((k) =>
       idle |> List.concatMap((p) => List.nth(k, p.gables) |> Option.defaultValue([]))))],
    // Their glass, but the broken windows': two draws for the town.
    [Glass.panesOf(idleOf(look) |> List.concatMap((id) =>
      let name = Block.copyIn(Block.townAt(look.town), id).name in
      let broken = look.broken |> List.filter((w) => let (copy, i) = w in copy == name) |> List.map((w) => let (copy, i) = w in i) in
      List.nth(id, town) |> Option.map((p) => Glass.unbroken(broken, p.panes)) |> Option.defaultValue([])))],
  ]))
