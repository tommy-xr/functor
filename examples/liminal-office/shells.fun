// Stand-ins for the grid's inactive copies: the office's massing, canopy,
// apron, window bands and lamp posts as lit boxes, and its neon, lamp
// shades, entrance and real windows as glowing ones — four `Scene.instanced`
// cubes for the whole town. The windows and entrance glow like the lit
// rooms behind them, so a distant copy reads as the office it becomes at
// `Block.activeOn`.

open Layout
open Building

// The massing: the facade's outer faces, up to the roofline.
let half = Exterior.half
let massing = Exterior.box(footprint.x0 - half, Exterior.street, footprint.z0 - half, footprint.x1 + half, facadeHeight, footprint.z1 + half)

// An opening on side `s`'s outer face, as the window bands sit: 2 cm proud.
let opening = (s: Side, level: float, h: Hole): Exterior.Box =>
  let f = footprint in
  let o = half + 0.02 in
  let y0 = level * storey + h.bottom in
  let y1 = level * storey + h.top in
  match s with
  | South => Exterior.box(h.lo, y0, f.z0 - o, h.hi, y1, f.z0 - half)
  | North => Exterior.box(h.lo, y0, f.z1 + half, h.hi, y1, f.z1 + o)
  | West => Exterior.box(f.x0 - o, y0, h.lo, f.x0 - half, y1, h.hi)
  | East => Exterior.box(f.x1 + half, y0, h.lo, f.x1 + o, y1, h.hi)

// The entrance and layout.fun's windows (tests.fun checks a panel lights
// every window's room; the lobby is lit).
let openings: List<Exterior.Box> = List.flatten([
  [opening(South, 0.0, Exterior.entrance)],
  windows |> List.map((w) => opening(w.side, roomAt(w.room).level, windowHole(w))),
])

// The lit boxes and their colours, the same at every hour.
let litBoxes: List<(Exterior.Box, Color.t)> = List.flatten([
  [(massing, Exterior.concrete), (Exterior.canopyBox, Exterior.concrete), (Exterior.apronBox, Exterior.apronGrey)],
  Exterior.bandBoxes |> List.map((b) => (b, Exterior.glass)),
  Exterior.postBoxes |> List.map((b) => (b, Exterior.postGrey)),
])

// Box `b` of copy `c` as an instance of the unit cube.
let place = (c: Block.Copy, b: Exterior.Box): Instance.t =>
  let mid = Vec3.make((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) in
  Instance.trs(Block.toWorld(c, mid), Angle.degrees(c.yaw), b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0)

// A copy's shell as instances: the lit boxes (tinted), and what glows, by
// colour, so the hour changes a template's colour, not the instances.
type Parts = { lit: List<Instance.t>, neon: List<Instance.t>, shades: List<Instance.t>, openings: List<Instance.t> }
let partsOf = (c: Block.Copy): Parts => {
  lit: litBoxes |> List.map((e) => let (b, color) = e in place(c, b) |> Instance.tint(color)),
  neon: Exterior.neonBoxes |> List.map((b) => place(c, b)),
  shades: Exterior.shadeBoxes |> List.map((b) => place(c, b)),
  openings: openings |> List.map((b) => place(c, b)),
}
// Every copy's parts, built once at load: an id is the same copy in every
// town, so the largest town's serve them all.
let parts: List<Parts> = Block.towns |> List.last |> Option.map((t) => t.copies) |> Option.defaultValue([]) |> List.map(partsOf)
let none = { lit: [], neon: [], shades: [], openings: [] }

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
  let idle = idleOf(look) |> List.map((id) => List.nth(id, parts) |> Option.defaultValue(none)) in
  let cubes = (material: (Scene.t) => Scene.t, of: (Parts) => List<Instance.t>): Scene.t =>
    Scene.cube() |> material |> Scene.instanced(idle |> List.concatMap(of)) in
  Scene.group([
    cubes((s) => s |> Scene.lit(white), (p) => p.lit),
    cubes((s) => s |> Scene.color(Exterior.neonColor(look.neon)), (p) => p.neon),
    cubes((s) => s |> Scene.color(Exterior.shadeColor(look.lamps)), (p) => p.shades),
    cubes((s) => s |> Scene.color(if look.day then dayGlass else roomGlow), (p) => p.openings),
  ])
