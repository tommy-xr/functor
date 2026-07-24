// Controller-driven XR example. The same sampled-input code runs against real
// Quest Touch controllers or the desktop `--emulate-xr` adapter.
//
// Desktop:
//   functor -d examples/xr-controllers run native --emulate-xr
//   Mouse moves the right controller; left-click/Space grabs the orb.
// Deterministic capture:
//   functor -d examples/xr-controllers run native --emulate-xr \
//     --input-script grab.script --capture-frame /tmp/xr-grab.png \
//     --capture-at-frame 60
// Quest:
//   functor -d examples/xr-controllers run vr

let camera =
  Camera.lookAt(
    Vec3.make(0.0, 1.6, -3.0),
    Vec3.make(0.0, 1.2, 0.0))

let fallbackPose = (x) =>
  Camera.mapTrackedPose(camera, {
    position: { x: x, y: -0.12, z: -0.55 },
    orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
  })

let fallbackLeft = fallbackPose(-0.24)
let fallbackRight = fallbackPose(0.24)

let point = (x, y, z): Input.point3 => { x: x, y: y, z: z }

let init = {
  leftGrip: fallbackLeft,
  leftAim: fallbackLeft,
  leftGripTracked: false,
  leftAimTracked: false,
  rightGrip: fallbackRight,
  rightAim: fallbackRight,
  rightGripTracked: false,
  rightAimTracked: false,
  orb: point(0.0, 1.15, -0.7),
  grabbing: false,
  squeeze: 0.0
}

let mappedPose = (fallback, trackedPose) =>
  match trackedPose with
  | Option.None => fallback
  | Option.Some(pose) => Camera.mapTrackedPose(camera, pose)

let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None =>
    {
      model with
      leftGripTracked: false,
      leftAimTracked: false,
      rightGripTracked: false,
      rightAimTracked: false,
      grabbing: false,
      squeeze: 0.0
    }
  | Option.Some(xr) =>
    let leftGripTracked = xr.left.active && Option.isSome(xr.left.grip) in
    let leftAimTracked = xr.left.active && Option.isSome(xr.left.aim) in
    let rightGripTracked = xr.right.active && Option.isSome(xr.right.grip) in
    let rightAimTracked = xr.right.active && Option.isSome(xr.right.aim) in
    let leftGrip = mappedPose(fallbackLeft, xr.left.grip) in
    let rightGrip = mappedPose(fallbackRight, xr.right.grip) in
    {
      model with
      leftGrip: leftGrip,
      leftAim: mappedPose(leftGrip, xr.left.aim),
      leftGripTracked: leftGripTracked,
      leftAimTracked: leftAimTracked,
      rightGrip: rightGrip,
      rightAim: mappedPose(rightGrip, xr.right.aim),
      rightGripTracked: rightGripTracked,
      rightAimTracked: rightAimTracked,
      grabbing: rightAimTracked && xr.right.trigger > 0.5,
      squeeze: if rightAimTracked then xr.right.squeeze else 0.0
    }

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

let tick = (model, dt, tts) =>
  if model.grabbing then
    let amount = Math.min(1.0, dt * 8.0) in
    { model with orb: approach(model.orb, ahead(model.rightAim, 0.3), amount) }
  else model

let at = (point, scene) =>
  scene |> Scene.translate(Vec3.make(point.x, point.y, point.z))

let visible = (shown, scene) =>
  if shown then scene else Scene.group([])

let controller = (gripTracked, aimTracked, grip, aim, color) =>
  Scene.group([
    visible(
      gripTracked,
      Scene.cube()
        |> Scene.scaleXYZ(0.035, 0.05, 0.09)
        |> Scene.emissive(color)
        |> at(grip.position)),
    visible(
      aimTracked,
      Scene.sphere()
        |> Scene.scale(0.025)
        |> Scene.emissive(Color.rgb(0.95, 0.98, 1.0))
        |> at(ahead(aim, 0.3))),
  ])

let orb = (model) =>
  Scene.sphere()
    |> Scene.scale(0.09 + model.squeeze * 0.04)
    |> Scene.emissive(
      if model.grabbing
      then Color.rgb(1.0, 0.2, 0.72)
      else Color.rgb(0.1, 0.85, 1.0))
    |> at(model.orb)

let draw = (model, tts) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane()
        |> Scene.scale(8.0)
        |> Scene.lit(Color.rgb(0.08, 0.09, 0.16)),
      Scene.cylinder()
        |> Scene.scaleXYZ(0.45, 0.03, 0.45)
        |> Scene.emissive(Color.rgb(0.15, 0.18, 0.32))
        |> Scene.translate(Vec3.make(0.0, 0.02, -0.7)),
      orb(model),
      controller(
        model.leftGripTracked,
        model.leftAimTracked,
        model.leftGrip,
        model.leftAim,
        Color.rgb(0.45, 0.35, 1.0)),
      controller(
        model.rightGripTracked,
        model.rightAimTracked,
        model.rightGrip,
        model.rightAim,
        if model.grabbing
        then Color.rgb(1.0, 0.2, 0.72)
        else Color.rgb(0.1, 0.85, 1.0)),
    ]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.2)),
      Light.directional(
        Vec3.make(-0.4, -1.0, 0.3),
        Color.rgb(0.8, 0.9, 1.0),
        0.8),
    ])
    |> Frame.withClearColor(Color.rgb(0.015, 0.02, 0.06))
