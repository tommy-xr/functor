---
name: functor-lang
description: Write, run, and debug Functor Lang (.fun) — Functor's F#-inspired game-logic language. Use whenever creating or editing .fun files, answering Functor Lang syntax/semantics questions, or debugging Functor Lang parse/run/check errors. Functor Lang is a custom language — do NOT guess from F#/OCaml intuition; this skill is the source of truth for the current subset.
---

# Functor Lang — the current language, exactly

Functor Lang is Functor's interpreted game-logic language (roadmap: `docs/functor-lang.md`;
design notes: `~/notes/ideas/functor-lang/`). It is deliberately small; this
file describes **everything that exists today**. If a construct isn't here,
it doesn't parse — do not invent syntax from F#/OCaml habits.

## Verification loop (always available, no GPU)

```sh
cargo run -q -p functor-lang -- parse file.fun    # surface AST (spans on every node; this file only)
cargo run -q -p functor-lang -- ir file.fun      # name-resolved core IR (merged project)
cargo run -q -p functor-lang -- run file.fun     # evaluate: main()'s result, or the entry's bindings
cargo run -q -p functor-lang -- trace file.fun   # enter/exit call story with values (kept on failure)
cargo run -q -p functor-lang -- check file.fun   # typechecker: ALL diagnostics, exit 1
```

`ir`/`check`/`run`/`trace` treat the file as a PROJECT ENTRY: every sibling
`.fun` in its directory loads with it (file = module — see Modules below),
so scratch files must live in their own directory, not a shared one.

Errors are always `file:line:col: error: message`. Tests live in `functor-lang/tests/`
with goldens next to `functor-lang/examples/` (`UPDATE_GOLDENS=1 cargo test -p functor-lang`
regenerates). VSCode gets live parse/lower/type diagnostics,
`name : Type` hover, and go-to-definition via `tools/functor-lang-lsp`.

## Syntax subset

