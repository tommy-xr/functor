// OrbitCamera — a camera circling a target: drag with the left button to orbit,
// scroll to zoom, and a slow turntable while left alone.
//
// The state is plain model data (angles, zoom, drag), so time travel and hot
// reload carry it like any field; the target and size are passed to `camera`
// each frame, so the view follows whatever it frames.
//
//   let mouseMove = (m, x, y) => { m with orbit: OrbitCamera.move(m.orbit, x, y) }
//   let draw = (m, tts) => Frame.create(OrbitCamera.camera(m.orbit, target, size), …)

// Seconds without input before the turntable resumes.
let settle = 2.0

// Looking down a little from the front-right, turning from the start.
let init = { yaw: 0.6, pitch: 0.45, zoom: 1.0, dragging: false, x: 0.0, y: 0.0, idle: settle }

// Pointer motion (the mouse hook's coordinates): orbits while dragging.
let move = (o, x: float, y: float) =>
  if o.dragging
  then
    { o with
      yaw: o.yaw - (x - o.x) * 0.01,
      pitch: Math.clamp(-1.4, 1.4, o.pitch + (y - o.y) * 0.01),
      x: x,
      y: y,
      idle: 0.0 }
  else { o with x: x, y: y }

// The orbit button went down or up.
let press = (o, isDown: bool) => { o with dragging: isDown, idle: 0.0 }

// One wheel notch moves 10% closer (positive) or farther.
let scroll = (o, delta: float) =>
  { o with zoom: Math.clamp(0.2, 8.0, o.zoom * Math.pow(1.1, delta)), idle: 0.0 }

// Advance by `dt` seconds: the turntable turns once idle.
let tick = (o, dt: float) =>
  let idle = o.idle + dt in
  let spinning = idle > settle && o.dragging == false in
  { o with idle: idle, yaw: if spinning then o.yaw + dt * 0.4 else o.yaw }

// A camera on the orbit around `target`, far enough back that something
// `size` across fills the view at zoom 1.
let camera = (o, target: Vec3.t, size: float): Camera3D.t =>
  let distance = size * 1.5 / o.zoom in
  let flat = distance * Math.cos(o.pitch) in
  let eye =
    Vec3.add(
      target,
      Vec3.make(flat * Math.sin(o.yaw), distance * Math.sin(o.pitch), flat * Math.cos(o.yaw))) in
  Camera3D.lookAt(eye, target) |> Camera3D.clip(distance / 100.0, distance * 100.0)
