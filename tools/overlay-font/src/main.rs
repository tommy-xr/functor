//! Builds the headset overlay's MSDF font: one RGB atlas PNG plus a Rust
//! table of glyph metrics and pair kerning, from two static TrueType files.
//! Run through `generate.sh`, which fetches the pinned fonts.
//!
//! `overlay-font <regular.ttf> <semibold.ttf> <out-dir>` writes
//! `<out-dir>/atlas.png` and `<out-dir>/metrics.rs`.

use std::fmt::Write as _;

use fdsm::bezier::scanline::FillRule;
use fdsm::correct_error::{correct_error_msdf, ErrorCorrectionConfig};
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::Shape;
use fdsm::transform::Transform;
use image::{ImageBuffer, Rgb, RgbImage};
use nalgebra::{Affine2, Similarity2, Vector2};
use rustybuzz::{Feature, UnicodeBuffer};
use ttf_parser::{GlyphId, Tag};

/// Atlas pixels per em, and the distance range in atlas pixels (the field
/// spans ±RANGE/2 around the outline).
const EM_PX: f64 = 32.0;
const RANGE: f64 = 4.0;
/// Empty texels around each outline: the field reaches "outside" at the
/// cell's edge, so a quad's border draws nothing.
const PAD: f64 = RANGE / 2.0 + 1.0;
const ATLAS_WIDTH: u32 = 1024;
/// Empty texels between cells: a bilinear tap at a cell's edge must not mix
/// in a neighbour's channels (an MSDF channel alone can read "inside" far
/// from its outline).
const GAP: u32 = 2;

/// Tabular digits live at these keys (a private-use block): the pairing
/// code's figures.
const TABULAR: u32 = 0xE030;
/// The missing-glyph box (the font's `.notdef`).
const NOTDEF: u32 = 0;

fn charset() -> Vec<char> {
    let mut out: Vec<char> = (0x20u32..=0x7E).chain(0xA0..=0xFF).filter_map(char::from_u32).collect();
    // Typographic punctuation, arrows and symbols overlay copy uses.
    out.extend([
        '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201C}', '\u{201D}',
        '\u{201E}', '\u{2022}', '\u{2026}', '\u{2039}', '\u{203A}', '\u{20AC}', '\u{2190}',
        '\u{2191}', '\u{2192}', '\u{2193}', '\u{2212}', '\u{2713}',
    ]);
    out
}

