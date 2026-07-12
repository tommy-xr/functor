//! B4 verification (docs/functor-lang.md): the `broken.fun` diagnostic golden, the
//! shipped examples checking clean, individual diagnostic message + position
//! assertions, and gradual-typing cases that must NOT error.

use std::fs;
use std::path::Path;

/// Parse + lower `src` and typecheck it; returns each diagnostic as
/// (message, line, col).
fn check_src(src: &str) -> Vec<(String, usize, usize)> {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    functor_lang::check(&module)
        .into_iter()
        .map(|diag| {
            let (line, col) = functor_lang::line_col(src, diag.span.start);
            (diag.message, line, col)
        })
        .collect()
}

/// Assert `src` produces exactly one diagnostic, and return it.
fn single_diag(src: &str) -> (String, usize, usize) {
    let mut diags = check_src(src);
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    diags.remove(0)
}

fn assert_clean(src: &str) {
    let diags = check_src(src);
    assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
}

/// The `functor-lang check` diagnostics for `examples/checks/broken.fun`, compared
/// against the committed `broken.check` golden (rendered exactly as the CLI
/// prints them). Regenerate with `UPDATE_GOLDENS=1 cargo test -p functor-lang`.
/// (The file lives in its own subdirectory: with B8's file=module project
/// loading, every `.fun` in a directory loads together, and a deliberately
/// broken sibling would fail `functor-lang run` on the clean examples.)
#[test]
fn golden_check_broken() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("checks");
    let src = fs::read_to_string(dir.join("broken.fun")).unwrap();
    let golden_path = dir.join("broken.check");
    let actual: String = check_src(&src)
        .into_iter()
        .map(|(message, line, col)| format!("broken.fun:{line}:{col}: error: {message}\n"))
        .collect();
    assert!(!actual.is_empty(), "broken.fun should produce diagnostics");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(&golden_path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — generate with UPDATE_GOLDENS=1 cargo test -p functor-lang",
            golden_path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "diagnostics for broken.fun diverged from broken.check — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p functor-lang"
    );
}

// The shipped examples are annotated and must check clean.

#[test]
fn example_pure_pipeline_checks_clean() {
    example_checks_clean("pure_pipeline");
}

#[test]
fn example_records_checks_clean() {
    example_checks_clean("records");
}

#[test]
fn example_functions_checks_clean() {
    example_checks_clean("functions");
}

#[test]
fn example_shapes_checks_clean() {
    example_checks_clean("shapes");
}

#[test]
fn example_tuples_checks_clean() {
    example_checks_clean("tuples");
}

#[test]
fn example_lists_checks_clean() {
    example_checks_clean("lists");
}

#[test]
fn example_strings_checks_clean() {
    example_checks_clean("strings");
}

fn example_checks_clean(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = fs::read_to_string(dir.join(format!("{name}.fun"))).unwrap();
    let diags = check_src(&src);
    assert!(
        diags.is_empty(),
        "{name}.fun should check clean, got {diags:?}"
    );
}

// Individual diagnostics: message text and position.

#[test]
fn error_arithmetic_on_a_string() {
    let (message, line, col) = single_diag("let f = (a: float) => a + \"s\"");
    assert_eq!(message, "`+` needs float operands, got string");
    assert_eq!((line, col), (1, 27));
}

#[test]
fn error_comparison_on_a_bool() {
    let (message, line, col) = single_diag("let f = (b: bool) =>\n  b < 1.0");
    assert_eq!(message, "`<` needs float operands, got bool");
    assert_eq!((line, col), (2, 3));
}

#[test]
fn error_equality_across_known_types() {
    let (message, line, col) = single_diag("let x = 1.0 == \"1\"");
    assert_eq!(
        message,
        "`==` compares different types float and string (always false)"
    );
    assert_eq!((line, col), (1, 9));
}

#[test]
fn error_negating_a_bool() {
    let (message, line, col) = single_diag("let f = (b: bool) => -b");
    assert_eq!(message, "unary `-` needs a float operand, got bool");
    assert_eq!((line, col), (1, 23));
}

#[test]
fn error_record_literal_extra_and_missing_fields() {
    let diags = check_src("type P = { x: float }\nlet f = (): P => { y: 1.0 }");
    // Sorted by position: the missing-field diagnostic sits at the record
    // literal's opening brace, before the extra field.
    assert_eq!(
        diags,
        [
            (
                "record literal for `P` is missing field `x`".to_string(),
                2,
                18
            ),
            ("`P` has no field `y`".to_string(), 2, 20),
        ]
    );
}

#[test]
fn error_record_literal_field_type() {
    let (message, line, col) = single_diag("type P = { x: float }\nlet f = (): P => { x: \"s\" }");
    assert_eq!(message, "field `x` of `P`: expected float, got string");
    assert_eq!((line, col), (2, 23));
}

#[test]
fn error_field_access_missing_field() {
    let (message, line, col) = single_diag("type P = { x: float }\nlet f = (p: P) => p.z");
    assert_eq!(message, "`P` has no field `z`");
    assert_eq!((line, col), (2, 19));
}

#[test]
fn error_field_access_on_a_float() {
    let (message, line, col) = single_diag("let f = (a: float) => a.x");
    assert_eq!(message, "`.x` on float, not a record");
    assert_eq!((line, col), (1, 23));
}

// Currying: under-application is a legal partial application, not an arity
// error — `f(1.0)` on a 2-arg `f` yields a `(float) => float`. Here the
// partial is the (unannotated) return value, a genuinely function-typed
// position, so it stays clean — the forgotten-argument diagnostic fires only
// when a partial reaches a NON-function position (see the tests below).
#[test]
fn partial_application_of_user_function_is_accepted() {
    assert_clean(
        "let f = (a: float, b: float): float => a + b\n\
         let g = () => f(1.0)",
    );
}

#[test]
fn error_call_argument_type() {
    let (message, line, col) = single_diag(
        "let f = (a: float): float => a\n\
         let g = () => f(\"s\")",
    );
    assert_eq!(message, "argument 1 of `f`: expected float, got string");
    assert_eq!((line, col), (2, 17));
}

#[test]
fn error_builtin_argument_type() {
    let (message, line, col) = single_diag("let x = () => Math.clamp01(\"s\")");
    assert_eq!(
        message,
        "argument 1 of `Math.clamp01`: expected float, got string"
    );
    assert_eq!((line, col), (1, 28));
}

// The math builtins check like any other: a two-arg builtin flags a bad
// argument, and `Math.pi` is a plain `Float` value (usable in arithmetic).
#[test]
fn math_builtins_check() {
    assert_clean("let x = () => Math.sqrt(2.0)");
    assert_clean("let x = () => Math.pow(2.0, 8.0)");
    assert_clean("let area = (r: float): float => Math.pi * r * r");
    let (message, _, _) = single_diag("let x = () => Math.mod(1.0, \"s\")");
    assert_eq!(message, "argument 2 of `Math.mod`: expected float, got string");
}

// `Random.step`/`Random.range` are typed builtins: the seed is a float and the
// result is a `(float, float)` tuple, so a destructuring `let` checks cleanly.
#[test]
fn random_step_typechecks() {
    assert_clean("let x = () => let (v, s) = Random.step(1.0) in v + s");
    assert_clean("let x = () => let (v, s) = Random.range(0.0, 10.0, 1.0) in v + s");
}

