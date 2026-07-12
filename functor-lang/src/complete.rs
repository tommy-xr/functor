//! Code completion: what to offer at a byte offset in a (possibly broken)
//! live buffer — the language-aware half of the LSP's
//! `textDocument/completion`. Like [`crate::hover`] and [`crate::goto`], the
//! editor server converts positions and speaks the protocol; this module
//! decides the candidates, so it is unit-testable without an editor.
//!
//! Completion fires on code that does not parse (`let s = Scene.`), so the
//! two halves of the answer come from different places:
//!
//! - **Context** — am I after `Ident.` (a member access) or typing a bare
//!   word (a top-level position)? — is derived **textually** from the live
//!   buffer by lexing the prefix up to the cursor.
//! - **Candidates** come from a **loaded [`Project`]** (the LSP's last-good
//!   parse): its `.funi` prelude signatures, sibling defs, ADT constructors,
//!   the builtin registry, keywords, and module names.
//!
//! Beyond v1's context + candidates, two **scope-aware** layers run when the
//! live buffer is FRESH — the cached project parsed from exactly this text, so
//! offsets into its AST are sound (see [`fresh_offset`]): **locals in scope**
//! extend the top-level candidates, and **typed record fields** (`pos.` where
//! `pos`'s checked type is a declared record) extend the member candidates.
//! Member context additionally tolerates the one edit completion itself
//! causes — the `.partial` tail being typed, which breaks the parse (see
//! [`member_fresh_offset`]) — so fields appear at the trigger keystroke. Any
//! other stale or broken buffer skips both layers and falls back to the
//! textual-only answer — the low-confidence rule, in its simplest honest form.
//!
//! Non-goals: chained members (`a.b.`), type-position members (`Scene.t` /
//! `Pieces.Shape` in annotations), `open`ed modules' exports offered bare (the
//! merged module does not retain `open` metadata), and expression receivers
//! (`foo().`). Each testable one is pinned as an empty result by a boundary
//! test below.

use std::collections::BTreeSet;

use crate::ast::{TypeBody, VariantDecl};
use crate::eval::{builtin_name, Builtin};
use crate::hover::{children, type_name_text};
use crate::ir::{BindingId, Def, Expr, ExprKind, Pattern, PatternKind};
use crate::lexer::{Token, TokenKind};
use crate::project::Project;
use crate::span::Span;
use crate::types::{builtin_signature, ExprTypes, Type};

/// One completion candidate: the inserted `label`, an optional hover-style
/// `detail` (`name : Type`), and its editor `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: CompletionKind,
}

/// What a candidate is — the editor renders each with its own icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Function,
    Value,
    Module,
    Keyword,
    Constructor,
    /// A record field, offered after a record-typed value (`pos.x`).
    Field,
}

/// The complete builtin registry. Hand-listed because [`Builtin`] is not
/// iterable — keep in sync with `eval::Builtin` (33 variants).
const BUILTINS: [Builtin; 33] = [
    Builtin::ListMap,
    Builtin::ListFilter,
    Builtin::ListFold,
    Builtin::ListRange,
    Builtin::ListGrid,
    Builtin::ListMaximum,
    Builtin::ListLength,
    Builtin::ListAppend,
    Builtin::ListFlatten,
    Builtin::ListAny,
    Builtin::ListAll,
    Builtin::ListReverse,
    Builtin::ListIsEmpty,
    Builtin::MathSin,
    Builtin::MathCos,
    Builtin::MathSqrt,
    Builtin::MathAbs,
    Builtin::MathFloor,
    Builtin::MathAtan2,
    Builtin::MathMod,
    Builtin::MathMin,
    Builtin::MathMax,
    Builtin::MathPow,
    Builtin::MathPi,
    Builtin::TextConcat,
    Builtin::TextFromFloat,
    Builtin::TextFixed,
    Builtin::TextToBullets,
    Builtin::TextSplit,
    Builtin::TextJoin,
    Builtin::TextParseFloat,
    Builtin::MathClamp01,
    Builtin::DebugLog,
];

/// The keywords offered at an expression position: the lexer keywords plus
/// the contextual `open` (`lexer.rs`).
const KEYWORDS: [&str; 12] = [
    "let", "type", "open", "mut", "with", "in", "match", "true", "false", "if", "then", "else",
];

/// OFFSET CONTRACT: `offset` is LOCAL to `live_text` (the live buffer), NOT
/// project-wide. Context comes purely from `live_text`; candidates purely from
/// `project` (possibly a stale last-good load). The two span spaces never mix.
pub fn complete(
    project: &Project,
    current_module: &str,
    live_text: &str,
    offset: usize,
) -> Vec<CompletionItem> {
    match context_at(live_text, offset) {
        Context::None => Vec::new(),
        Context::Member {
            qualifier,
            partial,
            dot,
        } => {
            // Member context tolerates the one edit completion itself causes:
            // the buffer that is exactly the cached text plus the `.partial`
            // tail being typed (see [`member_fresh_offset`]).
            let fresh = member_fresh_offset(project, current_module, live_text, dot, offset);
            member_candidates(project, current_module, &qualifier, &partial, fresh)
        }
        Context::TopLevel { partial } => {
            let fresh = fresh_offset(project, current_module, live_text, offset);
            top_level_candidates(project, current_module, &partial, fresh)
        }
    }
}

/// FRESHNESS GATE (the low-confidence rule). The scope-aware layers index into
/// the cached project's AST, but `complete`'s `offset` is LOCAL to the live
/// buffer and the cache may be stale. This returns the PROJECT-WIDE offset
/// (`file.base + offset`) ONLY when the current module's (non-interface) file
/// in the cache is byte-for-byte the live buffer — i.e. the buffer parsed and
/// the cache is fresh. `None` (a broken or stale buffer) means the scope-aware
/// layers are skipped and completion falls back to v1's textual-only answers.
fn fresh_offset(
    project: &Project,
    current_module: &str,
    live_text: &str,
    offset: usize,
) -> Option<usize> {
    let file = fresh_file(project, current_module)?;
    (file.src == live_text).then_some(file.base + offset)
}

/// The member-context freshness gate. Typing `.` breaks the parse (`v.` is not
/// an expression), so at completion's own trigger keystroke the cache is one
/// edit behind the buffer. Accept exactly that edit: the live text with the
/// member tail `[dot..offset)` (the `.` and any partial) removed must restore
/// the cached text byte-for-byte. The returned PROJECT-WIDE offset is the dot's
/// position — the qualifier's scope, valid in the cached AST. An exact match
/// (the cursor inside an already-valid `v.x`) passes the same way.
fn member_fresh_offset(
    project: &Project,
    current_module: &str,
    live_text: &str,
    dot: usize,
    offset: usize,
) -> Option<usize> {
    let file = fresh_file(project, current_module)?;
    let restored = file.src == live_text
        || (file.src.len() + (offset - dot) == live_text.len()
            && file
                .src
                .as_bytes()
                .starts_with(&live_text.as_bytes()[..dot])
            && file.src.as_bytes()[dot..] == live_text.as_bytes()[offset..]);
    restored.then_some(file.base + dot)
}

