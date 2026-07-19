//! The native webview overlay: renders the game's `webview(model)` HTML/CSS
//! tree over the 3D frame, and routes pointer input back as slot-stamped
//! [`UiEvent`]s (docs/ui-interaction.md, the webview flavor).
//!
//! Pipeline (see `~/notes` html-css-ui research → CLAUDE.md "webview"):
//! the producer's [`HtmlNode`] tree is serialized to an HTML string, parsed by
//! **blitz** (`blitz-dom`: Stylo styles + Taffy layout + Parley text), painted
//! headlessly on the CPU (`anyrender_vello_cpu`) into a premultiplied-RGBA
//! buffer, uploaded as a GL texture, and composited as a fullscreen quad with
//! `(ONE, ONE_MINUS_SRC_ALPHA)` blending. No wgpu anywhere — the CPU painter
//! keeps the whole path on the existing glow stack.
//!
//! **Threaded**: the CPU-heavy half (parse → resolve → paint, plus blitz event
//! processing) runs on a dedicated worker thread that OWNS the blitz document,
//! the CPU rasterizer, and the shared `FontContext` — blitz types never cross
//! threads. Only plain data crosses two mpsc channels: per-frame inputs
//! (HTML-on-change, viewport, the pointer sample, the animation clock) go in;
//! completed RGBA frames, [`UiEvent`]s, the `wants_pointer` latch, and the
//! interactive-element rects come back. The frame loop drains the return
//! channel non-blocking and uploads a texture only when a new frame arrived —
//! it never blocks on blitz, so a slow repaint (debug builds are ~200×
//! release) shows up as overlay *latency*, not frame stalls. The one
//! exception: the FIRST frame the webview ever activates blocks (bounded,
//! once per overlay lifetime) for the worker's first paint, so
//! `--capture-frame` runs deterministically include the overlay instead of
//! racing it.
//!
//! Retained, not immediate: the DOM is rebuilt only when the serialized HTML
//! actually changes (one string compare per frame on the main thread), and
//! re-rasterized only when something visible could have changed (new HTML,
//! hover transitions, clicks, resize) — the dirty logic lives in the worker.
//! CSS animations/transitions are ticked: while `doc.is_animating()` reports
//! live @keyframes or transitions the worker repaints every cycle under our
//! `resolve(t)` clock (bounded, and off the frame loop).
//!
//! Interaction mirrors the egui overlay: pointer events feed blitz's
//! `EventDriver`, which synthesizes DOM semantics (click = press+release on
//! the same element, hover chains for `:hover` CSS). A DOM `click`/`input`
//! event walks the bubble chain for the nearest `data-fn-click` /
//! `data-fn-input` attribute — the handler slot the `Attr.onClick` /
//! `Attr.onInput` builder stamped — and comes back as a [`UiEvent`] the shell
//! folds through `GameProducer::webview_event`. Press arbitration
//! ([`WebviewOverlay::hit_interactive_css`]) is a point-in-rect test against
//! the worker's latest snapshot of interactive-element boxes, so the run
//! loop's synchronous press decision never waits on the worker.
//!
//! Keyboard: clicking an `<input>` focuses it (blitz's pointer handling),
//! which flips the `wants_keyboard` latch the shell routes on (the
//! `Ui.textInput` focus gate). The shell's keystrokes cross the channel as
//! plain [`WebviewKey`]s, are lowered to blitz key events
//! (`webview_keys::lower_key`), and type into the focused field; the DOM
//! `input` events they generate come back as `TextChanged` [`UiEvent`]s.
//! Because a controlled input's keystroke re-renders the HTML (a FRESH
//! document), focus is restored across rebuilds by the focused element's
//! `data-fn-input` slot, caret at the end (the documented `Ui.textInput`
//! rule; per-caret preservation for identity round-trips is part of the
//! DOM-reconciliation follow-up, docs/todo.md § Webview). Like
//! `wants_pointer`, the latch is a worker-cycle-stale snapshot: keys typed
//! in the frame(s) between clicking the field and the worker's report still
//! go to the game (and vice versa after Escape) — the accepted async-overlay
//! tradeoff. Not wired (follow-ups): IME composition (CJK/dead keys),
//! modifier combos (shift-selection, select-all, clipboard), and
//! `placeholder` rendering (blitz has none).

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, Document, DocumentConfig, EventDriver, EventHandler, FontContext};
use blitz_html::HtmlDocument;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, DomEvent, DomEventData, EventState, MouseEventButton,
    MouseEventButtons, PointerCoords, UiEvent as BlitzUiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use functor_runtime_common::ui::{PointerState, UiEvent, UiEventKind};
use glow::HasContext;
use markup5ever::LocalName;

use crate::webview_keys::{lower_key, WebviewKey};

/// How long the main thread will block for the worker's FIRST frame when the
/// webview first activates (capture determinism — see the module doc).
/// Generous because a debug-build retina paint is ~600ms; it is paid at most
/// once per overlay lifetime, and only if the worker is slower than the loop.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// What one webview frame produced: interactions to fold through `update`,
/// whether the pointer is over an interactive element (the shell's
/// click-arbitration latch, like `ui_wants_pointer`), and whether a text
/// input is focused (the shell's keyboard-routing latch, like
/// `ui_wants_keyboard`).
pub struct WebviewOutput {
    pub events: Vec<UiEvent>,
    pub wants_pointer: bool,
    pub wants_keyboard: bool,
}

impl WebviewOutput {
    fn empty() -> Self {
        WebviewOutput {
            events: Vec::new(),
            wants_pointer: false,
            wants_keyboard: false,
        }
    }
}

