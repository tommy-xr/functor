// examples/held-pose — an asymmetric marker held in the right hand, placed
// with `Scene.atPose`: the grip's position and full orientation, roll included.
//
//   functor -d examples/held-pose run native               # a fixed demo pose
//   functor -d examples/held-pose run native --emulate-xr  # the emulated hand
//   functor -d examples/held-pose run vr
//   npm run test:held-pose                                 # injected rolled grips
//
// The marker is authored facing +Z (the long cyan arm) and upright in +Y
// (green), with a short pink arm on +X: the hand's left. Placed with a
// mirrored basis, pink would sit on the right.

// The authored rig: the head and hands move within it.
let rig =
  Camera3D.firstPerson(
    Vec3.make(0.0, 1.5, 0.0), Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(60.0))

// Without a tracked right hand: ahead and a little low, turned 30 degrees to
// the left, tipped up 20, and rolled 40 to the left (a tracking-space pose).
let demoGrip =
  Camera3D.mapTrackedPose(rig, {
    position: { x: 0.0, y: -0.1, z: -0.45 },
    orientation: { x: 0.24479232, y: 0.18214797, z: 0.28311405, w: 0.90925534 }
  })

let init = { grip: demoGrip, tracked: false }

let rightGrip = (s: Input.snapshot) =>
  match s.xr with
  | Option.None => Option.None
  | Option.Some(xr) => if xr.right.active then xr.right.grip else Option.None

let sampledInput = (m, s: Input.snapshot) =>
  match rightGrip(s) with
  | Option.None => { grip: demoGrip, tracked: false }
  | Option.Some(pose) => { grip: Camera3D.mapTrackedPose(rig, pose), tracked: true }

let tick = (m, dt, tts) => m

// One box of the marker: size (x, y, z), centered at `at` in marker space.
let bar = (color, x, y, z, at) =>
  Scene.cube()
    |> Scene.scaleXYZ(x, y, z)
    |> Scene.translate(at)
    |> Scene.color(color)

let marker =
  Scene.group([
    bar(Color.rgb(0.95, 0.95, 1.0), 0.03, 0.03, 0.03, Vec3.make(0.0, 0.0, 0.0)),
    bar(Color.rgb(0.1, 0.85, 1.0), 0.015, 0.015, 0.12, Vec3.make(0.0, 0.0, 0.075)),
    bar(Color.rgb(0.3, 1.0, 0.4), 0.015, 0.07, 0.015, Vec3.make(0.0, 0.05, 0.0)),
    bar(Color.rgb(1.0, 0.2, 0.72), 0.04, 0.015, 0.015, Vec3.make(0.035, 0.0, 0.0)),
  ])

let draw = (m, tts) =>
  Frame.create(
    rig |> Camera3D.clip(0.05, 50.0),
    Scene.group([
      Scene.plane() |> Scene.scale(6.0) |> Scene.color(Color.rgb(0.08, 0.09, 0.16)),
      marker |> Scene.atPose(m.grip),
    ]))
    |> Frame.withClearColor(Color.rgb(0.015, 0.02, 0.06))
