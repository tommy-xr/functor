[![Build Native](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-native.yml) [![Build WebAssembly](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml/badge.svg)](https://github.com/tommy-xr/functor/actions/workflows/build-wasm.yml)

# functor

Functor: A functional toolkit for building 3D games in F#.

You write your game in **F#** against the `Functor.Game` library. [Fable](https://fable.io/)
transpiles that F# to **Rust**, which is then compiled to one of two targets:

- **native** — a dynamic library loaded by the `functor-runner` desktop runtime (GLFW + OpenGL),
  with hot-reloading for fast iteration.
- **wasm** — a WebAssembly bundle (built with `wasm-pack`) served to the browser (WebGL2).

## Writing a game

Games follow an Elm-style MVU loop: your model is immutable, and `input`/`tick`/`update` are pure
functions that return a new model plus an `effect` (which can produce messages handled by
`update`). `draw3d` describes a frame — a camera plus a scene — from the model. A game is built
fluently with `GameBuilder`:

```fsharp
GameBuilder.local initialModel
|> GameBuilder.init (Effect.wrapped Start)  // startup effect, run once before the first frame
|> GameBuilder.input input                  // 'model -> Input.t      -> 'model * effect<'msg>
|> GameBuilder.tick tick                    // 'model -> FrameTime    -> 'model * effect<'msg>
|> GameBuilder.update update                // 'model -> 'msg         -> 'model * effect<'msg>
|> GameBuilder.subscriptions subscriptions  // 'model -> Sub<'msg>    (timers, e.g. Sub.every)
|> GameBuilder.draw3d draw3d                // 'model -> FrameTime    -> Graphics.Frame
|> Runtime.runGame
```

See `examples/hello/hello.fs` for a complete game using all of these.

## Design principles

- **Functional-core, imperative shell.** As much functionality as possible lives in the pure
  functional core; side effects are pushed to a thin imperative shell.
- **LLM-native.** Functor functionality is introspectable by LLMs at runtime — favoring live
  evaluation, a text-only runtime, and inspectable state.
- **Simplicity and incrementality.** Prefer small, incremental PRs, leveraging stacked PRs where
  applicable.
- **Fast inner loop.** Iterating and experimenting is fast for both humans and LLMs.

## Repository layout

| Path | What it is |
| --- | --- |
| `src/Functor.Game/` | The F# game framework (`Game`, `Scene3D`, `Camera`, `Input`, `Effect`, `Sub`, `Math`, …) |
| `src/` | `functor-lib` — the Rust crate produced by Fable from `Functor.Game` |
| `runtime/functor-runtime-common/` | Shared Rust runtime: rendering, assets, geometry, materials |
| `runtime/functor-runtime-desktop/` | Desktop runtime; builds the `functor-runner` binary (native/GLFW) |
| `runtime/functor-runtime-web/` | Web runtime (WebGL2); built into a wasm bundle |
| `cli/` | The `functor` CLI (`build` / `run` / `develop`; `init` is not yet implemented) |
| `examples/hello/` | Sample game (`hello.fs`) — a Pong-style scene with a WASD + mouse free-look camera |

## Prerequisites

Install the following (the versions in parentheses are known-good):

- [.NET SDK 8](https://dotnet.microsoft.com/download) (`8.0.x`) — runs Fable
- [Rust](https://rustup.rs/) stable (`1.91`) with the wasm target:
  `rustup target add wasm32-unknown-unknown`
- [Node.js + npm](https://nodejs.org/) (`node 22`, `npm 10`)
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) (`0.12+`) — `npm install -g wasm-pack`
- [`watchexec`](https://github.com/watchexec/watchexec) — only needed for `functor develop`

On Linux you also need the native GL/X11 dev packages (see
`.github/workflows/build-native.yml` for the exact `apt` list).

## Building the CLI

One-time, install the Fable local tool:

```sh
dotnet tool restore
```

Then build the CLI. **Order matters:** the CLI embeds the web runtime bundle at compile
time (via `include_bytes!`), so the wasm bundle must exist before the `functor` binary is built.

```sh
cargo build --bin functor-runner                            # desktop runtime
wasm-pack build runtime/functor-runtime-web --target=web     # web bundle (embedded into the CLI)
cargo build --bin functor                                   # the CLI itself
```

Or use the bundled convenience script, which runs all three in order:

```sh
npm run build:cli
```

This produces two binaries in `target/debug/`: `functor` (the CLI) and `functor-runner`
(the desktop runtime). The CLI looks for `functor-runner` next to itself, so keep them
together.

## Running the sample (`examples/hello`)

First, fetch the sample's model assets (they aren't checked in; they download from
[BabylonJS Assets](https://github.com/BabylonJS/Assets/)):

```sh
npm run fetch:assets
```

The CLI operates on a directory containing a `functor.json`. Point it at the example with
`-d`. The `run` command transpiles the F#, compiles the game, and launches it:

```sh
# Native — opens a window
./target/debug/functor -d examples/hello run native

# Web — builds the wasm bundle, serves it, and opens http://127.0.0.1:8080
./target/debug/functor -d examples/hello run wasm
```

`native` is the default environment, so `... run` is equivalent to `... run native`.

### CLI commands

| Command | Description |
| --- | --- |
| `functor -d <dir> build [native\|wasm]` | Transpile F# → Rust and compile the game |
| `functor -d <dir> run [native\|wasm]` | Build, then run (native window / browser) |
| `functor -d <dir> develop [native\|wasm]` | Hot-reload dev loop (requires `watchexec`) |

### What `build`/`run` do under the hood

1. `npm run build:examples:hello:rust` — Fable transpiles `examples/hello/hello.fs` to
   Rust (`examples/hello/hello.rs`) and generates `fable_modules/`.
2. `cargo build` in `examples/hello/build-native` (native) or
   `wasm-pack build` in `examples/hello/build-wasm` (wasm) — compiles the game.
3. (native) `functor-runner` loads the resulting `libgame_native` dylib;
   (wasm) a dev server serves the bundle to the browser.

## Credits

- Demo assets are from [BabylonJS Assets](https://github.com/BabylonJS/Assets/)
