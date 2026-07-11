//! B3 verification (docs/functor-lang.md): run/trace goldens per example plus
//! interpreter-semantics and runtime-error assertions.

use functor_lang::value::Value;
use functor_lang::{RunOutcome, Tracing};
use std::fs;
use std::path::Path;

/// Parse + lower + run `src`, panicking with a rendered position on failure.
fn run_src(src: &str, tracing: Tracing) -> functor_lang::RunRecord {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    match functor_lang::run(&module, tracing) {
        Ok(record) => record,
        Err(failure) => {
            let (line, col) = functor_lang::line_col(src, failure.error.span.start);
            panic!("{line}:{col}: error: {}", failure.error.message);
        }
    }
}

/// Run `src` expecting a runtime failure.
fn run_failure(src: &str, tracing: Tracing) -> functor_lang::RunFailure {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    match functor_lang::run(&module, tracing) {
        Err(failure) => failure,
        Ok(_) => panic!("source should fail at runtime"),
    }
}

/// Run `src` expecting a runtime error; returns (message, line, col).
fn run_err(src: &str) -> (String, usize, usize) {
    let failure = run_failure(src, Tracing::Off);
    let (line, col) = functor_lang::line_col(src, failure.error.span.start);
    (failure.error.message, line, col)
}

/// `main`'s printed result for `src`.
fn main_result(src: &str) -> String {
    match run_src(src, Tracing::Off).outcome {
        RunOutcome::Main(value) => value.to_string(),
        RunOutcome::Bindings(_) => panic!("expected a main result"),
    }
}

/// The `functor-lang run` / `functor-lang trace` output for `examples/{name}.fun`, compared
/// against the committed `{name}.run` / `{name}.trace` golden.
/// Regenerate with `UPDATE_GOLDENS=1 cargo test -p functor-lang`.
fn check_golden(name: &str, extension: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = fs::read_to_string(dir.join(format!("{name}.fun"))).unwrap();
    let golden_path = dir.join(format!("{name}.{extension}"));
    let tracing = if extension == "trace" {
        Tracing::On
    } else {
        Tracing::Off
    };
    let record = run_src(&src, tracing);
    let actual = if extension == "trace" {
        functor_lang::render_trace(&record.trace)
    } else {
        match record.outcome {
            RunOutcome::Main(value) => format!("{value}\n"),
            RunOutcome::Bindings(bindings) => bindings
                .into_iter()
                .map(|(name, value)| format!("{name} = {value}\n"))
                .collect(),
        }
    };
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
        "output for {name}.fun diverged from {name}.{extension} — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p functor-lang"
    );
}

#[test]
fn golden_run_pure_pipeline() {
    check_golden("pure_pipeline", "run");
}

#[test]
fn golden_run_records() {
    check_golden("records", "run");
}

#[test]
fn golden_run_functions() {
    check_golden("functions", "run");
}

#[test]
fn golden_run_shapes() {
    check_golden("shapes", "run");
}

#[test]
fn golden_run_tuples() {
    check_golden("tuples", "run");
}

#[test]
fn golden_run_lists() {
    check_golden("lists", "run");
}

#[test]
fn golden_run_strings() {
    check_golden("strings", "run");
}

// One trace golden pins the full enter/exit format; the other examples'
// traces exercise no additional formatting.
#[test]
fn golden_trace_pure_pipeline() {
    check_golden("pure_pipeline", "trace");
}

#[test]
fn closures_capture_their_environment() {
    // `adder(3)` captures `n = 3`; the returned closure sees it later.
    let result = main_result(
        "let adder = (n) => (x) => x + n\n\
         let main = () => adder(3.0)(4.0)",
    );
    assert_eq!(result, "7");
}

#[test]
fn globals_are_late_bound_in_function_bodies() {
    // `first` calls `second`, defined after it — resolved at call time.
    let result = main_result(
        "let first = (x) => second(x) * 2.0\n\
         let second = (x) => x + 1.0\n\
         let main = () => first(5.0)",
    );
    assert_eq!(result, "12");
}

#[test]
fn error_top_level_forward_value_reference() {
    // Top-level values evaluate eagerly in file order; demanding a later
    // global at the top level (not inside a lambda) fails.
    let (message, line, col) = run_err("let a = b + 1.0\nlet b = 2.0");
    assert_eq!(message, "global `b` used before its definition");
    assert_eq!((line, col), (1, 9));
}

// Currying: under-application yields a partial rather than a runtime arity
// error (the accepted quality regression of the currying migration).
#[test]
fn under_application_yields_a_partial() {
    assert_eq!(
        main_result(
            "let f = (a, b) => a + b\n\
             let main = () => f(1.0)"
        ),
        "<partial 1 more>"
    );
}

#[test]
fn error_calling_a_non_function() {
    let (message, line, col) = run_err("let x = 3.0\nlet main = () => x(1.0)");
    assert_eq!(message, "cannot call a number");
    assert_eq!((line, col), (2, 18));
}