/// The current module's (non-interface) source file in the cached project.
fn fresh_file<'a>(
    project: &'a Project,
    current_module: &str,
) -> Option<&'a crate::project::SourceFile> {
    project
        .sources
        .files()
        .iter()
        .find(|file| !file.interface && file.module == current_module)
}

/// The cursor's completion context, derived textually from the prefix.
enum Context {
    /// After `Qualifier.` — offer that module's members filtered by `partial`.
    /// `dot` is the byte offset of the `.` in the live buffer.
    Member {
        qualifier: String,
        partial: String,
        dot: usize,
    },
    /// A bare word (or fresh position) — offer keywords, globals, modules.
    TopLevel { partial: String },
    /// No completion here (inside a string/comment, after a literal, …).
    None,
}

/// Classify the cursor by lexing the prefix `live_text[..offset]`. Lexing is
/// not parsing: a buffer broken only at the parse level (`let s = Scene.`)
/// still lexes cleanly. A genuine lex error (an unterminated string, a bare
/// `'`) honestly means "no completions".
fn context_at(live_text: &str, offset: usize) -> Context {
    if offset > live_text.len() || !live_text.is_char_boundary(offset) {
        return Context::None;
    }
    let prefix = &live_text[..offset];
    let Ok(mut tokens) = crate::lexer::lex(prefix, 0) else {
        return Context::None;
    };
    tokens.pop(); // drop the Eof sentinel the lexer always appends

    // The cursor sits in a line comment when a `//` lies between the last
    // token and the cursor on the CURSOR'S line: the lexer skips comments, so
    // it emits no token for the tail the cursor is inside. A comment on an
    // earlier line (a newline follows it) leaves a fresh top-level position.
    let last_end = tokens.last().map_or(0, |t| t.span.end);
    let gap = &prefix[last_end..];
    if gap[gap.rfind('\n').map_or(0, |i| i + 1)..].contains("//") {
        return Context::None;
    }

    let n = tokens.len();
    if n == 0 {
        return Context::TopLevel {
            partial: String::new(),
        };
    }
    let last = &tokens[n - 1];
    let touches = last.span.end == offset;

    match &last.kind {
        // `Qualifier.` — a member access with no partial yet. Same-line
        // whitespace before the cursor keeps the member context; a line break
        // ends it (the cursor is on a fresh line, not finishing the access).
        TokenKind::Dot if !prefix[last.span.end..].contains('\n') => {
            if n >= 2 {
                if let TokenKind::Ident(_) = tokens[n - 2].kind {
                    return Context::Member {
                        qualifier: qualifier_chain(&tokens, n - 2),
                        partial: String::new(),
                        dot: last.span.start,
                    };
                }
            }
            // `1.`, `).`, a bare `.` — not a module member (expression-member
            // completion is PR 2).
            Context::None
        }
        // A word touching the cursor: `Qualifier.partial` or a top-level
        // partial. (A word NOT touching the cursor has whitespace after it —
        // it falls through to a fresh top-level position.)
        TokenKind::Ident(partial) if touches => {
            if n >= 2 && tokens[n - 2].kind == TokenKind::Dot {
                if n >= 3 {
                    if let TokenKind::Ident(_) = tokens[n - 3].kind {
                        return Context::Member {
                            qualifier: qualifier_chain(&tokens, n - 3),
                            partial: partial.clone(),
                            dot: tokens[n - 2].span.start,
                        };
                    }
                }
                // `1.cu` — the qualifier head is not an identifier.
                return Context::None;
            }
            Context::TopLevel {
                partial: partial.clone(),
            }
        }
        // A full keyword touching the cursor still wants the popup (the user
        // is mid-typing `let`): treat it as a top-level partial of its own
        // text.
        kind if touches && is_keyword(kind) => Context::TopLevel {
            partial: prefix[last.span.start..last.span.end].to_string(),
        },
        // A literal / range touching the cursor offers nothing — there is no
        // member completion off a number or string in v1.
        TokenKind::Number(_) | TokenKind::Str(_) | TokenKind::DotDot | TokenKind::TypeVar(_)
            if touches =>
        {
            Context::None
        }
        // Anywhere else — after an operator, a delimiter, or whitespace — is a
        // fresh top-level position.
        _ => Context::TopLevel {
            partial: String::new(),
        },
    }
}

/// The dotted qualifier ending at `tokens[end]` (an `Ident`): walk back over an
/// `Ident (Dot Ident)*` chain so `a.b.` yields `"a.b"`. A multi-segment
/// qualifier matches no module in v1 (its candidates come back empty — the
/// chained-member boundary).
fn qualifier_chain(tokens: &[Token], end: usize) -> String {
    let mut names = Vec::new();
    let mut i = end;
    while let TokenKind::Ident(name) = &tokens[i].kind {
        names.push(name.clone());
        if i >= 2 && tokens[i - 1].kind == TokenKind::Dot {
            if let TokenKind::Ident(_) = tokens[i - 2].kind {
                i -= 2;
                continue;
            }
        }
        break;
    }
    names.reverse();
    names.join(".")
}

fn is_keyword(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Let
            | TokenKind::Type
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Mut
            | TokenKind::With
            | TokenKind::In
            | TokenKind::Match
    )
}

