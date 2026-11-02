// PartMR — printable parts at true scale in your real room, in a headset's
// passthrough, before you print them: 1 mm in a part is 1 mm in the room.
// Each view is a name and its pieces (`{ solid, color }`, mm, Z up;
// PartViews pieces work), e.g. an assembly and its exploded view.
//
// Quest: squeeze the right grip near a part to pick it up; let go and it
// drops onto the real surface below it (a desk, the floor). Assembled, you
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

// Level (pitch 0) so the room stays level on the headset. The eye and field
// of view frame the desktop view (the headset uses its own optics), wide and
// above the stand-in desk so its depth fuses into a surface to rest on.
let camera = () =>
  Camera3D.firstPerson(Vec3.make(0.0, 1.2, -0.3), Angle.degrees(0.0), Angle.degrees(0.0), Angle.degrees(90.0))
    |> Camera3D.clip(0.02, 50.0)

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

// One decimal place, dropping a trailing ".0": 60, 1.8.
let mm = (v: float) => Text.fromFloat(Math.round(v * 10.0) / 10.0)

// A part-space point (mm, Z up) in scene metres (Y up), turned 180° about Y
// as the pieces are, so the part's front (-Y) faces you.
let fromPart = (v: Vec3.t): Input.point3 =>
  point(0.0 - Vec3.x(v) / 1000.0, Vec3.z(v) / 1000.0, Vec3.y(v) / 1000.0)

// Half extents in scene metres of a part-space size: x across, y up, z deep.
let halfOf = (size: Vec3.t): Input.point3 =>
  point(Vec3.x(size) / 2000.0, Vec3.z(size) / 2000.0, Vec3.y(size) / 2000.0)

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
    whole: Scene.group(pieces |> List.map((p) => p.scene |> Scene.translate(Vec3.make(p.home.x, p.home.y, p.home.z)))),
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
type item = { position: Input.point3, yaw: float, pitch: float }

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

let add = (a: Input.point3, b: Input.point3) => point(a.x + b.x, a.y + b.y, a.z + b.z)
let sub = (a: Input.point3, b: Input.point3) => point(a.x - b.x, a.y - b.y, a.z - b.z)

// The pieces of `view` where the assembled `whole` puts them.
let partsOf = (view, whole: item): List<item> =>
  view.pieces |> List.map((p) =>
    { position: add(whole.position, turn(whole.yaw, tilt(whole.pitch, p.home))), yaw: whole.yaw, pitch: whole.pitch })

// The grip pose when nothing tracks it (desktop without --emulate-xr).
let fallbackGrip = () =>
  Camera3D.mapTrackedPose(camera(), { position: { x: 0.2, y: -0.3, z: -0.3 }, orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 } })

// Which way a pose faces about +Y.
let heading = (pose: Camera3D.mappedPose) => Math.atan2(pose.forward.x, pose.forward.z)

// `grip`: the right controller's grip pose; `squeezing`: its grip held;
// `index`: the view shown; `whole`: where the assembled view is; `apart`:
// taken apart, each piece at `parts`; `held`: the item in hand (-1 none; 0 is
// the whole view when assembled, else a piece), kept at `offset` from the
// grip and `yawOffset` from its heading; `active`: the item the thumbstick
// turns; the `…Pressed` fields latch buttons so a hold acts once.
type t = {
  grip: Camera3D.mappedPose, gripTracked: bool, squeezing: bool, stick: Input.point2,
  index: float, whole: item, apart: bool, parts: List<item>,
  held: float, offset: Input.point3, yawOffset: float, active: float,
  mixedReality: bool, togglePressed: bool, cyclePressed: bool, apartPressed: bool
}

// Half a metre ahead at desk height; it drops onto what the room has below.
let init = (views): t =>
  { grip: fallbackGrip(), gripTracked: false, squeezing: false, stick: { x: 0.0, y: 0.0 },
    index: 0.0, whole: { position: point(0.0, 0.9, 0.5), yaw: 0.0, pitch: 0.0 }, apart: false, parts: [],
    held: -1.0, offset: point(0.0, 0.0, 0.0), yawOffset: 0.0, active: 0.0,
    mixedReality: true, togglePressed: false, cyclePressed: false, apartPressed: false }

