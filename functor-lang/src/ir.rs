//! Core IR for the B2 milestone — the lowered, name-resolved form of a
//! parsed program, produced by [`crate::lower`]. Differences from the
//! surface AST:
//!
//! - **Stable IDs.** Every top-level item, lambda parameter, and expression
//!   node carries a deterministic sequential ID assigned during lowering
//!   (file order / traversal order), so the same source always produces
//!   byte-identical IR. Top-level items *also* carry their name — the stable
//!   name-based identity that future hot-reload rebinds on (docs/functor-lang.md B5).
//! - **Names are resolved** (see [`crate::lower`] for the rules): every
//!   identifier is a [`ExprKind::Local`], [`ExprKind::Global`], or
//!   [`ExprKind::External`] reference.
//! - **No pipelines.** `x |> f |> g(a)` desugars to `g(f(x), a)` during
//!   lowering (see [`crate::lower`]); the IR has no pipeline node.
//! - **Spans everywhere.** Every node keeps the source span of the AST node
//!   it came from.
//! - **Type annotations stay symbolic** ([`TypeName`] carried through
//!   verbatim) — no inference or checking until B4.

use crate::ast::{BinOp, LogicalOp, TypeBody, TypeName};
use crate::span::Span;
use std::fmt;
use std::rc::Rc;

/// ID of a top-level item ([`TypeDef`] or [`Def`]), assigned in file order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DefId(pub(crate) u32);

/// ID of a value binding (a lambda parameter), assigned in traversal order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BindingId(pub(crate) u32);

/// ID of an expression node, assigned in traversal order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExprId(pub(crate) u32);

impl ExprId {
    /// The raw id, for keying external per-node tables
    /// ([`crate::types::ExprTypes`]).
    pub fn raw(self) -> u32 {
        self.0
    }
}

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
    /// Bodyless value signatures from interface (`.funi`) modules: the
    /// declared type of a HOST-implemented value (`Scene.cube : () =>
    /// SceneNode`). They give the checker types for otherwise-`Unknown`
    /// externals; they have no body, so evaluation never sees them (the host
    /// provides the value at runtime — there is no paired `.fun`).
    pub signatures: Vec<Signature>,
    /// `expect <expr>` inline tests, in file order. Deliberately OUTSIDE
    /// `defs`: loading a session ([`crate::eval::Session::load`]) never
    /// evaluates them — only test tooling does
    /// ([`crate::eval::run_expects`]).
    pub expects: Vec<ExpectDef>,
}

/// One lowered `expect <expr>` test. `module` is the owning module's
/// canonical prefix (`"Utils"`; empty for the entry) — the checker scopes
/// bare record literals by it, like a def's name prefix. Unnamed: the span
/// is the test's identity.
#[derive(Debug)]
pub struct ExpectDef {
    pub module: String,
    pub expr: Expr,
    pub span: Span,
}

/// One `.funi` value signature: a canonical name (`Scene.cube`) and its
/// declared type (carried symbolically, like a def annotation).
#[derive(Debug)]
pub struct Signature {
    pub name: String,
    pub ty: TypeName,
    pub span: Span,
}

/// `type Position = { x: Float, y: Float }` or
/// `type Shape = | Circle(radius: Float) | Point` — the body ([`TypeBody`])
/// is carried through from the AST verbatim, fields keeping their symbolic
/// [`TypeName`]s; other types reference this one by name. Constructor names
/// live in the *value* namespace (see [`crate::lower`]).
#[derive(Debug)]
pub struct TypeDef {
    pub id: DefId,
    pub name: String,
    /// Declared type parameters (`type Box<a>` → `["a"]`).
    pub params: Vec<String>,
    pub body: TypeBody,
    pub span: Span,
}

