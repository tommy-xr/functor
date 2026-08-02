---
name: functor-lang
description: Write, run, and debug Functor Lang (.fun) — Functor's F#-inspired game-logic language. Use whenever creating or editing .fun files, answering Functor Lang syntax/semantics questions, or debugging Functor Lang parse/run/check errors. Functor Lang is a custom language — do NOT guess from F#/OCaml intuition; this skill is the source of truth for the current subset.
---

# Functor Lang — the current language, exactly

Functor Lang is Functor's interpreted game-logic language (roadmap:
`docs/functor-lang.md`). It is deliberately small; this
file describes **everything that exists today**. If a construct isn't here,
it doesn't parse — do not invent syntax from F#/OCaml habits.

## Quick facts (do NOT guess from F#/OCaml)

The habits that break first, in one place. Each is expanded below.

- **Assignment is `:=`**, only on a `let mut … in` slot, and it must be
  followed by `;` and a continuation expression. `<-` does not exist yet.
- **Pipelines are thread-LAST**: `x |> f(a)` is `f(a, x)` — every builtin and
  prelude function takes its subject (list, scene) LAST.
- **`if cond then a else b` is an EXPRESSION**: both branches required (no
  else-less `if`), chained with `else if` (there is no `elif`). A
  bool-literal `match` is equally valid.
- **Operators are exhaustive**: `+ - * /`, `< > <= >= == !=`, `&& || not`.
  There is no `<>` (inequality is `!=` only), no `%` (`Math.mod`), no `^`
  (`Math.pow`), and no string-concatenation operator (use `$"…"`).
  `+ - * /` and `== != < > <= >=` also work on BRANDS that declare them —
  `90deg + 45deg`, `1.5s - 200ms`, `45deg * 2.0`, `90deg == 90deg`,
  `1.5s < 2000ms` (see Units below).
- **No loops** — iterate with `List.map` / `List.filter` / `List.fold`.
- **All numbers are `float`** (f64); primitive type names are lowercase
  (`float`, `string`, `bool`), while generic containers are `List<…>` and
  `Map<…, …>`.
- **Every `match` arm and every variant alternative needs a leading `|`**,
  the first one included. Arm bodies are full expressions, so a nested
  `match` must be parenthesized.
- **Nullary constructors take no parens** (`Point`, never `Point()`), and
  constructors resolve bare — type-qualifying one (`Shape.Circle`) is a load
  error telling you to write `Circle`.
- **Local bindings are `let … in`** inside a body; top-level defs are
  mutually visible.
- **File = module**: every sibling `.fun` in the entry's directory loads with
  the project, referenced or not. A file may also declare **inline** modules
  (`module Server { … }`, one level deep) — see Modules below.
- **The engine prelude (`Scene.*`, `Camera3D.*`, `Frame.*`, `Physics.*`) exists
  only under the runner host**, not in plain `functor-lang run`. Its branded
  values refuse bare numbers: `Angle.degrees(60.0)`, `Time.seconds(0.5)` — or
  their **unit-suffix literals**, `60deg` / `0.5s` (see `unit` below).
- **A game is `init` / `tick` / `draw`** plus optional hooks — see The game
  contract below.

## Verification loop (always available, no GPU)

Builtin-module MEMBER names are validated at `check` time: `List.tail`,
`List.partition`, `Text.padLeft` (none exist) are check ERRORS with a near-miss
hint or the namespace's member list — a typo in a `List.*` / `Text.*` /
`Math.*` call no longer survives to runtime. Note this is a hard error with
no escape hatch, and it gates hot-reload: a builtin typo in a DEAD branch
now fails `build`/reload where it previously ran.

```sh
cargo run -q -p functor-lang -- parse file.fun    # surface AST (spans on every node; this file only)
cargo run -q -p functor-lang -- ir file.fun      # name-resolved core IR (merged project)
cargo run -q -p functor-lang -- run file.fun     # evaluate: main()'s result, or the entry's bindings
cargo run -q -p functor-lang -- trace file.fun   # enter/exit call story with values (kept on failure)
cargo run -q -p functor-lang -- check file.fun   # typechecker: ALL diagnostics, exit 1
cargo run -q -p functor-lang -- test file.fun    # run the `expect` tests: per-test ok/FAILED, exit 1
```

`ir`/`check`/`run`/`trace`/`test` treat the file as a PROJECT ENTRY: every sibling
`.fun` in its directory loads with it (file = module — see Modules below),
so scratch files must live in their own directory, not a shared one.

Errors are always `file:line:col: error: message`. Tests live in `functor-lang/tests/`
with goldens next to `functor-lang/examples/` (`UPDATE_GOLDENS=1 cargo test -p functor-lang`
regenerates). VSCode gets live parse/lower/type diagnostics,
`name : Type` hover (with the doc-comment block above the definition — the
`.funi` prose for prelude calls), and go-to-definition via
`tools/functor-lang-lsp` — including JUMPING INTO the prelude: definition on
`Scene.cube` opens the materialized `Scene.funi` interface at its signature.

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
  | Rect(w, _) => w * w                       // sub-patterns: names, _, or literals (no deeper nesting)
  | Point => 0.0                              // exhaustiveness checked when s's type is known

let sizeOf = (s: Shape): string =>
  match area(s) > 10.0 with                   // bool-literal match: still valid, and the
  | true => "big"                             //   general form (number/string literal arms
  | false => "small"                          //   exist too — they need a catch-all
                                              //   `| x =>` or `| _ =>`)

let sizeOf2 = (s: Shape): string =>
  if area(s) > 10.0 then "big" else "small"   // if/else EXPRESSION — both branches required
                                              //   (no else-less form); branches must unify
let grade = (n: float): string =>             // `else if` chains — just an `if` in the
  if n > 90.0 then "A"                         //   else position, no `elif` keyword
  else if n > 80.0 then "B"                   //   (only the taken branch is evaluated)
  else "C"

let threshold = 10                            // top-level let; ints/floats are all float (f64)
type Px = | Px(value: float)                  // a single-ctor brand…
unit px = Px                                  // …with a literal SUFFIX: `16px` == `Px(16.0)`
let width = 16px                              //   the suffix must TOUCH the digits (`16 px` is
                                              //   two tokens); `-2.5px` == `Px(-2.5)`
unit px (+) = addPx                           // …and arithmetic on the brand: `16px + 4px`
unit px (<) = lessPx                          // …and ordering: `4px < 16px`, plus > <= >=
let origin: Position = { x: 0.0, y: 0.0 }     // OPTIONAL binding annotation `let name: Type = …`
                                              //   (checked against the value; also on `let … in`)
let scores = [1.0, 2.0, 3.0]                  // list literal; [x, ..xs] prepends
let sumList = (xs: List<float>): float =>     // list PATTERNS: [] / [a,b] / [h, ..t]
  match xs with
  | [] => 0.0
  | [head, ..rest] => head + sumList(rest)    // refutable; needs a catch-all or [..r]
let s = "text\n"                              // strings: escapes \" \\ \n \t
let label = $"score: {threshold}; {{ready}}"  // interpolation: `$"…"` with full expressions in {}
                                              //   strings inline raw; other values use canonical
                                              //   display; {{ and }} are literal braces
let flag = true                               // bools

let isHigh = (score: float): bool => score > threshold   // annotations OPTIONAL (gradual)
let inRange = (n: float): bool => n >= 0.0 && n <= 1.0   // `<=` `>=` `!=` are ordinary
let changed = (a: float, b: float): bool => a != b       //   comparisons (no `<>`)
let describe = (score) => $"score: {score}"

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

Operators: `+ - * /` `< > <= >= == !=` (conventional precedence; pipelines
bind loosest), unary `-`, and the **short-circuiting booleans** `&&` / `||`
plus prefix `not`. That list is EXHAUSTIVE — there is **no `<>`, `%`, or
`^`**. Inequality is `!=` ONLY; F#'s `<>` is not an alias (it lexes as `<`
then `>` and fails as an ordinary parse error), and bare `!` is not prefix
negation (that stays `not`) — `!` exists only as part of `!=`.
`<=`/`>=` are valid wherever `<`/`>` are (both operands must be `float`),
and `!=` is the exact NEGATION of `==`: same operand rules, same errors on
the same inputs — comparing functions with `!=` is the same check-time
rejection (`` functions cannot be compared with `!=` ``), and host/engine
values are the same runtime error.
Modulo is `Math.mod(a, b)` and exponentiation `Math.pow(base, exp)`.
Precedence (tightest→loosest): comparisons > `not` > `&&` > `||`
> pipelines. So `not a == b` is `not (a == b)`, `a || b && c` is
`a || (b && c)`, and all three operands are checked as `bool`. `&&`/`||`
short-circuit — `false && e` / `true || e` never evaluate `e` (so
`isReady && risky()` is safe).

The conditional is `if cond then a else b` — a full **expression**, so
**both branches are required** (there is no else-less `if`; omitting the
`else` is a parse error, since the expression must yield a value). Chain with
`else if` (an `if` in the `else` position — no `elif` keyword); only the taken
branch is evaluated, and the branches must unify to one type (like `match`
arms). The condition must be `bool` (comparisons/`&&`/`||`/`not` all fit: `if a
&& b then …`). The `if` form sits at the loosest level (like `match`/`let … in`),
so a trailing pipe folds into the else branch: `if c then a else b |> f` is `if
c then a else (b |> f)`. The **bool-literal match** (`match x > 3.0 with | true
=> a | false => b`) remains equally valid — it and `if`/`else` share the same
lazy branch selection (only the taken branch runs). There are **no** loops or
string-concatenation operator —
iteration is `List.map/filter/fold`. Tuples are structural:
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
`pure-pipeline.fun`). A non-identifier stem is NOT a load error — the file
is **skipped with a warning** (`[functor-lang] ignoring pure-pipeline.fun —
its file stem is not a valid module identifier (editor temp file?); rename
it to load it as a module`) and the rest of the project loads normally. So
a hyphenated file contributes nothing; rename it to make its definitions
visible.

