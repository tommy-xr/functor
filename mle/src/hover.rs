//! Hover/quick-info: what to show for a byte offset in a checked module —
//! the language-aware half of the LSP's `textDocument/hover` (Track D). The
//! editor server converts positions and speaks the protocol; this module
//! decides content, so it is unit-testable without an editor.
//!
//! The answer is the **innermost** expression (or lambda parameter, or
//! top-level definition name) whose span contains the offset, rendered as
//! `name : Type` for named things and bare `Type` otherwise. Types come from
//! the checker's per-expression table ([`crate::types::ExprTypes`]) — in
//! unannotated code they are honestly `Unknown` (the language is gradually
//! typed; see [`crate::types`]).

use crate::ast::TypeName;
use crate::ir::{Expr, ExprKind, Module, Pattern, PatternKind};
use crate::span::Span;
use crate::types::{ExprTypes, Type};

/// The hover for `offset`, if any: the span the hover applies to plus its
/// text (one line, `name : Type` shaped).
pub fn hover_text(module: &Module, types: &ExprTypes, offset: usize) -> Option<(Span, String)> {
    let mut best: Option<(Span, String)> = None;
    let mut consider = |span: Span, text: String| {
        if span.start <= offset && offset < span.end {
            let tighter = match &best {
                Some((held, _)) => span.end - span.start <= held.end - held.start,
                None => true,
            };
            if tighter {
                best = Some((span, text));
            }
        }
    };

    for def in &module.defs {
        // The definition name itself: inside the def's span but before its
        // value (i.e. the `let name =` part).
        if def.span.start <= offset && offset < def.value.span.start {
            let ty = types.expr(def.value.id).cloned().unwrap_or(Type::Unknown);
            consider(
                Span::new(def.span.start, def.value.span.start),
                format!("{} : {ty}", def.name),
            );
        }
        visit(&def.value, types, &mut consider);
    }
    best
}

fn visit(expr: &Expr, types: &ExprTypes, consider: &mut impl FnMut(Span, String)) {
    let ty = types.expr(expr.id).cloned().unwrap_or(Type::Unknown);
    match &expr.kind {
        ExprKind::Local { name, .. } | ExprKind::Global(name) => {
            consider(expr.span, format!("{name} : {ty}"));
        }
        // A constructor reference hovers with its signature
        // (`Circle : (Float) => Shape`; nullary: `Point : Shape`).
        ExprKind::Ctor { name, .. } => {
            consider(expr.span, format!("{name} : {ty}"));
        }
        ExprKind::LocalMut { name, .. } => {
            consider(expr.span, format!("mut {name} : {ty}"));
        }
        ExprKind::External(path) => {
            consider(expr.span, format!("{} : {ty}", path.join(".")));
        }
        ExprKind::Let {
            name,
            mutable,
            value,
            body,
            ..
        } => {
            // The binder-name region (`let [mut] name =`), like def names.
            if expr.span.start < value.span.start {
                let value_ty = types.expr(value.id).cloned().unwrap_or(Type::Unknown);
                let label = if *mutable {
                    format!("mut {name} : {value_ty}")
                } else {
                    format!("{name} : {value_ty}")
                };
                consider(Span::new(expr.span.start, value.span.start), label);
            }
            consider(expr.span, ty.to_string());
            visit(value, types, consider);
            visit(body, types, consider);
        }
        ExprKind::Lambda { params, body, .. } => {
            consider(expr.span, ty.to_string());
            for param in params.iter() {
                let shown = param
                    .ty
                    .as_ref()
                    .map(type_name_text)
                    .unwrap_or_else(|| "Unknown".to_string());
                consider(param.span, format!("{} : {shown}", param.name));
            }
            visit(body, types, consider);
        }
        ExprKind::Match { scrutinee, arms } => {
            consider(expr.span, ty.to_string());
            visit(scrutinee, types, consider);
            for arm in arms {
                pattern_vars(&arm.pattern, types, consider);
                visit(&arm.body, types, consider);
            }
        }
        _ => {
            consider(expr.span, ty.to_string());
            for child in children(expr) {
                visit(child, types, consider);
            }
        }
    }
}