/// Candidates for `Qualifier.partial`: the module's `.funi` signatures, its
/// sibling defs, its ADT constructors, and the builtins in its namespace.
fn member_candidates(
    project: &Project,
    current_module: &str,
    qualifier: &str,
    partial: &str,
    fresh: Option<usize>,
) -> Vec<CompletionItem> {
    let module = &project.module;
    let prefix = format!("{qualifier}.");
    // Lazy: only checked now that a context matched (defs need the checker's
    // types for their details). The check is on `project` (last-good), never
    // on the live buffer.
    let (_, types) = project.check_with_types();

    // Value bindings shadow namespaces — lowering resolves locals and
    // top-level lets BEFORE sibling/builtin modules (see `crate::lower`'s
    // resolution order), so `List.x` on a record-typed binder named `List` is
    // a field access, not a builtin call. When the buffer is fresh and the
    // qualifier is a single segment that names a value, answer as that value:
    // a declared record offers its fields, anything else offers nothing
    // (field access on a non-record has no members). Chained `a.b.` stays
    // empty — the chained-member boundary.
    if !qualifier.contains('.') {
        if let Some(offset) = fresh {
            if let Some(ty) = qualifier_type(project, current_module, qualifier, offset, &types) {
                return finish(record_fields_of(project, &ty), partial);
            }
        }
    }

    let mut items = Vec::new();

    // Host-provided values (`Scene.cube : () => Scene.t`).
    for sig in &module.signatures {
        if let Some(label) = sig.name.strip_prefix(&prefix) {
            let kind = if sig.ty.name == "=>" {
                CompletionKind::Function
            } else {
                CompletionKind::Value
            };
            items.push(CompletionItem {
                label: label.to_string(),
                detail: Some(format!("{} : {}", sig.name, type_name_text(&sig.ty))),
                kind,
            });
        }
    }

    // Sibling/user-module defs (detail from the checker, hover-identical).
    for def in &module.defs {
        if let Some(label) = def.name.strip_prefix(&prefix) {
            let ty = types.expr(def.value.id).cloned().unwrap_or(Type::Unknown);
            items.push(CompletionItem {
                label: label.to_string(),
                detail: Some(format!("{} : {ty}", def.name)),
                kind: value_kind(&ty),
            });
        }
    }

    // ADT constructors (value namespace; symbolic detail from the decl).
    for ty in &module.types {
        if let TypeBody::Variants(decls) = &ty.body {
            let ret = ctor_return(ty);
            for variant in decls {
                if let Some(label) = variant.name.strip_prefix(&prefix) {
                    items.push(CompletionItem {
                        label: label.to_string(),
                        detail: Some(ctor_detail(variant, &ret)),
                        kind: CompletionKind::Constructor,
                    });
                }
            }
        }
    }

    // Builtins whose namespace is the qualifier (`List.map`, …). Most are
    // functions, but a constant like `Math.pi` is a plain value.
    for &b in &BUILTINS {
        let name = builtin_name(b);
        if let Some(label) = name.strip_prefix(&prefix) {
            items.push(CompletionItem {
                label: label.to_string(),
                detail: Some(format!("{name} : {}", builtin_signature(b))),
                kind: value_kind(&builtin_signature(b)),
            });
        }
    }

    finish(items, partial)
}

/// The fields of a declared record type, or empty. Only a [`Type::Record`]
/// offers fields (gradual honesty — an unknown/non-record type offers
/// nothing).
fn record_fields_of(project: &Project, ty: &Type) -> Vec<CompletionItem> {
    let Type::Record(name, _) = ty else {
        return Vec::new();
    };
    // The declaration by its EXACT (canonical) name — a sibling's record is
    // `Utils.Vec2` in both the type and the declaration.
    let Some(decl) = project.module.types.iter().find(|decl| &decl.name == name) else {
        return Vec::new();
    };
    let TypeBody::Record(fields) = &decl.body else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|field| CompletionItem {
            label: field.name.clone(),
            detail: Some(format!("{} : {}", field.name, type_name_text(&field.ty))),
            kind: CompletionKind::Field,
        })
        .collect()
}

/// The checked type of a single-segment member qualifier at `offset`, when it
/// names a VALUE: a binder in scope (innermost wins), else an own-module def
/// (bare for the entry, `Module.name` for a sibling). A binding that exists
/// but has no checked type is `Unknown` (it still shadows — the caller must
/// not fall through to namespace members). `None` only when no value binding
/// has that name.
fn qualifier_type(
    project: &Project,
    current_module: &str,
    qualifier: &str,
    offset: usize,
    types: &ExprTypes,
) -> Option<Type> {
    if let Some(def) = enclosing_def(project, current_module, offset) {
        if let Some((_, binding, _)) = binders_in_scope(&def.value, offset)
            .into_iter()
            .find(|(name, _, _)| name == qualifier)
        {
            return Some(types.binding(binding).cloned().unwrap_or(Type::Unknown));
        }
    }
    let canonical = if current_module == project.entry {
        qualifier.to_string()
    } else {
        format!("{current_module}.{qualifier}")
    };
    let def = project
        .module
        .defs
        .iter()
        .find(|def| def.name == canonical)?;
    Some(types.expr(def.value.id).cloned().unwrap_or(Type::Unknown))
}

/// Candidates at a top-level position: keywords, the current module's own
/// defs and constructors (bare), and the visible module names.
fn top_level_candidates(
    project: &Project,
    current_module: &str,
    partial: &str,
    fresh: Option<usize>,
) -> Vec<CompletionItem> {
    let module = &project.module;
    let entry = &project.entry;
    let (_, types) = project.check_with_types();
    let mut items = Vec::new();

    // Locals in scope (scope-aware, fresh buffers only). Pushed BEFORE the
    // keywords/globals below so `finish`'s stable sort + dedup lets an inner
    // binder shadow a same-named global (locals appear first, dedup keeps them).
    if let Some(offset) = fresh {
        if let Some(def) = enclosing_def(project, current_module, offset) {
            for (name, binding, mutable) in binders_in_scope(&def.value, offset) {
                items.push(binder_item(&name, binding, mutable, &types));
            }
        }
    }

    for kw in KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            detail: None,
            kind: CompletionKind::Keyword,
        });
    }

    // This module's own defs, referenced bare in source. (Entry defs are
    // already bare; a sibling's are `Module.name` — the same strip either way.)
    for def in &module.defs {
        if owning_module(&def.name, entry) == current_module {
            let ty = types.expr(def.value.id).cloned().unwrap_or(Type::Unknown);
            items.push(CompletionItem {
                label: bare_name(&def.name).to_string(),
                detail: Some(format!("{} : {ty}", def.name)),
                kind: value_kind(&ty),
            });
        }
    }

    // This module's own constructors, also bare.
    for ty in &module.types {
        if let TypeBody::Variants(decls) = &ty.body {
            let ret = ctor_return(ty);
            for variant in decls {
                if owning_module(&variant.name, entry) == current_module {
                    items.push(CompletionItem {
                        label: bare_name(&variant.name).to_string(),
                        detail: Some(ctor_detail(variant, &ret)),
                        kind: CompletionKind::Constructor,
                    });
                }
            }
        }
    }

    // Module names: the first segment of every dotted signature/def/ctor name,
    // plus the builtin namespaces, minus the current module (you don't qualify
    // your own defs).
    let mut modules: BTreeSet<String> = BTreeSet::new();
    for sig in &module.signatures {
        module_segment(&sig.name, &mut modules);
    }
    for def in &module.defs {
        module_segment(&def.name, &mut modules);
    }
    for ty in &module.types {
        if let TypeBody::Variants(decls) = &ty.body {
            for variant in decls {
                module_segment(&variant.name, &mut modules);
            }
        }
    }
    for &b in &BUILTINS {
        module_segment(builtin_name(b), &mut modules);
    }
    modules.remove(current_module);
    for name in modules {
        items.push(CompletionItem {
            label: name,
            detail: None,
            kind: CompletionKind::Module,
        });
    }

    finish(items, partial)
}