#[test]
fn random_step_rejects_non_float_seed() {
    let (message, _, _) = single_diag("let x = () => Random.step(\"s\")");
    assert_eq!(
        message,
        "argument 1 of `Random.step`: expected float, got string"
    );
}

// Currying: a partially-applied builtin is a legal value, not an arity error
// — `Text.concat("a")` is a `(String) => String`.
#[test]
fn partial_application_of_builtin_is_accepted() {
    assert_clean("let x = () => Text.concat(\"a\")");
}

// Currying: a partial has the type of the not-yet-supplied params, so
// saturating it later still checks its remaining argument.
#[test]
fn partial_application_then_saturate_checks() {
    assert_clean(
        "let f = (a: float, b: float): float => a + b\n\
         let g = () => let inc = f(1.0) in inc(2.0)",
    );
    let (message, _, _) = single_diag(
        "let f = (a: float, b: float): float => a + b\n\
         let g = () => let inc = f(1.0) in inc(\"s\")",
    );
    assert_eq!(message, "argument 1 of `inc`: expected float, got string");
}

// Currying: over-application checks the surplus args against the result type,
// which must itself be a function.
#[test]
fn over_application_checks_surplus_against_result() {
    // A curried function over-applied in one call — clean, and the surplus arg
    // is checked against the inner function's param.
    assert_clean(
        "let adder = (a: float) => (b: float) => a + b\n\
         let main = () => adder(3.0, 4.0)",
    );
    // Over-applying a non-function result is an error.
    let (message, _, _) = single_diag(
        "let f = (a: float): float => a\n\
         let g = () => f(1.0, 2.0)",
    );
    assert_eq!(message, "cannot call float, not a function");
}

// Currying's error-quality recovery (OCaml Warning-5 / F# FS0020): a partial
// application flowing into a CONCRETE non-function position is a forgotten
// argument. The diagnostic names the missing parameter(s) — precise because
// the checker knows the callee's full arity, param names, and types.
#[test]
fn forgotten_argument_of_user_function_into_argument_position() {
    let (message, line, col) = single_diag(
        "let add = (a: float, b: float): float => a + b\n\
         let use = (x: float): float => x\n\
         let main = () => use(add(1.0))",
    );
    assert_eq!(
        message,
        "`add` is applied to 1 of 2 arguments here — missing `b: float`. \
         Did you forget an argument?"
    );
    // Points at the under-applied call, not the outer call.
    assert_eq!((line, col), (3, 22));
}

// The same recovery in a return-annotation position (a concrete non-function
// expectation), naming the second parameter as the one forgotten.
#[test]
fn forgotten_argument_into_return_annotation() {
    let (message, _, _) = single_diag(
        "let shift = (dx: float, dy: float): float => dx + dy\n\
         let go = (): float => shift(1.0)",
    );
    assert_eq!(
        message,
        "`shift` is applied to 1 of 2 arguments here — missing `dy: float`. \
         Did you forget an argument?"
    );
}

// A builtin has no param names in its signature, so the diagnostic falls back
// to the missing parameter's TYPE alone (still names the arity gap).
#[test]
fn forgotten_argument_of_builtin() {
    let (message, _, _) = single_diag(
        "let use = (x: string): string => x\n\
         let main = () => use(Text.concat(\"a\"))",
    );
    assert_eq!(
        message,
        "`Text.concat` is applied to 1 of 2 arguments here — missing `string`. \
         Did you forget an argument?"
    );
}

// A constructor carries its field names, so the diagnostic names the missing
// field — and the enriched message REPLACES the generic mismatch (one diag).
#[test]
fn forgotten_argument_of_constructor() {
    let (message, _, _) = single_diag(
        "type Pair = | MkPair(a: float, b: float)\n\
         let use = (x: float): float => x\n\
         let main = () => use(MkPair(1.0))",
    );
    assert_eq!(
        message,
        "`MkPair` is applied to 1 of 2 arguments here — missing `b: float`. \
         Did you forget an argument?"
    );
}

// The discriminator must NOT fire on legitimate partials — a partial that
// reaches a FUNCTION-typed position is intended. An inline partial passed as
// `List.map`'s callback stays clean.
#[test]
fn partial_into_function_position_is_clean() {
    assert_clean(
        "let add = (a: float, b: float): float => a + b\n\
         let go = (xs: List<float>): List<float> => List.map(add(1.0), xs)",
    );
}

// …and a partial bound to a let, then used as a callback, is also clean (the
// binding value is inferred, never checked against a non-function expectation).
#[test]
fn bound_partial_used_as_callback_is_clean() {
    assert_clean(
        "let add = (a: float, b: float): float => a + b\n\
         let go = (xs: List<float>): List<float> =>\n\
         let inc = add(1.0) in\n\
         xs |> List.map(inc)",
    );
}

// A builtin's known callback shape checks: List.filter's predicate must
// return bool (the generic slots stay Unknown, so only the bool part fires).
#[test]
fn error_filter_predicate_must_return_bool() {
    let (message, _, _) =
        single_diag("let g = (xs: List<float>) => List.filter((x): float => x, xs)");
    assert_eq!(
        message,
        // B7: the element type flows from `xs` into the predicate's
        // signature — float, not Unknown. The expected function type flows
        // INTO the predicate, so the mismatch localizes to its return value
        // rather than the whole callback (funi 2b `(Lambda, Fn)` checking).
        "return value: expected bool, got float"
    );
}

#[test]
fn error_return_annotation_mismatch() {
    let (message, line, col) = single_diag("let f = (): bool => 1.0");
    assert_eq!(message, "return value: expected bool, got float");
    assert_eq!((line, col), (1, 21));
}

#[test]
fn error_calling_a_known_non_function() {
    let (message, line, col) = single_diag("let x = 1.0\nlet g = () => x()");
    assert_eq!(message, "cannot call float, not a function");
    assert_eq!((line, col), (2, 15));
}

// A *known* type name applied at the wrong arity is an error (check #8)…
#[test]
fn error_type_argument_arity() {
    let (message, line, col) = single_diag("type P = { x: float }\nlet f = (p: P<float>) => p");
    assert_eq!(message, "`P` takes 0 type argument(s), got 1");
    assert_eq!((line, col), (2, 13));

    let (message, _, _) = single_diag("let f = (xs: List) => xs");
    assert_eq!(message, "`List` takes 1 type argument(s), got 0");
}

// Gradual typing: these must NOT error.

// …but an *unknown* type name is not an error — it may be a generic
// parameter (`T`) or a type this module doesn't declare.
#[test]
fn unknown_type_names_are_not_errors() {
    assert_clean("let id = (x: T): T => x");
    // An Unknown annotation checks against nothing, even with a known body.
    assert_clean("let f = (x: T): T => 1.0");
}

#[test]
fn unannotated_code_is_inferred() {
    // B7: no annotations anywhere, and the arithmetic still pins `f` to
    // (float, float) => float — the bad call is caught by INFERENCE.
    let diags = check_src(
        "let f = (a, b) => a + b\n\
         let g = () => f(\"s\", true)",
    );
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(diags[0].0, "argument 1 of `f`: expected float, got string");
    assert_eq!(diags[1].0, "argument 2 of `f`: expected float, got bool");
    // A call through an unannotated parameter CONSTRAINS it instead.
    assert_clean("let apply = (f) => f(1.0, 2.0)");
}

