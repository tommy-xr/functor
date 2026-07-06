# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Functor is a functional toolkit for building 3D games in **MLE** — Functor's own interpreted,
F#-inspired game-logic language (roadmap and design: `docs/mle.md`; syntax/semantics source of
truth: the **`mle-language` skill**, `.claude/skills/mle-language/`). You write a game as a `.mle`
file. There is **no transpile or compile step for game logic**: the Rust runtime *interprets* the
`.mle` directly, on one of two targets:

- **native** — the `functor-runner` desktop runtime (GLFW + OpenGL) loads and runs your `.mle`,
  with hot-reloading on save.
- **wasm** — the web runtime (WebGL2) ships the `.mle` source as text and interprets it in the
  browser.

The same tree-walking interpreter runs everywhere Rust runs. (Functor formerly used F# + Fable →
Rust; that pipeline was deleted in roadmap **E3** — see `docs/mle.md`.)

## Design principles

These shape how features should be built. Weigh changes against them.

1. **Functional-core, imperative shell.** Push as much logic as possible into the pure functional
   core (the MLE game logic: `init`/`update`/`tick`/`draw` are pure functions of `model`). Keep
   side effects (rendering, GLFW/window, file IO, the interpreter/wasm boundary) in the thin
   imperative shell — the Rust runtimes under `runtime/`.
2. **LLM-native.** Functor functionality must be introspectable by LLMs at runtime — favor live
   evaluation, a text-only runtime path, and serializable/inspectable state over opaque binary
   state. When adding runtime capabilities, preserve the ability to drive and observe the game
   without a GPU window.
3. **Simplicity and incrementality.** Prefer small, incremental PRs; use stacked PRs where
   applicable. Recent history (see `git log`) is a series of tightly scoped changes — match that.
4. **Fast inner loop.** Iterating and experimenting must be extremely fast for both humans and
   LLMs. Protect hot-reload, keep build steps minimal, and don't regress dev-loop latency.

## Architecture

**The MVU loop (Elm-style).** A game is a set of top-level MLE bindings the runner looks up by
name (contract in the `mle-language` skill; reference: `examples/mle-hello-gltf/game.mle`):

- `init` — the initial model, a plain MLE value
- `input = (model, key, isDown) => model'` — OPTIONAL; keyboard events, keys as canonical names
  ("W", "Up", "Space"). `mouseMove`/`mouseWheel` are the analogous optional entry points
- `tick = (model, dt, tts) => model'` — per-frame simulation step
- `update = (model, msg) => model'` — OPTIONAL; handles messages (ADT variants) from subscriptions/effects
- `subscriptions = (model) => Sub.every(...)` — OPTIONAL declarative timers, polled each frame (requires `update`)
- `draw = (model, tts) => Frame.create(camera, scene)` — pure frame description: a `Camera` plus a scene
- `physics = (model) => Physics.scene(...)`, `soundScape = (model) => AudioScene.create(...)`,
  `ui = (model) => …` — OPTIONAL hooks

The model-updating entry points (`tick`, `input`, `mouseMove`, `mouseWheel`, `update`) may return
a `(model', effect)` tuple instead of a bare model, whose effect result folds back through
`update`. `init` is a plain value (an Effect in it is rejected at load); `draw`/`physics`/
`soundScape`/`ui`/`subscriptions` return their own specific values. The model is a plain
`mle::Value` the host holds between frames.

