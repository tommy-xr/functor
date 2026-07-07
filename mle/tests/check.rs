//! B4 verification (docs/mle.md): the `broken.mle` diagnostic golden, the
//! shipped examples checking clean, individual diagnostic message + position
//! assertions, and gradual-typing cases that must NOT error.

use std::fs;
use std::path::Path;

/// Parse + lower `src` and typecheck it; returns each diagnostic as
/// (message, line, col).
fn check_src(src: &str) -> Vec<(String, usize, usize)> {
    let program = mle::parse(src).expect("source should parse");
    let module = mle::lower(program).expect("source should lower");
    mle::check(&module)
        .into_iter()
        .map(|diag| {
            let (line, col) = mle::line_col(src, diag.span.start);
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

/// The `mle check` diagnostics for `examples/checks/broken.mle`, compared
/// against the committed `broken.check` golden (rendered exactly as the CLI
/// prints them). Regenerate with `UPDATE_GOLDENS=1 cargo test -p mle`.
/// (The file lives in its own subdirectory: with B8's file=module project
/// loading, every `.mle` in a directory loads together, and a deliberately
/// broken sibling would fail `mle run` on the clean examples.)
#[test]
fn golden_check_broken() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("checks");
    let src = fs::read_to_string(dir.join("broken.mle")).unwrap();
    let golden_path = dir.join("broken.check");
    let actual: String = check_src(&src)
        .into_iter()
        .map(|(message, line, col)| format!("broken.mle:{line}:{col}: error: {message}\n"))
        .collect();
    assert!(!actual.is_empty(), "broken.mle should produce diagnostics");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(&golden_path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — generate with UPDATE_GOLDENS=1 cargo test -p mle",
            golden_path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "diagnostics for broken.mle diverged from broken.check — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p mle"
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
    let src = fs::read_to_string(dir.join(format!("{name}.mle"))).unwrap();
    let diags = check_src(&src);
    assert!(
        diags.is_empty(),
        "{name}.mle should check clean, got {diags:?}"
    );
}

// Individual diagnostics: message text and position.

#[test]
fn error_arithmetic_on_a_string() {
    let (message, line, col) = single_diag("let f = (a: Float) => a + \"s\"");
    assert_eq!(message, "`+` needs Float operands, got String");
    assert_eq!((line, col), (1, 27));
}

#[test]
fn error_comparison_on_a_bool() {
    let (message, line, col) = single_diag("let f = (b: Bool) =>\n  b < 1.0");
    assert_eq!(message, "`<` needs Float operands, got Bool");
    assert_eq!((line, col), (2, 3));
}

#[test]
fn error_equality_across_known_types() {
    let (message, line, col) = single_diag("let x = 1.0 == \"1\"");
    assert_eq!(
        message,
        "`==` compares different types Float and String (always false)"
    );
    assert_eq!((line, col), (1, 9));
}

#[test]
fn error_negating_a_bool() {
    let (message, line, col) = single_diag("let f = (b: Bool) => -b");
    assert_eq!(message, "unary `-` needs a Float operand, got Bool");
    assert_eq!((line, col), (1, 23));
}

#[test]
fn error_record_literal_extra_and_missing_fields() {
    let diags = check_src("type P = { x: Float }\nlet f = (): P => { y: 1.0 }");
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
    let (message, line, col) = single_diag("type P = { x: Float }\nlet f = (): P => { x: \"s\" }");
    assert_eq!(message, "field `x` of `P`: expected Float, got String");
    assert_eq!((line, col), (2, 23));
}

#[test]
fn error_field_access_missing_field() {
    let (message, line, col) = single_diag("type P = { x: Float }\nlet f = (p: P) => p.z");
    assert_eq!(message, "`P` has no field `z`");
    assert_eq!((line, col), (2, 19));
}

#[test]
fn error_field_access_on_a_float() {
    let (message, line, col) = single_diag("let f = (a: Float) => a.x");
    assert_eq!(message, "`.x` on Float, not a record");
    assert_eq!((line, col), (1, 23));
}

// Currying: under-application is a legal partial application, not an arity
// error — `f(1.0)` on a 2-arg `f` yields a `(Float) => Float`. Here the
// partial is the (unannotated) return value, a genuinely function-typed
// position, so it stays clean — the forgotten-argument diagnostic fires only
// when a partial reaches a NON-function position (see the tests below).
#[test]
fn partial_application_of_user_function_is_accepted() {
    assert_clean(
        "let f = (a: Float, b: Float): Float => a + b\n\
         let g = () => f(1.0)",
    );
}

#[test]
fn error_call_argument_type() {
    let (message, line, col) = single_diag(
        "let f = (a: Float): Float => a\n\
         let g = () => f(\"s\")",
    );
    assert_eq!(message, "argument 1 of `f`: expected Float, got String");
    assert_eq!((line, col), (2, 17));
}

#[test]
fn error_builtin_argument_type() {
    let (message, line, col) = single_diag("let x = () => Math.clamp01(\"s\")");
    assert_eq!(
        message,
        "argument 1 of `Math.clamp01`: expected Float, got String"
    );
    assert_eq!((line, col), (1, 28));
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
        "let f = (a: Float, b: Float): Float => a + b\n\
         let g = () => let inc = f(1.0) in inc(2.0)",
    );
    let (message, _, _) = single_diag(
        "let f = (a: Float, b: Float): Float => a + b\n\
         let g = () => let inc = f(1.0) in inc(\"s\")",
    );
    assert_eq!(message, "argument 1 of `inc`: expected Float, got String");
}

// Currying: over-application checks the surplus args against the result type,
// which must itself be a function.
#[test]
fn over_application_checks_surplus_against_result() {
    // A curried function over-applied in one call — clean, and the surplus arg
    // is checked against the inner function's param.
    assert_clean(
        "let adder = (a: Float) => (b: Float) => a + b\n\
         let main = () => adder(3.0, 4.0)",
    );
    // Over-applying a non-function result is an error.
    let (message, _, _) = single_diag(
        "let f = (a: Float): Float => a\n\
         let g = () => f(1.0, 2.0)",
    );
    assert_eq!(message, "cannot call Float, not a function");
}

// Currying's error-quality recovery (OCaml Warning-5 / F# FS0020): a partial
// application flowing into a CONCRETE non-function position is a forgotten
// argument. The diagnostic names the missing parameter(s) — precise because
// the checker knows the callee's full arity, param names, and types.
#[test]
fn forgotten_argument_of_user_function_into_argument_position() {
    let (message, line, col) = single_diag(
        "let add = (a: Float, b: Float): Float => a + b\n\
         let use = (x: Float): Float => x\n\
         let main = () => use(add(1.0))",
    );
    assert_eq!(
        message,
        "`add` is applied to 1 of 2 arguments here — missing `b: Float`. \
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
        "let shift = (dx: Float, dy: Float): Float => dx + dy\n\
         let go = (): Float => shift(1.0)",
    );
    assert_eq!(
        message,
        "`shift` is applied to 1 of 2 arguments here — missing `dy: Float`. \
         Did you forget an argument?"
    );
}

// A builtin has no param names in its signature, so the diagnostic falls back
// to the missing parameter's TYPE alone (still names the arity gap).
#[test]
fn forgotten_argument_of_builtin() {
    let (message, _, _) = single_diag(
        "let use = (x: String): String => x\n\
         let main = () => use(Text.concat(\"a\"))",
    );
    assert_eq!(
        message,
        "`Text.concat` is applied to 1 of 2 arguments here — missing `String`. \
         Did you forget an argument?"
    );
}

// A constructor carries its field names, so the diagnostic names the missing
// field — and the enriched message REPLACES the generic mismatch (one diag).
#[test]
fn forgotten_argument_of_constructor() {
    let (message, _, _) = single_diag(
        "type Pair = | MkPair(a: Float, b: Float)\n\
         let use = (x: Float): Float => x\n\
         let main = () => use(MkPair(1.0))",
    );
    assert_eq!(
        message,
        "`MkPair` is applied to 1 of 2 arguments here — missing `b: Float`. \
         Did you forget an argument?"
    );
}

// The discriminator must NOT fire on legitimate partials — a partial that
// reaches a FUNCTION-typed position is intended. An inline partial passed as
// `List.map`'s callback stays clean.
#[test]
fn partial_into_function_position_is_clean() {
    assert_clean(
        "let add = (a: Float, b: Float): Float => a + b\n\
         let go = (xs: List<Float>): List<Float> => List.map(add(1.0), xs)",
    );
}

// …and a partial bound to a let, then used as a callback, is also clean (the
// binding value is inferred, never checked against a non-function expectation).
#[test]
fn bound_partial_used_as_callback_is_clean() {
    assert_clean(
        "let add = (a: Float, b: Float): Float => a + b\n\
         let go = (xs: List<Float>): List<Float> =>\n\
         let inc = add(1.0) in\n\
         xs |> List.map(inc)",
    );
}

// A builtin's known callback shape checks: List.filter's predicate must
// return Bool (the generic slots stay Unknown, so only the Bool part fires).
#[test]
fn error_filter_predicate_must_return_bool() {
    let (message, _, _) =
        single_diag("let g = (xs: List<Float>) => List.filter((x): Float => x, xs)");
    assert_eq!(
        message,
        // B7: the element type flows from `xs` into the predicate's
        // signature — Float, not Unknown. Subject-last: the predicate is arg 1.
        "argument 1 of `List.filter`: expected (Float) => Bool, got (Float) => Float"
    );
}

#[test]
fn error_return_annotation_mismatch() {
    let (message, line, col) = single_diag("let f = (): Bool => 1.0");
    assert_eq!(message, "return value: expected Bool, got Float");
    assert_eq!((line, col), (1, 21));
}

#[test]
fn error_calling_a_known_non_function() {
    let (message, line, col) = single_diag("let x = 1.0\nlet g = () => x()");
    assert_eq!(message, "cannot call Float, not a function");
    assert_eq!((line, col), (2, 15));
}

// A *known* type name applied at the wrong arity is an error (check #8)…
#[test]
fn error_type_argument_arity() {
    let (message, line, col) = single_diag("type P = { x: Float }\nlet f = (p: P<Float>) => p");
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
    // (Float, Float) => Float — the bad call is caught by INFERENCE.
    let diags = check_src(
        "let f = (a, b) => a + b\n\
         let g = () => f(\"s\", true)",
    );
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(diags[0].0, "argument 1 of `f`: expected Float, got String");
    assert_eq!(diags[1].0, "argument 2 of `f`: expected Float, got Bool");
    // A call through an unannotated parameter CONSTRAINS it instead.
    assert_clean("let apply = (f) => f(1.0, 2.0)");
}

#[test]
fn records_flow_gradually() {
    // `mk`'s unannotated return type is Unknown, so passing its result where
    // a Position is expected (and accessing fields on it) is unchecked.
    assert_clean(
        "type Position = { x: Float, y: Float }\n\
         let mk = () => { x: 1.0, y: 2.0 }\n\
         let getX = (p: Position): Float => p.x\n\
         let go = () => getX(mk()) + mk().y",
    );
}

#[test]
fn generic_builtin_slots_stay_unknown() {
    // List.map's result is List<Unknown>, which is compatible with the
    // List<Float> that List.maximum expects (no generic instantiation).
    assert_clean("let best = (xs: List<Float>): Float => List.maximum(List.map((x) => x, xs))");
}

#[test]
fn forward_type_declarations_resolve() {
    // Record type names resolve regardless of declaration order.
    assert_clean("let getX = (p: Later): Float => p.x\ntype Later = { x: Float }");
}

// Expected types propagate into list literals element by element, so a list
// of record literals checks against List<Player> — and a bad element is
// caught (the one non-gradual list case).
#[test]
fn expected_list_types_check_elements() {
    assert_clean(
        "type P = { x: Float }\n\
         let f = (ps: List<P>): List<P> => ps\n\
         let go = () => f([{ x: 1.0 }, { x: 2.0 }])",
    );
    let (message, _, _) = single_diag(
        "type P = { x: Float }\n\
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
        "type A = { x: Float }\n\
         type B = { x: Float }\n\
         let same = (a: A, b: B): Bool => a == b",
    );
}

// Differing declared shapes guarantee structural inequality — error.
#[test]
fn different_shaped_records_compare_error() {
    let diags = check_src(
        "type A = { x: Float }\n\
         type B = { y: Float }\n\
         let never = (a: A, b: B): Bool => a == b",
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
        "let f = (x: Float): Float => x\n\
         let g = (x: Float): Float => x\n\
         let bad = (): Bool => f == g",
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
         | Circle(radius: Float)\n\
         let f = (x): Bool => Circle == x",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "functions cannot be compared with `==`");
}

// A record literal can never satisfy a known non-record type. [Claude M1]
#[test]
fn record_literal_in_float_position_errors() {
    let diags = check_src(
        "let f = (a: Float): Float => a + a\n\
         let main = () => f({ x: 1.0 })",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].0,
        "argument 1 of `f`: expected Float, got a record literal"
    );
}

// Quiet enrichment: an unannotated lambda return is inferred from its body,
// so downstream checks fire. [Codex High, probe 1]
#[test]
fn inferred_return_type_flows_to_callers() {
    let diags = check_src(
        "let f = (a: Float) => a\n\
         let main = (): Bool => f(1.0)",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "return value: expected Bool, got Float");
}

// Quiet enrichment: a top-level list literal contributes its element type.
// [Codex High, probe 2]
#[test]
fn top_level_list_literal_type_flows() {
    let diags = check_src(
        "let xs = [1.0]\n\
         let main = (): String => Text.toBullets(xs)",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].0,
        "argument 1 of `Text.toBullets`: expected List<String>, got List<Float>"
    );
}

// --- Record updates + local let/mut ---

#[test]
fn assignment_keeps_the_slot_type() {
    let diags = check_src("let f = (x: Float) => let mut a = x in a := \"s\"; a");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "assignment to `a`: expected Float, got String");
}

