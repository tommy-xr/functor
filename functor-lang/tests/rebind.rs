//! B5 part 2 — closures stored in the model rebind across a hot reload:
//! new code, old captured env (docs/functor-lang.md; the `rebind` module docs cover
//! the design). These tests simulate the reload seam exactly as the
//! producer drives it: run module A to get a model holding closures, lower
//! the edited module B, `rebind_value`, then call the stored closure
//! through B's session.

use functor_lang::ir::Module;
use functor_lang::{rebind_value, NoHost, RunOutcome, Session, Tracing, Value};

fn module(src: &str) -> Module {
    functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower")
}

/// `main()`'s value under `NoHost`.
fn main_value(module: &Module) -> Value {
    let record = match functor_lang::run(module, Tracing::Off) {
        Ok(record) => record,
        Err(failure) => panic!("run failed: {}", failure.error.message),
    };
    match record.outcome {
        RunOutcome::Main(value) => value,
        _ => panic!("expected a main result"),
    }
}

/// Call `apply(f, n)` in `module`'s session — how a reloaded program invokes
/// a stored closure.
fn apply(module: &Module, f: Value, n: f64) -> Value {
    let session = match Session::load(module, &mut NoHost) {
        Ok(session) => session,
        Err(failure) => panic!("load failed: {}", failure.error.message),
    };
    match session.call("apply", vec![f, Value::Number(n)], &mut NoHost) {
        Ok(value) => value,
        Err(err) => panic!("apply failed: {}", err.message),
    }
}

fn num(value: &Value) -> f64 {
    match value {
        Value::Number(n) => *n,
        other => panic!("expected a number, got {other}"),
    }
}

const APPLY: &str = "let apply = (f, n) => f(n)\n";

/// The core payoff: the stored closure runs the EDITED body with the
/// captured env carried over — new code, old data.
#[test]
fn stored_closure_adopts_edited_body_and_keeps_env() {
    let a = module(&format!(
        "{APPLY}let mul = (k) => (x) => x * k\nlet main = () => mul(3.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mul = (k) => (x) => x * k + 100.0\nlet main = () => mul(3.0)"
    ));
    let stored = main_value(&a);
    // Sanity: pre-reload behavior.
    assert_eq!(num(&apply(&a, stored.clone(), 2.0)), 6.0);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    // 2 * 3 + 100 — the new body, with the OLD captured k = 3.
    assert_eq!(num(&apply(&b, rebound, 2.0)), 106.0);
}

/// Closures inside lists, records, and variants are all reached.
#[test]
fn closures_rebind_inside_containers() {
    let src = |body: &str| {
        format!(
            "{APPLY}type Slot = | Held(f: float)\n\
             let mul = (k) => (x) => {body}\n\
             let main = () => {{ fns: [mul(2.0)], slot: Held(mul(10.0)) }}"
        )
    };
    let a = module(&src("x * k"));
    let b = module(&src("x * k + 1.0"));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 2, "warnings: {:?}", report.warnings);
    let Value::Record(fields) = &rebound else {
        panic!("expected the model record")
    };
    let Value::List(fns) = &fields[0].1 else {
        panic!("expected fns list")
    };
    assert_eq!(num(&apply(&b, fns[0].clone(), 5.0)), 11.0);
    let Value::Variant { args, .. } = &fields[1].1 else {
        panic!("expected the Held variant")
    };
    assert_eq!(num(&apply(&b, args[0].clone(), 5.0)), 51.0);
}

/// A captured closure (a closure in another's env) is rebound too.
#[test]
fn captured_closures_rebind_recursively() {
    let src = |body: &str| {
        format!(
            "{APPLY}let add = (a) => (x) => {body}\n\
             let wrap = (f) => (x) => f(x) * 2.0\n\
             let main = () => wrap(add(10.0))"
        )
    };
    let a = module(&src("x + a"));
    let b = module(&src("x + a + 0.5"));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    // wrap/fn and the captured add/fn both rebind.
    assert_eq!(report.rebound, 2, "warnings: {:?}", report.warnings);
    // (5 + 10 + 0.5) * 2
    assert_eq!(num(&apply(&b, rebound, 5.0)), 31.0);
}

