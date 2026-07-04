//! B2 verification (docs/mle.md): IR snapshot goldens per example,
//! lowering determinism, name-resolution error + span assertions, and the
//! pipeline-desugar shape.

use mle::ir::{Expr, ExprKind, Module};
use std::fs;
use std::path::Path;

/// Parse + lower `src`, panicking with a rendered position on failure.
fn lower_src(src: &str) -> Module {
    let program = mle::parse(src).expect("source should parse");
    match mle::lower(program) {
        Ok(module) => module,
        Err(err) => {
            let (line, col) = mle::line_col(src, err.span.start);
            panic!("{line}:{col}: error: {}", err.message);
        }
    }
}

/// Lower `examples/{name}.mle` and compare the pretty-Debug IR against the
/// committed `examples/{name}.ir` golden.
/// Regenerate with `UPDATE_GOLDENS=1 cargo test -p mle`.
fn check_golden(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = fs::read_to_string(dir.join(format!("{name}.mle"))).unwrap();
    let golden_path = dir.join(format!("{name}.ir"));
    let actual = format!("{:#?}\n", lower_src(&src));
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
        "IR for {name}.mle diverged from {name}.ir — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p mle"
    );
}

#[test]
fn golden_pure_pipeline() {
    check_golden("pure-pipeline");
}

#[test]
fn golden_records() {
    check_golden("records");
}

#[test]
fn golden_functions() {
    check_golden("functions");
}

#[test]
fn golden_shapes() {
    check_golden("shapes");
}

#[test]
fn golden_tuples() {
    check_golden("tuples");
}

/// Same source must always produce byte-identical IR (stable IDs are
/// sequential, never random or time-based).
#[test]
fn lowering_is_deterministic() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = fs::read_to_string(dir.join("functions.mle")).unwrap();
    let first = format!("{:#?}", lower_src(&src));
    let second = format!("{:#?}", lower_src(&src));
    assert_eq!(first, second);
}

/// Lower a deliberately broken input; return the error's (message, line, col).
fn lower_err(src: &str) -> (String, usize, usize) {
    let program = mle::parse(src).expect("source should parse");
    let err = mle::lower(program).expect_err("source should fail to lower");
    let (line, col) = mle::line_col(src, err.span.start);
    (err.message, line, col)
}

#[test]
fn error_unknown_name() {
    assert_eq!(
        lower_err("let f = (a) => a + b"),
        ("unknown name `b`".to_string(), 1, 20)
    );
}

/// Redeclaring a builtin type name would shadow the primitive in annotations
/// ("expected Float, got Float"). [Claude L — B5 review]
#[test]
fn error_redeclare_builtin_type() {
    let (message, _, _) = lower_err("type Float = | Foo");
    assert_eq!(message, "cannot redeclare builtin type `Float`");
    let (message, _, _) = lower_err("type List = { head: Float }");
    assert_eq!(message, "cannot redeclare builtin type `List`");
}

/// A lowering error's position points at the offending identifier itself.
#[test]
fn error_span_points_at_identifier() {
    assert_eq!(
        lower_err("let f = (a) =>\n  velocity * a"),
        ("unknown name `velocity`".to_string(), 2, 3)
    );
}

/// Duplicate top-level names are errors: a def's name is its stable identity
/// (the hot-reload rebind key), so shadowing at the top level is not allowed.
#[test]
fn error_duplicate_definition() {
    assert_eq!(
        lower_err("let a = 1\nlet a = 2"),
        ("duplicate definition `a`".to_string(), 2, 1)
    );
}

/// The lambda body of `defs[index]`, as (params, body).
fn lambda_body(module: &Module, index: usize) -> (&[mle::ir::Param], &Expr) {
    let ExprKind::Lambda { params, body, .. } = &module.defs[index].value.kind else {
        panic!("expected def {index} to be a lambda");
    };
    (params, body)
}

/// A parameter named like a global resolves to the parameter, not the global.
#[test]
fn param_shadows_global() {
    let module = lower_src("let speed = 1.0\nlet f = (speed) => speed");
    let (params, body) = lambda_body(&module, 1);
    let ExprKind::Local { binding, name } = &body.kind else {
        panic!("expected the body to resolve to a local, got {body:?}");
    };
    assert_eq!(name, "speed");
    assert_eq!(*binding, params[0].binding);
}

/// Top-level defs are mutually visible: a def may call one declared later in
/// the file.
#[test]
fn global_resolves_before_its_definition() {
    let module = lower_src("let first = () => second()\nlet second = () => 1.0");
    let (_, body) = lambda_body(&module, 0);
    let ExprKind::Call { callee, .. } = &body.kind else {
        panic!("expected a call, got {body:?}");
    };
    assert!(matches!(&callee.kind, ExprKind::Global(name) if name == "second"));
}

