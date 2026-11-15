//! The GL-free half of `Scene.text`: a string laid out as glyph quads of the
//! Inter MSDF atlas (`overlay_font`) in the text's own plane, cached by
//! content so a label that doesn't change is laid out once.
//!
//! Units: one line box is 1 tall, so `Scene.scale(s)` makes lines `s` tall.
//! X runs along the line, Y up; the text reads from +Z.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::overlay_font;

/// How a `Scene.text` leaf is set: a label (`width: None`) centres each
/// line and breaks only at `\n`; a block wraps to `width`, its lines
/// left-aligned.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextLayout {
    /// Wrap at this width (line boxes).
    pub width: Option<f32>,
    /// At most this many lines; the last is cut with an ellipsis.
    pub lines: u32,
}

impl TextLayout {
    /// `Scene.text`.
    pub const LABEL: TextLayout = TextLayout {
        width: None,
        lines: u32::MAX,
    };
}

/// From one line's top to the next's, in line boxes: room for descenders.
pub const LINE_PITCH: f32 = 1.25;

/// One glyph: its quad in the text plane and its atlas cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    /// Left, bottom, right, top.
    pub rect: [f32; 4],
    /// Atlas UV: left, top, right, bottom (V down, as the atlas rows).
    pub uv: [f32; 4],
}

/// The quads of `text` set by `layout`, centred on the origin (a block's
/// middle; each line aligned within the block's width). Remembered by
/// content, so a label that doesn't change lays out once.
pub fn layout(text: &str, layout: TextLayout) -> Arc<[GlyphQuad]> {
    // Keyed by a hash, the text kept to confirm a hit: a lookup — once per
    // label per pass — allocates nothing.
    type Entry = (String, TextLayout, Arc<[GlyphQuad]>);
    thread_local! {
        static MEMO: RefCell<HashMap<u64, Entry>> = RefCell::default();
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = crate::host_registry::FxHasher::default();
        text.hash(&mut hasher);
        (layout.width.map(f32::to_bits), layout.lines).hash(&mut hasher);
        hasher.finish()
    };
    let hit = MEMO.with_borrow(|memo| {
        memo.get(&key)
            .filter(|(t, l, _)| t == text && *l == layout)
            .map(|(_, _, quads)| quads.clone())
    });
    if let Some(hit) = hit {
        return hit;
    }
    let quads = layout_uncached(text, layout);
    MEMO.with_borrow_mut(|memo| {
        // Far above a scene's distinct labels; past it, they lay out again.
        if memo.len() >= 16384 {
            memo.clear();
        }
        memo.insert(key, (text.to_string(), layout, quads.clone()));
    });
    quads
}

/// `text`'s lines: each `\n`-separated paragraph wrapped to the width,
/// at most `lines` in all (the last cut with an ellipsis).
fn lines_of(text: &str, layout: TextLayout) -> Vec<String> {
    let style = overlay_font::Style::REGULAR;
    let budget = layout.lines as usize;
    let mut out: Vec<String> = Vec::new();
    let paragraphs: Vec<&str> = text.split('\n').map(|p| p.strip_suffix('\r').unwrap_or(p)).collect();
    for (k, paragraph) in paragraphs.iter().enumerate() {
        let left = budget - out.len();
        if left == 0 {
            break;
        }
        let more = k + 1 < paragraphs.len();
        let lines = match layout.width {
            Some(width) => {
                // An empty paragraph is a blank line, as `\n\n` reads.
                let wrapped = overlay_font::wrap_in(paragraph, 1.0, style, width, left);
                if wrapped.is_empty() { vec![String::new()] } else { wrapped }
            }
            None => vec![paragraph.to_string()],
        };
        out.extend(lines);
        // Paragraphs past the budget: mark the cut on the last line kept.
        if more && out.len() >= budget {
            if let Some(last) = out.last_mut().filter(|last| !last.ends_with('\u{2026}')) {
                let width = layout.width.unwrap_or(f32::INFINITY);
                *last = overlay_font::fit_in(&format!("{last}\u{2026}"), 1.0, style, width);
            }
            break;
        }
    }
    out.truncate(budget);
    out
}