#[test]
fn update_checks_fields_against_the_declared_type() {
    let diags = check_src(
        "type Position = { x: Float, y: Float }\n\
         let f = (p: Position): Position => { p with x: \"s\", z: 1.0 }",
    );
    assert_eq!(diags.len(), 2);
    assert_eq!(
        diags[0].0,
        "field `x` of `Position`: expected Float, got String"
    );
    assert_eq!(diags[1].0, "`Position` has no field `z`");
}

#[test]
fn update_on_known_non_record_errors() {
    let diags = check_src("let f = (x: Float) => { x with y: 1.0 }");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "`with` update on Float, not a record");
}

#[test]
fn let_in_types_flow_to_the_body() {
    let diags = check_src("let f = (): Bool => let x = 1.0 in x");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, "return value: expected Bool, got Float");
}

#[test]
fn contradictory_mut_use_is_caught() {
    // B7: `a := a + 1.0` pins the slot to Float, so the record update on it
    // is a contradiction — caught, where the gradual checker stayed silent.
    let (message, _, _) =
        single_diag("let f = (x) => let mut a = x in a := a + 1.0; { a with n: a }");
    assert_eq!(message, "`with` update on Float, not a record");
}

// --- Variants + match (B5 part 1) ---

const SHAPE: &str = "type Shape = | Circle(r: Float) | Rect(w: Float, h: Float) | Point\n";

