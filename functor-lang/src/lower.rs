//! AST → core-IR lowering — Track B2 of `docs/functor-lang.md`. Assigns stable IDs,
//! resolves names, and desugars pipelines. No typechecking, no evaluation.
//!
//! ## Top-level visibility
//!
//! Top-level `let`s are **mutually visible**: a def may reference another def
//! declared later in the file (top-level bindings behave letrec-style, as in
//! other functional languages), so definition order never forces a
//! topological rewrite of game code. Because a def's *name* is its stable
//! identity (the hot-reload rebind key, docs/functor-lang.md B5), duplicate top-level
//! names are lowering errors rather than shadowing. Types and values are
//! **separate namespaces** (as in F#/OCaml): `type Foo` and `let Foo` may
//! coexist — each namespace keys its own identities — but a duplicate within
//! either namespace is an error. **Constructors live in the value
//! namespace**: they resolve bare (`Circle(2.0)`, not `Shape.Circle`), so a
//! constructor name must be unique across every variant type in the module
//! AND must not collide with a top-level `let` — both are lowering errors.
//! Duplicate parameter names within one lambda are errors for the same
//! reason (the last one would silently win).
//!
//! ## Modules (B8)
//!
//! Within a project (see [`crate::project`]) a module lowers with a
//! [`ProjectEnv`]: every sibling file's exports are visible, **qualified by
//! default** (`Utils.clamp`, `Utils.Circle`, `Utils.Shape` — no import
//! needed), and `open Utils` brings a module's defs/constructors/types into
//! scope unqualified (collisions with this module's own names or another
//! `open` are errors naming both sides). Lowering **canonicalizes** names
//! into one project-wide namespace: a non-entry module `M`'s defs, types,
//! and constructor tags become `M.name`, while the **entry module stays
//! bare** — so a single-file project lowers byte-identically to plain
//! [`lower`], and the entry's `init`/`tick`/`draw` contract keys don't
//! change. Cross-module value references become ordinary
//! [`ExprKind::Global`]s ("Utils.clamp") — late-bound at call time like any
//! global, which is exactly the hot-reload rebind seam — and constructor
//! references/patterns carry canonical tags, so cross-module `match` works
//! structurally. The lowered modules concatenate into ONE merged
//! [`Module`]; [`IdBases`] threads ID counters across files so the merged
//! module is a single ID space (as it is a single span space — see
//! [`crate::lexer::lex`]).
//!
//! A file may also declare **inline** `module Name { … }` blocks (one level
//! deep). They are namespaces within the file: their members canonicalize
//! with one extra segment (`utils.fun` + `module Grid` → `Utils.Grid.cell`;
//! the entry's stay bare → `Grid.cell`), their own names shadow the file's
//! same-named ones, and they are reachable BARE inside the declaring file
//! (`Grid.cell`) — which is why a project refuses an inline module name
//! that collides with a builtin namespace, another file's module, an
//! `open`ed name, or the file's own top-level names.
//!
//! ## Name resolution
//!
//! For an identifier `first(.rest)*` (the parser only builds multi-segment
//! names while the qualifier segment starts uppercase — see
//! [`crate::ast::ExprKind::Ident`]):
//!
//! - `first` names an enclosing lambda parameter → [`ExprKind::Local`]
//!   (innermost scope wins, so a parameter — or a pattern variable — shadows
//!   a same-named global or constructor); any remaining segments become
//!   [`ExprKind::FieldAccess`] on it.
//! - `first` names a top-level `let` → [`ExprKind::Global`]; remaining
//!   segments likewise become field access.
//! - `first` names a declared variant constructor → [`ExprKind::Ctor`]
//!   (the `Type.Ctor` qualified form is deliberately NOT supported — naming
//!   a type's own constructor through it is an error teaching the bare
//!   form, since it could otherwise only fail as an unknown external at run
//!   time).
//! - `first` names an `open`ed module's def or constructor → the qualified
//!   [`ExprKind::Global`] / [`ExprKind::Ctor`].
//! - `first` names one of THIS file's inline `module` blocks, or the
//!   leading segments name a sibling module (`Utils`, `Game.Server`) →
//!   resolve the next segment against its exports (def or constructor; an
//!   unknown member is an error); further segments become field access.
//!   (Builtins like `List.map` cannot collide: module names — file *and*
//!   inline — matching builtin/prelude namespaces are refused at project
//!   load.)
//! - Otherwise, a qualified name (`Text.toBullets`) → [`ExprKind::External`],
//!   the builtin/host seam — and an unqualified name is an "unknown name"
//!   error at the identifier's span (with a hint when it names a module).
//!
//! ## Match lowering
//!
//! Pattern variables get fresh [`BindingId`]s scoped to their arm's body
//! (each arm is its own scope level — bindings never leak between arms);
//! they are plain immutable bindings, so lambdas may capture them. A
//! duplicate variable within one pattern, an unknown constructor in a
//! pattern, and a constructor pattern whose sub-pattern count differs from
//! the declared field count are all lowering errors.
//!
//! ## Pipeline desugaring
//!
//! Each pipeline stage becomes a call with the piped value **appended** as
//! the LAST argument (thread-last): `x |> f` → `f(x)`, `x |> g(a)` → `g(a, x)`,
//! so `x |> f |> g(a)` → `g(a, f(x))`. Because `|>` is syntax, the stage
//! lowers directly to the SATURATED call — `x |> g(a)` is `g(a, x)`, never a
//! partial `g(a)` — so pipes stay allocation-free. The desugared call carries the span of
//! its stage — which means its first argument's span lies *outside* the
//! call's own span (it came from an earlier stage); diagnostics must not
//! assume parent spans contain child spans. The IR has no pipeline node.

use crate::ast;
use crate::ir::*;
use crate::span::Span;
use crate::LowerError;
use std::collections::{HashMap, HashSet};

/// Lower a parsed [`ast::Program`] to an IR [`Module`] (single-file: no
/// project context, IDs from zero).
pub fn lower(program: ast::Program) -> Result<Module, LowerError> {
    lower_module(program, None, IdBases::default()).map(|(module, _, _)| module)
}

/// What a module contributes to sibling files, derived from its AST before
/// lowering: top-level `let` names, constructors (with arities), and type
/// names. Duplicates are NOT validated here — lowering the module itself
/// reports them.
#[derive(Default)]
pub(crate) struct Exports {
    pub defs: HashSet<String>,
    pub ctors: HashMap<String, usize>,
    pub types: HashSet<String>,
    /// Interface (`.funi`) value signature names — a qualified reference to one
    /// stays an [`ExprKind::External`] (host-resolved at runtime), unlike a
    /// `def` which becomes a [`ExprKind::Global`].
    pub signatures: HashSet<String>,
}