/// `Pos.x` parses as a qualified name (uppercase qualifier), but when `Pos`
/// is a binding it must resolve to field access on it, not an external.
#[test]
fn qualified_name_on_binding_is_field_access() {
    let module = lower_src("let getX = (Pos) => Pos.x");
    let (params, body) = lambda_body(&module, 0);
    let ExprKind::FieldAccess { object, field } = &body.kind else {
        panic!("expected field access, got {body:?}");
    };
    assert_eq!(field, "x");
    let ExprKind::Local { binding, .. } = &object.kind else {
        panic!("expected the object to be a local, got {object:?}");
    };
    assert_eq!(*binding, params[0].binding);
}

/// `a |> f |> g(b)` desugars to `g(f(a), b)`: each stage becomes a call with
/// the piped value prepended as the first argument, no pipeline node remains,
/// and each desugared call carries the span of its stage.
#[test]
fn pipeline_desugars_to_nested_calls() {
    let src = "let f = (x) => x\nlet g = (x, y) => x\nlet r = (a, b) => a |> f |> g(b)";
    let module = lower_src(src);
    let (params, body) = lambda_body(&module, 2);

    // Outer call: g(<inner>, b), with the span of the `g(b)` stage.
    let ExprKind::Call { callee, args } = &body.kind else {
        panic!("expected a call, got {body:?}");
    };
    assert!(matches!(&callee.kind, ExprKind::Global(name) if name == "g"));
    assert_eq!(args.len(), 2);
    assert!(
        matches!(&args[1].kind, ExprKind::Local { binding, .. } if *binding == params[1].binding)
    );
    assert_eq!(&src[body.span.start..body.span.end], "g(b)");

    // Inner call: f(a), with the span of the `f` stage.
    let inner = &args[0];
    let ExprKind::Call { callee, args } = &inner.kind else {
        panic!("expected the first argument to be a call, got {inner:?}");
    };
    assert!(matches!(&callee.kind, ExprKind::Global(name) if name == "f"));
    assert_eq!(args.len(), 1);
    assert!(
        matches!(&args[0].kind, ExprKind::Local { binding, .. } if *binding == params[0].binding)
    );
    assert_eq!(&src[inner.span.start..inner.span.end], "f");
}

#[test]
fn error_duplicate_parameter() {
    let (message, line, col) = lower_err("let f = (a, a) => a");
    assert_eq!(message, "duplicate parameter `a`");
    assert_eq!((line, col), (1, 13));
}

// Types and values are separate namespaces (as in F#/OCaml) — `type Foo` and
// `let Foo` coexist; each namespace keys its own hot-reload identities.
#[test]
fn type_and_let_may_share_a_name() {
    let module = lower_src("type Foo = { x: Float }\nlet Foo = 1");
    assert_eq!(module.types.len(), 1);
    assert_eq!(module.defs.len(), 1);
    assert_eq!(module.types[0].name, "Foo");
    assert_eq!(module.defs[0].name, "Foo");
}

// Duplicate record fields would make record equality asymmetric at runtime
// (fields match by name) — rejected at lowering like duplicate params.
#[test]
fn error_duplicate_record_field() {
    let (message, line, col) = lower_err("let a = { x: 1.0, x: 2.0 }");
    assert_eq!(message, "duplicate record field `x`");
    assert_eq!((line, col), (1, 19));
}

// --- Mutability rules (see ~/notes mutability.md) ---

#[test]
fn error_lambda_captures_mut_read() {
    let (message, _, _) = lower_err("let f = (x) => let mut a = x in (y) => a + y");
    assert_eq!(message, "a function cannot capture the mutable binding `a`");
}

#[test]
fn error_lambda_captures_mut_assign() {
    let (message, _, _) = lower_err("let f = (x) => let mut a = x in (y) => a := y; a");
    assert_eq!(message, "a function cannot capture the mutable binding `a`");
}

#[test]
fn error_assign_to_immutable_let() {
    let (message, _, _) = lower_err("let f = (x) => let a = x in a := 1.0; a");
    assert_eq!(message, "cannot assign to immutable binding `a`");
}

#[test]
fn error_assign_to_param() {
    let (message, _, _) = lower_err("let f = (x) => x := 1.0; x");
    assert_eq!(message, "cannot assign to immutable binding `x`");
}

#[test]
fn error_assign_to_global() {
    let (message, _, _) = lower_err("let g = 1.0\nlet f = (x) => g := 2.0; x");
    assert_eq!(
        message,
        "cannot assign to top-level `g` (globals are immutable)"
    );
}

#[test]
fn error_duplicate_update_field() {
    let (message, _, _) = lower_err("let f = (p) => { p with x: 1.0, x: 2.0 }");
    assert_eq!(message, "duplicate record field `x`");
}

// --- Variants + match (B5 part 1) ---

/// Constructors resolve bare, so a name shared by two variant types would be
/// ambiguous — unique ACROSS types, not just within one.
#[test]
fn error_ctor_collision_across_types() {
    let (message, line, col) = lower_err(
        "type Shape = | Circle(r: Float)\n\
         type Blob = | Circle(r: Float)",
    );
    assert_eq!(message, "duplicate constructor `Circle`");
    assert_eq!((line, col), (2, 15));
}

#[test]
fn error_ctor_collision_within_a_type() {
    let (message, _, _) = lower_err("type Shape = | Circle(r: Float) | Circle(d: Float)");
    assert_eq!(message, "duplicate constructor `Circle`");
}

