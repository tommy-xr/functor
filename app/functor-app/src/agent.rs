//! Agent runs: per run, a `functor mcp --http` tool server child and an agent
//! (`claude` / `codex` through `mis-agent`, or [`FakeAgent`]) offered it;
//! each agent event goes to the core as an `Event::Run`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use functor_app_core::tasks::mcp_listening_url;
use functor_app_core::{AgentEvent, Event, Harness, RunEvent, RunSpec};
use mis_agent::{run_guarded, Agent, BoxFut, Mcp, RunOpts};
use mis_types::{PermissionMode, Permissions};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// The env var `functor mcp --http` reads its bearer token from.
const TOKEN_ENV: &str = "FUNCTOR_MCP_TOKEN";
/// How long the tool server may take to print its listening line.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the tool server gets to exit after its stdin closes (it stops
/// the games it launched) before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// Tool-server output lines kept for a failure report.
const TAIL_LINES: usize = 20;

/// Told to the agent at the start of each new agent session.
const PREAMBLE: &str = "You are working on a Functor game project: this directory. \
Your MCP server `mis` has tools for its running game — state, scene, captures, input, \
launching and stopping it — and `app_context`, which says what the user is looking at \
in the Functor app. Prefer reading live state and captures over guessing. Functor Lang \
(`.fun`) is a custom language, not F#: its rules are in the repo's `functor-lang` skill \
(`.claude/skills/functor-lang/`) and its docs; read them before writing it.";

/// What a run may do unattended: folder-scoped on the repo root. Edits
/// anywhere in the repo are accepted, the game tools are allowed (`mis-agent`
/// allows its own MCP server) and anything no rule allows is denied, never
/// asked. Shell commands run sandboxed to the repo: claude's need an allow
/// rule (none here), codex runs any. This mode also loads the user's own
/// agent settings, hooks and MCP servers, as in their terminal (D19), so
/// their allow rules apply too; an isolated mode would be chosen here.
pub fn permissions(repo_root: &Path) -> Permissions {
    Permissions {
        mode: PermissionMode::Folders,
        folders: vec![repo_root.to_string_lossy().into_owned()],
        mcp: vec![],
        commands: vec![],
    }
}

/// The prompt for `message`: a new agent session gets the [`PREAMBLE`].
fn prompt(message: &str, resumed: bool) -> String {
    match resumed {
        true => message.to_string(),
        false => format!("{PREAMBLE}\n\n{message}"),
    }
}

/// Which agent runs tasks.
#[derive(Clone)]
pub enum Agents {
    /// The task's harness, on `PATH`.
    Real,
    /// [`FakeAgent`] replaying this fixture, whatever the harness.
    Fake(PathBuf),
}

impl Agents {
    fn get(&self, harness: Harness) -> Box<dyn Agent> {
        match (self, harness) {
            (Self::Fake(fixture), _) => Box::new(FakeAgent {
                fixture: fixture.clone(),
            }),
            (Self::Real, Harness::Claude) => Box::new(mis_agent::claude::ClaudeAgent::default()),
            (Self::Real, Harness::Codex) => Box::new(mis_agent::codex::CodexAgent::default()),
        }
    }
}

/// The runs in flight, by task.
pub struct Runner {
    functor: PathBuf,
    agents: Agents,
    events: UnboundedSender<Event>,
    live: HashMap<u64, Live>,
}

struct Live {
    run: u64,
    /// Fired (or dropped) to cancel the run.
    cancel: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Runner {
    pub fn new(functor: PathBuf, agents: Agents, events: UnboundedSender<Event>) -> Self {
        Self {
            functor,
            agents,
            events,
            live: HashMap::new(),
        }
    }

    pub fn start(&mut self, spec: RunSpec) {
        self.live.retain(|_, live| !live.task.is_finished());
        let (cancel, cancelled) = oneshot::channel();
        let run = spec.run;
        let id = spec.task;
        let task = tokio::spawn(run_task(
            spec,
            self.functor.clone(),
            self.agents.clone(),
            self.events.clone(),
            cancelled,
        ));
        self.live.insert(id, Live { run, cancel, task });
    }

    /// Cancels run `run` of `task` and waits until its agent and tool server
    /// are gone.
    pub async fn stop(&mut self, task: u64, run: u64) {
        if self.live.get(&task).is_some_and(|l| l.run == run) {
            let live = self.live.remove(&task).expect("checked");
            let _ = live.cancel.send(());
            let _ = live.task.await;
        }
    }
}

/// One run, then its `Ended`, also after a panic. A cancelled run sends
/// none (the core has moved on). Every path stops the tool server.
async fn run_task(
    spec: RunSpec,
    functor: PathBuf,
    agents: Agents,
    events: UnboundedSender<Event>,
    cancel: oneshot::Receiver<()>,
) {
    let send = |event| {
        let _ = events.send(Event::Run {
            task: spec.task,
            run: spec.run,
            event,
        });
    };
    let run = run(&spec, &functor, &agents, &send, cancel);
    match futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(run)).await {
        Ok(Some(error)) => send(RunEvent::Ended { error }),
        Ok(None) => {}
        Err(_) => send(RunEvent::Ended {
            error: Some("the run panicked".to_string()),
        }),
    }
}

