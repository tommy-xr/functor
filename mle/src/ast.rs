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
    Open(OpenDecl),
    /// A bodyless value SIGNATURE (`let name : Type`), only in interface files
    /// (`.mlei`): the type of a value the host implements (a module is either a
    /// `.mle` or a `.mlei`, never both, so there is no paired implementation).
    Sig(SigDecl),
}

/// `let name : Type` in a `.mlei` — a value signature with no body.
#[derive(Debug)]
pub struct SigDecl {
    pub name: String,
    pub ty: TypeName,
    pub span: Span,
}

/// `open Utils` — bring a sibling module's defs, constructors, and types
/// into scope unqualified (B8 modules; qualified access `Utils.x` needs no
/// `open`). `open` is a *contextual* keyword: it only means this at the top
/// level, so `open` stays usable as an ordinary name everywhere else.
#[derive(Debug)]
pub struct OpenDecl {
    pub module: String,
    pub span: Span,
}

/// `let name = expr` or `let name: Type = expr` — the expr may be a lambda
/// (`let f = (a, b) => …`). An optional binding type annotation (`ty`) is
/// checked against the value (see `crate::types`).
#[derive(Debug)]
pub struct LetDecl {
    pub name: String,
    pub ty: Option<TypeName>,
    pub value: Expr,
    pub span: Span,
}

/// `type Position = { x: Float, y: Float }` (a record type) or
/// `type Shape = | Circle(radius: Float) | Point` (a variant type),
/// optionally generic: `type Box<a> = | Full(v: a) | Empty`.
#[derive(Debug)]
pub struct TypeDecl {
    pub name: String,
    /// Declared type parameters, lowercase (`<a, b>`); empty when the
    /// declaration is not generic.
    pub params: Vec<String>,
    pub body: TypeBody,
    pub span: Span,
}

/// What a `type` declares: a record shape, one-or-more variant constructors
/// (each `|`-led, including the first), or — with no `= body` — an ABSTRACT
/// (opaque) nominal type: a named handle with no fields and no constructors,
/// produced/consumed only by host functions (`type SceneNode`).
#[derive(Debug)]
pub enum TypeBody {
    Record(Vec<FieldTy>),
    Variants(Vec<VariantDecl>),
    Abstract,
}

/// One `| Ctor(name: Type, …)` / `| Ctor` alternative of a variant type.
/// Fields are named in the declaration (self-documenting) but constructors
/// are *called* positionally: `Circle(2.0)`.
#[derive(Debug)]
pub struct VariantDecl {
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
    /// `{ base with x: 1.0 }` — a copy of `base` with the listed fields
    /// replaced. Pure; every named field must exist on the base at runtime.
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
    /// `(1.0, "a")` — at least two elements (`(e)` is grouping, not a
    /// 1-tuple).
    Tuple(Vec<Expr>),
    /// `let x = e in body` / `let mut x = e in body` — an expression-level
    /// binding scoped to `body`. Only `mut` bindings may be assigned
    /// ([`Self::Assign`]), and a lambda may not capture one (lowering
    /// enforces both — see `~/notes/ideas/mle-language/mutability.md`).
    Let {
        mutable: bool,
        name: String,
        ty: Option<TypeName>,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    /// `x := e; rest` — rebind a `mut` binding, then continue with `rest`
    /// (the assignment always carries its continuation, so no sequencing
    /// operator or Unit type is needed).
    Assign {
        name: String,
        value: Box<Expr>,
        rest: Box<Expr>,
    },
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
    /// `match expr with | pattern => expr | …` — first matching arm wins,
    /// top to bottom. Arm bodies are full expressions, so a nested `match`
    /// inside an arm greedily consumes the following `|` arms — parenthesize
    /// inner matches (the F#/OCaml convention; see [`crate::parser`]).
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

/// One `| pattern => body` arm of a `match`.
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

/// The deliberately-minimal B5 pattern language: constructor patterns whose
/// sub-patterns are variable bindings or `_` (no deeper nesting), bare
/// variables, `_`, and literal (equality) patterns.
#[derive(Debug)]
pub enum PatternKind {
    /// `_` — matches anything, binds nothing.
    Wildcard,
    /// A bare lowercase name — matches anything, binds it.
    Var(String),
    /// `Circle(r)` / `Point` — an uppercase name is always a constructor
    /// pattern (never a variable).
    Ctor {
        name: String,
        args: Vec<Pattern>,
    },
    /// `(x, _)` — element sub-patterns are variable bindings or `_` only
    /// (like ctor patterns); arity must match the matched tuple exactly.
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
