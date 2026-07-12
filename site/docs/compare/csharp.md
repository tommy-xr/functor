# Coming from C#

This is the biggest jump on this page — so start with what the loop *feels* like.
In Unity-style C#, a change means edit → recompile → domain reload: the editor
tears down your play session, reloads the assemblies, and hands you back a fresh
scene with your live state gone. In Functor Lang you edit and **the runtime
hot-swaps your code under the running model, mid-frame** — the sphere keeps
spinning, the count keeps its value, nothing reloads. On top of that the whole
session is recorded, so you can **scrub the running game's timeline back and
forth** deterministically — pause, drag, single-step — with no test harness.

The paradigm is the other jump. Functor Lang has **no classes, no inheritance, no
`null`**. A game is plain **values** and **pure functions**: an `init` value, an
`update` that folds messages into new models, and a `draw` that *derives* the
scene from the model instead of mutating objects each frame. Data is **records**
and **variants**, not objects — and the whole thing is an Elm-style
**Model–View–Update** loop. Types are **gradual** Hindley–Milner: real inference,
annotations optional.

## The same game, side by side

A counter. On the left, a Unity `MonoBehaviour` that mutates its transform every
frame; on the right, the same loop in Functor Lang — the count lives in the model,
and `draw` derives the sphere's scale from it (no mutation). Press **▶ try it** to
run the Functor version live.

<div class="compare-cols">

```csharp
using UnityEngine;

public class Counter : MonoBehaviour
{
    int count = 0;

    void Update()
    {
        if (Input.GetKeyDown(KeyCode.UpArrow)) count++;
        if (Input.GetKeyDown(KeyCode.DownArrow)) count--;
        transform.localScale = Vector3.one * (1f + count * 0.15f);
    }
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

The shape is the same, but the mechanism is inverted. Unity **mutates** a field
and a transform in place each frame; Functor Lang keeps the count in an immutable
model and `draw` **derives** the scene from it. That single change — deriving the
frame instead of mutating objects — is what makes hot reload and time travel work.

## Syntax mapping

| C# (Unity) | Functor Lang |
| --- | --- |
| `record Model(float Count)` · `class` | `type Model = { count: float }` |
| `enum Msg { Inc, Dec }` · sealed hierarchy | `type Msg = \| Increment \| Decrement` |
| `switch (msg) { case Inc: ... }` | `match msg with \| Increment => ...` |
| `if (c) a; else b;` · `c ? a : b` | `match c with \| true => a \| false => b` |
| `count++;` (mutation) | `{ m with count: m.count + 1.0 }` (new value) |
| `x => x + 1` | `(x) => x + 1.0` |
| `list.Select(f)` (LINQ) | `List.map(f, list)` · `xs \|> List.map(f)` |
| `n.ToString()` | `Text.fromFloat(n)` |
| `null` | a variant case (there is no `null`) |
| `void Update()` (mutate each frame) | `tick` / `draw` (pure functions of `model`) |

## What you'll miss

- **No classes, no inheritance, no interfaces.** There is no object layer at all —
  no `class`, `new`, `this`, methods-on-data, or subtype hierarchies. Records and
  variants are the whole data story; generics live at the type level
  (`type Box<'v>`, `List<'a>`).
- **No `null`, and that's the point.** There is no `null` and no nullable
  reference — so no `NullReferenceException`. Model absence with a variant (declare
  your own `Option`-shaped type) and the checker forces you to handle both cases.
- **No .NET / BCL / NuGet.** The standard library is small (`List`, `Text`,
  `Math`, `Debug`) with no `System.*` beneath it and no package manager.
- **No LINQ, no `for`/`while` loops.** There is no `IEnumerable` query syntax and
  no loop statements — you iterate with `List.map` / `filter` / `fold`. Local
  mutation exists (`let mut x = ... in`, then `x := v;`) but can't be captured by a
  closure, and there is no top-level mutable state.
- **No `async`/`await` or `Task`.** Effects are plain values the runtime performs
  and folds back through `update`; timers are declarative `subscriptions`.

## What you'll love

- **No recompile, no domain reload.** Save and the runtime swaps your code under
  the live model in about one frame — the scene keeps running and its state is
  preserved. Compare the Unity round-trip of edit → recompile → domain reload →
  lost play state.
- **Whole-game time travel.** Scrub the running game's timeline back and forth,
  deterministically — pause, drag, single-step. No record-and-replay harness to
  set up; it's built into the runtime.
- **No null reference exceptions.** Absence is a variant you match on, checked at
  the boundary — the entire class of `NullReferenceException` is gone.
- **3D scenes as values.** `draw` returns a `Frame` — a `Camera` plus a scene
  graph — that you build by piping, and that is a pure function of the model.
- **Gradual types.** Full Hindley–Milner inference when you want it, silence when
  you don't — and a type warning never blocks a run. No build, no project file.

## Muscle-memory gotchas

- **No `if`/`else`, no ternary.** C# has `if (c) ... else ...` and `c ? a : b`;
  Functor Lang's only conditional is a **bool-literal match**:
  `match c with | true => a | false => b`.
- **No `for`/`while`.** There are no loop statements — iterate with `List.map`,
  `List.filter`, and `List.fold` (its callback is `(acc, x) => ...`).
- **Values, not objects.** No `new`, no `this`, no methods hanging off data. You
  don't mutate a transform each frame — `draw` derives the whole scene from the
  model, and `update` returns a *new* model rather than mutating fields.
- **`==` is structural.** Like a C# `record`'s `==` (and unlike class reference
  equality), `==` compares by value — `{ count: 1.0 } == { count: 1.0 }` is `true`.
- **One number type.** `int`, `float`, and `double` all collapse to one `float` —
  `0` and `0.0` are the same value, and `n.ToString()` becomes `Text.fromFloat(n)`.
