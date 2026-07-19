[![Release](https://img.shields.io/github/v/release/tommy-xr/functor?include_prereleases&label=release&color=41d8e6)](https://github.com/tommy-xr/functor/releases) [![Build Native](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml) [![Build WebAssembly](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml)

# functor

![Live-editing the hero scene's color while it hot-reloads with the model preserved, then scrubbing the running scene back through its own recorded timeline](docs/media/readme-hero.gif)

*This is the live hero at [functor.games](https://functor.games): scrub the running
scene back through its own recorded timeline (whole-game time travel), and
live-edit the code to watch it hot-reload with the model preserved
([still frame](docs/media/readme-hero.png)).*

Functor is a functional toolkit for building 3D games in **Functor Lang** — Functor's
own tiny, interpreted, F#-inspired game-logic language. You write your game as pure
Model–View–Update functions in a `.fun` file: there is **no transpile or compile step
for game logic** — the Rust runtime *interprets* the `.fun` directly, with
state-preserving hot reload as you save, whole-game time travel over the running model,
and the same source running on **native and wasm**.

> **Status: alpha.** Functor is early software under active development — the
> language, the prelude, and file formats can all change between releases without a
> deprecation path. Binaries and changelogs are published on the
> [releases page](https://github.com/tommy-xr/functor/releases).

## Try it in the browser

No install needed:

- **[functor.games](https://functor.games)** — the landing page's hero is a live
  Functor Lang scene you can edit in place (the GIF above).
- **[functor.games/sandbox.html](https://functor.games/sandbox.html)** — a full
  in-browser sandbox: edit a `.fun`, watch it hot-reload, and scrub the timeline.

(The redesigned pages ship with this branch.)

## Quick start

**Download a prebuilt binary** — no toolchain needed. Grab the archive for your
platform from the [releases page](https://github.com/tommy-xr/functor/releases),
extract it to get the single `functor` binary, and put it somewhere on your `PATH`:

| Platform | Asset |
| --- | --- |
| macOS (Apple Silicon) | `functor-<version>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `functor-<version>-x86_64-apple-darwin.tar.gz` |
| Linux (x86-64) | `functor-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86-64) | `functor-<version>-x86_64-pc-windows-msvc.zip` |

Then scaffold a game and run it — a window opens; edit `my-game/game.fun` and save
to hot-reload with the model preserved:

```sh
functor -d my-game init         # scaffold a starter project
functor -d my-game run native   # open a window and run it
```

Prefer to build from source? See [DEVELOPMENT.md](DEVELOPMENT.md).

## Writing a game

Games follow an Elm-style MVU loop: your model is an immutable value, and
`input`/`tick`/`update` are pure functions that return a new model (optionally
paired with an `effect`, whose result is folded back through `update`). `draw`
describes a frame — a camera plus a scene — from the model. A runner-hosted game
defines these top-level Functor Lang bindings:

```functor
let init = { … }                        // the initial model (a value)
let tick = (model, dt, tts) => model'   // per-frame step
let draw = (model, tts) => Frame.create(camera, scene)
let input = (model, key, isDown) => model'         // OPTIONAL; key: Key.t (Key.W, Key.Up, …)
let update = (model, msg) => model'                // OPTIONAL; msgs are ADT variants
let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)  // OPTIONAL timers
let physics = (model) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [body, …]) // OPTIONAL
let soundScape = (model) => AudioScene.create([source, …])         // OPTIONAL looping audio
```

The model-updating entry points (`tick`, `input`, `mouseMove`, `mouseWheel`,
`update`) may return a `(model', effect)` tuple instead of a bare model; the
effect's result folds back through `update`. (`init` is a plain value — no
effect; `draw`/`physics`/`soundScape`/`ui`/`subscriptions` return their own
specific values.) The full language and prelude
(`Scene.*` / `Camera.*` / `Frame.*` / `Light.*` / `Physics.*` / …)
are documented in the `functor-lang` skill (`.claude/skills/functor-lang/`) and
`docs/functor-lang.md`. See `examples/hello/game.fun` or
`examples/primitives/game.fun` for complete games.

Because the model is a plain, cheap-to-clone value the host holds between frames,
the runtime can **hot-reload your edits with the model preserved** and **record the
model every rendered frame** — that's what powers the live editing and whole-game
time-travel scrubber you see at [functor.games](https://functor.games) (design notes:
`docs/time-travel.md`).

## Design principles

- **Functional-core, imperative shell.** As much functionality as possible lives in the pure
  functional core (the Functor Lang game logic); side effects are pushed to a thin imperative shell.
- **LLM-native.** Functor functionality is introspectable by LLMs at runtime — favoring live
  evaluation, a text-only runtime, and inspectable state.
- **Simplicity and incrementality.** Prefer small, incremental PRs, leveraging stacked PRs where
  applicable.
- **Fast inner loop.** Iterating and experimenting is fast for both humans and LLMs.

## Repository layout

| Path | What it is |
| --- | --- |
| `functor-lang/` | The Functor Lang language crate — parser, IR, interpreter, typechecker (`functor-lang parse`/`ir`/`run`/`trace`/`check`) |
| `runtime/functor-runtime-common/` | Shared Rust runtime: rendering, assets, geometry, materials, the Functor Lang prelude (`FunctorHost`) |
| `runtime/functor-runtime-desktop/` | Desktop runtime (native/GLFW), including the Functor Lang producer — a library the `functor` CLI links in and runs in-process |
| `runtime/functor-runtime-web/` | Web runtime (WebGL2); built into a wasm bundle, interprets the `.fun` in the browser |
| `cli/` | The `functor` CLI (`init` / `build` / `run` / `develop`) |
| `tools/` | Editor tooling: `functor-lang-vscode` (extension), `functor-lang-lsp` (language server), `functor-sdk` (TS debug-runtime SDK) |
| `examples/*/` | Sample games — e.g. `hello` (a lineup of glTF sample models with a WASD + mouse free-look camera), `primitives`, `lighting` |

## Running a sample

The bundled sample games live in this repo, so clone it to run them. Some samples
reference glTF model assets that aren't checked in (they download from
[BabylonJS Assets](https://github.com/BabylonJS/Assets/)); fetch them first:

```sh
npm run fetch:assets
```

The CLI operates on a directory containing a `functor.json`
(`{"language": "functor-lang", "entry": "game.fun"}`). Point it at the example with `-d`.
The `run` command interprets the game's `.fun` and launches it — no build step:

```sh
# Native — opens a window
functor -d examples/hello run native

# A primitives-only sample (no assets needed)
functor -d examples/primitives run native

# Web — serves the .fun + wasm bundle at http://127.0.0.1:8080
functor -d examples/primitives run wasm
```

`native` is the default environment, so `... run` is equivalent to `... run native`.
(These commands assume `functor` is on your `PATH`; when running from a source build,
use `./target/release/functor` instead — see [DEVELOPMENT.md](DEVELOPMENT.md).)

### CLI commands

| Command | Description |
| --- | --- |
| `functor -d <dir> init [3d\|fps]` | Scaffold a new Functor Lang project (`3d` is the default) |
| `functor -d <dir> build [native\|wasm]` | Typecheck the `.fun` project (diagnostics are errors) |
| `functor -d <dir> run [native\|wasm]` | Interpret and run the game (native window / browser) |
| `functor -d <dir> develop [native\|wasm]` | Same as `run` — Functor Lang hot-reload is built into the runtime |

For build-from-source instructions and what `build`/`run` do under the hood, see
[DEVELOPMENT.md](DEVELOPMENT.md).

## Credits

- Demo 3D assets are from [BabylonJS Assets](https://github.com/BabylonJS/Assets/)
  (CC-BY 4.0). `Xbot.glb` (`examples/hello`, `examples/animation`, `examples/crossfade`) is Adobe
  Mixamo's "X Bot" character, distributed via BabylonJS Assets.
- The hand model in `examples/glove` (`vr_glove_model.glb`) is from Valve's
  [SteamVR Unity Plugin](https://github.com/ValveSoftware/steamvr_unity_plugin)
  (© Valve Corporation, BSD-3-Clause — notice at `examples/glove/LICENSE.steamvr`),
  converted to glTF with FBX2glTF.
- Demo audio (`examples/*/*.wav`) is procedurally synthesized — original/CC0, no
  third-party samples. Regenerate with `npm run generate:audio`
  (`scripts/generate-audio.mjs`).
- `examples/asteroids` uses [Kenney](https://kenney.nl) assets (CC0): sounds from
  the [Sci-Fi Sounds](https://kenney.nl/assets/sci-fi-sounds) pack and the ship
  model (`craft_racer`) from the [Space Kit](https://kenney.nl/assets/space-kit)
  (fetched by `npm run fetch:assets`, not checked in). Thanks, Kenney! Details
  per file in `examples/asteroids/ASSETS.md`.