pub(crate) fn exports_of(items: &[ast::Item]) -> Exports {
    let mut exports = Exports::default();
    for item in items {
        match item {
            ast::Item::Let(decl) => {
                exports.defs.insert(decl.name.clone());
            }
            ast::Item::Type(decl) => {
                exports.types.insert(decl.name.clone());
                if let ast::TypeBody::Variants(variants) = &decl.body {
                    for variant in variants {
                        exports
                            .ctors
                            .insert(variant.name.clone(), variant.fields.len());
                    }
                }
            }
            ast::Item::Sig(decl) => {
                exports.signatures.insert(decl.name.clone());
            }
            ast::Item::Open(_) => {}
            // An expect binds nothing.
            ast::Item::Expect(_) => {}
            // An inline module's members belong to ITS namespace, not the
            // file's: the caller calls `exports_of` again on its own items.
            ast::Item::Module(_) => {}
        }
    }
    exports
}

/// The project context a module lowers in (see the module docs): its own
/// name, the entry module's name (entry members canonicalize BARE), and
/// every module's exports (self included).
pub(crate) struct ProjectEnv<'a> {
    pub name: &'a str,
    pub entry: &'a str,
    pub modules: &'a HashMap<String, Exports>,
}

/// Starting IDs for a module's lowering: a project threads these across its
/// files so [`DefId`]/[`BindingId`]/[`ExprId`]s are unique project-wide.
#[derive(Clone, Copy, Default)]
pub(crate) struct IdBases {
    pub def: u32,
    pub binding: u32,
    pub expr: u32,
}

/// [`lower`] within a project. Returns the lowered module, the next free
/// IDs, and the sibling modules this one references — the project's
/// dependency edges (`open`s included).
pub(crate) fn lower_in_project(
    program: ast::Program,
    env: &ProjectEnv,
    bases: IdBases,
) -> Result<(Module, IdBases, HashSet<String>), LowerError> {
    lower_module(program, Some(env), bases)
}

/// Which module `open <written>` in the file `file` names: one of that
/// file's own inline `module` blocks (keyed `File.Written`) if it has one,
/// otherwise the name as written (`Utils`, or a sibling's `Game.Server`).
/// The result may still be absent from `modules` — the caller reports that.
pub(crate) fn open_module_path(
    file: &str,
    written: &str,
    modules: &HashMap<String, Exports>,
) -> String {
    let relative = format!("{file}.{written}");
    if modules.contains_key(&relative) {
        relative
    } else {
        written.to_string()
    }
}

/// One namespace's declared names — a file's top level, or an inline
/// `module` block's body.
#[derive(Default)]
struct Names {
    globals: HashSet<String>,
    ctors: HashMap<String, usize>,
    types: HashSet<String>,
    /// Constructors → the variant type that declares them. Only used to
    /// teach the `Shape.Circle` mistake (constructors resolve bare);
    /// resolution itself never consults it.
    ctor_types: HashMap<String, String>,
}

/// Pass 1 over one namespace's items: collect names so defs are mutually
/// visible (see module docs) and duplicates fail loud. Types and values are
/// separate namespaces; constructors join the VALUE namespace (they resolve
/// bare), so they must not collide with `let`s or with each other — across
/// variant types too.
fn collect_names(items: &[ast::Item]) -> Result<Names, LowerError> {
    let Names {
        mut globals,
        mut ctors,
        types: mut type_names,
        mut ctor_types,
    } = Names::default();
    for item in items {
        match item {
            ast::Item::Open(_) => {}
            // An expect declares no names.
            ast::Item::Expect(_) => {}
            // Inline modules are their own namespaces, collected separately
            // (and checked against these names by the caller).
            ast::Item::Module(_) => {}
            // A signature shares the value namespace with `let`s for DUPLICATE
            // DETECTION only — it is never lowered to a `Global` (references
            // stay `External`; `.funi` files have no value expressions, so
            // `globals` is only read for defs). Grouped here purely to reuse
            // the collision checks.
            ast::Item::Let(ast::LetDecl { name, span, .. })
            | ast::Item::Sig(ast::SigDecl { name, span, .. }) => {
                if ctors.contains_key(name) {
                    return Err(LowerError {
                        message: format!(
                            "duplicate definition `{name}` (constructors live in the value namespace)"
                        ),
                        span: *span,
                    });
                }
                if !globals.insert(name.clone()) {
                    return Err(LowerError {
                        message: format!("duplicate definition `{name}`"),
                        span: *span,
                    });
                }
            }
            ast::Item::Type(decl) => {
                // Builtin type names would shadow the primitives in
                // annotations (the checker resolves `float` before user
                // types), yielding nonsense like "expected float, got float".
                if matches!(
                    decl.name.as_str(),
                    "float" | "bool" | "string" | "unknown" | "List" | "Map"
                ) {
                    return Err(LowerError {
                        message: format!("cannot redeclare builtin type `{}`", decl.name),
                        span: decl.span,
                    });
                }
                if !type_names.insert(decl.name.clone()) {
                    return Err(LowerError {
                        message: format!("duplicate definition `{}`", decl.name),
                        span: decl.span,
                    });
                }
                if let ast::TypeBody::Variants(variants) = &decl.body {
                    for variant in variants {
                        if ctors.contains_key(&variant.name) {
                            return Err(LowerError {
                                message: format!("duplicate constructor `{}`", variant.name),
                                span: variant.span,
                            });
                        }
                        if globals.contains(&variant.name) {
                            return Err(LowerError {
                                message: format!(
                                    "duplicate definition `{}` (constructors live in the value \
namespace)",
                                    variant.name
                                ),
                                span: variant.span,
                            });
                        }
                        ctors.insert(variant.name.clone(), variant.fields.len());
                        ctor_types.insert(variant.name.clone(), decl.name.clone());
                    }
                }
            }
        }
    }
    Ok(Names {
        globals,
        ctors,
        types: type_names,
        ctor_types,
    })
}

