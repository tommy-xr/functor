//! `functor mcp` — an MCP (Model Context Protocol) server over the debug
//! runtime (`docs/debug-runtime.md`).
//!
//! The debug runtime already lets an external driver observe and control a
//! running game over localhost HTTP. This wraps that surface in the standard
//! interface coding agents already speak, so "run this game, pause it, press a
//! key, look at the model" needs no bespoke script.
//!
//! Everything here is CLI-only: the server is an HTTP client of the runtimes,
//! never a part of them. It manages N concurrent sessions — each a base URL,
//! plus a child process when the session was launched (rather than attached)
//! by this server.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use functor_docgen::{ApiItem, ApiReference};
use functor_runtime_common::net::ConnCommand;
use functor_runtime_common::{debug_protocol::DEBUG_PROTOCOL_SERVICE, Key};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, Command};

use super::mcp_net::NetCoordinator;

/// How often the background routing pump drains and delivers a live group's
/// traffic. Between `step_all` rounds a group still runs on wall-clock time,
/// and its packets must move without a tool call to push them — this is the
/// coordinator standing in for the kernel. It is a HOST-side poll of two tiny
/// localhost endpoints, not per-frame runtime work.
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// Largest delivery body the pump will post, comfortably under the debug
/// server's own 64 KB command limit. A backlog larger than this is split across
/// rounds rather than posted whole (which would 413 forever).
const MAX_DELIVERY_BYTES: usize = 32 * 1024;

/// Rows `wire_log` returns when the caller names no limit, and the ceiling it
/// will return at all.
const WIRE_LOG_DEFAULT_ROWS: usize = 100;
const WIRE_LOG_MAX_ROWS: usize = 2_000;

/// Total `payload_text` bytes one `wire_log` answer will carry. The row cap
/// bounds the count; this bounds the SIZE, which the rows themselves do not.
const MAX_WIRE_PAYLOAD_BYTES: usize = 256 * 1024;

/// How long `launch_game` waits for a spawned runtime to answer discovery.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `step` tolerates a queued batch making NO progress before giving
/// up. A large batch legitimately takes far longer than this to drain in total.
const STEP_STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Wall-clock deadline for one acquired step or submitted-code operation.
/// Progress does not extend this deadline, so a cancelled request cannot
/// monopolize the per-runtime gate indefinitely.
const OPERATION_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum time spent confirming termination of the direct Node child after
/// code completes, throws, times out, or is cancelled.
const NODE_CHILD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// Maximum time spent confirming termination of a runtime process owned by
/// this MCP server. Failure preserves the exact-URL quarantine tombstone.
const RUNTIME_CHILD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Aggregate safety-cleanup budget after submitted code fails. This includes
/// queued-step cancellation, one current-input snapshot, and every required
/// input-level transition. Expiry quarantines the exact runtime URL.
const CODE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-request timeout for the (loopback or adb-forwarded) debug server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Lowest debug-protocol version whose contract these tools actually keep
/// (`pending_steps` on `/state`, `frames` on `/time advance`, structured
/// `model`, and `/time cancel`).
const REQUIRED_PROTOCOL_VERSION: u64 = 8;
/// Bytes of a launched child's stdout/stderr kept for failure reporting.
const LOG_TAIL_BYTES: usize = 8 * 1024;
/// Maximum body retained from any ordinary debug-runtime text response.
const MAX_RUNTIME_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum raw bytes retained for one `POST /capture` response.
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum submitted JavaScript function source.
const MAX_NODE_CODE_BYTES: usize = 64 * 1024;
/// Maximum one-line message accepted from or sent to the Node child. This is
/// above an 8 MiB capture after base64 encoding.
const MAX_NODE_PROTOCOL_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum SDK calls accepted from one submitted program. This is deliberately
/// above `stepUntil`'s worst-case 20,001 base calls (initial state plus one
/// step/state pair for each of 10,000 frames).
const MAX_NODE_CALLS: usize = 25_000;
/// Maximum aggregate console text retained from one submitted program.
const MAX_NODE_LOG_BYTES: usize = 64 * 1024;
/// Maximum serialized JSON text returned by one submitted-code run.
const MAX_CODE_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum decoded bytes in one asset sent through the JSON child protocol.
/// The standalone SDK can use the runtime's larger binary HTTP limit.
const MAX_CODE_ASSET_BYTES: usize = 8 * 1024 * 1024;
/// Maximum raw PNG bytes retained across all captures in one code run.
const MAX_CODE_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum base64 text placed in MCP image content across one code run.
const MAX_CODE_ENCODED_CAPTURE_BYTES: usize = 24 * 1024 * 1024;
/// The JavaScript host is embedded in the Rust binary and evaluated by the
/// user's Node executable. It is an RPC adapter, not a security sandbox.
const NODE_RUNNER_SOURCE: &str = include_str!("mcp_node_runner.mjs");
/// Maximum frame batch accepted by the lower-level `step` tool.
const MAX_STEP_FRAMES: u32 = 10_000;
/// How many API-reference items one `api_reference` call returns.
const MAX_API_RESULTS: usize = 20;
/// The Functor Lang language guide, embedded verbatim from the `functor-lang`
/// skill — the repository's declared source of truth for the language
/// (`CLAUDE.md` requires it to be updated whenever the language changes). It is
/// embedded rather than paraphrased so this tool cannot drift from it: there is
/// exactly one copy, and `language_guide` serves sections of it mechanically.
const LANGUAGE_GUIDE: &str = include_str!("../../../.claude/skills/functor-lang/SKILL.md");
/// The section whose bullets front the table of contents — the "do NOT guess
/// from F#/OCaml" list, which is what an agent needs before it writes a line.
/// Matched as a slug PREFIX, so rewording the heading's tail cannot lose it.
const QUICK_FACTS_SLUG: &str = "quick-facts";

/// Serve MCP over stdio until the client disconnects.
pub async fn execute() -> io::Result<()> {
    let server = FunctorMcp::new();
    let sessions = server.sessions.clone();
    let service = server
        .serve(stdio())
        .await
        .map_err(|error| io::Error::other(format!("failed to start the MCP server: {error}")))?;
    // Owned children are killed on drop (`kill_on_drop`), but a client that
    // signals the server instead of closing stdin would never reach that drop —
    // and every orphan is a whole desktop runtime holding a GL context. So
    // shutdown is reached from either side.
    let running = service.waiting();
    tokio::pin!(running);
    let quit = tokio::select! {
        result = &mut running => result.map(|_| ()),
        _ = shutdown_signal() => Ok(()),
    };
    Registry::shutdown(&sessions);
    quit.map_err(|error| io::Error::other(format!("the MCP server task failed: {error}")))
}

/// Resolve when the host asks this process to stop (SIGTERM or Ctrl-C).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = terminate.recv() => return,
                _ = tokio::signal::ctrl_c() => return,
            }
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// One game the server can talk to.
struct Session {
    url: String,
    /// Serializes state-changing operations for this session. It is cloned out
    /// of the registry before awaiting, so the synchronous registry mutex is
    /// never held across runtime I/O.
    operation_gate: Arc<tokio::sync::Mutex<()>>,
    /// Exact-URL safety marker shared by aliases. A failed abort cleanup makes
    /// the runtime state ambiguous, so no alias may mutate it again. Killing an
    /// owned runtime clears it; attached runtimes require a runtime + MCP restart.
    quarantined: Arc<AtomicBool>,
    /// Per-session lifecycle marker. Exact-URL aliases share the operation
    /// gate but remain independently stoppable registry entries.
    closing: Arc<AtomicBool>,
    /// Stable even while stop temporarily takes `child` out to await it.
    owned: bool,
    /// The port this server reserved for a launched runtime, held so a
    /// concurrent launch cannot be handed the same one.
    port: Option<u16>,
    /// `Some` only when this server spawned the runtime. An attached session
    /// (`connect_game`) is never killed — the runtime belongs to someone else.
    child: Option<Child>,
    /// The temporary project directory written for an inline `launch_game`,
    /// held so it lives exactly as long as the session that runs from it.
    /// Never read — its `Drop` is the whole point.
    #[allow(dead_code)]
    scratch: Option<ScratchDir>,
    /// The `functor.json` this session BOOTED with, verbatim — the launch
    /// directory's, or the inline file set's. `GET /project` reports modules,
    /// not project metadata, so this is the only place a launched session's
    /// manifest survives, and `save_project` writes it rather than
    /// reconstructing one (a multi-entry project reconstructed as
    /// `{"entry": "game.fun"}` builds green and fails at run). `None` for an
    /// ATTACHED session: the manifest of a runtime someone else started is not
    /// knowable over the wire.
    manifest: Option<String>,
}

/// One role of a session GROUP: a runtime launched on the embedder transport,
/// whose network is this process.
#[derive(Clone, Debug)]
struct GroupMember {
    /// Registry session id (`s3`).
    session: String,
    /// The routing identity in the wire log — the role name, disambiguated
    /// when a role is launched more than once (`server`, `client1`, `client2`).
    label: String,
    /// The functor.json entry this member runs.
    role: String,
    url: String,
}

/// A set of runtimes wired to each other by this process.
///
/// One `tokio::sync::Mutex` per group covers BOTH the coordinator and the
/// pump's HTTP round-trips, so a background pump and a `step_all` round can
/// never interleave halfway through routing a batch.
struct Group {
    members: Vec<GroupMember>,
    net: NetCoordinator,
    /// Members whose last pump round failed, so the report is printed once per
    /// outage instead of 125 times a second.
    unreachable: BTreeSet<String>,
}

impl Group {
    /// Report a member the pump could not reach — once, until it recovers.
    fn note_unreachable(&mut self, label: &str, what: String) {
        if self.unreachable.insert(label.to_string()) {
            eprintln!("[coordinator] {label} unreachable — {what}");
        }
    }

    fn note_reachable(&mut self, label: &str) {
        if self.unreachable.remove(label) {
            eprintln!("[coordinator] {label} is reachable again");
        }
    }
}

/// Whether a `/net/deliver` batch is safe to keep and retry.
enum DeliveryOutcome {
    Applied,
    /// The runtime provably did not fold it — requeue.
    NotApplied(String),
    /// It may or may not have folded — report, and do NOT retry.
    Ambiguous(String),
}

/// What `spawn_runtime` hands back to the two launch tools.
struct SpawnedRuntime {
    session: String,
    url: String,
    port: u16,
    discovery: Value,
}

/// The await-safe part of a registry session.
#[derive(Clone)]
struct SessionTarget {
    url: String,
    operation_gate: Arc<tokio::sync::Mutex<()>>,
    quarantined: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}

struct PendingConnect {
    operation_gate: Arc<tokio::sync::Mutex<()>>,
    quarantined: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
    reservations: usize,
}

struct ConnectReservation {
    registry: Weak<Mutex<Registry>>,
    url: String,
    operation_gate: Arc<tokio::sync::Mutex<()>>,
    quarantined: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
    active: bool,
}

#[derive(Default)]
struct CodeOutputBudget {
    retained_text_bytes: usize,
    capture_bytes: usize,
}

#[derive(Default)]
struct CodeInputRestore {
    baseline_keys: BTreeSet<String>,
    baseline_mouse_buttons: BTreeSet<String>,
    touched_keys: BTreeSet<String>,
    touched_mouse_buttons: BTreeSet<String>,
}

impl CodeInputRestore {
    fn from_state(state: &Value) -> Self {
        let baseline_keys = state
            .pointer("/input/held_keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| {
                serde_json::from_value::<Key>(value.clone())
                    .ok()
                    .or_else(|| value.as_str().and_then(Key::from_name))
            })
            .filter(|key| *key != Key::Unknown)
            .map(Key::name)
            .collect();
        let baseline_mouse_buttons = ["left", "right", "middle"]
            .into_iter()
            .filter(|button| {
                state
                    .pointer(&format!("/input/mouse/buttons/{button}"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .map(str::to_string)
            .collect();
        Self {
            baseline_keys,
            baseline_mouse_buttons,
            ..Self::default()
        }
    }

    fn touch_key(&mut self, key: &str) {
        if let Some(key) = Key::from_name(key) {
            self.touched_keys.insert(key.name());
        }
    }

    fn touch_mouse_button(&mut self, button: &str) {
        self.touched_mouse_buttons
            .insert(button.to_ascii_lowercase());
    }

    fn is_empty(&self) -> bool {
        self.touched_keys.is_empty() && self.touched_mouse_buttons.is_empty()
    }
}

impl CodeOutputBudget {
    fn retain_json(&mut self, value: &Value) -> Result<(), String> {
        let bytes = json_encoded_len(value)?;
        self.retained_text_bytes = checked_output_total(
            self.retained_text_bytes,
            bytes,
            MAX_CODE_TEXT_BYTES,
            "submitted-code aggregate text output",
        )?;
        Ok(())
    }

    fn retain_capture(&mut self, bytes: usize) -> Result<(), String> {
        self.capture_bytes = checked_output_total(
            self.capture_bytes,
            bytes,
            MAX_CODE_CAPTURE_BYTES,
            "submitted-code aggregate raw capture output",
        )?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NodeMessage {
    Ready {
        node_version: String,
    },
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        args: Vec<Value>,
    },
    Log {
        level: String,
        text: String,
    },
    Complete {
        ok: bool,
        #[serde(default)]
        value: Value,
        error: Option<NodeCodeError>,
    },
    Fatal {
        error: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct NodeCodeError {
    name: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack: Option<String>,
}

struct NodeCodeOutput {
    node_version: String,
    return_value: Value,
    code_error: Option<NodeCodeError>,
    failure: Option<String>,
    trace: Vec<Value>,
    logs: Vec<Value>,
    captures: Vec<Vec<u8>>,
    calls_executed: usize,
    final_state: Option<Value>,
    final_state_summary: Option<Value>,
    output_budget: CodeOutputBudget,
}

struct SdkCallOutput {
    value: Value,
    trace_result: Value,
}

enum AdvanceRequestError {
    /// A 4xx response proves the runtime rejected the command before queuing
    /// clock work, so cancelling here could delete somebody else's queue.
    Rejected(String),
    /// Transport, response-body, and 5xx failures cannot prove whether the
    /// runtime accepted the command. They require queued-step cleanup.
    Ambiguous(String),
}

/// A project directory this server created and owns. Dropping it (when the
/// session is stopped, or when the whole server shuts down) removes the
/// directory — an inline-launched game's source has no durable home unless
/// `save_project` gives it one.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Create a fresh directory this server owns, under the system temp dir.
fn scratch_dir() -> Result<ScratchDir, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut last = String::new();
    // `create_dir` rather than `create_dir_all`: an existing directory of this
    // name is someone else's (a killed earlier server with the same pid), not
    // ours to fill and later delete — so take the next name instead.
    for _ in 0..16 {
        let suffix = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("functor-mcp-{}-{suffix}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => {
                // The game's source is the client's, and on a shared /tmp the
                // default would be world-readable.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
                }
                return Ok(ScratchDir(path));
            }
            Err(error) => last = error.to_string(),
        }
    }
    Err(format!("could not create a scratch project dir: {last}"))
}

#[derive(Default)]
struct Registry {
    next_id: u32,
    sessions: BTreeMap<String, Session>,
    /// Exact-URL safety tombstones. A quarantined marker deliberately outlives
    /// attached session ids, so detach/reconnect cannot bless ambiguous state
    /// in the same MCP process.
    url_quarantines: BTreeMap<String, Arc<AtomicBool>>,
    /// Transient exact-URL lifecycles created before connect discovery. Each
    /// entry is removed when its RAII reservations complete or are cancelled.
    pending_connects: BTreeMap<String, PendingConnect>,
    /// Ports handed to a launch. The OS never offers a port a live runtime
    /// already holds, but it happily offers the same one twice inside the
    /// window between `free_port` and the child's own bind — and MCP tool calls
    /// can overlap, which is exactly the two-game case sessions exist for.
    reserved: BTreeSet<u16>,
    /// Coordinated groups by id (`g1`), and the reverse index a `step_all`
    /// uses to find the group a session belongs to.
    groups: BTreeMap<String, Arc<tokio::sync::Mutex<Group>>>,
    session_groups: BTreeMap<String, String>,
    next_group: u32,
}

impl Registry {
    fn insert(
        &mut self,
        url: String,
        port: Option<u16>,
        child: Option<Child>,
        scratch: Option<ScratchDir>,
        manifest: Option<String>,
    ) -> String {
        // A launched child is a process this server just created, so it starts
        // a fresh exact-URL lifecycle even if an old attached runtime used the
        // same port. Attached insertions must retain any tombstone.
        let existing = child
            .is_none()
            .then(|| {
                self.pending_connects
                    .get(&url)
                    .map(|pending| (pending.operation_gate.clone(), pending.quarantined.clone()))
                    .or_else(|| {
                        self.sessions
                            .values()
                            .find(|session| session.url == url)
                            .map(|session| {
                                (session.operation_gate.clone(), session.quarantined.clone())
                            })
                    })
            })
            .flatten();
        let (operation_gate, quarantined) = existing.unwrap_or_else(|| {
            (
                Arc::new(tokio::sync::Mutex::new(())),
                if child.is_some() {
                    Arc::new(AtomicBool::new(false))
                } else {
                    self.url_quarantines
                        .get(&url)
                        .cloned()
                        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
                },
            )
        });
        self.insert_with_gate(url, port, child, scratch, manifest, operation_gate, quarantined)
    }

    fn insert_with_gate(
        &mut self,
        url: String,
        port: Option<u16>,
        child: Option<Child>,
        scratch: Option<ScratchDir>,
        manifest: Option<String>,
        operation_gate: Arc<tokio::sync::Mutex<()>>,
        quarantined: Arc<AtomicBool>,
    ) -> String {
        self.next_id += 1;
        let id = format!("s{}", self.next_id);
        let owned = child.is_some();
        self.url_quarantines
            .insert(url.clone(), quarantined.clone());
        self.sessions.insert(
            id.clone(),
            Session {
                url,
                operation_gate,
                quarantined,
                closing: Arc::new(AtomicBool::new(false)),
                owned,
                port,
                child,
                scratch,
                manifest,
            },
        );
        id
    }

    fn reserve_connect(
        &mut self,
        url: &str,
    ) -> (
        Arc<tokio::sync::Mutex<()>>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        if let Some(pending) = self.pending_connects.get_mut(url) {
            pending.reservations += 1;
            return (
                pending.operation_gate.clone(),
                pending.quarantined.clone(),
                pending.closing.clone(),
            );
        }
        let existing = self
            .sessions
            .values()
            .find(|session| session.url == url)
            .map(|session| (session.operation_gate.clone(), session.quarantined.clone()));
        let (operation_gate, quarantined) = existing.unwrap_or_else(|| {
            (
                Arc::new(tokio::sync::Mutex::new(())),
                self.url_quarantines
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
            )
        });
        self.url_quarantines
            .insert(url.to_string(), quarantined.clone());
        let owned_stop_in_progress = self.sessions.values().any(|session| {
            session.url == url && session.owned && session.closing.load(Ordering::Acquire)
        });
        let closing = Arc::new(AtomicBool::new(owned_stop_in_progress));
        self.pending_connects.insert(
            url.to_string(),
            PendingConnect {
                operation_gate: operation_gate.clone(),
                quarantined: quarantined.clone(),
                closing: closing.clone(),
                reservations: 1,
            },
        );
        (operation_gate, quarantined, closing)
    }

    fn release_connect(&mut self, url: &str, lifecycle: &Arc<AtomicBool>) {
        let remove = match self.pending_connects.get_mut(url) {
            Some(pending) if Arc::ptr_eq(&pending.closing, lifecycle) => {
                pending.reservations = pending.reservations.saturating_sub(1);
                pending.reservations == 0
            }
            _ => false,
        };
        if remove {
            self.pending_connects.remove(url);
        }
    }

    /// Claim a port no other session has been handed.
    fn reserve_port(&mut self) -> Result<u16, String> {
        for _ in 0..64 {
            let port = free_port()?;
            if self.reserved.insert(port) {
                return Ok(port);
            }
        }
        Err("could not find a free port that is not already reserved by another session".into())
    }

    fn release_port(&mut self, port: u16) {
        self.reserved.remove(&port);
    }

    /// The session's base URL, or an error naming the sessions that do exist —
    /// a stale id is the most common agent mistake, so it must be self-correcting.
    fn url(&self, id: &str) -> Result<String, String> {
        self.target(id).map(|target| target.url)
    }

    /// Resolve a diagnostic read without treating an exact-URL quarantine as
    /// a mutation. Reads are how a caller inspects the ambiguous state before
    /// deciding which owned session to stop or attached runtime to restart.
    fn read_url(&self, id: &str) -> Result<String, String> {
        match self.sessions.get(id) {
            Some(session) if session.closing.load(Ordering::Acquire) => {
                Err(format!("session {id:?} is stopping; no new operation can start"))
            }
            Some(session) => Ok(session.url.clone()),
            None if self.sessions.is_empty() => Err(format!(
                "unknown session {id:?}: no sessions yet — start one with launch_game or connect_game"
            )),
            None => Err(format!(
                "unknown session {id:?}: known sessions are {}",
                self.ids().join(", ")
            )),
        }
    }

    /// Clone the runtime address and async operation gate while holding the
    /// registry briefly. Callers may then drop the registry guard and await.
    fn target(&self, id: &str) -> Result<SessionTarget, String> {
        match self.sessions.get(id) {
            Some(session) if session.quarantined.load(Ordering::Acquire) => {
                Err(format!(
                    "session {id:?} is quarantined because abort cleanup could not be confirmed; \
stop an owned session, or restart both an attached runtime and this MCP server, before mutating {}",
                    session.url
                ))
            }
            Some(session) if session.closing.load(Ordering::Acquire) => {
                Err(format!("session {id:?} is stopping; no new operation can start"))
            }
            Some(session) => Ok(SessionTarget {
                url: session.url.clone(),
                operation_gate: session.operation_gate.clone(),
                quarantined: session.quarantined.clone(),
                closing: session.closing.clone(),
            }),
            None if self.sessions.is_empty() => Err(format!(
                "unknown session {id:?}: no sessions yet — start one with launch_game or connect_game"
            )),
            None => Err(format!(
                "unknown session {id:?}: known sessions are {}",
                self.ids().join(", ")
            )),
        }
    }

    fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// The `functor.json` this session booted with, if this server launched it.
    fn manifest(&self, id: &str) -> Option<String> {
        self.sessions.get(id).and_then(|s| s.manifest.clone())
    }

    fn remove(&mut self, id: &str) -> Result<Session, String> {
        match self.sessions.remove(id) {
            Some(session) => {
                if let Some(port) = session.port {
                    self.release_port(port);
                }
                Ok(session)
            }
            None => Err(self.url(id).expect_err("id is absent")),
        }
    }

    /// Mark the appropriate lifecycle closing while holding the registry
    /// mutex. Owned stop closes the whole exact-URL group and pending connects;
    /// attached stop closes only the requested id.
    fn begin_stop(&mut self, id: &str) -> Result<(SessionTarget, bool), String> {
        let Some(session) = self.sessions.get(id) else {
            return Err(self.url(id).expect_err("id is absent"));
        };
        if session.closing.load(Ordering::Acquire) {
            return Err(format!("session {id:?} is already stopping"));
        }
        let url = session.url.clone();
        let operation_gate = session.operation_gate.clone();
        let closing = session.closing.clone();
        let owned = session.owned;
        if owned {
            for alias in self.sessions.values().filter(|alias| alias.url == url) {
                alias.closing.store(true, Ordering::Release);
            }
            if let Some(pending) = self.pending_connects.get(&url) {
                pending.closing.store(true, Ordering::Release);
            }
        } else {
            closing.store(true, Ordering::Release);
        }
        Ok((
            SessionTarget {
                url,
                operation_gate,
                quarantined: session.quarantined.clone(),
                closing,
            },
            owned,
        ))
    }

    fn take_child(&mut self, id: &str) -> Result<Option<Child>, String> {
        self.sessions
            .get_mut(id)
            .map(|session| session.child.take())
            .ok_or_else(|| self.url(id).expect_err("id is absent"))
    }

    fn remove_url(&mut self, url: &str) -> Vec<Session> {
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.url == url)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| self.remove(&id).ok())
            .collect()
    }

    /// Clear a tombstone only after this server has killed the runtime it
    /// owned. Detaching an attached id is intentionally insufficient.
    fn clear_owned_url_tombstone(&mut self, url: &str) {
        self.url_quarantines.remove(url);
    }

    /// Kill every owned child. Attached sessions are only forgotten.
    fn shutdown(registry: &Arc<Mutex<Registry>>) {
        let mut guard = registry.lock().expect("mcp registry poisoned");
        for (_, mut session) in std::mem::take(&mut guard.sessions) {
            if let Some(child) = session.child.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

impl ConnectReservation {
    fn target(&self) -> SessionTarget {
        SessionTarget {
            url: self.url.clone(),
            operation_gate: self.operation_gate.clone(),
            quarantined: self.quarantined.clone(),
            closing: self.closing.clone(),
        }
    }

    fn finish(mut self) -> Result<String, String> {
        let registry = self
            .registry
            .upgrade()
            .ok_or_else(|| "the MCP session registry shut down during connect".to_string())?;
        let mut guard = registry.lock().expect("mcp registry poisoned");
        let same_lifecycle = guard
            .pending_connects
            .get(&self.url)
            .is_some_and(|pending| {
                Arc::ptr_eq(&pending.operation_gate, &self.operation_gate)
                    && Arc::ptr_eq(&pending.quarantined, &self.quarantined)
                    && Arc::ptr_eq(&pending.closing, &self.closing)
            });
        let result = if self.quarantined.load(Ordering::Acquire) {
            Err(format!(
                "runtime at {} is quarantined because abort cleanup could not be confirmed; \
stop it if owned, or restart both the attached runtime and this MCP server",
                self.url
            ))
        } else if self.closing.load(Ordering::Acquire) {
            Err(format!(
                "runtime at {} is stopping; connect did not create a session",
                self.url
            ))
        } else if !same_lifecycle {
            Err(format!(
                "the connection lifecycle for {} changed before insertion",
                self.url
            ))
        } else {
            Ok(guard.insert_with_gate(
                self.url.clone(),
                None,
                None,
                None,
                // Attached: the runtime's manifest is not on this wire.
                None,
                self.operation_gate.clone(),
                self.quarantined.clone(),
            ))
        };
        guard.release_connect(&self.url, &self.closing);
        self.active = false;
        result
    }
}

impl Drop for ConnectReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(registry) = self.registry.upgrade() {
            registry
                .lock()
                .expect("mcp registry poisoned")
                .release_connect(&self.url, &self.closing);
        }
    }
}

#[derive(Clone)]
pub struct FunctorMcp {
    http: reqwest::Client,
    sessions: Arc<Mutex<Registry>>,
    /// The prelude API reference, generated on first use. The `.funi` sources
    /// are embedded in this binary, so it cannot change while we run.
    docs: Arc<OnceLock<Result<ApiReference, String>>>,
}

fn ok_text(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// A tool-level error: the request was valid, the operation failed in a way the
/// caller should read (a 400 from `/input`, a load error from `/reload-source`).
fn tool_error(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(text)]))
}

