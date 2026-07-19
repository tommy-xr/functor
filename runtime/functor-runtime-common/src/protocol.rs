//! The logic↔runtime protocol: the versioned contract between a game's pure
//! logic (today the Fable-generated F# dylib/wasm module; later any producer —
//! see `docs/functor-lang.md`, Track A) and the imperative runtime shells.
//!
//! Everything that crosses the boundary is enumerated here, split by whether it
//! crosses **as serializable data** (the protocol proper — language-neutral,
//! introspectable, what a second producer must speak), as **unversioned debug
//! text**, or **in-process only** (a same-binary handoff that is *not* yet part
//! of the data protocol). The tests below pin the wire shape of the data types;
//! changing one is a protocol change and should bump [`PROTOCOL_VERSION`].
//!
//! # Per-frame, runtime → logic
//!
//! - [`crate::FrameTime`] — `tts`/`dts` seconds.
//! - Input events, as scalars: key code ([`crate::Key`] as `i32`) + down flag,
//!   mouse position (`i32` pixels), wheel delta (`i32`). The `Key` enum's
//!   **`i32` discriminants** are the wire representation on both sides
//!   (mirrored by F# `Input.ofKeyCode`) — inserting a variant mid-enum is a
//!   protocol break even though serde names don't change.
//! - [`crate::ui::UiEvent`] — an interaction on an interactive UI widget
//!   (slot-addressed; see docs/ui-interaction.md), delivered like the input
//!   scalars above.
//!
//! # Per-frame, logic → runtime
//!
//! - [`crate::Frame`] — camera + [`crate::Scene3D`] + [`crate::Light`]s; the
//!   value returned by `draw3d`, also serialized verbatim for `GET /scene`.
//! - [`crate::ui::View`] — the declarative UI tree (`emit_ui`).
//! - Drained command queues, each a JSON array over the boundary:
//!   [`crate::net::NetCommand`] (HTTP), [`crate::net::ConnCommand`]
//!   (connections), [`crate::audio::AudioCommand`] (one-shots).
//! - [`crate::audio::AudioScene`] — the desired soundscape, reconciled by the
//!   host against its live voices.
//!
//! # Async, runtime → logic (inbox pushes)
//!
//! - HTTP results: `(token, status, body)` / `(token, message)` scalars,
//!   matching [`crate::net::NetInbound`].
//! - Connection events: `(key, conn, text)` scalars; kinds as in
//!   `KeyedEvent::kind`.
//! - Audio one-shot completions: `token`.
//!
//! # Debug text (unversioned, human/LLM-facing — not protocol data)
//!
//! - `emit_state_debug` → `String`: a Rust-`Debug` pretty-print of the live
//!   model, surfaced as the `model` field of the debug server's `GET /state`.
//!   Free-form by design; consumers must not parse it.
//!
//! # In-process only (NOT part of the data protocol — known limitations)
//!
//! - [`crate::OpaqueState`] — the hot-reload state bundle is a `Box<dyn Any>`
//!   moved between dylib generations with a layout-compatibility assumption,
//!   not serialized data. It carries the **model only**: pending effects are
//!   deliberately dropped on reload (an `Http` effect's tagger is a closure
//!   into the old dylib and would dangle — see `getState` in
//!   `src/Functor.Game/Runtime.fs`). A data-native state representation is
//!   what makes state durable/inspectable across producers (`docs/functor-lang.md`,
//!   Track C).
//! - Effect *commands* cross as data (above); the producer holds any per-effect
//!   tagger/closure on its own side (the Functor Lang producer keeps them per session),
//!   so nothing closure-shaped crosses the boundary.
//! - Control signals with no payload: `init` and `quit`.

/// Version of the serialized logic↔runtime contract. Bump when the wire shape
/// of any type enumerated in this module changes incompatibly. Informational
/// for now — nothing transmits or checks it; [`GameProducer`] impls all speak
/// the current version.
///
/// v2: `Asset.whilePending` — `ModelDescription.while_pending` (defaulted,
/// omitted when empty, so v1 frames read back and chainless frames stay v1-
/// shaped) and the `TextureDescription::FileWhilePending` variant (a v1
/// reader cannot decode a frame carrying one).
pub const PROTOCOL_VERSION: u32 = 2;