#[test]
fn error_unknown_external_with_span() {
    let (message, line, col) = run_err("let main = () => List.frobnicate(1.0)");
    assert_eq!(message, "unknown external `List.frobnicate`");
    assert_eq!((line, col), (1, 18));
}

#[test]
fn error_missing_record_field() {
    let (message, _, _) = run_err("let main = () => { x: 1.0 }.y");
    assert_eq!(message, "record has no field `y`");
}

#[test]
fn equality_is_structural_but_not_for_functions() {
    assert_eq!(
        main_result("let main = () => { x: 1.0, y: [2.0] } == { y: [2.0], x: 1.0 }"),
        "true"
    );
    assert_eq!(main_result("let main = () => 1.0 == \"1\""), "false");
    let (message, _, _) = run_err(
        "let f = (x) => x\n\
         let main = () => f == f",
    );
    assert_eq!(message, "functions cannot be compared with `==`");
}

#[test]
fn text_split_join_round_trip() {
    // split then join with the same separator is identity on the wire.
    assert_eq!(
        main_result("let main = () => Text.join(\",\", Text.split(\",\", \"a,b,c\"))"),
        "\"a,b,c\""
    );
    // splitting an empty string yields one empty field (like F#'s String.Split).
    assert_eq!(
        main_result("let main = () => Text.split(\"|\", \"\")"),
        "[\"\"]"
    );
    // multi-char separator (the reason sep is a String, not a char).
    assert_eq!(
        main_result("let main = () => Text.split(\"::\", \"a::b::c\")"),
        "[\"a\", \"b\", \"c\"]"
    );
}

#[test]
fn text_split_rejects_empty_separator() {
    let (message, _, _) = run_err("let main = () => Text.split(\"\", \"abc\")");
    assert_eq!(message, "Text.split needs a non-empty separator");
}

#[test]
fn text_parse_float_defaults_to_zero_on_garbage() {
    // Mirrors the F# ports' `trim().parse().unwrap_or(0)`.
    assert_eq!(main_result("let main = () => Text.parseFloat(\"  -12  \")"), "-12");
    assert_eq!(main_result("let main = () => Text.parseFloat(\"bogus\")"), "0");
    // "nan"/"inf" parse as f64 but are non-finite garbage — degrade to 0 too,
    // so a corrupt field never injects NaN/inf into the model.
    assert_eq!(main_result("let main = () => Text.parseFloat(\"nan\")"), "0");
    assert_eq!(main_result("let main = () => Text.parseFloat(\"inf\")"), "0");
    assert_eq!(main_result("let main = () => Text.parseFloat(\"-inf\")"), "0");
}

#[test]
fn text_join_rejects_non_strings() {
    let (message, _, _) = run_err("let main = () => Text.join(\",\", [1.0])");
    assert_eq!(message, "Text.join expects strings, got a number");
}

#[test]
fn list_grid_tabulates_a_2d_grid() {
    // f(row, col) over a 2x3 grid, both 0-based (the procedural-heightmap form).
    assert_eq!(
        main_result("let main = () => List.grid((r, c) => r * 10.0 + c, 2.0, 3.0)"),
        "[[0, 1, 2], [10, 11, 12]]"
    );
    // A zero dimension yields an empty structure (no closure calls).
    assert_eq!(
        main_result("let main = () => List.grid((r, c) => r, 0.0, 3.0)"),
        "[]"
    );
}

#[test]
fn list_grid_rejects_non_integer_or_negative_dims() {
    for src in [
        "let main = () => List.grid((r, c) => r, 2.5, 3.0)",
        "let main = () => List.grid((r, c) => r, -1.0, 3.0)",
    ] {
        let (message, _, _) = run_err(src);
        assert!(
            message.contains("whole, non-negative counts"),
            "got: {message}"
        );
    }
}

#[test]
fn error_infinite_recursion_is_a_clean_error() {
    let (message, _, _) = run_err(
        "let spin = (n) => spin(n + 1.0)\n\
         let main = () => spin(0.0)",
    );
    // The improved cap error names the numeric cap and points at the
    // iterative builtins (List.fold), which don't consume evaluation depth.
    assert!(
        message.contains("128") && message.contains("List.fold"),
        "got: {message}"
    );
}

// The parser builds left-assoc chains iteratively and eval walks the lhs
// spine iteratively, so flat straight-line arithmetic never hits the depth
// cap regardless of length.
#[test]
fn long_flat_expression_chains_evaluate() {
    let terms = vec!["1.0"; 500].join(" + ");
    assert_eq!(main_result(&format!("let main = {terms}")), "500");
}

#[test]
fn error_maximum_of_empty_list() {
    let (message, _, _) = run_err("let main = () => List.maximum([])");
    assert_eq!(message, "List.maximum of an empty list");
}

#[test]
fn division_follows_ieee() {
    assert_eq!(main_result("let main = () => 1.0 / 0.0"), "inf");
}

#[test]
fn builtins_evaluate() {
    assert_eq!(main_result("let main = () => Math.clamp01(1.5)"), "1");
    // List-first, so fold composes with |> like map/filter.
    assert_eq!(
        main_result("let main = () => [1.0, 2.0, 3.0] |> List.fold((acc, x) => acc + x, 0.0)"),
        "6"
    );
}

