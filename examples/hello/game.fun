// examples/hello — the Functor Lang port of examples/hello (F#), the glTF
// lineup with a WASD + mouse free-look camera (docs/functor-lang.md E1). Assets are
// the BabylonJS/Assets samples (npm run fetch:assets); paths resolve
// relative to this dir, exactly as the F# game's Model.file paths. Run with:
//
//   functor -d examples/hello run native
//
// Reproduce the F#-parity capture (same pose, deterministic):
//
//   functor -d examples/hello run native \
//     --capture-frame /tmp/functor-lang.png --fixed-time 2.0 --capture-time 0.8
//   functor -d examples/hello run native \
//     --capture-frame /tmp/fs.png --fixed-time 2.0 --capture-time 0.8
//
// (The HUD's "frame N" line counts ticks, which run at wall-clock rate even
// under --fixed-time — so that one line's digits differ between ANY two
// runs, F#-vs-F# included. Every other pixel is directly comparable.)
//
// Transform-order note (the primitives rule): F#'s
// `x |> translateX a |> rotateY r |> scale s` right-multiplies (scale hits
// the vertex first); Functor Lang transforms apply OUTERMOST-LAST, so the same
// composition reads `x |> Scene.scale(s) |> Scene.rotateY(r)
// |> Scene.translate(a, …)` — the pipe order reverses.
//
// Deliberate deltas from hello.fs (all invisible):
// - the vestigial pong state (paddles/ball) and the message-loop noise
//   (Effect.wrapped/Effect.map ping-pong, Debug.log prints) are not ported —
//   they never rendered anything. The `counter` IS ported (the HUD shows
//   it): +3 per tick, +1 per Tick subscription second, exactly as F#.
// - heights: hello.fs samples its ripple in f32; Functor Lang numbers are f64, so
//   ~half the 1024 heights differ by ~1 ulp of f32 (<= 8e-9) after the
//   boundary cast — sub-visual, but not guaranteed bit-identical shading
//   on every terrain pixel.
// Camera, input math, terrain, textures, materials, the model lineup, the
// lights, and the HUD otherwise mirror hello.fs value for value.

// Mouse sensitivity, radians of rotation per pixel of motion.
let sensitivity = 0.003
// Clamp pitch just short of straight up/down (~85 degrees) to avoid flipping.
let pitchLimit = 1.5
// WASD move speed, units per second.
let speed = 3.0

// Textures over the asset pipeline, declared once and passed as values
// (F#'s `Texture.file`). dirt.png/grid.png live beside this file, exactly
// as in examples/hello.
let dirtTexture = Texture.file("dirt.png")
let gridTexture = Texture.file("grid.png")

// The previous mouse position, or NoMouse before the first event — so the
// view turns by per-frame deltas and never jumps on the first sample
// (F#'s `lastMouse: (float32 * float32) option`).
type Mouse =
  | NoMouse
  | MouseAt(x: float, y: float)

// Fired every second by the subscription below (F#'s `Msg.Tick`).
type Msg =
  | Tick

let init = {
  held: { up: false, down: false, left: false, right: false },
  eye: { x: 0.0, y: 0.0, z: -5.0 },
  yaw: 0.0,
  pitch: 0.0,
  lastMouse: NoMouse,
  counter: 0.0,
}

// Map WASD and the arrow keys onto the held-key flags. Both key-downs and
// key-ups flow through here; repeats just re-set the same flag.
let setHeld = (held, key, isDown) =>
  match key with
  | "W" => { held with up: isDown }
  | "Up" => { held with up: isDown }
  | "S" => { held with down: isDown }
  | "Down" => { held with down: isDown }
  | "A" => { held with left: isDown }
  | "Left" => { held with left: isDown }
  | "D" => { held with right: isDown }
  | "Right" => { held with right: isDown }
  | _ => held

let input = (model, key, isDown) =>
  { model with held: setHeld(model.held, key, isDown) }

let clamp = (v, lo, hi) =>
  match v < lo with
  | true => lo
  | false => (match hi < v with | true => hi | false => v)

// Mouse right turns the view right; mouse up looks up.
let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: clamp(model.pitch - (y - lastY) * sensitivity, 0.0 - pitchLimit, pitchLimit),
        lastMouse: MouseAt(x, y) }

let axis = (neg, pos) =>
  (match pos with | true => 1.0 | false => 0.0)
    - (match neg with | true => 1.0 | false => 0.0)

// Move the eye from the held keys, relative to where we're looking, scaled
// by dt for frame-rate independence. Forward/right in the ground plane from
// the current yaw (yaw = 0 -> +Z): forward = (sin yaw, 0, cos yaw),
// right = (-cos yaw, 0, sin yaw). The counter mirrors F#'s tick (+3 per
// frame); the Tick subscription below adds 1 per second.
let tick = (model, dt, tts) =>
  let f = axis(model.held.down, model.held.up) in
  let r = axis(model.held.left, model.held.right) in
  let k = speed * dt in
  let e = model.eye in
  { model with
      counter: model.counter + 3.0,
      eye: {
        x: e.x + k * (f * Math.sin(model.yaw) - r * Math.cos(model.yaw)),
        y: e.y,
        z: e.z + k * (f * Math.cos(model.yaw) + r * Math.sin(model.yaw)) } }

