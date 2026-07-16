// Locomotion blending + programmatic head-look — the `Scene.animate` /
// `Anim.blend` / `Anim.rotate` demo.
//
// Xbot's idle/walk/run clips are mixed by a single speed parameter
// (0 = idle, 0.5 = walk, 1 = run — a 1D blend space), and the head joint is
// aimed with an additive `Anim.rotate` on top of the blend. The playheads,
// weights, and head angles are derived here, in game code, from `tts` and
// the model — the engine owns no animation clock, so scrubbing time-travel
// replays the exact pose.
//
// Keys: 1 = idle, 2 = walk, 3 = run, 0 = auto-cycle (default). Speed eases
// toward the target, so clip transitions crossfade smoothly. Move the mouse
// to make the head follow the pointer (the auto mode sweeps it until then).

let init = {
  speed: 0.0,
  target: 0.0,
  auto: true,
  head: { yaw: 0.0, pitch: 0.0 },
  pointerDrivesHead: false,
}

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | Key.Num1 => { model with target: 0.0, auto: false }
    | Key.Num2 => { model with target: 0.5, auto: false }
    | Key.Num3 => { model with target: 1.0, auto: false }
    | Key.Num0 => { model with auto: true, pointerDrivesHead: false }
    | _ => model

// Pointer position (window pixels) -> head aim: the pointer's offset from
// the window center becomes yaw/pitch targets, clamped to a natural range.
// (Assumes the default 800x600 window — there is no window-size query yet;
// at other sizes the mapping just saturates earlier.)
let mouseMove = (model, x, y) =>
  let yaw = (Math.clamp01(x / 800.0) - 0.5) * 1.6 in
  let pitch = (Math.clamp01(y / 600.0) - 0.5) * 0.9 in
  { model with head: { yaw: yaw, pitch: pitch }, pointerDrivesHead: true }

let tick = (model, dt, tts) =>
  // Auto mode sweeps the target through idle -> walk -> run and back.
  let target =
    (match model.auto with
     | true => (1.0 - Math.cos(tts * 0.6)) * 0.5
     | false => model.target) in
  let rate = Math.clamp01(dt * 4.0) in
  // Until the pointer takes over, sweep the head so the look-at is visible.
  let head =
    (match model.pointerDrivesHead with
     | true => model.head
     | false => { yaw: Math.sin(tts * 0.9) * 0.6, pitch: Math.sin(tts * 1.7) * 0.25 }) in
  { model with speed: model.speed + (target - model.speed) * rate, head: head }

let absF = (x: float): float =>
  match x < 0.0 with
  | true => 0.0 - x
  | false => x

// The 1D blend space: each clip's weight peaks at its point on the speed
// axis (idle at 0, walk at 0.5, run at 1) and fades linearly to its
// neighbors. Anim.blend normalizes, so adjacent weights crossfade.
let idleWeight = (s: float): float => Math.clamp01(1.0 - s * 2.0)
let walkWeight = (s: float): float => Math.clamp01(1.0 - absF(s - 0.5) * 2.0)
let runWeight = (s: float): float => Math.clamp01(s * 2.0 - 1.0)

// Clip names come from the generated `assets.fun` (`functor import`) — a typo
// in a typed constant is a check-time error, not a silent bind pose.
let locomotion = (s: float, tts: float): Anim.t =>
  Anim.blend([
    (Anim.clip(Assets.xbot.idle.name, tts), idleWeight(s)),
    (Anim.clip(Assets.xbot.walk.name, tts), walkWeight(s)),
    (Anim.clip(Assets.xbot.run.name, tts), runWeight(s)),
  ])

// The full pose: the locomotion blend with the head aimed on top — an
// additive local rotation on the head joint (survives the blend beneath it).
let pose = (model, tts) =>
  locomotion(model.speed, tts)
    |> Anim.rotate("mixamorig:Head",
         Angle.radians(model.head.pitch),
         Angle.radians(model.head.yaw),
         Angle.radians(0.0))

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 1.4, -3.2, 0.0, 0.9, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(10.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      // Xbot stands ~1.8 units tall, Y-up, at authored scale; glTF forward
      // is +Z, so turn it to face the camera.
      Scene.model("Xbot.glb")
        |> Scene.animate(pose(model, tts))
        |> Scene.rotateY(Angle.degrees(180.0)),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(-0.5, -1.0, 0.4, Color.rgb(1.0, 0.96, 0.88), 1.0) |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Anim.blend: idle / walk / run by speed + Anim.rotate head-look"),
    Ui.text(Text.concat("speed: ", Text.fixed(model.speed, 2.0))),
    Ui.text(Text.concat("head yaw: ", Text.fixed(model.head.yaw, 2.0))),
    Ui.text("keys: 1 idle, 2 walk, 3 run, 0 auto — mouse aims the head"),
  ]) |> Ui.panel(Ui.topLeft())
