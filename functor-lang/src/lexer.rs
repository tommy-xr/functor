//! Hand-rolled lexer for the B1 subset. Produces spanned tokens; the stream
//! always ends with exactly one `Eof` token so the parser can peek ahead
//! without bounds checks.

use crate::span::Span;
use crate::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Number(f64),
    Str(String),
    InterpolatedStart,
    InterpolatedText(String),
    InterpolatedOpen,
    InterpolatedClose,
    InterpolatedEnd,
    Ident(String),
    /// A type variable in type position: `'a`, `'msg`. Stored WITH the leading
    /// apostrophe, so the string is exactly the source spelling.
    TypeVar(String),
    Let,
    Type,
    True,
    False,
    Mut,
    With,
    In,
    Match,
    If,
    Then,
    Else,
    Not,
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
    DotDot,
    Eq,
    EqEq,
    FatArrow,
    PipeGt,
    PipePipe,
    Pipe,
    AmpAmp,
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
        InterpolatedStart => "an interpolated string".to_string(),
        InterpolatedText(_) => "interpolated string text".to_string(),
        InterpolatedOpen => "`{` in an interpolated string".to_string(),
        InterpolatedClose => "`}` in an interpolated string".to_string(),
        InterpolatedEnd => "the end of an interpolated string".to_string(),
        Ident(name) => format!("`{name}`"),
        TypeVar(name) => format!("`{name}`"),
        Let => "`let`".to_string(),
        Type => "`type`".to_string(),
        True => "`true`".to_string(),
        False => "`false`".to_string(),
        Mut => "`mut`".to_string(),
        With => "`with`".to_string(),
        In => "`in`".to_string(),
        Match => "`match`".to_string(),
        If => "`if`".to_string(),
        Then => "`then`".to_string(),
        Else => "`else`".to_string(),
        Not => "`not`".to_string(),
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
        DotDot => "`..`".to_string(),
        Eq => "`=`".to_string(),
        EqEq => "`==`".to_string(),
        FatArrow => "`=>`".to_string(),
        PipeGt => "`|>`".to_string(),
        PipePipe => "`||`".to_string(),
        Pipe => "`|`".to_string(),
        AmpAmp => "`&&`".to_string(),
        Plus => "`+`".to_string(),
        Minus => "`-`".to_string(),
        Star => "`*`".to_string(),
        Slash => "`/`".to_string(),
        Lt => "`<`".to_string(),
        Gt => "`>`".to_string(),
        Eof => "end of input".to_string(),
    }
}

/// Lex `src`. `base` is added to every span — a project loads several files
/// into one global span space (each file gets a distinct base), so every
/// downstream span (AST, IR, diagnostics, runtime errors) identifies its
/// file as well as its position; `functor_lang::project::SourceMap` renders them.
/// Single-file callers pass 0.
pub fn lex(src: &str, base: usize) -> Result<Vec<Token>, ParseError> {
    lex_with_interpolation_depth(src, base, 0)
}

// Keep recursive interpolation scanning within the parser's nesting budget:
// machine-generated source must fail as a diagnostic, never a host overflow.
const MAX_INTERPOLATION_DEPTH: usize = 76;

fn lex_with_interpolation_depth(
    src: &str,
    base: usize,
    interpolation_depth: usize,
) -> Result<Vec<Token>, ParseError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'"') {
            let (mut interpolation, next) =
                lex_interpolated_string(src, i, base, interpolation_depth + 1)?;
            tokens.append(&mut interpolation);
            i = next;
            continue;
        }
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
            b'.' => match bytes.get(i + 1) {
                Some(b'.') => {
                    i += 2;
                    TokenKind::DotDot
                }
                _ => {
                    i += 1;
                    TokenKind::Dot
                }
            },
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
                Some(b'|') => {
                    i += 2;
                    TokenKind::PipePipe
                }
                // Bare `|` begins a variant alternative or a match arm.
                _ => {
                    i += 1;
                    TokenKind::Pipe
                }
            },
            // `&&` is the only use of `&` — a bare `&` is a lex error.
            b'&' if bytes.get(i + 1) == Some(&b'&') => {
                i += 2;
                TokenKind::AmpAmp
            }
            b'"' => {
                let (kind, next) = lex_string(src, i, base)?;
                i = next;
                kind
            }
            // A type variable: `'a`, `'msg`. The apostrophe must be followed by
            // an identifier char; a bare `'` is a lex error.
            b'\'' => {
                i += 1; // consume `'`
                if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
                    return Err(ParseError {
                        message: "expected a type-variable name after `'` (e.g. `'a`)".to_string(),
                        span: Span::new(base + start, base + i),
                    });
                }
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                TokenKind::TypeVar(src[start..i].to_string())
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
                    "match" => TokenKind::Match,
                    "if" => TokenKind::If,
                    "then" => TokenKind::Then,
                    "else" => TokenKind::Else,
                    "not" => TokenKind::Not,
                    name => TokenKind::Ident(name.to_string()),
                }
            }
            _ => {
                let c = src[i..].chars().next().expect("lex is on a char boundary");
                return Err(ParseError {
                    message: format!("unexpected character `{c}`"),
                    span: Span::new(base + i, base + i + c.len_utf8()),
                });
            }
        };
        tokens.push(Token {
            kind,
            span: Span::new(base + start, base + i),
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(base + src.len(), base + src.len()),
    });
    Ok(tokens)
}