#[test]
fn list_length_isempty_reverse() {
    assert_eq!(main_result("let main = () => List.length([7.0, 8.0, 9.0])"), "3");
    assert_eq!(main_result("let main = () => List.length([])"), "0");
    assert_eq!(main_result("let main = () => List.isEmpty([])"), "true");
    assert_eq!(main_result("let main = () => List.isEmpty([1.0])"), "false");
    assert_eq!(
        main_result("let main = () => List.reverse([1.0, 2.0, 3.0])"),
        "[3, 2, 1]"
    );
    assert_eq!(main_result("let main = () => List.reverse([])"), "[]");
}

#[test]
fn list_append_threads_last() {
    // Subject-LAST: `xs |> List.append(ys)` yields xs followed by ys.
    assert_eq!(
        main_result("let main = () => [1.0, 2.0] |> List.append([3.0, 4.0])"),
        "[1, 2, 3, 4]"
    );
    assert_eq!(
        main_result("let main = () => List.append([3.0], [1.0, 2.0])"),
        "[1, 2, 3]"
    );
    assert_eq!(main_result("let main = () => List.append([], [])"), "[]");
}

#[test]
fn list_flatten_one_level() {
    assert_eq!(
        main_result("let main = () => List.flatten([[1.0, 2.0], [3.0], []])"),
        "[1, 2, 3]"
    );
    assert_eq!(main_result("let main = () => List.flatten([])"), "[]");
    let (message, _, _) = run_err("let main = () => List.flatten([1.0])");
    assert!(message.contains("list of lists"), "got: {message}");
}

#[test]
fn list_any_all_predicates() {
    assert_eq!(
        main_result("let main = () => [1.0, 2.0, 3.0] |> List.any((x) => x > 2.0)"),
        "true"
    );
    assert_eq!(
        main_result("let main = () => [1.0, 2.0, 3.0] |> List.any((x) => x > 5.0)"),
        "false"
    );
    assert_eq!(
        main_result("let main = () => [2.0, 3.0] |> List.all((x) => x > 1.0)"),
        "true"
    );
    assert_eq!(
        main_result("let main = () => [2.0, 3.0] |> List.all((x) => x > 2.0)"),
        "false"
    );
    // Vacuous truth / falsity on the empty list.
    assert_eq!(main_result("let main = () => List.all((x) => x > 0.0, [])"), "true");
    assert_eq!(main_result("let main = () => List.any((x) => x > 0.0, [])"), "false");
    let (message, _, _) = run_err("let main = () => List.any((x) => x, [1.0])");
    assert!(message.contains("must return a bool"), "got: {message}");
}

// The builtins loop in Rust, so folding/iterating a large list never trips
// the eval-depth cap that a hand-rolled recursion hits around n≈60.
#[test]
fn list_builtins_do_not_consume_eval_depth() {
    assert_eq!(
        main_result("let main = () => List.length(List.range(1000.0))"),
        "1000"
    );
    assert_eq!(
        main_result("let main = () => List.isEmpty(List.reverse(List.range(1000.0)))"),
        "false"
    );
    assert_eq!(
        main_result("let main = () => List.range(1000.0) |> List.any((x) => x > 900.0)"),
        "true"
    );
    assert_eq!(
        main_result("let main = () => List.range(1000.0) |> List.all((x) => x < 1000.0)"),
        "true"
    );
}

#[test]
fn math_builtins_evaluate() {
    assert_eq!(main_result("let main = () => Math.sqrt(9.0)"), "3");
    assert_eq!(main_result("let main = () => Math.abs(0.0 - 4.0)"), "4");
    assert_eq!(main_result("let main = () => Math.floor(3.7)"), "3");
    // atan2(y, x): straight up the +Y axis is a quarter turn (pi/2 ≈ 1.5708).
    assert_eq!(
        main_result("let main = () => Math.floor(Math.atan2(1.0, 0.0) * 1000.0)"),
        "1570"
    );
    assert_eq!(main_result("let main = () => Math.min(2.0, 5.0)"), "2");
    assert_eq!(main_result("let main = () => Math.max(2.0, 5.0)"), "5");
    assert_eq!(main_result("let main = () => Math.pow(2.0, 10.0)"), "1024");
    // Math.pi is a constant value (not a callable).
    assert_eq!(
        main_result("let main = () => Math.floor(Math.pi * 100.0)"),
        "314"
    );
}

// Euclidean mod: the result takes the sign of the DIVISOR, so negative inputs
// wrap positively — the wraparound games want.
#[test]
fn math_mod_is_euclidean() {
    assert_eq!(main_result("let main = () => Math.mod(9.0, 8.0)"), "1");
    assert_eq!(main_result("let main = () => Math.mod(0.0 - 1.0, 8.0)"), "7");
    assert_eq!(main_result("let main = () => Math.mod(0.0 - 3.0, 8.0)"), "5");
    // Always non-negative, even with a negative divisor.
    assert_eq!(main_result("let main = () => Math.mod(0.0 - 1.0, 0.0 - 8.0)"), "7");
}

