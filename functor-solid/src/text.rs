//! Text as a [`Sketch`]: one line laid out with HarfBuzz shaping (kerning,
//! ligatures) and glyph outlines flattened to the chord tolerance.
//!
//! Two fonts are built in (OFL subsets, see `fonts/README.md`) so text works
//! on every target; any TrueType/OpenType font loads from its bytes.

use std::sync::{Arc, OnceLock};

use harfrust::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

use crate::{Sketch, CHORD_TOLERANCE};

#[derive(Debug, Clone)]
pub enum Font {
    /// A readable semi-cursive script (derived from Courgette).
    Script,
    /// A clean sans-serif (derived from Lato).
    Sans,
    /// A font file's bytes (see [`Font::from_bytes`]).
    Custom(Arc<CustomFont>),
}

/// A TrueType/OpenType font loaded from bytes, with its shaping caches.
pub struct CustomFont {
    name: String,
    bytes: Vec<u8>,
    shaper: OnceLock<ShaperData>,
}

impl std::fmt::Debug for CustomFont {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "CustomFont({})", self.name)
    }
}

impl Font {
    /// A font from a `.ttf`/`.otf` file's bytes; `name` labels its errors.
    pub fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<Font, String> {
        let face = FontRef::new(&bytes)
            .map_err(|e| format!("{name} is not a TrueType or OpenType font ({e})"))?;
        if face.outline_glyphs().format().is_none() {
            return Err(format!("{name} has no glyph outlines"));
        }
        // Text is scaled by the em: a broken one would make NaN geometry.
        let upem = face
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        if !(16..=16384).contains(&upem) {
            return Err(format!("{name} has an invalid em size ({upem} units)"));
        }
        Ok(Font::Custom(Arc::new(CustomFont {
            name: name.to_string(),
            bytes,
            shaper: OnceLock::new(),
        })))
    }

    fn name(&self) -> &str {
        match self {
            Font::Script => "the script typeface",
            Font::Sans => "the sans typeface",
            Font::Custom(font) => &font.name,
        }
    }

    fn face(&self) -> FontRef<'_> {
        let bytes: &[u8] = match self {
            Font::Script => include_bytes!("../fonts/FunctorScript-Regular.ttf"),
            Font::Sans => include_bytes!("../fonts/FunctorSans-Regular.ttf"),
            Font::Custom(font) => &font.bytes,
        };
        FontRef::new(bytes).expect("fonts are checked when created")
    }

    /// Shaping lookup caches, built once per font.
    fn shaper_data(&self) -> &ShaperData {
        static SCRIPT: OnceLock<ShaperData> = OnceLock::new();
        static SANS: OnceLock<ShaperData> = OnceLock::new();
        let cell = match self {
            Font::Script => &SCRIPT,
            Font::Sans => &SANS,
            Font::Custom(font) => &font.shaper,
        };
        cell.get_or_init(|| ShaperData::new(&self.face()))
    }
}

/// OpenSCAD's `text(size=…)` convention, so ported models match: the em is
/// `size × 100/72` mm (FreeType's point size at 100 dpi).
const EM_PER_SIZE: f64 = 100.0 / 72.0;

/// Shaped glyphs of one line: (glyph, pen x, y offset) in font units, plus the
/// total advance. Errors on a character the font can't draw (including line
/// breaks), rather than leaving a silent gap in a printed part.
fn shape(
    font: &Font,
    face: &FontRef,
    text: &str,
) -> Result<(Vec<(GlyphId, f64, f64)>, f64), String> {
    let charmap = face.charmap();
    if let Some(c) = text.chars().find(|&c| charmap.map(c).is_none()) {
        let covers = match font {
            Font::Custom(_) => "one line of text",
            _ => "one line of Latin-1 text",
        };
        return Err(format!("{c:?} isn't in {} ({covers})", font.name()));
    }
    let shaper = font.shaper_data().shaper(face).build();
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let shaped = shaper.shape(buffer, ShapeOptions::new());
    let mut pen_x = 0.0;
    let glyphs = shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| {
            let placed = (
                GlyphId::new(info.glyph_id),
                pen_x + pos.x_offset as f64,
                pos.y_offset as f64,
            );
            pen_x += pos.x_advance as f64;
            placed
        })
        .collect();
    Ok((glyphs, pen_x))
}