/// Lex a string literal starting at its opening quote. Escapes: `\"`, `\\`,
/// `\n`, `\t`. Returns the token kind and the index just past the closing
/// quote. An unterminated string reports its span at the opening quote.
fn lex_string(src: &str, start: usize, base: usize) -> Result<(TokenKind, usize), ParseError> {
    let bytes = src.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = start + 1;
    loop {
        match bytes.get(i) {
            None => {
                return Err(ParseError {
                    message: "unterminated string".to_string(),
                    span: Span::new(base + start, base + start + 1),
                })
            }
            Some(b'"') => {
                let s = String::from_utf8(out).expect("string contents are valid UTF-8");
                return Ok((TokenKind::Str(s), i + 1));
            }
            Some(b'\\') => {
                let (escaped, next) = lex_escape(src, i, base)?;
                out.push(escaped);
                i = next;
            }
            Some(&b) => {
                out.push(b);
                i += 1;
            }
        }
    }
}

/// Lex an F#-style interpolated string (`$"hello {name}"`). Interpolation
/// holes are lexed recursively as ordinary Functor expressions; dedicated
/// boundary tokens let the parser consume exactly one expression per hole.
/// `{{` and `}}` are literal braces in the surrounding text.
fn lex_interpolated_string(
    src: &str,
    start: usize,
    base: usize,
    depth: usize,
) -> Result<(Vec<Token>, usize), ParseError> {
    if depth > MAX_INTERPOLATION_DEPTH {
        return Err(ParseError {
            message: "interpolation nested too deeply".to_string(),
            span: Span::new(base + start, base + start + 2),
        });
    }
    let bytes = src.as_bytes();
    let mut tokens = vec![Token {
        kind: TokenKind::InterpolatedStart,
        span: Span::new(base + start, base + start + 2),
    }];
    let mut text = Vec::new();
    let mut text_start = start + 2;
    let mut i = text_start;

    loop {
        match bytes.get(i) {
            None => {
                return Err(ParseError {
                    message: "unterminated interpolated string".to_string(),
                    span: Span::new(base + start, base + start + 2),
                })
            }
            Some(b'"') => {
                push_interpolated_text(&mut tokens, &mut text, text_start, i, base);
                tokens.push(Token {
                    kind: TokenKind::InterpolatedEnd,
                    span: Span::new(base + i, base + i + 1),
                });
                return Ok((tokens, i + 1));
            }
            Some(b'{') if bytes.get(i + 1) == Some(&b'{') => {
                text.push(b'{');
                i += 2;
            }
            Some(b'}') if bytes.get(i + 1) == Some(&b'}') => {
                text.push(b'}');
                i += 2;
            }
            Some(b'{') => {
                push_interpolated_text(&mut tokens, &mut text, text_start, i, base);
                tokens.push(Token {
                    kind: TokenKind::InterpolatedOpen,
                    span: Span::new(base + i, base + i + 1),
                });
                let close = find_interpolation_close(src, i + 1, base, i, depth)?;
                let mut hole_tokens =
                    lex_with_interpolation_depth(&src[i + 1..close], base + i + 1, depth)?;
                hole_tokens.pop(); // the containing interpolation is the delimiter
                tokens.append(&mut hole_tokens);
                tokens.push(Token {
                    kind: TokenKind::InterpolatedClose,
                    span: Span::new(base + close, base + close + 1),
                });
                i = close + 1;
                text_start = i;
            }
            Some(b'}') => {
                return Err(ParseError {
                    message:
                        "unmatched `}` in interpolated string (write `}}` for a literal brace)"
                            .to_string(),
                    span: Span::new(base + i, base + i + 1),
                })
            }
            Some(b'\\') => {
                let (escaped, next) = lex_escape(src, i, base)?;
                text.push(escaped);
                i = next;
            }
            Some(&b) => {
                text.push(b);
                i += 1;
            }
        }
    }
}

