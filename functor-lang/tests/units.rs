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

/// The desugared call splits the literal's span between its halves, so the
/// editor teaches instead of showing three nodes fighting over one span:
/// the digits hover as `float`, the suffix as the function it calls.
#[test]
fn hovering_a_suffixed_literal_shows_both_halves() {
    let src = "type Px = | Px(value: float)\nunit px = Px\nlet width = 16px\n";
    let program = functor_lang::parse(src).expect("parses");
    let module = functor_lang::lower(program).expect("lowers");
    let (_diags, types) = functor_lang::check_with_types(&module);
    let literal = src.find("16px").expect("the literal");
    let hover = |offset: usize| {
        functor_lang::hover::hover_text(&module, &types, offset).map(|(_, text)| text)
    };
    assert_eq!(hover(literal).as_deref(), Some("float"), "the digits");
    assert_eq!(
        hover(literal + 2).as_deref(),
        Some("Px : (float) => Px"),
        "the suffix"
    );
}

/// A keyword touching a number is NOT a unit: `1else 2` lexes as it always
/// has. (A keyword could never be declared as a suffix anyway.)
#[test]
fn a_keyword_touching_a_number_is_not_a_suffix() {
    let tokens = lex("1else", 0).expect("lexes");
    assert_eq!(tokens[0].kind, TokenKind::Number(1.0));
    assert_eq!(tokens[1].kind, TokenKind::Else);
    let src = "let pick = (c: bool): float => if c then 1else 2.0\n";
    assert!(functor_lang::parse(src).is_ok(), "`1else 2.0` still parses");
}

/// `1_000` is a digit-separator attempt, not a unit — say so.
#[test]
fn a_digit_separator_is_taught_not_reported_as_a_unit() {
    let message = lower_err("let a = 1_000\n");
    assert!(
        message.contains("no digit separators"),
        "{message}"
    );
}

/// A plain function target works too — nothing about units is brand-specific.
#[test]
fn a_plain_function_target_works() {
    let src = "let twice = (n: float): float => n * 2.0\n\
               unit x = twice\n\
               let main = () => 21x\n";
    assert_eq!(main_result(src), "42");
}

// -------------------------------------------------- operators (`unit px (+)`)

/// The declaration form: a suffix, an operator in parens, and an
/// implementation (a name, or — in a `.fun` — a lambda).
#[test]
fn an_operator_declaration_parses_with_a_name_target() {
    let program = functor_lang::parse("unit deg (+) = Angle.add\n").expect("parses");
    let Item::UnitOp(decl) = &program.items[0] else {
        panic!("expected a unit operator item, got {:?}", program.items[0]);
    };
    assert_eq!((decl.suffix.as_str(), decl.op.symbol()), ("deg", "+"));
}

#[test]
fn all_four_arithmetic_operators_are_declarable() {
    for symbol in ["+", "-", "*", "/"] {
        let src = format!("unit px ({symbol}) = f\n");
        let program = functor_lang::parse(&src).expect("parses");
        let Item::UnitOp(decl) = &program.items[0] else {
            panic!("expected a unit operator item");
        };
        assert_eq!(decl.op.symbol(), symbol);
    }
}

/// A lambda implementation is an ordinary expression, so it parses like one.
#[test]
fn an_operator_declaration_takes_a_lambda() {
    let program = functor_lang::parse("unit px (+) = (a, b) => a\n").expect("parses");
    assert!(matches!(&program.items[0], Item::UnitOp(_)));
}

#[test]
fn malformed_operator_declarations_are_targeted_parse_errors() {
    // Six operators are declarable — the four arithmetic ones, `==`, and `<`.
    let message = parse_err("unit px (&&) = both\n");
    assert!(
        message.contains("`+`, `-`, `*`, `/`, `==`, or `<`"),
        "{message}"
    );
    // The DERIVED comparisons name the base they come from rather than
    // listing everything again.
    let message = parse_err("unit px (!=) = ne\n");
    assert!(message.contains("`!=` is derived from `==`"), "{message}");
    for src in ["unit px (>) = gt\n", "unit px (<=) = le\n", "unit px (>=) = ge\n"] {
        let message = parse_err(src);
        assert!(message.contains("are derived from `<`"), "{message}");
    }
    let message = parse_err("unit px (+ = add\n");
    assert!(message.contains("`)` after the operator"), "{message}");
    let message = parse_err("unit px (+)\n");
    assert!(
        message.contains("`=` after the declared operator"),
        "{message}"
    );
}

