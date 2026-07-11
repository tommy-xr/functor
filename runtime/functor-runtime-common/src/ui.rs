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
    /// An interactive button (docs/ui-interaction.md U3). `slot` indexes the
    /// per-frame handler table the producer kept from this tree's `ui(model)`
    /// evaluation — a click comes back as `UiEvent { slot, Clicked }`. The
    /// handler itself (a msg `Value`) never crosses: the tree stays
    /// serializable.
    Button { slot: u32, label: String },
    /// An interactive slider over `min..=max` (docs/ui-interaction.md U4).
    /// `value` is the model's CONTROLLED value; a drag comes back as
    /// `UiEvent { slot, SliderChanged(v) }` per change, and the overlay keeps
    /// a small per-slot buffer so the one-frame model echo never rubber-bands
    /// the handle (see `SliderBuffer`).
    Slider {
        slot: u32,
        min: f64,
        max: f64,
        value: f64,
    },
    /// An interactive single-line text input (docs/ui-interaction.md U4).
    /// `value` is the model's CONTROLLED text; an edit comes back as
    /// `UiEvent { slot, TextChanged(s) }` per change. The overlay keeps the
    /// live editing buffer (see `TextBuffer`) — egui owns cursor/selection
    /// state, the model owns the canonical text.
    TextInput { slot: u32, value: String },
}

/// 0..1 float components -> 8-bit color, clamped. Shared by the F#-facing
/// `View` constructors and the Functor Lang prelude's `Ui.textColor`, so the two
/// hosts quantize colors identically.
pub fn rgb_u8(r: f32, g: f32, b: f32) -> [u8; 3] {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [c(r), c(g), c(b)]
}

impl View {
    /// The empty view — renders nothing. The other variants (`Text`, `Column`,
    /// `Row`, `Panel`) are constructed directly by the Functor Lang `Ui.*` prelude.
    pub fn empty() -> View {
        View::Empty
    }
}

/// An interaction the shell detected on an interactive [`View`] widget,
/// delivered to the producer via `GameProducer::ui_event` and folded through
/// the game's `update` (docs/ui-interaction.md U2). `slot` addresses the
/// widget: interactive `Ui.*` constructors register their handler (a msg or a
/// tagger) in a per-frame table during `ui(model)` evaluation, in construction
/// order, and stamp the node with the index. Serializable — it crosses the
/// debug-server wire (`POST /input`) and is recorded for replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiEvent {
    pub slot: u32,
    pub kind: UiEventKind,
}

/// What happened to the widget. `Clicked` pairs with a verbatim-msg handler
/// (a button); the payload-carrying kinds pair with a tagger applied to the
/// new value (a slider / text input).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiEventKind {
    Clicked,
    SliderChanged(f64),
    TextChanged(String),
}

/// Point size of the overlay's monospace text.
const UI_FONT_SIZE: f32 = 14.0;
/// Inset of an anchored panel from the screen edge, in points.
const MARGIN: f32 = 10.0;

/// Whether the subtree holds any interactive widget — a Panel's egui `Area`
/// is interactable only then, so a pure-text HUD keeps letting clicks
/// through (a click over it still recaptures the cursor for free-look).
fn contains_interactive(view: &View) -> bool {
    match view {
        View::Empty | View::Text { .. } => false,
        View::Column(items) | View::Row(items) => items.iter().any(contains_interactive),
        View::Panel { child, .. } => contains_interactive(child),
        View::Button { .. } | View::Slider { .. } | View::TextInput { .. } => true,
    }
}

/// A keyboard event for the game-UI pass, in shell-neutral vocabulary — the
/// shells collect these while egui wants the keyboard (a text input is
/// focused) and [`TextOverlay::draw_view`] lowers them to egui events. `Char`
/// is printable input (desktop: GLFW char events; web: single-char `e.key`);
/// `Edit` is the editing-key subset egui's `TextEdit` needs. No modifier
/// combos yet (no shift-selection / cmd-shortcuts) — v1 is basic editing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiKeyboardEvent {
    Char(char),
    Edit(UiEditKey),
}

/// The editing keys a focused text input consumes (see [`UiKeyboardEvent`]).
/// `Escape` releases focus (egui's built-in behavior) — shells route Escape
/// here INSTEAD of their own Escape handling while a field is focused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiEditKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
}