#[test]
fn records_flow_gradually() {
    // `mk`'s unannotated return type is Unknown, so passing its result where
    // a Position is expected (and accessing fields on it) is unchecked.
    assert_clean(
        "type Position = { x: float, y: float }\n\
         let mk = () => { x: 1.0, y: 2.0 }\n\
         let getX = (p: Position): float => p.x\n\
         let go = () => getX(mk()) + mk().y",
    );
}

#[test]
fn generic_builtin_slots_stay_unknown() {
    // List.map's result is List<Unknown>, which is compatible with the
    // List<float> that List.maximum expects (no generic instantiation).
    assert_clean("let best = (xs: List<float>): float => List.maximum(List.map((x) => x, xs))");
}

#[test]
fn forward_type_declarations_resolve() {
    // Record type names resolve regardless of declaration order.
    assert_clean("let getX = (p: Later): float => p.x\ntype Later = { x: float }");
}

// Expected types propagate into list literals element by element, so a list
// of record literals checks against List<Player> — and a bad element is
// caught (the one non-gradual list case).
#[test]
fn expected_list_types_check_elements() {
    assert_clean(
        "type P = { x: float }\n\
         let f = (ps: List<P>): List<P> => ps\n\
         let go = () => f([{ x: 1.0 }, { x: 2.0 }])",
    );
    let (message, _, _) = single_diag(
        "type P = { x: float }\n\
         let f = (ps: List<P>): List<P> => ps\n\
         let go = () => f([{ y: 1.0, x: 0.0 }])",
    );
    assert_eq!(message, "`P` has no field `y`");
}

// --- Cross-engine review pins (C1-stack review of B4) ---

// Runtime equality is structural: two same-shaped nominal types may compare
// (and can be true) — NOT an error. [Claude H1]
#[test]
fn same_shaped_records_may_compare() {
    assert_clean(
        "type A = { x: float }\n\
         type B = { x: float }\n\
         let same = (a: A, b: B): bool => a == b",
    );
}

// Differing declared shapes guarantee structural inequality — error.
#[test]
fn different_shaped_records_compare_error() {
    let diags = check_src(
        "type A = { x: float }\n\
         type B = { y: float }\n\
         let never = (a: A, b: B): bool => a == b",
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0]
        .0
        .contains("`==` compares records with different shapes"));
}

// Comparing two known functions always fails at runtime. [Claude L2]
#[test]
fn function_equality_is_an_error() {
    let diags = check_src(
        "let f = (x: float): float => x\n\
         let g = (x: float): float => x\n\
         let bad = (): bool => f == g",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "functions cannot be compared with `==`");
}

// ONE known function operand is enough — the runtime rejects `==` whenever
// either side is a function (including an unapplied constructor), so an
// Unknown other side cannot make it succeed. [Codex M — B5 review]
#[test]
fn function_equality_against_unknown_is_an_error() {
    let diags = check_src(
        "type Shape =\n\
         | Circle(radius: float)\n\
         let f = (x): bool => Circle == x",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "functions cannot be compared with `==`");
}

// A record literal can never satisfy a known non-record type. [Claude M1]
#[test]
fn record_literal_in_float_position_errors() {
    let diags = check_src(
        "let f = (a: float): float => a + a\n\
         let main = () => f({ x: 1.0 })",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].0,
        "argument 1 of `f`: expected float, got a record literal"
    );
}

// Quiet enrichment: an unannotated lambda return is inferred from its body,
// so downstream checks fire. [Codex High, probe 1]
#[test]
fn inferred_return_type_flows_to_callers() {
    let diags = check_src(
        "let f = (a: float) => a\n\
         let main = (): bool => f(1.0)",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "return value: expected bool, got float");
}

// Quiet enrichment: a top-level list literal contributes its element type.
// [Codex High, probe 2]
#[test]
fn top_level_list_literal_type_flows() {
    let diags = check_src(
        "let xs = [1.0]\n\
         let main = (): string => Text.toBullets(xs)",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].0,
        "argument 1 of `Text.toBullets`: expected List<string>, got List<float>"
    );
}

// --- Record updates + local let/mut ---

#[test]
fn assignment_keeps_the_slot_type() {
    let diags = check_src("let f = (x: float) => let mut a = x in a := \"s\"; a");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "assignment to `a`: expected float, got string");
}

#[test]
fn update_checks_fields_against_the_declared_type() {
    let diags = check_src(
        "type Position = { x: float, y: float }\n\
         let f = (p: Position): Position => { p with x: \"s\", z: 1.0 }",
    );
    assert_eq!(diags.len(), 2);
    assert_eq!(
        diags[0].0,
        "field `x` of `Position`: expected float, got string"
    );
    assert_eq!(diags[1].0, "`Position` has no field `z`");
}

#[test]
fn update_on_known_non_record_errors() {
    let diags = check_src("let f = (x: float) => { x with y: 1.0 }");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "`with` update on float, not a record");
}

#[test]
fn let_in_types_flow_to_the_body() {
    let diags = check_src("let f = (): bool => let x = 1.0 in x");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "return value: expected bool, got float");
}

#[test]
fn contradictory_mut_use_is_caught() {
    // B7: `a := a + 1.0` pins the slot to float, so the record update on it
    // is a contradiction — caught, where the gradual checker stayed silent.
    let (message, _, _) =
        single_diag("let f = (x) => let mut a = x in a := a + 1.0; { a with n: a }");
    assert_eq!(message, "`with` update on float, not a record");
}

// --- Variants + match (B5 part 1) ---

const SHAPE: &str = "type Shape = | Circle(r: float) | Rect(w: float, h: float) | Point\n";

/// Non-exhaustive match on a known variant type: the diagnostic names the
/// missing constructor(s), in declaration order.
#[test]
fn error_non_exhaustive_variant_match() {
    let (message, line, col) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r"
    ));
    assert_eq!(
        message,
        "match on `Shape` is not exhaustive: missing `Rect`, `Point`"
    );
    assert_eq!((line, col), (2, 30));
}

/// A catch-all arm (`_` or a variable) makes any match exhaustive.
#[test]
fn catch_all_arms_are_exhaustive() {
    assert_clean(&format!(
        "{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r | _ => 0.0"
    ));
    assert_clean(&format!(
        "{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r | other => 0.0"
    ));
}

/// Every constructor covered is exhaustive without a catch-all.
#[test]
fn full_ctor_coverage_is_exhaustive() {
    assert_clean(&format!(
        "{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r | Rect(w, h) => w * h | Point => 0.0"
    ));
}

/// A constructor from another variant type can never match.
#[test]
fn error_foreign_ctor_in_a_match() {
    let (message, line, col) = single_diag(&format!(
        "{SHAPE}type Blob = | Splat(size: float)\n\
         let f = (s: Shape): float => match s with | Splat(n) => n | _ => 0.0"
    ));
    assert_eq!(message, "`Splat` is not a constructor of `Shape`");
    assert_eq!((line, col), (3, 45));
}

/// A constructor pattern against a known non-variant scrutinee.
#[test]
fn error_ctor_pattern_against_a_float() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (x: float): float => match x with | Circle(r) => r | _ => 0.0"
    ));
    assert_eq!(
        message,
        "pattern `Circle` matches `Shape`, but the scrutinee is float"
    );
}

