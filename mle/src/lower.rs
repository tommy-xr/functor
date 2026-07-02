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
//! names are lowering errors rather than shadowing.
//!
//! ## Name resolution
//!
//! For an identifier `first(.rest)*` (the parser only builds multi-segment
//! names while the qualifier segment starts uppercase — see
//! [`crate::ast::ExprKind::Ident`]):
//!
//! - `first` names an enclosing lambda parameter → [`ExprKind::Local`]
//!   (innermost scope wins, so a parameter shadows a same-named global);
//!   any remaining segments become [`ExprKind::FieldAccess`] on it.
//! - `first` names a top-level `let` → [`ExprKind::Global`]; remaining
//!   segments likewise become field access.
//! - Otherwise, a qualified name (`Text.toBullets`) → [`ExprKind::External`],
//!   kept symbolic until the builtin registry arrives in B3 — and an
//!   unqualified name is an "unknown name" error at the identifier's span.
//!
//! ## Pipeline desugaring
//!
//! Each pipeline stage becomes a call with the piped value **prepended** as
//! the first argument: `x |> f` → `f(x)`, `x |> g(a)` → `g(x, a)`, so
//! `x |> f |> g(a)` → `g(f(x), a)`. The desugared call carries the span of
//! its stage. The IR has no pipeline node.

use crate::ast;
use crate::ir::*;
use crate::span::Span;
use crate::LowerError;
use std::collections::HashSet;

/// Lower a parsed [`ast::Program`] to an IR [`Module`].
pub fn lower(program: ast::Program) -> Result<Module, LowerError> {
    // Pass 1: collect top-level names so defs are mutually visible (see
    // module docs) and duplicates fail loud. Types and values are separate
    // namespaces.
    let mut globals = HashSet::new();
    let mut type_names = HashSet::new();
    for item in &program.items {
        let (names, name, span) = match item {
            ast::Item::Let(decl) => (&mut globals, &decl.name, decl.span),
            ast::Item::Type(decl) => (&mut type_names, &decl.name, decl.span),
        };
        if !names.insert(name.clone()) {
            return Err(LowerError {
                message: format!("duplicate definition `{name}`"),
                span,
            });
        }
    }

    // Pass 2: lower items in file order.
    let mut lowerer = Lowerer {
        globals,
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
                fields: decl.fields,
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
    /// One scope per enclosing lambda; lookup walks innermost-first.
    scopes: Vec<Vec<(String, BindingId)>>,
    next_binding: u32,
    next_expr: u32,
}

impl Lowerer {
    fn expr_id(&mut self) -> ExprId {
        let id = ExprId(self.next_expr);
        self.next_expr += 1;
        id
    }

    // Recursion depth is bounded by the parser's `MAX_DEPTH` guard, so
    // lowering needs no depth check of its own.
    fn expr(&mut self, expr: ast::Expr) -> Result<Expr, LowerError> {
        let span = expr.span;
        let kind = match expr.kind {
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
                let mut lowered = Vec::new();
                for field in fields {
                    lowered.push(Field {
                        name: field.name,
                        value: self.expr(field.value)?,
                        span: field.span,
                    });
                }
                ExprKind::Record(lowered)
            }
            ast::ExprKind::FieldAccess { object, field } => ExprKind::FieldAccess {
                object: Box::new(self.expr(*object)?),
                field,
            },
            ast::ExprKind::Lambda { params, ret, body } => {
                let mut scope = Vec::new();
                let mut lowered = Vec::new();
                for param in params {
                    let binding = BindingId(self.next_binding);
                    self.next_binding += 1;
                    scope.push((param.name.clone(), binding));
                    lowered.push(Param {
                        binding,
                        name: param.name,
                        ty: param.ty,
                        span: param.span,
                    });
                }
                self.scopes.push(scope);
                let body = self.expr(*body);
                self.scopes.pop();
                ExprKind::Lambda {
                    params: lowered,
                    ret,
                    body: Box::new(body?),
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
            ast::ExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
                op,
                lhs: Box::new(self.expr(*lhs)?),
                rhs: Box::new(self.expr(*rhs)?),
            },
            ast::ExprKind::Neg(inner) => ExprKind::Neg(Box::new(self.expr(*inner)?)),
        };
        Ok(Expr {
            id: self.expr_id(),
            kind,
            span,
        })
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
        let base = if let Some(binding) = self.lookup(first) {
            Some(ExprKind::Local {
                binding,
                name: first.clone(),
            })
        } else if self.globals.contains(first) {
            Some(ExprKind::Global(first.clone()))
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

    fn lookup(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.iter().rev().find(|(n, _)| n == name))
            .map(|(_, binding)| *binding)
    }
}