/// A deleted/renamed def leaves the stored closure on its OLD body, loudly.
#[test]
fn deleted_def_keeps_old_body_with_warning() {
    let a = module(&format!(
        "{APPLY}let mul = (k) => (x) => x * k\nlet main = () => mul(3.0)"
    ));
    let b = module(&format!(
        "{APPLY}let other = (x) => x\nlet main = () => 0.0"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0].contains("`mul/fn` has no match after the edit"),
        "unexpected warning: {}",
        report.warnings[0]
    );
    // Old behavior still runs (its body Rcs keep the old IR alive).
    assert_eq!(num(&apply(&b, rebound, 2.0)), 6.0);
}

/// An edit that makes the closure capture a name its saved env lacks keeps
/// the old body, loudly.
#[test]
fn unresolvable_new_capture_keeps_old_body_with_warning() {
    let a = module(&format!(
        "{APPLY}let mul = (k) => (x) => x * k\nlet main = () => mul(3.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mul = (k) => let j = k + 1.0 in (x) => x * j\nlet main = () => mul(3.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert!(
        report.warnings[0].contains("now captures `j`"),
        "unexpected warning: {}",
        report.warnings[0]
    );
    assert_eq!(num(&apply(&b, rebound, 2.0)), 6.0);
}

/// The pointer-identity guard: a closure kept (with a warning) across reload
/// A→B carries an id from A. Rebinding B→C must refuse to identify it —
/// never mis-rebind it to whatever C happens to have at that id.
#[test]
fn stale_closures_from_two_reloads_ago_never_misrebind() {
    let a = module(&format!(
        "{APPLY}let mul = (k) => (x) => x * k\nlet main = () => mul(3.0)"
    ));
    // B deletes `mul` — the stored closure survives with a warning.
    let b = module(&format!("{APPLY}let main = () => 0.0"));
    // C reintroduces defs whose ids could collide with A's id space.
    let c = module(&format!(
        "{APPLY}let decoy = (q) => (x) => x * 1000.0\nlet main = () => 0.0"
    ));
    let stored = main_value(&a);
    let (kept, report_ab) = rebind_value(&stored, &a, &b);
    assert_eq!(report_ab.rebound, 0);
    let (still, report_bc) = rebind_value(&kept, &b, &c);
    assert_eq!(report_bc.rebound, 0);
    assert!(
        report_bc.warnings[0].contains("predates the previous reload"),
        "unexpected warning: {}",
        report_bc.warnings[0]
    );
    // Still the ORIGINAL behavior — not the decoy's.
    assert_eq!(num(&apply(&c, still, 2.0)), 6.0);
}

/// Plain data passes through untouched (and cheaply — shared, not copied).
#[test]
fn plain_data_is_preserved() {
    let a = module("let main = () => { n: 1.5, s: \"hi\", l: [true, false] }");
    let b = module("let main = () => 0.0");
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert!(report.warnings.is_empty());
    assert_eq!(rebound.to_string(), stored.to_string());
}

// --- Review-driven cases (Codex + Claude adversarial probes) ---

/// Path-based ids use NAMED segments: inserting a sibling record field does
/// not shift the others' identity (the ordinal-drift attack from review —
/// under `#k` ordinals the stored `mul` would silently have become `add`).
#[test]
fn record_field_insertion_does_not_shift_identity() {
    let a = module(&format!(
        "{APPLY}let make = (k) => {{ add: (x) => x + k, mul: (x) => x * k }}\n\
         let main = () => let m = make(3.0) in m.mul"
    ));
    let b = module(&format!(
        "{APPLY}let make = (k) => {{ sub: (x) => x - k, add: (x) => x + k, mul: (x) => x * k + 1.0 }}\n\
         let main = () => let m = make(3.0) in m.mul"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    // Still mul (with its edit), never add/sub: 2 * 3 + 1.
    assert_eq!(num(&apply(&b, rebound, 2.0)), 7.0);
}

/// A `let` body is a transparent path segment: wrapping the stored lambda's
/// def in a new helper `let` keeps its identity (Claude's insert probe — the
/// old code rebound to the helper and returned constant 0).
#[test]
fn inserting_a_helper_let_does_not_shift_identity() {
    let a = module(&format!(
        "{APPLY}let mk = (k) => (x) => x + k\nlet main = () => mk(10.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mk = (k) => let helper = (h) => 0.0 in (x) => x + k + 1.0\n\
         let main = () => mk(10.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    // The stored closure is still `mk/fn` (not `helper`): 2 + 10 + 1.
    assert_eq!(num(&apply(&b, rebound, 2.0)), 13.0);
}

/// An arity change reports at the reload boundary, not one frame later at
/// every call site (Claude M).
#[test]
fn arity_change_keeps_old_body_with_warning() {
    let a = module(&format!(
        "{APPLY}let mk = (k) => (x) => x + k\nlet main = () => mk(10.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mk = (k) => (x, y) => x + y + k\nlet main = () => mk(10.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert!(
        report.warnings[0].contains("changed arity (1 -> 2 parameters)"),
        "unexpected warning: {}",
        report.warnings[0]
    );
    assert_eq!(num(&apply(&b, rebound, 2.0)), 12.0);
}

/// A capture whose binder KIND changed (a parameter before, a `let` now)
/// keeps the old body loudly — the saved value can't stand in for the new
/// initializer's semantics (Codex M / Claude's shadowing probe: without the
/// kind check, the old param value silently replaced the new `let`'s).
#[test]
fn capture_kind_change_keeps_old_body_with_warning() {
    let a = module(&format!(
        "{APPLY}let mk = (k) => (x) => x * k\nlet main = () => mk(3.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mk = (k) => let k = k + 1.0 in (x) => x * k\nlet main = () => mk(3.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert!(
        report.warnings[0].contains("captures `k` differently after the edit"),
        "unexpected warning: {}",
        report.warnings[0]
    );
    assert_eq!(num(&apply(&b, rebound, 2.0)), 6.0);
}

/// The reverse shadowing direction (Claude's probe 3): the old body captured
/// an inner `let k`, the edit removes it so the new body's `k` is the param.
/// The kind check refuses to substitute the stale inner value.
#[test]
fn removed_shadow_keeps_old_body_with_warning() {
    let a = module(&format!(
        "{APPLY}let mk = (k) => let k = k * 10.0 in (x) => x + k\nlet main = () => mk(3.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mk = (k) => (x) => x + k\nlet main = () => mk(3.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 0);
    assert!(
        report.warnings[0].contains("captures `k` differently after the edit"),
        "unexpected warning: {}",
        report.warnings[0]
    );
    // Old behavior: x + 30.
    assert_eq!(num(&apply(&b, rebound, 2.0)), 32.0);
}

/// Closures inside tuples are reached by the reload walk.
#[test]
fn closures_rebind_inside_tuples() {
    let a = module(&format!(
        "{APPLY}let mk = (k) => (x) => x * k\nlet main = () => (mk(2.0), 1.0)"
    ));
    let b = module(&format!(
        "{APPLY}let mk = (k) => (x) => x * k + 1.0\nlet main = () => (mk(2.0), 1.0)"
    ));
    let stored = main_value(&a);
    let (rebound, report) = rebind_value(&stored, &a, &b);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    let Value::Tuple(items) = &rebound else {
        panic!("expected a tuple")
    };
    assert_eq!(num(&apply(&b, items[0].clone(), 5.0)), 11.0);
}
