// Window glass, as two panes over each opening: one multiplies what is
// behind it by a smudged, grimy texture, the other adds the sky's
// reflection (clear head-on, a mirror at grazing angles). Neither writes
// depth or touches a portal, so interiors stay visible and culled.
// Real windows, the dark window bands, a tower's window grid, the cinema's
// entrance and front and the mall's open shopfronts are glazed; shells
// glaze their bands, windows and grid the same way.

open Layout
open Building

// ---- The panes -----------------------------------------------------------------

// The smudge texture's multiplier: faintly green glass, a few soft smudges
// and fingerprint whorls at hand height, and grime gathering along the
// bottom.
type Blot = { u: float, t: float, r: float, print: bool }
let blots = [
  { u: 0.22, t: 0.42, r: 0.07, print: true },
  { u: 0.31, t: 0.47, r: 0.06, print: true },
  { u: 0.71, t: 0.52, r: 0.07, print: true },
  { u: 0.55, t: 0.3, r: 0.2, print: false },
  { u: 0.15, t: 0.7, r: 0.16, print: false },
  { u: 0.85, t: 0.22, r: 0.14, print: false },
]
let blotAt = (u: float, t: float, b: Blot): float =>
  let d = Math.sqrt((u - b.u) * (u - b.u) + (t - b.t) * (t - b.t) * 1.6) / b.r in
  if d >= 1.0 then 0.0
  else if b.print then (1.0 - d) * (0.55 + 0.45 * Math.sin(d * 38.0))
  else (1.0 - d) * (1.0 - d)
let smudgeSize = 64.0
let smudgeTexel = (x: float, y: float): Paint =>
  let u = x / (smudgeSize - 1.0) in
  // Row 0 is the pane's bottom: t runs 0 at the top to 1 at the bottom.
  let t = 1.0 - y / (smudgeSize - 1.0) in
  let grime = Math.clamp(0.0, 1.0, (t - 0.7) / 0.3) in
  let blot = blots |> List.map((b) => blotAt(u, t, b)) |> List.fold((a, b) => a + b, 0.0) in
  let k = 1.0 - 0.4 * grime * grime - 0.15 * Math.min(blot, 1.0) - 0.06 * noise(x + smudgeSize * y) in
  paint(236.0 * k * (1.0 - 0.08 * grime), 246.0 * k * (1.0 - 0.12 * grime), 242.0 * k * (1.0 - 0.2 * grime))
let smudges = Texture.fromPixels(image(smudgeSize, smudgeSize, smudgeTexel))
// Kept on the GPU, so the first glazed building drawn (say, entering a
// cinema whose shell has no glass) uploads nothing.
let resident = Scene.quad() |> Scene.multiply(smudges) |> Scene.keepResident

// What the reflection adds: the sky, times this, times the fresnel term
// from `base` head-on to 1 at grazing.
let sheen = Color.rgb(0.8, 0.85, 0.9)
let base = 0.03

// Panes as instances of a unit quad: the smudged pane and the reflection,
// one draw each however many.
let panesOf = (panes: List<Instance.t>): Scene.t =>
  if List.isEmpty(panes) then Scene.group([])
  else Scene.group([
    Scene.quad() |> Scene.multiply(smudges) |> Scene.instanced(panes),
    Scene.quad() |> Scene.reflection(sheen, base) |> Scene.instanced(panes),
  ])

// A pane: its centre, the way it faces (degrees about y; a quad faces +z)
// and its size.
type Pane = { at: Vec3.t, yaw: float, w: float, h: float }
let instanceOf = (p: Pane): Instance.t => Instance.trs(p.at, Angle.degrees(p.yaw), p.w, p.h, 1.0)

// Hole `h` on footprint side `s` of `l`, its sill `y` above the street,
// `out` metres outside the wall line.
let paneOn = (l: Layout, s: Side, y: float, h: Hole, out: float): Pane =>
  let f = l.footprint in
  let u = (h.lo + h.hi) / 2.0 in
  let mid = y + (h.bottom + h.top) / 2.0 in
  let w = h.hi - h.lo in
  let hh = h.top - h.bottom in
  match s with
  | South => { at: v(u, mid, f.z0 - out), yaw: 180.0, w: w, h: hh }
  | North => { at: v(u, mid, f.z1 + out), yaw: 0.0, w: w, h: hh }
  | West => { at: v(f.x0 - out, mid, u), yaw: 270.0, w: w, h: hh }
  | East => { at: v(f.x1 + out, mid, u), yaw: 90.0, w: w, h: hh }

