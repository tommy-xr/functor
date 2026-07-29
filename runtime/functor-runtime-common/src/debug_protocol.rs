//! Transport-neutral wire contract for Functor runtime debugging.
//!
//! Desktop and device runtimes expose this same protocol. Their HTTP servers
//! only parse/encode transport details; requests cross to the runtime loop as
//! [`DebugRequest`] values so rendering and game state remain on that loop's
//! thread.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

use crate::{ui::UiEventKind, InputSnapshot, XrInputSnapshot};

/// Stable name returned by the discovery endpoint on every runtime target.
pub const DEBUG_PROTOCOL_SERVICE: &str = "functor debug runtime";

/// Version of the routes and JSON wire shapes in this module.
///
/// 3 added `pending_steps` to `GET /state` and `frames` to `POST /time`'s
/// `advance`, and made `/time` answer 409 under a `--fixed-time` pin. A client
/// that batches advances or waits on `pending_steps` needs a v3 runtime: a v2
/// one ignores `frames`, runs a single step, and reports no `pending_steps`.
///
/// 4 made `GET /state`'s `model` the structured (total, lossy) JSON view of
/// the model and moved the Rust-`Debug` pretty-print to `model_debug`. This
/// REDEFINES `model`: a pre-v4 runtime sends the Debug text under `model`
/// and has no `model_debug`, so clients must gate on the version before
/// reading `model` as data.
///
/// 5 added `GET /project`, the read half of the project-push routes: the
/// `.fun` sources the program is CURRENTLY running, which for a
/// wire-authored session exist nowhere else. Purely additive — every v4
/// shape is unchanged, so a v4 client needs no revision and only a caller of
/// `/project` needs a v5 runtime (a v4 one answers 404).
///
/// 6 added deterministic fixed-step edge fields to `GET /state`'s `input`:
/// `pressed_keys`, `released_keys`, `mouse.pressed`, and `mouse.released`.
/// This is additive; clients that support older runtimes can treat absent
/// fields as empty.
///
/// 7 added `POST /time {"type":"cancel"}` so an automation driver can abort
/// a queued batch without leaving clock work behind.
pub const DEBUG_PROTOCOL_VERSION: u32 = 7;

/// The well-known localhost port `functor develop` serves this protocol on
/// when no explicit `--debug-port` is given, so an agent can attach to a
/// human's live session without being told a port number.
pub const DEFAULT_DEVELOP_PORT: u16 = 8077;

/// Maximum accepted body size for either reload operation.
pub const MAX_RELOAD_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted size of one uploaded project asset. Assets transfer one
/// at a time so a project with several large models never has to exist as one
/// giant request in either the CLI or the runtime.
pub const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;

/// Maximum UTF-8 byte length of an uploaded asset's project-relative path.
pub const MAX_ASSET_PATH_BYTES: usize = 4 * 1024;

