//! Spike S2: start a hidden game, serve one MCP tool (`game_state`) over
//! loopback streamable HTTP with a bearer token, run an agent through
//! `mis-agent` against it, and print every `AgentEvent` as a JSON line.
//!
//! Usage: s2-agent <claude|codex> <read|edit>
//! Run from anywhere; paths resolve against the repo root (three levels up).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use mis_agent::{Agent, Mcp, RunOpts};
use mis_types::{PermissionMode, Permissions};
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// The one game tool: proxies the debug server's `GET /state`.
#[derive(Clone)]
struct GameTools {
    http: reqwest::Client,
    base: String,
    calls: Arc<AtomicUsize>,
}

#[tool_router]
impl GameTools {
    /// Read the running game's current state: `frame` (sim steps so far),
    /// `tts` (seconds since start) and `model` (the game's model as JSON).
    #[tool]
    async fn game_state(&self) -> Result<CallToolResult, ErrorData> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let state: Value = match self.state().await {
            Ok(v) => v,
            Err(e) => {
                let text = format!("GET /state failed: {e:#}");
                return Ok(CallToolResult::error(vec![ContentBlock::text(text)]));
            }
        };
        let subset = json!({
            "frame": state["frame"],
            "tts": state["tts"],
            "model": state["model"],
        });
        eprintln!("[s2] game_state call #{n} -> {subset}");
        Ok(CallToolResult::success(vec![ContentBlock::text(
            subset.to_string(),
        )]))
    }
}

impl GameTools {
    async fn state(&self) -> anyhow::Result<Value> {
        let url = format!("{}/state", self.base);
        Ok(self.http.get(url).send().await?.error_for_status()?.json().await?)
    }
}

#[tool_handler(name = "functor-s2", instructions = "Read a running Functor game's state.")]
impl ServerHandler for GameTools {}

/// Refuse any request without `Authorization: Bearer <token>`.
async fn bearer(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    let want = format!("Bearer {token}");
    let got = req.headers().get("authorization").and_then(|v| v.to_str().ok());
    if got != Some(want.as_str()) {
        eprintln!("[s2] refused MCP request without the bearer token");
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body("unauthorized".into())
            .unwrap();
    }
    next.run(req).await
}

/// Serve `tools` at `http://127.0.0.1:<port>/mcp`; returns the URL.
async fn serve_mcp(tools: GameTools, token: String) -> anyhow::Result<String> {
    let service: StreamableHttpService<GameTools, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let app = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(Arc::new(token), bearer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(url)
}

/// Start `functor run native --hidden --debug-port 0` and return the child and
/// the debug server's base URL, read from its `[debug-server] listening` line.
async fn start_game(root: &Path) -> anyhow::Result<(tokio::process::Child, String)> {
    let mut child = Command::new(root.join("target/release/functor"))
        .args(["-d", "examples/primitives", "run", "native", "--hidden", "--debug-port", "0"])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning functor")?;
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    const MARK: &str = "[debug-server] listening on ";
    let base = loop {
        let Some(line) = lines.next_line().await? else {
            bail!("functor exited before its debug server started");
        };
        if let Some(i) = line.find(MARK) {
            break line[i + MARK.len()..].trim().to_string();
        }
    };
    // Keep draining so the child never blocks on a full stderr pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok((child, base))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (backend, mode) = match &args[1..] {
        [b, m] => (b.as_str(), m.as_str()),
        _ => bail!("usage: s2-agent <claude|codex> <read|edit>"),
    };
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize()?;

    let (mut game, base) = start_game(&root).await?;
    eprintln!("[s2] game debug server at {base}");

    let calls = Arc::new(AtomicUsize::new(0));
    let tools = GameTools {
        http: reqwest::Client::new(),
        base,
        calls: calls.clone(),
    };
    let token = uuid::Uuid::new_v4().to_string();
    let url = serve_mcp(tools, token.clone()).await?;
    eprintln!("[s2] MCP server at {url}");

    // A fresh, empty working directory for the agent.
    let work = std::env::temp_dir().join(format!("s2-agent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;
    let work = work.canonicalize()?;
    let scratch = work.join("scratch.txt");
    std::fs::write(&scratch, "hello\n")?;
    eprintln!("[s2] working directory {}", work.display());

    let prompt = match mode {
        "read" => "Call the game_state tool and tell me the current frame number and one fact about the model.",
        "edit" => "Call the game_state tool, then replace the single line in scratch.txt (in the current directory) with `frame=<N>` where N is the frame number it returned. Use your file edit tool, not the shell. Then say what you wrote.",
        _ => bail!("mode must be read or edit"),
    };
    // Folders mode, no extra folders: the cwd is writable, nothing else.
    let permissions = Permissions {
        mode: PermissionMode::Folders,
        ..Default::default()
    };
    mis_agent::check(&permissions).map_err(anyhow::Error::msg)?;
    let mcp = Mcp {
        url,
        token,
    };
    let cwd = work.to_string_lossy().to_string();
    let opts = RunOpts {
        cwd: Some(&cwd),
        mcp: Some(&mcp),
        permissions: Some(&permissions),
        read_only: mode == "read",
        ..Default::default()
    };
    let agent: Box<dyn Agent> = match backend {
        "claude" => Box::new(mis_agent::claude::ClaudeAgent::default()),
        "codex" => Box::new(mis_agent::codex::CodexAgent {
            plan_tool: mis_agent::codex::probe_plan_tool(),
        }),
        _ => bail!("backend must be claude or codex"),
    };
    let argv = match backend {
        "claude" => mis_agent::claude::ClaudeAgent::default().argv(prompt, &opts),
        _ => mis_agent::codex::CodexAgent::default().argv(prompt, &opts),
    };
    eprintln!("[s2] argv: {}", serde_json::to_string(&argv)?);

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let task = uuid::Uuid::new_v4();
    let started = std::time::Instant::now();
    let run = mis_agent::run_guarded(agent.as_ref(), task, prompt, opts, tx);
    let print = async {
        while let Some(ev) = rx.recv().await {
            let line = json!({"ms": started.elapsed().as_millis() as u64, "event": ev});
            println!("{line}");
        }
    };
    tokio::join!(run, print);

    eprintln!("[s2] game_state calls served: {}", calls.load(Ordering::SeqCst));
    eprintln!("[s2] scratch.txt now: {:?}", std::fs::read_to_string(&scratch)?);
    game.kill().await.ok();
    std::fs::remove_dir_all(&work).ok();
    Ok(())
}