/// A literal pattern of the wrong type against a known scrutinee.
#[test]
fn error_literal_pattern_against_a_variant() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): float => match s with | true => 1.0 | _ => 0.0"
    ));
    assert_eq!(message, "pattern matches bool, but the scrutinee is Shape");
}

/// bool matches: `true` + `false` (or a catch-all) is exhaustive; a missing
/// literal is named.
#[test]
fn bool_match_exhaustiveness() {
    assert_clean("let f = (b: bool): float => match b with | true => 1.0 | false => 0.0");
    assert_clean("let f = (b: bool): float => match b with | true => 1.0 | _ => 0.0");
    let (message, _, _) = single_diag("let f = (b: bool): float => match b with | true => 1.0");
    assert_eq!(message, "match on bool is not exhaustive: missing `false`");
}

/// Number/string literal matches can never cover their type: they require a
/// catch-all arm when the scrutinee's type is known.
#[test]
fn literal_matches_require_a_catch_all() {
    let (message, _, _) =
        single_diag("let f = (x: float): float => match x with | 1.0 => 1.0 | 2.0 => 2.0");
    assert_eq!(
        message,
        "match on float is not exhaustive: literal patterns need a catch-all arm (`_` or a name)"
    );
    assert_clean("let f = (x: float): float => match x with | 1.0 => 1.0 | _ => 0.0");
    let (message, _, _) = single_diag("let f = (x: string): float => match x with | \"a\" => 1.0");
    assert_eq!(
        message,
        "match on string is not exhaustive: literal patterns need a catch-all arm (`_` or a name)"
    );
}

/// Arm result types must agree where known; the match's type is their join.
#[test]
fn error_incompatible_arm_types() {
    let (message, _, _) =
        single_diag("let f = (b: bool) => match b with | true => 1.0 | false => \"no\"");
    assert_eq!(
        message,
        "match arms have incompatible types float and string"
    );
}

/// The joined match type flows onward (here: into a return-type check).
#[test]
fn match_type_flows_to_the_return_annotation() {
    let (message, _, _) =
        single_diag("let f = (b: bool): string => match b with | true => 1.0 | false => 0.0");
    assert_eq!(message, "return value: expected string, got float");
}

/// Pattern variables get the declared field types — they flow into arm
/// bodies…
#[test]
fn pattern_vars_get_declared_field_types() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): string => match s with | Circle(r) => Text.concat(r, \"!\") | _ => \"\""
    ));
    assert_eq!(
        message,
        "argument 1 of `Text.concat`: expected string, got float"
    );
}

/// …and a catch-all variable binds the scrutinee's type.
#[test]
fn catch_all_var_binds_the_scrutinee_type() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let area = (s: Shape): float => 1.0\n\
         let f = (s: Shape): float => match s with | other => area(other) + other"
    ));
    assert_eq!(message, "`+` needs float operands, got Shape");
}

/// Construction checks like any call: declared field types are enforced on the
/// supplied args, and (currying) a partially-applied constructor is a legal
/// value rather than an arity error.
#[test]
fn error_ctor_argument_type_and_partial() {
    let (message, _, _) = single_diag(&format!("{SHAPE}let x = () => Circle(\"s\")"));
    assert_eq!(
        message,
        "argument 1 of `Circle`: expected float, got string"
    );
    // `Rect(1.0)` on a 2-arg ctor is a legal partial `(float) => Shape`.
    assert_clean(&format!("{SHAPE}let x = () => Rect(1.0)"));
}

/// Variant types are nominal in annotations, like records.
#[test]
fn variant_return_annotations_check() {
    assert_clean(&format!("{SHAPE}let f = (): Shape => Circle(2.0)"));
    assert_clean(&format!("{SHAPE}let f = (): Shape => Point"));
    let (message, _, _) = single_diag(&format!("{SHAPE}let f = (): Shape => 1.0"));
    assert_eq!(message, "return value: expected Shape, got float");
}

// Gradual: these must NOT error.

/// An Unknown scrutinee is unchecked: no exhaustiveness demands, ctor and
/// literal arms of any mix, and pattern variables still get their declared
/// field types without ever false-positives.
#[test]
fn match_patterns_constrain_the_scrutinee() {
    // B7: the first ctor pattern pins the scrutinee to Shape; foreign
    // literal arms are can-never-match errors now (a match has ONE
    // scrutinee type — the F#/Elm reading).
    let diags = check_src(&format!(
        "{SHAPE}let f = (s) => match s with | Circle(r) => r | 1.0 => 2.0 | \"s\" => 3.0"
    ));
    // Three diags: the two foreign arms AND the exhaustiveness hole the
    // review found (the ctor arm solved the scrutinee, so the re-zonked
    // exhaustiveness check now fires on inferred scrutinees too).
    assert_eq!(diags.len(), 3, "{diags:?}");
    let has = |needle: &str| diags.iter().any(|(m, _, _)| m.contains(needle));
    assert!(has(
        "match on `Shape` is not exhaustive: missing `Rect`, `Point`"
    ));
    assert!(has("pattern matches float, but the scrutinee is Shape"));
    assert!(has("pattern matches string, but the scrutinee is Shape"));
    // Constraining flows THROUGH calls: g is the identity, so matching
    // g(s) against Point pins s to Shape — and the inferred scrutinee gets
    // the SAME exhaustiveness protection an annotated one does.
    let diags = check_src(&format!(
        "{SHAPE}let g = (s) => s\n\
         let f = (s) => match g(s) with | Point => 1.0"
    ));
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0]
        .0
        .contains("match on `Shape` is not exhaustive: missing `Circle`, `Rect`"));
}

/// Mixed known/Unknown arm types join to Unknown (no diagnostic), so the
/// match result stays gradual.
#[test]
fn polymorphic_arm_results_are_constrained() {
    // B7: `g` is the identity, so `g(1.0)` IS float — returning it where
    // the annotation promises String is caught at the arm join (the old
    // gradual checker saw Unknown and stayed silent).
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let g = (x) => x\n\
         let f = (b: bool): string => match b with | true => g(1.0) | false => \"s\""
    ));
    assert_eq!(
        message,
        "match arms have incompatible types float and string"
    );
}

// --- Tuples ---

/// Product annotations check element-wise; a known-arity mismatch in a
/// pattern is a can-never-match diagnostic.
#[test]
fn tuple_pattern_arity_mismatch_is_flagged() {
    let diags = check_src("let f = (t: (float, string)): bool => match t with | (x, y, z) => true");
    assert_eq!(diags.len(), 2, "{diags:?}");
    let has = |needle: &str| diags.iter().any(|(m, _, _)| m.contains(needle));
    assert!(
        has("names 3 element(s), but the matched value is (float, string)"),
        "{diags:?}"
    );
    // The mismatched arm must NOT count as exhaustive: the match as a whole
    // is uncovered too. [Codex M — tuples review]
    assert!(
        has("not exhaustive: no arm matches a 2-element tuple"),
        "{diags:?}"
    );
}

/// A tuple pattern against a known non-tuple can never match.
#[test]
fn tuple_pattern_against_non_tuple_is_flagged() {
    let diags = check_src("let f = (n: float) => match n with | (a, b) => a");
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0].0.contains("a tuple pattern cannot match float"),
        "unexpected: {}",
        diags[0].0
    );
}