```functor
// line comments only
type Position = { x: float, y: float }        // record types; nominal in annotations

type Box<'v> =                                // GENERIC declarations: 'v type-var params
  | Full(value: 'v)                           //   Box<float> and Box<string> coexist;
  | Empty                                     //   params substitute through fields/patterns

type Shape =                                  // variant types (ADTs); nominal like records
  | Circle(radius: float)                     // leading | REQUIRED, first alternative too
  | Rect(w: float, h: float)                  // fields named in the decl…
  | Point                                     // …nullary ctor: no parens, ever

type SceneNode                                // ABSTRACT type (no `= body`): an opaque nominal —
                                              //   no fields, no constructor; host code makes its
                                              //   values. Use it in annotations (`(n: SceneNode)`).

let c = Circle(2.0)                           // …but ctors are CALLED positionally
let shapes = [c, Rect(3.0, 4.0), Point]       // bare Point IS the value

let area = (s: Shape): float =>
  match s with                                // match: | pattern => full-expression body
  | Circle(r) => 3.14 * r * r                 // ctor patterns bind positionally
  | Rect(w, _) => w * w                       // sub-patterns: names or _ ONLY (no nesting)
  | Point => 0.0                              // exhaustiveness checked when s's type is known

let sizeOf = (s: Shape): string =>
  match area(s) > 10.0 with                   // bool-literal match = the ONLY conditional
  | true => "big"
  | false => "small"                          // number/string literal arms exist too
                                              // (they need a catch-all: `| x =>` or `| _ =>`)

let threshold = 10                            // top-level let; ints/floats are all float (f64)
let origin: Position = { x: 0.0, y: 0.0 }     // OPTIONAL binding annotation `let name: Type = …`
                                              //   (checked against the value; also on `let … in`)
let scores = [1.0, 2.0, 3.0]                  // list literal; [x, ..xs] prepends
let sumList = (xs: List<float>): float =>     // list PATTERNS: [] / [a,b] / [h, ..t]
  match xs with
  | [] => 0.0
  | [head, ..rest] => head + sumList(rest)    // refutable; needs a catch-all or [..r]
let s = "text\n"                              // strings: escapes \" \\ \n \t
let flag = true                               // bools

let isHigh = (score: float): bool => score > threshold   // annotations OPTIONAL (gradual)
let describe = (score) => Text.concat("score: ", Text.fromFloat(score))

let report = (scores) =>
  scores
    |> List.filter(isHigh)                    // pipeline: |> APPENDS the piped value (thread-last)
    |> List.map(describe)                     //   x |> g(a)  ==  g(a, x)
    |> Text.toBullets

let nudge = (p: Position): Position => { p with x: p.x + 1.0 }  // record update (fields must exist)

let minMax = (a: float, b: float): (float, float) =>  // tuple TYPE: (A, B); value tuple: (e1,e2,…)
  match a < b with
  | true => (a, b)                            // `(e)` / `(A)` is GROUPING, not a 1-tuple
  | false => (b, a)

let apply = (f: (float) => float, x: float): float => f(x)  // function TYPE: (A, B) => C, () => C
// return-position function types need parens: (): ((A) => B) => …  (the outer => is the body)

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
loosest), unary `-`. There is **no** if/else, loops, or string-concatenation
operator — iteration is `List.map/filter/fold`,
and the conditional is a **bool-literal match**
(`match x > 3.0 with | true => a | false => b`). Tuples are structural:
`(1.0, "a") == (1.0, "a")`; tuple types annotate as `(float, string)` and
function types as `(A, B) => C` / `() => C`, with `(A)` as grouping. Prefer
named records for anything that outlives an expression; tuples are for
multiple returns.

## Modules (multi-file projects)

**File = module.** Every `.fun` file in the entry file's directory IS a
module, named by its filename stem with the first letter capitalized
(`utils.fun` → `Utils`); the entry file (functor.json `entry`, default
`game.fun`; the file you hand the CLI) is the program root. Loading is
EAGER and whole-program: ALL sibling `.fun` files load, check, and
evaluate together — an unreferenced (or broken, or stray scratch) sibling
still counts. File stems must be identifiers (`pure_pipeline.fun`, not
`pure-pipeline.fun` — that's a load error).

```functor
// utils.fun                                  // → module Utils
type Shape = | Circle(radius: float) | Point
let tau = 6.28
let area = (s: Shape): float =>
  match s with
  | Circle(r) => 3.14 * r * r
  | Point => 0.0
```

```functor
// game.fun (the entry)
open Utils                                    // bring Utils in unqualified

let a = area(Circle(2.0)) + tau               // via the open…
let b = Utils.area(Utils.Circle(2.0))         // …or QUALIFIED — no open needed
let biggest = (shapes: List<Utils.Shape>) =>  // qualified types in annotations
  shapes |> List.map(area) |> List.maximum
let grab = (s) =>
  match s with
  | Utils.Circle(r) => r                      // qualified ctor PATTERNS work too
  | Utils.Point => 0.0