/// The tool server, the app context, the agent: the run's error, or `None`
/// if cancelled.
async fn run(
    spec: &RunSpec,
    functor: &Path,
    agents: &Agents,
    send: &impl Fn(RunEvent),
    mut cancel: oneshot::Receiver<()>,
) -> Option<Option<String>> {
    let token = token();
    // A cancel while starting drops the child's stdin, which stops it.
    let started = tokio::select! {
        started = ToolServer::start(functor, spec.attach.as_deref(), &token) => started,
        _ = &mut cancel => return None,
    };
    let server = match started {
        Ok(server) => server,
        Err(error) => return Some(Some(error)),
    };
    let context = serde_json::to_string(&spec.context).expect("the context serializes");
    let auth = format!("Bearer {token}");
    let headers = [("Authorization", auth.as_str())];
    let put = crate::session::request(&server.base, "PUT", "/app-context", &headers, &context);
    // Not fatal: the agent can still use every other tool.
    if let Err(e) = put.await {
        eprintln!("functor-app: {e}");
    }

    let mcp = Mcp {
        url: format!("{}/mcp", server.base),
        token,
    };
    let permissions = permissions(&spec.repo_root);
    let cwd = spec.dir.to_string_lossy();
    let prompt = prompt(&spec.message, spec.resume.is_some());
    let opts = RunOpts {
        cwd: Some(&cwd),
        mcp: Some(&mcp),
        resume: spec.resume.as_deref(),
        // Follow-ups continue the session itself (claude would otherwise fork it).
        fork: Some(false),
        permissions: Some(&permissions),
        ..RunOpts::default()
    };
    let agent = agents.get(spec.harness);
    let outcome = match mis_agent::check(&permissions) {
        Err(e) => Some(Some(e)),
        Ok(()) => {
            let (tx, mut rx) = mpsc::channel(64);
            let run = run_guarded(&*agent, uuid::Uuid::new_v4(), &prompt, opts, tx);
            let forward = async {
                while let Some(event) = rx.recv().await {
                    if let Some(event) = mirror(event) {
                        send(RunEvent::Agent(redact(event, &mcp.token)));
                    }
                }
            };
            // Cancelling drops the run, and `mis-agent` kills the agent's
            // process tree with it.
            tokio::select! {
                _ = futures_util::future::join(run, forward) => Some(None),
                _ = &mut cancel => None,
            }
        }
    };
    server.stop().await;
    outcome
}

/// The app's copy of an agent event; `None` for those a transcript does
/// not show (limits, plans, status notes).
fn mirror(event: mis_types::AgentEvent) -> Option<AgentEvent> {
    use mis_types::AgentEvent as Up;
    Some(match event {
        Up::Started { session, model } => AgentEvent::Started { session, model },
        Up::TextDelta { text } => AgentEvent::TextDelta { text },
        Up::Text { text } => AgentEvent::Text { text },
        Up::ToolUse { name, input } => AgentEvent::ToolUse { name, input },
        Up::ToolResult { content } => AgentEvent::ToolResult { content },
        Up::Usage { input, output } => AgentEvent::Usage { input, output },
        Up::Finished { ok, summary } => AgentEvent::Finished { ok, summary },
        _ => return None,
    })
}

/// A fresh bearer token: 64 hex digits from two random UUIDs. Only ever in
/// the tool server's and the agent's environments, never argv or output.
fn token() -> String {
    let a = uuid::Uuid::new_v4().simple();
    let b = uuid::Uuid::new_v4().simple();
    format!("{a}{b}")
}

