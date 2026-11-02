// PartMR — printable parts at true scale on your real desk, in a headset's
// passthrough, before you print them: 1 mm in a part is 1 mm in the room.
// Each view is a name and its pieces (`{ solid, color }`, mm, Z up;
// PartViews pieces work), e.g. an assembly and its exploded view.
//
// Quest: the right trigger grabs, the right thumbstick turns (left/right) and
// tilts (forward/back), A cycles the views, X toggles passthrough. Desktop
// (`run native --emulate-xr`): a stand-in room; mouse + left-click (or Space)
// grabs, the arrows are the thumbstick, 1 (or N) cycles, P toggles
// passthrough. Start seated, facing your desk: the desk sits ahead of where
// you faced at launch.
//
//   let views = PartMR.views([{ name: "Assembled", pieces: assembled }, …])  // meshed once
//   let desk = { PartMR.desk with height: 0.74 }                           // your desk, metres
//   let config = PartMR.config()
//   let settings = PartMR.settings
//   let init = PartMR.init(views, desk)
//   let sampledInput = (m, s) => PartMR.sampledInput(views, m, s)
//   let tick = (m, dt, tts) => PartMR.tick(views, desk, m, dt)
//   let draw = (m, tts) => PartMR.draw(views, desk, m)
//
// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// The desk the parts rest on, in metres in passthrough's floor-level stage
// space (y = 0 on the real floor): its top's height, its size, and how far
// ahead its centre is. Edit `height` and save: parts settle onto it.
type desk = { height: float, width: float, depth: float, z: float }
let desk: desk = { height: 0.75, width: 1.0, depth: 0.6, z: 0.5 }

// Xr.scene: the room's depth, so real things in front (a hand, the tablet a
// case is for) hide the parts behind them.
let config = () =>
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))

let settings = (m) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.passthroughEnabled(m.mixedReality))

// Level (pitch 0) so the room stays level on the headset. The eye height and
// field of view only frame the desktop view: the headset uses its own optics.
let camera = () =>
  Camera3D.firstPerson(Vec3.make(0.0, 0.9, 0.0), Angle.degrees(0.0), Angle.degrees(0.0), Angle.degrees(42.0))
    |> Camera3D.clip(0.02, 50.0)

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

// One decimal place, dropping a trailing ".0": 60, 1.8.
let mm = (v: float) => Text.fromFloat(Math.round(v * 10.0) / 10.0)

// A view ready to show: its pieces centred on the origin, its half extents,
// and a label of its name and size in mm.
let prepare = (view) =>
  let b = PartViews.bounds(view.pieces) in
  let center = Vec3.add(b.min, b.max) |> Vec3.scale(-0.5) in
  let size = Vec3.sub(b.min, b.max) in
  let font = Typeface.sans() in
  let text = $"{view.name} · {mm(Vec3.x(size))} × {mm(Vec3.y(size))} × {mm(Vec3.z(size))} mm" in
  let label =
    Sketch.text(font, 8.0, text)
      |> Sketch.translate(0.0 - Sketch.textWidth(font, 8.0, text) / 2.0, -4.0)
      |> Solid.extrude(0.8) in
  {
    pieces:
      Scene.group(view.pieces |> List.map((p) => Solid.toScene(p.solid |> Solid.translate(center)) |> Scene.lit(p.color)))
        // The part's front (-Y) faces you.
        |> Scene.rotateY(Angle.degrees(180.0)),
    // Half extents in scene metres: x across, y up (the part's Z), z deep.
    half: point(Vec3.x(size) / 2000.0, Vec3.z(size) / 2000.0, Vec3.y(size) / 2000.0),
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

// Resting on the desk: the view's lowest point, tilted by `pitch`, touches it.
let restY = (desk: desk, view, pitch: float) =>
  desk.height + view.half.y * Math.abs(Math.cos(pitch)) + view.half.z * Math.abs(Math.sin(pitch))

let fallbackAim = () =>
  Camera3D.mapTrackedPose(camera(), { position: { x: 0.2, y: -0.3, z: -0.3 }, orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 } })

// `aim`: the right controller's pointing pose; `index`: the view shown;
// `position`, `yaw`, `pitch`: where it is and how it's turned; the
// `…Pressed` fields latch buttons so a hold acts once.
type t = {
  aim: Camera3D.mappedPose, aimTracked: bool, grabbing: bool, stick: Input.point2,
  index: float, position: Input.point3, yaw: float, pitch: float,
  mixedReality: bool, togglePressed: bool, cyclePressed: bool
}

let init = (views, desk: desk): t =>
  let rest = viewAt(views, 0.0) |> Option.map((v) => restY(desk, v, 0.0)) |> Option.defaultValue(desk.height) in
  { aim: fallbackAim(), aimTracked: false, grabbing: false, stick: { x: 0.0, y: 0.0 },
    index: 0.0, position: point(0.0, rest, desk.z + 0.05), yaw: 0.0, pitch: 0.0,
    mixedReality: true, togglePressed: false, cyclePressed: false }