fn lower_module(
    program: ast::Program,
    project: Option<&ProjectEnv>,
    bases: IdBases,
) -> Result<(Module, IdBases, HashSet<String>), LowerError> {
    let Names {
        globals,
        ctors,
        types: type_names,
        ctor_types,
    } = collect_names(&program.items)?;

    // Pass 1a: the file's inline `module` blocks. Their names live in the
    // file's value AND type namespaces (so `Server.step` always resolves to
    // the module, never to field access on a `let Server`), and each block's
    // own names are collected the same way.
    let mut inline_names: HashMap<String, Exports> = HashMap::new();
    for item in &program.items {
        let ast::Item::Module(decl) = item else {
            continue;
        };
        if globals.contains(&decl.name)
            || ctors.contains_key(&decl.name)
            || type_names.contains(&decl.name)
        {
            return Err(LowerError {
                message: format!(
                    "`module {}` collides with the top-level `{}` in the same file — rename one \
(a module name occupies both the value and type namespaces)",
                    decl.name, decl.name
                ),
                span: decl.span,
            });
        }
        if inline_names.contains_key(&decl.name) {
            return Err(LowerError {
                message: format!("duplicate module `{}` in this file", decl.name),
                span: decl.span,
            });
        }
        // `collect_names` is the DUPLICATE check; the namespace itself is
        // read through the same `exports_of` the project keys it by.
        collect_names(&decl.items)?;
        inline_names.insert(decl.name.clone(), exports_of(&decl.items));
    }

    // Pass 1b: process `open`s, after the module's own names are known (an
    // `open` may precede or follow the definitions it collides with).
    // Collisions are eager load errors naming both sides — even for names
    // the module never uses.
    let mut open_values: HashMap<String, OpenedName> = HashMap::new();
    let mut open_types: HashMap<String, String> = HashMap::new();
    let mut deps: HashSet<String> = HashSet::new();
    for item in &program.items {
        let ast::Item::Open(decl) = item else {
            continue;
        };
        let Some(env) = project else {
            return Err(LowerError {
                message: format!(
                    "unknown module `{}` — modules are sibling `.fun` files loaded through the \
project entry (this file is being lowered on its own)",
                    decl.module
                ),
                span: decl.span,
            });
        };
        if decl.module == env.name {
            return Err(LowerError {
                message: format!(
                    "`open {}` in module `{}` itself — a module's own names are already in scope",
                    decl.module, decl.module
                ),
                span: decl.span,
            });
        }
        // `open Server` may name one of THIS file's inline modules (keyed
        // `File.Server`); `open Utils` / `open Game.Server` name another
        // file's module or its inline module.
        let module_path = open_module_path(env.name, &decl.module, env.modules);
        let Some(exports) = env.modules.get(&module_path) else {
            return Err(LowerError {
                message: format!(
                    "unknown module `{}` — modules are the sibling `.fun` files next to the \
entry, and the `module` blocks they declare",
                    decl.module
                ),
                span: decl.span,
            });
        };
        let span = decl.span;
        // The dependency edge is on the owning FILE (decision: edges are
        // file-to-file), so `open Game.Server` deps on `Game`.
        let file = module_path.split('.').next().unwrap_or(&module_path);
        if file != env.name {
            deps.insert(file.to_string());
        }
        // Value namespace: the opened module's defs, constructors, and
        // interface signatures.
        let mut values: Vec<(&String, OpenedName)> = exports
            .defs
            .iter()
            .map(|d| (d, OpenedName::Def(module_path.clone())))
            .chain(
                exports
                    .ctors
                    .iter()
                    .map(|(c, a)| (c, OpenedName::Ctor(module_path.clone(), *a))),
            )
            .chain(
                exports
                    .signatures
                    .iter()
                    .map(|s| (s, OpenedName::Signature(module_path.clone()))),
            )
            .collect();
        values.sort_by_key(|(name, _)| name.as_str().to_string());
        for (name, opened) in values {
            // An inline module name occupies this file's value AND type
            // namespaces, so an opened name may not shadow it — otherwise
            // `Server.step` would silently become field access on the
            // imported `Server`.
            if inline_names.contains_key(name) || globals.contains(name) || ctors.contains_key(name)
            {
                return Err(LowerError {
                    message: format!(
                        "open {}: `{name}` collides with this module's own `{name}` — qualify \
uses as `{}.{name}` instead of opening",
                        module_path, module_path
                    ),
                    span,
                });
            }
            if let Some(prev) = open_values.get(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: `{name}` is already in scope from `open {}` — qualify uses \
(`{}.{name}` / `{}.{name}`)",
                        module_path,
                        prev.module(),
                        prev.module(),
                        module_path
                    ),
                    span,
                });
            }
            open_values.insert(name.clone(), opened);
        }
        // Type namespace.
        let mut types: Vec<&String> = exports.types.iter().collect();
        types.sort();
        for name in types {
            if inline_names.contains_key(name) || type_names.contains(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: type `{name}` collides with this module's own `{name}` — \
qualify uses as `{}.{name}` instead of opening",
                        module_path, module_path
                    ),
                    span,
                });
            }
            if let Some(prev) = open_types.get(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: type `{name}` is already in scope from `open {prev}` — \
qualify uses (`{prev}.{name}` / `{}.{name}`)",
                        module_path, module_path
                    ),
                    span,
                });
            }
            open_types.insert(name.clone(), module_path.clone());
        }
    }

    // Pass 2: lower items in file order.
    let mut lowerer = Lowerer {
        globals,
        ctors,
        types: type_names,
        ctor_types,
        local_modules: inline_names,
        inline: None,
        project,
        open_values,
        open_types,
        deps,
        scopes: Vec::new(),
        next_binding: bases.binding,
        next_expr: bases.expr,
    };
    let mut out = Lowered {
        next_def: bases.def,
        types: Vec::new(),
        defs: Vec::new(),
        signatures: Vec::new(),
        expects: Vec::new(),
    };
    for item in program.items {
        match item {
            // An inline module: lower its items in ITS namespace (its own
            // names shadow the file's, and canonical names gain the extra
            // segment).
            ast::Item::Module(decl) => {
                lowerer.inline = Some(decl.name);
                for item in decl.items {
                    lowerer.lower_item(item, &mut out)?;
                }
                lowerer.inline = None;
            }
            item => lowerer.lower_item(item, &mut out)?,
        }
    }
    let Lowered {
        next_def,
        types,
        defs,
        signatures,
        expects,
    } = out;
    let bases = IdBases {
        def: next_def,
        binding: lowerer.next_binding,
        expr: lowerer.next_expr,
    };
    Ok((
        Module {
            types,
            defs,
            signatures,
            expects,
        },
        bases,
        lowerer.deps,
    ))
}

/// The IR a namespace's items lower into — one accumulator shared by the
/// file's top level and each inline `module` block.
struct Lowered {
    next_def: u32,
    types: Vec<TypeDef>,
    defs: Vec<Def>,
    signatures: Vec<Signature>,
    expects: Vec<ExpectDef>,
}

/// Which module a dotted reference's leading segments named.
enum ModuleKey {
    /// An inline `module` block in the file being lowered — reachable
    /// unqualified (`Server.step`), and resolved BEFORE sibling files.
    Local(String),
    /// Another module in the project: a file, or one of ITS inline modules
    /// (`Utils`, `Game.Server`). Keyed as [`ProjectEnv::modules`] keys it.
    Project(String),
}

impl ModuleKey {
    /// How the module is named in diagnostics.
    fn label(&self) -> &str {
        let (ModuleKey::Local(name) | ModuleKey::Project(name)) = self;
        name
    }
}