/// `event` with every copy of `token` blanked: the agent has it in its
/// environment and could print it into the transcript.
fn redact(event: AgentEvent, token: &str) -> AgentEvent {
    let json = serde_json::to_string(&event).expect("an event serializes");
    if !json.contains(token) {
        return event;
    }
    // The token is hex, so replacing it keeps the JSON valid.
    serde_json::from_str(&json.replace(token, "[token]")).expect("still an event")
}

/// A `functor mcp --http 0` child. It stops (and stops the games it
/// launched) when its stdin closes, so dropping it is a graceful stop.
struct ToolServer {
    child: Child,
    stdin: ChildStdin,
    /// `http://127.0.0.1:<port>`.
    base: String,
}

impl ToolServer {
    async fn start(functor: &Path, attach: Option<&str>, token: &str) -> Result<Self, String> {
        let mut command = Command::new(functor);
        command.args(["mcp", "--http", "0"]);
        if let Some(url) = attach {
            command.args(["--attach", url]);
        }
        let mut child = command
            .env(TOKEN_ENV, token)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", functor.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let (lines_tx, mut lines) = mpsc::unbounded_channel();
        crate::session::forward(child.stderr.take().expect("piped"), lines_tx);
        let mut tail = VecDeque::new();
        let listening = tokio::time::timeout(LISTEN_TIMEOUT, async {
            while let Some(line) = lines.recv().await {
                if let Some(base) = mcp_listening_url(&line) {
                    return Some(base);
                }
                if tail.len() == TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            None
        })
        .await;
        let base = match listening {
            Ok(Some(base)) => base,
            Ok(None) => {
                let cause = crate::session::cause(&tail).map(|l| format!(": {l}"));
                let cause = cause.unwrap_or_default();
                return Err(format!("functor mcp exited before listening{cause}"));
            }
            Err(_) => {
                // Nothing launched yet, so killing it leaves nothing behind.
                let _ = child.start_kill();
                let secs = LISTEN_TIMEOUT.as_secs();
                return Err(format!("functor mcp did not listen within {secs}s"));
            }
        };
        tokio::spawn(async move {
            while let Some(line) = lines.recv().await {
                eprintln!("[functor mcp] {line}");
            }
        });
        Ok(Self { child, stdin, base })
    }

    /// Closes stdin, the server's signal to stop; if it has not exited
    /// within [`STOP_GRACE`], kills it and any games it launched.
    async fn stop(self) {
        let Self {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        if tokio::time::timeout(STOP_GRACE, child.wait())
            .await
            .is_err()
        {
            if let Some(pid) = child.id() {
                kill_tree(pid);
            }
            let _ = child.wait().await;
        }
    }
}

/// Kills `pid` and its descendants, found by parent pid. Best effort: a
/// process started after the snapshot survives.
fn kill_tree(pid: u32) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut tree = vec![Pid::from_u32(pid)];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        let born = sys.process(parent).map_or(0, |p| p.start_time());
        // Started after its parent: Windows keeps a dead parent's pid on its
        // children, so an older process naming a reused pid is not one.
        let kids = sys
            .processes()
            .iter()
            .filter(|(_, p)| p.parent() == Some(parent) && p.start_time() >= born);
        tree.extend(kids.map(|(pid, _)| *pid));
        i += 1;
    }
    for pid in tree {
        if let Some(p) = sys.process(pid) {
            p.kill();
        }
    }
}

/// Replays a recorded run (`{"ms": …, "event": <AgentEvent>}` lines) from a
/// child process, as the real agents read theirs: what follows the child's
/// output is the code path a real run takes. Selected by
/// `FUNCTOR_APP_FAKE_AGENT=<fixture>`.
pub struct FakeAgent {
    fixture: PathBuf,
}

impl Agent for FakeAgent {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn models(&self) -> Vec<String> {
        vec![]
    }