// What can be picked up now, with each one's half extents.
let items = (views, m: t) =>
  match viewAt(views, m.index) with
  | Option.None => []
  | Option.Some(view) =>
    if m.apart then List.zip(view.pieces, m.parts) |> List.map((pair) => let (it, piece) = pair in { item: it, half: piece.half })
    else [{ item: m.whole, half: view.half }]

let withItem = (m: t, i: float, next: item): t =>
  if m.apart then { m with parts: m.parts |> List.indexedMap((j, p) => if j == i then next else p) }
  else { m with whole: next }

let itemAt = (views, m: t, i: float) =>
  List.nth(i, items(views, m)) |> Option.map((e) => e.item)

// The item the grip can take: the nearest whose bounds come within `slack`.
let slack = 0.08
let inReach = (views, m: t): float =>
  items(views, m)
    |> List.indexedMap((i, e) => { i: i, gap: Vec3.length(Vec3.make(e.item.position.x - m.grip.position.x, e.item.position.y - m.grip.position.y, e.item.position.z - m.grip.position.z)) - radius(e.half) })
    |> List.filter((c) => c.gap <= slack)
    |> List.sortBy((c) => c.gap)
    |> List.head
    |> Option.map((c) => c.i)
    |> Option.defaultValue(-1.0)

// Take the item in reach, keeping where the hand holds it.
let grabbed = (views, m: t): t =>
  let i = inReach(views, m) in
  match itemAt(views, m, i) with
  | Option.None => m
  | Option.Some(it) =>
    let h = heading(m.grip) in
    { m with held: i, active: i, offset: turn(0.0 - h, sub(it.position, m.grip.position)), yawOffset: it.yaw - h }

// Apart: each piece where the assembly had it, so nothing jumps. Together:
// back into the assembly, wherever it was.
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

// Keyboard edges, so a held key's auto-repeat doesn't act again.
let pressed = (s: Input.snapshot, key) => s.pressedKeys |> List.any((k) => k == key)

let sampledInput = (views, current: t, s: Input.snapshot): t =>
  let byKey = if pressed(s, Key.P) then toggled(current) else current in
  let byKey = if pressed(s, Key.B) then apartToggled(views, byKey) else byKey in
  let m = if pressed(s, Key.N) then cycled(views, byKey) else byKey in
  match s.xr with
  | Option.None => { m with gripTracked: false, squeezing: false, held: -1.0, stick: { x: 0.0, y: 0.0 } }
  | Option.Some(xr) =>
    let gripTracked = xr.right.active && Option.isSome(xr.right.grip) in
    let squeezing = gripTracked && xr.right.squeeze > 0.5 in
    let toggle = xr.left.primaryPressed in
    let cycle = xr.right.primaryPressed in
    let apart = xr.right.secondaryPressed in
    let next = { m with
      grip: xr.right.grip |> Option.map((pose) => Camera3D.mapTrackedPose(camera(), pose)) |> Option.defaultValue(fallbackGrip()),
      gripTracked: gripTracked,
      squeezing: squeezing,
      stick: xr.right.thumbstick,
      togglePressed: toggle,
      cyclePressed: cycle,
      apartPressed: apart } in
    let next = if toggle && not m.togglePressed then toggled(next) else next in
    let next = if apart && not m.apartPressed then apartToggled(views, next) else next in
    let next = if cycle && not m.cyclePressed then cycled(views, next) else next in
    if squeezing && not m.squeezing then grabbed(views, next)
    else if squeezing then next
    else { next with held: -1.0 }

let approach = (from: Input.point3, target: Input.point3, amount: float): Input.point3 =>
  point(from.x + (target.x - from.x) * amount, from.y + (target.y - from.y) * amount, from.z + (target.z - from.z) * amount)

// The thumbstick dead zone, and the turn rate in radians per second at full tilt.
let deadZone = 0.2
let turnRate = 2.0
let axis = (v: float) => if Math.abs(v) < deadZone then 0.0 else v