/// Element types flow through patterns: destructuring a known product gives
/// typed variables (a String element used as float errors).
#[test]
fn tuple_element_types_flow_through_patterns() {
    let diags = check_src("let f = (t: (float, string)): float => let (n, s) = t in n + s");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].0.contains("string"), "unexpected: {}", diags[0].0);
}

/// Tuple literals meet their product expectation element-wise, so a record
/// element gets the declared-type check instead of hiding behind Unknown.
/// [Codex H — tuples review]
#[test]
fn tuple_elements_meet_declared_types() {
    let diags = check_src("type P = { x: float }\nlet main = (): (P, float) => ({ y: 1.0 }, 2.0)");
    assert!(
        diags.iter().any(|(m, _, _)| m.contains("`y`")),
        "expected the record-literal field check to fire: {diags:?}"
    );
}

/// `==` on tuples with a known function element is a certain runtime error.
/// [Codex M — tuples review]
#[test]
fn tuple_equality_with_function_elements_is_an_error() {
    let diags = check_src("let main = () => ((x) => x, 1.0) == ((x) => x, 1.0)");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].0, "functions cannot be compared with `==`");
}

/// An arity-matching arm among mismatched ones IS exhaustive — only the
/// per-arm mismatch diags fire.
#[test]
fn matching_arity_arm_satisfies_exhaustiveness() {
    let diags = check_src(
        "let f = (t: (float, string)): float => match t with | (x, y, z) => 0.0 | (x, y) => x",
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].0.contains("names 3 element(s)"));
}

// --- B7: Hindley–Milner inference ---

/// Let-polymorphism with teeth: `id` used at float AND String in one
/// module (the SCC-ordered generalization — whole-module letrec would
/// have rejected this).
#[test]
fn polymorphic_defs_instantiate_per_use() {
    assert_clean(
        "let id = (x) => x\n\
         let a = id(1.0)\n\
         let b = id(\"s\")\n\
         let both = (n: float, s: string) => (id(n) + 1.0, Text.concat(id(s), \"!\"))",
    );
}

/// Apostrophe-prefixed annotation names are scoped type variables — the
/// generic annotations the roadmap promised.
#[test]
fn apostrophe_annotations_are_type_variables() {
    assert_clean(
        "let first = (pair: ('a, 'b)): 'a => match pair with | (x, _) => x\n\
         let go = () => (first((1.0, \"s\")) + 1.0, first((true, 2.0)))",
    );
    // …and they CONSTRAIN: both params share `'a`, so mixed args error.
    let (message, _, _) = single_diag(
        "let same = (x: 'a, y: 'a): 'a => x\n\
         let bad = () => same(1.0, \"s\")",
    );
    assert_eq!(message, "argument 2 of `same`: expected float, got string");
}

/// The occurs check: `'a = List<'a>` is an infinite type, reported not
/// looped on.
#[test]
fn occurs_check_reports_infinite_types() {
    let (message, _, _) = single_diag("let g = (f) => f(f)");
    assert!(
        message.contains("cannot construct the infinite type"),
        "unexpected: {message}"
    );
}

/// Mixed-element lists are real errors now (one element type, unified).
#[test]
fn mixed_lists_are_errors() {
    let (message, _, _) = single_diag("let xs = [1.0, \"s\"]");
    assert_eq!(message, "list element: expected float, got string");
}

/// Ambiguous record literals ask for an annotation (nominal F#-style
/// resolution — the B7 record decision).
#[test]
fn ambiguous_record_literals_ask_for_annotation() {
    let (message, _, _) = single_diag(
        "type Vec2 = { x: float, y: float }\n\
         type Point2 = { x: float, y: float }\n\
         let p = { x: 1.0, y: 2.0 }",
    );
    assert_eq!(
        message,
        "ambiguous record literal: fields match Point2 and Vec2 — annotate which one is meant"
    );
}

/// Inference flows through mutual recursion (one SCC, monomorphic inside,
/// generalized after).
#[test]
fn mutual_recursion_infers() {
    assert_clean(
        "let even = (n) => match n < 1.0 with | true => true | false => odd(n - 1.0)\n\
         let odd = (n) => match n < 1.0 with | true => false | false => even(n - 1.0)\n\
         let use = (): bool => even(4.0)",
    );
}

/// Unannotated code gets full inferred signatures — the roadmap's verify
/// line, via hover's type table.
#[test]
fn unannotated_defs_get_inferred_signatures() {
    let src = "let scale = (xs, k) => List.map((x) => x * k, xs)";
    let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
    let (diags, types) = functor_lang::check_with_types(&module);
    assert!(diags.is_empty(), "{diags:?}");
    // The def's value expression carries the full inferred signature.
    let ty = types
        .expr(module.defs[0].value.id)
        .expect("type recorded")
        .to_string();
    assert_eq!(ty, "(List<float>, float) => List<float>");
}

// --- B7 review fixes (both engines) ---

/// Unary `-` constrains its operand like binary arithmetic — a check-clean
/// negate-a-string is gone. [BOTH engines]
#[test]
fn unary_minus_constrains_the_operand() {
    let (message, _, _) = single_diag("let f = (x) => -x\nlet y = f(\"s\")");
    assert_eq!(message, "argument 1 of `f`: expected float, got string");
}

/// Match arms unify into ONE result type — a var arm is pinned by its
/// siblings instead of collapsing the match to Unknown. [BOTH engines]
#[test]
fn arm_results_unify() {
    let (message, _, _) = single_diag(
        "let f = (b, x) => match b with | true => x | false => 1.0\n\
         let z = f(true, \"s\")",
    );
    assert_eq!(message, "argument 2 of `f`: expected float, got string");
}

/// `==` pins variables at ANY depth, not just top level. [Codex H]
#[test]
fn equality_constrains_nested_variables() {
    let (message, _, _) = single_diag("let f = (x) => (x, 1.0) == (1.0, 1.0)\nlet y = f((z) => z)");
    assert_eq!(message, "argument 1 of `f`: expected float, got ('a) => 'a");
}

/// Unreachable arms (after a catch-all) are checked but must not CONSTRAIN
/// the scrutinee. [Codex M]
#[test]
fn unreachable_arms_do_not_constrain() {
    assert_clean("let f = (x) => (match x with | _ => 1.0 | \"s\" => 2.0) + x");
}

/// bool and literal matches on INFERRED scrutinees get exhaustiveness too
/// (the stale-zonk hole). [BOTH engines, High]
#[test]
fn inferred_scrutinees_get_exhaustiveness() {
    let (message, _, _) = single_diag("let f = (x) => match x with | true => 1.0");
    assert!(
        message.contains("match on bool is not exhaustive: missing `false`"),
        "unexpected: {message}"
    );
    let (message, _, _) = single_diag("let f = (x) => match x with | 2.0 => 1.0");
    assert!(message.contains("not exhaustive"), "unexpected: {message}");
}

/// Type variables in DECLARATIONS are refused with a teaching error —
/// a declaration-held variable would be module-global (first use pins it
/// for everyone). [BOTH engines]
#[test]
fn undeclared_type_params_are_refused() {
    let (message, _, _) = single_diag("type Box = | Full(v: 'a) | Empty\nlet p = Full(1.0)");
    assert!(
        message.contains("undeclared type parameter `'a` — declare it on the type"),
        "unexpected: {message}"
    );
}

