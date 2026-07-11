[![Build Native](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml) [![Build WebAssembly](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml)

# functor

Functor: A functional toolkit for building 3D games in **Functor Lang**, Functor's own
interpreted, F#-inspired game-logic language.

You write your game as a `.fun` file. There is **no transpile or compile step for
game logic** — the Rust runtime *interprets* the `.fun` directly:

- **native** — the `functor` desktop runtime (GLFW + OpenGL), run in-process by the
  `functor` CLI, loads and runs your `.fun`, with hot-reloading on save for fast iteration.
- **wasm** — the web runtime (WebGL2) ships the `.fun` source as text and
  interprets it in the browser.

The same interpreter runs everywhere Rust runs. The heavy per-frame work
(rendering, skinning, physics) stays in the Rust shell; only the game logic is
Functor Lang.

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
let input = (model, key, isDown) => model'         // OPTIONAL; key = "W"/"Up"/"Space"
let update = (model, msg) => model'                // OPTIONAL; msgs are ADT variants
let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)  // OPTIONAL timers
let physics = (model) => Physics.scene(0.0, -9.81, 0.0, [body, …]) // OPTIONAL
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

## Prerequisites

Install the following (the versions in parentheses are known-good):

- [Rust](https://rustup.rs/) stable (`1.91`) with the wasm target:
  `rustup target add wasm32-unknown-unknown`
- [Node.js + npm](https://nodejs.org/) (`node 22`, `npm 10`)
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) (`0.12+`) — `npm install -g wasm-pack`

`watchexec` is no longer needed — Functor Lang hot-reload is built into the runtime, so
`functor develop` needs no external file watcher. There is also **no .NET / Fable
dependency**: the toolchain is Rust + Node only.

On Linux you also need the native GL/X11 dev packages (see
`.github/workflows/build-native.yml` for the exact `apt` list).

## Building the CLI

Build the CLI. **Order matters:** the CLI embeds the web runtime bundle at compile
time (via `include_bytes!`), so the wasm bundle must exist before the `functor` binary is built.

```sh
wasm-pack build runtime/functor-runtime-web --target=web     # web bundle (embedded into the CLI)
cargo build --bin functor                                    # the CLI (embeds the desktop runtime)
```

Or use the bundled convenience script, which runs both in order:

```sh
npm run build:cli
```

This produces a single binary in `target/debug/`: `functor` (the CLI, with the desktop
runtime linked in as a library and run in-process — there is no separate `functor-runner`).

## Running a sample (`examples/hello`)

Some samples reference glTF model assets that aren't checked in (they download from
[BabylonJS Assets](https://github.com/BabylonJS/Assets/)); fetch them first:

```sh
npm run fetch:assets
```

The CLI operates on a directory containing a `functor.json`
(`{"language": "functor-lang", "entry": "game.fun"}`). Point it at the example with `-d`.
The `run` command interprets the game's `.fun` and launches it — no build step:

```sh
# Native — opens a window
./target/debug/functor -d examples/hello run native

# A primitives-only sample (no assets needed)
./target/debug/functor -d examples/primitives run native

# Web — serves the .fun + wasm bundle at http://127.0.0.1:8080
./target/debug/functor -d examples/primitives run wasm
```

`native` is the default environment, so `... run` is equivalent to `... run native`.

### CLI commands

| Command | Description |
| --- | --- |
| `functor -d <dir> init [3d\|fps]` | Scaffold a new Functor Lang project (`3d` is the default) |
| `functor -d <dir> build [native\|wasm]` | Typecheck the `.fun` project (diagnostics are errors) |
| `functor -d <dir> run [native\|wasm]` | Interpret and run the game (native window / browser) |
| `functor -d <dir> develop [native\|wasm]` | Same as `run` — Functor Lang hot-reload is built into the runtime |

### What `build`/`run` do under the hood

1. `build` loads the project (the entry `.fun` plus every sibling `.fun` — file = module)
   and typechecks the whole program; diagnostics are errors here.
2. (native) `run` runs the desktop runtime in-process on the entry `.fun` (no separate
   process); it **interprets** the `.fun` each frame and hot-reloads it on save,
   preserving the model.
3. (wasm) `run` serves the project directory — the `.fun` ships as text; the embedded
   web runtime fetches and interprets it. (File-watch hot-reload is native-only;
   reload the page to pick up saved edits.)

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
