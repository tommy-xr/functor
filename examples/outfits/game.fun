// Outfits kata v0: Quaternius character presets (`npm run build:characters`;
// ASSETS.md) in two modes.
//
// - Loadout: one character on an inspection platform. Pick a body and an
//   outfit (a preset), and idle/walk/run.
// - Crowd: `count` characters in distance tiers (crowd.fun). Each one's
//   static subtree is built once by `Scene.memo`, keyed on its loadout data;
//   per frame it only gets a pose and a transform.
//
// Keys: M mode · B body · O outfit · 1/2/3 idle/walk/run · C count · S shadows.
// `GET /state` reports `skinned` draws/pose evals and `stages` timings.
// Assets: `npm run fetch:assets`.

type Mode =
  | LoadoutView
  | CrowdView

type Gait =
  | Idle
  | Walk
  | Run

type Msg =
  | SetMode(mode: Mode)
  | SetBody(body: Loadout.Body)
  | SetOutfit(outfit: Loadout.Outfit)
  | SetGait(gait: Gait)
  | SetCount(count: float)
  | ToggleShadows

let counts = [1.0, 25.0, 50.0, 100.0]

let startCount = 100.0

let init = {
  mode: LoadoutView,
  kit: Loadout.dress(Loadout.Male, Loadout.Ranger),
  gait: Idle,
  count: startCount,
  shadows: true,
  crowd: Crowd.layout(startCount),
}

// The next item after `x` in `xs`, wrapping.
let cycle = (xs, x) =>
  let at = xs |> List.indexedMap((j, y) => if y == x then j else -1.0) |> List.maximum |> Option.defaultValue(-1.0) in
  xs |> List.nth(Math.mod(at + 1.0, List.length(xs))) |> Option.defaultValue(x)

let outfitOf = (model) => Loadout.outfitOf(model.kit) |> Option.defaultValue(Loadout.Bare)

let update = (model, msg: Msg) =>
  match msg with
  | SetMode(mode) => { model with mode: mode }
  | SetBody(body) => { model with kit: Loadout.dress(body, outfitOf(model)) }
  | SetOutfit(outfit) => { model with kit: Loadout.dress(model.kit.body, outfit) }
  | SetGait(gait) => { model with gait: gait }
  | SetCount(count) => { model with count: count, crowd: Crowd.layout(count) }
  | ToggleShadows => { model with shadows: not model.shadows }

let input = (model, key, isDown) =>
  if not isDown then model
  else
    match key with
    | Key.M => update(model, SetMode(if model.mode == LoadoutView then CrowdView else LoadoutView))
    | Key.B => update(model, SetBody(cycle(Loadout.bodies, model.kit.body)))
    | Key.O => update(model, SetOutfit(cycle(Loadout.outfits, outfitOf(model))))
    | Key.Num1 => update(model, SetGait(Idle))
    | Key.Num2 => update(model, SetGait(Walk))
    | Key.Num3 => update(model, SetGait(Run))
    | Key.C => update(model, SetCount(cycle(counts, model.count)))
    | Key.S => update(model, ToggleShadows)
    | _ => model

let tick = (model, dt: float, tts: float) => model

// Quest targets: 120 Hz, no foveation (desktop ignores both).
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0))

let settings = (model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))

// Every preset shares these clip names.
let clips = Assets.male_base_lod0Clips

let gaitClip = (gait: Gait): string =>
  match gait with
  | Idle => clips.idle.name
  | Walk => clips.walk.name
  | Run => clips.run.name

// A character's static subtree, built once per memo key. Presets are a single
// model; part toggles will make it a group without changing the deps.
let figure = (look: Crowd.Look): Scene.t => Scene.model(Loadout.asset(look.kit, look.lod))

let walkRun = (phase: float, run: float, tts: float): Anim.t =>
  Anim.blend([
    (Anim.clip(clips.walk.name, tts + phase), 1.0 - run),
    (Anim.clip(clips.run.name, tts + phase), run),
  ])

let place = (seat: Crowd.Seat, scene: Scene.t): Scene.t =>
  scene |> Scene.rotateY(Angle.radians(seat.yaw)) |> Scene.translate(Vec3.make(seat.x, 0.0, seat.z))

let member = (m: Crowd.Member, tts: float): Scene.t =>
  Scene.memo(m.key, m.look, figure)
    |> Scene.animate(walkRun(m.phase, m.run, tts))
    |> place(m.seat)

// A far batch's instanced subtree, built once per (preset, bucket).
let farBatch = (b: Crowd.Batch): Scene.t =>
  figure(b.look)
    |> Scene.instanced(b.seats |> List.map((s) =>
         Instance.at(Vec3.make(s.x, 0.0, s.z)) |> Instance.rotateY(Angle.radians(s.yaw))))

let batch = (b: Crowd.Batch, tts: float): Scene.t =>
  Scene.memo(b.key, b, farBatch)
    |> Scene.animate(walkRun(Crowd.bucketPhase(b.bucket), Crowd.bucketRun(b.bucket), tts))

