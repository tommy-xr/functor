// Small things strewn about the furnished rooms: loose papers, a carton, a
// mug, a binder, a fallen chair. Not baked: each copy strews its own (from
// its seed), at most `cap` a room, clear of the furniture and the doors,
// drawn as one instanced unlit cube per copy in their cells, each box tinted
// to the light the bake gives its spot. Each is a debris candidate, a tag
// and a body's box shape. Once a blast or a broken window disturbs a copy
// with colliders, each is a dynamic body (`Body`, in a `Group`), resting
// where it was strewn; one that has moved (loose) draws on its own at its
// body's pose, the rest stay in the instanced cube. A broken window blows
// litter (papers, leaves, a takeaway carton) in.

open Layout

let v = Vec3.make
let noise = Building.noise

type Thing =
  | Paper
  | Carton
  | Mug
  | Binder
  | FallenChair
  | Leaf
  | Takeaway

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
let leafBrown = (0.42, 0.28, 0.14)
let leafOchre = (0.62, 0.5, 0.2)

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
  | Leaf => [box(0.0, 0.0, 0.0, 0.07 + 0.04 * noise(k), 0.004, 0.05 + 0.03 * noise(k + 1.0), if noise(k + 2.0) < 0.5 then leafBrown else leafOchre)]
  | Takeaway => [box(0.0, 0.0, 0.0, 0.13, 0.08, 0.1, mugWhite), box(0.0, 0.03, 0.0, 0.132, 0.025, 0.102, binderRed)]

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
// A copy's debris, drawn, but for its `loose` ones (by candidate tag).
let ofCopy = (c: Block.Copy, l: Layout, archetype: string, day: bool, loose: List<string>): Scene.t =>
  sceneOf(candidatesOf(swatchesFor(archetype), seedOf(c), l) |> List.filter((d) => not (loose |> List.any((t) => t == d.tag))),
          l, day, baseFor(archetype))
// The furniture swatches an archetype paints its chairs in.
let swatchesFor = (archetype: string): List<Building.Paint> =>
  if archetype == "cinema" then CinemaBuilding.furnishSwatches
  else if archetype == "house-ochre" then HousePalettes.swatchesOf(HousePalettes.ochreColours)
  else if archetype == "house-magenta" then HousePalettes.swatchesOf(HousePalettes.magentaColours)
  else Building.officeSwatches
// One instanced unlit cube for all of `cs`, in their cells.
let sceneOf = (cs: List<Candidate>, l: Layout, day: bool, base: float): Scene.t =>
  if List.length(cs) == 0.0 then Scene.group([])
  else cubes(cs |> List.concatMap((c) => placed(c.at, c.yaw, Vec3.y(c.shape), lightAt(l, c.cell, c.at, day, base), c.parts)))
    |> Scene.inCells(roomsOf(l))
// An item's parts as instances, its body's centre at `at`, turned `yaw`
// degrees, `h` tall, tinted `k`.
let placed = (at: Vec3.t, yaw: float, h: float, k: (float, float, float), parts: List<Part>): List<Instance.t> =>
  let (kr, kg, kb) = k in
  parts |> List.map((p) =>
    let (dx, dz) = Block.turn(yaw, p.x, p.z) in
    Instance.trs(v(Vec3.x(at) + dx, Vec3.y(at) - h / 2.0 + p.y, Vec3.z(at) + dz), Angle.degrees(yaw), p.w, p.h, p.d)
      |> Instance.tint(Color.rgb(p.r * kr, p.g * kg, p.b * kb)))
