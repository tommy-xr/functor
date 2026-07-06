# MLE: replacing F#/Fable with our own language

> **STATUS: COMPLETE (2026-07-05).** The endgame landed — MLE is now the *only*
> game-logic language, and the F#/Fable pipeline (Fable, dotnet tooling,
> `.fsproj`s, `fable_modules/`, the `.fs`/`.fsi`/`.rs` triplication, the dylib
> hot-reload path) has been deleted (roadmap **E3**, below). `npm run build:cli`
> needs only Rust + Node. This document is retained as the design record and the
> history of how MLE was built; the `mle-language` skill is the live source of
> truth for the language as it exists today.

> Design + phased roadmap. Each step was independently verifiable and landed as
> its own small PR. The endgame was a complete replacement of the F#/Fable
> pipeline, reached pull-based (no flag-day): F# and MLE coexisted behind one
> seam until MLE won on every axis.

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

**Results (2026-07-01; the spike was deleted in C2 when the real producer
replaced it — `--mle` now runs the actual interpreter):**

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
- [x] **B5. Match/ADTs + storable closures.** The game-logic essentials.
      **Part 1 — ADTs + `match` — done (2026-07-03):** variant `type`
      declarations (`| Ctor(name: Type, …)` / nullary `| Ctor`; leading `|`
      required, first alternative included); constructors live in the value
      namespace (resolve bare, unique across types, collide with `let`s),
      are called positionally, and are first-class when unapplied; `match
      expr with | pattern => expr` with constructor/variable/`_`/literal
      patterns (bool-literal arms are the language's first conditional; arms
      parse greedily — parenthesize nested matches); structural variant
      equality; gradual checking with exhaustiveness (missing ctors named),
      foreign-ctor/literal-compatibility diagnostics, typed pattern
      variables, and arm-result joins; hover for ctor signatures and pattern
      vars; `examples/shapes.mle` + goldens.
      **Part 2 — storable closures — done (2026-07-03):** closures stored
      in the model rebind across a hot reload (`mle::rebind`; design:
      `closures.md` — rebind not content-address, ids by stable name,
      identity resolved at the boundary). A lambda's id is its def's name
      + `#k` traversal ordinals; runtime closures carry only their ExprId
      (both id tables derived at reload time, off the hot path); captured
      envs carry over BY NAME with recursive rebinding through containers
      and captured closures; unmatched ids / unresolvable captures keep
      the old body with a loud warning, and an Rc pointer guard makes
      stale ids from older modules unidentifiable rather than
      misidentified. Serialize-to-bytes rides on the state protocol later
      — the `(stable-id, env)` split is done here.
      *Verify (done):* 7 rebind unit tests (adopt+keep-env, containers,
      recursive captures, deleted-def / new-capture warnings, stale-id
      guard, data passthrough); pinned-clock SDK e2e — edit a stored
      `vel` closure's body live, `x' = x + newBody(oldK · dt)` exactly.
- [x] **Language: tuples** (done 2026-07-03). Real F#-style tuples, landing BEFORE B6 (which
      consumes them: `update`/`tick` return `(model, effects)` pairs, the
      F# contract's shape). Scope: `(a, b)` literals (≥ 2 elements — `(e)`
      stays grouping); tuple patterns in `match` (`| (x, y) =>`, shallow
      like ctor sub-patterns) and a destructuring let
      (`let (a, b) = e in …`) since multiple-returns is the point;
      `Float * Float` product types in annotations; `Value::Tuple` with
      structural equality and `(1, 2)` display; checker arity + element
      types, exhaustiveness-compatible; hover/LSP display; rebind walk;
      no positional field access (`t.0`) — destructure instead (named
      records stay the LLM-native default for anything that outlives an
      expression). Skill updated in the same PR. *Verify:* semantics +
      error + checker tests, goldens, an example using a
      multiple-return function. *Verify (done):* `mle/examples/tuples.mle`
      + full goldens; parser/run/check/rebind pin tests (arity-mismatch =
      non-match, structural equality, `(e)` stays grouping, 1-tuple and
      mut-destructure teaching errors, element types flow through
      patterns, closures inside tuples rebind); 205 mle tests green.
- [x] **B6. Minimal effect broker** (done 2026-07-03). `Effect.none/now/
      random/batch` prelude values (taggers validated callable at
      construction); any entry point may return a `(model, effect)` tuple —
      `split_model_effect` sniffs the pair (an Effect value in model data is
      meaningless, so the sniff is unambiguous) and both producers funnel
      every return through one `absorb` path. `drain_effects` (shared,
      prelude-level) performs each effect through an **EffectRunner** —
      `RealEffects` (wasm-safe clock: `Date.now()` on wasm32, where
      `SystemTime` panics), `FakeEffects`, `ReplayEffects` — applies the
      tagger via the new `Session::apply`, folds the message through
      `update`, and drains chained effects to a fixed point (1000/frame
      cap). Every performed effect lands in the **structured EffectLog**
      (`{kind, value}` — replay's input format). Taggers run same-frame, so
      no closure outlives its session (no reload hazard). *Verify (done):*
      the broker contract as exact arithmetic — same program under fake and
      replay produces the same model, the fake run's log IS replay's input,
      divergent replay fails loud with position; construction teaching
      errors; runaway-chain cap; SDK e2e through the real runner (key →
      Effect.random → chained Effect.now → both sentinels replaced).
- [x] **Language: record updates + local mutability** (2026-07-02; design:
      `~/notes/ideas/mle-language/mutability.md`). `{ base with x: 1.0 }`
      pure record updates; expression-level `let [mut] x = e in body` with
      `x := e; rest` assignment (`:=`, not `<-` — that's reserved for B6
      do-blocks). `mut` bindings are **non-capturable** (a lowering error,
      F#-style) and rejected at top level, so the acyclic-RC/serializable-
      state/replay invariants survive untouched. Typechecked: slot types fix
      at the initializer; updates check against declared record types.
      *Verify:* 18 semantics/error/diagnostic tests + example goldens. (done)
- [x] **Units, tier 1: branded `Angle` values** (2026-07-03; design:
      `~/notes/ideas/mle-language/units.md`). `Angle.degrees(n)` /
      `Angle.radians(n)` opaque host values; rotations and camera angles
      REFUSE bare numbers with a teaching error — degree/radian confusion
      is unrepresentable, matching the F# side's `Math.Angle` discipline.
      Tier 2 (F#-style units of measure with unit algebra) rides on B7.
- [x] **B7. Hindley–Milner inference** (done 2026-07-04; decided 2026-07-02; **after effects
      land** — B6 + the `effect[...]` header checking, so type inference and
      effect rows are designed against each other, not retrofitted). Upgrade
      the B4 gradual checker to real inference: type variables + unification,
      let-polymorphism, and generic instantiation (element types flow through
      `List.map`; `Unknown` shrinks to genuinely-dynamic seams like host
      values). Gates to clear first: the nominal-vs-structural record
      decision (B4 checks nominally, the runtime is structural — inference
      with teeth needs one answer), and unification-error UX (every mismatch
      must cite the source spans of *both* sides — legible errors were the
      reason annotations came first; see `~/notes` `open-questions.md`).
      **Done:** type variables + unification with occurs check;
      let-polymorphism generalized per SCC of the def call graph (mutual
      recursion monomorphic inside its group; `id` at two types in one
      module works); generic builtin schemes (element types flow through
      `List.map`/`filter`/`fold`); lowercase annotation names are scoped
      type variables; record literals resolve nominally F#-style (user
      decision: unique field-set match, ambiguity asks for annotation;
      anonymous records stay gradual); Unknown remains ONLY the
      host/dynamic seam and never binds a variable; conflicts report once
      with full zonked types, labeled by where the expectation came from.
      Inference has teeth: unannotated bad calls, mixed lists,
      contradictory mut use, and foreign match arms are all caught now.
      *Verify (done):* both shipped games + all example goldens check
      clean under inference; `(xs, k) => List.map(xs, (x) => x * k)`
      hovers as `(List<Float>, Float) => List<Float>`; teeth + occurs +
      ambiguity + SCC polymorphism pinned (215 mle tests).
      *Original verify:* unannotated examples get full inferred signatures (an
      `mle types` dump, goldened); the B4 diagnostic suite still passes;
      probe battery re-run (no legal program rejected).

- [x] **Language: list patterns + cons** (2026-07-05). `[a, b]` / `[]` /
      `[head, ..rest]` patterns in `match` (element + tail sub-patterns are
      names/`_`; exact-length unless `..`), and `[x, ..xs]` cons in
      expressions. Element types flow through both (inference with teeth:
      `["s", ..floats]` errors); proper exhaustiveness — `[] | [h, ..t]`
      IS exhaustive, `[a, b]` alone needs a catch-all. Full stack: lexer
      (`..`), parser, lower, eval, HM types, hover/goto/rebind. Verify:
      `mle/examples/lists.mle` + goldens; run/parser/check pin tests.
- [x] **Language: generic type declarations** (done 2026-07-04, the B7
      follow-up both review engines asked for). `type Box<v> = | Full(value:
      v) | Empty` / `type Pair<x, y> = { … }`: checker-only (the runtime
      was already type-erased) — declarations store parameter placeholders
      in an out-of-band Var namespace (the builtin-scheme trick),
      substituted at every use; ctors instantiate fresh per use; record
      literals solve the parameters; patterns take the scrutinee's
      arguments; arity + duplicate/uppercase/undeclared-param teaching
      errors. Found and fixed: three type walkers (zonk / instantiate /
      renumber) predating generics treated nominal types as leaves, so
      vars inside `Box<'a>` never substituted. *Verify:* Box/Pair at two
      types in one module; instantiation constrains; pattern-field types
      flow; erased-runtime pin; goldens.
