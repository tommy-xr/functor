// Room-bounce kata: shoot bouncy balls at the real room.
//
// Quest:   functor -d examples/room-bounce run vr
//          right trigger shoots a ball along the controller's aim; X shows or
//          hides the fused room the balls bounce off
// Desktop: functor -d examples/room-bounce run native
//          a stand-in room (floor, desk, three walls) stands in for the real
//          one; Space shoots along the view, a click through the clicked
//          point, G shows or hides the fused room (`shoot.script` replays a
//          few shots)
//
// `Physics.fusedRoom` is the whole room as one static body: the runtime fuses
// the depth into a mesh as you look around, and each chunk collides.

let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))

// Level, so the room stays level on the headset.
let eye = Vec3.make(0.0, 1.6, -1.2)
let camera = Camera3D.firstPerson(eye, 0deg, 0deg, 90deg)

let radius = 0.05
let speed = 5.5
let maxBalls = 40.0

let roomTag = Physics.tag("room")
let ballTag = (ball) => Physics.tag(Text.concat("ball-", Text.fromFloat(ball.id)))

let init = { balls: [], nextId: 0.0, showRoom: true, trigger: false, xButton: false }

// A ball leaves `from` along `dir` (unit length). Past the cap, the oldest goes.
let shoot = (model, from, dir) =>
  let ball = {
    id: model.nextId,
    from: Vec3.add(from, Vec3.scale(0.1, dir)),
    velocity: Vec3.scale(speed, dir)
  } in
  let balls = model.balls |> List.append([ball]) in
  { model with
      balls: List.drop(Math.max(0.0, List.length(balls) - maxBalls), balls),
      nextId: model.nextId + 1.0 }

let ahead = Vec3.make(0.0, 0.0, 1.0)
let shot = (n) => List.range(n) |> List.fold((m, i) => shoot(m, eye, ahead), init)

expect List.length(shot(3.0).balls) == 3.0
expect List.length(shot(45.0).balls) == maxBalls
expect (List.head(shot(45.0).balls) |> Option.map((b) => b.id)) == Option.Some(5.0)
expect shot(45.0).nextId == 45.0

// A button acts on its press: held down, it does nothing more.
let pressed = (was, now) => now && not was

expect pressed(false, true)
expect not pressed(true, true)
expect not pressed(true, false)

let toggled = (model, press) =>
  if press then { model with showRoom: not model.showRoom } else model

let point = (p: Input.point3) => Vec3.make(p.x, p.y, p.z)

// Quest: the right trigger shoots along the controller's aim.
let fromController = (model, xr: Input.xr) =>
  let trigger = xr.right.trigger > 0.5 in
  let next =
    toggled({ model with trigger: trigger, xButton: xr.left.primaryPressed },
            pressed(model.xButton, xr.left.primaryPressed)) in
  match xr.right.aim with
  | Option.Some(aim) =>
    if pressed(model.trigger, trigger) then
      let pose = Camera3D.mapTrackedPose(camera, aim) in
      shoot(next, point(pose.position), point(pose.forward))
    else next
  | Option.None => next

// Desktop: Space shoots along the view, a click through the clicked point.
let fromDesk = (model, snapshot: Input.snapshot) =>
  let keys = snapshot.pressedKeys in
  let next = toggled(model, keys |> List.any((k) => k == Key.G)) in
  let next =
    if keys |> List.any((k) => k == Key.Space) then
      shoot(next, eye, ahead)
    else next in
  if snapshot.mouse.pressed.left then
    match Camera3D.toWorldRay(snapshot.mouse, camera) with
    | Option.Some(ray) => shoot(next, ray.origin, ray.direction)
    | Option.None => next
  else next

let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.Some(xr) => fromController(model, xr)
  | Option.None => fromDesk(model, snapshot)

let tick = (model, dt, tts) => model

let ballBody = (ball) =>
  Physics.dynamic(ballTag(ball), Physics.sphere(radius))
    |> Physics.at(ball.from)
    |> Physics.velocity(ball.velocity)
    |> Physics.restitution(0.9)
    |> Physics.friction(0.3)
    |> Physics.linearDamping(0.6)
    |> Physics.angularDamping(1.0)
    // The room's walls are surfaces: sweep the ball so it can't step through.
    |> Physics.continuous

let physics = (model) =>
  Physics.scene(
    Vec3.make(0.0, -9.81, 0.0),
    [Physics.fusedRoom(roomTag) |> Physics.restitution(0.7) |> Physics.friction(0.6)]
      |> List.append(model.balls |> List.map(ballBody)))

let colors = [
  Color.rgb(1.0, 0.25, 0.45),
  Color.rgb(1.0, 0.8, 0.15),
  Color.rgb(0.2, 0.95, 0.5),
  Color.rgb(0.25, 0.65, 1.0),
  Color.rgb(0.8, 0.35, 1.0),
]

let ballScene = (ball) =>
  let color =
    List.nth(Math.mod(ball.id, List.length(colors)), colors)
      |> Option.defaultValue(Color.rgb(1.0, 1.0, 1.0)) in
  Scene.sphere() |> Scene.scale(radius) |> Scene.lit(color) |> Physics.transformed(ballTag(ball))

// The stand-in room, in metres with the floor at y = 0: what desktop and web
// show, and fuse, where the headset would see the real room.
let block = (w, h, d, x, y, z, shade) =>
  Scene.cube()
    |> Scene.scaleXYZ(w, h, d)
    |> Scene.translate(Vec3.make(x, y, z))
    |> Scene.lit(Color.rgb(shade, shade, shade * 1.04))

let room =
  Scene.group([
    block(6.0, 0.02, 6.0, 0.0, -0.01, 1.5, 0.42),   // floor
    block(1.2, 0.75, 0.6, 0.3, 0.375, 1.0, 0.55),   // desk, 75 cm tall
    block(6.0, 2.6, 0.05, 0.0, 1.3, 2.4, 0.62),     // back wall
    block(0.05, 2.6, 3.0, -1.8, 1.3, 0.9, 0.58),    // side walls
    block(0.05, 2.6, 3.0, 1.8, 1.3, 0.9, 0.58),
  ])

let draw = (model, tts) =>
  let frame =
    Frame.createLit(
      camera,
      Scene.group(model.balls |> List.map(ballScene)),
      [
        Light.ambient(Color.rgb(0.35, 0.35, 0.4)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.6), Color.rgb(1.0, 0.95, 0.9), 0.8),
      ])
      |> Frame.withPassthrough
      |> Frame.withStandInRoom(room)
  in
  if model.showRoom then frame |> Frame.withFusedRoom(0.25, Color.rgb(0.3, 0.55, 0.65)) else frame