/// Maximum JSON size of a complete uploaded-asset path manifest. This is
/// intentionally larger than ordinary debug commands: projects can contain
/// thousands of individually small assets.
pub const MAX_ASSET_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

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
        description: "runtime state JSON: frame, tts, pending_steps (queued clock steps not yet run), viewport, views, input snapshot (held_keys + mouse + optional xr), model (structured lossy JSON view of the model), model_debug (Rust Debug text)",
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
        description: "inject input — {\"type\":\"key\",\"key\":\"w\",\"down\":true} | {\"type\":\"mouse_move\",\"x\":0,\"y\":0} | {\"type\":\"mouse_wheel\",\"delta\":1} | {\"type\":\"mouse_button\",\"button\":\"left\",\"down\":true} (edge + held level, like key) | {\"type\":\"ui_event\",\"slot\":0,\"kind\":\"Clicked\"} | {\"type\":\"webview_event\",\"slot\":0,\"kind\":\"Clicked\"} | {\"type\":\"xr\",\"left\":{...},\"right\":{...},\"head\":{...}} (desktop only; level state until the next xr command) | {\"type\":\"xr_clear\"} (drop it, restoring the emulator or no device)",
    },
    DebugRoute {
        method: "POST",
        path: "/time",
        description: "clock control — {\"type\":\"set\",\"tts\":2.0} (pause) | {\"type\":\"advance\",\"dts\":0.016,\"frames\":1} (queue that many steps; advances accumulate) | {\"type\":\"cancel\"} (drop queued steps, stay paused) | {\"type\":\"resume\"} — 409 while --fixed-time pins the clock",
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
        path: "/load-project",
        description: "load a new whole project from a JSON array of [path, source] pairs (entry first), model initialized from init — 400 with the load error on a broken push",
    },
    DebugRoute {
        method: "GET",
        path: "/project",
        description: "the running program's own .fun sources as a JSON array of [path, source] pairs (entry first) — the wire truth after a pushed edit; 501 for producers whose logic is not source-shaped",
    },
    DebugRoute {
        method: "POST",
        path: "/reload-asset",
        description: "upload one project asset as a binary path+bytes envelope and evict its decoded render data",
    },
    DebugRoute {
        method: "POST",
        path: "/sync-assets",
        description: "finish an asset sync from a JSON array of current project-relative paths, removing uploads absent from the manifest",
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

/// Snapshot returned by `GET /state`.
///
/// `viewport`, `input`, and `model` retain the desktop wire shape. `views` is
/// the target-neutral representation of mono or stereo output; `viewport` is
/// retained as the primary/legacy output extent for SDK compatibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub frame: u64,
    pub tts: f32,
    /// Clock steps queued by `POST /time` `advance` and not yet run. Zero means
    /// every requested step has been simulated, which is how a harness knows a
    /// batched advance has fully landed without guessing a frame target.
    pub pending_steps: u32,
    pub viewport: RuntimeViewport,
    pub views: Vec<RuntimeView>,
    /// The structured JSON view of the model
    /// ([`crate::protocol::GameProducer::state_json`]) — the default thing to
    /// read: parseable, total, lossy (callables/host values are sigil
    /// placeholders). `Null` for producers without a structured model.
    /// `default` because pre-v4 payloads carry Debug TEXT under this key —
    /// version-gate before reading (see [`DEBUG_PROTOCOL_VERSION`]).
    #[serde(default)]
    pub model: serde_json::Value,
    /// The Rust-`Debug` pretty-print of the model — the human/eyeball view,
    /// strictly more faithful where `model` is lossy (full depth,
    /// construction order, closure params). Opaque text; don't parse it.
    #[serde(default)]
    pub model_debug: String,
    pub input: InputSnapshot,
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
    /// A mouse-button edge, `button` spelled `"left"` / `"right"` /
    /// `"middle"` (the [`crate::MouseButton::from_name`] wire spelling).
    ///
    /// Both an edge AND level state, exactly like `key`: it calls the game's
    /// `mouseButton` hook and updates the held buttons the next step's
    /// `sampledInput` samples — so full-auto fire is scriptable by holding
    /// `down: true` across several `/time advance` steps.
    MouseButton { button: String, down: bool },
    UiEvent { slot: u32, kind: UiEventKind },
    WebviewEvent { slot: u32, kind: UiEventKind },
    /// Set the XR device sample the next fixed step's `sampledInput` sees, so
    /// tracked poses, grips, and buttons are scriptable without a headset.
    ///
    /// Level state, not an edge event: the injected sample stays in force until
    /// the next `xr` command replaces it, exactly like a held key. It is a
    /// WHOLE-sample replacement — a field the body omits takes its default
    /// (inactive hand, no pose, `0.0`), so a driver sends both hands each step.
    /// Boxed: the sample is an order of magnitude larger than the other
    /// commands, and would otherwise inflate every one of them.
    Xr(Box<XrInputSnapshot>),
    /// Drop an injected sample, restoring whatever the runtime would sample on
    /// its own — the `--emulate-xr` rig, or no `xr` domain at all.
    ///
    /// The release half of `Xr`'s held-key contract. Without it injection is a
    /// one-way door: the first `xr` command would disable the emulator and make
    /// a game's "no XR device" branch unreachable for the rest of the process.
    XrClear,
}