// `Math.pi` is a value, so applying it as a function is a runtime error.
#[test]
fn error_calling_math_pi() {
    let (message, _, _) = run_err("let main = () => Math.pi(1.0)");
    assert_eq!(message, "cannot call a number");
}

// NaN follows f64::max (IEEE maximumNumber): ignored unless all-NaN.
#[test]
fn maximum_ignores_nan_unless_all_nan() {
    assert_eq!(
        main_result("let main = () => List.maximum([0.0 / 0.0, 1.0])"),
        "1"
    );
    assert_eq!(
        main_result("let main = () => List.maximum([0.0 / 0.0])"),
        "NaN"
    );
}

// Currying: a partially-applied callback is no longer a runtime arity error —
// each element maps to a partial. (The old "blames the callback" arity message
// is unreachable now that under-application is legal.)
#[test]
fn partial_callback_maps_to_partials() {
    assert_eq!(
        main_result(
            "let add = (a, b) => a + b\n\
             let main = () => [1.0] |> List.map(add)"
        ),
        "[<partial 1 more>]"
    );
}

#[test]
fn error_main_bound_to_a_builtin() {
    let (message, _, _) = run_err("let main = List.map");
    assert_eq!(message, "`main` must take no parameters to be runnable");
}

// A failing run keeps the trace recorded up to the error — the execution
// story is most valuable exactly then.
#[test]
fn failing_runs_keep_their_partial_trace() {
    let failure = run_failure(
        "let boom = (x) => x + \"s\"\n\
         let main = () => boom(1.0)",
        Tracing::On,
    );
    let rendered = functor_lang::render_trace(&failure.trace);
    assert!(rendered.contains("> main()"), "trace was: {rendered}");
    assert!(rendered.contains("> boom(1)"), "trace was: {rendered}");
}

#[test]
fn main_may_be_a_plain_value() {
    let record = run_src("let main = 41.0 + 1.0", Tracing::Off);
    match record.outcome {
        RunOutcome::Main(Value::Number(n)) => assert_eq!(n, 42.0),
        _ => panic!("expected main value"),
    }
}

#[test]
fn no_main_reports_all_bindings() {
    let record = run_src("let a = 1.0\nlet b = \"hi\"", Tracing::Off);
    match record.outcome {
        RunOutcome::Bindings(bindings) => {
            let rendered: Vec<String> =
                bindings.iter().map(|(n, v)| format!("{n} = {v}")).collect();
            assert_eq!(rendered, ["a = 1", "b = \"hi\""]);
        }
        _ => panic!("expected bindings"),
    }
}

// --- Record updates + local let/mut (see ~/notes mutability.md) ---

#[test]
fn record_update_replaces_fields() {
    assert_eq!(
        main_result("let main = () => { { x: 1.0, y: 2.0 } with x: 9.0 }"),
        "{ x: 9, y: 2 }"
    );
}

#[test]
fn error_update_of_missing_field() {
    let (message, _, _) = run_err("let main = () => { { x: 1.0 } with y: 2.0 }");
    assert_eq!(message, "record has no field `y` to update");
}

#[test]
fn error_update_on_non_record() {
    let (message, _, _) = run_err("let main = () => { 3.0 with x: 1.0 }");
    assert_eq!(message, "`with` update on a number, not a record");
}

#[test]
fn let_in_binds_and_shadows() {
    // The initializer sees the OUTER binding; the body sees the new one.
    assert_eq!(
        main_result("let x = 1.0\nlet main = () => let x = x + 1.0 in let x = x * 10.0 in x"),
        "20"
    );
}

#[test]
fn mut_accumulates_through_assignments() {
    assert_eq!(
        main_result(
            "let sum3 = (a, b, c) =>\n\
             \x20 let mut acc = a in\n\
             \x20 acc := acc + b;\n\
             \x20 acc := acc + c;\n\
             \x20 acc\n\
             let main = () => sum3(10.0, 20.0, 30.0)"
        ),
        "60"
    );
}

#[test]
fn mut_slots_are_fresh_per_activation() {
    // Each callback invocation gets its own slot for the same static binding.
    assert_eq!(
        main_result(
            "let bump = (x) => let mut a = x in a := a + 1.0; a\n\
             let main = () => [1.0, 5.0] |> List.map(bump)"
        ),
        "[2, 6]"
    );
}

#[test]
fn closures_may_capture_immutable_lets() {
    assert_eq!(
        main_result(
            "let f = (x) => let k = x * 2.0 in (y) => k + y\n\
             let main = () => f(2.0)(1.0)"
        ),
        "5"
    );
}

