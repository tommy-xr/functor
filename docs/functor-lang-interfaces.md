# Functor Lang interface files (`.functori`) — design

**Status:** design / not yet implemented. **Owner:** language.
**Related:** `docs/functor-lang.md` (roadmap), the `functor-lang` skill (syntax source of
truth — update it alongside each slice), `functor-lang/src/project.rs` (module loading),
`functor-lang/src/types.rs` (checker, `builtin_signature`).

## Goal

Give the Functor Lang typechecker **real types for the host prelude** (`Scene.*`,
`Camera.*`, `Physics.*`, `Effect.*`, `Sub.*`, …) and let users **ascribe types**
to their own bindings — so inference flows real types through engine-touching
code instead of collapsing to `Unknown`.

Today the engine API resolves to `ExprKind::External` → `Type::Unknown`, so a
signature like `ringCube` shows `(float, float) => 'i` in hover/codelens instead
of `(float, float) => SceneNode`. The vehicle is **interface files**: a `.functori`
declares *types without implementations*, the way OCaml `.mli` / TypeScript
`.d.ts` do.

Why a real language feature (not a Rust signature table): **LLM-native.** A text
file listing the whole engine API is introspectable and self-documenting — an
agent that can read `scene.functori` knows the signatures. That is worth more than a
hidden Rust `match`, and it makes interfaces a reusable capability (user FFI,
and later module encapsulation).

## Current state (verified) — what parses today

| Position | Simple / generic (`float`, `List<float>`) | `*`-tuple (`float * float`) | Function type (`(A) => B`) |
| --- | --- | --- | --- |
| Function parameter `(x: T)` | ✅ | ✅ | ❌ |
| Return type `(): T =>` | ✅ | ✅ | ❌ |
| Record field `{ x: T }` | ✅ | ✅ | ❌ |
| `let name: T = …` (top-level / let-in) | ❌ (no annotation slot at all) | ❌ | ❌ |
| `type Name` (abstract, no body) | ❌ | — | — |
| `val name : T` (signature) | ❌ (no such form) | — | ❌ |

The type-expression grammar is `type := tatom ("*" tatom)*` (parser.rs) — a name
with optional `<generics>`, joined by `*` into flat products. **There is no
function-type syntax and no grouping**, and `let` bindings have no annotation
slot. Those two gaps are the foundation everything else needs.

## The five slices

Each is independently shippable and updates the `functor-lang` skill in the same
change.

### 2a — Type-expression grammar: function types, tuples `(A, B)`, grouping

Extend the type grammar so a type can be a **function type**, a **tuple**
written `(A, B)`, and a **parenthesized group**. Tuple types move from `*` to
`(A, B)` for symmetry with value tuples / patterns (which are already `(a, b)`):

```
type   := arrow
arrow  := post ( "=>" arrow )?                // right-assoc: A => B => C = A => (B => C)
post   := tapp | "(" typelist? ")"            // paren group: fn param-list (if "=>" follows),
                                              //   tuple (>=2 elems), or grouping (1 elem)
typelist := type ("," type)*
tapp   := NAME ( "<" type ("," type)* ">" )?  // existing name<generics>
```

