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
use crate::ir::{Expr, ExprKind, Module};
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
            let ty = types
                .get(&def.value.id.raw())
                .cloned()
                .unwrap_or(Type::Unknown);
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
    let ty = types.get(&expr.id.raw()).cloned().unwrap_or(Type::Unknown);
    match &expr.kind {
        ExprKind::Local { name, .. } | ExprKind::Global(name) => {
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
                let value_ty = types.get(&value.id.raw()).cloned().unwrap_or(Type::Unknown);
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
        _ => {
            consider(expr.span, ty.to_string());
            for child in children(expr) {
                visit(child, types, consider);
            }
        }
    }
}

/// The direct sub-expressions of a node (Lambda handled by the caller).
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
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Local { .. }
        | ExprKind::LocalMut { .. }
        | ExprKind::Global(_)
        | ExprKind::External(_)
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
