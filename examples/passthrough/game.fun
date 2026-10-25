// Mixed-reality kata: lit primitives floating at arm's length over your real
// room, a floor grid that fades out, and a part you can grab.
//
// Quest:   functor -d examples/passthrough run vr
//          right trigger grabs the part; X (left primary) toggles passthrough
// Desktop: functor -d examples/passthrough run native --emulate-xr
//          the room is a flat grey placeholder; mouse + left-click grab, P toggles
//
// In passthrough 1 unit = 1 metre in the headset's floor-level stage space:
// y = 0 is the real floor and the rig stays level, so `part` below can be any
// mesh authored in metres and it appears at true scale.

// Launch option: prepare the headset's camera feed for this run.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true))

// Live option: run or pause the feed with the model.
let settings = (model) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.passthroughEnabled(model.mixedReality))

// Level (pitch 0) so the room stays level on the headset; the wide field of
// view only frames the desktop view (the headset uses its own optics).
let camera =
  Camera3D.firstPerson(
    Vec3.make(0.0, 1.6, 0.0),
    Angle.degrees(0.0),
    Angle.degrees(0.0),
    Angle.degrees(90.0))

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

let fallbackAim =
  Camera3D.mapTrackedPose(camera, {
    position: { x: 0.2, y: -0.3, z: -0.3 },
    orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
  })

let init = {
  aim: fallbackAim,
  aimTracked: false,
  grabbing: false,
  part: point(0.0, 1.32, 0.5),
  mixedReality: true,
  togglePressed: false
}

let mappedPose = (fallback, trackedPose) =>
  match trackedPose with
  | Option.None => fallback
  | Option.Some(pose) => Camera3D.mapTrackedPose(camera, pose)

let toggled = (model) => { model with mixedReality: not model.mixedReality }

let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => { model with aimTracked: false, grabbing: false }
  | Option.Some(xr) =>
    let aimTracked = xr.right.active && Option.isSome(xr.right.aim) in
    let pressed = xr.left.primaryPressed in
    let next = {
      model with
      aim: mappedPose(fallbackAim, xr.right.aim),
      aimTracked: aimTracked,
      grabbing: aimTracked && xr.right.trigger > 0.5,
      togglePressed: pressed
    } in
    if pressed && not model.togglePressed then toggled(next) else next

let input = (model, key, isDown) =>
  match key with
  | Key.P => if isDown then toggled(model) else model
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

let tick = (model, dt, tts) =>
  if model.grabbing then
    { model with part: approach(model.part, ahead(model.aim, reach), Math.min(1.0, dt * 10.0)) }
  else model

let at = (p, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

// The grabbable part: any mesh in metres works here (10 x 5 x 12 cm).
let part = Scene.cube() |> Scene.scaleXYZ(0.1, 0.05, 0.12)

// Floor grid: 25 cm cells out to 3 m, in rings that fade toward transparent
// so the virtual floor dissolves into the real one.
let cell = 0.25
let rings = 6.0
let radius = 3.0
let coords = List.range(25.0) |> List.map((i) => (i - 12.0) * cell)
let segment = (x, z, yaw) => { x: x, z: z, yaw: yaw }
let segments =
  coords |> List.concatMap((a) =>
    coords |> List.concatMap((b) =>
      [segment(a, b + cell * 0.5, 0.0), segment(a + cell * 0.5, b, 90.0)]))
let ringOf = (s) => Math.floor(Math.sqrt(s.x * s.x + s.z * s.z) / (radius / rings))
let ring = (k) =>
  Scene.cube()
    |> Scene.emissive(Color.rgb(0.35, 0.9, 1.0))
    |> Scene.instanced(
      segments
        |> List.filter((s) => ringOf(s) == k)
        |> List.map((s) =>
          Instance.trs(Vec3.make(s.x, 0.002, s.z), Angle.degrees(s.yaw), 0.008, 0.003, cell)))
    |> Scene.opacity(0.85 * (1.0 - k / rings))
let grid = Scene.group(List.range(rings) |> List.map(ring))

let floating = (tts) =>
  Scene.group([
    Scene.sphere()
      |> Scene.scale(0.12)
      |> Scene.lit(Color.rgb(1.0, 0.3, 0.55))
      |> at(point(-0.36, 1.48 + 0.02 * Math.sin(tts * 1.3), 0.7)),
    Scene.cube()
      |> Scene.scale(0.1)
      |> Scene.rotateY(Angle.radians(tts * 0.8))
      |> Scene.rotateX(Angle.degrees(25.0))
      |> Scene.lit(Color.rgb(0.95, 0.85, 0.3))
      |> at(point(0.0, 1.56, 0.75)),
    Scene.cylinder()
      |> Scene.scaleXYZ(0.1, 0.12, 0.1)
      |> Scene.lit(Color.rgb(0.3, 0.65, 1.0))
      |> at(point(0.36, 1.48 + 0.02 * Math.cos(tts * 1.1), 0.7)),
  ])

let aimDot = (model) =>
  if model.aimTracked then
    Scene.sphere()
      |> Scene.scale(0.015)
      |> Scene.emissive(Color.rgb(0.95, 0.98, 1.0))
      |> at(ahead(model.aim, reach))
  else Scene.group([])

let draw = (model, tts) =>
  let frame =
    Frame.createLit(
      camera,
      Scene.group([
        grid,
        floating(tts),
        part
          |> Scene.lit(
            if model.grabbing then Color.rgb(1.0, 0.25, 0.7) else Color.rgb(1.0, 0.55, 0.15))
          |> at(model.part),
        aimDot(model),
      ]),
      [
        Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.95, 0.9), 0.9),
      ])
  in
  if model.mixedReality then frame |> Frame.withPassthrough
  else frame |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.06))