Disambiguation is a bounded peek (ReasonML's rule): read the whole `( … )`, then
if `=>` follows it was a **function parameter list**; otherwise it's a **tuple**
(≥2 elems) or **grouping** (1 elem). `()` is only valid immediately before `=>`
(a zero-arg function).

- **Function type:** `(A, B) => C`, `() => C`, `(A) => B`.
- **Tuple:** `(A, B)`, `(A, B, C)`. **`*` product syntax is removed** — see the
  migration below.
- **Grouping:** `(T)` is just `T` (needed for `((A) => B) => C`).
- **Return-position rule:** a function type used directly as a **lambda return
  annotation** must be parenthesized — `(): ((A) => B) => body` — because Functor Lang
  reuses `=>` for both the function-type arrow and the lambda body. Param / `let`
  / `val` positions have a clean terminator (`,` `)` `=`), so no parens needed
  there; tuples/products in return position are unambiguous (no inner `=>`).

**Migration (same slice — the corpus is small):** rewrite the ~8 `*`-tuple
annotations in the codebase (`examples/lighting`, `mpclient`, `mpserver`,
`functor-lang/examples/{tuples,lists,strings}.functor`) to `(A, B)`, remove `*` from the type
grammar (it stays a value-level multiply operator), and switch the type
**Display** (`types.rs` `Type::Tuple`, `hover.rs::type_name_text`) to render
`(A, B)` so hover/codelens output matches the input spelling.

This also fixes higher-order annotations that error today:
`let apply = (f: (float) => float, x: float) => f(x)`.

*Verify:* parse + check tests for every form (function, tuple, grouping, nested,
return-parens); the migrated examples still check clean; hover/codelens now
render `(A, B)` and `(A) => B`, and parse-what-we-print round-trips.

### 2b — `let`-binding annotations

Add an optional `: type` between a binder name and `=`:

```
let name : Type = value          // top-level
let name : Type = value in body  // let-in
```

- AST: add `ty: Option<TypeName>` to `LetDecl` and the let-in `Let` node.
- Parser: after the binder name, optionally consume `:` + a type.
- Lower: thread the annotation through.
- Check: unify the value's inferred type against the annotation (exactly what
  parameter annotations already do) — so `let m: Model = { wrongField: … }`
  becomes an error instead of silently `Unknown`.
- Inlay hints: an annotated binding is skipped (like an annotated param).

*Verify:* `let x: float = 3.0` checks; a mismatched annotation errors at the
value's span; inlay suppresses the hint on annotated bindings.

### 2c — Abstract types