/// An operator on a suffix nobody declared fails at the declaration, with the
/// same teaching the literal gets.
#[test]
fn an_operator_on_an_unknown_suffix_is_a_lowering_error() {
    let message = lower_err(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit em (+) = Px\n",
    );
    assert!(message.contains("unknown unit `em`"), "{message}");
}

const PX: &str = "type Px = | Px(value: float)\n\
                  unit px = Px\n\
                  let unwrap = (p: Px): float => match p with | Px(n) => n\n\
                  unit px (+) = (a, b) => Px(unwrap(a) + unwrap(b))\n\
                  unit px (*) = (a, k) => Px(unwrap(a) * k)\n";

/// The headline: a declared operator resolves on both operand positions and
/// evaluates to exactly the implementation's call.
#[test]
fn a_declared_operator_resolves_and_evaluates() {
    let src = format!("{PX}let main = () => 16px + 4px\n");
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(main_result(&src), "Px(20)");
}

/// The scalar form takes the brand on the left; multiplication also commutes,
/// so a bare number may lead.
#[test]
fn the_scalar_form_works_from_either_side_of_a_product() {
    let src = format!("{PX}let main = () => (3px * 2.0, 2.0 * 3px)\n");
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(main_result(&src), "(Px(6), Px(6))");
}

/// A branded operand keeps its brand: the result flows into a branded
/// position with no unwrapping.
#[test]
fn an_operator_result_keeps_the_brand() {
    let diags = check_src(&format!("{PX}let total: Px = 1px + 2px\n"));
    assert!(diags.is_empty(), "{diags:?}");
    let diags = check_src(&format!("{PX}let total: float = 1px + 2px\n"));
    assert!(
        diags.iter().any(|d| d.contains("expected float, got Px")),
        "{diags:?}"
    );
}

/// An operator the brand does NOT declare keeps the old teaching error, now
/// naming what the brand does have.
#[test]
fn an_undeclared_operator_names_the_declared_ones() {
    let diags = check_src(&format!("{PX}let bad = 1px - 2px\n"));
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`Px` declares `+`, `*`, but not `-`")),
        "{diags:?}"
    );
}

/// The operator belongs to the BRAND, not the suffix — so two suffixes of one
/// brand share one implementation, and declaring it twice is an error.
#[test]
fn one_brand_declares_each_operator_once() {
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit em = Px\n\
         unit px (+) = add\n\
         unit em (+) = add\n\
         let add = (a: Px, b: Px): Px => a\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("duplicate operator") && d.contains("through `unit px`")),
        "{diags:?}"
    );

    // …and the OTHER suffix of that brand uses the one declaration.
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit em = Px\n\
         unit px (+) = add\n\
         let add = (a: Px, b: Px): Px => a\n\
         let total: Px = 1px + 2em\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// The implementation is checked against the operator's declared SHAPE, so a
/// wrong one is an error at the declaration rather than a puzzle at a use.
#[test]
fn a_wrong_shaped_implementation_is_rejected() {
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit px (+) = scale\n\
         let scale = (a: Px, k: float): Px => a\n",
    );
    assert!(
        diags.iter().any(|d| d.contains("`unit px (+)`")),
        "{diags:?}"
    );

    // The scalar form is the mirror image: `*` does NOT take two brands.
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         unit px (*) = add\n\
         let add = (a: Px, b: Px): Px => a\n",
    );
    assert!(
        diags.iter().any(|d| d.contains("`unit px (*)`")),
        "{diags:?}"
    );
}

