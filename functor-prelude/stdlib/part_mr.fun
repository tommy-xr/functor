// PartMR — printable parts at true scale in your real room, in a headset's
// passthrough, before you print them: 1 mm in a part is 1 mm in the room.
// Each view is a name and its pieces (`{ solid, color }`, mm, Z up;
// PartViews pieces work), e.g. an assembly and its exploded view.
//
// Quest: a beam from the right hand shows where it points and highlights the
// part a squeeze would take: the one in reach of the hand, else the one the
// beam is on. Squeeze the right grip to pick it up; let go and it drops onto
// the real surface below it (a desk, the floor). Assembled, you
// hold the whole thing; B takes it apart so each piece is held on its own,
// and B again puts it back together. The right thumbstick turns (left/right)
// and tilts (forward/back) the part last held, A cycles the views, X toggles
// passthrough. Desktop (`run native --emulate-xr`): a stand-in room with a
// desk; Enter squeezes, the arrows are the thumbstick, 1 (or N) cycles, B
// takes apart, P toggles passthrough.
//
//   let views = PartMR.views([{ name: "Assembled", pieces: assembled }, …])  // meshed once
//   let config = PartMR.config()
//   let settings = PartMR.settings
//   let physics = PartMR.physics
//   let init = PartMR.init(views)
//   let sampledInput = (m, s) => PartMR.sampledInput(views, m, s)
//   let tick = (m, dt, tts) => PartMR.tick(views, m, dt)
//   let draw = (m, tts) => PartMR.draw(views, m)
//
// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// Xr.scene: the room's depth, so real things in front (a hand, the tablet a
// case is for) hide the parts behind them, and the room fuses into surfaces
// the parts rest on.
let config = () =>
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))

let settings = (m) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.passthroughEnabled(m.mixedReality))

// The room the headset fuses from its depth, solid, so a ray down from a part
// finds the surface under it.
let roomTag = () => Physics.tag("room")
let physics = (m) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [Physics.fusedRoom(roomTag())])

// Level (pitch 0) so the room stays level on the headset. On the headset the
// eye is where your head was at launch, so places here are relative to it;
// the field of view only frames the desktop view, wide so the stand-in
// desk's depth fuses into a surface to rest on.
let eye = () => Vec3.make(0.0, 1.1, 0.0)
let camera = () =>
  Camera3D.firstPerson(eye(), Angle.degrees(0.0), Angle.degrees(0.0), Angle.degrees(90.0))
    |> Camera3D.clip(0.02, 50.0)

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

// One decimal place, dropping a trailing ".0": 60, 1.8.
let mm = (v: float) => Text.fromFloat(Math.round(v * 10.0) / 10.0)

// A part-space point (mm, Z up) in scene metres (Y up), turned 180° about Y
// as the pieces are, so the part's front (-Y) faces you.
let fromPart = (v: Vec3.t): Input.point3 =>
  let s = PartViews.toScene(v) in
  point(0.0 - Vec3.x(s), Vec3.y(s), 0.0 - Vec3.z(s))

// Half extents in scene metres of a part-space size: x across, y up, z deep.
let halfOf = (size: Vec3.t): Input.point3 =>
  let s = PartViews.toScene(size) in
  point(Vec3.x(s) / 2.0, Vec3.y(s) / 2.0, Math.abs(Vec3.z(s)) / 2.0)

