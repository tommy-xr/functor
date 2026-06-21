//! In-game text overlay, rendered with egui on top of the 3D frame.
//!
//! This is the *shared* 2D pass: the shells (desktop runner, web runtime) and the
//! netsim visualizer call it after [`crate::render_frame`], handing it a set of
//! absolutely-positioned text labels. egui lives entirely here in the imperative
//! shell — game code never touches it. A declarative `model -> View` F# API will
//! later sit on top of this, lowering to the same `Label` list.
//!
//! egui is *immediate mode* (imperative); Functor is MVU (declarative). They
//! reconcile by giving egui one job — rendering/layout/text — while the engine's
//! public surface stays declarative. This module is that seam.

use std::sync::Arc;

use fable_library_rust::NativeArray_::Array;
use fable_library_rust::String_::LrcStr;
use glow::HasContext;
use serde::{Deserialize, Serialize};

/// A single piece of screen-space text. `x`/`y` are in **points** measured from the
/// top-left corner (points == pixels when `pixels_per_point` is 1.0).
pub struct Label {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub color: [u8; 3],
}

impl Label {
    /// A white label at `(x, y)` points from the top-left.
    pub fn new(text: impl Into<String>, x: f32, y: f32) -> Self {
        Self {
            text: text.into(),
            x,
            y,
            color: [255, 255, 255],
        }
    }

    pub fn with_color(mut self, color: [u8; 3]) -> Self {
        self.color = color;
        self
    }
}

/// A font referenced by logical family name + size in points. The actual font
/// *bytes* live in the shell's font registry; a `View` only ever names a font, so
/// the tree stays serializable/inspectable. Only the default font is wired today —
/// the family is carried for forward-compatibility and unknown names fall back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FontRef {
    pub family: String,
    pub size: f32,
}

/// Which screen corner a [`View::Panel`] pins its subtree to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// A declarative, serializable 2D UI tree — the lowering target for the F# `Ui`
/// API (`ui : 'model -> View`). It carries only text, layout, colors and *names*
/// (e.g. fonts), never bytes, so it round-trips as JSON across the wasm boundary
/// and stays introspectable. [`View::lower`] flattens it to absolute [`Label`]s.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum View {
    /// Renders nothing — the default `ui` for games that don't draw a HUD.
    Empty,
    Text {
        text: String,
        color: [u8; 3],
        #[serde(default)]
        font: Option<FontRef>,
    },
    /// Children stacked top-to-bottom.
    Column(Vec<View>),
    /// Children laid out left-to-right.
    Row(Vec<View>),
    /// Pin a subtree to a screen corner.
    Panel { anchor: Anchor, child: Box<View> },
}

fn rgb_u8(r: f32, g: f32, b: f32) -> [u8; 3] {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [c(r), c(g), c(b)]
}

// F#-facing constructors. Each takes Fable's string/array types (`LrcStr`,
// `Array`) and mirrors the `Scene3D`/`TextureDescription` Emit-shim pattern.
impl View {
    pub fn empty() -> View {
        View::Empty
    }

    pub fn text(s: LrcStr) -> View {
        View::Text {
            text: s.to_string(),
            color: [255, 255, 255],
            font: None,
        }
    }

    pub fn text_color(r: f32, g: f32, b: f32, s: LrcStr) -> View {
        View::Text {
            text: s.to_string(),
            color: rgb_u8(r, g, b),
            font: None,
        }
    }

    /// Text in a named font at `size` points. Only the default font renders today;
    /// the family is recorded for when the font registry lands.
    pub fn text_font(family: LrcStr, size: f32, s: LrcStr) -> View {
        View::Text {
            text: s.to_string(),
            color: [255, 255, 255],
            font: Some(FontRef {
                family: family.to_string(),
                size,
            }),
        }
    }

    pub fn column(items: Array<View>) -> View {
        View::Column(items.to_vec())
    }

    pub fn row(items: Array<View>) -> View {
        View::Row(items.to_vec())
    }

    pub fn panel(anchor: Anchor, child: View) -> View {
        View::Panel {
            anchor,
            child: Box::new(child),
        }
    }

    /// Flatten this tree into absolutely-positioned labels over a `sw`×`sh`-point
    /// screen. Layout is deliberately simple: fixed line height / char width
    /// (matching the default monospace face), columns stack, rows advance, panels
    /// anchor to a corner. Font sizing is honored once real fonts are wired.
    pub fn lower(&self, sw: f32, sh: f32) -> Vec<Label> {
        let mut out = Vec::new();
        // A bare (non-panel) root defaults to the top-left, inset by a margin.
        match self {
            View::Panel { .. } => {
                place(self, 0.0, 0.0, sw, sh, &mut out);
            }
            _ => {
                place(self, MARGIN, MARGIN, sw, sh, &mut out);
            }
        }
        out
    }
}

const LINE_H: f32 = 18.0;
const CHAR_W: f32 = 8.2;
const ROW_GAP: f32 = 8.0;
const MARGIN: f32 = 10.0;