A `type` declaration with **no body** is an opaque nominal — a named handle
whose representation is hidden (a host value, or a user's opaque type):

```
type SceneNode           // abstract: no constructors, no fields
```

- AST: `TypeBody` gains an `Abstract` case (today: `Record | Variants`).
- Parser: `type Name` with no `=` → abstract; `type Name<a>` allowed.
- Check: abstract types are nominal and unify only with themselves (by canonical
  name, e.g. `Scene.SceneNode`); they have no constructors, so you can hold and
  pass one but only host `val`s produce/consume it. Plug into the existing
  nominal-type table (`Type::Record(string,…)` / `Variant(string,…)`); likely a
  new `Type::Opaque(string, Vec<Type>)` (name + type args) variant.

*Verify:* `type T` + a function passing a `T` through checks; constructing a `T`
(no ctor exists) errors; cross-module `Mod.T` resolves to the same nominal.

### 2d — `val` signatures + `.functori` grammar + interface-only modules

The interface file itself. A `.functori` contains **only** declarations — no bodies:

```
// scene.functori  →  module `Scene`
type SceneNode
type Color = { r: float, g: float, b: float }   // concrete types allowed too

val cube    : () => SceneNode
val color   : (float, float, float, SceneNode) => SceneNode
val group   : (List<SceneNode>) => SceneNode
val rotateY : (Angle, SceneNode) => SceneNode
```

- **`val name : Type`** — a name bound to a type, no body; the implementation
  lives elsewhere (the Rust host, or — later — a paired `.functor`).
- **`.functori` grammar** is the `.functor` grammar restricted to `type` (concrete or
  abstract) and `val` items — a `let` with a body in a `.functori` is an error.
- **Interface-only modules (the key module enhancement):** the project loader
  accepts a module defined *solely* by a `.functori` (no `.functor`). Its `type`/`val`
  declarations feed the **same exports map** (`exports_of`) and typing
  environment as any other module, so:
  - qualified access (`Scene.cube`, `Scene.SceneNode`) resolves through the
    existing cross-module path,
  - `open Scene` brings members unqualified for free,
  - the checker resolves what were `External`/`Unknown` refs against the loaded
    `val` signatures.
- **Runtime is unchanged:** host-module `val`s still resolve to host values via
  the existing External→host path at run time; `.functori` is a *check-time overlay*.
  A user `.functori` `val` with no host backing and no `.functor` body is a runtime
  "unknown external" if actually called (existing error) — acceptable.

*Verify:* a scratch project with a hand-written `foo.functori` (no `foo.functor`) gives
`Foo.bar` a real type in `check`/hover; a `let … = …` body in a `.functori` is a load
error; `open` and qualified access both resolve.

### 2e — Ship the host prelude `.functori` + drift test

Author the FunctorHost surface as `.functori` files (`scene.functori`, `camera.functori`,
`physics.functori`, …), **bundle them into the `functor_lang` crate** (`include_str!`), and
load them by default whenever the checker runs — so host-awareness works
everywhere (`functor-lang check`, the LSP) with **no config**. This flips the
`PROTECTED_NAMESPACES` hack: `Scene` etc. stop being magic `Unknown` externals
and become ordinary interface-only modules owned by the bundled prelude.

- **Drift test:** a Rust test asserting every `val` in the bundled `.functori` maps
  to a real entry in the host registry (`functor_lang_prelude.rs`) and vice versa, so the
  declared types and the Rust implementations cannot silently diverge.
- Result: `ringCube : (float, float) => SceneNode` in hover/codelens; inlay/
  codelens across a real game stop showing `'a`/`Unknown` for engine calls.

## Module semantics summary

The one real module enhancement is **interface-only modules** (a module backed
by a `.functori` with no `.functor`). Everything else reuses existing machinery:
`file = module` naming, qualified-by-default access, `open`, cross-module type
resolution (`Mod.T`), and cycle refusal. The protected-namespace special case
is *replaced* by real interface-only modules for the host.

## Deferred / explicitly out of scope

- **Interface *checking* of a paired `.functor`** (OCaml `.mli` model: the impl must
  satisfy the interface; non-`val` names are hidden). This is module
  *encapsulation* — valuable, but not needed for host types (host modules have no
  `.functor`). Design `.functori` so it slots in later; don't build it now.
- **Scope ergonomics for host types** — whether common prelude types
  (`SceneNode`, `Camera`) are always written qualified (`Scene.SceneNode`) or a
  curated set is bare-in-scope like builtins. Decide before 2e; not a blocker for
  the grammar work.
- **Unifying builtins into `.functori`** — the 17 builtins (`List.map`, …) get types
  from Rust `builtin_signature` today. Once `.functori` exists they *could* move to a
  bundled `builtins.functori` for one declarative mechanism. A nice follow-up, not
  required.

## Open questions

1. **Tuple spelling.** ~~`A * B` vs `(A, B)`~~ **RESOLVED:** adopt `(A, B)` (symmetry
   with value tuples/patterns), **remove `*`** from the type grammar and migrate
   the small corpus, disambiguate function-vs-tuple by the trailing-`=>` peek
   (ReasonML's rule), and require parens on function types in lambda-return
   position. See slice 2a.
2. **Signature form: `val` vs bodyless `let`.** **LEANING: bodyless `let`** — since
   2b adds `let name : Type = value`, an interface signature is just that with the
   `= value` dropped (`let cube : () => SceneNode`), so a `.functori` is "the `.functor`
   with bodies erased" — no new keyword. Bodies are required in `.functor`, forbidden
   in `.functori`. (Alternative: explicit `val`, clearer decl-vs-def, matches
   OCaml/SML.) Decide at 2d.
3. **Host-type scope.** **RESOLVED via existing `open`:** default qualified
   (`Scene.cube`), `open Scene` brings members bare (like OCaml). Optional later
   polish: implicitly `open` one core prelude module. Not a blocker.
4. **Abstract type representation** — new `Type::Opaque(name, args)` vs reusing
   `Type::Record(name, args)` with an empty/hidden field list.
5. **`.functori` beside `.functor`** — for host modules there's only `.functori`. If a user
   ships both `foo.functor` and `foo.functori`, that's the deferred encapsulation feature;
   until then, define the loader behavior (ignore the `.functori`, or error).
