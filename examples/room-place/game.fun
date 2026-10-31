// Room-place kata: put markers flush on the real room.
//
// Quest:   functor -d examples/room-place run vr
//          a ring sits where the right controller's ray meets the room; the
//          trigger plants a marker there, standing straight out of the surface
// Desktop: functor -d examples/room-place run native
//          a stand-in room (floor, desk, three walls) stands in for the real
//          one; the ring follows the mouse, and a click or Space plants
//          (`--input-script place.script --capture-size 1600x1200` replays a
//          few). `--emulate-xr` aims with the emulated controller instead.
//
// `Physics.fusedRoom` makes the fused room something a ray can hit, and
// `Physics.cast` gives the point and the surface's normal.

let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))

// Level, so the room stays level on the headset.
let eye = Vec3.make(0.0, 1.6, -1.2)
let camera = Camera3D.firstPerson(eye, 0deg, 0deg, 90deg)

let reach = 10.0
let maxMarkers = 12.0

let roomTag = Physics.tag("room")

let init = { markers: [], nextId: 0.0, aim: Option.None, trigger: false }

// Where a ray meets the room: the point, and the surface's normal facing the
// ray, as plain records so `/state` shows them. A miss is `None`.
let spotAlong = (ray: Camera3D.ray) =>
  let hit = Physics.cast(ray.origin, ray.direction, reach) in
  if hit.hit then
    Option.Some({ at: { x: hit.x, y: hit.y, z: hit.z }, normal: { x: hit.nx, y: hit.ny, z: hit.nz } })
  else Option.None

// A marker at `spot`. Past the cap, the oldest goes.
let plant = (model, spot) =>
  let markers = model.markers |> List.append([{ id: model.nextId, spot: spot }]) in
  { model with
      markers: List.drop(Math.max(0.0, List.length(markers) - maxMarkers), markers),
      nextId: model.nextId + 1.0 }

let floorSpot = { at: { x: 0.0, y: 0.0, z: 1.0 }, normal: { x: 0.0, y: 1.0, z: 0.0 } }
let planted = (n) => List.range(n) |> List.fold((m, i) => plant(m, floorSpot), init)

expect List.length(planted(3.0).markers) == 3.0
expect List.length(planted(15.0).markers) == maxMarkers
expect (List.head(planted(15.0).markers) |> Option.map((m) => m.id)) == Option.Some(3.0)

// A button acts on its press: held down, it does nothing more.
let pressed = (was, now) => now && not was

let point = (p: Input.point3) => Vec3.make(p.x, p.y, p.z)

// The aim: the right controller's ray on the headset (and under
// `--emulate-xr` or an injected `xr` sample), else the mouse's.
let aimRay = (snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.Some(xr) =>
    xr.right.aim |> Option.map((aim) =>
      let pose = Camera3D.mapTrackedPose(camera, aim) in
      { origin: point(pose.position), direction: point(pose.forward) })
  | Option.None => Camera3D.toWorldRay(snapshot.mouse, camera)

let triggerHeld = (snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.Some(xr) => xr.right.trigger > 0.5
  | Option.None => false

// The trigger's press plants; without a controller, a click or Space.
let planting = (model, snapshot: Input.snapshot) =>
  pressed(model.trigger, triggerHeld(snapshot))
    || (Option.isNone(snapshot.xr)
        && (snapshot.mouse.pressed.left || (snapshot.pressedKeys |> List.any((k) => k == Key.Space))))

let sampledInput = (model, snapshot: Input.snapshot) =>
  let aim = aimRay(snapshot) |> Option.bind(spotAlong) in
  let next = { model with aim: aim, trigger: triggerHeld(snapshot) } in
  match aim with
  | Option.Some(spot) => if planting(model, snapshot) then plant(next, spot) else next
  | Option.None => next

let tick = (model, dt, tts) => model

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), [Physics.fusedRoom(roomTag)])

// Turn a node built standing on +Y to stand on the unit normal `n`: tilt
// about X by the angle from +Y, then turn about Y toward `n`.
let standOn = (n, node) =>
  node
    |> Scene.rotateX(Angle.radians(Math.acos(Math.clamp(-1.0, 1.0, n.y))))
    |> Scene.rotateY(Angle.radians(Math.atan2(n.x, n.z)))

// Built on the spot's surface: its +Y is the normal, its origin the point.
let onSpot = (spot, node) => node |> standOn(spot.normal) |> Scene.translate(point(spot.at))

let colors = [
  Color.rgb(1.0, 0.25, 0.45),
  Color.rgb(1.0, 0.8, 0.15),
  Color.rgb(0.2, 0.95, 0.5),
  Color.rgb(0.8, 0.35, 1.0),
]

// A pin: a 14 cm post with a 5 cm head, its foot on the surface.
let markerScene = (marker) =>
  let color =
    List.nth(Math.mod(marker.id, List.length(colors)), colors)
      |> Option.defaultValue(Color.rgb(1.0, 1.0, 1.0)) in
  Scene.group([
    Scene.cylinder()
      |> Scene.scaleXYZ(0.015, 0.14, 0.015)
      |> Scene.translate(Vec3.make(0.0, 0.07, 0.0))
      |> Scene.lit(Color.rgb(0.85, 0.85, 0.9)),
    Scene.sphere() |> Scene.scale(0.025) |> Scene.translate(Vec3.make(0.0, 0.14, 0.0)) |> Scene.lit(color),
  ])
    |> onSpot(marker.spot)

// The reticle: a flat 16 cm ring, 2.5 cm off the surface. Over the desktop
// stand-in room the drawn fused room hides anything within 2 cm of it.
let reticle = (spot) =>
  Scene.group([
    Scene.cylinder() |> Scene.scaleXYZ(0.16, 0.003, 0.16) |> Scene.emissive(Color.rgb(0.3, 1.0, 1.0)),
    Scene.cylinder() |> Scene.scaleXYZ(0.11, 0.004, 0.11) |> Scene.emissive(Color.rgb(0.05, 0.2, 0.25)),
  ])
    |> Scene.translate(Vec3.make(0.0, 0.025, 0.0))
    |> onSpot(spot)

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
  let aimed = model.aim |> Option.map(reticle) |> Option.toList in
  Frame.createLit(
    camera,
    Scene.group(model.markers |> List.map(markerScene) |> List.append(aimed)),
    [
      Light.ambient(Color.rgb(0.35, 0.35, 0.4)),
      Light.directional(Vec3.make(-0.4, -1.0, 0.6), Color.rgb(1.0, 0.95, 0.9), 0.8),
    ])
    |> Frame.withPassthrough
    |> Frame.withStandInRoom(room)
    |> Frame.withFusedRoom(0.25, Color.rgb(0.3, 0.55, 0.65))
