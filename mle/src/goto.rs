//! Go-to-definition: the definition site of the reference at a byte offset —
//! the language-aware half of the LSP's `textDocument/definition` (Track D,
//! D3b). Like [`crate::hover`], the editor server converts positions and
//! speaks the protocol; this module decides the answer, so it is
//! unit-testable without an editor.
//!
//! Resolution is purely over the lowered IR (spans on every node):
//!
//! - a `Local`/`LocalMut` reference (including a `name := …` assignment
//!   target) → its binder's span — a lambda parameter, a `let [mut] name =`
//!   binder region, or a pattern variable. References carry [`BindingId`]s,
//!   so shadowing is already resolved: the innermost binder wins by
//!   construction.
//! - a `Global` reference → its top-level def's `let name =` region.
//! - a constructor use (expression or pattern) → its [`VariantDecl`] in the
//!   `type` declaration.
//! - a declared record/variant type's name in an annotation (params,
//!   returns, type-decl fields) → that `type` declaration.
//!
//! Anything else — literals, operators, binders themselves,
//! builtins/externals, primitive type names — has no definition in this
//! module and answers `None`.

use std::collections::HashMap;

use crate::ast::{TypeBody, TypeName, VariantDecl};
use crate::hover::children;
use crate::ir::{Expr, ExprKind, Module, Pattern, PatternKind};
use crate::span::Span;

/// The definition site for the reference at `offset`, if any. Among
/// overlapping reference spans (nested type-name arguments), the innermost
/// wins, like [`crate::hover`].
pub fn definition_span(module: &Module, offset: usize) -> Option<Span> {
    let targets = index_targets(module);
    let mut best: Option<(Span, Span)> = None;
    let mut consider = |span: Span, target: Span| {
        if span.start <= offset && offset < span.end {
            let tighter = match &best {
                Some((held, _)) => span.end - span.start <= held.end - held.start,
                None => true,
            };
            if tighter {
                best = Some((span, target));
            }
        }
    };
    for def in &module.defs {
        visit(&def.value, &targets, &mut consider);
    }
    // Annotation type names inside `type` declarations are references too.
    for ty in &module.types {
        for field in type_body_fields(&ty.body) {
            type_names(field, &targets, &mut consider);
        }
    }
    best.map(|(_, target)| target)
}

/// Every definition site of a module, keyed by what references carry:
/// bindings by [`BindingId`], the rest by name (each already unique in its
/// namespace — lowering rejects duplicates).
#[derive(Default)]
struct Targets {
    /// Binder spans by binding id (params, `let` binders, pattern vars).
    binders: HashMap<u32, Span>,
    /// Top-level defs: name → the `let name =` region (the IR carries no
    /// separate name span; starting at `let` is close enough to jump to).
    globals: HashMap<String, Span>,
    /// Constructors: name → the [`VariantDecl`]'s span.
    ctors: HashMap<String, Span>,
    /// Declared types: name → the whole `type` declaration's span.
    types: HashMap<String, Span>,
}

fn index_targets(module: &Module) -> Targets {
    let mut targets = Targets::default();
    for ty in &module.types {
        targets.types.insert(ty.name.clone(), ty.span);
        if let TypeBody::Variants(variants) = &ty.body {
            for variant in variants {
                targets.ctors.insert(variant.name.clone(), variant.span);
            }
        }
    }
    for def in &module.defs {
        targets.globals.insert(
            def.name.clone(),
            Span::new(def.span.start, def.value.span.start),
        );
        collect_binders(&def.value, &mut targets.binders);
    }
    targets
}

