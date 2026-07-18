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
//! Retained, not immediate: the DOM is rebuilt only when the serialized HTML
//! actually changes, and re-rasterized only when something visible could have
//! changed (new HTML, hover transitions, clicks, resize). An idle webview
//! costs one string compare per frame.
//!
//! Interaction mirrors the egui overlay: pointer events feed blitz's
//! `EventDriver`, which synthesizes DOM semantics (click = press+release on
//! the same element, hover chains for `:hover` CSS). A DOM `click`/`input`
//! event walks the bubble chain for the nearest `data-fn-click` /
//! `data-fn-input` attribute — the handler slot the `Attr.onClick` /
//! `Attr.onInput` builder stamped — and comes back as a [`UiEvent`] the shell
//! folds through `GameProducer::webview_event`.

use std::sync::Arc;
use std::time::Instant;

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

/// What one webview frame produced: interactions to fold through `update`,
/// and whether the pointer is over an interactive element (the shell's
/// click-arbitration latch, like `ui_wants_pointer`).
pub struct WebviewOutput {
    pub events: Vec<UiEvent>,
    pub wants_pointer: bool,
}

impl WebviewOutput {
    fn empty() -> Self {
        WebviewOutput {
            events: Vec::new(),
            wants_pointer: false,
        }
    }
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
    /// The live DOM, rebuilt when `html` changes. `None` until the game's
    /// first `webview` tree arrives.
    doc: Option<BaseDocument>,
    /// The serialized HTML the current `doc` was parsed from — the
    /// change-detection key (a tree diff can replace this later).
    html: String,
    /// CPU rasterizer + its output buffer, recreated on resize.
    renderer: Option<VelloCpuImageRenderer>,
    buf: Vec<u8>,
    tex_w: u32,
    tex_h: u32,
    /// Repaint latch: HTML changed, hover moved, a click landed, resized.
    dirty: bool,
    /// The animation clock blitz `resolve(t)` ticks on — ours, per the
    /// headless embedding (the shell owns the loop).
    start: Instant,
    /// One font context shared across reparses. A fresh context per document
    /// re-enumerates system fonts — ~55ms per model-driven re-render (~30ms
    /// release), measured; sharing drops the reparse to single-digit ms.
    font_ctx: FontContext,
    /// Pointer state last frame, to synthesize press/release edges.
    pointer_was_down: bool,
    hover_pos: Option<(f32, f32)>,
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
        WebviewOverlay {
            gl,
            program,
            vao,
            texture,
            doc: None,
            html: String::new(),
            renderer: None,
            buf: Vec::new(),
            tex_w: 0,
            tex_h: 0,
            dirty: false,
            start: Instant::now(),
            font_ctx: FontContext::default(),
            pointer_was_down: false,
            hover_pos: None,
        }
    }

    /// Synchronous hit-test in CSS px (== window points): is the pointer over
    /// an interactive element RIGHT NOW? The shell's press arbitration uses
    /// this instead of the one-frame-stale `wants_pointer` latch — after a
    /// model-driven re-render, a stationary click's latch still reads the OLD
    /// tree, and the press would recapture the cursor instead of clicking.
    pub fn hit_interactive_css(&self, x: f32, y: f32) -> bool {
        let Some(doc) = &self.doc else {
            return false;
        };
        doc.hit(x, y)
            .map(|hit| chain_has_handler(doc, hit.node_id))
            .unwrap_or(false)
    }

    /// Run one webview frame: reconcile the DOM against `html`, feed pointer
    /// input, repaint if anything changed, and draw the overlay quad.
    /// `pointer.pos` is in framebuffer pixels (the egui overlays' space);
    /// blitz works in CSS px, so it is divided by `dpi_scale` here.
    pub fn frame(
        &mut self,
        fb_width: u32,
        fb_height: u32,
        dpi_scale: f32,
        html: Option<&str>,
        pointer: PointerState,
    ) -> WebviewOutput {
        let Some(html) = html else {
            // Hook absent (or nothing built yet): drop any retained DOM so a
            // deleted `webview` clears the overlay (the `ui` reload rule).
            self.doc = None;
            self.html.clear();
            self.pointer_was_down = false;
            return WebviewOutput::empty();
        };
        if fb_width == 0 || fb_height == 0 {
            return WebviewOutput::empty();
        }

        let scale = if dpi_scale > 0.0 { dpi_scale } else { 1.0 };
        let viewport_changed = self.tex_w != fb_width || self.tex_h != fb_height;
        let clock = self.start.elapsed().as_secs_f64();

        // ── Reconcile the DOM (prototype: full reparse on change) ─────────
        if html != self.html || self.doc.is_none() {
            self.html.clear();
            self.html.push_str(html);
            let viewport = Viewport::new(fb_width, fb_height, scale, ColorScheme::Dark);
            let mut doc = HtmlDocument::from_html(
                html,
                DocumentConfig {
                    viewport: Some(viewport),
                    // Shared across reparses — a fresh context re-enumerates
                    // system fonts (tens of ms) on every re-render.
                    font_ctx: Some(self.font_ctx.clone()),
                    ..Default::default()
                },
            )
            .into_inner();
            // A fresh document has NO layout until resolved — the hover
            // re-establishing move below would hit-test nothing, leaving
            // `wants_pointer` false until the mouse physically moves, and the
            // next stationary click would recapture the cursor instead of
            // pressing the button (the repeated-click repro). Resolve now so
            // same-frame events see real geometry.
            doc.resolve(clock);
            self.doc = Some(doc);
            self.dirty = true;
            // Re-establish hover on the fresh DOM: clearing `hover_pos` makes
            // the next pointer sample look moved, so the move re-synthesizes
            // below even for a stationary cursor (`:hover` and wants_pointer
            // would otherwise go stale until the mouse physically moves).
            // [xreview]
            self.hover_pos = None;
            self.pointer_was_down = false;
        } else if viewport_changed {
            if let Some(doc) = &mut self.doc {
                doc.set_viewport(Viewport::new(fb_width, fb_height, scale, ColorScheme::Dark));
                self.dirty = true;
            }
        }
        let Some(doc) = &mut self.doc else {
            return WebviewOutput::empty();
        };

        // ── Pointer input → blitz event driver ─────────────────────────────
        let mut collector = EventCollector { events: Vec::new() };
        let css_pos = pointer
            .pos
            .map(|(x, y)| (x / scale, y / scale));
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

        // Is the pointer over an interactive element? That click belongs to
        // the webview, not to free-look recapture (the `ui_wants_pointer`
        // rule). Text hit or not, walk up from the hovered node.
        let wants_pointer = doc
            .get_hover_node_id()
            .map(|id| chain_has_handler(doc, id))
            .unwrap_or(false);

        // ── Layout/style resolution + CPU repaint (only when dirty) ────────
        doc.resolve(clock);
        if self.dirty || viewport_changed {
            if self.renderer.is_none() || viewport_changed {
                self.renderer = Some(VelloCpuImageRenderer::new(fb_width, fb_height));
            }
            let renderer = self.renderer.as_mut().expect("just created");
            renderer.render_to_vec(
                |scene| {
                    blitz_paint::paint_scene(
                        scene,
                        doc,
                        scale as f64,
                        fb_width,
                        fb_height,
                        0,
                        0,
                    )
                },
                &mut self.buf,
            );
            unsafe {
                let gl = &self.gl;
                gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    fb_width as i32,
                    fb_height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&self.buf)),
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            self.tex_w = fb_width;
            self.tex_h = fb_height;
            self.dirty = false;
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
            events: collector.events,
            wants_pointer,
        }
    }
}

/// Render an HTML string to a premultiplied-RGBA buffer, headlessly (no GL) —
/// the GL-free core of [`WebviewOverlay::frame`], exposed for tests and
/// tooling (an agent can verify webview rendering without a window).
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
