//! B3 verification (docs/mle.md): run/trace goldens per example plus
//! interpreter-semantics and runtime-error assertions.

use mle::value::Value;
use mle::{RunOutcome, Tracing};
use std::fs;
use std::path::Path;

/// Parse + lower + run `src`, panicking with a rendered position on failure.
fn run_src(src: &str, tracing: Tracing) -> mle::RunRecord {
    let program = mle::parse(src).expect("source should parse");
    let module = mle::lower(program).expect("source should lower");
    match mle::run(&module, tracing) {
        Ok(record) => record,
        Err(failure) => {
            let (line, col) = mle::line_col(src, failure.error.span.start);
            panic!("{line}:{col}: error: {}", failure.error.message);
        }
    }
}

/// Run `src` expecting a runtime failure.
fn run_failure(src: &str, tracing: Tracing) -> mle::RunFailure {
    let program = mle::parse(src).expect("source should parse");
    let module = mle::lower(program).expect("source should lower");
    match mle::run(&module, tracing) {
        Err(failure) => failure,
        Ok(_) => panic!("source should fail at runtime"),
    }
}

/// Run `src` expecting a runtime error; returns (message, line, col).
fn run_err(src: &str) -> (String, usize, usize) {
    let failure = run_failure(src, Tracing::Off);
    let (line, col) = mle::line_col(src, failure.error.span.start);
    (failure.error.message, line, col)
}

/// `main`'s printed result for `src`.
fn main_result(src: &str) -> String {
    match run_src(src, Tracing::Off).outcome {
        RunOutcome::Main(value) => value.to_string(),
        RunOutcome::Bindings(_) => panic!("expected a main result"),
    }
}

/// The `mle run` / `mle trace` output for `examples/{name}.mle`, compared
/// against the committed `{name}.run` / `{name}.trace` golden.
/// Regenerate with `UPDATE_GOLDENS=1 cargo test -p mle`.
fn check_golden(name: &str, extension: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = fs::read_to_string(dir.join(format!("{name}.mle"))).unwrap();
    let golden_path = dir.join(format!("{name}.{extension}"));
    let tracing = if extension == "trace" {
        Tracing::On
    } else {
        Tracing::Off
    };
    let record = run_src(&src, tracing);
    let actual = if extension == "trace" {
        mle::render_trace(&record.trace)
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
            "missing golden {} — generate with UPDATE_GOLDENS=1 cargo test -p mle",
            golden_path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "output for {name}.mle diverged from {name}.{extension} — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p mle"
    );
}

#[test]
fn golden_run_pure_pipeline() {
    check_golden("pure-pipeline", "run");
}

#[test]
fn golden_run_records() {
    check_golden("records", "run");
}

#[test]
fn golden_run_functions() {
    check_golden("functions", "run");
}

// One trace golden pins the full enter/exit format; the other examples'
// traces exercise no additional formatting.
#[test]
fn golden_trace_pure_pipeline() {
    check_golden("pure-pipeline", "trace");
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

#[test]
fn error_arity_mismatch() {
    let (message, _, _) = run_err(
        "let f = (a, b) => a + b\n\
         let main = () => f(1.0)",
    );
    assert_eq!(message, "`f` takes 2 argument(s), got 1");
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
fn error_infinite_recursion_is_a_clean_error() {
    let (message, _, _) = run_err(
        "let spin = (n) => spin(n + 1.0)\n\
         let main = () => spin(0.0)",
    );
    assert_eq!(
        message,
        "evaluation nested too deeply (deep recursion, or deeply nested expressions)"
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

#[test]
fn arity_error_blames_the_callback_not_the_builtin() {
    let (message, _, _) = run_err(
        "let add = (a, b) => a + b\n\
         let main = () => [1.0] |> List.map(add)",
    );
    assert_eq!(
        message,
        "the function passed to List.map takes 2 argument(s), got 1"
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
    let rendered = mle::render_trace(&failure.trace);
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
    let rendered = mle::render_trace(&failure.trace);
    assert!(
        !rendered.contains("List.maximum"),
        "the replacement must not have run: {rendered}"
    );
}

// --- C2 review pins: Session semantics + new builtins ---

// [AGREED by both engines] loading a module must NOT execute a `main` def —
// a game file may define one for `mle run` debugging.
#[test]
fn session_load_does_not_run_main() {
    let module = mle::lower(
        mle::parse(
            "let init = { n: 1.0 }\n\
             let main = () => 1.0 + \"boom\"",
        )
        .unwrap(),
    )
    .unwrap();
    let session = match mle::Session::load(&module, &mut mle::NoHost) {
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
    let module = mle::lower(mle::parse("let tick = (n) => { v: n + 1.0 }").unwrap()).unwrap();
    let session = match mle::Session::load(&module, &mut mle::NoHost) {
        Ok(session) => session,
        Err(failure) => panic!("load failed: {}", failure.error.message),
    };
    let out = match session.call("tick", vec![Value::Number(1.0)], &mut mle::NoHost) {
        Ok(out) => out,
        Err(err) => panic!("call failed: {}", err.message),
    };
    assert_eq!(out.to_string(), "{ v: 2 }");
    let missing = session.call("nope", vec![], &mut mle::NoHost);
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