/// Non-exhaustive match on a known variant type: the diagnostic names the
/// missing constructor(s), in declaration order.
#[test]
fn error_non_exhaustive_variant_match() {
    let (message, line, col) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): Float => match s with | Circle(r) => r"
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
        "{SHAPE}let f = (s: Shape): Float => match s with | Circle(r) => r | _ => 0.0"
    ));
    assert_clean(&format!(
        "{SHAPE}let f = (s: Shape): Float => match s with | Circle(r) => r | other => 0.0"
    ));
}

/// Every constructor covered is exhaustive without a catch-all.
#[test]
fn full_ctor_coverage_is_exhaustive() {
    assert_clean(&format!(
        "{SHAPE}let f = (s: Shape): Float => match s with | Circle(r) => r | Rect(w, h) => w * h | Point => 0.0"
    ));
}

/// A constructor from another variant type can never match.
#[test]
fn error_foreign_ctor_in_a_match() {
    let (message, line, col) = single_diag(&format!(
        "{SHAPE}type Blob = | Splat(size: Float)\n\
         let f = (s: Shape): Float => match s with | Splat(n) => n | _ => 0.0"
    ));
    assert_eq!(message, "`Splat` is not a constructor of `Shape`");
    assert_eq!((line, col), (3, 45));
}