**Coordinates: Y-up, right-handed** (like OpenGL / glTF / Unity / Godot; *not* Unreal's Z-up). +Y
is up, +X is right, and the ground is the XZ plane. The camera's up is `[0,1,0]` and view uses
`look_at_rh`; `Camera.firstPerson` treats yaw = 0 / pitch = 0 as looking down **+Z**, with positive
pitch looking up. glTF models are authored Y-up, so this matches imported assets with no conversion.
By convention, `plane` geometry lies in XZ (ground) and `quad` in XY (screen/wall-facing).

**The effect broker drains to a fixed point.** Each frame the producer folds subscription
messages and effect results through `update`, **draining the effect queue to a fixed point**
(capped at 1000 effects/frame to avoid hangs) before running `tick` on the settled state. The
frame order is `subscriptions → update → tick → physics → draw`. This machinery is shared,
prelude-level Rust in `functor_runtime_common::mle_prelude` (`drain_effects`, an `EffectRunner`:
`RealEffects` / `FakeEffects` / `ReplayEffects`), consumed by both producers — every performed
effect lands in a structured log, so under a fake/replay runner the same program is exactly
deterministic (the test seam).

**The MLE producer is the seam between game logic and the shells.** `mle_game.rs` (desktop) and
its wasm sibling in `runtime/functor-runtime-web/` run `.mle` logic through an `mle::Session` with
the **Functor prelude** (`FunctorHost` in `functor_runtime_common::mle_prelude`): the host-provided
externals that make `Scene.*` / `Camera.*` / `Frame.*` / `Light.*` / `Physics.*` / `Effect.*` /
`Sub.*` resolve to real protocol values. Both producers implement the shared
`functor_runtime_common::protocol::GameProducer` trait the runtime loop consumes; the versioned
logic↔runtime boundary is enumerated in `functor_runtime_common::protocol`. When you add or change
a prelude surface, the real implementation lives in `mle_prelude.rs` and both producers must wire it.

**Hot-reload and state persistence.** MLE hot-reload is built into the producer: it polls the
project files' mtime each frame and on change reparses → rechecks → builds a new `Session` with
**the model preserved** (it is a plain value the host holds). Closures stored *inside* the model
rebind to the edited code, carrying their captured values over (matched by the enclosing def's
name; a renamed/deleted def keeps its old body with a loud `[mle]` warning). A broken edit prints
once and keeps the old program running. The physics world (like the model) survives reload. Pending
effects are reset on reload (an in-flight HTTP tagger would dangle). Native watches every project
`.mle`; on wasm, hot-reload is native-only (reload the page, or push source via a
`{ type: "mle-set-source", source }` postMessage). See the `mle-language` skill for the exact rules.

### Layout

| Path | What it is |
| --- | --- |
| `mle/` | The MLE language crate — parser, IR, interpreter (`Session`), typechecker; `mle parse/ir/run/trace/check` |
| `runtime/functor-runtime-common/` | Shared Rust runtime: rendering, assets, geometry, materials, the effect broker, and the MLE prelude (`mle_prelude::FunctorHost`) |
| `runtime/functor-runtime-desktop/` | Desktop runtime → the `functor-runner` binary (GLFW/OpenGL); the native MLE producer (`mle_game.rs`) |
| `runtime/functor-runtime-web/` | Web runtime (WebGL2) → wasm bundle; the wasm MLE producer (`mle_game.rs`) |
| `cli/` | The `functor` CLI (`build`/`run`/`develop`/`init`); MLE projects route through `cli/src/commands/mle_project.rs` |
| `tools/` | Editor tooling: `mle-vscode` (extension), `mle-lsp` (language server), `functor-sdk` (TS debug-runtime SDK) |
| `examples/mle-*/` | Sample games (each a dir with `functor.json` + `game.mle`) — e.g. `mle-hello-gltf`, `mle-primitives`, `mle-lighting`, `mle-physics` |
| `docs/todo.md` | The backlog — incomplete work only |

MLE files use `file = module`: every sibling `.mle` in the entry's directory loads with it. The
language, prelude, and semantics are documented in the **`mle-language` skill** — treat it and
`docs/mle.md` as the source of truth, and keep the skill in sync when you change the language.

## Commands

**Prerequisites:** Rust stable + `wasm32-unknown-unknown` target, Node 22 / npm 10, and
`wasm-pack`. **No .NET / Fable** — the toolchain is Rust + Node only. (`watchexec` is optional;
MLE hot-reload is built into the runtime, so `develop` does not need it.)

**Build the CLI.** Order matters — the CLI embeds the web runtime bundle at compile time via
`include_bytes!`, so the wasm bundle must exist before the `functor` binary is built:

```sh
npm run build:cli   # functor-runner, then the wasm bundle, then the functor CLI
```

Produces `target/debug/functor` (CLI) and `target/debug/functor-runner` (desktop runtime). The CLI
looks for `functor-runner` next to itself — keep them together.

**Run / build a game.** The CLI operates on a directory with a `functor.json`
(`{"language": "mle", "entry": "game.mle"}`); `-d` points to it:

```sh
./target/debug/functor -d examples/mle-primitives run native   # opens a window (native is the default env)
./target/debug/functor -d examples/mle-primitives run wasm      # serves the .mle + wasm at http://127.0.0.1:8080
./target/debug/functor -d examples/mle-primitives build [native|wasm]
./target/debug/functor -d examples/mle-primitives develop [native|wasm]   # = run; MLE hot-reload is built in
```

Under the hood: `build` typechecks the whole `.mle` project (diagnostics are errors). `run native`
spawns `functor-runner --mle --game-path <entry>` from the game dir; the runner **interprets** the
`.mle` each frame — nothing compiles. `run wasm` serves the project directory: the `.mle` ships as
text and is interpreted by the embedded web runtime. `develop` is `run` (hot-reload is built in; on
wasm, reload the page).

**Verify the language without a GPU:** `cargo run -q -p mle -- run|check|trace|parse|ir <file.mle>`
drives the interpreter/typechecker headlessly (the plain-`mle` prelude, no engine host). See the
`mle-language` skill.

**Capture a frame to PNG** (no OS screen-recording permission needed — the runner reads back its
own framebuffer; ideal for verifying rendering changes). The CLI forwards extra args to
`functor-runner` (a leading `--` is optional):

```sh
./target/debug/functor -d examples/mle-primitives run native \
  --capture-frame /tmp/frame.png --capture-time 3        # capture after 3s of wall-clock, then exit
```

Add `--fixed-time T` to pin the game's frame time to a constant `T`, making the rendered pose
deterministic (byte-identical PNGs) for reproducible captures and golden images.

`--capture-frame` implies `--hidden`: the GL window is created invisible and never takes focus
or captures the cursor, so capture runs don't steal input from the user. For debug-server
sessions (`--debug-port`) prefer passing `--hidden` explicitly — or `--headless` when no pixels
are needed at all (see `docs/debug-runtime.md`).

**Golden-image test:** `npm run test:golden` renders the MLE samples (`mle-hello-gltf`,
`mle-lighting`, `mle-primitives`, `mle-synthwave` — the scenarios in `golden-scenarios.json`) at a
fixed time and compares each capture to a committed reference
(`runtime/functor-runtime-desktop/tests/golden.rs`). It's `#[ignore]`d (needs a GL display), so it
runs locally/manually, not in CI. Goldens are renderer/display-specific — the regeneration command
is in the test's doc comment.

**Tests** are Rust: the runtime in `functor-runtime-common`
(`cargo test -p functor_runtime_common`, includes the MLE prelude) and the language in the `mle`
crate (`cargo test -p mle`; `UPDATE_GOLDENS=1` regenerates its snapshots).

## Visual changes

Whenever a change adds or alters something **visible** (a new example/scene, a
rendering/material/lighting/camera feature, a shader), capture a short looping
**GIF** *and* a still **PNG** of it and embed them in the PR — this is part of the
definition of done for visual work, so reviewers (human and LLM) can see the
result. When the change *modifies* an existing visual, include a **before/after**
too (capture the base ref at the same fixed time). Use the **`pr-visuals` skill**
(`.claude/skills/pr-visuals/`): it drives
the headless `--capture-frame` / `--fixed-time` path (no screen, deterministic),
assembles the GIF, hosts the binaries in a gist, and embeds them in the PR body —
and it runs the capture in a subagent so the image-heavy work stays out of the
main context.

## Gotchas

- **The `mle-language` skill is the source of truth for MLE.** MLE is a small, custom language —
  do NOT guess syntax/semantics from F#/OCaml intuition (e.g. there is no `if`/`else`; the
  conditional is a bool-literal `match`; assignment is `:=`; pipelines *prepend* the subject).
  When a change touches the language or the prelude, update the skill in the same PR.
