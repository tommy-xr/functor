//! Transport-neutral wire contract for Functor runtime debugging.
//!
//! Desktop and device runtimes expose this same protocol. Their HTTP servers
//! only parse/encode transport details; requests cross to the runtime loop as
//! [`DebugRequest`] values so rendering and game state remain on that loop's
//! thread.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

use crate::{ui::UiEventKind, GamepadSnapshot, InputSnapshot, TouchPhase, XrInputSnapshot};

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
/// 7 adds `surface_width` / `surface_height` to `GET /state`'s mouse sample.
/// They are logical window/CSS dimensions in the same coordinate space as
/// `x` / `y`; clients using resize-correct pointer mapping need v7.
///
/// 8 added `POST /time {"type":"cancel"}` so an external driver can abort a
/// queued batch without leaving clock work behind.
///
/// 9 lets a project push DECLARE its same-file entry role in the query string
/// of `POST /load-project` / `POST /reload-project` (`?module=Server` or
/// `?prefix=server`; see [`encode_entry_role_query`]). Tolerant in both
/// directions: a pre-v9 runtime ignores the query and boots the unprefixed
/// contract (so `functor run vr` refuses to push a role to one), and a v9
/// runtime treats a push with no role query as "the role already in force
/// stands".
///
/// 10 added `model_revision` and `pending_net` to `GET /state`, the two facts
/// a driver of a NETWORKED session cannot derive from `frame`: pausing freezes
/// the clock, not the transport, so a paused game's model still changes as
/// inbound messages fold through `update`. `model_revision` counts model
/// replacements and `pending_net` reports the shell's undelivered inbound
/// events. Additive — a pre-v10 runtime simply omits both, which deserializes
/// as `0`; a client that waits on either must gate on the version rather than
/// read a constant zero as "quiescent".
///
/// 11 added the EMBEDDER TRANSPORT's two endpoints — `GET /net/outbound` and
/// `POST /net/deliver` — through which a host process IS the network for a
/// runtime started with `--net-transport embedder` (no socket is ever opened).
/// Additive, and inert unless that argument was passed: under the default
/// socket transport both routes answer 409, because draining the game's
/// outbound commands there would steal them from the real dispatcher and
/// delivering into it would inject events no peer sent. A pre-v11 runtime
/// answers 404.
///
/// 12 adds gamepad injection — `POST /input` `{"type":"gamepad",…}` /
/// `{"type":"gamepad_clear"}` (the `xr`/`xr_clear` contract for the gamepad
/// domain) — and the optional `gamepad` field on `GET /state`'s input
/// snapshot.
///
/// 13 adds the `Instanced` scene node returned by `GET /scene` — a template
/// subtree plus compact per-copy channel records (position, quaternion
/// rotation, per-axis scale, tint). Clients that decode scene variants
/// exhaustively must gate before reading it.
///
/// 14 adds touch injection — `POST /input` `{"type":"touch","phase":…}`
/// transitions folded through the shared reducer (evented, like `key`, not
/// whole-sample like `xr`) — and the optional `touch` field on `GET /state`'s
/// input snapshot.
pub const DEBUG_PROTOCOL_VERSION: u32 = 14;

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
        description: "runtime state JSON: frame, tts, pending_steps (queued clock steps not yet run), model_revision (model replacements so far — the version label for a networked model, since pause freezes the clock and not the network), pending_net (inbound network events accepted but not yet delivered to the game), viewport, views, input snapshot (held_keys + mouse position/logical surface/buttons + optional xr/gamepad/touch), model (structured lossy JSON view of the model), model_debug (Rust Debug text)",
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
        description: "inject input — {\"type\":\"key\",\"key\":\"w\",\"down\":true} | {\"type\":\"mouse_move\",\"x\":0,\"y\":0} | {\"type\":\"mouse_wheel\",\"delta\":1} | {\"type\":\"mouse_button\",\"button\":\"left\",\"down\":true} (edge + held level, like key) | {\"type\":\"ui_event\",\"slot\":0,\"kind\":\"Clicked\"} | {\"type\":\"webview_event\",\"slot\":0,\"kind\":\"Clicked\"} | {\"type\":\"xr\",\"left\":{...},\"right\":{...},\"head\":{...}} (desktop only; level state until the next xr command) | {\"type\":\"xr_clear\"} (drop it, restoring the emulator or no device) | {\"type\":\"gamepad\",\"left_stick\":[0.0,1.0],\"south\":true,...} (desktop only; level state until the next gamepad command) | {\"type\":\"gamepad_clear\"} (drop it, restoring the physical pad or no device) | {\"type\":\"touch\",\"phase\":\"begin\",\"id\":0,\"x\":10,\"y\":20} (evented, like key: phases begin/move/end/cancel fold through the shared touch reducer)",
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
        description: "swap the whole project from a JSON array of [path, source] pairs (entry first), model preserved — 400 with the load error on a broken push; ?module=Server / ?prefix=server declares the same-file entry role",
    },
    DebugRoute {
        method: "POST",
        path: "/load-project",
        description: "load a new whole project from a JSON array of [path, source] pairs (entry first), model initialized from init — 400 with the load error on a broken push; ?module=Server / ?prefix=server declares the same-file entry role",
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
    DebugRoute {
        method: "GET",
        path: "/net/outbound",
        description: "embedder transport (--net-transport embedder): take-and-consume the game's queued ConnCommands as JSON — the host process is this runtime's network; 409 under the default socket transport",
    },
    DebugRoute {
        method: "POST",
        path: "/net/deliver",
        description: "embedder transport (--net-transport embedder): deliver inbound network events, a JSON array of {kind:\"connected\"|\"message\"|\"disconnected\"|\"error\", key, conn, text?/message?}; folded through update before the response; 409 under the default socket transport",
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
    /// How many times the game's model has been REPLACED by game logic since
    /// the program loaded — every `tick`/`input`/`update` return and every
    /// effect or network fold, counted at the producer's single model
    /// assignment.
    ///
    /// This is the version label for a networked model, and `frame` is not:
    /// pausing pins the CLOCK, not the transport, so a paused session keeps
    /// folding inbound messages through `update` while `frame` stands still.
    /// A driver that wants "did anything land since my snapshot" compares this.
    ///
    /// It counts replacements BY GAME LOGIC. Replacing the model from OUTSIDE
    /// the game deliberately does not count: a hot reload (which rebinds it),
    /// a whole-project load, a `/rewind` or a timeline seek. Those are things
    /// the driver itself just performed, and each hands back fresh state to
    /// re-baseline from — a counter that moved for them too could not tell
    /// "the network changed my model" from "I rewound it". `default` because a
    /// pre-v10 runtime omits it.
    #[serde(default)]
    pub model_revision: u64,
    /// Inbound network events the shell has accepted from its transport and
    /// not yet delivered into the game
    /// ([`crate::net::inbound_pending`]) — connection events and completed
    /// HTTP responses. Zero is the quiescence signal a harness waits on before
    /// snapshotting a baseline; it cannot see a packet still on the wire, so it
    /// is a lower bound on outstanding network work. `default` because a
    /// pre-v10 runtime omits it.
    #[serde(default)]
    pub pending_net: u64,
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
    /// Set the gamepad sample the next fixed step's `sampledInput` sees, so
    /// stick/trigger/button state is scriptable without a physical pad.
    ///
    /// The `Xr` contract exactly: level state until the next `gamepad` command
    /// replaces it, a WHOLE-sample replacement (an omitted field takes its
    /// default), no entry-point call — the sample reaches the game through the
    /// same `sampled_input` path a real pad would take, which is what makes it
    /// land in the recorded input log and replay identically. Unboxed, unlike
    /// `Xr`: the sample is a few words of `Copy` scalars, not an order of
    /// magnitude above the other commands.
    Gamepad(GamepadSnapshot),
    /// Drop an injected gamepad sample, restoring whatever the runtime would
    /// sample on its own — the GLFW-polled pad on desktop, or no `gamepad`
    /// domain when none is connected. The release half of `Gamepad`'s
    /// held-key contract.
    GamepadClear,
    /// One touch-contact transition — `{"type":"touch","phase":"begin",
    /// "id":0,"x":10,"y":20}` with phases `begin`/`move`/`end`/`cancel`.
    ///
    /// Unlike `Xr`/`Gamepad` (whole-sample level state), touch is EVENTED:
    /// each command folds through the same transition reducer real platform
    /// touch events take, updating the held contacts later steps sample and
    /// recording the same de-duplicated one-step edges — so a scripted tap
    /// or drag replays identically. Ending every begun id releases the
    /// domain's contacts; no separate clear command exists.
    Touch {
        phase: TouchPhase,
        id: u32,
        x: f32,
        y: f32,
    },
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

/// Maximum length of a role name in the project-push query string. Role names
/// are Functor Lang identifiers; this only exists so a hostile query cannot
/// make the runtime allocate.
const MAX_ENTRY_ROLE_NAME_BYTES: usize = 128;

/// Encode a project push's same-file entry ROLE as the query string of
/// `POST /load-project` / `POST /reload-project` — `"?module=Server"` or
/// `"?prefix=server"`, the two forms every other role carrier transports
/// ([`EntryRole::from_parts`](crate::functor_lang_producer::EntryRole::from_parts)).
///
/// The role is a *declaration*, so the plain contract encodes explicitly as
/// `"?prefix="`: a pusher that always declares can switch a live session back
/// to the unprefixed contract, while a pusher that declares nothing (an older
/// CLI, `functor push`, the MCP tools) sends no query and leaves the running
/// role alone. Role names are identifiers, so nothing here needs escaping —
/// [`parse_entry_role_query`] rejects anything else rather than mangling it.
pub fn encode_entry_role_query(role: &crate::functor_lang_producer::EntryRole) -> String {
    use crate::functor_lang_producer::EntryRole;
    match role {
        EntryRole::Module(name) => format!("?module={name}"),
        EntryRole::Prefix(prefix) => format!("?prefix={prefix}"),
    }
}

/// Decode [`encode_entry_role_query`]'s query string (the part AFTER `?`).
///
/// `Ok(None)` means the push declared no role at all — the runtime keeps the
/// role it is already running.
///
/// Everything else about the query is REFUSED rather than interpreted
/// loosely: an unknown key (so `?moduel=Client` is a 400 instead of a typo
/// that silently keeps the previous contract), a key declared twice, both
/// forms at once, and any name the other role carriers would reject — the
/// validation is literally theirs ([`EntryRole::is_valid_module`] /
/// [`EntryRole::is_valid_prefix`]), so this seam cannot boot a role
/// functor.json or the web page would refuse. Silently running the wrong
/// contract is the failure this query exists to prevent, and it is worse
/// than a 400.
pub fn parse_entry_role_query(
    query: &str,
) -> Result<Option<crate::functor_lang_producer::EntryRole>, String> {
    use crate::functor_lang_producer::EntryRole;
    let mut module: Option<&str> = None;
    let mut prefix: Option<&str> = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match key {
            "module" => &mut module,
            "prefix" => &mut prefix,
            _ => {
                return Err(format!(
                    "unknown entry-role key `{key}` — this push declares `module` or `prefix`"
                ))
            }
        };
        if slot.is_some() {
            return Err(format!("the push declares `{key}` more than once"));
        }
        *slot = Some(value);
    }
    for (key, value) in [("module", module), ("prefix", prefix)] {
        let Some(value) = value else { continue };
        if value.len() > MAX_ENTRY_ROLE_NAME_BYTES {
            return Err(format!("entry {key} is too long"));
        }
        let valid = match key {
            "module" => EntryRole::is_valid_module(value),
            _ => EntryRole::is_valid_prefix(value),
        };
        if !valid {
            return Err(match key {
                "module" => format!(
                    "entry module `{value}` must be a Capitalized inline module name \
(it is the block's own name: `module {value} {{ … }}`)"
                ),
                _ => format!(
                    "entry prefix `{value}` must be a valid identifier \
(it becomes the binding prefix: `{value}Init`, `{value}Tick`, …)"
                ),
            });
        }
    }
    match (module, prefix) {
        (None, None) => Ok(None),
        (Some(module), Some(prefix)) if !module.is_empty() && !prefix.is_empty() => Err(format!(
            "the push declares both an entry module `{module}` and an entry prefix `{prefix}`"
        )),
        (module, prefix) => Ok(Some(EntryRole::from_parts(
            module.unwrap_or(""),
            prefix.unwrap_or(""),
        ))),
    }
}

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
    /// The pushed file set plus the same-file entry role the push DECLARED
    /// (`None` = it declared none, so the running role stands).
    ReloadProject(
        ProjectSources,
        Option<crate::functor_lang_producer::EntryRole>,
        Sender<Result<String, String>>,
    ),
    LoadProject(
        ProjectSources,
        Option<crate::functor_lang_producer::EntryRole>,
        Sender<Result<String, String>>,
    ),
    ReloadAsset(ProjectAsset, Sender<Result<String, String>>),
    SyncAssets(ProjectAssetPaths, Sender<Result<String, String>>),
    Rewind(u64, Sender<Result<String, String>>),
    /// Embedder transport: take-and-consume the game's queued `ConnCommand`s.
    /// `Err` when the runtime is on the default socket transport, where the
    /// real dispatcher owns that queue.
    NetOutbound(Sender<Result<String, String>>),
    /// Embedder transport: push inbound events into the game. `Err` for the
    /// same reason as [`DebugRequest::NetOutbound`].
    NetDeliver(
        Vec<crate::net::DeliveredEvent>,
        Sender<Result<String, String>>,
    ),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::{Key, TrackingPose};
    use serde_json::{json, Value};

    use super::*;

    /// The role a project push declares round-trips through its query string,
    /// and — the point of encoding it at all — the two forms stay DISTINCT
    /// while an absent declaration stays distinguishable from the plain
    /// contract's explicit one.
    #[test]
    fn a_pushed_entry_role_round_trips_through_the_query_string() {
        use crate::functor_lang_producer::EntryRole;
        for role in [
            EntryRole::Module("Server".to_string()),
            EntryRole::Prefix("server".to_string()),
            EntryRole::Prefix(String::new()),
        ] {
            let query = encode_entry_role_query(&role);
            assert_eq!(
                parse_entry_role_query(query.trim_start_matches('?')),
                Ok(Some(role.clone())),
                "{query}"
            );
        }
        // Declaring nothing is NOT declaring the plain contract: a runtime
        // that hears nothing keeps the role it is already running.
        assert_eq!(parse_entry_role_query(""), Ok(None));
        assert_eq!(
            parse_entry_role_query("prefix="),
            Ok(Some(EntryRole::Prefix(String::new())))
        );
    }

    /// A role that cannot be honored exactly must be REFUSED, never coerced:
    /// silently booting the wrong contract is the failure this whole query
    /// exists to prevent.
    #[test]
    fn a_malformed_pushed_role_is_refused_rather_than_coerced() {
        assert!(parse_entry_role_query("module=Server&prefix=server").is_err());
        assert!(parse_entry_role_query("module=My%20Server").is_err());
        assert!(parse_entry_role_query("prefix=a.b").is_err());
        assert!(parse_entry_role_query("module=A&module=B").is_err());
        assert!(parse_entry_role_query(&format!("module={}", "A".repeat(200))).is_err());
        // The SAME rules the other role carriers apply: a module names a
        // Capitalized block, a prefix is an identifier. A name one seam
        // accepts and another refuses is how a role boots the wrong contract.
        assert!(parse_entry_role_query("module=server").is_err());
        assert!(parse_entry_role_query("prefix=9server").is_err());
        // A typo'd key is refused too — treating it as "declared nothing"
        // would silently keep the previous contract with a 200.
        assert!(parse_entry_role_query("moduel=Client").is_err());
        // The SAME rules the other role carriers apply: a module names a
        // Capitalized block, a prefix is an identifier. A name one seam
        // accepts and another refuses is how a role boots the wrong contract.
        assert!(parse_entry_role_query("module=server").is_err());
        assert!(parse_entry_role_query("prefix=9server").is_err());
        // A typo'd key is refused too — treating it as "declared nothing"
        // would silently keep the previous contract with a 200.
        assert!(parse_entry_role_query("moduel=Client").is_err());
        // Both declared but only one non-empty is unambiguous, not a conflict.
        assert_eq!(
            parse_entry_role_query("module=Server&prefix="),
            Ok(Some(crate::functor_lang_producer::EntryRole::Module(
                "Server".to_string()
            )))
        );
    }

    #[test]
    fn runtime_state_json_preserves_desktop_shape_and_reports_views() {
        let state = RuntimeState {
            frame: 42,
            tts: 1.5,
            pending_steps: 3,
            model_revision: 17,
            pending_net: 2,
            viewport: RuntimeViewport::new(1920, 1080),
            views: vec![RuntimeView::new("main", 1920, 1080)],
            model: json!({ "label": "hello" }),
            model_debug: "Model {\n  label: \"hello\"\n}".into(),
            input: InputSnapshot {
                held_keys: vec![Key::W, Key::Up],
                mouse: crate::MouseSnapshot {
                    x: 10,
                    y: 20,
                    surface_width: 960,
                    surface_height: 540,
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
                "model_revision": 17,
                "pending_net": 2,
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
                        "surface_width": 960,
                        "surface_height": 540,
                        "buttons": { "left": false, "right": false, "middle": false },
                        "pressed": { "left": false, "right": false, "middle": false },
                        "released": { "left": false, "right": false, "middle": false }
                    }
                }
            })
        );
        assert_eq!(actual["input"]["mouse"]["surface_width"], 960);
        assert_eq!(actual["input"]["mouse"]["surface_height"], 540);
        assert_ne!(
            actual["input"]["mouse"]["surface_width"],
            actual["viewport"]["width"],
            "the logical pointer surface must remain distinct from the framebuffer viewport"
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

    /// The gamepad command follows the `xr` contract: a partial body defaults
    /// what it omits, a misspelled field is a 400 (same silent-success trap —
    /// an all-default sample would flip a game from "no pad" to "pad present,
    /// nothing held" and pin it there), and the minimal body is a fully
    /// default sample.
    #[test]
    fn a_gamepad_command_decodes_partially_and_rejects_typos() {
        let command = serde_json::from_str::<InputCommand>(
            r#"{"type":"gamepad",
                "left_stick": [-0.5, 1.0],
                "right_trigger": 0.25,
                "south": true,
                "dpad_left": true}"#,
        )
        .expect("gamepad command decodes");
        let InputCommand::Gamepad(sample) = command else {
            panic!("expected a gamepad command");
        };
        assert_eq!(sample.left_stick, [-0.5, 1.0]);
        assert_eq!(sample.right_trigger, 0.25);
        assert!(sample.south);
        assert!(sample.dpad_left);
        // Omitted controls take their defaults.
        assert_eq!(sample.right_stick, [0.0, 0.0]);
        assert_eq!(sample.left_trigger, 0.0);
        assert!(!sample.east);
        assert!(!sample.select);

        let bare = serde_json::from_str::<InputCommand>(r#"{"type":"gamepad"}"#).unwrap();
        assert_eq!(bare, InputCommand::Gamepad(GamepadSnapshot::default()));

        assert_eq!(
            serde_json::from_str::<InputCommand>(r#"{"type":"gamepad_clear"}"#).unwrap(),
            InputCommand::GamepadClear
        );

        for body in [
            r#"{"type":"gamepad","lstick":[0.0,0.0]}"#, // misspelled stick
            r#"{"type":"gamepad","suoth":true}"#,       // misspelled button
        ] {
            let err = serde_json::from_str::<InputCommand>(body)
                .expect_err(&format!("{body} must be rejected"));
            assert!(
                err.to_string().contains("unknown field"),
                "{body} rejected for the wrong reason: {err}"
            );
        }
    }

    #[test]
    fn a_touch_command_decodes_each_phase_and_rejects_unknown_ones() {
        for (phase, expected) in [
            ("begin", TouchPhase::Begin),
            ("move", TouchPhase::Move),
            ("end", TouchPhase::End),
            ("cancel", TouchPhase::Cancel),
        ] {
            let body = format!(r#"{{"type":"touch","phase":"{phase}","id":2,"x":10.5,"y":20}}"#);
            assert_eq!(
                serde_json::from_str::<InputCommand>(&body).unwrap(),
                InputCommand::Touch {
                    phase: expected,
                    id: 2,
                    x: 10.5,
                    y: 20.0
                }
            );
        }
        // An unknown phase is a 400, like every malformed command.
        assert!(
            serde_json::from_str::<InputCommand>(
                r#"{"type":"touch","phase":"start","id":0,"x":0,"y":0}"#
            )
            .is_err()
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
            "GET /project",
            "POST /input",
            "POST /time",
            "POST /reload-source",
            "POST /reload-project",
            "POST /load-project",
            "POST /reload-asset",
            "POST /sync-assets",
            "POST /rewind",
            "GET /net/outbound",
            "POST /net/deliver",
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
        assert_eq!(DEBUG_PROTOCOL_VERSION, 14);
    }

    /// The v10 fields are ADDITIVE: a pre-v10 payload (which carries neither)
    /// still deserializes, reading `0` for both. That is exactly why a client
    /// waiting on either must gate on `protocol_version` first — a constant
    /// zero from an old runtime is indistinguishable from real quiescence.
    #[test]
    fn a_pre_v10_state_payload_still_decodes_with_zeroed_v10_fields() {
        let state: RuntimeState = serde_json::from_value(json!({
            "frame": 7,
            "tts": 0.5,
            "pending_steps": 0,
            "viewport": { "width": 8, "height": 8 },
            "views": [],
            "model": null,
            "model_debug": "",
            "input": serde_json::to_value(InputSnapshot::default()).unwrap(),
        }))
        .expect("a pre-v10 /state payload decodes");
        assert_eq!(state.model_revision, 0);
        assert_eq!(state.pending_net, 0);
    }
}
