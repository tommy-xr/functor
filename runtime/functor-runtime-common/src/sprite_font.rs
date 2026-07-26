//! The built-in 8x8 bitmap font behind `Sprite.text`.
//!
//! `Sprite.text` must work with no asset, on every target, so the font ships as
//! a table of glyph bitmaps compiled into the runtime rather than a PNG loaded
//! through the asset cache. The table is plain text (one row of 8 pixels per
//! byte), so it diffs, reviews, and is inspectable like the rest of the engine.
//!
//! # Provenance
//!
//! The glyph bitmaps are the printable-ASCII subset (U+0020..U+007E) of
//! **unscii-8** by Viznut (<http://viznut.fi/unscii/>), which is in the
//! **public domain**. Only the separate `unscii-16-full` variant carries a GPL
//! obligation (it embeds GNU Unifont); the 8x8 variant used here does not.
//! U+007F is included as a blank cell so the atlas is a full 16x6 grid.
//!
//! # Layout
//!
//! Every glyph is an 8x8 cell whose rightmost column and (for characters
//! without a descender) bottom row are blank, so adjacent cells carry their own
//! letter spacing and line gap. Cells are packed row-major into a 16x6 atlas
//! (128x48 px), cell index `c - 0x20`, uploaded top-row-first like a file
//! texture. Because the cell is square and self-spacing, one text `size` (the
//! cell height in world units) is also the per-character advance -- the font is
//! monospace.

use crate::texture::{PixelFormat, TextureData};

/// Side length of one glyph cell, in atlas pixels.
pub const GLYPH_PIXELS: u32 = 8;
/// Glyph cells per atlas row.
pub const ATLAS_COLUMNS: u32 = 16;
/// Rows of glyph cells in the atlas.
pub const ATLAS_ROWS: u32 = 6;
/// Codepoint of the first cell in the atlas.
const FIRST_CHAR: u32 = 0x20;

/// Atlas width in pixels.
pub const ATLAS_WIDTH: u32 = ATLAS_COLUMNS * GLYPH_PIXELS;
/// Atlas height in pixels.
pub const ATLAS_HEIGHT: u32 = ATLAS_ROWS * GLYPH_PIXELS;

