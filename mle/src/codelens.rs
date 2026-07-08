//! Code lenses: the inferred signature of each top-level definition, shown
//! on the line above it — the language-aware half of the LSP's
//! `textDocument/codeLens`. Like [`crate::hover`] / [`crate::inlay`], this
//! module decides *content* (an anchor span + a title) and the editor server
//! speaks the protocol, so it is unit-testable without an editor.
//!
//! One lens per top-level value `let`, titled `name : Type` from the
//! checker's per-expression table ([`crate::types::ExprTypes`]) — e.g.
//! `update : (Model, Msg) => Model` above `let update = …`. A def whose whole
//! type is `Unknown` (the gradual seam — host values, unresolved cross-file
//! refs) gets no lens: a bare `: Unknown` is noise, not a signature.

use crate::ir::Module;
use crate::span::Span;
use crate::types::{ExprTypes, Type};

/// One signature lens: render `title` (a `name : Type` line) on the line
/// above `span` (the definition it describes).
pub struct SignatureLens {
    pub span: Span,
    pub title: String,
}

/// A signature lens for each top-level def with a known type, in file order.
pub fn signatures(module: &Module, types: &ExprTypes) -> Vec<SignatureLens> {
    module
        .defs
        .iter()
        .filter_map(|def| {
            let ty = types.expr(def.value.id)?;
            // The gradual seam: a fully-unknown def has no signature worth
            // showing (a partial one like `(Float) => Unknown` still does).
            if matches!(ty, Type::Unknown) {
                return None;
            }
            Some(SignatureLens {
                span: def.span,
                title: format!("{} : {ty}", def.name),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::check_with_types;

    fn sigs(src: &str) -> Vec<String> {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let (_, types) = check_with_types(&module);
        signatures(&module, &types)
            .into_iter()
            .map(|s| s.title)
            .collect()
    }

    #[test]
    fn signature_of_a_function_def() {
        assert_eq!(
            sigs("let double = (x: float): float => x * 2.0"),
            vec!["double : (float) => float"]
        );
    }

    #[test]
    fn signature_of_a_constant() {
        assert_eq!(sigs("let threshold = 10.0"), vec!["threshold : float"]);
    }

    #[test]
    fn infers_an_unannotated_signature() {
        // The whole signature is recovered with no annotations written.
        assert_eq!(sigs("let f = (x) => x + 1.0"), vec!["f : (float) => float"]);
    }

    #[test]
    fn polymorphic_signature() {
        assert_eq!(sigs("let id = (x) => x"), vec!["id : ('a) => 'a"]);
    }

    #[test]
    fn one_lens_per_def_in_file_order() {
        assert_eq!(
            sigs("let a = 1.0\nlet b = (x) => x + 1.0"),
            vec!["a : float", "b : (float) => float"]
        );
    }

    #[test]
    fn the_lens_anchors_to_its_def() {
        let src = "let a = 1.0\nlet b = 2.0";
        let module = crate::lower(crate::parse(src).unwrap()).unwrap();
        let (_, types) = check_with_types(&module);
        let lenses = signatures(&module, &types);
        // `b`'s lens anchors at the start of its `let`, on line 2.
        assert_eq!(&src[lenses[1].span.start..lenses[1].span.start + 5], "let b");
    }
}