// ── The main↔worker protocol (plain data only — no blitz types) ────────────

/// The HTML side of a per-frame input. The main thread keeps the
/// change-detection string compare, so an idle webview sends `Unchanged`
/// (no allocation) and the worker reparses only on `Set`.
enum HtmlMsg {
    Unchanged,
    /// `Arc<str>`: one allocation shared between the worker's message and
    /// the main thread's change-detection cache.
    Set(Arc<str>),
    /// The `webview` hook disappeared — drop the retained DOM.
    Clear,
}

/// One frame's worth of input, main → worker.
struct WorkerInput {
    html: HtmlMsg,
    fb_width: u32,
    fb_height: u32,
    dpi_scale: f32,
    /// Pointer sample in framebuffer pixels (the overlays' shared space).
    pointer: PointerState,
    /// Keyboard events for the focused text input this frame, in order
    /// (collected by the shell only while `wants_keyboard` — the focus gate).
    keys: Vec<WebviewKey>,
    /// The animation clock blitz `resolve(t)` ticks on (the shell owns it).
    clock: f64,
    /// Bumped on `Clear`; outputs stamped with an older epoch are stale
    /// results from a previous document and are discarded by the main thread.
    epoch: u64,
}

/// A completed CPU-painted frame, worker → main (only when repainted).
struct WorkerFrame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    generation: u64,
}

/// One work cycle's results, worker → main. Sent only when something changed
/// (a repaint, events, a latch flip, or moved rects) so an idle webview stays
/// idle on both sides.
struct WorkerOutput {
    epoch: u64,
    events: Vec<UiEvent>,
    wants_pointer: bool,
    /// Whether the document's focused node is an editable text input — the
    /// shell's keyboard-routing latch (`webview_wants_keyboard`).
    wants_keyboard: bool,
    /// CSS-px boxes `(x, y, w, h)` of elements carrying `data-fn-click` /
    /// `data-fn-input`, from the resolved layout. `None` = unchanged since
    /// the last send (the main thread keeps its snapshot).
    interactive_rects: Option<Vec<(f32, f32, f32, f32)>>,
    frame: Option<WorkerFrame>,
}

/// Collects DOM events out of blitz's event driver, resolving the bubble
/// chain to handler slots (`data-fn-click` / `data-fn-input`).
struct EventCollector {
    events: Vec<UiEvent>,
}

impl EventHandler for &mut EventCollector {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _state: &mut EventState,
    ) {
        let doc = doc.inner();
        let slot_on_chain = |attr: &str| {
            chain.iter().find_map(|&node_id| {
                doc.get_node(node_id)?
                    .attr(LocalName::from(attr))?
                    .parse::<u32>()
                    .ok()
            })
        };
        match &event.data {
            DomEventData::Click(_) => {
                if let Some(slot) = slot_on_chain("data-fn-click") {
                    self.events.push(UiEvent {
                        slot,
                        kind: UiEventKind::Clicked,
                    });
                }
            }
            DomEventData::Input(input) => {
                if let Some(slot) = slot_on_chain("data-fn-input") {
                    self.events.push(UiEvent {
                        slot,
                        kind: UiEventKind::TextChanged(input.value.clone()),
                    });
                }
            }
            _ => {}
        }
    }
}

pub struct WebviewOverlay {
    gl: Arc<glow::Context>,
    program: glow::Program,
    vao: glow::VertexArray,
    texture: glow::Texture,
    /// Input channel to the worker; dropped first in `Drop` so the worker's
    /// blocking `recv` errors out and the thread exits.
    tx: Option<Sender<WorkerInput>>,
    rx: Receiver<WorkerOutput>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// The serialized HTML last sent — the change-detection key (an idle
    /// webview costs one string compare per frame, as before).
    last_html: Option<Arc<str>>,
    /// Whether a `webview` hook is currently live (drives Clear on removal).
    active: bool,
    epoch: u64,
    /// One-shot latch, once per overlay LIFETIME: the first frame after the
    /// webview first activates blocks (bounded) for the worker's first paint
    /// — capture determinism (module doc). Never reset on deactivation: a
    /// model-toggled webview (a menu) must not stall the frame loop on every
    /// re-show; its reappearance just lands a cycle later.
    waited_first_frame: bool,
    /// Latest latch/rect snapshot received from the worker (≤1 frame stale;
    /// buttons that keep their place across re-renders — the repeat-click
    /// case — test identically to the old live hit-test).
    wants_pointer: bool,
    wants_keyboard: bool,
    interactive_rects: Vec<(f32, f32, f32, f32)>,
    /// Size of the currently uploaded texture (0 = nothing to composite).
    tex_w: u32,
    tex_h: u32,
    /// Generation of the currently uploaded texture — the upload gate.
    uploaded_generation: u64,
}

const VERTEX_SRC: &str = r#"
    out vec2 vUv;
    void main() {
        // A fullscreen strip from gl_VertexID — no VBO. v flipped: the CPU
        // painter's buffer is top-down, GL's texture space bottom-up.
        vec2 pos = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
        vUv = vec2(pos.x, 1.0 - pos.y);
        gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);
    }
"#;

const FRAGMENT_SRC: &str = r#"
    in vec2 vUv;
    out vec4 fragColor;
    uniform sampler2D uTex;
    void main() {
        // Premultiplied alpha straight through; blend state does the rest.
        fragColor = texture(uTex, vUv);
    }
"#;