/// The producer side of the protocol: one game logic instance as consumed by a
/// runtime shell's frame loop. Every method carries a payload enumerated in
/// this module's boundary doc (drains return JSON arrays, pushes take the
/// inbox scalars, `render` returns the [`crate::Frame`]).
///
/// Impls: the Functor Lang interpreter producers `FunctorLangGame` / `ReplayGame` (desktop)
/// and `FunctorLangWebGame` (web). The seam exists so a producer can be swapped
/// without the shells knowing what language or pipeline produced the logic —
/// see `docs/functor-lang.md`, Track A2.
///
/// Some capabilities are shell-specific; producers for shells that lack them
/// implement the honest no-op (e.g. the web producer's `check_hot_reload` —
/// the browser reloads the whole page — and `audio_push_finished` — web
/// one-shots are fire-and-forget).
pub trait GameProducer {
    /// Poll for and apply a logic update (native dylib hot-reload). Shells
    /// with a reload path call this once per frame before anything else;
    /// shells without one (the web runtime — the browser reloads the whole
    /// page) may never call it.
    fn check_hot_reload(&mut self, frame_time: crate::FrameTime);

    /// Replace the game's logic from source text pushed over the wire — the
    /// network hot-reload path (a dev machine pushing to a runner on another
    /// device, e.g. a headset). Same semantics as the file-watch reload:
    /// the model is preserved, a broken push keeps the old program running.
    /// `Ok` carries a short status line for the pusher. The default is the
    /// honest refusal for producers whose logic isn't source-shaped
    /// (compiled dylibs, replays).
    fn reload_source(&mut self, _source: &str) -> Result<String, String> {
        Err("this producer does not support source reload (not an .fun game)".to_string())
    }

    /// Replace the game's logic from a full pushed FILE SET — `(path, source)`
    /// pairs, the entry first, then siblings (`file = module`). The multi-file
    /// sibling of [`GameProducer::reload_source`], for pushers that hold the
    /// whole project in memory (the web IDE) rather than editing one buffer
    /// over served files. Same semantics: model preserved, a broken push
    /// keeps the old program running.
    fn reload_project(&mut self, _files: &[(String, String)]) -> Result<String, String> {
        Err("this producer does not support project reload (not an .fun game)".to_string())
    }

    /// Rewind the whole scene — model AND physics world — to an earlier
    /// RENDERED frame, restoring both to the state they had at the end of that
    /// frame and branching the recorded future from there (docs/time-travel.md
    /// T1, the coupled seek). `Ok` carries a short status line; the default is
    /// the honest refusal for producers that don't record a model history.
    /// Shell-driven (the time-travel scrubber), not a game hook.
    fn rewind_scene_to(&mut self, _rendered_frame: u64) -> Result<String, String> {
        Err("this producer does not support scene rewind".to_string())
    }

    /// The newest recorded rendered frame — what the time-travel scrubber shows
    /// and rewinds relative to (docs/time-travel.md T1). `None` for producers
    /// that don't record a model history.
    fn current_scene_frame(&self) -> Option<u64> {
        None
    }

    /// The recorded rendered-frame window `(oldest, newest)` the scrubber can
    /// seek within (docs/time-travel.md T3). `None` if nothing is recorded.
    fn scene_frame_range(&self) -> Option<(u64, u64)> {
        None
    }

    /// Plain-data inputs recorded on one rendered frame. Timeline UIs use this
    /// to draw authoritative event markers from the replay log rather than from
    /// raw shell events that may have been discarded while paused.
    fn recorded_inputs_at(&self, _rendered_frame: u64) -> Vec<crate::RecordedInput> {
        Vec::new()
    }

    /// Revision of the authoritative recording generation. It changes when a
    /// destructive branch or reload can replace frame-indexed timeline data.
    fn scene_timeline_generation(&self) -> u64 {
        0
    }