/// A constructor pattern against a known non-variant scrutinee.
#[test]
fn error_ctor_pattern_against_a_float() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (x: Float): Float => match x with | Circle(r) => r | _ => 0.0"
    ));
    assert_eq!(
        message,
        "pattern `Circle` matches `Shape`, but the scrutinee is Float"
    );
}

/// A literal pattern of the wrong type against a known scrutinee.
#[test]
fn error_literal_pattern_against_a_variant() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): Float => match s with | true => 1.0 | _ => 0.0"
    ));
    assert_eq!(message, "pattern matches Bool, but the scrutinee is Shape");
}

/// Bool matches: `true` + `false` (or a catch-all) is exhaustive; a missing
/// literal is named.
#[test]
fn bool_match_exhaustiveness() {
    assert_clean("let f = (b: Bool): Float => match b with | true => 1.0 | false => 0.0");
    assert_clean("let f = (b: Bool): Float => match b with | true => 1.0 | _ => 0.0");
    let (message, _, _) = single_diag("let f = (b: Bool): Float => match b with | true => 1.0");
    assert_eq!(message, "match on Bool is not exhaustive: missing `false`");
}

/// Number/string literal matches can never cover their type: they require a
/// catch-all arm when the scrutinee's type is known.
#[test]
fn literal_matches_require_a_catch_all() {
    let (message, _, _) =
        single_diag("let f = (x: Float): Float => match x with | 1.0 => 1.0 | 2.0 => 2.0");
    assert_eq!(
        message,
        "match on Float is not exhaustive: literal patterns need a catch-all arm (`_` or a name)"
    );
    assert_clean("let f = (x: Float): Float => match x with | 1.0 => 1.0 | _ => 0.0");
    let (message, _, _) = single_diag("let f = (x: String): Float => match x with | \"a\" => 1.0");
    assert_eq!(
        message,
        "match on String is not exhaustive: literal patterns need a catch-all arm (`_` or a name)"
    );
}