fn lex_escape(src: &str, slash: usize, base: usize) -> Result<(u8, usize), ParseError> {
    let escaped = match src.as_bytes().get(slash + 1) {
        Some(b'"') => b'"',
        Some(b'\\') => b'\\',
        Some(b'n') => b'\n',
        Some(b't') => b'\t',
        _ => {
            // Size the span by the escaped char's UTF-8 width so it stays
            // sliceable; a trailing `\` spans just itself.
            let escaped_len = src[slash + 1..].chars().next().map_or(0, char::len_utf8);
            return Err(ParseError {
                message: "unknown escape sequence".to_string(),
                span: Span::new(base + slash, base + slash + 1 + escaped_len),
            });
        }
    };
    Ok((escaped, slash + 2))
}

fn push_interpolated_text(
    tokens: &mut Vec<Token>,
    text: &mut Vec<u8>,
    start: usize,
    end: usize,
    base: usize,
) {
    if text.is_empty() {
        return;
    }
    let value = String::from_utf8(std::mem::take(text))
        .expect("interpolated string contents are valid UTF-8");
    tokens.push(Token {
        kind: TokenKind::InterpolatedText(value),
        span: Span::new(base + start, base + end),
    });
}

/// Find the `}` ending one interpolation hole, ignoring balanced record
/// braces, comments, quoted strings, and nested interpolated strings.
fn find_interpolation_close(
    src: &str,
    mut i: usize,
    base: usize,
    opening: usize,
    depth: usize,
) -> Result<usize, ParseError> {
    let bytes = src.as_bytes();
    let mut braces = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'$' if bytes.get(i + 1) == Some(&b'"') => {
                i = scan_interpolated_string(src, i, base, depth + 1)?;
            }
            b'"' => {
                let (_, next) = lex_string(src, i, base)?;
                i = next;
            }
            b'{' => {
                braces += 1;
                i += 1;
            }
            b'}' if braces == 0 => return Ok(i),
            b'}' => {
                braces -= 1;
                i += 1;
            }
            _ => {
                let c = src[i..].chars().next().expect("scan is on a char boundary");
                i += c.len_utf8();
            }
        }
    }
    Err(ParseError {
        message: "unterminated interpolation hole".to_string(),
        span: Span::new(base + opening, base + opening + 1),
    })
}

fn scan_interpolated_string(
    src: &str,
    start: usize,
    base: usize,
    depth: usize,
) -> Result<usize, ParseError> {
    if depth > MAX_INTERPOLATION_DEPTH {
        return Err(ParseError {
            message: "interpolation nested too deeply".to_string(),
            span: Span::new(base + start, base + start + 2),
        });
    }
    let bytes = src.as_bytes();
    let mut i = start + 2;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Ok(i + 1),
            b'\\' => i += if i + 1 < bytes.len() { 2 } else { 1 },
            b'{' if bytes.get(i + 1) == Some(&b'{') => i += 2,
            b'}' if bytes.get(i + 1) == Some(&b'}') => i += 2,
            b'{' => {
                i = find_interpolation_close(src, i + 1, base, i, depth)? + 1;
            }
            b'}' => {
                return Err(ParseError {
                    message:
                        "unmatched `}` in interpolated string (write `}}` for a literal brace)"
                            .to_string(),
                    span: Span::new(base + i, base + i + 1),
                })
            }
            _ => {
                let c = src[i..].chars().next().expect("scan is on a char boundary");
                i += c.len_utf8();
            }
        }
    }
    Err(ParseError {
        message: "unterminated interpolated string".to_string(),
        span: Span::new(base + start, base + start + 2),
    })
}
