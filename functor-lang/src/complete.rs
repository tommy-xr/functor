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
//! v1 non-goals (PR 2): locals/parameters, typed record fields (`pos.`),
//! chained members (`a.b.`), type-position members (`Scene.t` /
//! `Pieces.Shape` in annotations), and `open`ed modules' exports offered
//! bare (the merged module does not retain `open` metadata). Each of the
//! testable ones is pinned as an empty result by a boundary test below so
//! PR 2 flips assertions rather than discovering surprises.

use std::collections::BTreeSet;

use crate::ast::{TypeBody, VariantDecl};
use crate::eval::{builtin_name, Builtin};
use crate::hover::type_name_text;
use crate::lexer::{Token, TokenKind};
use crate::project::Project;
use crate::types::{builtin_signature, Type};

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
}

/// The complete builtin registry. Hand-listed because [`Builtin`] is not
/// iterable — keep in sync with `eval::Builtin` (17 variants).
const BUILTINS: [Builtin; 17] = [
    Builtin::ListMap,
    Builtin::ListFilter,
    Builtin::ListFold,
    Builtin::ListRange,
    Builtin::ListGrid,
    Builtin::ListMaximum,
    Builtin::MathSin,
    Builtin::MathCos,
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

/// The keywords offered at a top-level position: the 8 lexer keywords plus
/// the contextual `open` (`lexer.rs`).
const KEYWORDS: [&str; 9] = [
    "let", "type", "open", "mut", "with", "in", "match", "true", "false",
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
        Context::Member { qualifier, partial } => member_candidates(project, &qualifier, &partial),
        Context::TopLevel { partial } => top_level_candidates(project, current_module, &partial),
    }
}

/// The cursor's completion context, derived textually from the prefix.
enum Context {
    /// After `Qualifier.` — offer that module's members filtered by `partial`.
    Member { qualifier: String, partial: String },
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
fn member_candidates(project: &Project, qualifier: &str, partial: &str) -> Vec<CompletionItem> {
    let module = &project.module;
    let prefix = format!("{qualifier}.");
    // Lazy: only checked now that a context matched (defs need the checker's
    // types for their details). The check is on `project` (last-good), never
    // on the live buffer.
    let (_, types) = project.check_with_types();
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

    // Builtins whose namespace is the qualifier (`List.map`, …).
    for &b in &BUILTINS {
        let name = builtin_name(b);
        if let Some(label) = name.strip_prefix(&prefix) {
            items.push(CompletionItem {
                label: label.to_string(),
                detail: Some(format!("{name} : {}", builtin_signature(b))),
                kind: CompletionKind::Function,
            });
        }
    }

    finish(items, partial)
}

/// Candidates at a top-level position: keywords, the current module's own
/// defs and constructors (bare), and the visible module names.
fn top_level_candidates(
    project: &Project,
    current_module: &str,
    partial: &str,
) -> Vec<CompletionItem> {
    let module = &project.module;
    let entry = &project.entry;
    let (_, types) = project.check_with_types();
    let mut items = Vec::new();

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

    // 10. Lowercase qualifier `pos.` (a record field) — empty today. PR 2:
    // typed record-field completion flips this.
    #[test]
    fn record_field_qualifier_is_empty_pr2_boundary() {
        let items = game(STUB, &[("Scene", SCENE)], "let x = pos.");
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // 11. Locals are not offered at the top level. PR 2: locals/params flip
    // this.
    #[test]
    fn locals_not_offered_pr2_boundary() {
        let items = game("let f = (score) => score", &[], "let f = (score) => sc");
        assert!(!has(&items, "score"), "local leaked into completion");
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
                | Builtin::MathSin
                | Builtin::MathCos
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
        assert_eq!(BUILTINS.len(), 17, "BUILTINS must list every Builtin");
    }

    // 19. A full keyword typed (`let`, cursor at end) still offers `let`.
    #[test]
    fn full_keyword_still_offered() {
        let items = game(STUB, &[], "let");
        assert!(has(&items, "let"));
    }
}