impl WebviewOverlay {
    pub fn new(gl: Arc<glow::Context>, shader_version: &str) -> Self {
        let (program, vao, texture) = unsafe {
            let program = gl.create_program().expect("webview: create_program");
            let compile = |kind, src: &str| {
                let shader = gl.create_shader(kind).expect("webview: create_shader");
                gl.shader_source(shader, &format!("{shader_version}\n{src}"));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    panic!("webview shader: {}", gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shader
            };
            let vs = compile(glow::VERTEX_SHADER, VERTEX_SRC);
            let fs = compile(glow::FRAGMENT_SHADER, FRAGMENT_SRC);
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                panic!("webview link: {}", gl.get_program_info_log(program));
            }
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            let vao = gl.create_vertex_array().expect("webview: create_vao");
            let texture = gl.create_texture().expect("webview: create_texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            (program, vao, texture)
        };
        let (tx, worker_rx) = std::sync::mpsc::channel::<WorkerInput>();
        let (worker_tx, rx) = std::sync::mpsc::channel::<WorkerOutput>();
        let worker = std::thread::Builder::new()
            .name("webview-render".into())
            .spawn(move || worker_loop(worker_rx, worker_tx))
            .expect("webview: spawn worker");
        WebviewOverlay {
            gl,
            program,
            vao,
            texture,
            tx: Some(tx),
            rx,
            worker: Some(worker),
            last_html: None,
            active: false,
            epoch: 0,
            waited_first_frame: false,
            wants_pointer: false,
            wants_keyboard: false,
            interactive_rects: Vec::new(),
            tex_w: 0,
            tex_h: 0,
            uploaded_generation: 0,
        }
    }

    /// Synchronous hit-test in CSS px (== window points): is the pointer over
    /// an interactive element? The shell's press arbitration uses this
    /// instead of the `wants_pointer` latch — a point-in-rect test against
    /// the worker's latest interactive-rect snapshot, so it never blocks on
    /// the worker. The rects are ≤1 worker cycle stale, but the repeat-click
    /// case the old live hit-test existed for (a stationary click after a
    /// model-driven re-render) keeps its buttons in place, so the rect test
    /// resolves it identically.
    pub fn hit_interactive_css(&self, x: f32, y: f32) -> bool {
        self.interactive_rects
            .iter()
            .any(|&(rx, ry, w, h)| x >= rx && x < rx + w && y >= ry && y < ry + h)
    }

    /// Run one webview frame: forward inputs to the worker, drain its
    /// results non-blocking, upload the newest frame (if any) as the overlay
    /// texture, and composite the quad. `pointer.pos` is in framebuffer
    /// pixels (the egui overlays' space); the worker divides by `dpi_scale`
    /// for blitz's CSS px. `keys` is this frame's keyboard input for the
    /// focused text field, in order (the shell collects it only while
    /// `wants_keyboard` — the focus gate). `clock` is the shell's frame time
    /// (`tts`, seconds) — the clock CSS animations tick on, so `--fixed-time`
    /// pins webview poses too (deterministic captures) and pausing freezes
    /// the overlay's animations coherently with the game.
    pub fn frame(
        &mut self,
        fb_width: u32,
        fb_height: u32,
        dpi_scale: f32,
        html: Option<&str>,
        pointer: PointerState,
        keys: Vec<WebviewKey>,
        clock: f64,
    ) -> WebviewOutput {
        let Some(html) = html else {
            // Hook absent (or nothing built yet): tell the worker to drop the
            // retained DOM so a deleted `webview` clears the overlay (the
            // `ui` reload rule), and reset the main-side snapshot NOW — a
            // stale rect must not capture a press for a dead overlay.
            if self.active {
                self.active = false;
                self.last_html = None;
                self.wants_pointer = false;
                self.wants_keyboard = false;
                self.interactive_rects = Vec::new();
                self.tex_w = 0;
                self.tex_h = 0;
                self.epoch += 1;
                if let Some(tx) = &self.tx {
                    let _ = tx.send(WorkerInput {
                        html: HtmlMsg::Clear,
                        fb_width,
                        fb_height,
                        dpi_scale,
                        pointer: PointerState::default(),
                        keys: Vec::new(),
                        clock,
                        epoch: self.epoch,
                    });
                }
            }
            // Discard in-flight results from the previous document.
            for _ in self.rx.try_iter() {}
            return WebviewOutput::empty();
        };
        if fb_width == 0 || fb_height == 0 {
            return WebviewOutput::empty();
        }

        // ── Send this frame's input (HTML only when changed) ───────────────
        self.active = true;
        let html_msg = if self.last_html.as_deref() == Some(html) {
            HtmlMsg::Unchanged
        } else {
            let shared: Arc<str> = Arc::from(html);
            self.last_html = Some(shared.clone());
            HtmlMsg::Set(shared)
        };
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerInput {
                html: html_msg,
                fb_width,
                fb_height,
                dpi_scale,
                pointer,
                keys,
                clock,
                epoch: self.epoch,
            });
        }

        // ── Drain worker results (non-blocking, except the first frame) ────
        let mut outputs: Vec<WorkerOutput> = self.rx.try_iter().collect();
        if !self.waited_first_frame {
            // Block (bounded) for the worker's FIRST paint of this document,
            // so captures and the overlay's appearance don't race the worker.
            self.waited_first_frame = true;
            let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
            while !outputs
                .iter()
                .any(|o| o.epoch == self.epoch && o.frame.is_some())
            {
                let timeout = deadline.saturating_duration_since(Instant::now());
                if timeout.is_zero() {
                    break;
                }
                match self.rx.recv_timeout(timeout) {
                    Ok(out) => outputs.push(out),
                    Err(_) => break,
                }
            }
        }
        let mut events: Vec<UiEvent> = Vec::new();
        let mut latest_frame: Option<WorkerFrame> = None;
        for out in outputs {
            if out.epoch != self.epoch {
                continue; // a stale result from before the last Clear
            }
            events.extend(out.events);
            self.wants_pointer = out.wants_pointer;
            self.wants_keyboard = out.wants_keyboard;
            if let Some(rects) = out.interactive_rects {
                self.interactive_rects = rects;
            }
            if let Some(frame) = out.frame {
                latest_frame = Some(frame); // messages arrive in generation order
            }
        }

