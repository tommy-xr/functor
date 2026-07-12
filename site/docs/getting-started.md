# Getting started

The fastest way in is the browser — no install, no toolchain. When you're ready to
build a real project, set up the CLI locally for the same live loop on your desktop.

## Start in the sandbox

The **[sandbox](/sandbox.html)** runs Functor Lang entirely in your browser. It opens
on a small program you can edit in place — try this one:

```functor run
let init = {}
let tick = (m, dt, tts) => m
let draw = (m, tts: float) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),
    Scene.cube()
      |> Scene.emissive(1.0, 0.2, 0.8)
      |> Scene.rotateY(Angle.radians(tts)))
```

Three things to try:

- **Edit and watch it hot-swap.** Change a color or a number. The scene reloads under
  the *running* model — no restart. (A live edit resets the recorded timeline, so you
  start scrubbing fresh from the edit.)
- **Scrub the timeline.** The scrubber pauses the running scene and steps back through
  the frames it has already recorded — the whole game's state, not a video.
- **Break it on purpose.** An unbalanced paren surfaces the parse error while the last
  good frame keeps rendering; fix it and it recovers.

## Set up locally

For a real project you build the `functor` CLI once, then work against a directory on
disk with the same hot-reload loop.

### Prerequisites

- **Rust** (stable) with the wasm target: `rustup target add wasm32-unknown-unknown`
- **Node 22+** (npm 10)
- **[`wasm-pack`](https://rustwasm.github.io/wasm-pack/)** — `npm install -g wasm-pack`

There is no .NET / Fable dependency and no external file watcher — hot reload is built
into the runtime.

### Build the CLI

Order matters: the CLI embeds the web-runtime wasm bundle at compile time, so the
bundle must be built first. The bundled script runs both steps in order:

```sh
npm run build:cli    # builds the wasm bundle, then the functor CLI
```

This produces a single binary at `target/debug/functor` — the CLI with the desktop
runtime linked in.

### Scaffold a game

`init` writes a starter project (it won't overwrite an existing `functor.json` or
`game.fun`). `3d` is the default template; `fps` is a first-person starter:

```sh
./target/debug/functor -d my-game init        # 3d starter (the scene on the overview)
./target/debug/functor -d my-game init fps     # first-person starter
```

A project is just a directory with a `functor.json` and an entry `.fun`:

```sh
# my-game/functor.json
{ "language": "functor-lang", "entry": "game.fun" }
```

Every sibling `.fun` in that directory loads with the project (`file = module`).

### Run it

```sh
./target/debug/functor -d my-game run native     # opens a desktop window (native is the default)
./target/debug/functor -d my-game run wasm        # serves the .fun + wasm at http://127.0.0.1:8080
./target/debug/functor -d my-game develop         # same as run — hot reload is always on
./target/debug/functor -d my-game build           # typecheck the whole project (diagnostics are errors)
```

`run` interprets the `.fun` each frame — nothing compiles. Edit `game.fun`, save, and
the window updates in about a frame with the model preserved. On wasm, file-watch
hot-reload is native-only; reload the page to pick up saved edits.

### Typed asset names

If your game loads glTF models, `import` scans the project's `*.glb`/`*.gltf`
(headless, no GPU) and writes an `assets.fun` module of typed clip constants, so a
misspelled animation clip is a check-time error instead of a silent bind pose:

```sh
./target/debug/functor -d my-game import     # writes my-game/assets.fun
```

Rerun it when your models change, and check the generated file in.

## Hot reload: what to expect

Saving an edit reloads the program in about a frame **with the model preserved** — a
bouncing ball keeps bouncing, mid-arc, under your new gravity. `init` does not re-run,
a broken edit keeps the last good scene rendering, and the running game records itself
so you can pause and scrub back through it.

The **[time travel & hot reload](/docs/time-travel/)** guide has the full rules —
closure rebinding, effect reset, and how a live edit resets the scrub timeline.

From here, the **[language reference](/docs/language/)** covers the whole of Functor
Lang — syntax, semantics, and the engine prelude (`Scene.*` / `Camera.*` / `Frame.*` /
`Physics.*` / …).