let at = (p: Input.point3, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

// A piece ready to show: centred on its own middle, where that middle sits
// in the assembly (`home`, from the view's middle), and its half extents.
let preparePiece = (viewCenter: Vec3.t, piece) =>
  let b = PartViews.bounds([piece]) in
  let center = Vec3.add(b.min, b.max) |> Vec3.scale(0.5) in
  {
    scene:
      Solid.toScene(piece.solid |> Solid.translate(center |> Vec3.scale(-1.0)))
        |> Scene.lit(piece.color)
        |> Scene.rotateY(Angle.degrees(180.0)),
    home: fromPart(Vec3.sub(viewCenter, center)),
    half: halfOf(Vec3.sub(b.min, b.max)),
  }

// A view ready to show: its pieces, all of them assembled (`whole`), its
// half extents, and a label of its name and size in mm.
let prepare = (view) =>
  let b = PartViews.bounds(view.pieces) in
  let center = Vec3.add(b.min, b.max) |> Vec3.scale(0.5) in
  let size = Vec3.sub(b.min, b.max) in
  let font = Typeface.sans() in
  let text = $"{view.name} · {mm(Vec3.x(size))} × {mm(Vec3.y(size))} × {mm(Vec3.z(size))} mm" in
  let label =
    Sketch.text(font, 8.0, text)
      |> Sketch.translate(0.0 - Sketch.textWidth(font, 8.0, text) / 2.0, -4.0)
      |> Solid.extrude(0.8) in
  let pieces = view.pieces |> List.map((p) => preparePiece(center, p)) in
  {
    pieces: pieces,
    whole: Scene.group(pieces |> List.map((p) => p.scene |> at(p.home))),
    half: halfOf(size),
    // Turned to read from your side, then propped up 45°.
    label:
      Solid.toScene(label)
        |> Scene.emissive(Color.rgb(0.35, 0.9, 1.0))
        |> Scene.rotateY(Angle.degrees(180.0))
        |> Scene.rotateX(Angle.degrees(-45.0))
        |> Scene.translate(Vec3.make(0.0, 0.003, 0.0)),
  }

// Every view prepared: call once at top level, so each is meshed once.
let views = (named) => List.map(prepare, named)

let viewAt = (views, i: float) =>
  match List.nth(i, views) with
  | Option.Some(v) => Option.Some(v)
  | Option.None => List.head(views)

// Something you can pick up: the whole view, or one piece when apart.
// `settles`: it rests on the room — not while it sits where the assembly
// put it, until it has been held and let go.
type item = { position: Input.point3, yaw: float, pitch: float, settles: bool }

// How far below its middle an item's lowest point is, tilted by `pitch`.
let drop = (half: Input.point3, pitch: float) =>
  half.y * Math.abs(Math.cos(pitch)) + half.z * Math.abs(Math.sin(pitch))

let radius = (half: Input.point3) => Math.sqrt(half.x * half.x + half.y * half.y + half.z * half.z)

// `v` turned by `yaw` about +Y, as Scene.rotateY turns it.
let turn = (yaw: float, v: Input.point3): Input.point3 =>
  point(v.x * Math.cos(yaw) + v.z * Math.sin(yaw), v.y, v.z * Math.cos(yaw) - v.x * Math.sin(yaw))

// `v` tilted by `pitch` about +X, as Scene.rotateX tilts it.
let tilt = (pitch: float, v: Input.point3): Input.point3 =>
  point(v.x, v.y * Math.cos(pitch) - v.z * Math.sin(pitch), v.y * Math.sin(pitch) + v.z * Math.cos(pitch))

// Point arithmetic, a + b and a − b (Vec3.sub is pipe-last, b − a).
let plus = (a: Input.point3, b: Input.point3) => point(a.x + b.x, a.y + b.y, a.z + b.z)
let minus = (a: Input.point3, b: Input.point3) => point(a.x - b.x, a.y - b.y, a.z - b.z)

// The pieces of `view` where the assembled `whole` puts them, staying there.
let partsOf = (view, whole: item): List<item> =>
  view.pieces |> List.map((p) =>
    { position: plus(whole.position, turn(whole.yaw, tilt(whole.pitch, p.home))), yaw: whole.yaw, pitch: whole.pitch, settles: false })

// The grip pose when nothing tracks it (desktop without --emulate-xr).
let fallbackGrip = () =>
  Camera3D.mapTrackedPose(camera(), { position: { x: 0.2, y: -0.3, z: -0.3 }, orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 } })