```

- **Qualified access needs NO import**: `Utils.clamp(x)`, `Utils.Circle(…)`
  (expressions and patterns, first-class when unapplied), `Utils.Shape` /
  `Utils.Box<float>` in annotations. `open Utils` adds unqualified access;
  a name collision with the module's own defs or another `open` is a load
  error naming both sides (qualify instead). `open` is contextual — it
  stays a valid binding name.
- **Cross-file dependency cycles are refused** (load error with the path,
  `Game → Utils → Game`); ANY cross-file reference — a qualified use, an
  `open` (even unused), a type annotation — is a dependency edge. Within
  one file, letrec-style mutual recursion is unchanged. A module's
  top-level initializers may demand globals of modules it depends on
  (they evaluate first); siblings may reference the entry (`Game.foo`) if
  that creates no cycle.
- **Protected namespaces**: a file whose module name collides with a
  builtin/prelude namespace (Net, List, Text, Math, Debug, Scene, Anim, Camera,
  Frame, Light, Fog, Skybox, Angle, Texture, Time, Sub, Effect, Physics,
  RenderTarget, Ui, AudioSource, AudioScene) is a load error — rename the file.
- **`Net` is a built-in module**, always in scope: `type NetEvent =
  | Connected(id: float) | Message(id: float, text: string) |
  Disconnected(id: float) | Error(id: float, text: string)`. A `Sub.connect`/
  `Sub.listen` tagger receives these — `match ev with | Net.Connected(id)
  => …` — with no declaration needed.
- Constructor names must be unique per MODULE (not per project); values
  from a non-entry module display with their canonical tag
  (`Utils.Circle(2)` in run/trace/`/state` output). The entry's own names
  stay bare — a single-file project behaves exactly as before.
- **Hot reload watches every project file**: editing, adding, or removing
  ANY `.fun` in the directory reloads with the model preserved (stored
  closures rebind per module — a def moved between files is a rename and
  keeps its old body with a warning).
- Current limits: `run wasm` and the VSCode live preview interpret ONE
  source text (multi-file is native-only for now).

### Interface files (`.funi`)

A sibling `.funi` is an INTERFACE module: it declares **types** and **bodyless
value signatures** for values the **host runtime** implements. (A module is
either a `.fun` or a `.funi`, never both — same-stem files are a load error —
so there is no paired-`.fun` implementation.) Bodies are forbidden in a
`.funi`; a bodyless `let` is forbidden in a `.fun`.

```functor
// widget.funi                              → interface module Widget
type Handle                                 // abstract type (opaque; host-made)
let make : () => Handle                     // bodyless SIGNATURE (the chosen form —
let size : (Handle) => float                //   `let name : Type`, no `= body`)
```

```functor
// game.fun
let area = (h: Widget.Handle): float => Widget.size(h)   // qualified; typed by widget.funi
open Widget                                              // …or open, bringing make/size/Handle bare
```

- Signatures give the checker real types for what were `Unknown` externals
  (`Widget.make()` is `Widget.Handle`, not `Unknown`), and mismatches are
  caught. They surface in hover / inlay / codelens like any type.
- **Runtime is unchanged**: an interface member stays an `External` (the host
  provides its value at run time), so `.funi` is a pure check-time overlay.
- This is how the **engine prelude's types are declared**: the `functor-prelude`
  crate ships a `.funi` for every host namespace (`Scene`, `Camera`, `Frame`,
  `Light`, `Fog`, `Skybox`, `RenderTarget`, `Texture`, `Angle`, `Time`, `Sub`,
  `Effect`, `Physics`, `Ui`, `AudioSource`, `AudioScene`), loaded by the runner
  so engine calls carry real types (no longer `Unknown`). Each module's primary
  opaque handle is `Mod.t` (`Camera.t`, `Frame.t`, `Effect.t`, …); modules that
  own several name each (`Scene.t`; `Physics.shape`/`body`/`world`;
  `Ui.view`/`anchor`). Physics query/event results are records
  (`Physics.position`, `Physics.rayHit`, `Physics.collisionEvent`).

## Semantics rules that WILL bite you

- **Pipelines append (thread-last)**: `x |> f(a)` is `f(a, x)`. Every
  builtin/prelude function therefore takes its "subject" (list, scene) LAST.
  Because `|>` is syntax, `x |> f(a)` lowers directly to the saturated `f(a, x)`
  (never a partial `f(a)`), so scene/list pipes allocate nothing.
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

All are **subject-LAST** (the collection/subject is the final arg), so they
thread through `|>` (which appends): `list |> List.map(fn)` == `List.map(fn, list)`.

`List.map(fn, list)` · `List.filter(fn, list)` · `List.fold(fn, init, list)`
(callback is `(acc, x) => …`) · `List.range(n)` (`[0 … n-1]`) ·
`List.grid(fn, rows, cols)` (→ `List<List<'a>>`; calls `fn(row, col)`, both
0-based, per cell — the engine-loop form of a procedural heightmap, e.g.
`Scene.heightmap(List.grid(height, r, c))`) ·
`List.maximum(list)` · `Text.concat(a, b)` · `Text.fromFloat(n)` ·
`Text.fixed(n, decimals)` (fixed-decimal; `Text.fixed(42.0, 0.0)` = `"42"`, the
`%d` shape) · `Text.toBullets(list)` · `Text.split(sep, s)` (→ `List<string>`;
empty `sep` is an error; `Text.split(sep, "")` = `[""]`) · `Text.join(sep, list)`
(strings only) · `Text.parseFloat(s)` (trims; unparseable → `0.0`, the F#
`unwrap_or(0)` shape) · `Math.clamp01(n)` · `Math.sin(n)` · `Math.cos(n)` ·
`Debug.log(label, value)` — `(string, 'a) => 'a`: an Elm-style trace. Logs
`label: <value>` (the value rendered exactly as `functor-lang run`/`trace` displays it —
any type) and returns `value` **unchanged**, so it's pure to the program
result and safe to drop into a pipe: `m.x |> Debug.log("x") |> clamp(0.0, 1.0)`
logs then passes the value on. Label-FIRST / subject-LAST (thread-last), so it
reads Elm-style standalone (`Debug.log("x", m.x)`) AND threads in a pipe. An
impure observability escape hatch — it can't affect the
model/sim (a game with vs without it is byte-identical). Under plain `functor-lang run`
the line prints to stdout; under the runner it routes region-aware to the
CLI's log stream (shown by default — no `-v`; `docs/cli-output.md`) — or the
browser console on wasm. Not rate-limited: a `Debug.log` in `tick`/`draw` fires
every frame (~60/s), so prefer an event path (`input`/`update`), or remove it
when done.

