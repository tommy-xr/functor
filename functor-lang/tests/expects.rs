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
fn immutable_map_updates_charge_the_written_output() {
    // Four inserts into a one-leaf map write leaves of 1+2+3+4 entries. With
    // their logarithmic searches this must exceed a budget that a
    // per-builtin-call-only implementation would fit.
    let src = "expect Map.member(4.0,\n\
               Map.insert(4.0, 4.0,\n\
               Map.insert(3.0, 3.0,\n\
               Map.insert(2.0, 2.0,\n\
               Map.insert(1.0, 1.0, Map.empty())))))\n";
    let out = budgeted(src, 15);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));

    // A realistic budget still completes the same operation.
    let out = budgeted(src, 100);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn map_bulk_construction_and_iteration_charge_their_entries() {
    // The three top-level lists cost 19 container-construction steps and fit
    // the 20-step load budget. The expect then gets a fresh budget: a
    // per-call-only implementation would fit its three Map calls, but
    // fromList's input/sort and values/toList's materialized entries cannot.
    let src = "let pairs = [(8.0, 8.0), (7.0, 7.0), (6.0, 6.0), (5.0, 5.0), \
                           (4.0, 4.0), (3.0, 3.0), (2.0, 2.0), (1.0, 1.0)]\n\
               let expected = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (4.0, 4.0), \
                               (5.0, 5.0), (6.0, 6.0), (7.0, 7.0), (8.0, 8.0)]\n\
               let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]\n\
               expect (\n\
                 let map = Map.fromList(pairs) in\n\
                 (Map.values(map), Map.toList(map)) == (values, expected)\n\
               )\n";
    let out = budgeted(src, 20);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));

    let out = budgeted(src, 200);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn map_searches_charge_key_comparisons() {
    // Build without fromList so its sort preflight is not what this test
    // measures. Repeated binary searches would fit under call-only
    // accounting, but their key comparisons must exhaust the fresh expect
    // budget.
    let src = "let map = Map.empty()\n\
                 |> Map.insert(1.0, true) |> Map.insert(2.0, true)\n\
                 |> Map.insert(3.0, true) |> Map.insert(4.0, true)\n\
                 |> Map.insert(5.0, true) |> Map.insert(6.0, true)\n\
                 |> Map.insert(7.0, true) |> Map.insert(8.0, true)\n\
               expect List.all((_) => Map.member(8.0, map), List.range(32.0))\n";
    let out = budgeted(src, 100);
    assert!(matches!(out[0].outcome, ExpectOutcome::Error(_)));

    let out = budgeted(src, 500);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn map_string_searches_charge_compared_bytes() {
    // The map is built during def loading without comparing keys. The
    // expect's lookup compares two strings whose common prefix is long:
    // charging merely one step per comparison would let this fit.
    let prefix = "a".repeat(4096);
    let src = format!(
        "let map = Map.insert(\"{prefix}x\", true, Map.empty())\n\
         expect Map.member(\"{prefix}y\", map) == false\n"
    );
    let out = budgeted(&src, 100);
    let ExpectOutcome::Error(err) = &out[0].outcome else {
        panic!("expected the long string-key comparison to exceed the budget");
    };
    assert!(err.message.contains("step budget"), "unexpected: {err:?}");

    let out = budgeted(&src, 10_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

#[test]
fn map_from_list_preflights_string_comparison_bytes() {
    // Sorting and duplicate folding both compare keys. Preflight their
    // conservative byte-work bound before the first potentially long
    // comparison, then charge the actual work only once on success.
    let prefix = "a".repeat(4096);
    let src = format!(
        "expect Map.values(Map.fromList(\
         [(\"{prefix}y\", 2.0), (\"{prefix}x\", 1.0)])) == [1.0, 2.0]\n"
    );
    let out = budgeted(&src, 100);
    let ExpectOutcome::Error(err) = &out[0].outcome else {
        panic!("expected Map.fromList preflight to exceed the budget");
    };
    assert!(err.message.contains("step budget"), "unexpected: {err:?}");

    let out = budgeted(&src, 30_000);
    assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
}

/// The smallest budget under which `expect` passes for `value`. Fuel is
/// deterministic, so this is the exact cost of building `value`. The
/// trailing pad outweighs every preflight ceiling here (the 6.8k sort's is
/// ~180k), so the binding bound is always the actual charge.
fn min_budget(defs: &str, value: &str) -> u64 {
    let src =
        format!("{defs}\nexpect (let m = {value} in let pad = List.range(250000.0) in true)\n");
    let passes = |budget| matches!(budgeted(&src, budget)[0].outcome, ExpectOutcome::Pass);
    let (mut lo, mut hi) = (0u64, 10_000_000u64);
    assert!(passes(hi), "{value} should fit the largest budget");
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if passes(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Fuel charged by applying `op` (a pipeline step) to `setup`'s value.
fn op_cost(defs: &str, setup: &str, op: &str) -> u64 {
    let cost = min_budget(defs, &format!("({setup}) |> {op}")) - min_budget(defs, setup);
    // Zero would mean a setup preflight, not the op, set both budgets.
    assert!(cost > 0, "{op} measured nothing");
    cost
}

const MAP_DEFS: &str = "let sorted = (n) => List.range(n) |> List.map((i) => (i, i))";

/// Per-call overhead of a Map builtin: replacing the value in a one-entry
/// map does 1 comparison and writes 1 entry.
fn map_call_overhead() -> u64 {
    op_cost(
        MAP_DEFS,
        "Map.insert(0.0, 0.0, Map.empty())",
        "Map.insert(0.0, 1.0)",
    ) - 2
}

#[test]
fn map_updates_charge_comparisons_plus_the_nodes_they_write() {
    let call = map_call_overhead();
    // A full 16-entry leaf takes a 17th key: 4 comparisons, then the
    // split writes both halves (17) and a new two-child root (2).
    let split = op_cost(
        MAP_DEFS,
        "Map.fromList(sorted(16.0))",
        "Map.insert(16.0, 0.0)",
    );
    assert_eq!(split, call + 4 + 17 + 2);
    // Leaves 9 + 8; removing 16 underfills the right leaf, which borrows
    // from the left: 4 comparisons; donor 8 + borrower 8 + root 2.
    let borrow = op_cost(MAP_DEFS, "Map.fromList(sorted(17.0))", "Map.remove(16.0)");
    assert_eq!(borrow, call + 4 + 8 + 8 + 2);
    // Leaves 8 + 8; removing 16 merges them: 4 comparisons; the merged
    // leaf (15) plus the one-child root (1) that then collapses.
    let merge = op_cost(
        MAP_DEFS,
        "Map.fromList(sorted(17.0)) |> Map.remove(0.0)",
        "Map.remove(16.0)",
    );
    assert_eq!(merge, call + 4 + 15 + 1);
}

#[test]
fn map_bulk_build_charges_entries_sort_and_internal_nodes() {
    let call = map_call_overhead();
    for n in [16usize, 17, 300] {
        // The sort's comparison count on this (already sorted) input, as
        // `Map.fromList` will run it; numeric keys cost 1 unit each.
        let mut sort = 0u64;
        let mut keys: Vec<f64> = (0..n).map(|i| i as f64).collect();
        keys.sort_by(|a, b| {
            sort += 1;
            a.partial_cmp(b).unwrap()
        });
        let dedup = n as u64 - 1;
        // Internal nodes written by the bulk build: none for one leaf, the
        // root's 2 children at 17, 19 leaves + a 2-child root at 300.
        let internal = match n {
            16 => 0,
            17 => 2,
            _ => 19 + 2,
        };
        let cost = op_cost(MAP_DEFS, &format!("sorted({n}.0)"), "Map.fromList");
        assert_eq!(cost, call + n as u64 + sort + dedup + internal, "n = {n}");
    }
}

#[test]
fn map_internal_separators_charge_compared_string_bytes() {
    let call = map_call_overhead();
    // 17 keys sharing a 1000-byte prefix: leaves "a".."i" + "j".."q".
    // Finding "q" compares against separator "j", then 3 leaf keys; each
    // comparison costs the shorter key's length + 1 = 1002.
    let prefix = "a".repeat(1000);
    let keys: Vec<String> = ('a'..='q')
        .map(|c| format!("(\"{prefix}{c}\", 0.0)"))
        .collect();
    let setup = format!("Map.fromList([{}])", keys.join(", "));
    let probe = format!("Map.member(\"{prefix}q\")");
    let member = op_cost("", &setup, &probe) - call;
    assert_eq!(member, 4 * 1002);
}

#[test]
fn large_map_updates_charge_the_path_not_the_map() {
    // The old flat-vector copy charged ~6800 per update; the tree charges
    // its root-to-leaf path (depth 4 here).
    let cost = op_cost(
        MAP_DEFS,
        "Map.fromList(sorted(6800.0))",
        "Map.insert(3.5, 0.0)",
    );
    assert!(cost < 100, "insert into 6.8k entries cost {cost}");
    let cost = op_cost(MAP_DEFS, "Map.fromList(sorted(6800.0))", "Map.remove(3.0)");
    assert!(cost < 100, "remove from 6.8k entries cost {cost}");
}

#[test]
fn maps_with_different_edit_histories_are_equal() {
    // Different insert/remove orders build different tree shapes; equality
    // and display compare entries in key order.
    let src = "let up = List.range(300.0) |> List.fold((m, k) => Map.insert(k, k, m), Map.empty())\n\
               let down = List.range(600.0) |> List.reverse\n\
                 |> List.fold((m, k) => Map.insert(k, k, m), Map.empty())\n\
                 |> (m) => List.range(300.0) |> List.fold((acc, k) => Map.remove(k + 300.0, acc), m)\n\
               expect up == down\n";
    let out = reports(src);
    assert!(
        matches!(out[0].outcome, ExpectOutcome::Pass),
        "{:?}",
        out[0].outcome
    );
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
fn string_interpolation_doubling_cannot_amplify_past_the_budget() {
    let src = r#"
let grow = (s, _) => $"{s}{s}"
expect List.fold(grow, "x", List.range(26.0)) == ""
"#;
    let out = budgeted(src, 10_000);
    let ExpectOutcome::Error(err) = &out[0].outcome else {
        panic!("expected interpolation growth to exceed the budget");
    };
    assert!(err.message.contains("step budget"), "unexpected: {err:?}");
}

#[test]
fn container_construction_is_charged() {
    // The depth-amplifier probe: a fold whose body is a NESTED LITERAL adds
    // ~5 levels of value depth per single call charge — under per-call-only
    // charging this passes a 4k budget (~2k call charges for 1000
    // iterations) while building 5000 levels deep. Container-construction
    // charges (~5k more) must trip it — the arithmetic behind the
    // run_expects_budgeted stack contract (value depth ≤ budget).
    let src = "let deep = List.range(1000.0) |> List.fold((acc, x) => [[[[[acc]]]]], [0.0])\n\
               expect List.length(deep) == 1.0\n";
    let failure = functor_lang::run_expects_budgeted(&lower(src), &mut NoHost, Some(4_000))
        .err()
        .expect("the def-load fold should exceed the budget");
    assert!(failure.error.message.contains("step budget"));
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

// ------------------------------------------------- deep values must not crash

#[test]
fn deep_values_are_safe_under_the_documented_stack_contract() {
    // A fold wrapping `[acc]` per step builds value DEPTH iteratively —
    // ~50k levels here, far past any recursion limit. The run_expects_budgeted
    // stack contract: a budgeted run can't build deeper than its budget, so
    // in-process embedders run it on a worker whose stack covers the budget
    // (~100 B/level). This test IS that embedding: budget 500k on a worker
    // with a 256MB stack — build, compare (iterative value_eq, both the
    // passing and the Display-rendered failing case), and the deep value's
    // DROP (recursive, hence the contract) must all complete. On the default
    // 2MB test-thread stack the drop alone aborted the process.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let src = "let nest = List.range(50000.0) |> List.fold((acc, x) => [acc], [0.0])\n\
                       expect nest == nest\n\
                       expect nest == [1.0]\n";
            let out = functor_lang::run_expects_budgeted(&lower(src), &mut NoHost, Some(500_000))
                .unwrap_or_else(|failure| panic!("defs should load: {}", failure.error.message));
            assert!(matches!(out[0].outcome, ExpectOutcome::Pass));
            let ExpectOutcome::Fail(Some(cmp)) = &out[1].outcome else {
                panic!("expected a decomposed failure");
            };
            // The rendering really is the ~50k-deep value, not a truncation.
            assert!(cmp.lhs.starts_with("[[[["));
            assert!(cmp.lhs.len() > 100_000);
        })
        .expect("spawn worker")
        .join()
        .expect("deep-value worker must complete without overflowing");
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

#[test]
fn session_load_only_evaluates_the_kept_defs() {
    // Only `a` and `c` are evaluated; `b` (a host call `NoHost` lacks) is not.
    let src = "let a = 1.0\nlet b = List.head([])\nlet c = a + 1.0\n";
    let keep = |name: &str| name != "b";
    let session = functor_lang::Session::load_only(&lower(src), &keep, &mut NoHost)
        .unwrap_or_else(|failure| panic!("load_only: {}", failure.error.message));
    assert_eq!(
        session.global("c").map(|v| v.to_string()),
        Some("2".to_string())
    );
    assert!(session.global("b").is_none());
}
