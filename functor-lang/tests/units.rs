//! Unit-suffix literals (`90deg`, `0.5s`, `16px`) and the `unit` declaration
//! that gives a suffix its meaning: lexing, parsing, resolution, checking,
//! and evaluation.
//!
//! The design is deliberately layered: the LEXER is dumb (it knows a literal
//! carries identifier characters, never which units exist), and LOWERING
//! desugars each suffixed literal to exactly the call the `unit` names — so
//! everything downstream sees an ordinary call.

use functor_lang::ast::{ExprKind, Item};
use functor_lang::lexer::{lex, TokenKind};
use functor_lang::{RunOutcome, Tracing};

// ------------------------------------------------------------------- lexing

/// Identifier characters TOUCHING a numeric literal are one token carrying
/// the value and the spelled suffix — integers and decimals alike.
#[test]
fn digits_followed_by_identifier_chars_lex_as_one_token() {
    let tokens = lex("90deg 0.5rad 16px", 0).expect("lexes");
    let kinds: Vec<&TokenKind> = tokens.iter().map(|t| &t.kind).collect();
    assert_eq!(
        kinds[..3],
        [
            &TokenKind::NumberUnit(90.0, "deg".to_string()),
            &TokenKind::NumberUnit(0.5, "rad".to_string()),
            &TokenKind::NumberUnit(16.0, "px".to_string()),
        ]
    );
}

/// Adjacency is the whole rule: a space makes it a number and a name again,
/// exactly as before this feature existed.
#[test]
fn a_space_keeps_the_number_and_the_name_apart() {
    let tokens = lex("90 deg", 0).expect("lexes");
    assert_eq!(tokens[0].kind, TokenKind::Number(90.0));
    assert_eq!(tokens[1].kind, TokenKind::Ident("deg".to_string()));
}

/// A plain literal is untouched — including one followed by a non-identifier
/// character (there is no scientific notation, so nothing else is ambiguous).
#[test]
fn plain_literals_are_unchanged() {
    let tokens = lex("1.5 + 2", 0).expect("lexes");
    assert_eq!(tokens[0].kind, TokenKind::Number(1.5));
    assert_eq!(tokens[2].kind, TokenKind::Number(2.0));
}

/// The suffix spans the whole identifier, digits included (`16px2`), so a
/// typo can never silently split into "number, suffix, stray name".
#[test]
fn a_suffix_runs_to_the_end_of_the_identifier() {
    let tokens = lex("16px2", 0).expect("lexes");
    assert_eq!(tokens[0].kind, TokenKind::NumberUnit(16.0, "px2".to_string()));
}

/// The lexer never produces a negative literal (as for plain numbers) — the
/// minus is a separate token, and the PARSER folds it into the literal.
#[test]
fn unary_minus_is_a_separate_token() {
    let tokens = lex("-2.5px", 0).expect("lexes");
    assert_eq!(tokens[0].kind, TokenKind::Minus);
    assert_eq!(
        tokens[1].kind,
        TokenKind::NumberUnit(2.5, "px".to_string())
    );
}

/// A prefix minus folds INTO the literal (the number-pattern precedent), so
/// `-90deg` is `Angle.degrees(-90.0)` rather than a negation of the branded
/// value — which would have no meaning.
#[test]
fn a_prefix_minus_folds_into_the_literal() {
    let program = functor_lang::parse("let a = -2.5px\n").expect("parses");
    let Item::Let(decl) = &program.items[0] else {
        panic!("expected a let");
    };
    match &decl.value.kind {
        ExprKind::NumberUnit { value, suffix } => {
            assert_eq!((*value, suffix.as_str()), (-2.5, "px"))
        }
        other => panic!("expected a negative unit literal, got {other:?}"),
    }
}

/// Subtraction is untouched: the minus between two operands stays binary.
#[test]
fn subtraction_of_suffixed_literals_still_parses_as_binary() {
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               let unwrap = (p: Px): float => match p with | Px(n) => n\n\
               let main = () => unwrap(5px) - unwrap(2px)\n";
    assert_eq!(main_result(src), "3");
}

// ------------------------------------------------------------------ parsing

#[test]
fn a_unit_item_parses_with_a_qualified_target() {
    let program = functor_lang::parse("unit deg = Angle.degrees\n").expect("parses");
    let Item::Unit(decl) = &program.items[0] else {
        panic!("expected a unit item, got {:?}", program.items[0]);
    };
    assert_eq!(decl.suffix, "deg");
    assert_eq!(decl.target, vec!["Angle".to_string(), "degrees".to_string()]);
}

#[test]
fn a_unit_item_parses_with_a_bare_constructor_target() {
    let program =
        functor_lang::parse("type Px = | Px(value: float)\nunit px = Px\n").expect("parses");
    let Item::Unit(decl) = &program.items[1] else {
        panic!("expected a unit item");
    };
    assert_eq!((decl.suffix.as_str(), decl.target.len()), ("px", 1));
}

/// A suffixed literal reaches the AST with both halves intact (the parser
/// does not resolve it — that is lowering's job).
#[test]
fn a_suffixed_literal_parses_as_a_literal_with_its_suffix() {
    let program = functor_lang::parse("let a = 90deg\n").expect("parses");
    let Item::Let(decl) = &program.items[0] else {
        panic!("expected a let");
    };
    match &decl.value.kind {
        ExprKind::NumberUnit { value, suffix } => {
            assert_eq!((*value, suffix.as_str()), (90.0, "deg"))
        }
        other => panic!("expected a unit literal, got {other:?}"),
    }
}