/// A name an `open` brought into scope: which module provides it, and (for
/// constructors) the declared arity.
#[derive(Clone)]
enum OpenedName {
    Def(String),
    Ctor(String, usize),
    /// An interface (`.funi`) signature — a bare use resolves to an
    /// [`ExprKind::External`], like a qualified one.
    Signature(String),
}

impl OpenedName {
    fn module(&self) -> &str {
        match self {
            OpenedName::Def(module)
            | OpenedName::Ctor(module, _)
            | OpenedName::Signature(module) => module,
        }
    }
}

struct Lowerer<'p> {
    globals: HashSet<String>,
    /// Declared variant constructors: name → declared field count. Part of
    /// the value namespace (pass 1 guarantees no overlap with `globals`).
    ctors: HashMap<String, usize>,
    /// This module's own declared type names (bare).
    types: HashSet<String>,
    /// This module's own constructors → the variant type that declares them.
    /// Only used to teach the `Shape.Circle` mistake (constructors resolve
    /// bare); resolution itself never consults it.
    ctor_types: HashMap<String, String>,
    /// This file's inline `module` blocks, by name. Qualified references
    /// (`Server.step`) resolve here BEFORE sibling files, so a file's own
    /// module shadows a same-named sibling.
    local_modules: HashMap<String, Exports>,
    /// The inline `module` block being lowered, if any. Its names are
    /// searched BEFORE the file's own (`globals`/`ctors`/`types`) and
    /// canonicalize with the extra module segment.
    inline: Option<String>,
    /// The project context, when lowering as part of one (B8 modules).
    project: Option<&'p ProjectEnv<'p>>,
    /// Names brought into scope by `open`s (collision-free by pass 1b).
    open_values: HashMap<String, OpenedName>,
    /// Type names brought into scope by `open`s: name → providing module.
    open_types: HashMap<String, String>,
    /// Sibling modules this module references (the project dep edges).
    deps: HashSet<String>,
    /// One level per enclosing lambda or `let … in`; lookup walks
    /// innermost-first. A lambda's level is a *boundary*: a `mut` binding
    /// found past one has been captured, which is an error (see the module
    /// docs and `~/notes/ideas/functor-lang/mutability.md`).
    scopes: Vec<ScopeLevel>,
    next_binding: u32,
    next_expr: u32,
}

struct ScopeLevel {
    lambda_boundary: bool,
    /// (name, binding, mutable)
    vars: Vec<(String, BindingId, bool)>,
}

/// A resolved local: where it lives and how it may be used.
struct Resolved {
    binding: BindingId,
    mutable: bool,
    /// The reference crosses a lambda boundary (it is a capture).
    captured: bool,
}