A project may instead declare named **`entries`** (`{"entries": {"client":
"client.fun", "server": "server.fun"}}`) — multiple program roots over the
SAME directory of sibling modules; `functor --entry <name>` picks one
(default `client`, or the sole entry). Each entry is its own program (its
bindings are the bare `init`/`tick`/`draw` roots; the other entry is just a
qualified sibling module), so roles share code — a `client.fun` and a
`server.fun` share a `protocol.fun` wire codec — without drifting. A role may also
be an object pointing at a **shared file**, so two roles live in ONE file and
hot-reload atomically. The preferred form on native names an **inline module**
—
`"server": { "file": "game.fun", "module": "Server" }` — whose members ARE
that role's contract (`Server.init`/`Server.tick`/`Server.draw`/…, resolved
against the block's canonical path; an unknown block name is a load/build
error listing the blocks the file does declare). The transitional form names
a binding **prefix** — `"server": { "file": "game.fun", "prefix": "server" }`
— resolving every canonical entry binding through the prefix as camelCase
(`serverInit`/`serverTick`/…). A role declares at most one of the two.
`examples/orbs` is the multiplayer reference and the SAME-FILE shape — one
`game.fun` whose `client` and `server` roles are BOTH inline modules of it
(`module Client { … }` / `module Server { … }`), with the protocol, the world
step and the renderer shared bare above them, so one buffer hot-reloads both
roles atomically (its sandbox client and server panes boot the same source at
`?module=Client` / `?module=Server`). The roles-as-FILES form — one `.fun` per
role over shared siblings — is the shape to reach for once roles outgrow one
buffer or want independent deploy units; it is equally supported but ships with
no bundled example.
**Both forms run on every shell.**
`run wasm`/`build wasm` bake the
role into the served/exported page's boot config (`window.__functorLangEntryModule`
/ `…EntryPrefix`; the site player takes `?module=<Ident>` or `?prefix=<ident>`,
one of the two). On **vr** the role rides each device push as a query string
(`POST /load-project?module=Server` / `?prefix=server`), so the APK's embedded
producer re-resolves it on every re-push exactly as native re-resolves it on
every save. `entry` and `entries` together are refused.

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
  shapes |> List.map(area) |> List.maximum |> Option.defaultValue(0.0)
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
  builtin/prelude or bundled-core namespace (Net, Key, Mouse, Random, Option, Result,
  List, Map, Text, Math, Debug, Scene,
  Sprite, Anim, Asset, Camera3D, Camera2D, Frame, Light, Fog, Color, Vec3, Skybox,
  Angle, Texture, Time, Input, Sub, Effect, Physics, RenderTarget, Ui, Html, Attr,
  Style, AudioSource, AudioScene) is a
  load error — rename the file. (`assets.fun` → `Assets` — the generated
  manifest — is fine; only the exact name collides.)
- **`Net` is a built-in module**, always in scope: `type NetEvent =
  | Connected(id: float) | Message(id: float, text: string) |
  Data(id: float, value: NetData) | Disconnected(id: float) |
  Error(id: float, text: string)`. A `Sub.connect`/
  `Sub.listen` tagger receives these — `match ev with | Net.Connected(id)
  => …` — with no declaration needed. The same module also declares
  `type HttpResponse = | Response(status: float, body: string) |
  Failure(error: string)`, the value an `Effect.httpGet`/`httpPost` tagger
  receives (`Response` = the request completed at ANY HTTP status;
  `Failure` = a transport error). `Data` carries an `Effect.sendMsg`
  payload decoded back into a plain-data value; its field type is
  deliberately `unknown` (the EXPLICIT gradual seam), so the bound value
  matches directly against whatever ADT the two ends share — declare the
  protocol ADT once in a shared sibling module and match its ctors
  (`match w with | Protocol.Ping(n) => …`). A corrupt/version-skewed typed
  frame arrives as `Net.Error`, never as garbled `Message` text. Ctors
  match by CANONICAL tag (module prefix included) — the same ADT declared
  separately on each end, or entry-declared on one side, tags differently
  and falls through the peer's catch-all silently: share ONE module.
  Non-finite numbers (NaN/Infinity) in a payload are teaching errors at
  the `sendMsg` call site (JSON cannot carry them).
- **`Key` is a built-in module**: `Key.t`, the variant the `input` hook's
  `key` parameter carries — `Key.A`..`Key.Z`, `Key.Up`/`Down`/`Left`/`Right`,
  `Key.Space`/`Enter`/`Escape`, and the digit row as `Key.Num0`..`Key.Num9`
  (NOT bare digits). Match constructors (`| Key.W =>`) or compare
  (`key == Key.Enter`); a typo (`Key.Enterr`) is a load-time error — the
  reason keys stopped being strings. `Random` is likewise built-in (the
  abstract `Random.Seed` — see the Random builtins above).
- **`Mouse` is a built-in module**: `Mouse.t`, the variant the `mouseButton`
  hook's `button` parameter carries — `Mouse.Left`, `Mouse.Right`,
  `Mouse.Middle`. The `Key` story exactly: match constructors
  (`| Mouse.Left =>`) or compare (`button == Mouse.Right`), and a typo is a
  load-time error. Buttons the platform reports that Functor does not name are
  never delivered (no `Unknown` variant reaches game logic).
- **`Input` is the engine's continuously sampled input module.**
  `Input.snapshot` contains keyboard levels and fixed-step transitions:
  `heldKeys`, `pressedKeys`, and `releasedKeys` (all `List<Key.t>`). Its
  top-left-origin logical `mouse` record has `x`, `y`, `surfaceWidth`,
  `surfaceHeight`, and three `{left,right,middle}` sets: `buttons` for held
  levels plus `pressed` / `released` for transitions. Position and surface
  extent share one coordinate space — GLFW window points natively, CSS pixels
  on web — so they are stable across Retina/device-pixel-ratio changes.
  `Camera2D.toWorld` treats the ideal half-open fit in that logical space as
  authoritative; only the renderer rounds its corresponding viewport to
  physical framebuffer pixels.
  A quick tap can appear in both edge sets while the held set carries the final
  level. The snapshot also contains
  `xr: Option.t<Input.xr>`. XR head/grip/aim poses are rig-local
  plain data: position `{x,y,z}` and quaternion `{x,y,z,w}`, with +X right,
  +Y up, and -Z forward. Each controller also carries `active`, analog
  trigger/squeeze/thumbstick state, and named button booleans. Missing XR,
  inactive controllers, and temporarily invalid poses are explicit
  `Option.None`/`active: false`, never stale values. The snapshot also
  contains `gamepad: Option.t<Input.gamepad>` — the primary connected pad's
  held state, `Option.None` while no pad is connected. Sticks are `point2` in
  `-1..1` with **up-positive `y`** (the XR thumbstick convention); triggers
  are `0..1`; face buttons are POSITIONAL (`south` is the bottom face button —
  A on Xbox, Cross on PlayStation), plus bumpers, stick clicks, dpad, and
  `start`/`select`. Levels only, raw values: detect edges against your model
  and apply your own deadzone. A future mobile-touch domain belongs as another
  typed sibling on the snapshot.
- **Bundled modules use the ordinary module semantics.** The language-owned
  `Net.fun` / `Key.fun` / `Mouse.fun` builtins, `Random.funi` interface, and
  `Option.fun` / `Result.fun` standard-library implementations are in-memory
  sources distributed with every embedding. Engine hosts additionally bundle
  `Animator.fun`, a pure Functor Lang crossfade helper built on the host's
  `Anim` interface. Hosts may inject additional bundled `.fun`
  implementations and `.funi` interfaces through the same project linker:
  they parse, lower, check, evaluate, participate in dependency ordering,
  appear in source maps (`<stdlib>/…` / `<prelude>/…`), and rebind stored
  closures like project modules. Their namespaces are reserved automatically.
  They are fixed for the lifetime of the host binary and therefore are
  re-linked on a project hot reload but are not themselves file-watched.
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

### Inline `module` declarations

A `.fun` file may also declare **named modules inline** — one file, several
namespaces. This is how a single-file multiplayer game separates its
authoritative and presentation halves without splitting into files:

```functor
// game.fun                                   → module Game (the entry)
type World = { tick: float }                  // shared, at the FILE's top level

module Server {
  type Cmd = | Spawn(id: float) | Despawn(id: float)
  let step = (c: Cmd, w: World): World =>     // `World` is visible bare
    match c with
    | Spawn(id) => { tick: w.tick + id }
    | Despawn(_) => { tick: w.tick - 1.0 }
  expect step(Spawn(2.0), { tick: 1.0 }).tick == 3.0
}

module Client {
  type Cmd = | Spawn(id: float)               // `Spawn` again: ctor uniqueness
  let describe = (w: World) => $"tick {w.tick}"   //   is per MODULE
}

let tick = (m, dt, tts) => Server.step(Server.Spawn(1.0), m)
```

- **One level.** `module` inside `module` is a parse error ("nested modules
  are not supported yet"). A `module` in a `.funi` is a parse error too.
- **Items allowed inside**: `let`, `type`, `expect`. `open` inside a module
  body is a parse error for now — put `open`s at the file's top level (they
  are in scope inside the modules too).
- **Canonical names** carry the extra segment. The ENTRY file's members stay
  bare, so `game.fun`'s `module Server` yields `Server.step` /
  `Server.Spawn`; a sibling `utils.fun`'s `module Grid` yields
  `Utils.Grid.cell` / `Utils.Grid.Full`. That is what `run`/`trace`/`/state`
  display and what closures rebind by.
- **Referencing them**: bare-qualified inside the declaring file
  (`Server.step`), fully qualified from a sibling (`Game.Server.step`) — in
  expressions, type annotations (`(c: Game.Server.Cmd)`), and constructor
  patterns (`| Game.Server.Spawn(id) =>`).
- **Name resolution inside a module body**: locals → the module's own
  defs/ctors/types → the enclosing FILE's top-level names → file-level
  `open`ed names → builtins/externals. The module's own names **shadow**
  same-named file-level ones (they are distinct canonical defs, not a
  collision) — including record types, so a bare literal inside the module
  picks the module's own shape. Top-level code references module members
  qualified only.
- **`open` works on them**: `open Server` for a module declared in the same
  file, `open Game.Server` for a sibling's — same collision rules as any
  `open`, and (like any cross-file reference) an `open` of a sibling's
  inline module is a dependency edge on the owning FILE.
- **Ctor uniqueness is per module**, so `Server` and `Client` may each
  declare `Spawn`.
- **Collisions are load errors** naming both sides. A module name occupies
  its file's value AND type namespaces, and it is reachable BARE inside its
  own file, so it may not collide with:
  - a top-level `let` / constructor / `type` in the same file,
  - another inline module in the same file,
  - a name an `open` brings in (`open Utils` exporting a `Server` next to a
    `module Server`),
  - a protected/bundled namespace — `module Scene` is refused in ANY file,
    not just the entry, because it would silently steal every `Scene.cube`
    in the declaring file,
  - another FILE's module name (`module Server` + a `server.fun` sibling).

  Two DIFFERENT files may each declare `module Grid`: those canonicalize
  apart (`Utils.Grid` / `Helpers.Grid`) and neither shadows the other.