/// A clock command sent through `POST /time`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimeCommand {
    Set { tts: f32 },
    /// Step the clock, then hold. `frames` (default 1) is the BATCH form: it
    /// queues that many `dts` steps in one request instead of one round trip
    /// per step. Queued steps accumulate — `n` advances always run `n` steps.
    Advance {
        dts: f32,
        #[serde(default = "one_frame")]
        frames: u32,
    },
    /// Drop queued debug/fixed steps without changing the current game time,
    /// and remain paused. This is the safe abort twin of `Advance`.
    Cancel,
    Resume,
}

fn one_frame() -> u32 {
    1
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

/// One project-relative asset uploaded by `POST /reload-asset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The set of uploaded project assets that should remain available.
pub type ProjectAssetPaths = Vec<String>;

/// Encode one asset for `POST /reload-asset`: a big-endian u32 path length,
/// UTF-8 project-relative path, then the raw file bytes.
pub fn encode_project_asset(path: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    validate_project_asset_path(path)?;
    let path_len = u32::try_from(path.len()).map_err(|_| "asset path is too long".to_string())?;
    let mut body = Vec::with_capacity(4 + path.len() + bytes.len());
    body.extend_from_slice(&path_len.to_be_bytes());
    body.extend_from_slice(path.as_bytes());
    body.extend_from_slice(bytes);
    Ok(body)
}

/// Decode the binary body accepted by `POST /reload-asset`.
pub fn decode_project_asset(body: Vec<u8>) -> Result<ProjectAsset, String> {
    if body.len() < 4 {
        return Err("asset body is missing its path length".to_string());
    }
    let path_len = u32::from_be_bytes(body[..4].try_into().unwrap()) as usize;
    if path_len > MAX_ASSET_PATH_BYTES {
        return Err(format!(
            "asset path is too long ({path_len} bytes; limit is {MAX_ASSET_PATH_BYTES})"
        ));
    }
    let path_end = 4usize
        .checked_add(path_len)
        .filter(|end| *end <= body.len())
        .ok_or_else(|| "asset body is shorter than its declared path".to_string())?;
    let path = std::str::from_utf8(&body[4..path_end])
        .map_err(|_| "asset path must be UTF-8".to_string())?
        .to_string();
    validate_project_asset_path(&path)?;
    let bytes_len = body.len() - path_end;
    if bytes_len > MAX_ASSET_BYTES {
        return Err(format!(
            "asset is too large ({bytes_len} bytes; limit is {MAX_ASSET_BYTES})"
        ));
    }
    Ok(ProjectAsset {
        path,
        bytes: body[path_end..].to_vec(),
    })
}

/// Asset locators uploaded from a project must be portable, relative paths.
/// The bytes remain in memory, but rejecting ambiguous/escaping names keeps
/// browser, desktop, and Quest lookups identical.
pub fn validate_project_asset_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("asset path must not be empty".to_string());
    }
    if path.len() > MAX_ASSET_PATH_BYTES {
        return Err(format!(
            "asset path is too long ({} bytes; limit is {MAX_ASSET_PATH_BYTES})",
            path.len()
        ));
    }
    if path.contains('\\') {
        return Err("asset path must use forward slashes".to_string());
    }
    if path.contains('\0') || path.contains("://") || path.starts_with('/') {
        return Err("asset path must be project-relative".to_string());
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err("asset path must not contain empty, `.` or `..` segments".to_string());
    }
    Ok(())
}