/// Diagnostics normalize BOTH sides with one variable order — the same
/// variable never wears two names in one message. [Claude M]
#[test]
fn diagnostic_variables_share_one_order() {
    let (message, _, _) = single_diag("let f = (x) => List.fold(x, 1.0, x)");
    // Subject-last `List.fold(fn, init, list)`: x is both the folder (arg 1)
    // and the list (arg 3), so 'a — the element type — appears on BOTH sides
    // of the message with one consistent variable order.
    assert_eq!(
        message,
        "argument 3 of `List.fold`: expected List<'a>, got (float, 'a) => float"
    );
}

// --- Generic type declarations ---

/// The whole point: one declaration, many instantiations — Box<float> and
/// Box<string> coexist, and element types flow through patterns.
#[test]
fn generic_adts_instantiate_per_use() {
    assert_clean(
        "type Box<'v> = | Full(value: 'v) | Empty\n\
         let unwrapOr = (b: Box<'v>, fallback: 'v): 'v =>\n\
           match b with\n\
           | Full(value) => value\n\
           | Empty => fallback\n\
         let a = unwrapOr(Full(41.0), 0.0) + 1.0\n\
         let b = Text.concat(unwrapOr(Full(\"hi\"), \"\"), \"!\")",
    );
    // …and the instantiation CONSTRAINS: a float box can't take a String
    // fallback.
    let (message, _, _) = single_diag(
        "type Box<'v> = | Full(value: 'v) | Empty\n\
         let unwrapOr = (b: Box<'v>, fallback: 'v): 'v =>\n\
           match b with\n\
           | Full(value) => value\n\
           | Empty => fallback\n\
         let bad = unwrapOr(Full(1.0), \"s\")",
    );
    assert_eq!(
        message,
        "argument 2 of `unwrapOr`: expected float, got string"
    );
}

/// Generic records: literals solve the parameters, field access and `with`
/// updates substitute them.
#[test]
fn generic_records_solve_from_literals() {
    assert_clean(
        "type Pair<'x, 'y> = { first: 'x, second: 'y }\n\
         let swap = (p: Pair<'x, 'y>): Pair<'y, 'x> => { first: p.second, second: p.first }\n\
         let go = () => swap({ first: 1.0, second: \"s\" }).second + 1.0",
    );
    let (message, _, _) = single_diag(
        "type Pair<'x, 'y> = { first: 'x, second: 'y }\n\
         let go = (): float => { first: 1.0, second: \"s\" }.second",
    );
    assert_eq!(message, "return value: expected float, got string");
}

/// Pattern fields get the scrutinee's arguments (Full(v) on Box<float>
/// binds v: float — and arithmetic on it checks).
#[test]
fn pattern_fields_take_scrutinee_arguments() {
    let (message, _, _) = single_diag(
        "type Box<'v> = | Full(value: 'v) | Empty\n\
         let f = (b: Box<string>): float =>\n\
           match b with\n\
           | Full(value) => value + 1.0\n\
           | Empty => 0.0",
    );
    assert_eq!(message, "`+` needs float operands, got string");
}

/// Type-argument arity is checked against the declaration.
#[test]
fn generic_arity_is_checked() {
    let (message, _, _) = single_diag(
        "type Box<'v> = | Full(value: 'v) | Empty\n\
         let f = (b: Box) => b",
    );
    assert_eq!(message, "`Box` takes 1 type argument(s), got 0");
}

// --- Generics review fixes (Codex) ---

/// Recursive and forward generic references resolve at the declared arity
/// (variant param counts are pre-seeded, like records). [Codex H]
#[test]
fn recursive_generic_declarations_resolve() {
    assert_clean(
        "type L<'a> = | Cons(h: 'a, t: L<'a>) | Nil\n\
         let sum = (l: L<float>): float =>\n\
           match l with\n\
           | Cons(h, t) => h + sum(t)\n\
           | Nil => 0.0\n\
         let go = () => sum(Cons(1.0, Cons(2.0, Nil)))",
    );
}

/// Same declaration with incompatible arguments is certainly-false `==`;
/// cross-declaration shape comparison uses SUBSTITUTED fields. [Codex H]
#[test]
fn generic_record_equality_certainty() {
    let (message, _, _) = single_diag(
        "type R<'a> = { x: 'a }\n\
         let eq = (a: R<string>, b: R<float>): bool => a == b",
    );
    assert!(message.contains("always false"), "unexpected: {message}");
    let (message, _, _) = single_diag(
        "type R<'a> = { x: 'a }\n\
         type S = { x: float }\n\
         let eq = (r: R<string>, s: S): bool => r == s",
    );
    assert!(message.contains("always false"), "unexpected: {message}");
}

/// A function smuggled through a generic ARGUMENT is still a certain `==`
/// runtime error. [Codex H]
#[test]
fn functions_inside_generic_nominals_cannot_compare() {
    let (message, _, _) = single_diag(
        "type Box<'a> = | Full(v: 'a)\n\
         let main = (): bool => Full((x) => x) == Full((x) => x)",
    );
    assert_eq!(message, "functions cannot be compared with `==`");
}

/// The B6 contract lift: a bare-model arm beside a (model, effect) arm
/// joins as the pair (the producer treats bare as (m, Effect.none())) —
/// without this, every effect game failed `functor build` on the occurs
/// check ('a = 'a * Unknown). Both arm orders; annotated models too.
#[test]
fn effect_pair_arms_join_with_bare_model_arms() {
    for src in [
        "type Msg = | Roll | Rolled(n: float)\n\
         let update = (m, msg) => match msg with | Roll => (m, Effect.random(Rolled)) | Rolled(n) => m",
        "type Msg = | Roll | Rolled(n: float)\n\
         let update = (m, msg) => match msg with | Rolled(n) => m | Roll => (m, Effect.random(Rolled))",
        "type Model = { roll: float }\n\
         type Msg = | Roll | Rolled(n: float)\n\
         let update = (m: Model, msg) => match msg with | Roll => (m, Effect.random(Rolled)) | Rolled(n) => { m with roll: n }",
    ] {
        let diags = check_src(src);
        assert!(diags.is_empty(), "should lift: {src}\n{diags:?}");
    }
    // The lift keys on the HOST seam — a real tuple mismatch still errors.
    let diags = check_src("let f = (b, m) => match b with | true => (m, 1.0) | false => m");
    assert_eq!(diags.len(), 1, "{diags:?}");
}

// --- List patterns + cons ---

/// Element types flow through cons and list patterns.
#[test]
fn list_patterns_flow_element_types() {
    // A String element used as a float via a list pattern errors (a
    // catch-all keeps the ONLY diagnostic the type mismatch).
    let (message, _, _) = single_diag(
        "let f = (xs: List<string>): float =>\n\
         match xs with | [a, ..rest] => a + 1.0 | _ => 0.0",
    );
    assert!(message.contains("string"), "unexpected: {message}");
    // Cons unifies the head with the tail's element type.
    let (message, _, _) = single_diag("let f = (xs: List<float>) => [\"s\", ..xs]");
    assert!(
        message.contains("`..` tail") || message.contains("string"),
        "unexpected: {message}"
    );
}

