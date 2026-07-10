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
//!   (the `Type.Ctor` qualified form is deliberately NOT supported — a
//!   qualified name whose head is a type name stays an unknown external).
//! - `first` names an `open`ed module's def or constructor → the qualified
//!   [`ExprKind::Global`] / [`ExprKind::Ctor`].
//! - `first` names a sibling module → resolve `rest[0]` against its exports
//!   (def or constructor; an unknown member is an error); further segments
//!   become field access. (Builtins like `List.map` cannot collide: module
//!   names matching builtin/prelude namespaces are refused at project load.)
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

pub(crate) fn exports_of(program: &ast::Program) -> Exports {
    let mut exports = Exports::default();
    for item in &program.items {
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

fn lower_module(
    program: ast::Program,
    project: Option<&ProjectEnv>,
    bases: IdBases,
) -> Result<(Module, IdBases, HashSet<String>), LowerError> {
    // Pass 1: collect top-level names so defs are mutually visible (see
    // module docs) and duplicates fail loud. Types and values are separate
    // namespaces; constructors join the VALUE namespace (they resolve bare),
    // so they must not collide with `let`s or with each other — across
    // variant types too.
    let mut globals = HashSet::new();
    let mut ctors: HashMap<String, usize> = HashMap::new();
    let mut type_names = HashSet::new();
    for item in &program.items {
        match item {
            ast::Item::Open(_) => {}
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
                if matches!(decl.name.as_str(), "float" | "bool" | "string" | "List") {
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
                    }
                }
            }
        }
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
        let Some(exports) = env.modules.get(&decl.module) else {
            return Err(LowerError {
                message: format!(
                    "unknown module `{}` — modules are the sibling `.fun` files next to the entry",
                    decl.module
                ),
                span: decl.span,
            });
        };
        deps.insert(decl.module.clone());
        // Value namespace: the opened module's defs, constructors, and
        // interface signatures.
        let mut values: Vec<(&String, OpenedName)> = exports
            .defs
            .iter()
            .map(|d| (d, OpenedName::Def(decl.module.clone())))
            .chain(
                exports
                    .ctors
                    .iter()
                    .map(|(c, a)| (c, OpenedName::Ctor(decl.module.clone(), *a))),
            )
            .chain(
                exports
                    .signatures
                    .iter()
                    .map(|s| (s, OpenedName::Signature(decl.module.clone()))),
            )
            .collect();
        values.sort_by_key(|(name, _)| name.as_str().to_string());
        for (name, opened) in values {
            if globals.contains(name) || ctors.contains_key(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: `{name}` collides with this module's own `{name}` — qualify \
uses as `{}.{name}` instead of opening",
                        decl.module, decl.module
                    ),
                    span: decl.span,
                });
            }
            if let Some(prev) = open_values.get(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: `{name}` is already in scope from `open {}` — qualify uses \
(`{}.{name}` / `{}.{name}`)",
                        decl.module,
                        prev.module(),
                        prev.module(),
                        decl.module
                    ),
                    span: decl.span,
                });
            }
            open_values.insert(name.clone(), opened);
        }
        // Type namespace.
        let mut types: Vec<&String> = exports.types.iter().collect();
        types.sort();
        for name in types {
            if type_names.contains(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: type `{name}` collides with this module's own `{name}` — \
qualify uses as `{}.{name}` instead of opening",
                        decl.module, decl.module
                    ),
                    span: decl.span,
                });
            }
            if let Some(prev) = open_types.get(name) {
                return Err(LowerError {
                    message: format!(
                        "open {}: type `{name}` is already in scope from `open {prev}` — \
qualify uses (`{prev}.{name}` / `{}.{name}`)",
                        decl.module, decl.module
                    ),
                    span: decl.span,
                });
            }
            open_types.insert(name.clone(), decl.module.clone());
        }
    }

    // Pass 2: lower items in file order.
    let mut lowerer = Lowerer {
        globals,
        ctors,
        types: type_names,
        project,
        open_values,
        open_types,
        deps,
        scopes: Vec::new(),
        next_binding: bases.binding,
        next_expr: bases.expr,
    };
    let mut next_def = bases.def;
    let mut types = Vec::new();
    let mut defs = Vec::new();
    let mut signatures = Vec::new();
    for item in program.items {
        match item {
            // Opens were consumed in pass 1b; they leave no IR (and no
            // DefId — a file without opens numbers exactly as before).
            ast::Item::Open(_) => {}
            ast::Item::Type(decl) => {
                let id = DefId(next_def);
                next_def += 1;
                types.push(TypeDef {
                    params: decl.params.clone(),
                    id,
                    name: lowerer.self_qualify(&decl.name),
                    body: lowerer.canon_type_body(decl.body)?,
                    span: decl.span,
                });
            }
            ast::Item::Let(decl) => {
                let id = DefId(next_def);
                next_def += 1;
                defs.push(Def {
                    id,
                    name: lowerer.self_qualify(&decl.name),
                    ty: decl.ty,
                    value: lowerer.expr(decl.value)?,
                    span: decl.span,
                });
            }
            // An interface signature: no body, no DefId. Canonicalize its name
            // and type (which may reference this module's — or another's —
            // types) so the checker can type externals against it.
            ast::Item::Sig(decl) => {
                signatures.push(Signature {
                    name: lowerer.self_qualify(&decl.name),
                    ty: lowerer.canon_type(decl.ty)?,
                    span: decl.span,
                });
            }
        }
    }
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
        },
        bases,
        lowerer.deps,
    ))
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

    /// Canonicalize `member` of `module`: `M.member`, except the ENTRY
    /// module's members stay bare (see the module docs).
    fn qualify(&self, module: &str, member: &str) -> String {
        match self.project {
            Some(env) if module != env.entry => format!("{module}.{member}"),
            _ => member.to_string(),
        }
    }

    /// Canonicalize one of this module's own members.
    fn self_qualify(&self, member: &str) -> String {
        match self.project {
            Some(env) => self.qualify(env.name, member),
            None => member.to_string(),
        }
    }

    /// Record a cross-module reference (the project dependency edge).
    fn dep(&mut self, module: &str) {
        if self.project.is_some_and(|env| env.name != module) {
            self.deps.insert(module.to_string());
        }
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
        let name = if let Some((module, member)) = ty.name.split_once('.') {
            match self.project.and_then(|env| env.modules.get(module)) {
                Some(exports) => {
                    if !exports.types.contains(member) {
                        return Err(LowerError {
                            message: format!("module `{module}` has no type `{member}`"),
                            span: ty.span,
                        });
                    }
                    let module = module.to_string();
                    self.dep(&module);
                    self.qualify(&module, member)
                }
                None => ty.name,
            }
        } else if self.types.contains(&ty.name) {
            self.self_qualify(&ty.name)
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
            ast::ExprKind::Neg(inner) => ExprKind::Neg(Box::new(self.expr(*inner)?)),
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
        if let Some((module, member)) = name.split_once('.') {
            let Some(exports) = self.project.and_then(|env| env.modules.get(module)) else {
                return Err(LowerError {
                    message: format!("unknown module `{module}` in pattern `{name}`"),
                    span,
                });
            };
            let Some(&arity) = exports.ctors.get(member) else {
                return Err(LowerError {
                    message: format!("module `{module}` has no constructor `{member}`"),
                    span,
                });
            };
            let module = module.to_string();
            self.dep(&module);
            return Ok((self.qualify(&module, member), arity));
        }
        if let Some(&arity) = self.ctors.get(name) {
            return Ok((self.self_qualify(name), arity));
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
        } else if self.globals.contains(first) {
            Some(ExprKind::Global(self.self_qualify(first)))
        } else if let Some(&arity) = self.ctors.get(first) {
            Some(ExprKind::Ctor {
                name: self.self_qualify(first),
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
        let (kind, consumed) = match base {
            Some(kind) => (kind, 1),
            // A module-qualified reference: `Utils.clamp` / `Utils.Circle`.
            // (Module names never collide with builtin namespaces like
            // `List` — the project refuses them at load — so trying modules
            // before the External seam is unambiguous.)
            None if segments.len() > 1
                && self
                    .project
                    .is_some_and(|env| env.modules.contains_key(first)) =>
            {
                let module = first.clone();
                let member = &segments[1];
                let exports = self
                    .project
                    .and_then(|env| env.modules.get(&module))
                    .expect("checked above");
                let kind = if exports.defs.contains(member) {
                    ExprKind::Global(self.qualify(&module, member))
                } else if let Some(&arity) = exports.ctors.get(member) {
                    ExprKind::Ctor {
                        name: self.qualify(&module, member),
                        arity,
                    }
                } else if exports.signatures.contains(member) {
                    // An interface (`.funi`) signature: keep it EXTERNAL so the
                    // host resolves the value at runtime (like an unknown
                    // qualified name) — the checker types it from the signature.
                    ExprKind::External(vec![module.clone(), member.clone()])
                } else {
                    return Err(LowerError {
                        message: format!("module `{module}` has no `{member}`"),
                        span,
                    });
                };
                self.dep(&module);
                (kind, 2)
            }
            None if segments.len() > 1 => {
                return Ok(Expr {
                    id: self.expr_id(),
                    kind: ExprKind::External(segments),
                    span,
                })
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
