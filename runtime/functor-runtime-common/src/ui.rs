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

/// 0..1 float components -> 8-bit color, clamped. Shared by the F#-facing
/// `View` constructors and the MLE prelude's `Ui.textColor`, so the two
/// hosts quantize colors identically.
pub fn rgb_u8(r: f32, g: f32, b: f32) -> [u8; 3] {
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

}

/// Point size of the overlay's monospace text.
const UI_FONT_SIZE: f32 = 14.0;
/// Inset of an anchored panel from the screen edge, in points.
const MARGIN: f32 = 10.0;

/// Render a declarative [`View`] into `ui` using egui's own layout (vertical /
/// horizontal / anchored `Area`), so line height and spacing come from the font
/// being rendered — no manual metrics, and lines never overlap.
fn render_view(ui: &mut egui::Ui, view: &View) {
    match view {
        View::Empty => {}
        View::Text { text, color, .. } => {
            let [r, g, b] = *color;
            ui.label(
                egui::RichText::new(text)
                    .font(egui::FontId::monospace(UI_FONT_SIZE))
                    .color(egui::Color32::from_rgb(r, g, b)),
            );
        }
        View::Column(items) => {
            ui.vertical(|ui| {
                for item in items {
                    render_view(ui, item);
                }
            });
        }
        View::Row(items) => {
            ui.horizontal(|ui| {
                for item in items {
                    render_view(ui, item);
                }
            });
        }
        View::Panel { anchor, child } => {
            let (align, offset) = anchor_align(*anchor);
            let ctx = ui.ctx().clone();
            egui::Area::new(egui::Id::new(("functor_ui_panel", anchor_id(*anchor))))
                .anchor(align, offset)
                .interactable(false)
                .show(&ctx, |ui| render_view(ui, child));
        }
    }
}

/// egui corner alignment + inset offset for an [`Anchor`].
fn anchor_align(anchor: Anchor) -> (egui::Align2, egui::Vec2) {
    match anchor {
        Anchor::TopLeft => (egui::Align2::LEFT_TOP, egui::vec2(MARGIN, MARGIN)),
        Anchor::TopRight => (egui::Align2::RIGHT_TOP, egui::vec2(-MARGIN, MARGIN)),
        Anchor::BottomLeft => (egui::Align2::LEFT_BOTTOM, egui::vec2(MARGIN, -MARGIN)),
        Anchor::BottomRight => (egui::Align2::RIGHT_BOTTOM, egui::vec2(-MARGIN, -MARGIN)),
    }
}

/// A stable per-corner id so distinct panels get distinct egui `Area` ids.
fn anchor_id(anchor: Anchor) -> u8 {
    match anchor {
        Anchor::TopLeft => 0,
        Anchor::TopRight => 1,
        Anchor::BottomLeft => 2,
        Anchor::BottomRight => 3,
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

    /// Paint absolutely-positioned `labels` over the bound framebuffer.
    /// `width`/`height` are the physical framebuffer size in pixels;
    /// `pixels_per_point` maps points -> pixels (1.0 on a non-HiDPI display; the
    /// device pixel ratio on retina / browser canvases).
    pub fn draw(&mut self, width: u32, height: u32, pixels_per_point: f32, labels: &[Label]) {
        if labels.is_empty() {
            return;
        }
        self.run_and_paint(width, height, pixels_per_point, |ui| {
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
                                .font(egui::FontId::monospace(UI_FONT_SIZE))
                                .color(egui::Color32::from_rgb(r, g, b)),
                        );
                    });
            }
        });
    }

    /// Paint a declarative [`View`], laid out with egui's own containers so line
    /// height and spacing come from the rendered font (no manual metrics, no
    /// overlap). `width`/`height` are physical framebuffer pixels.
    pub fn draw_view(&mut self, width: u32, height: u32, pixels_per_point: f32, view: &View) {
        if matches!(view, View::Empty) {
            return;
        }
        self.run_and_paint(width, height, pixels_per_point, |ui| match view {
            // A panel anchors itself; a bare root sits at the top-left with a margin.
            View::Panel { .. } => render_view(ui, view),
            other => {
                let ctx = ui.ctx().clone();
                egui::Area::new(egui::Id::new("functor_ui_root"))
                    .anchor(egui::Align2::LEFT_TOP, egui::vec2(MARGIN, MARGIN))
                    .interactable(false)
                    .show(&ctx, |ui| render_view(ui, other));
            }
        });
    }

    /// Run one egui frame (building the UI with `build`), tessellate, paint to the
    /// bound framebuffer, and restore the GL state the 3D path expects. egui's
    /// fonts only exist *during* `run`, so all layout/measurement happens in here.
    fn run_and_paint(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        build: impl FnMut(&mut egui::Ui),
    ) {
        if width == 0 || height == 0 {
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

        let output = self.ctx.run_ui(raw_input, build);

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
}