    /// The recorded `tts` of the frame the scene currently sits on (the scrubbed
    /// frame while dragging, else the newest recorded frame). Shells read this to
    /// REBASE their [`crate::GameClock`] when a time-travel branch resumes — on a
    /// resume-from-scrub, after a seek, or after a `POST /rewind` — so play
    /// continues from the scrubbed scene time instead of snapping to wall-clock
    /// "now" (docs/time-travel.md). `None` for producers that don't record a
    /// model history.
    fn current_scene_tts(&self) -> Option<f64> {
        None
    }

    /// Forward-step the scene `divisions` fixed frames of `dt` seconds from
    /// `start_tts` and return the drawn [`crate::Frame`] for each, PAIRED with
    /// the division-boundary [`crate::FrameTime`] the frame was drawn at — the
    /// forward-ghosting trajectory preview (docs/time-travel.md T6d). The shell
    /// composites the returned frames into one image (chronophotography), so
    /// moving elements smear into a strobe of their future positions, and must
    /// render EACH frame at ITS paired time so render-time animation (the
    /// skinned-skeleton pose, sampled from the render pass's `tts`) advances
    /// through the strobe instead of freezing at the paused pose. Each frame
    /// carries the *paused* camera so only world motion smears, not the view.
    ///
    /// `script_inputs` is the F2 forward-ghost-a-script mode (docs/time-travel.md
    /// F2): when `Some`, the ghost forward-steps from the current live model (the
    /// anchor) replaying THIS per-fine-step input slice instead of the recorder's
    /// own input log — so editing a constant and hot-reloading re-renders the
    /// scripted trajectory (the Bret-Victor "tweak a constant, see the arc" loop).
    /// When `None`, the T6d behavior: replay the recorder's recorded inputs after
    /// the fork point. The default is empty (no ghosting) for producers without a
    /// model history.
    fn ghost_frames(
        &self,
        _divisions: usize,
        _dt: f32,
        _start_tts: f64,
        _script_inputs: Option<&[Vec<crate::RecordedInput>]>,
    ) -> Vec<(crate::Frame, crate::FrameTime)> {
        Vec::new()
    }

    /// Seek the whole scene to a rendered frame for DISPLAY, WITHOUT branching
    /// (docs/time-travel.md T3, the draggable scrubber): restore model + world
    /// so the user can scrub back and forth freely while paused. The future is
    /// discarded only when play resumes from the scrubbed point (which commits
    /// a `rewind_scene_to` branch). The default is the honest refusal.
    fn seek_scene_to(&mut self, _rendered_frame: u64) -> Result<String, String> {
        Err("this producer does not support scene seek".to_string())
    }

    fn tick(&mut self, frame_time: crate::FrameTime);

    /// Deliver a keyboard event. `code` is a [`crate::Key`] as `i32`.
    fn key_event(&mut self, code: i32, is_down: bool);

    /// Deliver a mouse-move event in window pixel coordinates.
    fn mouse_move(&mut self, x: i32, y: i32);

    /// Deliver a mouse-wheel event (vertical scroll offset).
    fn mouse_wheel(&mut self, delta: i32);

    /// Deliver an interaction on an interactive UI widget
    /// ([`crate::ui::UiEvent`], slot-addressed — docs/ui-interaction.md). The
    /// default drops it: the honest no-op for producers with no interactive
    /// UI (replays, producers whose games define no `ui`).
    fn ui_event(&mut self, _event: crate::ui::UiEvent) {}

    fn render(&mut self, frame_time: crate::FrameTime) -> crate::Frame;

    /// Record the shell-measured GL cost of the frame just presented — the wall
    /// time of the scene render pass (`render_ns`, interpreter → GL submission)
    /// and of the buffer swap (`swap_ns`, which captures vsync blocking). Unlike
    /// `tick`/`draw` (interpreter cost the producer times itself), these are
    /// measured in the imperative shell around the GL calls, so the shell folds
    /// them back here once per windowed frame after `swap_buffers`; the producer
    /// averages them into its rolling frame-stats window. The default drops them
    /// — the honest no-op for producers with no GL shell (replays, web). The
    /// headless desktop loop uses the same producer but never calls this (no
    /// GL), so its `render_us`/`swap_us` stay 0.
    fn record_gl_timing(&mut self, _render_ns: u64, _swap_ns: u64) {}