/// A list match needs a catch-all (fixed-length / `[h, ..t]` are refutable);
/// `[..all]` counts as one.
#[test]
fn list_match_exhaustiveness() {
    let (message, _, _) =
        single_diag("let f = (xs: List<float>): float => match xs with | [a, b] => a + b");
    assert!(
        message.contains("not exhaustive: add"),
        "unexpected: {message}"
    );
    // The canonical recursion [] + [h, ..t] IS exhaustive now.
    assert_clean(
        "let sum = (xs: List<float>): float =>
         match xs with | [] => 0.0 | [h, ..t] => h + sum(t)",
    );
    assert_clean("let f = (xs: List<float>): float => match xs with | [..all] => 0.0");
    assert_clean(
        "let f = (xs: List<float>): float =>\n\
         match xs with | [a] => a | _ => 0.0",
    );
}

/// A list pattern against a known non-list scrutinee can never match.
#[test]
fn list_pattern_against_non_list() {
    let (message, _, _) =
        single_diag("let f = (n: float): float => match n with | [a, ..t] => a | _ => 0.0");
    assert!(
        message.contains("a list pattern cannot match float"),
        "unexpected: {message}"
    );
}

// ── Function & tuple type syntax (funi slice 2a) ─────────────────────────

/// A higher-order parameter annotated with a function type checks clean and
/// its type flows: `apply` applies `f` to `x`.
#[test]
fn function_type_annotation_on_a_parameter() {
    assert_clean("let apply = (f: (float) => float, x: float): float => f(x)");
    assert_clean("let twice = (f: (float) => float, x: float): float => f(f(x))");
}

/// A function-typed argument is checked against its declared signature — the
/// expected `(float) => float` flows into the lambda, so a `(String) => String`
/// argument flags both its param and its return precisely.
#[test]
fn function_typed_argument_is_checked() {
    let diags = check_src(
        "let apply = (f: (float) => float, x: float): float => f(x)\n\
         let bad = () => apply((s: string): string => s, 1.0)",
    );
    let msgs: Vec<&str> = diags.iter().map(|(m, _, _)| m.as_str()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("parameter `s`") && m.contains("expected float")),
        "want a param mismatch, got {msgs:?}"
    );
}

/// A zero-argument function type parses and checks.
#[test]
fn nullary_function_type() {
    assert_clean("let run = (cb: () => float): float => cb()");
}

/// `(A, B)` is the tuple type spelling (the old `*` is gone).
#[test]
fn paren_tuple_type_annotation() {
    assert_clean(
        "let swap = (p: (float, string)): (string, float) => match p with | (a, b) => (b, a)",
    );
}

/// A function type used as a *return* annotation must be parenthesized — then
/// it checks (currying: `adder(n)` returns a `(float) => float`).
#[test]
fn parenthesized_function_return_type() {
    assert_clean("let adder = (n: float): ((float) => float) => (x: float): float => x + n");
}

/// `(A)` is grouping — the same as `A`.
#[test]
fn parenthesized_group_is_the_inner_type() {
    assert_clean("let f = (x: (float)): float => x + 1.0");
}

/// The removed `*` product spelling is now a parse error in type position.
#[test]
fn star_tuple_type_is_rejected() {
    assert!(
        functor_lang::parse("let f = (p: float * float): float => 0.0").is_err(),
        "`*` should no longer parse as a tuple type"
    );
}

/// Bare `()` is only a zero-argument function type, not an empty tuple.
#[test]
fn empty_parens_require_an_arrow() {
    assert!(
        functor_lang::parse("let f = (x: ()): float => 0.0").is_err(),
        "`()` alone is not a type"
    );
}

// ── let-binding type annotations (funi slice 2b) ─────────────────────────

/// A top-level binding annotation checks against the value and flows into an
/// unannotated body: `let f: (float) => float = (x) => …` gives `x: float`.
#[test]
fn top_level_binding_annotation() {
    assert_clean("let x: float = 3.0");
    assert_clean("let f: (float) => float = (x) => x + 1.0");
    assert_clean("type M = { a: bool }\nlet m: M = { a: true }");
}

/// A binding annotation is enforced — a simple, a record-literal, and a
/// function return-type mismatch all error.
#[test]
fn binding_annotation_mismatch_is_flagged() {
    let (message, _, _) = single_diag("let x: bool = 3.0");
    assert_eq!(message, "`x`: expected bool, got float");

    let (message, _, _) = single_diag("type M = { a: bool }\nlet m: M = { a: 3.0 }");
    assert_eq!(message, "field `a` of `M`: expected bool, got float");

    // The annotation flows `x: float` in and pushes the expected return into
    // the body, so the mismatch localizes to the return value.
    let (message, _, _) = single_diag("let f: (float) => bool = (x) => x");
    assert_eq!(message, "return value: expected bool, got float");
}

/// let-in bindings take annotations too, checked the same way.
#[test]
fn let_in_binding_annotation() {
    assert_clean("let g = () => let y: float = 3.0 in y + 1.0");
    let (message, _, _) = single_diag("let g = () => let y: bool = 3.0 in y");
    assert_eq!(message, "`y`: expected bool, got float");
}

/// A `mut` let-in binding can be annotated.
#[test]
fn annotated_mut_binding() {
    assert_clean("let f = (n: float): float => let mut acc: float = n in acc := acc + 1.0; acc");
}

/// An invalid binding annotation (here a `List` arity error) reports at the
/// top level — not only inside `let … in` (funi 2b review: was swallowed
/// because a top-level `def.ty` is resolved just once, silently).
#[test]
fn invalid_top_level_annotation_reports() {
    let (message, _, _) = single_diag("let xs: List = [1.0, 2.0]");
    assert!(
        message.contains('`') && message.to_lowercase().contains("type argument"),
        "want an arity diagnostic, got {message:?}"
    );
}

/// A binding annotation's param types flow into an unannotated lambda body,
/// so a bad field access against the declared param type is caught (funi 2b
/// review Finding 2 — the `(Lambda, Fn)` checking path).
#[test]
fn binding_annotation_flows_into_body() {
    let (message, _, _) = single_diag(
        "type Position = { x: float }\n\
         let getX: (Position) => float = (p) => p.z",
    );
    assert_eq!(message, "`Position` has no field `z`");
}

// ── abstract (opaque) types (funi slice 2c) ──────────────────────────────

/// A `type Name` with no body is an opaque nominal: it resolves in
/// annotations and unifies with itself.
#[test]
fn abstract_type_resolves_and_unifies() {
    assert_clean(
        "type SceneNode\n\
         let identity = (n: SceneNode): SceneNode => n\n\
         let pair = (a: SceneNode, b: SceneNode): SceneNode => a",
    );
    // Also usable in a 2b binding annotation and generic position.
    assert_clean(
        "type SceneNode\n\
         let mk = (n: SceneNode): List<SceneNode> => let one: SceneNode = n in [one]",
    );
}

/// An abstract type is distinct from every other type — a mismatch is caught.
#[test]
fn abstract_type_mismatch_is_flagged() {
    let (message, _, _) = single_diag("type SceneNode\nlet bad = (n: SceneNode): float => n");
    assert_eq!(message, "return value: expected float, got SceneNode");
}

/// An abstract type has NO constructor — its values come only from host code,
/// so naming it as a value is an unresolved name (at lowering).
#[test]
fn abstract_type_has_no_constructor() {
    let program = functor_lang::parse("type SceneNode\nlet bad = () => SceneNode").expect("parse");
    let err = functor_lang::lower(program).expect_err("SceneNode is a type, not a value");
    assert!(err.message.contains("SceneNode"), "unexpected: {}", err.message);
}