/// Lower a [`UiKeyboardEvent`] to the egui event(s) it means. An edit key
/// arrives as a full press+release pair so egui's key-repeat bookkeeping
/// stays consistent frame to frame.
fn keyboard_to_egui(event: UiKeyboardEvent, out: &mut Vec<egui::Event>) {
    match event {
        UiKeyboardEvent::Char(c) => {
            // egui ignores control characters in Text events; filter the
            // obvious ones so a stray '\r' from a shell can't sneak in.
            if !c.is_control() {
                out.push(egui::Event::Text(c.to_string()));
            }
        }
        UiKeyboardEvent::Edit(key) => {
            let key = match key {
                UiEditKey::Backspace => egui::Key::Backspace,
                UiEditKey::Delete => egui::Key::Delete,
                UiEditKey::Left => egui::Key::ArrowLeft,
                UiEditKey::Right => egui::Key::ArrowRight,
                UiEditKey::Home => egui::Key::Home,
                UiEditKey::End => egui::Key::End,
                UiEditKey::Enter => egui::Key::Enter,
                UiEditKey::Escape => egui::Key::Escape,
            };
            for pressed in [true, false] {
                out.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
    }
}

/// Per-slot slider reconciliation (docs/ui-interaction.md U4): `live` is the
/// value egui's handle edits; `last_emitted` is the last value sent up as a
/// `SliderChanged` msg. The model's incoming value overwrites `live` only
/// when it differs from `last_emitted`: our own edit echoing back one frame
/// late leaves the handle alone (no rubber-banding mid-drag), while a change
/// we did NOT cause — game logic, a Reset button, an `update` that clamps —
/// is programmatic and snaps the handle to it. (Corollary: an `update` that
/// clamps NARROWER than the slider's own range churns per frame while a drag
/// is held past the cap — the snap and the pointer fight. Prefer matching
/// the slider's min/max to the accepted range.)
struct SliderBuffer {
    live: f64,
    last_emitted: f64,
}

/// Per-slot text-input reconciliation (docs/ui-interaction.md U4) — the
/// text analogue of [`SliderBuffer`], with the same echo rule: the model's
/// incoming value overwrites the live editing buffer only when it differs
/// from `last_emitted`. Comparing against the BUFFER instead would clobber
/// every keystroke (the model echo is one frame behind while typing) — the
/// comparison target is load-bearing. A transform in `update` (uppercase,
/// clamp-length) reads as programmatic: the field resets to it with the
/// cursor moved to the end — the React-parity wart, accepted.
struct TextBuffer {
    live: String,
    last_emitted: String,
}

/// The mutable per-frame state a [`View`] render threads through the tree:
/// the slot-stamped interactions egui detected, plus the stateful widgets'
/// reconciliation buffers (owned by [`TextOverlay`], keyed by slot; `seen`
/// collects this frame's live slots so stale buffers drop after the pass).
struct UiFrameState<'a> {
    events: &'a mut Vec<UiEvent>,
    sliders: &'a mut std::collections::HashMap<u32, SliderBuffer>,
    seen_sliders: &'a mut std::collections::HashSet<u32>,
    texts: &'a mut std::collections::HashMap<u32, TextBuffer>,
    seen_texts: &'a mut std::collections::HashSet<u32>,
}

/// Render a declarative [`View`] into `ui` using egui's own layout (vertical /
/// horizontal / anchored `Area`), so line height and spacing come from the font
/// being rendered — no manual metrics, and lines never overlap. Interactions
/// egui detects on the view's widgets land in `state.events`, slot-stamped.
fn render_view(ui: &mut egui::Ui, view: &View, state: &mut UiFrameState) {
    match view {
        View::Empty => {}
        View::Text { text, color, .. } => {
            let [r, g, b] = *color;
            // Extend, never soft-wrap: a corner Area remembers its width, so
            // dynamic text that shrinks then grows back would wrap at the
            // STALE narrower constraint (e.g. an echo line after a model
            // reset). HUD text is one line unless the author breaks it.
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .font(egui::FontId::monospace(UI_FONT_SIZE))
                        .color(egui::Color32::from_rgb(r, g, b)),
                )
                .wrap_mode(egui::TextWrapMode::Extend),
            );
        }
        View::Column(items) => {
            ui.vertical(|ui| {
                for item in items {
                    render_view(ui, item, state);
                }
            });
        }
        View::Row(items) => {
            ui.horizontal(|ui| {
                for item in items {
                    render_view(ui, item, state);
                }
            });
        }
        View::Panel { anchor, child } => {
            let (align, offset) = anchor_align(*anchor);
            let ctx = ui.ctx().clone();
            egui::Area::new(egui::Id::new(("functor_ui_panel", anchor_id(*anchor))))
                .anchor(align, offset)
                .interactable(contains_interactive(child))
                .show(&ctx, |ui| render_view(ui, child, state));
        }
        View::Button { slot, label } => {
            let clicked = ui
                .button(egui::RichText::new(label).font(egui::FontId::monospace(UI_FONT_SIZE)))
                .clicked();
            if clicked {
                state.events.push(UiEvent {
                    slot: *slot,
                    kind: UiEventKind::Clicked,
                });
            }
        }
        View::Slider {
            slot,
            min,
            max,
            value,
        } => {
            state.seen_sliders.insert(*slot);
            let buf = state.sliders.entry(*slot).or_insert(SliderBuffer {
                live: *value,
                last_emitted: *value,
            });
            // Echo vs programmatic: see `SliderBuffer`. (Exact f64 equality
            // is right here — the echo is the emitted value round-tripped
            // through the interpreter unchanged.)
            if *value != buf.last_emitted {
                buf.live = *value;
                buf.last_emitted = *value;
            }
            let mut v = buf.live;
            let changed = ui.add(egui::Slider::new(&mut v, *min..=*max)).changed();
            buf.live = v;
            if changed {
                buf.last_emitted = v;
                state.events.push(UiEvent {
                    slot: *slot,
                    kind: UiEventKind::SliderChanged(v),
                });
            }
        }
        View::TextInput { slot, value } => {
            state.seen_texts.insert(*slot);
            let buf = state.texts.entry(*slot).or_insert_with(|| TextBuffer {
                live: value.clone(),
                last_emitted: value.clone(),
            });
            // Echo vs programmatic: see `TextBuffer`.
            if *value != buf.last_emitted {
                buf.live = value.clone();
                buf.last_emitted = value.clone();
            }
            let changed = ui
                .add(
                    egui::TextEdit::singleline(&mut buf.live)
                        .font(egui::FontId::monospace(UI_FONT_SIZE)),
                )
                .changed();
            if changed {
                buf.last_emitted = buf.live.clone();
                state.events.push(UiEvent {
                    slot: *slot,
                    kind: UiEventKind::TextChanged(buf.live.clone()),
                });
            }
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

/// The game-UI pass's output for one frame (docs/ui-interaction.md U3).
#[derive(Default)]
pub struct UiOutput {
    /// Interactions egui detected on the view's widgets, slot-stamped — the
    /// shell forwards each to `GameProducer::ui_event`.
    pub events: Vec<UiEvent>,
    /// egui is using the pointer (hovering/clicking a widget), so the shell
    /// must NOT treat a click as a free-look recapture (the scrubber rule).
    pub wants_pointer: bool,
    /// egui wants the keyboard (a text input is focused): the shell routes
    /// keys to the overlay as [`UiKeyboardEvent`]s and suppresses the game's
    /// `input` hook (docs/ui-interaction.md U4).
    pub wants_keyboard: bool,
}

pub struct TextOverlay {
    ctx: egui::Context,
    painter: egui_glow::Painter,
    gl: Arc<glow::Context>,
    pointer: PointerBridge,
    /// Per-slot slider reconciliation state, kept across frames (see
    /// [`SliderBuffer`]); entries whose slot leaves the view are dropped.
    sliders: std::collections::HashMap<u32, SliderBuffer>,
    /// Per-slot text-input editing buffers (see [`TextBuffer`]), same rules.
    texts: std::collections::HashMap<u32, TextBuffer>,
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
            pointer: PointerBridge::default(),
            sliders: std::collections::HashMap::new(),
            texts: std::collections::HashMap::new(),
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
        // Label overlays (netsim, the F# path) are display-only: no pointer.
        self.run_and_paint(width, height, pixels_per_point, Vec::new(), |ui| {
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
    /// overlap). `width`/`height` are physical framebuffer pixels. `pointer`
    /// drives the view's interactive widgets ([`PointerState::default`] for a
    /// display-only pass) and `keyboard` feeds a focused text input (empty
    /// unless the shell saw `wants_keyboard` last frame); interactions come
    /// back slot-stamped in the [`UiOutput`] for the shell to forward to
    /// `GameProducer::ui_event`.
    pub fn draw_view(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        pointer: PointerState,
        keyboard: &[UiKeyboardEvent],
        view: &View,
    ) -> UiOutput {
        // Tick the bridge even when nothing draws, so button edges spanning
        // an Empty frame don't replay as phantom clicks later.
        let mut input_events = self.pointer.events(pointer, pixels_per_point);
        for event in keyboard {
            keyboard_to_egui(*event, &mut input_events);
        }
        if matches!(view, View::Empty) {
            // Still feed egui any release/PointerGone the bridge synthesized:
            // a view hidden mid-press must not leave the context holding a
            // stuck press for the view's return. [xreview]
            if !input_events.is_empty() {
                self.run_and_paint(width, height, pixels_per_point, input_events, |_| {});
            }
            return UiOutput::default();
        }
        // A zero-size frame never runs the build (`run_and_paint` bails), so
        // bail BEFORE the buffer sweep below — a minimized window must not
        // wipe every widget buffer via an all-unseen retain. [xreview]
        if width == 0 || height == 0 {
            return UiOutput::default();
        }
        let mut events = Vec::new();
        let mut seen_sliders = std::collections::HashSet::new();
        let mut seen_texts = std::collections::HashSet::new();
        // Moved out for the pass — `run_and_paint` borrows self mutably too.
        let mut sliders = std::mem::take(&mut self.sliders);
        let mut texts = std::mem::take(&mut self.texts);
        self.run_and_paint(width, height, pixels_per_point, input_events, |ui| {
            let mut state = UiFrameState {
                events: &mut events,
                sliders: &mut sliders,
                seen_sliders: &mut seen_sliders,
                texts: &mut texts,
                seen_texts: &mut seen_texts,
            };
            match view {
                // A panel anchors itself; a bare root sits at the top-left with a margin.
                View::Panel { .. } => render_view(ui, view, &mut state),
                other => {
                    let ctx = ui.ctx().clone();
                    egui::Area::new(egui::Id::new("functor_ui_root"))
                        .anchor(egui::Align2::LEFT_TOP, egui::vec2(MARGIN, MARGIN))
                        .interactable(contains_interactive(other))
                        .show(&ctx, |ui| render_view(ui, other, &mut state));
                }
            }
        });
        // A slot that left the view is a different widget if it ever returns
        // (positional identity) — drop its buffer rather than resurrect it.
        sliders.retain(|slot, _| seen_sliders.contains(slot));
        texts.retain(|slot, _| seen_texts.contains(slot));
        self.sliders = sliders;
        self.texts = texts;
        UiOutput {
            events,
            wants_pointer: self.ctx.egui_wants_pointer_input(),
            wants_keyboard: self.ctx.egui_wants_keyboard_input(),
        }
    }

    /// Run one egui frame (building the UI with `build`), tessellate, paint to the
    /// bound framebuffer, and restore the GL state the 3D path expects. egui's
    /// fonts only exist *during* `run`, so all layout/measurement happens in here.
    fn run_and_paint(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        events: Vec<egui::Event>,
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
            events,
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

/// Turns per-frame [`PointerState`] snapshots into egui input events: a move
/// whenever the pointer is on the overlay, and a primary-button event on the
/// press/release EDGE (egui needs both edges to register a click). One bridge
/// per egui context — it holds the previous button state to find the edges, so
/// every interactive pass (the scrubber, the game UI) synthesizes identical
/// input from the same shell state.
#[derive(Default)]
pub struct PointerBridge {
    last_primary_down: bool,
    /// Last on-overlay position (points) — where a press is released if the
    /// pointer leaves the overlay while held.
    last_pos: Option<egui::Pos2>,
}

impl PointerBridge {
    /// This frame's egui events. `pixels_per_point` maps the snapshot's
    /// physical-pixel position into egui's point space. While `pos` is `None`
    /// (cursor captured / off-canvas) button state is still tracked — a press
    /// begun off-overlay is swallowed, never replayed. On the transition OFF
    /// the overlay, a press still held from on-overlay is RELEASED at its
    /// last position (egui must not hold a stuck press), followed by
    /// `PointerGone` so hover state clears.
    pub fn events(&mut self, pointer: PointerState, pixels_per_point: f32) -> Vec<egui::Event> {
        let mut events = Vec::new();
        match pointer.pos {
            Some((px, py)) => {
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
                self.last_pos = Some(pos);
            }
            None => {
                if let Some(pos) = self.last_pos.take() {
                    // Leaving the overlay: flush a held press, then the
                    // pointer is gone.
                    if self.last_primary_down {
                        events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    events.push(egui::Event::PointerGone);
                }
            }
        }
        self.last_primary_down = pointer.primary_down;
        events
    }
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
    /// Forward-ghosting (docs/time-travel.md T6d) toggle: composite the ~window-s
    /// future into a strobe. Interactive companion to the `--ghost` launch flag.
    pub ghost_on: bool,
    /// Divisions composited by the ghost (1..=8, the compositor `MAX_GHOST` cap).
    pub ghost_divisions: usize,
    /// The forward window in seconds; `dt = ghost_window / ghost_divisions`.
    pub ghost_window: f32,
    /// Scene-diff preview mode (docs/time-travel.md T6): trail dots and/or
    /// scene-space strobe copies. Interactive companion to the
    /// `--trajectory`/`--strobe` launch flags.
    pub preview_mode: crate::trajectory::PreviewMode,
}

/// A control the user activated in the scrubber this frame.
// No `Eq`: `SetGhostWindow` carries an `f32`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScrubberAction {
    TogglePause,
    /// Non-destructive scrub to a rendered frame (dragging the timeline).
    SeekTo(u64),
    Step,
    /// Toggle forward-ghosting on/off (the `ghost` checkbox).
    SetGhost(bool),
    /// Set the ghost's forward divisions (clamped 1..=8 by the shell).
    SetGhostDivisions(usize),
    /// Set the ghost's forward window in seconds.
    SetGhostWindow(f32),
    /// Set the scene-diff preview mode (the `preview:` cycle button).
    SetPreviewMode(crate::trajectory::PreviewMode),
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
    pointer: PointerBridge,
}

impl Scrubber {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .expect("failed to create egui_glow painter for the scrubber");
        Self {
            ctx: egui::Context::default(),
            painter,
            gl,
            pointer: PointerBridge::default(),
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

        // Synthesize egui pointer events from the shell's current pointer state
        // (a move every frame, button events on the press/release edge).
        let events = self.pointer.events(pointer, pixels_per_point);

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

                            // Forward-ghosting controls (docs/time-travel.md T6d):
                            // an in-app companion to the `--ghost` launch flag.
                            ui.separator();
                            let mut ghost_on = state.ghost_on;
                            if ui.checkbox(&mut ghost_on, "ghost").changed() {
                                action = Some(ScrubberAction::SetGhost(ghost_on));
                            }
                            // Divisions (1..=8, the compositor MAX_GHOST cap) and
                            // the forward window in seconds (dt = window / divisions).
                            let mut divisions = state.ghost_divisions.clamp(1, 8);
                            if ui
                                .add(
                                    egui::DragValue::new(&mut divisions)
                                        .range(1..=8)
                                        .prefix("÷"),
                                )
                                .changed()
                            {
                                action = Some(ScrubberAction::SetGhostDivisions(divisions));
                            }
                            let mut window = state.ghost_window;
                            if ui
                                .add(
                                    egui::Slider::new(&mut window, 0.5..=5.0).suffix("s"),
                                )
                                .changed()
                            {
                                action = Some(ScrubberAction::SetGhostWindow(window));
                            }

                            // Scene-diff preview (docs/time-travel.md T6): a
                            // compact cycle button (off → trail → strobe → both)
                            // — one widget until the scrubber declutter pass
                            // gives previews a proper popover.
                            ui.separator();
                            if ui
                                .button(format!("preview: {}", state.preview_mode.label()))
                                .clicked()
                            {
                                action = Some(ScrubberAction::SetPreviewMode(
                                    state.preview_mode.next(),
                                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn on(x: f32, y: f32, down: bool) -> PointerState {
        PointerState {
            pos: Some((x, y)),
            primary_down: down,
        }
    }

    fn off(down: bool) -> PointerState {
        PointerState {
            pos: None,
            primary_down: down,
        }
    }

    #[test]
    fn hover_emits_only_a_move() {
        let mut bridge = PointerBridge::default();
        let events = bridge.events(on(30.0, 40.0, false), 1.0);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], egui::Event::PointerMoved(p) if p == egui::pos2(30.0, 40.0)));
    }

    #[test]
    fn press_and_release_emit_button_edges_and_holds_do_not() {
        let mut bridge = PointerBridge::default();

        let press = bridge.events(on(5.0, 5.0, true), 1.0);
        assert_eq!(press.len(), 2);
        // The button event carries the position egui hit-tests the click at.
        assert!(matches!(
            press[1],
            egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, pos, .. }
                if pos == egui::pos2(5.0, 5.0)
        ));

        // Held: moves only, no repeated button event.
        let held = bridge.events(on(6.0, 5.0, true), 1.0);
        assert_eq!(held.len(), 1);
        assert!(matches!(held[0], egui::Event::PointerMoved(_)));

        let release = bridge.events(on(6.0, 5.0, false), 1.0);
        assert_eq!(release.len(), 2);
        assert!(matches!(
            release[1],
            egui::Event::PointerButton { pressed: false, .. }
        ));
    }

    #[test]
    fn captured_cursor_emits_nothing_but_still_tracks_button_state() {
        let mut bridge = PointerBridge::default();

        // Press while the cursor is captured (off-overlay): swallowed…
        assert!(bridge.events(off(true), 1.0).is_empty());

        // …and not replayed when the pointer arrives already-down: egui never
        // sees a press edge, so the drag-in can't click anything.
        let arrive = bridge.events(on(10.0, 10.0, true), 1.0);
        assert_eq!(arrive.len(), 1);
        assert!(matches!(arrive[0], egui::Event::PointerMoved(_)));

        // The release edge on-overlay is still delivered.
        let release = bridge.events(on(10.0, 10.0, false), 1.0);
        assert_eq!(release.len(), 2);
        assert!(matches!(
            release[1],
            egui::Event::PointerButton { pressed: false, .. }
        ));
    }

    #[test]
    fn leaving_the_overlay_mid_press_releases_at_the_last_position() {
        let mut bridge = PointerBridge::default();
        bridge.events(on(5.0, 5.0, true), 1.0); // press on-overlay

        // Pointer leaves (cursor captured / off-canvas) while still held:
        // egui must not keep a stuck press — release where it last was,
        // then the pointer is gone.
        let leave = bridge.events(off(true), 1.0);
        assert_eq!(leave.len(), 2);
        assert!(matches!(
            leave[0],
            egui::Event::PointerButton { pressed: false, pos, .. } if pos == egui::pos2(5.0, 5.0)
        ));
        assert!(matches!(leave[1], egui::Event::PointerGone));

        // Staying off-overlay emits nothing further.
        assert!(bridge.events(off(true), 1.0).is_empty());
        assert!(bridge.events(off(false), 1.0).is_empty());
    }

    #[test]
    fn positions_scale_physical_pixels_to_points() {
        let mut bridge = PointerBridge::default();
        let events = bridge.events(on(200.0, 100.0, false), 2.0);
        assert!(matches!(events[0], egui::Event::PointerMoved(p) if p == egui::pos2(100.0, 50.0)));
    }

    /// Run one HEADLESS egui frame (no painter/GL — only painting needs a
    /// context) over a view, returning the widget events it produced.
    fn run_widget_frame(
        ctx: &egui::Context,
        input: Vec<egui::Event>,
        texts: &mut std::collections::HashMap<u32, TextBuffer>,
        view: &View,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let mut sliders = std::collections::HashMap::new();
        let mut seen_sliders = std::collections::HashSet::new();
        let mut seen_texts = std::collections::HashSet::new();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events: input,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            let mut state = UiFrameState {
                events: &mut events,
                sliders: &mut sliders,
                seen_sliders: &mut seen_sliders,
                texts,
                seen_texts: &mut seen_texts,
            };
            render_view(ui, view, &mut state);
        });
        events
    }

    /// The risky U4 path, exercised headlessly end to end: click to focus the
    /// field, type through egui's `TextEdit`, and check both reconciliation
    /// rules — the emitted echo leaves the buffer alone, a programmatic model
    /// change resets it.
    #[test]
    fn typing_into_a_focused_text_input_emits_and_reconciles() {
        let ctx = egui::Context::default();
        let mut texts = std::collections::HashMap::new();
        let view = |value: &str| View::TextInput {
            slot: 0,
            value: value.to_string(),
        };

        // Warmup: egui hit-tests input against the PREVIOUS frame's widget
        // rects, so the field must exist for a frame before a click can land.
        assert!(run_widget_frame(&ctx, Vec::new(), &mut texts, &view("cube")).is_empty());

        // Then click inside the field (it renders at the top-left of the
        // root Ui, rect ~280x19) to focus it. egui needs both button edges.
        let click_pos = egui::pos2(30.0, 12.0);
        let press = vec![
            egui::Event::PointerMoved(click_pos),
            egui::Event::PointerButton {
                pos: click_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ];
        assert!(run_widget_frame(&ctx, press, &mut texts, &view("cube")).is_empty());
        let release = vec![egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }];
        assert!(run_widget_frame(&ctx, release, &mut texts, &view("cube")).is_empty());
        assert!(ctx.egui_wants_keyboard_input(), "click should focus the field");

        // Frame 3: a typed character (the shell's Char event, lowered the
        // same way draw_view lowers it) must emit exactly one TextChanged.
        let mut typed = Vec::new();
        keyboard_to_egui(UiKeyboardEvent::Char('s'), &mut typed);
        let events = run_widget_frame(&ctx, typed, &mut texts, &view("cube"));
        assert_eq!(events.len(), 1);
        let UiEventKind::TextChanged(new_text) = &events[0].kind else {
            panic!("expected TextChanged, got {:?}", events[0].kind);
        };
        assert!(new_text.contains('s'), "typed char should land: {new_text:?}");
        let emitted = new_text.clone();

        // Frame 4 — the ECHO: the model comes back equal to what we emitted;
        // the buffer must be left alone and nothing re-emitted.
        assert!(run_widget_frame(&ctx, Vec::new(), &mut texts, &view(&emitted)).is_empty());
        assert_eq!(texts.get(&0).unwrap().live, emitted);

        // Frame 5 — PROGRAMMATIC: a model value that isn't our echo (a game
        // reset) snaps the buffer to it.
        assert!(run_widget_frame(&ctx, Vec::new(), &mut texts, &view("reset")).is_empty());
        assert_eq!(texts.get(&0).unwrap().live, "reset");
        assert_eq!(texts.get(&0).unwrap().last_emitted, "reset");
    }

    /// Backspace through the synthesized edit-key pair deletes in a focused
    /// field — the editing-key half of the keyboard path.
    #[test]
    fn backspace_edits_a_focused_text_input() {
        let ctx = egui::Context::default();
        let mut texts = std::collections::HashMap::new();
        let view = View::TextInput {
            slot: 0,
            value: "ab".to_string(),
        };

        // Warmup so the click can hit (see the sibling test), then focus
        // with a click; End pins the cursor after the last char.
        let _ = run_widget_frame(&ctx, Vec::new(), &mut texts, &view);
        let click_pos = egui::pos2(30.0, 12.0);
        let mut input = vec![
            egui::Event::PointerMoved(click_pos),
            egui::Event::PointerButton {
                pos: click_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: click_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ];
        keyboard_to_egui(UiKeyboardEvent::Edit(UiEditKey::End), &mut input);
        let _ = run_widget_frame(&ctx, input, &mut texts, &view);

        let mut backspace = Vec::new();
        keyboard_to_egui(UiKeyboardEvent::Edit(UiEditKey::Backspace), &mut backspace);
        let events = run_widget_frame(&ctx, backspace, &mut texts, &view);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].kind,
            UiEventKind::TextChanged(s) if s == "a"
        ));
    }

}