/// One byte per pixel row, most significant bit leftmost.
#[rustfmt::skip]
const GLYPHS: [[u8; GLYPH_PIXELS as usize]; (ATLAS_COLUMNS * ATLAS_ROWS) as usize] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // U+0020 ' '
    [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00], // U+0021 '!'
    [0x66, 0x66, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00], // U+0022 '"'
    [0x6C, 0x6C, 0xFE, 0x6C, 0xFE, 0x6C, 0x6C, 0x00], // U+0023 '#'
    [0x18, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x18, 0x00], // U+0024 '$'
    [0x00, 0xC6, 0xCC, 0x18, 0x30, 0x66, 0xC6, 0x00], // U+0025 '%'
    [0x38, 0x6C, 0x38, 0x76, 0xDC, 0xCC, 0x76, 0x00], // U+0026 '&'
    [0x18, 0x18, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00], // U+0027 "'"
    [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00], // U+0028 '('
    [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00], // U+0029 ')'
    [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00], // U+002A '*'
    [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00], // U+002B '+'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30], // U+002C ','
    [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00], // U+002D '-'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00], // U+002E '.'
    [0x03, 0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0x00], // U+002F '/'
    [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00], // U+0030 '0'
    [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00], // U+0031 '1'
    [0x3C, 0x66, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00], // U+0032 '2'
    [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00], // U+0033 '3'
    [0x1C, 0x3C, 0x6C, 0xCC, 0xFE, 0x0C, 0x0C, 0x00], // U+0034 '4'
    [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00], // U+0035 '5'
    [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00], // U+0036 '6'
    [0x7E, 0x06, 0x06, 0x0C, 0x18, 0x18, 0x18, 0x00], // U+0037 '7'
    [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00], // U+0038 '8'
    [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0x00], // U+0039 '9'
    [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00], // U+003A ':'
    [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x30], // U+003B ';'
    [0x0C, 0x18, 0x30, 0x60, 0x30, 0x18, 0x0C, 0x00], // U+003C '<'
    [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00], // U+003D '='
    [0x60, 0x30, 0x18, 0x0C, 0x18, 0x30, 0x60, 0x00], // U+003E '>'
    [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x00, 0x18, 0x00], // U+003F '?'
    [0x7C, 0xC6, 0xDE, 0xDE, 0xDE, 0xC0, 0x7C, 0x00], // U+0040 '@'
    [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00], // U+0041 'A'
    [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00], // U+0042 'B'
    [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00], // U+0043 'C'
    [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00], // U+0044 'D'
    [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x7E, 0x00], // U+0045 'E'
    [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x00], // U+0046 'F'
    [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00], // U+0047 'G'
    [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00], // U+0048 'H'
    [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00], // U+0049 'I'
    [0x06, 0x06, 0x06, 0x06, 0x06, 0x66, 0x3C, 0x00], // U+004A 'J'
    [0xC6, 0xCC, 0xD8, 0xF0, 0xD8, 0xCC, 0xC6, 0x00], // U+004B 'K'
    [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00], // U+004C 'L'
    [0xC6, 0xEE, 0xFE, 0xD6, 0xC6, 0xC6, 0xC6, 0x00], // U+004D 'M'
    [0xC6, 0xE6, 0xF6, 0xDE, 0xCE, 0xC6, 0xC6, 0x00], // U+004E 'N'
    [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00], // U+004F 'O'
    [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00], // U+0050 'P'
    [0x3C, 0x66, 0x66, 0x66, 0x66, 0x6C, 0x36, 0x00], // U+0051 'Q'
    [0x7C, 0x66, 0x66, 0x7C, 0x6C, 0x66, 0x66, 0x00], // U+0052 'R'
    [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00], // U+0053 'S'
    [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00], // U+0054 'T'
    [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00], // U+0055 'U'
    [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00], // U+0056 'V'
    [0xC6, 0xC6, 0xC6, 0xD6, 0xFE, 0xEE, 0xC6, 0x00], // U+0057 'W'
    [0xC3, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0xC3, 0x00], // U+0058 'X'
    [0xC3, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x18, 0x00], // U+0059 'Y'
    [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00], // U+005A 'Z'
    [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00], // U+005B '['
    [0xC0, 0x60, 0x30, 0x18, 0x0C, 0x06, 0x03, 0x00], // U+005C '\\'
    [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00], // U+005D ']'
    [0x10, 0x38, 0x6C, 0xC6, 0x00, 0x00, 0x00, 0x00], // U+005E '^'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF], // U+005F '_'
    [0x18, 0x0C, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00], // U+0060 '`'
    [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00], // U+0061 'a'
    [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00], // U+0062 'b'
    [0x00, 0x00, 0x3C, 0x60, 0x60, 0x60, 0x3C, 0x00], // U+0063 'c'
    [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00], // U+0064 'd'
    [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00], // U+0065 'e'
    [0x1C, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x00], // U+0066 'f'
    [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x7C], // U+0067 'g'
    [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00], // U+0068 'h'
    [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x1E, 0x00], // U+0069 'i'
    [0x0C, 0x00, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x78], // U+006A 'j'
    [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0x00], // U+006B 'k'
    [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x1E, 0x00], // U+006C 'l'
    [0x00, 0x00, 0xCC, 0xFE, 0xD6, 0xD6, 0xC6, 0x00], // U+006D 'm'
    [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00], // U+006E 'n'
    [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00], // U+006F 'o'
    [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60], // U+0070 'p'
    [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06], // U+0071 'q'
    [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0x00], // U+0072 'r'
    [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x00], // U+0073 's'
    [0x30, 0x30, 0x7E, 0x30, 0x30, 0x30, 0x1E, 0x00], // U+0074 't'
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00], // U+0075 'u'
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00], // U+0076 'v'
    [0x00, 0x00, 0xC6, 0xC6, 0xD6, 0x7C, 0x6C, 0x00], // U+0077 'w'
    [0x00, 0x00, 0xC6, 0x6C, 0x38, 0x6C, 0xC6, 0x00], // U+0078 'x'
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C], // U+0079 'y'
    [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00], // U+007A 'z'
    [0x0E, 0x18, 0x18, 0x70, 0x18, 0x18, 0x0E, 0x00], // U+007B '{'
    [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00], // U+007C '|'
    [0x70, 0x18, 0x18, 0x0E, 0x18, 0x18, 0x70, 0x00], // U+007D '}'
    [0x76, 0xDC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // U+007E '~'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // U+007F DEL (blank)
];