## Functor prelude (only under the engine host — `FunctorHost`)

Available in runner-hosted Functor Lang (and tests via
`functor_runtime_common::functor_lang_prelude`), NOT in plain `functor-lang run`:

```functor
Scene.cube() / sphere() / cylinder() / quad() / plane()   // zero args, enforced
Scene.model("shark.glb")                                   // glTF by path, relative to the
                                                           //   game dir; missing file =
                                                           //   logged error + empty fallback
Scene.group([scene, …])
scene |> Scene.color(r, g, b)                              // scene-last: pipes
scene |> Scene.lit(r, g, b)                                // diffuse+specular
scene |> Scene.litNormalMapped(r, g, b, normalTex)         // + tangent-space
                                                           //   normal map (a
                                                           //   Texture value):
                                                           //   bumps catch the
                                                           //   lights/specular
scene |> Scene.emissive(r, g, b)                           // unlit glow
scene |> Scene.translate(x, y, z)
scene |> Scene.rotateX(angle) / rotateY / rotateZ          // Angle VALUES only:
Angle.degrees(60.0) / Angle.radians(1.57)                  //   never bare numbers
scene |> Scene.scale(k)                                    // uniform
scene |> Scene.scaleXYZ(x, y, z)                            // non-uniform (F#
                                                           //   scaleX/Y/Z): a
                                                           //   wide backdrop
                                                           //   quad, or a
                                                           //   heightmap sized
                                                           //   in XZ with Y
                                                           //   left at author
                                                           //   scale
scene |> Scene.animate(anim)                               // set the pose on the Model
                                                           //   node(s) in the subtree;
                                                           //   Anim VALUES only (the
                                                           //   Angle rule — never a bare
                                                           //   clip-name string). Without
                                                           //   it a skinned model auto-
                                                           //   plays its FIRST clip on
                                                           //   the game clock
Anim.clip("walk", playheadSeconds)                         // a named glTF clip at an
                                                           //   EXPLICIT playhead (loops by
                                                           //   the clip's duration;
                                                           //   negative wraps backwards).
                                                           //   The engine owns no
                                                           //   animation clock — derive
                                                           //   the playhead from tts /
                                                           //   model state, so poses
                                                           //   rewind/replay exactly. An
                                                           //   unknown clip name warns
                                                           //   once + renders the bind
                                                           //   pose (`functor inspect`
                                                           //   lists a model's clips)
Anim.blend([(anim, weight), …])                            // weighted pose mix (lerp T/S,
                                                           //   normalized quat mix for R).
                                                           //   Weights normalize; non-
                                                           //   positive entries drop;
                                                           //   entries may nest blends
Anim.rest()                                                // the bind pose — the base for
                                                           //   purely programmatic posing
                                                           //   (a model with no clips)
anim |> Anim.add(layerAnim, weight)                        // additive layer: the layer's
                                                           //   delta-from-bind on top of
                                                           //   anim (headShake over walk).
                                                           //   Weight clamps to [0,1];
                                                           //   applies only where the base
                                                           //   has influence
anim |> Anim.mask(["jointName", …])                        // restrict anim's influence to
                                                           //   the named joints' SUBTREES;
                                                           //   uncovered joints fall out
                                                           //   (bind pose, or the other
                                                           //   inputs of an enclosing
                                                           //   blend). Unknown names warn
                                                           //   once
anim |> Anim.rotate("jointName", ax, ay, az)               // additive local XYZ rotation
                                                           //   on ONE joint (head aim,
                                                           //   finger curl) — Angle VALUES
                                                           //   only; the joint counts as
                                                           //   fully driven (masks BENEATH
                                                           //   this node can't drop it; an
                                                           //   enclosing mask still can)
Camera.lookAt(ex, ey, ez, tx, ty, tz)                      // up=+Y, fov 45°
Camera.firstPerson(ex, ey, ez, yaw, pitch, fov)           // all three: Angles
Light.ambient(r, g, b) / Light.point(px, py, pz, r, g, b, intensity, range)
Light.directional(dx, dy, dz, r, g, b, intensity) |> Light.castShadows
Light.spot(px, py, pz, dx, dy, dz, r, g, b, intensity, range, coneAngle)
                                                           // cone from pos
                                                           //   along dir;
                                                           //   coneAngle is an
                                                           //   Angle VALUE.
                                                           //   |> Light.castShadows
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
Fog.linear(near, far, r, g, b)                             // Fog VALUES only (the Angle
Fog.exp(density, r, g, b)                                  //   rule); near >= 0, far >
                                                           //   near, density > 0 enforced
                                                           //   with teaching errors
frame |> Frame.withFog(fog)                                // distance fog on all forward
                                                           //   materials incl. emissive;
                                                           //   the fog color is also the
                                                           //   pass's clear color
Skybox.files(px, nx, py, ny, pz, nz)                       // Skybox VALUES only: six face
frame |> Frame.withSkybox(sky)                             //   paths (+X..-Z). Faces are
                                                           //   fetched assets (not checked
                                                           //   in) resolved from the game
                                                           //   dir; while they load the
                                                           //   clear color shows, a failed
                                                           //   face = one warning + no sky.
                                                           //   Fog never fogs the sky
Time.seconds(1.0) / Time.millis(500.0)                     // Duration VALUES only
Sub.every(duration, msg) / Sub.none() / Sub.batch([sub,…]) // what subscriptions returns
Effect.random(tagger) / Effect.now(tagger)                 // one-shots; tagger: (float) => Msg
Sub.connect(url, tagger) / Sub.listen(addr, tagger)        // persistent connections; tagger: (Net.NetEvent) => Msg
Effect.send(connId, text)                                  // send on an open connection
Effect.none() / Effect.batch([fx, …])                      //   random: [0,1); now: epoch secs

Effect.play(sound)                                         // one-shot: fire-and-forget,
Effect.playAt(sound, x, y, z)                              //   non-spatial / positioned
Effect.playThen(sound, msg)                                // one-shot; delivers msg (a VALUE,
                                                           //   not a tagger) through `update`
                                                           //   when the sound FINISHES
AudioSource.ambient(key, sound)                            // soundScape voice: non-spatial bed
AudioSource.at(key, sound, x, y, z)                        //   / positioned emitter (key =
                                                           //   cross-frame identity)
source |> AudioSource.gain(g)                              // source-last: linear gain (1.0=full)
AudioScene.create([source, …]) / AudioScene.empty()       // what `soundScape` returns

Physics.box(w, h, d) / sphere(r) / capsule(halfH, r)       // -> Shape (box = FULL extents)
Physics.dynamic("tag", shape)                              // simulated body
Physics.kinematic("tag", shape) / Physics.fixed("tag", shape)
body |> Physics.at(x, y, z)                                // body-last: pipes
body |> Physics.velocity(vx, vy, vz)
body |> Physics.mass(m) / Physics.friction(f) / Physics.restitution(r)
body |> Physics.sensor                                     // overlap-only, no forces
Physics.scene(gx, gy, gz, [body, …])                       // what `physics` returns
Physics.position("tag")                                    // {x, y, z} of the LIVE body
scene |> Physics.transformed("tag")                        // scene at the body's live pose
Physics.applyImpulse("tag", x, y, z)                       // -> Effect (fire-and-forget)
Physics.applyForce("tag", x, y, z)                         //   force lasts ONE stepped frame
Physics.setVelocity("tag", x, y, z) / Physics.teleport("tag", x, y, z)
Physics.raycast(ox, oy, oz, dx, dy, dz, maxDist, tagger)   // -> Effect (QUERY): tagger gets
                                                           //   {hit, x, y, z, nx, ny, nz,
                                                           //    distance, tag} — hit: false
                                                           //   (zeroed) for a miss
Physics.events(tagger)                                     // -> Sub (from `subscriptions`):
                                                           //   tagger gets {started, a, b,
                                                           //   sensor} per contact begin/end
```

