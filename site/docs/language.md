# Language reference

This is the whole of Functor Lang — the interpreted, F#-inspired language you write your
game logic in. It is deliberately small: if a construct isn't on this page, it doesn't
parse. Don't reach for `if`/`else`, loops, or a string-concat operator out of habit —
none exist, and the sections below give you what replaces each one.

You can check any snippet here without a GPU. The pure language runs headless:

```sh
cargo run -q -p functor-lang -- check file.fun    # typecheck: every diagnostic, exits 1 on error
cargo run -q -p functor-lang -- run file.fun      # evaluate main() (or the entry's bindings)
```

`check`/`run` treat the file as a **project entry**: every sibling `.fun` in its directory
loads with it (see [Modules](#modules)), so keep scratch files in their own directory.
Anything that calls the engine prelude (`Scene.*`, `Camera.*`, …) only resolves under the
runner — those snippets carry a **▶ try it** button that runs them live in the
[sandbox](/sandbox.html).

## The game contract

A runner-hosted game defines these top-level bindings:

<table>
<thead><tr><th>binding</th><th>shape</th><th></th></tr></thead>
<tbody>
<tr><td><code>init</code></td><td><code>{ … }</code></td><td>the initial model — a <em>value</em>, not a function</td></tr>
<tr><td><code>tick</code></td><td><code>(model, dt, tts) => model'</code></td><td>per-frame step; <code>dt</code> seconds since last frame, <code>tts</code> total time</td></tr>
<tr><td><code>draw</code></td><td><code>(model, tts) => Frame</code></td><td>pure frame description</td></tr>
<tr><td><code>input</code></td><td><code>(model, key, isDown) => model'</code></td><td>optional; <code>key</code> is <code>"W"</code>, <code>"Up"</code>, <code>"Space"</code>, … — key <em>repeats</em> arrive as <code>isDown = true</code>, so latch if you need edges</td></tr>
<tr><td><code>mouseMove</code></td><td><code>(model, x, y) => model'</code></td><td>optional; window pixels</td></tr>
<tr><td><code>mouseWheel</code></td><td><code>(model, delta) => model'</code></td><td>optional</td></tr>
<tr><td><code>update</code></td><td><code>(model, msg) => model'</code></td><td>optional; messages are your variant values</td></tr>
<tr><td><code>subscriptions</code></td><td><code>(model) => Sub</code></td><td>optional (requires <code>update</code>); declarative timers</td></tr>
<tr><td><code>physics</code></td><td><code>(model) => Physics.scene(…)</code></td><td>optional; declarative bodies, reconciled each frame</td></tr>
<tr><td><code>ui</code></td><td><code>(model) => Ui…</code></td><td>optional; the 2D HUD, drawn over the frame</td></tr>
<tr><td><code>soundScape</code></td><td><code>(model) => AudioScene…</code></td><td>optional; continuous looping voices, reconciled by key</td></tr>
</tbody>
</table>

The model-returning entry points (`tick`, `input`, `mouseMove`, `mouseWheel`, `update`)
may instead return a 2-tuple `(model', effect)` to issue one-shot
[effects](#time-subscriptions-effects). `init` is a plain value — an effect in it is
rejected at load.

**Frame order:** subscriptions → `update` → `tick` → physics (fixed-step 60&nbsp;Hz) →
`draw`. Physics reads in `draw` see *this* frame's stepped world; reads in `tick` see the
previous frame's — so on the very first frame declared bodies don't exist yet. Keep
physics reads in `draw`.

Here is a whole game — model, step, view — in three functions. Press **▶ try it**, then
edit a number and watch it hot-reload with the model preserved:

```functor run
// a spinning cube — the smallest complete game
let init = { spin: 0.0 }

let tick = (model, dt, tts) => { model with spin: model.spin + dt }

let draw = (model, tts) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),
    Scene.cube()
      |> Scene.emissive(1.0, 0.2, 0.8)
      |> Scene.rotateY(Angle.radians(model.spin)))
```

## Values and bindings

```functor
let threshold = 10.0        // every number is a float (f64) — no separate int
let name = "neon\n"         // strings: escapes \" \\ \n \t
let on = true               // bools: true / false
let origin = { x: 0.0, y: 0.0 }   // a record literal
let scores = [1.0, 2.0, 3.0]      // a list literal
```

Line comments only (`//`). The three primitive types are written lowercase — `float`,
`string`, `bool` — and they are the names the typechecker knows (more in
[Typechecking](#typechecking)). Top-level definitions are mutually visible inside function
bodies, and late-bound — that's the hot-reload seam — but a top-level *initializer* may
only use names defined above it.

## Functions

```functor
let area = (w: float, h: float): float => w * h   // annotations optional
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))
let main = () => describe(area(2.0, 3.0))          // zero-param main is run's entry point
```

A function is a `(params) => body` lambda; the body is a single expression. Parameter and
return annotations are **optional** — typing is gradual, so unannotated code is inferred
(see [Typechecking](#typechecking)). Recursion depth is capped (~200); deep iteration
belongs in `List.*`, not hand-rolled recursion.

## The conditional

There is no `if`/`else`. The conditional is a **bool-literal match** — you match the
boolean on the two literal arms:

```functor
let sizeOf = (n: float): string =>
  match n > 10.0 with
  | true => "big"
  | false => "small"
```

`match` is the workhorse of the language: every branch, every destructure goes through it.
Each arm is `| pattern => expression`; the leading `|` is required before *every* arm.

## Records

```functor
type Position = { x: float, y: float }        // a record type; nominal in annotations

let p = { x: 0.0, y: 1.0 }                    // a record literal
let nudged = { p with x: p.x + 1.0 }          // update: named fields must already exist
```

Records are nominal in annotations but resolve structurally at the literal: `{ x: 0.0, y:
0.0 }` is inferred to be the unique declared type with exactly that field set (see
[Typechecking](#typechecking)). Field names are unique within a record; the `with` form
copies a record and overrides listed fields (which must exist). Prefer named records for
anything that outlives a single expression.

## Variants and match

```functor
type Shape =
  | Circle(radius: float)     // leading | required — first alternative too
  | Rect(w: float, h: float)
  | Point                     // nullary: no parens, ever

let c = Circle(2.0)           // constructors are CALLED positionally
let shapes = [c, Rect(3.0, 4.0), Point]   // bare Point IS the value

let area = (s: Shape): float =>
  match s with
  | Circle(r) => 3.14 * r * r // ctor patterns bind fields positionally
  | Rect(w, _) => w * w       // sub-patterns are names or _ only (no nesting)
  | Point => 0.0
```

Variant types (ADTs) are declared with leading `|` on every alternative, the first
included. Constructors resolve **bare** and live in the value namespace: you write
`Circle(2.0)`, never `Shape.Circle(2.0)`. Because they're bare, constructor names must be
unique across all variant types in a module. Nullary constructors never take parens — bare
`Point` is the value. Unapplied constructors are first-class, so `xs |> List.map(Circle)`
maps each element through `Circle`.

## Pattern matching

Patterns are intentionally minimal. The full set:

- constructor: `Circle(r)`, `Rect(w, _)` — binds fields positionally
- nullary constructor: `Point`
- tuple: `(x, _)` — matched by **exact** arity
- list: `[]`, `[a, b]`, `[head, ..tail]` (see [Lists](#lists))
- a bare name (binds anything) or `_` (matches, binds nothing)
- literals: `true`, `false`, numbers (including negative), strings — an equality match

Constructor and tuple sub-patterns are names or `_` **only** — no nesting, no literals
inside them. The first matching arm wins; if no arm matches, that's a spanned runtime
error. When the scrutinee's type is known, `match` is checked for exhaustiveness — a
missing constructor is a real error:

```functor
type Shape = | Circle(radius: float) | Square(side: float)
// REJECTED — `match on Shape is not exhaustive: missing Square`
let area = (s: Shape): float =>
  match s with
  | Circle(r) => 3.14 * r * r
```

Number- and string-literal arms are *not* exhaustive on their own, so they need a catch-all
(`| _ =>` or a bare-name arm); `true` + `false` together *are* exhaustive.

```functor
let grade = (n: float): string =>
  match n with
  | 0.0 => "none"
  | 1.0 => "one"
  | _ => "many"        // literal arms need a catch-all
let main = () => grade(2.0)
```

A pattern whose type can never match the scrutinee is caught at check time — a foreign
literal arm against a variant scrutinee is a can-never-match error:

```functor
type Shape = | Circle(radius: float) | Point
// REJECTED — `pattern matches string, but the scrutinee is Shape`
let describe = (s: Shape): string =>
  match s with
  | Circle(r) => "circle"
  | "hello" => "nope"
  | Point => "point"
```

**Arms are greedy.** Arm bodies are full expressions, so a nested `match` inside an arm
swallows the following `|` arms as its own. Parenthesize the inner match:

```functor
let classify = (a: float, b: float): string =>
  match a > 0.0 with
  | true => (match b > 0.0 with | true => "both" | false => "a")   // parenthesized
  | false => "neither"
let main = () => classify(1.0, -1.0)
```

## Lists

A list is a homogeneous sequence written `[a, b, c]`. `[x, ..xs]` prepends `x` (cons):

```functor
let empty = []
let one = [1.0]
let more = [0.0, ..one]     // [0.0, 1.0]
```

Lists are consumed with the `List.*` builtins (`map`/`filter`/`fold`/…, all subject-last —
see [Pipelines](#pipelines)) and destructured with list patterns. `[]` matches the empty
list, `[a, b]` a two-element list by exact length, and `[head, ..tail]` peels the first
element off a non-empty list. The recursive shape is refutable, so it needs a catch-all —
here `[]`:

```functor
let sumList = (xs: List<float>): float =>
  match xs with
  | [] => 0.0
  | [head, ..rest] => head + sumList(rest)
let main = () => sumList([1.0, 2.0, 3.0])     // 6
```

## Tuples

```functor
let minMax = (a: float, b: float): (float, float) =>   // tuple TYPE is (A, B)
  match a < b with
  | true => (a, b)            // a tuple VALUE is (e1, e2, …); (e) is grouping, not a 1-tuple
  | false => (b, a)

let span = (a, b) =>
  let (lo, hi) = minMax(a, b) in   // destructuring let — sugar for a single-arm match
  hi - lo
let main = () => span(5.0, 2.0)
```

Tuples are structural and start at two elements: `(1.0, "a") == (1.0, "a")`. A tuple **type**
annotates as `(float, string)` and a function **type** as `(A, B) => C` / `() => C`;
`(A)` in a type is grouping, not a one-tuple. Tuples are for multiple returns — prefer a
named record for anything that outlives an expression.

## Pipelines

`|>` **appends** the piped value: `x |> f(a)` is exactly `f(a, x)` (thread-last, F#/Elm
style). Every builtin and prelude function therefore takes its subject — the list, the
scene, the body — as its **last** argument, so it threads cleanly:

```functor
let threshold = 10.0
let isHigh = (score: float): bool => score > threshold
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))

let report = (scores) =>
  scores
    |> List.filter(isHigh)    // == List.filter(isHigh, scores)
    |> List.map(describe)
    |> Text.toBullets
let main = () => report([12.0, 3.5, 40.0])
```

Because `|>` is syntax, `x |> f(a)` lowers directly to the saturated call `f(a, x)` — never
a partial `f(a)` — so pipelines allocate nothing.

## Local mutation

```functor
let sum3 = (a, b, c) =>
  let mut acc = a in          // a rebindable slot; expression let-in
  acc := acc + b;             // assignment is := and carries a continuation
  acc := acc + c;
  acc
let main = () => sum3(1.0, 2.0, 3.0)
```

`mut` marks a **local, rebindable** slot. Assignment is `:=` (never `<-`, which is reserved)
and must be followed by `;` and a continuation expression. The rules that bite:

- No top-level `let mut` — `mut` is function-local only.
- A lambda may **not** read or assign an enclosing `mut` slot. Capturing one is a compile
  error:

```functor
// REJECTED — `a function cannot capture the mutable binding acc`
let f = (n: float) =>
  let mut acc = 0.0 in
  let g = () => acc := acc + n; acc in
  g()
```

Params, globals, and plain `let`s are always immutable.

## Operators and equality

`+ - * /`, `< > ==`, and unary `-`; conventional precedence, with pipelines binding
loosest. `==` is **structural** (comparing two functions is a runtime error). Division is
IEEE — `1.0 / 0.0` is `inf` — but the engine boundary rejects non-finite numbers, so a
`NaN` reaching a transform is an error. There is no `&&`/`||`; branch with a bool-literal
`match`. There is no string-concatenation operator — use `Text.concat`.

## Generics and abstract types

Type declarations can take **type parameters**, written with a leading apostrophe (`'v`).
They substitute through fields and patterns, and each use instantiates fresh, so
`Box<float>` and `Box<string>` coexist:

```functor
type Box<'v> =
  | Full(value: 'v)
  | Empty
type Pair<'x, 'y> = { first: 'x, second: 'y }

let unwrap = (b: Box<float>, dflt: float): float =>
  match b with
  | Full(v) => v
  | Empty => dflt
let main = () => unwrap(Full(3.0), 0.0)       // 3
```

Functions are generic through **let-polymorphism**: an apostrophe-prefixed annotation name
is a type variable, and the function instantiates fresh at every call site. Element types
flow through `List.map`/`filter`/`fold` the same way.

```functor
let firstOf = (p: Pair<'a, 'b>): 'a => p.first
let main = () => firstOf({ first: 1.0, second: "label" })   // 1
```

A declaration with no `= body` is an **abstract type**: an opaque nominal with no fields and
no constructor. Its values are made by host code; you name it only in annotations. This is
how engine handles (`Camera.t`, `Physics.body`, …) are typed — and how you'd declare your
own host-backed handle in a [`.funi`](#interface-files-funi):

```functor
type Handle                       // opaque — no fields, no constructor
let size = (h: Handle): float => 1.0
let main = () => 0.0
```

One current limit: **function types can't be written in annotations yet** — `f: ('a) => 'b`
does not parse. Leave higher-order parameters unannotated and let inference type them. An
*undeclared* type variable inside a declaration (a `'z` that isn't a parameter) is a
teaching error.

## Modules

**File = module.** Every `.fun` file in the entry file's directory *is* a module, named by
its filename stem with the first letter capitalized (`utils.fun` → module `Utils`). The
entry file (the `functor.json` `entry`, default `game.fun`) is the program root. Loading is
**eager and whole-program**: all sibling `.fun` files load, check, and evaluate together —
an unreferenced, broken, or stray scratch sibling still counts, so keep scratch files
elsewhere. File stems must be identifiers (`pure_pipeline.fun`, not `pure-pipeline.fun`).

```functor
// utils.fun  →  module Utils
type Shape = | Circle(radius: float) | Point
let tau = 6.28
let area = (s: Shape): float =>
  match s with
  | Circle(r) => 3.14 * r * r
  | Point => 0.0
```

```functor
// game.fun  (the entry)
open Utils                                    // bring Utils' names in unqualified

let a = area(Circle(2.0)) + tau               // via the open…
let b = Utils.area(Utils.Circle(2.0))         // …or QUALIFIED — no open needed
let biggest = (shapes: List<Utils.Shape>) =>  // qualified types in annotations
  shapes |> List.map(area) |> List.maximum
```

- **Qualified access needs no import.** `Utils.area(x)`, `Utils.Circle(…)` (expressions and
  patterns, first-class when unapplied), and `Utils.Shape` in annotations all work with no
  `open`. `open Utils` merely *adds* unqualified access; a name collision it introduces is
  a load error naming both sides — qualify instead. `open` is contextual, so it stays a
  valid binding name elsewhere.
- **Constructor names are unique per module** (not per project). Values from a non-entry
  module display with their canonical tag (`Utils.Circle(2)` in `run`/`trace` output); the
  entry's own names stay bare, so a single-file project behaves exactly as one file.
- **Cross-file cycles are refused** — any cross-file reference (a qualified use, an `open`,
  even a type annotation) is a dependency edge, and `Game → Utils → Game` is a load error.
  Within one file, mutual recursion is unchanged.
- **Protected namespaces.** A file whose module name would collide with a builtin/prelude
  namespace — `Net`, `List`, `Text`, `Math`, `Debug`, `Scene`, `Anim`, `Camera`, `Frame`,
  `Light`, `Fog`, `Skybox`, `Angle`, `Texture`, `Time`, `Sub`, `Effect`, `Physics`,
  `RenderTarget`, `Ui`, `AudioSource`, `AudioScene` — is a load error. Rename the file.
- **`Net` is a built-in module**, always in scope: its `NetEvent` variants (`Connected`,
  `Message`, `Disconnected`, `Error`) arrive from `Sub.connect`/`Sub.listen` taggers and
  match as `Net.Connected(id)` with no declaration.
- **Hot reload watches every project file** — editing, adding, or removing any `.fun`
  reloads with the model preserved.

One current limit: `run wasm` and the VSCode live preview interpret a **single** source
text, so multi-file projects are native-only for now.

## Interface files (`.funi`)

A sibling `.funi` is an **interface module**: it declares types and **bodyless value
signatures** for values the *host runtime* implements. A module is either a `.fun` or a
`.funi`, never both (same-stem files are a load error), so there's no paired implementation
file. Bodies are forbidden in a `.funi`; a bodyless `let` is forbidden in a `.fun`.

```functor
// widget.funi  →  interface module Widget
type Handle                                 // abstract type (opaque; host-made)
let make : () => Handle                     // bodyless SIGNATURE: `let name : Type`, no = body
let size : (Handle) => float
```

```functor
// game.fun
let area = (h: Widget.Handle): float => Widget.size(h)   // typed by widget.funi
let main = () => 0.0
```

The signatures give the checker real types for what would otherwise be `Unknown` host
values (`Widget.make()` is a `Widget.Handle`, not `Unknown`), and mismatches are caught;
they surface in hover / inlay / codelens like any type. **The runtime is unchanged** — an
interface member stays a host-provided external at run time, so `.funi` is a pure
check-time overlay. This is exactly how the **engine prelude's types are declared**: the
prelude ships a `.funi` for every host namespace (`Scene`, `Camera`, `Frame`, `Light`, …),
so engine calls carry real types instead of `Unknown`.

## Typechecking

`functor-lang check` runs **real inference** — Hindley–Milner unification with
let-polymorphism, over gradual seams. Unannotated code gets full types: generic functions
instantiate fresh at every use, element types flow through `List.map`/`filter`/`fold`, and
apostrophe-prefixed annotation names are type variables. Inference has teeth — these are all
*errors*, with no annotation needed:

```functor
let mixed = [1.0, "two", 3.0]      // REJECTED — list element: expected float, got string
```

```functor
let oops = 1.0 + "x"               // REJECTED — `+` needs float operands, got string
```

The gradual part: **`Unknown`** remains only at genuinely dynamic seams — host values and
unrecognized type names — and it absorbs anything. That's why the primitive types are the
lowercase `float` / `string` / `bool`: those are the names inference knows. A capitalized
`Float` is simply an unrecognized type name, so it degrades to `Unknown` and buys you no
checking — reach for the lowercase names. In hovers and codelenses, `Unknown` is what you
see wherever a type genuinely couldn't be pinned down.

A few rules worth internalizing:

- **Record literals resolve nominally**, F#-style: a literal takes the unique declared type
  with exactly its field set. No declared match is fine (it stays anonymous data); two
  same-shaped declarations make a bare literal ambiguous — annotate it.
- **A `match`'s patterns constrain its scrutinee.** The first constructor arm pins the
  variant type; a foreign-literal arm is a can-never-match error; exhaustiveness checks all
  constructors (or `true`+`false`, or a catch-all); and all arm results must agree in type.
- **A `mut` slot's type fixes at its initializer.**
- **Function types can't be annotated yet** (`f: ('a) => 'b` doesn't parse) — leave
  higher-order params unannotated.

Diagnostics are **advisory in the live dev loop** — a program with type errors still loads,
hot-swaps, and runs, so you can iterate through a red squiggle. They become **strict in
`functor build`**, where every diagnostic is an error and a dirty typecheck fails the build.

## The engine prelude

These namespaces resolve only under the **runner host** (the CLI and the sandbox) — not in a
bare `functor-lang run`. Coordinates are **Y-up, right-handed**: +Y up, +X right, the ground
in the XZ plane. Branded values — `Angle`, `Time`/`Duration`, `Fog`, `Skybox`, render
targets — refuse bare numbers and strings with a teaching error: pass `Angle.degrees(60.0)`,
not `60`.

Here is a lit scene using the prelude end to end — **▶ try it**:

```functor run
let init = {}
let tick = (m, dt, tts) => m

let draw = (m, tts) =>
  Frame.createLit(
    Camera.lookAt(5.0, 4.0, -7.0, 0.0, 0.5, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(20.0) |> Scene.lit(0.35, 0.38, 0.42),
      Scene.sphere()
        |> Scene.translate(0.0, 0.6, 0.0)
        |> Scene.lit(0.3, 0.7, 1.0),
    ]),
    [
      Light.ambient(0.12, 0.12, 0.16),
      Light.directional(0.5, -1.0, 0.35, 1.0, 0.96, 0.9, 1.0)
        |> Light.castShadows,
    ])
```

### Scene

<table>
<tbody>
<tr><td><code>Scene.cube() / sphere() / cylinder() / quad() / plane()</code></td><td>primitives (zero args, enforced); <code>plane</code> lies in XZ (ground), <code>quad</code> in XY — a quad's front is +Z</td></tr>
<tr><td><code>Scene.model("shark.glb")</code></td><td>a glTF model by path (relative to the game dir); a missing file logs an error and renders empty</td></tr>
<tr><td><code>Scene.group([scene, …])</code></td><td>compose</td></tr>
<tr><td><code>scene |> Scene.color(r, g, b)</code></td><td>flat unlit color</td></tr>
<tr><td><code>scene |> Scene.lit(r, g, b)</code></td><td>diffuse + specular (needs lights)</td></tr>
<tr><td><code>scene |> Scene.emissive(r, g, b)</code></td><td>unlit glow</td></tr>
<tr><td><code>scene |> Scene.translate(x, y, z)</code></td><td rowspan="4">transforms wrap outward: the <em>outer</em> call applies last in world space — <code>s |> rotateY(r) |> translate(x, 0.0, 0.0)</code> rotates in place, then moves</td></tr>
<tr><td><code>scene |> Scene.rotateX/Y/Z(angle)</code></td></tr>
<tr><td><code>scene |> Scene.scale(k)</code></td></tr>
<tr><td><code>scene |> Scene.scaleXYZ(x, y, z)</code></td></tr>
<tr><td><code>scene |> Scene.animate(anim)</code></td><td>set an <code>Anim</code> pose on the model(s) in the subtree — see <em>Animation</em> below</td></tr>
</tbody>
</table>

Angles are branded values: `Angle.degrees(60.0)` / `Angle.radians(1.57)` — never a bare
number.

### Camera, lights, frames

<table>
<tbody>
<tr><td><code>Camera.lookAt(ex, ey, ez, tx, ty, tz)</code></td><td>up = +Y, fov 45°</td></tr>
<tr><td><code>Camera.firstPerson(ex, ey, ez, yaw, pitch, fov)</code></td><td>all three are Angles; yaw 0 / pitch 0 looks down +Z</td></tr>
<tr><td><code>Light.ambient(r, g, b)</code></td><td></td></tr>
<tr><td><code>Light.directional(dx, dy, dz, r, g, b, intensity)</code></td><td><code>|> Light.castShadows</code> for a shadowed key light</td></tr>
<tr><td><code>Light.point(px, py, pz, r, g, b, intensity, range)</code></td><td></td></tr>
<tr><td><code>Light.spot(px, py, pz, dx, dy, dz, r, g, b, intensity, range, coneAngle)</code></td><td>cone from a position along a direction; <code>coneAngle</code> is an Angle; <code>|> Light.castShadows</code></td></tr>
<tr><td><code>Frame.create(camera, scene)</code></td><td>what <code>draw</code> returns</td></tr>
<tr><td><code>Frame.createLit(camera, scene, [light, …])</code></td><td>lit + shadowed</td></tr>
</tbody>
</table>

### Animation

Skinned glTF clips are posed explicitly — the engine owns no animation clock, so you derive
the playhead from `tts` or model state, and poses rewind and replay exactly.

<table>
<tbody>
<tr><td><code>Anim.clip("walk", playheadSeconds)</code></td><td>a named clip at an explicit playhead (loops by the clip's duration; an unknown name warns once and renders the bind pose)</td></tr>
<tr><td><code>Anim.blend([(anim, weight), …])</code></td><td>weighted pose mix (weights normalize)</td></tr>
<tr><td><code>Anim.rest()</code></td><td>the bind pose — the base for programmatic posing</td></tr>
<tr><td><code>anim |> Anim.add(layerAnim, weight)</code></td><td>additive layer on top of <code>anim</code></td></tr>
<tr><td><code>anim |> Anim.mask(["joint", …])</code></td><td>restrict influence to the named joints' subtrees</td></tr>
<tr><td><code>anim |> Anim.rotate("joint", ax, ay, az)</code></td><td>additive local rotation on one joint (Angles)</td></tr>
</tbody>
</table>

### Time, subscriptions, effects

<table>
<tbody>
<tr><td><code>Time.seconds(1.0) / Time.millis(500.0)</code></td><td>Durations are branded, like Angles</td></tr>
<tr><td><code>Sub.every(duration, Msg)</code></td><td>stateless global time grid: a long frame fires once; timers tick through hot reload</td></tr>
<tr><td><code>Sub.none() / Sub.batch([sub, …])</code></td><td></td></tr>
<tr><td><code>Sub.connect(url, tagger) / Sub.listen(addr, tagger)</code></td><td>persistent connections; the tagger takes a <code>Net.NetEvent</code></td></tr>
<tr><td><code>Effect.random(Tagger) / Effect.now(Tagger)</code></td><td>one-shots; the tagger is <code>(float) => Msg</code> — <code>random</code> gives [0,1), <code>now</code> epoch seconds</td></tr>
<tr><td><code>Effect.none() / Effect.batch([fx, …])</code></td><td>return <code>(model', effect)</code> from any entry point</td></tr>
</tbody>
</table>

Effects are one-shot commands. The producer performs each, applies its tagger to the result
(`Effect.random(Rolled)` → the message `Rolled(0.42)`), and folds the message back through
`update` — which may itself return more effects, drained same-frame to a fixed point.
Taggers must be functions; `Effect.now(3.0)` is a construction-time error. Subscription
timers are **stateless**: `Sub.every` fires when an integer multiple of its period lies in
`(prevTts, tts]`, so a long frame fires exactly once. `Sub.every(0.5, …)` is a teaching
error — pass a `Time` value: `Sub.every(Time.seconds(0.5), Beat)`.

### Render targets

Declare a target **once** and use the value at both sites — the writer that renders into it
and the reader that shows it — never a bare string:

<table>
<tbody>
<tr><td><code>RenderTarget.named("id") |> RenderTarget.sized(w, h)</code></td><td>an offscreen texture (512×512 unless sized)</td></tr>
<tr><td><code>frame |> Frame.withRenderTarget(target, targetFrame)</code></td><td>writer: renders a full frame (own camera + lights) into the target before the main pass</td></tr>
<tr><td><code>scene |> Scene.screen(target)</code></td><td>reader: an emissive screen showing the target</td></tr>
</tbody>
</table>

A scene sampling its own target sees last frame's image. The *Render targets* example in the
sandbox is the reference.

### Physics

<table>
<tbody>
<tr><td><code>Physics.box(w, h, d) / sphere(r) / capsule(halfH, r)</code></td><td>shapes; box takes <em>full</em> extents</td></tr>
<tr><td><code>Physics.dynamic("tag", shape)</code></td><td>simulated; also <code>kinematic</code> / <code>fixed</code></td></tr>
<tr><td><code>body |> Physics.at(x, y, z)</code></td><td>spawn pose; also <code>velocity</code>, <code>mass</code>, <code>friction</code>, <code>restitution</code>, <code>sensor</code></td></tr>
<tr><td><code>Physics.scene(gx, gy, gz, [body, …])</code></td><td>what the <code>physics</code> hook returns</td></tr>
<tr><td><code>Physics.position("tag")</code></td><td><code>{x, y, z}</code> of the live body</td></tr>
<tr><td><code>scene |> Physics.transformed("tag")</code></td><td>draw a scene at the body's live pose</td></tr>
<tr><td><code>Physics.applyImpulse / applyForce / setVelocity / teleport("tag", x, y, z)</code></td><td>command effects (no tagger; observe via the reads)</td></tr>
<tr><td><code>Physics.raycast(ox,oy,oz, dx,dy,dz, maxDist, tagger)</code></td><td>a query effect; the tagger gets a hit record</td></tr>
<tr><td><code>Physics.events(tagger)</code></td><td>a Sub of contact begin/end</td></tr>
</tbody>
</table>

The tag is cross-frame identity: the same tag is the same body; drop a body by not
declaring it. Re-declaring an unchanged body leaves the simulation alone; changing its
declared position teleports it. Reading a tag your `physics` hook doesn't declare is a
runtime error. The physics world survives hot reload, like the model.

### HUD and audio

<table>
<tbody>
<tr><td><code>Ui.text("line") / Ui.textColor(r, g, b, "line")</code></td><td>HUD text (monospace)</td></tr>
<tr><td><code>Ui.column([view, …]) / Ui.row([view, …])</code></td><td>stack vertically / horizontally</td></tr>
<tr><td><code>view |> Ui.panel(Ui.topLeft())</code></td><td>pin to a corner (anchors are branded values)</td></tr>
<tr><td><code>Ui.button("label", msg) / Ui.slider(min, max, value, tagger) / Ui.textInput(value, tagger)</code></td><td>interactive widgets — a click/drag/edit folds a message through <code>update</code></td></tr>
<tr><td><code>AudioSource.ambient(key, sound) / AudioSource.at(key, sound, x, y, z)</code></td><td><code>soundScape</code> voices, reconciled by key</td></tr>
<tr><td><code>AudioScene.create([source, …]) / AudioScene.empty()</code></td><td>what <code>soundScape</code> returns</td></tr>
</tbody>
</table>

Engine values (`<Scene>`, `<Camera>`, `<Frame>`, …) are opaque: pass them around, but you
can't inspect, compare, or serialize them.

## Builtins

The pure builtins are always available (no host needed). Data comes **last**, so they
pipeline (F#/Elm style). This is the most-used slice; the
[`functor-lang` skill](https://github.com/tommy-xr/functor/blob/main/.claude/skills/functor-lang/SKILL.md)
lists the whole registry.

<table>
<tbody>
<tr><td><code>List.map(fn, list)</code> · <code>List.filter(fn, list)</code></td><td rowspan="2">subject-last — pipeline-friendly</td></tr>
<tr><td><code>List.fold(fn, init, list)</code> — callback is <code>(acc, x) => …</code></td></tr>
<tr><td><code>List.range(n)</code> · <code>List.maximum(list)</code></td><td><code>range</code> is <code>[0 … n-1]</code></td></tr>
<tr><td><code>List.grid(fn, rows, cols)</code></td><td><code>List&lt;List&lt;'a&gt;&gt;</code>; calls <code>fn(row, col)</code> per cell (the heightmap loop)</td></tr>
<tr><td><code>Text.concat(a, b)</code> · <code>Text.fromFloat(n)</code> · <code>Text.fixed(n, decimals)</code></td><td>there is no string-concat operator</td></tr>
<tr><td><code>Text.split(sep, s)</code> · <code>Text.join(sep, list)</code> · <code>Text.toBullets(list)</code> · <code>Text.parseFloat(s)</code></td><td></td></tr>
<tr><td><code>Math.sin(n)</code> · <code>Math.cos(n)</code> · <code>Math.clamp01(n)</code></td><td></td></tr>
<tr><td><code>Debug.log(label, value)</code></td><td><code>(string, 'a) => 'a</code> — logs <code>label: value</code> and returns the value unchanged; pure to the result, safe to drop in a pipe</td></tr>
</tbody>
</table>

## Sharp edges

- `x |> f(a)` is `f(a, x)` — pipelines *append* (thread-last).
- Assignment is `:=` (not `<-`) and must be followed by `;` and a continuation.
- No `if`/`else`, no `&&`/`||`, no loops — use a bool-literal `match` and `List.*`.
- Primitive type names are lowercase (`float`, `string`, `bool`); a capitalized `Float`
  degrades to `Unknown` and buys no checking.
- Tuple types annotate as `(float, string)` — *not* `float * float`, which doesn't parse.
- Angles, Durations, and render targets are branded *values* — `Sub.every(0.5, …)` and
  `Scene.rotateY(1.57)` are errors.
- A nested `match` inside an arm eats the following arms — parenthesize it.
- Constructor names are unique to a module's value namespace; nullary constructors never
  take parens (`Point`, never `Point()`).
- Key repeats arrive as `isDown = true` — latch for rising edges.
- Physics reads belong in `draw`; in `tick` you see last frame's world.
- Engine values (`<Scene>`, `<Frame>`, …) are opaque: pass them around, don't compare or
  inspect them.

<p class="docs-footnote">
This page tracks the language exactly — if a construct isn't here, it doesn't parse.
The roadmap lives in <a href="https://github.com/tommy-xr/functor/blob/main/docs/functor-lang.md">docs/functor-lang.md</a>.
</p>