struct Glyph {
    key: u32,
    weight: usize,
    advance: f64,
    /// Plane box in em from the pen on the baseline: left, bottom, right, top.
    plane: [f64; 4],
    image: Option<RgbImage>,
    /// Atlas cell: x, y, w, h (y down).
    cell: [u32; 4],
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, regular, semibold, out] = &args[..] else {
        eprintln!("usage: overlay-font <regular.ttf> <semibold.ttf> <out-dir>");
        std::process::exit(2);
    };
    let fonts = [std::fs::read(regular).unwrap(), std::fs::read(semibold).unwrap()];
    let chars = charset();
    let mut glyphs = Vec::new();
    let mut kerning: Vec<Vec<(u32, i32)>> = Vec::new();
    let mut metrics = String::new();
    for (weight, data) in fonts.iter().enumerate() {
        let face = rustybuzz::Face::from_slice(data, 0).expect("parse font");
        let upem = face.units_per_em() as f64;
        if weight == 0 {
            let em = |v: Option<i16>| v.unwrap_or(0) as f64 / upem;
            write!(
                metrics,
                "pub const ASCENDER: f32 = {:.5};\npub const DESCENDER: f32 = {:.5};\n\
                 pub const CAP_HEIGHT: f32 = {:.5};\npub const X_HEIGHT: f32 = {:.5};\n",
                face.ascender() as f64 / upem,
                face.descender() as f64 / upem,
                em(face.capital_height()),
                em(face.x_height()),
            )
            .unwrap();
        }
        // Tailed lowercase l (tells it from I and 1); no contextual
        // substitutions, since the overlay draws one glyph per character.
        let base = [feature(b"cv05", 1), feature(b"calt", 0), feature(b"liga", 0)];
        let tabular = [base[0], base[1], base[2], feature(b"tnum", 1)];
        let shape = |text: &str, features: &[Feature]| {
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(text);
            let shaped = rustybuzz::shape(&face, features, buffer);
            shaped
                .glyph_infos()
                .iter()
                .zip(shaped.glyph_positions())
                .map(|(i, p)| (i.glyph_id, p.x_advance))
                .collect::<Vec<_>>()
        };
        let mut nominal = std::collections::HashMap::new();
        let mut add = |key: u32, gid: u32, advance: i32| {
            glyphs.push(render(&face, key, weight, GlyphId(gid as u16), advance as f64 / upem));
        };
        add(NOTDEF, 0, face.glyph_hor_advance(GlyphId(0)).unwrap_or(0) as i32);
        for &c in &chars {
            let s = shape(&c.to_string(), &base);
            let &[(gid, advance)] = &s[..] else { panic!("{c:?} shaped to {s:?}") };
            if gid == 0 {
                eprintln!("warning: {c:?} (U+{:04X}) is not in the font; skipped", c as u32);
                continue;
            }
            nominal.insert(c, advance);
            add(c as u32, gid, advance);
        }
        for d in 0..10u32 {
            let c = char::from_digit(d, 10).unwrap();
            let s = shape(&c.to_string(), &tabular);
            add(TABULAR + d, s[0].0, s[0].1);
        }
        // Pair kerning: what shaping a pair moves against the two alone.
        let mut pairs = Vec::new();
        for &a in nominal.keys() {
            for &b in nominal.keys() {
                let s = shape(&format!("{a}{b}"), &base);
                if s.len() != 2 {
                    continue;
                }
                let kern = s[0].1 + s[1].1 - nominal[&a] - nominal[&b];
                if kern != 0 {
                    pairs.push(((a as u32) << 16 | b as u32, kern));
                }
            }
        }
        pairs.sort();
        // In 1/2048 em, whatever the font's units.
        let pairs = pairs.into_iter().map(|(k, v)| (k, (v as f64 * 2048.0 / upem).round() as i32));
        kerning.push(pairs.filter(|(_, v)| *v != 0).collect());
    }

    // Shelf-pack the cells, tallest first.
    let mut order: Vec<usize> = (0..glyphs.len()).filter(|&i| glyphs[i].image.is_some()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(glyphs[i].image.as_ref().unwrap().height()));
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for &i in &order {
        let img = glyphs[i].image.as_ref().unwrap();
        let (w, h) = (img.width(), img.height());
        if x + w > ATLAS_WIDTH {
            (x, y, shelf) = (0, y + shelf + GAP, 0);
        }
        glyphs[i].cell = [x, y, w, h];
        x += w + GAP;
        shelf = shelf.max(h);
    }
    let height = (y + shelf).next_multiple_of(4);
    let mut atlas: RgbImage = ImageBuffer::new(ATLAS_WIDTH, height);
    for g in &glyphs {
        if let Some(img) = &g.image {
            image::imageops::replace(&mut atlas, img, g.cell[0] as i64, g.cell[1] as i64);
        }
    }
    let out = std::path::Path::new(out);
    atlas.save(out.join("atlas.png")).expect("write atlas");

    let mut rs = String::new();
    writeln!(rs, "// Generated by tools/overlay-font (generate.sh); do not edit.\n").unwrap();
    write!(
        rs,
        "pub const ATLAS_WIDTH: u32 = {ATLAS_WIDTH};\npub const ATLAS_HEIGHT: u32 = {height};\n\
         /// Atlas pixels per em, and the field's distance range in atlas pixels.\n\
         pub const EM_PX: f32 = {EM_PX:.1};\npub const RANGE: f32 = {RANGE:.1};\n{metrics}"
    )
    .unwrap();
    for (weight, name) in ["REGULAR", "SEMIBOLD"].iter().enumerate() {
        let mut mine: Vec<&Glyph> = glyphs.iter().filter(|g| g.weight == weight).collect();
        mine.sort_by_key(|g| g.key);
        writeln!(rs, "\n#[rustfmt::skip]\npub static {name}: [Glyph; {}] = [", mine.len()).unwrap();
        for g in mine {
            let [l, b, r, t] = g.plane;
            let [cx, cy, cw, ch] = g.cell;
            writeln!(
                rs,
                "    Glyph {{ key: 0x{:04X}, advance: {:.5}, plane: [{l:.5}, {b:.5}, {r:.5}, {t:.5}], cell: [{cx}, {cy}, {cw}, {ch}] }},",
                g.key, g.advance
            )
            .unwrap();
        }
        writeln!(rs, "];").unwrap();
        let pairs = &kerning[weight];
        writeln!(rs, "\n/// `(left << 16 | right, adjustment in 1/2048 em)`, sorted.\n#[rustfmt::skip]\npub static {name}_KERNING: [(u32, i16); {}] = [", pairs.len()).unwrap();
        for chunk in pairs.chunks(6) {
            let row: Vec<String> = chunk.iter().map(|(k, v)| format!("(0x{k:08X}, {v})")).collect();
            writeln!(rs, "    {},", row.join(", ")).unwrap();
        }
        writeln!(rs, "];").unwrap();
    }
    std::fs::write(out.join("metrics.rs"), rs).expect("write metrics");
    eprintln!(
        "atlas {ATLAS_WIDTH}x{height}, {} glyphs, kerning pairs {} / {}",
        glyphs.len(),
        kerning[0].len(),
        kerning[1].len()
    );
}

