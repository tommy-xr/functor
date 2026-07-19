//! `expect` inline tests: parsing (contextual keyword), checking (must be
//! bool), evaluation (`run_expects` — pass / decomposed comparison failure /
//! runtime error, independence between expects), and the load-inertness
//! guarantee (`run` / `Session::load` never evaluate them).

use functor_lang::{ExpectOutcome, NoHost, Tracing};

fn lower(src: &str) -> functor_lang::ir::Module {
    let program = functor_lang::parse(src).expect("source should parse");
    functor_lang::lower(program).expect("source should lower")
}

fn reports(src: &str) -> Vec<functor_lang::ExpectReport> {
    functor_lang::run_expects(&lower(src), &mut NoHost)
        .unwrap_or_else(|failure| panic!("defs should load: {}", failure.error.message))
}

// ---------------------------------------------------------------- parsing

#[test]
fn expect_parses_at_top_level() {
    let module = lower("let x = 2.0\nexpect x == 2.0\n");
    assert_eq!(module.expects.len(), 1);
    assert_eq!(module.defs.len(), 1);
}

#[test]
fn expect_stays_a_usable_name() {
    // Contextual keyword: binding and using `expect` as an ordinary name
    // still parses (the `open` rule).
    let module = lower("let expect = 2.0\nlet f = (expect) => expect + 1.0\n");
    assert_eq!(module.defs.len(), 2);
    assert!(module.expects.is_empty());
}