// [Codex review] an invalid update target rejects BEFORE evaluating the
// replacement — with host externals the RHS can have effects.
#[test]
fn update_validates_target_before_evaluating_value() {
    let failure = run_failure(
        "let main = () => { { x: 1.0 } with y: List.maximum([]) }",
        Tracing::On,
    );
    assert_eq!(failure.error.message, "record has no field `y` to update");
    let rendered = functor_lang::render_trace(&failure.trace);
    assert!(
        !rendered.contains("List.maximum"),
        "the replacement must not have run: {rendered}"
    );
}

// --- C2 review pins: Session semantics + new builtins ---

// [AGREED by both engines] loading a module must NOT execute a `main` def —
// a game file may define one for `functor-lang run` debugging.
#[test]
fn session_load_does_not_run_main() {
    let module = functor_lang::lower(
        functor_lang::parse(
            "let init = { n: 1.0 }\n\
             let main = () => 1.0 + \"boom\"",
        )
        .unwrap(),
    )
    .unwrap();
    let session = match functor_lang::Session::load(&module, &mut functor_lang::NoHost) {
        Ok(session) => session,
        Err(failure) => panic!("load must not run main: {}", failure.error.message),
    };
    match session.global("init") {
        Some(Value::Record(_)) => {}
        Some(other) => panic!("expected the init record, got {other}"),
        None => panic!("init missing"),
    }
}

#[test]
fn session_calls_top_level_functions() {
    let module = functor_lang::lower(functor_lang::parse("let tick = (n) => { v: n + 1.0 }").unwrap()).unwrap();
    let session = match functor_lang::Session::load(&module, &mut functor_lang::NoHost) {
        Ok(session) => session,
        Err(failure) => panic!("load failed: {}", failure.error.message),
    };
    let out = match session.call("tick", vec![Value::Number(1.0)], &mut functor_lang::NoHost) {
        Ok(out) => out,
        Err(err) => panic!("call failed: {}", err.message),
    };
    assert_eq!(out.to_string(), "{ v: 2 }");
    let missing = session.call("nope", vec![], &mut functor_lang::NoHost);
    assert!(missing.is_err());
}

// [review High] a non-finite or huge range count is a frame error, not a
// process-killing allocator panic.
#[test]
fn range_rejects_non_finite_and_huge_counts() {
    let (message, _, _) = run_err("let main = () => List.range(1.0 / 0.0)");
    assert_eq!(
        message,
        "List.range needs a finite count up to 1000000, got inf"
    );
    let (message, _, _) = run_err("let main = () => List.range(1000000000000.0)");
    assert!(message.starts_with("List.range needs a finite count"));
}

#[test]
fn new_builtins_evaluate() {
    assert_eq!(main_result("let main = () => List.range(2.7)"), "[0, 1]");
    assert_eq!(main_result("let main = () => List.range(-3.0)"), "[]");
    assert_eq!(main_result("let main = () => Math.sin(0.0)"), "0");
    assert_eq!(main_result("let main = () => Math.cos(0.0)"), "1");
}

/// `Text.fixed(n, decimals)` — the F# `sprintf "%.1f"` shape (HUD text):
/// fixed decimals, rounding, and 0 decimals as the integer (`%d`) shape.
#[test]
fn text_fixed_formats_fixed_decimals() {
    assert_eq!(
        main_result("let main = () => Text.fixed(-5.0, 1.0)"),
        "\"-5.0\""
    );
    assert_eq!(
        main_result("let main = () => Text.fixed(3.14159, 2.0)"),
        "\"3.14\""
    );
    assert_eq!(
        main_result("let main = () => Text.fixed(42.0, 0.0)"),
        "\"42\""
    );
    assert_eq!(
        main_result("let main = () => Text.fixed(0.0, 0.0)"),
        "\"0\""
    );
    // Decimals must be a whole number in a sane range — a fractional or
    // negative count is a teaching error, not a silent truncation.
    let (message, _, _) = run_err("let main = () => Text.fixed(1.0, 1.5)");
    assert_eq!(
        message,
        "Text.fixed needs a whole number of decimals between 0 and 12, got 1.5"
    );
    let (message, _, _) = run_err("let main = () => Text.fixed(1.0, -1.0)");
    assert_eq!(
        message,
        "Text.fixed needs a whole number of decimals between 0 and 12, got -1"
    );
    let (message, _, _) = run_err("let main = () => Text.fixed(\"x\", 1.0)");
    assert_eq!(message, "Text.fixed(n, decimals) expects two numbers");
}

// --- Variants + match (B5 part 1) ---

const SHAPE: &str = "type Shape = | Circle(r: float) | Rect(w: float, h: float) | Point\n";

/// First matching arm wins, top to bottom — a catch-all above a more
/// specific arm shadows it.
#[test]
fn first_matching_arm_wins() {
    assert_eq!(
        main_result("let main = () => match 1.0 with | x => \"first\" | 1.0 => \"second\""),
        "\"first\""
    );
    assert_eq!(
        main_result("let main = () => match 2.0 with | 1.0 => \"a\" | 2.0 => \"b\" | _ => \"c\""),
        "\"b\""
    );
}

