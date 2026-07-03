//! Recursive-descent parser for the B1 subset.
//!
//! Grammar (whitespace and newlines are insignificant):
//!
//! ```text
//! program   := (letDecl | typeDecl)*
//! letDecl   := "let" ident "=" expr
//! typeDecl  := "type" ident "=" ("{" (ident ":" type),* "}" | variant+)
//! variant   := "|" upperIdent ("(" (ident ":" type),+ ")")?
//! type      := ident ("<" type ("," type)* ">")?
//! expr      := letIn | assign | match | pipeline
//! letIn     := "let" "mut"? ident "=" expr "in" expr
//! assign    := ident ":=" expr ";" expr
//! match     := "match" expr "with" ("|" pattern "=>" expr)+
//! pattern   := "_" | lowerIdent | upperIdent ("(" subpat,+ ")")?
//!            | "true" | "false" | number | string
//! subpat    := "_" | lowerIdent
//! pipeline  := cmp ("|>" cmp)*
//! cmp       := add (("<" | ">" | "==") add)*        (left-assoc)
//! add       := mul (("+" | "-") mul)*               (left-assoc)
//! mul       := unary (("*" | "/") unary)*           (left-assoc)
//! unary     := "-" unary | postfix
//! postfix   := primary ("(" expr,* ")" | "." ident)*
//! primary   := number | string | "true" | "false" | qualifiedIdent
//!            | record | list | lambda | "(" expr ")"
//! record    := "{" (ident ":" expr),* "}"
//!            | "{" expr "with" (ident ":" expr),+ "}"
//! list      := "[" expr,* "]"
//! lambda    := "(" (ident (":" type)?),* ")" (":" type)? "=>" expr
//! ```
//!
//! Comma lists allow a trailing comma. `(` is disambiguated between lambda
//! and parenthesized expression by scanning to the matching `)`: it is a
//! lambda iff the next token is `=>` or `:` (a return-type annotation).
//!
//! **Greedy match arms.** Arm bodies are full expressions, so a nested
//! `match` inside an arm consumes the following `|` arms as its own —
//! parenthesize the inner match to return to the outer one (the same
//! convention as F#/OCaml). The leading `|` is required before *every* arm
//! (and every variant alternative), including the first — that keeps the
//! layout-free grammar unambiguous.

use crate::ast::*;
use crate::lexer::{describe, lex, Token, TokenKind};
use crate::span::Span;
use crate::ParseError;

/// Parse a whole source file into a [`Program`].
pub fn parse(src: &str) -> Result<Program, ParseError> {
    let tokens = lex(src)?;
    Parser {
        tokens,
        pos: 0,
        depth: 0,
    }
    .program()
}

