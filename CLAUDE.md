# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Functor is a functional toolkit for building 3D games in **Functor Lang** — Functor's own interpreted,
F#-inspired game-logic language (roadmap and design: `docs/functor-lang.md`; syntax/semantics source of
truth: the **`functor-lang` skill**, `.claude/skills/functor-lang/`). You write a game as a `.fun`
file. There is **no transpile or compile step for game logic**: the Rust runtime *interprets* the
`.fun` directly, on one of two targets:

- **native** — the desktop runtime (GLFW + OpenGL), now built into the single `functor` binary
  (the desktop crate is a library the CLI drives in-process), loads and runs your `.fun`,
  with hot-reloading on save.
- **wasm** — the web runtime (WebGL2) ships the `.fun` source as text and interprets it in the
  browser.

The same tree-walking interpreter runs everywhere Rust runs. (Functor formerly used F# + Fable →
Rust; that pipeline was deleted in roadmap **E3** — see `docs/functor-lang.md`.)

## Design principles

These shape how features should be built. Weigh changes against them.

1. **Functional-core, imperative shell.** Push as much logic as possible into the pure functional
   core (the Functor Lang game logic: `init`/`update`/`tick`/`draw` are pure functions of `model`). Keep
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

## Pull requests

Prefix every PR title with a Conventional Commits-style type, optionally followed by a scope:
`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `perf:`, `build:`, `ci:`, or `chore:`. For example,
`fix: preserve the timeline across hot reloads` or `feat(web): add timeline event markers`.

Open PRs as **drafts** (`gh pr create --draft`) and mark them ready (`gh pr ready <N>`) only
once the PR is actually complete: verification re-run after any review fixes, review findings
dispositioned in the body, and — for visual changes — the pr-visuals GIF/PNG embedded. Draft
status is the signal that captures/review are still landing, so an in-flight PR doesn't get
merged before its media and dispositions do.

## Architecture

**The MVU loop (Elm-style).** A game is a set of top-level Functor Lang bindings the runner looks up by
name (contract in the `functor-lang` skill; reference: `examples/hello/game.fun`):

- `init` — the initial model, a plain Functor Lang value
- `input = (model, key, isDown) => model'` — OPTIONAL; keyboard events, keys as the built-in
  `Key` module's variants (`Key.W`, `Key.Up`, `Key.Space`, `Key.Num0`..`Key.Num9`).
  `mouseMove`/`mouseWheel` are the analogous optional entry points
- `sampledInput = (model, snapshot: Input.snapshot) => model'` — OPTIONAL; per-fixed-step
  held/device state. `snapshot` has keyboard/mouse plus typed device domains (`xr` first;
  gamepad and mobile touch extend it as siblings)
- `tick = (model, dt, tts) => model'` — per-frame simulation step
- `update = (model, msg) => model'` — OPTIONAL; handles messages (ADT variants) from subscriptions/effects
- `subscriptions = (model) => Sub.every(...)` — OPTIONAL declarative timers, polled each frame (requires `update`)
- `draw = (model, tts) => Frame.create(camera, scene)` — pure frame description: a `Camera` plus a scene
- `physics = (model) => Physics.scene(...)`, `soundScape = (model) => AudioScene.create(...)`,
  `ui = (model) => …` — OPTIONAL hooks

The model-updating entry points (`tick`, `input`, `sampledInput`, `mouseMove`, `mouseWheel`, `update`) may return
a `(model', effect)` tuple instead of a bare model, whose effect result folds back through
`update`. `init` is a plain value (an Effect in it is rejected at load); `draw`/`physics`/
`soundScape`/`ui`/`subscriptions` return their own specific values. The model is a plain
`functor_lang::Value` the host holds between frames.

