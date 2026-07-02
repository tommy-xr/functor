//! MLE surface parser, core IR, and interpreter — Tracks B1–B3 of
//! `docs/mle.md`.
//!
//! Lexer + hand-rolled recursive-descent parser producing a surface AST in
//! which every node carries a byte-offset [`Span`] (line/col derive from the
//! source via [`line_col`]); a lowering pass ([`lower`]) from that AST to the
//! name-resolved core IR ([`ir`]); and a tree-walking interpreter over the IR
//! ([`eval`]) with an optional call trace. No typechecking — that is B4.

pub mod ast;
pub mod eval;
pub mod ir;
mod lexer;
mod lower;
mod parser;
mod span;
pub mod value;

pub use eval::{
    render_trace, run, run_with_host, Host, NoHost, RunFailure, RunOutcome, RunRecord, Tracing,
};
pub use lower::lower;
pub use parser::parse;
pub use span::{line_col, Span};
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