// Which way a pose faces about +Y: its forward's level part, with its up's
// (turned to the same side) blended in as the hand tips toward straight down
// (setting a part on a desk) or up, so it still faces ahead there, has no
// jump in between, and a level hand's roll doesn't turn it.
let heading = (pose: Camera3D.mappedPose) =>
  let f = pose.forward in
  let u = pose.up in
  let w = (if f.y >= 0.0 then 1.0 else -1.0) * f.y * f.y in
  Math.atan2(f.x - w * u.x, f.z - w * u.z)

// `grip`: the right controller's grip pose; `squeezing`: its grip held;
// `index`: the view shown; `whole`: where the assembled view is; `apart`:
// taken apart, each piece at `parts`; `held`: the item in hand (-1 none; 0 is
// the whole view when assembled, else a piece), kept at `offset` from the
// grip and `yawOffset` from its heading; `active`: the item the thumbstick
// turns; the `…Pressed` fields latch buttons so a hold acts once.
// `aim`: its pointing pose; `pointed`: the item the beam is on (-1 none),
// and `beamEnd` where the beam stops (that item, else the room); `along`:
// how far down the beam an item taken with it rides (-1 for one in hand).
type t = {
  grip: Camera3D.mappedPose, gripTracked: bool, squeezing: bool, stick: Input.point2,
  aim: Camera3D.mappedPose, aimTracked: bool, pointed: float, beamEnd: Input.point3, along: float,
  index: float, whole: item, apart: bool, parts: List<item>,
  held: float, offset: Input.point3, yawOffset: float, active: float,
  mixedReality: bool, togglePressed: bool, cyclePressed: bool, apartPressed: bool
}

// Half a metre ahead of where you faced at launch, a little below your eyes;
// it drops onto what the room has below.
let init = (views): t =>
  let e = eye() in
  { grip: fallbackGrip(), gripTracked: false, squeezing: false, stick: { x: 0.0, y: 0.0 },
    aim: fallbackGrip(), aimTracked: false, pointed: -1.0, beamEnd: point(0.0, 0.0, 0.0), along: -1.0,
    index: 0.0, whole: { position: point(Vec3.x(e), Vec3.y(e) - 0.1, Vec3.z(e) + 0.5), yaw: 0.0, pitch: 0.0, settles: true },
    apart: false, parts: [],
    held: -1.0, offset: point(0.0, 0.0, 0.0), yawOffset: 0.0, active: 0.0,
    mixedReality: true, togglePressed: false, cyclePressed: false, apartPressed: false }

// Apart, with a part for each of the view's pieces (a hot reload can change
// how many there are).
let isApart = (view, m: t) => m.apart && List.length(m.parts) == List.length(view.pieces)

// What can be picked up now, with each one's half extents.
let items = (views, m: t) =>
  match viewAt(views, m.index) with
  | Option.None => []
  | Option.Some(view) =>
    if isApart(view, m) then List.zip(view.pieces, m.parts) |> List.map((pair) => let (it, piece) = pair in { item: it, half: piece.half })
    else [{ item: m.whole, half: view.half }]

// The item the grip can take: the nearest whose bounds come within `slack`.
let slack = 0.08
let inReach = (views, m: t): float =>
  items(views, m)
    |> List.indexedMap((i, e) => { i: i, gap: radius(minus(e.item.position, m.grip.position)) - radius(e.half) })
    |> List.filter((c) => c.gap <= slack)
    |> List.sortBy((c) => c.gap)
    |> List.head
    |> Option.map((c) => c.i)
    |> Option.defaultValue(-1.0)

// What a squeeze takes: the item in reach of the hand, else the one the beam
// is on (-1 none).
let selected = (views, m: t): float =>
  let near = inReach(views, m) in
  if near >= 0.0 then near else m.pointed

let dot = (a: Input.point3, b: Input.point3) => a.x * b.x + a.y * b.y + a.z * b.z
let scaled = (k: float, v: Input.point3) => point(v.x * k, v.y * k, v.z * k)

