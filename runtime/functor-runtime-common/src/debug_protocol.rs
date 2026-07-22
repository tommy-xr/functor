//! Transport-neutral wire contract for Functor runtime debugging.
//!
//! Desktop and device runtimes expose this same protocol. Their HTTP servers
//! only parse/encode transport details; requests cross to the runtime loop as
//! [`DebugRequest`] values so rendering and game state remain on that loop's
//! thread.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

use crate::{ui::UiEventKind, Key};

/// Stable name returned by the discovery endpoint on every runtime target.
pub const DEBUG_PROTOCOL_SERVICE: &str = "functor debug runtime";

/// Version of the routes and JSON wire shapes in this module.
pub const DEBUG_PROTOCOL_VERSION: u32 = 1;

/// Maximum accepted body size for either reload operation.
pub const MAX_RELOAD_BYTES: usize = 4 * 1024 * 1024;

/// One endpoint in the canonical debug-runtime surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugRoute {
    pub method: &'static str,
    pub path: &'static str,
    pub description: &'static str,
}

impl DebugRoute {
    /// Discovery-map key used by the existing desktop protocol.
    pub fn label(self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

/// The complete endpoint surface. Both desktop and XR discovery responses are
/// built from this table so adding a route cannot silently create API drift.
pub const DEBUG_ROUTES: &[DebugRoute] = &[
    DebugRoute {
        method: "GET",
        path: "/",
        description: "this endpoint index",
    },
    DebugRoute {
        method: "POST",
        path: "/capture",
        description: "PNG (image/png) of the next rendered frame",
    },
    DebugRoute {
        method: "GET",
        path: "/state",
        description: "runtime state JSON: frame, tts, viewport, views, input (held_keys + mouse), model (Debug text)",
    },
    DebugRoute {
        method: "GET",
        path: "/scene",
        description: "current frame as JSON: camera + scene + lights",
    },
    DebugRoute {
        method: "GET",
        path: "/trace",
        description: "paused-inspector trace: last real frame's entry-point invocations (bindings + result) replayed while paused; {paused:false, invocations:[]} while playing",
    },
    DebugRoute {
        method: "POST",
        path: "/input",
        description: "inject input — {\"type\":\"key\",\"key\":\"w\",\"down\":true} | {\"type\":\"mouse_move\",\"x\":0,\"y\":0} | {\"type\":\"mouse_wheel\",\"delta\":1} | {\"type\":\"ui_event\",\"slot\":0,\"kind\":\"Clicked\"} | {\"type\":\"webview_event\",\"slot\":0,\"kind\":\"Clicked\"}",
    },
    DebugRoute {
        method: "POST",
        path: "/time",
        description: "clock control — {\"type\":\"set\",\"tts\":2.0} (pause) | {\"type\":\"advance\",\"dts\":0.016} (step one frame) | {\"type\":\"resume\"}",
    },
    DebugRoute {
        method: "POST",
        path: "/reload-source",
        description: "swap game logic from the request body (raw .fun source), model preserved — 400 with the load error on a broken push",
    },
    DebugRoute {
        method: "POST",
        path: "/reload-project",
        description: "swap the whole project from a JSON array of [path, source] pairs (entry first), model preserved — 400 with the load error on a broken push",
    },
    DebugRoute {
        method: "POST",
        path: "/rewind",
        description: "coupled scene rewind — {\"frame\":42} restores model + physics to that rendered frame (pin the clock first); 400 if unrecorded/pruned",
    },
];

/// Build the JSON body returned by `GET /` on every runtime target.
pub fn discovery_json() -> String {
    let endpoints: BTreeMap<_, _> = DEBUG_ROUTES
        .iter()
        .map(|route| (route.label(), route.description))
        .collect();
    serde_json::json!({
        "service": DEBUG_PROTOCOL_SERVICE,
        "protocol_version": DEBUG_PROTOCOL_VERSION,
        "endpoints": endpoints,
    })
    .to_string()
}

/// A pixel rectangle in the runtime's output surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeViewport {
    pub width: u32,
    pub height: u32,
}

impl RuntimeViewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// One rendered view. Desktop reports one `main` view; stereo XR reports one
/// entry per eye. `name` is descriptive rather than a closed enum so future
/// runtimes can add views without revising the protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeView {
    pub name: String,
    pub viewport: RuntimeViewport,
}

impl RuntimeView {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            viewport: RuntimeViewport::new(width, height),
        }
    }
}

/// Runtime-owned input state, independent of the game model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInput {
    pub held_keys: Vec<Key>,
    pub mouse: RuntimeMouse,
}

/// Last known pointer position in output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMouse {
    pub x: i32,
    pub y: i32,
}

/// Snapshot returned by `GET /state`.
///
/// `viewport`, `input`, and `model` retain the desktop wire shape. `views` is
/// the target-neutral representation of mono or stereo output; `viewport` is
/// retained as the primary/legacy output extent for SDK compatibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub frame: u64,
    pub tts: f32,
    pub viewport: RuntimeViewport,
    pub views: Vec<RuntimeView>,
    pub model: String,
    pub input: RuntimeInput,
}

