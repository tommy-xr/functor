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

use glow::HasContext;

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
}