// Take the selected item. In reach, it keeps where the hand holds it; taken
// with the beam, it rides the beam that far out, so pointing moves it.
let grabbed = (views, m: t): t =>
  let near = inReach(views, m) >= 0.0 in
  let i = selected(views, m) in
  match List.nth(i, items(views, m)) with
  | Option.None => m
  | Option.Some(e) =>
    if near then
      let h = heading(m.grip) in
      { m with held: i, active: i, along: -1.0, offset: turn(0.0 - h, minus(e.item.position, m.grip.position)), yawOffset: e.item.yaw - h }
    else
      let h = heading(m.aim) in
      let along = dot(minus(e.item.position, m.aim.position), m.aim.forward) in
      let onBeam = plus(m.aim.position, scaled(along, m.aim.forward)) in
      { m with held: i, active: i, along: along, offset: turn(0.0 - h, minus(e.item.position, onBeam)), yawOffset: e.item.yaw - h }

// Let go: the held item rests on the room from now on.
let released = (m: t): t =>
  if m.held < 0.0 then m
  else if m.apart then { m with held: -1.0, parts: m.parts |> List.indexedMap((j, p) => if j == m.held then { p with settles: true } else p) }
  else { m with held: -1.0, whole: { m.whole with settles: true } }

// Apart: each piece where the assembly had it, so nothing jumps, staying
// there until moved. Together: back into the assembly, wherever it was.
let apartToggled = (views, m: t): t =>
  if m.apart then { m with apart: false, held: -1.0, active: 0.0 }
  else match viewAt(views, m.index) with
  | Option.None => m
  | Option.Some(view) => { m with apart: true, parts: partsOf(view, m.whole), held: -1.0, active: 0.0 }

let toggled = (m: t): t => { m with mixedReality: not m.mixedReality }
// The next view takes the last one's place, turned and tilted the same, so
// an assembly and its exploded view compare side by side. It shows
// assembled. With no views there is nothing to cycle.
let cycled = (views, m: t): t =>
  if List.length(views) == 0.0 then m
  else { m with index: Math.mod(m.index + 1.0, List.length(views)), apart: false, held: -1.0, active: 0.0 }

// How far along the ray from `o` (unit direction `d`) it enters the item's
// posed box, if it does ahead of `o` (0 from inside it): the ray brought
// into the item's own frame (undo its place, turn and tilt), then the slab
// test on each axis.
let alongTo = (o: Input.point3, d: Input.point3, it: item, half: Input.point3) =>
  let local = (v) => tilt(0.0 - it.pitch, turn(0.0 - it.yaw, v)) in
  let lo = local(minus(o, it.position)) in
  let ld = local(d) in
  let slab = (o1, d1, h) =>
    if Math.abs(d1) < 0.000001 then (if Math.abs(o1) <= h then { near: -1000000.0, far: 1000000.0 } else { near: 1.0, far: 0.0 })
    else
      let a = (0.0 - h - o1) / d1 in
      let b = (h - o1) / d1 in
      { near: Math.min(a, b), far: Math.max(a, b) } in
  let x = slab(lo.x, ld.x, half.x) in
  let y = slab(lo.y, ld.y, half.y) in
  let z = slab(lo.z, ld.z, half.z) in
  let near = Math.max(Math.max(x.near, y.near), z.near) in
  let far = Math.min(Math.min(x.far, y.far), z.far) in
  if near > far || far < 0.0 then Option.None else Option.Some(Math.max(near, 0.0))

// How far the beam goes before the room stops it, and how far ahead of the
// hand the room counts (the depth sensor sees the hand, so it can fuse).
let beamLength = 3.0
let handClear = 0.1