impl RuntimeState {
    /// Serialize with serde so multi-line, quote-bearing model text is escaped.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("RuntimeState contains only serializable values")
    }
}

/// An event injected by `POST /input`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputCommand {
    Key { key: String, down: bool },
    MouseMove { x: i32, y: i32 },
    MouseWheel { delta: i32 },
    UiEvent { slot: u32, kind: UiEventKind },
    WebviewEvent { slot: u32, kind: UiEventKind },
}

/// A clock command sent through `POST /time`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimeCommand {
    Set { tts: f32 },
    Advance { dts: f32 },
    Resume,
}

/// A coupled model-and-physics rewind sent through `POST /rewind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct RewindCommand {
    pub frame: u64,
}

/// Why `POST /capture` could not return pixels.
pub enum CaptureError {
    /// No framebuffer is available, such as in desktop headless mode (HTTP 503).
    Unavailable(String),
    /// Readback or PNG encoding failed (HTTP 500).
    Failed(String),
}

/// A whole-project push: `(path, source)` pairs with the entry first.
pub type ProjectSources = Vec<(String, String)>;

/// Request delivered from a runtime's transport thread to its frame loop.
pub enum DebugRequest {
    Capture(Sender<Result<Vec<u8>, CaptureError>>),
    State(Sender<RuntimeState>),
    Scene(Sender<String>),
    Trace(Sender<String>),
    Input(InputCommand, Sender<Result<(), String>>),
    Time(TimeCommand, Sender<()>),
    ReloadSource(String, Sender<Result<String, String>>),
    ReloadProject(ProjectSources, Sender<Result<String, String>>),
    Rewind(u64, Sender<Result<String, String>>),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{json, Value};

    use super::*;

    #[test]
    fn runtime_state_json_preserves_desktop_shape_and_reports_views() {
        let state = RuntimeState {
            frame: 42,
            tts: 1.5,
            viewport: RuntimeViewport::new(1920, 1080),
            views: vec![RuntimeView::new("main", 1920, 1080)],
            model: "Model {\n  label: \"hello\"\n}".into(),
            input: RuntimeInput {
                held_keys: vec![Key::W, Key::Up],
                mouse: RuntimeMouse { x: 10, y: 20 },
            },
        };

        let actual: Value = serde_json::from_str(&state.to_json()).unwrap();
        assert_eq!(
            actual,
            json!({
                "frame": 42,
                "tts": 1.5,
                "viewport": { "width": 1920, "height": 1080 },
                "views": [{
                    "name": "main",
                    "viewport": { "width": 1920, "height": 1080 }
                }],
                "model": "Model {\n  label: \"hello\"\n}",
                "input": {
                    "held_keys": ["W", "Up"],
                    "mouse": { "x": 10, "y": 20 }
                }
            })
        );
    }

    #[test]
    fn command_decoding_matches_the_existing_wire_shapes() {
        assert_eq!(
            serde_json::from_str::<InputCommand>(r#"{"type":"key","key":"w","down":true}"#)
                .unwrap(),
            InputCommand::Key {
                key: "w".into(),
                down: true
            }
        );
        assert_eq!(
            serde_json::from_str::<InputCommand>(
                r#"{"type":"ui_event","slot":3,"kind":{"SliderChanged":0.5}}"#
            )
            .unwrap(),
            InputCommand::UiEvent {
                slot: 3,
                kind: UiEventKind::SliderChanged(0.5)
            }
        );
        assert_eq!(
            serde_json::from_str::<TimeCommand>(r#"{"type":"advance","dts":0.016}"#).unwrap(),
            TimeCommand::Advance { dts: 0.016 }
        );
        assert_eq!(
            serde_json::from_str::<RewindCommand>(r#"{"frame":42}"#).unwrap(),
            RewindCommand { frame: 42 }
        );
    }

    #[test]
    fn routes_are_unique_complete_and_drive_discovery() {
        let labels: BTreeSet<_> = DEBUG_ROUTES.iter().map(|route| route.label()).collect();
        assert_eq!(labels.len(), DEBUG_ROUTES.len(), "duplicate method/path");

        let expected: BTreeSet<_> = [
            "GET /",
            "POST /capture",
            "GET /state",
            "GET /scene",
            "GET /trace",
            "POST /input",
            "POST /time",
            "POST /reload-source",
            "POST /reload-project",
            "POST /rewind",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(labels, expected);

        let discovery: Value = serde_json::from_str(&discovery_json()).unwrap();
        let endpoints = discovery["endpoints"].as_object().unwrap();
        assert_eq!(endpoints.len(), DEBUG_ROUTES.len());
        for route in DEBUG_ROUTES {
            assert_eq!(
                endpoints[&route.label()],
                Value::String(route.description.into())
            );
        }
    }

    #[test]
    fn discovery_reports_the_protocol_identity_and_version() {
        let discovery: Value = serde_json::from_str(&discovery_json()).unwrap();
        assert_eq!(discovery["service"], DEBUG_PROTOCOL_SERVICE);
        assert_eq!(discovery["protocol_version"], DEBUG_PROTOCOL_VERSION);
        assert_eq!(DEBUG_PROTOCOL_VERSION, 1);
    }
}