/// Ad-hoc overloading has one hard rule: a node whose operands inference
/// never pinned down is a teaching error asking for an annotation — never a
/// silent float guess.
#[test]
fn an_unresolvable_operator_asks_for_an_annotation() {
    let diags = check_src(&format!("{PX}let add = (a, b) => a + b\n"));
    assert!(
        diags
            .iter()
            .any(|d| d.contains("could be float arithmetic or `Px` arithmetic")
                && d.contains("annotate an operand")),
        "{diags:?}"
    );

    // The annotation the message asks for fixes it, on either operand.
    let diags = check_src(&format!("{PX}let add = (a: Px, b) => a + b\n"));
    assert!(diags.is_empty(), "{diags:?}");
    let diags = check_src(&format!("{PX}let add = (a, b: Px) => a + b\n"));
    assert!(diags.is_empty(), "{diags:?}");
    // …and so does anything else that pins a type — an annotated result, or
    // a float operand — so ordinary float code stays untouched.
    let diags = check_src(&format!("{PX}let add = (a, b): float => a + b\n"));
    assert!(diags.is_empty(), "{diags:?}");
    let diags = check_src(&format!("{PX}let add = (a) => a + 1.0\n"));
    assert!(diags.is_empty(), "{diags:?}");
}

/// `v * v` is not ambiguous even with a scalar `*` declared: the scalar form's
/// operands have DIFFERENT types, so one unsolved operand used twice can only
/// be float. (`v + v` genuinely could be either, and still asks.)
#[test]
fn squaring_one_unsolved_operand_is_not_ambiguous() {
    let diags = check_src(&format!("{PX}let sq = (v) => v * v\n"));
    assert!(diags.is_empty(), "{diags:?}");
    let diags = check_src(&format!("{PX}let double = (v) => v + v\n"));
    assert!(
        diags.iter().any(|d| d.contains("annotate an operand")),
        "{diags:?}"
    );
}

/// With no operator declared anywhere, arithmetic infers exactly as it always
/// has: the ambiguity error can only fire where an ambiguity exists.
#[test]
fn unannotated_arithmetic_is_untouched_without_unit_operators() {
    let diags = check_src("let add = (a, b) => a + b\nlet main = () => add(1.0, 2.0)\n");
    assert!(diags.is_empty(), "{diags:?}");
}

/// Top-level constants are evaluated EAGERLY, before anything else runs, so
/// the dispatch table has to exist by then: branded arithmetic in a top-level
/// initializer must work, not die at load. [xreview: Critical]
#[test]
fn a_top_level_constant_may_use_branded_arithmetic() {
    let src = format!("{PX}let total = 16px + 4px\nlet main = () => unwrap(total)\n");
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(main_result(&src), "20");
}

/// …including when the implementation is a top-level NAME rather than a
/// lambda: like every other global reference it stays late-bound, so it obeys
/// exactly the same "an initializer may only use globals defined above it"
/// rule — and says so when it doesn't.
#[test]
fn a_named_implementation_is_late_bound_like_any_global() {
    let px = "type Px = | Px(value: float)\n\
              unit px = Px\n\
              unit px (+) = addPx\n\
              let unwrap = (p: Px): float => match p with | Px(n) => n\n\
              let addPx = (a: Px, b: Px): Px => Px(unwrap(a) + unwrap(b))\n";
    let src = format!("{px}let total = 16px + 4px\nlet main = () => unwrap(total)\n");
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(main_result(&src), "20");

    // The implementation defined BELOW the constant that uses it: the
    // language's ordinary eager-initializer rule, with a message that says so.
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               unit px (+) = addPx\n\
               let total = 16px + 4px\n\
               let addPx = (a: Px, b: Px): Px => a\n";
    let program = functor_lang::parse(src).expect("parses");
    let module = functor_lang::lower(program).expect("lowers");
    let failure = functor_lang::run(&module, Tracing::Off)
        .err()
        .expect("`addPx` is not defined yet");
    assert!(
        failure.error.message.contains("used before its definition"),
        "{}",
        failure.error.message
    );
}