fn scale(face: &FontRef, size: f64) -> f64 {
    let upem = face
        .metrics(Size::unscaled(), LocationRef::default())
        .units_per_em;
    size * EM_PER_SIZE / upem as f64
}

/// One line of `text`, `size` as in OpenSCAD, starting at the origin on the
/// baseline. Overlapping glyphs merge; counters ("o", "e") stay holes.
pub fn text(font: &Font, size: f64, text: &str) -> Result<Sketch, String> {
    let face = font.face();
    let (glyphs, _) = shape(font, &face, text)?;
    let outlines = face.outline_glyphs();
    let mut pen = FlatteningPen {
        k: scale(&face, size),
        origin: [0.0, 0.0],
        contours: Vec::new(),
    };
    for (gid, x, y) in glyphs {
        let glyph = outlines
            .get(gid)
            .ok_or_else(|| format!("{} is missing a glyph it maps", font.name()))?;
        pen.origin = [x, y];
        glyph
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                &mut pen,
            )
            .map_err(|e| format!("a glyph in {} can't be drawn ({e})", font.name()))?;
    }
    let contours: Vec<Vec<[f64; 2]>> = pen.contours.into_iter().filter(|c| c.len() >= 3).collect();
    Ok(Sketch {
        shapes: contours.simplify_shape(FillRule::NonZero),
    })
}

/// The advance width of one line of `text`, in mm — for right or centered
/// alignment.
pub fn text_width(font: &Font, size: f64, text: &str) -> Result<f64, String> {
    let face = font.face();
    Ok(shape(font, &face, text)?.1 * scale(&face, size))
}

/// Collects glyph contours in mm, flattening curves so no chord sags more than
/// [`CHORD_TOLERANCE`].
struct FlatteningPen {
    /// mm per font unit.
    k: f64,
    /// Glyph origin, font units.
    origin: [f64; 2],
    contours: Vec<Vec<[f64; 2]>>,
}

impl FlatteningPen {
    fn mm(&self, x: f32, y: f32) -> [f64; 2] {
        [
            (self.origin[0] + x as f64) * self.k,
            (self.origin[1] + y as f64) * self.k,
        ]
    }

    fn last(&self) -> [f64; 2] {
        *self
            .contours
            .last()
            .and_then(|c| c.last())
            .expect("a segment follows move_to")
    }

    fn push(&mut self, p: [f64; 2]) {
        self.contours.last_mut().expect("move_to first").push(p);
    }

    /// Segments for a curve whose control polygon bends by `bend` mm, so the
    /// sag stays within tolerance (error ≈ bend / 4n² for a quadratic).
    fn segments(bend: f64) -> usize {
        (bend / (4.0 * CHORD_TOLERANCE)).sqrt().ceil().max(1.0) as usize
    }
}

fn bend(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (x, y) = (a[0] - 2.0 * b[0] + c[0], a[1] - 2.0 * b[1] + c[1]);
    (x * x + y * y).sqrt()
}