`Physics.position` / `Physics.transformed` read the live stepped world
(Functor Lang runs in the shell's process — no boundary). A tag not in the world is
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
(or a non-dynamic body) is a deduped `[functor-lang]` warning, not an error (the
body may have despawned in flight). `teleport` moves the live body without
touching its declaration (no snap-back next frame). Command effects need
no `update` hook (they produce no message).

`Physics.raycast` is a **query effect**: DEFERRED through the frame's
pre-step drains and performed right after the physics step — "commands
apply at the step; queries answer after it" — so the tagger's record
answers against THIS frame's fresh world, and any model change it causes
is visible in this frame's `draw`. (On a frame where the fixed-step
accumulator takes zero substeps — normal at >60fps — queries carry to the
next simulated frame, like pending commands: they never answer against a
world that hasn't stepped.) Rays see sensor colliders too — a trigger
volume can occlude the solid body behind it. The tagger may be a plain closure
(`(hit) => hit` makes the record itself the message) or a ctor. A `GotHit`
handler chaining a command queues it for next frame's step; chaining
another query answers immediately (the world already stepped). Under the
fake/replay runners raycasts are canned/recorded — physics-query logic is
testable with no world at all.

`Physics.events` is a **Sub** (return it from `subscriptions`, alone or in
`Sub.batch`; it requires `update`). Every contact begin/end from this
frame's physics step arrives post-step as `{started: bool, a: Text,
b: Text, sensor: bool}` — `a`/`b` are the pair's tags in rapier's
(deterministic) order, so check both; `sensor: true` marks an overlap with
a `Physics.sensor` body (no contact forces). Events for a pair whose body
was despawned this frame are dropped (there is nothing left to name), and
a frame's undelivered events never carry over.

The physics drive is **recorded** (docs/physics.md), but time travel is the
SHELL's tool, not a game API: the runner's scrubber overlay (`~` on desktop,
the DOM scrubber on web) pauses, scrubs, and rewinds the whole scene — the
MVU model and the physics world together (docs/time-travel.md). Resuming
from a scrubbed frame **branches** — the old future is discarded. History
is bounded (~15s at 60Hz). Everything is deterministic: replaying identical
inputs from a rewind reproduces the run byte-for-byte. (The game-authored
timeline effects — `Physics.pause`/`resume`/`stepOnce`/`rewindTo`/
`timelineFrame` — were removed when the whole-game scrubber superseded
them.)


A runner-hosted game (`functor -d <project-dir> run native`, with
`functor.json` selecting `game.fun`) defines:

```functor
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
let soundScape = (model) => AudioScene.create([source, …])  // OPTIONAL; continuous
                                            // looping voices, reconciled by key each
                                            // frame (needs no `update`)
