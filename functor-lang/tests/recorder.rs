//! Binding-site recorder verification (the paused visual-debugger's PR1 seam,
//! `Session::call_recorded`): a bounded, per-call-armed record of every `let` /
//! parameter / match-variable value, with last-value + hit-count per site and a
//! `truncated` cap flag — and NO effect on evaluation itself.

use functor_lang::value::Value;
use functor_lang::{NoHost, RecordedBinding, RecordedInvocation, Session};
use std::rc::Rc;

/// Parse + lower + load `src` into a session, panicking with a rendered
/// position on failure.
fn session(src: &str) -> Session {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    match Session::load(&module, &mut NoHost) {
        Ok(session) => session,
        Err(failure) => {
            let (line, col) = functor_lang::line_col(src, failure.error.span.start);
            panic!("{line}:{col}: load error: {}", failure.error.message);
        }
    }
}

/// The recorded binding named `name` (panics if absent).
fn binding<'a>(inv: &'a RecordedInvocation, name: &str) -> &'a RecordedBinding {
    inv.bindings
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no recorded binding `{name}`"))
}

/// The exact source text the binding's span points at (proves spans are real
/// byte offsets into the loaded source).
fn spanned<'a>(src: &'a str, b: &RecordedBinding) -> &'a str {
    &src[b.span.start..b.span.end]
}

#[test]
fn records_let_bindings_and_params_and_result() {
    let src = "let update = (model) =>\n  let doubled = model + 1.0 in\n  doubled + 10.0";
    let session = session(src);
    let (result, inv) = session
        .call_recorded("update", vec![Value::Number(5.0)], &mut NoHost)
        .expect("call_recorded");

    assert_eq!(result.to_string(), "16");
    assert_eq!(inv.entry, "update");
    assert_eq!(inv.result, "16");
    assert!(!inv.truncated);

    let model = binding(&inv, "model");
    assert_eq!(model.value, "5");
    assert_eq!(model.count, 1);
    assert_eq!(spanned(src, model), "model");

    let doubled = binding(&inv, "doubled");
    assert_eq!(doubled.value, "6");
    assert_eq!(doubled.count, 1);
    // A `let`'s span is the `let name =` binder region (goto/hover convention).
    assert!(spanned(src, doubled).contains("doubled"));
}

#[test]
fn records_match_binders() {
    let src = "type Shape = | Circle(radius: Float) | Square(side: Float)\n\
               let area = (shape) =>\n  \
               match shape with\n  \
               | Circle(r) => r + 1.0\n  \
               | Square(s) => s + 2.0";
    let session = session(src);
    let circle = Value::Variant {
        ctor: Rc::from("Circle"),
        args: Rc::new(vec![Value::Number(3.0)]),
    };
    let (result, inv) = session
        .call_recorded("area", vec![circle], &mut NoHost)
        .expect("call_recorded");

    assert_eq!(result.to_string(), "4");
    // The scrutinee parameter renders as the whole variant.
    assert_eq!(binding(&inv, "shape").value, "Circle(3)");
    // The winning arm's binder is recorded; the losing arm's `s` is not.
    let r = binding(&inv, "r");
    assert_eq!(r.value, "3");
    assert_eq!(r.count, 1);
    assert_eq!(spanned(src, r), "r");
    assert!(inv.bindings.iter().all(|b| b.name != "s"));
}

#[test]
fn loop_site_keeps_last_value_and_counts_hits() {
    let src = "let sum = (xs) => List.fold((acc, x) => acc + x, 0.0, xs)";
    let session = session(src);
    let xs = Value::List(Rc::new(vec![
        Value::Number(1.0),
        Value::Number(2.0),
        Value::Number(3.0),
    ]));
    let (result, inv) = session
        .call_recorded("sum", vec![xs], &mut NoHost)
        .expect("call_recorded");

    assert_eq!(result.to_string(), "6");
    assert!(!inv.truncated);

    // The fold closure's params re-bind once per element: last value wins,
    // count is the element count.
    let acc = binding(&inv, "acc");
    assert_eq!(acc.count, 3);
    assert_eq!(acc.value, "3"); // acc going into the final `acc + x` (1 + 2)
    let x = binding(&inv, "x");
    assert_eq!(x.count, 3);
    assert_eq!(x.value, "3"); // the last element
    assert_eq!(binding(&inv, "xs").count, 1);
}

#[test]
fn cap_breach_truncates_but_result_is_exact() {
    // 60_000 elements × 2 closure params = 120_000 binding events, past the
    // 100_000 event cap — recording stops, `truncated` is set, and the fold
    // still returns the exact sum (recording never changes evaluation).
    let src = "let sum = (xs) => List.fold((acc, x) => acc + x, 0.0, xs)\n\
               let total = (n) => sum(List.range(n))";
    let session = session(src);

    let (recorded, inv) = session
        .call_recorded("total", vec![Value::Number(60_000.0)], &mut NoHost)
        .expect("call_recorded");
    assert!(inv.truncated);

    // Same call with the recorder OFF must yield the identical value.
    let plain = session
        .call("total", vec![Value::Number(60_000.0)], &mut NoHost)
        .expect("call");
    assert_eq!(recorded.to_string(), plain.to_string());
}