// Where the beam from the aim is: the nearest item it meets in front of
// the room, else as far as `room` says the room lets it go (the fused room
// in play; tests pass their own). Nothing while not aiming.
let aimedWith = (views, m: t, room): t =>
  if not m.aimTracked then { m with pointed: -1.0 }
  else
    let o = m.aim.position in
    let d = m.aim.forward in
    let wall = room(plus(o, scaled(handClear, d)), d) |> Option.map((t) => t + handClear) |> Option.defaultValue(beamLength) in
    let hit =
      items(views, m)
        |> List.indexedMap((i, e) => { i: i, t: alongTo(o, d, e.item, e.half) |> Option.defaultValue(-1.0) })
        |> List.filter((c) => c.t >= 0.0 && c.t < wall)
        |> List.sortBy((c) => c.t)
        |> List.head in
    let reach = hit |> Option.map((c) => c.t) |> Option.defaultValue(wall) in
    { m with
      pointed: hit |> Option.map((c) => c.i) |> Option.defaultValue(-1.0),
      beamEnd: plus(o, scaled(reach, d)) }

// How far along the ray the fused room stops it, if within `beamLength`.
let roomAlong = (o: Input.point3, d: Input.point3) =>
  let hit = Physics.cast(Vec3.make(o.x, o.y, o.z), Vec3.make(d.x, d.y, d.z), beamLength) in
  if hit.hit then Option.Some(hit.distance) else Option.None

// Keyboard edges, so a held key's auto-repeat doesn't act again.
let pressed = (s: Input.snapshot, key) => s.pressedKeys |> List.any((k) => k == key)

// The grip takes above 0.6 and lets go below 0.4, so a squeeze held near
// halfway doesn't drop and retake.
let squeezed = (was: bool, squeeze: float) => if was then squeeze > 0.4 else squeeze > 0.6

// `room` is `roomAlong` in play: the beam is re-aimed with this step's poses
// and view, so a squeeze takes what the beam is on now.
let sampledInputWith = (views, current: t, s: Input.snapshot, room): t =>
  let byKey = if pressed(s, Key.P) then toggled(current) else current in
  let byKey = if pressed(s, Key.B) then apartToggled(views, byKey) else byKey in
  let m = if pressed(s, Key.N) then cycled(views, byKey) else byKey in
  match s.xr with
  | Option.None => released({ m with gripTracked: false, squeezing: false, aimTracked: false, stick: { x: 0.0, y: 0.0 } })
  | Option.Some(xr) =>
    let gripTracked = xr.right.active && Option.isSome(xr.right.grip) in
    let squeezing = gripTracked && squeezed(m.squeezing, xr.right.squeeze) in
    let toggle = xr.left.primaryPressed in
    let cycle = xr.right.primaryPressed in
    let apart = xr.right.secondaryPressed in
    let next = { m with
      grip: xr.right.grip |> Option.map((pose) => Camera3D.mapTrackedPose(camera(), pose)) |> Option.defaultValue(fallbackGrip()),
      gripTracked: gripTracked,
      aim: xr.right.aim |> Option.map((pose) => Camera3D.mapTrackedPose(camera(), pose)) |> Option.defaultValue(fallbackGrip()),
      aimTracked: xr.right.active && Option.isSome(xr.right.aim),
      squeezing: squeezing,
      stick: xr.right.thumbstick,
      togglePressed: toggle,
      cyclePressed: cycle,
      apartPressed: apart } in
    let next = if toggle && not m.togglePressed then toggled(next) else next in
    let next = if apart && not m.apartPressed then apartToggled(views, next) else next in
    let next = if cycle && not m.cyclePressed then cycled(views, next) else next in
    let next = aimedWith(views, next, room) in
    if squeezing && not m.squeezing then grabbed(views, next)
    else if squeezing then next
    else released(next)

let sampledInput = (views, current: t, s: Input.snapshot): t => sampledInputWith(views, current, s, roomAlong)

let approach = (from: Input.point3, target: Input.point3, amount: float): Input.point3 =>
  point(from.x + (target.x - from.x) * amount, from.y + (target.y - from.y) * amount, from.z + (target.z - from.z) * amount)

// The thumbstick dead zone, and the turn rate in radians per second at full tilt.
let deadZone = 0.2
let turnRate = 2.0
let axis = (v: float) => if Math.abs(v) < deadZone then 0.0 else v

