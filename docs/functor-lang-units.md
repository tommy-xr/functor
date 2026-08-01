# Unit-suffix literals

Functor Lang lets a numeric literal carry a **unit suffix** — `90deg`, `0.5s`,
`16px` — where a `unit` declaration says what the suffix means, and lets that
brand carry arithmetic: `90deg + 45deg`, `1.5s - 200ms`, `45deg * 2.0`. This
document covers both shipped phases — the literals (Phase 1) and the operators
(Phase 2).

Syntax and semantics for day-to-day use live in the `functor-lang` skill; the
prelude's own suffixes are documented at their source
(`functor-prelude/prelude/angle.funi`, `time.funi`) and appear in the generated
API reference.

## Why

The engine's branded values (`Angle.t`, `Time.t`, and the user's own single
constructor brands) exist so that degrees cannot be passed where radians
belong, or milliseconds where seconds do. That safety costs a call at every
site — `Scene.rotateY(Angle.degrees(90.0))`, `Sub.every(Time.seconds(0.5), …)`
— and the noise is what pushes people toward bare numbers, which is exactly the
mistake the brand exists to prevent. A suffix keeps the brand and removes the
ceremony: `Scene.rotateY(90deg)`.

## Phase 1 (shipped)

### The literal

A numeric literal (`digits` or `digits.digits`) immediately followed by
identifier characters lexes as ONE token carrying the value and the suffix
spelling. Adjacency is the whole rule — `90 deg` is still a number and a name.
The lexer deliberately does **not** know which units exist; there is no
scientific notation in Functor Lang, so no `1e5` ambiguity, and `90deg` was
previously a parse error, so the syntax was unclaimed.

A prefix minus folds into the literal (`-90deg` is `Angle.degrees(-90.0)`,
never a negation of the branded value), matching how number patterns already
absorb a leading `-`. Binary subtraction is untouched.

### The declaration

```functor
unit deg = Angle.degrees          // a `.funi` signature
unit px = Px                      // a single-constructor brand: type Px = | Px(value: float)
unit turns = fullTurns            // any (float) => 't function
```

`unit <suffix> = <name>` is a top-level item in both `.fun` and `.funi` files.
The target is a NAME (never an arbitrary expression) resolved in the declaring
module's scope, and it is typechecked as exactly `(float) => 't`.

- **Units are project-wide.** `file = module` makes them behave like
  constructors: a suffix declared in any module means the same thing in every
  module, and declaring one twice anywhere in a project is an error.
- **Resolution happens once**, in the declaring module, before any file lowers
  — so a use site may precede its declaration, or live in a different file.
- **`unit` is contextual**, like `open` / `expect` / `module`: it only means a
  declaration in item position, so `unit` stays usable as an ordinary name.

### Desugaring

Lowering rewrites each suffixed literal to exactly the call the unit names:
`90deg` becomes `Angle.degrees(90.0)`, with no unit node in the IR. Hover,
inlay hints, go-to-definition, typechecking, the interpreter, and the host's
teaching errors therefore all see an ordinary call — there is no second code
path to keep honest, and no per-frame cost (the desugar happens at load).

An undeclared suffix is a load/check-time error listing the units that ARE
declared. A branded-value type error now teaches both spellings:

```
expected Angle.t, got float — `Angle.t` is a branded value: write `90deg` or `Angle.degrees(90.0)`
```

### The built-in units

Declared in the prelude interfaces they belong to, not in a lexer table:

| Suffix | Expands to | Home |
| --- | --- | --- |
| `deg` | `Angle.degrees` | `angle.funi` |
| `rad` | `Angle.radians` | `angle.funi` |
| `s` | `Time.seconds` | `time.funi` |
| `ms` | `Time.millis` | `time.funi` |
| `us` | `Time.micros` | `time.funi` |
| `min` | `Time.minutes` | `time.funi` |
| `hr` | `Time.hours` | `time.funi` |

They resolve only where the engine prelude does (a runner-hosted project, or
the headless test seam), like every other prelude name. A project declares its
own units for its own brands.

## Phase 2 (shipped): operators on units

Phase 1 gives a brand a cheap way IN from a number. What it did not give is
arithmetic: `90deg + 45deg` did not typecheck, because `+` was `(float, float)
=> float`, so the answer was to unwrap, add, and rebrand — exactly the ceremony
the suffix removed. It typechecks now.

### The declaration

An operator is its own top-level item, naming the suffix whose brand it acts
on:

```functor
type Px = | Px(value: float)

unit px = Px
unit px (+) = addPx                  // (Px, Px) => Px
unit px (-) = subPx                  // (Px, Px) => Px
unit px (*) = (a, k) => scalePx(a, k)   // (Px, float) => Px — the scalar form
unit px (/) = dividePx               // (Px, float) => Px
```

- `+` and `-` take `('t, 't) => 't`: adding two lengths gives a length.
- `*` and `/` take `('t, float) => 't`: scaling a length by a number gives a
  length. A brand-by-brand product would be a *different* type (an area), which
  is dimensional analysis — see the non-goal below.