- **`file = module`.** Every `.mle` in the entry's directory loads with the project — an
  unreferenced (or stray scratch) sibling still parses, checks, and evaluates. Keep scratch `.mle`
  files in their own directory, and don't leave a broken sibling next to a game.
- **The engine prelude only exists under the host.** `Scene.*`/`Camera.*`/`Frame.*`/`Physics.*`
  etc. resolve only in runner-hosted MLE (and tests via `functor_runtime_common::mle_prelude`), NOT
  in a plain `cargo run -p mle -- run`. Branded values (`Angle`, `Time`/`Duration`, `Fog`, render
  targets) refuse bare numbers/strings with a teaching error — pass `Angle.degrees(60.0)`, not `60`.
- `cli/src/main.rs`'s `Init` command is currently a TODO stub.
- **`functor run` does not rebuild the runtimes.** It only (re)loads the *game* `.mle`, which the
  runner interprets. The asset pipeline and rendering execute in the shells, which are prebuilt:
  natively in the `functor-runner` binary, and on wasm in the web-runtime bundle that is
  `include_bytes!`-embedded into the `functor` CLI. After changing `runtime/` crates (including the
  MLE prelude), run `npm run build:cli` first or the running shell silently won't have your change.
- **Sample glTF assets vary wildly in units.** The demo assets come from
  [BabylonJS/Assets](https://github.com/BabylonJS/Assets/) (`meshes/*.glb`): `ExplodingBarrel.glb`
  is ~72 units tall, Mixamo-style humanoids (`Xbot.glb`) are centimeter scale, and `fish.glb` is an
  entire multi-fish scene — hence the per-model `Scene.scale` values in `examples/mle-hello-gltf`.
  No models are checked in (`*.glb` is gitignored there); fetch them with `npm run fetch:assets`. A
  missing asset logs an error and renders as the fallback (empty) asset.
- `AGENTS.md` is a symlink to this file — edit `CLAUDE.md` only.
