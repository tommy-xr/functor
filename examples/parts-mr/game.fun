// Mixed-reality parts viewer: a printable part (parts.fun) at true scale on
// your real desk, before you print it. 1 mm in the part is 1 mm in the room.
//
// Quest:   functor -d examples/parts-mr run vr
//          right trigger grabs, right thumbstick turns (left/right) and
//          tilts (forward/back), A cycles parts, X toggles passthrough
// Desktop: functor -d examples/parts-mr run native --emulate-xr
//          a stand-in room; mouse + left-click (or Space) grabs, arrows are
//          the thumbstick, 1 (or N) cycles parts, P toggles passthrough
//
// In passthrough 1 unit = 1 metre in the headset's floor-level stage space,
// with y = 0 on the real floor.

// Your desk's height above the floor, in metres. Edit and save: the part
// settles onto the new height without restarting.
let deskHeight = 0.75

let config = Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true))

let settings = (model) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.passthroughEnabled(model.mixedReality))

// Level (pitch 0) so the room stays level on the headset. The eye height and
// field of view only frame the desktop view: the headset uses its own optics
// and puts the real floor at y = 0 whatever the eye height says.
let camera =
  Camera3D.firstPerson(
    Vec3.make(0.0, 0.9, 0.0),
    Angle.degrees(0.0),
    Angle.degrees(0.0),
    Angle.degrees(42.0))
    |> Camera3D.clip(0.02, 50.0)

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

// One decimal place, dropping a trailing ".0": 60, 1.8.
let mm = (v: float) => Text.fromFloat(Math.round(v * 10.0) / 10.0)

// A part ready to show: its pieces centered on the origin, its half extents,
// and a label of its size in mm.
let prepare = (part: Parts.part) =>
  let all = Solid.group(part.pieces |> List.map((p) => p.solid)) in
  let b = Solid.bounds(all) in
  let center =
    Vec3.make(
      0.0 - (Vec3.x(b.min) + Vec3.x(b.max)) / 2.0,
      0.0 - (Vec3.y(b.min) + Vec3.y(b.max)) / 2.0,
      0.0 - (Vec3.z(b.min) + Vec3.z(b.max)) / 2.0) in
  let size = Vec3.sub(b.min, b.max) in
  let font = Typeface.sans() in
  let text = $"{mm(Vec3.x(size))} × {mm(Vec3.y(size))} × {mm(Vec3.z(size))} mm" in
  let label =
    Sketch.text(font, 8.0, text)
      |> Sketch.translate(0.0 - Sketch.textWidth(font, 8.0, text) / 2.0, -4.0)
      |> Solid.extrude(0.8) in
  {
    pieces:
      Scene.group(
        part.pieces
          |> List.map((p) => Solid.toScene(p.solid |> Solid.translate(center)) |> Scene.lit(p.color)))
        // The part's +Y faces away, so text on it reads from the viewer's side.
        |> Scene.rotateY(Angle.degrees(180.0)),
    // Half extents in scene metres: x across, y up (the part's Z), z deep.
    half: point(Vec3.x(size) / 2000.0, Vec3.z(size) / 2000.0, Vec3.y(size) / 2000.0),
    // Turned to read from the viewer's side, then propped up 45°.
    label:
      Solid.toScene(label)
        |> Scene.emissive(Color.rgb(0.35, 0.9, 1.0))
        |> Scene.rotateY(Angle.degrees(180.0))
        |> Scene.rotateX(Angle.degrees(-45.0))
        |> Scene.translate(Vec3.make(0.0, 0.003, 0.0))
  }

// The parts A cycles through. Built once here, so each is meshed once.
let tagView = prepare(Parts.tag)
let views = [tagView, prepare(Parts.bookmark), prepare(Parts.calibrationCube)]
let count = List.length(views)

let viewAt = (i) =>
  match List.nth(i, views) with
  | Option.Some(v) => v
  | Option.None => tagView

// Resting on the desk: the part's lowest point, tilted by `pitch`, touches it.
let restY = (view, pitch) =>
  deskHeight + view.half.y * Math.abs(Math.cos(pitch)) + view.half.z * Math.abs(Math.sin(pitch))

let fallbackAim =
  Camera3D.mapTrackedPose(camera, {
    position: { x: 0.2, y: -0.3, z: -0.3 },
    orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
  })

let init = {
  aim: fallbackAim,
  aimTracked: false,
  grabbing: false,
  stick: { x: 0.0, y: 0.0 },
  index: 0.0,
  position: point(0.0, restY(viewAt(0.0), 0.0), 0.55),
  yaw: 0.0,
  pitch: 0.0,
  mixedReality: true,
  togglePressed: false,
  cyclePressed: false
}