/// An annotated RESULT decides the node on its own: `+` stays inside its
/// brand, so `(a, b): Px => a + b` needs no operand annotation. [xreview:
/// High]
#[test]
fn an_annotated_result_resolves_the_operands() {
    let diags = check_src(&format!("{PX}let add = (a, b): Px => a + b\n"));
    assert!(diags.is_empty(), "{diags:?}");
    // A binding annotation on the def works the same way.
    let diags = check_src(&format!("{PX}let add: (Px, Px) => Px = (a, b) => a + b\n"));
    assert!(diags.is_empty(), "{diags:?}");
    // (A helper whose type is only pinned by a LATER call site is still
    // ambiguous — it generalizes first, the ordinary let-polymorphism rule.)
}

/// A unit whose own constructor is a top-level `let` cannot be probed before
/// the defs run — the table is completed as the defs land, so an initializer
/// below the constructor still gets its operators. [xreview: High]
#[test]
fn a_unit_built_by_a_top_level_function_still_dispatches() {
    let src = "type Px = | Px(value: float)\n\
               let make = (n: float): Px => Px(n)\n\
               unit px = make\n\
               let unwrap = (p: Px): float => match p with | Px(n) => n\n\
               unit px (+) = (a, b) => make(unwrap(a) + unwrap(b))\n\
               let total = 1px + 2px\n\
               let main = () => unwrap(total)\n";
    assert!(check_src(src).is_empty(), "{:?}", check_src(src));
    assert_eq!(main_result(src), "3");
}

/// The interpreter dispatches on a value's runtime TAG, so a brand whose
/// values carry none (a record) or carry several (a multi-constructor type)
/// is refused at the declaration instead of checking clean and failing at
/// run time. [xreview: High]
#[test]
fn a_brand_must_be_distinguishable_at_run_time() {
    let diags = check_src(
        "type Length = | Px(value: float) | Em(value: float)\n\
         unit px = Px\n\
         unit px (+) = add\n\
         let add = (a: Length, b: Length): Length => a\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("distinguishable at run time") && d.contains("`Length`")),
        "{diags:?}"
    );

    let diags = check_src(
        "type Px = { value: float }\n\
         let px = (n: float): Px => { value: n }\n\
         unit px2 = px\n\
         unit px2 (+) = add\n\
         let add = (a: Px, b: Px): Px => a\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("distinguishable at run time")),
        "{diags:?}"
    );
}

/// A brand's operator is dispatched by the INTERPRETER too (`run` does not
/// typecheck), and it produces the same value the handwritten call does.
#[test]
fn the_interpreter_dispatches_on_a_branded_operand() {
    let src = format!("{PX}let main = () => 16px + 4px\n");
    assert_eq!(
        main_result(&src),
        main_result(&format!(
            "{PX}let main = () => Px(unwrap(16px) + unwrap(4px))\n"
        ))
    );
}

/// The unchecked path refuses a duplicate declaration too, rather than
/// silently picking whichever came last. [xreview: Medium]
#[test]
fn the_interpreter_refuses_a_duplicate_declaration() {
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               unit em = Px\n\
               let add = (a: Px, b: Px): Px => a\n\
               unit px (+) = add\n\
               unit em (+) = add\n";
    let program = functor_lang::parse(src).expect("parses");
    let module = functor_lang::lower(program).expect("lowers");
    let failure = functor_lang::run(&module, Tracing::Off)
        .err()
        .expect("`+` is declared twice for one brand");
    assert!(
        failure.error.message.contains("duplicate operator"),
        "{}",
        failure.error.message
    );
}

/// …and a brand with no implementation for the operator errors at runtime
/// with the same teaching the checker gives.
#[test]
fn the_interpreter_teaches_when_no_implementation_exists() {
    let src = format!("{PX}let main = () => 16px - 4px\n");
    let program = functor_lang::parse(&src).expect("parses");
    let module = functor_lang::lower(program).expect("lowers");
    let failure = functor_lang::run(&module, Tracing::Off)
        .err()
        .expect("`-` is not declared for Px");
    assert!(
        failure
            .error
            .message
            .contains("`Px` declares `+`, `*`, but not `-`"),
        "{}",
        failure.error.message
    );
}

// ------------------------------------------------------ comparison on brands