- **Dependency edges are between FILES**: a sibling's `Game.Server.foo`
  records a dep on `Game`, so cycles are detected exactly as before.
- **The default entry contract is unchanged**: `init`/`tick`/`draw` are bare
  top-level lookups, so `module Main { let tick = … }` does NOT satisfy a
  plain entry. A functor.json ROLE may opt into a block instead —
  `"server": { "file": "game.fun", "module": "Server" }` resolves
  `Server.init`/`Server.tick`/… (see `entries` above). Such a role runs on
  every shell — on vr it rides the device push's query string
  (`?module=Server`), which needs a tool APK speaking debug protocol v9 or
  newer (`run vr` refuses an older one rather than letting it boot the
  unprefixed contract).
- **The shadowing trap when a role moves into a block.** A block's own names
  shadow the file's, so `module Client { let init = init }` is
  **self-referential**, not an alias of a top-level `init` — you get
  `error: global Client.init used before its definition`, and the same goes
  for any `tick`/`draw`/`update` the file also defines at top level. Give
  shared values names the contract does not use (`initialGame`, `board`) and
  let each block reference those. The same rule holds for TYPES: a
  `module Client` may not sit next to a top-level `type Client` (a module
  name occupies its file's value and type namespaces — that is a collision
  load error, not shadowing), so move the role's model type INTO the block
  (`Client.Model`).
- **Hot reload**: canonical names are the rebind identity, so an inline
  module's closures rebind normally — but MOVING a def into (or out of) a
  module renames it (`step` → `Server.step`), which the rebinder treats
  like any rename: the stored closure keeps its OLD body and prints a loud
  `[functor-lang]` warning.
- **In the editor**: completion follows the CURSOR into a block — inside
  `module Server { … }` the module's own names are offered bare (shadowing
  the file's, which stay visible), `Utils.` offers the nested module `Grid`,
  `Utils.Grid.` offers its members, and inside the declaring file a block is
  referenced bare (`Grid.`). Hover shows the canonical name
  (`Server.step : …`), and each `module` block is a folding range.

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
  crate ships a `.funi` for every host namespace (`Scene`, `Sprite`, `Asset`,
  `Camera3D`, `Camera2D`, `Frame`, `Light`, `Fog`, `Skybox`, `RenderTarget`,
  `Texture`, `Angle`, `Time`, `Sub`, `Effect`, `Physics`, `Ui`, `Html`, `Attr`,
  `Style`, `AudioSource`, `AudioScene`),
  loaded by the
  runner so engine calls carry real types (no longer `Unknown`). Each module's
  primary opaque handle is `Mod.t` (`Camera3D.t`, `Frame.t`, `Effect.t`, …);
  modules that own several name each (`Scene.t`; `Physics.shape`/`body`/`world`;
  `Ui.view`/`anchor`; `Html.node`/`Attr.t`; `Asset.Model`/`Texture`/`Sound`). Physics query/event results are records
  (`Physics.position`, `Physics.rayHit`, `Physics.collisionEvent`).
- Prelude interface documentation uses `//!` for the module overview and `///`
  directly above a type/signature for its public API prose. Plain `//` remains
  an ordinary comment and still appears in user-code hover for backwards
  compatibility. `functor docs` renders the embedded API as Markdown or
  JSON — the engine prelude AND the language standard library (`List`, `Map`,
  `Text`, `Math`, `Random`, `Debug`, `Option`, `Result`, `Key`, `Mouse`,
  documented in `functor-lang/stdlib/`), as two groups; in the repository,
  `npm run generate:docs` recreates the gitignored local reference artifacts
  and `npm run check:docs` validates both renderers. Both repository scripts
  reject a module, type, or signature without explicit public documentation.
  The builtin namespaces have no Functor Lang source of their own, so their
  `.funi` documentation files are pinned to `builtin_signature` by a drift
  test — a builtin added or retyped without a documentation update fails
  `cargo test -p functor-lang`.

## Inline tests (`expect`)

`expect <bool-expr>` is a top-level ITEM — an inline test written next to
the code it exercises:

```functor
let area = (s: Shape): float => …

expect area(Point) == 0.0
expect area(Rect(3.0, 4.0)) == 12.0
expect (                                      // any expression works — a
  let m = tick(init, 0.016, 0.016) in         //   let-in chain is the
  m.score == 0.0                              //   multi-step setup block
)
```

- **Unnamed** — the span (`file:line`) is the test's identity. `expect` is
  contextual (the `open` rule): only item position means a test, so the
  name stays usable everywhere else. `.funi` files refuse it.