        // ── Upload the newest frame (only on a NEW generation) ─────────────
        if let Some(frame) = latest_frame.filter(|f| f.generation > self.uploaded_generation) {
            self.uploaded_generation = frame.generation;
            unsafe {
                let gl = &self.gl;
                gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    frame.width as i32,
                    frame.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&frame.rgba)),
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            // The texture keeps the frame's OWN size: after a resize it
            // stretches over the quad for the cycle or two until the
            // newly-sized frame lands.
            self.tex_w = frame.width;
            self.tex_h = frame.height;
        }

        // ── Composite the overlay quad ──────────────────────────────────────
        if self.tex_w > 0 {
            unsafe {
                let gl = &self.gl;
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::SCISSOR_TEST);
                gl.enable(glow::BLEND);
                // The CPU painter emits PREMULTIPLIED alpha.
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.use_program(Some(self.program));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                gl.bind_vertex_array(Some(self.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.use_program(None);
                // Restore the slate the 3D path expects (the
                // `restore_gl_after_egui` convention).
                gl.disable(glow::BLEND);
                gl.enable(glow::DEPTH_TEST);
            }
        }

        WebviewOutput {
            events,
            wants_pointer: self.wants_pointer,
            wants_keyboard: self.wants_keyboard,
        }
    }
}

