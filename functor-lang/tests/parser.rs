//! B1 verification (docs/functor-lang.md): AST snapshot goldens per example,
//! parse-error message + position assertions, and span → source-text sanity.

use functor_lang::ast::{ExprKind, Item};
use functor_lang::Span;
use std::fs;
use std::path::Path;

/// Parse `examples/{name}.fun` and compare the pretty-Debug AST against the
/// committed `examples/{name}.ast` golden.
/// Regenerate with `UPDATE_GOLDENS=1 cargo test -p functor-lang`.
fn check_golden(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src_path = dir.join(format!("{name}.fun"));
    let golden_path = dir.join(format!("{name}.ast"));
    let src = fs::read_to_string(&src_path).unwrap();
    let program = match functor_lang::parse(&src) {
        Ok(program) => program,
        Err(err) => {
            let (line, col) = functor_lang::line_col(&src, err.span.start);
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
            "missing golden {} — generate with UPDATE_GOLDENS=1 cargo test -p functor-lang",
            golden_path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "AST for {name}.fun diverged from {name}.ast — if intended, \
         regenerate with UPDATE_GOLDENS=1 cargo test -p functor-lang"
    );
}

#[test]
fn golden_pure_pipeline() {
    check_golden("pure_pipeline");
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

#[test]
fn golden_lists() {
    check_golden("lists");
}

#[test]
fn golden_strings() {
    check_golden("strings");
}

/// Parse a deliberately broken input; return the error's (message, line, col).
fn parse_err(src: &str) -> (String, usize, usize) {
    let err = functor_lang::parse(src).expect_err("input should fail to parse");
    let (line, col) = functor_lang::line_col(src, err.span.start);
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
    // With tuples in the grammar, the comma reads as a tuple continuing —
    // the error lands on the `=>` that can't follow an element.
    assert_eq!(
        parse_err("let f = (a, b => a"),
        ("expected `,` or `)`, found `=>`".to_string(), 1, 15)
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
/// overflow (Functor Lang sources may be machine-generated).
#[test]
fn error_deeply_nested_expression() {
    let src = format!("let x = {}1{}", "(".repeat(300), ")".repeat(300));
    let err = functor_lang::parse(&src).expect_err("should hit the depth limit");
    assert_eq!(err.message, "expression nested too deeply");
}

#[test]
fn generic_args_allow_trailing_comma() {
    assert!(functor_lang::parse("type T = { xs: List<float,> }").is_ok());
}

/// Error spans must stay sliceable (char-boundary aligned) even when the
/// offending character is multi-byte.
#[test]
fn unknown_escape_span_is_sliceable() {
    let src = "let s = \"a\\é\"";
    let err = functor_lang::parse(src).expect_err("unknown escape should fail");
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
    let program = functor_lang::parse(src).unwrap();
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

// [AGREED review] `{ base with }` is a silent no-op copy — rejected.
#[test]
fn empty_record_update_is_an_error() {
    let err = functor_lang::parse("let f = (p) => { p with }").expect_err("should fail");
    assert!(err
        .message
        .contains("at least one `name: value` after `with`"));
}

// [review] a stray `:=` after a non-name target gets a targeted error.
#[test]
fn assignment_to_field_is_a_targeted_error() {
    let err = functor_lang::parse("let f = (p) => p.x := 1.0; p").expect_err("should fail");
    assert!(
        err.message
            .contains("assignment targets must be a bare `let mut` name"),
        "got: {}",
        err.message
    );
}

// --- Variant declarations + match (B5 part 1) ---

/// Both `type` bodies parse; the variant form allows nullary constructors
/// and trailing commas in a constructor's field list.
#[test]
fn variant_declaration_forms_parse() {
    assert!(
        functor_lang::parse("type Shape = | Circle(radius: float) | Rect(w: float, h: float,) | Point")
            .is_ok()
    );
    assert!(functor_lang::parse("type Answer = | Yes").is_ok());
}

/// The leading `|` is required before the FIRST alternative too.
#[test]
fn error_variant_requires_leading_bar() {
    assert_eq!(
        parse_err("type Shape = Circle(radius: float)"),
        (
            "expected `{` (a record type) or `|` (a variant alternative), found `Circle`"
                .to_string(),
            1,
            14
        )
    );
}

#[test]
fn error_lowercase_constructor_name() {
    assert_eq!(
        parse_err("type Shape = | circle(radius: float)"),
        (
            "constructor name `circle` must start uppercase".to_string(),
            1,
            16
        )
    );
}

/// A constructor's fields are named in the declaration.
#[test]
fn error_variant_field_needs_a_name() {
    assert_eq!(
        parse_err("type Shape = | Circle(float)"),
        (
            "expected `:` after field name, found `)`".to_string(),
            1,
            28
        )
    );
}

#[test]
fn match_parses_all_pattern_kinds() {
    let src = "let f = (s) => match s with\n\
               | Circle(r, _) => r\n\
               | Point => 0.0\n\
               | true => 1.0\n\
               | 2.0 => 2.0\n\
               | \"s\" => 3.0\n\
               | x => x\n\
               | _ => 0.0";
    let program = functor_lang::parse(src).unwrap();
    let Item::Let(decl) = &program.items[0] else {
        panic!("expected a let declaration");
    };
    let ExprKind::Lambda { body, .. } = &decl.value.kind else {
        panic!("expected a lambda");
    };
    let ExprKind::Match { arms, .. } = &body.kind else {
        panic!("expected a match, got {body:?}");
    };
    assert_eq!(arms.len(), 7);
    use functor_lang::ast::PatternKind::*;
    assert!(
        matches!(&arms[0].pattern.kind, Ctor { name, args } if name == "Circle" && args.len() == 2)
    );
    assert!(
        matches!(&arms[1].pattern.kind, Ctor { name, args } if name == "Point" && args.is_empty())
    );
    assert!(matches!(&arms[2].pattern.kind, Bool(true)));
    assert!(matches!(&arms[3].pattern.kind, Number(n) if *n == 2.0));
    assert!(matches!(&arms[4].pattern.kind, String(s) if s == "s"));
    assert!(matches!(&arms[5].pattern.kind, Var(name) if name == "x"));
    assert!(matches!(&arms[6].pattern.kind, Wildcard));
}

/// The leading `|` is required before the first arm.
#[test]
fn error_match_requires_leading_bar() {
    assert_eq!(
        parse_err("let f = (s) => match s with s => 1.0"),
        (
            "expected `|` to begin a match arm, found `s`".to_string(),
            1,
            29
        )
    );
}

/// Sub-patterns are bindings or `_` only — constructor patterns don't nest.
#[test]
fn error_nested_constructor_pattern() {
    assert_eq!(
        parse_err("let f = (s) => match s with | Circle(Point) => 1.0 | _ => 0.0"),
        (
            "expected a binding name or `_` (constructor patterns do not nest), found `Point`"
                .to_string(),
            1,
            38
        )
    );
}

/// GREEDY ARMS: a nested match inside an arm consumes the following `|`
/// arms as its own; parenthesizing restores them to the outer match (the
/// documented F#/OCaml convention).
#[test]
fn nested_match_consumes_following_arms_greedily() {
    let arms_of = |src: &str| -> (usize, usize) {
        let program = functor_lang::parse(src).unwrap();
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected a let declaration");
        };
        let ExprKind::Match { arms, .. } = &decl.value.kind else {
            panic!("expected a match, got {:?}", decl.value.kind);
        };
        let ExprKind::Match { arms: inner, .. } = &arms[0].body.kind else {
            panic!("expected the first arm body to be a match");
        };
        (arms.len(), inner.len())
    };
    // Unparenthesized: the inner match eats `| false => 2.0`.
    let (outer, inner) =
        arms_of("let x = match true with | true => match false with | true => 1.0 | false => 2.0");
    assert_eq!((outer, inner), (1, 2));
    // Parenthesized: the outer match keeps its two arms.
    let (outer, inner) = arms_of(
        "let x = match true with | true => (match false with | true => 1.0) | false => 2.0",
    );
    assert_eq!((outer, inner), (2, 1));
}

/// `match` binds loosest, like let-in: an arm body is a full expression.
#[test]
fn match_arm_bodies_are_full_expressions() {
    let src = "let x = match true with | true => 1.0 + 2.0 | false => 0.0";
    let program = functor_lang::parse(src).unwrap();
    let Item::Let(decl) = &program.items[0] else {
        panic!("expected a let declaration");
    };
    let ExprKind::Match { arms, .. } = &decl.value.kind else {
        panic!("expected a match");
    };
    assert_eq!(arms.len(), 2);
    assert!(matches!(&arms[0].body.kind, ExprKind::Binary { .. }));
}

// --- Tuples ---

#[test]
fn error_one_element_tuple() {
    assert_eq!(
        parse_err("let a = (1.0,)"),
        (
            "a tuple needs at least two elements (`(e)` is grouping)".to_string(),
            1,
            9
        )
    );
}

#[test]
fn error_one_element_tuple_pattern() {
    assert_eq!(
        parse_err("let f = (t) => match t with | (a) => a"),
        (
            "a tuple pattern needs at least two elements".to_string(),
            1,
            31
        )
    );
}

#[test]
fn error_mut_cannot_destructure() {
    assert_eq!(
        parse_err("let f = (t) => let mut (a, b) = t in a"),
        (
            "`mut` cannot destructure — bind a name, or use plain `let`".to_string(),
            1,
            24
        )
    );
}

// --- Generic type declarations ---

#[test]
fn error_duplicate_type_parameter() {
    assert_eq!(
        parse_err("type Pair<'a, 'a> = { x: 'a }"),
        ("duplicate type parameter `'a`".to_string(), 1, 15)
    );
}

#[test]
fn error_non_typevar_type_parameter() {
    assert_eq!(
        parse_err("type Box<T> = | Full(v: T)"),
        (
            "expected a type parameter (e.g. `'a`), found `T`".to_string(),
            1,
            10
        )
    );
}

// --- List patterns + cons ---

#[test]
fn error_spread_needs_a_leading_element() {
    assert_eq!(
        parse_err("let a = [..xs]"),
        (
            "`..tail` needs at least one element before it (`[x, ..xs]`)".to_string(),
            1,
            10
        )
    );
}

// --- Boolean operators `&&` / `||` / `not` ---

/// Parse `let v = <expr>` and hand back the expression's kind.
fn parsed_value(src: &str) -> ExprKind {
    let program = functor_lang::parse(src).unwrap();
    let Item::Let(decl) = program.items.into_iter().next().unwrap() else {
        panic!("expected a let declaration");
    };
    decl.value.kind
}

#[test]
fn and_binds_tighter_than_or() {
    // `a || b && c` parses as `a || (b && c)`.
    use functor_lang::ast::LogicalOp;
    let ExprKind::Logical { op: LogicalOp::Or, rhs, .. } = parsed_value("let v = a || b && c")
    else {
        panic!("expected an `||` at the root");
    };
    assert!(
        matches!(rhs.kind, ExprKind::Logical { op: LogicalOp::And, .. }),
        "expected `&&` on the right of `||`, got {:?}",
        rhs.kind
    );
}

#[test]
fn logical_is_looser_than_comparison() {
    // `a > b && c > d` parses as `(a > b) && (c > d)`.
    use functor_lang::ast::LogicalOp;
    let ExprKind::Logical { op: LogicalOp::And, lhs, .. } =
        parsed_value("let v = a > b && c > d")
    else {
        panic!("expected an `&&` at the root");
    };
    assert!(
        matches!(lhs.kind, ExprKind::Binary { .. }),
        "expected a comparison on the left of `&&`, got {:?}",
        lhs.kind
    );
}

#[test]
fn not_is_looser_than_comparison() {
    // `not a == b` parses as `not (a == b)`, so the child is the comparison.
    let ExprKind::Not(inner) = parsed_value("let v = not a == b") else {
        panic!("expected a `not` at the root");
    };
    assert!(
        matches!(inner.kind, ExprKind::Binary { .. }),
        "expected a comparison under `not`, got {:?}",
        inner.kind
    );
}

#[test]
fn bare_ampersand_is_a_lex_error() {
    let (message, _, _) = parse_err("let v = a & b");
    assert_eq!(message, "unexpected character `&`");
}