let cubes = (is: List<Instance.t>): Scene.t => Scene.cube() |> Scene.color(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(is)

// ---- Bodies ----------------------------------------------------------------------

// A debris body, as plain data the model holds: its world tag and its
// candidate's (`local`), its kind and cell (-1: outdoors), where it rests in
// the world (its centre, turned `yaw` degrees), its box, its parts, their
// tints by night and by day, and where it is once it has moved (`pose`).
type Pose = { x: float, y: float, z: float, still: bool }
type Body = { tag: string, local: string, kind: Thing, cell: float, x: float, y: float, z: float, yaw: float,
              w: float, h: float, d: float, parts: List<Part>, night: (float, float, float), day: (float, float, float),
              pose: Option.t<Pose> }

let bodyOf = (c: Block.Copy, local: string, kind: Thing, cell: float, at: Vec3.t, yaw: float, parts: List<Part>,
              night: (float, float, float), day: (float, float, float)): Body =>
  let p = Block.toWorld(c, at) in
  let e = extentOf(parts) in
  { tag: Block.tagOf(c, local), local: local, kind: kind, cell: cell, x: Vec3.x(p), y: Vec3.y(p), z: Vec3.z(p), yaw: yaw + c.yaw,
    w: e.x1 - e.x0, h: e.top, d: e.z1 - e.z0, parts: parts, night: night, day: day, pose: Option.None }

// Copy `c`'s strewn debris, resting where it was strewn.
let strewnBodies = (c: Block.Copy): List<Body> =>
  let l = Block.archetypeOf(c).layout in
  let base = baseFor(c.archetype) in
  candidatesOf(swatchesFor(c.archetype), seedOf(c), l) |> List.map((d) =>
    bodyOf(c, d.tag, d.kind, d.cell, d.at, d.yaw, d.parts, lightAt(l, d.cell, d.at, false, base), lightAt(l, d.cell, d.at, true, base)))

// Litter outside copy `c`'s window `i` (`o`), there once it breaks: papers,
// leaves and a takeaway carton across the opening, just outside it, between
// its sill and head, already loose. Its draft carries them in; they take
// the light 1.5 m inside.
let litterKinds = [Paper, Leaf, Paper, Leaf, Takeaway, Paper]
let litterOf = (c: Block.Copy, i: float, o: Glass.Opening): List<Body> =>
  let n = List.length(litterKinds) in
  let l = Block.archetypeOf(c).layout in
  let inside = Block.toLocal(c, v(o.x - 1.5 * o.nx, o.floor, o.z - 1.5 * o.nz)) in
  let lit = (day: bool) => lightAt(l, o.room, inside, day, baseFor(c.archetype)) in
  litterKinds |> List.indexedMap((k, kind) =>
    let s = seedOf(c) + i * 5.3 + k * 1.9 in
    let along = (o.w - 0.5) * ((k + 0.5) / n - 0.5) in
    let out = 0.35 + 0.3 * noise(s + 0.7) in
    let up = o.sill + 0.3 + (o.head - o.sill - 0.6) * noise(s + 0.3) in
    let at = Block.toLocal(c, v(o.x + o.nx * out + o.ax * along, up, o.z + o.nz * out + o.az * along)) in
    let b = bodyOf(c, $"litter-{i}-{k}", kind, -1.0, at, 360.0 * noise(s + 0.9), centred(partsOf([], kind, s)), lit(false), lit(true)) in
    { b with pose: Option.Some({ x: b.x, y: b.y, z: b.z, still: false }) })

// How each kind moves: its mass (kg) and damping. Paper, leaves and the
// takeaway drift down slowly and settle; a carton or a chair barely notices
// a draft.
type Stuff = { mass: float, linear: float, angular: float }
let stuffOf = (kind: Thing): Stuff =>
  match kind with
  | Paper => { mass: 0.03, linear: 3.0, angular: 1.0 }
  | Leaf => { mass: 0.03, linear: 3.0, angular: 1.0 }
  | Takeaway => { mass: 0.04, linear: 1.5, angular: 0.6 }
  | Mug => { mass: 0.35, linear: 0.2, angular: 0.4 }
  | Binder => { mass: 0.8, linear: 0.3, angular: 0.4 }
  | Carton => { mass: 3.0, linear: 0.2, angular: 0.3 }
  | FallenChair => { mass: 5.0, linear: 0.1, angular: 0.2 }

// A body as a dynamic body at its rest. Swept (`continuous`): thin things
// thrown fast would otherwise step through the floor's or a wall's face.
let declare = (b: Body): Physics.body =>
  let s = stuffOf(b.kind) in
  Physics.dynamic(Physics.tag(b.tag), Physics.box(b.w, b.h, b.d)) |> Physics.at(v(b.x, b.y, b.z)) |> Physics.rotateY(Angle.degrees(b.yaw))
    |> Physics.mass(s.mass) |> Physics.linearDamping(s.linear) |> Physics.angularDamping(s.angular) |> Physics.continuous

// ---- Groups ----------------------------------------------------------------------

// A copy's debris as bodies. A copy's debris stays in its instanced cube,
// with no bodies, until something disturbs it: a blast that reaches it, or
// a window it breaks. Each body is declared every frame (dynamic bodies
// can't be memoised), so an untouched building costs nothing. `windows`:
// the broken windows whose litter it holds; `loose`: the candidate tags of
// its loose bodies; `moving`: whether any of those is awake.
type Group = { copy: string, windows: List<float>, bodies: List<Body>, loose: List<string>, moving: bool }

// Each group's bodies read back while one might move (`stirred`, or one
// is): a body more than 2 cm from its rest is loose, and still once it
// sleeps (a sleeper's velocity is exactly zero). A pre-step read: the last
// step's world.
let observed = (stirred: bool, groups: List<Group>): List<Group> =>
  groups |> List.map((g) =>
    if not (stirred || g.moving) then g
    else
      let bodies = g.bodies |> List.map((b) =>
        let t = Physics.tag(b.tag) in
        let p = Physics.position(t) in
        let (dx, dy, dz) = (p.x - b.x, p.y - b.y, p.z - b.z) in
        if dx * dx + dy * dy + dz * dz < 0.0004 && b.pose == Option.None then b
        else
          let u = Physics.linearVelocity(t) in
          { b with pose: Option.Some({ x: p.x, y: p.y, z: p.z, still: u.x == 0.0 && u.y == 0.0 && u.z == 0.0 }) }) in
      withLoose({ g with bodies: bodies }))
let withLoose = (g: Group): Group =>
  let posed = g.bodies |> List.concatMap((b) => match b.pose with
    | Option.Some(p) => [(b.local, p.still)]
    | Option.None => []) in
  { g with loose: posed |> List.map((e) => let (t, s) = e in t), moving: posed |> List.any((e) => let (t, s) = e in not s) }

// A broken window: its copy's name, its index in the layout's windows, and
// the draft through it.
type Broken = { copy: string, window: float, draft: Glass.Draft }

// The groups for town `town`'s copies `solid`: each solid copy keeps its
// group (its bodies where they are), and gets one when a blast reached it
// (`blasted`) or it has a broken window; a broken window without its litter
// gets it. A copy that leaves the solid ones drops its group.
let groupsFor = (town: Block.Town, solid: List<float>, broken: List<Broken>, blasted: List<float>, was: List<Group>): List<Group> =>
  solid |> List.concatMap((id) =>
    let c = Block.copyIn(town, id) in
    let windows = broken |> List.filter((w) => w.copy == c.name) |> List.map((w) => w.window) in
    match was |> List.find((g) => g.copy == c.name) with
    | Option.Some(g) => [littered(c, windows, g)]
    | Option.None =>
      if List.isEmpty(windows) && not Block.isIn(id, blasted) then []
      else [littered(c, windows, { copy: c.name, windows: [], bodies: strewnBodies(c), loose: [], moving: false })])
let littered = (c: Block.Copy, windows: List<float>, g: Group): Group =>
  let fresh = windows |> List.filter((i) => not Block.isIn(i, g.windows)) in
  if List.isEmpty(fresh) then g
  else
    withLoose({ g with windows: windows, bodies: g.bodies |> List.append(Glass.openingsOf(c)
      |> List.filter((e) => let (i, o) = e in Block.isIn(i, fresh)) |> List.concatMap((e) => let (i, o) = e in litterOf(c, i, o))) })

// Every loose body, each drawn at its body's live pose: one instanced cube
// of its parts round its centre, built once (`Scene.memo`).
type Look = { h: float, parts: List<Part>, tint: (float, float, float) }
let looseScenes = (groups: List<Group>, day: bool): List<Scene.t> =>
  groups |> List.concatMap((g) => g.bodies |> List.filter((b) => b.pose != Option.None) |> List.map((b) =>
    Scene.memo(b.tag, { h: b.h, parts: b.parts, tint: if day then b.day else b.night }, lookScene) |> Physics.transformed(Physics.tag(b.tag))))
let lookScene = (k: Look): Scene.t => cubes(placed(v(0.0, 0.0, 0.0), 0.0, k.h, k.tint, k.parts))