**Coordinates: Y-up, right-handed** (like OpenGL / glTF / Unity / Godot; *not* Unreal's Z-up). +Y
is up, +X is right, and the ground is the XZ plane. The camera's up is `[0,1,0]` and view uses
`look_at_rh`; `Camera.firstPerson` treats yaw = 0 / pitch = 0 as looking down **+Z**, with positive
pitch looking up. glTF models are authored Y-up, so this matches imported assets with no conversion.
By convention, `plane` geometry lies in XZ (ground) and `quad` in XY (screen/wall-facing).

**The effect broker drains to a fixed point.** Each frame the producer folds subscription
messages and effect results through `update`, **draining the effect queue to a fixed point**
(capped at 1000 effects/frame to avoid hangs) before running `tick` on the settled state. The
frame order is `sampledInput → subscriptions → update → tick → physics → draw`. Sampled
input is recorded as plain data for deterministic forward replay. This machinery is shared,
prelude-level Rust in `functor_runtime_common::functor_lang_prelude` (`drain_effects`, an `EffectRunner`:
`RealEffects` / `FakeEffects` / `ReplayEffects`), consumed by both producers — every performed
effect lands in a structured log, so under a fake/replay runner the same program is exactly
deterministic (the test seam).

**The Functor Lang producer is the seam between game logic and the shells.** `functor_lang_game.rs` (desktop) and
its wasm sibling in `runtime/functor-runtime-web/` run `.fun` logic through an `functor_lang::Session` with
the **Functor prelude** (`FunctorHost` in `functor_runtime_common::functor_lang_prelude`): the host-provided
externals that make `Scene.*` / `Camera.*` / `Frame.*` / `Light.*` / `Physics.*` / `Effect.*` /
`Sub.*` resolve to real protocol values. Both producers implement the shared
`functor_runtime_common::protocol::GameProducer` trait the runtime loop consumes; the versioned
logic↔runtime boundary is enumerated in `functor_runtime_common::protocol`. When you add or change
a prelude surface, the real implementation lives in `functor_lang_prelude.rs` and both producers must wire it —
prefer REGISTERING it in the typed external registry (`host_registry.rs` + `register_*` in
`functor_lang_prelude.rs`: one typed closure, arity/usage/conversion errors derived) over adding a
legacy `match path` arm; a drift test pins `.funi` signatures ≡ (registry ∪ legacy `PATHS`).

**Hot-reload and state persistence.** Functor Lang hot-reload is built into the producer: it polls the
project files' mtime each frame and on change reparses → rechecks → builds a new `Session` with
**the model preserved** (it is a plain value the host holds). Closures stored *inside* the model
rebind to the edited code, carrying their captured values over (matched by the enclosing def's
name; a renamed/deleted def keeps its old body with a loud `[functor-lang]` warning). A broken edit prints
once and keeps the old program running. The physics world (like the model) survives reload. Pending
effects are reset on reload (an in-flight HTTP tagger would dangle). Native watches every project
`.fun`; on wasm, hot-reload is native-only (reload the page, or push source via a
`{ type: "functor-lang-set-source", source }` postMessage). See the `functor-lang` skill for the exact rules.

### Layout

| Path | What it is |
| --- | --- |
| `functor-lang/` | The Functor Lang language crate — parser, IR, interpreter (`Session`), typechecker; `functor-lang parse/ir/run/trace/check` |
| `runtime/functor-runtime-common/` | Shared Rust runtime: rendering, assets, geometry, materials, the effect broker, and the Functor Lang prelude (`functor_lang_prelude::FunctorHost`) |
| `runtime/functor-runtime-desktop/` | Desktop runtime (GLFW/OpenGL) — a library the `functor` CLI drives in-process (no separate binary); the native Functor Lang producer (`functor_lang_game.rs`) |
| `runtime/functor-runtime-web/` | Web runtime (WebGL2) → wasm bundle; the wasm Functor Lang producer (`functor_lang_game.rs`) |
| `cli/` | The `functor` CLI (`build`/`run`/`develop`/`init`); Functor Lang projects route through `cli/src/commands/functor_lang_project.rs` |
| `tools/` | Editor tooling: `functor-lang-vscode` (extension), `functor-lang-lsp` (language server), `functor-sdk` (TS debug-runtime SDK) |
| `examples/*/` | Sample games (each a dir with `functor.json` + `game.fun`) — e.g. `hello`, `primitives`, `lighting`, `physics` |

Functor Lang files use `file = module`: every sibling `.fun` in the entry's directory loads with it. The
language, prelude, and semantics are documented in the **`functor-lang` skill** — treat it and
`docs/functor-lang.md` as the source of truth, and keep the skill in sync when you change the language.

## Commands

**Prerequisites:** Rust stable + `wasm32-unknown-unknown` target, Node 22 / npm 10, and
`wasm-pack`.

**Build the CLI.** Order matters — the CLI embeds the web runtime bundle at compile time via
`include_bytes!`, so the wasm bundle must exist before the `functor` binary is built:

```sh
npm run build:cli           # the wasm bundle, then the functor CLI (RELEASE) → target/release/functor
npm run build:cli:debug     # same, but a debug build → target/debug/functor
```

(The human-facing docs mirror this split: `README.md` covers installing and using
Functor; `DEVELOPMENT.md` covers building the CLI from source.)

Produces `target/release/functor` (or `target/debug/functor` via `build:cli:debug`) — a single binary with the
desktop runtime built in. **Use the RELEASE binary for interactive native testing** — the
debug build is fine for headless checks and captures, but its CPU-bound paths are unusably
slow live: the webview overlay's CPU raster is ~200× slower (~0.6s per repaint at a retina
window — feels like a hang), and rapier physics steps are similarly far off release speed.
The wasm bundle is unaffected either way: `wasm-pack build` is release by default, so
`run wasm` and the site always ship optimized wasm.

