# Unit-suffix literals

Functor Lang lets a numeric literal carry a **unit suffix** — `90deg`, `0.5s`,
`16px` — where a `unit` declaration says what the suffix means, and lets that
brand carry arithmetic and comparison: `90deg + 45deg`, `1.5s - 200ms`,
`45deg * 2.0`, `90deg == 90deg`, `1.5s < 2000ms`. This document covers all
three shipped phases — the literals (Phase 1), the arithmetic operators
(Phase 2), and the comparison operators (Phase 3).

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

## Phase 2 (shipped): arithmetic operators on units

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
  ordinary expression: a name, or a lambda. The prelude's `.funi` interfaces
  name host externals.
- **The operator belongs to the BRAND, not the suffix.** The suffix is only how
  the brand is named: `s`, `ms`, `us`, `min`, and `hr` are all `Time.t`, so ONE
  `unit s (+) = Time.add` makes `1.5s - 200ms` work. Declaring the same brand +
  operator twice — through any suffix — is a duplicate error.
- **The brand must be distinguishable at run time**, because the interpreter
  dispatches on a value's tag: a single-constructor variant, or an opaque host
  type (`Angle.t`). A record brand carries no tag and a multi-constructor type
  carries a different one per constructor, so an operator on either is a check
  error at the declaration.
- Comparisons are declarable too, but only their two BASES (`==` and `<`) —
  see Phase 3.

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

The node's RESULT counts as evidence too: `+` and `-` stay inside their brand,
so an annotated `(a, b): Px => a + b` resolves with no operand annotation at
all. (The scalar `*` and `/` cannot be decided that way — which SIDE is the
brand would still be unknown.)

The critical rule is what happens when the deferred node is *still* undecided —
both operands and its result unsolved. That is a **teaching error** asking for
an annotation, never a silent float guess:

```
`+` here could be float arithmetic or `Px` arithmetic — annotate an operand
(e.g. `(a: Px)`) so the operator can be resolved
```

One more case is decided rather than reported: `v * v` — the SAME unsolved
operand on both sides of a scalar operator. The scalar form's operands have
different types, so no branded reading exists; it can only be float. (`v + v`
*does* have a branded reading, so it still asks.)

Note how narrow that leaves the error: it needs BOTH operands *and* the result
to be unconstrained, and a branded reading to be possible at all.

> **This is a breaking change, and deliberately so.** The engine prelude always
> declares `+`, `-`, and `*` on `Angle.t` and `Time.t`, so a helper like
> `let plus = (a, b) => a + b` — which used to infer as float — now asks for an
> annotation, even in a game that never touches a brand. That is the price of
> resolving operators deterministically instead of guessing; the alternative
> (default to float, and report the mismatch at the call site instead) was
> rejected in the design. Every example in this repository typechecks
> unchanged, so the practical blast radius is small, but existing user code with
> fully unannotated numeric helpers needs one annotation each.

Everything else infers and runs exactly as before: `(a, b): float => a + b`,
`(a) => a + 1.0`, and every `+` in a project with no operator declarations. And
a brand with no implementation for the operator keeps the old teaching error,
now naming what the brand *does* declare:

```
`-` needs float operands, got Angle.t — `Angle.t` declares `+`, `*`, but not `-`
```

### Runtime

Unlike a suffixed literal, an operator is **not** desugared at lowering — the
operand types are not known there. So the interpreter dispatches too: an
arithmetic node whose operands are not both numbers consults a table of the
declared implementations, keyed by the operand's runtime tag (a variant's
constructor, or an opaque host value's type name). It is built by applying each
unit's own constructor to a probe value and reading the tag off the result —
which works for every unit target shape without any type information.

Two consequences worth stating:

- **Plain float arithmetic is untouched.** The number/number case is still the
  first match in the same `match`; the table is consulted only when an operand
  is not a number. `frame_bench` shows no allocation or byte change.
- **The table exists before the module's defs are evaluated**, so a top-level
  constant may use branded arithmetic (`let turn: Angle.t = 90deg + 45deg`). A
  named implementation stays LATE-BOUND, like every other global, so it obeys
  the same rule as any initializer: it must be defined above the constant that
  uses it. A unit whose own *constructor* is a top-level `let` cannot be probed
  that early, so the table is completed as the defs land — the first
  initializer that could use it already can.
- **Nothing is resolved until it is needed.** A declaration's *implementation*
  stays symbolic at load: a host external (`Angle.add`) is looked up only when
  the operator actually dispatches, and a top-level name is late-bound like any
  global. This is not an optimization — the same declarations load under the
  PLAIN, hostless interpreter (the editor's expect gutter runs a project's defs
  that way with the engine `.funi` interfaces linked), where `Angle.add` has no
  implementation and must not be an error until something uses it. For the same
  reason, a brand whose constructor cannot run in this interpreter simply gets
  no entry rather than failing the load: with no host you cannot build one of
  its values either, so nothing can dispatch on it.