/// Request delivered from a runtime's transport thread to its frame loop.
pub enum DebugRequest {
    Capture(Sender<Result<Vec<u8>, CaptureError>>),
    State(Sender<RuntimeState>),
    Scene(Sender<String>),
    Trace(Sender<String>),
    Input(InputCommand, Sender<Result<(), String>>),
    /// `Err` is a conflict the operator must resolve — today, a `/time` command
    /// under an unconditional `--fixed-time` pin, which the clock cannot honor.
    Time(TimeCommand, Sender<Result<(), String>>),
    /// `None` when the producer's logic is not source-shaped (a replay, a
    /// compiled dylib) — reported as 501 rather than an empty project.
    Project(Sender<Option<ProjectSources>>),
    ReloadSource(String, Sender<Result<String, String>>),
    ReloadProject(ProjectSources, Sender<Result<String, String>>),
    LoadProject(ProjectSources, Sender<Result<String, String>>),
    ReloadAsset(ProjectAsset, Sender<Result<String, String>>),
    SyncAssets(ProjectAssetPaths, Sender<Result<String, String>>),
    Rewind(u64, Sender<Result<String, String>>),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::{Key, TrackingPose};
    use serde_json::{json, Value};

    use super::*;

    #[test]
    fn runtime_state_json_preserves_desktop_shape_and_reports_views() {
        let state = RuntimeState {
            frame: 42,
            tts: 1.5,
            pending_steps: 3,
            viewport: RuntimeViewport::new(1920, 1080),
            views: vec![RuntimeView::new("main", 1920, 1080)],
            model: json!({ "label": "hello" }),
            model_debug: "Model {\n  label: \"hello\"\n}".into(),
            input: InputSnapshot {
                held_keys: vec![Key::W, Key::Up],
                mouse: crate::MouseSnapshot {
                    x: 10,
                    y: 20,
                    ..Default::default()
                },
                xr: None,
                ..InputSnapshot::default()
            },
        };

        let actual: Value = serde_json::from_str(&state.to_json()).unwrap();
        assert_eq!(
            actual,
            json!({
                "frame": 42,
                "tts": 1.5,
                "pending_steps": 3,
                "viewport": { "width": 1920, "height": 1080 },
                "views": [{
                    "name": "main",
                    "viewport": { "width": 1920, "height": 1080 }
                }],
                "model": { "label": "hello" },
                "model_debug": "Model {\n  label: \"hello\"\n}",
                "input": {
                    "held_keys": ["W", "Up"],
                    "pressed_keys": [],
                    "released_keys": [],
                    "mouse": {
                        "x": 10,
                        "y": 20,
                        "buttons": { "left": false, "right": false, "middle": false },
                        "pressed": { "left": false, "right": false, "middle": false },
                        "released": { "left": false, "right": false, "middle": false }
                    }
                }
            })
        );
    }

    #[test]
    fn project_asset_binary_round_trips_nested_paths_and_bytes() {
        let body = encode_project_asset("models/ship.glb", &[0, 1, 2, 255]).unwrap();
        assert_eq!(
            decode_project_asset(body).unwrap(),
            ProjectAsset {
                path: "models/ship.glb".into(),
                bytes: vec![0, 1, 2, 255],
            }
        );
    }

