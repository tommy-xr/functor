// Stand-ins for the grid's inactive copies: the office's massing, canopy,
// apron and window bands as lit boxes, and its neon, lamp shades, entrance and
// real windows as glowing ones — two `Scene.instanced` cubes for the whole
// town. The windows and entrance glow like the lit rooms behind them, so a
// distant copy reads as the office it becomes at `Block.activeOn`.

open Layout
open Building

let half = facade / 2.0

// The massing: the facade's outer faces, up to the roofline.
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

// layout.fun's windows, each with whether a panel lights its room, and the
// entrance (the lobby is lit).
let isLit = (i: float): bool => roomAt(i).panels |> List.any((p) => p.on)
let windowOpenings: List<(Exterior.Box, bool)> =
  windows |> List.map((w) => (opening(w.side, roomAt(w.room).level, windowHole(w)), isLit(w.room)))
let entranceOpening = (opening(South, 0.0, Exterior.entrance), true)

// What glows at the hour: the neon and sodium shades as on the street side,
// and the lit openings warm by night, neutral glass by day.
type Look = { town: float, active: List<float>, neon: float, lamps: float, day: bool }
let roomGlow = Color.rgb(0.62, 0.66, 0.5)
let dayGlass = Color.rgb(0.36, 0.39, 0.42)
let glowOf = (look: Look): List<(Exterior.Box, Color.t)> =>
  let n = look.neon in
  List.flatten([
    Exterior.neonBoxes |> List.map((b) => (b, rgb(110.0 * n, 240.0 * n, 230.0 * n))),
    Exterior.shadeBoxes |> List.map((b) => (b, Hours.color(Hours.mix(Exterior.shadeByDay, Exterior.sodiumGlow, look.lamps)))),
    List.append([entranceOpening], windowOpenings) |> List.map((e) =>
      let (b, lit) = e in (b, if look.day then dayGlass else if lit then roomGlow else Exterior.glass)),
  ])

// The lit boxes and their colours, the same at every hour.
let solidBoxes: List<(Exterior.Box, Color.t)> = List.flatten([
  [(massing, Exterior.concrete), (Exterior.canopyBox, Exterior.concrete), (Exterior.apronBox, Exterior.apronGrey)],
  Exterior.bandBoxes |> List.map((b) => (b, Exterior.glass)),
])

// Box `b` of copy `c`, tinted `color`, as an instance of the unit cube.
let place = (c: Block.Copy, e: (Exterior.Box, Color.t)): Instance.t =>
  let (b, color) = e in
  let mid = Vec3.make((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0) in
  Instance.trs(Block.toWorld(c, mid), Angle.degrees(c.yaw), b.x1 - b.x0, b.y1 - b.y0, b.z1 - b.z0) |> Instance.tint(color)

// The town's copies with no interior drawing.
let idleOf = (look: Look): List<Block.Copy> =>
  Block.townAt(look.town).copies |> List.indexedMap((id, c) => (id, c))
    |> List.filter((e) => let (id, c) = e in not Block.isIn(id, look.active))
    |> List.map((e) => let (id, c) = e in c)

// Every idle copy's shell: one lit and one glowing instanced cube. Rebuilt
// (and re-uploaded) only when the active set or the hour's look changes.
let white = Color.rgb(1.0, 1.0, 1.0)
let shellsOf = (look: Look): Scene.t =>
  let idle = idleOf(look) in
  let glow = glowOf(look) in
  Scene.group([
    Scene.cube() |> Scene.lit(white) |> Scene.instanced(idle |> List.concatMap((c) => solidBoxes |> List.map((e) => place(c, e)))),
    Scene.cube() |> Scene.color(white) |> Scene.instanced(idle |> List.concatMap((c) => glow |> List.map((e) => place(c, e)))),
  ])
