//! Surface AST for the B1 syntax subset. Every node carries a [`Span`].
//!
//! Deliberately close to the source: pipelines stay explicit
//! ([`ExprKind::Pipeline`]) rather than desugaring to nested calls — that is
//! lowering's job ([`crate::lower`], the core IR), and keeping the surface
//! shape makes error spans and future formatting honest.

use crate::span::Span;

#[derive(Debug)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub enum Item {
    Let(LetDecl),
    Type(TypeDecl),
}

/// `let name = expr` — the expr may be a lambda (`let f = (a, b) => …`).
#[derive(Debug)]
pub struct LetDecl {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// `type Position = { x: Float, y: Float }` — record types only in B1.
#[derive(Debug)]
pub struct TypeDecl {
    pub name: String,
    pub fields: Vec<FieldTy>,
    pub span: Span,
}

/// One `name: Type` entry of a type declaration.
#[derive(Debug)]
pub struct FieldTy {
    pub name: String,
    pub ty: TypeName,
    pub span: Span,
}

/// A type reference: a name plus optional generic arguments (`Float`,
/// `List<String>`, `Position`).
#[derive(Debug)]
pub struct TypeName {
    pub name: String,
    pub args: Vec<TypeName>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind {
    /// Ints and floats both parse to f64 (MLE has one number type for now).
    Number(f64),
    String(String),
    Bool(bool),
    /// A possibly-qualified name: `x` is `["x"]`, `Text.toBullets` is
    /// `["Text", "toBullets"]`. The parser absorbs a `.segment` into the
    /// name while the segment left of the `.` starts uppercase (a module or
    /// type qualifier); a lowercase name followed by `.` is [`Self::FieldAccess`]
    /// instead (`position.x`). The rule is purely syntactic — field access on
    /// a value bound to an uppercase name also parses as a qualified name;
    /// B2 name resolution reinterprets the segments against the environment
    /// (nothing is lost: segments and span are preserved).
    Ident(Vec<String>),
    /// `{ x: 1.0, y: 2.0 }`
    Record(Vec<Field>),
    /// `[1.0, 2.0, 3.0]`
    List(Vec<Expr>),
    /// `position.x`
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    /// `(a: Type, b) : RetType => body` — param and return annotations are
    /// optional.
    Lambda {
        params: Vec<Param>,
        ret: Option<TypeName>,
        body: Box<Expr>,
    },
    /// `f(x, y)`
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `head |> stages[0] |> stages[1] …` — kept explicit, not desugared
    /// (see module docs).
    Pipeline {
        head: Box<Expr>,
        stages: Vec<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Unary minus.
    Neg(Box<Expr>),
}

/// One `name: value` entry of a record expression.
#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// A lambda parameter with its optional type annotation.
#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeName>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    Eq,
}