// A pane over quad `q` (its origin and edges), `out` metres in front.
let paneOver = (q: Quad, out: float): Pane =>
  let n = normalOf(q) in
  let mid = q.o |> Vec3.add(Vec3.scale(0.5, q.a)) |> Vec3.add(Vec3.scale(0.5, q.b)) in
  { at: mid |> Vec3.add(Vec3.scale(out, n)), yaw: Math.atan2(Vec3.x(n), Vec3.z(n)) * 180.0 / Math.pi,
    w: Vec3.length(q.a), h: Vec3.length(q.b) }

// A doorway's glazing, leaving it walkable: a transom over 2.2 m, in lights
// at most 1.6 m wide so the smudges don't stretch, and on a doorway wider
// than 2.6 m sidelights either side of a 1.8 m gap (`sidelights`).
let doorGlass = (h: Hole): List<Hole> => List.append(transomOf(h), sidelights(h))
let transomOf = (h: Hole): List<Hole> =>
  if h.top <= 2.4 then []
  else
    let n = Math.ceil((h.hi - h.lo) / 1.6) in
    let w = (h.hi - h.lo) / n in
    List.range(n) |> List.map((k) => { lo: h.lo + k * w, hi: h.lo + (k + 1.0) * w, bottom: 2.2, top: h.top })
let sidelights = (h: Hole): List<Hole> =>
  let mid = (h.lo + h.hi) / 2.0 in
  if h.hi - h.lo > 2.6
  then [{ h with hi: mid - 0.9, top: Math.min(h.top, 2.2) }, { h with lo: mid + 0.9, top: Math.min(h.top, 2.2) }]
  else []

// Every other pane mirrored, so neighbours don't repeat the same smudges.
let alternated = (panes: List<Pane>): List<Pane> =>
  panes |> List.indexedMap((i, p) => if Math.mod(i, 2.0) == 0.0 then p else { p with w: 0.0 - p.w })

// ---- Per archetype -------------------------------------------------------------

// Where the panes stand, from the wall line out: a real window's 5 cm
// inside the facade's face; a band's 5 cm in front of its recessed panel; a
// grid's `gridGap` in front of its quads. A shell's stand `shellGap` in front
// of its openings (shells.fun).
let windowOut = (l: Layout): float => l.facade / 2.0 - 0.05
let bandOut = (l: Layout): float => l.facade / 2.0 - Exterior.recess + 0.05
let gridGap = 0.08
let shellGap = 0.1

// The layout's real windows and bands, `out` from the wall line.
let windowsAt = (l: Layout, out: float): List<Pane> =>
  l.windows |> List.map((w) => paneOn(l, w.side, roomAt(l, w.room).level * l.storey, windowHole(w), out))
let bandsAt = (l: Layout, out: float): List<Pane> =>
  levelsOf(l) |> List.concatMap((level) => [South, North, West, East] |> List.concatMap((s) =>
    Exterior.bandHoles(l, level, s) |> List.map((h) => paneOn(l, s, level * l.storey, h, out))))

// A tower's window grid, by archetype name (`Archetypes.towerOf`'s sizes).
let towerFloors = [8.0, 12.0, 16.0]
let gridOf = (name: string, l: Layout): List<Pane> =>
  towerFloors |> List.filter((n) => $"tower-{n}" == name)
    |> List.concatMap((n) => Tower.gridFaces(l, n) |> List.map((e) => let (f, _) = e in paneOver(f, gridGap)))

// A doorway's panes on the wall line `z` (facing ±z), its floor at `y`.
let doorPanes = (holes: List<Hole>, y: float, z: float): List<Pane> =>
  holes |> List.map((h) => { at: v((h.lo + h.hi) / 2.0, y + (h.bottom + h.top) / 2.0, z), yaw: 0.0, w: h.hi - h.lo, h: h.top - h.bottom })
// The cinema's entrance doors, inset like a window.
let entranceOf = (name: string, l: Layout, glass: (Hole) => List<Hole>): List<Pane> =>
  if name != "cinema" then []
  else doorPanes(glass(Exterior.entranceOf(l)), 0.0, l.footprint.z0 - windowOut(l))

// The cinema front's own: its door leaves, booth window and poster cases.
let frontOf = (name: string): List<Pane> =>
  if name != "cinema" then []
  else CinemaFront.panes |> List.map((e) => let (at, yaw, w, h) = e in { at: at, yaw: yaw, w: w, h: h })
// An archetype's street-side panes, in its own space.
let streetPanesOf = (name: string, l: Layout): List<Pane> =>
  alternated(List.flatten([windowsAt(l, windowOut(l)), bandsAt(l, bandOut(l)), gridOf(name, l), entranceOf(name, l, doorGlass), frontOf(name)]))
// A shell's, `out` from the wall line: its windows and bands with its
// openings, and its grid.
let shellOf = (name: string, l: Layout, out: float): List<Pane> =>
  alternated(List.flatten([windowsAt(l, out), bandsAt(l, out), gridOf(name, l)]))