/// A brand that declares the two COMPARISON bases. `unwrap` is the seam every
/// implementation goes through — the point being that a brand's comparison is
/// ordinary code, not a privileged builtin.
const ORD: &str = "type Px = | Px(value: float)\n\
                   unit px = Px\n\
                   let unwrap = (p: Px): float => match p with | Px(n) => n\n\
                   unit px (==) = (a, b) => unwrap(a) == unwrap(b)\n\
                   unit px (<) = (a, b) => unwrap(a) < unwrap(b)\n";

/// The headline: `==` and `<` resolve on a brand, and the four DERIVED
/// spellings come from them — `>` swaps `<`, `<=`/`>=` negate it, `!=`
/// negates `==`. All six are checked AND evaluated here, so a derivation that
/// disagreed with its base would show up as a wrong answer.
#[test]
fn a_brand_compares_and_orders_through_two_declarations() {
    let src = format!(
        "{ORD}let main = () => \
(16px == 16px, 16px == 4px, 16px != 4px, 4px < 16px, 16px > 4px, 4px <= 4px, 4px >= 16px)\n"
    );
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(
        main_result(&src),
        "(true, false, true, true, true, true, false)"
    );
}

/// A comparison answers `bool` whichever way it resolves, so a branded one
/// flows into `if` with no ceremony and does not infect the surrounding type.
#[test]
fn a_branded_comparison_is_an_ordinary_bool() {
    let src = format!("{ORD}let main = () => if 4px < 16px then 1.0 else 2.0\n");
    assert!(check_src(&src).is_empty(), "{:?}", check_src(&src));
    assert_eq!(main_result(&src), "1");
}

/// Ordering is refused on a brand that declares only equality — with the same
/// teaching arithmetic gets, naming the DECLARABLE base (`<`, never the
/// derived `>` the user actually wrote).
#[test]
fn an_undeclared_ordering_names_the_declarable_base() {
    let diags = check_src(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let unwrap = (p: Px): float => match p with | Px(n) => n\n\
         unit px (==) = (a, b) => unwrap(a) == unwrap(b)\n\
         let bad = 16px > 4px\n",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`Px` declares `==`, but not `<`")),
        "{diags:?}"
    );
}

/// A fully unannotated comparison is ambiguous once a brand declares `<` —
/// the same rule arithmetic has, for the same reason. A comparison's RESULT
/// is `bool` either way, so only an operand can decide it.
#[test]
fn an_unresolvable_comparison_asks_for_an_annotation() {
    let diags = check_src(&format!("{ORD}let less = (a, b) => a < b\n"));
    assert!(
        diags
            .iter()
            .any(|d| d.contains("could be float comparison") && d.contains("annotate an operand")),
        "{diags:?}"
    );
    // …and every ordinary way of pinning ONE operand resolves it. A side
    // already known to be a non-brand settles the node on the spot, which is
    // what keeps `Math.abs(d) < step` (and the `rate * dt` behind `step`)
    // decidable without any annotation at all.
    for tail in [
        "let less = (a: float, b) => a < b\n",
        "let less = (a, b: float) => a < b\n",
        "let less = (a) => a < 1.0\n",
        "let less = (a: Px, b) => a < b\n",
    ] {
        let src = format!("{ORD}{tail}");
        assert!(check_src(&src).is_empty(), "{tail}: {:?}", check_src(&src));
    }
}

/// The interpreter derives the same four spellings from the same two
/// implementations — `run` skips the checker, so this is the gradual seam
/// agreeing with the static one.
#[test]
fn the_interpreter_derives_the_orderings_too() {
    let src = format!("{ORD}let main = () => (16px > 4px, 4px <= 4px, 16px != 4px)\n");
    assert_eq!(main_result(&src), "(true, true, true)");
}

/// A brand with NO declared `==` keeps STRUCTURAL equality: a plain-data
/// variant compares as it always did, and nothing about `unit` changed that.
#[test]
fn a_brand_without_an_equality_declaration_stays_structural() {
    let src = "type Px = | Px(value: float)\n\
               unit px = Px\n\
               let main = () => (16px == 16px, 16px == 4px)\n";
    assert!(check_src(src).is_empty(), "{:?}", check_src(src));
    assert_eq!(main_result(src), "(true, false)");
}