- [ ] **B8. Multi-file modules + `.mlei` interface files** (added
      2026-07-02; design in `~/notes` `syntax.md` — the OCaml `.mli` split).
      **Part 1 — multi-file modules — done (2026-07-04):** file = module —
      every `.mle` in the entry file's directory IS a module named by its
      capitalized filename stem (`utils.mle` → `Utils`; stems must be
      identifiers); loading is EAGER (whole-program: unreferenced siblings
      still load, check, and evaluate). Qualified access needs NO import
      (`Utils.clamp(x)`, `Utils.Circle(…)` in expressions AND patterns,
      `Utils.Shape` in annotations, generics included); `open Utils` brings
      a module in unqualified, with collisions (own names, other opens)
      refused naming both sides. Cross-file dependency CYCLES are refused
      with the cycle path (within-file letrec unchanged); module names
      colliding with builtin/prelude namespaces (List, Scene, …) are
      refused. The link is one MERGED module (`mle::project::load`):
      per-file lowering canonicalizes names (non-entry defs/types/ctor tags
      become `M.name`; the entry stays bare — a single-file project is
      byte-identical to before), spans offset into one project-wide space
      (`SourceMap` renders errors per file), IDs thread across files — so
      eval, checker, `Session`, and `rebind` consume it UNCHANGED:
      cross-module calls are ordinary late-bound globals, and rebind stable
      ids inherit the module prefix from def names (cross-file reloads
      rebind stored closures correctly, same-named defs in different
      modules stay distinct). `mle ir/check/run/trace` load the project;
      `functor build` checks the whole program; the native producer watches
      EVERY project file (edit/add/remove any `.mle` → hot reload, model
      preserved). *Verify (done):* `examples/project/` fixture + 27 project
      tests (collisions, cycle paths, protected names, per-file
      diagnostics, byte-identity pin, cross-file rebind); SDK e2e — editing
      a NON-entry module hot-reloads with exact-arithmetic model
      preservation (18/18 headless).
      Follow-ups deferred from part 1: LSP cross-file support (the
      per-file view errors on `open` — honest but red; project-load the
      buffer's siblings), wasm/live-preview multi-file (the web producer
      and `reload-source` interpret ONE source text; native-first), and
      `functor push --watch` watching only the entry file.
      **Part 2 — `.mlei` interface files** (next): a module's public
      contract as a checked file: exported types (including **abstract**
      types that hide their representation), function signatures, and —
      now that B6 landed — effect requirements. `mle check` verifies the
      implementation satisfies its interface; consumers typecheck against
      the `.mlei` alone. The LLM payoff is the point: an interface file is
      the concise, load-into-context summary of a module.

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
      emit, incl. wire round-trip and the hello-cubes mapped-group shape;
      prelude + mle crate build for wasm32 (ready for C5). (done)