- **`functor-lang run` (which does not typecheck) behaves like the checked
  path**, and so does a fake/plain prelude: both roads end at exactly the call
  the declaration names, and both refuse a duplicate declaration. An operand
  with no implementation gets the same teaching sentence at runtime the checker
  gives at check time — naming the brand as that side knows it (the checker has
  the type, `Angle.t`; the interpreter has the runtime tag, `Angle`).

### Where the implementations live for prelude brands

`Angle` and `Time` are host types, so their implementations are host externals
(`Angle.add` / `Angle.sub` / `Angle.scale`, `Time.add` / `Time.sub` /
`Time.scale`) declared beside the units in `angle.funi` / `time.funi`. Both are
public API and appear in the generated reference.

## Phase 3 (shipped): comparison on brands

Phase 2 let a brand add. What it did not let it do is *answer a question*:
`90deg == 90deg` typechecked and then died at run time with `host values
cannot be compared`, and `1.5s < 2000ms` did not typecheck at all. Both work
now, through exactly the Phase 2 machinery.

### Two declarations, six operators

```functor
type Px = | Px(value: float)

unit px = Px
unit px (==) = (a, b) => unwrap(a) == unwrap(b)   // (Px, Px) => bool
unit px (<)  = (a, b) => unwrap(a) < unwrap(b)    // (Px, Px) => bool
```

Only `==` and `<` are declarable. The other four spellings are **derived**:

| Written | Evaluates as |
| --- | --- |
| `a == b` | `equals(a, b)` |
| `a != b` | `not equals(a, b)` |
| `a < b` | `less(a, b)` |
| `a > b` | `less(b, a)` |
| `a <= b` | `not less(b, a)` |
| `a >= b` | `not less(a, b)` |

Deriving rather than declaring is the whole design choice, and it is worth
being explicit about why, because the language's comparison operators do
**not** share a desugaring today — the parser produces six distinct `BinOp`s
and the interpreter gives each its own IEEE float closure. So the mapping
above is new, and it buys three things:

1. **A brand's ordering cannot contradict itself.** With four declarable
   orderings, a brand could ship `a < b` and `a >= b` that disagree. With one,
   `<` *is* the order, and the rest are theorems about it.
2. **Two implementations per brand, not six** — the prelude, and every game
   brand, writes half as much.
3. **It matches what a brand is.** A unit brand wraps ONE scalar (radians,
   seconds, pixels), and a scalar is totally ordered.

The cost is one honest divergence from float semantics, stated plainly: for
floats, `a <= b` is *not* `not (b < a)` when either side is NaN (`nan <= nan`
is false; `not (nan < nan)` is true). A brand built from a NaN therefore
compares as if NaN were an ordinary value under `<=` and `>=`. Float
comparison itself is untouched — this applies only to a branded operand — and
a NaN angle or duration is already a bug the engine boundary rejects.

Everything else is Phase 2's rules verbatim:

- The implementations are typed `('t, 't) => bool`, checked **at the
  declaration** against that shape.
- The operator belongs to the **BRAND**, not the suffix — one `unit s (<)`
  covers `s`, `ms`, `us`, `min`, and `hr`, so `1.5s < 2000ms` works.
- The brand must be **distinguishable at run time** (a single-constructor
  variant, or a host type), because the interpreter dispatches on the tag.
- Declaring the same brand + operator twice, through any suffix, is a
  duplicate error — and `unit px (!=)` / `(>)` / `(<=)` / `(>=)` are parse
  errors naming the base to declare instead.

### Resolution

Identical to Phase 2's post-inference ad-hoc overloading: zonk the operands,
let a declaring brand claim the node, otherwise fall through to float, and
defer a node whose operands are still unsolved. Only the ANSWER differs — a
comparison is `bool` whichever way it resolves, so unlike `+`/`-` its result
carries no evidence and only an operand can decide it.

That makes an unannotated comparison ambiguous once a brand declares `<`:

```
`<` here could be float comparison or `Angle.t` comparison or `Time.t`
comparison — annotate an operand (e.g. `(a: Angle.t)`) so the operator can be
resolved
```

> **This is the same breaking change Phase 2 made, extended to `<`.** A helper
> like `let clamp = (v, lo, hi) => if v < lo then lo else …` now needs one
> annotation (`(v: float, lo, hi)`). Three examples in this repository needed
> exactly that one-word edit; nothing else in the repo moved.

Phase 3 also **narrows** Phase 2's deferral, which strictly reduces how often
that error can fire. A node only defers while a branded reading is still
possible, and which side could still be a brand depends on the operator:
`*` puts the brand on either side, `/` only on the left, and everything
else — `+`, `-`, and the four orderings — combines or compares like with
like. So a side already solved to a non-brand settles the node immediately.
That is what keeps `Math.abs(d) < step` deciding `step` on the spot (and with
it the `rate * dt` that produced `step`), with no annotation anywhere.

