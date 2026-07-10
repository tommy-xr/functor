use std::fmt;

/// Half-open byte range into the source text. Byte offsets are the single
/// source of truth; line/col are derived on demand by [`line_col`] so AST
/// nodes stay small and slicing `&src[span.start..span.end]` always yields
/// the exact source text of a node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The span from the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }
}

// Compact `start..end` form keeps the pretty-Debug AST (and the committed
// `.ast` goldens) readable.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// 1-based (line, column) of a byte offset. Columns count characters from the
/// start of the line.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, c) in src.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