- Each implementation is typechecked against the shape above **at the
  declaration**, exactly as Phase 1 typechecks the constructor. It is an
  ordinary expression: a name, or a lambda (a `.funi` has no bodies, so a
  prelude declaration always names a host external).
- **The operator belongs to the BRAND, not the suffix.** The suffix is only how
  the brand is named: `s`, `ms`, `us`, `min`, and `hr` are all `Time.t`, so ONE
  `unit s (+) = Time.add` makes `1.5s - 200ms` work. Declaring the same brand +
  operator twice — through any suffix — is a duplicate error.
- Comparisons (`<`, `>`, `==`) are deliberately not declarable: `==` is already
  structural for plain-data brands, and ordering wants its own design pass.

> **Deviation from the original design.** The design sketch attached operators
> to the unit declaration as a block (`unit px = Px { (+) = … }`). Shipping them
> as separate items keeps the `.funi` prelude honest — `Angle` and `Time`
> declare their suffixes and their operators in the same flat item list the rest
> of an interface uses, docgen renders each one with its own `///` prose, and
> a project can add an operator to a brand without touching the (possibly
> already-published) unit declaration.

### Resolution: ad-hoc overloading, after inference

Operator selection happens **after** ordinary Hindley–Milner inference has
solved the operand types — never during unification, which would make inference
order-dependent:

1. Infer as today, with `+` constrained as usual.
2. At each arithmetic node, zonk the operands. If either resolved type is a
   brand that declares that operator, the node IS that implementation: the
   other operand is checked against the implementation's signature (the same
   brand for `+`/`-`, a plain float for the scalar side of `*` and `/`) and the
   node's type is the brand.
3. If neither operand is a declaring brand, the node is float arithmetic and
   its ordinary `float` constraint applies — unchanged.
4. A node whose operands are *still unsolved* when it is first seen is
   **deferred** to a pass that runs when the enclosing definition group has
   settled, then re-resolved by the same rules. That is what lets a brand that
   only becomes known later still reach the node.

Scaling commutes, so a brand may sit on either side of `*` (`2.0 * 45deg` is
the declared call with its arguments swapped). Division does not: `2.0 / 45deg`
is refused, because the scalar form divides a branded value BY a number.

The critical rule is what happens when the deferred node is *still* undecided —
both operands and its result unsolved. That is a **teaching error** asking for
an annotation, never a silent float guess:

```
`+` here could be float arithmetic or `Px` arithmetic — annotate an operand
(e.g. `(a: Px)`) so the operator can be resolved
```

Note how narrow that is: the error needs BOTH operands *and* the result to be
unconstrained. `(a, b) => a + b` in a project that declares an operator for `+`
is ambiguous and says so; `(a, b): float => a + b`, `(a) => a + 1.0`, and every
`+` in a project with no operator declarations infer and run exactly as they
did before. And a brand with no implementation for the operator keeps the old
teaching error, now naming what the brand *does* declare:

```
`-` needs float operands, got Angle.t — `Angle.t` declares `*`, `+`, but not `-`
```

### Runtime

Unlike a suffixed literal, an operator is **not** desugared at lowering — the
operand types are not known there. So the interpreter dispatches too: an
arithmetic node whose operands are not both numbers consults a table of the
declared implementations, keyed by the operand's runtime tag (a variant's
constructor, or an opaque host value's type name). The table is built once when
a session loads, by applying each unit's own constructor to a probe value and
reading the tag off the result — which works for every unit target shape
without any type information.

Two consequences worth stating:

- **Plain float arithmetic is untouched.** The number/number case is still the
  first match in the same `match`; the table is consulted only when an operand
  is not a number. `frame_bench` shows no allocation or byte change.
- **`functor-lang run` (which does not typecheck) behaves identically to the
  checked path**, and so does a fake/plain prelude: both roads end at exactly
  the call the declaration names. An operand with no implementation gets the
  same teaching text at runtime that the checker gives at check time.

### Where the implementations live for prelude brands

`Angle` and `Time` are host types, so their implementations are host externals
(`Angle.add` / `Angle.sub` / `Angle.scale`, `Time.add` / `Time.sub` /
`Time.scale`) declared beside the units in `angle.funi` / `time.funi`. Both are
public API and appear in the generated reference.

### Still open

- **Mixed literals.** `90deg + 45` is an error (the bare number is not lifted
  through the unit's constructor). Erroring is the conservative start.
- **Prefix minus.** `-(someAngle)` would want a `negate` declaration, or to
  derive from `(-)` with a zero the brand cannot supply.
- **Comparisons.** `==`, `<`, `>` on brands are a follow-up.

### Explicit non-goal: dimensional analysis

Functor Lang will **not** track dimensions — no `m/s` derived from `m` and `s`,
no compile-time algebra over unit exponents, no rule that `Px * Px` is an area.
A unit here is a brand with a convenient literal and (Phase 2) arithmetic that
stays inside the brand. Games want "these two numbers are not the same kind of
thing", which brands already deliver; full dimensional analysis is a
substantially larger type-system commitment with a much worse error-message
story, and it is not on the roadmap.
