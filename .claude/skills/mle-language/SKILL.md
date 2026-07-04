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

type Box<v> =                                 // GENERIC declarations: lowercase params
  | Full(value: v)                            //   Box<Float> and Box<String> coexist;
  | Empty                                     //   params substitute through fields/patterns

type Shape =                                  // variant types (ADTs); nominal like records
  | Circle(radius: Float)                     // leading | REQUIRED, first alternative too
  | Rect(w: Float, h: Float)                  // fields named in the decl…
  | Point                                     // …nullary ctor: no parens, ever

let c = Circle(2.0)                           // …but ctors are CALLED positionally
let shapes = [c, Rect(3.0, 4.0), Point]       // bare Point IS the value

let area = (s: Shape): Float =>
  match s with                                // match: | pattern => full-expression body
  | Circle(r) => 3.14 * r * r                 // ctor patterns bind positionally
  | Rect(w, _) => w * w                       // sub-patterns: names or _ ONLY (no nesting)
  | Point => 0.0                              // exhaustiveness checked when s's type is known

let sizeOf = (s: Shape): String =>
  match area(s) > 10.0 with                   // bool-literal match = the ONLY conditional
  | true => "big"
  | false => "small"                          // number/string literal arms exist too
                                              // (they need a catch-all: `| x =>` or `| _ =>`)

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

let minMax = (a: Float, b: Float): Float * Float =>   // tuple: (e1, e2, …), 2+ elements
  match a < b with
  | true => (a, b)                            // `(e)` is GROUPING, not a 1-tuple
  | false => (b, a)

let span = (a, b) =>
  let (lo, hi) = minMax(a, b) in              // destructuring let (sugar for a
  hi - lo                                     //   single-arm match; no `mut`)

let sum3 = (a, b, c) =>
  let mut acc = a in                          // expression let-in; `mut` = rebindable slot
  acc := acc + b;                             // assignment is `:=` and carries a continuation
  acc := acc + c;
  acc

let main = () => report([12.0, 3.5, 40.0])    // zero-param main is run's entry point
```

Operators: `+ - * /` `< > ==` (conventional precedence; pipelines bind
loosest), unary `-`. There is **no** if/else, loops, string-concatenation
operator, modules, or imports yet — iteration is `List.map/filter/fold`,
and the conditional is a **bool-literal match**
(`match x > 3.0 with | true => a | false => b`). Tuples are structural:
`(1.0, "a") == (1.0, "a")`; product types annotate as `Float * String`
(flat — no grouping in type position). Prefer named records for anything
that outlives an expression; tuples are for multiple returns.

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
- **Greedy match arms**: arm bodies are full expressions, so a nested
  `match` inside an arm consumes the following `|` arms as its own —
  parenthesize the inner match (F#/OCaml convention). The leading `|` is
  required before every arm and every variant alternative, first included.
- **Constructors resolve bare and live in the VALUE namespace**: `Circle(2.0)`
  works anywhere (`Shape.Circle` does NOT — it stays an unknown external),
  which is why ctor names must be unique ACROSS all variant types in the
  module, and `let Circle = …` alongside a ctor `Circle` is a
  duplicate-definition error. An (uppercase) param may still shadow a ctor;
  pattern vars can't (they are forced lowercase).
- **Patterns are minimal**: `Ctor(x, _)` / `Ctor` / `(x, _)` (tuple) /
  bare name / `_` / literals (`true`, `false`, numbers incl. negative,
  strings — equality match). Ctor and tuple sub-patterns are names or `_`
  only — no nesting, no literals inside. A tuple pattern matches by EXACT
  arity (mismatch = non-match, like ctors).
  Pattern vars are immutable bindings; lambdas may capture them. First
  matching arm wins; no arm matching is a spanned runtime error. Unapplied
  ctors are first-class (`xs |> List.map(Circle)`); the runtime checks ctor
  ARITY only (field types are the checker's job).
- **Duplicates are errors**: top-level names (per namespace — `type Foo` and
  `let Foo` may coexist, but constructors share the value namespace with
  `let`s), record fields (literal and update), lambda params, pattern
  variables within one pattern.
- Recursion depth is capped (~200); deep iteration belongs in `List.*`.

## Builtins (the whole registry)

`List.map(list, fn)` · `List.filter(list, fn)` · `List.fold(list, fn, init)`
(callback is `(acc, x) => …`) · `List.range(n)` (`[0 … n-1]`) ·
`List.maximum(list)` · `Text.concat(a, b)` · `Text.fromFloat(n)` ·
`Text.toBullets(list)` · `Math.clamp01(n)` · `Math.sin(n)` · `Math.cos(n)`

## Functor prelude (only under the engine host — `FunctorHost`)

Available in runner-hosted MLE (and tests via
`functor_runtime_common::mle_prelude`), NOT in plain `mle run`:

```mle
Scene.cube() / sphere() / cylinder() / quad() / plane()   // zero args, enforced
Scene.group([scene, …])
scene |> Scene.color(r, g, b)                              // scene-first: pipes
scene |> Scene.lit(r, g, b)                                // diffuse+specular
scene |> Scene.emissive(r, g, b)                           // unlit glow
scene |> Scene.translate(x, y, z)
scene |> Scene.rotateX(angle) / rotateY / rotateZ          // Angle VALUES only:
Angle.degrees(60.0) / Angle.radians(1.57)                  //   never bare numbers
scene |> Scene.scale(k)
Camera.lookAt(ex, ey, ez, tx, ty, tz)                      // up=+Y, fov 45°
Camera.firstPerson(ex, ey, ez, yaw, pitch, fov)           // all three: Angles
Light.ambient(r, g, b) / Light.point(px, py, pz, r, g, b, intensity, range)
Light.directional(dx, dy, dz, r, g, b, intensity) |> Light.castShadows
Frame.create(camera, scene)                                // what draw returns
Frame.createLit(camera, scene, [light, …])                 // lit + shadowed
RenderTarget.named("id")                                   // offscreen texture, 512x512…
RenderTarget.named("id") |> RenderTarget.sized(w, h)       // …unless sized. Declare ONCE,
                                                           //   use the VALUE at both sites
                                                           //   (never a bare string — the
                                                           //   Angle rule for identity)