- The expression must CHECK as `bool` (`check`: "an `expect` test:
  expected bool, got …").
- **Inert in the game loop**: `run native`/`run wasm`/`Session::load`
  never evaluate expects — only the test commands below do (defs load
  first, then each expect independently; one failure never stops the
  rest; exit 1 on any failure). Sibling-module expects load and run with
  the project.
- A failed TOP-LEVEL comparison (`==`/`<`/`>`) is decomposed: the report
  carries both sides' actual values (`left: 12, right: 12.5`).
- Pipe-then-compare needs parens (pipelines bind loosest):
  `expect (xs |> List.map(f)) == ys` — without them the `==` folds into
  the pipe's argument.
- Floats compare exactly; for computed floats prefer
  `Math.abs(a - b) < 0.001` over `==`.
### Running them: `functor test` (a GAME) vs `functor-lang test` (pure logic)

```sh
functor -d examples/counter test                # a game project, under the ENGINE prelude
cargo run -q -p functor-lang -- test file.fun   # a plain .fun, no engine
```

**Use `functor test` for anything in a game directory.** It typechecks the
project (the `build` gate), then evaluates every expect in the entry and
its siblings under the real engine host — headlessly: no GPU, no window,
no game loop. Each failure prints at its `file:line:col` with the source
line and, for a top-level comparison, both sides' values; the command
exits non-zero if any expect failed. A project with no expects is a pass.
In a multi-entry project `--entry` picks which entry is TYPECHECKED; it does
not narrow the tests, because `file = module` loads every sibling either way.

`functor-lang test` is the LANGUAGE crate's dev command and runs under the
plain `NoHost` prelude, where `Scene.*` / `Color.*` / `Physics.*` don't
exist. Because `file = module` loads every sibling, pointing it at a game
directory fails on the first engine call in ANY sibling — typically a
top-level def like `let sky = Color.rgb(…)`, which aborts the def load
before a single expect runs. Reach for it only for engine-free `.fun`
files. Don't copy pure modules to a scratch directory to work around this;
that's what `functor test` is for.

- Expects may freely call engine externals under `functor test`
  (`Scene.*`, `Color.*`, …): no external performs IO or touches GL, and
  `Effect.*` only builds a *descriptor* — nothing is performed. Note that
  opaque engine values (`Scene.t`, `Frame.t`, `Effect.t`, `Color.t`,
  `Vec3.t`, …) support no `==` — the CHECKER rejects it now, naming the
  type — so assert on numbers/records you derive instead. Brands DO
  compare: angles, durations (`90deg == 90deg`, `1.5s < 2000ms`), and
  `Physics.tag`. The highest-value tests are pure logic anyway:
  model/`tick`/`update` math.

## Units: suffixed literals and their operators (`unit`)

A numeric literal may carry a **unit suffix** — `90deg`, `0.5s`, `16px` — which
is exactly the call the suffix's `unit` declaration names, and a brand may
declare **arithmetic and comparison** on itself (`90deg + 45deg`,
`1.5s < 2000ms`). Full design: `docs/functor-lang-units.md`.

```functor
unit deg = Angle.degrees            // a top-level ITEM, in `.fun` and `.funi`
unit px = Px                        // target: any (float) => 't function OR constructor

let turn = Scene.rotateY(90deg)     // == Scene.rotateY(Angle.degrees(90.0))
let beat = Sub.every(0.5s, Tick)    // == Sub.every(Time.seconds(0.5), Tick)
```

- **Adjacency is the rule**: the suffix must touch the digits. `90 deg` is
  still a number and a name; `16px2` is one suffix (`px2`), never a split.
- **A prefix minus folds in**: `-90deg` is `Angle.degrees(-90.0)`, not a
  negation of the branded value (unary minus on a brand is not declarable).
  Binary subtraction is untouched.
- **Units are project-wide**, like constructors: a suffix declared in ANY
  module means the same thing in every module, and declaring one twice
  anywhere in the project is an error.
- **The target is typechecked as exactly `(float) => 't`** at the
  declaration, and it is a NAME (`Angle.degrees`, `Px`, `Utils.Meters`), never
  an expression.
- **It desugars at load**: the IR holds the ordinary call, so hover, inlay,
  go-to-definition, typechecking, and runtime teaching errors all see
  `Angle.degrees(90.0)` — and there is no per-frame cost.
- **An undeclared suffix is a load/check error** listing the declared units,
  and a bare number in a branded position now teaches both spellings
  (``expected Angle.t, got float — write `90deg` or `Angle.degrees(90.0)` ``).
- **Built-in suffixes (engine prelude only)**: `deg` / `rad` (`Angle.degrees`
  / `Angle.radians`) and `s` / `ms` / `us` / `min` / `hr` (`Time.seconds` /
  `millis` / `micros` / `minutes` / `hours`). They are declared in
  `angle.funi` / `time.funi`, so — like every prelude name — they exist only
  under the runner host, not in a plain `functor-lang run`.
- **`unit` is contextual** (the `open` / `expect` / `module` rule): only item
  position declares one, so the name stays usable everywhere else.

### Operators on a brand (`unit px (+) = …`)

```functor
type Px = | Px(value: float)
unit px = Px
unit px (+)  = (a, b) => Px(unwrap(a) + unwrap(b))  // (Px, Px) => Px
unit px (*)  = (a, k) => Px(unwrap(a) * k)          // (Px, float) => Px
unit px (==) = (a, b) => unwrap(a) == unwrap(b)     // (Px, Px) => bool
unit px (<)  = (a, b) => unwrap(a) < unwrap(b)      // (Px, Px) => bool

let total: Px = 16px + 4px          // …and 3px * 2.0, and 2.0 * 3px
let ordered = 4px < 16px            // …and >, <=, >=, ==, != (all derived)
```

- **Six declarable operators: `+` `-` `*` `/` `==` `<`.** `+` and `-` are
  typechecked as `('t, 't) => 't`; `*` and `/` as the SCALAR `('t, float) => 't`
  (a brand times a brand would be a different
  type — Functor Lang does not do dimensional analysis); `==` and `<` as
  `('t, 't) => bool`. The other four comparisons are **DERIVED**, never
  declared — `a != b` is `not equals(a, b)`; `a > b` is `less(b, a)`;
  `a <= b` is `not less(b, a)`; `a >= b` is `not less(a, b)`. Writing
  `unit px (>) = …` is a parse error naming the base to declare instead. (One
  honest consequence: for a BRAND, `<=` is the negation of `<`, so a NaN-built
  brand compares as an ordinary value under `<=`/`>=` where a raw float would
  be false. Float comparison itself is unchanged.)
- **The operator belongs to the BRAND, not the suffix.** `s`/`ms`/`us`/`min`/
  `hr` are all `Time.t`, so one declaration serves them all and `1.5s - 200ms`
  works. Declaring the same brand + operator twice — through ANY suffix — is a
  duplicate error.
- **The implementation is an expression**: a name, or a lambda (the prelude's
  `.funi` declarations name host externals). It is checked against the shape
  above at the DECLARATION, and RESOLVED only when the operator dispatches — a
  name is late-bound like any global (so it must be defined above any top-level
  constant that uses the operator), and a host external is looked up at the use
  site, so these declarations still load under the plain, hostless
  interpreter.
- **The brand must be distinguishable at run time** — a single-constructor
  variant, or a host type like `Angle.t`. A record brand or a multi-constructor
  type is a check error at the declaration (the interpreter dispatches on a
  value's tag).
- **Scaling commutes, division does not**: `2.0 * 45deg` works (same call,
  arguments swapped); `2.0 / 45deg` is an error.
- **Resolution is ad-hoc overloading AFTER inference**: a node whose operand
  resolves to a brand with that operator becomes that call; everything else is
  float arithmetic exactly as before. A node whose operands AND result all stay
  unsolved is a teaching error asking for an annotation, never a silent float
  guess — ```+` here could be float arithmetic or `Px` arithmetic — annotate an
  operand (e.g. `(a: Px)`)``. So `(a, b) => a + b` needs an annotation in a
  project that declares `+` on a brand, while `(a, b): Px => a + b` (the RESULT
  decides it for `+`/`-`), `(a, b): float => a + b`,
  `(a) => a + 1.0`, and `(v) => v * v` (the scalar form's operands have
  DIFFERENT types, so one operand twice can only be float) do not.
  A side already solved to a NON-brand settles the node on the spot, since
  only `*` (either side) and `/` (left side) can still take a brand there —
  which is why `Math.abs(d) < step` decides `step` with no annotation.
- **A comparison is the same, minus the result evidence**: `a < b` answers
  `bool` whichever way it resolves, so ONLY an operand can decide it and
  `(a, b) => a < b` asks for an annotation once a brand declares `<`
  (```<` here could be float comparison or `Angle.t` comparison — annotate an
  operand``). `(v: float, lo, hi) => …` is the fix. A brand with NO declared
  `==` keeps structural equality exactly as before.
- **A brand with no implementation** keeps the old error, now naming what it
  has: ```-` needs float operands, got Angle.t — `Angle.t` declares `+`, `*`, but
  not `-```. It names the DECLARABLE base, so `>` on a brand that lacks `<`
  reports ``but not `<` ``. The interpreter says the same thing on the same inputs.
- **Built-in operators (engine prelude only)**: `+`, `-`, scalar `*`, `==`,
  and `<` on BOTH `Angle.t` and `Time.t` (`Angle.add`/`sub`/`scale`/`equals`/
  `less`, `Time.add`/`sub`/`scale`/`equals`/`less` — all public API in the
  generated reference). Neither declares `/`. So `90deg == 90deg`,
  `45deg < 90deg`, `1.5s < 2000ms`, and `2min > 90s` all work — but note that
  `==` on an angle or duration is **float equality** on the underlying
  radians/seconds, so an accumulated value may miss an exact literal by a
  rounding step (and there is no way back out of the brand to compare with a
  tolerance — keep the plain float where that matters).

## Semantics rules that WILL bite you

- **Pipelines append (thread-last)**: `x |> f(a)` is `f(a, x)`. Every
  builtin/prelude function therefore takes its "subject" (list, scene) LAST.
  Because `|>` is syntax, `x |> f(a)` lowers directly to the saturated `f(a, x)`
  (never a partial `f(a)`), so scene/list pipes allocate nothing.
- **`:=` not `<-`** — `<-` is reserved for future do-block binds. Writing
  `acc <- acc + 1.0;` is a parse error naming `:=` (it would otherwise lex as
  the comparison `acc < (-acc + 1.0)`). Assignment must be followed by `;`
  and a continuation expression.
- **`mut` is non-capturable**: a lambda may not read or assign an enclosing
  `mut` binding (lowering error). Params, globals, and plain `let`s are
  immutable. No top-level `let mut`.
- **Top-level defs are mutually visible** (letrec-style) inside function
  bodies (late-bound at call time — this is the hot-reload rebind seam), but
  a *top-level initializer* may only demand globals defined above it.
- **Equality `==` is structural**; comparing functions is rejected at
  CHECK time (`` functions cannot be compared with `==` ``), not just at
  run time. `!=` behaves identically in every respect — it IS `==`
  negated, so it rejects the same operands with the same (reworded)
  errors.
- **Engine values are opaque and refuse `==` at CHECK time.**
  ``` `==` on `Scene.t`: engine values are opaque — compare the numbers you
  derived from them instead ```. That covers every host handle the prelude
  declares `type t = host` (scenes, frames, cameras, effects, subs, lights,
  colors, vec3s, render targets, assets, physics shapes/bodies/worlds, UI
  nodes…) — the runtime error stays as the gradual-seam backstop. Brands
  over data are NOT affected: `Physics.tag` (a string underneath) and
  `Sprite.t` compare structurally, and `Angle.t`/`Time.t` compare because
  they DECLARE `==` (see Units). Two limits: the rule walks tuples, map
  keys/values, and a nominal's type ARGUMENTS but not a record's fields (a
  host value buried in a model still fails at run time), and it is
  DIRECT-only — equality is polymorphic, so `let same = (a, b) => a == b`
  called with two scenes checks clean and fails at run time.
- **`Scene.equals` / `Frame.equals` are the escape hatch** — an explicit
  structural walk over the pure-data `draw` output, which is why the
  refusal above ends `` — `Scene.equals(a, b)` compares structurally `` for
  those two. Deliberately a FUNCTION, not `==`: the walk is O(scene size),
  so it belongs in inline `expect` tests over `draw`, not in per-frame
  logic. Floats compare exactly (no epsilon), assets compare by LOCATOR
  rather than loaded content, animation compares as declared (clip name +
  playhead), and children are ordered. The hint is derived, not
  special-cased: any host-opaque type whose module declares
  `equals : (t, t) => bool` gets it.
- **Comparisons are IEEE**, so NaN (`0.0 / 0.0`) is false against
  everything — including itself — under `<`, `>`, `<=`, `>=`, and `==`;
  `nan != nan` is therefore `true`.
- **Division is IEEE** (`1.0/0.0` = `inf`); the engine boundary rejects
  non-finite numbers.
- **Greedy match arms**: arm bodies are full expressions, so a nested
  `match` inside an arm consumes the following `|` arms as its own —
  parenthesize the inner match (F#/OCaml convention). The checker catches
  this: the swallowed arms report as `` `Y` is not a constructor of `B` ``
  with a hint naming the enclosing match (beside the outer match's now
  non-exhaustive error). The leading `|` is
  required before every arm and every variant alternative, first included.
- **Constructors resolve bare and live in the VALUE namespace**: `Circle(2.0)`
  works anywhere (`Shape.Circle` does NOT — TYPE-qualifying a constructor is
  a load error pointing at the bare form; MODULE-qualifying one is fine,
  `Utils.Circle`),
  which is why ctor names must be unique ACROSS all variant types in the
  module, and `let Circle = …` alongside a ctor `Circle` is a
  duplicate-definition error. An (uppercase) param may still shadow a ctor;
  pattern vars can't (they are forced lowercase).
- **Patterns are minimal**: `Ctor(x, _)` / `Ctor` / `(x, _)` (tuple) /
  bare name / `_` / literals (`true`, `false`, numbers incl. negative,
  strings — equality match). Ctor and tuple sub-patterns are names, `_`, or a
  LITERAL (`| ("Enter", true) =>`, `| Circle(0.0) =>` — number incl. negative,
  string, bool) — but still no deeper nesting (no ctor/tuple/list inside).
  A literal sub-pattern is REFUTABLE, so a tuple/ctor arm with one no longer
  counts toward exhaustiveness (`| ("Enter", true) =>` needs a catch-all; a
  lone `| Circle(0.0) =>` still leaves `Circle` "missing"). This is
  conservative — even a nominally-total set like `(true, _) | (false, _)`
  still wants a catch-all (nested products aren't split into cases). LIST
  element/tail sub-patterns stay names/`_` only (no literals — list
  exhaustiveness is length-based). A tuple pattern matches by EXACT arity
  (mismatch = non-match, like ctors).
  Pattern vars are immutable bindings; lambdas may capture them. First
  matching arm wins; no arm matching is a spanned runtime error.
- **Constructors apply FULLY — functions curry, constructors don't.** Calling
  one with the wrong argument count is an immediate error at the call site,
  under- and over-application alike (`` `Rect` takes 2 argument(s), got 1 —
  constructors apply fully ``): at `check` time, and at run time for the
  gradual seams `check` can't see through. A dropped argument is therefore an
  error where you wrote it, not a `<partial>` drifting through your model
  until a `match` or an `==` fails far away. To stage arguments on purpose,
  say so with a lambda: `let mkTall = (h) => Rect(2.0, h)`.
  A BARE constructor reference is unchanged — still a first-class function
  value for higher-order use (`xs |> List.map(Circle)`,
  `List.map(Option.Some, xs)`, `Utils.Circle` from a sibling module), and it
  rebinds across a hot reload like any stored value. Nullary constructors are
  unaffected (they take no parens at all — `Point`, never `Point()`).
  ⚠️ The two phases see different things, and that is **the one place a clean
  `check` still fails at run time**: `check` reads CALL SYNTAX (the callee's
  declared arity), while the interpreter refuses the constructor VALUE. So an
  aliased ctor — `let make = Rect` then `make(1.0)`, or one handed to a
  higher-order function — types as an ordinary curried function, checks clean,
  and errors when it runs. A constructor value never curries anywhere; the type
  system just can't say so.
- **Duplicates are errors**: top-level names (per namespace — `type Foo` and
  `let Foo` may coexist, but constructors share the value namespace with
  `let`s), record fields (literal and update), lambda params, pattern
  variables within one pattern.
- Recursion depth is capped (128 eval levels); deep iteration belongs in the
  iterative `List.*` builtins (`List.fold`/`map`/`filter`/`any`/`all`/`length`/…),
  which loop in the interpreter and consume no evaluation depth. A hand-rolled
  recursive walk trips the cap somewhere between **n≈40 and n≈60** — each
  user-level call burns several eval levels, and the exact limit depends on
  shape. Bisected: a bare tail call (`countdown(n - 1.0)`) reaches 62, while
  wrapping the call in an operator (`1.0 + countdown(n - 1.0)`, or a
  `match`-based `h + sumList(t)` list walk) costs an extra level per call and
  fails at 42. Budget for the conservative number. The depth error names the
  cap value (128) and points at `List.fold`.

## Standard library (available everywhere, no host needed)

**Signatures live in the generated reference, not here.** Every module below is
documented at its source with `//!` / `///` doc comments, and that is the source
of truth for names, argument order, and per-function behavior:

- `functor docs` prints the whole reference as Markdown to stdout
  (`--format json`, `--output <path>`, `--check <path>`); `npm run generate:docs`
  writes the local artifacts.
- The sources are in-repo and readable directly: interfaces in
  `functor-lang/stdlib/*.funi` (`list`, `map`, `text`, `math`, `random`,
  `debug`) and bundled implementations in `functor-lang/stdlib/*.fun`
  (`option`, `result`, `key`, `mouse`).
- In VSCode, hover shows the same doc block and go-to-definition opens the
  interface at the signature.

| Module | What it is |
| --- | --- |
| `List` | immutable list operations — the ONLY iteration (there are no loops) |
| `Map` | immutable keyed collections; keys are `bool`, FINITE `float`, or `string` |
| `Text` | string building, formatting, inspection (there is no char type) |
| `Math` | numeric functions and `Math.pi`; `Math.mod` / `Math.pow` stand in for `%` / `^` |
| `Random` | pure seeded PRNG threaded through the model — no hidden global generator |
| `Debug` | `Debug.log` — the one impure observability hatch, returns its value unchanged |
| `Option` | `Option.Some` / `Option.None`, plus helpers — what every partial accessor answers with |
| `Result` | `Result.Ok` / `Result.Error`, plus helpers |
| `Key` / `Mouse` | the variant sets the `input` / `mouseButton` hooks receive (`Key.W`, `Key.Num0`, `Mouse.Left`) |

One more module is bundled but is NOT part of that generated reference and has
no `.funi`: **`Animator`** (engine hosts only) — documented in full below.

What a signature can't tell you:

- **Everything is subject-LAST**, so it threads through `|>`:
  `xs |> List.map(fn)` == `List.map(fn, xs)`. Where a function takes another
  argument that looks like a subject, the *pipe* position is the one in the
  docs — `List.append(other, list)`, `List.zip(other, list)` (the PIPED list
  fills the first tuple slot), `Text.contains(needle, s)`,
  `Math.clamp(low, high, n)`, `Math.lerp(target, t, from)`.
- **The builtin registry is CLOSED.** A builtin namespace (`List`, `Map`,
  `Text`, `Math`, `Random`, `Debug`) owns exactly the members in the reference,
  in every embedding, so anything else is a **check-time error** —
  `functor-lang check` (and `functor build`) reject `List.tail` /
  `Text.padLeft` with `` `List` has no builtin `tail` `` plus the nearest name
  or the namespace's full member list. This is a hard error with no escape
  hatch, and it gates hot-reload: a builtin typo in a DEAD branch fails
  `build`/reload. Do NOT assume an F#/Elm stdlib function exists because it is
  idiomatic there — `List` has no
  `tail`/`partition`/`unzip`/`chunk`/`mapMaybe`/`sortWith`/`foldRight`, and
  `Text` no `padLeft`/`startsWith`/`slice`. Build those from `List.fold` /
  `List.filter` / `List.take` + `List.drop` / `Math.min` + `Math.max`.
  (`Scene.*` and the rest of the engine prelude are host-provided, so under
  plain `functor-lang check` — no host — they stay the gradual `Unknown` seam
  and only resolve under the runner, where the prelude's `.funi` interfaces
  make an unknown member a load error.)
- **Always qualify `Option` / `Result` constructors.** Bare `Some` / `None` /
  `Ok` / `Error` do NOT resolve (the loader says ``unknown name `Some` ``) —
  they are `Option`'s and `Result`'s ctors, not entry-module names. Write
  `Option.Some(x)`, `Option.None`, `Result.Ok(x)`, `Result.Error(e)` in both
  expressions and patterns (or `open Option` first), and annotate with
  `Option.t<'value>` / `Result.t<'value, 'error>`:

```functor
let label =
  Option.Some(41.0)
  |> Option.map((n) => n + 1.0)
  |> Option.defaultWith(() => 0.0)

let message = (result: Result.t<float, string>) =>
  match result with
  | Result.Ok(value) => Text.fromFloat(value)
  | Result.Error(error) => error
```

- **Absence is `Option.t`, uniformly.** Every partial accessor —
  `List.nth` / `head` / `last` / `find` / `maximum` / `minimum` and `Map.get` —
  answers `Option.t`, never a sentinel and never an error on an empty list. So
  `xs |> List.maximum |> Option.defaultValue(0.0)` is the one-liner when you
  want a fallback.
- **Recursion is capped (128 eval levels)** — see "Semantics rules" above. Deep
  iteration belongs in the iterative `List.*` builtins, which loop in the
  interpreter and consume no evaluation depth.

### `Animator` (engine hosts only; not in the generated reference)

`Animator` is an ordinary bundled `.fun` module built on `Anim`
(`functor-prelude/stdlib/animator.fun`), available to engine-hosted projects. Its state is plain record data suitable for model
storage, hot reload, and time travel:

```functor
let init = { anim: Animator.start("idle", 0.0) }

let run = (model, tts) =>
  { model with anim: Animator.play("run", tts, model.anim) }

let draw = (model, tts) =>
  Scene.model(Assets.character)
    |> Scene.animate(Animator.pose(model.anim, 0.5, tts))
```

`Animator.start(clip, tts)` starts a clip with no initial fade ·
`Animator.play(clip, tts, state)` records a transition (replaying the current
clip is a no-op) · `Animator.pose(state, fadeSeconds, tts)` derives an `Anim.t`
using a smoothstep crossfade and clip-local playheads. A play during an
in-flight transition uses the current clip as the new outgoing clip and
restarts the fade; it does not snapshot the mid-blend pose.

`Animator` is a reserved bundled namespace. A project that copied the earlier
`examples/crossfade/animator.fun` should delete that sibling to use the bundled
module, or rename it if it carries a customized implementation.

## Functor prelude (only under the engine host — `FunctorHost`)

Available in runner-hosted Functor Lang (and tests via
`functor_runtime_common::functor_lang_prelude`), NOT in plain
`functor-lang run`.

**Signatures live in the generated reference, not here.** Read them with
`functor docs`, or open the interface sources directly —
`functor-prelude/prelude/*.funi`, one file per generated module, each with `//!`
module prose and `///` per-function docs. (VSCode's go-to-definition on
`Scene.cube` opens a read-only materialized copy of that interface at the
signature — edits belong in the repo file, not the copy.)

The generated modules:

| Module | What it is |
| --- | --- |
| `Scene` | 3D scene nodes: primitives, models, terrain, materials, transforms |
| `Sprite` | pure 2D picture values: shapes, text, images, transforms |
| `Camera3D` | 3D cameras (`lookAt`, `firstPerson`), clip planes, screen→world rays |
| `Camera2D` | center-origin 2D camera: pan, zoom, screen→world |
| `Frame` | what `draw` returns: 3D / lit / 2D frames, fog, skybox, clear color, render targets, 2D layers |
| `Light` | ambient / directional / point / spot lights and shadow casting |
| `Fog` | linear and exponential distance fog |
| `Skybox` | six-face cubemaps |
| `Color` | `Color.rgb` — the ONE color type across 3D, lighting, fog, and UI |
| `Vec3` | branded 3D vectors and their arithmetic |
| `Angle` | branded angles (`degrees` / `radians`; suffixes `deg` / `rad`) |
| `Time` | branded durations (`seconds` / `millis` / `micros` / `minutes` / `hours`; suffixes `s` / `ms` / `us` / `min` / `hr`) |
| `Texture` | `Texture.file` — a texture value from a path (a plain string, not an asset locator) |
| `RenderTarget` | named offscreen targets for render-to-texture |
| `Asset` | branded model / texture / sound locators, and `whilePending` placeholders |
| `Anim` | animation poses: clips, blends, masks, additive layers, joint rotation, look-at |
| `Terrain` | finite asset-backed heightfield terrain shared by rendering and physics |
| `Physics` | shapes, bodies, the `physics` hook's world, live reads, commands, raycasts, events |
| `Effect` | one-shot commands returned beside a model (time, random, HTTP, net sends, sounds, preloads) |
| `Sub` | what `subscriptions` returns: timers, connections, asset progress, physics events |
| `AudioScene` / `AudioSource` | what `soundScape` returns, and its keyed continuous voices |
| `Ui` | the lightweight HUD `ui` hook: text, rows/columns, anchored panels, button/slider/textInput |
| `Html` / `Attr` / `Style` | the `webview` hook: an Elm-style HTML tree, attributes/handlers, typed inline CSS |
| `Input` | the types `sampledInput` receives: snapshot, mouse, XR poses, `point2`/`point3` |

Two more namespaces resolve under the host but have no `.funi` and are NOT in
the generated reference:

| Namespace | What it is |
| --- | --- |
| `Net` | built-in message types for `Effect.httpGet`/`httpPost` and `Sub.connect`/`listen` — spelled out below |
| `Assets` | the per-project module `functor import` generates: asset, clip, and joint constants |

Terrain rendering is a camera-relative quadtree over a shared GPU grid (16-bit
height sampling, skirts, bounded instanced grass) — one `Scene.terrain` node,
not thousands of scene nodes.

### The cross-cutting rules (what a signature won't tell you)

**Branded values, never bare numbers or strings** — "the Angle rule". Every one
of these is a VALUE you construct, and a bare number/string in its place is a
teaching error at check or construction time:

```functor
Angle.degrees(60.0)  Time.seconds(0.5)  Color.rgb(1.0, 0.2, 0.2)
Vec3.make(0.0, 1.0, 0.0)  Fog.linear(…)  Skybox.files(…)
RenderTarget.named("id")  Physics.tag("player")  Asset.model(…)  Anim.clip(…)
```

Declare the identity-shaped ones (`RenderTarget.named`, `Physics.tag`) ONCE as a
top-level `let` and use that value at every site. `Scene.animate` takes an
`Anim` value, never a bare clip-name string; `Sub.every` takes a `Time`, never
`0.5`; `Style` values, not CSS strings, go into `Attr.styles`.

**Angles and durations also have literal SUFFIXES** (see "Unit-suffix literals"
above): `90deg` / `0.5rad` are `Angle.degrees(90.0)` / `Angle.radians(0.5)`,
and `0.5s` / `500ms` / `250us` / `2min` / `1hr` are the matching `Time.*`
calls. They are the same value by a shorter name — the brand is not weakened,
and a bare `90.0` is still an error (one that now names the suffix in its fix).
Declare `unit` suffixes for your own brands the same way.

**Everything pipes subject-last**, like the stdlib: `scene |> Scene.color(c)`,
`body |> Physics.at(v)`, `sprite |> Sprite.move(x, y)`,
`camera |> Camera3D.clip(near, far)`, `source |> AudioSource.gain(g)`,
`view |> Ui.panel(Ui.topLeft())`, `terrain |> Terrain.grass(…)`. `Light.castShadows`
is the exception in shape only (the light is its sole argument).

**Transforms wrap in Group nodes: the outer call applies last in world space** —
`s |> Scene.rotateY(r) |> Scene.translate(v)` rotates in place, then moves (the
order the source reads). `Physics.rotateX/Y/Z` follows the same rule.

**Zero-argument constructors take their parens** — `Scene.cube()`,
`Sprite.blank()`, `Anim.rest()`, `Effect.none()`, `Map.empty()`,
`Ui.topLeft()`. The arity is enforced.

**Opaque vs. plain data.** Most engine values (`<Scene>`, `<Camera3D>`,
`<Camera2D>`, `<Frame>`, `<Anim>`, `<Effect>`) are opaque: pass them around, but
they cannot be inspected, compared, or serialized — except that `Scene.t` and
`Frame.t` compare through the explicit `Scene.equals` / `Frame.equals` (an
O(size) structural walk meant for `expect` tests over `draw`, not per-frame
logic). `Sprite.t` is the deliberate
exception — its abstract type hides a private plain-data picture tree, so sprite
values DO support structural display/equality, snapshots, and hot reload
(lowering to 3D happens at `Frame.create2D` / `Frame.with2D`).

**`Vec3` in, records out.** Constructors take `Vec3` values, but the live reads
hand back plain records: `Physics.position`/`linearVelocity` give `{x, y, z}`,
`Physics.cast` gives the `rayHit` record, and `Input` carries `point2`/`point3`.
`Input.point2` is the ONE shared 2D point type — `Sprite.polygon`/`line` take it
directly, and a second `{x, y}` record type would make every bare point literal
an ambiguous-record check error.

**Assets are branded values, not paths** (the B.6 flag day) — `asset.funi` has
the consumer list and the exceptions; what it can't tell you is where the
values come from: `functor import` generates an `Assets` module of constants
from the project's own files, and that (not `Asset.model(…)`, which is for data
boundaries) is what a game should reference.
`functor import` also generates typed clip and joint constants
(`Assets.xbotClips.walk.name`, `Assets.xbotJoints.mixamorig_Head`), which turn a
misspelled clip or joint from a silent bind pose into a check error; glTF names
sanitize deterministically (`mixamorig:Head` → `mixamorig_Head`, with `_2`,
`_3`, … for collisions).

**The `Net` module is built in but not in the generated reference.** Its types
need no declaration, and these are the variants to match:

```functor
| Net.Response(status, body)   // any completed httpGet/httpPost — check status yourself
| Net.Failure(error)           // transport error
| Net.Connected(id) | Net.Message(id, text) | Net.Disconnected(id) | Net.Error(id, message)
| Net.Data(id, value)          // an Effect.sendMsg payload, decoded — its type is `unknown`
```

`Effect.sendMsg` crosses lists, maps, tuples, records, and variants
structurally (usually a shared-module ADT), with no string codec; functions and
opaque host values in the payload are teaching errors.

**Interactive widgets are numbered by SLOT in construction order**, which is how
they are driven headlessly through the debug server —
`Ui.button`/`slider`/`textInput` via
`POST /input {"type":"ui_event","slot":0,"kind":"Clicked"}` (also
`{"kind":{"SliderChanged":0.5}}`, `{"kind":{"TextChanged":"hi"}}`), and
`Attr.onClick`/`onInput` via `{"type":"webview_event","slot":0,…}` numbered
across the whole webview tree. All of them require an `update` hook, and — like
`Sub.every`'s message — the msg↔`update` type link is a runtime check.

**Worked references:** `examples/hello` (models, camera, lights) ·
`examples/counter` (Ui + update) · `examples/ui` (every widget) ·
`examples/webview` (Html/Attr/Style) · `examples/loading` (`Sub.assets`,
`Effect.preloadThen`) · `examples/physics` and `examples/physics-controller`
(the `physics` hook) · `examples/orbs` (multi-role entries).
Prose docs: `docs/ui-interaction.md` (widget interaction and headless driving),
`docs/physics.md`, `docs/time-travel.md`.

### Physics: reads, commands, and queries

`Physics.position` / `Physics.linearVelocity` / `Physics.cast` /
`Physics.castExcluding` / `Physics.transformed` read the live stepped world
(Functor Lang runs in the shell's process — no boundary). They work in **any**
entry point, `tick` and the `physics` hook included, under one rule: a read
answers the LAST STEPPED world. Every pre-step caller (`tick`, `input`, the
hook) sees the previous step; only `draw` (and post-step `update`s) sees this
frame's. That one step of read latency is inherent
to read → decide → step. The world is primed from `init` before frame 1, so
even the first frame's reads answer. A tag not in the world is
a **spanned runtime error** (there is no Option-shaped return to match on),
so only read tags your `physics` hook declares. The tag is cross-frame
identity: same tag = same body; drop a body by not declaring it.
Re-declaring an *unchanged* body leaves the simulation alone; *changing*
its declared position or rotation drives that field (the divergence rule,
docs/physics.md): dynamic/fixed bodies teleport the changed fields immediately,
while a kinematic body receives the changed pose as its next-step target so it
carries velocity into contacts.

Physics **command effects** are returned beside the model like any effect
— `(model, Physics.applyImpulse(ballTag, Vec3.make(0.0, 5.0, 0.0)))` — but carry no
tagger: nothing folds back through `update`; observe outcomes via the
physics reads. Commands queue at perform time and apply at **the next
physics step after they queue**, on its first substep and **after
reconcile** — so declaring a body and commanding it in the same frame
works. The rule is simply **the next physics step after the command
queues**. Because the step comes after `tick`, a command from a pre-step
source (`tick` / `input` / a subscription-driven `update`) normally reaches
the immediately following step and is already visible to this frame's
`draw` — verified with a `Physics.teleport` returned from `tick`, whose
teleported position is read back by the very next `draw`. Commands from
POST-step sources — a `Physics.raycast` tagger, a `Physics.events` handler
— queue for the next frame's step instead. And on a frame where the
fixed-step accumulator takes ZERO substeps (normal above 60fps) nothing
steps at all, so EVERY command defers to the next simulated frame — the
same exception the raycast paragraph below describes. A command naming an unknown tag
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

`Physics.cast` / `Physics.castExcluding` are the **synchronous** counterparts:
same `rayHit` record, but answered in place, so `tick` can branch on the result
while deciding. Use them for a character controller; use the `Physics.raycast`
effect when you want the answer against THIS frame's post-step world (a hitscan
weapon fired on the frame it lands). They are world reads, not environment reads
— not routed through the effect runner and not logged, so unlike the effect they
are not canned under `FakeEffects`; they read whatever world is live (an empty
one misses). Determinism holds because the world is state the physics Timeline
reconstructs from recorded declarations and commands. A miss is `hit: false`
with zeroed fields, never an error.

**The character-controller loop** is therefore one frame, not two:

```functor
let probe = (p) =>
  Physics.castExcluding(playerTag, Vec3.make(p.x, p.y - 0.9, p.z),
                        Vec3.make(0.0, -1.0, 0.0), 0.2)
let tick = (m, dt, tts) =>
  let pos = Physics.position(playerTag) in    // frame 1 included: the world is
  let vel = Physics.linearVelocity(playerTag) in  //   primed from `init`
  let onGround = probe(pos).hit in
  // Steer the horizontal plane only — the solver owns `y`.
  (m, if onGround && m.jump then
        Effect.batch([Physics.setVelocityXZ(playerTag, vel.x, vel.z),
                      Physics.setVelocityY(playerTag, 6.0)])
      else Physics.setVelocityXZ(playerTag, vel.x, vel.z))
```

The read happens in `tick`, the command drains immediately after `tick` (no
`update` hook needed), and the next step applies it — normally this frame's
(when the 60 Hz accumulator is short of a full step, no step runs at all that
frame and it lands on the next one). Reads and writes both live in `tick`.

Note the rule this example obeys: a local `let` needs `in`. It needs no
first-frame gate — the world is primed from `init` before frame 1, so the
pre-step reads answer from the very first tick. (The `probe` cast is the one
exception: ray queries need a step, so frame 1 reads "not grounded".)

⚠️ The sketch above is the LOOP SHAPE, not a usable controller — it has no
coyote time, jump buffering, or post-jump lockout. See the next section.

### Character controllers on the `physics` hook

`examples/physics-controller` is the worked reference (a dynamic capsule, a
moving kinematic deck, coyote time, jump buffering, landing squash, walls),
with its whole feel layer as pure functions under `expect`. Read it before
writing a controller. The recipe, and the parts that are NOT obvious:

**Declare the body `|> Physics.upright`.** This is not optional. A dynamic
capsule that can rotate picks up angular velocity from any glancing contact and
topples — measured at 40-80° off vertical mid-jump, then creeping sideways along
a wall with no input. It also silently corrupts everything below, because a
tipped capsule's lowest point is `radius + halfHeight·cos θ` below its center,
not the fixed `feetOffset` the probe and clamp assume.

**Grounding** is `Physics.castExcluding` from the capsule's center, straight
down, reaching `feetOffset + skin`. Excluding your own tag is mandatory — a ray
starting inside your collider otherwise hits it at distance 0 and reports the
character standing on itself. Keep `skin` small (~0.15): it is also the knob
that decides how far AHEAD of physical contact your landing response fires.

**Moving-platform carry needs no platform identity.** The probe reports which
body it hit, so ask that body for its velocity — one line, no per-platform
state, no "which deck was I on last frame". Fixed bodies read back zero, so
static ground needs no special case; a kinematic body whose declared pose is
re-derived each frame reads back the velocity rapier derived from that motion:

```functor
let surfaceVelocity = (probe) =>
  if probe.hit then Physics.linearVelocity(probe.tag) else zeroVelocity
```

Then steer RELATIVE to that surface (`target = carry.x + wish * speed`), so
standing still rides along and a jump inherits the deck's motion.

**Steer with `Physics.setVelocityXZ`, and never write `vy` while grounded.**
This is the single most important rule, and it is what the per-axis command
exists for. `Physics.setVelocity` replaces all three components, so a
controller using it has to invent a vertical velocity every frame — and the
only values available are a stale read or a guess. `setVelocityXZ` writes the
horizontal plane and leaves `y` exactly as the solver left it, so the ground
contact, the landing impulse, and gravity all stay where they belong:

```functor
(next, Physics.setVelocityXZ(playerTag, want.x, want.z))
```

The one frame that *should* write the vertical axis is the jump, and
`Physics.setVelocityY` writes only that — so it keeps the run's horizontal
momentum, and (unlike `applyImpulse`) does not scale with the body's mass:

```functor
Effect.batch([
  Physics.setVelocityXZ(playerTag, want.x, want.z),
  Physics.setVelocityY(playerTag, jumpSpeed + carry.y),
])
```

The two masks are disjoint, so that pair applies as one whole-vector write.
More generally, velocity commands in the same frame compose as **last-write-
wins per axis**, against the live body at apply time.

Doing this deleted the hand-written **ground clamp** the example used to
carry — a `error / dt` correction toward the standing height, plus the `dt = 0`
NaN guard it needed. Measured across a 400-frame scripted run, dropping the
clamp for `setVelocityXZ` left the resting height (0.8987 vs 0.8988), the wall
stop (`x = −6.3012`), the jump rise (`+0.9363`), and the platform ride (91
frames on the deck) all unchanged; the only difference is a transient 0.04 dip
during a hard landing that the clamp used to correct away instantly.

⚠️ A clamp is still the answer for one thing the solver will not do:
**sticking to a surface that drops away**. Without it a character on a
descending platform goes briefly ballistic. Add it back only if you have
descending platforms, and only for that.

**Still arm a post-jump lockout** (~0.1 s) when the jump fires, and treat
grounding as false for its duration. For the first frames after takeoff the
feet are still within the probe's reach, so the character reads as grounded
while it is physically leaving the ground: coyote time refills, a second tap
gets a free second jump, and steering uses the ground acceleration rate in
mid-air.

**Keep the decisions pure.** Read the world in `tick`, pass a plain
`observation` record (`{ grounded, vx, vy, vz }` — no probe distance and no
rest height, because the controller never writes the vertical axis) to
pure functions, and command the result. The controller's feel then unit-tests
under `functor test` with no GPU and no world — and the physics drive is
recorded, so a scripted `--input-script` run is bit-deterministic too
(verified: 400 frames of identical trace and a byte-identical capture across
two runs).

**Everything else comes free from the solver**: walls stop you, edges drop you,
and props can be knocked around — which is the whole reason to put a character
on the `physics` hook rather than hand-rolling kinematics like
`examples/platformer`.

**There is no `postTick` hook, and a controller does not want one.** Reads in
`tick` see the previous step, which is inherent to read → decide → step. A
post-step hook would read fresher but its commands would wait a full frame (the
write asymmetry), so total loop latency would not improve — and measured against
real landings, the grounding probe's `skin` already makes the landing response
fire 1–3 frames *before* physical contact, so removing a frame of read latency
pushes it further from contact, not closer. Anything purely visual can already
read the fresh post-step world in `draw`; post-step events already arrive
through `Physics.events`.

`Physics.events` is a **Sub** (return it from `subscriptions`, alone or in
`Sub.batch`; it requires `update`). Every contact begin/end from this
frame's physics step arrives post-step as `{started: bool, a: Physics.tag,
b: Physics.tag, sensor: bool}` — `a`/`b` are the pair's tags in rapier's
(deterministic) order, so check both (compare against your declared tag
VALUES: `e.a == ballTag` — tags are strings underneath, so `==` works); `sensor: true` marks an overlap with
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


## The game contract (entry points, effects, hot reload)

A runner-hosted game (`functor -d <project-dir> run native`, with
`functor.json` selecting `game.fun`) defines:

```functor
let init = { … }                       // the initial model (a value)
let tick = (model, dt, tts) => model'  // per-frame step
let draw = (model, tts) => Frame.create(camera, scene)
let input = (model, key, isDown) => model'  // OPTIONAL; key: Key.t — match
                                            // | Key.W / Key.Up / Key.Space /
                                            // Key.Num0..Num9 (never strings)
let sampledInput = (model, snapshot: Input.snapshot) => model'
                                            // OPTIONAL; called once immediately
                                            // before every simulation tick
let mouseMove = (model, x, y) => model'     // OPTIONAL; logical window/CSS coordinates
let mouseWheel = (model, delta) => model'   // OPTIONAL
let mouseButton = (model, button, isDown) => model'
                                            // OPTIONAL; button: Mouse.t — match
                                            // | Mouse.Left / Mouse.Right /
                                            // Mouse.Middle. Captured projects
                                            // receive it under free-look;
                                            // visible projects receive absolute
                                            // pointer clicks
let update = (model, msg) => model'         // OPTIONAL; msgs are ADT variants
                                            // ANY entry point may instead return
                                            // (model', effect) — a 2-tuple whose
                                            // second element is an Effect value
let subscriptions = (model) => Sub.every(Time.seconds(1.0), Beat)
                                            // OPTIONAL, but requires update
let physics = (model) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [body, …])  // OPTIONAL
let webview = (model) => Html.div([…], […])  // OPTIONAL; the HTML/CSS overlay
                                            // (blitz natively, a DOM overlay on
                                            // wasm). Attr.onClick msgs arrive
                                            // through `update`
let ui = (model) => Ui.column([…]) |> Ui.panel(Ui.topLeft())  // OPTIONAL; the 2D HUD,
                                            // drawn over the frame. Ui.button clicks
                                            // arrive as msgs through `update`
let soundScape = (model) => AudioScene.create([source, …])  // OPTIONAL; continuous
                                            // looping voices, reconciled by key each
                                            // frame (needs no `update`)
```

The three mouse hooks receive captured shell input by default. Set
`"mouseCapture":false` to disable that path, or use `"cursor":"visible"` for
absolute pointer input (which also disables capture). Programs without
captured mouse hooks show no capture control. Native capture starts on a non-UI
click and Escape releases it. Manifest-less site IDE/inline sessions use
`?mouseCapture=false` or `?cursor=visible` on the page URL.

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

Frame order: `sampledInput` → subscriptions→`update` → `tick` → `physics` (reconcile +
fixed-step, 60Hz accumulator) → `draw`.

`Physics.position` / `Physics.transformed` are **live-world reads**, and they
work in `tick`, `update`, `input`, and `draw`. What differs is only how fresh
the pose is, and that follows WHEN the caller runs: `tick` / `input` and the
subscription-driven `update`s run before the step, so they see the *previous*
frame's stepped pose; `draw` runs after it and sees this frame's. `update` is
not uniformly pre-step — when it is handling a `Physics.raycast` tagger or a
`Physics.events` message it runs POST-step, and its reads see the current
frame's world.

Three rules follow from that one:

- **The `physics` hook is a pre-step reader like any other.** Reading inside
  it is allowed and answers with the LAST stepped world — the same pose `tick`
  saw that frame. (It used to be a DEADLOCK: the read raised, so the world was
  never declared, so it raised again, forever. It no longer is.) The hook is
  still a DECLARATION, so keep it cheap; a read there is for deriving one
  body's declared pose from another's.
- **The first frame is primed, so it reads.** Before the first frame the
  runtime evaluates the hook once on `init` and reconciles it with ZERO steps,
  so frame 1's `tick`/hook reads answer with the initial declared poses. A
  `started: bool` guard is no longer needed — `examples/terrain` and
  `examples/physics-controller` deleted theirs. Three footnotes: priming re-runs
  at a RESTART (and at a reload that ADDS the hook) but not at an ordinary hot
  reload, where the world survives with the model; inside the prime evaluation
  itself — the one call with no stepped world behind it — a body read answers
  the identity pose (origin, zero velocity) instead of raising; and priming
  answers BODY reads only — `Physics.cast` needs a step (rapier ingests
  colliders there), so a grounding probe still answers from frame 2.
- **A hook error degrades, loudly.** An error inside the hook (or a
  non-`Physics.scene` return) keeps the previous frame's declaration, keeps
  stepping the world, and reports the teaching error ONCE — the hot-reload
  broken-edit discipline. An unknown tag is still a spanned error everywhere
  else: `` no body tagged "ball" in the physics world ``.

The physics world survives hot reload (like the model); deleting the
`physics` hook drops it. Gotcha: `--fixed-time T` pins the clock with
`dts = 0`, so physics takes exactly **one** bootstrap substep and then never
steps again (and the subscription grid never crosses) — bodies render at
their declared pose, to within that single 1/60 substep. Capture physics
with plain `--capture-time` (and a settled
scene for reproducibility) instead; capture timer-driven changes via the
debug server's `/time` advance. To *see* colliders, run with
`--debug-render physics`: normal shading plus the live world's wireframes
(collider outlines, contacts, body frames).

`sampledInput` carries both levels and deterministic fixed-step edges alongside
the event-oriented `input` / `mouseMove` / `mouseButton` hooks.
`snapshot.mouse.buttons.left` tells you a button is held;
`snapshot.mouse.pressed.left` and `.released.left` report transitions since the
previous fixed step. Keyboard uses `heldKeys`, `pressedKeys`, and
`releasedKeys`. Edges are de-duplicated: native OS repeat still reaches the
legacy `input` hook but does not repeat `pressedKeys`; down/up/down before one
step can put a control in both edge sets with the final level held. Shells
retain transitions across render frames with no simulation step, consume them
on the first catch-up step, and clear them for later substeps. Focus/cursor-loss
releases use the same path. A hook may
return either a bare model or `(model, effect)` like other model-updating entry
points. Samples are plain data recorded in the frame input log, so rewind,
forward ghosting, and counterfactual history replay re-run the same snapshot
before the same tick. Future projection clears already-consumed edges, carries
the latest level/pose state through unrecorded ticks, and synthesizes one-step
edges from scripted key/mouse events. Dense samples share the bounded 900-frame
model/world horizon; sparse
edge events remain session-long. Adding the hook during hot reload keeps
selected-snapshot semantics because the old timeline has no historical
coeffects to replay. A game without the hook pays no snapshot-conversion cost.

A project dir with `functor.json` `{"language": "functor-lang", "entry": "game.fun"}`
(or a named `entries` map — see Modules; `--entry <name>` selects the role)
works with the CLI: `functor -d dir build` (typecheck, diagnostics are
errors), `run native`, `develop` (hot reload is built in), and `run wasm`
(the `.fun` ships as text and is interpreted in the browser; file-watch hot
reload is native-only — reload the page to pick up saved edits, or push
source with a `{ type: "functor-lang-set-source", source }` postMessage to the page
for a model-preserving in-place reload; the VSCode **"Functor: Open Live
Preview"** command does exactly that from the live buffer as you type).
Captured game mouse input defaults on. When the running program implements
captured mouse hooks, native offers click-to-capture and web shows the
mouse-capture button, routing `mouseMove`/`mouseWheel`/`mouseButton` into the
ordinary game hooks. On native, a non-UI click captures; Escape or focus loss
releases. `"mouseCapture":false` is the explicit free-pointer exception.
Absolute pointer-led games instead add `"cursor":"visible"`; native keeps the
system cursor free and web routes its absolute CSS-pixel movement/buttons
without Pointer Lock. Visible pointer mode overrides the implicit capture
default; explicitly combining it with `"mouseCapture":true` is refused.

The timeline separately exposes a universal, runtime-owned **Debug camera**
while playing or paused; this is NOT a manifest setting. Activating it snapshots
the current authored view. A 3D or mixed frame gets FPS mouse look, WASD
movement, Q/E down/up, and wheel-adjusted FOV. A pure `Frame.create2D` frame gets
mouse/WASD panning and wheel zoom. Camera navigation is shell input: it never
enters the model, recorded input, replay, or `GET /scene`, and the renderer keeps
the frame's authored camera for terrain/culling and render-target/portal passes.
Escape releases capture without discarding the snapshot; click the viewport to
recapture. **Exit debug view** reattaches. Pause/resume preserves the debug view
so it can inspect both live and pinned games. This is independent of
`mouseCapture`.

`examples/hello/game.fun` is the reference
(`examples/physics/game.fun` for the physics hook, including the
rising-edge input latch — GLFW key repeats arrive as `isDown = true`).
The model shows live at the
debug server's `GET /state`. **Hot reload is on by default**: saving the
`.fun` file reloads it in ~1 frame with the model preserved (a broken edit
keeps the old program running; an edited `init` takes effect on restart).
PENDING EFFECTS are reset by a reload — an in-flight `Effect.httpGet` tagger
would dangle, so its message never arrives.
The time-travel history also survives the reload when every retained model is
plain data (the usual constant-tweak case), so old frames are immediately
rewindable under the new program. These are old data snapshots interpreted by
new code, not a retroactive replay: draw-only tweaks affect every retained
frame, while `tick` changes affect evolution after resume. A history containing
a callable or opaque host value makes the selected frame a reload boundary. An
unsafe discarded future is dropped while a remaining plain-data prefix stays
seekable; if the retained prefix is also unsafe, a new one-frame generation
starts at the selected frame. Unavailable frames remain visible as stripes
rather than silently implying they are safe to restore.
When the retained history and authoritative live model are entirely reload-safe
plain data, pausing on an earlier frame and reloading keeps the selected cursor
and the entire recorded future. Resume remains the explicit branch point: it
discards that future, while the scrubber holds its prior visual span until the
new branch fills it.
Forward **extrapolation** adds one counterfactual step for input-only model games:
after a safe reload at a scrubbed frame, it replays the plain-data input and
exact frame-clock logs from the NEW program's `init` through the newest retained
frame once. Sparse edge input remains session-long. Continuously sampled input
is retained for the same 900-frame horizon as model/world snapshots; once its
session origin ages out, reload keeps the selected snapshot instead of
pretending a partial replay is complete. The rebuilt selected model becomes the visible anchor and
Resume branch, later scrubs remain O(1) snapshot restores, and its future is
projected from new-code history. This prevents old derived state (such as a
jump's stored launch velocity) from dragging a constant-tweaked trajectory back
to the recorded outcome. Replay is exact, including future key-up frames; Resume
cuts off that recorded future, while an optional paused **coast from here**
control is deferred. Games with `update` or physics keep selected-snapshot
behavior until the fuller event/coeffect replay log exists.
Closures **stored in the model** rebind too: they adopt the edited code
with their captured values carried over (matched by the enclosing def's
name; a closure whose def was renamed/deleted keeps its old body with a
loud `[functor-lang] reload:` warning). First-class variant constructors
stored in the model likewise adopt the edited declaration's arity.

Transforms wrap in Group nodes: the **outer call applies last in world
space** — `s |> Scene.rotateY(r) |> Scene.translate(Vec3.make(x, 0.0, 0.0))` rotates in
place, then moves (the order the source reads). Most engine values (`<Scene>`,
`<Camera3D>`, `<Camera2D>`, `<Frame>`) are opaque: they can be passed around but
not inspected, compared, or serialized — `Scene.equals` / `Frame.equals` are the
explicit exception, an O(size) structural walk for `expect` tests over `draw`.
`Sprite.t` is the deliberate
exception: its abstract type hides a private plain-data picture tree, so sprite
values do support structural display/equality, snapshots, and hot reload.

## Typechecking model (Hindley–Milner + gradual seams)

`functor-lang check` runs REAL INFERENCE (B7): unannotated code gets full types via
unification with let-polymorphism — generic functions instantiate fresh at
every use, element types flow through `List.map`/`filter`/`fold`, key/value
types flow through every `Map` operation, and apostrophe-prefixed annotation
names are type variables (`(xs: List<'a>, seed: 'b): List<'b>`). Map keys are
limited to `bool`, finite `float`, and `string`; concrete violations diagnose
during checking, while polymorphic/`unknown` seams repeat that validation at
runtime. Inference has teeth: unannotated bad calls, mixed-element
lists, and contradictory `mut` use are errors now. `Unknown` remains ONLY
at genuinely-dynamic seams (host values, and the `unknown` annotation you
write on purpose — an UNRECOGNIZED type name is an error, not a seam)
and absorbs anything — but a BUILTIN namespace is not such a seam:
its member set is closed, so `List.tail` / `Text.padLeft` are check errors, not
`Unknown` (see "Standard library"). Function TYPES **do** annotate —
`(f: (float) => float)`, `(f: ('a) => 'b)`, and the parenthesized
return position `(): ((A) => B) =>` all parse and check (see the Syntax
subset above); leaving higher-order params unannotated and letting
inference type them is still fine. Generic declarations (`type Pair<'x, 'y> = { first: 'x, second: 'y }`)
instantiate fresh per use; an UNDECLARED type variable in a declaration is
a teaching error. Record literals resolve nominally, F#-style:
the unique declared type with exactly that field set (no match = anonymous
data, still fine; two same-shaped declarations make a bare literal
ambiguous — annotate). A `mut` slot's type fixes at its initializer. A
`match`'s patterns CONSTRAIN its scrutinee (first ctor arm pins the
variant type; a foreign literal arm is a can-never-match error);
exhaustiveness checks all ctors / `true`+`false` / catch-all; arm results
must agree.