impl Drop for WebviewOverlay {
    fn drop(&mut self) {
        // Close the input channel; the worker's blocking recv errors and the
        // thread exits its loop.
        self.tx = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// ── The worker: owns all blitz state; nothing here touches GL ──────────────

/// Everything the worker thread owns — the blitz document, the CPU
/// rasterizer, the shared font context, and the dirty/pointer bookkeeping
/// that used to live on [`WebviewOverlay`] directly.
struct WorkerState {
    doc: Option<BaseDocument>,
    renderer: Option<VelloCpuImageRenderer>,
    /// Framebuffer size the current `renderer` was created for.
    renderer_size: Option<(u32, u32)>,
    /// One font context shared across reparses. A fresh context per document
    /// re-enumerates system fonts — ~55ms per model-driven re-render (~30ms
    /// release), measured; sharing drops the reparse to single-digit ms.
    font_ctx: FontContext,
    /// Repaint latch: HTML changed, hover moved, a click landed, resized.
    dirty: bool,
    /// Pointer state last cycle, to synthesize press/release edges.
    pointer_was_down: bool,
    hover_pos: Option<(f32, f32)>,
    /// The current viewport (from the latest input); a change re-flows.
    view_w: u32,
    view_h: u32,
    view_scale: f32,
    clock: f64,
    epoch: u64,
    generation: u64,
    /// Last-sent snapshots, so an idle cycle sends nothing at all.
    sent_rects: Vec<(f32, f32, f32, f32)>,
    sent_wants_pointer: bool,
    sent_wants_keyboard: bool,
}

impl WorkerState {
    fn new() -> Self {
        WorkerState {
            doc: None,
            renderer: None,
            renderer_size: None,
            font_ctx: FontContext::default(),
            dirty: false,
            pointer_was_down: false,
            hover_pos: None,
            view_w: 0,
            view_h: 0,
            view_scale: 1.0,
            clock: 0.0,
            epoch: 0,
            generation: 0,
            sent_rects: Vec::new(),
            sent_wants_pointer: false,
            sent_wants_keyboard: false,
        }
    }

    /// Run one whole batch of inputs (everything queued while the previous
    /// cycle worked): reconcile the DOM once against the batch's LAST HTML
    /// directive — intermediate documents would never be painted, so parsing
    /// them would only let a backlog starve the worker (e.g. a debug build
    /// with per-frame re-renders) — then feed every pointer/keyboard sample
    /// of the current epoch, in order, so no press/release edge (or
    /// keystroke) is coalesced away. The expensive resolve+paint happens
    /// once, in [`WorkerState::finish_cycle`].
    fn run_cycle(&mut self, batch: Vec<WorkerInput>) -> Option<WorkerOutput> {
        let last = batch.last().expect("batch is non-empty");
        let epoch = last.epoch;
        self.epoch = epoch;
        self.clock = last.clock;
        let scale = if last.dpi_scale > 0.0 {
            last.dpi_scale
        } else {
            1.0
        };
        let viewport_changed = self.view_w != last.fb_width
            || self.view_h != last.fb_height
            || self.view_scale != scale;
        self.view_w = last.fb_width;
        self.view_h = last.fb_height;
        self.view_scale = scale;

        // Split the batch: the last HTML directive wins; pointer/keyboard
        // samples stamped with an OLDER epoch belong to a document Cleared
        // mid-batch — the main thread discards their results anyway, so
        // don't synthesize edges from them on the new document.
        let mut html_action: Option<HtmlMsg> = None;
        let mut samples: Vec<(PointerState, Vec<WebviewKey>)> = Vec::with_capacity(batch.len());
        for input in batch {
            if !matches!(input.html, HtmlMsg::Unchanged) {
                html_action = Some(input.html);
            }
            if input.epoch == epoch {
                samples.push((input.pointer, input.keys));
            }
        }

        // ── Reconcile the DOM (prototype: full reparse on change) ─────────
        match html_action {
            Some(HtmlMsg::Clear) => {
                self.doc = None;
                self.pointer_was_down = false;
                self.hover_pos = None;
                self.dirty = false;
                // The main thread cleared ITS snapshot on Clear — reset the
                // sent-caches too, or an identical re-shown UI would compare
                // equal and never be re-sent (dead press arbitration).
                self.sent_rects = Vec::new();
                self.sent_wants_pointer = false;
                self.sent_wants_keyboard = false;
                return None;
            }
            Some(HtmlMsg::Set(html)) => {
                // Focus survival across re-renders: every keystroke in a
                // controlled input changes the model → new HTML → a FRESH
                // document, which would drop focus (and the wants-keyboard
                // latch) after the first character. Remember the focused
                // element's handler slot — the stable identity across
                // rebuilds — and re-focus the matching node below.
                let focused_slot = self.doc.as_ref().and_then(focused_input_slot);
                let viewport =
                    Viewport::new(self.view_w, self.view_h, scale, ColorScheme::Dark);
                let mut doc = HtmlDocument::from_html(
                    &html,
                    DocumentConfig {
                        viewport: Some(viewport),
                        // Shared across reparses — a fresh context re-enumerates
                        // system fonts (tens of ms) on every re-render.
                        font_ctx: Some(self.font_ctx.clone()),
                        ..Default::default()
                    },
                )
                .into_inner();
                // Anchor CSS animations at game-time ZERO before resolving at
                // the current clock: blitz starts an animation at the
                // timestamp of the resolve that first sees it, so anchoring a
                // fresh document at 0 makes the animation timeline the GAME
                // clock itself. Without this, every reparse (a model-driven
                // re-render) restarts running @keyframes mid-loop, and
                // `--fixed-time T` — where the first resolve already happens
                // at T — pins animations to their from-pose instead of their
                // pose at T. Known constraint [xreview]: a NON-infinite
                // (one-shot) animation on an element that appears mid-session
                // resolves as already-finished under this anchor and never
                // plays — per-animation start-time preservation needs the DOM
                // reconciliation follow-up (docs/todo.md § Webview). Game
                // HUDs should use `infinite` loops + transitions meanwhile.
                doc.resolve(0.0);
                // A fresh document has NO layout until resolved — the hover
                // re-establishing move below would hit-test nothing, leaving
                // `wants_pointer` false until the mouse physically moves, and
                // the next stationary click would recapture the cursor instead
                // of pressing the button (the repeated-click repro). Resolve
                // now so same-cycle events see real geometry.
                doc.resolve(self.clock);
                // Re-focus the input the OLD document had focused (matched by
                // its `data-fn-input` slot), caret at the end — the
                // documented controlled-input semantics ("an update that
                // transforms the text resets the cursor to the end"). A
                // vanished slot (the input left the tree) just drops focus.
                if let Some(id) = focused_slot.and_then(|slot| find_input_by_slot(&doc, slot)) {
                    doc.set_focus_to(id);
                    let mut sink = EventCollector { events: Vec::new() };
                    let mut driver = EventDriver::new(&mut doc as &mut dyn Document, &mut sink);
                    for ev in lower_key(&WebviewKey::End) {
                        driver.handle_ui_event(ev);
                    }
                }
                self.doc = Some(doc);
                self.dirty = true;
                // Re-establish hover on the fresh DOM: clearing `hover_pos`
                // makes the next pointer sample look moved, so the move
                // re-synthesizes below even for a stationary cursor (`:hover`
                // and wants_pointer would otherwise go stale until the mouse
                // physically moves). [xreview]
                self.hover_pos = None;
                self.pointer_was_down = false;
            }
            Some(HtmlMsg::Unchanged) | None => {
                if viewport_changed {
                    if let Some(doc) = &mut self.doc {
                        doc.set_viewport(Viewport::new(
                            self.view_w,
                            self.view_h,
                            scale,
                            ColorScheme::Dark,
                        ));
                        self.dirty = true;
                    }
                }
            }
        }

        let mut events: Vec<UiEvent> = Vec::new();
        for (pointer, keys) in samples {
            // Pointer before keys within one frame's sample: a click that
            // focuses a field and the keys typed the same frame land in
            // program order (focus first, then text).
            self.apply_pointer(pointer, scale, &mut events);
            self.apply_keys(keys, &mut events);
        }
        self.finish_cycle(events)
    }

    /// Feed ONE pointer sample through blitz's event driver (a no-op without
    /// a document), synthesizing hover moves and press/release edges.
    fn apply_pointer(&mut self, pointer: PointerState, scale: f32, events: &mut Vec<UiEvent>) {
        let Some(doc) = &mut self.doc else {
            return;
        };

        // ── Pointer input → blitz event driver ─────────────────────────────
        let mut collector = EventCollector { events: Vec::new() };
        let css_pos = pointer.pos.map(|(x, y)| (x / scale, y / scale));
        let hover_before = doc.get_hover_node_id();
        let mut press_edge = false;
        {
            let mut driver = EventDriver::new(doc as &mut dyn Document, &mut collector);
            match css_pos {
                Some((x, y)) => {
                    if self.hover_pos != css_pos {
                        // A hover move carries the CURRENT button level —
                        // `Primary` on a plain move would read as a drag.
                        // [xreview]
                        driver.handle_ui_event(BlitzUiEvent::PointerMove(pointer_event(
                            x,
                            y,
                            self.pointer_was_down,
                        )));
                    }
                    if pointer.primary_down && !self.pointer_was_down {
                        driver
                            .handle_ui_event(BlitzUiEvent::PointerDown(pointer_event(x, y, true)));
                        press_edge = true;
                    } else if !pointer.primary_down && self.pointer_was_down {
                        driver
                            .handle_ui_event(BlitzUiEvent::PointerUp(pointer_event(x, y, false)));
                        press_edge = true;
                    }
                }
                None => {
                    // Pointer left (captured for free-look / suppressed):
                    // release any held press so a drag can't stick.
                    if self.pointer_was_down {
                        if let Some((x, y)) = self.hover_pos {
                            driver.handle_ui_event(BlitzUiEvent::PointerUp(pointer_event(
                                x, y, false,
                            )));
                            press_edge = true;
                        }
                    }
                }
            }
        }
        self.pointer_was_down = pointer.primary_down && css_pos.is_some();
        self.hover_pos = css_pos;
        if doc.get_hover_node_id() != hover_before || press_edge || !collector.events.is_empty() {
            // Hover transitions restyle (`:hover`), press/release edges
            // restyle (`:active`) [xreview], clicks re-render below anyway
            // once the model folds — repaint on any of them.
            self.dirty = true;
        }
        events.extend(collector.events);
    }

    /// Feed one frame's keyboard input for the focused text field through
    /// blitz's editing stack (a no-op without a document). Each key lowers to
    /// the `KeyDown`/`KeyUp` pair blitz edits on (`webview_keys::lower_key`);
    /// the resulting DOM `input` events come back through the collector as
    /// slot-stamped `TextChanged` [`UiEvent`]s. `Escape` instead DEFOCUSES
    /// the field (blitz has no built-in Escape behavior), dropping the
    /// wants-keyboard latch so the shell's next Escape releases the cursor —
    /// the `Ui.textInput` two-step rule.
    fn apply_keys(&mut self, keys: Vec<WebviewKey>, events: &mut Vec<UiEvent>) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        if keys.is_empty() {
            return;
        }
        let mut collector = EventCollector { events: Vec::new() };
        for key in &keys {
            if matches!(key, WebviewKey::Escape) {
                // Blur directly: the driver has no defocus event, and no
                // handler observes blur — the latch flip is the effect.
                doc.clear_focus();
            } else {
                let mut driver = EventDriver::new(doc as &mut dyn Document, &mut collector);
                for ev in lower_key(key) {
                    driver.handle_ui_event(ev);
                }
            }
        }
        // Any key changes what's painted: text, caret/selection, or the
        // focus ring (Escape).
        self.dirty = true;
        events.extend(collector.events);
    }

    /// Resolve + repaint once for the whole input batch, and package the
    /// results. Returns `None` when there is nothing to report (idle).
    fn finish_cycle(&mut self, events: Vec<UiEvent>) -> Option<WorkerOutput> {
        let Some(doc) = &mut self.doc else {
            // Cleared: the main thread already reset its own snapshot.
            return None;
        };

        // ── Layout/style resolution + CPU repaint (only when dirty) ────────
        doc.resolve(self.clock);
        let wants_pointer = doc
            .get_hover_node_id()
            .map(|id| chain_has_handler(doc, id))
            .unwrap_or(false);
        // Keyboard latch: an editable element is focused (a click on an
        // `<input>` focuses it in blitz's pointer handling; Escape or a
        // click elsewhere clears it).
        let wants_keyboard = focused_editable(doc);
        // Repaint when dirty OR while CSS animations/transitions are active:
        // `resolve(t)` above already advanced blitz's clock, and
        // `is_animating()` reports live @keyframes/transitions, so ticking
        // them costs one repaint per cycle — bounded, and off the frame loop.
        let frame = if self.dirty || doc.is_animating() {
            // Recreate the rasterizer whenever the framebuffer size moved.
            if self.renderer_size != Some((self.view_w, self.view_h)) {
                self.renderer = Some(VelloCpuImageRenderer::new(self.view_w, self.view_h));
                self.renderer_size = Some((self.view_w, self.view_h));
            }
            let renderer = self.renderer.as_mut().expect("just created");
            // The render context ACCUMULATES scene commands across paints —
            // without a reset, content that moved (an animating element)
            // ghosts its previous positions over the transparent background.
            renderer.reset();
            let mut rgba = Vec::new();
            let (w, h, scale) = (self.view_w, self.view_h, self.view_scale);
            renderer.render_to_vec(
                |scene| blitz_paint::paint_scene(scene, doc, scale as f64, w, h, 0, 0),
                &mut rgba,
            );
            self.dirty = false;
            self.generation += 1;
            Some(WorkerFrame {
                rgba,
                width: w,
                height: h,
                generation: self.generation,
            })
        } else {
            None
        };

        // Interactive-element boxes for the shell's synchronous press
        // arbitration, sent only when they moved/changed.
        let rects = interactive_rects(doc);
        let rects_msg = if rects != self.sent_rects {
            self.sent_rects = rects.clone();
            Some(rects)
        } else {
            None
        };

        if frame.is_none()
            && events.is_empty()
            && rects_msg.is_none()
            && wants_pointer == self.sent_wants_pointer
            && wants_keyboard == self.sent_wants_keyboard
        {
            return None; // truly idle — keep both channels quiet
        }
        self.sent_wants_pointer = wants_pointer;
        self.sent_wants_keyboard = wants_keyboard;
        Some(WorkerOutput {
            epoch: self.epoch,
            events,
            wants_pointer,
            wants_keyboard,
            interactive_rects: rects_msg,
            frame,
        })
    }
}

/// The worker thread: block until inputs arrive, drain everything queued
/// into one batch, and run a single cycle over it — one parse (the last HTML
/// directive) + every pointer edge + one resolve/repaint. The expensive half
/// is what coalesces, so a backlog shrinks instead of compounding. Exits
/// when the input channel closes (the overlay was dropped).
fn worker_loop(rx: Receiver<WorkerInput>, tx: Sender<WorkerOutput>) {
    let mut state = WorkerState::new();
    loop {
        let first = match rx.recv() {
            Ok(input) => input,
            Err(_) => return,
        };
        let mut batch = vec![first];
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(input) => batch.push(input),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some(output) = state.run_cycle(batch) {
            if tx.send(output).is_err() {
                return;
            }
        }
        if disconnected {
            return;
        }
    }
}

/// Render an HTML string to a premultiplied-RGBA buffer, headlessly (no GL) —
/// the GL-free core of the worker's parse→resolve→paint cycle, exposed for
/// tests and tooling (an agent can verify webview rendering without a
/// window).
pub fn render_html_to_rgba(html: &str, width: u32, height: u32, scale: f32) -> Vec<u8> {
    let viewport = Viewport::new(width, height, scale, ColorScheme::Dark);
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        },
    )
    .into_inner();
    doc.resolve(0.0);
    let mut renderer = VelloCpuImageRenderer::new(width, height);
    let mut buf = Vec::new();
    renderer.render_to_vec(
        |scene| blitz_paint::paint_scene(scene, &mut doc, scale as f64, width, height, 0, 0),
        &mut buf,
    );
    buf
}