#[test]
fn constructor_patterns_bind_positionally() {
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let main = () => match Rect(3.0, 4.0) with | Circle(r) => r | Rect(w, h) => w * h | Point => 0.0"
        )),
        "12"
    );
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let main = () => match Point with | Point => \"origin\" | _ => \"elsewhere\""
        )),
        "\"origin\""
    );
}

/// A leading `-` folds into a number-literal pattern. [Claude L — B5 review]
#[test]
fn negative_number_literal_pattern() {
    assert_eq!(
        main_result("let main = () => match 0.0 - 1.0 with | -1.0 => \"neg\" | _ => \"other\""),
        "\"neg\""
    );
    assert_eq!(
        main_result("let main = () => match 1.0 with | -1.0 => \"neg\" | _ => \"other\""),
        "\"other\""
    );
}

/// No arm matching is a spanned runtime error naming the value.
#[test]
fn error_no_pattern_matched() {
    let (message, line, col) = run_err(&format!(
        "{SHAPE}let main = () => match Circle(2.0) with | Point => 0.0"
    ));
    assert_eq!(message, "no pattern matched Circle(2)");
    assert_eq!((line, col), (2, 18));
}

/// Variants display as `Ctor(args…)` / bare `Ctor`, and equality is
/// structural: same constructor, equal args.
#[test]
fn variant_display_and_structural_equality() {
    assert_eq!(
        main_result(&format!("{SHAPE}let main = () => [Circle(2.0), Point]")),
        "[Circle(2), Point]"
    );
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let main = () => Circle(2.0) == Circle(1.0 + 1.0)"
        )),
        "true"
    );
    assert_eq!(
        main_result(&format!("{SHAPE}let main = () => Circle(2.0) == Point")),
        "false"
    );
    // Different kinds are simply unequal, like everywhere else.
    assert_eq!(
        main_result(&format!("{SHAPE}let main = () => Circle(2.0) == 2.0")),
        "false"
    );
}

/// A function argument inside a variant hits the same function-comparison
/// error as everywhere else. (Constructors check arity at runtime, not
/// field types — the checker owns those.)
#[test]
fn error_variant_equality_over_functions() {
    let (message, _, _) = run_err(&format!(
        "{SHAPE}let main = () => Circle((x) => x) == Circle((x) => x)"
    ));
    assert_eq!(message, "functions cannot be compared with `==`");
}

/// An unapplied constructor is a function value with no equality.
#[test]
fn error_comparing_unapplied_ctors() {
    let (message, _, _) = run_err(&format!("{SHAPE}let main = () => Circle == Circle"));
    assert_eq!(message, "functions cannot be compared with `==`");
}

// Currying: over-applying a saturated constructor is still an error (the
// resulting variant isn't callable); under-applying it (here, zero args) is a
// legal partial rather than an arity error.
#[test]
fn ctor_over_application_at_runtime_errors() {
    let (message, _, _) = run_err(&format!("{SHAPE}let main = () => Circle(1.0, 2.0)"));
    assert_eq!(message, "cannot call a variant");
    assert_eq!(
        main_result(&format!("{SHAPE}let main = () => Circle()")),
        "<partial 1 more>"
    );
}

/// A parameterful constructor is first-class: it pipes through the builtins
/// like any function.
#[test]
fn ctor_as_a_function_argument() {
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let main = () => [1.0, 2.0] |> List.map(Circle)"
        )),
        "[Circle(1), Circle(2)]"
    );
}

/// A nullary constructor used bare IS the value — calling it is calling a
/// variant, not a function.
#[test]
fn error_calling_a_nullary_ctor() {
    let (message, _, _) = run_err(&format!("{SHAPE}let main = () => Point()"));
    assert_eq!(message, "cannot call a variant");
}

/// bool-literal matches are the language's first conditional.
#[test]
fn bool_match_is_a_conditional() {
    assert_eq!(
        main_result("let main = () => match 4.0 > 3.0 with | true => \"yes\" | false => \"no\""),
        "\"yes\""
    );
}

/// Lambdas may capture pattern variables (plain immutable bindings).
#[test]
fn closures_capture_pattern_vars() {
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let f = (s) => match s with | Circle(r) => (x) => r + x | _ => (x) => x\n\
             let main = () => f(Circle(2.0))(3.0)"
        )),
        "5"
    );
}

// --- Tuples ---

/// Arity mismatch is a non-match (like ctors), not an error — and equality
/// is structural with arity difference simply unequal.
#[test]
fn tuple_match_and_equality_semantics() {
    assert_eq!(
        main_result(
            "let main = () => match (1.0, 2.0) with | (a, b, c) => a | (a, b) => a + b | _ => 0.0"
        ),
        "3"
    );
    assert_eq!(
        main_result("let main = () => (1.0, 2.0) == (1.0, 2.0, 3.0)"),
        "false"
    );
    assert_eq!(
        main_result("let main = () => ((1.0, \"x\"), true) == ((1.0, \"x\"), true)"),
        "true"
    );
}

/// `(e)` stays grouping; a trailing comma is allowed in real tuples.
#[test]
fn parens_are_grouping_not_one_tuples() {
    assert_eq!(main_result("let main = () => (1.0 + 2.0) * 2.0"), "6");
    assert_eq!(main_result("let main = () => (1.0, 2.0,)"), "(1, 2)");
}

