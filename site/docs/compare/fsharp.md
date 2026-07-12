# Coming from F#

Functor Lang is F#'s syntax cousin. It borrows the surface you already know —
`let` bindings and lambdas, `type` for records and discriminated unions, `match
... with` and `|>`, structural tuples, generics with `'a` type variables — and
Hindley–Milner inference underneath. Functor itself began as F# compiled through
Fable; the language grew out of that lineage. If you write F#, the code below
will read almost like home.

The differences are the interesting part, and they are head-on: Functor Lang is
**interpreted, not compiled** — no build step, and the runtime hot-swaps your
code under a running game. It is a **small** language: no .NET underneath, no
object programming, no computation expressions. Types are **gradual** — real
inference, but annotations optional and diagnostics advisory. And a program is
an Elm-style **Model–View–Update** loop whose `draw` returns a 3D `Frame`.

## The same game, side by side

A counter, as an MVU loop: messages fold through `update`, the view reflects the
model. On the left, idiomatic **Elmish** (F#'s MVU library — the closest
apples-to-apples); on the right, the same loop in Functor Lang, drawing a sphere
that grows with the count. Press **▶ try it** to run the Functor version live.

<div class="compare-cols">

```fsharp
module Counter

open Elmish

type Model = { Count: float }

type Msg =
    | Increment
    | Decrement

let init () : Model =
    { Count = 0.0 }

let update (msg: Msg) (model: Model) : Model =
    match msg with
    | Increment -> { model with Count = model.Count + 1.0 }
    | Decrement -> { model with Count = model.Count - 1.0 }

let view (model: Model) dispatch =
    div [] [
        button [ OnClick (fun _ -> dispatch Decrement) ] [ str "-" ]
        str (sprintf "%g" model.Count)
        button [ OnClick (fun _ -> dispatch Increment) ] [ str "+" ]
    ]

Program.mkSimple init update view
|> Program.run
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

## Syntax mapping

| F# | Functor Lang |
| --- | --- |
| `type Model = { Count: float }` | `type Model = { count: float }` |
| `type Msg = \| Inc \| Dec` | `type Msg = \| Inc \| Dec` (same) |
| `match msg with` | `match msg with` (same) |
| `if c then a else b` | `match c with \| true => a \| false => b` |
| `{ model with Count = n }` | `{ model with count: n }` (`:`, not `=`) |
| `let f x = x + 1` · `fun x -> x + 1` | `let f = (x) => x + 1.0` |
| `List.map f xs` · `xs \|> List.map f` | `List.map(f, xs)` · `xs \|> List.map(f)` |
| `string n` · `sprintf "%d" n` | `Text.fromFloat(n)` · `Text.fixed(n, 0.0)` |
| `x <- v` (mutable field) | `x := v` (in a `let mut ... in`) |
| `let x = 1 in ...` | `let x = 1.0 in ...` |

## What you'll miss

- **No .NET.** No BCL, no NuGet, no `System.*` — the standard library is small
  (`List`, `Text`, `Math`, `Debug`) and there is no interop layer beneath it.
- **No computation expressions.** No `async { }`, `seq { }`, `task { }`, or
  custom builders. Effects are plain values the runtime performs and folds back
  through `update`; timers are declarative `subscriptions`.
- **No units of measure, no type classes / interfaces, no object programming.**
  No `[<Measure>]`, no member methods, no inheritance — records and variants are
  the whole data story.
- **Leaner pattern matching.** Patterns are minimal: constructor, tuple, name,
  `_`, and literals, with no nested sub-patterns, no `when` guards, and no active
  patterns. Nest a `match` inside an arm and you parenthesize it (F#'s
  convention).
- **One number type and no currying.** Ints and floats are all `float`, and
  functions take fixed argument lists — you call them saturated (`f(a, b)`), not
  partially applied. Bare constructors and function names are still first-class
  values.

## What you'll love

- **3D scenes as values.** `draw` returns a `Frame` — a `Camera` plus a scene
  graph you build with the same `|>` you already pipe with.
- **State-preserving hot reload.** Save and the runtime swaps your code under
  the live model; closures rebind to the edited code with their captured values
  carried over. No restart.
- **Whole-game time travel.** Scrub the running game's timeline back and forth —
  pause, drag, single-step; every frame restores exactly as it was recorded. (Editing the program resets the
  recorded history.)
- **Gradual types.** The same HM inference you know, but annotations are optional
  and a type warning never blocks a run.
- **Zero toolchain.** The sandbox on this site *is* the runtime — no `dotnet`, no
  build, no project file.

## Muscle-memory gotchas

- **No `if`/`else`.** F# has `if c then a else b`; Functor Lang's only conditional
  is a **bool-literal match**: `match c with | true => a | false => b`.
- **`|>` threads last, like F#'s — but write saturated calls.** Your pipe habit
  carries over unchanged: the piped value lands last, just as `xs |> List.map f`
  puts the list last. The one difference is the parentheses — `x |> f(a)` lowers
  to `f(a, x)`, never a partial `f(a)`.
- **`:=`, not `<-`.** Local mutation uses `let mut x = ... in` and then
  `x := v;` with a continuation. F#'s `<-` is reserved here for future
  do-block binds — and `mut` slots can't be captured by a lambda.
- **Records use `:` in literals and updates.** It's `{ x: 0.0 }` and
  `{ model with count: n }` — the field separator is `:`, where F# writes `=`.
- **File = module.** Every sibling `.fun` is a module named by its capitalized
  filename stem, and *all* of them load together (there is no explicit signature
  or compile-order list). Qualified access needs no `open`; `open Utils` adds
  the bare names.