/// A lowered top-level `let`. `name` is the def's stable identity. `ty` is the
/// optional binding annotation (`let name: Type = …`), carried symbolically
/// for the checker.
#[derive(Debug)]
pub struct Def {
    pub id: DefId,
    pub name: String,
    pub ty: Option<TypeName>,
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
    /// `{ base with x: 1.0 }` — every field must exist on the base at
    /// runtime.
    RecordUpdate {
        base: Box<Expr>,
        fields: Vec<Field>,
    },
    /// `[1.0, 2.0, 3.0]`
    List(Vec<Expr>),
    /// `[a, b, ..tail]` — a list built by prepending `items` onto the list
    /// `tail` (the cons/spread form; `..tail` must be last). Plain
    /// `[a, b]` stays [`Self::List`].
    ListCons {
        items: Vec<Expr>,
        tail: Box<Expr>,
    },
    /// `(1.0, "a")` — at least two elements.
    Tuple(Vec<Expr>),
    /// Reference to an enclosing `let mut` slot (never crosses a lambda
    /// boundary — lowering rejects capture; see `crate::lower`).
    LocalMut {
        binding: BindingId,
        name: String,
    },
    /// `let [mut] name [: ty] = value in body`.
    Let {
        binding: BindingId,
        name: String,
        mutable: bool,
        ty: Option<TypeName>,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    /// `name := value; rest` — rebinds a `let mut` slot, then continues.
    Assign {
        binding: BindingId,
        name: String,
        value: Box<Expr>,
        rest: Box<Expr>,
    },
    /// `position.x`
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    /// Params and body are `Rc` (not `Box`) so a closure *value* can share
    /// them without lifetimes tied to the [`Module`] — see [`crate::eval`].
    /// (`Rc`'s `Debug` delegates, so the pretty-IR goldens are unaffected.)
    Lambda {
        params: Rc<Vec<Param>>,
        ret: Option<TypeName>,
        body: Rc<Expr>,
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
    /// `a && b` / `a || b` — short-circuiting; the right operand is evaluated
    /// only when the left doesn't decide the result (see [`crate::eval`]).
    Logical {
        op: LogicalOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Neg(Box<Expr>),
    /// `not e` — boolean negation.
    Not(Box<Expr>),
    /// `if cond then a else b` — a conditional; only the taken branch is
    /// evaluated. `else if` chains nest down `else_branch`, walked iteratively
    /// in eval/types (see [`crate::eval`]).
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// Reference to a declared variant constructor (an uppercase identifier
    /// that resolved against the module's constructors — see
    /// [`crate::lower`]). `arity` is the declared field count, carried here
    /// so evaluation needs no type-table lookup: a nullary constructor
    /// evaluates directly to its variant value, a parameterful one to a
    /// callable constructor value.
    Ctor {
        name: String,
        arity: usize,
    },
    /// `match scrutinee with | pattern => body | …` — first matching arm
    /// wins; no arm matching is a runtime error.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

/// One lowered `| pattern => body` arm. Pattern variables are bindings
/// scoped to `body` (they may shadow; duplicates within one pattern are
/// lowering errors).
#[derive(Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

/// The lowered pattern language (see [`crate::ast::PatternKind`]);
/// variables carry their [`BindingId`]s.
#[derive(Debug)]
pub enum PatternKind {
    Wildcard,
    Var {
        binding: BindingId,
        name: String,
    },
    /// Lowering guarantees `name` is a declared constructor and `args`
    /// matches its declared field count.
    Ctor {
        name: String,
        args: Vec<Pattern>,
    },
    /// `(x, _)` — sub-patterns are bindings or `_`; arity must match.
    Tuple(Vec<Pattern>),
    /// `[]` (empty), `[a, b]` (exact length), `[head, ..rest]` (at least
    /// `items.len()`, `rest` binds the remainder as a list). Element and
    /// tail sub-patterns are variable bindings or `_` only (like tuple/ctor
    /// patterns). `tail: None` means an exact-length match.
    List {
        items: Vec<Pattern>,
        tail: Option<Box<Pattern>>,
    },
    Number(f64),
    Bool(bool),
    String(String),
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