/// Pattern variables hover like binder names: `name : Type`, from the
/// checker's binding table (a pattern variable has no expression node).
fn pattern_vars(pattern: &Pattern, types: &ExprTypes, consider: &mut impl FnMut(Span, String)) {
    match &pattern.kind {
        PatternKind::Var { binding, name } => {
            let ty = types.binding(*binding).cloned().unwrap_or(Type::Unknown);
            consider(pattern.span, format!("{name} : {ty}"));
        }
        PatternKind::Ctor { args, .. } => {
            for arg in args {
                pattern_vars(arg, types, consider);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// The direct sub-expressions of a node (Lambda and Match handled by the
/// caller).
fn children(expr: &Expr) -> Vec<&Expr> {
    match &expr.kind {
        ExprKind::Record(fields) => fields.iter().map(|f| &f.value).collect(),
        ExprKind::RecordUpdate { base, fields } => std::iter::once(base.as_ref())
            .chain(fields.iter().map(|f| &f.value))
            .collect(),
        ExprKind::List(items) => items.iter().collect(),
        ExprKind::FieldAccess { object, .. } => vec![object],
        ExprKind::Call { callee, args } => std::iter::once(callee.as_ref())
            .chain(args.iter())
            .collect(),
        ExprKind::Binary { lhs, rhs, .. } => vec![lhs, rhs],
        ExprKind::Neg(inner) => vec![inner],
        ExprKind::Let { value, body, .. } => vec![value, body],
        ExprKind::Assign { value, rest, .. } => vec![value, rest],
        ExprKind::Match { scrutinee, arms } => std::iter::once(scrutinee.as_ref())
            .chain(arms.iter().map(|arm| &arm.body))
            .collect(),
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Local { .. }
        | ExprKind::LocalMut { .. }
        | ExprKind::Global(_)
        | ExprKind::External(_)
        | ExprKind::Ctor { .. }
        | ExprKind::Lambda { .. } => vec![],
    }
}

/// Render a surface annotation (`List<Float>`) for param hovers, which have
/// no expression node of their own.
fn type_name_text(ty: &TypeName) -> String {
    if ty.args.is_empty() {
        return ty.name.clone();
    }
    let args: Vec<String> = ty.args.iter().map(type_name_text).collect();
    format!("{}<{}>", ty.name, args.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::check_with_types;

    fn hover_at(src: &str, needle: &str) -> Option<String> {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let (_, types) = check_with_types(&module);
        let offset = src.find(needle).expect("needle present");
        hover_text(&module, &types, offset).map(|(_, text)| text)
    }

    #[test]
    fn hover_on_a_builtin_shows_its_signature() {
        let text = hover_at("let f = (xs) => xs |> List.maximum", "List.maximum").unwrap();
        assert_eq!(text, "List.maximum : (List<Float>) => Float");
    }

    #[test]
    fn hover_on_an_annotated_param_shows_its_type() {
        let text = hover_at("let f = (score: Float): Bool => score > 1.0", "score:").unwrap();
        assert_eq!(text, "score : Float");
    }

    #[test]
    fn hover_on_a_local_reference_shows_the_inferred_type() {
        let text = hover_at("let f = (score: Float): Bool => score > 1.0", "score >").unwrap();
        assert_eq!(text, "score : Float");
    }

    #[test]
    fn hover_on_a_global_reference_shows_its_signature() {
        let src = "let double = (x: Float): Float => x * 2.0\nlet main = () => double(2.0)";
        let text = hover_at(src, "double(2.0)").unwrap();
        assert_eq!(text, "double : (Float) => Float");
    }

    #[test]
    fn hover_on_a_mut_binding_says_mut() {
        let src = "let f = (x: Float) => let mut a = x in a := a + 1.0; a";
        let offset = src.rfind("a").unwrap();
        let module = crate::lower(crate::parse(src).unwrap()).unwrap();
        let (_, types) = check_with_types(&module);
        let (_, text) = hover_text(&module, &types, offset).unwrap();
        assert_eq!(text, "mut a : Float");
    }

    #[test]
    fn hover_on_the_definition_name_shows_its_type() {
        let text = hover_at("let threshold = 10", "threshold").unwrap();
        assert_eq!(text, "threshold : Float");
    }

    #[test]
    fn unannotated_code_hovers_as_unknown() {
        let text = hover_at("let f = (x) => x", "x)").unwrap();
        assert_eq!(text, "x : Unknown");
    }

    #[test]
    fn no_hover_outside_any_node() {
        let src = "let a = 1.0";
        let module = crate::lower(crate::parse(src).unwrap()).unwrap();
        let (_, types) = check_with_types(&module);
        assert!(hover_text(&module, &types, src.len()).is_none());
    }

    // --- Variants + match (B5 part 1) ---

    const SHAPE: &str = "type Shape = | Circle(r: Float) | Point\n";

    #[test]
    fn hover_on_a_ctor_shows_its_signature() {
        let src = format!("{SHAPE}let c = Circle(2.0)");
        let text = hover_at(&src, "Circle(2.0)").unwrap();
        assert_eq!(text, "Circle : (Float) => Shape");
    }

    #[test]
    fn hover_on_a_nullary_ctor_shows_the_variant_type() {
        let src = format!("{SHAPE}let p = Point");
        let offset = src.rfind("Point").unwrap();
        let module = crate::lower(crate::parse(&src).unwrap()).unwrap();
        let (_, types) = check_with_types(&module);
        let (_, text) = hover_text(&module, &types, offset).unwrap();
        assert_eq!(text, "Point : Shape");
    }

    #[test]
    fn hover_on_a_pattern_var_shows_the_field_type() {
        let src =
            format!("{SHAPE}let f = (s: Shape): Float => match s with | Circle(r) => r | _ => 0.0");
        let text = hover_at(&src, "r) =>").unwrap();
        assert_eq!(text, "r : Float");
    }

    #[test]
    fn hover_on_a_catch_all_var_shows_the_scrutinee_type() {
        let src = format!("{SHAPE}let f = (s: Shape): Float => match s with | other => 0.0");
        let text = hover_at(&src, "other =>").unwrap();
        assert_eq!(text, "other : Shape");
    }

    #[test]
    fn hover_on_a_match_shows_its_joined_type() {
        let src = "let f = (b: Bool) => match b with | true => 1.0 | false => 0.0";
        let text = hover_at(src, "match").unwrap();
        assert_eq!(text, "Float");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::types::check_with_types;

    fn hover_at(src: &str, needle: &str) -> String {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let (_, types) = check_with_types(&module);
        let offset = src.find(needle).expect("needle present");
        hover_text(&module, &types, offset)
            .map(|(_, text)| text)
            .expect("hover present")
    }

    // [AGREED review] literals in structurally-checked positions hover with
    // their checked type, not Unknown.
    #[test]
    fn checked_record_literal_hovers_with_its_type() {
        let src = "type P = { x: Float }\nlet mk = (): P => { x: 1.0 }";
        assert_eq!(hover_at(src, "{ x: 1.0 }"), "P");
    }

    #[test]
    fn checked_list_literal_hovers_with_its_type() {
        let src = "let f = (): List<Float> => [1.0, 2.0]";
        assert_eq!(hover_at(src, "[1.0"), "List<Float>");
    }

    // [review] inner nodes of a binary spine are recorded too.
    #[test]
    fn inner_binary_nodes_hover_with_their_type() {
        let src = "let f = (x: Float) => x + x + x";
        let inner_plus = src.find('+').unwrap();
        let module = crate::lower(crate::parse(src).unwrap()).unwrap();
        let (_, types) = check_with_types(&module);
        let (_, text) = hover_text(&module, &types, inner_plus).unwrap();
        assert_eq!(text, "Float");
    }

    // [review] let/mut binder names hover with `name : Type`.
    #[test]
    fn let_binder_name_hovers_named() {
        let src = "let f = (x: Float) => let y = x in y + 1.0";
        assert_eq!(hover_at(src, "y ="), "y : Float");
    }

    #[test]
    fn mut_binder_name_hovers_named() {
        let src = "let f = (x: Float) => let mut a = x in a := a + 1.0; a";
        assert_eq!(hover_at(src, "mut a ="), "mut a : Float");
    }
}