// The mall's open shopfronts (a shop's door onto its hall), on the shop's
// front wall line, glazed like a doorway.
let shopDoors = MallLayout.doors |> List.filter((d) =>
  MallInterior.cellAt(d.a).kind == MallLayout.Shop && MallInterior.cellAt(d.b).kind != MallLayout.Stock)
let shopfrontPanes = (d: Door, glass: (Hole) => List<Hole>): List<Pane> =>
  doorPanes(glass(holeOf(d)), MallInterior.baseOf(d.a), MallInterior.rectOf(d.a).z0)
// Each in its shop's and its hall's cells.
let shopfronts: List<Scene.t> =
  shopDoors |> List.map((d) => shopfrontPanes(d, doorGlass) |> alternated |> List.map(instanceOf) |> panesOf |> Scene.inCells([d.a, d.b]))

// Each archetype's panes as scenes, built once: outside (unmarked, so
// always drawn) and inside (in their cells, so under its portal walk).
type Glazing = { outside: Scene.t, inside: List<Scene.t> }
let glazing: Map<string, Glazing> = Map.fromList(Archetypes.all |> List.map((a) =>
  (a.name, { outside: panesOf(streetPanesOf(a.name, a.layout) |> List.map(instanceOf)),
             inside: if a.name == "mall" then shopfronts else [] })))
let glazingOf = (name: string): Glazing =>
  Map.get(name, glazing) |> Option.defaultValue({ outside: Scene.group([]), inside: [] })

// Sidelights are solid: a thin box collider each (doorways face ±z).
let colliderOf = (i: float, p: Pane): Collider =>
  { tag: $"glass-{i}", shape: Physics.box(Math.abs(p.w), p.h, 0.06), at: p.at }
let sidelightColliders = (name: string): List<Collider> =>
  if name == "cinema" then entranceOf(name, Archetypes.cinema.layout, sidelights) |> List.indexedMap(colliderOf)
  else if name == "mall" then shopDoors |> List.concatMap((d) => shopfrontPanes(d, sidelights)) |> List.indexedMap(colliderOf)
  else []
// A copy's sidelight bodies, and a thin box across each of its real
// windows but those in `broken` (indices into its layout's windows).
let bodiesOf = (c: Block.Copy, broken: List<float>): List<Physics.body> =>
  let l = Block.archetypeOf(c).layout in
  Block.bodiesFrom(c, List.append(sidelightColliders(c.archetype),
    windowsAt(l, windowOut(l)) |> List.indexedMap((i, p) => if Block.isIn(i, broken) then [] else [windowCollider(i, p)]) |> List.flatten))
// A pane faces ±z (yaw 0, 180) or ±x (90, 270).
let windowCollider = (i: float, p: Pane): Collider =>
  let across = Math.abs(Math.sin(p.yaw * Math.pi / 180.0)) > 0.5 in
  { tag: $"window-{i}", shape: if across then Physics.box(0.06, p.h, Math.abs(p.w)) else Physics.box(Math.abs(p.w), p.h, 0.06), at: p.at }

// ---- Broken windows ----------------------------------------------------------------

// An archetype's street-side panes without the real windows in `broken`
// (the windows come first in `streetPanesOf`).
let outsideWithout = (name: string, l: Layout, broken: List<float>): Scene.t =>
  streetPanesOf(name, l) |> List.indexedMap((i, p) => if Block.isIn(i, broken) then [] else [instanceOf(p)]) |> List.flatten |> panesOf

// Copy `c`'s window `i` in the world: its pane's centre, which way is out
// (n) and along it (a), as unit (x, z), its width, and its room's floor,
// sill and head heights.
type Opening = { x: float, y: float, z: float, nx: float, nz: float, ax: float, az: float, w: float, floor: float, sill: float, head: float }
let openingOf = (c: Block.Copy, i: float, win: Window): Opening =>
  let l = Block.archetypeOf(c).layout in
  let base = roomAt(l, win.room).level * l.storey in
  let at = Block.toWorld(c, paneOn(l, win.side, base, windowHole(win), windowOut(l)).at) in
  let (ox, oz) = match win.side with
    | South => (0.0, -1.0)
    | North => (0.0, 1.0)
    | West => (-1.0, 0.0)
    | East => (1.0, 0.0) in
  let (nx, nz) = Block.turn(c.yaw, ox, oz) in
  { x: Vec3.x(at), y: Vec3.y(at), z: Vec3.z(at), nx: nx, nz: nz, ax: 0.0 - nz, az: nx, w: win.width,
    floor: base, sill: base + win.sill, head: base + win.head }
// Every real window of copy `c`, by index.
let openingsOf = (c: Block.Copy): List<(float, Opening)> =>
  Block.archetypeOf(c).layout.windows |> List.indexedMap((i, win) => (i, openingOf(c, i, win)))