let mappedPose = (fallback, trackedPose) =>
  match trackedPose with
  | Option.None => fallback
  | Option.Some(pose) => Camera3D.mapTrackedPose(camera, pose)

let toggled = (model) => { model with mixedReality: not model.mixedReality }
// The next part starts level and facing you, where the last one was.
let cycled = (model) => { model with index: Math.mod(model.index + 1.0, count), yaw: 0.0, pitch: 0.0 }

let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => { model with aimTracked: false, grabbing: false, stick: { x: 0.0, y: 0.0 } }
  | Option.Some(xr) =>
    let aimTracked = xr.right.active && Option.isSome(xr.right.aim) in
    let toggle = xr.left.primaryPressed in
    let cycle = xr.right.primaryPressed in
    let next = {
      model with
      aim: mappedPose(fallbackAim, xr.right.aim),
      aimTracked: aimTracked,
      grabbing: aimTracked && xr.right.trigger > 0.5,
      stick: xr.right.thumbstick,
      togglePressed: toggle,
      cyclePressed: cycle
    } in
    let next = if toggle && not model.togglePressed then toggled(next) else next in
    if cycle && not model.cyclePressed then cycled(next) else next

let input = (model, key, isDown) =>
  match key with
  | Key.P => if isDown then toggled(model) else model
  | Key.N => if isDown then cycled(model) else model
  | _ => model

let ahead = (pose, distance): Input.point3 => {
  x: pose.position.x + pose.forward.x * distance,
  y: pose.position.y + pose.forward.y * distance,
  z: pose.position.z + pose.forward.z * distance
}

let approach = (from, target, amount): Input.point3 => {
  x: from.x + (target.x - from.x) * amount,
  y: from.y + (target.y - from.y) * amount,
  z: from.z + (target.z - from.z) * amount
}

// How far ahead of the controller the part is held, in metres.
let reach = 0.12

// Thumbstick dead zone, and turn rate in radians per second at full tilt.
let deadZone = 0.2
let turnRate = 2.0
let axis = (v) => if Math.abs(v) < deadZone then 0.0 else v

let tick = (model, dt, tts) =>
  let yaw = model.yaw + axis(model.stick.x) * turnRate * dt in
  let pitch = Math.clamp(0.0 - Math.pi / 2.0, Math.pi / 2.0, model.pitch + axis(model.stick.y) * turnRate * dt) in
  let p = model.position in
  // Held: follow the controller. Released: settle onto the desk where it is.
  let target = if model.grabbing then ahead(model.aim, reach) else point(p.x, restY(viewAt(model.index), pitch), p.z) in
  { model with yaw: yaw, pitch: pitch, position: approach(p, target, Math.min(1.0, dt * 10.0)) }

let at = (p, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

// A see-through desktop: 1 m × 0.6 m, its top at `deskHeight`. In passthrough
// it should sit on your real desk; if it floats or sinks, edit `deskHeight`.
let desk =
  Scene.cube()
    |> Scene.scaleXYZ(1.0, 0.01, 0.6)
    |> Scene.lit(Color.rgb(0.6, 0.62, 0.68))
    |> Scene.opacity(0.35)
    |> at(point(0.0, deskHeight - 0.005, 0.5))

let aimDot = (model) =>
  if model.aimTracked then
    Scene.sphere()
      |> Scene.scale(0.006)
      |> Scene.emissive(
        if model.grabbing then Color.rgb(0.3, 1.0, 0.45) else Color.rgb(0.95, 0.98, 1.0))
      |> at(ahead(model.aim, reach))
  else Scene.group([])

let draw = (model, tts) =>
  let view = viewAt(model.index) in
  let p = model.position in
  // The label props up on the desk just in front of the part.
  let front = Math.sqrt(view.half.x * view.half.x + Math.max(view.half.y, view.half.z) * Math.max(view.half.y, view.half.z)) in
  let frame =
    Frame.createLit(
      camera,
      Scene.group([
        desk,
        view.pieces
          |> Scene.rotateX(Angle.radians(model.pitch))
          |> Scene.rotateY(Angle.radians(model.yaw))
          |> at(p),
        view.label |> at(point(p.x, deskHeight, p.z - front - 0.03)),
        aimDot(model),
      ]),
      [
        Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.95, 0.9), 0.9),
      ])
  in
  if model.mixedReality then frame |> Frame.withPassthrough
  else frame |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.06))

expect mm(60.0) == "60"
expect mm(1.8000001) == "1.8"
expect Math.abs(restY(tagView, 0.0) - (deskHeight + 0.002)) < 0.000001
// Stood on its long edge, the tag rests on half its 30 mm depth.
expect Math.abs(restY(tagView, Math.pi / 2.0) - (deskHeight + 0.015)) < 0.000001
expect cycled({ init with index: count - 1.0 }).index == 0.0