    fn run<'a>(
        &'a self,
        _task: mis_types::TaskId,
        _prompt: &'a str,
        opts: RunOpts<'a>,
        tx: mpsc::Sender<mis_types::AgentEvent>,
    ) -> BoxFut<'a> {
        Box::pin(async move {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg("--replay-events").arg(&self.fixture);
            if let Some(session) = opts.resume {
                command.arg(format!("--replay-resume={session}"));
            }
            if let Some(dir) = opts.cwd {
                command.current_dir(dir);
            }
            let mut child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn()?;
            let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
            let mut finished = false;
            while let Some(line) = lines.next_line().await? {
                let event: mis_types::AgentEvent = serde_json::from_str(&line)?;
                finished |= matches!(event, mis_types::AgentEvent::Finished { .. });
                if tx.send(event).await.is_err() {
                    return Ok(());
                }
            }
            let status = child.wait().await?;
            if !finished {
                let summary = Some(format!("fake agent exited: {status}"));
                let ok = status.success();
                let _ = tx
                    .send(mis_types::AgentEvent::Finished { ok, summary })
                    .await;
            }
            Ok(())
        })
    }
}

/// `--replay-events`: prints `fixture`'s events on stdout, one JSON line
/// each, after each line's `ms` (from the run's start). A `resume`d run
/// reports that session, as an agent continuing it would.
pub fn replay(fixture: &Path, resume: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    let text =
        std::fs::read_to_string(fixture).map_err(|e| format!("{}: {e}", fixture.display()))?;
    let start = std::time::Instant::now();
    let mut out = std::io::stdout().lock();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let bad = |e: &dyn std::fmt::Display| format!("{}:{}: {e}", fixture.display(), n + 1);
        let mut line: serde_json::Value = serde_json::from_str(line).map_err(|e| bad(&e))?;
        let at = Duration::from_millis(line["ms"].as_u64().unwrap_or(0));
        std::thread::sleep(at.saturating_sub(start.elapsed()));
        let event = &mut line["event"];
        if let (Some(session), "started") = (resume, event["type"].as_str().unwrap_or("")) {
            event["session"] = session.into();
        }
        writeln!(out, "{event}")
            .and_then(|()| out.flush())
            .map_err(|e| bad(&e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preamble_starts_only_new_sessions() {
        assert_eq!(prompt("And now?", true), "And now?");
        let first = prompt("What is the count?", false);
        assert!(first.starts_with(PREAMBLE), "{first}");
        assert!(first.ends_with("\n\nWhat is the count?"), "{first}");
        assert!(first.contains("app_context"), "{first}");
    }

    #[test]
    fn runs_are_folder_scoped_on_the_repo_root() {
        let root = std::env::temp_dir();
        let p = permissions(&root);
        assert_eq!(p.mode, PermissionMode::Folders);
        assert_eq!(p.folders, [root.to_string_lossy()]);
        assert!(p.mcp.is_empty() && p.commands.is_empty(), "{p:?}");
        assert_eq!(mis_agent::check(&p), Ok(()));
    }

    #[test]
    fn an_agent_printing_its_token_has_it_blanked() {
        let token = token();
        let event = AgentEvent::ToolResult {
            content: format!("MIS_MCP_TOKEN={token}\n"),
        };
        assert_eq!(
            redact(event, &token),
            AgentEvent::ToolResult {
                content: "MIS_MCP_TOKEN=[token]\n".to_string()
            }
        );
        let plain = AgentEvent::Text {
            text: "hi".to_string(),
        };
        assert_eq!(redact(plain.clone(), &token), plain);
    }

    #[test]
    fn tokens_are_fresh_hex() {
        let (a, b) = (token(), token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
    }

    #[test]
    fn upstream_events_mirror_by_shape() {
        let up = |json: serde_json::Value| -> mis_types::AgentEvent {
            serde_json::from_value(json).unwrap()
        };
        for json in [
            serde_json::json!({"type": "started", "session": "s", "model": null}),
            serde_json::json!({"type": "text_delta", "text": "a"}),
            serde_json::json!({"type": "text", "text": "a"}),
            serde_json::json!({"type": "tool_use", "name": "Read", "input": {"x": 1}}),
            serde_json::json!({"type": "tool_result", "content": "c"}),
            serde_json::json!({"type": "usage", "input": 1, "output": 2}),
            serde_json::json!({"type": "finished", "ok": true, "summary": null}),
        ] {
            let mirrored = mirror(up(json.clone())).unwrap();
            assert_eq!(serde_json::to_value(mirrored).unwrap(), json);
        }
        assert_eq!(
            mirror(up(serde_json::json!({"type": "limits", "windows": []}))),
            None
        );
    }
}