- [x] **C2. `MleGame` — the real producer.** `mle_game.rs` runs `.mle` logic
      through `mle::Session` + the C1 prelude behind the existing `--mle`
      flag, **deleting the Milestone-0 spike**. Contract: `init` value,
      `tick(model, dt, tts)`, `draw(model, tts) -> Frame`; the model is a
      plain MLE value held by the host (the C3 reload seam). Type
      diagnostics print as warnings at load; per-frame errors keep the last
      good model/frame. First game: `examples/hello-cubes` (ring of cubes +
      pulsing sphere; exercises `with`-updates, `let`, pipelines,
      `List.range`/`Math.sin` — both added here). Release perf: tick 5.2µs +
      draw 47.7µs = 0.3% of budget at 13 entities.
      *Verify (done):* byte-identical `--fixed-time` captures; headless
      `/state` shows the live MLE model. (`functor.json` `language` field —
      CLI wiring — deferred to C4 alongside input.)
- [x] **C3. Hot-reload — the payoff.** The producer polls the file's mtime
      each frame; on change: reparse → recheck → new `Session`, **model
      preserved** (it is a plain value the host holds — the C2 architecture's
      whole point). A broken edit prints once and keeps the old program;
      contract violations (missing `tick`, function `init`) reject the new
      session the same way. Reload observed at **0.14ms** re-parse +
      ≤ 1 frame poll — versus the multi-second Fable+cargo loop this project
      exists to kill. (The original caveat — closure values stored *inside*
      the model kept pre-reload bodies — was removed by B5 part 2: stored
      closures now rebind too.)
      *Verify (done):* SDK e2e (`mle-hot-reload.e2e.test.ts`, headless): with
      the debug clock pinned, spin `0.3` + one post-edit step = exactly
      `0.3 + dt×(-5)` — state survived AND behavior changed as arithmetic,
      not a race; latency asserted < 100ms; broken-edit resilience asserted.
      The SDK gained an `mlePath` launch option.
