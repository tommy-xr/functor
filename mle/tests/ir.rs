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