/// Constructors live in the VALUE namespace: `let Circle` and a constructor
/// `Circle` collide (in either declaration order).
#[test]
fn error_ctor_collides_with_let() {
    let expected = "duplicate definition `Circle` (constructors live in the value namespace)";
    let (message, line, col) = lower_err("let Circle = 1\ntype Shape = | Circle(r: Float)");
    assert_eq!(message, expected);
    assert_eq!((line, col), (2, 16));
    let (message, line, col) = lower_err("type Shape = | Circle(r: Float)\nlet Circle = 1");
    assert_eq!(message, expected);
    assert_eq!((line, col), (2, 1));
}

/// …but `type Shape` itself stays in the type namespace: a `let Shape` is
/// fine, and `Shape.Circle` is deliberately NOT a constructor reference
/// (only bare `Circle` is) — it stays an unknown external.
#[test]
fn qualified_ctor_form_is_not_supported() {
    let module = lower_src("type Shape = | Circle(r: Float)\nlet f = () => Shape.Circle(1.0)");
    let (_, body) = lambda_body(&module, 0);
    let ExprKind::Call { callee, .. } = &body.kind else {
        panic!("expected a call, got {body:?}");
    };
    assert!(
        matches!(&callee.kind, ExprKind::External(path) if path == &["Shape", "Circle"]),
        "expected an external, got {callee:?}"
    );
}

/// A bare uppercase identifier resolves to a constructor reference; the
/// declared arity rides along in the IR.
#[test]
fn bare_ctor_resolves_with_arity() {
    let module = lower_src("type Shape = | Circle(r: Float) | Point\nlet f = () => Circle(2.0)");
    let (_, body) = lambda_body(&module, 0);
    let ExprKind::Call { callee, .. } = &body.kind else {
        panic!("expected a call, got {body:?}");
    };
    assert!(
        matches!(&callee.kind, ExprKind::Ctor { name, arity } if name == "Circle" && *arity == 1),
        "expected a ctor ref, got {callee:?}"
    );
}

/// A local binding shadows a constructor, exactly like it shadows a global.
#[test]
fn param_shadows_ctor() {
    let module = lower_src("type Shape = | Circle(r: Float)\nlet f = (Circle) => Circle");
    let (params, body) = lambda_body(&module, 0);
    let ExprKind::Local { binding, .. } = &body.kind else {
        panic!("expected a local, got {body:?}");
    };
    assert_eq!(*binding, params[0].binding);
}

/// An unknown constructor in a pattern is a lowering error (in expressions
/// an unknown uppercase name is the existing "unknown name" error).
#[test]
fn error_unknown_ctor_in_pattern() {
    let (message, line, col) = lower_err("let f = (s) => match s with | Nope => 1.0");
    assert_eq!(message, "unknown constructor `Nope`");
    assert_eq!((line, col), (1, 31));
}

/// A constructor pattern must name exactly the declared field count.
#[test]
fn error_pattern_arity_mismatch() {
    let (message, _, _) = lower_err(
        "type Shape = | Circle(r: Float)\n\
         let f = (s) => match s with | Circle(a, b) => a | _ => 0.0",
    );
    assert_eq!(message, "`Circle` has 1 field(s), but the pattern names 2");

    let (message, _, _) = lower_err(
        "type Shape = | Circle(r: Float)\n\
         let f = (s) => match s with | Circle => 1.0 | _ => 0.0",
    );
    assert_eq!(message, "`Circle` has 1 field(s), but the pattern names 0");
}

#[test]
fn error_duplicate_pattern_variable() {
    let (message, line, col) = lower_err(
        "type Shape = | Rect(w: Float, h: Float)\n\
         let f = (s) => match s with | Rect(a, a) => a | _ => 0.0",
    );
    assert_eq!(message, "duplicate pattern variable `a`");
    assert_eq!((line, col), (2, 39));
}

/// Pattern variables are scoped to their own arm's body — they never leak
/// into later arms.
#[test]
fn pattern_vars_do_not_leak_between_arms() {
    let (message, _, _) = lower_err(
        "type Shape = | Circle(r: Float) | Point\n\
         let f = (s) => match s with | Circle(r) => r | Point => r",
    );
    assert_eq!(message, "unknown name `r`");
}

/// Pattern variables are plain immutable bindings: lambdas may capture them
/// (unlike `mut` slots)…
#[test]
fn lambdas_may_capture_pattern_vars() {
    let module = lower_src(
        "type Shape = | Circle(r: Float)\n\
         let f = (s) => match s with | Circle(r) => (x) => r + x | _ => (x) => x",
    );
    assert_eq!(module.defs.len(), 1);
}

/// …and they cannot be assigned.
#[test]
fn error_assign_to_pattern_var() {
    let (message, _, _) = lower_err(
        "type Shape = | Circle(r: Float)\n\
         let f = (s) => match s with | Circle(r) => r := 1.0; r | _ => 0.0",
    );
    assert_eq!(message, "cannot assign to immutable binding `r`");
}
