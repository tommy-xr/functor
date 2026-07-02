# MLE: replacing F#/Fable with our own language

> Design + phased roadmap. Each step is independently verifiable and lands as its
> own small PR. The endgame is a complete replacement of the F#/Fable pipeline,
> but nothing here requires a flag-day — F# and MLE coexist behind one seam until
> MLE wins on every axis.

## Problem

Game logic is written in F# and compiled through **two stacked compilers**
(F# —Fable→ Rust —cargo/wasm-pack→ dylib | wasm). The pain:

- a logic-only edit pays a full Rust rebuild + dylib link before anything is visible
- the dotnet/Fable toolchain is flaky and sits on top of Rust + Node
- the dylib hot-reload path is fragile; `fable_modules/` caches need clearing
- `.fs`/`.fsi`/`.rs` triplication in `src/Functor.Game/`

The exploration: **MLE**, a custom F#-inspired functional language with a
Rust-hosted interpreter/VM, as Functor's first-class game-logic layer.

## Prior decisions (from the design notes)

Language design lives outside this repo — `~/notes/ideas/mle-language/` (start at
`README.md`) and `~/notes/projects/functor/bytecode-plan.md`. Already settled
there; not re-litigated here:

- **Rust-hosted interpreter first.** No JS engine, no native codegen early. One
  VM codebase runs everywhere Rust runs — native *and* wasm.
- **The data seam comes first and needs no MLE.** A language-neutral protocol
  (logic emits `Scene3D` + `Effect`, consumes `Input`) is decision-proof: it
  fixes the dev loop for F# today and lets MLE slot in as a second producer.
- **Effects are explicit in source** (no inference initially); typed failures are
  `Raise<E>` effects.
- **All closures storable by default**, represented `(stable-id, env)`;
  **rebind on hot-reload** (stored behavior adopts edits); ids resolved at the
  serialize boundary; rename-of-stored-function fails loud. See `closures.md`.
- **Ref-counting** for memory management.
- **Functional-core/imperative-shell makes logic cheap** — the heavy per-frame
  work (rendering, skinning) stays in the Rust shell, so the interpreter may be
  adequate for prod and the mle→Rust codegen may never be needed.

Layout decision: MLE lives **in this repo** as crates in the root workspace
(e.g. `mle/`), keeping the loop with its forcing client tight. Extract later if
the standalone direction takes off.

## Milestone 0 — de-risking spike (throwaway)

The one real bet: **can a tree-walking interpreter run per-frame game logic at
60fps and hot-reload with state intact?** Everything else is known-buildable.

- Hand-rolled AST + minimal evaluator (no parser polish, no types, no effects)
  computing a `Scene3D`-shaped value each frame from a model + `FrameTime`,
  embedded in `functor-runner` behind a flag.
- Measure interpreted tick+draw cost for `hello`-scale logic (tens of entities).
- Hot-reload: re-parse on file change, keep the model value, rebind functions;
  measure edit→visible latency.

**Verify:** a ms/frame number and a reload-latency number (target <100ms
edit→frame vs today's multi-second rebuild). If perf is bad, the plan pivots to
bytecode-VM-first; Track A is unaffected either way. Code is explicitly
discarded afterwards.

**Results (2026-07-01, spike in `runtime/functor-runtime-desktop/src/mle_spike.rs`,
run via `functor-runner --mle --game-path examples/mle-spike/game.mle`):**

- **Perf: yes, decisively.** Release build, naive tree-walker, scene rebuilt
  from scratch every frame: **63.6µs/frame at 51 entities (0.4%** of a 60fps
  budget); **645.9µs at 501 entities (3.9%)**. Even the unoptimized debug
  build held 51 entities at 2.5% of budget. No bytecode VM needed for
  Functor-scale logic — roadmap phase 7 stays deferred. (The number is
  isolated interpreter throughput — a tight tick+draw loop through the real
  `Game` paths, including ~0.5% of stats bookkeeping — not in-situ frame cost;
  the in-frame `[mle] avg` stats agree.)
- **Hot reload: 0.07ms re-parse**, edit→visible bounded by the one-frame file
  poll (~16ms). Model value survives the reload (spin continued from its live
  value while an edited `speed` constant reversed its direction). A
  syntax-error edit fails loud and keeps the old program running. Scope
  caveat (flagged by both review engines): this validates **global-name
  rebind only** — the spike's closures keep their parse-time bodies, so
  *closures stored in the model* would NOT adopt edits. The `(stable-id, env)`
  stored-closure rebind from `closures.md` remains unproven until B5/C3.
- Renders correctly through the real pipeline (`--capture-frame` verified —
  note a `--fixed-time` capture pins `dts = 0`, so it evidences interpreted
  scene construction at the `spin = 0` pose, not motion; motion was verified
  separately via the headless `/state` probes) and drives headless +
  debug-server (`/state` shows the MLE model).
- Two boundary semantics to pin down explicitly in Track A's protocol (both
  found the hard way here): `Material` nodes ignore their own `xform` in
  `Scene3D::render` (unlike `Group`), so transforms must be applied inside a
  material wrapper; and `Scene3D::transform` right-multiplies
  (`self.xform * xform`), so `translate(rotateY(x), …)` applies the
  translation *first* — wrapper order reads backwards from what it does.

## Track A — the language-neutral data seam (no MLE required)

Today the boundary is a shared-crate ABI (`test_render` returns a
`Graphics.Frame` struct; ~20 `no_mangle` exports in `Runtime.fs`), not a
versioned protocol.

- [x] **A1. Formalize the protocol.** Version the logic↔runtime contract in
      `functor-runtime-common` (`protocol.rs`: `PROTOCOL_VERSION` + the full
      boundary enumeration): `Frame`/`Scene3D`/`View` out, effect *commands*
      (`NetCommand`/`ConnCommand`/`AudioCommand`/`AudioScene`) out, `Input`
      key codes + `FrameTime` in — all serde round-trip-tested. Two pieces are
      documented as **in-process only**, not (yet) data: `OpaqueState` (an
      any-box with a layout assumption — made serializable by Track C's
      data-native state) and `Effect`/`EffectQueue` themselves (message
      payloads + HTTP taggers are closures; their commands cross as data).
      *Verify:* round-trip serde tests per boundary type (done); rendering
      code untouched so goldens are unaffected.
- [x] **A2. `GameProducer` trait.** Abstract the runtime's "thing that ticks and
      draws" — previously hardcoded to dylib exports (`static_game.rs`,
      `hot_reload_game.rs`) and wasm-bindgen calls — behind one trait the loop
      consumes: `functor_runtime_common::protocol::GameProducer`. The desktop
      producers implement it (re-exported as the runner's `Game`); the web
      runtime's loop drives a `WasmGame` bridge over the wasm-bindgen exports.
      *Verify (done):* cargo tests, wasm-pack bundle, headless SDK e2e 12/12;
      zero behavior change proven by byte-identical `--fixed-time` captures
      from the pre- and post-change runners.
- [x] **A3. Proof producer.** A trivial second impl (hardcoded scene or
      recorded-frame replay) selectable by flag: `--replay` plays back a
      recorded-frame JSON (`replay_game.rs`; a `Frame` or array of them, the
      exact `GET /scene` wire format) through the unchanged shells. Sample
      recording in `examples/replay/scene.json`.
      *Verify (done):* headless `GET /scene` returns the recording verbatim
      (round-trip equality); deterministic `--fixed-time` capture renders it.

Each is a small standalone PR, valuable even if MLE dies.

## Track B — the MLE vertical slice (parser → IR → interpreter)

Roadmap phases from `~/notes/ideas/mle-language/roadmap.md`, scoped to what
Functor needs (notebook features deferred). Every step is `cargo test` +
snapshots — no GPU, fully agent-verifiable.

- [x] **B1. Examples + parser → AST.** (the `mle/` crate) `.mle` subset: `let`,
      functions, records, field access, literals, pipelines, type annotations;
      source spans. *Verify:* AST snapshots per example (`UPDATE_GOLDENS=1` to
      regenerate); `mle parse`; errors point at spans. (done)
- [x] **B2. AST → core IR.** Stable IDs, name resolution, pipeline desugaring,
      spans on every node. *Verify:* `mle ir` snapshot fixtures (the
      parser↔runtime contract). (done — top-level defs are mutually visible
      and carry their name as the stable hot-reload identity)
- [x] **B3. Interpreter + run/trace.** Tree-walking evaluator over the IR:
      literals, records, **lists** (list literals added through the full
      stack — nothing could construct one before), closures with captured
      environments, late-bound globals (the hot-reload rebind semantics), a
      first builtin registry (`List.map`/`filter`/`fold`/`maximum`,
      `Text.concat`/`fromFloat`/`toBullets`, `Math.clamp01`), spanned runtime
      errors, and a depth cap that turns infinite recursion into a clean
      error. `mle run` evaluates a module (calling a zero-param `main` when
      present); `mle trace` prints the enter/exit call story with rendered
      values — kept even when the run fails. *Verify:* `.run` goldens per
      example, a `.trace` golden, and a semantics/runtime-error suite
      (closures, late binding, arity, depth caps, NaN policy). (done)
- [x] **B4. Basic types.** Gradual checking over the core IR (`mle check`),
      with annotations, not inference: primitives (`Float`/`String`/`Bool`),
      nominal declared record types, `List<T>`, and function types from
      lambda annotations. Anything unannotated or unrecognized (e.g. a
      generic parameter) is Unknown, and a check fires only where both sides
      are known — so unannotated code never false-positives. Checks:
      arithmetic/comparison/`==`/negation operand types, record literals and
      field access against declared record types, call arity + argument
      types (builtins carry real signatures with generic slots as Unknown),
      return-annotation mismatches, and type-argument arity. `mle run` stays
      check-free (integration comes later). *Verify:* `examples/broken.mle`
      + committed `broken.check` diagnostic golden (all diagnostics, sorted,
      `file:line:col`); per-diagnostic message/span unit tests; the three
      examples check clean. (done)
- [ ] **B5. Match/ADTs + storable closures.** The game-logic essentials;
      closures serialize as `(stable-id, env)`. *Verify:* serialize a value
      graph containing a closure → deserialize → call it; rename-then-restore
      fails loud.
- [ ] **B6. Minimal effect broker.** `Clock.Now`, `Random` with real/fake/replay
      handlers. *Verify:* same program under real vs fake vs replay; structured
      effect log.

## Track C — MLE as a second producer behind the seam

Starts once A2 + B3 exist.

- [x] **C1. Functor prelude.** The `mle::Host` seam (host-provided externals
      + opaque `HostData` values, added to the interpreter) and
      `functor_runtime_common::mle_prelude::FunctorHost`: `Scene.*`
      constructors/transforms/color/group, `Camera.lookAt`, `Frame.create` —
      MLE snippets emit real protocol `Frame`s (extracted via `frame_value`).
      Transforms wrap in `Group` nodes, which makes them immune to the
      Material-drops-its-xform quirk AND apply outermost-last (the order the
      source reads) — both pinned by test.
      *Verify (done):* unit tests assert the protocol data `.mle` snippets
      emit, incl. wire round-trip and the mle-hello mapped-group shape;
      prelude + mle crate build for wasm32 (ready for C5). (done)
- [ ] **C2. `MleProducer`.** Implements `GameProducer`; `functor.json` grows a
      `language` field. First game: spinning cube (`examples/mle-hello`).
      *Verify:* `--capture-frame` golden; headless SDK e2e reads the MLE model
      via `/state`.
- [ ] **C3. Hot-reload — the payoff.** File-watch → reparse → rebind, model
      preserved (already serializable data; no dylib, no cargo, no cache).
      *Verify:* SDK e2e: mutate state → edit `.mle` → assert state survived AND
      behavior changed; assert edit→frame latency budget.
- [ ] **C4. MVU parity.** Full `Game` contract (init/update/tick/input/
      subscriptions/draw3d, effect-queue drain-to-fixed-point semantics) from
      MLE. *Verify:* port `examples/primitives`; golden-compare vs the F#
      rendering.
- [ ] **C5. Wasm.** The interpreter crate compiles to wasm32; `.mle` source
      ships in the bundle. *Verify:* wasm build of mle-hello renders.
- [ ] **C6. Perf gate.** Measure C4 at 60fps with headroom; bytecode VM
      (roadmap phase 7) only if the tree-walker doesn't hold.
      *Verify:* frame-time assertion in the e2e harness.

## Track D — IDE tooling

First-class `.mle` editor support, built on the `mle` crate's front-end
(`parse`/`lower`/`line_col`) — independent of the runtime tracks.

- [x] **D1. TextMate grammar + VSCode extension.** `tools/mle-vscode/`:
      grammar, language configuration (comments/brackets/auto-close), and a
      plain-JS LSP client launching `mle-lsp` from PATH. *Verify:* grammar +
      manifests JSON-checked and `test/sample.mle` (every construct) kept
      parse/lower-clean by `cargo test -p mle-lsp`; visual check in the editor.
- [x] **D2. LSP diagnostics.** `tools/mle-lsp/`: hand-rolled stdio LSP server
      (blocking loop + serde_json, no async framework) publishing parse/lower
      errors as spanned diagnostics on open/change. *Verify:* framed-protocol
      e2e test drives the real binary (broken doc → diagnostic, fix → clear,
      unknown method → MethodNotFound).
- [ ] **D3. Hover types & go-to-definition.** Deferred: hover needs B4's
      typechecker; go-to-def needs a use→definition query over the IR (it
      already carries spans on every node, but no query API yet).

## Endgame — replace F#

Pull-based: port examples as MLE proves itself; no flag-day.

- [ ] **E1.** Port `examples/hello` (glTF lineup, free-look camera) — the
      real-world bar. *Verify:* golden parity.
- [ ] **E2.** Port remaining examples, one PR each. *Verify:* per-example
      goldens + e2e.
- [ ] **E3.** Delete the F# pipeline: Fable, dotnet tooling, `.fsproj`s,
      `fable_modules/`, the `.fs`/`.fsi`/`.rs` triplication, the dylib
      hot-reload path. *Verify:* full CI green with no dotnet installed;
      `npm run build:cli` needs only Rust + Node.

## Sequencing & risks

- Milestone 0 first (a focused spike). Then **A and B in parallel** (they share
  no code). C starts at A2+B3. Endgame is pull-based.
- **Dev/prod divergence** (if a compiled backend ever lands): define MLE
  semantics to be backend-portable up front and grow a shared conformance suite
  from the first second-backend commit. Cheap early, miserable retrofitted.
- **Interpreter too slow:** caught at Milestone 0 / C6; the pivot (bytecode VM)
  changes the execution representation, not the seam, the language, or the
  tests.
