//! The logic↔runtime protocol: the versioned contract between a game's pure
//! logic (today the Fable-generated F# dylib/wasm module; later any producer —
//! see `docs/mle.md`, Track A) and the imperative runtime shells.
//!
//! Everything that crosses the boundary is enumerated here, split by whether it
//! crosses **as serializable data** (the protocol proper — language-neutral,
//! introspectable, what a second producer must speak) or **in-process only**
//! (a same-binary handoff that is *not* yet part of the data protocol). The
//! round-trip tests below pin the serialized shape of every data type; changing
//! one is a protocol change and should bump [`PROTOCOL_VERSION`].
//!
//! # Per-frame, runtime → logic
//!
//! - [`crate::FrameTime`] — `tts`/`dts` seconds.
//! - Input events, as scalars: key code ([`crate::Key`] as `i32`) + down flag,
//!   mouse position (`i32` pixels), wheel delta (`i32`). The `Key` enum is the
//!   canonical code space on both sides.
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
//! # In-process only (NOT part of the data protocol — known limitations)
//!
//! - [`crate::OpaqueState`] — the hot-reload state bundle (model + pending
//!   effect queue) is a `Box<dyn Any>` moved between dylib generations with a
//!   layout-compatibility assumption, not serialized data. A data-native state
//!   representation is what makes state durable/inspectable across producers
//!   (`docs/mle.md`, Track C).
//! - [`crate::Effect`] / [`crate::EffectQueue`] — effect *commands* cross as
//!   data (above), but message payloads and `Http` taggers are closures, so
//!   the queue itself cannot cross the boundary.

/// Version of the serialized logic↔runtime contract. Bump when the serialized
/// shape of any type enumerated in this module changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioScene, AudioSource};
    use crate::net::{ConnCommand, HttpMethod, NetCommand, NetInbound};
    use crate::ui::{Anchor, View};
    use crate::{Camera, Frame, FrameTime, Key, Light, Scene3D, SceneObject};

    /// Round-trip a value through JSON and assert the serialized form is
    /// stable (serialize → deserialize → serialize gives the same string).
    /// Used for types without `PartialEq` (e.g. anything holding a `Matrix4`).
    fn assert_json_stable<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    /// Round-trip a `PartialEq` value through JSON and assert equality.
    fn assert_round_trips<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*value, back);
    }

    #[test]
    fn version_is_pinned() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    // A representative draw3d output: every SceneObject variant, transformed
    // geometry, a material wrapper, and all four light kinds.
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
            ],
        };
        assert_json_stable(&frame);

        // `lights` was added to Frame later; a frame serialized without it
        // must still decode (serde(default)) — old producers stay readable.
        let legacy: Frame =
            serde_json::from_str(r#"{"camera":{"eye":[0.0,0.0,-5.0],"target":[0.0,0.0,0.0],"up":[0.0,1.0,0.0],"fov_radians":0.785,"near":0.1,"far":100.0},"scene":{"obj":{"Geometry":"Cube"},"xform":[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0],[0.0,0.0,0.0,1.0]]}}"#)
                .expect("frame without lights decodes");
        assert!(legacy.lights.is_empty());
    }

    #[test]
    fn frame_time_and_key_codes_round_trip() {
        let time = FrameTime { tts: 12.5, dts: 0.016 };
        let json = serde_json::to_string(&time).unwrap();
        let back: FrameTime = serde_json::from_str(&json).unwrap();
        assert_eq!(time.tts, back.tts);
        assert_eq!(time.dts, back.dts);

        for key in [Key::A, Key::Z, Key::Up, Key::Space, Key::Escape, Key::Unknown] {
            assert_round_trips(&key);
        }
    }

    #[test]
    fn net_commands_round_trip() {
        assert_round_trips(&NetCommand::HttpRequest {
            token: 7,
            method: HttpMethod::Post,
            url: "https://example.com/state".to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"{}".to_vec(),
        });
        for cmd in [
            ConnCommand::Connect {
                key: "ws://server".to_string(),
                url: "ws://server".to_string(),
            },
            ConnCommand::Listen {
                key: "0.0.0.0:9001".to_string(),
                addr: "0.0.0.0:9001".to_string(),
            },
            ConnCommand::CloseKey {
                key: "ws://server".to_string(),
            },
        ] {
            assert_round_trips(&cmd);
        }
        assert_round_trips(&NetInbound::HttpResponse {
            token: 7,
            status: 200,
            body: b"ok".to_vec(),
        });
    }

    #[test]
    fn audio_boundary_round_trips() {
        let scene = AudioScene::new(vec![
            AudioSource::ambient("bed".to_string(), "wind.ogg".to_string()),
            AudioSource::at("fountain".to_string(), "water.ogg".to_string(), 1.0, 0.0, 2.0),
        ]);
        assert_round_trips(&scene);
        // The exact hop the runtimes use (audio_scene_json).
        let json = crate::audio::scene_to_json(&scene);
        let back: AudioScene = serde_json::from_str(&json).unwrap();
        assert_eq!(scene, back);
    }

    #[test]
    fn ui_view_round_trips() {
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
        assert_json_stable(&view);

        // A Text serialized before the optional `font` field existed must
        // still decode (serde(default)).
        let legacy: View =
            serde_json::from_str(r#"{"Text":{"text":"hi","color":[1,2,3]}}"#).unwrap();
        assert_json_stable(&legacy);
    }
}