let toggled = (m: t): t => { m with mixedReality: not m.mixedReality }
// The next view takes the last one's place, turned and tilted the same, so
// an assembly and its exploded view compare side by side. With no views
// there is nothing to cycle.
let cycled = (views, m: t): t =>
  if List.length(views) == 0.0 then m else { m with index: Math.mod(m.index + 1.0, List.length(views)) }

// Keyboard edges, so a held key's auto-repeat doesn't act again.
let pressed = (s: Input.snapshot, key) => s.pressedKeys |> List.any((k) => k == key)

let sampledInput = (views, current: t, s: Input.snapshot): t =>
  let byKey = if pressed(s, Key.P) then toggled(current) else current in
  let m = if pressed(s, Key.N) then cycled(views, byKey) else byKey in
  match s.xr with
  | Option.None => { m with aimTracked: false, grabbing: false, stick: { x: 0.0, y: 0.0 } }
  | Option.Some(xr) =>
    let aimTracked = xr.right.active && Option.isSome(xr.right.aim) in
    let toggle = xr.left.primaryPressed in
    let cycle = xr.right.primaryPressed in
    let next = { m with
      aim: xr.right.aim |> Option.map((pose) => Camera3D.mapTrackedPose(camera(), pose)) |> Option.defaultValue(fallbackAim()),
      aimTracked: aimTracked,
      grabbing: aimTracked && xr.right.trigger > 0.5,
      stick: xr.right.thumbstick,
      togglePressed: toggle,
      cyclePressed: cycle } in
    let next = if toggle && not m.togglePressed then toggled(next) else next in
    if cycle && not m.cyclePressed then cycled(views, next) else next

let ahead = (pose: Camera3D.mappedPose, distance: float): Input.point3 =>
  point(pose.position.x + pose.forward.x * distance, pose.position.y + pose.forward.y * distance, pose.position.z + pose.forward.z * distance)

let approach = (from: Input.point3, target: Input.point3, amount: float): Input.point3 =>
  point(from.x + (target.x - from.x) * amount, from.y + (target.y - from.y) * amount, from.z + (target.z - from.z) * amount)

// How far ahead of the controller a view is held, in metres; the thumbstick
// dead zone; the turn rate in radians per second at full tilt.
let reach = 0.12
let deadZone = 0.2
let turnRate = 2.0
let axis = (v: float) => if Math.abs(v) < deadZone then 0.0 else v

let tick = (views, desk: desk, m: t, dt: float): t =>
  let yaw = m.yaw + axis(m.stick.x) * turnRate * dt in
  let pitch = Math.clamp(0.0 - Math.pi / 2.0, Math.pi / 2.0, m.pitch + axis(m.stick.y) * turnRate * dt) in
  let p = m.position in
  let ease = Math.min(1.0, dt * 10.0) in
  if m.grabbing then { m with yaw: yaw, pitch: pitch, position: approach(p, ahead(m.aim, reach), ease) }
  else
    // Released: settle onto the desk within its edges, never sinking into it.
    let rest = viewAt(views, m.index) |> Option.map((v) => restY(desk, v, pitch)) |> Option.defaultValue(desk.height) in
    let x = Math.clamp(0.0 - desk.width / 2.0, desk.width / 2.0, p.x) in
    let z = Math.clamp(desk.z - desk.depth / 2.0, desk.z + desk.depth / 2.0, p.z) in
    let q = approach(p, point(x, rest, z), ease) in
    { m with yaw: yaw, pitch: pitch, position: { q with y: Math.max(q.y, rest) } }

let at = (p: Input.point3, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

// A see-through desktop, its top at `desk.height`. In passthrough it should
// sit on your real desk; if it floats or sinks, change the height.
let deskScene = (desk: desk) =>
  Scene.cube()
    |> Scene.scaleXYZ(desk.width, 0.01, desk.depth)
    |> Scene.lit(Color.rgb(0.6, 0.62, 0.68))
    |> Scene.opacity(0.35)
    |> at(point(0.0, desk.height - 0.005, desk.z))

let aimDot = (m: t) =>
  if m.aimTracked then
    Scene.sphere()
      |> Scene.scale(0.006)
      |> Scene.emissive(if m.grabbing then Color.rgb(0.3, 1.0, 0.45) else Color.rgb(0.95, 0.98, 1.0))
      |> at(ahead(m.aim, reach))
  else Scene.group([])

let draw = (views, desk: desk, m: t) =>
  let p = m.position in
  let shown = viewAt(views, m.index) |> Option.map((view) =>
    // The label props up on the desk just in front of the view.
    let front = Math.sqrt(view.half.x * view.half.x + Math.max(view.half.y, view.half.z) * Math.max(view.half.y, view.half.z)) in
    [
      view.pieces |> Scene.rotateX(Angle.radians(m.pitch)) |> Scene.rotateY(Angle.radians(m.yaw)) |> at(p),
      view.label |> at(point(p.x, desk.height, p.z - front - 0.03)),
    ]) |> Option.defaultValue([]) in
  let frame =
    Frame.createLit(
      camera(),
      Scene.group([deskScene(desk)] |> List.append(shown) |> List.append([aimDot(m)])),
      [
        Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.95, 0.9), 0.9),
      ]) in
  if m.mixedReality then frame |> Frame.withPassthrough |> Frame.withRoomOcclusion
  else frame |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.06))
