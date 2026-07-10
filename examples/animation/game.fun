// Locomotion blending — the `Scene.animate` / `Anim.blend` demo.
//
// Xbot's idle/walk/run clips are mixed by a single speed parameter
// (0 = idle, 0.5 = walk, 1 = run — a 1D blend space). The playheads and
// weights are derived here, in game code, from `tts` and the model — the
// engine owns no animation clock, so scrubbing time-travel replays the
// exact pose.
//
// Keys: 1 = idle, 2 = walk, 3 = run, 0 = auto-cycle (default). Speed eases
// toward the target, so clip transitions crossfade smoothly.

let init = { speed: 0.0, target: 0.0, auto: true }

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | "1" => { model with target: 0.0, auto: false }
    | "2" => { model with target: 0.5, auto: false }
    | "3" => { model with target: 1.0, auto: false }
    | "0" => { model with auto: true }
    | _ => model

let tick = (model, dt, tts) =>
  // Auto mode sweeps the target through idle -> walk -> run and back.
  let target =
    (match model.auto with
     | true => (1.0 - Math.cos(tts * 0.6)) * 0.5
     | false => model.target) in
  let rate = Math.clamp01(dt * 4.0) in
  { model with speed: model.speed + (target - model.speed) * rate }

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

let locomotion = (s: float, tts: float): Anim.t =>
  Anim.blend([
    (Anim.clip("idle", tts), idleWeight(s)),
    (Anim.clip("walk", tts), walkWeight(s)),
    (Anim.clip("run", tts), runWeight(s)),
  ])

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 1.4, -3.2, 0.0, 0.9, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(10.0) |> Scene.lit(0.42, 0.47, 0.55),
      // Xbot's skinned pose comes out in raw Mixamo rig space (cm, lying
      // along -Z): the loader drops the Armature node's transform (a known
      // skinning bug — see docs/todo.md). Compensate: stand it up, face the
      // camera, meter scale.
      Scene.model("Xbot.glb")
        |> Scene.animate(locomotion(model.speed, tts))
        |> Scene.rotateX(Angle.degrees(90.0))
        |> Scene.rotateY(Angle.degrees(180.0))
        |> Scene.scale(0.01),
    ]),
    [
      Light.ambient(0.25, 0.25, 0.3),
      Light.directional(-0.5, -1.0, 0.4, 1.0, 0.96, 0.88, 1.0) |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Anim.blend: idle / walk / run by speed"),
    Ui.text(Text.concat("speed: ", Text.fixed(model.speed, 2.0))),
    Ui.text("keys: 1 idle, 2 walk, 3 run, 0 auto"),
  ]) |> Ui.panel(Ui.topLeft())
