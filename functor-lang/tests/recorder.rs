//! Binding-site recorder verification (the paused visual-debugger's PR1 seam,
//! `Session::call_recorded`): a bounded, per-call-armed record of every `let` /
//! parameter / match-variable value AND every variable-reference read, with
//! last-value + hit-count per site, kind/preview rendering policy, and a
//! `truncated` cap flag — and NO effect on evaluation itself.

use functor_lang::value::Value;
use functor_lang::{
    NoHost, RecordedBinding, RecordedInvocation, RecordedKind, RecordedSite, Session,
};
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
fn records_reference_sites_per_use() {
    // `model` is READ twice on the body line — each read is its own site
    // (distinct span), name-precise, tagged Ref; the param binder is Binder.
    let src = "let update = (model) => model + model";
    let session = session(src);
    let (_, inv) = session
        .call_recorded("update", vec![Value::Number(2.0)], &mut NoHost)
        .expect("call_recorded");

    let refs: Vec<&RecordedBinding> = inv
        .bindings
        .iter()
        .filter(|b| b.name == "model" && b.site == RecordedSite::Ref)
        .collect();
    assert_eq!(refs.len(), 2, "each read is its own site");
    for r in &refs {
        assert_eq!(spanned(src, r), "model");
        assert_eq!(r.value, "2");
        assert_eq!(r.count, 1);
    }
    assert_ne!(refs[0].span.start, refs[1].span.start);
    assert_eq!(binding(&inv, "model").site, RecordedSite::Binder);
}

#[test]
fn callee_references_are_not_recorded() {
    // Reading `helper` to CALL it is not data worth overlaying — only `m`'s
    // read records.
    let src = "let helper = (x) => x + 1.0\nlet update = (m) => helper(m)";
    let session = session(src);
    let (_, inv) = session
        .call_recorded("update", vec![Value::Number(4.0)], &mut NoHost)
        .expect("call_recorded");

    assert!(
        inv.bindings
            .iter()
            .all(|b| !(b.name == "helper" && b.site == RecordedSite::Ref)),
        "callable reference leaked into the record"
    );
    // The nested call's param binder still records (flat map).
    assert_eq!(binding(&inv, "x").value, "4");
}

#[test]
fn kind_and_preview_follow_the_inline_vs_hover_policy() {
    // A composite model previews one level deep (nested records elide to …);
    // a primitive's preview IS its value.
    let src = "let update = (model) =>\n  let hp = model.hp in\n  hp";
    let session = session(src);
    let model = Value::Record(Rc::new(vec![
        (
            "pos".to_string(),
            Value::Record(Rc::new(vec![
                ("x".to_string(), Value::Number(1.0)),
                ("y".to_string(), Value::Number(2.0)),
            ])),
        ),
        ("hp".to_string(), Value::Number(3.0)),
    ]));
    let (_, inv) = session
        .call_recorded("update", vec![model], &mut NoHost)
        .expect("call_recorded");

    let m = binding(&inv, "model");
    assert_eq!(m.kind, RecordedKind::Composite);
    assert_eq!(m.preview, "{ pos: …, hp: 3 }");
    assert_eq!(m.value, "{ pos: { x: 1, y: 2 }, hp: 3 }");

    let hp = binding(&inv, "hp");
    assert_eq!(hp.kind, RecordedKind::Primitive);
    assert_eq!(hp.preview, hp.value);
    assert_eq!(hp.value, "3");

    // The invocation result carries a preview too.
    assert_eq!(inv.result_preview, "3");
}

#[test]
fn long_values_elide_in_previews() {
    let src = "let idText = (s) => s\nlet firstOf = (xs) => xs";
    let session = session(src);

    // A long string caps at 40 chars with a marked tail.
    let long = "x".repeat(60);
    let (_, inv) = session
        .call_recorded("idText", vec![Value::String(Rc::from(long.as_str()))], &mut NoHost)
        .expect("call_recorded");
    let s = binding(&inv, "s");
    assert_eq!(s.kind, RecordedKind::Composite, "a long string is not inline-complete");
    assert_eq!(s.preview, format!("\"{}…\"", "x".repeat(40)));

    // A long list elides after 4 items.
    let xs = Value::List(Rc::new((0..10).map(|i| Value::Number(i as f64)).collect()));
    let (_, inv) = session
        .call_recorded("firstOf", vec![xs], &mut NoHost)
        .expect("call_recorded");
    assert_eq!(binding(&inv, "xs").preview, "[0, 1, 2, 3, …]");
}

#[test]
fn loop_reference_sites_count_reads() {
    // Inside the fold closure, `acc`'s READ site is hit once per element.
    let src = "let sum = (xs) => List.fold((acc, x) => acc + x, 0.0, xs)";
    let session = session(src);
    let xs = Value::List(Rc::new(vec![
        Value::Number(1.0),
        Value::Number(2.0),
        Value::Number(3.0),
    ]));
    let (_, inv) = session
        .call_recorded("sum", vec![xs], &mut NoHost)
        .expect("call_recorded");

    let acc_ref = inv
        .bindings
        .iter()
        .find(|b| b.name == "acc" && b.site == RecordedSite::Ref)
        .expect("acc read site");
    assert_eq!(acc_ref.count, 3);
    assert_eq!(acc_ref.value, "3"); // the read feeding the final `acc + x`
}

