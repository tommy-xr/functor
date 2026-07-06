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

impl View {
    /// The empty view — renders nothing. The other variants (`Text`, `Column`,
    /// `Row`, `Panel`) are constructed directly by the MLE `Ui.*` prelude.
    pub fn empty() -> View {
        View::Empty
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
/// Restore the GL state the shared 3D path expects after an egui pass.
/// egui_glow enables BLEND + SCISSOR and leaves DEPTH_TEST as-is; the 3D path
/// enables DEPTH_TEST once at startup and re-arms SCISSOR per frame, so reset
/// to that slate. Shared by every egui pass ([`TextOverlay`], [`Scrubber`]).
fn restore_gl_after_egui(gl: &glow::Context) {
    unsafe {
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.enable(glow::DEPTH_TEST);
    }
}

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
        restore_gl_after_egui(&self.gl);
    }
}

/// Pointer state fed to an interactive overlay, in physical framebuffer pixels.
/// `pos` is `None` while the cursor is captured for free-look (the overlay is
/// not being pointed at), so the scrubber neither hovers nor clicks.
#[derive(Clone, Copy, Default)]
pub struct PointerState {
    pub pos: Option<(f32, f32)>,
    pub primary_down: bool,
}

/// The time-travel state the shell hands the scrubber to render.
#[derive(Clone, Copy)]
pub struct ScrubberState {
    /// The frame the handle sits on (the scrubbed-to frame, or the newest).
    pub frame: u64,
    /// The seekable window `(oldest, newest)` — the draggable range. `None`
    /// until something is recorded (the slider is then hidden).
    pub range: Option<(u64, u64)>,
    pub paused: bool,
}

/// A control the user activated in the scrubber this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrubberAction {
    TogglePause,
    /// Non-destructive scrub to a rendered frame (dragging the timeline).
    SeekTo(u64),
    Step,
}

/// The scrubber's output for one frame.
pub struct ScrubberOutput {
    pub action: Option<ScrubberAction>,
    /// egui is using the pointer (hovering/clicking a control), so the shell
    /// must NOT treat the click as a free-look recapture.
    pub wants_pointer: bool,
}

/// The shell-owned time-travel scrubber (docs/time-travel.md T3): an imperative
/// egui panel — separate from the game's declarative [`View`] — that drives the
/// coupled scene rewind. It keeps its OWN `egui::Context` so its pointer/click
/// accounting never interleaves with the game HUD's [`TextOverlay`] frames.
/// Runtime-owned, not a game hook.
pub struct Scrubber {
    ctx: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
    last_primary_down: bool,
}

impl Scrubber {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .expect("failed to create egui_glow painter for the scrubber");
        Self {
            ctx: egui::Context::default(),
            painter,
            gl,
            last_primary_down: false,
        }
    }

    /// Draw the scrubber and return any control the user activated plus whether
    /// egui wants the pointer this frame. `width`/`height` are physical pixels.
    pub fn draw(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        pointer: PointerState,
        state: ScrubberState,
    ) -> ScrubberOutput {
        if width == 0 || height == 0 {
            return ScrubberOutput {
                action: None,
                wants_pointer: false,
            };
        }
        self.ctx.set_pixels_per_point(pixels_per_point);
        let screen_points = egui::vec2(
            width as f32 / pixels_per_point,
            height as f32 / pixels_per_point,
        );

        // Synthesize egui pointer events from the shell's current pointer state:
        // a move every frame, and a button event on the press/release EDGE (egui
        // needs both to register a click).
        let mut events = Vec::new();
        if let Some((px, py)) = pointer.pos {
            let pos = egui::pos2(px / pixels_per_point, py / pixels_per_point);
            events.push(egui::Event::PointerMoved(pos));
            if pointer.primary_down != self.last_primary_down {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: pointer.primary_down,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        self.last_primary_down = pointer.primary_down;

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_points)),
            events,
            ..Default::default()
        };

        let mut action = None;
        let output = self.ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();
            egui::Area::new(egui::Id::new("functor_scrubber"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -MARGIN))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let label = match state.range {
                                Some((_, hi)) => format!("time-travel · {} / {hi}", state.frame),
                                None => format!("time-travel · frame {}", state.frame),
                            };
                            ui.label(
                                egui::RichText::new(label).font(egui::FontId::monospace(UI_FONT_SIZE)),
                            );
                            if ui.button(if state.paused { "Resume" } else { "Pause" }).clicked() {
                                action = Some(ScrubberAction::TogglePause);
                            }
                            // The draggable timeline: drag anywhere in the
                            // recorded window to scrub (non-destructive). Only
                            // shown once there's a range to drag across.
                            if let Some((lo, hi)) = state.range {
                                if hi > lo {
                                    let mut f = state.frame.clamp(lo, hi);
                                    ui.spacing_mut().slider_width = 260.0;
                                    let slider = ui.add(
                                        egui::Slider::new(&mut f, lo..=hi)
                                            .show_value(false)
                                            .handle_shape(egui::style::HandleShape::Rect {
                                                aspect_ratio: 0.5,
                                            }),
                                    );
                                    if slider.changed() {
                                        action = Some(ScrubberAction::SeekTo(f));
                                    }
                                }
                            }
                            if ui.button("Step >").clicked() {
                                action = Some(ScrubberAction::Step);
                            }
                        });
                    });
                });
        });

        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            [width, height],
            output.pixels_per_point,
            &primitives,
            &output.textures_delta,
        );
        // Restore the GL slate the 3D path expects (see `TextOverlay::run_and_paint`).
        restore_gl_after_egui(&self.gl);

        ScrubberOutput {
            action,
            wants_pointer: self.ctx.egui_wants_pointer_input(),
        }
    }
}
