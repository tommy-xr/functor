---
name: mle-language
description: Write, run, and debug MLE (.mle) — Functor's F#-inspired game-logic language. Use whenever creating or editing .mle files, answering MLE syntax/semantics questions, or debugging MLE parse/run/check errors. MLE is a custom language — do NOT guess from F#/OCaml intuition; this skill is the source of truth for the current subset.
---

# MLE — the current language, exactly

MLE is Functor's interpreted game-logic language (roadmap: `docs/mle.md`;
design notes: `~/notes/ideas/mle-language/`). It is deliberately small; this
file describes **everything that exists today**. If a construct isn't here,
it doesn't parse — do not invent syntax from F#/OCaml habits.

## Verification loop (always available, no GPU)

```sh
cargo run -q -p mle -- parse file.mle    # surface AST (spans on every node)
cargo run -q -p mle -- ir file.mle      # name-resolved core IR
cargo run -q -p mle -- run file.mle     # evaluate: main()'s result, or all bindings
cargo run -q -p mle -- trace file.mle   # enter/exit call story with values (kept on failure)
cargo run -q -p mle -- check file.mle   # gradual typechecker: ALL diagnostics, exit 1
```

Errors are always `file:line:col: error: message`. Tests live in `mle/tests/`
with goldens next to `mle/examples/` (`UPDATE_GOLDENS=1 cargo test -p mle`
regenerates). VSCode gets live parse/lower/type diagnostics and
`name : Type` hover via `tools/mle-lsp`.

## Syntax subset

```mle
// line comments only
type Position = { x: Float, y: Float }        // record types; nominal in annotations

let threshold = 10                            // top-level let; ints/floats are all Float (f64)
let origin = { x: 0.0, y: 0.0 }               // record literal
let scores = [1.0, 2.0, 3.0]                  // list literal
let s = "text\n"                              // strings: escapes \" \\ \n \t
let flag = true                               // bools

let isHigh = (score: Float): Bool => score > threshold   // annotations OPTIONAL (gradual)
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))

let report = (scores) =>
  scores
    |> List.filter(isHigh)                    // pipeline: |> PREPENDS the piped value
    |> List.map(describe)                     //   x |> g(a)  ==  g(x, a)
    |> Text.toBullets

let nudge = (p: Position): Position => { p with x: p.x + 1.0 }  // record update (fields must exist)

let sum3 = (a, b, c) =>
  let mut acc = a in                          // expression let-in; `mut` = rebindable slot
  acc := acc + b;                             // assignment is `:=` and carries a continuation
  acc := acc + c;
  acc

let main = () => report([12.0, 3.5, 40.0])    // zero-param main is run's entry point
```

Operators: `+ - * /` `< > ==` (conventional precedence; pipelines bind
loosest), unary `-`. There is **no** if/else, match, loops, strings
concatenation operator, modules, or imports yet — iteration is
`List.map/filter/fold`, conditionals don't exist (design them out or wait
for the roadmap).

## Semantics rules that WILL bite you

- **Pipelines prepend**: `x |> f(a)` is `f(x, a)`. Every builtin/prelude
  function therefore takes its "subject" (list, scene) FIRST.
- **`:=` not `<-`** — `<-` is reserved for future do-block binds.
  Assignment must be followed by `;` and a continuation expression.
- **`mut` is non-capturable**: a lambda may not read or assign an enclosing
  `mut` binding (lowering error). Params, globals, and plain `let`s are
  immutable. No top-level `let mut`.
- **Top-level defs are mutually visible** (letrec-style) inside function
  bodies (late-bound at call time — this is the hot-reload rebind seam), but
  a *top-level initializer* may only demand globals defined above it.
- **Equality `==` is structural**; comparing functions is a runtime error.
- **Division is IEEE** (`1.0/0.0` = `inf`); the engine boundary rejects
  non-finite numbers.
- **Duplicates are errors**: top-level names (per namespace — `type Foo` and
  `let Foo` may coexist), record fields (literal and update), lambda params.
- Recursion depth is capped (~200); deep iteration belongs in `List.*`.

## Builtins (the whole registry)

`List.map(list, fn)` · `List.filter(list, fn)` · `List.fold(list, fn, init)`
(callback is `(acc, x) => …`) · `List.maximum(list)` · `Text.concat(a, b)` ·
`Text.fromFloat(n)` · `Text.toBullets(list)` · `Math.clamp01(n)`

## Functor prelude (only under the engine host — `FunctorHost`)

Available in runner-hosted MLE (and tests via
`functor_runtime_common::mle_prelude`), NOT in plain `mle run`:

```mle
Scene.cube() / sphere() / cylinder() / quad() / plane()   // zero args, enforced
Scene.group([scene, …])
scene |> Scene.color(r, g, b)                              // scene-first: pipes
scene |> Scene.translate(x, y, z)
scene |> Scene.rotateX(rad) / rotateY / rotateZ
scene |> Scene.scale(k)
Camera.lookAt(ex, ey, ez, tx, ty, tz)                      // up=+Y, fov 45°
Frame.create(camera, scene)                                // what draw returns
```

Transforms wrap in Group nodes: the **outer call applies last in world
space** — `s |> Scene.rotateY(r) |> Scene.translate(x, 0.0, 0.0)` rotates in
place, then moves (the order the source reads). Engine values (`<Scene>`,
`<Camera>`, `<Frame>`) are opaque: they can be passed around but not
inspected, compared, or serialized.

## Typechecking model (gradual)

`mle check` fires only where BOTH sides are known; unannotated = `Unknown` =
compatible with everything. Annotations buy diagnostics. Record annotations
are nominal; the runtime is structural. A `mut` slot's type fixes at its
initializer. Full Hindley–Milner is roadmapped (B7) after effects.

## Keeping this skill honest

This file must track the language. When a PR adds syntax/builtins/semantics
(see `docs/mle.md` Track B/C checkboxes), update this skill in the same PR.
