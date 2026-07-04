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

/// The `mle check` diagnostics for `examples/broken.mle`, compared against
/// the committed `broken.check` golden (rendered exactly as the CLI prints
/// them). Regenerate with `UPDATE_GOLDENS=1 cargo test -p mle`.
#[test]
fn golden_check_broken() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
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
    example_checks_clean("pure-pipeline");
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

#[test]
fn error_call_arity() {
    let (message, line, col) = single_diag(
        "let f = (a: Float, b: Float): Float => a + b\n\
         let g = () => f(1.0)",
    );
    assert_eq!(message, "`f` takes 2 argument(s), got 1");
    assert_eq!((line, col), (2, 15));
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

#[test]
fn error_builtin_arity() {
    let (message, _, _) = single_diag("let x = () => Text.concat(\"a\")");
    assert_eq!(message, "`Text.concat` takes 2 argument(s), got 1");
}

// A builtin's known callback shape checks: List.filter's predicate must
// return Bool (the generic slots stay Unknown, so only the Bool part fires).
#[test]
fn error_filter_predicate_must_return_bool() {
    let (message, _, _) =
        single_diag("let g = (xs: List<Float>) => List.filter(xs, (x): Float => x)");
    assert_eq!(
        message,
        "argument 2 of `List.filter`: expected (Unknown) => Bool, got (Unknown) => Float"
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
fn unannotated_code_is_unchecked() {
    // No annotations anywhere: every check needs a known side, so nothing
    // fires — even though `f(\"s\", 1.0)` would fail at runtime.
    assert_clean(
        "let f = (a, b) => a + b\n\
         let g = () => f(\"s\", true)",
    );
    // A call through an unannotated parameter is unchecked too.
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
    assert_clean("let best = (xs: List<Float>): Float => List.maximum(List.map(xs, (x) => x))");
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
fn gradual_mut_never_false_positives() {
    assert_clean("let f = (x) => let mut a = x in a := a + 1.0; { a with n: a }");
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

/// Construction checks like any call: declared field types and arity.
#[test]
fn error_ctor_argument_type_and_arity() {
    let (message, _, _) = single_diag(&format!("{SHAPE}let x = () => Circle(\"s\")"));
    assert_eq!(
        message,
        "argument 1 of `Circle`: expected Float, got String"
    );
    let (message, _, _) = single_diag(&format!("{SHAPE}let x = () => Rect(1.0)"));
    assert_eq!(message, "`Rect` takes 2 argument(s), got 1");
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
fn gradual_match_never_false_positives() {
    assert_clean(&format!(
        "{SHAPE}let f = (s) => match s with | Circle(r) => r | 1.0 => 2.0 | \"s\" => 3.0"
    ));
    // A match on an unannotated call result is unchecked too.
    assert_clean(&format!(
        "{SHAPE}let g = (s) => s\n\
         let f = (s) => match g(s) with | Point => 1.0"
    ));
}

/// Mixed known/Unknown arm types join to Unknown (no diagnostic), so the
/// match result stays gradual.
#[test]
fn mixed_unknown_arm_types_stay_gradual() {
    assert_clean(&format!(
        "{SHAPE}let g = (x) => x\n\
         let f = (b: Bool): String => match b with | true => g(1.0) | false => \"s\""
    ));
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