fn feature(tag: &[u8; 4], value: u32) -> Feature {
    Feature::new(Tag::from_bytes(tag), value, ..)
}

/// One glyph's MSDF cell (none for an empty outline, like a space).
fn render(face: &ttf_parser::Face, key: u32, weight: usize, gid: GlyphId, advance: f64) -> Glyph {
    let upem = face.units_per_em() as f64;
    let mut glyph = Glyph { key, weight, advance, plane: [0.0; 4], image: None, cell: [0; 4] };
    let (Some(mut shape), Some(bbox)) =
        (fdsm_ttf_parser::load_shape_from_face(face, gid), face.glyph_bounding_box(gid))
    else {
        return glyph;
    };
    let shrink = upem / EM_PX;
    let (x0, y0) = (bbox.x_min as f64 / shrink, bbox.y_min as f64 / shrink);
    let (x1, y1) = (bbox.x_max as f64 / shrink, bbox.y_max as f64 / shrink);
    // Whole-pixel origin, so the plane box maps to texel edges exactly.
    let (ox, oy) = ((x0 - PAD).floor(), (y0 - PAD).floor());
    let width = ((x1 + PAD).ceil() - ox) as u32;
    let height = ((y1 + PAD).ceil() - oy) as u32;
    let transform = nalgebra::convert::<_, Affine2<f64>>(Similarity2::new(
        Vector2::new(-ox, -oy),
        0.0,
        1.0 / shrink,
    ));
    shape.transform(&transform);
    let colored = Shape::edge_coloring_simple(shape, 0.03, 69441337420);
    let prepared = colored.prepare();
    let mut field = ImageBuffer::<Rgb<f32>, Vec<f32>>::new(width, height);
    generate_msdf(&prepared, RANGE, &mut field);
    correct_error_msdf(&mut field, &colored, &prepared, RANGE, &ErrorCorrectionConfig::default());
    correct_sign_msdf(&mut field, &prepared, FillRule::Nonzero);
    // Font space is y-up; the atlas is stored top row first.
    let img = RgbImage::from_fn(width, height, |x, y| {
        let p = field.get_pixel(x, height - 1 - y).0;
        Rgb(p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
    });
    let em = EM_PX;
    glyph.plane = [ox / em, oy / em, (ox + width as f64) / em, (oy + height as f64) / em];
    glyph.image = Some(img);
    glyph
}