/// The source rectangle of `character`'s cell as whole atlas pixels
/// `(x, y, width, height)`, with the top-left image origin `Sprite.region`
/// uses.
///
/// Returns `None` for anything that draws nothing: a space, a character outside
/// printable ASCII, and any cell whose bitmap is empty. Callers skip the quad
/// entirely rather than drawing a fully transparent one -- but the character
/// still advances the pen, so unsupported text reads as gaps instead of
/// shifting the rest of the line.
pub fn glyph_cell(character: char) -> Option<(f32, f32, f32, f32)> {
    let index = (character as u32).checked_sub(FIRST_CHAR)? as usize;
    let bitmap = GLYPHS.get(index)?;
    if bitmap.iter().all(|row| *row == 0) {
        return None;
    }
    let column = index as u32 % ATLAS_COLUMNS;
    let row = index as u32 / ATLAS_COLUMNS;
    Some((
        (column * GLYPH_PIXELS) as f32,
        (row * GLYPH_PIXELS) as f32,
        GLYPH_PIXELS as f32,
        GLYPH_PIXELS as f32,
    ))
}

/// The lines of `text`: `\n` starts a new line, and a trailing newline yields a
/// final empty line, so `"a\n"` is two lines. A `\r` immediately before the
/// newline is dropped, so CRLF text does not gain a phantom blank cell at the
/// end of every line.
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

/// How many character advances wide one LINE is.
///
/// The font is monospace, so this is simply the character count -- but it is the
/// one place that knows so. A future proportional font changes the metric here
/// and stays consistent everywhere.
pub fn advance_count(line: &str) -> f64 {
    line.chars().count() as f64
}

/// The size of `text` in glyph cells as `(columns, rows)`: the widest line's
/// character count, and the number of lines (always at least one, so the empty
/// string measures one line tall and zero wide).
///
/// Scaled by a text size this is exactly `Sprite.measure`'s answer, and
/// `Sprite.text`'s glyph layout lays out against the same numbers -- so
/// measurement and rendering cannot drift apart.
pub fn measure_cells(text: &str) -> (f64, f64) {
    let mut columns = 0.0f64;
    let mut rows = 0.0f64;
    for line in lines(text) {
        columns = columns.max(advance_count(line));
        rows += 1.0;
    }
    (columns, rows)
}