/// Arm result types must agree where known; the match's type is their join.
#[test]
fn error_incompatible_arm_types() {
    let (message, _, _) =
        single_diag("let f = (b: Bool) => match b with | true => 1.0 | false => \"no\"");
    assert_eq!(
        message,
        "match arms have incompatible types Float and String"
    );
}

/// The joined match type flows onward (here: into a return-type check).
#[test]
fn match_type_flows_to_the_return_annotation() {
    let (message, _, _) =
        single_diag("let f = (b: Bool): String => match b with | true => 1.0 | false => 0.0");
    assert_eq!(message, "return value: expected String, got Float");
}

/// Pattern variables get the declared field types — they flow into arm
/// bodies…
#[test]
fn pattern_vars_get_declared_field_types() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let f = (s: Shape): String => match s with | Circle(r) => Text.concat(r, \"!\") | _ => \"\""
    ));
    assert_eq!(
        message,
        "argument 1 of `Text.concat`: expected String, got Float"
    );
}

/// …and a catch-all variable binds the scrutinee's type.
#[test]
fn catch_all_var_binds_the_scrutinee_type() {
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let area = (s: Shape): Float => 1.0\n\
         let f = (s: Shape): Float => match s with | other => area(other) + other"
    ));
    assert_eq!(message, "`+` needs Float operands, got Shape");
}

/// Construction checks like any call: declared field types are enforced on the
/// supplied args, and (currying) a partially-applied constructor is a legal
/// value rather than an arity error.
#[test]
fn error_ctor_argument_type_and_partial() {
    let (message, _, _) = single_diag(&format!("{SHAPE}let x = () => Circle(\"s\")"));
    assert_eq!(
        message,
        "argument 1 of `Circle`: expected Float, got String"
    );
    // `Rect(1.0)` on a 2-arg ctor is a legal partial `(Float) => Shape`.
    assert_clean(&format!("{SHAPE}let x = () => Rect(1.0)"));
}

