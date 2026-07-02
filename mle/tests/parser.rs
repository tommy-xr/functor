//! B1 verification (docs/mle.md): AST snapshot goldens per example,
//! parse-error message + position assertions, and span → source-text sanity.

use mle::ast::{ExprKind, Item};
use mle::Span;
use std::fs;
use std::path::Path;

/// Parse `examples/{name}.mle` and compare the pretty-Debug AST against the
/// committed `examples/{name}.ast` golden.
/// Regenerate with `UPDATE_GOLDENS=1 cargo test -p mle`.
fn check_golden(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src_path = dir.join(format!("{name}.mle"));
    let golden_path = dir.join(format!("{name}.ast"));
    let src = fs::read_to_string(&src_path).unwrap();
    let program = match mle::parse(&src) {
        Ok(program) => program,
        Err(err) => {
            let (line, col) = mle::line_col(&src, err.span.start);
            panic!(
                "{}:{line}:{col}: error: {}",
                src_path.display(),
                err.message
            );
        }
    };
    let actual = format!("{program:#?}\n");
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
        "AST for {name}.mle diverged from {name}.ast — if intended, \
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

/// Parse a deliberately broken input; return the error's (message, line, col).
fn parse_err(src: &str) -> (String, usize, usize) {
    let err = mle::parse(src).expect_err("input should fail to parse");
    let (line, col) = mle::line_col(src, err.span.start);
    (err.message, line, col)
}

#[test]
fn error_missing_let_name() {
    assert_eq!(
        parse_err("let = 1"),
        ("expected a name after `let`, found `=`".to_string(), 1, 5)
    );
}

#[test]
fn error_unterminated_string() {
    assert_eq!(
        parse_err("let s = \"abc"),
        ("unterminated string".to_string(), 1, 9)
    );
}

#[test]
fn error_record_field_missing_colon() {
    assert_eq!(
        parse_err("let p = { x: 1.0, y }"),
        (
            "expected `:` after field name, found `}`".to_string(),
            1,
            21
        )
    );
}

#[test]
fn error_unclosed_paren() {
    assert_eq!(
        parse_err("let f = (a, b => a"),
        ("expected `)`, found `,`".to_string(), 1, 11)
    );
}

#[test]
fn error_missing_operand_reports_second_line() {
    assert_eq!(
        parse_err("let a = 1\nlet b = 2 +"),
        (
            "expected an expression, found end of input".to_string(),
            2,
            12
        )
    );
}

/// Pathological nesting must fail as a clean parse error, not a stack
/// overflow (MLE sources may be machine-generated).
#[test]
fn error_deeply_nested_expression() {
    let src = format!("let x = {}1{}", "(".repeat(300), ")".repeat(300));
    let err = mle::parse(&src).expect_err("should hit the depth limit");
    assert_eq!(err.message, "expression nested too deeply");
}

#[test]
fn generic_args_allow_trailing_comma() {
    assert!(mle::parse("type T = { xs: List<Float,> }").is_ok());
}

/// Error spans must stay sliceable (char-boundary aligned) even when the
/// offending character is multi-byte.
#[test]
fn unknown_escape_span_is_sliceable() {
    let src = "let s = \"a\\é\"";
    let err = mle::parse(src).expect_err("unknown escape should fail");
    assert_eq!(err.message, "unknown escape sequence");
    assert_eq!(&src[err.span.start..err.span.end], "\\é");
}

fn text(src: &str, span: Span) -> &str {
    &src[span.start..span.end]
}

/// Spans are byte offsets into the source: slicing a node's span must yield
/// its exact source text.
#[test]
fn spans_map_to_source_text() {
    let src = "let move = (p) => p.x + speed * 2.0";
    let program = mle::parse(src).unwrap();
    let Item::Let(decl) = &program.items[0] else {
        panic!("expected a let declaration");
    };
    assert_eq!(text(src, decl.span), src);
    assert_eq!(text(src, decl.value.span), "(p) => p.x + speed * 2.0");
    let ExprKind::Lambda { body, .. } = &decl.value.kind else {
        panic!("expected a lambda");
    };
    assert_eq!(text(src, body.span), "p.x + speed * 2.0");
    let ExprKind::Binary { lhs, rhs, .. } = &body.kind else {
        panic!("expected a binary expression");
    };
    assert_eq!(text(src, lhs.span), "p.x");
    assert_eq!(text(src, rhs.span), "speed * 2.0");
}
