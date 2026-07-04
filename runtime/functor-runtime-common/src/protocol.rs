//! The logic↔runtime protocol: the versioned contract between a game's pure
//! logic (today the Fable-generated F# dylib/wasm module; later any producer —
//! see `docs/mle.md`, Track A) and the imperative runtime shells.
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
//!   what makes state durable/inspectable across producers (`docs/mle.md`,
//!   Track C).
//! - [`crate::Effect`] / [`crate::EffectQueue`] — effect *commands* cross as
//!   data (above), but message payloads and `Http` taggers are closures, so
//!   the queue itself cannot cross the boundary.
//! - Control signals with no payload: the dylib `init` entry point and `quit`.

/// Version of the serialized logic↔runtime contract. Bump when the wire shape
/// of any type enumerated in this module changes incompatibly. Informational
/// for now — nothing transmits or checks it; [`GameProducer`] impls all speak
/// the current version.
pub const PROTOCOL_VERSION: u32 = 1;

/// The producer side of the protocol: one game logic instance as consumed by a
/// runtime shell's frame loop. Every method carries a payload enumerated in
/// this module's boundary doc (drains return JSON arrays, pushes take the
/// inbox scalars, `render` returns the [`crate::Frame`]).
///
/// Impls: the desktop runner's dylib producers (`StaticGame`,
/// `HotReloadGame`), the web runtime's `WasmGame` bridge over the
/// `wasm_bindgen` game exports, and (Track C) the MLE interpreter. The seam
/// exists so a producer can be swapped without the shells knowing what
/// language or pipeline produced the logic — see `docs/mle.md`, Track A2.
///
/// Some capabilities are shell-specific; producers for shells that lack them
/// implement the honest no-op (e.g. the web bridge's `check_hot_reload` — the
/// browser reloads the whole page — and `audio_push_finished` — web one-shots
/// are fire-and-forget).
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
        Err("this producer does not support source reload (not an .mle game)".to_string())
    }

    fn tick(&mut self, frame_time: crate::FrameTime);

    /// Deliver a keyboard event. `code` is a [`crate::Key`] as `i32`.
    fn key_event(&mut self, code: i32, is_down: bool);

    /// Deliver a mouse-move event in window pixel coordinates.
    fn mouse_move(&mut self, x: i32, y: i32);

    /// Deliver a mouse-wheel event (vertical scroll offset).
    fn mouse_wheel(&mut self, delta: i32);

    fn render(&mut self, frame_time: crate::FrameTime) -> crate::Frame;

    /// The game's declarative UI tree (`ui model`), lowered by the shell to a
    /// text overlay drawn on top of the frame.
    fn ui(&self) -> crate::ui::View;

    /// A pretty-printed (Rust `Debug`) view of the live game model, for
    /// introspection (the debug server's `GET /state` `model` field). Opaque
    /// debug text — see the module doc; consumers must not parse it.
    fn state_debug(&self) -> String;

    /// Take the networking commands the game queued this frame (a JSON array
    /// of [`crate::net::NetCommand`]). The shell performs the I/O and reports
    /// results back with `net_push_http_response` / `net_push_http_error`.
    fn net_drain_commands(&self) -> String;

    /// Deliver a completed HTTP response into the game's async inbox.
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String);

    /// Deliver a transport-level failure for a request into the async inbox.
    fn net_push_http_error(&mut self, token: i32, message: String);

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
    // variant, transformed geometry, a material wrapper, and all four light
    // kinds.
    #[test]
    fn frame_round_trips() {
        use crate::math::Angle;
        use crate::{MaterialDescription, ModelDescription, ModelHandle};
        use cgmath::{Matrix4, SquareMatrix};
        use fable_library_rust::NativeArray_;

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
                Scene3D::heightmap(2, 2, NativeArray_::array_from(vec![0.0, 0.5, 1.0, 0.25])),
                Scene3D::model(ModelDescription {
                    handle: ModelHandle::File("barrel.glb".to_string()),
                    overrides: vec![],
                }),
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
        };
        assert_json_stable(&frame);

        // `lights` was added to Frame later; a frame serialized without it
        // must still decode (serde(default)) — old producers stay readable.
        // This literal also pins the minimal Frame wire shape.
        let legacy: Frame =
            serde_json::from_str(r#"{"camera":{"eye":[0.0,0.0,-5.0],"target":[0.0,0.0,0.0],"up":[0.0,1.0,0.0],"fov_radians":0.785,"near":0.1,"far":100.0},"scene":{"obj":{"Geometry":"Cube"},"xform":[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0],[0.0,0.0,0.0,1.0]]}}"#)
                .expect("frame without lights decodes");
        assert!(legacy.lights.is_empty());
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
    }
}