/// Whether the document's focused node is an editable text element — the
/// keyboard-routing latch (`wants_keyboard`).
fn focused_editable(doc: &BaseDocument) -> bool {
    doc.get_focussed_node_id()
        .and_then(|id| doc.get_node(id))
        .and_then(|node| node.element_data())
        .is_some_and(|el| el.text_input_data().is_some())
}

/// The `data-fn-input` slot of the focused element — the stable identity
/// used to restore focus across document rebuilds. Walks up the parent
/// chain defensively: blitz focuses the pointer hit-test's node, which can
/// be a descendant of the slot-stamped `<input>` rather than the element
/// itself.
fn focused_input_slot(doc: &BaseDocument) -> Option<u32> {
    let input = LocalName::from("data-fn-input");
    let mut current = doc.get_focussed_node_id();
    while let Some(id) = current {
        let node = doc.get_node(id)?;
        if let Some(slot) = node.attr(input.clone()).and_then(|v| v.parse::<u32>().ok()) {
            return Some(slot);
        }
        current = node.parent;
    }
    None
}

/// Find the element carrying `data-fn-input="slot"` — the re-focus target
/// after a rebuild ([`focused_input_slot`]'s inverse).
fn find_input_by_slot(doc: &BaseDocument, slot: u32) -> Option<usize> {
    let input = LocalName::from("data-fn-input");
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.attr(input.clone()).and_then(|v| v.parse::<u32>().ok()) == Some(slot) {
            return Some(id);
        }
        stack.extend(node.children.iter().copied());
    }
    None
}