    #[test]
    fn project_asset_paths_reject_escaping_or_ambiguous_names() {
        for path in [
            "",
            "/ship.glb",
            "../ship.glb",
            "models/../ship.glb",
            "models//ship.glb",
            "models\\ship.glb",
            "https://example.com/ship.glb",
        ] {
            assert!(
                validate_project_asset_path(path).is_err(),
                "should reject {path:?}"
            );
        }
        assert!(validate_project_asset_path("models/ship.glb").is_ok());
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
            serde_json::from_str::<InputCommand>(
                r#"{"type":"mouse_button","button":"left","down":true}"#
            )
            .unwrap(),
            InputCommand::MouseButton {
                button: "left".into(),
                down: true
            }
        );
        assert_eq!(
            serde_json::from_str::<TimeCommand>(r#"{"type":"advance","dts":0.016}"#).unwrap(),
            TimeCommand::Advance {
                dts: 0.016,
                frames: 1
            },
            "an advance with no `frames` is still exactly one step"
        );
        assert_eq!(
            serde_json::from_str::<TimeCommand>(r#"{"type":"advance","dts":0.016,"frames":120}"#)
                .unwrap(),
            TimeCommand::Advance {
                dts: 0.016,
                frames: 120
            }
        );
        assert_eq!(
            serde_json::from_str::<TimeCommand>(r#"{"type":"cancel"}"#).unwrap(),
            TimeCommand::Cancel
        );
        assert_eq!(
            serde_json::from_str::<RewindCommand>(r#"{"frame":42}"#).unwrap(),
            RewindCommand { frame: 42 }
        );
    }

    #[test]
    fn an_xr_command_decodes_a_whole_sample_and_defaults_what_it_omits() {
        let command = serde_json::from_str::<InputCommand>(
            r#"{"type":"xr",
                "head": { "position": [0.0, 0.1, 0.0] },
                "left": { "active": true,
                          "grip": { "position": [-0.3, -0.1, -0.6],
                                    "orientation": [0.0, 0.38, 0.0, 0.92] } },
                "right": { "active": true,
                           "grip": { "position": [-0.05, -0.05, 0.12] },
                           "trigger": 1.0 }}"#,
        )
        .expect("xr command decodes");
        let InputCommand::Xr(sample) = command else {
            panic!("expected an xr command");
        };

        // An omitted `orientation` is IDENTITY, not an all-zero quaternion
        // (which is not a rotation at all).
        assert_eq!(
            sample.head,
            Some(TrackingPose::new([0.0, 0.1, 0.0], [0.0, 0.0, 0.0, 1.0]))
        );
        assert_eq!(
            sample.left.grip,
            Some(TrackingPose::new([-0.3, -0.1, -0.6], [0.0, 0.38, 0.0, 0.92]))
        );
        // Omitted controller fields take their defaults: no `aim` pose, and the
        // left hand's trigger is released even though the right hand's is held.
        assert_eq!(sample.left.aim, None);
        assert_eq!(sample.left.trigger, 0.0);
        assert_eq!(sample.right.trigger, 1.0);
        assert_eq!(sample.right.thumbstick, [0.0, 0.0]);
        assert!(!sample.right.primary_pressed);

        // The minimal body is a fully-default sample: both hands inactive,
        // no head pose — i.e. "XR present, nothing tracked".
        let bare = serde_json::from_str::<InputCommand>(r#"{"type":"xr"}"#).unwrap();
        assert_eq!(bare, InputCommand::Xr(Box::default()));
    }

    /// Every field of an `xr` body is optional, so WITHOUT `deny_unknown_fields`
    /// a typo is not a no-op — it is worse: the command still succeeds and
    /// installs an all-default sample, flipping the game from "no XR device" to
    /// "XR present, nothing tracked" and pinning it there. For a driver an agent
    /// writes blind, that silent success is the expensive failure, so a
    /// misspelling must be a 400 like every other malformed command.
    #[test]
    fn a_misspelled_xr_field_is_rejected_rather_than_silently_defaulted() {
        for body in [
            r#"{"type":"xr","lft":{"active":true}}"#,          // misspelled hand
            r#"{"type":"xr","right":{"triger":1.0}}"#,         // misspelled control
            r#"{"type":"xr","right":{"grip":{"pos":[0.0,0.0,0.0]}}}"#, // misspelled pose field
        ] {
            let err = serde_json::from_str::<InputCommand>(body)
                .expect_err(&format!("{body} must be rejected"));
            assert!(
                err.to_string().contains("unknown field"),
                "{body} rejected for the wrong reason: {err}"
            );
        }

        // The tag itself must NOT count as an unknown field: `Xr` is an
        // internally-tagged NEWTYPE variant, so `type` travels in the same map
        // as the sample's own fields and a naive `deny_unknown_fields` would
        // reject every valid body.
        serde_json::from_str::<InputCommand>(r#"{"type":"xr","right":{"trigger":1.0}}"#)
            .expect("a well-formed body still decodes");
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
            "GET /project",
            "POST /input",
            "POST /time",
            "POST /reload-source",
            "POST /reload-project",
            "POST /load-project",
            "POST /reload-asset",
            "POST /sync-assets",
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
        assert_eq!(DEBUG_PROTOCOL_VERSION, 7);
    }
}