fn collect_binders(expr: &Expr, binders: &mut HashMap<u32, Span>) {
    match &expr.kind {
        ExprKind::Lambda { params, body, .. } => {
            for param in params.iter() {
                binders.insert(param.binding.0, param.span);
            }
            collect_binders(body, binders);
        }
        ExprKind::Let { binding, value, .. } => {
            // The `let [mut] name =` region, like hover's binder hover.
            if expr.span.start < value.span.start {
                binders.insert(binding.0, Span::new(expr.span.start, value.span.start));
            }
            for child in children(expr) {
                collect_binders(child, binders);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_binders(scrutinee, binders);
            for arm in arms {
                pattern_binders(&arm.pattern, binders);
                collect_binders(&arm.body, binders);
            }
        }
        _ => {
            for child in children(expr) {
                collect_binders(child, binders);
            }
        }
    }
}

fn pattern_binders(pattern: &Pattern, binders: &mut HashMap<u32, Span>) {
    match &pattern.kind {
        PatternKind::Var { binding, .. } => {
            binders.insert(binding.0, pattern.span);
        }
        PatternKind::List { items, tail } => {
            for arg in items {
                pattern_binders(arg, binders);
            }
            if let Some(tail) = tail {
                pattern_binders(tail, binders);
            }
        }
        PatternKind::Ctor { args, .. } | PatternKind::Tuple(args) => {
            for arg in args {
                pattern_binders(arg, binders);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// Walk one expression tree offering every *reference* span (with its
/// target) to `consider`.
fn visit(expr: &Expr, targets: &Targets, consider: &mut impl FnMut(Span, Span)) {
    match &expr.kind {
        ExprKind::Local { binding, name } | ExprKind::LocalMut { binding, name } => {
            let region = name_region(expr.span, name);
            offer(region, targets.binders.get(&binding.0), consider);
        }
        ExprKind::Global(name) => {
            offer(
                name_region(expr.span, name),
                targets.globals.get(name),
                consider,
            );
        }
        ExprKind::Ctor { name, .. } => {
            offer(
                name_region(expr.span, name),
                targets.ctors.get(name),
                consider,
            );
        }
        // The `name := …` target is a mut reference too.
        ExprKind::Assign { binding, name, .. } => {
            let region = name_region(expr.span, name);
            offer(region, targets.binders.get(&binding.0), consider);
            for child in children(expr) {
                visit(child, targets, consider);
            }
        }
        ExprKind::Lambda { params, ret, body } => {
            for param in params.iter() {
                if let Some(ty) = &param.ty {
                    type_names(ty, targets, consider);
                }
            }
            if let Some(ret) = ret {
                type_names(ret, targets, consider);
            }
            visit(body, targets, consider);
        }
        ExprKind::Match { scrutinee, arms } => {
            visit(scrutinee, targets, consider);
            for arm in arms {
                pattern_refs(&arm.pattern, targets, consider);
                visit(&arm.body, targets, consider);
            }
        }
        _ => {
            for child in children(expr) {
                visit(child, targets, consider);
            }
        }
    }
}

/// A constructor name in a pattern references its [`VariantDecl`]. The
/// clickable region is the name part only — sub-patterns are binders, not
/// references (and today they cannot nest further constructors).
fn pattern_refs(pattern: &Pattern, targets: &Targets, consider: &mut impl FnMut(Span, Span)) {
    if let PatternKind::Ctor { name, .. } = &pattern.kind {
        let region = name_region(pattern.span, name);
        offer(region, targets.ctors.get(name), consider);
    }
}

/// The leading-name region of a reference span. A reference's span is not
/// always just the name: an uppercase-qualified field access (`Foo.x` with
/// `Foo` a binding) lowers to a chain whose every node carries the whole
/// `Foo.x` span (see `lower`'s `ident`), an assignment's span covers
/// `name := value; rest`, and a ctor pattern's covers `Ctor(x, _)` — in
/// each, only the leading name references the definition.
fn name_region(span: Span, name: &str) -> Span {
    Span::new(span.start, (span.start + name.len()).min(span.end))
}

/// A type annotation references a declared type wherever its name appears,
/// generic arguments included (`List<Position>` — the inner name wins by
/// the innermost-span rule). Undeclared names (`Float`, `List`, the
/// product marker `*`) resolve to nothing.
fn type_names(ty: &TypeName, targets: &Targets, consider: &mut impl FnMut(Span, Span)) {
    offer(ty.span, targets.types.get(&ty.name), consider);
    for arg in &ty.args {
        type_names(arg, targets, consider);
    }
}

fn offer(span: Span, target: Option<&Span>, consider: &mut impl FnMut(Span, Span)) {
    if let Some(&target) = target {
        consider(span, target);
    }
}

/// All the annotated fields of a `type` body, record or variant.
fn type_body_fields(body: &TypeBody) -> Vec<&TypeName> {
    match body {
        TypeBody::Record(fields) => fields.iter().map(|f| &f.ty).collect(),
        TypeBody::Variants(variants) => variants
            .iter()
            .flat_map(|VariantDecl { fields, .. }| fields.iter().map(|f| &f.ty))
            .collect(),
        // An abstract type has no annotated fields.
        TypeBody::Abstract => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source text of the definition the offset at `needle` resolves to.
    fn def_at<'a>(src: &'a str, needle: &str) -> Option<&'a str> {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let offset = src.find(needle).expect("needle present");
        definition_span(&module, offset).map(|span| &src[span.start..span.end])
    }

    fn def_at_last<'a>(src: &'a str, needle: &str) -> Option<&'a str> {
        let module = crate::lower(crate::parse(src).expect("parse")).expect("lower");
        let offset = src.rfind(needle).expect("needle present");
        definition_span(&module, offset).map(|span| &src[span.start..span.end])
    }

    // --- Locals ---

    #[test]
    fn param_reference_resolves_to_the_parameter() {
        let src = "let f = (score: float): bool => score > 1.0";
        assert_eq!(def_at(src, "score >"), Some("score: float"));
    }

    #[test]
    fn let_reference_resolves_to_the_binder_region() {
        let src = "let f = (x) => let y = x in y + 1.0";
        assert_eq!(def_at(src, "y +"), Some("let y = "));
    }

    #[test]
    fn shadowing_resolves_to_the_innermost_binder() {
        // `x` in the body is the inner `let x`, not the parameter.
        let src = "let f = (x: float) => let x = 2.0 in x + 1.0";
        assert_eq!(def_at(src, "x +"), Some("let x = "));
    }

    #[test]
    fn mut_reference_and_assign_target_resolve_to_the_mut_binder() {
        let src = "let f = (x: float) => let mut a = x in a := a + 1.0; a";
        assert_eq!(def_at_last(src, "a"), Some("let mut a = "));
        assert_eq!(def_at(src, "a :="), Some("let mut a = "));
        // …but the `:=` itself is not a reference.
        assert_eq!(def_at(src, ":="), None);
    }

    #[test]
    fn pattern_variable_reference_resolves_to_the_pattern() {
        let src = "let f = (p) => let (lo, hi) = p in hi - lo";
        assert_eq!(def_at(src, "hi -"), Some("hi"));
        assert_eq!(def_at_last(src, "lo"), Some("lo"));
    }

    // --- Globals ---

    #[test]
    fn global_reference_resolves_to_the_def() {
        let src = "let double = (x: float): float => x * 2.0\nlet main = () => double(2.0)";
        assert_eq!(def_at(src, "double(2.0)"), Some("let double = "));
    }

    // `Foo.x` on an uppercase binding lowers to a field-access chain whose
    // every node carries the whole `Foo.x` span — only the base name is the
    // reference [codex review Medium].
    #[test]
    fn qualified_field_access_resolves_the_base_only() {
        let src = "let Foo = { x: 1.0 }\nlet y = Foo.x";
        assert_eq!(def_at(src, "Foo.x"), Some("let Foo = "));
        assert_eq!(def_at_last(src, "x"), None); // the `.x` field segment
    }

    // --- Constructors ---

    const SHAPE: &str = "type Shape = | Circle(r: float) | Point\n";

    #[test]
    fn ctor_expression_resolves_to_the_variant_decl() {
        let src = format!("{SHAPE}let c = Circle(2.0)");
        assert_eq!(def_at(&src, "Circle(2.0)"), Some("Circle(r: float)"));
    }

    #[test]
    fn nullary_ctor_resolves_to_the_variant_decl() {
        let src = format!("{SHAPE}let p = Point");
        assert_eq!(def_at_last(&src, "Point"), Some("Point"));
        // …and it really is the declaration, not the use itself.
        let module = crate::lower(crate::parse(&src).unwrap()).unwrap();
        let span = definition_span(&module, src.rfind("Point").unwrap()).unwrap();
        assert_eq!(span.start, src.find("Point").unwrap());
    }

    #[test]
    fn ctor_pattern_resolves_to_the_variant_decl() {
        let src =
            format!("{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r | _ => 0.0");
        assert_eq!(def_at(&src, "Circle(r)"), Some("Circle(r: float)"));
    }

    #[test]
    fn a_pattern_variable_is_a_binder_not_a_reference() {
        let src =
            format!("{SHAPE}let f = (s: Shape): float => match s with | Circle(r) => r | _ => 0.0");
        assert_eq!(def_at(&src, "r) =>"), None);
    }

    // --- Type annotations ---

    #[test]
    fn annotation_resolves_to_the_type_decl() {
        let src = "type P = { x: float }\nlet mk = (p: P): P => p";
        assert_eq!(def_at(src, "P)"), Some("type P = { x: float }"));
        assert_eq!(def_at(src, "P =>"), Some("type P = { x: float }"));
    }

    #[test]
    fn generic_argument_resolves_to_the_type_decl() {
        let src = "type P = { x: float }\nlet f = (ps: List<P>) => ps";
        assert_eq!(def_at(src, "P>"), Some("type P = { x: float }"));
    }

    #[test]
    fn type_decl_field_annotation_resolves_too() {
        let src = "type P = { x: float }\ntype Q = | Wrap(p: P)";
        assert_eq!(def_at(src, "P)"), Some("type P = { x: float }"));
    }

    #[test]
    fn primitive_type_name_has_no_definition() {
        assert_eq!(def_at("let f = (x: float) => x", "float"), None);
    }

    // --- Nothing ---

    #[test]
    fn builtins_and_literals_have_no_definition() {
        let src = "let f = (xs) => xs |> List.maximum";
        assert_eq!(def_at(src, "List.maximum"), None);
        assert_eq!(def_at("let a = 1.0", "1.0"), None);
    }

    #[test]
    fn no_definition_outside_any_node() {
        let src = "let a = 1.0";
        let module = crate::lower(crate::parse(src).unwrap()).unwrap();
        assert!(definition_span(&module, src.len()).is_none());
    }
}
