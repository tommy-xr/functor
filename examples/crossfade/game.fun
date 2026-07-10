// Crossfade A/B — the `Animator` module demo.
//
// TWO Xbots receive the exact same discrete state changes (idle / agree /
// headShake / sneak_pose / run — gestures that are NOT on a blend axis):
//
//   - LEFT figure: hard cut — the raw clip at clip-local time. Transitions
//     pop (what a system without crossfade looks like).
//   - RIGHT figure: `Animator.pose` — the same state, derived as a
//     smoothstep crossfade. Transitions glide.
//
// The Animator is ~20 lines of pure Functor Lang in the sibling
// `animator.fun` (file = module): crossfade state is plain data in the
// model, the blend weights derive from `tts`, and time-travel replays a
// mid-fade frame exactly.
//
// Keys: 1-5 trigger states, 0 = auto-cycle (default, every 2.5s). Mashing
// keys mid-fade shows the truncation policy on the right figure.

let init = {
  anim: Animator.start("idle", 0.0),
  auto: true,
  timer: 2.5,
  index: 0.0,
  // input has no tts parameter; tick stores the latest so key handlers can
  // stamp state changes.
  tts: 0.0,
}

let clipOf = (i: float): string =>
  match i with
  | 0.0 => "idle"
  | 1.0 => "agree"
  | 2.0 => "headShake"
  | 3.0 => "sneak_pose"
  | _ => "run"

let trigger = (model, clip) =>
  { model with auto: false, anim: Animator.play(clip, model.tts, model.anim) }

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | "0" => { model with auto: true, timer: 2.5 }
    | "1" => trigger(model, "idle")
    | "2" => trigger(model, "agree")
    | "3" => trigger(model, "headShake")
    | "4" => trigger(model, "sneak_pose")
    | "5" => trigger(model, "run")
    | _ => model

let tick = (model, dt, tts) =>
  let m = { model with tts: tts } in
  match m.auto with
  | false => m
  | true =>
    let timer = m.timer - dt in
    (match timer < 0.0 with
     | true =>
       let index = (match m.index == 4.0 with | true => 0.0 | false => m.index + 1.0) in
       { m with
           // Reset to the full period (not `timer + 2.5`): a long frame
           // collapses to ONE transition instead of burst-firing a
           // transition per frame until the deficit clears (the Sub.every
           // missed-boundary rule, hand-rolled).
           timer: 2.5,
           index: index,
           anim: Animator.play(clipOf(index), tts, m.anim) }
     | false => { m with timer: timer })

// Xbot's skinned pose comes out in raw Mixamo rig space (a known skinning
// bug — see docs/todo.md): stand it up, face the camera, meter scale.
let figure = (anim: Anim.t, x: float): Scene.t =>
  Scene.model("Xbot.glb")
    |> Scene.animate(anim)
    |> Scene.rotateX(Angle.degrees(90.0))
    |> Scene.rotateY(Angle.degrees(180.0))
    |> Scene.scale(0.01)
    |> Scene.translate(x, 0.0, 0.0)

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 1.5, -4.4, 0.0, 0.9, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(12.0) |> Scene.lit(0.42, 0.47, 0.55),
      // Camera looks down +Z, so world +X is screen LEFT.
      figure(Anim.clip(model.anim.current, tts - model.anim.since), 1.2),
      figure(Animator.pose(model.anim, 0.5, tts), -1.2),
    ]),
    [
      Light.ambient(0.25, 0.25, 0.3),
      Light.directional(-0.5, -1.0, 0.4, 1.0, 0.96, 0.88, 1.0) |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Animator: same state changes, two players"),
    Ui.text(Text.concat("state: ", model.anim.current)),
    Ui.text("left: hard cut  |  right: 0.5s crossfade"),
    Ui.text("keys: 1 idle, 2 agree, 3 headShake, 4 sneak, 5 run, 0 auto"),
  ]) |> Ui.panel(Ui.topLeft())