    /// The game's declarative UI tree (`ui model`), lowered by the shell to a
    /// text overlay drawn on top of the frame.
    fn ui(&self) -> crate::ui::View;

    /// The game's HTML/CSS webview tree (`webview model`), or `None` when the
    /// game defines no `webview`. The shell renders it above the frame: blitz
    /// composited as a GL texture natively, a real DOM overlay on wasm.
    /// Interactions come back through [`GameProducer::webview_event`].
    fn webview(&self) -> Option<crate::webview::HtmlNode> {
        None
    }

    /// Deliver an interaction on an interactive webview element (slot-addressed
    /// like [`GameProducer::ui_event`], against the webview's own handler
    /// table). The default drops it: the honest no-op for producers whose
    /// games define no `webview`.
    fn webview_event(&mut self, _event: crate::ui::UiEvent) {}

    /// A pretty-printed (Rust `Debug`) view of the live game model, for
    /// introspection (the debug server's `GET /state` `model` field). Opaque
    /// debug text — see the module doc; consumers must not parse it.
    fn state_debug(&self) -> String;

    /// The paused-inspector trace (visual-debugger PR2): the wire-contract JSON
    /// for the last real frame's entry-point invocations, replayed on demand
    /// while paused (the debug server's `GET /trace`). `paused` is the shell's
    /// clock state. The unpaused doc carries no `frame`/`tts` (it must stay
    /// byte-identical across idle polls — the LSP dedups on the doc bytes).
    /// The default is an inert doc — only the interpreter producer keeps a
    /// replay journal (a compiled/replay producer has no source to trace).
    fn inspector_trace(&mut self, _paused: bool) -> String {
        "{\"paused\":false,\"sources\":[],\"invocations\":[]}".to_string()
    }

    /// The shell delivered debug-injected input (`POST /input`) while the clock
    /// is PAUSED (visual-debugger PR2): no `tick` will run to sweep the
    /// journaled entry-point calls into the last-frame journal, so the producer
    /// folds them in now — the injection becomes a first-class invocation in
    /// `GET /trace` instead of a phantom on the resume frame's journal.
    /// Default: no-op (producers without a replay journal have nothing to fold).
    fn absorb_paused_input(&mut self) {}

    /// Take the networking commands the game queued this frame (a JSON array
    /// of [`crate::net::NetCommand`]). The shell performs the I/O and reports
    /// results back with `net_push_http_response` / `net_push_http_error`.
    fn net_drain_commands(&self) -> String;

    /// Deliver a completed HTTP response into the game's async inbox.
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String);

    /// Deliver a transport-level failure for a request into the async inbox.
    fn net_push_http_error(&mut self, token: i32, message: String);

    /// Push the shell's current asset-loading snapshot
    /// ([`crate::asset::AssetCache::progress`]); a change fires the game's
    /// `Sub.assets` taggers with the frame's subscription messages. Default:
    /// no-op (drivers without an asset cache — replay, netsim — have nothing
    /// to report).
    fn push_asset_progress(&mut self, _progress: crate::asset::AssetProgress) {}

    /// Take the audio commands the game queued this frame (a JSON array of
    /// [`crate::audio::AudioCommand`]). The shell plays them on its device.
    fn audio_drain_commands(&self) -> String;

    /// The desired soundscape (`soundScape model`) as JSON
    /// ([`crate::audio::AudioScene`]), reconciled by the shell against its
    /// live voices.
    fn audio_scene_json(&self) -> String;

    /// Take the persistent-connection commands (connect/send/close) queued
    /// this frame, as a JSON array of [`crate::net::ConnCommand`].
    fn net_drain_conn_commands(&self) -> String;

    /// Deliver a connection event into the game's inbound queue, tagged with
    /// the connection's key (its endpoint url).
    fn net_push_connected(&mut self, key: String, conn: i32);
    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String);
    fn net_push_disconnected(&mut self, key: String, conn: i32);
    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String);

    /// Report that a `playThen` one-shot (`token`) finished, so the game
    /// delivers its completion message.
    fn audio_push_finished(&mut self, token: i32);

    fn quit(&mut self);
}

