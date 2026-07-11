//! Functor Lang surface parser, core IR, interpreter, and typechecker — Tracks B1–B4
//! of `docs/functor-lang.md`.
//!
//! Lexer + hand-rolled recursive-descent parser producing a surface AST in
//! which every node carries a byte-offset [`Span`] (line/col derive from the
//! source via [`line_col`]); a lowering pass ([`lower`]) from that AST to the
//! name-resolved core IR ([`ir`]); a tree-walking interpreter over the IR
//! ([`eval`]) with an optional call trace; and a gradual typechecker over the
//! IR ([`types`]) — checking with annotations, not inference.

pub mod ast;
pub mod codelens;
pub mod complete;
pub mod eval;
pub mod goto;
pub mod hover;
pub mod inlay;
pub mod ir;
mod lexer;
mod lower;
mod parser;
pub mod project;
pub mod rebind;
mod span;
pub mod trace;
pub mod types;
pub mod value;

pub use eval::{
    render_trace, run, run_with_host, Host, NoHost, RunFailure, RunOutcome, RunRecord, Session,
    Tracing,
};
pub use lower::lower;
pub use parser::{parse, parse_interface};
pub use rebind::{rebind_value, RebindReport};
pub use span::{line_col, Span};
pub use trace::set_trace_sink;
pub use types::{check, check_with_scopes_and_types, check_with_types, ExprTypes};
pub use value::{HostData, Value};

/// A lex or parse failure: a message plus the span of the offending source.
/// Render positions with [`line_col`] (`file:line:col: message`).
#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

/// A lowering failure (e.g. an unresolved name): same shape and rendering as
/// [`ParseError`].
#[derive(Debug)]
pub struct LowerError {
    pub message: String,
    pub span: Span,
}

/// A runtime failure (unknown external, arity mismatch, calling a
/// non-function, …): same shape and rendering as [`ParseError`].
#[derive(Debug)]
pub struct RunError {
    pub message: String,
    pub span: Span,
}

/// One typechecking diagnostic: same shape and rendering as [`ParseError`].
/// Unlike the other error kinds, [`check`] collects *all* of them rather
/// than stopping at the first.
#[derive(Debug)]
pub struct CheckError {
    pub message: String,
    pub span: Span,
}
