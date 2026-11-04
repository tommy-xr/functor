// Small things strewn about the furnished rooms: loose papers, a carton, a
// mug, a binder, a fallen chair. Not baked: each copy strews its own (from
// its seed), at most `cap` a room, clear of the furniture and the doors,
// drawn as one instanced unlit cube per copy in their cells, each box tinted
// to the light the bake gives its spot. Data first: each is a debris
// candidate, a tag and a body's box shape, which a later change can make a
// dynamic body without placing it again. No bodies yet.

open Layout

let v = Vec3.make
let noise = Building.noise

type Thing =
  | Paper
  | Carton
  | Mug
  | Binder
  | FallenChair

// A box of an item: its centre (x, y, z) in the item's frame, from the
// floor, its size, and its colour (0 to 1).
type Part = { x: float, y: float, z: float, w: float, h: float, d: float, r: float, g: float, b: float }
// A debris candidate: `tag` (the copy's tags prefix it), its cell, its
// place on the floor in the archetype's space, turned `yaw` degrees, the
// box a body for it would be (`shape`: its size, centred over `at`), and
// how it draws.
type Candidate = { tag: string, kind: Thing, cell: float, at: Vec3.t, yaw: float, shape: Vec3.t, parts: List<Part> }

let cap = 6.0

// ---- Items -----------------------------------------------------------------------

let box = (x: float, y0: float, z: float, w: float, h: float, d: float, c: (float, float, float)): Part =>
  let (r, g, b) = c in { x: x, y: y0 + h / 2.0, z: z, w: w, h: h, d: d, r: r, g: g, b: b }
let paper = (0.86, 0.85, 0.8)
let cardboard = (0.6, 0.45, 0.3)
let tape = (0.72, 0.62, 0.42)
let mugWhite = (0.82, 0.8, 0.74)
let binderBlue = (0.18, 0.26, 0.46)
let binderRed = (0.5, 0.16, 0.14)

// A furniture box as a part, in its swatch's colour from `swatches`.
let ofSolid = (swatches: List<Building.Paint>, s: Furniture.Solid): Part =>
  let extent = (f: (Vec3.t) => float) => 2.0 * (Math.abs(f(s.ax)) + Math.abs(f(s.ay)) + Math.abs(f(s.az))) in
  let p = List.nth(s.swatch, swatches) |> Option.defaultValue(Building.paint(128.0, 128.0, 128.0)) in
  { x: Vec3.x(s.c), y: Vec3.y(s.c), z: Vec3.z(s.c), w: extent(Vec3.x), h: extent(Vec3.y), d: extent(Vec3.z),
    r: p.r / 255.0, g: p.g / 255.0, b: p.b / 255.0 }

// What each kind is: its boxes, from `k`'s noise; a fallen chair is the
// furniture's stacking chair on its back, in the archetype's `swatches`.
let partsOf = (swatches: List<Building.Paint>, kind: Thing, k: float): List<Part> =>
  match kind with
  // A few sheets, fanned.
  | Paper => List.range(2.0 + Math.floor(3.0 * noise(k))) |> List.map((j) =>
      box(0.12 * (noise(k + j) - 0.5), 0.004 * j, 0.15 * (noise(k + 7.0 * j) - 0.5), 0.21, 0.003, 0.297, paper))
  | Carton => [box(0.0, 0.0, 0.0, 0.5, 0.36, 0.38, cardboard), box(0.0, 0.36, 0.0, 0.08, 0.005, 0.38, tape)]
  | Mug => [box(0.0, 0.0, 0.0, 0.08, 0.1, 0.08, mugWhite), box(0.055, 0.03, 0.0, 0.03, 0.05, 0.015, mugWhite)]
  | Binder => [box(0.0, 0.0, 0.0, 0.3, 0.06, 0.32, if noise(k) < 0.5 then binderBlue else binderRed),
               box(0.0, 0.003, 0.02, 0.27, 0.054, 0.28, paper)]
  | FallenChair => Furniture.grounded(Furniture.moved((p) => Furniture.tilted(-90.0, p), v(0.0, 0.0, 0.0), Furniture.sideChairParts))
      |> List.map((s) => ofSolid(swatches, s))

