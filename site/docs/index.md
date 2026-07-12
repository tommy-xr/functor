# Functor — a functional toolkit for 3D games

Functor is a functional toolkit for building 3D games in **Functor Lang** — a
deliberately small, F#-inspired game-logic language. You write your game as pure
Model–View–Update functions in a `.fun` file, and:

- **there is no compile step for game logic** — the Rust runtime *interprets* the
  `.fun` directly;
- **hot reload preserves your model** — save an edit and the running game swaps to
  the new code mid-flight, keeping its state;
- **you can time-travel the running game** — pause, scrub, and step back through the
  scene's own recorded timeline;
- **one source runs on native and wasm** — the same `.fun` runs in a desktop window
  and in the browser;
- **it's LLM-native** — a text-only runtime path and inspectable state, so an agent
  can drive and observe a game with no GPU window.

Every full program on this page has a **▶ try it** button that opens it live in the
[sandbox](/sandbox.html) — nothing to install.

## Your first scene

This is exactly what `functor init` scaffolds — a lit ground plane, a slowly
rotating cube, and a sphere. The first code you see is the code you'd start from.
Press **▶ try it**, then edit a color or a number and watch it hot-reload:

```functor run
// The scene `functor init` scaffolds. Saving an edit (locally, or in the
// sandbox) hot-reloads it with the model preserved.

let init = {}

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(6.0, 4.0, -8.0, 0.0, 0.5, 0.0),
    Scene.group([
      Scene.plane()
        |> Scene.scale(20.0)
        |> Scene.lit(0.35, 0.38, 0.42),
      Scene.cube()
        |> Scene.rotateY(Angle.radians(tts * 0.5))
        |> Scene.translate(0.0, 0.75, 0.0)
        |> Scene.lit(0.25, 0.65, 1.0),
      Scene.sphere()
        |> Scene.scale(0.55)
        |> Scene.translate(-2.0, 0.55, 1.0)
        |> Scene.lit(1.0, 0.35, 0.25),
    ]),
    [
      Light.ambient(0.12, 0.12, 0.16),
      Light.directional(0.5, -1.0, 0.35, 1.0, 0.96, 0.9, 1.0)
        |> Light.castShadows,
    ])
```

## What a game looks like

A game is Model–View–Update: `init` is the starting model, `tick` steps it every
frame, `draw` renders it, and messages (from timers, effects, input) fold through
`update`. This one pulses a sphere and counts beats once a second:

```functor run
// a pulsing sphere with a beat counter
type Msg = | Beat

let init = { spin: 0.0, beat: 0.0 }

let tick = (model, dt: float, tts: float) =>
  { model with spin: model.spin + dt }

let update = (model, msg) =>
  match msg with
  | Beat => { model with beat: model.beat + 1.0 }

let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)

let draw = (model, tts: float) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere()
      |> Scene.emissive(1.0, 0.3, 0.8)
      |> Scene.scale(1.0 + 0.2 * Math.sin(model.spin * 4.0)))
```

Input is another pure function — slide the cube with A and D (click the preview
first so it has keyboard focus):

```functor run
let init = { x: 0.0 }

let input = (model, key, isDown) =>
  match isDown with
  | false => model
  | true =>
    (match key with
     | "A" => { model with x: model.x - 0.5 }
     | "D" => { model with x: model.x + 0.5 }
     | _ => model)

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -8.0, 0.0, 0.0, 0.0),
    Scene.cube()
      |> Scene.emissive(0.2, 0.9, 1.0)
      |> Scene.translate(model.x, 0.0, 0.0))
```

And physics is declarative — describe the bodies each frame; the runtime reconciles
and steps the world:

```functor run
let init = {}
let tick = (m, dt, tts) => m

let physics = (m) =>
  Physics.scene(0.0, -9.81, 0.0, [
    Physics.fixed("ground", Physics.box(20.0, 0.4, 20.0))
      |> Physics.at(0.0, -0.2, 0.0),
    Physics.dynamic("ball", Physics.sphere(0.5))
      |> Physics.at(0.3, 4.0, 0.0)
      |> Physics.restitution(0.8),
  ])

let draw = (m, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 3.0, -8.0, 0.0, 1.0, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(20.0) |> Scene.lit(0.4, 0.45, 0.55),
      Scene.sphere()
        |> Scene.scale(0.5)
        |> Scene.lit(1.0, 0.4, 0.6)
        |> Physics.transformed("ball"),
    ]),
    [
      Light.ambient(0.15, 0.15, 0.2),
      Light.directional(0.4, -1.0, 0.3, 1.0, 0.95, 0.9, 0.9) |> Light.castShadows,
    ])
```

## Where next

- **[Getting started](/docs/getting-started/)** — start in the browser sandbox, then
  set up a local project and the hot-reload dev loop.
- **[Language reference](/docs/language/)** — the whole of Functor Lang: syntax,
  semantics, and the engine prelude.
- **[The sandbox](/sandbox.html)** — edit a `.fun` live, watch it hot-reload, and
  scrub the timeline. No install.
