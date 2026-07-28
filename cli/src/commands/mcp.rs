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
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use functor_docgen::{ApiItem, ApiReference};
use functor_runtime_common::debug_protocol::DEBUG_PROTOCOL_SERVICE;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use super::automation::{
    canonical_code, limits as automation_limits, model_value_at, parse_automation,
    usage as automation_usage, AutomationPlan, AutomationStep, AUTOMATION_DIALECT,
};

/// How long `launch_game` waits for a spawned runtime to answer discovery.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `step` tolerates a queued batch making NO progress before giving
/// up. A large batch legitimately takes far longer than this to drain in total.
const STEP_STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-request timeout for the (loopback or adb-forwarded) debug server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Lowest debug-protocol version whose contract these tools actually keep
/// (`pending_steps` on `/state`, `frames` on `/time advance`, `model`).
const REQUIRED_PROTOCOL_VERSION: u64 = 4;
/// Bytes of a launched child's stdout/stderr kept for failure reporting.
const LOG_TAIL_BYTES: usize = 8 * 1024;
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
    /// Ports handed to a launch. The OS never offers a port a live runtime
    /// already holds, but it happily offers the same one twice inside the
    /// window between `free_port` and the child's own bind — and MCP tool calls
    /// can overlap, which is exactly the two-game case sessions exist for.
    reserved: BTreeSet<u16>,
}

