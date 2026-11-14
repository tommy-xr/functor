//! The headset overlay's typeface: Inter, regular and semibold, as one
//! multi-channel signed distance field atlas (`atlas.png`) and its metrics
//! (`metrics.rs`), both generated offline by `tools/overlay-font`. The
//! painter samples the field with screen-space derivatives, so text stays
//! sharp from arm's length to across the room; this module measures and
//! lays out lines for it.
//!
//! A line is set in a [`Style`]. By default (the overlay's) it follows the
//! copy: an all-caps line is a label, set semibold and tracked; a line of
//! figures (the pairing code) gets tabular digits, tracked wide; anything
//! else is regular. Callers whose copy says nothing of its role (game text
//! like "HP 100") pass a style instead, through the `_in` functions. A
//! character the font lacks draws as a box with its own advance; whitespace
//! draws as a space.
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

/// The weight correction's exponent (see [`msdf_glsl`]).
const GAMMA: f32 = 1.6;

/// The field's coverage in GLSL, shared by every painter of this font
/// (prepend it to a fragment shader). It defines:
///
/// - `highp vec2 functorMsdfTexels(highp vec2 uv)`: atlas texels per screen
///   pixel. Call it outside any branch: derivatives need uniform control
///   flow.
/// - `float functorMsdfCoverage(sampler2D atlas, highp vec2 uv, highp vec2
///   texels, vec3 linearColor, bool linearBlend)`: the median of the three
///   channels is the distance to the outline, scaled to screen pixels so the
///   edge is one pixel wide at any distance or angle (at least one, so small
///   text softens rather than aliases). `linearBlend` when the target blends
///   in linear light (an sRGB attachment): that fattens light strokes on
///   dark and thins dark ones on light, and the coverage is curved by the
///   text colour's luminance to restore the weight a gamma-space blend gives.
pub fn msdf_glsl() -> String {
    format!(
        r#"
    const float FUNCTOR_MSDF_RANGE = {range:?};
    const highp vec2 FUNCTOR_MSDF_ATLAS = vec2({w:?}, {h:?});
    const float FUNCTOR_MSDF_GAMMA = {GAMMA:?};
    highp vec2 functorMsdfTexels(highp vec2 uv) {{
        return fwidth(uv) * FUNCTOR_MSDF_ATLAS;
    }}
    float functorMsdfCoverage(sampler2D atlas, highp vec2 uv, highp vec2 texels, vec3 linearColor, bool linearBlend) {{
        vec3 s = textureLod(atlas, uv, 0.0).rgb;
        float field = max(min(s.r, s.g), min(max(s.r, s.g), s.b)) - 0.5;
        float px = 0.5 * FUNCTOR_MSDF_RANGE * (1.0 / max(texels.x, 1e-6) + 1.0 / max(texels.y, 1e-6));
        float coverage = clamp(field * max(px, 1.0) + 0.5, 0.0, 1.0);
        if (linearBlend) {{
            float light = smoothstep(0.02, 0.2, dot(linearColor, vec3(0.2126, 0.7152, 0.0722)));
            coverage = pow(coverage, mix(1.0 / FUNCTOR_MSDF_GAMMA, FUNCTOR_MSDF_GAMMA, light));
        }}
        return coverage;
    }}
"#,
        range = RANGE,
        w = ATLAS_WIDTH as f32,
        h = ATLAS_HEIGHT as f32,
    )
}

/// The overlay's type scale, as line heights (`size`) in metres for a
/// surface 1 m away: multiply by the surface's distance. At the Quest 3's
/// ~15 px/deg eye buffer, body x-height is 0.78° (about 12 px), small 0.61°
/// (9 px), titles 1.04° (16 px). Every overlay surface sizes its text from
/// these, so they read alike.
pub const TITLE: f32 = 0.0267;
pub const BODY: f32 = 0.020;
pub const SMALL: f32 = 0.0156;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Semibold,
}

/// How a line is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub weight: Weight,
    /// Letter-spacing, in em.
    pub tracking: f32,
    /// Figures of one width.
    pub tabular: bool,
}

impl Style {
    pub const REGULAR: Style = Style {
        weight: Weight::Regular,
        tracking: 0.0,
        tabular: false,
    };
    /// An all-caps label: semibold, tracked.
    pub const LABEL: Style = Style {
        weight: Weight::Semibold,
        tracking: CAPS_TRACKING,
        tabular: false,
    };
    /// A code of figures: tabular, tracked wide.
    pub const FIGURES: Style = Style {
        weight: Weight::Regular,
        tracking: FIGURE_TRACKING,
        tabular: true,
    };