impl OutlinePen for FlatteningPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.mm(x, y);
        self.contours.push(vec![p]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.mm(x, y);
        self.push(p);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last(), self.mm(cx0, cy0), self.mm(x, y));
        let n = Self::segments(bend(p0, p1, p2));
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push(std::array::from_fn(|j| {
                u * u * p0[j] + 2.0 * u * t * p1[j] + t * t * p2[j]
            }));
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (
            self.last(),
            self.mm(cx0, cy0),
            self.mm(cx1, cy1),
            self.mm(x, y),
        );
        // A cubic bends up to 3× its control polygon's second difference.
        let n = Self::segments(3.0 * bend(p0, p1, p2).max(bend(p1, p2, p3)));
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push(std::array::from_fn(|j| {
                u * u * u * p0[j]
                    + 3.0 * u * u * t * p1[j]
                    + 3.0 * u * t * t * p2[j]
                    + t * t * t * p3[j]
            }));
        }
    }

    fn close(&mut self) {
        // Contours are implicitly closed; drop a duplicated closing point.
        if let Some(c) = self.contours.last_mut() {
            if c.len() > 1 && c.first() == c.last() {
                c.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(s: &Sketch) -> ([f64; 2], [f64; 2]) {
        let pts: Vec<_> = s.shapes.iter().flatten().flatten().collect();
        let lo = [0, 1].map(|i| pts.iter().map(|p| p[i]).fold(f64::MAX, f64::min));
        let hi = [0, 1].map(|i| pts.iter().map(|p| p[i]).fold(f64::MIN, f64::max));
        (lo, hi)
    }

    /// OpenSCAD renders Courgette "H" at size 10 as 10.606 mm tall (measured
    /// with OpenSCAD 2021.01); the size convention must reproduce it.
    #[test]
    fn size_matches_openscad() {
        let (lo, hi) = bounds(&text(&Font::Script, 10.0, "H").unwrap());
        assert!((hi[1] - lo[1] - 10.606).abs() < 0.01, "{}", hi[1] - lo[1]);
    }

    #[test]
    fn counters_are_holes_and_words_extrude_closed() {
        let o = text(&Font::Sans, 10.0, "o").unwrap();
        assert_eq!(o.shapes.len(), 1);
        assert_eq!(o.shapes[0].len(), 2, "outer ring + counter");
        for font in [Font::Script, Font::Sans] {
            for line in [
                "Love, Kid 1 & Kid 2",
                "The quick brown fox jumps over the lazy dog",
                "SPHINX OF BLACK QUARTZ, JUDGE MY VOW 0123456789",
            ] {
                let s = text(&font, 6.0, line).unwrap();
                assert_eq!(s.touching_point(), None, "{font:?} {line}");
                let mesh = s.extrude(0.6);
                assert_eq!(mesh.non_manifold_edges(), 0);
                assert!((mesh.volume() - s.area() * 0.6).abs() < 1e-6 * s.area());
            }
        }
    }

    #[test]
    fn every_mapped_character_draws() {
        for font in [Font::Script, Font::Sans] {
            let face = font.face();
            for (c, _) in face.charmap().mappings() {
                let c = char::from_u32(c).unwrap();
                assert!(text(&font, 5.0, &c.to_string()).is_ok(), "{font:?} {c:?}");
            }
        }
    }

    #[test]
    fn unsupported_characters_and_line_breaks_are_errors() {
        let err = text(&Font::Script, 6.0, "5 €").unwrap_err();
        assert!(err.contains("'€'"), "{err}");
        assert!(text(&Font::Sans, 6.0, "a\nb").is_err());
        assert!(text_width(&Font::Sans, 6.0, "♥").is_err());
    }

    /// Offsetting a word leaves point pairs nanometres apart; extrusion must
    /// still come out closed.
    #[test]
    fn fattened_words_extrude_closed() {
        let s = text(&Font::Script, 6.0, "Husband").unwrap().offset(0.12);
        let mesh = s.extrude(0.6);
        assert_eq!(mesh.shared_positions(), 0);
        assert_eq!(mesh.non_manifold_edges(), 0);
    }

    #[test]
    fn width_tracks_shaped_advance() {
        let w = text_width(&Font::Sans, 10.0, "Hello").unwrap();
        let (lo, hi) = bounds(&text(&Font::Sans, 10.0, "Hello").unwrap());
        assert!(w >= hi[0] - lo[0] && w < (hi[0] - lo[0]) * 1.3, "{w}");
        assert_eq!(text_width(&Font::Sans, 10.0, "").unwrap(), 0.0);
        assert!(text(&Font::Sans, 10.0, "").unwrap().is_empty());
    }

    #[test]
    fn a_font_file_draws_like_the_built_in_it_copies() {
        let bytes = include_bytes!("../fonts/FunctorSans-Regular.ttf").to_vec();
        let file = Font::from_bytes("Sans.ttf", bytes).unwrap();
        let line = "Love, Kid 1 & Kid 2";
        assert_eq!(
            text(&file, 6.0, line).unwrap().shapes,
            text(&Font::Sans, 6.0, line).unwrap().shapes
        );
        let err = text(&file, 6.0, "♥").unwrap_err();
        assert!(err.contains("Sans.ttf"), "{err}");
        let err = Font::from_bytes("notes.txt", b"not a font".to_vec()).unwrap_err();
        assert!(err.contains("notes.txt is not a TrueType"), "{err}");
    }
}