/// A constructor's return type as declared: the owning type's bare name with
/// its type parameters (`Shape`, `Option<'a>`).
fn ctor_return(ty: &crate::ir::TypeDef) -> String {
    let name = bare_name(&ty.name);
    if ty.params.is_empty() {
        name.to_string()
    } else {
        format!("{name}<{}>", ty.params.join(", "))
    }
}

/// Symbolic constructor detail from its declaration: nullary `Point : Shape`,
/// fielded `Circle : (float) => Shape` (field types via [`type_name_text`],
/// `ret` = the owning type's declared return, see [`ctor_return`]).
fn ctor_detail(variant: &VariantDecl, ret: &str) -> String {
    if variant.fields.is_empty() {
        format!("{} : {ret}", variant.name)
    } else {
        let params: Vec<String> = variant
            .fields
            .iter()
            .map(|f| type_name_text(&f.ty))
            .collect();
        format!("{} : ({}) => {ret}", variant.name, params.join(", "))
    }
}

/// A value's kind: [`CompletionKind::Function`] when its type is a function,
/// else [`CompletionKind::Value`].
fn value_kind(ty: &Type) -> CompletionKind {
    if matches!(ty, Type::Fn(..)) {
        CompletionKind::Function
    } else {
        CompletionKind::Value
    }
}

/// The top-level def whose span contains project-wide `offset` and whose name
/// belongs to `current_module` (the file being edited) — the def whose value
/// expression the scope-aware walk explores. Ends are inclusive so a cursor at
/// the very end of the def (the common completion position) still matches.
fn enclosing_def<'a>(project: &'a Project, current_module: &str, offset: usize) -> Option<&'a Def> {
    project.module.defs.iter().find(|def| {
        owning_module(&def.name, &project.entry) == current_module
            && scope_contains(def.span, offset)
    })
}

/// Every value binder in scope at `offset` within `root` (a def's value),
/// INNERMOST FIRST — `(name, binding id, is_mut)`. Scoping mirrors the
/// interpreter (see [`crate::lower`]): lambda params scope over the lambda
/// body, `let … in` binders over the `in` body (not their own value — `let` is
/// non-recursive), and match-arm pattern variables over that arm's body.
/// Innermost-first ordering lets a shadowing inner binder win dedup.
fn binders_in_scope(root: &Expr, offset: usize) -> Vec<(String, BindingId, bool)> {
    let mut out = Vec::new();
    collect_binders_in_scope(root, offset, &mut out);
    out.reverse();
    out
}

/// Half-open at the start, INCLUSIVE at the end — a binder is in scope right up
/// to the end of its body (where the cursor sits when completing).
fn scope_contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn collect_binders_in_scope(expr: &Expr, offset: usize, out: &mut Vec<(String, BindingId, bool)>) {
    match &expr.kind {
        ExprKind::Lambda { params, body, .. } => {
            if scope_contains(body.span, offset) {
                for param in params.iter() {
                    out.push((param.name.clone(), param.binding, false));
                }
            }
            collect_binders_in_scope(body, offset, out);
        }
        ExprKind::Let {
            binding,
            name,
            mutable,
            value,
            body,
            ..
        } => {
            if scope_contains(body.span, offset) {
                out.push((name.clone(), *binding, *mutable));
            }
            collect_binders_in_scope(value, offset, out);
            collect_binders_in_scope(body, offset, out);
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_binders_in_scope(scrutinee, offset, out);
            for arm in arms {
                if scope_contains(arm.body.span, offset) {
                    collect_pattern_binders(&arm.pattern, out);
                }
                collect_binders_in_scope(&arm.body, offset, out);
            }
        }
        // Lambda/Match are handled above; every other node's children come
        // from the shared walk (Assign's `name := …` target is a reference to
        // an enclosing `let mut`, not a new binder).
        _ => {
            for child in children(expr) {
                collect_binders_in_scope(child, offset, out);
            }
        }
    }
}

