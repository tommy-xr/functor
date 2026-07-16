// A first-person Functor starter. Move with WASD or the arrow keys and look
// with the mouse. Saving this file while the game runs hot-reloads it.

let moveSpeed = 4.0
let mouseSensitivity = 0.003
let pitchLimit = 1.5

type Mouse =
  | WaitingForMouse
  | MouseAt(x: float, y: float)

let init = {
  held: { forward: false, back: false, left: false, right: false },
  eye: { x: 0.0, y: 1.7, z: -6.0 },
  yaw: 0.0,
  pitch: 0.0,
  mouse: WaitingForMouse,
}

let setHeld = (held, key, isDown) =>
  match key with
  | Key.W => { held with forward: isDown }
  | Key.Up => { held with forward: isDown }
  | Key.S => { held with back: isDown }
  | Key.Down => { held with back: isDown }
  | Key.A => { held with left: isDown }
  | Key.Left => { held with left: isDown }
  | Key.D => { held with right: isDown }
  | Key.Right => { held with right: isDown }
  | _ => held

let input = (model, key, isDown) =>
  { model with held: setHeld(model.held, key, isDown) }

let clamp = (value, low, high) =>
  match value < low with
  | true => low
  | false => (match high < value with | true => high | false => value)

let mouseMove = (model, x, y) =>
  match model.mouse with
  | WaitingForMouse => { model with mouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * mouseSensitivity,
        pitch: clamp(model.pitch - (y - lastY) * mouseSensitivity, 0.0 - pitchLimit, pitchLimit),
        mouse: MouseAt(x, y) }

let axis = (negative, positive) =>
  (match positive with | true => 1.0 | false => 0.0)
    - (match negative with | true => 1.0 | false => 0.0)

let tick = (model, dt, tts) =>
  let forward = axis(model.held.back, model.held.forward) in
  let right = axis(model.held.left, model.held.right) in
  let distance = moveSpeed * dt in
  { model with eye: {
      x: model.eye.x + distance * (forward * Math.sin(model.yaw) - right * Math.cos(model.yaw)),
      y: model.eye.y,
      z: model.eye.z + distance * (forward * Math.cos(model.yaw) + right * Math.sin(model.yaw)) } }

let target = (x, z, r, g, b) =>
  Scene.cube()
    |> Scene.translate(x, 0.75, z)
    |> Scene.lit(r, g, b)

let draw = (model, tts) =>
  Frame.createLit(
    Camera.firstPerson(
      model.eye.x, model.eye.y, model.eye.z,
      Angle.radians(model.yaw), Angle.radians(model.pitch), Angle.degrees(70.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(40.0) |> Scene.lit(0.32, 0.34, 0.38),
      target(-2.5, 2.0, 1.0, 0.3, 0.25),
      target(0.0, 5.0, 0.25, 0.7, 1.0),
      target(3.0, 8.0, 0.75, 0.35, 1.0),
    ]),
    [
      Light.ambient(0.12, 0.12, 0.16),
      Light.directional(0.5, -1.0, 0.35, 1.0, 0.96, 0.9, 1.0)
        |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.text("WASD / arrows to move · mouse to look")
    |> Ui.panel(Ui.topLeft())