// How far below a part's top a surface may be for it to drop there. Farther,
// it waits where you let go: the desk under it may just not be fused yet,
// and the floor (fused, or the floor plane) would swallow it.
let reachDown = 0.5

// The height of the real surface under `p` (the fused room) within
// `reachDown`, if the headset has seen one there yet.
let surfaceBelow = (p: Input.point3) =>
  let hit = Physics.cast(Vec3.make(p.x, p.y, p.z), Vec3.make(0.0, -1.0, 0.0), reachDown) in
  if hit.hit then Option.Some(hit.y) else Option.None

// One item's step. Held, it follows the grip, the thumbstick turning it in
// the hand. Otherwise the thumbstick turns the active item, and one that
// settles drops onto the surface `below` finds under its top (so one let go
// half inside a desk still rests on it), never sinking into it, and stays
// put over nothing yet seen.
let stepped = (m: t, below, dt: float, i: float, it: item, half: Input.point3): item =>
  let spin = axis(m.stick.x) * turnRate * dt in
  let tilted = if i == m.active then { it with pitch: Math.clamp(0.0 - Math.pi / 2.0, Math.pi / 2.0, it.pitch + axis(m.stick.y) * turnRate * dt) } else it in
  if i == m.held then
    let hand = if m.along >= 0.0 then m.aim else m.grip in
    let from = if m.along >= 0.0 then plus(m.aim.position, scaled(m.along, m.aim.forward)) else m.grip.position in
    { tilted with position: plus(from, turn(heading(hand), m.offset)), yaw: heading(hand) + m.yawOffset + spin }
  else
    let turned = if i == m.active then { tilted with yaw: tilted.yaw + spin } else tilted in
    if not turned.settles then turned
    else match below({ turned.position with y: turned.position.y + drop(half, turned.pitch) }) with
    | Option.None => turned
    | Option.Some(surface) =>
      let rest = surface + drop(half, turned.pitch) in
      let p = approach(turned.position, { turned.position with y: rest }, Math.min(1.0, dt * 10.0)) in
      { turned with position: { p with y: Math.max(p.y, rest) } }

// `below` is `surfaceBelow` in play; tests pass their own floor. A view
// whose piece count changed under a hot reload goes back together.
let tickWith = (views, m: t, dt: float, below): t =>
  match viewAt(views, m.index) with
  | Option.None => m
  | Option.Some(view) =>
    let m = if m.apart && not isApart(view, m) then { m with apart: false, held: -1.0, active: 0.0 } else m in
    // The thumbstick turns the held item in the hand: fold it into the offset.
    let m = if m.held >= 0.0 then { m with yawOffset: m.yawOffset + axis(m.stick.x) * turnRate * dt } else m in
    let still = { m with stick: { m.stick with x: if m.held >= 0.0 then 0.0 else m.stick.x } } in
    if m.apart then
      { m with parts: List.zip(view.pieces, m.parts) |> List.indexedMap((i, pair) => let (it, piece) = pair in stepped(still, below, dt, i, it, piece.half)) }
    else { m with whole: stepped(still, below, dt, 0.0, m.whole, view.half) }

let tick = (views, m: t, dt: float): t => tickWith(views, m, dt, surfaceBelow)

let posed = (it: item, scene) =>
  scene |> Scene.rotateX(Angle.radians(it.pitch)) |> Scene.rotateY(Angle.radians(it.yaw)) |> at(it.position)

// Where the stand-in room is drawn (desktop, a paused feed): a floor and a
// 75 cm desk half a metre ahead, which also fuse into surfaces.
let standIn = () =>
  let e = eye() in
  let block = (w, h, d, x, y, z, shade) =>
    Scene.cube() |> Scene.scaleXYZ(w, h, d) |> Scene.lit(Color.rgb(shade, shade, shade + 0.04)) |> at(point(x, y, z)) in
  Scene.group([
    block(6.0, 0.02, 6.0, Vec3.x(e), -0.01, Vec3.z(e) + 1.0, 0.42),
    block(1.0, 0.75, 0.6, Vec3.x(e), 0.375, Vec3.z(e) + 0.5, 0.55),
  ])