let ground = Scene.plane() |> Scene.scale(60.0) |> Scene.lit(Color.rgb(0.2, 0.22, 0.26))

let lights = (model) =>
  let sun = Light.directional(Vec3.make(-0.4, -1.0, 0.5), Color.rgb(1.0, 0.96, 0.88), 1.0) in
  [Light.ambient(Color.rgb(0.32, 0.33, 0.4)), if model.shadows then sun |> Light.castShadows else sun]

// The inspection bay: platform + one LOD0 character swaying to show 3/4 views.
let loadoutScene = (model, tts: float): Scene.t =>
  Scene.group([
    ground,
    Scene.cylinder() |> Scene.scaleXYZ(2.0, 0.08, 2.0) |> Scene.translate(Vec3.make(0.0, 0.04, 0.0))
      |> Scene.lit(Color.rgb(0.3, 0.42, 0.46)),
    Scene.memo("loadout", { kit: model.kit, lod: 0.0 }, figure)
      |> Scene.animate(Anim.clip(gaitClip(model.gait), tts))
      |> Scene.rotateY(Angle.degrees(180.0 + 30.0 * Math.sin(tts * 0.6)))
      |> Scene.translate(Vec3.make(0.0, 0.08, 0.0)),
  ])

let crowdScene = (model, tts: float): Scene.t =>
  Scene.group([
    ground,
    Scene.group(model.crowd.members |> List.map((m) => member(m, tts))),
    Scene.group(model.crowd.batches |> List.map((b) => batch(b, tts))),
  ])

let loadoutCamera = Camera3D.lookAt(Vec3.make(0.0, 1.35, -3.6), Vec3.make(0.0, 0.95, 0.0))

let crowdCamera = Camera3D.lookAt(Vec3.make(0.0, 4.0, -1.5), Vec3.make(0.0, 0.4, 9.0))

let draw = (model, tts: float) =>
  match model.mode with
  | LoadoutView => Frame.createLit(loadoutCamera, loadoutScene(model, tts), lights(model))
  | CrowdView => Frame.createLit(crowdCamera, crowdScene(model, tts), lights(model))

// --- UI -------------------------------------------------------------------

let choice = (label: string, selected: bool, msg: Msg) =>
  Ui.button(if selected then $"[{label}]" else label, msg)

let loadoutPanel = (model) =>
  let outfit = outfitOf(model) in
  Ui.column(
    [
      Ui.text($"preset: {Loadout.presetName(model.kit, 0.0)}"),
      Ui.row(Loadout.bodies |> List.map((b) => choice(Loadout.bodyName(b), b == model.kit.body, SetBody(b)))),
      Ui.row(Loadout.outfits |> List.map((o) => choice(Loadout.outfitName(o), o == outfit, SetOutfit(o)))),
      Ui.row([
        choice("idle", model.gait == Idle, SetGait(Idle)),
        choice("walk", model.gait == Walk, SetGait(Walk)),
        choice("run", model.gait == Run, SetGait(Run)),
      ]),
    ]
      |> List.append(model.kit.wear |> List.map((w) => Ui.text($"{Loadout.slotName(w.slot)}: {Loadout.outfitName(w.item)}"))))

let crowdPanel = (model) =>
  let tally = (tier) => model.crowd.members |> List.filter((m) => m.tier == tier) |> List.length in
  let far = model.crowd.batches |> List.map((b) => List.length(b.seats)) |> List.sum in
  Ui.column([
    Ui.row(counts |> List.map((c) => choice($"{c}", c == model.count, SetCount(c)))),
    Ui.text($"near {tally(Crowd.Near)} · mid {tally(Crowd.Mid)} · far {far} in {List.length(model.crowd.batches)} batches"),
    choice(if model.shadows then "shadows on" else "shadows off", model.shadows, ToggleShadows),
  ])

let ui = (model) =>
  Ui.column([
    Ui.row([choice("loadout", model.mode == LoadoutView, SetMode(LoadoutView)), choice("crowd", model.mode == CrowdView, SetMode(CrowdView))]),
    if model.mode == LoadoutView then loadoutPanel(model) else crowdPanel(model),
    Ui.text("keys: M mode, B body, O outfit, 1-3 gait, C count, S shadows"),
  ]) |> Ui.panel(Ui.topLeft())

expect cycle(counts, 25.0) == 50.0
expect cycle(counts, 100.0) == 1.0
expect (update(init, SetOutfit(Loadout.Peasant)).kit) == Loadout.dress(Loadout.Male, Loadout.Peasant)
// Switching body keeps the outfit.
expect outfitOf(update(update(init, SetOutfit(Loadout.Peasant)), SetBody(Loadout.Female))) == Loadout.Peasant
expect (input(init, Key.C, true)).count == 1.0
expect (input(init, Key.C, true)).crowd == Crowd.layout(1.0)
expect (input(init, Key.O, true)) == update(init, SetOutfit(Loadout.Peasant))