### Runtime

The dispatch table Phase 2 built gains two slots. A comparison whose operands
are not both numbers looks up the brand's tag, and the derived spellings swap
and/or negate the one implementation. Two consequences carry over unchanged:

- **Float comparison is untouched** — number/number is still the first match,
  and the table is consulted only when it fails.
- **`==` reaches the table before the structural walk**, but only after two
  cheap gates, ordered cheapest-first: no declared operators at all, then an
  operand with no brand tag (a number, string, record, list, tuple —
  everything `frame_bench` compares). Neither reaches the table, and neither
  even computes an operator slot. `frame_bench` shows byte-identical
  `allocs/frame` and `bytes/frame` and wall-clock parity against the base ref
  (three interleaved release runs).

A brand with no declared `==` keeps STRUCTURAL equality exactly as before, so
adding units to a project cannot change what `==` already meant.

### Where the implementations live for prelude brands

`Angle.equals` / `Angle.less` and `Time.equals` / `Time.less`, declared beside
their units in `angle.funi` / `time.funi`. Both are public API in the
generated reference, and both document the sharp edge honestly: **`==` on an
angle or a duration is float equality underneath.** `90deg == 90deg` is true
because both sides build the same number, but an angle accumulated through
arithmetic may miss an exact literal by a rounding step, and there is no way
back out of the brand to compare with a tolerance — so keep the plain float
where that matters.

## Engine values refuse `==` at CHECK time

The other half of the same story. `myScene == otherScene` used to typecheck
and die at run time (`host values cannot be compared`). It is now a check
error:

```
`==` on `Scene.t`: engine values are opaque — compare the numbers you derived
from them instead (`Scene.t` supports no `==`)
```

An interface (`.funi`) file distinguishes the two kinds of abstract type with
a marker on the `type` item:

```functor
type t = host        // an opaque HOST value: Scene.t, Frame.t, Effect.t, …
type tag             // abstract, but ordinary Functor Lang data underneath
```

`= host` is a contextual keyword in the existing `type <name> = <body>`
position — the least invasive marker available, since docgen quotes a
declaration verbatim from its span and therefore renders it with no change at
all. It is rejected in a `.fun` file: only the host can be responsible for
what it claims.

The polarity is a deliberate deviation from the obvious one. `type t` stays
comparable and the OPAQUE case is what gets marked, because not every
abstract prelude type is a host handle — `Sprite.t` and `Sprite.region` are
plain Functor Lang data behind an opaque name (documented as comparable in
`sprite.funi`), and `Physics.tag` is a brand over a STRING that games compare
constantly:

```functor
match e.a == ballTag with | true => e.b | false => …    // examples/physics
onDeck: probe.hit && probe.tag == deckTag               // examples/physics-controller
```

Marking the opaque types keeps those working by construction. The 30 marked
types are every abstract type in the engine prelude EXCEPT those three.
`Angle.t` and `Time.t` are marked too — they are host values — but they are
exempt from the error because they DECLARE `==`, which is precisely what
makes them comparable.

Three limits, all deliberate:

- The rule walks exactly what the existing "functions cannot be compared"
  certainty rule walks — the operand's own type, map keys/values, tuple
  elements, and a nominal's type ARGUMENTS — not a record's fields. Runtime
  `==` has precisely two refusals (a function value and a host value), and
  each certainty rule reports one of them, so they share one traversal
  policy. A host value buried in a record still meets the runtime error,
  which remains the gradual-seam backstop.
- **Equality is polymorphic, so the rule is direct-only.** `let same = (a, b)
  => a == b` carries no constraint to its call sites, so
  `same(myScene, other)` still checks clean and fails at run time. Closing
  that needs constraint-based typeclasses (an `Eq` bound propagated through
  generalization), which is a much larger design than this change; the
  limitation is pinned by a test rather than left as a surprise.
- **Structural equality for `Scene.t` / `Frame.t` is out of scope.** Whether
  scenes should compare at all — and as `==` or as `Scene.equals` — is a
  separate decision. This check-time rejection is exactly the thing that
  decision would carve an exception into.

### Still open

- **Mixed literals.** `90deg + 45` is an error (the bare number is not lifted
  through the unit's constructor). Erroring is the conservative start.
- **Prefix minus.** `-(someAngle)` would want a `negate` declaration, or to
  derive from `(-)` with a zero the brand cannot supply.
- **Structural equality for engine values** — see above.

### Explicit non-goal: dimensional analysis

Functor Lang will **not** track dimensions — no `m/s` derived from `m` and `s`,
no compile-time algebra over unit exponents, no rule that `Px * Px` is an area.
A unit here is a brand with a convenient literal and (Phase 2) arithmetic that
stays inside the brand. Games want "these two numbers are not the same kind of
thing", which brands already deliver; full dimensional analysis is a
substantially larger type-system commitment with a much worse error-message
story, and it is not on the roadmap.
