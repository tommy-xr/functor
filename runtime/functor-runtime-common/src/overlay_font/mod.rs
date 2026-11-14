//! The headset overlay's typeface: Inter, regular and semibold, as one
//! multi-channel signed distance field atlas (`atlas.png`) and its metrics
//! (`metrics.rs`), both generated offline by `tools/overlay-font`. The
//! painter samples the field with screen-space derivatives, so text stays
//! sharp from arm's length to across the room; this module measures and
//! lays out lines for it.
//!
//! A line's style follows its copy: an all-caps line is a label, set
//! semibold and tracked; a line of figures (the pairing code) gets tabular
//! digits, tracked wide; anything else is regular. A character the font
//! lacks draws as a box with its own advance; whitespace draws as a space.
//!
//! `size` is a line's box height in metres, as the callers lay lines out:
//! capitals fill it from the top; descenders hang below it.

// Generated: it lists metrics the layout does not read (ascender, ...).
#[allow(dead_code)]
mod metrics {
    use super::Glyph;
    include!("metrics.rs");
}

use std::cell::RefCell;
use std::collections::HashMap;

pub use metrics::{ATLAS_HEIGHT, ATLAS_WIDTH, RANGE};
use metrics::{CAP_HEIGHT, REGULAR, REGULAR_KERNING, SEMIBOLD, SEMIBOLD_KERNING};

/// The atlas: RGB, top row first.
pub const ATLAS_PNG: &[u8] = include_bytes!("atlas.png");

/// Em per `size`: capitals a little taller than the old 8x8 font's.
const EM_PER_SIZE: f32 = 1.25;
/// From a line's top to its baseline, in `size`.
pub const BASELINE: f32 = CAP_HEIGHT * EM_PER_SIZE;
/// Letter-spacing, in em: caps labels, and the pairing code's figures.
const CAPS_TRACKING: f32 = 0.05;
const FIGURE_TRACKING: f32 = 0.12;
/// The ellipsis a cut line ends with.
const ELLIPSIS: char = '\u{2026}';
/// Where the generator put the tabular digits and the missing-glyph box.
const TABULAR: u32 = 0xE030;
const MISSING: u32 = 0;