// The draft through a broken window: a world-axis box (copies turn in
// quarter turns) from `draftOut` outside to `draftIn` inside, floor to above
// the head, blowing in and nearly floating a sheet of paper (0.29 N of
// weight), with gusts. Plain data, so the model can hold it.
type Draft = { tag: string, cx: float, cy: float, cz: float, sx: float, sy: float, sz: float, fx: float, fz: float }
let draftOut = 1.2
let draftIn = 3.5
let draftOf = (tag: string, o: Opening): Draft =>
  let mid = (draftOut - draftIn) / 2.0 in
  let deep = draftOut + draftIn in
  let wide = o.w + 0.4 in
  { tag: tag, cx: o.x + o.nx * mid, cy: (o.floor + o.head + 0.3) / 2.0, cz: o.z + o.nz * mid,
    sx: Math.abs(o.nx) * deep + Math.abs(o.ax) * wide, sy: o.head + 0.3 - o.floor, sz: Math.abs(o.nz) * deep + Math.abs(o.az) * wide,
    fx: -0.15 * o.nx, fz: -0.15 * o.nz }
let volumeOf = (d: Draft): Physics.body =>
  Physics.forceVolume(Physics.tag(d.tag), { center: v(d.cx, d.cy, d.cz), size: v(d.sx, d.sy, d.sz),
    force: v(d.fx, 0.26, d.fz), turbulence: 0.12, drag: 1.0 })

// A shell pane placed in copy `c`.
let place = (c: Block.Copy, p: Pane): Instance.t =>
  Instance.trs(Block.toWorld(c, p.at), Angle.degrees(p.yaw + c.yaw), p.w, p.h, 1.0)

// ---- Checks --------------------------------------------------------------------

// Every real window and band of the office is glazed, and so are its shell's.
let office = Archetypes.office.layout
expect List.length(streetPanesOf("office", office)) == List.length(office.windows) + List.length(bandsAt(office, 0.0))
expect List.length(shellOf("office", office, 1.0)) == List.length(streetPanesOf("office", office))
// Panes stand outside the wall line, inside the facade's face.
let southPane = paneOn(office, South, 0.0, { lo: 1.0, hi: 2.0, bottom: 1.0, top: 2.0 }, bandOut(office))
expect Vec3.z(southPane.at) < office.footprint.z0 && Vec3.z(southPane.at) > office.footprint.z0 - office.facade / 2.0
// A grid pane stands `proud` in front of its face (a south face faces -z).
let southFace = wallZ(0.0, 1.0, 2.0, 1.0, 2.0, 0.0 - 5.0, false)
expect Math.abs(Vec3.z(paneOver(southFace.q, gridGap).at) + 5.0 + gridGap) < 0.0001
expect paneOver(southFace.q, gridGap).w == 1.0
// Every tower's grid bays are glazed.
expect [Archetypes.tower8, Archetypes.tower12, Archetypes.tower16] |> List.all((a) =>
  List.length(gridOf(a.name, a.layout)) == List.length(Tower.gridFaces(a.layout, a.layout.levels - 1.0)))
// A wide doorway keeps a 1.8 m gap between its sidelights; a narrow one
// gets a transom only, if it is tall enough.
expect List.length(doorGlass({ lo: 0.0, hi: 3.2, bottom: 0.0, top: 2.7 })) == 4
expect List.length(sidelights({ lo: 0.0, hi: 3.2, bottom: 0.0, top: 2.7 })) == 2
expect List.length(doorGlass({ lo: 0.0, hi: 1.6, bottom: 0.0, top: 2.4 })) == 0
expect List.length(doorGlass({ lo: 0.0, hi: 1.6, bottom: 0.0, top: 2.7 })) == 1
// Each open shopfront is glazed (shuttered fronts aren't), its sidelights
// solid, as are the cinema's.
expect List.length(shopfronts) == List.length(MallLayout.bays)
expect List.length(sidelightColliders("mall")) == 2.0 * List.length(MallLayout.bays)
expect List.length(sidelightColliders("cinema")) == 2.0
// Each real window is solid until broken.
expect List.length(bodiesOf(Block.home, [])) == List.length(office.windows)
expect List.length(bodiesOf(Block.home, [0.0])) == List.length(office.windows) - 1.0
// Office 103's window faces west: its draft blows in along +x, from outside it.
expect (match openingsOf(Block.home) with
  | [first, .._] =>
    let (i, o) = first in
    let d = draftOf("d", o) in o.nx == -1.0 && d.fx > 0.0 && d.fz == 0.0 && d.cx > o.x && d.cx - d.sx / 2.0 < o.x
  | _ => false)
