// Quaternius character presets (`npm run build:characters`; ASSETS.md): one
// glb per body and LOD holding every piece as a part. The row: the male body
// with hair, then the Ranger outfit at LOD 0/1/2 and the Peasant at LOD 0.
// Every part rides the same retargeted idle/walk/run clips, so the row should
// hold one pose — a limb out of step is a skinning bug.
//
// Keys: 1 idle, 2 walk, 3 run, 0 auto-cycle (default; 4 s per clip).
// Assets: `npm run fetch:assets`.

type Gait =
  | Idle
  | Walk
  | Run

let init = { auto: true, gait: Idle }

// Auto mode: 4 s of each clip, repeating.
let gaitAt = (tts: float): Gait =>
  let phase = Math.mod(Math.floor(tts / 4.0), 3.0) in
  if phase == 0.0 then Idle else if phase == 1.0 then Walk else Run

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    match key with
    | Key.Num1 => { auto: false, gait: Idle }
    | Key.Num2 => { auto: false, gait: Walk }
    | Key.Num3 => { auto: false, gait: Run }
    | Key.Num0 => { model with auto: true }
    | _ => model

let tick = (model, dt: float, tts: float) => model

let gaitOf = (model, tts: float): Gait =>
  if model.auto then gaitAt(tts) else model.gait

// Every preset shares these clip names.
let clipName = (gait: Gait): string =>
  match gait with
  | Idle => Assets.male_lod0Clips.idle.name
  | Walk => Assets.male_lod0Clips.walk.name
  | Run => Assets.male_lod0Clips.run.name

// Loadouts: every LOD shares these part names. Unlisted parts (other hair,
// the other outfit) stay hidden; a worn piece hides the body region under it.
let parts = Assets.male_lod0Parts
let head = [parts.body_head, parts.eyes, parts.eyebrows]
let bare = [parts.body_torso, parts.body_arms, parts.body_legs, parts.body_feet, parts.hair_simple_parted]
let ranger = [parts.ranger_hood, parts.ranger_pauldrons, parts.ranger_body, parts.ranger_arms, parts.ranger_legs, parts.ranger_feet]
let peasant = [parts.hair_buzzed, parts.peasant_body, parts.peasant_arms, parts.peasant_legs, parts.peasant_feet]

// Slot 0 is leftmost on screen. glTF forward is +Z; turn to face the camera.
let figure = (asset, loadout: List<string>, slot: float, pose: Anim.t): Scene.t =>
  Scene.model(asset)
    |> Scene.modelParts(List.append(loadout, head))
    |> Scene.animate(pose)
    |> Scene.rotateY(Angle.degrees(180.0))
    |> Scene.translate(Vec3.make(2.0 - slot, 0.0, 0.0))

let camera = Camera3D.lookAt(Vec3.make(0.0, 1.3, -5.2), Vec3.make(0.0, 0.9, 0.0))

let draw = (model, tts: float) =>
  let pose = Anim.clip(clipName(gaitOf(model, tts)), tts) in
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(12.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      figure(Assets.male_lod0, bare, 0.0, pose),
      figure(Assets.male_lod0, ranger, 1.0, pose),
      figure(Assets.male_lod1, ranger, 2.0, pose),
      figure(Assets.male_lod2, ranger, 3.0, pose),
      figure(Assets.male_lod0, peasant, 4.0, pose),
    ]),
    [
      Light.ambient(Color.rgb(0.3, 0.3, 0.35)),
      Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.96, 0.88), 1.0)
        |> Light.castShadows,
    ])

let ui = (model) =>
  Ui.column([
    Ui.text("body | ranger LOD0 | LOD1 | LOD2 | peasant LOD0"),
    Ui.text("keys: 1 idle, 2 walk, 3 run, 0 auto"),
  ]) |> Ui.panel(Ui.topLeft())

expect gaitAt(1.0) == Idle
expect gaitAt(5.0) == Walk
expect gaitAt(9.0) == Run
expect gaitAt(13.0) == Idle
expect gaitOf(input(init, Key.Num3, true), 1.0) == Run
expect clipName(Walk) == Assets.male_lod2Clips.walk.name
