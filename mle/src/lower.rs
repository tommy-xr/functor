//! AST → core-IR lowering — Track B2 of `docs/mle.md`. Assigns stable IDs,
//! resolves names, and desugars pipelines. No typechecking, no evaluation.
//!
//! ## Top-level visibility
//!
//! Top-level `let`s are **mutually visible**: a def may reference another def
//! declared later in the file (top-level bindings behave letrec-style, as in
//! other functional languages), so definition order never forces a
//! topological rewrite of game code. Because a def's *name* is its stable
//! identity (the hot-reload rebind key, docs/mle.md B5), duplicate top-level
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
//! - Otherwise, a qualified name (`Text.toBullets`) → [`ExprKind::External`],
//!   kept symbolic until the builtin registry arrives in B3 — and an
//!   unqualified name is an "unknown name" error at the identifier's span.
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
//! Each pipeline stage becomes a call with the piped value **prepended** as
//! the first argument: `x |> f` → `f(x)`, `x |> g(a)` → `g(x, a)`, so
//! `x |> f |> g(a)` → `g(f(x), a)`. The desugared call carries the span of
//! its stage — which means its first argument's span lies *outside* the
//! call's own span (it came from an earlier stage); diagnostics must not
//! assume parent spans contain child spans. The IR has no pipeline node.

use crate::ast;
use crate::ir::*;
use crate::span::Span;
use crate::LowerError;
use std::collections::{HashMap, HashSet};

/// Lower a parsed [`ast::Program`] to an IR [`Module`].
pub fn lower(program: ast::Program) -> Result<Module, LowerError> {
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
            ast::Item::Let(decl) => {
                if ctors.contains_key(&decl.name) {
                    return Err(LowerError {
                        message: format!(
                            "duplicate definition `{}` (constructors live in the value namespace)",
                            decl.name
                        ),
                        span: decl.span,
                    });
                }
                if !globals.insert(decl.name.clone()) {
                    return Err(LowerError {
                        message: format!("duplicate definition `{}`", decl.name),
                        span: decl.span,
                    });
                }
            }
            ast::Item::Type(decl) => {
                // Builtin type names would shadow the primitives in
                // annotations (the checker resolves `Float` before user
                // types), yielding nonsense like "expected Float, got Float".
                if matches!(decl.name.as_str(), "Float" | "Bool" | "String" | "List") {
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

    // Pass 2: lower items in file order.
    let mut lowerer = Lowerer {
        globals,
        ctors,
        scopes: Vec::new(),
        next_binding: 0,
        next_expr: 0,
    };
    let mut types = Vec::new();
    let mut defs = Vec::new();
    for (index, item) in program.items.into_iter().enumerate() {
        let id = DefId(index as u32);
        match item {
            ast::Item::Type(decl) => types.push(TypeDef {
                id,
                name: decl.name,
                body: decl.body,
                span: decl.span,
            }),
            ast::Item::Let(decl) => defs.push(Def {
                id,
                name: decl.name,
                value: lowerer.expr(decl.value)?,
                span: decl.span,
            }),
        }
    }
    Ok(Module { types, defs })
}

struct Lowerer {
    globals: HashSet<String>,
    /// Declared variant constructors: name → declared field count. Part of
    /// the value namespace (pass 1 guarantees no overlap with `globals`).
    ctors: HashMap<String, usize>,
    /// One level per enclosing lambda or `let … in`; lookup walks
    /// innermost-first. A lambda's level is a *boundary*: a `mut` binding
    /// found past one has been captured, which is an error (see the module
    /// docs and `~/notes/ideas/mle-language/mutability.md`).
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

impl Lowerer {
    fn expr_id(&mut self) -> ExprId {
        let id = ExprId(self.next_expr);
        self.next_expr += 1;
        id
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
                    lowered.push(Param {
                        binding,
                        name: param.name,
                        ty: param.ty,
                        span: param.span,
                    });
                }
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
                let Some(&arity) = self.ctors.get(&name) else {
                    return Err(LowerError {
                        message: format!("unknown constructor `{name}`"),
                        span,
                    });
                };
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
                    name,
                    args: lowered,
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
            // `x |> g(a)` → `g(x, a)`
            ast::ExprKind::Call { callee, args } => (*callee, args),
            // `x |> f` → `f(x)` (any non-call stage is called directly)
            kind => (ast::Expr { kind, span }, Vec::new()),
        };
        let callee = Box::new(self.expr(callee)?);
        let mut args = vec![piped];
        for arg in rest {
            args.push(self.expr(arg)?);
        }
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
            Some(ExprKind::Global(first.clone()))
        } else if let Some(&arity) = self.ctors.get(first) {
            Some(ExprKind::Ctor {
                name: first.clone(),
                arity,
            })
        } else {
            None
        };
        let kind = match base {
            Some(kind) => kind,
            None if segments.len() > 1 => {
                return Ok(Expr {
                    id: self.expr_id(),
                    kind: ExprKind::External(segments),
                    span,
                })
            }
            None => {
                return Err(LowerError {
                    message: format!("unknown name `{first}`"),
                    span,
                })
            }
        };
        let mut expr = Expr {
            id: self.expr_id(),
            kind,
            span,
        };
        // `Foo.bar` where `Foo` is a binding: the qualifier syntax was really
        // field access on that value (see `ast::ExprKind::Ident`).
        for field in segments.into_iter().skip(1) {
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