**Primitive type names are lowercase**: `float`, `string`, `bool`; the
built-in generic containers are `List<…>` and `Map<key, value>`. A
miscased or misspelled name is a **check error** with a did-you-mean, so
you find out immediately:

```functor
let f = (x: Float): Float => x + 1.0
// error: unknown type name `Float` — did you mean `float`?
//        (Functor Lang's primitive types are lowercase)
```

`Int`, `Number`, and `Double` are errors too — Functor Lang has a single
number type, `float`. A mistyped nominal (`Postion` for `Position`)
suggests the declared type it is closest to; a name that resembles nothing
in scope tells you to declare it or to write `unknown`.

⚠️ **Historical note (this WAS a silent trap).** Until the diagnostic
landed, an unrecognized annotation resolved to `Unknown` — which absorbs
everything — so `let x: Float = "hi"` typechecked clean and every
annotation you added for safety was quietly inert. If you are reading older
`.fun` code (or older docs) written against that behavior, its `Float`/
`String` annotations were never checking anything, and fixing the casing
may surface real type errors that were always there.

`unknown` is the **explicit** gradual seam — the one annotation that
deliberately absorbs anything, in both directions:

```functor
let handle = (payload: unknown) => …   // a host value only the two ends type
```

Use it where a value is genuinely dynamic (`Net.Data`'s payload is declared
this way). Everywhere else, an unknown name is now an error rather than a
silent opt-out of checking.

## Keeping this skill honest

**The division of labor.** Module SURFACES — every name, signature, argument
order, and per-function behavior, for both the standard library and the engine
prelude — are owned by the doc comments on their sources
(`functor-lang/stdlib/*.funi` and `*.fun`, `functor-prelude/prelude/*.funi`) and
published by the generated reference (`functor docs`, `npm run generate:docs`).
This SKILL owns what a signature cannot carry: syntax, semantics rules, the game
contract, hot-reload behavior, the typechecking model, the verification loop,
cross-module interactions, worked patterns, and the module inventories that say
where to look.

So when a PR changes the language or the prelude, in that same PR:

- New or changed **members** → document them at the source (`///` on the
  binding, `//!` on the module) so the generated reference picks them up.
  `npm run check:docs` fails on an undocumented module, type, or signature.
- A new **module** → add one inventory line here, and the docs at the source.
- New **syntax/semantics/contract/gotcha** behavior (see `docs/functor-lang.md`
  Track B/C checkboxes) → update this skill.
- Do NOT re-list signatures here. If you catch this file restating something the
  `.funi` prose already says, delete it here; if it says something the `.funi`
  does not, move it there.
