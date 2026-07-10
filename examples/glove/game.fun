// Programmatic hand posing — the `Anim.rest` / `Anim.rotate` demo.
//
// The SteamVR glove model has a skeleton but NO authored clips: every pose
// here is built at runtime by rotating finger joints over the bind pose
// (`Anim.rest()`), shock2quest-style — each finger has a normalized curl in
// [0, 1] that maps to rotations on its three joints. All state is plain
// data in the model, so poses rewind exactly under time-travel.
//
// Keys: 1 open, 2 fist, 3 point, 4 thumbs-up, 5 pinch, 0 = auto wave
// (default). Curls ease toward their targets, so poses morph smoothly.
//
// Model: SteamVR Unity Plugin (© Valve Corporation, BSD-3-Clause — see
// LICENSE.steamvr in this directory).

let init = {
  auto: true,
  curls: { thumb: 0.0, index: 0.0, middle: 0.0, ring: 0.0, pinky: 0.0 },
  targets: { thumb: 0.0, index: 0.0, middle: 0.0, ring: 0.0, pinky: 0.0 },
}

let preset = (model, t, i, m, r, p) =>
  { model with auto: false, targets: { thumb: t, index: i, middle: m, ring: r, pinky: p } }

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | "1" => preset(model, 0.0, 0.0, 0.0, 0.0, 0.0)   // open
    | "2" => preset(model, 1.0, 1.0, 1.0, 1.0, 1.0)   // fist
    | "3" => preset(model, 0.9, 0.0, 1.0, 1.0, 1.0)   // point
    | "4" => preset(model, 0.0, 1.0, 1.0, 1.0, 1.0)   // thumbs-up
    | "5" => preset(model, 0.55, 0.6, 0.0, 0.0, 0.0)  // pinch
    | "0" => { model with auto: true }
    | _ => model

// Auto mode: a staggered wave rippling across the fingers.
let wave = (tts: float, phase: float): float =>
  (1.0 - Math.cos(tts * 2.0 + phase)) * 0.5

let ease = (current: float, target: float, rate: float): float =>
  current + (target - current) * rate

let tick = (model, dt, tts) =>
  let targets =
    (match model.auto with
     | true => {
         thumb: wave(tts, 0.0),
         index: wave(tts, 0.7),
         middle: wave(tts, 1.4),
         ring: wave(tts, 2.1),
         pinky: wave(tts, 2.8),
       }
     | false => model.targets) in
  let rate = Math.clamp01(dt * 8.0) in
  { model with
      targets: targets,
      curls: {
        thumb: ease(model.curls.thumb, targets.thumb, rate),
        index: ease(model.curls.index, targets.index, rate),
        middle: ease(model.curls.middle, targets.middle, rate),
        ring: ease(model.curls.ring, targets.ring, rate),
        pinky: ease(model.curls.pinky, targets.pinky, rate),
      } }

// Curl one finger: rotate its three joints (proximal, middle, distal) in
// their local frames by the normalized curl amount. The SteamVR rig curls
// about the local Z axis.
let curlJoint = (name: string, degrees: float, anim: Anim.t): Anim.t =>
  anim |> Anim.rotate(name, Angle.degrees(0.0), Angle.degrees(0.0), Angle.degrees(degrees))

let finger = (name: string, curl: float, anim: Anim.t): Anim.t =>
  anim
    |> curlJoint(Text.concat(Text.concat("finger_", name), "_0_r"), curl * 50.0)
    |> curlJoint(Text.concat(Text.concat("finger_", name), "_1_r"), curl * 60.0)
    |> curlJoint(Text.concat(Text.concat("finger_", name), "_2_r"), curl * 45.0)

// The thumb opposes rather than curls in-plane: smaller angles read better.
let thumb = (curl: float, anim: Anim.t): Anim.t =>
  anim
    |> curlJoint("finger_thumb_0_r", curl * 30.0)
    |> curlJoint("finger_thumb_1_r", curl * 40.0)
    |> curlJoint("finger_thumb_2_r", curl * 35.0)

let handPose = (c) =>
  Anim.rest()
    |> thumb(c.thumb)
    |> finger("index", c.index)
    |> finger("middle", c.middle)
    |> finger("ring", c.ring)
    |> finger("pinky", c.pinky)

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 0.25, -0.65, 0.0, 0.1, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(6.0) |> Scene.translate(0.0, -0.25, 0.0)
        |> Scene.lit(0.42, 0.47, 0.55),
      Scene.model("vr_glove_model.glb")
        |> Scene.animate(handPose(model.curls))
        |> Scene.rotateX(Angle.degrees(-60.0))
        |> Scene.rotateY(Angle.degrees(180.0))
        |> Scene.scale(0.02),
    ]),
    [
      Light.ambient(0.45, 0.45, 0.5),
      Light.directional(-0.4, -0.8, 0.6, 1.0, 0.96, 0.9, 1.1) |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("Anim.rest + Anim.rotate: programmatic finger curls (no clips)"),
    Ui.text(Text.concat("index curl: ", Text.fixed(model.curls.index, 2.0))),
    Ui.text("keys: 1 open, 2 fist, 3 point, 4 thumbs-up, 5 pinch, 0 wave"),
  ]) |> Ui.panel(Ui.topLeft())