impl Registry {
    fn insert(
        &mut self,
        url: String,
        port: Option<u16>,
        child: Option<Child>,
        scratch: Option<ScratchDir>,
    ) -> String {
        self.next_id += 1;
        let id = format!("s{}", self.next_id);
        self.sessions.insert(
            id.clone(),
            Session {
                url,
                port,
                child,
                scratch,
            },
        );
        id
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
        match self.sessions.get(id) {
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

    fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
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
    /// Named entry, for a project whose functor.json declares `entries`.
    pub entry: Option<String>,
    /// `"hidden"` (default) renders into an invisible window — `capture_frame`
    /// works. `"headless"` creates no GL context at all — no display or GPU
    /// needed, but `capture_frame` then fails.
    pub mode: Option<String>,
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
    /// How many steps to queue (default 1).
    pub frames: Option<u32>,
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
pub struct ValidateAutomationCodeArgs {
    /// One restricted JavaScript-shaped builder expression, for example:
    /// `automation("proof").pause().keyDown("w").step().expectModel("held.w", true)`.
    /// It is parsed into data and is never evaluated as JavaScript.
    pub code: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunAutomationCodeArgs {
    /// Session id from `launch_game` / `connect_game`.
    pub session: String,
    /// A complete restricted automation builder expression. The entire plan is
    /// parsed and validated before the first runtime request is made.
    pub code: String,
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

    /// Parse restricted TypeScript/JavaScript-shaped Functor automation source
    /// into a serializable plan, without a session and without side effects.
    /// The source must be ONE `automation("name").method(...)` chain. Allowed
    /// methods: pause, keyDown/keyUp/pressKey, mouseMove/mouseDown/mouseUp/
    /// mouseWheel, uiClick, step, inspect, expectModel, and capture. Success
    /// returns the normalized plan, deterministic canonical source that parses
    /// back to that same plan, and used/maximum budgets. Arguments are literals;
    /// imports, variables, callbacks/functions, loops, async/await, `new`,
    /// eval/Function/require, globals, fetch/timers, dynamic properties, and
    /// unknown calls are rejected. This parser never evaluates JavaScript.
    #[tool]
    async fn validate_automation_code(
        &self,
        Parameters(args): Parameters<ValidateAutomationCodeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        match parse_automation(&args.code) {
            Ok(plan) => ok_text(
                serde_json::json!({
                    "valid": true,
                    "dialect": AUTOMATION_DIALECT,
                    "canonical_code": canonical_code(&plan),
                    "budget": {
                        "used": automation_usage(&args.code, &plan),
                        "limits": automation_limits(),
                    },
                    "plan": plan,
                })
                .to_string(),
            ),
            Err(diagnostic) => ok_text(
                serde_json::json!({
                    "valid": false,
                    "dialect": AUTOMATION_DIALECT,
                    "errors": [diagnostic],
                    "budget": {
                        "used": { "source_bytes": args.code.len() },
                        "limits": automation_limits(),
                    },
                })
                .to_string(),
            ),
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
        let port = resolve!(self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .reserve_port());
        let exe = resolve!(std::env::current_exe()
            .map_err(|error| format!("cannot locate the functor executable: {error}")));

        let mut command = Command::new(exe);
        command.arg("-d").arg(&dir);
        if let Some(entry) = &args.entry {
            command.arg("--entry").arg(entry);
        }
        command
            .args(["run", "native", "--debug-port"])
            .arg(port.to_string())
            .arg(mode_flag)
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
                return tool_error(format!("failed to spawn the runtime: {error}"));
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
                return tool_error(format!("{message}\n\nruntime output:\n{}", tail(&log)));
            }
        };

        let id = self.sessions.lock().expect("mcp registry poisoned").insert(
            url.clone(),
            Some(port),
            Some(child),
            scratch,
        );
        ok_text(
            serde_json::json!({
                "session": id,
                "url": url,
                "port": port,
                "mode": mode,
                "owned": true,
                "dir": absolute(&dir),
                "discovery": discovery,
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
    ) -> Result<CallToolResult, ErrorData> {
        let url = args.url.trim_end_matches('/').to_string();
        let discovery = resolve!(self.discover(&url).await);
        let id = self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .insert(url.clone(), None, None, None);
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
    /// and whether the runtime currently answers `GET /state`.
    #[tool]
    async fn list_sessions(&self) -> Result<CallToolResult, ErrorData> {
        let known: Vec<(String, String, bool)> = {
            let guard = self.sessions.lock().expect("mcp registry poisoned");
            guard
                .sessions
                .iter()
                .map(|(id, session)| (id.clone(), session.url.clone(), session.child.is_some()))
                .collect()
        };
        let mut sessions = Vec::with_capacity(known.len());
        for (id, url, owned) in known {
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
            }));
        }
        ok_text(serde_json::json!({ "sessions": sessions }).to_string())
    }

    /// Drop a session. A launched (owned) game is killed; an attached one is
    /// only forgotten.
    #[tool]
    async fn stop_game(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut session = resolve!(self
            .sessions
            .lock()
            .expect("mcp registry poisoned")
            .remove(&args.session));
        let owned = session.child.is_some();
        if let Some(child) = session.child.as_mut() {
            let _ = child.start_kill();
            let _ = child.wait().await;
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
    /// fails — relaunch it in `hidden` mode.
    #[tool]
    async fn capture_frame(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(&args.session));
        let body = resolve!(self.capture_png(&url).await);
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
    /// `{"type":"xr", "head":…, "left":…, "right":…}`, `{"type":"xr_clear"}`.
    /// The list mirrors `POST /input` and is not exhaustive; an unrecognized
    /// shape comes back as the runtime's own 400, which names the problem.
    /// Keys, held buttons and XR samples are LEVEL state: they stay in force
    /// across steps until released, which is how a paused session is scripted.
    #[tool]
    async fn send_input(
        &self,
        Parameters(args): Parameters<InputArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_post(&args.session, "/input", args.command.to_string())
            .await
    }

    /// Parse and run one restricted automation builder expression against a
    /// session. The whole source is lowered to a bounded, serializable plan
    /// BEFORE session lookup or any runtime request; it is never evaluated as
    /// JavaScript. This collapses the deterministic jam loop into one call:
    /// `automation("proof").pause().keyDown("w").step().expectModel("x", 1)`.
    /// `inspect` snapshots state in the result; `capture` appends PNG image
    /// blocks and needs a hidden (not headless) session. Execution is ordered
    /// but not transactional: a runtime failure can leave earlier valid steps
    /// applied. Call `validate_automation_code` first for parse-only feedback.
    #[tool]
    async fn run_automation_code(
        &self,
        Parameters(args): Parameters<RunAutomationCodeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        // This ordering is the security boundary: syntax, allowlists, literal
        // shapes, and every resource budget are settled before even resolving
        // a session, let alone issuing an HTTP request.
        let plan = match parse_automation(&args.code) {
            Ok(plan) => plan,
            Err(diagnostic) => {
                return tool_error(
                    serde_json::json!({
                        "valid": false,
                        "dialect": AUTOMATION_DIALECT,
                        "errors": [diagnostic],
                    })
                    .to_string(),
                )
            }
        };
        let url = resolve!(self.url(&args.session));
        match self.execute_automation(&url, &plan).await {
            Ok((summary, captures)) => {
                let mut content = vec![ContentBlock::text(summary.to_string())];
                content.extend(captures.into_iter().map(|(_, png)| {
                    ContentBlock::image(
                        base64::engine::general_purpose::STANDARD.encode(png),
                        "image/png",
                    )
                }));
                Ok(CallToolResult::success(content))
            }
            Err(message) => tool_error(message),
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
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(&args.session));
        ok_text(resolve!(self.pin(&url, args.tts).await))
    }

    /// Run exactly `frames` simulation steps of `dts` seconds each, WAIT for
    /// them to land (polling until `pending_steps` is 0), then return the fresh
    /// `/state`. Step one frame at a time when the game must see input or I/O
    /// between steps — a batch runs up to 8 ticks per rendered frame, so it has
    /// proportionally fewer input/network/render points.
    #[tool]
    async fn step(
        &self,
        Parameters(args): Parameters<StepArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(&args.session));
        let state = resolve!(
            self.advance(&url, args.dts.unwrap_or(0.016), args.frames.unwrap_or(1))
                .await
        );
        ok_text(state.to_string())
    }

    /// Un-pin the clock: the game follows wall-clock time again, and window
    /// input reaches it once more.
    #[tool]
    async fn resume(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_post(
            &args.session,
            "/time",
            serde_json::json!({ "type": "resume" }).to_string(),
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
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(&args.session));
        let tts = resolve!(tts_of(&resolve!(self.state(&url).await)));
        // The clock must be pinned before a rewind or the next wall-clock frame
        // would immediately overwrite the restored model. `/state` does not
        // report whether it already is, so this re-pins at the CURRENT time —
        // a no-op for an already-paused session.
        resolve!(
            self.post(
                &url,
                "/time",
                serde_json::json!({ "type": "set", "tts": tts }).to_string(),
            )
            .await
        );
        resolve!(
            self.post(
                &url,
                "/rewind",
                serde_json::json!({ "frame": args.frame }).to_string(),
            )
            .await
        );
        ok_text(resolve!(self.state(&url).await).to_string())
    }

    /// Hot-reload the entry module from new source, preserving the live model.
    /// A source error is returned verbatim (the runtime keeps running the old
    /// program). Use `reload_project` when sibling modules changed too.
    #[tool]
    async fn reload_source(
        &self,
        Parameters(args): Parameters<ReloadSourceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.proxy_post(&args.session, "/reload-source", args.source)
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
    /// touched a file. A single-entry `functor.json` is synthesized when the
    /// directory has none (a project's own manifest is never rewritten, and a
    /// multi-entry one is not reconstructed). A directory that already holds a
    /// project is REFUSED unless `overwrite` is true.
    #[tool]
    async fn save_project(
        &self,
        Parameters(args): Parameters<SaveProjectArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(&args.session));
        let body = match self.get(&url, "/project").await {
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
        let files: Vec<(String, String)> = resolve!(serde_json::from_str(&body).map_err(
            |error| format!("GET /project did not return [path, source] pairs: {error}")
        ));
        let dir = std::path::PathBuf::from(&args.dir);
        let written = resolve!(save_project_to(
            &dir,
            &files,
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
    ) -> Result<CallToolResult, ErrorData> {
        let files: Vec<Vec<String>> = args
            .files
            .into_iter()
            .map(|(path, source)| vec![path, source])
            .collect();
        self.proxy_post(
            &args.session,
            "/reload-project",
            serde_json::to_string(&files).expect("string pairs serialize"),
        )
        .await
    }
}

impl FunctorMcp {
    /// The prelude API reference, generated once per process.
    fn api_docs(&self) -> Result<&ApiReference, String> {
        self.docs
            .get_or_init(|| {
                functor_docgen::generate()
                    .map_err(|error| {
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

    fn url(&self, session: &str) -> Result<String, String> {
        self.sessions
            .lock()
            .expect("mcp registry poisoned")
            .url(session)
    }

    async fn proxy_get(&self, session: &str, path: &str) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(session));
        ok_text(resolve!(self.get(&url, path).await))
    }

    async fn proxy_post(
        &self,
        session: &str,
        path: &str,
        body: String,
    ) -> Result<CallToolResult, ErrorData> {
        let url = resolve!(self.url(session));
        ok_text(resolve!(self.post(&url, path, body).await))
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

    async fn advance(&self, url: &str, dts: f64, frames: u32) -> Result<Value, String> {
        let body = serde_json::json!({
            "type": "advance",
            "dts": dts,
            "frames": frames,
        });
        self.post(url, "/time", body.to_string()).await?;
        // The timeout is a STALL timeout, not a total one: a large batch drains
        // at up to 8 steps per rendered frame, so a legitimate 100k-step skip
        // takes minutes. Every observed step resets the clock; only a queue that
        // stops moving is a failure.
        let mut deadline = Instant::now() + STEP_STALL_TIMEOUT;
        let mut remaining = u64::MAX;
        loop {
            let state = self.state(url).await?;
            let pending = state["pending_steps"].as_u64().unwrap_or(0);
            if pending == 0 {
                return Ok(state);
            }
            if pending < remaining {
                remaining = pending;
                deadline = Instant::now() + STEP_STALL_TIMEOUT;
            } else if Instant::now() >= deadline {
                return Err(format!(
                    "the queued steps stopped draining ({pending} still pending, no progress for \
{}s) — the game loop may be stuck or the runtime paused from elsewhere",
                    STEP_STALL_TIMEOUT.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn capture_png(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .http
            .post(format!("{url}/capture"))
            .send()
            .await
            .map_err(|error| format!("POST /capture on {url} failed: {error}"))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("reading the captured PNG failed: {error}"))?;
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
        Ok(body.to_vec())
    }

    async fn execute_automation(
        &self,
        url: &str,
        plan: &AutomationPlan,
    ) -> Result<(Value, Vec<(Option<String>, Vec<u8>)>), String> {
        let mut observations = Vec::new();
        let mut assertions = Vec::new();
        let mut captures = Vec::new();

        for (index, step) in plan.steps.iter().enumerate() {
            let step_number = index + 1;
            let result: Result<(), String> = match step {
                AutomationStep::Pause { tts } => self.pin(url, *tts).await.map(|_| ()),
                AutomationStep::Key { key, down } => self
                    .post(
                        url,
                        "/input",
                        serde_json::json!({
                            "type": "key",
                            "key": key,
                            "down": down,
                        })
                        .to_string(),
                    )
                    .await
                    .map(|_| ()),
                AutomationStep::PressKey { key } => {
                    let press = self
                        .post(
                            url,
                            "/input",
                            serde_json::json!({
                                "type": "key",
                                "key": key,
                                "down": true,
                            })
                            .to_string(),
                        )
                        .await;
                    match press {
                        Ok(_) => {
                            let advanced = self.advance(url, 0.016, 1).await.map(|_| ());
                            let released = self
                                .post(
                                    url,
                                    "/input",
                                    serde_json::json!({
                                        "type": "key",
                                        "key": key,
                                        "down": false,
                                    })
                                    .to_string(),
                                )
                                .await
                                .map(|_| ());
                            match (advanced, released) {
                                (Ok(()), Ok(())) => Ok(()),
                                (Err(step_error), Ok(())) => Err(step_error),
                                (Ok(()), Err(release_error)) => Err(release_error),
                                (Err(step_error), Err(release_error)) => Err(format!(
                                    "{step_error}; best-effort key release also failed: {release_error}"
                                )),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                AutomationStep::MouseMove { x, y } => self
                    .post(
                        url,
                        "/input",
                        serde_json::json!({
                            "type": "mouse_move",
                            "x": x,
                            "y": y,
                        })
                        .to_string(),
                    )
                    .await
                    .map(|_| ()),
                AutomationStep::MouseButton { button, down } => self
                    .post(
                        url,
                        "/input",
                        serde_json::json!({
                            "type": "mouse_button",
                            "button": button,
                            "down": down,
                        })
                        .to_string(),
                    )
                    .await
                    .map(|_| ()),
                AutomationStep::MouseWheel { delta } => self
                    .post(
                        url,
                        "/input",
                        serde_json::json!({
                            "type": "mouse_wheel",
                            "delta": delta,
                        })
                        .to_string(),
                    )
                    .await
                    .map(|_| ()),
                AutomationStep::UiClick { slot } => self
                    .post(
                        url,
                        "/input",
                        serde_json::json!({
                            "type": "ui_event",
                            "slot": slot,
                            "kind": "Clicked",
                        })
                        .to_string(),
                    )
                    .await
                    .map(|_| ()),
                AutomationStep::Step { frames, dts } => {
                    self.advance(url, *dts, *frames).await.map(|_| ())
                }
                AutomationStep::Inspect { label } => match self.state(url).await {
                    Ok(state) => {
                        observations.push(serde_json::json!({
                            "label": label,
                            "state": state,
                        }));
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
                AutomationStep::ExpectModel { path, equals } => match self.state(url).await {
                    Ok(state) => {
                        let model = &state["model"];
                        match model_value_at(model, path) {
                            Some(actual) if actual == equals => {
                                assertions.push(serde_json::json!({
                                    "path": path,
                                    "expected": equals,
                                    "actual": actual,
                                    "passed": true,
                                }));
                                Ok(())
                            }
                            Some(actual) => Err(format!(
                                "model assertion failed at {path:?}: expected {}, got {}",
                                equals, actual
                            )),
                            None => Err(format!(
                                "model assertion path {path:?} does not exist; model is {model}"
                            )),
                        }
                    }
                    Err(error) => Err(error),
                },
                AutomationStep::Capture { label } => match self.capture_png(url).await {
                    Ok(png) => {
                        captures.push((label.clone(), png));
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                return Err(format!(
                    "automation step {step_number} ({}) failed: {error}",
                    automation_step_name(step)
                ));
            }
        }

        let final_state = self
            .state(url)
            .await
            .map_err(|error| format!("automation completed but final state failed: {error}"))?;
        let capture_metadata: Vec<Value> = captures
            .iter()
            .enumerate()
            .map(|(index, (label, png))| {
                serde_json::json!({
                    "label": label,
                    "content_index": index + 1,
                    "mime_type": "image/png",
                    "bytes": png.len(),
                })
            })
            .collect();
        let summary = serde_json::json!({
            "ok": true,
            "dialect": AUTOMATION_DIALECT,
            "plan": plan,
            "steps_executed": plan.steps.len(),
            "assertions": assertions,
            "observations": observations,
            "captures": capture_metadata,
            "final_state": final_state,
        });
        Ok((summary, captures))
    }

    /// Fetch and validate the discovery document, so an `http://…` that answers
    /// something else is rejected here rather than at the first real call.
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
        // Below v4 the guarantees these tools advertise silently stop holding:
        // a pre-v3 runtime ignores a batched `frames` and reports no
        // `pending_steps` (so `step` would claim a 10-frame batch landed after
        // running one), and a pre-v4 one sends Debug text under `model`
        // instead of structured JSON. Refuse
        // rather than mislead — this matters for a device APK, which versions
        // independently of the CLI.
        let version = discovery["protocol_version"].as_u64().unwrap_or(0);
        if version < REQUIRED_PROTOCOL_VERSION {
            return Err(format!(
                "{url} speaks debug protocol v{version}, but these tools need \
v{REQUIRED_PROTOCOL_VERSION} (batched steps report pending_steps, and /state's model \
is structured JSON). Rebuild that runtime from this version of Functor."
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

#[tool_handler(
    name = "functor",
    instructions = "Drive Functor games over their debug runtime. Launch or attach to a game \
(launch_game / connect_game), then observe it (get_state — read model — get_scene, \
get_trace, capture_frame) and drive it (pause, send_input, step, resume, rewind, \
reload_source). `validate_automation_code` and `run_automation_code` collapse a whole \
pause/input/step/assert/inspect/capture sequence into one restricted SDK builder expression \
that is parsed into data and never evaluated as JavaScript. The lower-level deterministic \
loop is pause → send_input → step → get_state: while the \
clock is pinned nothing advances on its own, and injected input is level state that holds \
across steps. language_guide teaches the LANGUAGE (Functor Lang is not F#/OCaml — read it \
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

fn automation_step_name(step: &AutomationStep) -> &'static str {
    match step {
        AutomationStep::Pause { .. } => "pause",
        AutomationStep::Key { down: true, .. } => "keyDown",
        AutomationStep::Key { down: false, .. } => "keyUp",
        AutomationStep::PressKey { .. } => "pressKey",
        AutomationStep::MouseMove { .. } => "mouseMove",
        AutomationStep::MouseButton { down: true, .. } => "mouseDown",
        AutomationStep::MouseButton { down: false, .. } => "mouseUp",
        AutomationStep::MouseWheel { .. } => "mouseWheel",
        AutomationStep::UiClick { .. } => "uiClick",
        AutomationStep::Step { .. } => "step",
        AutomationStep::Inspect { .. } => "inspect",
        AutomationStep::ExpectModel { .. } => "expectModel",
        AutomationStep::Capture { .. } => "capture",
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
    let intro = headings.first().map_or(body, |(start, _, _)| &body[..*start]);
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
        false => format!("{}\n\nContinues in: {}\n", section.text, children.join(", ")),
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

/// Persist a session's sources into `dir`, returning what was written.
///
/// A directory that already holds a project is refused unless `overwrite` —
/// "it has the same entry file name" is not evidence that it is the same
/// project, since nearly every Functor project's entry is `game.fun`. With
/// `overwrite`, modules this session does NOT have are removed as well:
/// `file = module`, so a leftover sibling would still load and the saved copy
/// would not be the program that ran.
fn save_project_to(
    dir: &std::path::Path,
    files: &[(String, String)],
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
    // The manifest is not part of what the runtime reports, so write one for a
    // project that has none — and never rewrite the one already there, which
    // may declare named entries this session's sources cannot describe.
    if !files.iter().any(|(path, _)| path == "functor.json")
        && !existing.iter().any(|name| name == "functor.json")
    {
        files.push(("functor.json".to_string(), synthesized_manifest(&entry)));
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
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("reading the {path} response failed: {error}"))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("{path} → {status}: {body}"))
    }
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
        find_guide_section, guide_sections, render_api_hits, render_guide_contents,
        render_guide_section, search_api, strip_front_matter, Registry, LANGUAGE_GUIDE,
        QUICK_FACTS_SLUG,
    };
    use functor_docgen::ApiReference;

    fn reference() -> ApiReference {
        functor_docgen::generate().expect("the embedded prelude documents itself")
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
        assert!(ambiguous.contains("sound-design, sound-effects"), "{ambiguous}");
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
            false,
        )
        .unwrap();
        std::fs::write(dir.join("functor.json"), "{\"entry\":\"game.fun\"}").unwrap();

        super::save_project_to(&dir, &[("game.fun".into(), "let init = 2".into())], true).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("game.fun")).unwrap(),
            "let init = 2"
        );
        assert!(!dir.join("util.fun").exists(), "a dropped module is removed");
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
            super::save_project_to(&dir, &[("game.fun".into(), "let init = 1".into())], false)
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
        let first = registry.insert("http://127.0.0.1:1".into(), None, None, None);
        let second = registry.insert("http://127.0.0.1:2".into(), None, None, None);

        assert_eq!(first, "s1");
        assert_eq!(second, "s2");
        assert_eq!(registry.url("s2").unwrap(), "http://127.0.0.1:2");
    }

    #[test]
    fn an_unknown_id_names_the_sessions_that_exist() {
        let mut registry = Registry::default();
        let empty = registry.url("s1").unwrap_err();
        assert!(empty.contains("no sessions yet"), "{empty}");

        registry.insert("http://127.0.0.1:1".into(), None, None, None);
        registry.insert("http://127.0.0.1:2".into(), None, None, None);
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

        registry.insert("http://127.0.0.1:1".into(), Some(port), None, None);
        registry.remove("s1").unwrap();
        assert!(!registry.reserved.contains(&port));
    }

    #[test]
    fn removing_a_session_forgets_it_and_does_not_reuse_its_id() {
        let mut registry = Registry::default();
        registry.insert("http://127.0.0.1:1".into(), None, None, None);
        let removed = registry.remove("s1").unwrap();

        assert_eq!(removed.url, "http://127.0.0.1:1");
        assert!(registry.url("s1").is_err());
        assert_eq!(
            registry.insert("http://127.0.0.1:3".into(), None, None, None),
            "s2"
        );
        let Err(message) = registry.remove("s1") else {
            panic!("a removed id must not resolve again");
        };
        assert!(message.contains("unknown session"), "{message}");
    }
}