/// Expand the glyph table into an RGBA atlas: white where a glyph pixel is set,
/// transparent black elsewhere, so a tint multiply colors the text and the
/// alpha blend leaves the cell background alone.
pub fn atlas_texture_data() -> TextureData {
    let mut bytes = vec![0u8; (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize];
    for (index, bitmap) in GLYPHS.iter().enumerate() {
        let cell_x = index as u32 % ATLAS_COLUMNS * GLYPH_PIXELS;
        let cell_y = index as u32 / ATLAS_COLUMNS * GLYPH_PIXELS;
        for (row, pixels) in bitmap.iter().enumerate() {
            for column in 0..GLYPH_PIXELS {
                if pixels >> (GLYPH_PIXELS - 1 - column) & 1 == 0 {
                    continue;
                }
                let x = cell_x + column;
                let y = cell_y + row as u32;
                let offset = ((y * ATLAS_WIDTH + x) * 4) as usize;
                bytes[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    TextureData {
        bytes,
        width: ATLAS_WIDTH,
        height: ATLAS_HEIGHT,
        format: PixelFormat::RGBA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_printable_ascii_except_space_has_a_glyph() {
        // Guards the generated table against a truncated or misaligned paste: a
        // shifted table still compiles and still renders *something*, so the
        // only cheap defense is that the alphabet is dense and space is not.
        for character in ' '..='~' {
            let cell = glyph_cell(character);
            if character == ' ' {
                assert!(cell.is_none(), "space must draw nothing");
            } else {
                assert!(cell.is_some(), "{character:?} has no glyph");
            }
        }
    }

    #[test]
    fn glyph_cells_are_whole_pixels_inside_the_atlas() {
        for character in '!'..='~' {
            let (x, y, width, height) = glyph_cell(character).expect("glyph");
            assert_eq!((width, height), (8.0, 8.0));
            assert_eq!(x.fract(), 0.0);
            assert_eq!(y.fract(), 0.0);
            assert!(x + width <= ATLAS_WIDTH as f32);
            assert!(y + height <= ATLAS_HEIGHT as f32);
        }
    }

    #[test]
    fn glyph_cells_are_indexed_from_the_first_codepoint() {
        // 'A' is U+0041, cell 33 => column 1, row 2 of a 16-wide atlas.
        assert_eq!(glyph_cell('A'), Some((8.0, 16.0, 8.0, 8.0)));
        assert_eq!(glyph_cell('!'), Some((8.0, 0.0, 8.0, 8.0)));
    }

    #[test]
    fn characters_outside_printable_ascii_draw_nothing() {
        for character in ['\n', '\t', '\u{0}', 'é', '★', '\u{7f}'] {
            assert_eq!(glyph_cell(character), None, "{character:?}");
        }
    }

    #[test]
    fn every_character_advances_the_pen() {
        // Including ones that draw nothing: unsupported text must leave gaps,
        // not slide the rest of the line left.
        assert_eq!(advance_count(""), 0.0);
        assert_eq!(advance_count("SCORE"), 5.0);
        assert_eq!(advance_count("A B"), 3.0);
        assert_eq!(advance_count("é★"), 2.0);
    }

    #[test]
    fn newlines_break_lines_and_the_widest_line_sets_the_width() {
        assert_eq!(measure_cells(""), (0.0, 1.0));
        assert_eq!(measure_cells("SCORE"), (5.0, 1.0));
        assert_eq!(measure_cells("HI\nSCORE"), (5.0, 2.0));
        assert_eq!(measure_cells("SCORE\nHI"), (5.0, 2.0));
        // A trailing newline is a real (empty) final line.
        assert_eq!(measure_cells("HI\n"), (2.0, 2.0));
        // ...and an empty line in the middle keeps its row.
        assert_eq!(measure_cells("HI\n\nHO"), (2.0, 3.0));
        assert_eq!(measure_cells("\n"), (0.0, 2.0));
    }

    #[test]
    fn a_carriage_return_before_a_newline_does_not_widen_a_line() {
        // CRLF text would otherwise measure one cell wider per line than it
        // renders -- a silent layout bug with no visible cause.
        assert_eq!(measure_cells("HI\r\nHO"), (2.0, 2.0));
        assert_eq!(lines("HI\r\nHO").collect::<Vec<_>>(), vec!["HI", "HO"]);
        // A `\r` anywhere else is just an unsupported character.
        assert_eq!(measure_cells("H\rI"), (3.0, 1.0));
    }

    #[test]
    fn only_the_full_width_glyphs_paint_the_spacing_column() {
        // Nearly every glyph leaves the rightmost column blank, which is what
        // makes the advance equal to the cell size and so makes `size` alone a
        // complete metric. Exactly six glyphs are deliberately full width: `_`
        // must reach the cell edge or underlines break into dashes, and the
        // diagonals/asterisk are drawn edge to edge in unscii. Consequence,
        // documented rather than papered over: `XX` and `//` touch, as they do
        // in any 8px-cell terminal font. Pinned so a font swap has to state
        // whether it changes this.
        let full_width: String = ('!'..='~')
            .filter(|character| {
                let index = (*character as u32 - FIRST_CHAR) as usize;
                GLYPHS[index].iter().fold(0u8, |set, row| set | row) & 1 != 0
            })
            .collect();
        assert_eq!(full_width, "*/XY\\_");
    }

    #[test]
    fn the_atlas_is_white_on_transparent_at_the_expected_cells() {
        let atlas = atlas_texture_data();
        assert_eq!(atlas.width, 128);
        assert_eq!(atlas.height, 48);
        assert_eq!(atlas.bytes.len(), (128 * 48 * 4) as usize);

        let pixel = |x: u32, y: u32| {
            let offset = ((y * ATLAS_WIDTH + x) * 4) as usize;
            [
                atlas.bytes[offset],
                atlas.bytes[offset + 1],
                atlas.bytes[offset + 2],
                atlas.bytes[offset + 3],
            ]
        };

        // Every pixel of the space cell (cell 0, top-left) is transparent.
        for y in 0..GLYPH_PIXELS {
            for x in 0..GLYPH_PIXELS {
                assert_eq!(pixel(x, y), [0, 0, 0, 0], "space cell at {x},{y}");
            }
        }

        // '!' is cell 1: 0x18 on its top row => the two center columns are set
        // white and the rest of the row transparent.
        assert_eq!(pixel(8 + 3, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(8 + 4, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(8 + 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(8 + 7, 0), [0, 0, 0, 0]);
    }
}