/// The destructuring let is exactly a single-arm match: a wrong-arity value
/// is a spanned "no pattern matched" runtime error.
#[test]
fn destructuring_let_arity_mismatch_fails_loud() {
    let (message, _, _) =
        run_err("let f = (t) => let (a, b) = t in a + b\nlet main = () => f((1.0, 2.0, 3.0))");
    assert_eq!(message, "no pattern matched (1, 2, 3)");
}

/// Generic ADTs at runtime: the checker's parameters are erased — one Box
/// works at every type (the runtime was already untyped; this pins that
/// generics stayed checker-only).
#[test]
fn generic_adts_run_type_erased() {
    assert_eq!(
        main_result(
            "type Box<'v> = | Full(value: 'v) | Empty\n\
             let orElse = (b, d) => match b with | Full(v) => v | Empty => d\n\
             let main = () => (orElse(Full(1.0), 0.0), orElse(Full(\"s\"), \"\"), orElse(Empty, 9.0))"
        ),
        "(1, \"s\", 9)"
    );
}

// --- List patterns + cons ---

/// Cons builds a list by prepending; list patterns destructure by length,
/// and `[h, ..t]` binds the remainder.
#[test]
fn list_cons_and_patterns() {
    assert_eq!(
        main_result("let main = () => [0.0, ..[1.0, 2.0]]"),
        "[0, 1, 2]"
    );
    assert_eq!(
        main_result(
            "let head = (xs) => match xs with | [] => 0.0 | [h, ..t] => h\n\
             let main = () => (head([9.0, 8.0]), head([]))"
        ),
        "(9, 0)"
    );
    // Exact-length match: [a, b] matches only a 2-list.
    assert_eq!(
        main_result(
            "let f = (xs) => match xs with | [a, b] => a + b | _ => 0.0\n\
             let main = () => (f([3.0, 4.0]), f([3.0]), f([3.0, 4.0, 5.0]))"
        ),
        "(7, 0, 0)"
    );
    // The tail binds a list; `[..all]` matches anything.
    assert_eq!(
        main_result(
            "let rest = (xs) => match xs with | [_, ..t] => t | [] => []\n\
             let main = () => rest([1.0, 2.0, 3.0])"
        ),
        "[2, 3]"
    );
}

/// `..` spreads a list; a non-list tail is a spanned runtime error.
#[test]
fn cons_tail_must_be_a_list() {
    let (message, _, _) = run_err("let main = () => [1.0, ..2.0]");
    assert_eq!(message, "`..` spreads a list, but the tail is a number");
}

/// `Debug.log(value, label)` is an Elm-style trace: it emits `label: <value>`
/// through the process-wide sink AND returns the value UNCHANGED (so it is
/// transparent to the program result and can't affect the model/sim).
#[test]
fn debug_log_returns_the_value_unchanged_and_emits_through_the_sink() {
    use std::sync::{Arc, Mutex};

    // A capturing sink stands in for the real (host) region-aware one.
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_buf = Arc::clone(&captured);
    functor_lang::set_trace_sink(Box::new(move |m| sink_buf.lock().unwrap().push(m)));

    // Label-first and pipe-friendly: the subject (last arg) is logged and passed
    // on, so the arithmetic result is exactly what it would be without the trace.
    assert_eq!(
        main_result("let main = () => Debug.log(\"answer\", 41.0) + 1.0"),
        "42"
    );
    // Works through a pipeline too (`x |> Debug.log(label)` == the value).
    assert_eq!(
        main_result("let main = () => 3.0 |> Debug.log(\"piped\") |> Math.clamp01"),
        "1"
    );
    // Any value type renders with the interpreter's own display (records here).
    assert_eq!(
        main_result(
            "let main = () =>\n\
             let r = Debug.log(\"pt\", { x: 1.0, y: 2.0 }) in r.x"
        ),
        "1"
    );

    let lines = captured.lock().unwrap();
    assert_eq!(
        *lines,
        vec![
            "answer: 41".to_string(),
            "piped: 3".to_string(),
            "pt: { x: 1, y: 2 }".to_string(),
        ]
    );
}

// --- Currying / partial application (migration step 1) --------------------
// The interpreter curries call sites: under-application yields a partial,
// exact application dispatches as before, and over-application saturates then
// applies the remainder to the result. Piping is thread-LAST (the pipe APPENDS
// its subject as the final argument — migration step 3).

/// A partial captured in a `let ... in` binding, then saturated.
#[test]
fn partial_then_saturate() {
    assert_eq!(
        main_result(
            "let add = (a, b) => a + b\n\
             let main = () => let inc = add(1.0) in inc(10.0)"
        ),
        "11"
    );
    // Same, but the partial lives in a top-level binding across defs.
    assert_eq!(
        main_result(
            "let add = (a, b) => a + b\n\
             let inc = add(1.0)\n\
             let main = () => inc(41.0)"
        ),
        "42"
    );
}

