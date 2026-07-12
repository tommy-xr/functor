# Language reference

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
</tbody>
</table>

The model-returning entry points (`tick`, `input`,
`mouseMove`, `mouseWheel`, `update`) may instead
return a 2-tuple `(model', effect)` to issue one-shot
[effects](#prelude-effects).

**Frame order:** subscriptions → `update` → `tick`
→ physics (fixed-step 60&nbsp;Hz) → `draw`. Physics reads in
`draw` see *this* frame's stepped world; reads in `tick`
see the previous frame's — so on the very first frame declared bodies don't exist
yet. Keep physics reads in `draw`.

## The language

### Values and bindings

```functor
let threshold = 10          // every number is a Float (f64)
let name = "neon\n"         // strings: \" \\ \n \t
let on = true
let origin = { x: 0.0, y: 0.0 }
let scores = [1.0, 2.0, 3.0]
```

Line comments only (`//`). Top-level definitions are mutually visible
inside function bodies (and late-bound — that's the hot-reload seam), but a top-level
*initializer* may only use names defined above it.

### Functions

```functor
let area = (w: Float, h: Float): Float => w * h   // annotations optional
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))
```

Typing is *gradual*: unannotated values check against everything; annotations
buy diagnostics (`functor build` reports them all). Recursion depth is
capped (~200) — deep iteration belongs in `List.*`.

### Records

```functor
type Position = { x: Float, y: Float }        // nominal in annotations

let p = { x: 0.0, y: 1.0 }
let nudged = { p with x: p.x + 1.0 }          // update: fields must exist
```

### Variants and match

```functor
type Shape =
  | Circle(radius: Float)     // leading | required — first alternative too
  | Rect(w: Float, h: Float)
  | Point                     // nullary: no parens, ever

let c = Circle(2.0)           // constructors are CALLED positionally
let shapes = [c, Rect(3.0, 4.0), Point]

let area = (s: Shape): Float =>
  match s with
  | Circle(r) => 3.14 * r * r // ctor patterns bind positionally
  | Rect(w, _) => w * w       // sub-patterns: names or _ only (no nesting)
  | Point => 0.0
```

Constructors resolve *bare* and live in the value namespace
(`Shape.Circle` does **not** work), so constructor names must
be unique across all variant types in the file. Unapplied constructors are
first-class: `xs |> List.map(Circle)`. When the scrutinee's type is known,
exhaustiveness is checked. Patterns are minimal: `Ctor(x, _)`,
`Ctor`, tuple `(x, _)` (matched by exact arity), bare names,
`_`, and literals — number/string literal arms need a catch-all
(`true` + `false` together are exhaustive).

**Arms are greedy:** a nested `match` inside an arm consumes
the following `|` arms as its own — parenthesize the inner match.

### The conditional

There is no `if`/`else`. The conditional is a bool-literal match:

```functor
let sizeOf = (n: Float): String =>
  match n > 10.0 with
  | true => "big"
  | false => "small"
```

### Pipelines

`|>` **appends** the piped value:
`x |> f(a)` is `f(a, x)`. Every prelude function therefore
takes its subject (list, scene, body) last (thread-last, F#/Elm-style).

```functor
let isHigh = (score) => score > 10.0
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))

let report = (scores) =>
  scores
    |> List.filter(isHigh)
    |> List.map(describe)
    |> Text.toBullets
```

### Tuples

```functor
let minMax = (a: Float, b: Float): Float * Float =>
  match a < b with
  | true => (a, b)            // (e) is grouping, not a 1-tuple
  | false => (b, a)

let span = (a, b) =>
  let (lo, hi) = minMax(a, b) in   // destructuring let
  hi - lo
```

Tuples are structural (2+ elements) and are for multiple returns; prefer named
records for anything that outlives an expression.

### Local mutation

```functor
let sum3 = (a, b, c) =>
  let mut acc = a in          // a rebindable slot; expression let-in
  acc := acc + b;             // assignment is := and carries a continuation
  acc := acc + c;
  acc
```

`mut` is local-only: no top-level `let mut`, and a lambda may
not capture an enclosing `mut` slot. Params, globals, and plain
`let`s are immutable.

### Operators and equality

`+ - * /`, `< > ==`, unary `-`; conventional
precedence, pipelines bind loosest. `==` is structural (comparing
functions is a runtime error). Division is IEEE (`1.0/0.0` is
`inf`); the engine boundary rejects non-finite numbers. There are no
loops — iteration is `List.map/filter/fold`.

## The prelude

Available in runner-hosted games (the CLI, the sandbox) — not in the bare
`functor-lang` interpreter.

### Scene

<table>
<tbody>
<tr><td><code>Scene.cube() / sphere() / cylinder() / quad() / plane()</code></td><td>primitives (zero args, enforced); <code>plane</code> lies in XZ (ground), <code>quad</code> in XY — a quad's front is +Z</td></tr>
<tr><td><code>Scene.group([scene, …])</code></td><td>compose</td></tr>
<tr><td><code>scene |> Scene.color(r, g, b)</code></td><td>flat unlit color</td></tr>
<tr><td><code>scene |> Scene.lit(r, g, b)</code></td><td>diffuse + specular (needs lights)</td></tr>
<tr><td><code>scene |> Scene.emissive(r, g, b)</code></td><td>unlit glow</td></tr>
<tr><td><code>scene |> Scene.translate(x, y, z)</code></td><td rowspan="3">transforms wrap outward: the <em>outer</em> call applies last in world space — <code>s |> rotateY(r) |> translate(x, 0.0, 0.0)</code> rotates in place, then moves</td></tr>
<tr><td><code>scene |> Scene.rotateX/Y/Z(angle)</code></td></tr>
<tr><td><code>scene |> Scene.scale(k)</code></td></tr>
</tbody>
</table>

Coordinates are **Y-up, right-handed**: +Y up, +X right, ground in XZ.
Angles are branded values: `Angle.degrees(60.0)` /
`Angle.radians(1.57)` — never bare numbers.

### Camera, lights, frames

<table>
<tbody>
<tr><td><code>Camera.lookAt(ex, ey, ez, tx, ty, tz)</code></td><td>up = +Y, fov 45°</td></tr>
<tr><td><code>Camera.firstPerson(ex, ey, ez, yaw, pitch, fov)</code></td><td>all three are Angles; yaw 0 / pitch 0 looks down +Z</td></tr>
<tr><td><code>Light.ambient(r, g, b)</code></td><td></td></tr>
<tr><td><code>Light.directional(dx, dy, dz, r, g, b, intensity)</code></td><td><code>|> Light.castShadows</code> for a shadowed key light</td></tr>
<tr><td><code>Light.point(px, py, pz, r, g, b, intensity, range)</code></td><td></td></tr>
<tr><td><code>Frame.create(camera, scene)</code></td><td>what <code>draw</code> returns</td></tr>
<tr><td><code>Frame.createLit(camera, scene, [light, …])</code></td><td>lit + shadowed</td></tr>
</tbody>
</table>

### Render targets

```functor
let feed = RenderTarget.named("security") |> RenderTarget.sized(256.0, 256.0)
```

<table>
<tbody>
<tr><td><code>frame |> Frame.withRenderTarget(target, targetFrame)</code></td><td>writer: renders a full frame (own camera + lights) into the target before the main pass</td></tr>
<tr><td><code>scene |> Scene.screen(target)</code></td><td>reader: an emissive screen showing the target</td></tr>
</tbody>
</table>

Declare a target *once* and use the value at both sites (never a bare
string). A scene sampling its own target sees last frame's image. The
*Render targets* example in the sandbox is the reference.

<h3>Time, subscriptions<span id="prelude-effects"></span>, effects</h3>

<table>
<tbody>
<tr><td><code>Time.seconds(1.0) / Time.millis(500.0)</code></td><td>Durations are branded, like Angles</td></tr>
<tr><td><code>Sub.every(duration, Msg)</code></td><td>stateless global time grid: a long frame fires once; timers tick through hot reload</td></tr>
<tr><td><code>Sub.none() / Sub.batch([sub, …])</code></td><td></td></tr>
<tr><td><code>Effect.random(Tagger) / Effect.now(Tagger)</code></td><td>one-shots; the tagger is a function <code>(Float) => Msg</code> — <code>random</code> gives [0,1), <code>now</code> epoch seconds</td></tr>
<tr><td><code>Effect.none() / Effect.batch([fx, …])</code></td><td>return <code>(model', effect)</code> from any entry point</td></tr>
</tbody>
</table>

### Physics

<table>
<tbody>
<tr><td><code>Physics.box(w, h, d) / sphere(r) / capsule(halfH, r)</code></td><td>shapes; box takes <em>full</em> extents</td></tr>
<tr><td><code>Physics.dynamic("tag", shape)</code></td><td>simulated; also <code>kinematic</code> / <code>fixed</code></td></tr>
<tr><td><code>body |> Physics.at(x, y, z)</code></td><td>spawn pose; also <code>velocity</code>, <code>mass</code>, <code>friction</code>, <code>restitution</code>, <code>sensor</code></td></tr>
<tr><td><code>Physics.scene(gx, gy, gz, [body, …])</code></td><td>what the <code>physics</code> hook returns</td></tr>
<tr><td><code>Physics.position("tag")</code></td><td><code>{x, y, z}</code> of the live body</td></tr>
<tr><td><code>scene |> Physics.transformed("tag")</code></td><td>draw a scene at the body's live pose</td></tr>
</tbody>
</table>

The tag is cross-frame identity: same tag = same body; drop a body by not declaring
it. Re-declaring an unchanged body leaves the simulation alone; changing its declared
position teleports it. Reading a tag your `physics` hook doesn't declare
is a runtime error. The physics world survives hot reload, like the model.

## Builtins

<table>
<tbody>
<tr><td><code>List.map(fn, list)</code> · <code>List.filter(fn, list)</code></td><td rowspan="2">data comes last — pipeline-friendly (F#/Elm-style)</td></tr>
<tr><td><code>List.fold(fn, init, list)</code> — callback is <code>(acc, x) => …</code></td></tr>
<tr><td><code>List.range(n)</code></td><td><code>[0 … n-1]</code></td></tr>
<tr><td><code>List.maximum(list)</code></td><td></td></tr>
<tr><td><code>Text.concat(a, b)</code> · <code>Text.fromFloat(n)</code> · <code>Text.toBullets(list)</code></td><td></td></tr>
<tr><td><code>Math.sin(n)</code> · <code>Math.cos(n)</code> · <code>Math.clamp01(n)</code></td><td></td></tr>
</tbody>
</table>

## Sharp edges

- `x |> f(a)` is `f(a, x)` — pipelines *append* (thread-last).
- Assignment is `:=` (not `<-`) and must be followed by `;` and a continuation.
- No `if`/`else` — use a bool-literal `match`.
- Angles, Durations, and render targets are branded *values* — `Sub.every(0.5, …)` and `Scene.rotateY(1.57)` are errors.
- A nested `match` inside an arm eats the following arms — parenthesize it.
- Constructor names are global to the file's value namespace; nullary constructors never take parens.
- Key repeats arrive as `isDown = true` — latch for rising edges.
- Physics reads belong in `draw`; in `tick` you see last frame's world.
- Engine values (`<Scene>`, `<Frame>`, …) are opaque: pass them around, don't compare or inspect them.

<p class="docs-footnote">
This page tracks the language exactly — if a construct isn't here, it doesn't parse.
The roadmap lives in <a href="https://github.com/tommy-xr/functor/blob/main/docs/functor-lang.md">docs/functor-lang.md</a>.
</p>
