# Coming from OCaml

Functor Lang is the module-system kin you'll recognize fastest. **File = module**,
exactly like OCaml's convention: every `.fun` in a directory is a module named by
its capitalized filename stem, loaded together, with qualified access
(`Geometry.area`) that needs no `open`. It has records and variants typed by
Hindley–Milner inference, `match ... with` over constructor patterns, `let` and
lambdas, `|>` threading last, and generics with `'a` type variables. And its
interface files — `.funi` — are the direct analogue of your `.mli`.

The differences are deliberate. Functor Lang is **interpreted, not compiled** — no
`dune`, no build step, and the runtime hot-swaps your code under a running game.
It is a **small** language: no module functors, no nested modules, no objects,
one number type, and minimal patterns. Types are **gradual** — real inference,
but annotations are optional and diagnostics advisory. And a program is an
Elm-style **Model–View–Update** loop whose `draw` returns a 3D `Frame`.

## The same code, side by side

OCaml has no single UI convention, so we compare the shared core: the model
type, a `msg` variant, and an `update` that pattern-matches. On the left,
idiomatic OCaml; on the right, the same core as a running Functor Lang counter
whose sphere grows with the count. Press **▶ try it** to run the Functor version
live.

<div class="compare-cols">

```ocaml
type model = { count : float }

type msg =
  | Increment
  | Decrement

let init = { count = 0. }

let update model = function
  | Increment -> { model with count = model.count +. 1. }
  | Decrement -> { model with count = model.count -. 1. }

let render model =
  Printf.sprintf "count: %g" model.count
```

```functor run
type Model = { count: float }
type Msg = | Increment | Decrement

let init = { count: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Increment => { m with count: m.count + 1.0 }
  | Decrement => { m with count: m.count - 1.0 }

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.create(
    Camera.lookAt(0.0, 1.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere()
      |> Scene.emissive(0.3, 0.7, 1.0)
      |> Scene.scale(1.0 + m.count * 0.15))

let ui = (m: Model) =>
  Ui.row([
    Ui.button("-", Decrement),
    Ui.text(Text.fixed(m.count, 0.0)),
    Ui.button("+", Increment),
  ]) |> Ui.panel(Ui.topLeft())
```

</div>

## Modules and interface files

File = module is the OCaml convention, made mandatory: there is no `module M =
struct ... end`, and no compile-order list — every sibling `.fun` loads together.

```functor
// geometry.fun  →  module Geometry
type Shape = | Circle(radius: float) | Square(side: float)

let area = (s: Shape): float =>
  match s with
  | Circle(r) => 3.14159 * r * r
  | Square(a) => a * a
```

```functor
// game.fun  (the entry)
open Geometry                          // like OCaml's `open Geometry`…

let total = (shapes: List<Shape>): float =>
  shapes                               // …or qualify: Geometry.area — no open needed
    |> List.map(area)
    |> List.fold((acc, a) => acc + a, 0.0)
```

A sibling `.funi` is an interface module — the direct analogue of an `.mli`. It
declares abstract types and bodyless value signatures, and gives the checker real
types where a value would otherwise be `Unknown`:

```functor
// clock.funi  →  interface module Clock  (an .mli, in spirit)
type Clock                             // abstract type, like `type t`
let now : () => Clock
let elapsed : (Clock) => float
```

The one twist: a `.funi` types values the **host runtime** implements — there is
no paired `.fun` beside it (same-stem files are a load error). It's how the engine
prelude's own types (`Scene`, `Camera`, `Frame`, …) are declared. So it is an
`.mli` whose implementation lives in the runtime, not in a sibling `.ml`.

## Syntax mapping

| OCaml | Functor Lang |
| --- | --- |
| `type model = { count : float }` | `type Model = { count: float }` |
| `type msg = Inc \| Dec` | `type Msg = \| Inc \| Dec` (leading `\|` required) |
| `match m with` | `match m with` (same) |
| `if c then a else b` | `match c with \| true => a \| false => b` |
| `{ m with count = n }` | `{ m with count: n }` (`:`, not `=`) |
| `let f x = ...` · `fun x -> ...` | `let f = (x) => ...` |
| `x +. y` (float ops) | `x + y` (one number type) |
| `List.map f xs` · `xs \|> List.map f` | `List.map(f, xs)` · `xs \|> List.map(f)` |
| `string_of_float n` | `Text.fromFloat(n)` |
| `.mli` interface | `.funi` interface (types host values; no paired `.fun`) |
| `module M = struct ... end` | file = module (one per file; no nesting) |

## What you'll miss

- **No functors (the OCaml kind).** There are no parametrized modules — no
  `module Make (M : S) = ...`, no first-class modules. Generics live at the type
  level instead (`type Box<'v>`, `List<'a>`) with let-polymorphism, but a module
  is only ever a file.
- **No nested modules.** A module is a file, full stop — no `module Inner =
  struct ... end`, no submodule paths beyond `File.name`.
- **Leaner pattern matching.** Patterns are minimal: constructor, tuple, name,
  `_`, and literals — no nested sub-patterns, no or-patterns, no `when` guards.
  Nest a `match` inside an arm and you parenthesize it (OCaml's convention).
- **No polymorphic variants, no objects.** Variants are nominal only — no
  `` `Tag `` structural variants — and there is no object layer (no classes,
  methods, or row types). Records and variants are the whole data story.
- **No labeled or optional arguments, and no currying.** Functions take fixed
  positional argument lists called saturated (`f(a, b)`) — no `~label:`, no `?opt`,
  no partial application. Bare constructors and function names are still
  first-class values.

## What you'll love

- **3D scenes as values.** `draw` returns a `Frame` — a `Camera` plus a scene
  graph you build with the same `|>` you already pipe with.
- **State-preserving hot reload.** Save and the runtime swaps your code under the
  live model; closures rebind to the edited code with their captured values
  carried over. No restart, no `dune build`.
- **Whole-game time travel.** Scrub the running game's timeline back and forth —
  pause, drag, single-step; every frame restores exactly as it was recorded. (Editing the program resets the
  recorded history.)
- **Gradual types.** The same HM inference you know, but annotations are optional
  and a type warning never blocks a run.
- **Zero toolchain.** The sandbox on this site *is* the runtime — no `opam`, no
  `dune`, no project file.

## Muscle-memory gotchas

- **No `if`/`else`.** OCaml has `if c then a else b`; Functor Lang's only
  conditional is a **bool-literal match**: `match c with | true => a | false => b`.
- **`|>` threads last, like OCaml's — but write saturated calls.** Your pipe habit
  carries over unchanged: the piped value lands last, just as `xs |> List.map f`
  puts the list last. The difference is currying — `x |> f(a)` lowers to `f(a, x)`,
  never a partial `f(a)`.
- **One number type, one `+`.** OCaml separates `+` and `+.`; here ints and floats
  are all `float` and every arithmetic operator is unsuffixed — `0` and `0.0` are
  the same value.
- **Records use `:`, not `=`.** It's `{ count: 0.0 }` and `{ m with count: n }` —
  the field separator is `:`, where OCaml writes `=`.
- **File = module, `.funi` ≈ `.mli`.** Every sibling `.fun` is a module named by
  its capitalized filename stem, and *all* of them load together. Qualified access
  needs no `open`; an `.funi` types host values with no paired `.fun` beside it.
