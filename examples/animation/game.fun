// Locomotion blending + stacked head-look and two-bone arm reach.
//
// Xbot's idle/walk/run clips are mixed by a single speed parameter, then pure
// animation post-passes aim the head and solve both direct arm chains after the
// animated spine has settled. The cyan and pink model-space targets can be
// dragged with the visible pointer; Camera3D.toWorldRay supplies the pick plane.
// Every target, playhead, and weight is plain frame data, so time travel and
// replay reproduce the exact pose.
//
// Keys: 1 = idle, 2 = walk, 3 = run, 0 = auto-cycle (default).

type Drag =
  | NotDragging
  | DragLeft
  | DragRight

type Target = { x: float, y: float, z: float }

let target3 = (x: float, y: float, z: float): Target => { x: x, y: y, z: z }

let camera =
  Camera3D.lookAt(Vec3.make(0.0, 1.4, -3.2), Vec3.make(0.0, 0.9, 0.0))

let init = {
  speed: 0.0,
  target: 0.0,
  auto: true,
  leftTarget: target3(0.52, 1.24, 0.28),
  rightTarget: target3(-0.52, 1.24, 0.28),
  manualLeft: false,
  manualRight: false,
  dragging: NotDragging,
  pointerX: 0.0,
  pointerY: 0.0,
  pressX: 0.0,
  pressY: 0.0,
  pressPending: false,
}

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | Key.Num1 => { model with target: 0.0, auto: false }
    | Key.Num2 => { model with target: 0.5, auto: false }
    | Key.Num3 => { model with target: 1.0, auto: false }
    | Key.Num0 => { model with auto: true }
    | _ => model

// Mouse-button callbacks do not carry coordinates, so retain the preceding
// event-time mouse position. sampledInput can then distinguish a quick
// press/move/release whose final snapshot position is no longer over the
// sphere that was actually clicked.
let mouseMove = (model, x, y) =>
  { model with pointerX: x, pointerY: y }

let mouseButton = (model, button, isDown) =>
  if button == Mouse.Left && isDown then
    { model with
        pressX: model.pointerX,
        pressY: model.pointerY,
        pressPending: true }
  else model

// Both targets live on one vertical interaction plane in front of Xbot.
// Undo the model's 180-degree scene rotation after intersecting the world ray:
// the animation evaluator intentionally knows nothing about Scene transforms.
let pointerTarget = (mouse: Input.mouse) =>
  match Camera3D.toWorldRay(mouse, camera) with
  | Option.None => Option.None
  | Option.Some(ray) =>
    let dz = Vec3.z(ray.direction) in
    match dz > 0.0001 with
    | false => Option.None
    | true =>
      let worldPlaneZ = 0.0 - 0.28 in
      let distance = (worldPlaneZ - Vec3.z(ray.origin)) / dz in
      let worldTarget =
        ray.origin |> Vec3.add(ray.direction |> Vec3.scale(distance)) in
      Option.Some(target3(
        0.0 - Vec3.x(worldTarget),
        Vec3.y(worldTarget),
        0.0 - Vec3.z(worldTarget)))

let distance2D = (a: Target, b: Target) =>
  let dx = a.x - b.x in
  let dy = a.y - b.y in
  Math.sqrt(dx * dx + dy * dy)

let pickRadius = 0.16

let pickTarget = (pointer: Target, left: Target, right: Target) =>
  let leftDistance = distance2D(pointer, left) in
  let rightDistance = distance2D(pointer, right) in
  if leftDistance <= pickRadius && leftDistance <= rightDistance then DragLeft
  else if rightDistance <= pickRadius then DragRight
  else NotDragging

// Press edges choose a target, the held level drags it, and release ends the
// drag. The raw hook above preserves the event-time press coordinate; the
// sampled snapshot remains the deterministic source of held/released state.
let sampledInput = (model, snapshot: Input.snapshot) =>
  let pressed = model.pressPending || snapshot.mouse.pressed.left in
  let pressMouse =
    if model.pressPending then
      { snapshot.mouse with x: model.pressX, y: model.pressY }
    else snapshot.mouse in
  let selected =
    if pressed then
      match pointerTarget(pressMouse) with
      | Option.Some(pointer) =>
        pickTarget(pointer, model.leftTarget, model.rightTarget)
      | Option.None => NotDragging
    else model.dragging in
  let moved =
    match pointerTarget(snapshot.mouse) with
    | Option.None => model
    | Option.Some(pointer) =>
      match selected with
      | DragLeft =>
        { model with leftTarget: pointer, manualLeft: true }
      | DragRight =>
        { model with rightTarget: pointer, manualRight: true }
      | NotDragging => model in
  { moved with
      dragging:
        if snapshot.mouse.buttons.left then selected else NotDragging,
      pressPending: false }