pub struct Glyph {
    pub key: u32,
    /// Em.
    pub advance: f32,
    /// The cell's box in em from the pen on the baseline: left, bottom,
    /// right, top.
    pub plane: [f32; 4],
    /// Atlas pixels: x, y (down), w, h; empty for a space.
    pub cell: [u16; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Weight {
    Regular,
    Semibold,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Style {
    weight: Weight,
    tracking: f32,
    tabular: bool,
}

fn style(text: &str) -> Style {
    let (mut letters, mut lower, mut digits) = (false, false, false);
    for c in text.chars() {
        letters |= c.is_alphabetic();
        lower |= c.is_lowercase();
        digits |= c.is_ascii_digit();
    }
    match (letters, lower, digits) {
        (true, false, _) => Style { weight: Weight::Semibold, tracking: CAPS_TRACKING, tabular: false },
        (false, _, true) => Style { weight: Weight::Regular, tracking: FIGURE_TRACKING, tabular: true },
        _ => Style { weight: Weight::Regular, tracking: 0.0, tabular: false },
    }
}

fn glyph(weight: Weight, key: u32) -> &'static Glyph {
    let table: &'static [Glyph] = match weight {
        Weight::Regular => &REGULAR,
        Weight::Semibold => &SEMIBOLD,
    };
    let at = table.binary_search_by_key(&key, |g| g.key);
    &table[at.unwrap_or_else(|_| table.binary_search_by_key(&MISSING, |g| g.key).unwrap())]
}

/// Whether `c` draws as itself (not the missing-glyph box).
pub fn has_glyph(c: char) -> bool {
    c.is_whitespace() || REGULAR.binary_search_by_key(&(c as u32), |g| g.key).is_ok()
}

/// Pair kerning between `left` and `right`, in em.
fn kerning(weight: Weight, left: u32, right: u32) -> f32 {
    let table: &[(u32, i16)] = match weight {
        Weight::Regular => &REGULAR_KERNING,
        Weight::Semibold => &SEMIBOLD_KERNING,
    };
    if left > 0xFFFF || right > 0xFFFF {
        return 0.0;
    }
    let key = left << 16 | right;
    table
        .binary_search_by_key(&key, |&(k, _)| k)
        .map_or(0.0, |i| table[i].1 as f32 / 2048.0)
}

/// Walks `text` as one line: each glyph with its pen position in em, and
/// the line's advance in em.
fn layout(text: &str, mut each: impl FnMut(&'static Glyph, f32)) -> f32 {
    let style = style(text);
    let mut pen = 0.0;
    let mut previous: Option<u32> = None;
    for c in text.chars() {
        let key = match c {
            c if c.is_whitespace() => ' ' as u32,
            '0'..='9' if style.tabular => TABULAR + (c as u32 - '0' as u32),
            c => c as u32,
        };
        if let Some(left) = previous {
            pen += style.tracking;
            if !style.tabular {
                pen += kerning(style.weight, left, key);
            }
        }
        let g = glyph(style.weight, key);
        each(g, pen);
        pen += g.advance;
        previous = Some(key);
    }
    pen
}

/// The em of a line `size` tall.
pub fn em(size: f32) -> f32 {
    size * EM_PER_SIZE
}

/// `text`'s advance as one line `size` tall, in metres.
pub fn width(text: &str, size: f32) -> f32 {
    layout(text, |_, _| {}) * em(size)
}

/// Each glyph of `text` as one line `size` tall with its top-left at
/// `at` (metres, y up), with its pen on the baseline.
pub fn place(text: &str, size: f32, at: [f32; 2], mut each: impl FnMut(&'static Glyph, [f32; 2])) {
    let em = em(size);
    let baseline = at[1] - BASELINE * size;
    layout(text, |g, pen| each(g, [at[0] + pen * em, baseline]));
}

/// `text` as one line `size` tall at most `max` metres wide, cut with an
/// ellipsis if it is longer.
pub fn fit(text: &str, size: f32, max: f32) -> String {
    let fitted = remembered(text, size, max, usize::MAX, || vec![fit_uncached(text, size, max)]);
    fitted.into_iter().next().unwrap_or_default()
}

fn fit_uncached(text: &str, size: f32, max: f32) -> String {
    if width(text, size) <= max {
        return text.to_string();
    }
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let cut = |n: usize| format!("{}{ELLIPSIS}", text[..ends[n]].trim_end());
    cut(longest(ends.len() - 1, |n| width(&cut(n), size) <= max))
}

/// The largest `n` in `0..=most` that `fits` (0 if none does), for a test
/// that holds up to some `n` and fails after.
fn longest(most: usize, fits: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, most);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// `text` in lines `size` tall and at most `max` metres wide, broken at
/// spaces where it can be and inside a word too long for a line; at most
/// `lines` of them, the last cut with an ellipsis.
pub fn wrap(text: &str, size: f32, max: f32, lines: usize) -> Vec<String> {
    remembered(text, size, max, lines, || wrap_uncached(text, size, max, lines))
}

/// `make`'s lines for these arguments, remembered: the panel lays its text
/// out every frame it shows, and the text rarely changes.
fn remembered(
    text: &str,
    size: f32,
    max: f32,
    lines: usize,
    make: impl FnOnce() -> Vec<String>,
) -> Vec<String> {
    type Key = (String, u32, u32, usize);
    thread_local! {
        static MEMO: RefCell<HashMap<Key, Vec<String>>> = RefCell::default();
    }
    let key = (text.to_string(), size.to_bits(), max.to_bits(), lines);
    if let Some(hit) = MEMO.with_borrow(|memo| memo.get(&key).cloned()) {
        return hit;
    }
    let out = make();
    MEMO.with_borrow_mut(|memo| {
        if memo.len() >= 512 {
            memo.clear();
        }
        memo.insert(key, out.clone());
    });
    out
}

fn wrap_uncached(text: &str, size: f32, max: f32, lines: usize) -> Vec<String> {
    let mut words: std::collections::VecDeque<String> =
        text.split_whitespace().map(String::from).collect();
    let mut out = Vec::new();
    while !words.is_empty() && out.len() < lines {
        if out.len() + 1 == lines {
            let rest: Vec<String> = words.drain(..).collect();
            out.push(fit(&rest.join(" "), size, max));
            break;
        }
        let mut line = words.pop_front().unwrap();
        let ends: Vec<usize> = line.char_indices().map(|(i, _)| i).skip(1).collect();
        if !ends.is_empty() && width(&line, size) > max {
            // A word longer than a line: split it where it overflows, after
            // at least one character.
            let n = longest(ends.len(), |n| n == 0 || width(&line[..ends[n - 1]], size) <= max);
            let split = ends[n.max(1) - 1];
            words.push_front(line.split_off(split));
        } else {
            while let Some(next) = words.front() {
                let candidate = format!("{line} {next}");
                if width(&candidate, size) > max {
                    break;
                }
                line = candidate;
                words.pop_front();
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: f32 = 0.0135;

    #[test]
    fn wrap_breaks_at_spaces_within_the_width() {
        let text = "Hold the menu button for a second to pair with the Functor app on your computer.";
        let max = 0.2;
        let lines = wrap(text, SIZE, max, 10);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines.iter().all(|l| width(l, SIZE) <= max), "{lines:?}");
        assert_eq!(lines.join(" "), text, "nothing lost or cut");
        // Each line is as long as it can be: the next word would not fit.
        for pair in lines.windows(2) {
            let next = pair[1].split(' ').next().unwrap();
            assert!(width(&format!("{} {next}", pair[0]), SIZE) > max, "{pair:?}");
        }
        assert!(wrap(text, SIZE, max, 0).is_empty());
        assert!(wrap("   ", SIZE, max, 3).is_empty());
    }

    #[test]
    fn wrap_splits_a_long_token_and_cuts_the_last_line() {
        let token = "x".repeat(200);
        let lines = wrap(&token, SIZE, 0.1, 3);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| width(l, SIZE) <= 0.1), "{lines:?}");
        assert!(lines[2].ends_with(ELLIPSIS));
        // The first lines are full: one more character would overflow.
        assert!(width(&format!("{}x", lines[0]), SIZE) > 0.1);
        let words = wrap(&"word ".repeat(100), SIZE, 0.1, 2);
        assert_eq!(words.len(), 2);
        assert!(words[1].ends_with(ELLIPSIS) && width(&words[1], SIZE) <= 0.1, "{words:?}");
        // A non-ASCII token splits on character boundaries.
        let accents = wrap(&"é".repeat(300), SIZE, 0.1, 4);
        assert!(accents.iter().all(|l| width(l, SIZE) <= 0.1));
    }

    #[test]
    fn fit_keeps_what_fits_and_cuts_the_rest_with_an_ellipsis() {
        assert_eq!(fit("vr-lab", SIZE, 0.2), "vr-lab");
        let cut = fit("A very long project name that cannot fit", SIZE, 0.1);
        assert!(cut.ends_with(ELLIPSIS) && width(&cut, SIZE) <= 0.1, "{cut}");
        assert!(!cut.contains(" \u{2026}"), "no space before the ellipsis: {cut}");
        assert_eq!(fit("", SIZE, 0.0), "");
    }

    #[test]
    fn pairs_kern_and_the_code_keeps_tabular_figures() {
        let apart = |a: &str, b: &str| width(a, SIZE) + width(b, SIZE);
        for pair in ["AV", "To", "Ty", "LT"] {
            let (a, b) = pair.split_at(1);
            assert!(width(pair, SIZE) < apart(a, b), "{pair} is not kerned");
        }
        // Figures alone are tabular: every digit has one advance.
        assert!((width("111111", SIZE) - width("888888", SIZE)).abs() < 1e-6);
        // In words they are proportional.
        assert!(width("1 file", SIZE) < width("8 file", SIZE));
    }

    #[test]
    fn caps_are_semibold_and_tracked() {
        let (caps, plain) = (style("PAIRED"), style("Paired"));
        assert_eq!((caps.weight, plain.weight), (Weight::Semibold, Weight::Regular));
        assert!(caps.tracking > 0.0 && plain.tracking == 0.0);
        assert!(style("482 913").tabular && !style("Sending in 3").tabular);
    }

    #[test]
    fn a_missing_glyph_draws_a_box_and_whitespace_a_space() {
        assert!(has_glyph('é') && has_glyph('\u{2026}') && has_glyph('\t'));
        assert!(!has_glyph('\u{1F600}') && !has_glyph('\u{4E2D}'));
        let mut keys = Vec::new();
        place("a\u{1F600}b\tc", SIZE, [0.0, 0.0], |g, _| keys.push(g.key));
        assert_eq!(keys, ['a' as u32, MISSING, 'b' as u32, ' ' as u32, 'c' as u32]);
        let boxed = glyph(Weight::Regular, MISSING);
        assert!(boxed.advance > 0.0 && boxed.cell[2] > 0, "the box draws");
    }

    #[test]
    fn every_cell_lies_inside_the_atlas() {
        for g in REGULAR.iter().chain(SEMIBOLD.iter()) {
            let [x, y, w, h] = g.cell.map(u32::from);
            assert!(x + w <= ATLAS_WIDTH && y + h <= ATLAS_HEIGHT, "U+{:04X}", g.key);
        }
        let atlas = image::load_from_memory(ATLAS_PNG).unwrap();
        assert_eq!((atlas.width(), atlas.height()), (ATLAS_WIDTH, ATLAS_HEIGHT));
    }

    #[test]
    fn text_is_no_smaller_than_the_old_8px_font() {
        // The old font's capitals were 7/8 of `size`, its x-height 5/8.
        assert!(BASELINE >= 0.875);
        assert!(metrics::X_HEIGHT * EM_PER_SIZE >= 0.625);
    }
}
