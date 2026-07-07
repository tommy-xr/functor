//! Inlay hints: inferred-type ghost text for a checked module — the
//! language-aware half of the LSP's `textDocument/inlayHint`. Like
//! [`crate::hover`], this module decides *content* (offset + label) and the
//! editor server speaks the protocol, so it is unit-testable without an
//! editor.
//!
//! We annotate **unannotated lambda parameters** with their inferred type
//! (`(x) => x + 1.0` shows `x: Float`), the same signature-inference feel as
//! Merlin / ReasonML. Types come from the checker's per-binding table
//! ([`crate::types::ExprTypes`]); a parameter that already carries a source
//! annotation gets no hint (it would be redundant), and `Unknown` (the
//! gradual seam — host values, unresolved externals) is suppressed because a
//! `: Unknown` hint is noise, not information.

use crate::ir::{Expr, ExprKind, Module, Param};
use crate::types::{ExprTypes, Type};

/// One inferred-type hint: render `label` (a leading-colon string like
/// `: Float`) immediately after byte `offset` in the source.
pub struct InlayHint {
    pub offset: usize,
    pub label: String,
}

/// Every inferred-type hint for `module`, in source order.
pub fn inlay_hints(module: &Module, types: &ExprTypes) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    for def in &module.defs {
        visit(&def.value, types, &mut hints);
    }
    hints
}

/// Walk the expression tree, emitting a hint at each unannotated lambda
/// parameter. Lambda and Match are handled here (they recurse into bodies /
/// arm bodies); everything else recurses through [`crate::hover::children`],
/// which this shares so the two walks stay in lockstep.
fn visit(expr: &Expr, types: &ExprTypes, hints: &mut Vec<InlayHint>) {
    match &expr.kind {
        ExprKind::Lambda { params, body, .. } => {
            for param in params.iter() {
                if let Some(hint) = param_hint(param, types) {
                    hints.push(hint);
                }
            }
            visit(body, types, hints);
        }
        ExprKind::Match { scrutinee, arms } => {
            visit(scrutinee, types, hints);
            for arm in arms {
                visit(&arm.body, types, hints);
            }
        }
        _ => {
            for child in crate::hover::children(expr) {
                visit(child, types, hints);
            }
        }
    }
}

/// A hint for one parameter, if it is unannotated and has a known type.
fn param_hint(param: &Param, types: &ExprTypes) -> Option<InlayHint> {
    // An annotated param already shows its type in the source.
    if param.ty.is_some() {
        return None;
    }
    let ty = types.binding(param.binding)?;
    if matches!(ty, Type::Unknown) {
        return None;
    }
    Some(InlayHint {
        // `param.span` covers just the name for an unannotated param, so its
        // end sits right after it: `(x‸) =>` renders as `(x: Float) =>`.
        offset: param.span.end,
        label: format!(": {ty}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::check_with_types;

    fn hints(src: &str) -> Vec<(usize, String)> {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let (_, types) = check_with_types(&module);
        inlay_hints(&module, &types)
            .into_iter()
            .map(|h| (h.offset, h.label))
            .collect()
    }

    #[test]
    fn infers_an_unannotated_param() {
        let src = "let f = (x) => x + 1.0";
        let got = hints(src);
        assert_eq!(got.len(), 1);
        // The hint sits right after the `x`, labelled with the inferred type.
        assert_eq!(got[0].1, ": Float");
        assert_eq!(&src[got[0].0 - 1..got[0].0], "x");
    }

    #[test]
    fn skips_an_annotated_param() {
        assert!(hints("let f = (x: Float) => x + 1.0").is_empty());
    }

    #[test]
    fn shows_a_polymorphic_param() {
        // An unconstrained param is honestly generic (`'a`), like OCaml/Merlin
        // — not `Unknown`. (`Unknown` is reserved for the gradual seam, e.g.
        // host values under the engine prelude, and is suppressed there.)
        let got = hints("let f = (x) => x");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, ": 'a");
    }

    #[test]
    fn hints_every_param_of_a_multi_arg_lambda() {
        let got = hints("let f = (x, y) => x + y");
        assert_eq!(
            got.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(),
            vec![": Float", ": Float"]
        );
    }

    #[test]
    fn descends_into_nested_lambdas() {
        // A curried inner lambda's param is inferred through its use.
        let got = hints("let f = (x) => (y) => x + y");
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|(_, l)| l == ": Float"));
    }

    #[test]
    fn infers_a_list_param() {
        let got = hints("let f = (xs) => xs |> List.maximum");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, ": List<Float>");
    }
}
