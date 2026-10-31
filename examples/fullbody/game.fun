// examples/fullbody — a full upper body driven by the headset and two
// controllers, plus an inspection twin: a panel showing the same pose from
// the front.
//
//   functor -d examples/fullbody run native               # the scripted fixture
//   functor -d examples/fullbody run native --emulate-xr  # the emulated rig
//   functor -d examples/fullbody run vr
//
// `Body.step` keeps the history (calibration, the hips' lag, tracking-loss
// blends); `Body.pose` is a pure function of that state, drawn twice. Stand
// straight and press A and X together (C on desktop) to recalibrate your
// height. P toggles `solved`: the solved head and wrists in world space, read
// back through `Probe` for `GET /state`.

let config =
  Config.create()
    |> Config.xr(Xr.create() |> Xr.refreshRate(120.0) |> Xr.floorLocked(true))

let settings = (m) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))

/// The solved joints, in the world: what you'd check the body against.
type Solved = { head: Probe.Frame, left: Probe.Frame, right: Probe.Frame, leftElbow: Probe.Frame, rightElbow: Probe.Frame }

type Model = {
  body: Body.State,
  xr: Option.t<Input.xr>,
  calibrate: bool,
  probing: bool,
  solved: Option.t<Solved>
}

let init: Model = { body: Body.init, xr: Option.None, calibrate: false, probing: false, solved: Option.None }

let sampledInput = (m: Model, s: Input.snapshot): Model =>
  let pressed = (k: Key.t) => s.pressedKeys |> List.any((p) => p == k) in
  { m with
      xr: s.xr,
      calibrate: pressed(Key.C),
      probing: if pressed(Key.P) then not m.probing else m.probing }

let worldFrame = (b: Body.State, f: Probe.Frame): Probe.Frame =>
  let turn = (v: Input.point3) => Quat.rotate(Quat.yaw(b.yaw), v) in
  { position: Body.toWorldPoint(b, f.position), x: turn(f.x), y: turn(f.y), z: turn(f.z) }

let solve = (b: Body.State): Solved =>
  let posed = Probe.pose(Body.pose(b)) in
  let at = (name: string) => worldFrame(b, Probe.frame(posed, name)) in
  {
    head: at(Rig.j.mixamorig_Head),
    left: at(Rig.j.mixamorig_LeftHand),
    right: at(Rig.j.mixamorig_RightHand),
    leftElbow: at(Rig.j.mixamorig_LeftForeArm),
    rightElbow: at(Rig.j.mixamorig_RightForeArm)
  }

let tick = (m: Model, dt: float, tts: float): Model =>
  // Without a headset, play the fixture (by game time, so `--fixed-time` pins it).
  let xr = match m.xr with | Option.Some(x) => x | Option.None => Demo.xr(tts) in
  let body = Body.step(m.body, Body.fromXr(m.calibrate, xr), dt) in
  { m with body: body, solved: if m.probing then Option.Some(solve(body)) else Option.None }

// --- drawing ---

let fov = Angle.degrees(90.0)

// The rig: level at the feet, as `Xr.floorLocked` wants. A headset draws the
// eye at this camera plus the tracked head.
let rig = Camera3D.firstPerson(Vec3.make(0.0, 0.0, 0.0), 0rad, 0rad, fov) |> Camera3D.clip(0.05, 100.0)

// With no headset to compose, draw from the fixture's head.
let headCamera = (b: Body.State) =>
  match b.head with
  | Option.None => rig
  | Option.Some(h) =>
    Camera3D.fromPose({
      position: h.position,
      forward: Quat.rotate(h.rotation, Quat.v3(0.0, 0.0, 1.0)),
      up: Quat.rotate(h.rotation, Quat.v3(0.0, 1.0, 0.0))
    }, fov) |> Camera3D.clip(0.05, 100.0)

let avatar = (b: Body.State): Scene.t =>
  Scene.model(Rig.model) |> Scene.animate(Body.pose(b)) |> Body.place(b)

let room =
  Scene.group([
    Scene.plane() |> Scene.scale(12.0) |> Scene.lit(Color.rgb(0.16, 0.2, 0.22)),
    Scene.cylinder() |> Scene.scaleXYZ(0.9, 0.01, 0.9) |> Scene.lit(Color.rgb(0.25, 0.42, 0.44)),
  ])

let lights = [
  Light.ambient(Color.rgb(0.32, 0.34, 0.38)),
  Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.97, 0.9), 0.9) |> Light.castShadows,
]

// The inspection twin: the same avatar, filmed from in front of its hips,
// with small markers on the wrist targets.
let twin = RenderTarget.named("twin") |> RenderTarget.sized(512.0, 512.0)

let marker = (b: Body.State, h: Body.Hand, color) =>
  match h.wrist with
  | Option.None => Scene.group([])
  | Option.Some(w) =>
    Scene.sphere()
      |> Scene.scale(0.035 * b.scale)
      |> Scene.emissive(color)
      |> Scene.translate(Quat.toVec3(Body.toWorldPoint(b, w.position)))

let twinFrame = (b: Body.State) =>
  let at = (x: float, y: float, z: float) => Quat.toVec3(Body.toWorldPoint(b, Quat.v3(x, y, z))) in
  Frame.createLit(
    Camera3D.lookAt(at(0.0, 1.0, 2.6), at(0.0, 0.95, 0.0)),
    Scene.group([
      room,
      avatar(b),
      marker(b, b.left, Color.rgb(0.1, 0.95, 1.0)),
      marker(b, b.right, Color.rgb(1.0, 0.25, 0.7)),
    ]),
    lights)
    |> Frame.withClearColor(Color.rgb(0.05, 0.06, 0.09))

// The panel faces the player's starting spot. A quad's front is +Z, so it is
// turned to face back down -Z.
let panel =
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(1.5, 1.5, 0.04) |> Scene.lit(Color.rgb(0.07, 0.08, 0.1)),
    Scene.quad()
      |> Scene.screen(twin)
      |> Scene.rotateY(180deg)
      |> Scene.scale(1.4)
      |> Scene.translate(Vec3.make(0.0, 0.0, -0.025)),
  ])
    |> Scene.translate(Vec3.make(0.0, 1.5, 1.9))

let draw = (m: Model, tts: float) =>
  let camera = match m.xr with | Option.Some(_) => rig | Option.None => headCamera(m.body) in
  Frame.createLit(camera, Scene.group([room, avatar(m.body), panel]), lights)
    |> Frame.withClearColor(Color.rgb(0.05, 0.06, 0.09))
    |> Frame.withRenderTarget(twin, twinFrame(m.body))

let ui = (m: Model) =>
  Ui.column([
    Ui.text($"scale {Text.fixed(m.body.scale, 2.0)}  twist {Text.fixed(m.body.twist * 180.0 / Math.pi, 0.0)} deg"),
    Ui.text("C: recalibrate  P: solved joints in /state"),
  ]) |> Ui.panel(Ui.topLeft())