/// Place `view`'s labels starting at top-left `(x, y)` and return the (width,
/// height) it occupied (points). `sw`/`sh` are the screen size, for panel anchoring.
fn place(view: &View, x: f32, y: f32, sw: f32, sh: f32, out: &mut Vec<Label>) -> (f32, f32) {
    match view {
        View::Empty => (0.0, 0.0),
        View::Text { text, color, .. } => {
            out.push(Label {
                text: text.clone(),
                x,
                y,
                color: *color,
            });
            (text.chars().count() as f32 * CHAR_W, LINE_H)
        }
        View::Column(items) => {
            let (mut w, mut h) = (0.0f32, 0.0f32);
            for item in items {
                let (iw, ih) = place(item, x, y + h, sw, sh, out);
                w = w.max(iw);
                h += ih;
            }
            (w, h)
        }
        View::Row(items) => {
            let (mut w, mut h) = (0.0f32, 0.0f32);
            for item in items {
                let (iw, ih) = place(item, x + w, y, sw, sh, out);
                w += iw + ROW_GAP;
                h = h.max(ih);
            }
            (w, h)
        }
        View::Panel { anchor, child } => {
            // Lay the child out at the origin to measure it, then translate the
            // labels we just emitted to the anchored corner.
            let start = out.len();
            let (w, h) = place(child, 0.0, 0.0, sw, sh, out);
            let (ox, oy) = match anchor {
                Anchor::TopLeft => (MARGIN, MARGIN),
                Anchor::TopRight => (sw - w - MARGIN, MARGIN),
                Anchor::BottomLeft => (MARGIN, sh - h - MARGIN),
                Anchor::BottomRight => (sw - w - MARGIN, sh - h - MARGIN),
            };
            for label in &mut out[start..] {
                label.x += ox;
                label.y += oy;
            }
            (w, h)
        }
    }
}

/// Owns the egui context and the glow painter (font-atlas texture + shaders).
///
/// Construct once with the runtime's shared GL context, then call [`Self::draw`]
/// each frame after the 3D pass. The painter holds an `Arc<glow::Context>`, so the
/// shell must keep its context in an `Arc` and hand a clone here.
pub struct TextOverlay {
    ctx: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
}

impl TextOverlay {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        // `None` shader version -> egui_glow autodetects (GL 4.1 core vs WebGL2);
        // empty prefix, no dithering. Panics only on a GL too old for egui, which
        // can't happen given the runtime already requires GL3.3+/WebGL2.
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .expect("failed to create egui_glow painter");
        Self {
            ctx: egui::Context::default(),
            painter,
            gl,
        }
    }

    /// Paint `labels` over the bound framebuffer. `width`/`height` are the physical
    /// framebuffer size in pixels; `pixels_per_point` maps points -> pixels (1.0 on
    /// a non-HiDPI display; the device pixel ratio on retina / browser canvases).
    pub fn draw(&mut self, width: u32, height: u32, pixels_per_point: f32, labels: &[Label]) {
        if width == 0 || height == 0 || labels.is_empty() {
            return;
        }

        self.ctx.set_pixels_per_point(pixels_per_point);
        let screen_points = egui::vec2(
            width as f32 / pixels_per_point,
            height as f32 / pixels_per_point,
        );
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_points)),
            ..Default::default()
        };

        let output = self.ctx.run_ui(raw_input, |ui| {
            // Labels are floating, Context-attached Areas rather than children of
            // the root Ui, so pull the context back out of the supplied `ui`.
            let ctx = ui.ctx().clone();
            for (i, label) in labels.iter().enumerate() {
                // Each label is its own non-interactive Area pinned at (x, y), so
                // labels never push each other around — callers place them exactly.
                egui::Area::new(egui::Id::new(("functor_overlay", i)))
                    .fixed_pos(egui::pos2(label.x, label.y))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        let [r, g, b] = label.color;
                        ui.label(
                            egui::RichText::new(&label.text)
                                .monospace()
                                .color(egui::Color32::from_rgb(r, g, b)),
                        );
                    });
            }
        });

        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            [width, height],
            output.pixels_per_point,
            &primitives,
            &output.textures_delta,
        );

        // egui_glow mutates global GL state (enables BLEND + SCISSOR, leaves
        // DEPTH_TEST as-is) and does not restore it. The shared 3D path enables
        // DEPTH_TEST only once at startup and re-arms SCISSOR per frame, so reset
        // to the slate the next 3D frame expects.
        unsafe {
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable(glow::BLEND);
            self.gl.enable(glow::DEPTH_TEST);
        }
    }

    /// Lower a declarative [`View`] to labels and paint it. `width`/`height` are
    /// physical framebuffer pixels; the tree is laid out in points (pixels / ppp).
    pub fn draw_view(&mut self, width: u32, height: u32, pixels_per_point: f32, view: &View) {
        let labels = view.lower(
            width as f32 / pixels_per_point,
            height as f32 / pixels_per_point,
        );
        self.draw(width, height, pixels_per_point, &labels);
    }
}