fn layout_uncached(text: &str, layout: TextLayout) -> Arc<[GlyphQuad]> {
    let style = overlay_font::Style::REGULAR;
    let lines = lines_of(text, layout);
    let widths: Vec<f32> = lines.iter().map(|l| overlay_font::width_in(l, 1.0, style)).collect();
    let block = layout
        .width
        .unwrap_or_else(|| widths.iter().cloned().fold(0.0, f32::max));
    let height = 1.0 + LINE_PITCH * (lines.len().max(1) - 1) as f32;
    let (aw, ah) = (overlay_font::ATLAS_WIDTH as f32, overlay_font::ATLAS_HEIGHT as f32);
    let em = overlay_font::em(1.0);
    let mut quads = Vec::new();
    for (row, (line, width)) in lines.iter().zip(&widths).enumerate() {
        let left = match layout.width {
            Some(_) => -block / 2.0,
            None => -width / 2.0,
        };
        let top = height / 2.0 - LINE_PITCH * row as f32;
        overlay_font::place_in(line, 1.0, style, [left, top], |glyph, [x, baseline]| {
            let [cx, cy, cw, ch] = glyph.cell.map(f32::from);
            if cw == 0.0 || ch == 0.0 {
                return;
            }
            let [l, b, r, t] = glyph.plane;
            quads.push(GlyphQuad {
                rect: [x + l * em, baseline + b * em, x + r * em, baseline + t * em],
                uv: [cx / aw, cy / ah, (cx + cw) / aw, (cy + ch) / ah],
            });
        });
    }
    quads.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distance field's range around an outline, in line boxes.
    const PAD: f32 = 0.16;

    fn bounds(quads: &[GlyphQuad]) -> [f32; 4] {
        quads.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, q| {
            [b[0].min(q.rect[0]), b[1].min(q.rect[1]), b[2].max(q.rect[2]), b[3].max(q.rect[3])]
        })
    }

    #[test]
    fn a_label_is_centred_one_line_box_tall() {
        let quads = layout("Hello", TextLayout::LABEL);
        assert_eq!(quads.len(), 5);
        let [l, b, r, t] = bounds(&quads);
        assert!((l + r).abs() < 0.1, "centred: {l} .. {r}");
        // Capitals fill the line box from its top (0.5); quads add the
        // field's padding (PAD) around each outline.
        assert!(t <= 0.5 + PAD && t > 0.3, "{t}");
        assert!(b >= -0.5 - 0.25 - PAD, "{b}");
        // Every UV sits inside the atlas.
        assert!(quads.iter().all(|q| q.uv.iter().all(|v| (0.0..=1.0).contains(v))));
    }

    #[test]
    fn spaces_draw_nothing_and_unknown_characters_draw_a_box() {
        assert_eq!(layout("a b", TextLayout::LABEL).len(), 2);
        assert_eq!(layout("\u{4e2d}", TextLayout::LABEL).len(), 1, "the missing-glyph box");
        assert!(layout("", TextLayout::LABEL).is_empty());
    }

    #[test]
    fn a_block_wraps_within_its_width_and_aligns_each_line() {
        let text = "the quick brown fox jumps over the lazy dog";
        let set = TextLayout { width: Some(6.0), lines: 9 };
        let left = layout(text, set);
        let [l, b, r, t] = bounds(&left);
        assert!(l >= -3.0 - PAD && r <= 3.0 + PAD, "inside the width: {l} .. {r}");
        assert!(t - b > 1.5, "more than one line: {b} .. {t}");
        // Left-aligned: the first glyph of every line starts at the block's edge.
        assert!((l + 3.0).abs() < PAD, "flush left: {l}");
        // Two lines at most: the second is cut with an ellipsis.
        let short = layout(text, TextLayout { lines: 2, ..set });
        assert!(short.len() < left.len());
    }

    #[test]
    fn a_newline_breaks_the_line() {
        let two = layout("ab\ncd", TextLayout::LABEL);
        let [_, b, _, t] = bounds(&two);
        assert!(t - b > LINE_PITCH, "two rows: {b} .. {t}");
        // Past the line budget: the last kept line ends with an ellipsis.
        let cut = lines_of("one\ntwo\nthree", TextLayout { lines: 2, ..TextLayout::LABEL });
        assert_eq!(cut.len(), 2);
        assert!(cut[1].ends_with('\u{2026}'), "{cut:?}");
        // A blank paragraph is a blank line in a block.
        let block = TextLayout { width: Some(10.0), lines: 9 };
        assert_eq!(lines_of("a\n\nb", block), ["a", "", "b"]);
        // Windows line endings break the same way.
        assert_eq!(lines_of("a\r\nb", TextLayout::LABEL), ["a", "b"]);
    }

    #[test]
    fn the_same_text_lays_out_once() {
        let a = layout("cached", TextLayout::LABEL);
        let b = layout("cached", TextLayout::LABEL);
        assert!(Arc::ptr_eq(&a, &b));
    }
}