    /// The overlay's default, from the copy: an all-caps line is a label
    /// (scripts without case are not); only digits and spaces ("482 913",
    /// not "0.85") are figures.
    pub fn of(text: &str) -> Style {
        let letters = text.chars().any(|c| c.is_uppercase() || c.is_lowercase());
        let lower = text.chars().any(char::is_lowercase);
        let figures = text.chars().any(|c| c.is_ascii_digit())
            && text.chars().all(|c| c.is_ascii_digit() || c == ' ');
        match (letters && !lower, figures) {
            (true, _) => Style::LABEL,
            (_, true) => Style::FIGURES,
            _ => Style::REGULAR,
        }
    }

    /// A memo key: the floats as bits.
    fn key(self) -> (Weight, u32, bool) {
        (self.weight, self.tracking.to_bits(), self.tabular)
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
#[cfg(test)]
pub(crate) fn has_glyph(c: char) -> bool {
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

/// Walks `text` as one line in `style` (else its own, [`Style::of`]): each
/// glyph with its pen position in em, and the line's advance in em.
fn layout(text: &str, style: Option<Style>, mut each: impl FnMut(&'static Glyph, f32)) -> f32 {
    let style = style.unwrap_or_else(|| Style::of(text));
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
    measure(text, size, None)
}

/// [`width`] in `style`.
pub fn width_in(text: &str, size: f32, style: Style) -> f32 {
    measure(text, size, Some(style))
}

fn measure(text: &str, size: f32, style: Option<Style>) -> f32 {
    layout(text, style, |_, _| {}) * em(size)
}

/// Each glyph of `text` as one line `size` tall with its top-left at
/// `at` (metres, y up), with its pen on the baseline.
pub fn place(text: &str, size: f32, at: [f32; 2], each: impl FnMut(&'static Glyph, [f32; 2])) {
    place_with(text, size, None, at, each)
}

/// [`place`] in `style`.
pub fn place_in(
    text: &str,
    size: f32,
    style: Style,
    at: [f32; 2],
    each: impl FnMut(&'static Glyph, [f32; 2]),
) {
    place_with(text, size, Some(style), at, each)
}

fn place_with(
    text: &str,
    size: f32,
    style: Option<Style>,
    at: [f32; 2],
    mut each: impl FnMut(&'static Glyph, [f32; 2]),
) {
    let em = em(size);
    let baseline = at[1] - BASELINE * size;
    layout(text, style, |g, pen| each(g, [at[0] + pen * em, baseline]));
}

/// `text` as one line `size` tall at most `max` metres wide, cut with an
/// ellipsis if it is longer.
pub fn fit(text: &str, size: f32, max: f32) -> String {
    fit_with(text, size, max, None)
}

/// [`fit`] in `style`.
pub fn fit_in(text: &str, size: f32, style: Style, max: f32) -> String {
    fit_with(text, size, max, Some(style))
}

fn fit_with(text: &str, size: f32, max: f32, style: Option<Style>) -> String {
    let fitted = remembered(text, size, max, None, style, || {
        vec![fit_uncached(text, size, max, style)]
    });
    fitted.into_iter().next().unwrap_or_default()
}

fn fit_uncached(text: &str, size: f32, max: f32, style: Option<Style>) -> String {
    if measure(text, size, style) <= max {
        return text.to_string();
    }
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let cut = |n: usize| format!("{}{ELLIPSIS}", text[..ends[n]].trim_end());
    match longest(ends.len() - 1, |n| measure(&cut(n), size, style) <= max) {
        // Not even the ellipsis fits.
        0 if measure(&cut(0), size, style) > max => String::new(),
        n => cut(n),
    }
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
/// `lines` of them, the last cut with an ellipsis. Each line measures in
/// its own style ([`Style::of`]).
pub fn wrap(text: &str, size: f32, max: f32, lines: usize) -> Vec<String> {
    wrap_with(text, size, max, lines, None)
}

/// [`wrap`], every line in `style`: draw them in it too.
pub fn wrap_in(text: &str, size: f32, style: Style, max: f32, lines: usize) -> Vec<String> {
    wrap_with(text, size, max, lines, Some(style))
}

fn wrap_with(text: &str, size: f32, max: f32, lines: usize, style: Option<Style>) -> Vec<String> {
    remembered(text, size, max, Some(lines), style, || {
        wrap_uncached(text, size, max, lines, style)
    })
}

/// `make`'s lines for these arguments (`lines` is `wrap`'s; `None` for
/// `fit`), remembered: the panel lays its text out every frame it shows, and
/// the text rarely changes.
fn remembered(
    text: &str,
    size: f32,
    max: f32,
    lines: Option<usize>,
    style: Option<Style>,
    make: impl FnOnce() -> Vec<String>,
) -> Vec<String> {
    type Key = (String, u32, u32, Option<usize>, Option<(Weight, u32, bool)>);
    thread_local! {
        static MEMO: RefCell<HashMap<Key, Vec<String>>> = RefCell::default();
    }
    let key = (text.to_string(), size.to_bits(), max.to_bits(), lines, style.map(Style::key));
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

fn wrap_uncached(text: &str, size: f32, max: f32, lines: usize, style: Option<Style>) -> Vec<String> {
    let mut words: std::collections::VecDeque<String> =
        text.split_whitespace().map(String::from).collect();
    let mut out = Vec::new();
    while !words.is_empty() && out.len() < lines {
        if out.len() + 1 == lines {
            let rest: Vec<String> = words.drain(..).collect();
            out.push(fit_with(&rest.join(" "), size, max, style));
            break;
        }
        let mut line = words.pop_front().unwrap();
        let ends: Vec<usize> = line.char_indices().map(|(i, _)| i).skip(1).collect();
        if !ends.is_empty() && measure(&line, size, style) > max {
            // A word longer than a line: split it where it overflows, after
            // at least one character.
            let fits = |n: usize| n == 0 || measure(&line[..ends[n - 1]], size, style) <= max;
            let n = longest(ends.len(), fits);
            let split = ends[n.max(1) - 1];
            words.push_front(line.split_off(split));
        } else {
            while let Some(next) = words.front() {
                let candidate = format!("{line} {next}");
                if measure(&candidate, size, style) > max {
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
        assert_eq!(fit("Too wide", SIZE, 0.001), "", "not even the ellipsis fits");
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
    fn a_given_style_overrides_the_copy() {
        // Game text: caps and figures that are not labels or codes.
        for text in ["HP 100", "2048"] {
            assert_ne!(width(text, SIZE), width_in(text, SIZE, Style::REGULAR), "{text}");
        }
        assert_eq!(width_in("HP 100", SIZE, Style::LABEL), width("HP 100", SIZE));
        let mut keys = Vec::new();
        place_in("2048", SIZE, Style::REGULAR, [0.0, 0.0], |g, _| keys.push(g.key));
        assert_eq!(keys, ['2' as u32, '0' as u32, '4' as u32, '8' as u32], "proportional");
        // Wrapped in a style, every line measures in it: here a fragment
        // that is all caps keeps the paragraph's regular weight.
        let text = "Press the button marked OK OK OK OK OK OK OK OK OK OK OK OK OK";
        let max = width_in("Press the button marked", SIZE, Style::REGULAR);
        let lines = wrap_in(text, SIZE, Style::REGULAR, max, 10);
        assert!(lines.len() > 1 && lines.iter().any(|l| Style::of(l) == Style::LABEL));
        assert!(lines.iter().all(|l| width_in(l, SIZE, Style::REGULAR) <= max));
        // Inferred, those lines would draw wider: semibold and tracked.
        assert!(lines.iter().any(|l| width(l, SIZE) > width_in(l, SIZE, Style::REGULAR)));
        let cut = fit_in("HP 100 of 250", SIZE, Style::REGULAR, width_in("HP 100", SIZE, Style::REGULAR));
        assert!(cut.ends_with(ELLIPSIS) && width_in(&cut, SIZE, Style::REGULAR) <= width_in("HP 100", SIZE, Style::REGULAR));
    }

    #[test]
    fn the_shared_shader_source_names_its_functions() {
        let glsl = msdf_glsl();
        assert!(glsl.contains("functorMsdfTexels") && glsl.contains("functorMsdfCoverage"));
        assert!(glsl.contains(&format!("{:?}", ATLAS_WIDTH as f32)));
        // A script without case is not a label.
        assert_eq!(Style::of("\u{4E2D}\u{6587}"), Style::REGULAR);
    }

    #[test]
    fn caps_are_semibold_and_tracked() {
        let (caps, plain) = (Style::of("PAIRED"), Style::of("Paired"));
        assert_eq!((caps.weight, plain.weight), (Weight::Semibold, Weight::Regular));
        assert!(caps.tracking > 0.0 && plain.tracking == 0.0);
        assert!(Style::of("482 913").tabular && !Style::of("Sending in 3").tabular);
        assert!(!Style::of("0.85").tabular, "a number in prose stays proportional");
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