fn code_tool_error(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    tool_error(truncate_code_error(text.into()))
}

/// Collapse an early `Err(String)` into a tool error, or run the body.
macro_rules! resolve {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(message) => return tool_error(message),
        }
    };
}

#[derive(Deserialize, JsonSchema)]
pub struct LaunchArgs {
    /// Project directory (the one holding `functor.json`), absolute or relative
    /// to the MCP server's working directory. Give this OR `files`.
    pub dir: Option<String>,
    /// `[path, source]` pairs — the whole project inline, entry `.fun` first.
    /// The server writes them to a scratch directory it owns and removes when
    /// the session stops, so a client with no filesystem can run a game it
    /// just wrote. A `functor.json` pair is honored; without one the server
    /// synthesizes a single-entry manifest. Give this OR `dir`.
    pub files: Option<Vec<(String, String)>>,
    /// Named entry, for a project whose functor.json declares `entries` — the
    /// ROLE this session runs (`"server"`, `"client"`). Defaults to `client`,
    /// or the sole entry. Launch one session per role to simulate a
    /// multiplayer game.
    pub entry: Option<String>,
    /// `"hidden"` (default) renders into an invisible window — `capture_frame`
    /// works. `"headless"` creates no GL context at all — no display or GPU
    /// needed, but `capture_frame` then fails.
    pub mode: Option<String>,
    /// Include the runtime's full endpoint index in the response (default
    /// false). The response always carries `protocol_version`, which is the
    /// part a caller acts on; the index is the same ~2 KB on every launch, and
    /// `GET /` on the session's url serves it at any time.
    pub discovery: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ConnectArgs {
    /// Base URL of a running debug server, e.g. `http://127.0.0.1:8123`.
    pub url: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SessionArgs {
    /// Session id from `launch_game` / `connect_game` (see `list_sessions`).
    pub session: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct InputArgs {
    pub session: String,
    /// The `POST /input` body verbatim, tagged by `type`.
    pub command: Value,
}

#[derive(Deserialize, JsonSchema)]
pub struct PauseArgs {
    pub session: String,
    /// Time-to-show to pin at. Defaults to the session's current `tts`, so the
    /// game freezes exactly where it is.
    pub tts: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StepArgs {
    pub session: String,
    /// Delta time for each step, in seconds (default 0.016).
    pub dts: Option<f64>,
    /// How many steps to queue (default 1, maximum 10,000).
    pub frames: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StepAllArgs {
    /// The sessions to step, in the order they must step — producer first,
    /// then the authority, then any observer. The order is the caller's
    /// declaration and this tool preserves it exactly.
    pub sessions: Vec<String>,
    /// Delta time for each step, in seconds (default 0.016). Every session
    /// steps by the same amount, which is what keeps them in lockstep.
    pub dts: Option<f64>,
    /// How many steps each session runs per round (default 1, maximum 10,000).
    pub frames: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LaunchGroupArgs {
    /// Project directory (the one holding `functor.json`), absolute or
    /// relative to the MCP server's working directory. A group launches from a
    /// directory only: its roles come from that manifest's `entries`.
    pub dir: String,
    /// The roles to launch, IN ORDER — the launch order and the order to pass
    /// to `step_all`, so put the authority where the protocol needs it.
    /// Repeats are allowed and are how a group gets two clients:
    /// `["server", "client", "client"]`. Omit it for one session per declared
    /// entry, with a role named `server` first.
    pub roles: Option<Vec<String>>,
    /// `"headless"` (default here — a group is usually driven, not watched)
    /// creates no GL context at all; `"hidden"` renders into an invisible
    /// window so `capture_frame` returns pixels.
    pub mode: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WireLogArgs {
    /// Group id from `launch_session_group`. Omit it when only one group is
    /// running.
    pub group: Option<String>,
    /// Return only rows with a HIGHER `seq` than this — read the last row's
    /// `seq` before a `step_all` round and pass it after to get exactly that
    /// round's traffic.
    pub since: Option<u64>,
    /// Newest-first row cap (default 100, maximum 2000). Rows are returned
    /// oldest-first within that window.
    pub limit: Option<usize>,
    /// One session label (`"server"`, `"client2"`), or an unordered pair
    /// (`"client1:server"`) — only traffic touching it, or between them.
    pub link: Option<String>,
    /// With a single-label `link`: `"sent"` (rows FROM it) or `"received"`
    /// (rows TO it). Without one it is an error, since there is no reference
    /// point for a direction.
    pub direction: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RewindArgs {
    pub session: String,
    /// The recorded rendered frame to restore.
    pub frame: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReloadSourceArgs {
    pub session: String,
    /// The complete new `.fun` source for the entry module.
    pub source: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReloadProjectArgs {
    pub session: String,
    /// `[path, source]` pairs covering every sibling module, entry first.
    pub files: Vec<(String, String)>,
}

#[derive(Deserialize, JsonSchema)]
pub struct InitGameArgs {
    /// Directory to scaffold, absolute or relative to the MCP server's
    /// working directory. It is created if it does not exist.
    pub dir: String,
    /// `"3d"` (default — a small lit scene) or `"fps"` (WASD + mouse-look).
    pub template: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveProjectArgs {
    pub session: String,
    /// Directory to write the project into, absolute or relative to the MCP
    /// server's working directory. Created if it does not exist.
    pub dir: String,
    /// Replace a project already in `dir` (default false, which refuses):
    /// its `.fun`/`.funi` modules are overwritten, and any this session does
    /// NOT have is deleted, so the directory ends up being exactly the
    /// running program. Its `functor.json` is kept.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ApiReferenceArgs {
    /// What to look for, matched case-insensitively against item names,
    /// qualified paths (`Scene.cube`), signatures and doc text. Omit it (with
    /// `module` set) to list a whole module.
    pub query: Option<String>,
    /// Narrow to one prelude module, e.g. `"Scene"` or `"Effect"`.
    pub module: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LanguageGuideArgs {
    /// A section name from the guide's own table of contents, e.g.
    /// `"syntax-subset"` or `"the-game-contract"` (a unique fragment of one is
    /// enough). Omit it to get the table of contents plus the quick facts.
    pub section: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunGameCodeUnsafeArgs {
    /// Session id from `launch_game` / `connect_game`.
    pub session: String,
    /// A JavaScript function invoked as `await program(game)`, for example:
    /// `async (game) => { await game.pause(); await game.step(); return await game.state(); }`.
    /// This is arbitrary local Node.js code and is RCE-equivalent by design.
    pub code: String,
    /// Execution wall-clock limit in milliseconds (default and maximum
    /// 120,000). It covers setup, code, and the final state snapshot. Direct
    /// child shutdown (2 seconds) and safety cleanup (30 seconds) are separately
    /// bounded and may extend the response. Processes deliberately started by
    /// trusted submitted code are outside this lifecycle boundary.
    pub timeout_ms: Option<u64>,
    /// Include the complete final `/state` response (default true). Set false
    /// when submitted code already selects its proof into `return_value`: the
    /// parent still takes the final snapshot for cleanup/integrity, but returns
    /// only `final_state_summary` with runtime/input facts and model byte size.
    pub include_final_state: Option<bool>,
}

#[tool_router]
impl FunctorMcp {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("failed to build the MCP HTTP client"),
            sessions: Arc::new(Mutex::new(Registry::default())),
            docs: Arc::new(OnceLock::new()),
        }
    }

    /// Search the engine API reference: the `.funi` prelude embedded in this
    /// binary — the same reference `functor docs` renders. `query` matches
    /// case-insensitively against item names, qualified paths (`Scene.cube`),
    /// signatures and doc text, best matches first; `module` narrows to one
    /// prelude module, and lists all of its items when `query` is omitted. This
    /// tool needs no session — it answers before any game is launched.
    #[tool]
    async fn api_reference(
        &self,
        Parameters(args): Parameters<ApiReferenceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let reference = resolve!(self.api_docs());
        let query = args.query.as_deref().unwrap_or("");
        let hits = resolve!(search_api(reference, query, args.module.as_deref()));
        if hits.is_empty() {
            return tool_error(format!(
                "no API items match {query:?}{}. {}",
                match args.module.as_deref() {
                    Some(module) => format!(" in module {module:?}"),
                    None => String::new(),
                },
                modules_hint(reference)
            ));
        }
        // A whole-module listing is bounded by the module itself, so browsing
        // is never truncated — only an open search is.
        let limit = (!query.trim().is_empty()).then_some(MAX_API_RESULTS);
        ok_text(render_api_hits(&hits, limit))
    }

    /// Read the Functor Lang LANGUAGE guide: syntax, semantics, the modules
    /// model, the `init`/`tick`/`draw` game contract, and hot-reload rules —
    /// the `functor-lang` skill, embedded verbatim, which is this repository's
    /// source of truth for the language. Functor Lang is NOT F# or OCaml: it
    /// has `:=` assignment, thread-LAST pipelines (`x |> f(a)` is `f(a, x)`),
    /// `if/then/else` only as an expression, no loops and no `<>` — guessing
    /// from F#/OCaml habits produces parse errors, so read this first. Called
    /// with no arguments it returns the table of contents plus those quick
    /// facts; `section` returns one section's full text. This is the language;
    /// `api_reference` is the prelude API (`Scene.cube`'s signature). Neither
    /// needs a session.
    #[tool]
    async fn language_guide(
        &self,
        Parameters(args): Parameters<LanguageGuideArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let (intro, sections) = guide_sections(LANGUAGE_GUIDE);
        match args
            .section
            .as_deref()
            .map(str::trim)
            .filter(|section| !section.is_empty())
        {
            Some(wanted) => {
                let index = resolve!(find_guide_section(&sections, wanted));
                ok_text(render_guide_section(&sections, index))
            }
            None => ok_text(render_guide_contents(intro, &sections)),
        }
    }

    /// Start a game as a child process with its debug server on a free port,
    /// and return its session id. The project comes from `dir` (a directory
    /// holding `functor.json`) OR from `files` — the whole project inline, so
    /// a client with no filesystem can run a game it just wrote. Defaults to
    /// `hidden` mode (an invisible GL window, so `capture_frame` returns
    /// pixels); `headless` needs no display or GPU at all but has no pixels,
    /// so `capture_frame` fails there.
    #[tool]
    async fn launch_game(
        &self,
        Parameters(args): Parameters<LaunchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mode = args.mode.as_deref().unwrap_or("hidden");
        let mode_flag = match mode {
            "hidden" => "--hidden",
            "headless" => "--headless",
            other => {
                return tool_error(format!(
                    "unknown mode {other:?}: expected \"hidden\" (pixels, capture_frame works) \
or \"headless\" (no GL, no capture)"
                ))
            }
        };
        // Exactly one source of game source: a project already on disk, or the
        // whole project inline. The inline form is written to a scratch
        // directory this server owns and then launched like any other, so the
        // runtime's own load path, file-watch hot reload, and `reload_source`
        // all keep working afterwards.
        let (dir, scratch) = match (&args.dir, &args.files) {
            (Some(dir), None) => (std::path::PathBuf::from(dir), None),
            (None, Some(files)) => {
                let files = resolve!(scratch_project_files(files));
                let scratch = resolve!(scratch_dir());
                resolve!(write_project_files(&scratch.0, &files));
                (scratch.0.clone(), Some(scratch))
            }
            (Some(_), Some(_)) => {
                return tool_error(
                    "give dir OR files, not both: dir launches a project already on disk, \
files writes the inline project to a scratch directory this server owns",
                )
            }
            (None, None) => {
                return tool_error(
                    "give dir (a project directory holding functor.json) or files \
([path, source] pairs, the entry .fun first) to launch a project that is not on disk",
                )
            }
        };
        // Remember the manifest this session boots with, BEFORE the game runs:
        // `GET /project` reports modules, not project metadata, so this is the
        // only copy `save_project` can write back. Both arms above leave a
        // `functor.json` in `dir` (the inline path synthesizes one), and an
        // unreadable manifest is not this tool's error to raise — the launch
        // below fails on it with the runtime's own message.
        let manifest = std::fs::read_to_string(dir.join("functor.json")).ok();
        let spawned = resolve!(
            self.spawn_runtime(&dir, args.entry.as_deref(), mode_flag, scratch, manifest, false)
                .await
        );
        let (id, url, port, discovery) = (
            spawned.session,
            spawned.url,
            spawned.port,
            spawned.discovery,
        );
        let mut response = serde_json::json!({
            "session": id,
            "url": url,
            "port": port,
            "mode": mode,
            "owned": true,
            "dir": absolute(&dir),
            "protocol_version": discovery.get("protocol_version").cloned().unwrap_or(Value::Null),
        });
        // The endpoint index is ~2 KB of text that says the same thing on every
        // launch, and a client launching three roles paid for it three times.
        // The version is the part a caller acts on; the index stays one
        // argument away (and `connect_game`, where the runtime is a stranger,
        // still returns it unconditionally).
        if args.discovery.unwrap_or(false) {
            response["discovery"] = discovery;
        }
        ok_text(response.to_string())
    }

    /// Launch a whole MULTIPLAYER session at once, with THIS PROCESS as the
    /// network. Every role starts on the embedder transport, so no socket is
    /// ever opened: each runtime's `Sub.listen` / `Sub.connect` / `Effect.send`
    /// traffic is drained by this server, routed by authority (a client url's
    /// `host:port` matching a server's bind), and delivered back — and every
    /// routed packet is readable as data through `wire_log`.
    ///
    /// Roles come from the project's `functor.json` `entries`. `roles` is the
    /// launch order AND the order to hand `step_all`; repeats give a group two
    /// clients. Returns the group id and one session per role, labelled the way
    /// the wire log names them.
    ///
    /// Routing runs on this server's own loop: continuously in the background
    /// (~8 ms) while the group runs live, and — the part that matters for
    /// evaluation — at every session boundary INSIDE a `step_all` round, with
    /// the background pump held off for the whole round. So a packet a client
    /// sends in round N reaches the authority before the authority steps.
    ///
    /// Delivery in v1 is IMMEDIATE and in order (a perfect, reliable link):
    /// the router never drops or reorders, and a failed delivery is re-queued
    /// at the front and retried. Latency, jitter, and step-time scheduling
    /// arrive with barrier stepping.
    #[tool]
    async fn launch_session_group(
        &self,
        Parameters(args): Parameters<LaunchGroupArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mode = args.mode.as_deref().unwrap_or("headless");
        let mode_flag = match mode {
            "hidden" => "--hidden",
            "headless" => "--headless",
            other => {
                return tool_error(format!(
                    "unknown mode {other:?}: expected \"headless\" (default; no GL) or \
\"hidden\" (an invisible window, so capture_frame works)"
                ))
            }
        };
        let dir = std::path::PathBuf::from(&args.dir);
        let manifest_text = resolve!(std::fs::read_to_string(dir.join("functor.json")).map_err(
            |error| format!("cannot read {}/functor.json: {error}", absolute(&dir))
        ));
        let declared = resolve!(declared_entries(&manifest_text));
        let roles = resolve!(group_roles(args.roles.as_deref(), &declared));
        let labels = group_labels(&roles);

        let group_id = {
            let mut registry = self.sessions.lock().expect("mcp registry poisoned");
            registry.next_group += 1;
            format!("g{}", registry.next_group)
        };

        let mut members: Vec<GroupMember> = Vec::with_capacity(roles.len());
        for (role, label) in roles.iter().zip(&labels) {
            match self
                .spawn_runtime(
                    &dir,
                    Some(role),
                    mode_flag,
                    None,
                    Some(manifest_text.clone()),
                    true,
                )
                .await
            {
                Ok(spawned) => members.push(GroupMember {
                    session: spawned.session,
                    label: label.clone(),
                    role: role.clone(),
                    url: spawned.url,
                }),
                Err(message) => {
                    // A half-launched group is a half-wired network: tear the
                    // launched roles down rather than leave the caller to guess
                    // which ones exist.
                    let orphaned = self.abandon_group_members(&members).await;
                    let mut error =
                        format!("launch_session_group stopped at role {role:?}: {message}");
                    if !orphaned.is_empty() {
                        error.push_str(&format!(
                            "\n\nWARNING: could not confirm termination of {} — those runtimes \
may still be running, and this server no longer tracks them.",
                            orphaned.join(", ")
                        ));
                    }
                    return tool_error(error);
                }
            }
        }

        let mut net = NetCoordinator::new();
        for member in &members {
            net.add_session(&member.label);
        }
        let group = Arc::new(tokio::sync::Mutex::new(Group {
            members: members.clone(),
            net,
            unreachable: BTreeSet::new(),
        }));
        {
            let mut registry = self.sessions.lock().expect("mcp registry poisoned");
            for member in &members {
                registry
                    .session_groups
                    .insert(member.session.clone(), group_id.clone());
            }
            registry.groups.insert(group_id.clone(), group.clone());
        }
        self.spawn_pump(group_id.clone(), group);

        ok_text(
            serde_json::json!({
                "group": group_id,
                "dir": absolute(&dir),
                "mode": mode,
                "transport": "embedder (this process is the network; no sockets)",
                "sessions": members
                    .iter()
                    .map(|member| serde_json::json!({
                        "session": member.session,
                        "label": member.label,
                        "role": member.role,
                        "url": member.url,
                    }))
                    .collect::<Vec<_>>(),
                "step_order": members.iter().map(|m| m.session.clone()).collect::<Vec<_>>(),
            })
            .to_string(),
        )
    }

    /// The WIRE, as data: every packet this process routed for a group, in
    /// order. One row per delivery — `{seq, frame, at_ms, from, to, conn, kind,
    /// size, payload_text}`.
    ///
    /// `payload_text` is the message exactly as the receiving runtime sees it,
    /// undecoded on purpose: the framing belongs to the game (`Effect.sendMsg`
    /// writes JSON), so YOU decode it — a coordinator that parsed payloads
    /// would be inventing a protocol it does not own.
    ///
    /// `frame` is `null` in v1: delivery is immediate and unscheduled, so there
    /// is no step to name it with. `seq` is the stable cursor — read the last
    /// row's `seq`, run a `step_all` round, then pass it as `since` to get
    /// exactly that round's traffic.
    #[tool]
    async fn wire_log(
        &self,
        Parameters(args): Parameters<WireLogArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let (group_id, group) = resolve!(self.group_by_id(args.group.as_deref()));
        let limit = args.limit.unwrap_or(WIRE_LOG_DEFAULT_ROWS);
        if limit == 0 || limit > WIRE_LOG_MAX_ROWS {
            return tool_error(format!(
                "limit must be between 1 and {WIRE_LOG_MAX_ROWS} (got {limit})"
            ));
        }
        let guard = group.lock().await;
        let known: Vec<String> = guard.net.labels().to_vec();
        let filter = resolve!(WireFilter::parse(
            args.link.as_deref(),
            args.direction.as_deref(),
            &known
        ));
        let matched: Vec<&crate::commands::mcp_net::Packet> = guard
            .net
            .packets()
            .filter(|packet| args.since.is_none_or(|since| packet.seq > since))
            .filter(|packet| filter.matches(packet))
            .collect();
        let total = matched.len();
        // A row cap alone is not an output bound — a game chooses how big a
        // message is, and 2000 rows of a chatty protocol is megabytes straight
        // into a model's context. So payloads also share a byte budget, spent
        // NEWEST-first (a reader asking for a window wants its tail); an
        // elided row keeps every other field and says so.
        let mut payload_budget = MAX_WIRE_PAYLOAD_BYTES;
        let mut elided = 0usize;
        let mut rows: Vec<Value> = matched
            .into_iter()
            .skip(total.saturating_sub(limit))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|packet| {
                let payload = match &packet.payload_text {
                    Some(text) if text.len() <= payload_budget => {
                        payload_budget -= text.len();
                        Value::String(text.clone())
                    }
                    Some(_) => {
                        elided += 1;
                        Value::Null
                    }
                    None => Value::Null,
                };
                serde_json::json!({
                    "seq": packet.seq,
                    // Always null in v1 — delivery is immediate and
                    // unscheduled, so there is no step to name it with.
                    "frame": packet.frame,
                    "at_ms": (packet.at_ms * 1000.0).round() / 1000.0,
                    "from": packet.from,
                    "to": packet.to,
                    "conn": packet.conn,
                    "kind": packet.kind,
                    "size": packet.size,
                    "payload_text": payload,
                    "payload_elided": packet.payload_text.is_some() && payload.is_null(),
                })
            })
            .collect();
        rows.reverse();
        ok_text(
            serde_json::json!({
                "group": group_id,
                "sessions": known,
                "routed": guard.net.routed(),
                "oldest_seq": guard.net.oldest_seq(),
                "matched": total,
                "returned": rows.len(),
                // How many returned rows dropped their payload to stay inside
                // the byte budget — narrow the window or the `link` to see them.
                "payloads_elided": elided,
                "connections": guard.net.connection_counts(),
                "rows": rows,
            })
            .to_string(),
        )
    }

    /// Attach to a debug server this process does NOT own — a runtime someone
    /// else started, or an adb-forwarded Quest on `http://127.0.0.1:8123`.
    /// `stop_game` on such a session only forgets it; the runtime keeps running.
    #[tool]
    async fn connect_game(
        &self,
        Parameters(args): Parameters<ConnectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let url = args.url.trim_end_matches('/').to_string();
        let reservation = self.reserve_connect(&url);
        let target = reservation.target();
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        let discovery = resolve!(self.discover(&url).await);
        // Discovery may have overlapped an owned stop marking this transient
        // lifecycle closing. Recheck atomically with insertion.
        let id = resolve!(reservation.finish());
        ok_text(
            serde_json::json!({
                "session": id,
                "url": url,
                "owned": false,
                "discovery": discovery,
            })
            .to_string(),
        )
    }

    /// List the known sessions: id, url, whether this server owns the process,
    /// whether it currently answers `GET /state`, and whether an ambiguous
    /// abort cleanup quarantined its exact URL from further mutations. The
    /// result also lists quarantined URL tombstones with no remaining id.
    #[tool]
    async fn list_sessions(&self) -> Result<CallToolResult, ErrorData> {
        let (known, quarantined_urls): (Vec<(String, String, bool, bool)>, Vec<String>) = {
            let guard = self.sessions.lock().expect("mcp registry poisoned");
            let known = guard
                .sessions
                .iter()
                .map(|(id, session)| {
                    (
                        id.clone(),
                        session.url.clone(),
                        session.owned,
                        session.quarantined.load(Ordering::Acquire),
                    )
                })
                .collect();
            let quarantined_urls = guard
                .url_quarantines
                .iter()
                .filter(|(_, quarantined)| quarantined.load(Ordering::Acquire))
                .map(|(url, _)| url.clone())
                .collect();
            (known, quarantined_urls)
        };
        let mut sessions = Vec::with_capacity(known.len());
        for (id, url, owned, quarantined) in known {
            let alive = self
                .http
                .get(format!("{url}/state"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false);
            sessions.push(serde_json::json!({
                "session": id, "url": url, "owned": owned, "alive": alive,
                "quarantined": quarantined,
            }));
        }
        ok_text(
            serde_json::json!({
                "sessions": sessions,
                "quarantined_urls": quarantined_urls,
            })
            .to_string(),
        )
    }

    /// Stop a session. An attached id is detached independently. Stopping a
    /// launched owner closes/removes all exact-URL aliases and pending
    /// connects, and keeps closing tombstones visible until its child exits.
    #[tool]
    async fn stop_game(
        &self,
        Parameters(args): Parameters<SessionArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if context.ct.is_cancelled() {
            return tool_error(format!(
                "request for session {:?} was cancelled before stop began",
                args.session
            ));
        }
        let (target, owned) = resolve!(self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .begin_stop(&args.session));
        // De-route BEFORE taking the session gate, for two reasons. It removes
        // a lock-order inversion: `step_all` takes a group lock and THEN the
        // per-session gates, so a stop holding a gate while waiting for the
        // group lock could hang both for the life of the process (neither
        // tokio mutex has a timeout). And it runs on every exit from here,
        // including the unconfirmed-termination error return below — otherwise
        // a group could outlive its last member and its pump would poll a dead
        // runtime forever. `begin_stop` has already marked the session closing,
        // so nothing new can start on it either way.
        self.forget_group_member(&args.session).await;
        // Stop is a lifecycle boundary: after `closing` is visible it must
        // drain the current operation and finish cleanup even if its own MCP
        // request is subsequently cancelled.
        let _operation = target.operation_gate.clone().lock_owned().await;
        let mut child = resolve!(self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .take_child(&args.session));
        let child_shutdown = match child.as_mut() {
            Some(child) => {
                terminate_child(child, "owned runtime child", RUNTIME_CHILD_SHUTDOWN_TIMEOUT).await
            }
            None if owned => {
                Err("owned runtime child was absent, so termination could not be confirmed".into())
            }
            None => Ok(()),
        };
        drop(child);
        if child_shutdown.is_err() {
            // Killing the owned process was not proven. Preserve the URL
            // tombstone after removing its unusable closing session records.
            target.quarantined.store(true, Ordering::Release);
        }
        if owned {
            // The owner and every exact-URL alias remain closing tombstones
            // until child cleanup finishes. Only now can the group disappear.
            let removed = {
                let mut registry = self.sessions.lock().expect("mcp registry poisoned");
                let removed = registry.remove_url(&target.url);
                if child_shutdown.is_ok() {
                    // Confirmed termination proves no ambiguous state survives
                    // at this lifecycle; a later process may safely reuse the URL.
                    registry.clear_owned_url_tombstone(&target.url);
                }
                removed
            };
            drop(removed);
        } else {
            let session = resolve!(self
                .sessions
                .lock()
                .expect("mcp registry poisoned")
                .remove(&args.session));
            drop(session);
        }
        if let Err(error) = child_shutdown {
            return tool_error(format!(
                "stopped session {} but could not confirm owned runtime termination: {error}. \
The closing session records were removed, but the exact URL {} remains quarantined. Restart this \
MCP server before reconnecting or reusing that runtime URL.",
                args.session, target.url
            ));
        }
        ok_text(format!(
            "stopped session {} ({})",
            args.session,
            if owned {
                "killed the launched runtime"
            } else {
                "detached; the runtime is still running"
            }
        ))
    }

    /// Runtime state JSON: `frame`, `tts`, `pending_steps`, viewports, the
    /// sampled `input`, and the game model — `model` is the structured,
    /// parseable JSON view (the thing to read); `model_debug` is the
    /// human-facing `Debug` text.
    #[tool]
    async fn get_state(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_get(&args.session, "/state").await
    }

    /// The current frame as data: the camera, the scene graph, and the lights
    /// the game's `draw` produced. Works headlessly (it never renders).
    #[tool]
    async fn get_scene(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_get(&args.session, "/scene").await
    }

    /// The paused inspector trace: the last real frame's entry-point
    /// invocations with the value at every binder and variable read. Pause the
    /// session first — while it is playing this reports `{"paused": false}`.
    #[tool]
    async fn get_trace(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_get(&args.session, "/trace").await
    }

    /// Render the next frame and return it as a PNG image. Requires a GL
    /// context: a session launched in `headless` mode has no pixels and this
    /// fails — relaunch it in `hidden` mode. Raw capture responses are capped
    /// at 8 MiB.
    #[tool]
    async fn capture_frame(
        &self,
        Parameters(args): Parameters<SessionArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let target = resolve!(self.target(&args.session));
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        let body = resolve!(self.capture_png(&target.url).await);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(&body),
            "image/png",
        )]))
    }

    /// Inject one input event. `command` is the `POST /input` body verbatim,
    /// tagged by `type`: `{"type":"key","key":"w","down":true}`,
    /// `{"type":"mouse_move","x":10,"y":20}`, `{"type":"mouse_wheel","delta":1}`,
    /// `{"type":"mouse_button","button":"left","down":true}`,
    /// `{"type":"ui_event","slot":0,"kind":"Clicked"}` (also
    /// `{"SliderChanged":0.5}` / `{"TextChanged":"hi"}`),
    /// `{"type":"xr", "head":…, "left":…, "right":…}`, `{"type":"xr_clear"}`,
    /// `{"type":"gamepad", "left_stick":[0,1], "south":true}`,
    /// `{"type":"gamepad_clear"}`.
    /// The list mirrors `POST /input` and is not exhaustive; an unrecognized
    /// shape comes back as the runtime's own 400, which names the problem.
    /// Keys, held buttons, XR and gamepad samples are LEVEL state: they stay
    /// in force across steps until released, which is how a paused session is
    /// scripted.
    #[tool]
    async fn send_input(
        &self,
        Parameters(args): Parameters<InputArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_mutating_post(&args.session, "/input", args.command.to_string(), &context)
            .await
    }

    /// Run one Playwright-style JavaScript function in a Node child process,
    /// invoked as `await program(game)`. The injected `game` object mirrors the
    /// TypeScript SDK method surface, within this runner's documented bounds:
    /// state/scene/trace/capture, pause/step/stepFrames/resume, input helpers,
    /// reload/rewind, `waitForState`, and deterministic `stepUntil`. Calls
    /// route back through this MCP parent, so the existing exact-URL operation
    /// gate, runtime checks, cancellation, output caps, and cleanup remain
    /// authoritative. The result includes the JSON return value plus a bounded
    /// observability trace of SDK RPC received from trusted code; captures are
    /// returned as MCP image blocks.
    ///
    /// UNSAFE means exactly what it says: submitted code is arbitrary local
    /// Node.js and is RCE-equivalent. The child-process boundary contains a
    /// Node crash and lets the parent kill that direct child on timeout; it is
    /// NOT a security sandbox or a process-tree sandbox. Use only with code you
    /// trust as much as the MCP client itself. Node.js 20+ must be installed
    /// (`FUNCTOR_NODE` may name a non-default executable).
    #[tool]
    async fn run_game_code_unsafe(
        &self,
        Parameters(args): Parameters<RunGameCodeUnsafeArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.code.len() > MAX_NODE_CODE_BYTES {
            return code_tool_error(output_limit_error(
                "submitted Node.js source",
                args.code.len(),
                MAX_NODE_CODE_BYTES,
            ));
        }
        if args.code.trim().is_empty() {
            return code_tool_error(
                "code must be a JavaScript function, for example: async (game) => { return await game.state(); }",
            );
        }
        let timeout_ms = args
            .timeout_ms
            .unwrap_or(OPERATION_TOTAL_TIMEOUT.as_millis() as u64);
        let timeout = match timeout_ms {
            1..=120_000 => Duration::from_millis(timeout_ms),
            other => {
                return code_tool_error(format!(
                    "timeout_ms must be between 1 and 120000, got {other}"
                ))
            }
        };
        let target = match self.target(&args.session) {
            Ok(target) => target,
            Err(message) => return code_tool_error(message),
        };
        let _operation = match self.acquire_operation(&target, &context).await {
            Ok(operation) => operation,
            Err(message) => return code_tool_error(message),
        };

        match self
            .execute_node_code(
                &target,
                &args.code,
                timeout,
                args.include_final_state.unwrap_or(true),
                &context,
            )
            .await
        {
            Ok(output) => match node_code_call_result(output) {
                Ok(result) => Ok(result),
                Err(message) => code_tool_error(message),
            },
            Err(message) => code_tool_error(message),
        }
    }

    /// Pause: pin the clock to a constant time, so nothing advances until
    /// `step` or `resume`. Window keyboard/mouse input is ignored while pinned,
    /// but injected `send_input` still applies — this is how a driver gets
    /// deterministic control. Defaults to pinning at the current `tts`.
    #[tool]
    async fn pause(
        &self,
        Parameters(args): Parameters<PauseArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let target = resolve!(self.target(&args.session));
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        ok_text(resolve!(self.pin(&target.url, args.tts).await))
    }

    /// Run exactly `frames` simulation steps of `dts` seconds each, WAIT for
    /// them to land (polling until `pending_steps` is 0), then return the fresh
    /// `/state`. Step one frame at a time when the game must see input or I/O
    /// between steps — a batch runs up to 8 ticks per rendered frame, so it has
    /// proportionally fewer input/network/render points. `frames` must be
    /// between 1 and 10,000 and the acquired operation has a 120-second
    /// wall-clock deadline. Timeout/stop abort cancels any unlanded queue before
    /// releasing the session gate.
    #[tool]
    async fn step(
        &self,
        Parameters(args): Parameters<StepArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let (dts, frames) = resolve!(step_args("step", args.dts, args.frames));
        let target = resolve!(self.target(&args.session));
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        let operation_deadline = Instant::now() + OPERATION_TOTAL_TIMEOUT;
        let state = resolve!(self.advance(&target, dts, frames, operation_deadline).await);
        ok_text(state.to_string())
    }

    /// Step several sessions ONE ROUND, strictly sequentially, in the order
    /// given — the multiplayer lockstep primitive. Each session's steps fully
    /// land (`pending_steps` back to 0) before the next session starts; a
    /// session's already-received network events are drained (`pending_net`
    /// back to 0) before IT steps; and the result carries every session's
    /// post-step state summary, in the order given.
    ///
    /// The order is the contract: **step producer → authority → observer**.
    /// A client's input has to reach the authority before the authority steps,
    /// and the authority's broadcast has to exist before an observer steps, or
    /// each round lands work an arbitrary number of rounds late. Stepping the
    /// same group CONCURRENTLY is not reproducible: whether a packet arrives
    /// before or after its receiver's step is then a race, so two identical
    /// runs disagree. Pause every session first — this steps the clock, it
    /// does not pin it.
    ///
    /// What it does NOT promise: a packet still ON THE WIRE is invisible to
    /// every process, so this orders the stepping and drains what has already
    /// arrived — it is not a transport barrier. A game whose correctness
    /// depends on delivery within one round needs its own convergence check
    /// (poll `get_state` for the game-level fact) between rounds.
    ///
    /// Sessions are resolved and checked for duplicates — by runtime URL, so
    /// two ids aliasing one game are refused — before anything steps, so a
    /// typo cannot leave the group half-advanced. A failure partway through
    /// still means the earlier sessions advanced: like the SDK's `stepAll`,
    /// there is no rollback, so treat it as terminal for the run. The whole
    /// call shares ONE 120-second wall-clock deadline.
    #[tool]
    async fn step_all(
        &self,
        Parameters(args): Parameters<StepAllArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let (dts, frames) = resolve!(step_args("step_all", args.dts, args.frames));
        if args.sessions.is_empty() {
            return tool_error(
                "step_all needs at least one session id, in the order they must step \
(producer → authority → observer)",
            );
        }
        // Resolve everything up front: a group half-stepped because the third
        // id was a typo is a desynced simulation with no way back. Duplicates
        // are rejected by URL rather than by id, because two ids can alias one
        // runtime — and stepping that game twice is exactly what this prevents.
        let mut targets: Vec<(String, SessionTarget)> = Vec::with_capacity(args.sessions.len());
        for session in &args.sessions {
            let target = resolve!(self.target(session));
            if let Some((other, _)) = targets.iter().find(|(_, seen)| seen.url == target.url) {
                return tool_error(format!(
                    "step_all lists {other:?} and {session:?}, which are the same runtime ({}) — \
a session that must step twice per round is two rounds, not one list",
                    target.url
                ));
            }
            targets.push((session.clone(), target));
        }

        // Coordinated groups among the listed sessions: their locks are held
        // for the WHOLE round, which suspends the background routing pump.
        // Routing then happens only where this loop puts it — at each session
        // boundary — so a packet lands before its receiver's step rather than
        // wherever an 8 ms timer fired. Locked in group-id order, and no other
        // path ever holds two, so this cannot deadlock.
        let mut coordinated: BTreeMap<String, Arc<tokio::sync::Mutex<Group>>> = BTreeMap::new();
        for (session, _) in &targets {
            if let Some((id, group)) = self.group_of(session) {
                coordinated.insert(id, group);
            }
        }
        // ONE deadline for the call, not one per session: N sessions must not
        // multiply how long a single tool call can block. Started BEFORE the
        // group locks, since waiting for a concurrent round to finish is time
        // this call spends like any other.
        let operation_deadline = Instant::now() + OPERATION_TOTAL_TIMEOUT;
        let mut groups: Vec<tokio::sync::OwnedMutexGuard<Group>> = Vec::new();
        for group in coordinated.into_values() {
            groups.push(group.lock_owned().await);
        }
        let mut stepped: Vec<Value> = Vec::with_capacity(targets.len());
        for (session, target) in targets {
            let desync = |message: String| {
                if stepped.is_empty() {
                    format!("step_all stopped at session {session:?}: {message}")
                } else {
                    format!(
                        "step_all stopped at session {session:?}: {message}\n\nSessions before it \
in the list have already advanced; the group is no longer in lockstep."
                    )
                }
            };
            // One gate at a time, in order: holding all of them would deadlock
            // against any other caller holding one of ours.
            let _operation = match self.acquire_operation(&target, &context).await {
                Ok(operation) => operation,
                Err(message) => return tool_error(desync(message)),
            };
            // Route first: whatever the previous session in this round emitted
            // becomes this one's inbound BEFORE it steps. `POST /net/deliver`
            // folds each event through `update` before answering, so by the
            // time this returns the model has already absorbed the round's
            // traffic.
            for group in groups.iter_mut() {
                self.pump_locked(group).await;
            }
            // Drain what this session has ALREADY received before it steps, so
            // the round folds it in rather than carrying it to the next one.
            if let Err(message) = self.settle_net(&target, operation_deadline).await {
                return tool_error(desync(message));
            }
            let state = match self.advance(&target, dts, frames, operation_deadline).await {
                Ok(state) => state,
                Err(message) => return tool_error(desync(message)),
            };
            let mut summary = summarize_state(&state);
            summary["session"] = Value::String(session);
            stepped.push(summary);
        }
        // One final round so the LAST session's emissions are routed before the
        // call returns: a `wire_log` taken straight afterwards then shows the
        // whole round rather than all-but-the-tail.
        for group in groups.iter_mut() {
            self.pump_locked(group).await;
        }
        ok_text(
            serde_json::json!({ "dts": dts, "frames": frames, "sessions": stepped }).to_string(),
        )
    }

    /// Un-pin the clock: the game follows wall-clock time again, and window
    /// input reaches it once more.
    #[tool]
    async fn resume(
        &self,
        Parameters(args): Parameters<SessionArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_mutating_post(
            &args.session,
            "/time",
            serde_json::json!({ "type": "resume" }).to_string(),
            &context,
        )
        .await
    }

    /// Restore the model and physics world to a recorded rendered frame, and
    /// return the resulting `/state`. Rewind requires a pinned clock, so this
    /// pins at the current time first. Frames outside the recorded window are
    /// an error.
    #[tool]
    async fn rewind(
        &self,
        Parameters(args): Parameters<RewindArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let target = resolve!(self.target(&args.session));
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        let tts = resolve!(tts_of(&resolve!(self.state(&target.url).await)));
        // The clock must be pinned before a rewind or the next wall-clock frame
        // would immediately overwrite the restored model. `/state` does not
        // report whether it already is, so this re-pins at the CURRENT time —
        // a no-op for an already-paused session.
        resolve!(
            self.post(
                &target.url,
                "/time",
                serde_json::json!({ "type": "set", "tts": tts }).to_string(),
            )
            .await
        );
        resolve!(
            self.post(
                &target.url,
                "/rewind",
                serde_json::json!({ "frame": args.frame }).to_string(),
            )
            .await
        );
        ok_text(resolve!(self.state(&target.url).await).to_string())
    }

    /// Hot-reload the entry module from new source, preserving the live model.
    /// A source error is returned verbatim (the runtime keeps running the old
    /// program). Use `reload_project` when sibling modules changed too.
    #[tool]
    async fn reload_source(
        &self,
        Parameters(args): Parameters<ReloadSourceArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_mutating_post(&args.session, "/reload-source", args.source, &context)
            .await
    }

    /// Scaffold a new project on disk — the same `functor.json` + `game.fun`
    /// starter `functor init` writes. `template` is `"3d"` (default, a small
    /// lit scene) or `"fps"` (WASD + mouse-look). Existing files are never
    /// overwritten. The directory it returns is ready to pass straight to
    /// `launch_game`'s `dir`.
    #[tool]
    async fn init_game(
        &self,
        Parameters(args): Parameters<InitGameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = args.template.as_deref().unwrap_or("3d");
        let template = match name {
            "3d" => crate::commands::init::Template::ThreeD,
            "fps" => crate::commands::init::Template::Fps,
            other => {
                return tool_error(format!(
                    "unknown template {other:?}: expected \"3d\" (a small lit 3D scene) \
or \"fps\" (a first-person WASD + mouse-look scene)"
                ))
            }
        };
        let dir = std::path::PathBuf::from(&args.dir);
        // The scaffolder's own refusal to overwrite is the contract here: a
        // half-written project is worse than a failed call.
        resolve!(crate::commands::init::execute(&dir, &template)
            .map_err(|error| format!("could not initialize a project: {error}")));
        ok_text(
            serde_json::json!({
                "dir": absolute(&dir),
                "template": name,
                "files": template.file_names(),
            })
            .to_string(),
        )
    }

    /// Write a session's CURRENT project source to a directory — how a game
    /// authored over the wire gets a durable home. The sources come from the
    /// RUNTIME (`GET /project`), so they are what it is actually running,
    /// including every `reload_source`/`reload_project` edit that never
    /// touched a file. The `functor.json` the session BOOTED with is written
    /// alongside them, so a multi-entry (multiplayer) project keeps its roles;
    /// a directory's own manifest is never rewritten, and an ATTACHED session,
    /// whose manifest is not knowable over the wire, gets a synthesized
    /// single-entry one. A directory that already holds a project is REFUSED
    /// unless `overwrite` is true.
    #[tool]
    async fn save_project(
        &self,
        Parameters(args): Parameters<SaveProjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let target = resolve!(self.target(&args.session));
        let _operation = resolve!(self.acquire_operation(&target, &context).await);
        let body = match self.get(&target.url, "/project").await {
            Ok(body) => body,
            // A 404 is specifically "this runtime predates the route"; a 501
            // is a producer that has no sources at all, which rebuilding
            // would not change.
            Err(message) if message.contains("→ 404") => {
                return tool_error(format!(
                    "{message}\n\nsave_project asks the runtime for its sources (an edited \
session's source may exist nowhere else), and GET /project needs debug protocol v5 — \
rebuild that runtime from this version of Functor."
                ))
            }
            Err(message) => return tool_error(message),
        };
        let files: Vec<(String, String)> = resolve!(serde_json::from_str(&body)
            .map_err(|error| format!("GET /project did not return [path, source] pairs: {error}")));
        let manifest = self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .manifest(&args.session);
        let dir = std::path::PathBuf::from(&args.dir);
        let written = resolve!(save_project_to(
            &dir,
            &files,
            manifest.as_deref(),
            args.overwrite.unwrap_or(false)
        ));
        ok_text(
            serde_json::json!({
                "dir": absolute(&dir),
                "files": written,
            })
            .to_string(),
        )
    }

    /// Hot-reload every sibling module at once from `[path, source]` pairs,
    /// entry first, preserving the live model. A load error is returned
    /// verbatim and the old program keeps running.
    #[tool]
    async fn reload_project(
        &self,
        Parameters(args): Parameters<ReloadProjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let files: Vec<Vec<String>> = args
            .files
            .into_iter()
            .map(|(path, source)| vec![path, source])
            .collect();
        self.proxy_mutating_post(
            &args.session,
            "/reload-project",
            serde_json::to_string(&files).expect("string pairs serialize"),
            &context,
        )
        .await
    }
}

impl FunctorMcp {
    /// The prelude API reference, generated once per process.
    fn api_docs(&self) -> Result<&ApiReference, String> {
        self.docs
            .get_or_init(|| {
                functor_docgen::generate().map_err(|error| {
                    format!("could not generate the embedded API reference: {error}")
                })
            })
            .as_ref()
            .map_err(String::clone)
    }

    fn release_port(&self, port: u16) {
        self.sessions
            .lock()
            .expect("mcp registry poisoned")
            .release_port(port);
    }

    fn reserve_connect(&self, url: &str) -> ConnectReservation {
        let (operation_gate, quarantined, closing) = self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .reserve_connect(url);
        ConnectReservation {
            registry: Arc::downgrade(&self.sessions),
            url: url.to_string(),
            operation_gate,
            quarantined,
            closing,
            active: true,
        }
    }

    fn read_url(&self, session: &str) -> Result<String, String> {
        self.sessions
            .lock()
            .expect("mcp registry poisoned")
            .read_url(session)
    }

    fn target(&self, session: &str) -> Result<SessionTarget, String> {
        self.sessions
            .lock()
            .expect("mcp registry poisoned")
            .target(session)
    }

    /// Spawn one runtime child, wait for it to answer discovery, and register
    /// it. The shared body of `launch_game` and `launch_session_group` — the
    /// only difference between them is `embedder_net`, which decides whether
    /// the child opens real sockets or hands its traffic to this process.
    async fn spawn_runtime(
        &self,
        dir: &Path,
        entry: Option<&str>,
        mode_flag: &str,
        scratch: Option<ScratchDir>,
        manifest: Option<String>,
        embedder_net: bool,
    ) -> Result<SpawnedRuntime, String> {
        let port = self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .reserve_port()?;
        let exe = std::env::current_exe()
            .map_err(|error| format!("cannot locate the functor executable: {error}"))?;

        let mut command = Command::new(exe);
        command.arg("-d").arg(dir);
        if let Some(entry) = entry {
            command.arg("--entry").arg(entry);
        }
        command
            .args(["run", "native", "--debug-port"])
            .arg(port.to_string())
            .arg(mode_flag);
        if embedder_net {
            // The runner arg the CLI forwards verbatim: no socket is opened,
            // and the drained ConnCommands wait for `GET /net/outbound`.
            command.args(["--net-transport", "embedder"]);
        }
        command
            // stdout is this server's JSON-RPC channel — a child must never
            // inherit it, or its logs would corrupt the protocol stream.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.release_port(port);
                return Err(format!("failed to spawn the runtime: {error}"));
            }
        };
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut drains = Vec::new();
        if let Some(stream) = child.stdout.take() {
            drains.push(drain_into(stream, log.clone()));
        }
        if let Some(stream) = child.stderr.take() {
            drains.push(drain_into(stream, log.clone()));
        }

        let url = format!("http://127.0.0.1:{port}");
        let discovery = match self.await_runtime(&url, &mut child).await {
            Ok(discovery) => discovery,
            Err(message) => {
                let _ = child.start_kill();
                self.release_port(port);
                // The output that explains a failed launch — a bind error, a
                // typecheck diagnostic — is written just before the child exits,
                // so let the drain tasks reach EOF before reading the tail.
                let _ = tokio::time::timeout(Duration::from_millis(500), async {
                    for drain in drains {
                        let _ = drain.await;
                    }
                })
                .await;
                return Err(format!("{message}\n\nruntime output:\n{}", tail(&log)));
            }
        };

        let session = self.sessions.lock().expect("mcp registry poisoned").insert(
            url.clone(),
            Some(port),
            Some(child),
            scratch,
            manifest,
        );
        Ok(SpawnedRuntime {
            session,
            url,
            port,
            discovery,
        })
    }

    /// Tear down the roles a failed group launch already started, returning
    /// the ones whose termination could NOT be confirmed. The caller is already
    /// reporting a failure; an orphaned runtime has to be named in it, since
    /// the session record is gone and `list_sessions` will not show it.
    async fn abandon_group_members(&self, members: &[GroupMember]) -> Vec<String> {
        let mut orphaned = Vec::new();
        for member in members {
            let child = {
                let mut registry = self.sessions.lock().expect("mcp registry poisoned");
                let child = registry.take_child(&member.session).ok().flatten();
                let _ = registry.remove(&member.session);
                child
            };
            if let Some(mut child) = child {
                if terminate_child(
                    &mut child,
                    "owned runtime child",
                    RUNTIME_CHILD_SHUTDOWN_TIMEOUT,
                )
                .await
                .is_err()
                {
                    orphaned.push(member.url.clone());
                }
            }
        }
        orphaned
    }

    /// Remove a session from its group's routing table, dropping the group
    /// (and so ending its pump) once it is empty.
    async fn forget_group_member(&self, session: &str) {
        let Some((group_id, group)) = self.group_of(session) else {
            return;
        };
        {
            let mut guard = group.lock().await;
            // Not `else { return }`: the registry cleanup below must run even
            // when the member is already gone, or `session_groups` keeps a
            // stale row and the group can never empty (so its pump never ends).
            if let Some(index) = guard
                .members
                .iter()
                .position(|member| member.session == session)
            {
                let label = guard.members.remove(index).label;
                guard.net.remove_session(&label);
            }
        }
        let mut registry = self.sessions.lock().expect("mcp registry poisoned");
        registry.session_groups.remove(session);
        let empty = registry
            .groups
            .get(&group_id)
            .is_some_and(|_| !registry.session_groups.values().any(|id| *id == group_id));
        if empty {
            registry.groups.remove(&group_id);
        }
    }

    /// The group a session belongs to, if any.
    fn group_of(&self, session: &str) -> Option<(String, Arc<tokio::sync::Mutex<Group>>)> {
        let registry = self.sessions.lock().expect("mcp registry poisoned");
        let id = registry.session_groups.get(session)?.clone();
        let group = registry.groups.get(&id)?.clone();
        Some((id, group))
    }

    fn group_by_id(&self, id: Option<&str>) -> Result<(String, Arc<tokio::sync::Mutex<Group>>), String> {
        let registry = self.sessions.lock().expect("mcp registry poisoned");
        match id {
            Some(id) => registry
                .groups
                .get(id)
                .cloned()
                .map(|group| (id.to_string(), group))
                .ok_or_else(|| {
                    if registry.groups.is_empty() {
                        format!(
                            "unknown group {id:?}: no groups yet — start one with \
launch_session_group"
                        )
                    } else {
                        format!(
                            "unknown group {id:?}: known groups are {}",
                            registry.groups.keys().cloned().collect::<Vec<_>>().join(", ")
                        )
                    }
                }),
            None if registry.groups.len() == 1 => {
                let (id, group) = registry.groups.iter().next().expect("one group");
                Ok((id.clone(), group.clone()))
            }
            None if registry.groups.is_empty() => Err(
                "no session group is running — launch_session_group starts one (its roles are \
wired to each other by this process, with no sockets)"
                    .to_string(),
            ),
            None => Err(format!(
                "several groups are running ({}) — name one with `group`",
                registry.groups.keys().cloned().collect::<Vec<_>>().join(", ")
            )),
        }
    }

    /// ONE routing round: take every member's queued `ConnCommand`s, route
    /// them, and hand each member what it is owed.
    ///
    /// The pump deliberately does NOT take a session's operation gate. It is
    /// the TRANSPORT — the thing a kernel does behind every other operation —
    /// so gating it would both deadlock against a `step_all` holding a gate and
    /// misrepresent what it models. `/net/outbound` and `/net/deliver` are the
    /// only endpoints it touches, and neither races the clock.
    ///
    /// A member that cannot be reached (it stopped, it is mid-reload) is
    /// skipped for the round rather than failing the group, and an undelivered
    /// batch is re-queued in order for the next one.
    async fn pump_group(&self, group: &Arc<tokio::sync::Mutex<Group>>) {
        let mut guard = group.lock().await;
        self.pump_locked(&mut guard).await;
    }

    /// [`Self::pump_group`] against a group lock the caller already holds — how
    /// `step_all` pumps at each session boundary while KEEPING the background
    /// pump out for the whole round. Holding the lock is what makes a round's
    /// deliveries fall on the boundaries the caller declared instead of
    /// wherever an 8 ms timer happened to fire.
    async fn pump_locked(&self, guard: &mut Group) {
        let members = guard.members.clone();
        for member in &members {
            let json = match self.get(&member.url, "/net/outbound").await {
                Ok(json) => json,
                // A member that cannot be drained is skipped for the round, but
                // never SILENTLY: the failure includes a 409 from a runtime
                // that is not on the embedder transport, which is the exact
                // misconfiguration `EMBEDDER_TRANSPORT_REQUIRED` exists to make
                // loud. Reported once per member until it succeeds again, so a
                // dead child does not print 125 lines a second.
                Err(error) => {
                    guard.note_unreachable(&member.label, format!("drain failed: {error}"));
                    continue;
                }
            };
            guard.note_reachable(&member.label);
            if json == "[]" {
                continue;
            }
            match serde_json::from_str::<Vec<ConnCommand>>(&json) {
                Ok(commands) => {
                    for command in commands {
                        guard.net.perform(&member.label, command);
                    }
                }
                // A runtime whose outbound JSON does not parse is a protocol
                // break, not a game bug — say so on stderr, which is this
                // server's log channel (stdout is JSON-RPC).
                Err(error) => eprintln!(
                    "[coordinator] {} sent unparseable outbound commands: {error}",
                    member.label
                ),
            }
        }
        guard.net.retry_pending();
        for (label, shed) in guard.net.take_drops() {
            eprintln!(
                "[coordinator] {label}'s queue overflowed — dropped {shed} undelivered event(s); \
that session is not consuming its traffic"
            );
        }
        for member in &members {
            let mut events = guard.net.take_outbox(&member.label);
            while !events.is_empty() {
                // CHUNKED, because `/net/deliver` caps its body: posting a
                // whole backlog as one request would 413, be re-queued whole,
                // and 413 again every round — a session's inbound dead for the
                // rest of the run, silently. Splitting means a backlog drains
                // over several rounds instead.
                let rest = events.split_off(deliverable_prefix(&events));
                let body = serde_json::to_string(&events).expect("delivered events serialize");
                match self.post_delivery(&member.url, body).await {
                    DeliveryOutcome::Applied => events = rest,
                    DeliveryOutcome::NotApplied(error) => {
                        // In order: the un-posted tail first, then this batch in
                        // front of it. A reliable channel must not reorder.
                        guard.net.requeue(&member.label, rest);
                        guard.net.requeue(&member.label, events);
                        guard.note_unreachable(&member.label, format!("delivery failed: {error}"));
                        break;
                    }
                    // Ambiguous: the runtime may already have folded it, so
                    // this batch is NOT retried (see `post_delivery`). The tail
                    // goes back, and the loss is loud.
                    DeliveryOutcome::Ambiguous(error) => {
                        guard.net.requeue(&member.label, rest);
                        guard.note_unreachable(
                            &member.label,
                            format!(
                                "delivery outcome unknown, so {} event(s) were NOT retried \
(retrying could double-fold them): {error}",
                                events.len()
                            ),
                        );
                        break;
                    }
                }
            }
        }
    }

    /// The background half of the cadence: while a group's sessions run live
    /// (between tool calls, on wall-clock time), its packets still have to
    /// move. One task per group, ending when the group does.
    fn spawn_pump(&self, group_id: String, group: Arc<tokio::sync::Mutex<Group>>) {
        let server = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PUMP_INTERVAL).await;
                let alive = server
                    .sessions
                    .lock()
                    .expect("mcp registry poisoned")
                    .groups
                    .contains_key(&group_id);
                if !alive {
                    return;
                }
                server.pump_group(&group).await;
            }
        });
    }

    async fn acquire_operation(
        &self,
        target: &SessionTarget,
        context: &RequestContext<RoleServer>,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        acquire_target_operation(target, context.ct.cancelled()).await
    }

    async fn proxy_get(&self, session: &str, path: &str) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.read_url(session));
        ok_text(resolve!(self.get(&url, path).await))
    }

    async fn proxy_mutating_post(
        &self,
        session: &str,
        path: &str,
        body: String,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let target = resolve!(self.target(session));
        let _operation = resolve!(self.acquire_operation(&target, context).await);
        ok_text(resolve!(self.post(&target.url, path, body).await))
    }

    async fn get(&self, url: &str, path: &str) -> Result<String, String> {
        let response = self
            .http
            .get(format!("{url}{path}"))
            .send()
            .await
            .map_err(|error| format!("GET {path} on {url} failed: {error}"))?;
        read_body(path, response).await
    }

    async fn post(&self, url: &str, path: &str, body: String) -> Result<String, String> {
        let response = self
            .http
            .post(format!("{url}{path}"))
            .body(body)
            .send()
            .await
            .map_err(|error| format!("POST {path} on {url} failed: {error}"))?;
        read_body(path, response).await
    }

    /// `POST /net/deliver`, classified by whether the batch can be SAFELY
    /// retried.
    ///
    /// `/net/deliver` applies its events before answering, so a blind retry on
    /// any failure would re-fold a batch the runtime may already have absorbed
    /// — duplicating messages on a channel `Sub.connect` promises is reliable
    /// and ordered, which is worse for a game than losing them. So only a
    /// failure that provably did NOT apply is re-queued: a refused connection
    /// (nothing was sent) or a status the runtime chose (it rejected the body).
    /// An ambiguous failure — a timeout, a truncated response — is reported and
    /// dropped rather than risked twice. A transactional batch id belongs with
    /// the barrier-stepping protocol, not with a v1 that talks to a child
    /// process over loopback.
    async fn post_delivery(&self, url: &str, body: String) -> DeliveryOutcome {
        let response = match self
            .http
            .post(format!("{url}/net/deliver"))
            .body(body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_connect() => {
                return DeliveryOutcome::NotApplied(format!("connection refused: {error}"))
            }
            Err(error) => return DeliveryOutcome::Ambiguous(format!("request failed: {error}")),
        };
        let rejected = response.status().is_client_error() || response.status().is_server_error();
        match read_body("/net/deliver", response).await {
            Ok(_) => DeliveryOutcome::Applied,
            // The runtime answered, and its answer was a refusal: nothing was
            // folded, so the batch is safe to keep.
            Err(message) if rejected => DeliveryOutcome::NotApplied(message),
            Err(message) => DeliveryOutcome::Ambiguous(message),
        }
    }

    async fn post_binary(&self, url: &str, path: &str, body: Vec<u8>) -> Result<String, String> {
        let response = self
            .http
            .post(format!("{url}{path}"))
            .header("content-type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|error| format!("POST {path} on {url} failed: {error}"))?;
        read_body(path, response).await
    }

    async fn post_advance(&self, url: &str, body: String) -> Result<String, AdvanceRequestError> {
        let response = self
            .http
            .post(format!("{url}/time"))
            .body(body)
            .send()
            .await
            .map_err(|error| {
                AdvanceRequestError::Ambiguous(format!("POST /time on {url} failed: {error}"))
            })?;
        let status = response.status();
        let (status, body) = read_bounded_response(
            response,
            MAX_RUNTIME_TEXT_BYTES,
            "generic runtime text response",
            "/time response",
        )
        .await
        .map_err(|error| {
            if status.is_client_error() {
                AdvanceRequestError::Rejected(format!(
                    "POST /time → {status}; reading the rejection failed: {error}"
                ))
            } else {
                AdvanceRequestError::Ambiguous(error)
            }
        })?;
        let body = String::from_utf8(body).map_err(|error| {
            let message = format!("reading the /time response failed: body is not UTF-8: {error}");
            if status.is_client_error() {
                AdvanceRequestError::Rejected(format!("POST /time → {status}: {message}"))
            } else {
                AdvanceRequestError::Ambiguous(message)
            }
        })?;
        if status.is_success() {
            Ok(body)
        } else {
            let message = format!("/time → {status}: {body}");
            if status.is_client_error() {
                Err(AdvanceRequestError::Rejected(message))
            } else {
                Err(AdvanceRequestError::Ambiguous(message))
            }
        }
    }

    async fn state(&self, url: &str) -> Result<Value, String> {
        let body = self.get(url, "/state").await?;
        serde_json::from_str(&body)
            .map_err(|error| format!("GET /state returned invalid JSON: {error}"))
    }

    async fn pin(&self, url: &str, requested_tts: Option<f64>) -> Result<String, String> {
        let tts = match requested_tts {
            Some(tts) => tts,
            None => tts_of(&self.state(url).await?)?,
        };
        self.post(
            url,
            "/time",
            serde_json::json!({ "type": "set", "tts": tts }).to_string(),
        )
        .await
    }

    async fn advance(
        &self,
        target: &SessionTarget,
        dts: f64,
        frames: u32,
        operation_deadline: Instant,
    ) -> Result<Value, String> {
        if let Err(error) = ensure_operation_active(target, operation_deadline) {
            // No advance request was issued, so cancelling here could discard
            // work queued by an external client.
            return Err(error);
        }
        let body = serde_json::json!({
            "type": "advance",
            "dts": dts,
            "frames": frames,
        });
        match self.post_advance(&target.url, body.to_string()).await {
            Ok(_) => {}
            // A 4xx response proves the runtime rejected the command before
            // queuing it. Cancelling could discard direct-client work.
            Err(AdvanceRequestError::Rejected(error)) => return Err(error),
            // A request timeout, malformed success response, or 5xx can still
            // mean the runtime accepted the queue.
            Err(AdvanceRequestError::Ambiguous(error)) => {
                return Err(self.abort_advance(target, error).await)
            }
        }
        // The stall deadline detects a queue that stops moving. The separate
        // operation deadline is absolute: progress never extends it, and an
        // owned stop marks the target closing so polling exits at this safe
        // boundary before stop waits on the same gate.
        let mut deadline = Instant::now() + STEP_STALL_TIMEOUT;
        let mut remaining = u64::MAX;
        loop {
            if let Err(error) = ensure_operation_active(target, operation_deadline) {
                return Err(self.abort_advance(target, error).await);
            }
            let state = match self.state(&target.url).await {
                Ok(state) => state,
                Err(error) => return Err(self.abort_advance(target, error).await),
            };
            let pending = state["pending_steps"].as_u64().unwrap_or(0);
            if pending == 0 {
                return Ok(state);
            }
            if pending < remaining {
                remaining = pending;
                deadline = Instant::now() + STEP_STALL_TIMEOUT;
            } else if Instant::now() >= deadline {
                let error = format!(
                    "the queued steps stopped draining ({pending} still pending, no progress for \
{}s) — the game loop may be stuck or the runtime paused from elsewhere",
                    STEP_STALL_TIMEOUT.as_secs()
                );
                return Err(self.abort_advance(target, error).await);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Wait until the session has delivered every inbound network event it has
    /// already received (`pending_net` back to 0), so the next step folds them
    /// in rather than carrying them into the following round.
    ///
    /// This is a DRAIN, not a transport barrier: a packet still on the wire is
    /// invisible to every process, so quiescence here means "nothing already
    /// received is unprocessed", not "nothing is coming". A pre-v10 runtime
    /// reports no `pending_net` at all, which reads as 0 and makes this a
    /// no-op — the honest degrade, since such a runtime cannot answer.
    async fn settle_net(
        &self,
        target: &SessionTarget,
        operation_deadline: Instant,
    ) -> Result<(), String> {
        let deadline = Instant::now() + STEP_STALL_TIMEOUT;
        loop {
            ensure_operation_active(target, operation_deadline)?;
            let state = self.state(&target.url).await?;
            if state["pending_net"].as_u64().unwrap_or(0) == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "inbound network events stopped draining ({} still queued after {}s) — the \
game loop may be stuck",
                    state["pending_net"],
                    STEP_STALL_TIMEOUT.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn abort_advance(&self, target: &SessionTarget, cause: String) -> String {
        match self
            .post(
                &target.url,
                "/time",
                serde_json::json!({ "type": "cancel" }).to_string(),
            )
            .await
        {
            Ok(_) => format!("{cause}; queued steps were cancelled before releasing the gate"),
            Err(cleanup) => {
                target.quarantined.store(true, Ordering::Release);
                format!(
                    "{cause}; failed to confirm queued-step cancellation: {cleanup}. The exact \
runtime URL is quarantined, so no alias can mutate it. Stop an owned session; for an attached \
runtime, restart both the runtime and this MCP server before reconnecting."
                )
            }
        }
    }

    async fn capture_png(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .http
            .post(format!("{url}/capture"))
            .send()
            .await
            .map_err(|error| format!("POST /capture on {url} failed: {error}"))?;
        let (status, body) = read_bounded_response(
            response,
            MAX_CAPTURE_BYTES,
            "individual capture response",
            "the captured PNG",
        )
        .await?;
        // 503 is "no pixels right now", and the runtime says WHICH reason —
        // headless, a dozing XR session, a capture timeout. Pass its own words
        // through and only append the hint, so an attached Quest is not told to
        // relaunch a process this server does not own.
        if status.as_u16() == 503 {
            return Err(format!(
                "POST /capture -> 503: {}\n\nA session launched with mode \"headless\" has no GL \
context at all — relaunch it with mode \"hidden\" to capture frames.",
                String::from_utf8_lossy(&body)
            ));
        }
        if !status.is_success() {
            return Err(format!(
                "POST /capture → {status}: {}",
                String::from_utf8_lossy(&body)
            ));
        }
        Ok(body)
    }

    async fn execute_node_code(
        &self,
        target: &SessionTarget,
        code: &str,
        timeout: Duration,
        include_final_state: bool,
        context: &RequestContext<RoleServer>,
    ) -> Result<NodeCodeOutput, String> {
        let timeout = timeout.min(OPERATION_TOTAL_TIMEOUT);
        let operation_deadline = Instant::now() + timeout;
        let mut child = spawn_node_child(node_executable())?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Node child did not expose stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Node child did not expose stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Node child did not expose stderr".to_string())?;
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let _stderr_task = drain_into(stderr, stderr_tail.clone());
        let mut stdout = BufReader::new(stdout);

        let baseline_state = match await_code_phase(
            target,
            context,
            operation_deadline,
            timeout,
            "the initial input snapshot",
            self.state(&target.url),
        )
        .await
        {
            Ok(state) => state,
            Err(error) => {
                let reap_error = terminate_node_child(&mut child).await.err();
                if reap_error.is_some() {
                    target.quarantined.store(true, Ordering::Release);
                }
                return Err(append_optional_error(
                    format!("submitted code could not snapshot input before starting: {error}"),
                    reap_error,
                ));
            }
        };
        let mut input_restore = CodeInputRestore::from_state(&baseline_state);
        let mut issued_advance = false;
        let mut output = NodeCodeOutput {
            node_version: String::new(),
            return_value: Value::Null,
            code_error: None,
            failure: None,
            trace: Vec::new(),
            logs: Vec::new(),
            captures: Vec::new(),
            calls_executed: 0,
            final_state: None,
            final_state_summary: None,
            output_budget: CodeOutputBudget::default(),
        };

        let result = await_code_phase(
            target,
            context,
            operation_deadline,
            timeout,
            "Node execution",
            self.drive_node_protocol(
                target,
                code,
                operation_deadline,
                &mut stdin,
                &mut stdout,
                &mut input_restore,
                &mut issued_advance,
                &mut output,
            ),
        )
        .await;

        // The submitted function has reported completion (or the driver
        // stopped waiting). Nothing useful remains in this one-shot child, so
        // terminate it unconditionally instead of trusting submitted code not
        // to replace `process.exit`.
        let child_shutdown_started = Instant::now();
        let child_shutdown = terminate_node_child(&mut child).await;
        let final_state_deadline = operation_deadline
            .checked_add(child_shutdown_started.elapsed())
            .unwrap_or(operation_deadline);
        drop(child);

        let mut failure = result.err().or_else(|| {
            output.code_error.as_ref().map(|error| {
                format!(
                    "submitted Node.js code threw {}: {}",
                    error.name, error.message
                )
            })
        });
        if let Err(error) = child_shutdown {
            // An unconfirmed arbitrary-code process must never race a later
            // mutating operation after this gate is released.
            target.quarantined.store(true, Ordering::Release);
            failure = Some(append_optional_error(
                failure.unwrap_or_else(|| {
                    "submitted Node.js child termination could not be confirmed".into()
                }),
                Some(format!(
                    "{error}. The exact runtime URL is quarantined because submitted code may \
still be alive; stop an owned session, or restart both an attached runtime and this MCP server."
                )),
            ));
        }
        let mut cleanup_done = false;
        if let Some(mut cause) = failure {
            let stderr = tail(&stderr_tail);
            if !stderr.trim().is_empty() {
                cause.push_str("\nNode stderr tail:\n");
                cause.push_str(&stderr);
            }
            cause = self
                .cleanup_submitted_code(target, cause, &input_restore, issued_advance)
                .await;
            output.failure = Some(cause);
            cleanup_done = true;
        }

        match await_code_phase(
            target,
            context,
            final_state_deadline,
            timeout,
            "the final state snapshot",
            self.state(&target.url),
        )
        .await
        {
            Ok(state) => {
                if let Err(error) = retain_final_snapshot(&mut output, state, include_final_state) {
                    let cause = format!(
                        "submitted code completed but its final state exceeded the output budget: \
{error}"
                    );
                    append_code_failure(
                        &mut output,
                        if cleanup_done {
                            cause
                        } else {
                            cleanup_done = true;
                            self.cleanup_submitted_code(
                                target,
                                cause,
                                &input_restore,
                                issued_advance,
                            )
                            .await
                        },
                    );
                }
            }
            Err(error) => {
                let cause = format!("final state snapshot failed: {error}");
                append_code_failure(
                    &mut output,
                    if cleanup_done {
                        cause
                    } else {
                        cleanup_done = true;
                        self.cleanup_submitted_code(
                            target,
                            format!("submitted code completed but its {cause}"),
                            &input_restore,
                            issued_advance,
                        )
                        .await
                    },
                );
            }
        }

        if let Err(error) = node_code_summary(&output) {
            let cause =
                format!("submitted code result could not be encoded within its bounds: {error}");
            let cause = if cleanup_done {
                append_optional_error(
                    output.failure.take().unwrap_or_else(|| cause.clone()),
                    Some(cause),
                )
            } else {
                self.cleanup_submitted_code(target, cause, &input_restore, issued_advance)
                    .await
            };
            return Err(cause);
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive_node_protocol<R, W>(
        &self,
        target: &SessionTarget,
        code: &str,
        operation_deadline: Instant,
        stdin: &mut W,
        stdout: &mut R,
        input_restore: &mut CodeInputRestore,
        issued_advance: &mut bool,
        output: &mut NodeCodeOutput,
    ) -> Result<(), String>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        match read_node_message(stdout).await? {
            Some(NodeMessage::Ready { node_version }) => {
                output.node_version = node_version;
            }
            Some(NodeMessage::Fatal { error }) => return Err(error),
            Some(other) => {
                return Err(format!(
                    "Node child sent {:?} before its ready message",
                    node_message_kind(&other)
                ))
            }
            None => {
                return Err(
                    "Node child exited before becoming ready. Is Node.js 20+ installed?".into(),
                )
            }
        }
        write_node_message(stdin, &serde_json::json!({ "type": "run", "code": code })).await?;

        let mut call_ids = BTreeSet::new();
        let mut retained_log_bytes = 0usize;
        loop {
            let message = read_node_message(stdout).await?.ok_or_else(|| {
                "Node child closed stdout before reporting completion".to_string()
            })?;
            match message {
                NodeMessage::Ready { .. } => {
                    return Err("Node child sent a duplicate ready message".into())
                }
                NodeMessage::Fatal { error } => return Err(error),
                NodeMessage::Log { level, text } => {
                    retained_log_bytes = checked_output_total(
                        retained_log_bytes,
                        text.len(),
                        MAX_NODE_LOG_BYTES,
                        "submitted-code console output",
                    )?;
                    let log = serde_json::json!({ "level": level, "text": text });
                    output.output_budget.retain_json(&log)?;
                    output.logs.push(log);
                }
                NodeMessage::Complete { ok, value, error } => {
                    if ok {
                        output.output_budget.retain_json(&value)?;
                        output.return_value = value;
                    } else {
                        let error = error.unwrap_or(NodeCodeError {
                            name: "Error".into(),
                            message: "Node child reported failure without an error".into(),
                            stack: None,
                        });
                        output.output_budget.retain_json(
                            &serde_json::to_value(&error).map_err(|e| e.to_string())?,
                        )?;
                        output.code_error = Some(error);
                    }
                    return Ok(());
                }
                NodeMessage::Call { id, method, args } => {
                    if id == 0 || !call_ids.insert(id) {
                        return Err(format!("Node child reused invalid call id {id}"));
                    }
                    output.calls_executed += 1;
                    if output.calls_executed > MAX_NODE_CALLS {
                        return Err(format!(
                            "submitted code exceeded the {MAX_NODE_CALLS} SDK-call limit"
                        ));
                    }
                    ensure_operation_active(target, operation_deadline)?;
                    let trace_args = summarize_sdk_args(&method, &args);
                    let started = Instant::now();
                    let result = self
                        .execute_sdk_call(
                            target,
                            &method,
                            &args,
                            operation_deadline,
                            input_restore,
                            issued_advance,
                            &mut output.captures,
                            &mut output.output_budget,
                        )
                        .await;
                    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                    let sequence = output.calls_executed;
                    match result {
                        Ok(call) => {
                            let trace = serde_json::json!({
                                "seq": sequence,
                                "method": method,
                                "args": trace_args,
                                "ok": true,
                                "elapsed_ms": elapsed_ms,
                                "result": call.trace_result,
                            });
                            output.output_budget.retain_json(&trace)?;
                            output.trace.push(trace);
                            write_node_message(
                                stdin,
                                &serde_json::json!({
                                    "type": "result",
                                    "id": id,
                                    "ok": true,
                                    "value": call.value,
                                }),
                            )
                            .await?;
                        }
                        Err(error) => {
                            let error = truncate_chars(error, 16 * 1024);
                            let trace = serde_json::json!({
                                "seq": sequence,
                                "method": method,
                                "args": trace_args,
                                "ok": false,
                                "elapsed_ms": elapsed_ms,
                                "error": error,
                            });
                            output.output_budget.retain_json(&trace)?;
                            output.trace.push(trace);
                            write_node_message(
                                stdin,
                                &serde_json::json!({
                                    "type": "result",
                                    "id": id,
                                    "ok": false,
                                    "error": {
                                        "name": "FunctorSdkError",
                                        "message": error,
                                    },
                                }),
                            )
                            .await?;
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_sdk_call(
        &self,
        target: &SessionTarget,
        method: &str,
        args: &[Value],
        operation_deadline: Instant,
        input_restore: &mut CodeInputRestore,
        issued_advance: &mut bool,
        captures: &mut Vec<Vec<u8>>,
        output_budget: &mut CodeOutputBudget,
    ) -> Result<SdkCallOutput, String> {
        let url = &target.url;
        let text = |value: String| SdkCallOutput {
            trace_result: summarize_value(&Value::String(value.clone())),
            value: Value::String(value),
        };
        let nothing = || SdkCallOutput {
            value: Value::Null,
            trace_result: Value::Null,
        };
        match method {
            "state" => {
                require_arity(method, args, 0)?;
                let value = self.state(url).await?;
                Ok(SdkCallOutput {
                    trace_result: summarize_state(&value),
                    value,
                })
            }
            "scene" | "trace" => {
                require_arity(method, args, 0)?;
                let path = if method == "scene" {
                    "/scene"
                } else {
                    "/trace"
                };
                let body = self.get(url, path).await?;
                let value: Value = serde_json::from_str(&body)
                    .map_err(|error| format!("GET {path} returned invalid JSON: {error}"))?;
                Ok(SdkCallOutput {
                    trace_result: summarize_value(&value),
                    value,
                })
            }
            "capture" => {
                require_arity(method, args, 0)?;
                let png = self.capture_png(url).await?;
                output_budget.retain_capture(png.len())?;
                let content_index = captures.len() + 1;
                let bytes = png.len();
                let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
                captures.push(png);
                let metadata = serde_json::json!({
                    "content_index": content_index,
                    "mime_type": "image/png",
                    "bytes": bytes,
                });
                Ok(SdkCallOutput {
                    value: serde_json::json!({ "$functor_buffer_base64": encoded }),
                    trace_result: metadata,
                })
            }
            "pause" => {
                require_arity_range(method, args, 0, 1)?;
                let tts = args
                    .first()
                    .map(|_| finite_f64_arg(method, args, 0))
                    .transpose()?;
                self.pin(url, tts).await?;
                Ok(nothing())
            }
            "resume" => {
                require_arity(method, args, 0)?;
                self.post(
                    url,
                    "/time",
                    serde_json::json!({ "type": "resume" }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "step" => {
                require_arity_range(method, args, 0, 1)?;
                let dts = args
                    .first()
                    .map(|_| positive_f64_arg(method, args, 0))
                    .transpose()?
                    .unwrap_or(1.0 / 60.0);
                *issued_advance = true;
                let result = self.advance(target, dts, 1, operation_deadline).await;
                // `advance` performs its own cleanup on a returned error. If
                // this future is cancelled mid-await, this assignment never
                // runs and the outer submitted-code cleanup still cancels the
                // possibly accepted queue.
                *issued_advance = false;
                result?;
                Ok(nothing())
            }
            "stepFrames" => {
                require_arity_range(method, args, 1, 2)?;
                let frames = u32_arg(method, args, 0)?;
                if frames > MAX_STEP_FRAMES {
                    return Err(format!(
                        "{method} frames must be between 0 and {MAX_STEP_FRAMES}, got {frames}"
                    ));
                }
                let dts = args
                    .get(1)
                    .map(|_| positive_f64_arg(method, args, 1))
                    .transpose()?
                    .unwrap_or(1.0 / 60.0);
                if frames > 0 {
                    *issued_advance = true;
                    let result = self.advance(target, dts, frames, operation_deadline).await;
                    // See `step`: only a dropped in-flight call leaves this set.
                    *issued_advance = false;
                    result?;
                }
                Ok(nothing())
            }
            "input" => {
                require_arity(method, args, 1)?;
                track_input_command(&args[0], input_restore)?;
                self.post(url, "/input", args[0].to_string()).await?;
                Ok(nothing())
            }
            "key" | "keyDown" | "keyUp" => {
                let expected = if method == "key" { 2 } else { 1 };
                require_arity(method, args, expected)?;
                let key = string_arg(method, args, 0)?;
                validate_key(&key)?;
                let down = match method {
                    "key" => bool_arg(method, args, 1)?,
                    "keyDown" => true,
                    "keyUp" => false,
                    _ => unreachable!(),
                };
                input_restore.touch_key(&key);
                self.post(
                    url,
                    "/input",
                    serde_json::json!({ "type": "key", "key": key, "down": down }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "mouseMove" => {
                require_arity(method, args, 2)?;
                let x = i32_arg(method, args, 0)?;
                let y = i32_arg(method, args, 1)?;
                self.post(
                    url,
                    "/input",
                    serde_json::json!({ "type": "mouse_move", "x": x, "y": y }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "mouseWheel" => {
                require_arity(method, args, 1)?;
                let delta = i32_arg(method, args, 0)?;
                self.post(
                    url,
                    "/input",
                    serde_json::json!({ "type": "mouse_wheel", "delta": delta }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "mouseButton" | "mouseDown" | "mouseUp" => {
                let expected = if method == "mouseButton" { 2 } else { 1 };
                require_arity(method, args, expected)?;
                let button = mouse_button_arg(method, args, 0)?;
                let down = match method {
                    "mouseButton" => bool_arg(method, args, 1)?,
                    "mouseDown" => true,
                    "mouseUp" => false,
                    _ => unreachable!(),
                };
                input_restore.touch_mouse_button(&button);
                self.post(
                    url,
                    "/input",
                    serde_json::json!({
                        "type": "mouse_button",
                        "button": button,
                        "down": down,
                    })
                    .to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "xr" => {
                require_arity(method, args, 1)?;
                let sample = args[0]
                    .as_object()
                    .ok_or_else(|| "xr sample must be an object".to_string())?;
                let mut command = sample.clone();
                command.insert("type".into(), Value::String("xr".into()));
                self.post(url, "/input", Value::Object(command).to_string())
                    .await?;
                Ok(nothing())
            }
            "xrClear" => {
                require_arity(method, args, 0)?;
                self.post(
                    url,
                    "/input",
                    serde_json::json!({ "type": "xr_clear" }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "gamepad" => {
                require_arity(method, args, 1)?;
                let sample = args[0]
                    .as_object()
                    .ok_or_else(|| "gamepad sample must be an object".to_string())?;
                let mut command = sample.clone();
                command.insert("type".into(), Value::String("gamepad".into()));
                self.post(url, "/input", Value::Object(command).to_string())
                    .await?;
                Ok(nothing())
            }
            "gamepadClear" => {
                require_arity(method, args, 0)?;
                self.post(
                    url,
                    "/input",
                    serde_json::json!({ "type": "gamepad_clear" }).to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "uiClick" => {
                require_arity(method, args, 1)?;
                let slot = u32_arg(method, args, 0)?;
                self.post(
                    url,
                    "/input",
                    serde_json::json!({
                        "type": "ui_event",
                        "slot": slot,
                        "kind": "Clicked",
                    })
                    .to_string(),
                )
                .await?;
                Ok(nothing())
            }
            "rewind" => {
                require_arity(method, args, 1)?;
                let frame = u64_arg(method, args, 0)?;
                Ok(text(
                    self.post(
                        url,
                        "/rewind",
                        serde_json::json!({ "frame": frame }).to_string(),
                    )
                    .await?,
                ))
            }
            "reloadSource" => {
                require_arity(method, args, 1)?;
                Ok(text(
                    self.post(url, "/reload-source", string_arg(method, args, 0)?)
                        .await?,
                ))
            }
            "reloadProject" | "loadProject" => {
                require_arity(method, args, 1)?;
                let path = if method == "reloadProject" {
                    "/reload-project"
                } else {
                    "/load-project"
                };
                Ok(text(self.post(url, path, args[0].to_string()).await?))
            }
            "reloadAsset" => {
                require_arity(method, args, 2)?;
                let path = string_arg(method, args, 0)?;
                let path_bytes = path.as_bytes();
                let path_len = u32::try_from(path_bytes.len())
                    .map_err(|_| "asset path is too long".to_string())?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(string_arg(method, args, 1)?)
                    .map_err(|error| format!("reloadAsset bytes are not valid base64: {error}"))?;
                if bytes.len() > MAX_CODE_ASSET_BYTES {
                    return Err(output_limit_error(
                        "submitted-code asset",
                        bytes.len(),
                        MAX_CODE_ASSET_BYTES,
                    ));
                }
                let mut body = Vec::with_capacity(4 + path_bytes.len() + bytes.len());
                body.extend_from_slice(&path_len.to_be_bytes());
                body.extend_from_slice(path_bytes);
                body.extend_from_slice(&bytes);
                Ok(text(self.post_binary(url, "/reload-asset", body).await?))
            }
            "syncAssets" => {
                require_arity(method, args, 1)?;
                Ok(text(
                    self.post(url, "/sync-assets", args[0].to_string()).await?,
                ))
            }
            other => Err(format!(
                "unknown game SDK method {other:?}; use the injected Functor SDK surface"
            )),
        }
    }

    async fn code_failure(
        &self,
        target: &SessionTarget,
        cause: String,
        input_restore: &CodeInputRestore,
    ) -> String {
        if input_restore.is_empty() {
            return cause;
        }
        let current = match self.state(&target.url).await {
            Ok(state) => CodeInputRestore::from_state(&state),
            Err(error) => {
                target.quarantined.store(true, Ordering::Release);
                return format!(
                    "{cause}; failed to snapshot current input before restoration: {error}. The \
exact runtime URL is quarantined, so no alias can mutate it. Stop an owned session; for an attached \
runtime, restart both the runtime and this MCP server before reconnecting."
                );
            }
        };
        let mut failures = Vec::new();
        for key in &input_restore.touched_keys {
            let baseline_down = input_restore.baseline_keys.contains(key);
            if current.baseline_keys.contains(key) == baseline_down {
                continue;
            }
            if let Err(error) = self
                .post(
                    &target.url,
                    "/input",
                    serde_json::json!({
                        "type": "key",
                        "key": key,
                        "down": baseline_down,
                    })
                    .to_string(),
                )
                .await
            {
                failures.push(format!("key {key:?}: {error}"));
            }
        }
        for button in &input_restore.touched_mouse_buttons {
            let baseline_down = input_restore.baseline_mouse_buttons.contains(button);
            if current.baseline_mouse_buttons.contains(button) == baseline_down {
                continue;
            }
            if let Err(error) = self
                .post(
                    &target.url,
                    "/input",
                    serde_json::json!({
                        "type": "mouse_button",
                        "button": button,
                        "down": baseline_down,
                    })
                    .to_string(),
                )
                .await
            {
                failures.push(format!("mouse button {button:?}: {error}"));
            }
        }
        if failures.is_empty() {
            format!("{cause}; code-touched key and mouse-button levels were restored")
        } else {
            target.quarantined.store(true, Ordering::Release);
            format!(
                "{cause}; failed to confirm code-touched input restoration ({}). The exact runtime URL \
is quarantined, so no alias can mutate it. Stop an owned session; for an attached runtime, restart \
both the runtime and this MCP server before reconnecting.",
                failures.join("; ")
            )
        }
    }

    async fn cleanup_submitted_code(
        &self,
        target: &SessionTarget,
        cause: String,
        input_restore: &CodeInputRestore,
        issued_advance: bool,
    ) -> String {
        self.cleanup_submitted_code_with_timeout(
            target,
            cause,
            input_restore,
            issued_advance,
            CODE_CLEANUP_TIMEOUT,
        )
        .await
    }

    async fn cleanup_submitted_code_with_timeout(
        &self,
        target: &SessionTarget,
        cause: String,
        input_restore: &CodeInputRestore,
        issued_advance: bool,
        cleanup_timeout: Duration,
    ) -> String {
        let timeout_cause = cause.clone();
        let cleanup = async {
            let mut cause = cause;
            if issued_advance {
                cause = self.abort_advance(target, cause).await;
            }
            self.code_failure(target, cause, input_restore).await
        };
        match tokio::time::timeout(cleanup_timeout, cleanup).await {
            Ok(cause) => cause,
            Err(_) => {
                target.quarantined.store(true, Ordering::Release);
                format!(
                    "{timeout_cause}; submitted-code safety cleanup exceeded its {}ms aggregate \
deadline. Queued clock work or touched input may be ambiguous. The exact runtime URL is \
quarantined, so no alias can mutate it. Stop an owned session; for an attached runtime, restart \
both the runtime and this MCP server before reconnecting.",
                    cleanup_timeout.as_millis()
                )
            }
        }
    }

    async fn discover(&self, url: &str) -> Result<Value, String> {
        let body = self.get(url, "/").await?;
        let discovery: Value = serde_json::from_str(&body)
            .map_err(|error| format!("{url}/ did not return JSON: {error}"))?;
        if discovery["service"] != DEBUG_PROTOCOL_SERVICE {
            return Err(format!(
                "{url} is not a Functor debug runtime (its / reports {})",
                discovery["service"]
            ));
        }
        // Below v8 the guarantees these tools advertise silently stop holding:
        // a pre-v3 runtime ignores a batched `frames` and reports no
        // `pending_steps` (so `step` would claim a 10-frame batch landed after
        // running one), and a pre-v4 one sends Debug text under `model`
        // instead of structured JSON. A pre-v8 runtime cannot cancel an
        // accepted step queue on an abort. Refuse rather than mislead — this
        // matters for a device APK, which versions independently of the CLI.
        let version = discovery["protocol_version"].as_u64().unwrap_or(0);
        if version < REQUIRED_PROTOCOL_VERSION {
            return Err(format!(
                "{url} speaks debug protocol v{version}, but these tools need \
v{REQUIRED_PROTOCOL_VERSION} (structured model state plus safe queued-step cleanup). \
Rebuild that runtime from this version of Functor."
            ));
        }
        Ok(discovery)
    }

    /// Poll a freshly spawned runtime until its discovery endpoint answers,
    /// failing fast if the child exits first.
    async fn await_runtime(&self, url: &str, child: &mut Child) -> Result<Value, String> {
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "the runtime exited before serving {url} ({status})"
                ));
            }
            if let Ok(discovery) = self.discover(url).await {
                return Ok(discovery);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the runtime did not serve {url} within {}s",
                    LAUNCH_TIMEOUT.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// The roles a project's `functor.json` declares, sorted — `manifest_entries`'
/// pairs with their files dropped. `entries` is a map, so its declaration order
/// is not part of what the manifest means; a group's launch order comes from
/// [`group_roles`] instead.
fn declared_entries(manifest: &str) -> Result<Vec<String>, String> {
    // A single-`entry` manifest parses as ONE NAMELESS role, which is exactly
    // the case to refuse: a "group" of a single-entry project is one game. So
    // is a manifest `manifest_entries` cannot reason about at all.
    let roles: Vec<String> = manifest_entries(manifest)
        .unwrap_or_default()
        .into_iter()
        .map(|(role, _)| role)
        .filter(|role| !role.is_empty())
        .collect();
    if roles.is_empty() {
        return Err(
            "this project declares no `entries`, so it has no roles to group — a multiplayer \
project's functor.json looks like {\"entries\": {\"client\": …, \"server\": …}}"
                .to_string(),
        );
    }
    Ok(roles)
}

/// The roles to launch, in order. Explicit `roles` wins verbatim (repeats are
/// how a group gets two clients); the default is one session per declared
/// entry, with `server` first because a client's `Connect` has nothing to
/// resolve until its authority exists.
fn group_roles(requested: Option<&[String]>, declared: &[String]) -> Result<Vec<String>, String> {
    if let Some(roles) = requested {
        if roles.is_empty() {
            return Err(format!(
                "roles must name at least one entry; this project declares {}",
                declared.join(", ")
            ));
        }
        if let Some(unknown) = roles.iter().find(|role| !declared.contains(role)) {
            return Err(format!(
                "unknown role {unknown:?}: this project declares {}",
                declared.join(", ")
            ));
        }
        return Ok(roles.to_vec());
    }
    let mut roles: Vec<String> = declared.to_vec();
    if let Some(index) = roles.iter().position(|role| role == "server") {
        let server = roles.remove(index);
        roles.insert(0, server);
    }
    Ok(roles)
}

/// Wire-log labels for a role list: the plain role name when it appears once,
/// and `client1` / `client2` when it repeats.
fn group_labels(roles: &[String]) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for role in roles {
        *seen.entry(role.as_str()).or_insert(0) += 1;
    }
    let mut used: BTreeMap<&str, usize> = BTreeMap::new();
    roles
        .iter()
        .map(|role| {
            if seen[role.as_str()] == 1 {
                role.clone()
            } else {
                let index = used.entry(role.as_str()).or_insert(0);
                *index += 1;
                format!("{role}{index}")
            }
        })
        .collect()
}

/// `wire_log`'s `link` + `direction`, resolved against the group's labels so a
/// typo is an error rather than an empty result a caller would read as "no
/// traffic".
#[derive(Debug)]
enum WireFilter {
    All,
    /// Rows between these two labels, in either direction.
    Pair(String, String),
    /// Rows touching one label, optionally narrowed to one direction.
    One {
        label: String,
        sent: Option<bool>,
    },
}

impl WireFilter {
    fn parse(link: Option<&str>, direction: Option<&str>, known: &[String]) -> Result<Self, String> {
        let sent = match direction {
            None => None,
            Some("sent") => Some(true),
            Some("received") => Some(false),
            Some(other) => {
                return Err(format!(
                    "unknown direction {other:?}: expected \"sent\" or \"received\""
                ))
            }
        };
        let check = |label: &str| -> Result<String, String> {
            if known.iter().any(|k| k == label) {
                Ok(label.to_string())
            } else {
                Err(format!(
                    "unknown session label {label:?}: this group has {}",
                    known.join(", ")
                ))
            }
        };
        match link {
            None if sent.is_some() => Err(
                "direction needs a single-label `link` to be relative to — \"sent\" from what?"
                    .to_string(),
            ),
            None => Ok(WireFilter::All),
            Some(link) => match link.split_once(':') {
                Some((a, b)) if sent.is_some() => {
                    let _ = (check(a.trim())?, check(b.trim())?);
                    Err(format!(
                        "direction does not apply to the pair {link:?} — it already names both \
ends; pass a single label instead"
                    ))
                }
                Some((a, b)) => Ok(WireFilter::Pair(check(a.trim())?, check(b.trim())?)),
                None => Ok(WireFilter::One {
                    label: check(link.trim())?,
                    sent,
                }),
            },
        }
    }

    fn matches(&self, packet: &crate::commands::mcp_net::Packet) -> bool {
        match self {
            WireFilter::All => true,
            WireFilter::Pair(a, b) => {
                (packet.from == *a && packet.to == *b) || (packet.from == *b && packet.to == *a)
            }
            WireFilter::One { label, sent } => match sent {
                Some(true) => packet.from == *label,
                Some(false) => packet.to == *label,
                None => packet.from == *label || packet.to == *label,
            },
        }
    }
}

/// How many leading events fit in one `/net/deliver` body. At least one always
/// does: a single event over the budget is posted alone and lets the runtime's
/// own 413 be the (loud) answer, rather than being silently stuck forever.
fn deliverable_prefix(events: &[functor_runtime_common::net::DeliveredEvent]) -> usize {
    let mut bytes = 2; // the enclosing `[]`
    for (index, event) in events.iter().enumerate() {
        bytes += serde_json::to_string(event).map_or(0, |json| json.len() + 1);
        if bytes > MAX_DELIVERY_BYTES {
            return index.max(1);
        }
    }
    events.len()
}

fn node_executable() -> String {
    std::env::var("FUNCTOR_NODE").unwrap_or_else(|_| "node".into())
}

fn spawn_node_child(program: impl AsRef<Path>) -> Result<Child, String> {
    let program = program.as_ref();
    let mut command = Command::new(program);
    command
        .arg("--input-type=module")
        .arg("--eval")
        .arg(NODE_RUNNER_SOURCE)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!(
                "run_game_code_unsafe requires Node.js 20 or newer on PATH, but {:?} was not found. \
Install Node.js, set FUNCTOR_NODE to its executable, and restart `functor mcp`.",
                program
            )
        } else {
            format!(
                "could not start Node.js executable {:?} for run_game_code_unsafe: {error}",
                program
            )
        }
    })
}

async fn terminate_node_child(child: &mut Child) -> Result<(), String> {
    terminate_child(child, "Node child", NODE_CHILD_SHUTDOWN_TIMEOUT).await
}

async fn terminate_child(child: &mut Child, label: &str, timeout: Duration) -> Result<(), String> {
    let kill_error = child.start_kill().err();
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(append_optional_error(
            format!("waiting for the {label} failed: {error}"),
            kill_error.map(|error| format!("requesting termination also failed: {error}")),
        )),
        Err(_) => {
            let message = format!(
                "{label} did not exit within {}ms after termination; kill-on-drop remains armed",
                timeout.as_millis()
            );
            Err(append_optional_error(
                message,
                kill_error.map(|error| format!("requesting termination failed: {error}")),
            ))
        }
    }
}

async fn await_code_phase<T>(
    target: &SessionTarget,
    context: &RequestContext<RoleServer>,
    operation_deadline: Instant,
    timeout: Duration,
    phase: &str,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::pin!(future);
    let closing = wait_until_target_inactive(target);
    tokio::pin!(closing);
    let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(operation_deadline));
    tokio::pin!(deadline);
    tokio::select! {
        result = &mut future => result,
        _ = context.ct.cancelled() => Err(format!(
            "submitted-code MCP request was cancelled during {phase}"
        )),
        reason = &mut closing => Err(format!("{reason} during {phase}")),
        _ = &mut deadline => Err(format!(
            "submitted code exceeded its {}ms wall-clock timeout during {phase}",
            timeout.as_millis()
        )),
    }
}

async fn wait_until_target_inactive(target: &SessionTarget) -> String {
    loop {
        if target.quarantined.load(Ordering::Acquire) {
            return format!(
                "submitted-code operation against {} became quarantined",
                target.url
            );
        }
        if target.closing.load(Ordering::Acquire) {
            return format!(
                "submitted-code operation against {} was interrupted because the session is stopping",
                target.url
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn append_optional_error(mut cause: String, error: Option<String>) -> String {
    if let Some(error) = error {
        cause.push_str("; ");
        cause.push_str(&error);
    }
    cause
}

fn append_code_failure(output: &mut NodeCodeOutput, cause: String) {
    match &mut output.failure {
        Some(failure) => {
            failure.push_str("; ");
            failure.push_str(&cause);
        }
        None => output.failure = Some(cause),
    }
}

async fn read_node_message<R>(reader: &mut R) -> Result<Option<NodeMessage>, String>
where
    R: AsyncBufRead + Unpin,
{
    let line = match read_bounded_line(
        reader,
        MAX_NODE_PROTOCOL_LINE_BYTES,
        "Node child protocol line",
    )
    .await?
    {
        Some(line) => line,
        None => return Ok(None),
    };
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(|error| format!("Node child sent invalid protocol JSON: {error}"))
}

async fn read_bounded_line<R>(
    reader: &mut R,
    limit: usize,
    cap_name: &str,
) -> Result<Option<Vec<u8>>, String>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| format!("reading {cap_name} failed: {error}"))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(format!("{cap_name} ended without a newline"))
            };
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                extend_bounded(&mut line, &available[..index], limit, cap_name)?;
                reader.consume(index + 1);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(line));
            }
            None => {
                let consumed = available.len();
                extend_bounded(&mut line, available, limit, cap_name)?;
                reader.consume(consumed);
            }
        }
    }
}

async fn write_node_message<W>(writer: &mut W, value: &Value) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let line = serialize_json_bounded(value, MAX_NODE_PROTOCOL_LINE_BYTES, "Node parent message")?;
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|error| format!("writing to Node child failed: {error}"))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| format!("writing to Node child failed: {error}"))?;
    writer
        .flush()
        .await
        .map_err(|error| format!("flushing Node child stdin failed: {error}"))
}

fn node_message_kind(message: &NodeMessage) -> &'static str {
    match message {
        NodeMessage::Ready { .. } => "ready",
        NodeMessage::Call { .. } => "call",
        NodeMessage::Log { .. } => "log",
        NodeMessage::Complete { .. } => "complete",
        NodeMessage::Fatal { .. } => "fatal",
    }
}

fn require_arity(method: &str, args: &[Value], expected: usize) -> Result<(), String> {
    require_arity_range(method, args, expected, expected)
}

fn require_arity_range(
    method: &str,
    args: &[Value],
    minimum: usize,
    maximum: usize,
) -> Result<(), String> {
    if (minimum..=maximum).contains(&args.len()) {
        Ok(())
    } else if minimum == maximum {
        Err(format!(
            "{method} expects {minimum} argument(s), got {}",
            args.len()
        ))
    } else {
        Err(format!(
            "{method} expects between {minimum} and {maximum} arguments, got {}",
            args.len()
        ))
    }
}

fn string_arg(method: &str, args: &[Value], index: usize) -> Result<String, String> {
    args.get(index)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{method} argument {} must be a string", index + 1))
}

fn bool_arg(method: &str, args: &[Value], index: usize) -> Result<bool, String> {
    args.get(index)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{method} argument {} must be a boolean", index + 1))
}

fn finite_f64_arg(method: &str, args: &[Value], index: usize) -> Result<f64, String> {
    args.get(index)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{method} argument {} must be a finite number", index + 1))
}

fn positive_f64_arg(method: &str, args: &[Value], index: usize) -> Result<f64, String> {
    let value = finite_f64_arg(method, args, index)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "{method} argument {} must be positive, got {value}",
            index + 1
        ))
    }
}

fn u64_arg(method: &str, args: &[Value], index: usize) -> Result<u64, String> {
    args.get(index).and_then(Value::as_u64).ok_or_else(|| {
        format!(
            "{method} argument {} must be a non-negative integer",
            index + 1
        )
    })
}

fn u32_arg(method: &str, args: &[Value], index: usize) -> Result<u32, String> {
    u32::try_from(u64_arg(method, args, index)?)
        .map_err(|_| format!("{method} argument {} exceeds u32", index + 1))
}

fn i32_arg(method: &str, args: &[Value], index: usize) -> Result<i32, String> {
    let value = args
        .get(index)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{method} argument {} must be an integer", index + 1))?;
    i32::try_from(value).map_err(|_| format!("{method} argument {} exceeds i32", index + 1))
}

fn validate_key(key: &str) -> Result<(), String> {
    Key::from_name(key).map(|_| ()).ok_or_else(|| {
        format!(
            "unknown key {key:?}; expected A-Z, Up/Down/Left/Right, Space, Enter, Escape, or 0-9"
        )
    })
}

fn mouse_button_arg(method: &str, args: &[Value], index: usize) -> Result<String, String> {
    let button = string_arg(method, args, index)?;
    match button.to_ascii_lowercase().as_str() {
        "left" | "right" | "middle" => Ok(button.to_ascii_lowercase()),
        _ => Err(format!(
            "{method} argument {} must be left, right, or middle",
            index + 1
        )),
    }
}

fn track_input_command(
    command: &Value,
    input_restore: &mut CodeInputRestore,
) -> Result<(), String> {
    let kind = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "input command must be an object with a string `type`".to_string())?;
    match kind {
        "key" => {
            let key = command
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| "key input requires a string `key`".to_string())?;
            validate_key(key)?;
            if !command.get("down").is_some_and(Value::is_boolean) {
                return Err("key input requires a boolean `down`".into());
            }
            input_restore.touch_key(key);
        }
        "mouse_button" => {
            let button = command
                .get("button")
                .and_then(Value::as_str)
                .ok_or_else(|| "mouse_button input requires a string `button`".to_string())?;
            match button.to_ascii_lowercase().as_str() {
                "left" | "right" | "middle" => {}
                _ => {
                    return Err("mouse_button input `button` must be left, right, or middle".into())
                }
            }
            if !command.get("down").is_some_and(Value::is_boolean) {
                return Err("mouse_button input requires a boolean `down`".into());
            }
            input_restore.touch_mouse_button(button);
        }
        _ => {}
    }
    Ok(())
}

/// Validate the `dts` / `frames` a step tool was given, defaulted. One copy for
/// `step` and `step_all` so the two cannot drift on what they accept.
fn step_args(tool: &str, dts: Option<f64>, frames: Option<u32>) -> Result<(f64, u32), String> {
    let frames = frames.unwrap_or(1);
    if !(1..=MAX_STEP_FRAMES).contains(&frames) {
        return Err(format!(
            "{tool} frames must be between 1 and {MAX_STEP_FRAMES}; got {frames}"
        ));
    }
    let dts = dts.unwrap_or(0.016);
    if !dts.is_finite() || dts <= 0.0 {
        return Err(format!(
            "{tool} dts must be a finite positive number; got {dts}"
        ));
    }
    Ok((dts, frames))
}

fn summarize_state(state: &Value) -> Value {
    serde_json::json!({
        "frame": state.get("frame").cloned().unwrap_or(Value::Null),
        "tts": state.get("tts").cloned().unwrap_or(Value::Null),
        "pending_steps": state.get("pending_steps").cloned().unwrap_or(Value::Null),
        // The networked facts (protocol v10): `frame` alone cannot say whether
        // anything landed on a session a peer can write to.
        "model_revision": state.get("model_revision").cloned().unwrap_or(Value::Null),
        "pending_net": state.get("pending_net").cloned().unwrap_or(Value::Null),
        "held_keys": state.pointer("/input/held_keys").cloned().unwrap_or(Value::Null),
        "model_bytes": json_encoded_len(state.get("model").unwrap_or(&Value::Null)).unwrap_or(0),
    })
}

fn compact_final_state_summary(state: &Value) -> Result<Value, String> {
    let model_json_bytes = json_encoded_len(state.get("model").unwrap_or(&Value::Null))?;
    let mut held_mouse_buttons: Vec<&str> = state
        .pointer("/input/mouse/buttons")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|buttons| buttons.iter())
        .filter_map(|(button, held)| {
            held.as_bool()
                .is_some_and(|held| held)
                .then_some(button.as_str())
        })
        .collect();
    held_mouse_buttons.sort_unstable();

    let mut summary = serde_json::json!({
        "frame": state.get("frame").cloned().unwrap_or(Value::Null),
        "tts": state.get("tts").cloned().unwrap_or(Value::Null),
        "pending_steps": state.get("pending_steps").cloned().unwrap_or(Value::Null),
        "held_keys": state.pointer("/input/held_keys").cloned().unwrap_or(Value::Null),
        "held_mouse_buttons": held_mouse_buttons,
        "model_json_bytes": model_json_bytes,
    });
    // Some future/attached runtimes may expose this clock fact directly.
    // Current protocol versions do not, so never infer it from pending work.
    if let Some(paused) = state.get("paused").filter(|value| value.is_boolean()) {
        summary
            .as_object_mut()
            .expect("compact state summary is an object")
            .insert("paused".into(), paused.clone());
    }
    Ok(summary)
}

fn retain_final_snapshot(
    output: &mut NodeCodeOutput,
    state: Value,
    include_final_state: bool,
) -> Result<(), String> {
    if include_final_state {
        output.output_budget.retain_json(&state)?;
        output.final_state = Some(state);
    } else {
        let summary = compact_final_state_summary(&state)?;
        output.output_budget.retain_json(&summary)?;
        output.final_state_summary = Some(summary);
    }
    Ok(())
}

fn summarize_value(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
        Value::Array(items) => serde_json::json!({
            "type": "array",
            "length": items.len(),
            "json_bytes": json_encoded_len(value).unwrap_or(usize::MAX),
        }),
        Value::Object(fields) => serde_json::json!({
            "type": "object",
            "keys": fields.len(),
            "json_bytes": json_encoded_len(value).unwrap_or(usize::MAX),
        }),
    }
}

fn summarize_sdk_args(method: &str, args: &[Value]) -> Value {
    match method {
        "reloadSource" => serde_json::json!({
            "source_bytes": args.first().and_then(Value::as_str).map(str::len),
        }),
        "reloadProject" | "loadProject" => serde_json::json!({
            "project_json_bytes": args.first().and_then(|value| json_encoded_len(value).ok()),
        }),
        "reloadAsset" => serde_json::json!({
            "path": args.first().cloned().unwrap_or(Value::Null),
            "base64_bytes": args.get(1).and_then(Value::as_str).map(str::len),
        }),
        _ => Value::Array(args.to_vec()),
    }
}

fn truncate_chars(mut text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut boundary = limit;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    text.push('…');
    text
}

fn ensure_operation_active(
    target: &SessionTarget,
    operation_deadline: Instant,
) -> Result<(), String> {
    if target.quarantined.load(Ordering::Acquire) {
        return Err(format!(
            "runtime at {} is quarantined because abort cleanup could not be confirmed",
            target.url
        ));
    }
    if target.closing.load(Ordering::Acquire) {
        return Err(format!(
            "session at {} is stopping; operation ended at a safe boundary",
            target.url
        ));
    }
    if Instant::now() >= operation_deadline {
        return Err(format!(
            "operation exceeded its {}s wall-clock deadline; split long step batches or code runs",
            OPERATION_TOTAL_TIMEOUT.as_secs()
        ));
    }
    Ok(())
}

async fn acquire_target_operation(
    target: &SessionTarget,
    cancelled: impl Future<Output = ()>,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    if target.quarantined.load(Ordering::Acquire) {
        return Err(format!(
            "runtime at {} is quarantined because abort cleanup could not be confirmed",
            target.url
        ));
    }
    if target.closing.load(Ordering::Acquire) {
        return Err(format!(
            "session at {} is stopping; no new operation can start",
            target.url
        ));
    }
    let gate = target.operation_gate.clone();
    tokio::pin!(cancelled);
    let guard = tokio::select! {
        biased;
        _ = &mut cancelled => {
            return Err(format!(
                "request for session at {} was cancelled before its operation began",
                target.url
            ));
        }
        guard = gate.lock_owned() => guard,
    };
    // Stop can mark this cloned target while it waits. Holding the gate here
    // proves no runtime operation is active, but it must still reject rather
    // than issue I/O behind the stop boundary.
    if target.quarantined.load(Ordering::Acquire) {
        return Err(format!(
            "runtime at {} became quarantined while the operation waited; queued operation did not run",
            target.url
        ));
    }
    if target.closing.load(Ordering::Acquire) {
        return Err(format!(
            "session at {} is stopping; queued operation did not run",
            target.url
        ));
    }
    Ok(guard)
}

#[tool_handler(
    name = "functor",
    instructions = "Drive Functor games over their debug runtime. Launch or attach to a game \
(launch_game / connect_game). For trusted local multi-step automation, prefer \
`run_game_code_unsafe`: it runs a Playwright-style JavaScript function against an injected \
`game` SDK in a Node.js child process, returning its value, SDK-call trace, logs, and captures. \
It is explicitly RCE-equivalent local code, not a security sandbox. For one-off operations or \
clients without Node.js, observe with get_state (read model), get_scene, get_trace, or \
capture_frame, and drive with pause, send_input, step, resume, rewind, or reload_source. The \
lower-level deterministic \
loop is pause → send_input → step → get_state: while the \
clock is pinned nothing advances on its own, and injected input is level state that holds \
across steps. Mutating calls on one session share an async operation gate: overlaps WAIT \
and then run without interleaving (waiter order is unspecified); exact normalized URL \
aliases share the gate, while different URLs remain independent. \
language_guide teaches the LANGUAGE (Functor Lang is not F#/OCaml — read it \
before writing any .fun) and api_reference searches the engine's prelude API; neither needs \
a session, so both answer before anything is launched. A game can also be AUTHORED \
here with no filesystem of your own: launch_game with inline `files` runs source that has \
no home, reload_source edits it live, and save_project writes what the runtime is actually \
running to a directory. init_game scaffolds a starter project on disk instead."
)]
impl ServerHandler for FunctorMcp {}

impl Default for FunctorMcp {
    fn default() -> Self {
        Self::new()
    }
}

/// One API-reference item that matched a search, with the rank that decides
/// how prominently it is reported.
#[derive(Debug)]
struct ApiHit<'a> {
    item: &'a ApiItem,
    /// 0 = the item's own name, 1 = its qualified path, 2 = its signature,
    /// 3 = its prose.
    rank: u8,
}

/// The orienting hint every API teaching error ends with.
fn modules_hint(reference: &ApiReference) -> String {
    format!(
        "The prelude modules are {}.",
        reference
            .modules
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Rank an item against an already-lowercased needle, or `None` if it misses.
fn rank_api_item(needle: &str, item: &ApiItem) -> Option<u8> {
    if item.name.eq_ignore_ascii_case(needle) || item.qualified_name.eq_ignore_ascii_case(needle) {
        return Some(0);
    }
    if item.qualified_name.to_lowercase().contains(needle) {
        return Some(1);
    }
    if item.declaration.to_lowercase().contains(needle) {
        return Some(2);
    }
    item.docs
        .as_deref()
        .is_some_and(|docs| docs.to_lowercase().contains(needle))
        .then_some(3)
}

/// Search the reference, best matches first. `Err` is a teaching message: an
/// unknown module, or a search with nothing to go on.
fn search_api<'a>(
    reference: &'a ApiReference,
    query: &str,
    module: Option<&str>,
) -> Result<Vec<ApiHit<'a>>, String> {
    let wanted = module.map(str::trim);
    let modules = match wanted {
        // A blank module is a client bug, not "no filter": answering it from
        // the whole prelude would silently widen the scope that was asked for.
        Some(wanted) => vec![reference
            .modules
            .iter()
            .find(|module| module.name.eq_ignore_ascii_case(wanted))
            .ok_or_else(|| format!("unknown module {wanted:?}. {}", modules_hint(reference)))?],
        None => reference.modules.iter().collect(),
    };
    let needle = query.trim().to_lowercase();
    if needle.is_empty() && wanted.is_none() {
        return Err(format!(
            "give a query to search for, or a module to list. {}",
            modules_hint(reference)
        ));
    }
    let mut hits: Vec<ApiHit> = modules
        .into_iter()
        .flat_map(|module| module.items.iter())
        .filter_map(|item| {
            let rank = if needle.is_empty() {
                Some(1)
            } else {
                rank_api_item(&needle, item)
            };
            rank.map(|rank| ApiHit { item, rank })
        })
        .collect();
    // A stable sort keeps the prelude's own order within a rank, which is how
    // the module-browse listing reads best.
    hits.sort_by_key(|hit| hit.rank);
    Ok(hits)
}

/// Render matches as compact text: qualified name at column 0, then the
/// signature and the prose indented under it — a record type's signature spans
/// several lines, so every line is indented or the shape would be ambiguous.
fn render_api_hits(hits: &[ApiHit], limit: Option<usize>) -> String {
    let shown = limit.unwrap_or(hits.len());
    let mut out = String::new();
    for hit in hits.iter().take(shown) {
        out.push_str(&hit.item.qualified_name);
        out.push('\n');
        let prose = hit.item.docs.as_deref().unwrap_or("");
        for line in hit.item.declaration.lines().chain(prose.lines()) {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    if hits.len() > shown {
        out.push_str(&format!(
            "({shown} of {} matches shown — narrow with a more specific query)\n",
            hits.len()
        ));
    }
    out
}

/// One addressable part of the language guide: a markdown heading plus the
/// text under it, stopping at the NEXT heading of any level. Sections are
/// derived from the skill's own headings, never hand-listed, so a restructured
/// skill re-sections itself.
///
/// A subsection is therefore addressed in its own right rather than also
/// living inside its parent (the prelude section alone is tens of kilobytes —
/// carrying its subsections would make one call most of the guide), and a
/// parent that has any says so in a `Continues in:` line.
#[derive(Debug)]
struct GuideSection<'a> {
    /// 2 for `##`, 3 for `###`.
    level: usize,
    slug: String,
    /// The heading line and its body, verbatim.
    text: &'a str,
}

/// Drop the skill's YAML front matter, which is loader metadata rather than
/// language documentation. CRLF is handled because a Windows checkout of a
/// `.md` is not pinned to LF, and leaking the metadata is silent.
fn strip_front_matter(guide: &str) -> &str {
    let Some(rest) = guide
        .strip_prefix("---\n")
        .or_else(|| guide.strip_prefix("---\r\n"))
    else {
        return guide;
    };
    ["\n---\n", "\n---\r\n"]
        .iter()
        .find_map(|delimiter| Some(&rest[rest.find(delimiter)? + delimiter.len()..]))
        .unwrap_or(guide)
}

/// The heading level of a line, for the levels this tool addresses.
fn heading_level(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    (matches!(level, 2 | 3) && line.as_bytes().get(level) == Some(&b' ')).then_some(level)
}

/// A heading's stable name: lowercase, non-alphanumerics collapsed to `-`.
fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Split the guide into the text before its first heading and its sections,
/// in document order.
fn guide_sections(guide: &str) -> (&str, Vec<GuideSection<'_>>) {
    let body = strip_front_matter(guide);
    // (offset, level, title) per heading. Fenced code blocks are skipped: a
    // shell comment in an example is not a section. A fence is recognized at
    // column 0 only, as every fence in the guide is written.
    let mut headings: Vec<(usize, usize, &str)> = Vec::new();
    let mut fenced = false;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            if let Some(level) = heading_level(trimmed) {
                headings.push((offset, level, trimmed[level + 1..].trim()));
            }
        }
        offset += line.len();
    }
    let intro = headings
        .first()
        .map_or(body, |(start, _, _)| &body[..*start]);
    let sections = headings
        .iter()
        .enumerate()
        .map(|(index, &(start, level, title))| {
            let end = headings
                .get(index + 1)
                .map_or(body.len(), |(next_start, _, _)| *next_start);
            GuideSection {
                level,
                slug: slugify(title),
                text: body[start..end].trim_end(),
            }
        })
        .collect();
    (intro, sections)
}

/// A section's one-line summary: its first sentence of prose, code blocks
/// skipped. The guide is hard-wrapped, so lines are joined until one ends a
/// sentence rather than reported as they are broken.
fn guide_summary(section: &GuideSection) -> String {
    let mut fenced = false;
    let mut summary = String::new();
    for line in section.text.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || trimmed.is_empty() {
            continue;
        }
        if !summary.is_empty() {
            summary.push(' ');
        }
        summary.push_str(trimmed.trim_start_matches(['-', '*', ' ']));
        if summary.ends_with('.') || summary.chars().count() >= 100 {
            break;
        }
    }
    let summary = summary.replace("**", "").replace('`', "");
    match summary.char_indices().nth(120) {
        Some((cut, _)) => format!("{}…", summary[..cut].trim_end()),
        None => summary,
    }
}

/// The guide's front page: what it is, the quick facts every caller needs
/// before writing a line of Functor Lang, and the sections it can ask for.
fn render_guide_contents(intro: &str, sections: &[GuideSection]) -> String {
    let mut out = String::from(
        "The Functor Lang language guide (the `functor-lang` skill, verbatim — the source of \
truth for the language). Call language_guide again with `section` for a section's full \
text; `api_reference` covers the prelude API instead.\n\n",
    );
    out.push_str(intro.trim());
    out.push_str("\n\n");
    if let Some(facts) = sections
        .iter()
        .find(|section| section.slug.starts_with(QUICK_FACTS_SLUG))
    {
        out.push_str(facts.text);
        out.push_str("\n\n");
    }
    out.push_str("## Sections\n\n");
    for section in sections {
        let indent = "  ".repeat(section.level - 2);
        let lines = section.text.lines().count();
        out.push_str(&format!(
            "{indent}- {} ({lines} lines) — {}\n",
            section.slug,
            guide_summary(section)
        ));
    }
    out
}

/// One section as served: its text, plus a pointer to the subsections that
/// continue it — without which a truncation at a `###` heading reads like the
/// end of the topic.
fn render_guide_section(sections: &[GuideSection], index: usize) -> String {
    let section = &sections[index];
    let children: Vec<&str> = sections[index + 1..]
        .iter()
        .take_while(|next| next.level > section.level)
        .map(|next| next.slug.as_str())
        .collect();
    match children.is_empty() {
        true => section.text.to_string(),
        false => format!(
            "{}\n\nContinues in: {}\n",
            section.text,
            children.join(", ")
        ),
    }
}

/// Look a section up by slug, or by a unique fragment of one — an agent
/// naturally asks for "syntax" or "the game contract". `Err` is a teaching
/// message naming what it could have asked for.
fn find_guide_section(sections: &[GuideSection], wanted: &str) -> Result<usize, String> {
    let needle = slugify(wanted);
    let unknown = || {
        format!(
            "unknown guide section {wanted:?}. The sections are {}.",
            join_slugs(sections.iter())
        )
    };
    if needle.is_empty() {
        return Err(unknown());
    }
    if let Some(exact) = sections.iter().position(|section| section.slug == needle) {
        return Ok(exact);
    }
    let matches: Vec<usize> = sections
        .iter()
        .enumerate()
        .filter(|(_, section)| section.slug.contains(&needle))
        .map(|(index, _)| index)
        .collect();
    match matches.as_slice() {
        [only] => Ok(*only),
        [] => Err(unknown()),
        several => Err(format!(
            "{wanted:?} matches several guide sections: {}. Name one exactly.",
            join_slugs(several.iter().map(|index| &sections[*index]))
        )),
    }
}

/// Name a set of sections the way every teaching error here does.
fn join_slugs<'a>(sections: impl Iterator<Item = &'a GuideSection<'a>>) -> String {
    sections
        .map(|section| section.slug.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A path for display: absolute where the filesystem can say so, and the
/// path as given otherwise (a directory that no longer exists, say).
fn absolute(dir: &std::path::Path) -> String {
    std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .display()
        .to_string()
}

/// The `functor.json` a project needs to be launchable, for a pushed file set
/// that carries none. Named entries are a project-file concern; an inline
/// project is by construction the single-entry case.
fn synthesized_manifest(entry: &str) -> String {
    serde_json::json!({ "language": "functor-lang", "entry": entry }).to_string()
}

/// A pushed file's path must name a file INSIDE the project directory: these
/// paths become real writes, and the file set arrives over the wire.
fn check_project_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("a project file needs a path".to_string());
    }
    if path.contains('\\') {
        return Err(format!("{path:?} must use forward slashes"));
    }
    if path.starts_with('/') || path.contains(':') {
        return Err(format!("{path:?} must be a project-relative path"));
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!(
            "{path:?} must not contain empty, `.` or `..` segments"
        ));
    }
    Ok(())
}

/// The entry module of a file set: the first `.fun`. `functor.json` may travel
/// with the sources, so "the first file" is not enough on its own.
fn entry_of(files: &[(String, String)]) -> Result<&str, String> {
    for (path, _) in files {
        check_project_path(path)?;
    }
    files
        .iter()
        .map(|(path, _)| path.as_str())
        .find(|path| path.ends_with(".fun"))
        .ok_or_else(|| "a project needs at least one .fun file (the entry, first)".to_string())
}

/// The complete file set to write for an inline `launch_game`: the caller's
/// files, plus the manifest the runtime's load path requires when they did
/// not supply one.
fn scratch_project_files(files: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let entry = entry_of(files)?.to_string();
    let mut files = files.to_vec();
    if !files.iter().any(|(path, _)| path == "functor.json") {
        files.push(("functor.json".to_string(), synthesized_manifest(&entry)));
    }
    Ok(files)
}

/// Write a project's files into `dir`, creating it (and any subdirectories the
/// paths name).
fn write_project_files(dir: &std::path::Path, files: &[(String, String)]) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    for (path, source) in files {
        check_project_path(path)?;
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&target, source)
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;
    }
    Ok(())
}

/// The project files already in `dir` — its manifest and modules, sorted.
/// Anything else there (a README, assets) is not this tool's business.
fn existing_project_files(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name == "functor.json" || name.ends_with(".fun") || name.ends_with(".funi"))
        .collect();
    found.sort();
    found
}

/// What a `functor.json` DECLARES, as sorted `(role, file)` pairs: the single
/// `entry` form as one nameless role, and every `entries` value (both the plain
/// `"client.fun"` form and the same-file `{ "file": …, "module": … }` one).
/// `None` means the text is not a manifest this can reason about.
///
/// The ROLE names matter as much as the files: a same-file multiplayer project
/// declares `client` and `server` against ONE `game.fun`, so comparing files
/// alone cannot tell it apart from a single-entry project on the same file —
/// which is exactly the corruption this exists to catch. An unknown/extra key
/// is not this function's business: the manifest is written verbatim and the
/// loader owns its own validation.
fn manifest_entries(manifest: &str) -> Option<Vec<(String, String)>> {
    let json: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let object = json.as_object()?;
    let mut declared = match (object.get("entry"), object.get("entries")) {
        // Both is what the loader calls Conflicting — not a manifest to reason
        // about. Reading it as "entry wins" would validate against a shape the
        // project could never boot.
        (Some(_), Some(_)) => return None,
        (Some(entry), None) => vec![(String::new(), entry.as_str()?.to_string())],
        (None, Some(serde_json::Value::Object(entries))) => entries
            .iter()
            .map(|(role, value)| {
                let file = match value {
                    serde_json::Value::String(file) => file.clone(),
                    serde_json::Value::Object(object) => object.get("file")?.as_str()?.to_string(),
                    _ => return None,
                };
                Some((role.clone(), file))
            })
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    // A set: `entries` is a map, so its declaration order is not part of what
    // the manifest means.
    declared.sort();
    Some(declared)
}

/// Name declared entries the way both teaching errors below do.
fn render_entries(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .map(|(role, file)| {
            if role.is_empty() {
                file.clone()
            } else {
                format!("{role} → {file}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Persist a session's sources into `dir`, returning what was written.
///
/// A directory that already holds a project is refused unless `overwrite` —
/// "it has the same entry file name" is not evidence that it is the same
/// project, since nearly every Functor project's entry is `game.fun`. With
/// `overwrite`, modules this session does NOT have are removed as well:
/// `file = module`, so a leftover sibling would still load and the saved copy
/// would not be the program that ran.
///
/// `manifest` is the `functor.json` the session BOOTED with, when this server
/// launched it. It is written verbatim rather than reconstructed: a multiplayer
/// project's `entries` map cannot be recovered from `GET /project` (which
/// reports modules, not project metadata), and reconstructing it as
/// `{"entry": "game.fun"}` produces a directory that builds green and then runs
/// the wrong role — the failure this argument exists to prevent. A manifest
/// that no longer describes the file set is REFUSED rather than adjusted.
fn save_project_to(
    dir: &std::path::Path,
    files: &[(String, String)],
    manifest: Option<&str>,
    overwrite: bool,
) -> Result<Vec<String>, String> {
    let entry = entry_of(files)?.to_string();
    let existing = existing_project_files(dir);
    if !existing.is_empty() && !overwrite {
        return Err(format!(
            "{} already holds a project ({}) — save to a new directory, or pass \
overwrite: true to replace it",
            dir.display(),
            existing.join(", ")
        ));
    }
    let mut files = files.to_vec();
    // Overwriting a directory that keeps its OWN manifest is the other way to
    // end up with a project that builds and runs the wrong role: the sources
    // become this session's, the manifest stays whoever's it was. Refuse when
    // the two disagree about the entries rather than write the mismatch.
    if let (Some(booted), true) = (
        manifest,
        existing.iter().any(|name| name == "functor.json"),
    ) {
        let there = std::fs::read_to_string(dir.join("functor.json")).unwrap_or_default();
        if let (Some(mine), Some(theirs)) = (manifest_entries(booted), manifest_entries(&there)) {
            if mine != theirs {
                return Err(format!(
                    "{} already holds a functor.json declaring {}, but this session runs {} — \
saving would leave that manifest in place over these sources, which builds and then runs the \
wrong entry. Save to a directory with no functor.json, or fix that one first.",
                    dir.display(),
                    render_entries(&theirs),
                    render_entries(&mine)
                ));
            }
        }
    }
    // The manifest is not part of what the runtime reports, so write one for a
    // project that has none — and never rewrite the one already there, which
    // may declare named entries this session's sources cannot describe.
    if !files.iter().any(|(path, _)| path == "functor.json")
        && !existing.iter().any(|name| name == "functor.json")
    {
        let manifest = match manifest {
            // The session's own manifest: roles, cursor policy, and every other
            // project-level setting survive the save exactly as launched.
            Some(manifest) => {
                if let Some(declared) = manifest_entries(manifest) {
                    let mut missing: Vec<&str> = declared
                        .iter()
                        .map(|(_, file)| file.as_str())
                        .filter(|file| !files.iter().any(|(path, _)| path == file))
                        .collect();
                    missing.dedup();
                    if !missing.is_empty() {
                        return Err(format!(
                            "the session's functor.json declares {} but the running program \
no longer has {} — saving would write a manifest that builds and then fails at run. Save the \
sources to a directory that already holds the right functor.json instead.",
                            render_entries(&declared),
                            missing.join(", ")
                        ));
                    }
                }
                manifest.to_string()
            }
            // An ATTACHED session: nothing on the wire reports the manifest, so
            // the single-entry one is the only honest guess.
            None => synthesized_manifest(&entry),
        };
        files.push(("functor.json".to_string(), manifest));
    }
    write_project_files(dir, &files)?;
    for stale in existing
        .iter()
        .filter(|name| name.ends_with(".fun") || name.ends_with(".funi"))
        .filter(|name| !files.iter().any(|(path, _)| path == *name))
    {
        std::fs::remove_file(dir.join(stale))
            .map_err(|error| format!("could not remove the stale module {stale}: {error}"))?;
    }
    Ok(files.into_iter().map(|(path, _)| path).collect())
}

/// The runtime's current time, from a `/state` document. Never defaulted: a
/// silent 0.0 would pin a paused game (or a rewind) to the start of time.
fn tts_of(state: &Value) -> Result<f64, String> {
    state["tts"].as_f64().ok_or_else(|| {
        format!(
            "GET /state did not report a numeric tts (got {})",
            state["tts"]
        )
    })
}

/// Read a response body, turning a non-2xx into a message that carries the
/// runtime's own text — the 400s from `/input`, `/time` and the reload routes
/// are teaching errors, so they must reach the caller verbatim.
async fn read_body(path: &str, response: reqwest::Response) -> Result<String, String> {
    let (status, body) = read_bounded_response(
        response,
        MAX_RUNTIME_TEXT_BYTES,
        "generic runtime text response",
        &format!("{path} response"),
    )
    .await?;
    let body = String::from_utf8(body).map_err(|error| {
        format!("reading the {path} response failed: body is not UTF-8: {error}")
    })?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("{path} → {status}: {body}"))
    }
}

fn output_limit_error(cap_name: &str, used: usize, limit: usize) -> String {
    format!("{cap_name} cap exceeded: used {used} bytes, limit {limit} bytes")
}

fn checked_output_total(
    retained: usize,
    additional: usize,
    limit: usize,
    cap_name: &str,
) -> Result<usize, String> {
    let used = retained
        .checked_add(additional)
        .ok_or_else(|| output_limit_error(cap_name, usize::MAX, limit))?;
    if used > limit {
        Err(output_limit_error(cap_name, used, limit))
    } else {
        Ok(used)
    }
}

/// Append only after proving the retained buffer remains within its cap.
/// A rejected chunk is never copied into `body`.
fn extend_bounded(
    body: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
    cap_name: &str,
) -> Result<(), String> {
    let used = body
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| output_limit_error(cap_name, usize::MAX, limit))?;
    if used > limit {
        return Err(output_limit_error(cap_name, used, limit));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl io::Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::other("serialized JSON byte count overflowed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_encoded_len(value: &Value) -> Result<usize, String> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| format!("could not account for submitted-code JSON output: {error}"))?;
    Ok(writer.bytes)
}

struct BoundedWriter<'a> {
    body: Vec<u8>,
    limit: usize,
    cap_name: &'a str,
}

impl io::Write for BoundedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        extend_bounded(&mut self.body, buf, self.limit, self.cap_name).map_err(io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize_json_bounded(value: &Value, limit: usize, cap_name: &str) -> Result<String, String> {
    let mut writer = BoundedWriter {
        body: Vec::new(),
        limit,
        cap_name,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
    String::from_utf8(writer.body)
        .map_err(|error| format!("serialized submitted-code JSON was not UTF-8: {error}"))
}

fn base64_encoded_len(raw_bytes: usize) -> Result<usize, String> {
    raw_bytes
        .checked_add(2)
        .map(|bytes| bytes / 3)
        .and_then(|groups| groups.checked_mul(4))
        .ok_or_else(|| "base64 encoded capture byte count overflowed".to_string())
}

fn encoded_capture_total(
    capture_lengths: impl IntoIterator<Item = usize>,
    limit: usize,
) -> Result<usize, String> {
    capture_lengths.into_iter().try_fold(0, |retained, raw| {
        checked_output_total(
            retained,
            base64_encoded_len(raw)?,
            limit,
            "submitted-code aggregate encoded MCP image content",
        )
    })
}

fn node_code_summary(output: &NodeCodeOutput) -> Result<String, String> {
    encoded_capture_total(
        output.captures.iter().map(Vec::len),
        MAX_CODE_ENCODED_CAPTURE_BYTES,
    )?;
    let captures: Vec<Value> = output
        .captures
        .iter()
        .enumerate()
        .map(|(index, png)| {
            serde_json::json!({
                "content_index": index + 1,
                "mime_type": "image/png",
                "bytes": png.len(),
            })
        })
        .collect();
    let ok = output.failure.is_none() && output.code_error.is_none();
    let mut summary = serde_json::json!({
        "ok": ok,
        "unsafe": {
            "rce_equivalent": true,
            "security_sandbox": false,
            "execution": "local_node_child_process",
        },
        "node_version": &output.node_version,
        "calls_executed": output.calls_executed,
        "return_value": &output.return_value,
        "error": &output.failure,
        "code_error": &output.code_error,
        "trace": &output.trace,
        "logs": &output.logs,
        "captures": captures,
        "final_state": &output.final_state,
    });
    // Preserve the default result shape exactly. Compact callers get one
    // additional, explicitly named field while `final_state` remains null.
    if let Some(final_state_summary) = &output.final_state_summary {
        summary
            .as_object_mut()
            .expect("submitted-code summary is an object")
            .insert("final_state_summary".into(), final_state_summary.clone());
    }
    serialize_json_bounded(
        &summary,
        MAX_CODE_TEXT_BYTES,
        "submitted-code aggregate text output",
    )
}

fn node_code_call_result(output: NodeCodeOutput) -> Result<CallToolResult, String> {
    let mut content = Vec::with_capacity(output.captures.len() + 1);
    content.push(ContentBlock::text(node_code_summary(&output)?));
    let ok = output.failure.is_none() && output.code_error.is_none();
    for png in output.captures {
        content.push(ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(png),
            "image/png",
        ));
    }
    if ok {
        Ok(CallToolResult::success(content))
    } else {
        Ok(CallToolResult::error(content))
    }
}

fn truncate_code_error(mut text: String) -> String {
    if text.len() <= MAX_CODE_TEXT_BYTES {
        return text;
    }
    let used = text.len();
    let suffix = format!(
        "\n… submitted-code error truncated: used {used} bytes, limit {MAX_CODE_TEXT_BYTES} bytes"
    );
    let keep = MAX_CODE_TEXT_BYTES.saturating_sub(suffix.len());
    let mut boundary = keep.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    text.push_str(&suffix[..suffix.len().min(MAX_CODE_TEXT_BYTES - text.len())]);
    text
}

/// Read an HTTP response without trusting `Content-Length`: reject an
/// oversized declared length before body allocation, then enforce the same cap
/// as chunks arrive (including chunked responses with no declared length).
async fn read_bounded_response(
    mut response: reqwest::Response,
    limit: usize,
    cap_name: &str,
    context: &str,
) -> Result<(reqwest::StatusCode, Vec<u8>), String> {
    let status = response.status();
    let declared = response.content_length();
    if declared.is_some_and(|used| used > limit as u64) {
        return Err(output_limit_error(
            cap_name,
            declared
                .unwrap_or(u64::MAX)
                .try_into()
                .unwrap_or(usize::MAX),
            limit,
        ));
    }
    let capacity = declared
        .and_then(|bytes| usize::try_from(bytes).ok())
        .unwrap_or(8 * 1024)
        .min(limit);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("reading {context} failed: {error}"))?
    {
        extend_bounded(&mut body, &chunk, limit, cap_name)?;
    }
    Ok((status, body))
}

/// Claim a free localhost port by binding and immediately releasing it. There
/// is a race window before the runtime binds; on loopback with sequential
/// launches it is not one worth a retry protocol.
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("could not find a free port: {error}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|error| format!("could not read the bound port: {error}"))
}

/// Accumulate a child's output into a bounded tail buffer, so a launch that
/// never serves can still report why.
fn drain_into<R>(mut stream: R, log: Arc<Mutex<Vec<u8>>>) -> tokio::task::JoinHandle<()>
where
    R: AsyncReadExt + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut chunk = [0u8; 4096];
        while let Ok(read) = stream.read(&mut chunk).await {
            if read == 0 {
                break;
            }
            let mut buffer = log.lock().expect("mcp log poisoned");
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.len() > LOG_TAIL_BYTES {
                let excess = buffer.len() - LOG_TAIL_BYTES;
                buffer.drain(..excess);
            }
        }
    })
}

fn tail(log: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&log.lock().expect("mcp log poisoned")).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_target_operation, base64_encoded_len, checked_output_total, declared_entries,
        deliverable_prefix, encoded_capture_total, group_labels, group_roles, MAX_DELIVERY_BYTES,
        ensure_operation_active, extend_bounded, find_guide_section, guide_sections,
        json_encoded_len, node_code_call_result, node_code_summary, read_body,
        read_bounded_response, render_api_hits, render_guide_contents, render_guide_section,
        retain_final_snapshot, search_api, serialize_json_bounded, spawn_node_child,
        strip_front_matter, truncate_code_error, CodeInputRestore, CodeOutputBudget, FunctorMcp,
        NodeCodeOutput, Registry, RunGameCodeUnsafeArgs, SessionTarget, WireFilter,
        LANGUAGE_GUIDE,
        MAX_CODE_CAPTURE_BYTES, MAX_CODE_TEXT_BYTES, QUICK_FACTS_SLUG,
    };
    use functor_docgen::ApiReference;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn reference() -> ApiReference {
        functor_docgen::generate().expect("the embedded prelude documents itself")
    }

    fn empty_node_code_output() -> NodeCodeOutput {
        NodeCodeOutput {
            node_version: "v22.0.0".into(),
            return_value: serde_json::Value::Null,
            code_error: None,
            failure: None,
            trace: Vec::new(),
            logs: Vec::new(),
            captures: Vec::new(),
            calls_executed: 0,
            final_state: None,
            final_state_summary: None,
            output_budget: CodeOutputBudget::default(),
        }
    }

    async fn fake_http_url(raw: impl AsRef<[u8]>) -> String {
        let raw = raw.as_ref().to_vec();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(&raw).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        format!("http://{address}")
    }

    async fn hanging_http_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            let _socket = socket;
            std::future::pending::<()>().await;
        });
        format!("http://{address}")
    }

    async fn fake_http_response(raw: &'static [u8]) -> reqwest::Response {
        reqwest::get(format!("{}/", fake_http_url(raw).await))
            .await
            .unwrap()
    }

    #[test]
    fn output_accounting_rejects_before_retaining_an_oversized_chunk() {
        assert_eq!(checked_output_total(10, 6, 16, "test output").unwrap(), 16);
        let total_error = checked_output_total(10, 7, 16, "test output").unwrap_err();
        assert!(total_error.contains("used 17 bytes"), "{total_error}");
        assert!(total_error.contains("limit 16 bytes"), "{total_error}");

        let mut retained = b"123456789".to_vec();
        let error = extend_bounded(&mut retained, b"abcdefghi", 16, "test stream").unwrap_err();
        assert_eq!(retained, b"123456789", "the rejected chunk is not copied");
        assert!(error.contains("used 18 bytes"), "{error}");
        assert!(error.contains("limit 16 bytes"), "{error}");
    }

    #[test]
    fn submitted_code_json_and_capture_budgets_are_explicit_and_bounded() {
        let value = serde_json::json!({"answer": 42, "ready": true});
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(json_encoded_len(&value).unwrap(), encoded.len());
        assert_eq!(
            serialize_json_bounded(&value, encoded.len(), "test JSON").unwrap(),
            encoded
        );
        let error = serialize_json_bounded(&value, encoded.len() - 1, "test JSON").unwrap_err();
        assert!(error.contains("test JSON cap exceeded"), "{error}");

        let mut budget = CodeOutputBudget {
            retained_text_bytes: MAX_CODE_TEXT_BYTES - 1,
            capture_bytes: MAX_CODE_CAPTURE_BYTES - 1,
        };
        budget.retain_json(&serde_json::json!(0)).unwrap();
        let text_error = budget.retain_json(&serde_json::json!(0)).unwrap_err();
        assert!(text_error.contains("submitted-code aggregate text output"));
        budget.retain_capture(1).unwrap();
        let capture_error = budget.retain_capture(1).unwrap_err();
        assert!(capture_error.contains("submitted-code aggregate raw capture output"));
    }

    #[test]
    fn submitted_code_final_state_defaults_to_the_compatible_full_response() {
        let args: RunGameCodeUnsafeArgs = serde_json::from_value(serde_json::json!({
            "session": "game",
            "code": "async () => null"
        }))
        .unwrap();
        assert_eq!(args.include_final_state, None);

        let state = serde_json::json!({
            "frame": 42,
            "tts": 1.5,
            "pending_steps": 0,
            "model": {"answer": 42},
            "model_debug": "answer = 42",
            "input": {
                "held_keys": [],
                "mouse": {"buttons": {"left": false, "right": false, "middle": false}}
            }
        });
        let mut output = empty_node_code_output();
        retain_final_snapshot(
            &mut output,
            state.clone(),
            args.include_final_state.unwrap_or(true),
        )
        .unwrap();

        let result: serde_json::Value =
            serde_json::from_str(&node_code_summary(&output).unwrap()).unwrap();
        assert_eq!(result["final_state"], state);
        assert!(
            result.get("final_state_summary").is_none(),
            "the default result shape stays unchanged"
        );
    }

    #[test]
    fn compact_submitted_code_final_state_has_only_runtime_and_input_facts() {
        let state = serde_json::json!({
            "frame": 81,
            "tts": 2.25,
            "paused": true,
            "pending_steps": 0,
            "model": {
                "boids": [
                    {"marker": "BOID_MODEL_MUST_NOT_ESCAPE"},
                    {"marker": "BOID_MODEL_MUST_NOT_ESCAPE"}
                ]
            },
            "model_debug": "MODEL_DEBUG_MUST_NOT_ESCAPE",
            "input": {
                "held_keys": ["W"],
                "mouse": {
                    "buttons": {"left": true, "right": false, "middle": true}
                }
            }
        });
        let expected_model_bytes = json_encoded_len(&state["model"]).unwrap();
        let mut output = empty_node_code_output();
        retain_final_snapshot(&mut output, state, false).unwrap();

        assert!(output.final_state.is_none());
        assert_eq!(
            output.final_state_summary,
            Some(serde_json::json!({
                "frame": 81,
                "tts": 2.25,
                "paused": true,
                "pending_steps": 0,
                "held_keys": ["W"],
                "held_mouse_buttons": ["left", "middle"],
                "model_json_bytes": expected_model_bytes,
            }))
        );
        let encoded = node_code_summary(&output).unwrap();
        assert!(!encoded.contains("BOID_MODEL_MUST_NOT_ESCAPE"));
        assert!(!encoded.contains("MODEL_DEBUG_MUST_NOT_ESCAPE"));
        let result: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert!(result["final_state"].is_null());
        assert!(result["final_state_summary"].get("model").is_none());
        assert!(result["final_state_summary"].get("model_debug").is_none());
    }

    #[test]
    fn compact_final_state_is_still_charged_but_not_charged_as_the_full_model() {
        let state = serde_json::json!({
            "frame": 1,
            "tts": 0.0,
            "pending_steps": 0,
            "model": "x".repeat(MAX_CODE_TEXT_BYTES),
            "model_debug": "also deliberately large",
            "input": {
                "held_keys": [],
                "mouse": {"buttons": {}}
            }
        });

        let mut full = empty_node_code_output();
        let full_error = retain_final_snapshot(&mut full, state.clone(), true).unwrap_err();
        assert!(
            full_error.contains("submitted-code aggregate text output"),
            "{full_error}"
        );
        assert!(full.final_state.is_none());

        let mut compact = empty_node_code_output();
        retain_final_snapshot(&mut compact, state.clone(), false).unwrap();
        let summary = compact.final_state_summary.as_ref().unwrap();
        assert_eq!(
            summary["model_json_bytes"],
            json_encoded_len(&state["model"]).unwrap()
        );
        assert!(compact.output_budget.retained_text_bytes < 256);

        let summary_bytes = json_encoded_len(summary).unwrap();
        let mut exhausted = empty_node_code_output();
        exhausted.output_budget.retained_text_bytes = MAX_CODE_TEXT_BYTES - summary_bytes + 1;
        let compact_error = retain_final_snapshot(&mut exhausted, state, false).unwrap_err();
        assert!(
            compact_error.contains("submitted-code aggregate text output"),
            "{compact_error}"
        );
        assert!(exhausted.final_state_summary.is_none());
    }

    #[test]
    fn submitted_code_input_restoration_canonicalizes_serialized_digit_keys() {
        let mut restore = CodeInputRestore::from_state(&serde_json::json!({
            "input": {
                "held_keys": ["W", "Num1"],
                "mouse": { "buttons": {} },
            }
        }));
        restore.touch_key("w");
        restore.touch_key("1");

        assert!(restore.baseline_keys.contains("W"));
        assert!(restore.baseline_keys.contains("1"));
        assert!(restore.touched_keys.contains("W"));
        assert!(restore.touched_keys.contains("1"));
    }

    #[test]
    fn encoded_capture_accounting_precedes_final_mcp_result_construction() {
        assert_eq!(base64_encoded_len(0).unwrap(), 0);
        assert_eq!(base64_encoded_len(1).unwrap(), 4);
        assert_eq!(base64_encoded_len(2).unwrap(), 4);
        assert_eq!(base64_encoded_len(3).unwrap(), 4);
        assert_eq!(base64_encoded_len(4).unwrap(), 8);
        assert_eq!(encoded_capture_total([1, 2, 3], 12).unwrap(), 12);
        let error = encoded_capture_total([1, 2, 3], 11).unwrap_err();
        assert!(error.contains("encoded MCP image content"), "{error}");

        let result = node_code_call_result(NodeCodeOutput {
            node_version: "v22.0.0".into(),
            return_value: serde_json::json!({"answer": 42}),
            code_error: None,
            failure: None,
            trace: Vec::new(),
            logs: Vec::new(),
            captures: vec![vec![1, 2, 3], vec![4]],
            calls_executed: 0,
            final_state: None,
            final_state_summary: None,
            output_budget: CodeOutputBudget::default(),
        })
        .unwrap();
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["content"].as_array().unwrap().len(), 3);
        assert_eq!(value["content"][1]["data"], "AQID");
        assert_eq!(value["content"][2]["data"], "BA==");
    }

    #[test]
    fn submitted_code_error_text_is_centrally_utf8_truncated() {
        let original = format!("{}é", "x".repeat(MAX_CODE_TEXT_BYTES));
        let truncated = truncate_code_error(original);
        assert_eq!(truncated.len(), MAX_CODE_TEXT_BYTES);
        assert!(truncated.contains("submitted-code error truncated"));
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[test]
    fn missing_node_has_a_teaching_error() {
        let missing = std::env::temp_dir().join(format!(
            "functor-node-that-does-not-exist-{}",
            std::process::id()
        ));
        let error = spawn_node_child(&missing).unwrap_err();
        assert!(error.contains("requires Node.js 20 or newer"), "{error}");
        assert!(error.contains("FUNCTOR_NODE"), "{error}");
    }

    #[tokio::test]
    async fn bounded_http_reader_rejects_declared_and_streamed_oversize_bodies() {
        let declared = fake_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\n",
        )
        .await;
        let declared_error =
            read_bounded_response(declared, 16, "test declared body", "test response")
                .await
                .unwrap_err();
        assert!(declared_error.contains("used 17 bytes"), "{declared_error}");
        assert!(
            declared_error.contains("limit 16 bytes"),
            "{declared_error}"
        );

        let streamed = fake_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
9\r\n123456789\r\n9\r\nabcdefghi\r\n0\r\n\r\n",
        )
        .await;
        let streamed_error =
            read_bounded_response(streamed, 16, "test streamed body", "test response")
                .await
                .unwrap_err();
        assert!(streamed_error.contains("used 18 bytes"), "{streamed_error}");
        assert!(
            streamed_error.contains("limit 16 bytes"),
            "{streamed_error}"
        );

        let teaching = fake_http_response(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 19\r\nConnection: close\r\n\r\nunknown key Frobble",
        )
        .await;
        let teaching_error = read_body("/input", teaching).await.unwrap_err();
        assert!(
            teaching_error.contains("400 Bad Request: unknown key Frobble"),
            "{teaching_error}"
        );
    }

    #[tokio::test]
    async fn uncertain_abort_cleanup_quarantines_every_exact_url_alias() {
        let server = FunctorMcp::new();
        let url = fake_http_url(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 6\r\nConnection: close\r\n\r\nfailed",
        )
        .await;
        {
            let mut registry = server.sessions.lock().unwrap();
            registry.insert(url.clone(), None, None, None, None);
            registry.insert(url.clone(), None, None, None, None);
        }
        let target = server.target("s1").unwrap();

        let error = server.abort_advance(&target, "advance failed".into()).await;

        assert!(error.contains("quarantined"), "{error}");
        let registry = server.sessions.lock().unwrap();
        let Err(owner_error) = registry.target("s1") else {
            panic!("the quarantined owner must reject mutations");
        };
        let Err(alias_error) = registry.target("s2") else {
            panic!("the quarantined alias must reject mutations");
        };
        assert!(owner_error.contains("quarantined"), "{owner_error}");
        assert!(alias_error.contains("quarantined"), "{alias_error}");
        drop(registry);

        {
            let mut registry = server.sessions.lock().unwrap();
            registry.remove("s1").unwrap();
            registry.remove("s2").unwrap();
            assert!(registry.sessions.is_empty());
        }
        let reconnect = server.reserve_connect(&url);
        let reconnect_target = reconnect.target();
        assert!(
            reconnect_target.quarantined.load(Ordering::Acquire),
            "an attached detach/reconnect must retain the URL tombstone"
        );
        let reconnect_error = acquire_target_operation(&reconnect_target, std::future::pending())
            .await
            .unwrap_err();
        assert!(reconnect_error.contains("quarantined"), "{reconnect_error}");
    }

    #[tokio::test]
    async fn failed_code_input_restoration_quarantines_the_url() {
        let server = FunctorMcp::new();
        let url = fake_http_url(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 6\r\nConnection: close\r\n\r\nfailed",
        )
        .await;
        let target = SessionTarget {
            url,
            operation_gate: Arc::new(tokio::sync::Mutex::new(())),
            quarantined: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        };
        let input_restore = CodeInputRestore {
            touched_keys: ["space".into()].into_iter().collect(),
            ..CodeInputRestore::default()
        };

        let error = server
            .code_failure(&target, "assertion failed".into(), &input_restore)
            .await;

        assert!(error.contains("quarantined"), "{error}");
        assert!(target.quarantined.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn code_cleanup_does_not_replay_an_already_correct_input_level() {
        let server = FunctorMcp::new();
        let body = r#"{"input":{"held_keys":["W"],"mouse":{"buttons":{}}}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let target = SessionTarget {
            url: fake_http_url(response).await,
            operation_gate: Arc::new(tokio::sync::Mutex::new(())),
            quarantined: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        };
        let input_restore = CodeInputRestore {
            baseline_keys: ["W".into()].into_iter().collect(),
            touched_keys: ["W".into()].into_iter().collect(),
            ..CodeInputRestore::default()
        };

        let result = server
            .code_failure(&target, "submitted code failed".into(), &input_restore)
            .await;

        assert!(result.contains("levels were restored"), "{result}");
        assert!(
            !target.quarantined.load(Ordering::Acquire),
            "an already-correct level must not trigger a duplicate POST"
        );
    }

    #[tokio::test]
    async fn submitted_code_cleanup_has_one_aggregate_deadline() {
        let server = FunctorMcp::new();
        let target = SessionTarget {
            url: hanging_http_url().await,
            operation_gate: Arc::new(tokio::sync::Mutex::new(())),
            quarantined: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        };
        let input_restore = CodeInputRestore {
            touched_keys: ["Space".into()].into_iter().collect(),
            ..CodeInputRestore::default()
        };
        let started = Instant::now();

        let result = server
            .cleanup_submitted_code_with_timeout(
                &target,
                "submitted code timed out".into(),
                &input_restore,
                false,
                Duration::from_millis(20),
            )
            .await;

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(result.contains("aggregate deadline"), "{result}");
        assert!(result.contains("quarantined"), "{result}");
        assert!(target.quarantined.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn a_definitive_advance_rejection_does_not_cancel_or_quarantine() {
        let server = FunctorMcp::new();
        let body = "the clock is pinned by --fixed-time";
        let response = format!(
            "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let target = SessionTarget {
            url: fake_http_url(response).await,
            operation_gate: Arc::new(tokio::sync::Mutex::new(())),
            quarantined: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        };

        let error = server
            .advance(&target, 0.016, 1, Instant::now() + Duration::from_secs(1))
            .await
            .expect_err("the runtime rejected the advance");

        assert!(error.contains("409"), "{error}");
        assert!(error.contains("--fixed-time"), "{error}");
        assert!(
            !target.quarantined.load(Ordering::Acquire),
            "a rejected command never owned queued work to clean up"
        );
    }

    #[test]
    fn an_exact_name_match_is_reported_first() {
        let reference = reference();
        let hits = search_api(&reference, "Scene.cube", None).unwrap();

        assert_eq!(hits[0].item.qualified_name, "Scene.cube");
        assert_eq!(hits[0].rank, 0);
        // The rendered text carries the signature and the prose, not just a name.
        let rendered = render_api_hits(&hits, Some(super::MAX_API_RESULTS));
        assert!(rendered.contains("let cube : () => t"), "{rendered}");
    }

    #[test]
    fn a_bare_item_name_finds_it_across_modules() {
        let reference = reference();
        let hits = search_api(&reference, "cube", None).unwrap();

        assert_eq!(hits[0].rank, 0);
        assert!(
            hits.iter()
                .any(|hit| hit.item.qualified_name == "Scene.cube"),
            "Scene.cube must match its own bare name"
        );
    }

    /// The drift guard. The guide is the `functor-lang` skill embedded
    /// verbatim, so a restructure that broke the embedding (or emptied it)
    /// must fail here rather than quietly serving an empty language surface.
    #[test]
    fn the_embedded_guide_still_teaches_the_language() {
        assert!(
            LANGUAGE_GUIDE.len() > 50_000,
            "the embedded skill is suspiciously small ({} bytes)",
            LANGUAGE_GUIDE.len()
        );
        let lowercase = LANGUAGE_GUIDE.to_lowercase();
        for marker in [
            ":=",                  // assignment, not `<-`
            "thread-last",         // `x |> f(a)` is `f(a, x)`
            "if cond then a else", // the conditional is an expression
            "let init",
            "let tick",
            "let draw",
        ] {
            assert!(
                lowercase.contains(&marker.to_lowercase()),
                "the language guide no longer covers {marker:?}"
            );
        }

        let (intro, sections) = guide_sections(LANGUAGE_GUIDE);
        assert!(sections.len() > 8, "sections: {}", sections.len());
        assert!(
            sections
                .iter()
                .any(|section| section.slug.starts_with(QUICK_FACTS_SLUG)),
            "the quick-facts section fronts the table of contents"
        );
        // Every heading became a section, and the last one runs to the end of
        // the file: an unbalanced code fence (or an unhandled heading level)
        // otherwise swallows the whole tail silently.
        let headings = LANGUAGE_GUIDE
            .lines()
            .filter(|line| line.starts_with("## ") || line.starts_with("### "))
            .count();
        assert_eq!(headings, sections.len(), "a heading was not sectioned");
        let body = strip_front_matter(LANGUAGE_GUIDE).trim_end();
        assert!(body.ends_with(sections.last().unwrap().text));
        // The front matter is loader metadata, not language documentation.
        assert!(intro.trim_start().starts_with("# Functor Lang"), "{intro}");
    }

    #[test]
    fn the_table_of_contents_is_short_and_leads_with_the_quick_facts() {
        let (intro, sections) = guide_sections(LANGUAGE_GUIDE);
        let contents = render_guide_contents(intro, &sections);

        assert!(contents.contains("Assignment is `:=`"), "{contents}");
        assert!(contents.contains("thread-LAST"), "{contents}");
        assert!(contents.contains("- syntax-subset ("), "{contents}");
        // The point of a table of contents is that it is not the whole guide.
        assert!(
            contents.len() < LANGUAGE_GUIDE.len() / 4,
            "the contents are {} bytes",
            contents.len()
        );
    }

    #[test]
    fn a_section_is_addressable_by_slug_or_by_a_unique_fragment() {
        let (_, sections) = guide_sections(LANGUAGE_GUIDE);
        let text = |wanted| {
            render_guide_section(&sections, find_guide_section(&sections, wanted).unwrap())
        };

        let syntax = text("syntax-subset");
        assert!(syntax.starts_with("## Syntax subset"), "{syntax}");
        assert!(syntax.contains("|> APPENDS"), "{syntax}");

        // A fragment, spelled the way an agent would ask for it.
        assert!(text("game contract").contains("let draw = (model, tts)"));
    }

    /// The parser's edges, on a fixture rather than on the skill's prose:
    /// nesting, a heading that is really a shell comment in a code fence, and
    /// the `Continues in:` pointer a truncated parent needs.
    #[test]
    fn sections_nest_fence_aware_and_a_parent_points_at_its_children() {
        const FIXTURE: &str = "---\nname: fixture\n---\n\nIntro prose.\n\n\
## First\n\nOne.\n\n```sh\n## not a heading\n```\n\n\
### Nested\n\nTwo.\n\n## Second\n\nThree.\n";
        let (intro, sections) = guide_sections(FIXTURE);

        assert_eq!(intro.trim(), "Intro prose.");
        assert_eq!(
            sections.iter().map(|s| s.slug.as_str()).collect::<Vec<_>>(),
            ["first", "nested", "second"]
        );
        // The fenced `## not a heading` stayed inside its section's text.
        assert!(sections[0].text.contains("## not a heading"));
        assert!(!sections[0].text.contains("### Nested"));

        let first = render_guide_section(&sections, 0);
        assert!(first.ends_with("Continues in: nested\n"), "{first}");
        assert_eq!(render_guide_section(&sections, 2), sections[2].text);
    }

    #[test]
    fn an_unknown_ambiguous_or_empty_section_names_the_ones_that_exist() {
        const FIXTURE: &str = "## Sound design\n\nOne.\n\n## Sound effects\n\nTwo.\n";
        let (_, sections) = guide_sections(FIXTURE);

        let unknown = find_guide_section(&sections, "monads").unwrap_err();
        assert!(unknown.contains("sound-design, sound-effects"), "{unknown}");
        // A blank name is a client bug, not "give me the first section".
        assert!(find_guide_section(&sections, "   ").is_err());

        let ambiguous = find_guide_section(&sections, "sound").unwrap_err();
        assert!(ambiguous.contains("matches several"), "{ambiguous}");
        assert!(
            ambiguous.contains("sound-design, sound-effects"),
            "{ambiguous}"
        );
    }

    #[test]
    fn front_matter_is_dropped_however_the_file_is_checked_out() {
        assert_eq!(strip_front_matter("---\na: b\n---\n# Title\n"), "# Title\n");
        assert_eq!(
            strip_front_matter("---\r\na: b\r\n---\r\n# Title\r\n"),
            "# Title\r\n"
        );
        // No front matter, or an unterminated one, is served as it is rather
        // than truncated to nothing.
        assert_eq!(strip_front_matter("# Title\n"), "# Title\n");
        assert_eq!(strip_front_matter("---\na: b\n"), "---\na: b\n");
    }

    #[test]
    fn a_module_filter_narrows_and_browses() {
        let reference = reference();
        let browsed = search_api(&reference, "", Some("effect")).unwrap();

        assert!(browsed.len() > 1, "browsing a module lists its items");
        assert!(browsed
            .iter()
            .all(|hit| hit.item.qualified_name.starts_with("Effect.")));
    }

    #[test]
    fn an_unknown_module_names_the_modules_that_exist() {
        let reference = reference();
        let error = search_api(&reference, "cube", Some("Nope")).unwrap_err();

        assert!(error.contains("unknown module"), "{error}");
        assert!(error.contains("Scene"), "{error}");
    }

    #[test]
    fn a_search_with_nothing_to_go_on_names_the_modules() {
        let reference = reference();
        let error = search_api(&reference, "  ", None).unwrap_err();

        assert!(error.contains("Scene"), "{error}");
    }

    #[test]
    fn a_query_matching_nothing_returns_no_hits() {
        let reference = reference();
        assert!(search_api(&reference, "zzzznotathing", None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_truncated_search_says_so_and_a_module_listing_is_whole() {
        let reference = reference();
        let hits = search_api(&reference, "", Some("Scene")).unwrap();
        assert!(hits.len() > super::MAX_API_RESULTS, "Scene is a big module");

        let capped = render_api_hits(&hits, Some(super::MAX_API_RESULTS));
        assert!(capped.contains("matches shown"), "{capped}");
        // Listing a module is not truncated: `module` is already the narrowing
        // the truncation notice would tell the caller to apply.
        let whole = render_api_hits(&hits, None);
        assert!(!whole.contains("matches shown"), "{whole}");
        assert_eq!(
            whole
                .lines()
                .filter(|line| line.starts_with("Scene."))
                .count(),
            hits.len()
        );
    }

    #[test]
    fn a_name_match_outranks_a_signature_match() {
        let reference = reference();
        let hits = search_api(&reference, "texture", None).unwrap();

        let first = &hits[0];
        assert!(
            first.item.qualified_name.to_lowercase().contains("texture"),
            "a name match must come before consumers that merely mention the type: {}",
            first.item.qualified_name
        );
    }

    #[test]
    fn a_blank_module_is_a_teaching_error_rather_than_a_widened_search() {
        let reference = reference();
        let error = search_api(&reference, "cube", Some("  ")).unwrap_err();

        assert!(error.contains("unknown module"), "{error}");
    }

    #[test]
    fn an_inline_project_gets_a_manifest_naming_its_entry() {
        let files =
            super::scratch_project_files(&[("game.fun".into(), "let init = { n: 0.0 }".into())])
                .unwrap();

        assert_eq!(files.len(), 2);
        let (path, manifest) = &files[1];
        assert_eq!(path, "functor.json");
        let parsed: serde_json::Value = serde_json::from_str(manifest).unwrap();
        assert_eq!(parsed["language"], "functor-lang");
        assert_eq!(parsed["entry"], "game.fun");
    }

    /// The entry is the first `.fun`, not the first file: a caller may push
    /// its own manifest ahead of the sources.
    #[test]
    fn a_supplied_manifest_is_kept_and_does_not_become_the_entry() {
        let files = super::scratch_project_files(&[
            ("functor.json".into(), "{\"entry\":\"main.fun\"}".into()),
            ("main.fun".into(), "let init = 1".into()),
        ])
        .unwrap();

        assert_eq!(files.len(), 2, "no manifest is synthesized: {files:?}");
        assert_eq!(super::entry_of(&files).unwrap(), "main.fun");
    }

    #[test]
    fn a_file_set_with_no_module_or_an_escaping_path_is_refused() {
        let no_module =
            super::scratch_project_files(&[("readme.md".into(), "hi".into())]).unwrap_err();
        assert!(no_module.contains(".fun"), "{no_module}");

        for path in ["../evil.fun", "/etc/evil.fun", "a//b.fun", ""] {
            let error =
                super::scratch_project_files(&[(path.into(), "let init = 1".into())]).unwrap_err();
            assert!(!error.is_empty(), "{path} must be refused");
        }
    }

    #[test]
    fn saving_writes_the_sources_and_a_manifest() {
        let dir = std::env::temp_dir().join(format!("functor-mcp-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let written = super::save_project_to(
            &dir,
            &[
                ("game.fun".into(), "let init = 1".into()),
                ("util.fun".into(), "let k = 2".into()),
            ],
            None,
            false,
        )
        .unwrap();

        assert_eq!(written, vec!["game.fun", "util.fun", "functor.json"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "let init = 1"
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("functor.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["entry"], "game.fun");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A MULTI-ENTRY project must survive the round trip. `GET /project`
    /// reports modules, not project metadata, so a reconstructed manifest
    /// would say `{"entry": "game.fun"}` — a directory that builds green and
    /// then runs the client when asked for the server. The session's own
    /// manifest is written verbatim instead, roles and all.
    #[test]
    fn saving_preserves_the_multi_entry_manifest_the_session_booted_with() {
        let dir = std::env::temp_dir().join(format!("functor-mcp-save-mp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let booted = "{\n  \"language\": \"functor-lang\",\n  \"entries\": {\n    \
\"client\": { \"file\": \"game.fun\", \"module\": \"Client\" },\n    \
\"server\": { \"file\": \"game.fun\", \"module\": \"Server\" }\n  }\n}\n";

        super::save_project_to(
            &dir,
            &[("game.fun".into(), "let x = 1".into())],
            Some(booted),
            false,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("functor.json")).unwrap(),
            booted,
            "the manifest is written verbatim, not reconstructed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A manifest that no longer describes the file set is REFUSED, not
    /// quietly adjusted: writing it would produce exactly the failure the
    /// preservation exists to prevent, one step later.
    #[test]
    fn saving_refuses_a_manifest_that_names_a_module_the_session_no_longer_has() {
        let dir = std::env::temp_dir().join(format!("functor-mcp-save-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let error = super::save_project_to(
            &dir,
            &[("game.fun".into(), "let x = 1".into())],
            Some("{\"language\":\"functor-lang\",\"entries\":{\"client\":\"client.fun\"}}"),
            false,
        )
        .unwrap_err();

        assert!(error.contains("client.fun"), "{error}");
        assert!(!dir.join("functor.json").exists(), "nothing was written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other way to end up running the wrong role: `overwrite` a directory
    /// that keeps its OWN manifest. The sources become this session's, the
    /// manifest stays whoever's it was — so a disagreement is refused, not
    /// written.
    #[test]
    fn overwriting_refuses_a_directory_whose_manifest_declares_other_entries() {
        let dir = std::env::temp_dir().join(format!("functor-mcp-save-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("functor.json"), "{\"entry\":\"game.fun\"}").unwrap();
        std::fs::write(dir.join("game.fun"), "someone else's game").unwrap();

        let error = super::save_project_to(
            &dir,
            &[("game.fun".into(), "let x = 1".into())],
            Some("{\"language\":\"functor-lang\",\"entries\":{\"client\":{\"file\":\"game.fun\",\"module\":\"Client\"},\"server\":{\"file\":\"game.fun\",\"module\":\"Server\"}}}"),
            true,
        )
        .unwrap_err();

        assert!(error.contains("wrong entry"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "someone else's game",
            "nothing was written"
        );

        // The same directory, same declared entries: saving over it is fine.
        std::fs::write(dir.join("functor.json"), "{\"entry\":\"game.fun\"}").unwrap();
        super::save_project_to(
            &dir,
            &[("game.fun".into(), "let x = 1".into())],
            Some("{\"language\":\"functor-lang\",\"entry\":\"game.fun\"}"),
            true,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "let x = 1"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An ATTACHED session's manifest is not knowable over the wire, so the
    /// synthesized single-entry one remains the honest answer there.
    #[test]
    fn saving_an_attached_session_still_synthesizes_a_single_entry_manifest() {
        let dir = std::env::temp_dir().join(format!("functor-mcp-save-att-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        super::save_project_to(&dir, &[("game.fun".into(), "let x = 1".into())], None, false)
            .unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("functor.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["entry"], "game.fun");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The saved directory must BE the running program — `file = module`, so a
    /// module the session dropped would still load from a stale file.
    #[test]
    fn overwriting_replaces_the_sources_and_removes_modules_the_session_lost() {
        let dir =
            std::env::temp_dir().join(format!("functor-mcp-save-again-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        super::save_project_to(
            &dir,
            &[
                ("game.fun".into(), "let init = 1".into()),
                ("util.fun".into(), "let k = 2".into()),
            ],
            None,
            false,
        )
        .unwrap();
        std::fs::write(dir.join("functor.json"), "{\"entry\":\"game.fun\"}").unwrap();

        super::save_project_to(&dir, &[("game.fun".into(), "let init = 2".into())], None, true)
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "let init = 2"
        );
        assert!(
            !dir.join("util.fun").exists(),
            "a dropped module is removed"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("functor.json")).unwrap(),
            "{\"entry\":\"game.fun\"}",
            "an existing manifest is never rewritten"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nearly every Functor project's entry is named `game.fun`, so a matching
    /// entry name is no evidence that this is the same project: saving into an
    /// occupied directory takes an explicit `overwrite`.
    #[test]
    fn saving_refuses_a_directory_that_already_holds_a_project() {
        let dir =
            std::env::temp_dir().join(format!("functor-mcp-save-other-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("functor.json"), "{}").unwrap();
        std::fs::write(dir.join("game.fun"), "someone else's game").unwrap();

        let error =
            super::save_project_to(&dir, &[("game.fun".into(), "let init = 1".into())], None, false)
                .unwrap_err();

        assert!(error.contains("already holds a project"), "{error}");
        assert!(error.contains("overwrite"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "someone else's game",
            "nothing may be written"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_are_short_sequential_and_resolve_to_their_url() {
        let mut registry = Registry::default();
        let first = registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        let alias = registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        let second = registry.insert("http://127.0.0.1:2".into(), None, None, None, None);

        assert_eq!(first, "s1");
        assert_eq!(alias, "s2");
        assert_eq!(second, "s3");
        assert_eq!(registry.url("s3").unwrap(), "http://127.0.0.1:2");
        let first_target = registry.target("s1").unwrap();
        let alias_target = registry.target("s2").unwrap();
        let second_target = registry.target("s3").unwrap();
        assert!(
            Arc::ptr_eq(&first_target.operation_gate, &alias_target.operation_gate),
            "exact normalized URL aliases serialize on the same operation gate"
        );
        assert!(
            !Arc::ptr_eq(&first_target.closing, &alias_target.closing),
            "aliases remain independently stoppable session ids"
        );
        assert!(
            Arc::ptr_eq(&first_target.quarantined, &alias_target.quarantined),
            "exact-URL aliases share an abort-safety quarantine"
        );
        assert!(
            !Arc::ptr_eq(&first_target.quarantined, &second_target.quarantined),
            "different exact URLs have independent quarantine state"
        );
        assert!(
            !Arc::ptr_eq(&first_target.operation_gate, &second_target.operation_gate),
            "different exact URLs must never block each other"
        );

        first_target.quarantined.store(true, Ordering::Release);
        let Err(first_error) = registry.target("s1") else {
            panic!("the quarantined owner must reject new operations");
        };
        let Err(alias_error) = registry.target("s2") else {
            panic!("the quarantined alias must reject new operations");
        };
        assert!(first_error.contains("quarantined"), "{first_error}");
        assert!(alias_error.contains("quarantined"), "{alias_error}");
        assert!(registry.target("s3").is_ok());
        assert!(
            registry.begin_stop("s1").is_ok(),
            "quarantine must block mutations without preventing explicit cleanup"
        );
    }

    #[tokio::test]
    async fn cancellation_wins_while_a_mutating_request_waits_for_the_gate() {
        let mut registry = Registry::default();
        registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        let target = registry.target("s1").unwrap();
        let held = target.operation_gate.clone().lock_owned().await;
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let waiter = tokio::spawn(async move {
            acquire_target_operation(&target, async {
                let _ = cancelled.await;
            })
            .await
        });

        tokio::task::yield_now().await;
        cancel.send(()).unwrap();
        let error = waiter.await.unwrap().unwrap_err();
        assert!(error.contains("cancelled before"), "{error}");
        drop(held);
    }

    #[test]
    fn operation_boundary_is_absolute_and_observes_stop() {
        let target = SessionTarget {
            url: "http://127.0.0.1:1".into(),
            operation_gate: Arc::new(tokio::sync::Mutex::new(())),
            quarantined: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        };
        let deadline_error =
            ensure_operation_active(&target, std::time::Instant::now()).unwrap_err();
        assert!(
            deadline_error.contains("wall-clock deadline"),
            "{deadline_error}"
        );

        target.closing.store(true, Ordering::Release);
        let stop_error = ensure_operation_active(
            &target,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        )
        .unwrap_err();
        assert!(stop_error.contains("safe boundary"), "{stop_error}");
    }

    #[tokio::test]
    async fn stop_closing_rejects_queued_clones_but_not_an_alias_id() {
        let mut registry = Registry::default();
        registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        let queued = registry.target("s1").unwrap();
        let held = queued.operation_gate.clone().lock_owned().await;
        let waiter =
            tokio::spawn(
                async move { acquire_target_operation(&queued, std::future::pending()).await },
            );
        tokio::task::yield_now().await;

        let (stopping, owned) = registry.begin_stop("s1").unwrap();
        assert!(!owned);
        let Err(stopping_error) = registry.target("s1") else {
            panic!("the stopping id must reject new targets");
        };
        assert!(stopping_error.contains("stopping"), "{stopping_error}");
        let alias = registry.target("s2").expect("the alias id remains valid");
        assert!(Arc::ptr_eq(&stopping.operation_gate, &alias.operation_gate));

        drop(held);
        let error = waiter.await.unwrap().unwrap_err();
        assert!(error.contains("queued operation did not run"), "{error}");
        registry.remove("s1").unwrap();
        assert_eq!(registry.url("s2").unwrap(), "http://127.0.0.1:1");
    }

    #[tokio::test]
    async fn owned_stop_closes_a_queued_connect_and_keeps_tombstones_until_cleanup() {
        let server = FunctorMcp::new();
        {
            let mut registry = server.sessions.lock().unwrap();
            registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
            registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
            registry.sessions.get_mut("s1").unwrap().owned = true;
        }
        let reservation = server.reserve_connect("http://127.0.0.1:1");
        let target = reservation.target();
        let held = target.operation_gate.clone().lock_owned().await;
        let waiter =
            tokio::spawn(
                async move { acquire_target_operation(&target, std::future::pending()).await },
            );
        tokio::task::yield_now().await;

        let stopping = {
            let mut registry = server.sessions.lock().unwrap();
            let (stopping, owned) = registry.begin_stop("s1").unwrap();
            assert!(owned);
            assert!(registry.target("s2").is_err(), "owned stop closes aliases");
            assert!(
                registry
                    .pending_connects
                    .get("http://127.0.0.1:1")
                    .unwrap()
                    .closing
                    .load(Ordering::Acquire),
                "owned stop closes the pending connect lifecycle"
            );
            stopping
        };
        assert_eq!(stopping.url, "http://127.0.0.1:1");

        drop(held);
        let error = waiter.await.unwrap().unwrap_err();
        assert!(error.contains("queued operation did not run"), "{error}");
        let error = reservation.finish().unwrap_err();
        assert!(error.contains("stopping"), "{error}");
        let mut registry = server.sessions.lock().unwrap();
        assert!(
            registry.pending_connects.is_empty(),
            "the transient lifecycle is released"
        );
        assert_eq!(
            registry.sessions.len(),
            2,
            "owner and alias remain closing tombstones until cleanup"
        );
        registry.remove_url("http://127.0.0.1:1");
        assert!(registry.sessions.is_empty());
    }

    #[test]
    fn an_unknown_id_names_the_sessions_that_exist() {
        let mut registry = Registry::default();
        let empty = registry.url("s1").unwrap_err();
        assert!(empty.contains("no sessions yet"), "{empty}");

        registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        registry.insert("http://127.0.0.1:2".into(), None, None, None, None);
        let unknown = registry.url("s9").unwrap_err();
        assert!(unknown.contains("s1, s2"), "{unknown}");
    }

    #[test]
    fn a_reserved_port_is_held_until_its_session_is_removed() {
        let mut registry = Registry::default();
        let port = registry.reserve_port().unwrap();
        assert!(registry.reserved.contains(&port));

        // A second reservation cannot be handed the same port, even though the
        // first one's listener is already closed.
        let other = registry.reserve_port().unwrap();
        assert_ne!(port, other);

        registry.insert("http://127.0.0.1:1".into(), Some(port), None, None, None);
        registry.remove("s1").unwrap();
        assert!(!registry.reserved.contains(&port));
    }

    #[test]
    fn removing_a_session_forgets_it_and_does_not_reuse_its_id() {
        let mut registry = Registry::default();
        registry.insert("http://127.0.0.1:1".into(), None, None, None, None);
        let removed = registry.remove("s1").unwrap();

        assert_eq!(removed.url, "http://127.0.0.1:1");
        assert!(registry.url("s1").is_err());
        assert_eq!(
            registry.insert("http://127.0.0.1:3".into(), None, None, None, None),
            "s2"
        );
        let Err(message) = registry.remove("s1") else {
            panic!("a removed id must not resolve again");
        };
        assert!(message.contains("unknown session"), "{message}");
    }

    // ---------------------------------------------------- session groups

    fn packet(from: &str, to: &str, seq: u64) -> crate::commands::mcp_net::Packet {
        crate::commands::mcp_net::Packet {
            seq,
            at_ms: 0.0,
            frame: None,
            from: from.into(),
            to: to.into(),
            conn: 1,
            kind: "message",
            size: 1,
            payload_text: Some("x".into()),
        }
    }

    /// A group's default order puts the AUTHORITY first: a client's `Connect`
    /// has nothing to resolve until something is listening, and the same order
    /// is what `step_all` wants.
    #[test]
    fn the_default_role_order_launches_the_server_first() {
        let declared = vec!["client".to_string(), "server".to_string()];
        assert_eq!(group_roles(None, &declared).unwrap(), ["server", "client"]);
    }

    /// Explicit roles are verbatim — including repeats, which is the only way
    /// to ask for two clients — and an unknown one is refused before anything
    /// launches.
    #[test]
    fn explicit_roles_are_taken_verbatim_and_checked() {
        let declared = vec!["client".to_string(), "server".to_string()];
        let asked = ["server".to_string(), "client".to_string(), "client".to_string()];
        assert_eq!(group_roles(Some(&asked), &declared).unwrap(), asked);

        let typo = ["sevrer".to_string()];
        let error = group_roles(Some(&typo), &declared).unwrap_err();
        assert!(error.contains("unknown role"), "{error}");
        assert!(group_roles(Some(&[]), &declared).is_err());
    }

    /// Labels are what the wire log reads by, so a repeated role has to become
    /// two distinguishable names while a unique one stays plain.
    #[test]
    fn repeated_roles_get_numbered_labels() {
        let roles: Vec<String> = ["server", "client", "client"]
            .iter()
            .map(|r| r.to_string())
            .collect();
        assert_eq!(group_labels(&roles), ["server", "client1", "client2"]);
    }

    #[test]
    fn a_project_with_no_entries_is_not_a_group() {
        let error = declared_entries(r#"{"entry":"game.fun"}"#).unwrap_err();
        assert!(error.contains("no `entries`"), "{error}");
        assert_eq!(
            declared_entries(r#"{"entries":{"client":{"file":"game.fun","module":"Client"},"server":{"file":"game.fun","module":"Server"}}}"#).unwrap(),
            ["client", "server"]
        );
    }

    /// The wire filter resolves against the group's real labels, so a typo is
    /// an ERROR rather than an empty result a caller reads as "no traffic".
    #[test]
    fn the_wire_filter_refuses_labels_the_group_does_not_have() {
        let known = vec!["server".to_string(), "client1".to_string()];
        let error = WireFilter::parse(Some("cleint1"), None, &known).unwrap_err();
        assert!(error.contains("unknown session label"), "{error}");
        let error = WireFilter::parse(Some("server"), Some("upward"), &known).unwrap_err();
        assert!(error.contains("unknown direction"), "{error}");
        // A direction with nothing to be relative to is a mistake, not a
        // silently-ignored argument.
        assert!(WireFilter::parse(None, Some("sent"), &known).is_err());
        assert!(WireFilter::parse(Some("server:client1"), Some("sent"), &known).is_err());
    }

    #[test]
    fn the_wire_filter_selects_by_link_and_direction() {
        let known = vec!["server".to_string(), "client1".to_string(), "client2".to_string()];
        let rows = [
            packet("client1", "server", 0),
            packet("server", "client1", 1),
            packet("server", "client2", 2),
        ];
        let matching = |filter: WireFilter| -> Vec<u64> {
            rows.iter().filter(|p| filter.matches(p)).map(|p| p.seq).collect()
        };
        assert_eq!(matching(WireFilter::parse(None, None, &known).unwrap()), [0, 1, 2]);
        assert_eq!(
            matching(WireFilter::parse(Some("client1"), None, &known).unwrap()),
            [0, 1]
        );
        assert_eq!(
            matching(WireFilter::parse(Some("client1"), Some("sent"), &known).unwrap()),
            [0]
        );
        assert_eq!(
            matching(WireFilter::parse(Some("client1"), Some("received"), &known).unwrap()),
            [1]
        );
        assert_eq!(
            matching(WireFilter::parse(Some("server:client2"), None, &known).unwrap()),
            [2]
        );
    }

    /// `/net/deliver` caps its body, so a backlog must be SPLIT rather than
    /// posted whole: posting whole would 413, be re-queued whole, and 413 again
    /// every round — that session's inbound dead for the rest of the run.
    #[test]
    fn a_delivery_batch_is_split_below_the_endpoint_limit() {
        use functor_runtime_common::net::DeliveredEvent;
        let event = |index: usize| DeliveredEvent::Message {
            key: "ws://127.0.0.1:9101/orbs".into(),
            conn: 1,
            text: format!("{index}").repeat(400),
        };
        let events: Vec<DeliveredEvent> = (0..200).map(event).collect();
        let prefix = deliverable_prefix(&events);
        assert!(prefix > 0 && prefix < events.len(), "split at {prefix}");
        assert!(
            serde_json::to_string(&events[..prefix]).unwrap().len() <= MAX_DELIVERY_BYTES + 4096,
            "the chosen prefix stays near the budget"
        );

        // A small backlog is one body.
        let small: Vec<DeliveredEvent> = (0..3).map(event).collect();
        assert_eq!(deliverable_prefix(&small), 3);

        // A SINGLE event over the budget still goes alone: the runtime's own
        // 413 is then the loud answer, rather than a batch stuck forever.
        let huge = vec![DeliveredEvent::Message {
            key: "k".into(),
            conn: 1,
            text: "x".repeat(MAX_DELIVERY_BYTES * 2),
        }];
        assert_eq!(deliverable_prefix(&huge), 1);
    }
}