- [ ] **C4. MVU parity.** Full `Game` contract (init/update/tick/input/
      subscriptions/draw3d, effect-queue drain-to-fixed-point semantics) from
      MLE. *Verify:* port `examples/primitives`; golden-compare vs the F#
      rendering.
      - [x] **C4a. Input + CLI wiring.** Optional `input` entry point —
        `(model, key, isDown) => model`, keys as canonical names ("W",
        "Up") — validated at load when present, reload-aware. And
        `functor.json` grows `"language": "mle"` (+ optional `entry`,
        default `game.mle`): `functor build` = parse+lower+**check as
        errors** (the strict gate; the runner keeps them warnings),
        `run native` spawns the interpreter (proven byte-identical to a
        direct runner invocation), `develop` = `run` (hot reload is built
        in — no watchexec), wasm errors cleanly until C5.
        *Verify (done):* SDK e2e asserts two key events reach the model
        with canonical names (14/14 suite); CLI build/run/wasm probes.
      - [x] **C4b-1. Mouse + the lit prelude + the primitives port.**
        Optional `mouseMove(model, x, y)` / `mouseWheel(model, delta)`
        entry points; prelude grows the lit pipeline — `Scene.lit`/
        `Scene.emissive`, all three `Light.*` kinds + `castShadows`,
        `Camera.firstPerson`, `Frame.createLit`. `examples/primitives`
        ports the F# golden scene (shadow-casting sun, orbiting colored
        point lights, emissive markers) — **0.000% pixels over the golden
        tolerance vs the F# render** at the same fixed time.
      - [x] **C4b-2. The MVU pair** (done 2026-07-03): optional
        `update(model, msg)` + `subscriptions(model)` entry points
        (messages are B5 ADT variants); prelude grows `Sub.every/none/
        batch` and branded `Time.seconds/millis` Durations (the Angle
        rule, applied to time). `Sub.every` is stateless — it fires when
        the global time grid crosses in `(prevTts, tts]`, the F#
        `crossedBoundary` rule verbatim — so timers need no identity and
        tick right through a hot reload; fired messages fold through
        `update` before `tick`, the drain seam B6's effects will feed.
        `examples/hello-cubes` gains a once-per-second Beat.
        *Verify (done):* pinned-clock SDK e2e proves exact arithmetic —
        one message per period step, millis/seconds parity, a long frame
        collapses missed boundaries into ONE firing, and a reload that
        ADDS subscriptions starts from the current frame edge (no
        catch-up burst); prelude unit tests pin the grid math and the
        teaching errors (15/15 e2e suite).
      - [x] **C4b-3. Render targets** (done 2026-07-04): prelude grows
        the render-to-texture surface over the engine feature —
        `RenderTarget.named`/`sized` (a branded handle: the Angle rule
        applied to *identity*, so writer/reader id typos are
        unrepresentable), `Frame.withRenderTarget(frame, target,
        targetFrame)` (the writer: a full inner frame — camera/scene/
        lights — rendered offscreen before the main pass, with the
        inner frame's own lights: `Frame.createLit` + `castShadows`
        gives a lit, shadowed feed), and `Scene.screen(scene, target)`
        (the reader: an emissive surface showing the target's texture;
        an undeclared id renders magenta + warns once).
        `examples/monitor` demos it: a panning security camera
        filming the courtyard, shown live on an in-world monitor.
        *Verify (done):* prelude unit tests pin the frame's target
        passes, the Scene.screen wire shape, and the branded-value
        teaching errors; deterministic `--fixed-time` captures of
        monitor show the second camera's view on the screen.
      - [x] **C4b-4. Fog** (done 2026-07-04): prelude grows the
        distance-fog surface over the engine feature — branded
        `Fog.linear(near, far, r, g, b)` / `Fog.exp(density, r, g, b)`
        values (degenerate parameters — near < 0, far <= near,
        density <= 0 — are teaching errors at construction) and
        `Frame.withFog(frame, fog)`. Fog applies to every forward
        material including emissive (fog occludes glow) and drives the
        pass's clear color, so geometry dissolves into the horizon.
        `examples/atmosphere` demos it: an identical-pillar
        colonnade receding into fog, with an emissive drifter.
        *Verify (done):* prelude unit tests pin the frame's fog wire
        shape and the teaching errors; a fog-less frame renders
        BYTE-IDENTICALLY to the pre-fog engine (base-vs-fog golden
        captures compared with cmp — the engine PR's contract).
      - [x] **C4b-5. Skybox** (done 2026-07-04): prelude grows the
        cubemap-sky surface — branded
        `Skybox.files(px, nx, py, ny, pz, nz)` (six non-empty face
        paths, +X..-Z) and `Frame.withSkybox(frame, sky)`. The sky
        draws behind everything after the pass's clear; while the six
        faces load the clear color shows, and a failed face disables
        the sky with one warning. Fog does not apply to the sky.
        `examples/atmosphere` gains the TropicalSunnyDay sky
        (fetched via `npm run fetch:assets`, gitignored), with the fog
        color tuned to the sky's horizon band so the colonnade
        dissolves INTO the sky.
        *Verify (done):* prelude unit tests pin the skybox wire shape,
        face order, and teaching errors; deterministic captures show
        the sky behind the fogged colonnade.
- [x] **C5. Wasm** (done 2026-07-03). `MleWebGame` in the web runtime — the
      wasm sibling of the desktop producer behind the same `GameProducer`
      seam: identical load-contract validation, MVU subscriptions pump,
      physics hook (rapier is pure Rust — the world steps in the browser),
      per-frame error dedupe + last-good-frame. Nothing compiles: the `.mle`
      source ships as TEXT — `functor run wasm` serves the project dir with
      an MLE index page (`index-mle.html`, entry substituted in by the dev
      server), the page sets `window.__mleGamePath`, and the runtime fetches
      + interprets it. Page input reaches the producer via `mle_*` wasm
      exports, queued and drained before each tick. Hot reload stays
      native-only — on web, reload the page (the server reads the file per
      fetch, so edits are one refresh away).
      *Verify (done):* mle + web runtime build clean for wasm32; CLI
      build/run wasm probes on `examples/hello-cubes` — the served index page,
      wasm bundle, and `.mle` source all curl back correctly; entry
      substitution unit-tested; headless-Chromium probe renders the game
      (ring + sphere) AND proves the Beat subscription folded through
      `update` (sphere center yellow → magenta across a 1s boundary, the
      exact beat arithmetic); `cargo test -p functor_runtime_common` green.
- [x] **C6. Perf gate** (done 2026-07-04). The tree-walker holds — no
      bytecode VM (roadmap phase 7) needed. Measured on a 100-entity lit
      scene (per-entity model updates in `tick`, per-entity transforms in
      `draw`, shadow-casting sun + two orbiting point lights — heavier
      than any shipped example), free-running headless at ~60Hz, debug
      build, Apple Silicon: **tick 211µs + draw 2116µs ≈ 2.3ms/frame =
      14.0% of the 16.6ms budget**. Draw dominates ~10:1 — the cost is
      building ~100 prelude scene nodes per frame, not the MVU fold.
      *Verify (done):* `mle-perf.e2e.test.ts` free-runs that load on the
      wall clock for two 300-frame stats windows and asserts the last
      window's tick+draw stays under **60% of the budget (10000µs)** —
      generous by design; the gate catches order-of-magnitude regressions
      (an accidental deep-clone per frame), not scheduler noise. It is
      **opt-in** (`FUNCTOR_PERF=1`, the golden-test precedent), NOT part
      of the per-PR e2e suite: the measurement depends on real-time frame
      THROUGHPUT, which shared CI runners can't guarantee (the same eval
      that finishes locally in ~13s repeatedly blew past a 240s wait on
      GitHub's macOS runners — contention, not a regression), and a flaky
      required check is worse than a reliable on-demand one. Run it
      deliberately or from a dedicated non-blocking perf job.

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
- [x] **D3a. Hover types + type diagnostics in-editor.** `mle::hover`
      (language-aware, unit-tested: innermost node at an offset →
      `name : Type` from the checker's per-expression table, honest
      `Unknown` for unannotated code) behind `textDocument/hover` with
      UTF-16-correct positions; `mle check`'s full diagnostic set now
      publishes alongside parse/lower errors. *Verify:* 8 hover unit tests +
      the framed e2e drives a real hover round-trip.
- [x] **D3b. Go-to-definition.** `mle::goto::definition_span` (a hover-style
      innermost-node walk over the IR): local references — params, `let`
      binders, pattern variables, `:=` targets — resolve by `BindingId` (so
      shadowing is already right); globals to their def's `let name =`
      region; constructor uses (expressions AND patterns) to their
      `VariantDecl`; declared-type annotation names (params, returns,
      type-decl fields, generic args) to their `type` declaration; anything
      else `None`. Surfaced as `textDocument/definition`
      (`definitionProvider: true` — VSCode wires F12 automatically).
      *Verify (done):* 16 unit tests cover every resolution case incl.
      shadowing, binders-are-not-references, and off-node → None; the
      framed-protocol e2e drives a real definition round-trip (hit → the
      correct range, empty spot → null; the unknown-method probe moved to
      `textDocument/implementation`); `cargo test -p mle -p mle-lsp` green.
- [x] **D4. Live game preview in the editor** (done 2026-07-03, needs C5).
      A VSCode webview panel hosting the wasm runtime: the extension's
      **"MLE: Open Live Preview"** command serves the project
      (`functor run wasm --no-open`, binary from `mle.functorPath`) in a
      full-size iframe and pushes the LIVE buffer (300ms debounce, unsaved
      included) into it, and the new `mle_set_source` wasm export mirrors
      the native reload path — parse → lower → check-as-warnings →
      `Session::load` → `mle::rebind_value` on the held model (the web
      producer keeps its lowered `Module` like the desktop one).
      `Session`/`rebind` are pure Rust, so **model-preserving hot reload
      runs in the browser**: type, and the running game updates beside the
      editor without losing state (a broken edit keeps the old program,
      same as native; the error lands in the status bar). v1 serves via
      `functor run wasm` + iframe; a bundled self-contained runtime can
      come later. *Verify (done):* headless-Chromium e2e
      (`node e2e/mle-preview-reload.mjs`, self-serving) drives the page's
      postMessage seam on `hello-cubes`: a green push reloads with "model
      preserved" and the center pixel turns green; a probe whose tick
      errors iff `spin <= 0.5` runs clean while its inversion errors —
      spin only exceeds 0.5 by accumulating ACROSS reloads, so the model
      demonstrably survived; a broken push is rejected with the parse
      error and the old program keeps rendering; a push after the broken
      one lands (12/12); manual: edit `hello-cubes` live in the panel.

## Endgame — replace F#

Pull-based: port examples as MLE proves itself; no flag-day.

- [x] **E1.** Port `examples/hello` (glTF lineup, free-look camera) — the
      real-world bar. *Verify (done 2026-07-04):* `examples/hello`
      renders **full-frame parity** with F# hello at `--fixed-time 2.0`.
      The prelude gained `Scene.model(path)` (E1a), then the three
      remaining engine gaps (E1b): `Scene.heightmap([[h,…],…])` (the
      ripple dunes), `Texture.file(path)` + `Scene.litTexture`/
      `emissiveTexture` (the dirt ground + neon sign), and the optional
      `ui(model)` entry point over `Ui.text`/`column`/`panel`/`topLeft`
      (the HUD). At 1600×1200 the two renders differ by **0.09% of
      pixels, ALL of them the HUD's frame-counter glyphs** (it counts
      wall-clock ticks even under `--fixed-time`, so it differs between
      any two runs — F#-vs-F# included); every scene pixel is identical
      at tolerance 16, and the 0.44%-at-tol-0 is sub-visual f32/f64
      heightmap-shading (documented in the game.mle header). Camera,
      input math, model lineup, lit primitives, neon sphere, and lights
      were already byte-identical from E1a.
- [x] **E2.** Port remaining examples, one PR each (done 2026-07-05 — every
      sample is now an `examples/*` project). *Verify:* per-example
      goldens + e2e.
      *Progress — networking (2026-07-05):* the MLE **net surface** landed
      so the multiplayer samples can port. A built-in **`Net` module**
      (injected by the project loader, always in scope) provides the
      `NetEvent` ADT — `type NetEvent = | Connected(id) | Message(id, text)
      | Disconnected(id) | Error(id, text)` — so games `match ev with |
      Net.Connected(id) => …` without declaring it (the prelude-provided
      ADT the user chose over a record-with-`kind`; `EffectValue::Variant`
      lets the host build the `Net.*` values). Prelude: `Sub.connect(url,
      tagger)` / `Sub.listen(addr, tagger)` (a `SubTree` the producer
      RECONCILES into Connect/Listen/CloseKey commands each frame, routing
      inbound events to the matching key's fresh tagger through `update` —
      the physics-events pattern) and `Effect.send(id, text)` (tagger-less,
      queues a `ConnCommand::Send`). Both producers wired.
      `examples/wsdemo` (client, ctor tagger) + `examples/wsserverdemo`
      (server, closure tagger, per-client ids) port their F# originals;
      headless unit tests drive the full lifecycle without a socket.
      Remaining: `mpclient`/`mpserver`, then HTTP (`netdemo`).
- [x] **E3.** Delete the F# pipeline: Fable, dotnet tooling, `.fsproj`s,
      `fable_modules/`, the `.fs`/`.fsi`/`.rs` triplication, the dylib
      hot-reload path (done — the atomic cut). *Verify (done):* CI green with
      no dotnet installed; `npm run build:cli` needs only Rust + Node.

## Sequencing & risks

- Milestone 0 first (a focused spike). Then **A and B in parallel** (they share
  no code). C starts at A2+B3. Endgame is pull-based.
- **Dev/prod divergence** (if a compiled backend ever lands): define MLE
  semantics to be backend-portable up front and grow a shared conformance suite
  from the first second-backend commit. Cheap early, miserable retrofitted.
- **Interpreter too slow:** caught at Milestone 0 / C6; the pivot (bytecode VM)
  changes the execution representation, not the seam, the language, or the
  tests.