impl Lowerer<'_> {
    fn expr_id(&mut self) -> ExprId {
        let id = ExprId(self.next_expr);
        self.next_expr += 1;
        id
    }

    /// Canonicalize `member` of module path `module`: `M.member` (or
    /// `M.Inline.member`), except the ENTRY module's members stay bare — so
    /// the entry's `module Server` yields `Server.member` (see the module
    /// docs).
    fn qualify(&self, module: &str, member: &str) -> String {
        let prefix = self.canonical_prefix(module);
        if prefix.is_empty() {
            member.to_string()
        } else {
            format!("{prefix}.{member}")
        }
    }

    /// The canonical prefix a module path's members carry: `""` for the entry
    /// file's top level, `"Server"` for the entry's `module Server`,
    /// `"Utils"` / `"Utils.Grid"` for a sibling and its inline module.
    fn canonical_prefix(&self, module: &str) -> String {
        let Some(env) = self.project else {
            return module.to_string();
        };
        if module == env.entry {
            String::new()
        } else if let Some(inline) = module
            .strip_prefix(env.entry)
            .and_then(|rest| rest.strip_prefix('.'))
        {
            inline.to_string()
        } else {
            module.to_string()
        }
    }

    /// The canonical prefix of the namespace being lowered — the file's
    /// module, or `File.Inline` inside a `module` block. `None` outside a
    /// project (single-file lowering).
    fn current_path(&self) -> Option<String> {
        let env = self.project?;
        Some(match &self.inline {
            Some(name) => format!("{}.{name}", env.name),
            None => env.name.to_string(),
        })
    }

    /// Canonicalize one of the CURRENT namespace's own members.
    fn self_qualify(&self, member: &str) -> String {
        match self.current_path() {
            Some(path) => self.qualify(&path, member),
            // Outside a project an inline module still namespaces its members.
            None => match &self.inline {
                Some(name) => format!("{name}.{member}"),
                None => member.to_string(),
            },
        }
    }

    /// This file's inline `module` block currently being lowered, if any.
    fn inline_exports(&self) -> Option<&Exports> {
        self.local_modules.get(self.inline.as_deref()?)
    }

    /// Canonicalize `member` of an already-resolved module reference.
    fn qualify_key(&self, key: &ModuleKey, member: &str) -> String {
        match key {
            ModuleKey::Local(name) => match self.project {
                Some(env) => self.qualify(&format!("{}.{name}", env.name), member),
                None => format!("{name}.{member}"),
            },
            ModuleKey::Project(path) => self.qualify(path, member),
        }
    }

    /// The exports of an already-resolved module reference.
    fn exports_of_key(&self, key: &ModuleKey) -> &Exports {
        match key {
            ModuleKey::Local(name) => &self.local_modules[name],
            ModuleKey::Project(path) => self
                .project
                .and_then(|env| env.modules.get(path))
                .expect("resolved module path"),
        }
    }

    /// Record the dependency edge a resolved reference implies (a local
    /// inline module is in this very file, so it implies none).
    fn dep_key(&mut self, key: &ModuleKey) {
        if let ModuleKey::Project(path) = key {
            let path = path.clone();
            self.dep(&path);
        }
    }

    /// Canonicalize one of the enclosing FILE's top-level members (visible
    /// bare from inside an inline module).
    fn file_qualify(&self, member: &str) -> String {
        match self.project {
            Some(env) => self.qualify(env.name, member),
            None => member.to_string(),
        }
    }

    /// Record a cross-module reference (the project dependency edge). Edges
    /// are between FILES, so an inline module path normalizes to its owner.
    fn dep(&mut self, module: &str) {
        let file = module.split('.').next().unwrap_or(module);
        if self.project.is_some_and(|env| env.name != file) {
            self.deps.insert(file.to_string());
        }
    }

    /// The longest module a dotted reference's leading segments name, and
    /// how many segments it consumed. This file's own inline modules win
    /// over sibling modules (they are the nearer scope).
    fn resolve_module_prefix(&self, segments: &[&str]) -> Option<(ModuleKey, usize)> {
        if segments.len() > 1 && self.local_modules.contains_key(segments[0]) {
            return Some((ModuleKey::Local(segments[0].to_string()), 1));
        }
        for len in (1..segments.len()).rev() {
            let prefix = segments[..len].join(".");
            if self
                .project
                .is_some_and(|env| env.modules.contains_key(&prefix))
            {
                return Some((ModuleKey::Project(prefix), len));
            }
        }
        None
    }

    /// Lower one item into `out`, in the current namespace.
    fn lower_item(&mut self, item: ast::Item, out: &mut Lowered) -> Result<(), LowerError> {
        match item {
            // Opens were consumed in pass 1b; they leave no IR (and no
            // DefId — a file without opens numbers exactly as before).
            ast::Item::Open(_) => {}
            // Pass 2 peels inline modules off before calling this.
            ast::Item::Module(_) => unreachable!("nested modules are refused by the parser"),
            ast::Item::Type(decl) => {
                let id = DefId(out.next_def);
                out.next_def += 1;
                out.types.push(TypeDef {
                    params: decl.params.clone(),
                    id,
                    name: self.self_qualify(&decl.name),
                    body: self.canon_type_body(decl.body)?,
                    span: decl.span,
                });
            }
            ast::Item::Let(decl) => {
                let id = DefId(out.next_def);
                out.next_def += 1;
                out.defs.push(Def {
                    id,
                    name: self.self_qualify(&decl.name),
                    ty: decl.ty.map(|ty| self.canon_type(ty)).transpose()?,
                    value: self.expr(decl.value)?,
                    span: decl.span,
                });
            }
            // An interface signature: no body, no DefId. Canonicalize its name
            // and type (which may reference this module's — or another's —
            // types) so the checker can type externals against it.
            ast::Item::Sig(decl) => {
                out.signatures.push(Signature {
                    name: self.self_qualify(&decl.name),
                    ty: self.canon_type(decl.ty)?,
                    span: decl.span,
                });
            }
            // An expect: no name, no DefId — its expression lowers like a
            // def body (globals resolve canonically), kept in its own list
            // so session loading never evaluates it.
            ast::Item::Expect(decl) => {
                // The owning namespace's canonical prefix (empty for the
                // entry file's top level), matching a def's name prefix.
                let module = self
                    .current_path()
                    .map(|path| self.canonical_prefix(&path))
                    .unwrap_or_default();
                out.expects.push(ExpectDef {
                    module,
                    expr: self.expr(decl.expr)?,
                    span: decl.span,
                });
            }
        }
        Ok(())
    }

    /// Canonicalize a type annotation (see the module docs): this module's
    /// own type names and `open`ed ones gain their module qualifier;
    /// `Module.Type` references resolve against the project (an unknown
    /// member of a KNOWN module is an error; an unknown head stays symbolic
    /// — the checker's gradual seam).
    fn canon_type(&mut self, ty: ast::TypeName) -> Result<ast::TypeName, LowerError> {
        let args = ty
            .args
            .into_iter()
            .map(|arg| self.canon_type(arg))
            .collect::<Result<Vec<_>, _>>()?;
        let name = if ty.name.contains('.') {
            let segments: Vec<&str> = ty.name.split('.').collect();
            match self.resolve_module_prefix(&segments) {
                Some((key, consumed)) => {
                    let member = segments[consumed..].join(".");
                    if !self.exports_of_key(&key).types.contains(&member) {
                        return Err(LowerError {
                            message: format!("module `{}` has no type `{member}`", key.label()),
                            span: ty.span,
                        });
                    }
                    self.dep_key(&key);
                    self.qualify_key(&key, &member)
                }
                None => ty.name,
            }
        } else if self
            .inline_exports()
            .is_some_and(|ns| ns.types.contains(&ty.name))
        {
            self.self_qualify(&ty.name)
        } else if self.types.contains(&ty.name) {
            self.file_qualify(&ty.name)
        } else if let Some(module) = self.open_types.get(&ty.name).cloned() {
            self.dep(&module);
            self.qualify(&module, &ty.name)
        } else {
            ty.name
        };
        Ok(ast::TypeName {
            name,
            args,
            span: ty.span,
        })
    }

    /// Canonicalize a type declaration's body: field annotations, and (for
    /// variants) the constructor names — the canonical tags runtime variant
    /// values carry.
    fn canon_type_body(&mut self, body: ast::TypeBody) -> Result<ast::TypeBody, LowerError> {
        let canon_fields = |lowerer: &mut Self,
                            fields: Vec<ast::FieldTy>|
         -> Result<Vec<ast::FieldTy>, LowerError> {
            fields
                .into_iter()
                .map(|field| {
                    Ok(ast::FieldTy {
                        ty: lowerer.canon_type(field.ty)?,
                        name: field.name,
                        span: field.span,
                    })
                })
                .collect()
        };
        Ok(match body {
            // No fields to canonicalize — an abstract type is opaque.
            ast::TypeBody::Abstract => ast::TypeBody::Abstract,
            ast::TypeBody::Record(fields) => ast::TypeBody::Record(canon_fields(self, fields)?),
            ast::TypeBody::Variants(variants) => ast::TypeBody::Variants(
                variants
                    .into_iter()
                    .map(|variant| {
                        Ok(ast::VariantDecl {
                            name: self.self_qualify(&variant.name),
                            fields: canon_fields(self, variant.fields)?,
                            span: variant.span,
                        })
                    })
                    .collect::<Result<Vec<_>, LowerError>>()?,
            ),
        })
    }

    // Recursion depth is bounded by the parser's `MAX_DEPTH` guard — except
    // for left-assoc binary chains (`a + b + c + …`), which the parser builds
    // iteratively with no depth cost, so lowering walks their lhs spine
    // iteratively too (as does eval).
    fn expr(&mut self, expr: ast::Expr) -> Result<Expr, LowerError> {
        let span = expr.span;
        let kind = match expr.kind {
            ast::ExprKind::Binary { .. } => {
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ast::ExprKind::Binary { op, lhs, rhs } = leaf.kind {
                    spine.push((op, *rhs, leaf.span));
                    leaf = *lhs;
                }
                // Lowering order (leaf, then each rhs, assigning the joining
                // node's ID after its rhs) matches what recursive descent
                // produced, so expression IDs — and the .ir goldens — are
                // unchanged.
                let mut acc = self.expr(leaf)?;
                for (op, rhs, node_span) in spine.into_iter().rev() {
                    let rhs = self.expr(rhs)?;
                    acc = Expr {
                        id: self.expr_id(),
                        kind: ExprKind::Binary {
                            op,
                            lhs: Box::new(acc),
                            rhs: Box::new(rhs),
                        },
                        span: node_span,
                    };
                }
                return Ok(acc);
            }
            ast::ExprKind::Ident(segments) => return self.ident(segments, span),
            ast::ExprKind::Pipeline { head, stages } => {
                let mut piped = self.expr(*head)?;
                for stage in stages {
                    piped = self.pipe_stage(piped, stage)?;
                }
                return Ok(piped);
            }
            ast::ExprKind::Number(n) => ExprKind::Number(n),
            ast::ExprKind::String(s) => ExprKind::String(s),
            ast::ExprKind::InterpolatedString(parts) => {
                let mut lowered = Vec::with_capacity(parts.len());
                for part in parts {
                    lowered.push(match part {
                        ast::StringPart::Text(text) => StringPart::Text(text),
                        ast::StringPart::Expr(expr) => StringPart::Expr(self.expr(expr)?),
                    });
                }
                ExprKind::InterpolatedString(lowered)
            }
            ast::ExprKind::Bool(b) => ExprKind::Bool(b),
            ast::ExprKind::Record(fields) => {
                let mut lowered: Vec<Field> = Vec::new();
                for field in fields {
                    // Duplicate fields would make record equality (which
                    // matches fields by name) asymmetric — reject them here,
                    // like duplicate params and duplicate top-level names.
                    if lowered.iter().any(|f| f.name == field.name) {
                        return Err(LowerError {
                            message: format!("duplicate record field `{}`", field.name),
                            span: field.span,
                        });
                    }
                    lowered.push(Field {
                        name: field.name,
                        value: self.expr(field.value)?,
                        span: field.span,
                    });
                }
                ExprKind::Record(lowered)
            }
            ast::ExprKind::List(items) => {
                let mut lowered = Vec::new();
                for item in items {
                    lowered.push(self.expr(item)?);
                }
                ExprKind::List(lowered)
            }
            ast::ExprKind::ListCons { items, tail } => {
                let mut lowered = Vec::new();
                for item in items {
                    lowered.push(self.expr(item)?);
                }
                ExprKind::ListCons {
                    items: lowered,
                    tail: Box::new(self.expr(*tail)?),
                }
            }
            ast::ExprKind::Tuple(items) => {
                let mut lowered = Vec::new();
                for item in items {
                    lowered.push(self.expr(item)?);
                }
                ExprKind::Tuple(lowered)
            }
            ast::ExprKind::RecordUpdate { base, fields } => {
                let base = Box::new(self.expr(*base)?);
                let mut lowered: Vec<Field> = Vec::new();
                for field in fields {
                    // Duplicates would make "last one wins" silent — same
                    // rule as record literals.
                    if lowered.iter().any(|f| f.name == field.name) {
                        return Err(LowerError {
                            message: format!("duplicate record field `{}`", field.name),
                            span: field.span,
                        });
                    }
                    lowered.push(Field {
                        name: field.name,
                        value: self.expr(field.value)?,
                        span: field.span,
                    });
                }
                ExprKind::RecordUpdate {
                    base,
                    fields: lowered,
                }
            }
            ast::ExprKind::Let {
                mutable,
                name,
                ty,
                value,
                body,
            } => {
                // The value is evaluated outside the binding's scope
                // (`let x = x in …` refers to the outer `x`).
                let ty = ty.map(|ty| self.canon_type(ty)).transpose()?;
                let value = Box::new(self.expr(*value)?);
                let binding = BindingId(self.next_binding);
                self.next_binding += 1;
                self.scopes.push(ScopeLevel {
                    lambda_boundary: false,
                    vars: vec![(name.clone(), binding, mutable)],
                });
                let body = self.expr(*body);
                self.scopes.pop();
                ExprKind::Let {
                    binding,
                    name,
                    mutable,
                    ty,
                    value,
                    body: Box::new(body?),
                }
            }
            ast::ExprKind::Assign { name, value, rest } => {
                let resolved = match self.lookup(&name) {
                    Some(resolved) => resolved,
                    None => {
                        let target = if self.globals.contains(&name) {
                            format!("cannot assign to top-level `{name}` (globals are immutable)")
                        } else {
                            format!("unknown name `{name}`")
                        };
                        return Err(LowerError {
                            message: target,
                            span,
                        });
                    }
                };
                if !resolved.mutable {
                    return Err(LowerError {
                        message: format!("cannot assign to immutable binding `{name}`"),
                        span,
                    });
                }
                if resolved.captured {
                    return Err(LowerError {
                        message: format!("a function cannot capture the mutable binding `{name}`"),
                        span,
                    });
                }
                ExprKind::Assign {
                    binding: resolved.binding,
                    name,
                    value: Box::new(self.expr(*value)?),
                    rest: Box::new(self.expr(*rest)?),
                }
            }
            ast::ExprKind::FieldAccess { object, field } => ExprKind::FieldAccess {
                object: Box::new(self.expr(*object)?),
                field,
            },
            ast::ExprKind::Lambda { params, ret, body } => {
                let mut scope: Vec<(String, BindingId, bool)> = Vec::new();
                let mut lowered = Vec::new();
                for param in params {
                    if scope.iter().any(|(n, _, _)| *n == param.name) {
                        return Err(LowerError {
                            message: format!("duplicate parameter `{}`", param.name),
                            span: param.span,
                        });
                    }
                    let binding = BindingId(self.next_binding);
                    self.next_binding += 1;
                    scope.push((param.name.clone(), binding, false));
                    let ty = param.ty.map(|ty| self.canon_type(ty)).transpose()?;
                    lowered.push(Param {
                        binding,
                        name: param.name,
                        ty,
                        span: param.span,
                    });
                }
                let ret = ret.map(|ty| self.canon_type(ty)).transpose()?;
                self.scopes.push(ScopeLevel {
                    lambda_boundary: true,
                    vars: scope,
                });
                let body = self.expr(*body);
                self.scopes.pop();
                ExprKind::Lambda {
                    params: std::rc::Rc::new(lowered),
                    ret,
                    body: std::rc::Rc::new(body?),
                }
            }
            ast::ExprKind::Call { callee, args } => {
                let callee = Box::new(self.expr(*callee)?);
                let mut lowered = Vec::new();
                for arg in args {
                    lowered.push(self.expr(arg)?);
                }
                ExprKind::Call {
                    callee,
                    args: lowered,
                }
            }
            // Like the Binary spine above: left-assoc `&&`/`||` chains nest
            // down the lhs, so lower them iteratively — a flat chain must not
            // grow the host stack per term.
            ast::ExprKind::Logical { .. } => {
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ast::ExprKind::Logical { op, lhs, rhs } = leaf.kind {
                    spine.push((op, *rhs, leaf.span));
                    leaf = *lhs;
                }
                let mut acc = self.expr(leaf)?;
                for (op, rhs, node_span) in spine.into_iter().rev() {
                    let rhs = self.expr(rhs)?;
                    acc = Expr {
                        id: self.expr_id(),
                        kind: ExprKind::Logical {
                            op,
                            lhs: Box::new(acc),
                            rhs: Box::new(rhs),
                        },
                        span: node_span,
                    };
                }
                return Ok(acc);
            }
            ast::ExprKind::Neg(inner) => ExprKind::Neg(Box::new(self.expr(*inner)?)),
            ast::ExprKind::Not(inner) => ExprKind::Not(Box::new(self.expr(*inner)?)),
            // `else if` chains nest down `else_branch`, so lower the spine
            // iteratively — a long chain must not grow the host stack per
            // link (like the Logical spine above).
            ast::ExprKind::If { .. } => {
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ast::ExprKind::If {
                    cond,
                    then_branch,
                    else_branch,
                } = leaf.kind
                {
                    spine.push((*cond, *then_branch, leaf.span));
                    leaf = *else_branch;
                }
                let mut acc = self.expr(leaf)?; // the final (non-`if`) else branch
                for (cond, then_branch, node_span) in spine.into_iter().rev() {
                    let cond = self.expr(cond)?;
                    let then_branch = self.expr(then_branch)?;
                    acc = Expr {
                        id: self.expr_id(),
                        kind: ExprKind::If {
                            cond: Box::new(cond),
                            then_branch: Box::new(then_branch),
                            else_branch: Box::new(acc),
                        },
                        span: node_span,
                    };
                }
                return Ok(acc);
            }
            ast::ExprKind::Match { scrutinee, arms } => {
                // The scrutinee is evaluated outside any arm's scope.
                let scrutinee = Box::new(self.expr(*scrutinee)?);
                let mut lowered = Vec::new();
                for arm in arms {
                    let mut vars: Vec<(String, BindingId, bool)> = Vec::new();
                    let pattern = self.pattern(arm.pattern, &mut vars)?;
                    // One scope level per arm: pattern variables are visible
                    // in that arm's body only (they never leak to later
                    // arms) and are plain immutable bindings — a lambda may
                    // capture them.
                    self.scopes.push(ScopeLevel {
                        lambda_boundary: false,
                        vars,
                    });
                    let body = self.expr(arm.body);
                    self.scopes.pop();
                    lowered.push(MatchArm {
                        pattern,
                        body: body?,
                        span: arm.span,
                    });
                }
                ExprKind::Match {
                    scrutinee,
                    arms: lowered,
                }
            }
        };
        Ok(Expr {
            id: self.expr_id(),
            kind,
            span,
        })
    }

    /// Resolve a pattern's constructor name to its canonical tag + arity:
    /// this module's own constructors, `open`ed ones, and module-qualified
    /// `Utils.Circle` forms.
    fn resolve_pattern_ctor(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(String, usize), LowerError> {
        if name.contains('.') {
            let segments: Vec<&str> = name.split('.').collect();
            let Some((key, consumed)) = self.resolve_module_prefix(&segments) else {
                return Err(LowerError {
                    message: format!("unknown module `{}` in pattern `{name}`", segments[0]),
                    span,
                });
            };
            let member = segments[consumed..].join(".");
            let Some(&arity) = self.exports_of_key(&key).ctors.get(&member) else {
                return Err(LowerError {
                    message: format!("module `{}` has no constructor `{member}`", key.label()),
                    span,
                });
            };
            self.dep_key(&key);
            return Ok((self.qualify_key(&key, &member), arity));
        }
        if let Some(arity) = self
            .inline_exports()
            .and_then(|ns| ns.ctors.get(name))
            .copied()
        {
            return Ok((self.self_qualify(name), arity));
        }
        if let Some(&arity) = self.ctors.get(name) {
            return Ok((self.file_qualify(name), arity));
        }
        if let Some(OpenedName::Ctor(module, arity)) = self.open_values.get(name).cloned() {
            self.dep(&module);
            return Ok((self.qualify(&module, name), arity));
        }
        Err(LowerError {
            message: format!("unknown constructor `{name}`"),
            span,
        })
    }

    /// Lower one pattern, appending its variable bindings to `vars` (the
    /// caller pushes them as the arm body's scope). Pattern nesting is
    /// bounded by the grammar (constructor sub-patterns are leaves), so
    /// recursion depth is at most two.
    fn pattern(
        &mut self,
        pattern: ast::Pattern,
        vars: &mut Vec<(String, BindingId, bool)>,
    ) -> Result<Pattern, LowerError> {
        let span = pattern.span;
        let kind = match pattern.kind {
            ast::PatternKind::Wildcard => PatternKind::Wildcard,
            ast::PatternKind::Var(name) => {
                if vars.iter().any(|(n, _, _)| *n == name) {
                    return Err(LowerError {
                        message: format!("duplicate pattern variable `{name}`"),
                        span,
                    });
                }
                let binding = BindingId(self.next_binding);
                self.next_binding += 1;
                vars.push((name.clone(), binding, false));
                PatternKind::Var { binding, name }
            }
            ast::PatternKind::Ctor { name, args } => {
                let (canonical, arity) = self.resolve_pattern_ctor(&name, span)?;
                if args.len() != arity {
                    return Err(LowerError {
                        message: format!(
                            "`{name}` has {arity} field(s), but the pattern names {}",
                            args.len()
                        ),
                        span,
                    });
                }
                let mut lowered = Vec::new();
                for arg in args {
                    lowered.push(self.pattern(arg, vars)?);
                }
                PatternKind::Ctor {
                    name: canonical,
                    args: lowered,
                }
            }
            ast::PatternKind::Tuple(args) => {
                let mut lowered = Vec::new();
                for arg in args {
                    lowered.push(self.pattern(arg, vars)?);
                }
                PatternKind::Tuple(lowered)
            }
            ast::PatternKind::List { items, tail } => {
                let mut lowered = Vec::new();
                for item in items {
                    lowered.push(self.pattern(item, vars)?);
                }
                let tail = match tail {
                    Some(t) => Some(Box::new(self.pattern(*t, vars)?)),
                    None => None,
                };
                PatternKind::List {
                    items: lowered,
                    tail,
                }
            }
            ast::PatternKind::Number(n) => PatternKind::Number(n),
            ast::PatternKind::Bool(b) => PatternKind::Bool(b),
            ast::PatternKind::String(s) => PatternKind::String(s),
        };
        Ok(Pattern { kind, span })
    }

    /// Desugar one pipeline stage (see module docs): the already-lowered
    /// `piped` value becomes the first argument of the stage's call.
    fn pipe_stage(&mut self, piped: Expr, stage: ast::Expr) -> Result<Expr, LowerError> {
        let span = stage.span;
        let (callee, rest) = match stage.kind {
            // `x |> g(a)` → `g(a, x)`
            ast::ExprKind::Call { callee, args } => (*callee, args),
            // `x |> f` → `f(x)` (any non-call stage is called directly)
            kind => (ast::Expr { kind, span }, Vec::new()),
        };
        // Thread-LAST: the piped value becomes the callee's FINAL argument, so
        // `x |> g(a)` lowers directly to the saturated `g(a, x)` (never a
        // partial `g(a)`) and a subject-last signature — `x |>
        // Debug.log("hp")` == `Debug.log("hp", x)` — threads x in as the
        // subject. Pipes stay allocation-free (a saturated call, no PAP).
        let callee = Box::new(self.expr(callee)?);
        let mut args = Vec::with_capacity(rest.len() + 1);
        for arg in rest {
            args.push(self.expr(arg)?);
        }
        args.push(piped);
        Ok(Expr {
            id: self.expr_id(),
            kind: ExprKind::Call { callee, args },
            span,
        })
    }

    /// Resolve an identifier per the module-doc rules. Every node of a
    /// reinterpreted field-access chain keeps the whole identifier's span
    /// (the AST node it came from).
    fn ident(&mut self, segments: Vec<String>, span: Span) -> Result<Expr, LowerError> {
        let first = &segments[0];
        let base = if let Some(resolved) = self.lookup(first) {
            if resolved.mutable && resolved.captured {
                return Err(LowerError {
                    message: format!("a function cannot capture the mutable binding `{first}`"),
                    span,
                });
            }
            if resolved.mutable {
                Some(ExprKind::LocalMut {
                    binding: resolved.binding,
                    name: first.clone(),
                })
            } else {
                Some(ExprKind::Local {
                    binding: resolved.binding,
                    name: first.clone(),
                })
            }
        } else if self
            .inline_exports()
            .is_some_and(|ns| ns.defs.contains(first))
        {
            Some(ExprKind::Global(self.self_qualify(first)))
        } else if let Some(arity) = self
            .inline_exports()
            .and_then(|ns| ns.ctors.get(first))
            .copied()
        {
            Some(ExprKind::Ctor {
                name: self.self_qualify(first),
                arity,
            })
        } else if self.globals.contains(first) {
            Some(ExprKind::Global(self.file_qualify(first)))
        } else if let Some(&arity) = self.ctors.get(first) {
            Some(ExprKind::Ctor {
                name: self.file_qualify(first),
                arity,
            })
        } else if let Some(opened) = self.open_values.get(first).cloned() {
            match opened {
                OpenedName::Def(module) => {
                    self.dep(&module);
                    Some(ExprKind::Global(self.qualify(&module, first)))
                }
                OpenedName::Ctor(module, arity) => {
                    self.dep(&module);
                    Some(ExprKind::Ctor {
                        name: self.qualify(&module, first),
                        arity,
                    })
                }
                // An opened interface signature stays External (host-resolved).
                OpenedName::Signature(module) => {
                    Some(ExprKind::External(vec![module, first.clone()]))
                }
            }
        } else {
            None
        };
        // The longest module path the leading segments name, if any (used
        // only when nothing bound `first`).
        let module_ref = if base.is_none() {
            let borrowed: Vec<&str> = segments.iter().map(String::as_str).collect();
            self.resolve_module_prefix(&borrowed)
        } else {
            None
        };
        let (kind, consumed) = match base {
            Some(kind) => (kind, 1),
            // A module-qualified reference: `Utils.clamp` / `Utils.Circle` /
            // `Game.Server.step` (a sibling file's inline module).
            // (Module names never collide with builtin namespaces like
            // `List` — the project refuses them at load — so trying modules
            // before the External seam is unambiguous.)
            None if module_ref.is_some() => {
                let (key, consumed) = module_ref.expect("checked above");
                let member = segments[consumed].clone();
                let exports = self.exports_of_key(&key);
                let kind = if exports.defs.contains(&member) {
                    ExprKind::Global(self.qualify_key(&key, &member))
                } else if let Some(&arity) = exports.ctors.get(&member) {
                    ExprKind::Ctor {
                        name: self.qualify_key(&key, &member),
                        arity,
                    }
                } else if exports.signatures.contains(&member) {
                    // An interface (`.funi`) signature: keep it EXTERNAL so the
                    // host resolves the value at runtime (like an unknown
                    // qualified name) — the checker types it from the signature.
                    ExprKind::External(vec![key.label().to_string(), member.clone()])
                } else {
                    return Err(LowerError {
                        message: format!("module `{}` has no `{member}`", key.label()),
                        span,
                    });
                };
                self.dep_key(&key);
                (kind, consumed + 1)
            }
            None if segments.len() > 1 => {
                // `Shape.Circle(2.0)` — TYPE-qualifying a constructor. It is
                // not a module reference, so it would otherwise fall through
                // to the host seam and die at run time as an unknown
                // external. Constructors resolve bare.
                if self.ctor_types.get(&segments[1]) == Some(first) {
                    return Err(LowerError {
                        message: format!(
                            "`{}` is a constructor of the type `{first}`, not a member of a \
module — constructors resolve bare: write `{}`",
                            segments[1], segments[1]
                        ),
                        span,
                    });
                }
                return Ok(Expr {
                    id: self.expr_id(),
                    kind: ExprKind::External(segments),
                    span,
                });
            }
            None => {
                let hint = if self
                    .project
                    .is_some_and(|env| env.modules.contains_key(first))
                {
                    format!(" — `{first}` is a module; reference a member (`{first}.name`)")
                } else {
                    String::new()
                };
                return Err(LowerError {
                    message: format!("unknown name `{first}`{hint}"),
                    span,
                });
            }
        };
        let mut expr = Expr {
            id: self.expr_id(),
            kind,
            span,
        };
        // `Foo.bar` where `Foo` is a binding: the qualifier syntax was really
        // field access on that value (see `ast::ExprKind::Ident`).
        for field in segments.into_iter().skip(consumed) {
            expr = Expr {
                id: self.expr_id(),
                kind: ExprKind::FieldAccess {
                    object: Box::new(expr),
                    field,
                },
                span,
            };
        }
        Ok(expr)
    }

    fn lookup(&self, name: &str) -> Option<Resolved> {
        let mut captured = false;
        for level in self.scopes.iter().rev() {
            if let Some((_, binding, mutable)) = level.vars.iter().rev().find(|(n, _, _)| n == name)
            {
                return Some(Resolved {
                    binding: *binding,
                    mutable: *mutable,
                    captured,
                });
            }
            // Not in this level: passing a lambda's params level means any
            // match further out is a capture.
            captured |= level.lambda_boundary;
        }
        None
    }
}
