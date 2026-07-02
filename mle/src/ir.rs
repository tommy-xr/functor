//! Core IR for the B2 milestone — the lowered, name-resolved form of a
//! parsed program, produced by [`crate::lower`]. Differences from the
//! surface AST:
//!
//! - **Stable IDs.** Every top-level item, lambda parameter, and expression
//!   node carries a deterministic sequential ID assigned during lowering
//!   (file order / traversal order), so the same source always produces
//!   byte-identical IR. Top-level items *also* carry their name — the stable
//!   name-based identity that future hot-reload rebinds on (docs/mle.md B5).
//! - **Names are resolved** (see [`crate::lower`] for the rules): every
//!   identifier is a [`ExprKind::Local`], [`ExprKind::Global`], or
//!   [`ExprKind::External`] reference.
//! - **No pipelines.** `x |> f |> g(a)` desugars to `g(f(x), a)` during
//!   lowering (see [`crate::lower`]); the IR has no pipeline node.
//! - **Spans everywhere.** Every node keeps the source span of the AST node
//!   it came from.
//! - **Type annotations stay symbolic** ([`TypeName`] carried through
//!   verbatim) — no inference or checking until B4.

use crate::ast::{BinOp, FieldTy, TypeName};
use crate::span::Span;
use std::fmt;

/// ID of a top-level item ([`TypeDef`] or [`Def`]), assigned in file order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DefId(pub(crate) u32);

/// ID of a value binding (a lambda parameter), assigned in traversal order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BindingId(pub(crate) u32);

/// ID of an expression node, assigned in traversal order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExprId(pub(crate) u32);

// Compact `d0` / `b0` / `e0` forms keep the pretty-Debug IR (and the
// committed `.ir` goldens) readable, like `Span`'s `start..end`.
impl fmt::Debug for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

impl fmt::Debug for BindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

impl fmt::Debug for ExprId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// A lowered source file. Item order within each list is file order;
/// [`DefId`]s number types and defs together, also in file order.
#[derive(Debug)]
pub struct Module {
    pub types: Vec<TypeDef>,
    pub defs: Vec<Def>,
}

/// `type Position = { x: Float, y: Float }` — fields keep their symbolic
/// [`TypeName`]s; other types reference this one by name.
#[derive(Debug)]
pub struct TypeDef {
    pub id: DefId,
    pub name: String,
    pub fields: Vec<FieldTy>,
    pub span: Span,
}

/// A lowered top-level `let`. `name` is the def's stable identity.
#[derive(Debug)]
pub struct Def {
    pub id: DefId,
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind {
    Number(f64),
    String(String),
    Bool(bool),
    /// Reference to an enclosing lambda's parameter. `name` duplicates the
    /// binding's name for readability; `binding` is authoritative.
    Local {
        binding: BindingId,
        name: String,
    },
    /// Reference to a top-level `let` in the same file.
    Global(String),
    /// A qualified name (`Text.toBullets`) that resolved to nothing in this
    /// file — kept symbolic until the builtin registry arrives in B3.
    External(Vec<String>),
    /// `{ x: 1.0, y: 2.0 }`
    Record(Vec<Field>),
    /// `position.x`
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    Lambda {
        params: Vec<Param>,
        ret: Option<TypeName>,
        body: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Neg(Box<Expr>),
}

/// One `name: value` entry of a record expression.
#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// A lambda parameter: its binding ID plus the annotation B1 parsed.
#[derive(Debug)]
pub struct Param {
    pub binding: BindingId,
    pub name: String,
    pub ty: Option<TypeName>,
    pub span: Span,
}