/// Variant types are nominal in annotations, like records.
#[test]
fn variant_return_annotations_check() {
    assert_clean(&format!("{SHAPE}let f = (): Shape => Circle(2.0)"));
    assert_clean(&format!("{SHAPE}let f = (): Shape => Point"));
    let (message, _, _) = single_diag(&format!("{SHAPE}let f = (): Shape => 1.0"));
    assert_eq!(message, "return value: expected Shape, got Float");
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
    assert!(has("pattern matches Float, but the scrutinee is Shape"));
    assert!(has("pattern matches String, but the scrutinee is Shape"));
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
    // B7: `g` is the identity, so `g(1.0)` IS Float — returning it where
    // the annotation promises String is caught at the arm join (the old
    // gradual checker saw Unknown and stayed silent).
    let (message, _, _) = single_diag(&format!(
        "{SHAPE}let g = (x) => x\n\
         let f = (b: Bool): String => match b with | true => g(1.0) | false => \"s\""
    ));
    assert_eq!(
        message,
        "match arms have incompatible types Float and String"
    );
}

// --- Tuples ---

/// Product annotations check element-wise; a known-arity mismatch in a
/// pattern is a can-never-match diagnostic.
#[test]
fn tuple_pattern_arity_mismatch_is_flagged() {
    let diags = check_src("let f = (t: Float * String): Bool => match t with | (x, y, z) => true");
    assert_eq!(diags.len(), 2, "{diags:?}");
    let has = |needle: &str| diags.iter().any(|(m, _, _)| m.contains(needle));
    assert!(
        has("names 3 element(s), but the matched value is Float * String"),
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
    let diags = check_src("let f = (n: Float) => match n with | (a, b) => a");
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0].0.contains("a tuple pattern cannot match Float"),
        "unexpected: {}",
        diags[0].0
    );
}

/// Element types flow through patterns: destructuring a known product gives
/// typed variables (a String element used as Float errors).
#[test]
fn tuple_element_types_flow_through_patterns() {
    let diags = check_src("let f = (t: Float * String): Float => let (n, s) = t in n + s");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].0.contains("String"), "unexpected: {}", diags[0].0);
}

/// Tuple literals meet their product expectation element-wise, so a record
/// element gets the declared-type check instead of hiding behind Unknown.
/// [Codex H — tuples review]
#[test]
fn tuple_elements_meet_declared_types() {
    let diags = check_src("type P = { x: Float }\nlet main = (): P * Float => ({ y: 1.0 }, 2.0)");
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
        "let f = (t: Float * String): Float => match t with | (x, y, z) => 0.0 | (x, y) => x",
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].0.contains("names 3 element(s)"));
}

// --- B7: Hindley–Milner inference ---

/// Let-polymorphism with teeth: `id` used at Float AND String in one
/// module (the SCC-ordered generalization — whole-module letrec would
/// have rejected this).
#[test]
fn polymorphic_defs_instantiate_per_use() {
    assert_clean(
        "let id = (x) => x\n\
         let a = id(1.0)\n\
         let b = id(\"s\")\n\
         let both = (n: Float, s: String) => (id(n) + 1.0, Text.concat(id(s), \"!\"))",
    );
}

/// Lowercase annotation names are scoped type variables — the generic
/// annotations the roadmap promised.
#[test]
fn lowercase_annotations_are_type_variables() {
    assert_clean(
        "let first = (pair: a * b): a => match pair with | (x, _) => x\n\
         let go = () => (first((1.0, \"s\")) + 1.0, first((true, 2.0)))",
    );
    // …and they CONSTRAIN: both params share `a`, so mixed args error.
    let (message, _, _) = single_diag(
        "let same = (x: a, y: a): a => x\n\
         let bad = () => same(1.0, \"s\")",
    );
    assert_eq!(message, "argument 2 of `same`: expected Float, got String");
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
    assert_eq!(message, "list element: expected Float, got String");
}

/// Ambiguous record literals ask for an annotation (nominal F#-style
/// resolution — the B7 record decision).
#[test]
fn ambiguous_record_literals_ask_for_annotation() {
    let (message, _, _) = single_diag(
        "type Vec2 = { x: Float, y: Float }\n\
         type Point2 = { x: Float, y: Float }\n\
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
         let use = (): Bool => even(4.0)",
    );
}