#[cfg(test)]
mod tests {
    use crate::audio::{AudioCommand, AudioScene, AudioSource};
    use crate::net::{ConnCommand, HttpMethod, NetCommand, NetInbound};
    use crate::ui::{Anchor, View};
    use crate::{Camera, Frame, FrameTime, Key, Light, Scene3D, SceneObject};

    /// Pin a value's exact wire form: it must serialize to `expected` and
    /// decode back from it. This is what makes a serde rename / retagging /
    /// field change fail a test (round-trip-only checks are self-consistent
    /// and would stay green).
    fn assert_wire<T>(value: &T, expected: &str)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        assert_eq!(serde_json::to_string(value).expect("serialize"), expected);
        let back: T = serde_json::from_str(expected).expect("deserialize");
        assert_eq!(*value, back);
    }

    /// Round-trip a value through JSON and assert the serialized form is
    /// stable (serialize → deserialize → serialize gives the same string).
    /// For types that don't derive `PartialEq` and are too large to pin as a
    /// literal (`Frame`); shape pinning for those relies on the hard-coded
    /// legacy-decode literals below.
    fn assert_json_stable<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    // A representative draw3d output: every Shape variant, every SceneObject
    // variant, transformed geometry, a material wrapper, all four light
    // kinds, and a render-target pass with a scene that samples it.
    #[test]
    fn frame_round_trips() {
        use crate::math::Angle;
        use crate::render_target::RenderTargetDescriptor;
        use crate::{
            MaterialDescription, ModelDescription, ModelHandle, RenderTargetPass, Shape,
            TextureDescription,
        };
        use cgmath::{Matrix4, SquareMatrix};

        let scene = Scene3D {
            obj: SceneObject::Group(vec![
                Scene3D {
                    obj: SceneObject::Material(
                        MaterialDescription::color(1.0, 0.5, 0.25, 1.0),
                        vec![Scene3D::cube()
                            .rotate_y(Angle::from_degrees(45.0))
                            .translate_x(2.0)],
                    ),
                    xform: Matrix4::identity(),
                },
                Scene3D::sphere().scale_y(2.0),
                Scene3D::cylinder(),
                Scene3D::quad(),
                Scene3D::plane(),
                Scene3D {
                    obj: SceneObject::Geometry(Shape::Heightmap {
                        rows: 2,
                        cols: 2,
                        heights: vec![0.0, 0.5, 1.0, 0.25],
                    }),
                    xform: Matrix4::identity(),
                },
                Scene3D::model(ModelDescription {
                    handle: ModelHandle::File("barrel.glb".to_string()),
                    overrides: vec![],
                    animation: None,
                    while_pending: vec![],
                }),
                // A monitor: samples the "feed" render target declared below.
                Scene3D {
                    obj: SceneObject::Material(
                        MaterialDescription::emissive_texture(TextureDescription::RenderTarget(
                            "feed".to_string(),
                        )),
                        vec![Scene3D::quad()],
                    ),
                    xform: Matrix4::identity(),
                },
            ]),
            xform: Matrix4::identity(),
        };
        let frame = Frame {
            camera: Camera::default(),
            scene,
            lights: vec![
                Light::ambient(0.1, 0.1, 0.1),
                Light::directional(-1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.8).cast_shadows(),
                Light::point(0.0, 2.0, 0.0, 1.0, 0.0, 0.0, 1.0, 10.0),
                Light::spot(
                    0.0, 3.0, 0.0, 0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 15.0, 0.5,
                ),
            ],
            render_targets: vec![RenderTargetPass {
                target: RenderTargetDescriptor::new("feed"),
                frame: Frame::new(Camera::default(), Scene3D::cube()),
            }],
            fog: Some(crate::fog::Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7)),
            skybox: Some(crate::skybox::SkyboxDescription::new(
                "px.jpg", "nx.jpg", "py.jpg", "ny.jpg", "pz.jpg", "nz.jpg",
            )),
            clear_color: Some([0.2, 0.4, 0.6]),
        };
        assert_json_stable(&frame);

        // `lights` was added to Frame later (`render_targets` and `fog` later
        // still); a frame serialized without them must still decode
        // (serde(default)) — old producers stay readable. This literal also
        // pins the minimal Frame wire shape.
        let legacy: Frame =
            serde_json::from_str(r#"{"camera":{"eye":[0.0,0.0,-5.0],"target":[0.0,0.0,0.0],"up":[0.0,1.0,0.0],"fov_radians":0.785,"near":0.1,"far":100.0},"scene":{"obj":{"Geometry":"Cube"},"xform":[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0],[0.0,0.0,0.0,1.0]]}}"#)
                .expect("frame without lights decodes");
        assert!(legacy.lights.is_empty());
        assert!(legacy.render_targets.is_empty());
        assert!(legacy.fog.is_none());
        assert!(legacy.skybox.is_none());
        assert!(legacy.clear_color.is_none());
    }

    // The render-target wire vocabulary: the reader (a texture by target id)
    // and the writer's descriptor (id + size). Both sides key on the id string.
    #[test]
    fn render_target_wire_is_pinned() {
        use crate::render_target::RenderTargetDescriptor;
        use crate::TextureDescription;

        assert_wire(
            &TextureDescription::RenderTarget("feed".to_string()),
            r#"{"RenderTarget":"feed"}"#,
        );
        assert_wire(
            &RenderTargetDescriptor::new("feed"),
            r#"{"id":"feed","width":512,"height":512}"#,
        );
    }

    // The `Asset.whilePending` wire vocabulary (protocol v2): the model
    // chain field is OMITTED when empty (chainless frames keep the v1
    // shape), and chained textures use the FileWhilePending variant.
    #[test]
    fn while_pending_wire_is_pinned() {
        use crate::scene3d::{ModelDescription, ModelHandle};
        use crate::TextureDescription;

        // ModelDescription has no PartialEq (matrix overrides), so pin the
        // JSON text and round-trip by re-serialization instead.
        let pin_model = |model: &ModelDescription, expected: &str| {
            let json = serde_json::to_string(model).expect("serialize");
            assert_eq!(json, expected);
            let back: ModelDescription = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        };
        let chainless = ModelDescription {
            handle: ModelHandle::File("boss.glb".to_string()),
            overrides: vec![],
            animation: None,
            while_pending: vec![],
        };
        pin_model(
            &chainless,
            r#"{"handle":{"File":"boss.glb"},"overrides":[],"animation":null}"#,
        );
        let chained = ModelDescription {
            while_pending: vec!["low.glb".to_string(), "cube.glb".to_string()],
            ..chainless
        };
        pin_model(
            &chained,
            r#"{"handle":{"File":"boss.glb"},"overrides":[],"animation":null,"while_pending":["low.glb","cube.glb"]}"#,
        );

        assert_wire(
            &TextureDescription::File("wood.png".to_string()),
            r#"{"File":"wood.png"}"#,
        );
        assert_wire(
            &TextureDescription::FileWhilePending {
                file: "wood.png".to_string(),
                while_pending: vec!["grey.png".to_string()],
            },
            r#"{"FileWhilePending":{"file":"wood.png","while_pending":["grey.png"]}}"#,
        );
    }

    // The skybox wire vocabulary: six face paths in GL upload order.
    #[test]
    fn skybox_wire_is_pinned() {
        use crate::skybox::SkyboxDescription;

        assert_wire(
            &SkyboxDescription::new("px.jpg", "nx.jpg", "py.jpg", "ny.jpg", "pz.jpg", "nz.jpg"),
            r#"{"px":"px.jpg","nx":"nx.jpg","py":"py.jpg","ny":"ny.jpg","pz":"pz.jpg","nz":"nz.jpg"}"#,
        );
    }

    // The fog wire vocabulary: both models, color as a plain [r,g,b].
    #[test]
    fn fog_wire_is_pinned() {
        use crate::fog::Fog;

        assert_wire(
            &Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7),
            r#"{"Linear":{"near":4.0,"far":30.0,"color":[0.5,0.6,0.7]}}"#,
        );
        assert_wire(
            &Fog::exp(0.08, 0.5, 0.6, 0.7),
            r#"{"Exp":{"density":0.08,"color":[0.5,0.6,0.7]}}"#,
        );
    }

    #[test]
    fn frame_time_round_trips() {
        let time = FrameTime { tts: 12.5, dts: 0.016 };
        let json = serde_json::to_string(&time).unwrap();
        let back: FrameTime = serde_json::from_str(&json).unwrap();
        assert_eq!(time.tts, back.tts);
        assert_eq!(time.dts, back.dts);
    }

    // The wire representation of a key is its i32 discriminant (key_event /
    // Input.ofKeyCode), NOT its serde name — anchor the discriminant table so
    // inserting a variant mid-enum (which renumbers everything after it) fails
    // here instead of silently breaking every F# game's input.
    #[test]
    fn key_discriminants_are_pinned() {
        assert_eq!(Key::Unknown as i32, 0);
        assert_eq!(Key::A as i32, 1);
        assert_eq!(Key::Z as i32, 26);
        assert_eq!(Key::Up as i32, 27);
        assert_eq!(Key::Down as i32, 28);
        assert_eq!(Key::Left as i32, 29);
        assert_eq!(Key::Right as i32, 30);
        assert_eq!(Key::Space as i32, 31);
        assert_eq!(Key::Enter as i32, 32);
        assert_eq!(Key::Escape as i32, 33);
        // Serde uses the names (the debug server's held_keys), pinned too.
        assert_wire(&Key::W, r#""W""#);
        assert_wire(&Key::Escape, r#""Escape""#);
    }

    #[test]
    fn net_commands_are_pinned() {
        assert_wire(
            &NetCommand::HttpRequest {
                token: 7,
                method: HttpMethod::Post,
                url: "https://example.com/state".to_string(),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: b"{}".to_vec(),
            },
            r#"{"HttpRequest":{"token":7,"method":"Post","url":"https://example.com/state","headers":[["content-type","application/json"]],"body":[123,125]}}"#,
        );
        // All five ConnCommand variants; Send also pins ConnectionId (u64) and
        // the bytes-as-number-array payload encoding.
        assert_wire(
            &ConnCommand::Connect {
                key: "ws://server".to_string(),
                url: "ws://server".to_string(),
            },
            r#"{"Connect":{"key":"ws://server","url":"ws://server"}}"#,
        );
        assert_wire(
            &ConnCommand::Listen {
                key: "0.0.0.0:9001".to_string(),
                addr: "0.0.0.0:9001".to_string(),
            },
            r#"{"Listen":{"key":"0.0.0.0:9001","addr":"0.0.0.0:9001"}}"#,
        );
        assert_wire(
            &ConnCommand::Send {
                conn: 3,
                payload: vec![1, 2],
            },
            r#"{"Send":{"conn":3,"payload":[1,2]}}"#,
        );
        assert_wire(&ConnCommand::CloseConn { conn: 3 }, r#"{"CloseConn":{"conn":3}}"#);
        assert_wire(
            &ConnCommand::CloseKey {
                key: "ws://server".to_string(),
            },
            r#"{"CloseKey":{"key":"ws://server"}}"#,
        );
        // Both NetInbound variants.
        assert_wire(
            &NetInbound::HttpResponse {
                token: 7,
                status: 200,
                body: b"ok".to_vec(),
            },
            r#"{"HttpResponse":{"token":7,"status":200,"body":[111,107]}}"#,
        );
        assert_wire(
            &NetInbound::HttpError {
                token: 7,
                message: "timeout".to_string(),
            },
            r#"{"HttpError":{"token":7,"message":"timeout"}}"#,
        );
    }

    #[test]
    fn audio_boundary_is_pinned() {
        // The one-shot command in its fullest form (completion token + spatial
        // position — Audio.playThen / Audio.playAt).
        assert_wire(
            &AudioCommand::PlayOneShot {
                token: Some(5),
                sound: "laser.wav".to_string(),
                gain: 0.5,
                position: Some([1.0, 2.0, 3.0]),
            },
            r#"{"PlayOneShot":{"token":5,"sound":"laser.wav","gain":0.5,"position":[1.0,2.0,3.0]}}"#,
        );
        // A command serialized before `position` existed must still decode
        // (serde(default)).
        let legacy: AudioCommand = serde_json::from_str(
            r#"{"PlayOneShot":{"token":null,"sound":"laser.wav","gain":1.0}}"#,
        )
        .unwrap();
        assert_eq!(
            legacy,
            AudioCommand::PlayOneShot {
                token: None,
                sound: "laser.wav".to_string(),
                gain: 1.0,
                position: None,
            }
        );

        let scene = AudioScene::new(vec![
            AudioSource::ambient("bed".to_string(), "wind.ogg".to_string()),
            AudioSource::at("fountain".to_string(), "water.ogg".to_string(), 1.0, 0.0, 2.0),
        ]);
        assert_wire(
            &scene,
            r#"{"sources":[{"key":"bed","sound":"wind.ogg","gain":1.0,"position":null},{"key":"fountain","sound":"water.ogg","gain":1.0,"position":[1.0,0.0,2.0]}]}"#,
        );
        // The exact hop the runtimes use (audio_scene_json).
        let back: AudioScene = serde_json::from_str(&crate::audio::scene_to_json(&scene)).unwrap();
        assert_eq!(scene, back);
    }

    // The UI-interaction event (docs/ui-interaction.md U2): slot-addressed,
    // one variant per widget kind. Crosses the debug-server wire and the
    // recorder's event log.
    #[test]
    fn ui_event_is_pinned() {
        use crate::ui::{UiEvent, UiEventKind};

        assert_wire(
            &UiEvent {
                slot: 0,
                kind: UiEventKind::Clicked,
            },
            r#"{"slot":0,"kind":"Clicked"}"#,
        );
        assert_wire(
            &UiEvent {
                slot: 2,
                kind: UiEventKind::SliderChanged(0.5),
            },
            r#"{"slot":2,"kind":{"SliderChanged":0.5}}"#,
        );
        assert_wire(
            &UiEvent {
                slot: 1,
                kind: UiEventKind::TextChanged("hi".to_string()),
            },
            r#"{"slot":1,"kind":{"TextChanged":"hi"}}"#,
        );
    }

    #[test]
    fn ui_view_is_pinned() {
        let view = View::Panel {
            anchor: Anchor::TopRight,
            child: Box::new(View::Column(vec![
                View::Text {
                    text: "score: 10".to_string(),
                    color: [255, 255, 0],
                    font: None,
                },
                View::Empty,
            ])),
        };
        // View doesn't derive PartialEq, so pin the encode side directly and
        // check the decode side re-encodes identically.
        let expected = r#"{"Panel":{"anchor":"TopRight","child":{"Column":[{"Text":{"text":"score: 10","color":[255,255,0],"font":null}},"Empty"]}}}"#;
        assert_eq!(serde_json::to_string(&view).unwrap(), expected);
        let back: View = serde_json::from_str(expected).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), expected);

        // A Text serialized before the optional `font` field existed must
        // still decode (serde(default)).
        let legacy: View =
            serde_json::from_str(r#"{"Text":{"text":"hi","color":[1,2,3]}}"#).unwrap();
        assert_json_stable(&legacy);

        // The interactive widgets (docs/ui-interaction.md U3/U4): slot-stamped,
        // handlers kept producer-side so the tree stays serializable.
        let button = View::Button {
            slot: 0,
            label: "Reset".to_string(),
        };
        let expected = r#"{"Button":{"slot":0,"label":"Reset"}}"#;
        assert_eq!(serde_json::to_string(&button).unwrap(), expected);
        let back: View = serde_json::from_str(expected).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), expected);

        let slider = View::Slider {
            slot: 1,
            min: 0.0,
            max: 10.0,
            value: 2.5,
        };
        let expected = r#"{"Slider":{"slot":1,"min":0.0,"max":10.0,"value":2.5}}"#;
        assert_eq!(serde_json::to_string(&slider).unwrap(), expected);
        let back: View = serde_json::from_str(expected).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), expected);

        let text_input = View::TextInput {
            slot: 2,
            value: "hi".to_string(),
        };
        let expected = r#"{"TextInput":{"slot":2,"value":"hi"}}"#;
        assert_eq!(serde_json::to_string(&text_input).unwrap(), expected);
        let back: View = serde_json::from_str(expected).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), expected);
    }
}
