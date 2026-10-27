// Scene-grid kata: a world-aligned grid on the real room's surfaces, from two
// sources of the same depth, side by side.
//
// Quest:   functor -d examples/scene-grid run vr
//          X (left primary) cycles raw -> fused -> off; the headset's depth
//          sensing finds the surfaces (grant the spatial data permission once
//          — see docs/vr.md)
// Desktop: functor -d examples/scene-grid run native
//          a stand-in room (floor, desk, wall) stands in for the real one; G
//          cycles (`--input-script toggle.script` does it headlessly)
//
// Raw: a fullscreen pass over this frame's depth. It lands on whatever is
// there now — floor, desk, walls, you — and wobbles with the depth noise.
// Fused: the same lines on a mesh the runtime fuses from the depth over time.
// Steadier, it keeps what is out of view and forgets moved furniture within a
// second or two. Off: plain passthrough.

// Launch options: the camera feed and the room's depth for this run.
let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))

// Level (pitch 0) so the room stays level on the headset; the field of view
// only frames the desktop view.
let camera =
  Camera3D.firstPerson(
    Vec3.make(0.0, 1.6, -0.6),
    Angle.degrees(0.0),
    Angle.degrees(0.0),
    Angle.degrees(90.0))

type Source =
  | Raw
  | Fused
  | Off

let init = { source: Raw, togglePressed: false }

let next = (source) =>
  match source with
  | Raw => Fused
  | Fused => Off
  | Off => Raw

let toggled = (model) => { model with source: next(model.source) }

expect toggled(init).source == Fused
expect toggled(toggled(init)).source == Off
expect toggled(toggled(toggled(init))).source == Raw

// X cycles on its press edge, so holding it does not skip ahead.
let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => model
  | Option.Some(xr) =>
    let pressed = xr.left.primaryPressed in
    let next = { model with togglePressed: pressed } in
    if pressed && not model.togglePressed then toggled(next) else next

let input = (model, key, isDown) =>
  match key with
  | Key.G => if isDown then toggled(model) else model
  | _ => model

let tick = (model, dt, tts) => model

// The stand-in room, in metres with the floor at y = 0: what desktop and web
// show (and shade) where the headset would see the real room.
let block = (w, h, d, x, y, z, shade) =>
  Scene.cube()
    |> Scene.scaleXYZ(w, h, d)
    |> Scene.translate(Vec3.make(x, y, z))
    |> Scene.lit(Color.rgb(shade, shade, shade * 1.04))

let room =
  Scene.group([
    block(6.0, 0.02, 6.0, 0.0, -0.01, 1.5, 0.42),   // floor
    block(1.2, 0.75, 0.6, 0.3, 0.375, 1.0, 0.55),   // desk, 75 cm tall
    block(6.0, 2.6, 0.05, 0.0, 1.3, 2.4, 0.62),     // wall
  ])

// Grid: 25 cm cells, thin cyan lines.
let gridCell = 0.25
let gridColor = Color.rgb(0.35, 0.9, 1.0)

let draw = (model, tts) =>
  let frame =
    Frame.createLit(
      camera,
      Scene.group([]),
      [
        Light.ambient(Color.rgb(0.35, 0.35, 0.4)),
        Light.directional(Vec3.make(-0.4, -1.0, 0.6), Color.rgb(1.0, 0.95, 0.9), 0.8),
      ])
      |> Frame.withPassthrough
      |> Frame.withStandInRoom(room)
  in
  match model.source with
  | Raw => frame |> Frame.withSurfaceGrid(gridCell, gridColor)
  | Fused => frame |> Frame.withFusedRoom(gridCell, gridColor)
  | Off => frame