let subscriptions = (model) => Sub.every(Time.seconds(1.0), Tick)

let update = (model, msg) =>
  match msg with
  | Tick => { model with counter: model.counter + 1.0 }

// hello.fs's ripple, sampled over grid coords (row, col) — gentle static
// dunes. (F# computes this in f32; see the header note on the ~1-ulp seam.)
let ripple = (r, c) => 0.05 * (Math.sin(c * 0.5) + Math.cos(r * 0.5))

// Synthwave terrain: a dirt-textured heightmap (XZ, Y-up), lit so the
// slopes catch the sun — F#'s `heightmapFn 32 32`, sampled in user space
// with List builtins (same placement: y = -2.5, pushed back, 30x).
let terrain = () =>
  Scene.heightmap(
    List.range(32.0) |> List.map((r) =>
      List.range(32.0) |> List.map((c) => ripple(r, c))))
    |> Scene.scale(30.0)
    |> Scene.translate(0.0, -2.5, 4.0)
    |> Scene.litTexture(dirtTexture)

// A lineup of glTF samples from BabylonJS/Assets exercising the model
// pipeline. Skinned + animated: shark, fish, Xbot. Non-skinned:
// ExplodingBarrel. Raw model units vary wildly (the barrel is ~72 units
// tall, Xbot is Mixamo-style cm scale), hence the per-model scales.
let models = () =>
  Scene.group([
    Scene.model("shark.glb")
      |> Scene.scale(0.002)
      |> Scene.rotateY(Angle.degrees(180.0))
      |> Scene.translate(3.0, 1.0, 3.0),
    Scene.model("fish.glb")
      |> Scene.scale(0.002)
      |> Scene.translate(-3.0, 1.0, 3.0),
    Scene.model("Xbot.glb")
      |> Scene.scale(0.015)
      |> Scene.translate(1.5, -1.0, 3.0),
    Scene.model("ExplodingBarrel.glb")
      |> Scene.scale(0.02)
      |> Scene.translate(0.0, -1.5, 3.0),
  ])

// A row of lit primitives in front of the lineup — Lambert shading reads
// clearly on the sphere (and the cube's faces).
let litRow = () =>
  Scene.group([
    Scene.cylinder() |> Scene.translate(0.0, -2.5, 0.0),
    Scene.sphere() |> Scene.scale(0.7) |> Scene.translate(-2.0, 1.0, 2.0),
    Scene.cube() |> Scene.translate(2.0, 1.0, 2.0),
  ]) |> Scene.lit(0.85, 0.85, 0.9)

let draw = (model, tts) =>
  Frame.createLit(
    // First-person camera: WASD moves the eye, the mouse turns yaw/pitch.
    Camera.firstPerson(
      model.eye.x, model.eye.y, model.eye.z,
      Angle.radians(model.yaw), Angle.radians(model.pitch), Angle.degrees(60.0)),
    Scene.group([
      terrain(),
      litRow(),
      // Self-lit neon sphere, rendered fullbright (emissive material).
      Scene.sphere() |> Scene.scale(0.5) |> Scene.translate(0.0, 2.3, 2.0)
        |> Scene.emissive(1.0, 0.2, 0.8),
      // The glowing grid-texture sign quad (emissive texture, fullbright).
      Scene.quad() |> Scene.scale(1.2) |> Scene.translate(0.0, 0.3, 1.5)
        |> Scene.emissiveTexture(gridTexture),
      models(),
    ]),
    // A low blue-ish ambient plus a warm "sun" angled down from the upper
    // right; lit surfaces shade by orientation, emissive ones stay bright.
    [
      Light.ambient(0.2, 0.2, 0.28),
      Light.directional(-0.5, -1.0, -0.35, 1.0, 0.96, 0.85, 1.1) |> Light.castShadows,
    ])

let join = (parts) => parts |> List.fold((acc, s) => Text.concat(acc, s), "")
let f1 = (n) => Text.fixed(n, 1.0)
let f0 = (n) => Text.fixed(n, 0.0)

// The HUD, a pure function of the model — hello.fs's `ui` line for line
// (Text.fixed formats like F#'s %.1f / %.0f / %d).
let ui = (model) =>
  Ui.column([
    Ui.text("functor · hello"),
    Ui.textColor(1.0, 0.85, 0.4,
      join(["eye  ", f1(model.eye.x), " ", f1(model.eye.y), " ", f1(model.eye.z)])),
    Ui.text(join(["look  yaw ", f0(model.yaw), "  pitch ", f0(model.pitch)])),
    Ui.text(Text.concat("frame ", f0(model.counter))),
  ]) |> Ui.panel(Ui.topLeft())
