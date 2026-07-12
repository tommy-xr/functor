# Coming from Elm

If you write Elm, you already know Functor Lang's shape. A game here is
**Model–View–Update**: an `init` value, an `update` that folds messages into new
models, and a `draw` that turns the model into a description of what to show.
Messages are variants, effects are values the runtime performs and folds back
through `update`, and timers are declarative `subscriptions`. It is the Elm
architecture — pointed at a 3D scene instead of the DOM.

The differences are deliberate: Functor Lang is **interpreted, not compiled**, so
there is no build step and the runtime hot-swaps your code under the running
model. Types are **gradual** — real Hindley–Milner inference with
let-polymorphism, but annotations are optional and diagnostics are on demand.
And `draw` returns a `Frame` — a camera and a scene graph — not `Html`.

## The same game, side by side

A counter: two buttons dispatch messages, `update` folds them into the model,
and the view reflects it. On the left, Elm's `Browser.sandbox`; on the right,
the same loop in Functor Lang, drawing a sphere that grows with the count. Press
**▶ try it** to run the Functor version live.

<div class="compare-cols">

```elm
module Main exposing (main)

import Browser
import Html exposing (button, div, text)
import Html.Events exposing (onClick)


type alias Model =
    Int


type Msg
    = Increment
    | Decrement


init : Model
init =
    0


update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            model + 1

        Decrement ->
            model - 1


view : Model -> Html.Html Msg
view model =
    div []
        [ button [ onClick Decrement ] [ text "-" ]
        , text (String.fromInt model)
        , button [ onClick Increment ] [ text "+" ]
        ]


main =
    Browser.sandbox
        { init = init
        , update = update
        , view = view
        }
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

| Elm | Functor Lang |
| --- | --- |
| `type alias Model = { ... }` | `type Model = { ... }` |
| `type Msg = Inc \| Dec` | `type Msg = \| Inc \| Dec` (leading `\|` required) |
| `case msg of` | `match msg with` |
| `if c then a else b` | `match c with \| true => a \| false => b` |
| `{ model \| count = n }` | `{ model with count: n }` |
| `\x -> x + 1` | `(x) => x + 1.0` |
| `List.map f xs` · `xs \|> List.map f` | `List.map(f, xs)` · `xs \|> List.map(f)` |
| `String.fromInt n` | `Text.fromFloat(n)` (numbers are all `float`) |
| `let x = 1 in ...` | `let x = 1.0 in ...` |
| `module Main exposing (..)` | file = module (stem capitalized; no export list) |

## What you'll miss

- **No package ecosystem.** There is no `elm install`, no community packages —
  the standard library is small (`List.map`/`filter`/`fold`/`range`/`grid`/`maximum`,
  `Text.*`, `Math.*`). No built-in `Dict`, `Set`, `Maybe`, or `Result`; you
  declare the variant types you need.
- **No JSON decoders or ports.** Networking is a built-in (`Sub.connect` /
  `Sub.listen` deliver `Net` events, `Effect.send` writes), but there is no
  `Json.Decode` pipeline and no port system for arbitrary JS interop.
- **No `elm-format` culture** and no compiler that refuses to run on a type
  error — diagnostics are advisory, and a program with a type warning still
  loads and runs. Missing annotations are fine.

## What you'll love

- **3D scenes as values.** `draw` returns a `Frame` — a `Camera` plus a scene
  graph you build with the same `|>` you pipe lists with.
- **State-preserving hot reload.** Save and the runtime swaps your code under
  the live model; closures rebind to the edited code with their captured values
  carried over. The scene keeps running.
- **Whole-game time travel.** Scrub the running game's timeline back and forth —
  pause, drag, single-step; every frame restores exactly as it was recorded. (Editing the program resets the
  recorded history.)
- **Gradual types.** Full inference when you want it, silence when you don't —
  annotate a boundary, leave the rest.
- **Zero toolchain.** The sandbox on this site *is* the runtime. No install, no
  build.

## Muscle-memory gotchas

- **No `if`/`else`.** Elm has `if c then a else b`; Functor Lang's only
  conditional is a **bool-literal match**: `match c with | true => a | false => b`.
- **`|>` still threads last — but write saturated calls.** Your Elm habit
  carries over: the piped value lands as the last argument, exactly as
  `xs |> List.map f` puts the list last. The one change is the parentheses —
  `x |> f(a)` lowers to `f(a, x)`, never a partial `f(a)`.
- **Records update with `:`, not `=`.** It's `{ model with count: n }` — and the
  keyword is `with`, like Elm's `{ model | ... }` in spirit but spelled out.
- **File = module.** Every sibling `.fun` is a module named by its capitalized
  filename stem, and *all* of them load together — there is no `exposing` list.
  Qualified access (`Utils.clamp`) needs no import; `open Utils` brings names in
  bare.
- **One number type.** Ints and floats are all `float` — `0` and `0.0` are the
  same value, and there is no `String.fromInt`; it's `Text.fromFloat`.