**Generate the API reference.** The lightweight generator reads the exact `.funi`
prelude embedded in Functor and recreates gitignored local Markdown + JSON artifacts;
it does not build the GL-linked CLI or the wasm runtime. The check command validates
both renderers without requiring generated files to exist. Generation and checking
fails if a prelude module, type, or signature lacks explicit `//!` / `///` public
documentation:

```sh
npm run generate:docs
npm run check:docs
```

The released CLI exposes the same generator as `functor docs` (Markdown to stdout
by default; `--format json`, `--output <path>`, and `--check <path>` are available).

**Scaffold a game.** `init` creates an embedded Functor Lang starter without overwriting an existing
`functor.json` or `game.fun`; `3d` is the default template:

```sh
./target/debug/functor -d my-game init [3d|fps]
```

**Run / build a game.** The CLI operates on a directory with a `functor.json`
(`{"language": "functor-lang", "entry": "game.fun"}`); `-d` points to it:

```sh
./target/debug/functor -d examples/primitives run native   # opens a window (native is the default env)
./target/debug/functor -d examples/primitives run wasm      # serves the .fun + wasm at http://127.0.0.1:8080
./target/debug/functor -d examples/primitives run vr        # runs it on an adb-attached headset (see runtime/functor-runtime-oculus/README.md), re-pushing on save
./target/debug/functor -d examples/primitives build [native|wasm]
./target/debug/functor -d examples/primitives develop [native|wasm]   # = run; Functor Lang hot-reload is built in
```

For Quest build/install, XR-session recovery, live push, raw stereo capture, and
on-device benchmarking, use the **`vr-device-loop` skill**
(`.claude/skills/vr-device-loop/`). In particular, recreate `adb forward` after
every reconnect and wait for `/state`'s frame to advance before capturing; the
debug server remains reachable while a dozing XR session correctly returns 503
for `/capture`.

Instead of one `entry`, a project may declare named **`entries`**
(`{"entries": {"client": "client.fun", "server": "server.fun"}}`) — roles sharing one
directory of sibling modules (file = module). `--entry <name>` picks the role
(default: `client`, or the sole entry), anywhere on the line:
`functor -d examples/mp run native --entry server`. `examples/mp` is the reference —
client + authoritative server over a shared `protocol.fun`.

Under the hood: `build` typechecks the whole `.fun` project (diagnostics are errors) and
**verifies every literal `Asset.*` locator**: a relative path must exist on disk (error — with
the fetch/reimport hints), a URL verifies via the remote disk cache then a HEAD request
(provably-404 = error; offline/unverifiable = warning, so offline builds stay usable). Bare
strings at asset consumers are check-time errors since the flag day (B.6). `build wasm`
then also exports a **self-contained static web bundle** to `<project>/dist/web` — the rendered
host page + the embedded web runtime + a copy of the project directory (hidden files and `dist/`
excluded), with a warn-only lint for string-literal asset references that won't be in the bundle.
Zip that folder for itch.io (HTML5) or serve it from any static host. `run native`
drives the built-in desktop runtime in-process from the game dir; it **interprets** the
`.fun` each frame — nothing compiles. `run wasm` serves the project directory: the `.fun` ships as
text and is interpreted by the embedded web runtime. `develop` is `run` (hot-reload is built in; on
wasm, reload the page).