/// Unannotated code gets full inferred signatures — the roadmap's verify
/// line, via hover's type table.
#[test]
fn unannotated_defs_get_inferred_signatures() {
    let src = "let scale = (xs, k) => List.map((x) => x * k, xs)";
    let module = mle::lower(mle::parse(src).unwrap()).unwrap();
    let (diags, types) = mle::check_with_types(&module);
    assert!(diags.is_empty(), "{diags:?}");
    // The def's value expression carries the full inferred signature.
    let ty = types
        .expr(module.defs[0].value.id)
        .expect("type recorded")
        .to_string();
    assert_eq!(ty, "(List<Float>, Float) => List<Float>");
}

// --- B7 review fixes (both engines) ---

/// Unary `-` constrains its operand like binary arithmetic — a check-clean
/// negate-a-string is gone. [BOTH engines]
#[test]
fn unary_minus_constrains_the_operand() {
    let (message, _, _) = single_diag("let f = (x) => -x\nlet y = f(\"s\")");
    assert_eq!(message, "argument 1 of `f`: expected Float, got String");
}

/// Match arms unify into ONE result type — a var arm is pinned by its
/// siblings instead of collapsing the match to Unknown. [BOTH engines]
#[test]
fn arm_results_unify() {
    let (message, _, _) = single_diag(
        "let f = (b, x) => match b with | true => x | false => 1.0\n\
         let z = f(true, \"s\")",
    );
    assert_eq!(message, "argument 2 of `f`: expected Float, got String");
}

/// `==` pins variables at ANY depth, not just top level. [Codex H]
#[test]
fn equality_constrains_nested_variables() {
    let (message, _, _) = single_diag("let f = (x) => (x, 1.0) == (1.0, 1.0)\nlet y = f((z) => z)");
    assert_eq!(message, "argument 1 of `f`: expected Float, got ('a) => 'a");
}

/// Unreachable arms (after a catch-all) are checked but must not CONSTRAIN
/// the scrutinee. [Codex M]
#[test]
fn unreachable_arms_do_not_constrain() {
    assert_clean("let f = (x) => (match x with | _ => 1.0 | \"s\" => 2.0) + x");
}