fn parse_err(src: &str) -> String {
    functor_lang::parse(src)
        .err()
        .unwrap_or_else(|| panic!("`{src}` should not parse"))
        .message
}

#[test]
fn malformed_unit_declarations_are_targeted_parse_errors() {
    assert!(
        parse_err("unit = Angle.degrees\n").contains("a unit suffix after `unit`"),
        "{}",
        parse_err("unit = Angle.degrees\n")
    );
    assert!(
        parse_err("unit deg\n").contains("`=` after the unit suffix"),
        "{}",
        parse_err("unit deg\n")
    );
    assert!(
        parse_err("unit deg =\n").contains("the name a unit literal calls"),
        "{}",
        parse_err("unit deg =\n")
    );
    // A leading underscore could never follow digits, so it is refused where
    // it is written rather than becoming an undeclarable suffix.
    assert!(
        parse_err("unit _px = Px\n").contains("starts with a letter"),
        "{}",
        parse_err("unit _px = Px\n")
    );
}

/// `unit` is contextual, like `open` / `expect` / `module`: it only means a
/// declaration in item position, so the name stays ordinary everywhere else.
#[test]
fn unit_is_still_usable_as_a_name() {
    let program = functor_lang::parse("let unit = 1.0\nlet main = () => unit\n").expect("parses");
    assert_eq!(program.items.len(), 2);
}

// --------------------------------------------------------- lowering + check

fn check_src(src: &str) -> Vec<String> {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    functor_lang::check(&module)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn lower_err(src: &str) -> String {
    let program = functor_lang::parse(src).expect("source should parse");
    functor_lang::lower(program)
        .err()
        .expect("source should not lower")
        .message
}

/// An undeclared suffix names the units that DO exist — the teaching error.
#[test]
fn an_unknown_suffix_lists_the_declared_units() {
    let message = lower_err(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let a = 90deg\n",
    );
    assert!(message.contains("unknown unit `deg`"), "{message}");
    assert!(message.contains("declared units: `px`"), "{message}");
}

/// …and with no units at all, it says how to declare one.
#[test]
fn an_unknown_suffix_without_units_teaches_the_declaration() {
    let message = lower_err("let a = 90deg\n");
    assert!(message.contains("no units are declared"), "{message}");
    assert!(message.contains("unit deg = SomeFn"), "{message}");
}

/// A suffix is declared once — units are project-wide, like constructors.
#[test]
fn a_duplicate_suffix_is_an_error() {
    let message = lower_err(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit px = Px\n",
    );
    assert!(message.contains("duplicate unit `px`"), "{message}");
}

/// The target must be exactly a one-float function; anything else is a check
/// error at the DECLARATION, not a puzzle at every use site.
#[test]
fn a_target_that_is_not_a_float_function_is_a_check_error() {
    let diags = check_src("let two = 2.0\nunit x = two\n");
    assert!(
        diags.iter().any(|d| d.contains("`unit x`") && d.contains("got float")),
        "{diags:?}"
    );

    let diags = check_src(
        "let pair = (a: float, b: float): float => a + b\n\
         unit x = pair\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`unit x`") && d.contains("(float, float) => float")),
        "{diags:?}"
    );
}

/// An unknown target name fails at the declaration, like any other name.
#[test]
fn an_unknown_target_is_a_lowering_error() {
    let message = lower_err("unit px = Nope\n");
    assert!(message.contains("unknown name `Nope`"), "{message}");
}

/// A declared unit types exactly as the call it desugars to.
#[test]
fn a_suffixed_literal_checks_as_its_target_type() {
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let width: Px = 16px\n",
    );
    assert!(diags.is_empty(), "{diags:?}");

    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let width: float = 16px\n",
    );
    assert!(
        diags.iter().any(|d| d.contains("expected float, got Px")),
        "{diags:?}"
    );
}

/// A bare number where a branded value belongs now teaches BOTH spellings,
/// quoting the literal the source actually wrote.
#[test]
fn a_bare_number_in_a_branded_position_teaches_the_suffix() {
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let width: Px = 16.0\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("write `16px` or `Px(16.0)`")),
        "{diags:?}"
    );
}

// ---------------------------------------------------------------- evaluation

fn main_result(src: &str) -> String {
    let program = functor_lang::parse(src).expect("source should parse");
    let module = functor_lang::lower(program).expect("source should lower");
    match functor_lang::run(&module, Tracing::Off) {
        Ok(record) => match record.outcome {
            RunOutcome::Main(value) => value.to_string(),
            RunOutcome::Bindings(_) => panic!("expected a main result"),
        },
        Err(failure) => panic!("run failed: {}", failure.error.message),
    }
}

/// A user brand, end to end: the literal builds the same value the
/// handwritten call does.
#[test]
fn a_user_declared_unit_evaluates_as_the_handwritten_call() {
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               let main = () => 16px\n";
    assert_eq!(main_result(src), "Px(16)");
    assert_eq!(
        main_result(src),
        main_result(
            "type Px = | Px(value: float)\n\
             unit px = Px\n\
             let main = () => Px(16.0)\n"
        )
    );
}

/// Unary minus applies to the built value, so a brand can hold a negative.
#[test]
fn unary_minus_negates_a_suffixed_literal_argument() {
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               let unwrap = (p: Px): float => match p with | Px(n) => n\n\
               let main = () => unwrap(-2.5px)\n";
    assert_eq!(main_result(src), "-2.5");
}

/// A plain function target works too — nothing about units is brand-specific.
#[test]
fn a_plain_function_target_works() {
    let src = "let twice = (n: float): float => n * 2.0\n\
               unit x = twice\n\
               let main = () => 21x\n";
    assert_eq!(main_result(src), "42");
}