/// Every variable a pattern binds (a match arm's binders), pushed to `out`.
/// Pattern variables are never mutable.
fn collect_pattern_binders(pattern: &Pattern, out: &mut Vec<(String, BindingId, bool)>) {
    match &pattern.kind {
        PatternKind::Var { binding, name } => out.push((name.clone(), *binding, false)),
        PatternKind::Ctor { args, .. } | PatternKind::Tuple(args) => {
            for arg in args {
                collect_pattern_binders(arg, out);
            }
        }
        PatternKind::List { items, tail } => {
            for arg in items {
                collect_pattern_binders(arg, out);
            }
            if let Some(tail) = tail {
                collect_pattern_binders(tail, out);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// A completion item for an in-scope binder: `name : Type` detail from the
/// checker's binding table (`mut name : Type` for a mutable slot, matching
/// hover), `Unknown` where the checker doesn't know it.
fn binder_item(name: &str, binding: BindingId, mutable: bool, types: &ExprTypes) -> CompletionItem {
    let ty = types.binding(binding).cloned().unwrap_or(Type::Unknown);
    let detail = if mutable {
        format!("mut {name} : {ty}")
    } else {
        format!("{name} : {ty}")
    };
    CompletionItem {
        label: name.to_string(),
        detail: Some(detail),
        kind: value_kind(&ty),
    }
}

/// The module a canonical name belongs to: the segment before its first `.`,
/// or the entry (whose members are bare).
fn owning_module<'a>(name: &'a str, entry: &'a str) -> &'a str {
    name.split_once('.').map_or(entry, |(module, _)| module)
}

/// A canonical name stripped of its module qualifier (`Utils.clamp` →
/// `clamp`; a bare name is unchanged).
fn bare_name(name: &str) -> &str {
    name.split_once('.').map_or(name, |(_, member)| member)
}

/// Record `name`'s module qualifier (its first dotted segment), if any.
fn module_segment(name: &str, out: &mut BTreeSet<String>) {
    if let Some((module, _)) = name.split_once('.') {
        out.insert(module.to_string());
    }
}

/// Keep only candidates matching `partial`, then sort and dedup by label for a
/// deterministic, popup-ready list.
fn finish(mut items: Vec<CompletionItem>, partial: &str) -> Vec<CompletionItem> {
    items.retain(|item| item.label.starts_with(partial));
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items.dedup_by(|a, b| a.label == b.label);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::load_sources_with_prelude;
    use std::path::PathBuf;

    /// A minimal, valid single-file project — the last-good parse the broken
    /// live buffers complete against.
    const STUB: &str = "let main = () => 1.0";

    /// The inline `Scene` prelude used by the A-layer tests: only `cube` and
    /// `sphere`, so exact-label assertions stay small. (`type t` canonicalizes
    /// to `Scene.t`, matching how the real `scene.funi` renders.)
    const SCENE: &str = "type t\nlet cube : () => t\nlet sphere : () => t";

    fn project_of(sources: &[(&str, &str)], prelude: &[(&str, &str)]) -> Project {
        let sources = sources
            .iter()
            .map(|(name, src)| (PathBuf::from(name), src.to_string()))
            .collect();
        let prelude: Vec<(String, String)> = prelude
            .iter()
            .map(|(module, src)| (module.to_string(), src.to_string()))
            .collect();
        load_sources_with_prelude(sources, &prelude)
            .unwrap_or_else(|e| panic!("project loads: {}", e.render()))
    }

    /// Complete in module `Game`, cursor at the END of `live`, with candidates
    /// from a valid single-file project (`project_src`) plus `prelude`. `live`
    /// deliberately differs from `project_src` — that IS the live-vs-last-good
    /// seam.
    fn game(project_src: &str, prelude: &[(&str, &str)], live: &str) -> Vec<CompletionItem> {
        let project = project_of(&[("game.fun", project_src)], prelude);
        complete(&project, "Game", live, live.len())
    }

    /// Complete in module `Game` at byte `offset` inside `src`, with the entry
    /// project (`game.fun`) loaded from EXACTLY `src` — a FRESH buffer, so the
    /// scope-aware layers (locals, record fields) run.
    fn fresh(src: &str, prelude: &[(&str, &str)], offset: usize) -> Vec<CompletionItem> {
        let project = project_of(&[("game.fun", src)], prelude);
        complete(&project, "Game", src, offset)
    }

    fn labels(items: &[CompletionItem]) -> Vec<String> {
        items.iter().map(|i| i.label.clone()).collect()
    }

    fn find<'a>(items: &'a [CompletionItem], label: &str) -> &'a CompletionItem {
        items
            .iter()
            .find(|i| i.label == label)
            .unwrap_or_else(|| panic!("no `{label}` in {:?}", labels(items)))
    }

    fn has(items: &[CompletionItem], label: &str) -> bool {
        items.iter().any(|i| i.label == label)
    }

    // 1 (+20). Prelude dot-completion — exact sorted Vec, kinds, details.
    #[test]
    fn prelude_member_completion() {
        let items = game(STUB, &[("Scene", SCENE)], "let s = Scene.");
        assert_eq!(
            items,
            vec![
                CompletionItem {
                    label: "cube".to_string(),
                    detail: Some("Scene.cube : () => Scene.t".to_string()),
                    kind: CompletionKind::Function,
                },
                CompletionItem {
                    label: "sphere".to_string(),
                    detail: Some("Scene.sphere : () => Scene.t".to_string()),
                    kind: CompletionKind::Function,
                },
            ]
        );
    }

    // 2. Builtin module `List.` — members present, nothing from `Text.`; pins
    // the `List.range` detail and the `'a`/`'b` Var rendering via `List.map`.
    #[test]
    fn builtin_member_completion() {
        let items = game(STUB, &[], "let s = List.");
        for member in ["map", "filter", "fold", "range", "grid", "maximum"] {
            assert!(
                has(&items, member),
                "missing {member} in {:?}",
                labels(&items)
            );
        }
        assert!(!has(&items, "concat"), "Text builtin leaked into List.");
        assert_eq!(
            find(&items, "range").detail.as_deref(),
            Some("List.range : (float) => List<float>")
        );
        assert_eq!(
            find(&items, "map").detail.as_deref(),
            Some("List.map : (('a) => 'b, List<'a>) => List<'b>")
        );
        assert_eq!(find(&items, "map").kind, CompletionKind::Function);
    }

    // Builtin module `Math.` — the function builtins complete as functions, but
    // the constant `Math.pi` completes as a Value (not a callable).
    #[test]
    fn math_member_completion_pi_is_a_value() {
        let items = game(STUB, &[], "let s = Math.");
        assert_eq!(find(&items, "sqrt").kind, CompletionKind::Function);
        assert_eq!(find(&items, "pi").kind, CompletionKind::Value);
        assert_eq!(find(&items, "pi").detail.as_deref(), Some("Math.pi : float"));
    }

    // 3 (+20). Sibling module `Utils.` from `game.fun` — exact Vec, kinds.
    #[test]
    fn sibling_member_completion() {
        let project = project_of(
            &[
                ("game.fun", STUB),
                (
                    "utils.fun",
                    "let clamp = (lo: float, hi: float, x: float): float => x\n\
                     let version = 3.0",
                ),
            ],
            &[],
        );
        let live = "let s = Utils.";
        let items = complete(&project, "Game", live, live.len());
        assert_eq!(
            items,
            vec![
                CompletionItem {
                    label: "clamp".to_string(),
                    detail: Some("Utils.clamp : (float, float, float) => float".to_string()),
                    kind: CompletionKind::Function,
                },
                CompletionItem {
                    label: "version".to_string(),
                    detail: Some("Utils.version : float".to_string()),
                    kind: CompletionKind::Value,
                },
            ]
        );
    }

    // 3b. Sibling ADT constructors `Pieces.` — ctors ∪ defs, sorted, kinds,
    // symbolic details.
    #[test]
    fn sibling_constructor_completion() {
        let project = project_of(
            &[
                ("game.fun", STUB),
                (
                    "pieces.fun",
                    "type Shape = | Circle(r: float) | Point\nlet count = 12.0",
                ),
            ],
            &[],
        );
        let live = "let s = Pieces.";
        let items = complete(&project, "Game", live, live.len());
        assert_eq!(labels(&items), ["Circle", "Point", "count"]);
        assert_eq!(find(&items, "Circle").kind, CompletionKind::Constructor);
        assert_eq!(
            find(&items, "Circle").detail.as_deref(),
            Some("Pieces.Circle : (float) => Shape")
        );
        assert_eq!(find(&items, "Point").kind, CompletionKind::Constructor);
        assert_eq!(
            find(&items, "Point").detail.as_deref(),
            Some("Pieces.Point : Shape")
        );
        assert_eq!(find(&items, "count").kind, CompletionKind::Value);
    }

    // 3c. Own-module bare constructors at the top level — for the entry, and
    // for the sibling that owns the ADT (via `current_module`).
    #[test]
    fn own_module_bare_constructors() {
        let entry = project_of(
            &[(
                "game.fun",
                "type Dir = | Left | Right\nlet main = () => 1.0",
            )],
            &[],
        );
        let items = complete(&entry, "Game", "let x = ", "let x = ".len());
        assert!(has(&items, "Left"));
        assert!(has(&items, "Right"));
        assert_eq!(find(&items, "Left").kind, CompletionKind::Constructor);

        let sibling = project_of(
            &[
                ("game.fun", STUB),
                ("pieces.fun", "type Shape = | Circle(r: float) | Point"),
            ],
            &[],
        );
        let items = complete(&sibling, "Pieces", "let x = ", "let x = ".len());
        assert!(
            has(&items, "Circle"),
            "own ctor bare in {:?}",
            labels(&items)
        );
        assert!(has(&items, "Point"));
        assert_eq!(find(&items, "Circle").kind, CompletionKind::Constructor);
    }

    // 4. Top-level: keywords + own globals + modules; NOT qualified labels, NOT
    // the entry name.
    #[test]
    fn top_level_completion() {
        let project = project_of(
            &[
                ("game.fun", STUB),
                (
                    "utils.fun",
                    "let clamp = (lo: float, hi: float, x: float): float => x",
                ),
            ],
            &[("Scene", SCENE)],
        );
        let live = "let x = ";
        let items = complete(&project, "Game", live, live.len());
        assert_eq!(find(&items, "let").kind, CompletionKind::Keyword);
        assert!(has(&items, "main"), "own def bare in {:?}", labels(&items));
        assert_eq!(find(&items, "Utils").kind, CompletionKind::Module);
        assert_eq!(find(&items, "Scene").kind, CompletionKind::Module);
        assert_eq!(find(&items, "List").kind, CompletionKind::Module);
        assert!(!has(&items, "Utils.clamp"), "qualified label leaked");
        assert!(!has(&items, "Game"), "entry name offered as a module");
    }

    // 5. Editing a sibling: its own defs are bare; its own name is not in the
    // module list.
    #[test]
    fn editing_a_sibling_offers_own_defs_bare() {
        let project = project_of(
            &[
                ("game.fun", STUB),
                (
                    "utils.fun",
                    "let clamp = (lo: float, hi: float, x: float): float => x\n\
                     let version = 3.0",
                ),
            ],
            &[("Scene", SCENE)],
        );
        let live = "let y = ";
        let items = complete(&project, "Utils", live, live.len());
        assert!(has(&items, "clamp"), "own def bare in {:?}", labels(&items));
        assert!(has(&items, "Scene"));
        assert!(!has(&items, "Utils"), "own module offered to itself");
    }

    // 6. Partial member `Scene.cu` → `cube` only.
    #[test]
    fn partial_member_filters() {
        let items = game(STUB, &[("Scene", SCENE)], "let s = Scene.cu");
        assert_eq!(labels(&items), ["cube"]);
    }

    // 7. Broken/last-good: candidates come from the loaded project even though
    // the live buffer does not parse.
    #[test]
    fn broken_buffer_uses_last_good_project() {
        // Project loaded from a good buffer that never mentions Scene; the
        // prelude gives it the signatures.
        let items = game(
            "let x = 1.0",
            &[("Scene", SCENE)],
            "let x = 1.0\nlet s = Scene.",
        );
        assert!(has(&items, "cube"));
        assert!(has(&items, "sphere"));
    }

    // 8. Broken elsewhere than the cursor line — the whole buffer lexes but
    // does not parse; members are still offered.
    #[test]
    fn broken_earlier_line_still_completes() {
        let items = game(
            "let x = 1.0",
            &[("Scene", SCENE)],
            "let broken =\nlet s = Scene.",
        );
        assert!(has(&items, "cube"));
    }

    // 9. Unknown module → empty.
    #[test]
    fn unknown_module_is_empty() {
        let items = game(STUB, &[("Scene", SCENE)], "let s = Nope.");
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // 10 (PR 2). A record-typed value's fields are offered after its dot. The
    // buffer is FRESH (`v.x` complete, cursor between the dot and `x`).
    #[test]
    fn record_field_completion() {
        let src = "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => v.x";
        let offset = src.find("v.").unwrap() + 2; // after the dot
        let items = fresh(src, &[("Scene", SCENE)], offset);
        assert_eq!(labels(&items), ["x", "y"]);
        assert_eq!(find(&items, "x").kind, CompletionKind::Field);
        assert_eq!(find(&items, "x").detail.as_deref(), Some("x : float"));
        assert_eq!(find(&items, "y").detail.as_deref(), Some("y : float"));
    }

    // 11 (PR 2, flip of the old boundary). Locals ARE offered when the buffer
    // is fresh: a partial `sc` inside the lambda body offers the param `score`.
    #[test]
    fn locals_offered_when_fresh() {
        // Fresh: the project src and the live buffer are the SAME valid text;
        // the cursor sits after `sc` of the body reference `score`.
        let src = "let f = (score) => score";
        let offset = src.rfind("score").unwrap() + 2; // inside the reference
        let items = fresh(src, &[], offset);
        assert!(
            has(&items, "score"),
            "local not offered: {:?}",
            labels(&items)
        );
    }

    // PR 2. A lambda param is offered (with its type) inside the lambda body,
    // and NOT in a later def's body.
    #[test]
    fn lambda_param_offered_in_its_body_only() {
        let src = "let f = (score: float) => score\nlet g = (yy) => yy";
        let project = project_of(&[("game.fun", src)], &[]);

        // Inside `f`'s body: `score` is offered, typed from its annotation.
        let in_f = complete(&project, "Game", src, src.find("=> score").unwrap() + 8);
        assert_eq!(
            find(&in_f, "score").detail.as_deref(),
            Some("score : float")
        );
        assert_eq!(find(&in_f, "score").kind, CompletionKind::Value);

        // Inside `g`'s body (end of buffer): `g`'s own param, but NOT `score`.
        let in_g = complete(&project, "Game", src, src.len());
        assert!(has(&in_g, "yy"), "g's own param: {:?}", labels(&in_g));
        assert!(!has(&in_g, "score"), "outer param leaked into a later def");
    }

    // PR 2. Match-arm pattern binders are in scope inside THAT arm only.
    #[test]
    fn match_arm_binders_scope_to_their_arm() {
        let src = "type Shape = | Circle(r: float) | Square(s: float)\n\
                   let f = (sh: Shape) => match sh with \
                   | Circle(rad) => rad | Square(side) => side";
        let project = project_of(&[("game.fun", src)], &[]);

        // In the Circle arm (after the body reference `rad`): `rad`, not `side`.
        let in_circle = complete(&project, "Game", src, src.find("rad |").unwrap() + 3);
        assert_eq!(
            find(&in_circle, "rad").detail.as_deref(),
            Some("rad : float")
        );
        assert!(!has(&in_circle, "side"), "other arm's binder leaked");

        // In the Square arm (end of buffer): `side`, not `rad`.
        let in_square = complete(&project, "Game", src, src.len());
        assert!(
            has(&in_square, "side"),
            "arm binder: {:?}",
            labels(&in_square)
        );
        assert!(!has(&in_square, "rad"), "other arm's binder leaked");
    }

    // PR 2. A `let … in` binder is in scope in the `in` body, NOT in its own
    // value (`let` is non-recursive — see `crate::lower`).
    #[test]
    fn let_in_binder_scopes_to_the_body() {
        let src = "let f = (x: float) => let y = x + 1.0 in y";
        let project = project_of(&[("game.fun", src)], &[]);

        // In the `in` body (end, partial `y`): the `let` binder `y` is offered.
        let in_body = complete(&project, "Game", src, src.len());
        assert_eq!(find(&in_body, "y").detail.as_deref(), Some("y : float"));

        // In `y`'s value (`x + 1.0`, empty partial): `x` is in scope, `y` NOT.
        let in_value = complete(&project, "Game", src, src.find("let y = ").unwrap() + 8);
        assert!(
            has(&in_value, "x"),
            "param in the value: {:?}",
            labels(&in_value)
        );
        assert!(!has(&in_value, "y"), "the binder leaked into its own value");
    }

    // PR 2. Shadowing: a param named like a global yields ONE `score` item,
    // and its detail is the LOCAL's type (the inner binder wins dedup).
    #[test]
    fn local_shadows_a_same_named_global() {
        let src = "let score = \"hi\"\nlet f = (score: float) => score";
        let items = fresh(src, &[], src.len());
        assert_eq!(
            items.iter().filter(|i| i.label == "score").count(),
            1,
            "expected a single `score`: {:?}",
            items
        );
        assert_eq!(
            find(&items, "score").detail.as_deref(),
            Some("score : float")
        );
    }

    // PR 2. THE trigger keystroke: typing `.` breaks the parse, so the cache
    // still holds the pre-dot text — the member gate accepts exactly that
    // one-edit shape and fields appear at the moment the editor auto-triggers.
    #[test]
    fn record_fields_offered_at_the_dot_keystroke() {
        let src = "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => v";
        let project = project_of(&[("game.fun", src)], &[]);
        let live = format!("{src}.");
        let items = complete(&project, "Game", &live, live.len());
        assert_eq!(labels(&items), ["x", "y"]);
        assert_eq!(find(&items, "x").kind, CompletionKind::Field);

        // And with a partial being typed after the dot: `.y` filters to `y`.
        let live = format!("{src}.y");
        let items = complete(&project, "Game", &live, live.len());
        assert_eq!(labels(&items), ["y"]);
    }

    // PR 2. The dot-keystroke gate also holds mid-buffer: inserting `.` before
    // existing trailing text still restores the cached src when removed.
    #[test]
    fn record_fields_offered_mid_buffer_dot() {
        let src = "type V = { x: float }\nlet f = (v: V) => v + 1.0";
        let project = project_of(&[("game.fun", src)], &[]);
        let dot = src.find("v +").unwrap() + 1;
        let live = format!("{}.{}", &src[..dot], &src[dot..]);
        let items = complete(&project, "Game", &live, dot + 1); // cursor after the dot
        assert_eq!(labels(&items), ["x"]);
    }

    // PR 2. Value bindings shadow namespaces, mirroring lowering's resolution
    // order: a record-typed param named `List` answers with its FIELDS (not
    // the builtins), and a non-record def named `Scene` answers with nothing
    // (not the prelude members — field access on a float has no members).
    #[test]
    fn value_bindings_shadow_namespaces() {
        let src = "type Vec2 = { x: float, y: float }\nlet f = (List: Vec2) => List.x";
        let offset = src.find("List.").unwrap() + 5; // after the dot
        let items = fresh(src, &[], offset);
        assert_eq!(labels(&items), ["x", "y"], "builtins leaked past a local");

        let src = "let Scene = 1.0\nlet f = () => Scene";
        let project = project_of(&[("game.fun", src)], &[("Scene", SCENE)]);
        let live = format!("{src}.");
        let items = complete(&project, "Game", &live, live.len());
        assert!(
            items.is_empty(),
            "prelude members leaked past a def: {:?}",
            labels(&items)
        );
    }

    // PR 2. A partial after the dot filters the record fields.
    #[test]
    fn record_field_partial_filters() {
        let src = "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => v.x";
        let items = fresh(src, &[], src.len()); // cursor after `v.x` → partial `x`
        assert_eq!(labels(&items), ["x"]);
    }

    // PR 2. A non-record (here inference-variable) qualifier offers no fields —
    // gradual honesty, matching how hover shows Unknown.
    #[test]
    fn unknown_typed_qualifier_offers_no_fields() {
        let src = "let f = (u) => u.z";
        let offset = src.find("u.").unwrap() + 2; // after the dot
        let items = fresh(src, &[], offset);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // PR 2, THE GATE (low-confidence rule). When the live buffer is NOT the
    // cached project src (broken/stale), the scope-aware layers are skipped:
    // no locals at the top level, and `v.` resolves no fields. The v1 textual
    // answers (keywords, modules, prelude members) still work.
    #[test]
    fn stale_buffer_falls_back_to_v1() {
        let project = project_of(
            &[(
                "game.fun",
                "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => v.x",
            )],
            &[("Scene", SCENE)],
        );

        // A broken top-level buffer (differs from the cached src) with an EMPTY
        // partial: the gate is closed, so the param local `v` is suppressed,
        // while the v1 keywords/modules still answer.
        let stale_top = "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => ";
        let top = complete(&project, "Game", stale_top, stale_top.len());
        assert!(!has(&top, "v"), "a local leaked through a stale buffer");
        assert!(has(&top, "let"), "v1 keyword still offered");
        assert!(has(&top, "Scene"), "v1 module still offered");

        // A stale member buffer `v.`: record fields must not resolve.
        let stale_v = "type Vec2 = { x: float, y: float }\nlet len = (v: Vec2) => v.";
        let member = complete(&project, "Game", stale_v, stale_v.len());
        assert!(
            member.is_empty(),
            "record fields resolved stale: {:?}",
            labels(&member)
        );

        // …but a v1 prelude member (no freshness needed) still answers.
        let scene = complete(&project, "Game", "let s = Scene.", "let s = Scene.".len());
        assert!(has(&scene, "cube"), "v1 prelude member still works");
    }

    // PR 2. Sibling-file offsets: a local in a def in utils.fun is offered when
    // editing utils.fun — the base translation (`file.base + offset`) works for
    // a file whose base is > 0.
    #[test]
    fn sibling_file_locals_use_base_translation() {
        let utils = "let g = (val: float) => val";
        let project = project_of(&[("game.fun", STUB), ("utils.fun", utils)], &[]);
        let items = complete(&project, "Utils", utils, utils.len());
        assert!(has(&items, "val"), "sibling local: {:?}", labels(&items));
        assert_eq!(find(&items, "val").detail.as_deref(), Some("val : float"));
    }

    // PR 2. A value typed as a SIBLING module's record offers that record's
    // fields — the TypeDef lookup uses the canonical name (`Utils.Vec2`).
    #[test]
    fn record_field_completion_for_sibling_typed_value() {
        let src = "let mk = (v: Utils.Vec2) => v.x";
        let project = project_of(
            &[
                ("game.fun", src),
                ("utils.fun", "type Vec2 = { x: float, y: float }"),
            ],
            &[],
        );
        let offset = src.find("v.").unwrap() + 2; // after the dot
        let items = complete(&project, "Game", src, offset);
        assert_eq!(labels(&items), ["x", "y"]);
        assert_eq!(find(&items, "x").kind, CompletionKind::Field);
        assert_eq!(find(&items, "x").detail.as_deref(), Some("x : float"));
    }

    // 12. Partial top-level `Sc` → `Scene`, not `let`, not `List`.
    #[test]
    fn partial_top_level_filters() {
        let items = game(STUB, &[("Scene", SCENE)], "let x = Sc");
        assert!(has(&items, "Scene"));
        assert!(!has(&items, "let"));
        assert!(!has(&items, "List"));
    }

    // 14. Offset 0 / empty buffer — top-level set, no panic.
    #[test]
    fn empty_buffer_offers_top_level() {
        let project = project_of(&[("game.fun", STUB)], &[]);
        let items = complete(&project, "Game", "", 0);
        assert!(has(&items, "let"));
    }

    // 15. `.` after a number → empty (the token before `Dot` is a Number).
    #[test]
    fn dot_after_number_is_empty() {
        assert!(game(STUB, &[], "let x = 1.").is_empty());
        assert!(game(STUB, &[], "let x = 1.5.").is_empty());
    }

    // 16. Chained `a.b.` → empty (a multi-segment qualifier matches no
    // module). PR 2: chained members flip this.
    #[test]
    fn chained_member_is_empty_pr2_boundary() {
        let items = game(STUB, &[("Scene", SCENE)], "let x = a.b.");
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // 17. Cursor inside a string → empty (the prefix lexes as an unterminated
    // string).
    #[test]
    fn cursor_in_string_is_empty() {
        let live = "let s = \"Scene.";
        let items = game(STUB, &[("Scene", SCENE)], live);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // 18. Cursor in a line comment → empty (the gap contains `//`).
    #[test]
    fn cursor_in_comment_is_empty() {
        let items = game(STUB, &[("Scene", SCENE)], "let x = 1.0 // Scene.");
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // 18b. A fresh line AFTER a trailing comment is a normal top-level
    // position — only a comment on the cursor's own line swallows it.
    #[test]
    fn fresh_line_after_comment_offers_top_level() {
        let items = game(STUB, &[], "let x = 1.0 // note\n");
        assert!(has(&items, "let"), "{:?}", labels(&items));
    }

    // 18c. A line break after `Ident.` ends the member context — the cursor is
    // on a fresh line, not finishing the access.
    #[test]
    fn newline_after_dot_is_top_level() {
        let items = game(STUB, &[("Scene", SCENE)], "let x = Scene.\n");
        assert!(!has(&items, "cube"), "{:?}", labels(&items));
        assert!(has(&items, "let"));
    }

    // A generic type's constructors carry its parameters in the detail.
    #[test]
    fn generic_constructor_detail_keeps_type_params() {
        let project = project_of(
            &[
                ("game.fun", STUB),
                ("pieces.fun", "type Box<'a> = | Full(value: 'a) | Empty"),
            ],
            &[],
        );
        let live = "let s = Pieces.";
        let items = complete(&project, "Game", live, live.len());
        assert_eq!(
            find(&items, "Full").detail.as_deref(),
            Some("Pieces.Full : ('a) => Box<'a>")
        );
        assert_eq!(
            find(&items, "Empty").detail.as_deref(),
            Some("Pieces.Empty : Box<'a>")
        );
    }

    // Drift guard: this match is exhaustive over `Builtin`, so adding a
    // variant fails to compile here until BUILTINS (above) offers it too.
    #[test]
    fn builtins_list_is_exhaustive() {
        for &b in &BUILTINS {
            match b {
                Builtin::ListMap
                | Builtin::ListFilter
                | Builtin::ListFold
                | Builtin::ListRange
                | Builtin::ListGrid
                | Builtin::ListMaximum
                | Builtin::ListLength
                | Builtin::ListAppend
                | Builtin::ListFlatten
                | Builtin::ListAny
                | Builtin::ListAll
                | Builtin::ListReverse
                | Builtin::ListIsEmpty
                | Builtin::MathSin
                | Builtin::MathCos
                | Builtin::MathSqrt
                | Builtin::MathAbs
                | Builtin::MathFloor
                | Builtin::MathAtan2
                | Builtin::MathMod
                | Builtin::MathMin
                | Builtin::MathMax
                | Builtin::MathPow
                | Builtin::MathPi
                | Builtin::MathClamp01
                | Builtin::TextConcat
                | Builtin::TextFromFloat
                | Builtin::TextFixed
                | Builtin::TextToBullets
                | Builtin::TextSplit
                | Builtin::TextJoin
                | Builtin::TextParseFloat
                | Builtin::DebugLog => {}
            }
        }
        assert_eq!(BUILTINS.len(), 33, "BUILTINS must list every Builtin");
    }

    // 19. A full keyword typed (`let`, cursor at end) still offers `let`.
    #[test]
    fn full_keyword_still_offered() {
        let items = game(STUB, &[], "let");
        assert!(has(&items, "let"));
    }
}
