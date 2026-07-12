# Functor Lang — the functor game language

Functor Lang is a deliberately small, F#-inspired language for game logic. It is
*interpreted* — the source ships as text and runs natively, in the browser, and
in the [sandbox](/sandbox.html) — and it hot-reloads with your game's state
preserved. Every full program on this page has a **▶ try it** button that
opens it live in the sandbox.

## Get started

The fastest path is the [sandbox](/sandbox.html) — nothing to install. For
local development you need Rust (stable), Node 22, and `wasm-pack`; then:

```sh
git clone https://github.com/tommy-xr/functor && cd functor
npm run build:cli                    # builds the functor CLI + runtimes
```

An Functor Lang project is a directory with two files:

```sh
# functor.json
{ "language": "functor-lang", "entry": "game.fun" }
```

And the game itself — here is the smallest interesting one:

```functor run
let init = {}
let tick = (m, dt, tts) => m
let draw = (m, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),
    Scene.cube()
      |> Scene.emissive(1.0, 0.2, 0.8)
      |> Scene.rotateY(Angle.radians(tts)))
```

Run it:

```sh
functor -d . run native      # desktop window
functor -d . run wasm        # serves it in the browser
functor -d . develop         # hot-reload loop: save the file, see it in ~1 frame
functor -d . build           # typecheck (diagnostics are errors)
```

**Hot reload preserves the model.** Under `develop` (and in
the sandbox), saving an edit swaps the program under the *running* state: a
bouncing ball keeps bouncing, mid-arc, with your new gravity. A broken edit keeps the
old program running and shows the error. An edited `init` takes effect on
restart, not on reload.

## A complete game

A game is Model–View–Update: `init` is the starting model,
`tick` steps it every frame, `draw` renders it, and messages
(from timers, effects) fold through `update`. This one pulses a sphere and
counts beats once a second:

```functor run
// a pulsing sphere with a beat counter
type Msg = | Beat

let init = { spin: 0.0, beat: 0.0 }

let tick = (model, dt: Float, tts: Float) =>
  { model with spin: model.spin + dt }

let update = (model, msg) =>
  match msg with
  | Beat => { model with beat: model.beat + 1.0 }

let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)

let draw = (model, tts: Float) =>
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
