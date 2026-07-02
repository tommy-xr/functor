//! MLE surface parser and core IR — Tracks B1 + B2 of `docs/mle.md`.
//!
//! Lexer + hand-rolled recursive-descent parser producing a surface AST in
//! which every node carries a byte-offset [`Span`] (line/col derive from the
//! source via [`line_col`]), and a lowering pass ([`lower`]) from that AST to
//! the name-resolved core IR ([`ir`]). No typechecking, no evaluation —
//! those are milestones B3+.

pub mod ast;
pub mod ir;
mod lexer;
mod lower;
mod parser;
mod span;

pub use lower::lower;
pub use parser::parse;
pub use span::{line_col, Span};

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