/// Whether `node_id` or any ancestor carries a webview handler attribute —
/// the pointer-arbitration test (a click here is for the webview).
fn chain_has_handler(doc: &BaseDocument, node_id: usize) -> bool {
    let click = LocalName::from("data-fn-click");
    let input = LocalName::from("data-fn-input");
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = doc.get_node(id) else {
            return false;
        };
        if node.attr(click.clone()).is_some() || node.attr(input.clone()).is_some() {
            return true;
        }
        current = node.parent;
    }
    false
}

/// CSS-px absolute boxes `(x, y, w, h)` of every element carrying a webview
/// handler attribute, from the RESOLVED layout — the worker's snapshot for
/// the shell's synchronous press arbitration. Coarser than a real hit-test
/// (no z-order/clip awareness), which errs toward the webview keeping a
/// click — never toward a surprise cursor recapture.
fn interactive_rects(doc: &BaseDocument) -> Vec<(f32, f32, f32, f32)> {
    let click = LocalName::from("data-fn-click");
    let input = LocalName::from("data-fn-input");
    let mut rects = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.attr(click.clone()).is_some() || node.attr(input.clone()).is_some() {
            let pos = node.absolute_position(0.0, 0.0);
            let size = node.final_layout.size;
            rects.push((pos.x, pos.y, size.width, size.height));
        }
        stack.extend(node.children.iter().copied());
    }
    rects
}