// An item's extent in its own frame: its body's box, centred on (0, 0).
type Extent = { x0: float, x1: float, z0: float, z1: float, top: float }
let extentOf = (parts: List<Part>): Extent =>
  let lo = (f: (Part) => float) => parts |> List.map(f) |> List.minimum |> Option.defaultValue(0.0) in
  let hi = (f: (Part) => float) => parts |> List.map(f) |> List.maximum |> Option.defaultValue(0.0) in
  { x0: lo((p) => p.x - p.w / 2.0), x1: hi((p) => p.x + p.w / 2.0), z0: lo((p) => p.z - p.d / 2.0), z1: hi((p) => p.z + p.d / 2.0),
    top: hi((p) => p.y + p.h / 2.0) }
// Its parts recentred on its extent's middle, so `at` is the body's centre.
let centred = (parts: List<Part>): List<Part> =>
  let e = extentOf(parts) in
  let cx = (e.x0 + e.x1) / 2.0 in
  let cz = (e.z0 + e.z1) / 2.0 in
  parts |> List.map((p) => { p with x: p.x - cx, z: p.z - cz })

// Mostly paper; a fallen chair only first in a room.
let kindAt = (k: float, first: bool): Thing =>
  let n = noise(k * 1.37) in
  if n < 0.45 then Paper else if n < 0.62 then Binder else if n < 0.76 then Mug else if n < 0.9 || not first then Carton else FallenChair

// ---- Where -------------------------------------------------------------------------

// Room `i`'s floor less a margin, and what to keep clear of: its furniture's
// boxes (loose ones too) and a metre in front of each of its doors.
let doorZones = (l: Layout, i: float): List<Rect> =>
  let r = Building.inner(l, Building.roomAt(l, i).rect) in
  Portals.sides |> List.concatMap((s) => Building.doorHolesIn(l, i, s) |> List.map((h) =>
    let lo = h.lo - 0.2 in
    let hi = h.hi + 0.2 in
    match s with
    | North => rect(lo, r.z1 - 1.0, hi, r.z1)
    | South => rect(lo, r.z0, hi, r.z0 + 1.0)
    | East => rect(r.x1 - 1.0, lo, r.x1, hi)
    | West => rect(r.x0, lo, r.x0 + 1.0, hi)))
let keepsOf = (l: Layout, i: float): List<Rect> =>
  let r = Building.inner(l, Building.roomAt(l, i).rect) in
  List.append(doorZones(l, i), Furniture.inRoom(l, i) |> List.map((f) =>
    let b = Furniture.boundsOf(Furniture.placedIn(r, f)) in rect(b.x0 - 0.1, b.z0 - 0.1, b.x1 + 0.1, b.z1 + 0.1)))
let clearOf = (keeps: List<Rect>, x: float, z: float, rad: float): bool =>
  keeps |> List.all((q) => x + rad < q.x0 || x - rad > q.x1 || z + rad < q.z0 || z - rad > q.z1)

// The rooms that get debris: the furnished ones.
let roomsOf = (l: Layout): List<float> =>
  l.furniture |> List.fold((out, f) => if out |> List.any((i) => i == f.room) then out else List.append([f.room], out), [])

// Up to `cap` items in room `i`, tried at noise spots over its floor, each
// kept where it is clear of the keeps and of the items before it.
let strewn = (swatches: List<Building.Paint>, seed: float, l: Layout, i: float): List<Candidate> =>
  let room = Building.roomAt(l, i) in
  let r = Building.inner(l, room.rect) in
  let keeps = keepsOf(l, i) in
  let y = Building.baseOf(l, room) in
  let tries = List.range(4.0 * cap) in
  let (out, _) = tries |> List.fold((acc, t) =>
    let (got, taken) = acc in
    let k = seed * 0.731 + i * 17.3 + t * 3.17 in
    let kind = kindAt(k, List.length(got) == 0.0) in
    let parts = centred(partsOf(swatches, kind, k + 0.5)) in
    let e = extentOf(parts) in
    // Its footprint's half diagonal: clear at any yaw.
    let rad = radiusOf(e) in
    let x = r.x0 + 0.3 + rad + (r.x1 - r.x0 - 0.6 - 2.0 * rad) * noise(k + 0.11) in
    let z = r.z0 + 0.3 + rad + (r.z1 - r.z0 - 0.6 - 2.0 * rad) * noise(k + 0.23) in
    if List.length(got) >= cap || not clearOf(List.append(taken, keeps), x, z, rad) then acc
    else
      (List.append([{ tag: $"debris-{i}-{List.length(got)}", kind: kind, cell: i, at: v(x, y + e.top / 2.0, z), yaw: 360.0 * noise(k + 0.37),
                      shape: v(e.x1 - e.x0, e.top, e.z1 - e.z0), parts: parts }], got),
       List.append([rect(x - rad, z - rad, x + rad, z + rad)], taken)), ([], [])) in
  out