#[test]
fn expect_is_rejected_in_interface_files() {
    let err = functor_lang::parse_interface("expect 1.0 == 1.0\n")
        .expect_err("interface expect should be rejected");
    assert!(
        err.message.contains("`expect` tests belong"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn expect_body_may_be_a_let_in_block() {
    let ok = reports("let double = (x) => x * 2.0\nexpect (\n  let y = double(3.0) in\n  y == 6.0\n)\n");
    assert!(matches!(ok[0].outcome, ExpectOutcome::Pass));
}

// --------------------------------------------------------------- checking

#[test]
fn non_bool_expect_is_a_check_error() {
    let module = lower("expect 1.0 + 1.0\n");
    let diags = functor_lang::check(&module);
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0]
            .message
            .contains("an `expect` test: expected bool, got float"),
        "unexpected message: {}",
        diags[0].message
    );
}

#[test]
fn bool_expect_checks_clean() {
    let module = lower("let x = 1.0\nexpect x > 0.0\nexpect x == 1.0\n");
    assert!(functor_lang::check(&module).is_empty());
}

// ------------------------------------------------------------- evaluation

#[test]
fn passing_and_failing_expects_report_independently() {
    let out = reports("let x = 2.0\nexpect x == 2.0\nexpect x == 3.0\nexpect x > 1.0\n");
    assert_eq!(out.len(), 3);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
    assert!(matches!(out[1].outcome, ExpectOutcome::Fail(Some(_))));
    assert!(matches!(out[2].outcome, ExpectOutcome::Pass));
}

#[test]
fn failed_comparison_decomposes_both_sides() {
    let out = reports("let area = (w, h) => w * h\nexpect area(3.0, 4.0) == 12.5\n");
    let ExpectOutcome::Fail(Some(cmp)) = &out[0].outcome else {
        panic!("expected a decomposed failure, got {:?}", out[0].outcome);
    };
    assert_eq!(cmp.op, "==");
    assert_eq!(cmp.lhs, "12");
    assert_eq!(cmp.rhs, "12.5");
}

#[test]
fn failed_bare_bool_has_no_decomposition() {
    let out = reports("expect false\n");
    assert!(matches!(out[0].outcome, ExpectOutcome::Fail(None)));
}

#[test]
fn structural_equality_covers_composite_values() {
    let out = reports(
        "let double = (x) => x * 2.0\nexpect ([1.0, 2.0] |> List.map(double)) == [2.0, 4.0]\n",
    );
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn erroring_expect_reports_and_the_rest_still_run() {
    // Comparing functions is a runtime error (the one structural-== error).
    let out = reports("let f = (x) => x\nexpect f == f\nexpect 1.0 == 1.0\n");
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
    assert!(matches!(out[1].outcome, ExpectOutcome::Pass));
}

#[test]
fn non_bool_expect_is_a_runtime_error_when_unchecked() {
    let out = reports("expect 1.0 + 1.0\n");
    let ExpectOutcome::Error(err) = &out[0].outcome else {
        panic!("expected an error outcome, got {:?}", out[0].outcome);
    };
    assert!(
        err.message.contains("must evaluate to a bool"),
        "unexpected message: {}",
        err.message
    );
}

// -------------------------------------------------------------- multi-file

#[test]
fn sibling_module_expects_run_with_the_project() {
    let dir = std::env::temp_dir().join(format!(
        "functor-lang-expects-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(
        dir.join("game.fun"),
        "let scale = 2.0\nexpect Utils.area(3.0, 4.0) * scale == 24.0\n",
    )
    .expect("write entry");
    std::fs::write(
        dir.join("utils.fun"),
        "let area = (w, h) => w * h\nexpect area(2.0, 2.0) == 4.0\n",
    )
    .expect("write sibling");
    let project = functor_lang::project::load(&dir.join("game.fun"))
        .unwrap_or_else(|err| panic!("project loads: {}", err.message));
    assert!(project.check().is_empty(), "project should check clean");
    let out = functor_lang::run_expects(&project.module, &mut NoHost)
        .unwrap_or_else(|failure| panic!("defs should load: {}", failure.error.message));
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| matches!(r.outcome, ExpectOutcome::Pass)));
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------- step budget

fn budgeted(src: &str, budget: u64) -> Vec<functor_lang::ExpectReport> {
    functor_lang::run_expects_budgeted(&lower(src), &mut NoHost, Some(budget))
        .unwrap_or_else(|failure| panic!("defs should load: {}", failure.error.message))
}

#[test]
fn runaway_expect_exceeds_the_budget() {
    let src = "let sum = (n) => List.range(n) |> List.fold((acc, x) => acc + x, 0.0)\n\
               expect sum(10000.0) == 0.0\n";
    let out = budgeted(src, 1_000);
    let ExpectOutcome::Error(err) = &out[0].outcome else {
        panic!("expected a budget error");
    };
    assert!(
        err.message.contains("step budget (1000 steps)"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn list_range_allocation_is_charged() {
    // No per-element evals happen in List.range — the bulk charge must
    // trip the budget anyway.
    let out = budgeted("expect List.length(List.range(1000000.0)) > 0.0\n", 10_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
}

#[test]
fn budget_resets_per_expect() {
    // Each expect fits the budget alone; together they'd exceed it. All
    // must pass — the budget is per-expect, not per-run.
    let src = "let sum = (n) => List.range(n) |> List.fold((acc, x) => acc + x, 0.0)\n\
               expect sum(100.0) == 4950.0\n\
               expect sum(100.0) == 4950.0\n\
               expect sum(100.0) == 4950.0\n";
    let out = budgeted(src, 3_000);
    assert!(out.iter().all(|r| matches!(r.outcome, ExpectOutcome::Pass)));
}

#[test]
fn a_budget_error_never_stops_the_remaining_expects() {
    let src = "let sum = (n) => List.range(n) |> List.fold((acc, x) => acc + x, 0.0)\n\
               expect sum(10000.0) == 0.0\n\
               expect 1.0 == 1.0\n";
    let out = budgeted(src, 500);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
    assert!(matches!(out[1].outcome, ExpectOutcome::Pass));
}

#[test]
fn call_charging_trips_without_any_range_in_the_expect() {
    // The def load's small range fits the budget; the expect's nested folds
    // are pure closure CALLS (no bulk allocation) and must trip it — pinning
    // that calls, not just List.range, are charged.
    let src = "let xs = List.range(10.0)\n\
               expect (xs |> List.fold((a, x) => a + (xs |> List.fold((b, y) => b + y, 0.0)), 0.0)) == 450.0\n";
    let out = budgeted(src, 60);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
    // Sanity: the same program passes unbudgeted.
    let ok = reports(src);
    assert!(matches!(ok[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn append_doubling_cannot_amplify_past_the_budget() {
    // The review probe: each d(x) doubles the list for O(1) calls, so a
    // per-call-only budget would permit exponential work (26 nestings =
    // 134M elements, seconds of wall-clock). The output-size charge on
    // List.append must trip the budget instead — and fast.
    // 13 doublings of a 2-element list: cumulative append output charges
    // sum to ~32k, well past the 10k budget, while the CALL count stays a
    // trivial ~15 — only the output-size charge can trip this.
    let src = "let d = (x) => List.append(x, x)\n\
               expect (\n\
                 let a = d(d(d(d(List.range(2.0))))) in\n\
                 let b = d(d(d(d(a)))) in\n\
                 let c = d(d(d(d(d(b))))) in\n\
                 List.length(c) == 0.0\n\
               )\n";
    let out = budgeted(src, 10_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
}

#[test]
fn text_join_doubling_cannot_amplify_past_the_budget() {
    // The string flavor of the same amplifier: join("", [s, s]) doubles s.
    let src = "let d = (s) => Text.join(\"\", [s, s])\n\
               expect d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(d(\"ab\"\
)))))))))))))))))))))))))) == \"\"\n";
    let out = budgeted(src, 10_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
}

#[test]
fn structural_equality_is_charged_by_size() {
    // A single `==` on a large value is O(size) work no call charge sees:
    // comparing a 500-element list 50 times is ~25k comparison nodes on a
    // trivial call count, so only the equality charge can trip this.
    let src = "let xs = List.range(500.0)\n\
               let f = (x) => if xs == xs then 1.0 else 0.0\n\
               expect (List.range(50.0) |> List.map(f) |> List.length) == 0.0\n";
    let out = budgeted(src, 5_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));
}

#[test]
fn def_load_is_budgeted_too() {
    let src = "let table = List.range(100000.0)\nexpect 1.0 == 1.0\n";
    let failure = functor_lang::run_expects_budgeted(&lower(src), &mut NoHost, Some(1_000))
        .err()
        .expect("def load should exceed the budget");
    assert!(
        failure.error.message.contains("step budget"),
        "unexpected message: {}",
        failure.error.message
    );
}

#[test]
fn unbudgeted_run_expects_is_unbounded() {
    let src = "let sum = (n) => List.range(n) |> List.fold((acc, x) => acc + x, 0.0)\n\
               expect sum(10000.0) == 49995000.0\n";
    let out = reports(src);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

// ---------------------------------------------------- inert in the game loop

#[test]
fn run_never_evaluates_expects() {
    // The expect would error at runtime (no arm matches 1); `run` must not
    // notice — expects are test-tooling-only.
    let src = "type Shape = | Circle(radius: float)\n\
               let area = (s: Shape) => match s with | Circle(r) => r\n\
               expect area(1.0) == 1.0\n\
               let main = () => 42.0\n";
    let record = functor_lang::run(&lower(src), Tracing::Off)
        .unwrap_or_else(|failure| panic!("run should ignore expects: {}", failure.error.message));
    match record.outcome {
        functor_lang::RunOutcome::Main(value) => assert_eq!(value.to_string(), "42"),
        functor_lang::RunOutcome::Bindings(_) => panic!("expected a main outcome"),
    }
}

#[test]
fn session_load_never_evaluates_expects() {
    let src = "let x = 1.0\nexpect x == 2.0\n";
    let session = functor_lang::Session::load(&lower(src), &mut NoHost)
        .unwrap_or_else(|failure| panic!("load should ignore expects: {}", failure.error.message));
    assert!(session.global("x").is_some());
}