/// Bool and literal matches on INFERRED scrutinees get exhaustiveness too
/// (the stale-zonk hole). [BOTH engines, High]
#[test]
fn inferred_scrutinees_get_exhaustiveness() {
    let (message, _, _) = single_diag("let f = (x) => match x with | true => 1.0");
    assert!(
        message.contains("match on Bool is not exhaustive: missing `false`"),
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
    let (message, _, _) = single_diag("type Box = | Full(v: a) | Empty\nlet p = Full(1.0)");
    assert!(
        message.contains("undeclared type parameter `a` — declare it on the type"),
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
        "argument 3 of `List.fold`: expected List<'a>, got (Float, 'a) => Float"
    );
}

// --- Generic type declarations ---

/// The whole point: one declaration, many instantiations — Box<Float> and
/// Box<String> coexist, and element types flow through patterns.
#[test]
fn generic_adts_instantiate_per_use() {
    assert_clean(
        "type Box<v> = | Full(value: v) | Empty\n\
         let unwrapOr = (b: Box<v>, fallback: v): v =>\n\
           match b with\n\
           | Full(value) => value\n\
           | Empty => fallback\n\
         let a = unwrapOr(Full(41.0), 0.0) + 1.0\n\
         let b = Text.concat(unwrapOr(Full(\"hi\"), \"\"), \"!\")",
    );
    // …and the instantiation CONSTRAINS: a Float box can't take a String
    // fallback.
    let (message, _, _) = single_diag(
        "type Box<v> = | Full(value: v) | Empty\n\
         let unwrapOr = (b: Box<v>, fallback: v): v =>\n\
           match b with\n\
           | Full(value) => value\n\
           | Empty => fallback\n\
         let bad = unwrapOr(Full(1.0), \"s\")",
    );
    assert_eq!(
        message,
        "argument 2 of `unwrapOr`: expected Float, got String"
    );
}

/// Generic records: literals solve the parameters, field access and `with`
/// updates substitute them.
#[test]
fn generic_records_solve_from_literals() {
    assert_clean(
        "type Pair<x, y> = { first: x, second: y }\n\
         let swap = (p: Pair<x, y>): Pair<y, x> => { first: p.second, second: p.first }\n\
         let go = () => swap({ first: 1.0, second: \"s\" }).second + 1.0",
    );
    let (message, _, _) = single_diag(
        "type Pair<x, y> = { first: x, second: y }\n\
         let go = (): Float => { first: 1.0, second: \"s\" }.second",
    );
    assert_eq!(message, "return value: expected Float, got String");
}

/// Pattern fields get the scrutinee's arguments (Full(v) on Box<Float>
/// binds v: Float — and arithmetic on it checks).
#[test]
fn pattern_fields_take_scrutinee_arguments() {
    let (message, _, _) = single_diag(
        "type Box<v> = | Full(value: v) | Empty\n\
         let f = (b: Box<String>): Float =>\n\
           match b with\n\
           | Full(value) => value + 1.0\n\
           | Empty => 0.0",
    );
    assert_eq!(message, "`+` needs Float operands, got String");
}

/// Type-argument arity is checked against the declaration.
#[test]
fn generic_arity_is_checked() {
    let (message, _, _) = single_diag(
        "type Box<v> = | Full(value: v) | Empty\n\
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
        "type L<a> = | Cons(h: a, t: L<a>) | Nil\n\
         let sum = (l: L<Float>): Float =>\n\
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
        "type R<a> = { x: a }\n\
         let eq = (a: R<String>, b: R<Float>): Bool => a == b",
    );
    assert!(message.contains("always false"), "unexpected: {message}");
    let (message, _, _) = single_diag(
        "type R<a> = { x: a }\n\
         type S = { x: Float }\n\
         let eq = (r: R<String>, s: S): Bool => r == s",
    );
    assert!(message.contains("always false"), "unexpected: {message}");
}

/// A function smuggled through a generic ARGUMENT is still a certain `==`
/// runtime error. [Codex H]
#[test]
fn functions_inside_generic_nominals_cannot_compare() {
    let (message, _, _) = single_diag(
        "type Box<a> = | Full(v: a)\n\
         let main = (): Bool => Full((x) => x) == Full((x) => x)",
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
        "type Msg = | Roll | Rolled(n: Float)\n\
         let update = (m, msg) => match msg with | Roll => (m, Effect.random(Rolled)) | Rolled(n) => m",
        "type Msg = | Roll | Rolled(n: Float)\n\
         let update = (m, msg) => match msg with | Rolled(n) => m | Roll => (m, Effect.random(Rolled))",
        "type Model = { roll: Float }\n\
         type Msg = | Roll | Rolled(n: Float)\n\
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
    // A String element used as a Float via a list pattern errors (a
    // catch-all keeps the ONLY diagnostic the type mismatch).
    let (message, _, _) = single_diag(
        "let f = (xs: List<String>): Float =>\n\
         match xs with | [a, ..rest] => a + 1.0 | _ => 0.0",
    );
    assert!(message.contains("String"), "unexpected: {message}");
    // Cons unifies the head with the tail's element type.
    let (message, _, _) = single_diag("let f = (xs: List<Float>) => [\"s\", ..xs]");
    assert!(
        message.contains("`..` tail") || message.contains("String"),
        "unexpected: {message}"
    );
}

/// A list match needs a catch-all (fixed-length / `[h, ..t]` are refutable);
/// `[..all]` counts as one.
#[test]
fn list_match_exhaustiveness() {
    let (message, _, _) =
        single_diag("let f = (xs: List<Float>): Float => match xs with | [a, b] => a + b");
    assert!(
        message.contains("not exhaustive: add"),
        "unexpected: {message}"
    );
    // The canonical recursion [] + [h, ..t] IS exhaustive now.
    assert_clean(
        "let sum = (xs: List<Float>): Float =>
         match xs with | [] => 0.0 | [h, ..t] => h + sum(t)",
    );
    assert_clean("let f = (xs: List<Float>): Float => match xs with | [..all] => 0.0");
    assert_clean(
        "let f = (xs: List<Float>): Float =>\n\
         match xs with | [a] => a | _ => 0.0",
    );
}

/// A list pattern against a known non-list scrutinee can never match.
#[test]
fn list_pattern_against_non_list() {
    let (message, _, _) =
        single_diag("let f = (n: Float): Float => match n with | [a, ..t] => a | _ => 0.0");
    assert!(
        message.contains("a list pattern cannot match Float"),
        "unexpected: {message}"
    );
}