let tick = (model, dt, tts) =>
  // Auto mode sweeps the locomotion blend through idle -> walk -> run and back.
  let target =
    if model.auto then (1.0 - Math.cos(tts * 0.6)) * 0.5
    else model.target in
  let rate = Math.clamp01(dt * 4.0) in
  // Each hand target idles independently until the user drags it.
  let leftTarget =
    if model.manualLeft then model.leftTarget
    else target3(
      0.52 + Math.sin(tts * 0.8) * 0.08,
      1.24 + Math.sin(tts * 1.1) * 0.12,
      0.28) in
  let rightTarget =
    if model.manualRight then model.rightTarget
    else target3(
      -0.52 + Math.sin(tts * 0.9 + 2.0) * 0.08,
      1.24 + Math.sin(tts * 1.3 + 1.0) * 0.12,
      0.28) in
  { model with
      speed: model.speed + (target - model.speed) * rate,
      leftTarget: leftTarget,
      rightTarget: rightTarget }

let absF = (x: float): float =>
  if x < 0.0 then 0.0 - x else x

// The 1D blend space: each clip's weight peaks at its point on the speed axis
// and fades linearly to its neighbors. Anim.blend normalizes the weights.
let idleWeight = (s: float): float => Math.clamp01(1.0 - s * 2.0)
let walkWeight = (s: float): float => Math.clamp01(1.0 - absF(s - 0.5) * 2.0)
let runWeight = (s: float): float => Math.clamp01(s * 2.0 - 1.0)

let locomotion = (s: float, tts: float): Anim.t =>
  Anim.blend([
    (Anim.clip(Assets.xbotClips.idle.name, tts), idleWeight(s)),
    (Anim.clip(Assets.xbotClips.walk.name, tts), walkWeight(s)),
    (Anim.clip(Assets.xbotClips.run.name, tts), runWeight(s)),
  ])

let focusTarget = (model, tts) =>
  match model.dragging with
  | DragLeft => model.leftTarget
  | DragRight => model.rightTarget
  | NotDragging =>
    if Math.sin(tts * 0.7) >= 0.0 then model.leftTarget else model.rightTarget

// The whole stack: locomotion settles the spine, lookAt follows one sphere,
// then each sibling arm chain reaches its own target. The evaluated locomotion
// pose supplies each elbow's bend side.
let pose = (model, tts) =>
  let focus = focusTarget(model, tts) in
  locomotion(model.speed, tts)
    |> Anim.lookAt(
         Assets.xbotJoints.mixamorig_Head,
         Vec3.make(focus.x, focus.y, focus.z),
         Angle.degrees(65.0),
         0.8)
    |> Anim.reach(
         Assets.xbotJoints.mixamorig_LeftArm,
         Assets.xbotJoints.mixamorig_LeftForeArm,
         Assets.xbotJoints.mixamorig_LeftHand,
         Vec3.make(model.leftTarget.x, model.leftTarget.y, model.leftTarget.z),
         1.0)
    |> Anim.reach(
         Assets.xbotJoints.mixamorig_RightArm,
         Assets.xbotJoints.mixamorig_RightForeArm,
         Assets.xbotJoints.mixamorig_RightHand,
         Vec3.make(model.rightTarget.x, model.rightTarget.y, model.rightTarget.z),
         1.0)

let modelToWorld = (target) =>
  Vec3.make(0.0 - target.x, target.y, 0.0 - target.z)

let targetMarker = (target, color, selected) =>
  Scene.sphere()
    |> Scene.scale(if selected then 0.09 else 0.075)
    |> Scene.emissive(color)
    |> Scene.translate(modelToWorld(target))

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
      targetMarker(
        model.leftTarget,
        Color.rgb(0.1, 0.95, 1.0),
        model.dragging == DragLeft),
      targetMarker(
        model.rightTarget,
        Color.rgb(1.0, 0.2, 0.65),
        model.dragging == DragRight),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0)
        |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Anim.lookAt + Anim.reach over the locomotion blend"),
    Ui.text(Text.concat("speed: ", Text.fixed(model.speed, 2.0))),
    Ui.text("drag the cyan / pink spheres — each hand follows its target"),
    Ui.text("keys: 1 idle, 2 walk, 3 run, 0 auto"),
  ]) |> Ui.panel(Ui.topLeft())

expect pickTarget(
  target3(0.50, 1.24, 0.28),
  target3(0.52, 1.24, 0.28),
  target3(-0.52, 1.24, 0.28)) == DragLeft

expect pickTarget(
  target3(-0.50, 1.24, 0.28),
  target3(0.52, 1.24, 0.28),
  target3(-0.52, 1.24, 0.28)) == DragRight

expect pickTarget(
  target3(0.0, 0.4, 0.28),
  target3(0.52, 1.24, 0.28),
  target3(-0.52, 1.24, 0.28)) == NotDragging