let radiusOf = (e: Extent): float => Math.sqrt((e.x1 - e.x0) * (e.x1 - e.x0) + (e.z1 - e.z0) * (e.z1 - e.z0)) / 2.0

// Every debris candidate of a copy whose seed is `seed`, its fallen chairs
// in `swatches`.
let candidatesOf = (swatches: List<Building.Paint>, seed: float, l: Layout): List<Candidate> =>
  roomsOf(l) |> List.concatMap((i) => strewn(swatches, seed, l, i))
// A copy's seed: its place, so copies differ and a copy never changes.
let seedOf = (c: Block.Copy): float => Math.abs(c.x * 3.1 + c.z * 7.7 + c.yaw * 0.013) + 1.0

// ---- Light -----------------------------------------------------------------------

// How lit the bake leaves a spot on room `i`'s floor, roughly, as (r, g,
// b): the bake's panels and amber lamps in the room, with the engine's
// falloff, over the ambient; by day, sky for a room with windows. `base`
// lifts an archetype whose bake lights aren't the layout's (the cinema's).
let lightAt = (l: Layout, i: float, p: Vec3.t, day: bool, base: float): (float, float, float) =>
  let room = Building.roomAt(l, i) in
  let y = Building.baseOf(l, room) in
  let falloff = (at: Vec3.t, g: Building.Glow): float =>
    let d = Vec3.length(at |> Vec3.sub(p)) in
    let k = Math.max(0.0, 1.0 - d * d / (g.range * g.range)) in
    g.intensity * k * k * Math.max(0.0, (Vec3.y(at) - Vec3.y(p)) / Math.max(0.1, d)) in
  let panels = room.panels |> List.filter((q) => q.on) |> List.map((q) => falloff(Building.panelAt(l, y, q), Building.panelGlow)) |> List.sum in
  let lamps = l.lamps |> List.filter((q) => q.room == i) |> List.map((q) => falloff(Building.lampAt(l, q), Building.lampGlow)) |> List.sum in
  let sky = if day && (l.windows |> List.any((w) => w.room == i)) then 0.25 else 0.0 in
  let channel = (panel: float, lamp: float, daylight: float): float =>
    Math.clamp(0.06, 1.1, base + 0.1 + 1.2 * panel * panels + 1.5 * lamp * lamps + daylight * sky) in
  let pg = Building.panelGlow in
  let lg = Building.lampGlow in
  (channel(pg.r, lg.r, 0.72), channel(pg.g, lg.g, 0.84), channel(pg.b, lg.b, 1.0))

// ---- Drawing ---------------------------------------------------------------------

// The cinema's bake lights aren't in its layout, and the mall's reach
// further: they start brighter.
let baseFor = (archetype: string): float => if archetype == "cinema" then 0.3 else if archetype == "mall" then 0.45 else 0.0
// A copy's debris, drawn.
let ofCopy = (c: Block.Copy, l: Layout, archetype: string, day: bool): Scene.t =>
  sceneOf(candidatesOf(swatchesFor(archetype), seedOf(c), l), l, day, baseFor(archetype))
// The furniture swatches an archetype paints its chairs in.
let swatchesFor = (archetype: string): List<Building.Paint> =>
  if archetype == "cinema" then CinemaBuilding.furnishSwatches
  else if archetype == "house-ochre" then HousePalettes.swatchesOf(HousePalettes.ochreColours)
  else if archetype == "house-magenta" then HousePalettes.swatchesOf(HousePalettes.magentaColours)
  else Building.officeSwatches
// One instanced unlit cube for all of `cs`, in their cells.
let sceneOf = (cs: List<Candidate>, l: Layout, day: bool, base: float): Scene.t =>
  if List.length(cs) == 0.0 then Scene.group([])
  else Scene.cube() |> Scene.color(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(cs |> List.concatMap((c) =>
      let (kr, kg, kb) = lightAt(l, c.cell, c.at, day, base) in
      c.parts |> List.map((p) =>
        let (dx, dz) = Block.turn(c.yaw, p.x, p.z) in
        Instance.trs(v(Vec3.x(c.at) + dx, Vec3.y(c.at) - Vec3.y(c.shape) / 2.0 + p.y, Vec3.z(c.at) + dz), Angle.degrees(c.yaw), p.w, p.h, p.d)
          |> Instance.tint(Color.rgb(p.r * kr, p.g * kg, p.b * kb)))))
    |> Scene.inCells(roomsOf(l))