let cyan = Color.rgb(0.35, 0.9, 1.0)

// A thin beam from `from` to `to`: the unit cylinder (along Y, about its
// middle) stood on its base and tipped along the direction.
let beamScene = (from: Input.point3, to: Input.point3, color) =>
  let d = minus(to, from) in
  let length = radius(d) in
  if length < 0.001 then Scene.group([])
  else
    Scene.group([
      Scene.cylinder()
        |> Scene.scaleXYZ(0.003, length, 0.003)
        |> Scene.translate(Vec3.make(0.0, length / 2.0, 0.0))
        |> Scene.rotateX(Angle.radians(Math.acos(Math.clamp(-1.0, 1.0, d.y / length))))
        |> Scene.rotateY(Angle.radians(Math.atan2(d.x, d.z)))
        |> Scene.emissive(color)
        |> at(from),
      Scene.sphere() |> Scene.scale(0.006) |> Scene.emissive(color) |> at(to),
    ])

// The beam while aiming and not holding: cyan on the part a squeeze takes,
// white elsewhere (or while one in reach of the hand is taken instead).
let beam = (views, m: t) =>
  if m.aimTracked && m.held < 0.0 then
    beamScene(m.aim.position, m.beamEnd, if m.pointed >= 0.0 && m.pointed == selected(views, m) then cyan else Color.rgb(0.8, 0.82, 0.86))
  else Scene.group([])

// A see-through box around the part a squeeze would take.
let highlight = (views, m: t) =>
  if m.held >= 0.0 then Scene.group([])
  else match List.nth(selected(views, m), items(views, m)) with
  | Option.None => Scene.group([])
  | Option.Some(e) =>
    Scene.cube()
      |> Scene.scaleXYZ(e.half.x * 2.0 + 0.006, e.half.y * 2.0 + 0.006, e.half.z * 2.0 + 0.006)
      |> Scene.emissive(cyan)
      |> Scene.opacity(0.22)
      |> posed(e.item)

// A dot at the hand: green holding, cyan with something in reach.
let gripDot = (views, m: t) =>
  if m.gripTracked then
    Scene.sphere()
      |> Scene.scale(0.008)
      |> Scene.emissive(
        if m.held >= 0.0 then Color.rgb(0.3, 1.0, 0.45)
        else if inReach(views, m) >= 0.0 then cyan
        else Color.rgb(0.95, 0.98, 1.0))
      |> at(m.grip.position)
  else Scene.group([])

let draw = (views, m: t) =>
  let shown = viewAt(views, m.index) |> Option.map((view) =>
    if isApart(view, m) then List.zip(view.pieces, m.parts) |> List.map((pair) => let (it, piece) = pair in piece.scene |> posed(it))
    else
      // The label props up just in front of the view's foot.
      let w = m.whole in
      let front = Math.sqrt(view.half.x * view.half.x + Math.max(view.half.y, view.half.z) * Math.max(view.half.y, view.half.z)) in
      [
        view.whole |> posed(w),
        view.label |> at(point(w.position.x, w.position.y - drop(view.half, w.pitch), w.position.z - front - 0.03)),
      ]) |> Option.defaultValue([]) in
  let frame =
    Frame.createLit(
      camera(),
      Scene.group(shown |> List.append([gripDot(views, m), beam(views, m), highlight(views, m)])),
      [
        Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.95, 0.9), 0.9),
      ]) in
  if m.mixedReality then
    let room = frame |> Frame.withPassthrough |> Frame.withStandInRoom(standIn()) in
    // Real things hide the parts, but not while one is held: the hand holding
    // it would cut into it.
    if m.held >= 0.0 then room else room |> Frame.withRoomOcclusion
  else frame |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.06))