**Generate the typed asset manifest.** `import` scans the project dir's assets — models
(`*.glb`/`*.gltf`), textures (`*.png`/`*.jpg`/`*.jpeg`/`*.hdr`), sounds (`*.wav`/`*.ogg`/`*.mp3`;
non-recursive, so `golden/` subdirs are excluded) — inspecting models headlessly (no GPU), and
writes a generated `assets.fun` module of branded constants: `let xbot = Asset.model("Xbot.glb")`
plus per-model clip records (`Assets.xbotClips.walk.name` / `.duration`). Games write
`Scene.model(Assets.xbot)` and `Anim.clip(Assets.xbotClips.walk.name, tts)` — a typo in either is
a check-time error (a bare-string typo silently renders the fallback/bind pose). Check the
generated file in: it typechecks without the gitignored models. `run`/`build` **auto-regenerate**
it when assets are added or change (mtime/set check against the header's `// files:` inventory) —
but never when listed assets are merely missing from disk (unfetched clones/CI must not lose
constants), so deleting or renaming an asset needs an explicit rerun:

```sh
./target/debug/functor -d examples/animation import   # writes examples/animation/assets.fun
```

The generator core is shared Rust (`functor_runtime_common::manifest`, IO-free) so future
tooling (the browser IDE's wasm build) emits byte-identical manifests; the CLI command owns
scanning, model inspection, and file IO.

**The flag day (B.6) has landed**: asset consumers (`Scene.model`, `Effect.play/playAt/
playThen`, `AudioSource.ambient/at` sound args) take branded `Asset` values ONLY — the
generated manifest's `Assets.*`, or `Asset.model/texture/sound(…)` at a data boundary. A bare
path string is a check-time error and a runtime teaching error. (`Texture.file`, `Skybox.files`,
`Anim.clip` names, and AudioSource KEYS are not asset coercion and keep their strings.)

**Remote (CDN) assets** are declared with a **sidecar**: `<name>.asset.json` containing
`{ "url": "https://…", "kind"?: "model"|"texture"|"sound" }` (`kind` only when the url's
extension can't infer it; unknown keys warn). The manifest then carries the URL locator
(`let shark = Asset.model("https://…/shark.glb")`), and `import` inspects remote models for
clips by fetching ONCE through the same content-addressed disk cache the runtime uses
(`~/.functor/cache`, `FUNCTOR_ASSET_CACHE` override) — so import warms exactly the entry the
game later loads, and reruns are offline-safe. A sidecar named after a local file is that
asset's (future) per-asset config seat; if both declare (`url` + local file), the local file
wins with a warning. Auto-reimport watches sidecar mtimes too, and a failed remote fetch
during auto-reimport keeps the existing manifest (offline must not strip clip constants).

**Verify the language without a GPU:** `cargo run -q -p functor-lang -- run|check|trace|test|parse|ir <file.fun>`
drives the interpreter/typechecker headlessly (the plain-`functor-lang` prelude, no engine host). See the
`functor-lang` skill.

**Capture a frame to PNG** (no OS screen-recording permission needed — the runner reads back its
own framebuffer; ideal for verifying rendering changes). The CLI forwards extra args to
the built-in desktop runtime (a leading `--` is optional):

```sh
./target/debug/functor -d examples/primitives run native \
  --capture-frame /tmp/frame.png --capture-time 3        # capture after 3s of wall-clock, then exit
```

Add `--fixed-time T` to pin the game's frame time to a constant `T`, making the rendered pose
deterministic (byte-identical PNGs) for reproducible captures and golden images.

`--capture-frame` implies `--hidden`: the GL window is created invisible and never takes focus
or captures the cursor, so capture runs don't steal input from the user. For debug-server
sessions (`--debug-port`) prefer passing `--hidden` explicitly — or `--headless` when no pixels
are needed at all (see `docs/debug-runtime.md`).

**Golden-image test:** `npm run test:golden` renders the Functor Lang samples (`hello`,
`lighting`, `primitives`, `synthwave` — the scenarios in `golden-scenarios.json`) at a
fixed time and compares each capture to a committed reference
(`runtime/functor-runtime-desktop/tests/golden.rs`). It's `#[ignore]`d (needs a GL display), so it
runs locally/manually, not in CI. Goldens are renderer/display-specific — the regeneration command
is in the test's doc comment.

**Tests** are Rust: the runtime in `functor-runtime-common`
(`cargo test -p functor_runtime_common`, includes the Functor Lang prelude) and the language in the `functor_lang`
crate (`cargo test -p functor-lang`; `UPDATE_GOLDENS=1` regenerates its snapshots).

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

## The marketing site (`site/`)

When working on the marketing site (`site/` — the landing page, sandbox, and docs;
static HTML/CSS/JS built by `site/build.mjs`, served with `npm run site:serve`),
**use the `frontend-design` skill** for any visual/layout/copy work. It calibrates the
design toward a distinctive, intentional aesthetic and away from generic defaults —
load it before editing markup or styles. The site has an established look (synthwave:
dark-violet base, cyan primary + pink accent, Orbitron display / JetBrains Mono body,
a shared spacing scale in `:root`); extend that system rather than inventing a new one,
and verify changes by building and screenshotting (headless Chrome via Playwright works)
across desktop **and** mobile widths.

**Site demo GIFs** (the feature showcases on the landing page) are generated by
**reproducible scripts in `site/demos/`** — e.g. `npm run demo:time-travel` builds the
site, serves it, drives the web player through its `window.__scrub` time-travel seam
(`runtime/functor-runtime-web/scrubber.js`), and renders a looping GIF with ffmpeg. Add
new feature captures there as committed scripts (never one-off ad-hoc captures) so any
GIF can be regenerated deterministically. They need the web runtime wasm bundle,
`@playwright/test`'s chromium, and `ffmpeg` on PATH.

## Performance-sensitive changes

Whenever a change touches a **per-frame hot path** — the interpreter/evaluator
(`functor-lang` core), the prelude/host bridge (`functor_lang_prelude.rs`,
`host_registry.rs`), the producers, or rendering internals — run the benchmarks
**before and after** (base ref vs. change, release builds, back-to-back on the
same machine) and include the comparison in the PR. This is part of the
definition of done for perf-sensitive work:

```sh
cargo run -q --release -p functor_runtime_common --example frame_bench
```

`frame_bench` is the honest macro number: a synthwave-shaped frame under the
REAL engine prelude, headless. Compare **`allocs/frame` and `bytes/frame`
first** — they are deterministic, so any delta is real; a regression there
needs a justification in the PR. Wall-clock `us/frame` is DVFS/load-noisy
(same-commit runs vary a few percent) — run each side 2–3×, compare the
**min** column, and treat deltas inside run-to-run spread as noise. Never
benchmark a debug build (the harness warns), and never use the windowed
runtime's `draw_us` telemetry for perf claims (it inflates ~2× on
sub-saturated scenes).

For language-only changes, `functor-lang/benches` (see its README) isolates
interpreter micro-ops under the plain prelude — useful for pinpointing, but a
frame_bench run is still the acceptance number: micro-derived estimates have
misjudged real per-frame cost before.

For changes to the **native webview overlay** (blitz parse/resolve/paint —
`webview_overlay.rs` or its blitz dependency pins), the analogous number is:

```sh
cargo run -q --release -p functor-runtime-desktop --example webview_bench
```

It times per-re-render parse+resolve (shared FontContext — a fresh one
re-scans system fonts, the ~30ms regression it exists to catch), per-repaint
CPU raster at three framebuffer sizes, and the press-time hit-test. Same
rules: release only, compare the **min** column.

## Gotchas

- **The `functor-lang` skill is the source of truth for Functor Lang.** Functor Lang is a small, custom language —
  do NOT guess syntax/semantics from F#/OCaml intuition (e.g. `if cond then a else b` exists
  as an EXPRESSION — both branches required, `else if` chains, no `elif` — alongside the
  equally-valid bool-literal `match`; assignment is `:=`; pipelines *append* the subject
  (thread-last: `x |> f(a)` == `f(a, x)`)).
  When a change touches the language or the prelude, update the skill in the same PR.
- **`file = module`.** Every `.fun` in the entry's directory loads with the project — an
  unreferenced (or stray scratch) sibling still parses, checks, and evaluates. Keep scratch `.fun`
  files in their own directory, and don't leave a broken sibling next to a game.
- **The engine prelude only exists under the host.** `Scene.*`/`Camera.*`/`Frame.*`/`Physics.*`
  etc. resolve only in runner-hosted Functor Lang (and tests via `functor_runtime_common::functor_lang_prelude`), NOT
  in a plain `cargo run -p functor-lang -- run`. Branded values (`Angle`, `Time`/`Duration`, `Fog`, render
  targets) refuse bare numbers/strings with a teaching error — pass `Angle.degrees(60.0)`, not `60`.
- **`functor run` does not rebuild the runtimes.** It only (re)loads the *game* `.fun`, which the
  runner interprets. The asset pipeline and rendering execute in the shells, which are prebuilt:
  natively in the desktop runtime built into the `functor` binary, and on wasm in the web-runtime
  bundle that is `include_bytes!`-embedded into the `functor` CLI. After changing `runtime/` crates (including the
  Functor Lang prelude), run `npm run build:cli` first or the running shell silently won't have your change.
- **Sample glTF assets vary wildly in units.** The demo assets come from
  [BabylonJS/Assets](https://github.com/BabylonJS/Assets/) (`meshes/*.glb`): `ExplodingBarrel.glb`
  is ~72 units tall, Mixamo-style humanoids (`Xbot.glb`) are centimeter scale, and `fish.glb` is an
  entire multi-fish scene — hence the per-model `Scene.scale` values in `examples/hello`.
  No models are checked in (`*.glb` is gitignored there); fetch them with `npm run fetch:assets`. A
  missing asset logs an error and renders as the fallback (empty) asset.
- `AGENTS.md` is a symlink to this file — edit `CLAUDE.md` only.
