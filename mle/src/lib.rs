//! MLE surface parser — Track B1 of `docs/mle.md`.
//!
//! Lexer + hand-rolled recursive-descent parser producing a surface AST in
//! which every node carries a byte-offset [`Span`] (line/col derive from the
//! source via [`line_col`]). Parser only: no IR, no name resolution, no
//! typechecking, no evaluation — those are milestones B2+.

pub mod ast;
mod lexer;
mod parser;
mod span;

pub use parser::parse;
pub use span::{line_col, Span};

/// A lex or parse failure: a message plus the span of the offending source.
/// Render positions with [`line_col`] (`file:line:col: message`).
#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}