/// Over-application: saturate with the callee's arity, then apply the leftover
/// args to the resulting function.
#[test]
fn over_application_applies_remainder_to_result() {
    assert_eq!(
        main_result(
            "let adder = (a) => (b) => a + b\n\
             let main = () => adder(3.0, 4.0)"
        ),
        "7"
    );
}

/// A partially-applied constructor is a first-class value; saturating it
/// builds the variant.
#[test]
fn constructor_partial_then_saturate() {
    assert_eq!(
        main_result(&format!(
            "{SHAPE}let mkTall = Rect(2.0)\n\
             let main = () => mkTall(5.0)"
        )),
        "Rect(2, 5)"
    );
}

/// A partial passed as a value flows through a builtin like any function —
/// each element saturates it.
#[test]
fn partial_passed_as_a_value() {
    assert_eq!(
        main_result(
            "let add = (a, b) => a + b\n\
             let main = () => [1.0, 2.0, 3.0] |> List.map(add(10.0))"
        ),
        "[11, 12, 13]"
    );
}

/// Thread-LAST: `xs |> List.map(f)` lowers to the subject-LAST call
/// `List.map(f, xs)` (the piped subject is APPENDED as the final argument), so
/// a piped form and the equivalent direct call agree.
#[test]
fn thread_last_pipe_appends() {
    assert_eq!(
        main_result(
            "let double = (x) => x * 2.0\n\
             let main = () => [1.0, 2.0, 3.0] |> List.map(double)"
        ),
        "[2, 4, 6]"
    );
    // The un-piped subject-LAST form is identical.
    assert_eq!(
        main_result(
            "let double = (x) => x * 2.0\n\
             let main = () => List.map(double, [1.0, 2.0, 3.0])"
        ),
        "[2, 4, 6]"
    );
}

// --- Boolean operators `&&` / `||` / `not` ---

#[test]
fn bool_operators_evaluate() {
    assert_eq!(main_result("let main = () => true && false"), "false");
    assert_eq!(main_result("let main = () => true && true"), "true");
    assert_eq!(main_result("let main = () => false || true"), "true");
    assert_eq!(main_result("let main = () => false || false"), "false");
    assert_eq!(main_result("let main = () => not true"), "false");
    assert_eq!(main_result("let main = () => not false"), "true");
}

#[test]
fn bool_operator_precedence() {
    // `&&` binds tighter than `||`: `false || true && false` is
    // `false || (true && false)` == false.
    assert_eq!(
        main_result("let main = () => false || true && false"),
        "false"
    );
    // Both are looser than comparison: `3.0 > 2.0 && 2.0 > 1.0` is
    // `(3.0 > 2.0) && (2.0 > 1.0)` == true.
    assert_eq!(
        main_result("let main = () => 3.0 > 2.0 && 2.0 > 1.0"),
        "true"
    );
    // `not` is looser than comparison: `not 1.0 == 2.0` is `not (1.0 == 2.0)`.
    assert_eq!(main_result("let main = () => not 1.0 == 2.0"), "true");
}

#[test]
fn logical_and_short_circuits() {
    // `boom(0.0)` would fail at runtime (no matching arm); `false && _`
    // must not evaluate it.
    assert_eq!(
        main_result(
            "let boom = (x) => match x with | 99.0 => true\n\
             let main = () => false && boom(0.0)"
        ),
        "false"
    );
}

#[test]
fn logical_or_short_circuits() {
    assert_eq!(
        main_result(
            "let boom = (x) => match x with | 99.0 => true\n\
             let main = () => true || boom(0.0)"
        ),
        "true"
    );
}

#[test]
fn logical_and_evaluates_rhs_when_needed() {
    // The complement: `true && _` DOES evaluate the right side, so the
    // failing call surfaces.
    let (message, _, _) = run_err(
        "let boom = (x) => match x with | 99.0 => true\n\
         let main = () => true && boom(0.0)",
    );
    assert_eq!(message, "no pattern matched 0");
}

#[test]
fn long_logical_chain_does_not_overflow() {
    // Left-assoc `&&`/`||` chains parse iteratively (no depth guard), so eval
    // must walk their spine iteratively too — a flat 2000-term chain must not
    // consume host stack per term. `true && … && true && false` == false.
    let chain = std::iter::repeat("true")
        .take(2000)
        .collect::<Vec<_>>()
        .join(" && ");
    assert_eq!(
        main_result(&format!("let main = () => {chain} && false")),
        "false"
    );
    let ors = std::iter::repeat("false")
        .take(2000)
        .collect::<Vec<_>>()
        .join(" || ");
    assert_eq!(
        main_result(&format!("let main = () => {ors} || true")),
        "true"
    );
}

#[test]
fn bool_operator_on_non_bool_errors() {
    // A non-bool operand is a runtime error (the checker also rejects it,
    // but `run` does not typecheck).
    let (message, _, _) = run_err("let main = () => not 3.0");
    assert_eq!(message, "boolean operator needs a bool, got a number");
}