/// Nesting cap for the recursive entry points: pathological input
/// (`((((…))))`) must fail as a clean spanned error, not a stack overflow —
/// MLE sources may be machine-generated. Each nesting level costs ~10 debug
/// frames, so the cap must fit a 2 MiB test-thread stack with margin.
const MAX_DEPTH: usize = 100;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    /// Kind of the token `n` past the cursor, clamped to the trailing `Eof`.
    fn nth_kind(&self, n: usize) -> &TokenKind {
        let idx = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[idx].kind
    }

    /// Consume and return the current token; never advances past `Eof`.
    fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    fn error<T>(&self, expected: &str) -> Result<T, ParseError> {
        Err(ParseError {
            message: format!("expected {expected}, found {}", describe(self.peek_kind())),
            span: self.peek().span,
        })
    }

    fn expect(&mut self, kind: TokenKind, expected: &str) -> Result<Token, ParseError> {
        if self.peek_kind() == &kind {
            Ok(self.bump())
        } else {
            self.error(expected)
        }
    }

    fn expect_ident(&mut self, expected: &str) -> Result<(String, Span), ParseError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                Ok((name, self.bump().span))
            }
            _ => self.error(expected),
        }
    }

    fn program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();
        while self.peek_kind() != &TokenKind::Eof {
            match self.peek_kind() {
                TokenKind::Let => items.push(Item::Let(self.let_decl()?)),
                TokenKind::Type => items.push(Item::Type(self.type_decl()?)),
                _ => return self.error("`let` or `type` at top level"),
            }
        }
        Ok(Program { items })
    }

    fn let_decl(&mut self) -> Result<LetDecl, ParseError> {
        let kw = self.bump();
        if self.peek_kind() == &TokenKind::Mut {
            return Err(ParseError {
                message: "top-level bindings cannot be mutable (globals are the hot-reload \
rebind surface); `mut` is for `let mut … in …` inside a function"
                    .to_string(),
                span: self.peek().span,
            });
        }
        let (name, _) = self.expect_ident("a name after `let`")?;
        self.expect(TokenKind::Eq, "`=`")?;
        let value = self.expr()?;
        let span = kw.span.to(value.span);
        Ok(LetDecl { name, value, span })
    }

    fn type_decl(&mut self) -> Result<TypeDecl, ParseError> {
        let kw = self.bump();
        let (name, _) = self.expect_ident("a name after `type`")?;
        self.expect(TokenKind::Eq, "`=`")?;
        match self.peek_kind() {
            TokenKind::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                while self.peek_kind() != &TokenKind::RBrace {
                    let (field_name, field_span) = self.expect_ident("a field name")?;
                    self.expect(TokenKind::Colon, "`:` after field name")?;
                    let ty = self.type_name()?;
                    fields.push(FieldTy {
                        span: field_span.to(ty.span),
                        name: field_name,
                        ty,
                    });
                    if self.peek_kind() == &TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
                let close = self.expect(TokenKind::RBrace, "`,` or `}`")?;
                Ok(TypeDecl {
                    name,
                    body: TypeBody::Record(fields),
                    span: kw.span.to(close.span),
                })
            }
            TokenKind::Pipe => {
                let mut variants = Vec::new();
                while self.peek_kind() == &TokenKind::Pipe {
                    self.bump();
                    variants.push(self.variant_decl()?);
                }
                let span = kw
                    .span
                    .to(variants.last().expect("at least one variant").span);
                Ok(TypeDecl {
                    name,
                    body: TypeBody::Variants(variants),
                    span,
                })
            }
            // A leading `|` is required before every alternative, including
            // the first (see the module docs).
            _ => self.error("`{` (a record type) or `|` (a variant alternative)"),
        }
    }

    /// One `Ctor(name: Type, …)` / `Ctor` alternative (its leading `|`
    /// already consumed).
    fn variant_decl(&mut self) -> Result<VariantDecl, ParseError> {
        let (name, name_span) = self.expect_ident("a constructor name after `|`")?;
        if !starts_uppercase(&name) {
            return Err(ParseError {
                message: format!("constructor name `{name}` must start uppercase"),
                span: name_span,
            });
        }
        let mut fields = Vec::new();
        let mut span = name_span;
        if self.peek_kind() == &TokenKind::LParen {
            self.bump();
            loop {
                let (field_name, field_span) = self.expect_ident("a field name")?;
                self.expect(TokenKind::Colon, "`:` after field name")?;
                let ty = self.type_name()?;
                fields.push(FieldTy {
                    span: field_span.to(ty.span),
                    name: field_name,
                    ty,
                });
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                    if self.peek_kind() == &TokenKind::RParen {
                        break; // trailing comma
                    }
                } else {
                    break;
                }
            }
            let close = self.expect(TokenKind::RParen, "`,` or `)`")?;
            span = name_span.to(close.span);
        }
        Ok(VariantDecl { name, fields, span })
    }

    fn type_name(&mut self) -> Result<TypeName, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError {
                message: "type nested too deeply".to_string(),
                span: self.peek().span,
            });
        }
        let (name, mut span) = self.expect_ident("a type name")?;
        let mut args = Vec::new();
        if self.peek_kind() == &TokenKind::Lt {
            self.bump();
            loop {
                args.push(self.type_name()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                    if self.peek_kind() == &TokenKind::Gt {
                        break; // trailing comma
                    }
                } else {
                    break;
                }
            }
            let close = self.expect(TokenKind::Gt, "`,` or `>`")?;
            span = span.to(close.span);
        }
        self.depth -= 1;
        Ok(TypeName { name, args, span })
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError {
                message: "expression nested too deeply".to_string(),
                span: self.peek().span,
            });
        }
        let result = match self.peek_kind() {
            TokenKind::Let => self.let_in(),
            TokenKind::Match => self.match_expr(),
            TokenKind::Ident(_) if self.nth_kind(1) == &TokenKind::ColonEq => self.assign(),
            _ => {
                let expr = self.pipeline();
                // A stray `:=` after a non-name expression would otherwise
                // surface as a baffling error at the enclosing context.
                if expr.is_ok() && self.peek_kind() == &TokenKind::ColonEq {
                    self.error("nothing (assignment targets must be a bare `let mut` name)")
                } else {
                    expr
                }
            }
        };
        self.depth -= 1;
        result
    }

    /// `let [mut] name = value in body` — expression-level binding.
    fn let_in(&mut self) -> Result<Expr, ParseError> {
        let kw = self.bump();
        let mutable = if self.peek_kind() == &TokenKind::Mut {
            self.bump();
            true
        } else {
            false
        };
        let (name, _) = self.expect_ident("a name after `let`")?;
        self.expect(TokenKind::Eq, "`=`")?;
        let value = self.expr()?;
        self.expect(TokenKind::In, "`in`")?;
        let body = self.expr()?;
        let span = kw.span.to(body.span);
        Ok(Expr {
            kind: ExprKind::Let {
                mutable,
                name,
                value: Box::new(value),
                body: Box::new(body),
            },
            span,
        })
    }

    /// `name := value; rest` — the assignment always carries its
    /// continuation (see the AST docs).
    fn assign(&mut self) -> Result<Expr, ParseError> {
        let (name, name_span) = self.expect_ident("a name")?;
        self.expect(TokenKind::ColonEq, "`:=`")?;
        let value = self.expr()?;
        self.expect(
            TokenKind::Semi,
            "`;` (an assignment carries its continuation)",
        )?;
        let rest = self.expr()?;
        let span = name_span.to(rest.span);
        Ok(Expr {
            kind: ExprKind::Assign {
                name,
                value: Box::new(value),
                rest: Box::new(rest),
            },
            span,
        })
    }

    /// `match expr with | pattern => expr | …` — lowest-precedence, like
    /// let-in. Arm bodies are full expressions, so arms are consumed
    /// greedily (see the module docs).
    fn match_expr(&mut self) -> Result<Expr, ParseError> {
        let kw = self.bump();
        let scrutinee = self.expr()?;
        self.expect(TokenKind::With, "`with` after the match scrutinee")?;
        if self.peek_kind() != &TokenKind::Pipe {
            // A leading `|` is required before every arm, including the first.
            return self.error("`|` to begin a match arm");
        }
        let mut arms = Vec::new();
        while self.peek_kind() == &TokenKind::Pipe {
            let bar = self.bump();
            let pattern = self.pattern()?;
            self.expect(TokenKind::FatArrow, "`=>` after the pattern")?;
            let body = self.expr()?;
            arms.push(MatchArm {
                span: bar.span.to(body.span),
                pattern,
                body,
            });
        }
        let span = kw.span.to(arms.last().expect("at least one arm").span);
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        })
    }

    fn pattern(&mut self) -> Result<Pattern, ParseError> {
        let span = self.peek().span;
        let kind = match self.peek_kind() {
            TokenKind::Number(n) => {
                let n = *n;
                self.bump();
                PatternKind::Number(n)
            }
            TokenKind::Str(s) => {
                let s = s.clone();
                self.bump();
                PatternKind::String(s)
            }
            TokenKind::True => {
                self.bump();
                PatternKind::Bool(true)
            }
            TokenKind::False => {
                self.bump();
                PatternKind::Bool(false)
            }
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                PatternKind::Wildcard
            }
            TokenKind::Ident(name) if !starts_uppercase(name) => {
                let name = name.clone();
                self.bump();
                PatternKind::Var(name)
            }
            // Uppercase: always a constructor pattern, never a variable.
            TokenKind::Ident(_) => return self.ctor_pattern(),
            _ => return self.error("a pattern"),
        };
        Ok(Pattern { kind, span })
    }

    /// `Circle(r, _)` / `Point` — sub-patterns are variable bindings or `_`
    /// only (the deliberately-minimal B5 pattern language; no nesting).
    fn ctor_pattern(&mut self) -> Result<Pattern, ParseError> {
        let (name, name_span) = self.expect_ident("a constructor name")?;
        let mut args = Vec::new();
        let mut span = name_span;
        if self.peek_kind() == &TokenKind::LParen {
            self.bump();
            loop {
                args.push(self.sub_pattern()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                    if self.peek_kind() == &TokenKind::RParen {
                        break; // trailing comma
                    }
                } else {
                    break;
                }
            }
            let close = self.expect(TokenKind::RParen, "`,` or `)`")?;
            span = name_span.to(close.span);
        }
        Ok(Pattern {
            kind: PatternKind::Ctor { name, args },
            span,
        })
    }

    fn sub_pattern(&mut self) -> Result<Pattern, ParseError> {
        let span = self.peek().span;
        let kind = match self.peek_kind() {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                PatternKind::Wildcard
            }
            TokenKind::Ident(name) if !starts_uppercase(name) => {
                let name = name.clone();
                self.bump();
                PatternKind::Var(name)
            }
            _ => return self.error("a binding name or `_` (constructor patterns do not nest)"),
        };
        Ok(Pattern { kind, span })
    }

    fn pipeline(&mut self) -> Result<Expr, ParseError> {
        let head = self.comparison()?;
        if self.peek_kind() != &TokenKind::PipeGt {
            return Ok(head);
        }
        let mut stages = Vec::new();
        while self.peek_kind() == &TokenKind::PipeGt {
            self.bump();
            stages.push(self.comparison()?);
        }
        let span = head
            .span
            .to(stages.last().expect("at least one stage").span);
        Ok(Expr {
            kind: ExprKind::Pipeline {
                head: Box::new(head),
                stages,
            },
            span,
        })
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        use TokenKind::*;
        self.left_assoc(
            &[(Lt, BinOp::Lt), (Gt, BinOp::Gt), (EqEq, BinOp::Eq)],
            Self::additive,
        )
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        use TokenKind::*;
        self.left_assoc(
            &[(Plus, BinOp::Add), (Minus, BinOp::Sub)],
            Self::multiplicative,
        )
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        use TokenKind::*;
        self.left_assoc(&[(Star, BinOp::Mul), (Slash, BinOp::Div)], Self::unary)
    }

    fn left_assoc(
        &mut self,
        ops: &[(TokenKind, BinOp)],
        next: fn(&mut Self) -> Result<Expr, ParseError>,
    ) -> Result<Expr, ParseError> {
        let mut lhs = next(self)?;
        loop {
            let Some((_, op)) = ops.iter().find(|(kind, _)| kind == self.peek_kind()) else {
                return Ok(lhs);
            };
            let op = *op;
            self.bump();
            let rhs = next(self)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
    }

    // Iterative (not recursive) so `----x` chains can't grow the stack past
    // the `expr` depth guard.
    fn unary(&mut self) -> Result<Expr, ParseError> {
        let mut minus_spans = Vec::new();
        while self.peek_kind() == &TokenKind::Minus {
            minus_spans.push(self.bump().span);
        }
        let mut expr = self.postfix()?;
        for minus_span in minus_spans.into_iter().rev() {
            let span = minus_span.to(expr.span);
            expr = Expr {
                kind: ExprKind::Neg(Box::new(expr)),
                span,
            };
        }
        Ok(expr)
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    while self.peek_kind() != &TokenKind::RParen {
                        args.push(self.expr()?);
                        if self.peek_kind() == &TokenKind::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let close = self.expect(TokenKind::RParen, "`,` or `)`")?;
                    let span = expr.span.to(close.span);
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let (field, field_span) = self.expect_ident("a field name after `.`")?;
                    let span = expr.span.to(field_span);
                    expr = Expr {
                        kind: ExprKind::FieldAccess {
                            object: Box::new(expr),
                            field,
                        },
                        span,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.peek().span;
        match self.peek_kind() {
            TokenKind::Number(n) => {
                let n = *n;
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Number(n),
                    span,
                })
            }
            TokenKind::Str(s) => {
                let s = s.clone();
                self.bump();
                Ok(Expr {
                    kind: ExprKind::String(s),
                    span,
                })
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span,
                })
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span,
                })
            }
            TokenKind::Ident(_) => self.ident_expr(),
            TokenKind::LBrace => self.record(),
            TokenKind::LBracket => self.list(),
            TokenKind::LParen => {
                if self.lambda_ahead() {
                    self.lambda()
                } else {
                    self.paren()
                }
            }
            _ => self.error("an expression"),
        }
    }

    /// A possibly-qualified identifier. `.segment`s are absorbed into the
    /// name while the segment left of the `.` starts uppercase (a module or
    /// type qualifier, e.g. `Text.toBullets`); once a segment starts
    /// lowercase, any further `.` is field access (handled by [`Self::postfix`]).
    fn ident_expr(&mut self) -> Result<Expr, ParseError> {
        let (first, mut span) = self.expect_ident("an identifier")?;
        let mut segments = vec![first];
        while starts_uppercase(segments.last().expect("non-empty"))
            && self.peek_kind() == &TokenKind::Dot
            && matches!(self.nth_kind(1), TokenKind::Ident(_))
        {
            self.bump();
            let (segment, segment_span) = self.expect_ident("an identifier")?;
            span = span.to(segment_span);
            segments.push(segment);
        }
        Ok(Expr {
            kind: ExprKind::Ident(segments),
            span,
        })
    }

    /// `{` begins a record literal (empty, or `name:` first) or a record
    /// update (`{ base with … }`).
    fn record(&mut self) -> Result<Expr, ParseError> {
        let open = self.bump();
        let literal = self.peek_kind() == &TokenKind::RBrace
            || (matches!(self.peek_kind(), TokenKind::Ident(_))
                && self.nth_kind(1) == &TokenKind::Colon);
        if literal {
            let fields = self.record_fields()?;
            let close = self.expect(TokenKind::RBrace, "`,` or `}`")?;
            return Ok(Expr {
                kind: ExprKind::Record(fields),
                span: open.span.to(close.span),
            });
        }
        let base = self.expr()?;
        self.expect(TokenKind::With, "`with` (or `name:` for a record literal)")?;
        let fields = self.record_fields()?;
        if fields.is_empty() {
            // A zero-field update is always a mistake (a silent copy).
            return self.error("at least one `name: value` after `with`");
        }
        let close = self.expect(TokenKind::RBrace, "`,` or `}`")?;
        Ok(Expr {
            kind: ExprKind::RecordUpdate {
                base: Box::new(base),
                fields,
            },
            span: open.span.to(close.span),
        })
    }

    /// The shared `name: expr` comma list of record literals and updates.
    fn record_fields(&mut self) -> Result<Vec<Field>, ParseError> {
        let mut fields = Vec::new();
        while self.peek_kind() != &TokenKind::RBrace {
            let (name, name_span) = self.expect_ident("a field name")?;
            self.expect(TokenKind::Colon, "`:` after field name")?;
            let value = self.expr()?;
            fields.push(Field {
                span: name_span.to(value.span),
                name,
                value,
            });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(fields)
    }

    fn list(&mut self) -> Result<Expr, ParseError> {
        let open = self.bump();
        let mut items = Vec::new();
        while self.peek_kind() != &TokenKind::RBracket {
            items.push(self.expr()?);
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        let close = self.expect(TokenKind::RBracket, "`,` or `]`")?;
        Ok(Expr {
            kind: ExprKind::List(items),
            span: open.span.to(close.span),
        })
    }

    /// `(` begins either a lambda or a parenthesized expression. Scan to the
    /// matching `)` (depth-counted): it is a lambda iff the token after it is
    /// `=>` or `:` (a return-type annotation). An unmatched `(` falls through
    /// to the paren-expression path, which reports the error at the offending
    /// token.
    fn lambda_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        loop {
            match &self.tokens[i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.tokens[i + 1].kind,
                            TokenKind::FatArrow | TokenKind::Colon
                        );
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn lambda(&mut self) -> Result<Expr, ParseError> {
        let open = self.bump();
        let mut params = Vec::new();
        while self.peek_kind() != &TokenKind::RParen {
            let (name, name_span) = self.expect_ident("a parameter name")?;
            let (ty, span) = if self.peek_kind() == &TokenKind::Colon {
                self.bump();
                let ty = self.type_name()?;
                let span = name_span.to(ty.span);
                (Some(ty), span)
            } else {
                (None, name_span)
            };
            params.push(Param { name, ty, span });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen, "`,` or `)`")?;
        let ret = if self.peek_kind() == &TokenKind::Colon {
            self.bump();
            Some(self.type_name()?)
        } else {
            None
        };
        self.expect(TokenKind::FatArrow, "`=>`")?;
        let body = self.expr()?;
        let span = open.span.to(body.span);
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                ret,
                body: Box::new(body),
            },
            span,
        })
    }

    /// Parentheses don't create an AST node; the inner expression's span is
    /// widened to cover them so every span still maps to exact source text.
    fn paren(&mut self) -> Result<Expr, ParseError> {
        let open = self.bump();
        let mut expr = self.expr()?;
        let close = self.expect(TokenKind::RParen, "`)`")?;
        expr.span = open.span.to(close.span);
        Ok(expr)
    }
}

fn starts_uppercase(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}
