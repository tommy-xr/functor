# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Functor is a functional toolkit for building 3D games in **F#**. You write a game against the
`Functor.Game` library; [Fable](https://fable.io/) transpiles the F# to **Rust**, which is then
compiled to one of two targets:

- **native** — a dynamic library (`libgame_native` dylib) loaded by the `functor-runner` desktop
  runtime (GLFW + OpenGL), with hot-reloading.
- **wasm** — a WebAssembly bundle (`wasm-pack`) served to the browser (WebGL2).

## Design principles

These shape how features should be built. Weigh changes against them.

1. **Functional-core, imperative shell.** Push as much logic as possible into the pure functional
   core (the F# game framework and game code: `init`/`update`/`tick`/`draw3d` are pure functions
   of `model`). Keep side effects (rendering, GLFW/window, file IO, the dylib/wasm boundary) in the
   thin imperative shell — the Rust runtimes under `runtime/`.
2. **LLM-native.** Functor functionality must be introspectable by LLMs at runtime — favor live
   evaluation, a text-only runtime path, and serializable/inspectable state over opaque binary
   state. When adding runtime capabilities, preserve the ability to drive and observe the game
   without a GPU window.
3. **Simplicity and incrementality.** Prefer small, incremental PRs; use stacked PRs where
   applicable. Recent history (see `git log`) is a series of tightly scoped changes — match that.
4. **Fast inner loop.** Iterating and experimenting must be extremely fast for both humans and
   LLMs. Protect hot-reload, keep build steps minimal, and don't regress dev-loop latency.

## Architecture

**The MVU loop (Elm-style).** A game is a `Game<'model, 'msg>` record (`src/Functor.Game/Game.fs`)
built fluently via `GameBuilder` and handed to `Runtime.runGame`:

- `initialState: 'model`
- `init  : effect<'msg>` — startup effect, seeded into the queue at construction so it drains
  before the first frame; **not** re-run across a hot reload (the persisted queue is restored)
- `input : 'model -> Input.t -> 'model * effect<'msg>` — keyboard/mouse events, applied immediately
- `tick  : 'model -> FrameTime -> 'model * effect<'msg>` — per-frame simulation step
- `update: 'model -> 'msg -> 'model * effect<'msg>` — handles messages produced by effects
- `subscriptions: 'model -> Sub<'msg>` — declarative timers (`Sub.every`), polled each frame
- `draw3d: 'model -> FrameTime -> Graphics.Frame` — pure frame description: a `Camera` plus a
  `Scene3D` (`Frame.create camera scene`)

`Runtime.GameExecutor` (`src/Functor.Game/Runtime.fs`) is the heart of the loop: each frame it
**drains the effect queue to a fixed point** (capped at `maxEffectsPerFrame = 1000` to avoid
hangs), feeding messages through `update` and re-enqueueing new effects, before running `tick` on
the settled state. The exact per-frame ordering (subscriptions are polled between drains) is
documented by comments in the executor itself.

**The F#→Rust boundary is thin bindings, not reimplementations.** Many `Functor.Game` types are
`[<Erase; Emit(...)>]` shims over Rust runtime types. For example `EffectQueue.fs` and the
`effect` type are F# facades over `functor_runtime_common::EffectQueue` / effect types in
`runtime/functor-runtime-common/src/`. When you change behavior, the real implementation usually
lives in the Rust runtime, with an F# `Emit` binding mirroring its signature — keep the two in sync.

**Hot-reload and state persistence.** The desktop runtime can run a game statically
(`static_game.rs`) or hot-reloaded (`hot_reload_game.rs`, via the `--hot` flag). Across a reload,
`getState`/`setState` (the `emit_state`/`set_state` `no_mangle` exports in `Runtime.fs`) bundle
**both the model and the pending effect queue** into an `OpaqueState` so in-flight effects survive
the reload. Preserve this contract when touching executor state.

**Two runtime exports.** `Runtime.Native` exposes `no_mangle` functions for the dylib;
`Runtime.Wasm` exposes `wasm_bindgen` equivalents (`_wasm` suffix, marshalling through JsValue;
no state pair — hot-reload is native-only). New runtime entry points generally need a parallel
native + wasm pair — see both modules in `Runtime.fs` for the current list.

### Layout

| Path | What it is |
| --- | --- |
| `src/Functor.Game/` | The F# game framework (`Game`, `Runtime`, `Scene3D`/`Graphics`, `Frame`, `Camera`, `Input`, `Effect`, `EffectQueue`, `Sub`, `Math`, `Time`) |
| `src/` | `functor-lib` — Rust crate Fable emits from `Functor.Game` (lib path is the generated `Platform.rs`) |
| `runtime/functor-runtime-common/` | Shared Rust runtime: rendering, assets, geometry, materials, `Effect`/`EffectQueue` |
| `runtime/functor-runtime-desktop/` | Desktop runtime → the `functor-runner` binary (GLFW/OpenGL) |
| `runtime/functor-runtime-web/` | Web runtime (WebGL2) → wasm bundle |
| `cli/` | The `functor` CLI (`build`/`run`/`develop`/`init`) |
| `examples/hello/` | Sample game (`hello.fs` — Pong-style scene with a WASD + mouse free-look camera); `build-native/` and `build-wasm/` are the per-target compile crates |

The `.fs`/`.fsi` pairs in `src/Functor.Game/` use the `.fsi` signature file as the public API — update both when changing a module's surface.

## Commands

**Prerequisites:** .NET SDK 8, Rust stable + `wasm32-unknown-unknown` target, Node 22 / npm 10,
`wasm-pack`, and `watchexec` (only for `develop`). Fable runs as a local dotnet tool (`fable 4.17.0`).

```sh
dotnet tool restore           # one-time: install the Fable local tool
```

**Build the CLI.** Order matters — the CLI embeds the web runtime bundle at compile time via
`include_bytes!`, so the wasm bundle must exist before the `functor` binary is built:

```sh
npm run build:cli   # functor-runner, then the wasm bundle, then the functor CLI
```

Produces `target/debug/functor` (CLI) and `target/debug/functor-runner` (desktop runtime). The CLI
looks for `functor-runner` next to itself — keep them together.

**Run / build a game.** The CLI operates on a directory containing a `functor.json` (`-d` points to it):

```sh
./target/debug/functor -d examples/hello run native   # opens a window (native is the default env)
./target/debug/functor -d examples/hello run wasm      # serves wasm at http://127.0.0.1:8080
./target/debug/functor -d examples/hello build [native|wasm]
./target/debug/functor -d examples/hello develop [native|wasm]   # hot-reload loop (needs watchexec)
```

Under the hood, `build`/`run`: (1) Fable transpiles `hello.fs` → `hello.rs` + `fable_modules/`;
(2) `cargo build` in `build-native/` or `wasm-pack build` in `build-wasm/`; (3) native loads the
dylib via `functor-runner`, wasm serves the bundle.

**Transpile F# only:** `npm run build:examples:hello:rust`
(`dotnet fable examples/hello/hello.fsproj --lang rust --outDir .`).

**Tests** are Rust, in `functor-runtime-common`, with test modules alongside source:
`cargo test -p functor_runtime_common`.

## Gotchas

- **uuid / wasm:** the generated `fable_library_rust` depends on `uuid`. It's constrained to
  `>=1.8, <1.12` in `runtime/functor-runtime-common/Cargo.toml` (shared by every workspace);
  without this, `wasm-pack build` fails with `could not find RngImp in imp` because newer uuid pulls
  `getrandom 0.3+` which needs an explicit RNG backend for `wasm32-unknown-unknown`. If hit, confirm
  the constraint or pin with `cargo update -p uuid --precise 1.11.1`.
- `examples/hello/build-native` and `build-wasm` are **separate Cargo workspaces** (`[workspace]`)
  that compile the generated `../hello.rs` as a `dylib`/`cdylib` respectively; they are not part of
  the root workspace.
- `cli/src/main.rs`'s `Init` command is currently a TODO stub.
- `AGENTS.md` is a symlink to this file — edit `CLAUDE.md` only.
