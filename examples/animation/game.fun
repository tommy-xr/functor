// Locomotion blending + model-space head look — the `Scene.animate` /
// `Anim.blend` / `Anim.lookAt` demo.
//
// Xbot's idle/walk/run clips are mixed by a single speed parameter
// (0 = idle, 0.5 = walk, 1 = run — a 1D blend space), and the head joint is
// aimed at a model-space point after the blend has animated its whole parent
// chain. The playheads, weights, and target are derived here, in game code,
// from `tts` and the model — the engine owns no animation clock or hidden IK
// state, so scrubbing time-travel replays the exact pose.
//
// Keys: 1 = idle, 2 = walk, 3 = run, 0 = auto-cycle (default). Speed eases
// toward the target, so clip transitions crossfade smoothly. Move the mouse
// to make the head follow the pointer (the auto mode sweeps it until then).

let camera =
  Camera.lookAt(Vec3.make(0.0, 1.4, -3.2), Vec3.make(0.0, 0.9, 0.0))

let init = {
  speed: 0.0,
  target: 0.0,
  auto: true,
  headTarget: { x: 0.0, y: 1.55, z: 1.2 },
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

// Pick a point on a plane between the camera and Xbot, then undo the model's
// 180-degree scene rotation: Anim.lookAt targets live in model space because
// the animation evaluator intentionally knows nothing about Scene transforms.
let pointerTarget = (mouse: Input.mouse) =>
  match Camera.toWorldRay(mouse, camera) with
  | Option.None => Option.None
  | Option.Some(ray) =>
    let dz = Vec3.z(ray.direction) in
    match dz > 0.0001 with
    | false => Option.None
    | true =>
      let distance = ((0.0 - 1.2) - Vec3.z(ray.origin)) / dz in
      let worldTarget =
        ray.origin |> Vec3.add(ray.direction |> Vec3.scale(distance)) in
      Option.Some({
        x: 0.0 - Vec3.x(worldTarget),
        y: Vec3.y(worldTarget),
        z: 0.0 - Vec3.z(worldTarget),
      })

let sampledInput = (model, snapshot: Input.snapshot) =>
  match model.pointerDrivesHead with
  | false => model
  | true =>
    match pointerTarget(snapshot.mouse) with
    | Option.None => model
    | Option.Some(target) => { model with headTarget: target }

// The edge handler only switches control modes. sampledInput owns the actual
// ray conversion so the target is sampled deterministically at fixed steps.
let mouseMove = (model, x, y) =>
  { model with pointerDrivesHead: true }

let tick = (model, dt, tts) =>
  // Auto mode sweeps the target through idle -> walk -> run and back.
  let target =
    (match model.auto with
     | true => (1.0 - Math.cos(tts * 0.6)) * 0.5
     | false => model.target) in
  let rate = Math.clamp01(dt * 4.0) in
  // Until the pointer takes over, sweep a point in front of the character so
  // the engine-side look-at remains obvious in a hands-off preview.
  let headTarget =
    (match model.pointerDrivesHead with
     | true => model.headTarget
     | false => {
         x: Math.sin(tts * 0.9) * 0.9,
         y: 1.55 + Math.sin(tts * 1.7) * 0.22,
         z: 1.2,
       }) in
  { model with
      speed: model.speed + (target - model.speed) * rate,
      headTarget: headTarget }

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
// in a typed constant is a check-time error, not a silent bind pose. The
// model itself is the branded `Assets.xbot` (`Asset.model`), so its
// reference is typo-proof too.
let locomotion = (s: float, tts: float): Anim.t =>
  Anim.blend([
    (Anim.clip(Assets.xbotClips.idle.name, tts), idleWeight(s)),
    (Anim.clip(Assets.xbotClips.walk.name, tts), walkWeight(s)),
    (Anim.clip(Assets.xbotClips.run.name, tts), runWeight(s)),
  ])

// The full pose: solve the head after locomotion has animated its spine.
// local +Z is Xbot's authored facing axis; the explicit limit keeps targets
// behind the character from producing a full turn.
let pose = (model, tts) =>
  locomotion(model.speed, tts)
    |> Anim.lookAt(
         Assets.xbotJoints.mixamorig_Head,
         Vec3.make(model.headTarget.x, model.headTarget.y, model.headTarget.z),
         Angle.degrees(75.0),
         1.0)

let draw = (model, tts) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(10.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      // Xbot stands ~1.8 units tall, Y-up, at authored scale; glTF forward
      // is +Z, so turn it to face the camera.
      Scene.model(Assets.xbot)
        |> Scene.animate(pose(model, tts))
        |> Scene.rotateY(Angle.degrees(180.0)),
      // Show the target in world space. This applies the same 180-degree
      // transform as the model so the marker and IK target stay coincident.
      Scene.sphere()
        |> Scene.scale(0.025)
        |> Scene.emissive(Color.rgb(0.1, 0.95, 1.0))
        |> Scene.translate(Vec3.make(
             0.0 - model.headTarget.x,
             model.headTarget.y,
             0.0 - model.headTarget.z)),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0) |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Anim.blend: idle / walk / run + Anim.lookAt post-pass"),
    Ui.text(Text.concat("speed: ", Text.fixed(model.speed, 2.0))),
    Ui.text(Text.concat("target x: ", Text.fixed(model.headTarget.x, 2.0))),
    Ui.text("keys: 1 idle, 2 walk, 3 run, 0 auto — mouse aims the head"),
  ]) |> Ui.panel(Ui.topLeft())
