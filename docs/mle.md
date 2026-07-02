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

## Track A — the language-neutral data seam (no MLE required)

Today the boundary is a shared-crate ABI (`test_render` returns a
`Graphics.Frame` struct; ~20 `no_mangle` exports in `Runtime.fs`), not a
versioned protocol.

- [ ] **A1. Formalize the protocol.** Version the logic↔runtime contract in
      `functor-runtime-common`: `Frame`/`Scene3D` out, `Effect` out, `Input` +
      `FrameTime` in, `OpaqueState` for persistence — all serde-serializable.
      *Verify:* round-trip serde tests per boundary type; existing goldens
      byte-identical with the F# producer.
- [ ] **A2. `GameProducer` trait.** Abstract the runtime's "thing that ticks and
      draws" — today hardcoded to dylib exports (`static_game.rs`,
      `hot_reload_game.rs`) and wasm-bindgen calls — behind one trait the loop
      consumes; the dylib producer becomes one impl.
      *Verify:* native, wasm, headless, SDK e2e all green; zero behavior change.
- [ ] **A3. Proof producer.** A trivial second impl (hardcoded scene or
      recorded-frame replay) selectable by flag.
      *Verify:* golden capture of the proof producer.

Each is a small standalone PR, valuable even if MLE dies.

## Track B — the MLE vertical slice (parser → IR → interpreter)

Roadmap phases from `~/notes/ideas/mle-language/roadmap.md`, scoped to what
Functor needs (notebook features deferred). Every step is `cargo test` +
snapshots — no GPU, fully agent-verifiable.

- [ ] **B1. Examples + parser → AST.** `.mle` subset: `let`, functions, records,
      field access, literals, pipelines, type annotations; source spans.
      *Verify:* AST snapshots per example; `mle parse`; errors point at spans.
- [ ] **B2. AST → core IR.** Stable IDs, name resolution, pipeline desugaring,
      spans on every node. *Verify:* `mle ir` snapshot fixtures (the
      parser↔runtime contract).
- [ ] **B3. Interpreter + run/trace.** Literals, records, calls, locals; run
      record with captured values. *Verify:* `mle run` / `mle trace` goldens.
- [ ] **B4. Basic types.** Primitives, records, function signatures, mismatch
      diagnostics. *Verify:* diagnostic snapshots on broken examples.
- [ ] **B5. Match/ADTs + storable closures.** The game-logic essentials;
      closures serialize as `(stable-id, env)`. *Verify:* serialize a value
      graph containing a closure → deserialize → call it; rename-then-restore
      fails loud.
- [ ] **B6. Minimal effect broker.** `Clock.Now`, `Random` with real/fake/replay
      handlers. *Verify:* same program under real vs fake vs replay; structured
      effect log.

## Track C — MLE as a second producer behind the seam

Starts once A2 + B3 exist.

- [ ] **C1. Functor prelude.** `Vec3`, `Color`, `Scene3D` constructors, `Input`
      as MLE values mapping onto the Track-A protocol (Rust-backed builtins).
      *Verify:* unit tests assert the protocol data an `.mle` snippet emits.
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

## Endgame — replace F#

Pull-based: port examples as MLE proves itself; no flag-day.

- [ ] **D1.** Port `examples/hello` (glTF lineup, free-look camera) — the
      real-world bar. *Verify:* golden parity.
- [ ] **D2.** Port remaining examples, one PR each. *Verify:* per-example
      goldens + e2e.
- [ ] **D3.** Delete the F# pipeline: Fable, dotnet tooling, `.fsproj`s,
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