/// Abstract types may be generic: `type Handle<'a>`.
#[test]
fn generic_abstract_type() {
    assert_clean(
        "type Handle<'a>\n\
         let use = (h: Handle<float>): Handle<float> => h",
    );
    // Arity is enforced, like other nominal types.
    let (message, _, _) = single_diag("type Handle<'a>\nlet f = (h: Handle): float => 0.0");
    assert!(
        message.contains("Handle") && message.contains("type argument"),
        "{message}"
    );
}

/// Forgetting `=` on a type decl keeps its targeted diagnostic — it is NOT
/// silently swallowed as an abstract type (funi 2c review, Finding 1).
#[test]
fn forgotten_type_equals_is_diagnosed() {
    for src in ["type Point { x: float }", "type Shape | Circle(r: float)"] {
        let err = functor_lang::parse(src).expect_err("a missing `=` should error");
        assert!(
            err.message.contains("`=` before the type body"),
            "unexpected for {src:?}: {}",
            err.message
        );
    }
}

// --- Boolean operators typecheck as Bool ---

#[test]
fn bool_operators_check_clean() {
    assert_clean(
        "let f = (a: bool, b: bool): bool => a && b || not a\n\
         let g = (x: float): bool => x > 0.0 && not (x == 1.0)",
    );
}

// --- Literals inside tuple / constructor patterns ---

#[test]
fn tuple_literal_patterns_check_clean() {
    assert_clean(
        "let f = (key: string, down: bool) =>\n\
         \x20 match (key, down) with\n\
         \x20 | (\"Enter\", true) => 1.0\n\
         \x20 | (_, _) => 0.0",
    );
}

#[test]
fn logical_operand_must_be_bool() {
    let (message, line, _) = single_diag("let f = () => 1.0 && true");
    assert_eq!(message, "`&&`/`||` needs bool operands, got float");
    assert_eq!(line, 1);
}

#[test]
fn not_operand_must_be_bool() {
    let (message, _, _) = single_diag("let f = () => not 3.0");
    assert_eq!(message, "`not` needs bool operands, got float");
}

#[test]
fn logical_result_is_bool() {
    // The `&&` result feeds a float context — the mismatch proves it typed
    // as Bool, not gradual Unknown.
    let (message, _, _) = single_diag(
        "let f = (a: bool, b: bool): float => a && b",
    );
    assert!(
        message.contains("bool") && message.contains("float"),
        "unexpected: {message}"
    );
}

// --- `if … then … else …` conditional expression ---

#[test]
fn if_else_checks_clean() {
    assert_clean(
        "let abs = (n: float): float => if n > 0.0 then n else 0.0 - n\n\
         let label = (n: float): string =>\n\
         \x20 if n > 10.0 then \"big\" else if n > 0.0 then \"small\" else \"zero-ish\"",
    );
}

#[test]
fn ctor_literal_patterns_check_clean() {
    assert_clean(
        "type Shape = | Circle(r: float) | Rect(w: float, h: float)\n\
         let f = (s: Shape) =>\n\
         \x20 match s with\n\
         \x20 | Circle(0.0) => \"pt\"\n\
         \x20 | Circle(r) => \"c\"\n\
         \x20 | Rect(w, h) => \"r\"",
    );
}

#[test]
fn if_condition_must_be_bool() {
    // A genuinely-non-bool condition (a float literal) — an unannotated param
    // would simply be inferred to bool instead.
    let (message, line, _) = single_diag("let f = () => if 3.0 then 1.0 else 2.0");
    assert_eq!(message, "`if` condition needs a bool, got float");
    assert_eq!(line, 1);
}

#[test]
fn if_branches_must_unify() {
    let (message, _, _) = single_diag("let f = (b: bool) => if b then 1.0 else \"two\"");
    assert_eq!(
        message,
        "`if` branches have incompatible types float and string"
    );
}

#[test]
fn tuple_literal_sub_pattern_is_not_a_catch_all() {
    // `("Enter", true)` is refutable, so without a catch-all the match is
    // not exhaustive.
    let (message, _, _) = single_diag(
        "let f = (a: string, b: bool) =>\n\
         \x20 match (a, b) with\n\
         \x20 | (\"Enter\", true) => 1.0",
    );
    // The arm matches the arity but is refutable — the message must point at
    // the missing catch-all, not claim no arm matches the arity.
    assert_eq!(
        message,
        "match on (string, bool) is not exhaustive: its arms are refutable — \
         add a catch-all (`_` or a name)"
    );
}

#[test]
fn if_result_type_flows() {
    // The `if` feeds a string context but yields float — the mismatch proves
    // the whole `if` typed as float, not gradual Unknown.
    let (message, _, _) = single_diag("let f = (b: bool): string => if b then 1.0 else 2.0");
    assert!(
        message.contains("float") && message.contains("string"),
        "unexpected: {message}"
    );
}

#[test]
fn else_if_chain_checks_clean() {
    assert_clean(
        "let sign = (n: float): float =>\n\
         \x20 if n > 0.0 then 1.0 else if n < 0.0 then 0.0 - 1.0 else 0.0",
    );
}

/// A nested `if` in an else-if chain keeps its OWN type in the type table
/// (hover), even when the outer chain is ill-typed — the iterative else-spine
/// records each node's own else-suffix type, not the shared outer result.
#[test]
fn nested_if_keeps_its_own_type() {
    use functor_lang::ir::ExprKind;
    // Outer branch is bool, inner if is float+float -> the outer chain can't
    // unify (bool vs float), but the inner `if c2 then 1.0 else 2.0` is float.
    let src = "let f = (c1: bool, c2: bool) => if c1 then true else if c2 then 1.0 else 2.0";
    let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
    let (_diags, types) = functor_lang::check_with_types(&module);
    let ExprKind::Lambda { body, .. } = &module.defs[0].value.kind else {
        panic!("expected a lambda");
    };
    let ExprKind::If { else_branch, .. } = &body.kind else {
        panic!("expected an outer `if`");
    };
    assert!(
        matches!(else_branch.kind, ExprKind::If { .. }),
        "expected a nested `if` in the else position"
    );
    let inner = types.expr(else_branch.id).expect("inner if type recorded");
    assert_eq!(inner.to_string(), "float");
}

#[test]
fn ctor_literal_sub_pattern_does_not_cover_the_ctor() {
    // `Circle(0.0)` matches some circles, not all — `Circle` is still missing.
    let diags = check_src(
        "type Shape = | Circle(r: float) | Rect(w: float, h: float)\n\
         let f = (s: Shape) =>\n\
         \x20 match s with\n\
         \x20 | Circle(0.0) => \"pt\"\n\
         \x20 | Rect(w, h) => \"r\"",
    );
    assert!(
        diags
            .iter()
            .any(|(m, _, _)| m.contains("not exhaustive") && m.contains("Circle")),
        "expected a 'missing Circle' diagnostic, got {diags:?}"
    );
}

#[test]
fn literal_sub_pattern_type_mismatch_is_diagnosed() {
    // A string literal where the tuple element is a float.
    let (message, _, _) = single_diag(
        "let f = (a: float, b: bool) =>\n\
         \x20 match (a, b) with\n\
         \x20 | (\"x\", true) => 1.0\n\
         \x20 | (_, _) => 0.0",
    );
    assert!(
        message.contains("string") && message.contains("float"),
        "unexpected: {message}"
    );
}