/// `primary_held` is the button LEVEL the event carries (`buttons` in DOM
/// terms): true for a press and for moves while held, false for a release
/// and plain hover moves — blitz reads a held-level move as a drag.
fn pointer_event(x: f32, y: f32, primary_held: bool) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons: if primary_held {
            MouseEventButtons::Primary
        } else {
            MouseEventButtons::None
        },
        mods: Default::default(),
        details: Default::default(),
        element: Default::default(),
        active_pointers: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    //! Headless keyboard/focus tests driving the worker's real cycle
    //! ([`WorkerState::run_cycle`]) — the exact code the render thread runs,
    //! no GL and no window: synthetic clicks focus, `WebviewKey`s type
    //! through blitz's editing stack, and focus survives a document rebuild.

    use super::*;

    const FB: (u32, u32) = (400, 300);

    /// A controlled input (slot 0, seeded from `value`) at the top-left —
    /// margins zeroed so the input's 200x30 box sits at the origin — plus a
    /// label that changes across "re-renders".
    fn page(value: &str, label: &str) -> String {
        format!(
            "<style>html,body{{margin:0}}input{{width:200px;height:30px}}</style>\
             <div><input value=\"{value}\" data-fn-input=\"0\"><p>{label}</p></div>"
        )
    }

    fn input(html: Option<&str>, pointer: PointerState, keys: Vec<WebviewKey>) -> WorkerInput {
        WorkerInput {
            html: match html {
                Some(h) => HtmlMsg::Set(Arc::from(h)),
                None => HtmlMsg::Unchanged,
            },
            fb_width: FB.0,
            fb_height: FB.1,
            dpi_scale: 1.0,
            pointer,
            keys,
            clock: 0.0,
            epoch: 0,
        }
    }

    fn hover(x: f32, y: f32, down: bool) -> PointerState {
        PointerState {
            pos: Some((x, y)),
            primary_down: down,
        }
    }

    /// Click inside the input (press then release, two frame samples) — the
    /// synthetic version of the shell's press/release edges.
    fn click_input(state: &mut WorkerState) -> Option<WorkerOutput> {
        let down = input(None, hover(50.0, 15.0, true), vec![]);
        let up = input(None, hover(50.0, 15.0, false), vec![]);
        state.run_cycle(vec![down, up])
    }

    fn text_changes(out: &WorkerOutput) -> Vec<&str> {
        out.events
            .iter()
            .filter_map(|e| match &e.kind {
                UiEventKind::TextChanged(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn click_focuses_the_input_and_typing_emits_text_changed() {
        let mut state = WorkerState::new();
        let out = state
            .run_cycle(vec![input(Some(&page("", "hello")), PointerState::default(), vec![])])
            .expect("first cycle paints");
        assert!(!out.wants_keyboard, "nothing focused on load");

        let out = click_input(&mut state).expect("click cycle reports");
        assert!(out.wants_keyboard, "clicking the input focuses it");

        let out = state
            .run_cycle(vec![input(
                None,
                hover(50.0, 15.0, false),
                vec![WebviewKey::Char('h'), WebviewKey::Char('i')],
            )])
            .expect("typing cycle reports");
        assert_eq!(text_changes(&out), vec!["h", "hi"], "each keystroke emits the full value");
        assert!(out.wants_keyboard);
    }

    #[test]
    fn backspace_and_enter_edit_through_the_os_specific_lowering() {
        let mut state = WorkerState::new();
        state.run_cycle(vec![input(Some(&page("hi", "x")), PointerState::default(), vec![])]);
        click_input(&mut state);
        // Backspace exercises the per-OS lowering (Apple standard keybinding
        // on macOS, a plain KeyDown elsewhere) against real blitz.
        let out = state
            .run_cycle(vec![input(None, hover(50.0, 15.0, false), vec![WebviewKey::Backspace])])
            .expect("edit cycle reports");
        assert_eq!(text_changes(&out), vec!["h"]);
    }

    #[test]
    fn focus_survives_a_rebuild_with_the_caret_at_the_end() {
        let mut state = WorkerState::new();
        state.run_cycle(vec![input(Some(&page("", "greeting")), PointerState::default(), vec![])]);
        click_input(&mut state);
        let out = state
            .run_cycle(vec![input(None, hover(50.0, 15.0, false), vec![WebviewKey::Char('h'), WebviewKey::Char('i')])])
            .expect("typing reports");
        assert_eq!(text_changes(&out), vec!["h", "hi"]);

        // The controlled round-trip: the model echoes the value into NEW html
        // (label changed too) — a fresh document. Focus must survive by slot.
        let out = state
            .run_cycle(vec![input(Some(&page("hi", "Hello, hi!")), hover(50.0, 15.0, false), vec![])])
            .expect("rebuild cycle reports");
        assert!(out.wants_keyboard, "focus restored on the rebuilt document");

        // Caret restored to the END: the next character appends (a lost
        // caret would prepend "!hi").
        let out = state
            .run_cycle(vec![input(None, hover(50.0, 15.0, false), vec![WebviewKey::Char('!')])])
            .expect("post-rebuild typing reports");
        assert_eq!(text_changes(&out), vec!["hi!"]);
    }

    #[test]
    fn escape_defocuses_and_drops_the_keyboard_latch() {
        let mut state = WorkerState::new();
        state.run_cycle(vec![input(Some(&page("", "x")), PointerState::default(), vec![])]);
        let out = click_input(&mut state).expect("click cycle reports");
        assert!(out.wants_keyboard);
        let out = state
            .run_cycle(vec![input(None, hover(50.0, 15.0, false), vec![WebviewKey::Escape])])
            .expect("escape cycle reports");
        assert!(!out.wants_keyboard, "Escape blurs the field");
    }

    #[test]
    fn a_vanished_slot_drops_focus_instead_of_guessing() {
        let mut state = WorkerState::new();
        state.run_cycle(vec![input(Some(&page("", "x")), PointerState::default(), vec![])]);
        click_input(&mut state);
        // Rebuild WITHOUT the input: no node carries the slot — focus (and
        // the latch) must drop, not land somewhere arbitrary.
        let out = state
            .run_cycle(vec![input(
                Some("<style>html,body{margin:0}</style><p>gone</p>"),
                hover(50.0, 15.0, false),
                vec![],
            )])
            .expect("rebuild cycle reports");
        assert!(!out.wants_keyboard);
    }
}