// The height of the real surface under `p` (the fused room), if the headset
// has seen one there yet.
let surfaceBelow = (p: Input.point3) =>
  let hit = Physics.cast(Vec3.make(p.x, p.y, p.z), Vec3.make(0.0, -1.0, 0.0), 3.0) in
  if hit.hit then Option.Some(hit.y) else Option.None

// One item's step: held, it follows the grip; otherwise it settles onto the
// surface `below` finds under it, never sinking into it, and stays put over
// nothing yet seen.
let stepped = (m: t, below, ease: float, i: float, e, turned: item): item =>
  if i == m.held then
    { turned with position: add(m.grip.position, turn(heading(m.grip), m.offset)), yaw: heading(m.grip) + m.yawOffset }
  else
    match below(e.item.position) with
    | Option.None => turned
    | Option.Some(surface) =>
      let rest = surface + drop(e.half, turned.pitch) in
      let p = approach(turned.position, { turned.position with y: rest }, ease) in
      { turned with position: { p with y: Math.max(p.y, rest) } }

// `below` is `surfaceBelow` in play; tests pass their own floor.
let tickWith = (views, m: t, dt: float, below): t =>
  let ease = Math.min(1.0, dt * 10.0) in
  items(views, m)
    |> List.indexedMap((i, e) =>
      let turned =
        if i == m.active then
          { e.item with
            yaw: e.item.yaw + axis(m.stick.x) * turnRate * dt,
            pitch: Math.clamp(0.0 - Math.pi / 2.0, Math.pi / 2.0, e.item.pitch + axis(m.stick.y) * turnRate * dt) }
        else e.item in
      { i: i, next: stepped(m, below, ease, i, e, turned) })
    |> List.fold((acc, s) => withItem(acc, s.i, s.next), m)

let tick = (views, m: t, dt: float): t => tickWith(views, m, dt, surfaceBelow)

let at = (p: Input.point3, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

let posed = (it: item, scene) =>
  scene |> Scene.rotateX(Angle.radians(it.pitch)) |> Scene.rotateY(Angle.radians(it.yaw)) |> at(it.position)

// Where the stand-in room is drawn (desktop, a paused feed): a floor and a
// 75 cm desk half a metre ahead, which also fuse into surfaces.
let standIn = () =>
  let block = (w, h, d, x, y, z, shade) =>
    Scene.cube() |> Scene.scaleXYZ(w, h, d) |> Scene.lit(Color.rgb(shade, shade, shade + 0.04)) |> at(point(x, y, z)) in
  Scene.group([
    block(6.0, 0.02, 6.0, 0.0, -0.01, 1.0, 0.42),
    block(1.0, 0.75, 0.6, 0.0, 0.375, 0.5, 0.55),
  ])

// A dot at the hand: green holding, cyan with something in reach.
let gripDot = (views, m: t) =>
  if m.gripTracked then
    Scene.sphere()
      |> Scene.scale(0.008)
      |> Scene.emissive(
        if m.held >= 0.0 then Color.rgb(0.3, 1.0, 0.45)
        else if inReach(views, m) >= 0.0 then Color.rgb(0.35, 0.9, 1.0)
        else Color.rgb(0.95, 0.98, 1.0))
      |> at(m.grip.position)
  else Scene.group([])

let draw = (views, m: t) =>
  let shown = viewAt(views, m.index) |> Option.map((view) =>
    if m.apart then List.zip(view.pieces, m.parts) |> List.map((pair) => let (it, piece) = pair in piece.scene |> posed(it))
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
      Scene.group(shown |> List.append([gripDot(views, m)])),
      [
        Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.95, 0.9), 0.9),
      ]) in
  // Real things hide the parts, but not while one is held: the hand holding
  // it would cut into it.
  if m.mixedReality && m.held >= 0.0 then frame |> Frame.withPassthrough |> Frame.withStandInRoom(standIn())
  else if m.mixedReality then
    frame |> Frame.withPassthrough |> Frame.withStandInRoom(standIn()) |> Frame.withRoomOcclusion
  else frame |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.06))
