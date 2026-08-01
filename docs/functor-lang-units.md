# Unit-suffix literals

Functor Lang lets a numeric literal carry a **unit suffix** — `90deg`, `0.5s`,
`16px` — where a `unit` declaration says what the suffix means. This document
covers what shipped (Phase 1) and the design for operators on branded units
(Phase 2), which is **not** implemented.

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

## Phase 2 (designed, not implemented): operators on units

Phase 1 gives a brand a cheap way IN from a number. What it does not give is
arithmetic: `90deg + 45deg` does not typecheck, because `+` is `(float, float)
=> float`. Today the answer is to unwrap, add, and rebrand — which is exactly
the ceremony the suffix removed.

### The declaration

A unit may declare operator implementations beside itself:

```functor
type Px = | Px(value: float)

unit px = Px {
  (+) = addPx        // (Px, Px) => Px
  (-) = subPx        // (Px, Px) => Px
  (*) = scalePx      // (Px, float) => Px   — the scalar form
  (/) = dividePx     // (Px, float) => Px
}
```

- `+` and `-` take `('t, 't) => 't`: adding two lengths gives a length.
- `*` and `/` take `('t, float) => 't`: scaling a length by a number gives a
  length. A brand-by-brand product would be a *different* type (an area), which
  is dimensional analysis — see the non-goal below.
- Every implementation is an ordinary named function, typechecked against the
  shape above at the declaration, exactly as Phase 1 typechecks the constructor.
- Comparisons (`<`, `>`, `==`) are deliberately out of this list for now: `==`
  is already structural for plain-data brands, and ordering wants its own
  design pass.

### Resolution: ad-hoc overloading, after inference

Operator selection happens **after** ordinary Hindley–Milner inference has run
and the operand types are zonked — never during unification, which would make
inference order-dependent:

1. Infer the whole program as today, with `+` constrained as usual.
2. Post-zonk, walk each arithmetic node. If an operand's solved type is a brand
   that declares that operator, replace the node with a call to the declared
   implementation.
3. If neither operand is a declaring brand, the node stays the float operator
   and its ordinary `float` constraint applies.

The critical rule is what happens when the operand type is still an **unsolved
type variable** at that point: that is a *teaching error* asking for an
annotation, never a silent guess.

```
`+` here could be float addition or `Px` addition — annotate the operand
(e.g. `(a: Px)`) so the operator can be resolved
```

This keeps two properties: a program either resolves every operator
deterministically or says so, and code that never uses a unit operator infers
and runs exactly as it does today.

Because resolution is a post-pass rewrite, the IR again ends up with an
ordinary call — the Phase 1 property that there is no second evaluation path is
preserved, so the interpreter is untouched and there is no per-frame cost.

### Open questions for Phase 2

- **Mixed literals.** `90deg + 45` — is the bare number an error, or is it
  lifted through the unit's constructor? Erroring is the conservative start.
- **Prefix minus.** `-(someAngle)` would want a `negate` in the same block, or
  to derive from `(-)` with a zero the brand cannot supply.
- **Where the implementations live** for prelude brands: `Angle` and `Time` are
  host types, so their operator impls are host externals, not Functor Lang
  functions. The declaration form is the same; only the target is.

### Explicit non-goal: dimensional analysis

Functor Lang will **not** track dimensions — no `m/s` derived from `m` and `s`,
no compile-time algebra over unit exponents, no rule that `Px * Px` is an area.
A unit here is a brand with a convenient literal and (Phase 2) arithmetic that
stays inside the brand. Games want "these two numbers are not the same kind of
thing", which brands already deliver; full dimensional analysis is a
substantially larger type-system commitment with a much worse error-message
story, and it is not on the roadmap.