#[test]
fn mut_reads_record_current_values() {
    // A `let mut` READ goes through its own eval arm (LocalMut) — the read
    // after the assignment must record the post-assign value.
    let src = "let update = (n) => let mut c = n in c := c + 1.0; c";
    let session = session(src);
    let (result, inv) = session
        .call_recorded("update", vec![Value::Number(4.0)], &mut NoHost)
        .expect("call_recorded");

    assert_eq!(result.to_string(), "5");
    let c_refs: Vec<&RecordedBinding> = inv
        .bindings
        .iter()
        .filter(|b| b.name == "c" && b.site == RecordedSite::Ref)
        .collect();
    // Two read sites: the assignment's RHS (pre-assign, 4) and the final
    // read (post-assign, 5).
    assert!(
        c_refs.iter().any(|r| r.value == "4") && c_refs.iter().any(|r| r.value == "5"),
        "mut reads missing or stale: {:?}",
        c_refs.iter().map(|r| r.value.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn reference_sites_cannot_starve_binders() {
    // More distinct reference sites than the per-class cap, followed by a
    // binder: the ref budget breaches (truncated), but the binder — the more
    // valuable record — still lands.
    let reads = vec!["x"; 1030].join(" + ");
    let src = format!("let update = (x) => let z = {reads} in z");
    let session = session(&src);
    let (_, inv) = session
        .call_recorded("update", vec![Value::Number(1.0)], &mut NoHost)
        .expect("call_recorded");

    assert!(inv.truncated, "1030 ref sites must breach the 1024 ref budget");
    let z = binding(&inv, "z");
    assert_eq!(z.site, RecordedSite::Binder);
    assert_eq!(z.value, "1030");
}

#[test]
fn string_previews_cap_by_characters_not_bytes() {
    let src = "let idText = (s) => s";
    let session = session(src);

    // 39 multibyte chars: primitive (the cap counts characters).
    let short = "é".repeat(39);
    let (_, inv) = session
        .call_recorded("idText", vec![Value::String(Rc::from(short.as_str()))], &mut NoHost)
        .expect("call_recorded");
    assert_eq!(binding(&inv, "s").kind, RecordedKind::Primitive);

    // 45 multibyte chars: composite, capped at 40 CHARACTERS.
    let long = "é".repeat(45);
    let (_, inv) = session
        .call_recorded("idText", vec![Value::String(Rc::from(long.as_str()))], &mut NoHost)
        .expect("call_recorded");
    let s = binding(&inv, "s");
    assert_eq!(s.kind, RecordedKind::Composite);
    assert_eq!(s.preview, format!("\"{}…\"", "é".repeat(40)));
}

#[test]
fn preview_cut_on_a_quote_stays_well_formed() {
    // The 40th character is a `"`: the escaped quote must survive the cut
    // (a trim would eat it along with the closing delimiter).
    let long = format!("{}\"{}", "x".repeat(39), "y".repeat(20));
    let src = "let idText = (s) => s";
    let session = session(src);
    let (_, inv) = session
        .call_recorded("idText", vec![Value::String(Rc::from(long.as_str()))], &mut NoHost)
        .expect("call_recorded");
    let preview = &binding(&inv, "s").preview;
    assert!(
        preview.ends_with("\\\"…\""),
        "escaped quote at the cut must survive: {preview}"
    );
}

#[test]
fn empty_collections_are_primitive() {
    let src = "let firstOf = (xs) => xs";
    let session = session(src);
    let (_, inv) = session
        .call_recorded("firstOf", vec![Value::List(Rc::new(vec![]))], &mut NoHost)
        .expect("call_recorded");
    let xs = binding(&inv, "xs");
    assert_eq!(xs.kind, RecordedKind::Primitive);
    assert_eq!(xs.preview, "[]");
    assert_eq!(xs.preview, xs.value);
}

#[test]
fn coverage_records_the_taken_arm_only() {
    // Runtime coverage is the recency gutter's data: the TAKEN match arm's
    // span is covered, the un-taken arm's is not (statically both are
    // runnable — see functor_lang::coverage::runnable_offsets).
    let src = "let pick = (b) =>\n  match b with\n  | true => 1.0\n  | false => 2.0";
    let session = session(src);
    let taken = src.find("1.0").unwrap();
    let untaken = src.find("2.0").unwrap();

    let (_, inv) = session
        .call_recorded("pick", vec![Value::Bool(true)], &mut NoHost)
        .expect("call_recorded");
    assert!(inv.coverage.contains(&taken), "taken arm covered: {:?}", inv.coverage);
    assert!(!inv.coverage.contains(&untaken), "un-taken arm NOT covered");
    assert!(inv.coverage.windows(2).all(|w| w[0] < w[1]), "sorted + deduped");

    // The coverage-only mode agrees and still returns the exact result.
    let (result, cov) = session
        .call_covered("pick", vec![Value::Bool(true)], &mut NoHost)
        .expect("call_covered");
    assert_eq!(result.to_string(), "1");
    assert_eq!(cov, inv.coverage);
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
