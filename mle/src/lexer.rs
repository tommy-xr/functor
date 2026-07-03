//! Hand-rolled lexer for the B1 subset. Produces spanned tokens; the stream
//! always ends with exactly one `Eof` token so the parser can peek ahead
//! without bounds checks.

use crate::span::Span;
use crate::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Number(f64),
    Str(String),
    Ident(String),
    Let,
    Type,
    True,
    False,
    Mut,
    With,
    In,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    ColonEq,
    Semi,
    Dot,
    Eq,
    EqEq,
    FatArrow,
    PipeGt,
    Plus,
    Minus,
    Star,
    Slash,
    Lt,
    Gt,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Human-readable token description for error messages.
pub fn describe(kind: &TokenKind) -> String {
    use TokenKind::*;
    match kind {
        Number(n) => format!("number `{n}`"),
        Str(_) => "a string".to_string(),
        Ident(name) => format!("`{name}`"),
        Let => "`let`".to_string(),
        Type => "`type`".to_string(),
        True => "`true`".to_string(),
        False => "`false`".to_string(),
        Mut => "`mut`".to_string(),
        With => "`with`".to_string(),
        In => "`in`".to_string(),
        LParen => "`(`".to_string(),
        RParen => "`)`".to_string(),
        LBrace => "`{`".to_string(),
        RBrace => "`}`".to_string(),
        LBracket => "`[`".to_string(),
        RBracket => "`]`".to_string(),
        Comma => "`,`".to_string(),
        Colon => "`:`".to_string(),
        ColonEq => "`:=`".to_string(),
        Semi => "`;`".to_string(),
        Dot => "`.`".to_string(),
        Eq => "`=`".to_string(),
        EqEq => "`==`".to_string(),
        FatArrow => "`=>`".to_string(),
        PipeGt => "`|>`".to_string(),
        Plus => "`+`".to_string(),
        Minus => "`-`".to_string(),
        Star => "`*`".to_string(),
        Slash => "`/`".to_string(),
        Lt => "`<`".to_string(),
        Gt => "`>`".to_string(),
        Eof => "end of input".to_string(),
    }
}

pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            // Line comment: `//` to end of line.
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'(' => {
                i += 1;
                TokenKind::LParen
            }
            b')' => {
                i += 1;
                TokenKind::RParen
            }
            b'{' => {
                i += 1;
                TokenKind::LBrace
            }
            b'}' => {
                i += 1;
                TokenKind::RBrace
            }
            b'[' => {
                i += 1;
                TokenKind::LBracket
            }
            b']' => {
                i += 1;
                TokenKind::RBracket
            }
            b',' => {
                i += 1;
                TokenKind::Comma
            }
            b':' => match bytes.get(i + 1) {
                Some(b'=') => {
                    i += 2;
                    TokenKind::ColonEq
                }
                _ => {
                    i += 1;
                    TokenKind::Colon
                }
            },
            b';' => {
                i += 1;
                TokenKind::Semi
            }
            b'.' => {
                i += 1;
                TokenKind::Dot
            }
            b'+' => {
                i += 1;
                TokenKind::Plus
            }
            // No negative number literals — unary minus is the parser's job.
            b'-' => {
                i += 1;
                TokenKind::Minus
            }
            b'*' => {
                i += 1;
                TokenKind::Star
            }
            b'/' => {
                i += 1;
                TokenKind::Slash
            }
            b'<' => {
                i += 1;
                TokenKind::Lt
            }
            b'>' => {
                i += 1;
                TokenKind::Gt
            }
            b'=' => match bytes.get(i + 1) {
                Some(b'=') => {
                    i += 2;
                    TokenKind::EqEq
                }
                Some(b'>') => {
                    i += 2;
                    TokenKind::FatArrow
                }
                _ => {
                    i += 1;
                    TokenKind::Eq
                }
            },
            b'|' => match bytes.get(i + 1) {
                Some(b'>') => {
                    i += 2;
                    TokenKind::PipeGt
                }
                _ => {
                    return Err(ParseError {
                        message: "unexpected character `|`".to_string(),
                        span: Span::new(i, i + 1),
                    })
                }
            },
            b'"' => {
                let (kind, next) = lex_string(src, i)?;
                i = next;
                kind
            }
            b'0'..=b'9' => {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let n = src[start..i].parse().expect("digit runs parse as f64");
                TokenKind::Number(n)
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                match &src[start..i] {
                    "let" => TokenKind::Let,
                    "type" => TokenKind::Type,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    "mut" => TokenKind::Mut,
                    "with" => TokenKind::With,
                    "in" => TokenKind::In,
                    name => TokenKind::Ident(name.to_string()),
                }
            }
            _ => {
                let c = src[i..].chars().next().expect("lex is on a char boundary");
                return Err(ParseError {
                    message: format!("unexpected character `{c}`"),
                    span: Span::new(i, i + c.len_utf8()),
                });
            }
        };
        tokens.push(Token {
            kind,
            span: Span::new(start, i),
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(src.len(), src.len()),
    });
    Ok(tokens)
}

/// Lex a string literal starting at its opening quote. Escapes: `\"`, `\\`,
/// `\n`, `\t`. Returns the token kind and the index just past the closing
/// quote. An unterminated string reports its span at the opening quote.
fn lex_string(src: &str, start: usize) -> Result<(TokenKind, usize), ParseError> {
    let bytes = src.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = start + 1;
    loop {
        match bytes.get(i) {
            None => {
                return Err(ParseError {
                    message: "unterminated string".to_string(),
                    span: Span::new(start, start + 1),
                })
            }
            Some(b'"') => {
                let s = String::from_utf8(out).expect("string contents are valid UTF-8");
                return Ok((TokenKind::Str(s), i + 1));
            }
            Some(b'\\') => {
                let escaped = match bytes.get(i + 1) {
                    Some(b'"') => b'"',
                    Some(b'\\') => b'\\',
                    Some(b'n') => b'\n',
                    Some(b't') => b'\t',
                    _ => {
                        // Size the span by the escaped char's UTF-8 width so
                        // it stays sliceable (spans must be char-boundary
                        // aligned); a `\` at end of input spans just itself.
                        let escaped_len = src[i + 1..].chars().next().map_or(0, char::len_utf8);
                        return Err(ParseError {
                            message: "unknown escape sequence".to_string(),
                            span: Span::new(i, i + 1 + escaped_len),
                        });
                    }
                };
                out.push(escaped);
                i += 2;
            }
            Some(&b) => {
                out.push(b);
                i += 1;
            }
        }
    }
}