```

Subscription timers are **stateless**: `Sub.every` fires when an integer
multiple of its period lies in `(prevTts, tts]` — the global time grid, so
a long frame fires ONCE (missed boundaries collapse) and timers tick right
through a hot reload. Fired messages fold through `update` before `tick`.
Durations, like Angles, are branded values — `Sub.every(0.5, …)` is a
teaching error; say `Time.seconds(0.5)` or `Time.millis(500.0)`.

A bare-model arm and a `(model, effect)` arm may mix in one match — the
checker lifts bare to `(model, Effect.none())`, matching the producer.
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

A project dir with `functor.json` `{"language": "functor-lang", "entry": "game.fun"}`
works with the CLI: `functor -d dir build` (typecheck, diagnostics are
errors), `run native`, `develop` (hot reload is built in), and `run wasm`
(the `.fun` ships as text and is interpreted in the browser; file-watch hot
reload is native-only — reload the page to pick up saved edits, or push
source with a `{ type: "functor-lang-set-source", source }` postMessage to the page
for a model-preserving in-place reload; the VSCode **"Functor Lang: Open Live
Preview"** command does exactly that from the live buffer as you type).

`examples/hello/game.fun` is the reference
(`examples/physics/game.fun` for the physics hook, including the
rising-edge input latch — GLFW key repeats arrive as `isDown = true`).
The model shows live at the
debug server's `GET /state`. **Hot reload is on by default**: saving the
`.fun` file reloads it in ~1 frame with the model preserved (a broken edit
keeps the old program running; an edited `init` takes effect on restart).
Closures **stored in the model** rebind too: they adopt the edited code
with their captured values carried over (matched by the enclosing def's
name; a closure whose def was renamed/deleted keeps its old body with a
loud `[functor-lang] reload:` warning).

Transforms wrap in Group nodes: the **outer call applies last in world
space** — `s |> Scene.rotateY(r) |> Scene.translate(x, 0.0, 0.0)` rotates in
place, then moves (the order the source reads). Engine values (`<Scene>`,
`<Camera>`, `<Frame>`) are opaque: they can be passed around but not
inspected, compared, or serialized.

## Typechecking model (Hindley–Milner + gradual seams)

`functor-lang check` runs REAL INFERENCE (B7): unannotated code gets full types via
unification with let-polymorphism — generic functions instantiate fresh at
every use, element types flow through `List.map`/`filter`/`fold`, and
apostrophe-prefixed annotation names are type variables (`(xs: List<'a>, seed: 'b): List<'b>`). Inference has teeth: unannotated bad calls, mixed-element
lists, and contradictory `mut` use are errors now. `Unknown` remains ONLY
at genuinely-dynamic seams (host values, unrecognized type
names) and absorbs anything. (Function TYPES cannot be written in
annotations yet — `f: ('a) => 'b` does not parse; leave higher-order
parameters unannotated and let inference type them.) Generic declarations (`type Pair<'x, 'y> = { first: 'x, second: 'y }`)
instantiate fresh per use; an UNDECLARED type variable in a declaration is
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
(see `docs/functor-lang.md` Track B/C checkboxes), update this skill in the same PR.
