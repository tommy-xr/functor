# Functor Lang interpreter benchmarks

A dependency-light, headless harness for timing the **Functor Lang interpreter** on a
corpus of `.fun` micro-benchmarks. It exists to validate interpreter
performance across current and future language features — e.g. to confirm the
pending currying spike doesn't regress the call hot path.

> **This is a diagnostic / A-B tool, not a CI gate.** Absolute numbers are
> machine-dependent, and raw perf thresholds flake on shared CI hardware. Do
> **not** wire `functor-lang bench` into per-PR CI as a pass/fail assertion. Use it to
> compare two versions *on the same machine* (see [A-B a change](#a-b-a-change)).

## Run it

```sh
npm run bench                                   # the whole corpus, human table
cargo run -q --release -p functor-lang -- bench --all    # same thing, directly
cargo run -q --release -p functor-lang -- bench functor-lang/benches/corpus/call_saturated.fun   # one file
cargo run -q --release -p functor-lang -- bench <dir>    # every *.fun in a directory
cargo run -q --release -p functor-lang -- bench --all --json    # machine-readable output
```

Always build `--release` — a debug interpreter is many times slower and not
representative.

## What it measures

The **interpreter's evaluation hot path, isolated from parsing and lowering.**
Each benchmark file is parsed, lowered, and loaded into an `functor_lang::Session`
**once** (untimed); the harness then times *repeated evaluation of `main()`*.
So `time/op` is the cost of one `main()` evaluation — not startup, parse, or
typecheck. Each timed call goes through `Session::call`, which stands up a fresh
interpreter over the session's globals exactly as the runtime does per frame, so
the number tracks real per-frame cost.

## Methodology (why the numbers are reproducible)

1. **Warmup** — evaluate `main()` for ~50 ms, discarding results (warms caches /
   branch predictor).
2. **Auto-calibrate** — grow the iteration count until one timed batch takes
   ≥ 100 ms, so a fast bench isn't dominated by clock resolution.
3. **Median of 5** — report the median ns/op across 5 timed batches, plus the
   min/max `spread` as a stability signal (typically < 3%).

Timing uses `std::time::Instant` only — no `criterion`, no extra deps, so the
harness stays trivially runnable. `spread` is `(max - min) / median`; if it is
large on your machine, close background load and re-run.

## The benchmark-file convention

A benchmark is an ordinary `.fun` project whose entry defines a **zero-arg
`let main`** that performs the unit of work to be timed:

```functor
// BENCH: <one line naming what this measures>
let step = (acc, x) => acc + x
let main = () => List.fold(step, 0.0, List.range(1000000))
```

Rules:

- **`main` is the timed unit.** Put the work *inside* `main` (or functions it
  calls), not in a top-level `let` initializer — initializers run once at load
  (untimed), so work there is invisible to the harness.
- Every benchmark file is therefore also a normal program:
  `functor-lang run corpus/<file>.fun` prints `main()`'s result. Handy for verifying a
  benchmark is correct before trusting its timing.
- The harness uses the plain-`functor_lang` prelude (no engine host), so benchmarks may
  use only core builtins (`List.*`, `Text.*`, `Math.*`, user code) — **not**
  `Scene.*`/`Camera.*`/etc., which resolve only under the runtime host.
- Because a directory is one Functor Lang project (`file = module`), every `.fun` in
  `corpus/` loads together. Keep each self-contained; sibling `main`/helper
  names don't collide (they are per-module).

## A-B a change

Absolute ns/op is machine-specific, so the useful signal is a **before/after on
the same machine**:

```sh
# on the base ref
git switch main
cargo run -q --release -p functor-lang -- bench --all --json > /tmp/before.json

# on your branch
git switch my-feature
cargo run -q --release -p functor-lang -- bench --all --json > /tmp/after.json

# compare ns_per_op per benchmark
```

Run each side 2–3× to confirm the delta exceeds the `spread`. For call-overhead
questions specifically, subtract `fold_floor` (the empty-step baseline) from the
call benches to isolate per-call cost from range-building + fold overhead.

## Adding a benchmark

1. Drop a `corpus/<name>.fun` with a `// BENCH:` header naming what it measures
   and a zero-arg `let main`.
2. Size the work so one `main()` call is at least a few hundred microseconds
   (fold/map over a range) — sub-microsecond benches are mostly noise.
3. Verify it: `cargo run -q --release -p functor-lang -- run functor-lang/benches/corpus/<name>.fun`.
4. It joins the table automatically (`--all` globs the directory).

Note: `call_partial.fun` (partial application / currying overhead) is **not**
included — it requires currying, which is not in `main`. It joins the corpus
when the currying spike lands.

## Corpus

| file | measures |
| --- | --- |
| `call_saturated.fun` | saturated multi-arg closure calls (~2M/eval) — the call hot path |
| `call_piped.fun` | piped calls that lower to a saturated call (the pipe hot path) |
| `fold_floor.fun` | the baseline: `List.range(1M)` + a 1M no-op fold (subtract to isolate call cost) |
| `list_map.fun` | `List.map` + `List.filter` pipeline over 100k, with intermediate allocations |
| `arith_loop.fun` | arithmetic-heavy fold (mul/add/div + `Math.sin`) over 500k |
| `recursion.fun` | self-recursion at volume (~1.5M shallow recursive calls) |
| `pattern_match.fun` | nested bool-literal `match` dispatch over 200k |
| `adt.fun` | ADT construct + variant-pattern match over 100k |
| `record_update.fun` | `{ r with … }` record update threaded through a 100k fold |

## Micro-suite vs the frame bench (which to use when)

This corpus times language **micro-ops under the plain prelude** — perfect for
isolating an interpreter change, but a *derived* per-frame estimate from it has
misjudged real game cost before. The windowed runtime's `draw_us` telemetry is
no substitute: it inflates ~2x on sub-saturated scenes (vsync idle + DVFS
downclocking between frames), so neither number tells you what a frame truly
costs on CPU.

For that, use the **macro frame bench** — headless, no GL, engine prelude
(`Scene.*`/`Camera.*`/`Frame.*` resolve for real), calling a synthwave-shaped
game's `draw` back-to-back at full clock, reporting µs/frame, µs/cell, and —
the deterministic, run-to-run-identical metric — **allocations per frame**:

```sh
cargo run -q --release -p functor_runtime_common --example frame_bench
```

Use the micro-suite to localize *what* regressed; use the frame bench to judge
*whether a game cares*. A/B it the same way as the corpus (base ref vs branch,
same machine, 2-3 runs each; the timed sample count per size is fixed, so runs
draw from comparable samples). Under background load the median inflates badly
— prefer `us/frame(min)` (least contaminated, though not immune) and the alloc
columns (exact). Details in the example's doc header
(`runtime/functor-runtime-common/examples/frame_bench.rs`).

## Baseline (for orientation only — your numbers will differ)

Apple M3 (Mac15,12), rustc 1.96.0, `--release`:

```
benchmark               iters       time/op         ops/s    spread
adt                         2      57.6  ms          17.4      0.4%
arith_loop                  1     165.8  ms           6.0      0.6%
call_piped                  1     322.4  ms           3.1      1.6%
call_saturated              1     329.1  ms           3.0      2.8%
fold_floor                  1     130.6  ms           7.7      0.7%
list_map                    3      37.5  ms          26.6      0.2%
pattern_match               2      70.0  ms          14.3      0.7%
record_update               4      30.9  ms          32.4      1.8%
recursion                   1     357.4  ms           2.8      0.7%
```

The full `--all` run takes ~14 s on that machine (the 1M-fold benches dominate).