frame |> Frame.withRenderTarget(target, targetFrame)       // writer: targetFrame (a full
                                                           //   Frame.create/createLit) is
                                                           //   rendered into the target
                                                           //   before frame's main pass
                                                           //   with its OWN lights — use
                                                           //   createLit + castShadows for
                                                           //   a lit/shadowed feed. A scene
                                                           //   sampling its OWN target
                                                           //   sees last frame's image
scene |> Scene.screen(target)                              // reader: emissive screen
                                                           //   showing the target; an
                                                           //   undeclared id = magenta +
                                                           //   one warning. A quad's front
                                                           //   is +Z — rotate the monitor
                                                           //   to face the viewer or the
                                                           //   feed shows mirrored
Time.seconds(1.0) / Time.millis(500.0)                     // Duration VALUES only
Sub.every(duration, msg) / Sub.none() / Sub.batch([sub,…]) // what subscriptions returns
Effect.random(tagger) / Effect.now(tagger)                 // one-shots; tagger: (Float) => Msg
Effect.none() / Effect.batch([fx, …])                      //   random: [0,1); now: epoch secs

Physics.box(w, h, d) / sphere(r) / capsule(halfH, r)       // -> Shape (box = FULL extents)
Physics.dynamic("tag", shape)                              // simulated body
Physics.kinematic("tag", shape) / Physics.fixed("tag", shape)
body |> Physics.at(x, y, z)                                // body-first: pipes
body |> Physics.velocity(vx, vy, vz)
body |> Physics.mass(m) / Physics.friction(f) / Physics.restitution(r)
body |> Physics.sensor                                     // overlap-only, no forces
Physics.scene(gx, gy, gz, [body, …])                       // what `physics` returns
Physics.position("tag")                                    // {x, y, z} of the LIVE body
scene |> Physics.transformed("tag")                        // scene at the body's live pose
Physics.applyImpulse("tag", x, y, z)                       // -> Effect (fire-and-forget)
Physics.applyForce("tag", x, y, z)                         //   force lasts ONE stepped frame
Physics.setVelocity("tag", x, y, z) / Physics.teleport("tag", x, y, z)
```

`Physics.position` / `Physics.transformed` read the live stepped world
(MLE runs in the shell's process — no boundary). A tag not in the world is
a **spanned runtime error** (there is no Option-shaped return to match on),
so only read tags your `physics` hook declares. The tag is cross-frame
identity: same tag = same body; drop a body by not declaring it.
Re-declaring an *unchanged* body leaves the simulation alone; *changing*
its declared position teleports it (the divergence rule, docs/physics.md).

Physics **command effects** are returned beside the model like any effect
— `(model, Physics.applyImpulse("ball", 0.0, 5.0, 0.0))` — but carry no
tagger: nothing folds back through `update`; observe outcomes via the
physics reads. Commands queue at perform time and apply at the next
stepped frame's first substep, **after reconcile** — so declaring a body
and commanding it in the same frame works. A command naming an unknown tag
is a deduped `[mle]` warning, not an error (the body may have despawned in
flight). `teleport` moves the live body without touching its declaration
(no snap-back next frame).

A runner-hosted game (`functor-runner --mle --game-path game.mle`) defines:

```mle
let init = { … }                       // the initial model (a value)
let tick = (model, dt, tts) => model'  // per-frame step
let draw = (model, tts) => Frame.create(camera, scene)
let input = (model, key, isDown) => model'  // OPTIONAL; key = "W"/"Up"/"Space"
let mouseMove = (model, x, y) => model'     // OPTIONAL; window pixels
let mouseWheel = (model, delta) => model'   // OPTIONAL
let update = (model, msg) => model'         // OPTIONAL; msgs are ADT variants
                                            // ANY entry point may instead return
                                            // (model', effect) — a 2-tuple whose
                                            // second element is an Effect value
let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)
                                            // OPTIONAL, but requires update
let physics = (model) => Physics.scene(0.0, -9.81, 0.0, [body, …])  // OPTIONAL
```

Subscription timers are **stateless**: `Sub.every` fires when an integer
multiple of its period lies in `(prevTts, tts]` — the global time grid, so
a long frame fires ONCE (missed boundaries collapse) and timers tick right
through a hot reload. Fired messages fold through `update` before `tick`.
Durations, like Angles, are branded values — `Sub.every(0.5, …)` is a
teaching error; say `Time.seconds(0.5)` or `Time.millis(500.0)`.

Effects are one-shot commands: the producer performs each one, applies its
tagger to the result (`Effect.random(Rolled)` → `Rolled(0.42)`), and folds
the message back through `update` — which may itself return more effects
(drained same-frame to a fixed point, capped). Every performed effect
lands in a structured log; under a fake/replay runner the same program is
exactly deterministic (that's the test seam). Taggers must be functions —
`Effect.now(3.0)` is a construction-time error.

Frame order: subscriptions→`update` → `tick` → `physics` (reconcile +
fixed-step, 60Hz accumulator) → `draw` — physics reads in `draw` see this
frame's stepped world; reads in `tick` see the *previous* frame's (so on
the very first frame, and inside the `physics` hook itself, declared bodies
don't exist yet — keep reads in `draw`). The physics world survives hot
reload (like the model); deleting the `physics` hook drops it. Gotcha:
`--fixed-time T` pins the clock with `dts = 0`, so physics **never steps**
under it (and the subscription grid never crosses) — bodies render at their
declared pose. Capture physics with plain `--capture-time` (and a settled
scene for reproducibility) instead; capture timer-driven changes via the
debug server's `/time` advance. To *see* colliders, run with
`--debug-render physics`: normal shading plus the live world's wireframes
(collider outlines, contacts, body frames).

A project dir with `functor.json` `{"language": "mle", "entry": "game.mle"}`
works with the CLI: `functor -d dir build` (typecheck, diagnostics are
errors), `run native`, `develop` (hot reload is built in), and `run wasm`
(the `.mle` ships as text and is interpreted in the browser; file-watch hot
reload is native-only — reload the page to pick up saved edits, or push
source with a `{ type: "mle-set-source", source }` postMessage to the page
for a model-preserving in-place reload; the VSCode **"MLE: Open Live
Preview"** command does exactly that from the live buffer as you type).

`examples/mle-hello/game.mle` is the reference
(`examples/mle-physics/game.mle` for the physics hook, including the
rising-edge input latch — GLFW key repeats arrive as `isDown = true`).
The model shows live at the
debug server's `GET /state`. **Hot reload is on by default**: saving the
`.mle` file reloads it in ~1 frame with the model preserved (a broken edit
keeps the old program running; an edited `init` takes effect on restart).
Closures **stored in the model** rebind too: they adopt the edited code
with their captured values carried over (matched by the enclosing def's
name; a closure whose def was renamed/deleted keeps its old body with a
loud `[mle] reload:` warning).

Transforms wrap in Group nodes: the **outer call applies last in world
space** — `s |> Scene.rotateY(r) |> Scene.translate(x, 0.0, 0.0)` rotates in
place, then moves (the order the source reads). Engine values (`<Scene>`,
`<Camera>`, `<Frame>`) are opaque: they can be passed around but not
inspected, compared, or serialized.

## Typechecking model (Hindley–Milner + gradual seams)

`mle check` runs REAL INFERENCE (B7): unannotated code gets full types via
unification with let-polymorphism — generic functions instantiate fresh at
every use, element types flow through `List.map`/`filter`/`fold`, and
lowercase annotation names are type variables (`(xs: List<a>, f: (a) =>
b): List<b>`). Inference has teeth: unannotated bad calls, mixed-element
lists, and contradictory `mut` use are errors now. `Unknown` remains ONLY
at genuinely-dynamic seams (host values, unrecognized Uppercase type
names) and absorbs anything. Generic declarations (`type Pair<x, y> = { first: x, second: y }`)
instantiate fresh per use; an UNDECLARED lowercase name in a declaration is
a teaching error. Record literals resolve nominally, F#-style:
the unique declared type with exactly that field set (no match = anonymous
data, still fine; two same-shaped declarations make a bare literal
ambiguous — annotate). A `mut` slot's type fixes at its initializer. A
`match`'s patterns CONSTRAIN its scrutinee (first ctor arm pins the
variant type; a foreign literal arm is a can-never-match error);
exhaustiveness checks all ctors / `true`+`false` / catch-all; arm results
must agree.

## Keeping this skill honest

This file must track the language. When a PR adds syntax/builtins/semantics
(see `docs/mle.md` Track B/C checkboxes), update this skill in the same PR.
