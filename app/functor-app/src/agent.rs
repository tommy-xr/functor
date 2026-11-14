//! Agent runs: per run, a `functor mcp --http` tool server child and an agent
//! (`claude` / `codex` through `mis-agent`, or [`FakeAgent`]) offered it;
//! each agent event goes to the core as an `Event::Run`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use functor_app_core::tasks::mcp_listening_url;
use functor_app_core::tasks::{TokenUsage, UsageDetail, Usd};
use functor_app_core::{AgentEvent, Event, Harness, ReviewEvent, ReviewSpec, RunEvent, RunSpec};
use mis_agent::{run_guarded, Agent, BoxFut, Mcp, RunOpts};
use mis_types::{PermissionMode, Permissions};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::changes::Git;

/// The env var `functor mcp --http` reads its bearer token from.
const TOKEN_ENV: &str = "FUNCTOR_MCP_TOKEN";
/// And its `--app-url` secret: the run's for the app's view route.
const APP_TOKEN_ENV: &str = "FUNCTOR_MCP_APP_TOKEN";
/// The app's route for its runs' view requests (see `server`).
pub const VIEW_PATH: &str = "/agent/view";
/// Context changes closer together than this go to the tool server as one.
const CONTEXT_DEBOUNCE: Duration = Duration::from_millis(200);
/// How long the tool server may take to print its listening line.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the tool server gets to exit after its stdin closes (it stops
/// the games it launched) before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// Tool-server output lines kept for a failure report.
const TAIL_LINES: usize = 20;

/// Told to the agent at the start of each new agent session, then
/// [`ATTACHED`] or [`NO_GAME`], then [`VIEWS`] when the run can show views:
/// it names only tools the run is offered. The ones it calls loaded are the
/// tool server's `ALWAYS_LOADED` (`cli/src/commands/mcp.rs`): keep the two
/// lists together, or each missing one costs a tool-search turn.
const PREAMBLE: &str = "You are working on a Functor game project: this directory. \
Your MCP server `mis` drives its game. Loaded and ready: `mcp__mis__get_state` (the live \
model and clock), `mcp__mis__step` and `mcp__mis__resume` (run it), `mcp__mis__send_input` \
(keys, mouse, UI events), `mcp__mis__capture_frame` (a picture), `mcp__mis__app_context` \
(what the user is looking at in the Functor app), `mcp__mis__language_guide` and \
`mcp__mis__api_reference` (the engine API, e.g. `Ui.row`). Its other \
tools (scene, trace, rewind, launch, stop) are there when you need them. Prefer reading \
live state and captures over guessing. Functor Lang (`.fun`) is a custom language, not F#: \
before writing it, read `mcp__mis__language_guide` (contents and quick facts, then only the \
sections you need) rather than the whole `functor-lang` skill. Make edits with your edit \
tools, not shell scripts.";

/// The app's game, attached by the tool server before anything else.
const ATTACHED: &str = "The game is running as session `s1`. It opens paused on its first \
frame (`clock.paused`): `step` or `resume` it to run it, as input lands only when it does.";

/// No game session when the run started.
const NO_GAME: &str = "No game is running: `launch_game` starts one.";

/// The app's view tools (`--app-url`).
const VIEWS: &str = "`mcp__mis__show_view` and `mcp__mis__present` show the user views in \
the app, beside their work. When you finish a change, call `present` once: the changed file \
ranges, then the game. Do not show anything else they did not ask to see.";

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

/// What a review (D31) may do: read. It runs `mis-agent`'s read-only mode
/// (`read_only` in its `RunOpts`) on [`permissions`]: what no rule allows is
/// denied, the only rule is its MCP server's tools, and the user's own
/// settings are not loaded, as their rules could allow writes; so, unlike a
/// task's run, a review is isolated from them (D19). Reading files and
/// read-only shell commands (`ls`, `git log`) still work. Claude only: that
/// mode has no Codex equivalent (`review::review_harness`).
pub fn review_permissions(repo_root: &Path) -> Permissions {
    permissions(repo_root)
}

/// The prompt for `message`: a new agent session gets the [`PREAMBLE`],
/// with the parts for whether it is `attached` to a game and has `views`.
fn prompt(message: &str, resumed: bool, attached: bool, views: bool) -> String {
    if resumed {
        return message.to_string();
    }
    let game = if attached { ATTACHED } else { NO_GAME };
    let views = if views {
        format!(" {VIEWS}")
    } else {
        String::new()
    };
    format!("{PREAMBLE} {game}{views}\n\n{message}")
}

/// A review stops once it has read more input than this (cached tokens
/// included), or run longer than [`REVIEW_MAX_TIME`]. Codex reports usage
/// only at its end, so only the time bounds it.
pub const REVIEW_MAX_INPUT: u64 = 300_000;
pub const REVIEW_MAX_TIME: Duration = Duration::from_secs(180);
/// The most of the diff a review's brief carries.
const REVIEW_DIFF_BYTES: u64 = 48 * 1024;

/// A review's instructions (D31), then the setting's own. The summary rules
/// are those of ASD-STE100 that matter in one paragraph.
const REVIEW_PREAMBLE: &str = "You prepare the decision page for a change that another \
agent made in this Functor game project. The user reads your report, then keeps the change or \
reverts it. You are read-only: do not try to edit files or run commands that change anything.

1. Call `mcp__mis__task_change`. It gives the user's request, what the agent did, the change \
with its diff, and the evidence the app has.
2. Check what you can, with few tool calls. Read the changed code and the code it touches. Look \
for mistakes, missed cases and side effects. Do not do the agent's work again.
3. Call `mcp__mis__report` once, then stop.

The summary is one paragraph in Simplified Technical English (ASD-STE100):
- At most 20 words in each sentence, and at most 6 sentences.
- Active voice. Simple present or simple past tense.
- One word for one thing: use the same term each time. Write code names as in the code, in \
backticks.
- No idioms, no slang, no filler.
- Give numbers with their units (2.5 m, 60 frames, 0.4 s).
- Say what changed, what the effect in the game is, and what the user should check.

`notes`: each is one concrete sentence the user must know before they keep the change: what \
nobody verified, what was assumed, a side effect, a risk. Write `[]` only when you checked and \
found nothing.

`evidence`: the page keeps the checks in the agent's own report. Add only checks that really \
ran and that report does not list, with their real result as `task_change` shows them (say they \
are the agent's). Reading code is not evidence. Set `ok` false for a check that failed.

`steps`: only when `task_change` says the agent gave no walkthrough: 1 to 4 highlights, each a \
changed file range (`{\"kind\": \"file\", \"path\", \"lines\": [first, last]}`, the path relative \
to this directory) with a short title.";

/// The review's prompt: [`REVIEW_PREAMBLE`] and the setting's instructions.
fn review_prompt(instructions: Option<&str>) -> String {
    match instructions {
        Some(more) => format!("{REVIEW_PREAMBLE}\n\nThe user's instructions for reviews:\n\n{more}"),
        None => REVIEW_PREAMBLE.to_string(),
    }
}

/// Which agent runs tasks.
#[derive(Clone)]
pub enum Agents {
    /// The task's harness, on `PATH`.
    Real,
    /// [`FakeAgent`] replaying `run` for each task's run and `review`, when
    /// given, for each review; whatever the harness.
    Fake { run: PathBuf, review: Option<PathBuf> },
}

impl Agents {
    /// The harnesses tasks can run with: those whose CLI is on `PATH`, or
    /// every one under the fake agent.
    pub fn available(&self) -> Vec<Harness> {
        let all = [Harness::Claude, Harness::Codex];
        match self {
            Self::Fake { .. } => all.to_vec(),
            Self::Real => all
                .into_iter()
                .filter(|h| mis_agent::which(h.name()))
                .collect(),
        }
    }

    /// Whether reviews can run: a fake agent needs a review fixture.
    pub fn can_review(&self) -> bool {
        !matches!(self, Self::Fake { review: None, .. })
    }

    fn get(&self, harness: Harness, review: bool) -> Box<dyn Agent> {
        match (self, harness) {
            (Self::Fake { run, review: fixture }, _) => Box::new(FakeAgent {
                fixture: match (review, fixture) {
                    (true, Some(fixture)) => fixture.clone(),
                    _ => run.clone(),
                },
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
    git: Option<Arc<Git>>,
    events: UnboundedSender<Event>,
    live: HashMap<u64, Live>,
    /// Per task, its review in flight: the run it reviews and its cancel,
    /// fired (or dropped) to stop it.
    reviews: HashMap<u64, (u64, oneshot::Sender<()>)>,
    views: Arc<ViewRoute>,
    /// Where a run keeps its task's moment (`store::moment_file`).
    home: PathBuf,
}

struct Live {
    run: u64,
    /// Fired (or dropped) to cancel the run.
    cancel: oneshot::Sender<()>,
    task: JoinHandle<()>,
    /// The latest app context, as JSON, for the tool server.
    context: watch::Sender<String>,
}

/// Who a view request comes from: run `run` of task `task`, or its review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub task: u64,
    pub run: u64,
    pub review: bool,
}

/// The app's `POST /agent/view` route: its URL once the server listens
/// (and whether only reviews get it), and one secret per run with a tool
/// server, each naming its caller. A secret is removed when its run ends
/// ([`RunSecret`]'s drop).
#[derive(Default)]
pub struct ViewRoute {
    url: OnceLock<(String, bool)>,
    secrets: Mutex<Vec<(String, Caller)>>,
}

impl ViewRoute {
    /// The server listens at `origin`: runs started from now get the route,
    /// or only reviews (`task`, whose runs show no views).
    pub fn listen(&self, origin: &str, reviews_only: bool) {
        let _ = self.url.set((format!("{origin}{VIEW_PATH}"), reviews_only));
    }

    /// The caller `presented` is the secret of, comparing each in constant time.
    pub fn find(&self, presented: &[u8]) -> Option<Caller> {
        let secrets = self.secrets.lock().expect("view secrets poisoned");
        let mut found = None;
        for (secret, caller) in secrets.iter() {
            if crate::server::same(secret.as_bytes(), presented) {
                found = Some(*caller);
            }
        }
        found
    }

    /// A fresh secret for `caller`; `None` with no server to route to.
    fn register(self: &Arc<Self>, caller: Caller) -> Option<RunSecret> {
        let (url, reviews_only) = self.url.get()?.clone();
        if reviews_only && !caller.review {
            return None;
        }
        let secret = token();
        let mut secrets = self.secrets.lock().expect("view secrets poisoned");
        secrets.push((secret.clone(), caller));
        Some(RunSecret {
            route: self.clone(),
            secret,
            url,
        })
    }
}

/// A run's view secret, valid until dropped.
struct RunSecret {
    route: Arc<ViewRoute>,
    secret: String,
    url: String,
}

impl Drop for RunSecret {
    fn drop(&mut self) {
        let mut secrets = self.route.secrets.lock().expect("view secrets poisoned");
        secrets.retain(|(s, ..)| *s != self.secret);
    }
}

impl Runner {
    pub fn new(
        functor: PathBuf,
        agents: Agents,
        git: Option<Arc<Git>>,
        events: UnboundedSender<Event>,
        views: Arc<ViewRoute>,
        home: PathBuf,
    ) -> Self {
        Self {
            functor,
            agents,
            git,
            events,
            live: HashMap::new(),
            reviews: HashMap::new(),
            views,
            home,
        }
    }

    /// Starts `spec`'s review, replacing any the task had.
    pub fn start_review(&mut self, spec: ReviewSpec) {
        let (cancel, cancelled) = oneshot::channel();
        self.reviews.insert(spec.task, (spec.run, cancel));
        tokio::spawn(run_review(
            spec,
            self.functor.clone(),
            self.agents.clone(),
            self.git.clone(),
            self.events.clone(),
            cancelled,
            self.views.clone(),
            self.home.clone(),
        ));
    }

    /// Cancels task `task`'s review of run `run`. Not waited for: it edits
    /// nothing, and stops its tool server and agent on its own.
    pub fn stop_review(&mut self, task: u64, run: u64) {
        if self.reviews.get(&task).is_some_and(|(r, _)| *r == run) {
            self.reviews.remove(&task);
        }
    }

    pub fn start(&mut self, spec: RunSpec) {
        self.live.retain(|_, live| !live.task.is_finished());
        let (cancel, cancelled) = oneshot::channel();
        let run = spec.run;
        let id = spec.task;
        let json = serde_json::to_string(&spec.context).expect("the context serializes");
        let (context, contexts) = watch::channel(json);
        let moment = crate::store::moment_file(&self.home, &spec.dir, spec.task);
        let task = tokio::spawn(run_task(
            spec,
            moment,
            self.functor.clone(),
            self.agents.clone(),
            self.git.clone(),
            self.events.clone(),
            cancelled,
            Some(self.views.clone()),
            contexts,
        ));
        self.live.insert(
            id,
            Live {
                run,
                cancel,
                task,
                context,
            },
        );
    }

    /// Run `run` of `task`'s tool server gets `context` (debounced).
    pub fn context(&self, task: u64, run: u64, context: &functor_app_core::AppContext) {
        if let Some(live) = self.live.get(&task).filter(|l| l.run == run) {
            let json = serde_json::to_string(context).expect("the context serializes");
            live.context.send_replace(json);
        }
    }

    /// The git new runs measure changes with: the open workspace's.
    pub fn set_git(&mut self, git: Option<Arc<Git>>) {
        self.git = git;
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

/// One run, then its change (when tracked) and its `Ended`, also after a
/// panic. A cancelled run sends neither (the core has moved on). Every path
/// stops the tool server.
#[allow(clippy::too_many_arguments)]
async fn run_task(
    spec: RunSpec,
    moment: PathBuf,
    functor: PathBuf,
    agents: Agents,
    git: Option<Arc<Git>>,
    events: UnboundedSender<Event>,
    mut cancel: oneshot::Receiver<()>,
    views: Option<Arc<ViewRoute>>,
    contexts: watch::Receiver<String>,
) {
    let send = |event| {
        let _ = events.send(Event::Run {
            task: spec.task,
            run: spec.run,
            event,
        });
    };
    let git = git.filter(|_| spec.snapshot || spec.baseline.is_some());
    let mut baseline = spec.baseline.clone();
    // Taken before the agent starts, so the change is all its own work (and
    // anyone else's edits meanwhile, which git cannot tell apart); the
    // game's moment with it, as the user left the game.
    if let (Some(git), true) = (&git, spec.snapshot) {
        let moment = async {
            match (&spec.attach, spec.moment) {
                (_, false) => None,
                (Some(url), true) => Some(crate::evidence::take_moment(url, &moment).await),
                (None, true) => Some(functor_app_core::evidence::Moment::Missing {
                    reason: "the game was not running when the task started".to_string(),
                }),
            }
        };
        tokio::select! {
            (tree, moment) = async { tokio::join!(git.snapshot(), moment) } => match tree {
                Ok(tree) => {
                    send(RunEvent::Baseline { tree: tree.clone() });
                    if let Some(moment) = moment {
                        send(RunEvent::Moment(moment));
                    }
                    baseline = Some(tree);
                }
                Err(e) => eprintln!("functor-app: cannot snapshot the working tree: {e}"),
            },
            _ = &mut cancel => return,
        }
    }
    let caller = Caller {
        task: spec.task,
        run: spec.run,
        review: false,
    };
    let secret = views.and_then(|v| v.register(caller));
    let run = run(
        &spec,
        &functor,
        &agents,
        &send,
        cancel,
        secret.as_ref(),
        contexts,
    );
    let error = match futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(run)).await
    {
        Ok(Some(error)) => error,
        Ok(None) => return,
        Err(_) => Some("the run panicked".to_string()),
    };
    // The route refuses the run's requests from here on.
    drop(secret);
    if let (Some(git), Some(baseline)) = (&git, baseline) {
        crate::changes::measure(git, spec.task, baseline, &events).await;
    }
    send(RunEvent::Ended { error });
}

/// The tool server, the app context, the agent: the run's error, or `None`
/// if cancelled.
async fn run(
    spec: &RunSpec,
    functor: &Path,
    agents: &Agents,
    send: &impl Fn(RunEvent),
    mut cancel: oneshot::Receiver<()>,
    secret: Option<&RunSecret>,
    mut contexts: watch::Receiver<String>,
) -> Option<Option<String>> {
    let token = token();
    let app = secret.map(|s| (s.url.as_str(), s.secret.as_str()));
    let args = tool_server_args(spec.attach.as_deref(), app.map(|(url, _)| url), None);
    // A cancel while starting drops the child's stdin, which stops it.
    let started = tokio::select! {
        started = ToolServer::start(functor, &spec.dir, &args, &token, app.map(|(_, s)| s)) => started,
        _ = &mut cancel => return None,
    };
    let server = match started {
        Ok(server) => server,
        Err(error) => return Some(Some(error)),
    };
    let context = contexts.borrow_and_update().clone();
    put_context(&server.base, &token, &context).await;
    // Later contexts (the layout, placement, project or session changed),
    // each burst as one; aborted with the run.
    let pusher = {
        let (base, token) = (server.base.clone(), token.clone());
        AbortOnDrop(tokio::spawn(async move {
            while contexts.changed().await.is_ok() {
                tokio::time::sleep(CONTEXT_DEBOUNCE).await;
                let context = contexts.borrow_and_update().clone();
                put_context(&base, &token, &context).await;
            }
        }))
    };

    let mcp = Mcp {
        url: format!("{}/mcp", server.base),
        token,
    };
    let permissions = permissions(&spec.repo_root);
    let cwd = spec.dir.to_string_lossy();
    let resumed = spec.resume.is_some();
    let prompt = prompt(
        &spec.message,
        resumed,
        spec.attach.is_some(),
        secret.is_some(),
    );
    let opts = RunOpts {
        cwd: Some(&cwd),
        mcp: Some(&mcp),
        resume: spec.resume.as_deref(),
        // Follow-ups continue the session itself (claude would otherwise fork it).
        fork: Some(false),
        permissions: Some(&permissions),
        ..RunOpts::default()
    };
    let agent = agents.get(spec.harness, false);
    let forward = |event: AgentEvent| send(RunEvent::Agent(event));
    let outcome = drive(&*agent, &prompt, opts, &mcp.token, None, &forward, &mut cancel)
        .await
        .map(Result::err);
    drop(pusher);
    server.stop().await;
    outcome
}

/// Runs `agent` on `prompt`, each event it reports mirrored to `send`
/// (the token blanked): `Some(Ok)` once it ends, `Some(Err)` if it could
/// not start or was cut past `max_input` tokens, `None` if cancelled.
/// Cancelling or cutting drops the run, and `mis-agent` kills the agent's
/// process tree with it.
async fn drive(
    agent: &dyn Agent,
    prompt: &str,
    opts: RunOpts<'_>,
    token: &str,
    max_input: Option<u64>,
    send: &impl Fn(AgentEvent),
    cancel: &mut oneshot::Receiver<()>,
) -> Option<Result<(), String>> {
    if let Some(permissions) = opts.permissions {
        if let Err(e) = mis_agent::check(permissions) {
            return Some(Err(e));
        }
    }
    let (tx, mut rx) = mpsc::channel(64);
    let run = run_guarded(agent, uuid::Uuid::new_v4(), prompt, opts, tx);
    // Ends when the run has sent its last event, or at the cut.
    let forward = async {
        while let Some(event) = rx.recv().await {
            let Some(event) = mirror(event) else {
                continue;
            };
            let over = match (&event, max_input) {
                (AgentEvent::Usage { input, .. }, Some(max)) => *input > max,
                _ => false,
            };
            send(redact(event, token));
            if let (true, Some(max)) = (over, max_input) {
                return Err(format!("cut: over {max} input tokens"));
            }
        }
        Ok(())
    };
    tokio::pin!(run, forward);
    tokio::select! {
        // The run is done: what it sent is still to forward.
        _ = &mut run => Some(forward.await),
        forwarded = &mut forward => Some(forwarded),
        _ = cancel => None,
    }
}

/// One review (D31), then its `Ended`, also after a panic; a cancelled one
/// sends nothing. The working tree is measured again after it, so a review
/// that edited anyway shows as a new change, which voids its report.
#[allow(clippy::too_many_arguments)]
async fn run_review(
    spec: ReviewSpec,
    functor: PathBuf,
    agents: Agents,
    git: Option<Arc<Git>>,
    events: UnboundedSender<Event>,
    mut cancel: oneshot::Receiver<()>,
    views: Arc<ViewRoute>,
    home: PathBuf,
) {
    let send = |event| {
        let (task, run) = (spec.task, spec.run);
        let _ = events.send(Event::Review { task, run, event });
    };
    let Some(git) = git else {
        let error = Some("changes are tracked only in a git repository".to_string());
        return send(ReviewEvent::Ended { error });
    };
    let review = review(&spec, &functor, &agents, &git, &views, &home, &send, &mut cancel);
    let error = match futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(review))
        .await
    {
        Ok(Some(error)) => error,
        Ok(None) => return,
        Err(_) => Some("the review panicked".to_string()),
    };
    // Its report is published only on a tree measured as unchanged.
    let error = match git.change(&spec.baseline).await {
        Ok(change) => {
            let (task, baseline) = (spec.task, spec.baseline.clone());
            let _ = events.send(Event::Change {
                task,
                baseline,
                change,
            });
            error
        }
        Err(e) => Some(format!("cannot measure the files after the review: {e}")),
    };
    send(ReviewEvent::Ended { error });
}

/// The brief (with the diff), the review's tool server and its agent, read
/// only and bounded: the review's error, or `None` if cancelled.
#[allow(clippy::too_many_arguments)]
async fn review(
    spec: &ReviewSpec,
    functor: &Path,
    agents: &Agents,
    git: &Git,
    views: &Arc<ViewRoute>,
    home: &Path,
    send: &impl Fn(ReviewEvent),
    cancel: &mut oneshot::Receiver<()>,
) -> Option<Option<String>> {
    // The core never asks for one: only Claude's can be held read-only.
    if spec.harness != Harness::Claude {
        return Some(Some(functor_app_core::review::NO_CODEX.to_string()));
    }
    let caller = Caller {
        task: spec.task,
        run: spec.run,
        review: true,
    };
    let Some(secret) = views.register(caller) else {
        return Some(Some("the app has no route for the review's report".to_string()));
    };
    let brief = match git.patch(&spec.baseline, &spec.tree, REVIEW_DIFF_BYTES).await {
        Ok((diff, cut)) => {
            let cut = match cut {
                true => "\nThe diff is cut here: read the files for the rest.\n",
                false => "",
            };
            format!("{}\n## The diff\n\n```diff\n{diff}```\n{cut}", spec.brief)
        }
        Err(e) => return Some(Some(format!("cannot read the change: {e}"))),
    };
    let brief = match BriefFile::write(home, &brief) {
        Ok(file) => file,
        Err(e) => return Some(Some(format!("cannot write the review's brief: {e}"))),
    };
    let token = token();
    let path = brief.0.to_string_lossy();
    let args = tool_server_args(None, Some(&secret.url), Some(&path));
    let started = tokio::select! {
        started = ToolServer::start(functor, &spec.dir, &args, &token, Some(&secret.secret)) => started,
        _ = &mut *cancel => return None,
    };
    let server = match started {
        Ok(server) => server,
        Err(error) => return Some(Some(error)),
    };
    let mcp = Mcp {
        url: format!("{}/mcp", server.base),
        token: token.clone(),
    };
    let permissions = review_permissions(&spec.repo_root);
    let cwd = spec.dir.to_string_lossy();
    let opts = RunOpts {
        cwd: Some(&cwd),
        model: spec.model.as_deref(),
        mcp: Some(&mcp),
        permissions: Some(&permissions),
        read_only: true,
        ..RunOpts::default()
    };
    let prompt = review_prompt(spec.instructions.as_deref());
    let agent = agents.get(spec.harness, true);
    // The agent's own failure (a bad model, say) is the review's reason.
    let failed = Mutex::new(None);
    let forward = |event: AgentEvent| {
        if let AgentEvent::Finished { ok: false, summary } = &event {
            *failed.lock().expect("unpoisoned") = Some(summary.clone().unwrap_or("the agent failed".into()));
        }
        send(ReviewEvent::Agent(event));
    };
    let max = Some(REVIEW_MAX_INPUT);
    let driven = drive(&*agent, &prompt, opts, &token, max, &forward, cancel);
    let outcome = match tokio::time::timeout(REVIEW_MAX_TIME, driven).await {
        Ok(Some(Ok(()))) => Some(failed.lock().expect("unpoisoned").take()),
        Ok(outcome) => outcome.map(Result::err),
        Err(_) => Some(Some(format!("cut: over {} s", REVIEW_MAX_TIME.as_secs()))),
    };
    server.stop().await;
    outcome
}

/// A review's brief, on disk for its tool server; removed when dropped.
struct BriefFile(PathBuf);

impl BriefFile {
    fn write(home: &Path, text: &str) -> std::io::Result<Self> {
        let name = format!("{}.md", uuid::Uuid::new_v4().simple());
        let file = Self(home.join("reviews").join(name));
        crate::store::save_private(&file.0, text.as_bytes())?;
        Ok(file)
    }
}

impl Drop for BriefFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `PUT /app-context` on the tool server at `base`. Not fatal: the agent can
/// still use every other tool.
async fn put_context(base: &str, token: &str, context: &str) {
    let auth = format!("Bearer {token}");
    let headers = [("Authorization", auth.as_str())];
    let put = crate::session::request(base, "PUT", "/app-context", &headers, context);
    if let Err(e) = put.await {
        eprintln!("functor-app: {e}");
    }
}

/// Aborts its task when dropped (a cancelled run drops it mid-await).
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
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
        Up::ToolUseMeta { id, parent_id } => AgentEvent::ToolUseMeta { id, parent_id },
        Up::ToolResultMeta { id, is_error } => AgentEvent::ToolResultMeta { id, is_error },
        Up::Usage { input, output } => AgentEvent::Usage { input, output },
        Up::UsageDetail {
            total,
            last,
            turns,
            cost_usd,
            api_ms,
            context_window,
        } => AgentEvent::UsageDetail(UsageDetail {
            total: tokens(total),
            last: last.map(tokens),
            turns,
            cost_usd: cost_usd.map(Usd),
            api_ms,
            context_window,
        }),
        Up::Finished { ok, summary } => AgentEvent::Finished { ok, summary },
        _ => return None,
    })
}

fn tokens(up: mis_types::TokenUsage) -> TokenUsage {
    TokenUsage {
        input: up.input,
        cache_read: up.cache_read,
        cache_creation: up.cache_creation,
        output: up.output,
        reasoning_output: up.reasoning_output,
    }
}

/// A fresh secret: 64 hex digits from two random UUIDs. As the tool server's
/// bearer token it is only ever in its and the agent's environments, never
/// argv or output; for the app's own, see `server::Token`.
pub(crate) fn token() -> String {
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

/// A `functor mcp --http 0` child. [`ToolServer::stop`] stops it gracefully
/// (it stops the games it launched when its stdin closes); dropping it
/// instead kills it.
struct ToolServer {
    child: Child,
    stdin: ChildStdin,
    /// `http://127.0.0.1:<port>`.
    base: String,
}

/// `functor`'s arguments for a tool server attached to `attach`, forwarding
/// view requests to `app` (its secret goes in the environment, never here),
/// or serving a review of the brief at `review`. Without `--no-unsafe-code`
/// the agent could run any Node.js outside its folder sandbox.
fn tool_server_args<'a>(
    attach: Option<&'a str>,
    app: Option<&'a str>,
    review: Option<&'a str>,
) -> Vec<&'a str> {
    let mut args = vec!["mcp", "--http", "0", "--no-unsafe-code"];
    if let Some(url) = attach {
        args.extend(["--attach", url]);
    }
    if let Some(url) = app {
        args.extend(["--app-url", url]);
    }
    if let Some(brief) = review {
        args.extend(["--review", brief]);
    }
    args
}

impl ToolServer {
    /// `functor <args>` in `dir`, the run's project, as the agent is: its
    /// tools resolve a relative directory against it, and the app's own
    /// working directory is wherever it was started, not the open folder.
    /// `app_secret` is `--app-url`'s.
    async fn start(
        functor: &Path,
        dir: &Path,
        args: &[&str],
        token: &str,
        app_secret: Option<&str>,
    ) -> Result<Self, String> {
        // Killed if dropped before `stop` (a cancel or failure while starting).
        let mut command = Command::new(functor);
        command
            .args(args)
            .current_dir(dir)
            .kill_on_drop(true)
            .env(TOKEN_ENV, token);
        if let Some(secret) = app_secret {
            command.env(APP_TOKEN_ENV, secret);
        }
        let mut child = command
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
            // Its scripted tool calls go to the run's real tool server, as an agent's do.
            if let Some(mcp) = opts.mcp {
                command
                    .env(FAKE_MCP_URL_ENV, &mcp.url)
                    .env(FAKE_MCP_TOKEN_ENV, &mcp.token);
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

/// Where the fake agent's child finds the run's tool server.
const FAKE_MCP_URL_ENV: &str = "FUNCTOR_APP_FAKE_MCP_URL";
const FAKE_MCP_TOKEN_ENV: &str = "FUNCTOR_APP_FAKE_MCP_TOKEN";

/// `--replay-events`: prints `fixture`'s events on stdout, one JSON line
/// each, after each line's `ms` (from the run's start). A `resume`d run
/// reports that session, as an agent continuing it would. A line with
/// `"edit": {"path", "old", "new"}` instead of an event makes the edit,
/// replacing `old`'s first match in `path` (from the working directory) as
/// an agent's edit tool would. A line with `"mcp": {"tool", "arguments"}`
/// calls that tool on the run's tool server over MCP, printing its
/// `tool_use` and `tool_result` as the agent's; with `"id"` (and
/// `"parent_id"`, for a subagent's call), their `_meta` events too.
pub fn replay(fixture: &Path, resume: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    let text =
        std::fs::read_to_string(fixture).map_err(|e| format!("{}: {e}", fixture.display()))?;
    let start = std::time::Instant::now();
    let mut out = std::io::stdout().lock();
    let mut mcp: Option<Result<FakeMcp, String>> = None;
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let bad = |e: &dyn std::fmt::Display| format!("{}:{}: {e}", fixture.display(), n + 1);
        let mut line: serde_json::Value = serde_json::from_str(line).map_err(|e| bad(&e))?;
        let at = Duration::from_millis(line["ms"].as_u64().unwrap_or(0));
        std::thread::sleep(at.saturating_sub(start.elapsed()));
        if let Some(edit) = line.get("edit") {
            let field = |key: &str| {
                edit[key]
                    .as_str()
                    .ok_or_else(|| bad(&format!("no edit.{key}")))
            };
            let (path, old, new) = (field("path")?, field("old")?, field("new")?);
            let text = std::fs::read_to_string(path).map_err(|e| bad(&e))?;
            if !text.contains(old) {
                return Err(bad(&format!("{path} has no {old:?}")));
            }
            std::fs::write(path, text.replacen(old, new, 1)).map_err(|e| bad(&e))?;
            continue;
        }
        if let Some(call) = line.get("mcp") {
            let tool = call["tool"].as_str().ok_or_else(|| bad(&"no mcp.tool"))?;
            let arguments = call["arguments"].clone();
            let name = format!("mcp__mis__{tool}");
            let id = call.get("id").filter(|id| id.is_string());
            let mut print = |event: serde_json::Value| {
                writeln!(out, "{event}")
                    .and_then(|()| out.flush())
                    .map_err(|e| bad(&e))
            };
            print(serde_json::json!({"type": "tool_use", "name": name, "input": arguments}))?;
            if let Some(id) = id {
                let parent = &call["parent_id"];
                print(serde_json::json!({"type": "tool_use_meta", "id": id, "parent_id": parent}))?;
            }
            let answer = match mcp.get_or_insert_with(FakeMcp::from_env) {
                Ok(client) => client.call(tool, arguments),
                Err(e) => Err(e.clone()),
            };
            let failed = answer.is_err();
            let content = answer.unwrap_or_else(|e| format!("error: {e}"));
            print(serde_json::json!({"type": "tool_result", "content": content}))?;
            if let Some(id) = id {
                print(
                    serde_json::json!({"type": "tool_result_meta", "id": id, "is_error": failed}),
                )?;
            }
            continue;
        }
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

/// The fake agent's MCP client: one session on the run's tool server, over
/// HTTP/1.0 so each answer ends with its connection.
struct FakeMcp {
    runtime: tokio::runtime::Runtime,
    url: String,
    token: String,
    session: String,
    next: u64,
}

impl FakeMcp {
    fn from_env() -> Result<Self, String> {
        let var = |name| std::env::var(name).map_err(|_| format!("{name} is not set"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let mut client = Self {
            runtime,
            url: var(FAKE_MCP_URL_ENV)?,
            token: var(FAKE_MCP_TOKEN_ENV)?,
            session: String::new(),
            next: 1,
        };
        let init = serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "functor-app-fake-agent", "version": "0"}}});
        let (head, _) = client.post(&init)?;
        client.session = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("mcp-session-id")
                    .then(|| v.trim().to_string())
            })
            .ok_or("no Mcp-Session-Id")?;
        client
            .post(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(client)
    }

    /// `tools/call`: the result's text, or an error with the tool's.
    fn call(&mut self, tool: &str, arguments: serde_json::Value) -> Result<String, String> {
        let id = self.next;
        self.next += 1;
        let call = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}});
        let (_, body) = self.post(&call)?;
        let response = body
            .lines()
            .map(|l| l.strip_prefix("data:").unwrap_or(l).trim())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|m| m["id"] == id)
            .ok_or_else(|| format!("no answer to {tool}"))?;
        let text = response["result"]["content"][0]["text"].as_str();
        match (text, response["result"]["isError"].as_bool()) {
            (Some(text), Some(true)) => Err(text.to_string()),
            (Some(text), _) => Ok(text.to_string()),
            _ => Err(response["error"]["message"]
                .as_str()
                .unwrap_or("no result")
                .to_string()),
        }
    }

    /// One POST to the tool server's `/mcp`: the response's head and body.
    fn post(&self, message: &serde_json::Value) -> Result<(String, String), String> {
        let base = self.url.trim_end_matches("/mcp");
        let auth = format!("Bearer {}", self.token);
        let mut headers = vec![
            ("Authorization", auth.as_str()),
            ("Accept", "application/json, text/event-stream"),
            ("Content-Type", "application/json"),
        ];
        if !self.session.is_empty() {
            headers.push(("Mcp-Session-Id", self.session.as_str()));
        }
        let body = message.to_string();
        let exchange = crate::session::exchange(base, "POST", "/mcp", &headers, &body);
        self.runtime.block_on(exchange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preamble_starts_only_new_sessions() {
        assert_eq!(prompt("And now?", true, true, true), "And now?");
        let first = prompt("What is the count?", false, false, false);
        assert!(first.starts_with(PREAMBLE), "{first}");
        assert!(first.ends_with("\n\nWhat is the count?"), "{first}");
        assert!(first.contains("app_context"), "{first}");
    }

    /// `task` has no view route: the agent is not told of tools it lacks.
    #[test]
    fn the_preamble_names_only_the_tools_a_run_has() {
        let bare = prompt("hi", false, false, false);
        assert!(
            !bare.contains("present") && !bare.contains("`s1`"),
            "{bare}"
        );
        assert!(bare.contains(NO_GAME), "{bare}");
        let app = prompt("hi", false, true, true);
        assert!(app.contains(ATTACHED) && app.contains(VIEWS), "{app}");
        let task = prompt("hi", false, true, false);
        assert!(
            task.contains(ATTACHED) && !task.contains("present"),
            "{task}"
        );
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
    fn the_tool_server_never_offers_unsandboxed_code() {
        assert_eq!(
            tool_server_args(None, None, None),
            ["mcp", "--http", "0", "--no-unsafe-code"]
        );
        let attached = tool_server_args(Some("http://127.0.0.1:9"), None, None);
        assert!(attached.contains(&"--no-unsafe-code"), "{attached:?}");
        assert!(
            attached.ends_with(&["--attach", "http://127.0.0.1:9"]),
            "{attached:?}"
        );
        // A review's never attaches the user's game.
        let review = tool_server_args(None, Some("http://127.0.0.1:9/v"), Some("/b.md"));
        assert_eq!(
            review[4..],
            ["--app-url", "http://127.0.0.1:9/v", "--review", "/b.md"]
        );
    }

    /// A review is read-only by the harness's own rules, not by asking: what
    /// no rule allows is denied (no edits), the user's settings (whose rules
    /// could allow writes) are not loaded, and the only rule is the review's
    /// own tool server.
    #[test]
    fn reviews_cannot_edit() {
        let root = std::env::temp_dir();
        let p = review_permissions(&root);
        let mcp = Mcp {
            url: "http://127.0.0.1:9/mcp".into(),
            token: "t".into(),
        };
        let opts = RunOpts {
            mcp: Some(&mcp),
            permissions: Some(&p),
            read_only: true,
            model: Some("sonnet"),
            ..RunOpts::default()
        };
        let argv = mis_agent::claude::ClaudeAgent::default().argv("review", &opts);
        let at = |flag: &str| argv.iter().position(|a| a == flag);
        let value = |flag: &str| at(flag).map(|i| argv[i + 1].as_str());
        assert_eq!(value("--permission-mode"), Some("dontAsk"), "{argv:?}");
        assert!(argv.contains(&"--setting-sources=".to_string()), "{argv:?}");
        assert!(argv.contains(&"--strict-mcp-config".to_string()), "{argv:?}");
        for writes in ["acceptEdits", "--dangerously-skip-permissions", "Edit", "Write"] {
            assert!(!argv.iter().any(|a| a == writes), "{writes}: {argv:?}");
        }
        let allowed = at("--allowedTools").expect("an allow list");
        assert_eq!(argv[allowed + 1], "mcp__mis", "{argv:?}");
        assert!(argv[allowed + 2].starts_with("--"), "only its server: {argv:?}");
        assert!(argv.contains(&"--model=sonnet".to_string()), "{argv:?}");
    }

    /// A run cancelled before its tool server listens leaves no server.
    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn a_cancel_while_the_tool_server_starts_kills_it() {
        use std::os::unix::fs::PermissionsExt;
        use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        // Never prints its listening line.
        let stub = dir.path().join("functor");
        let script = format!(
            "#!/bin/sh\necho $$ > '{0}.tmp' && mv '{0}.tmp' '{0}'\nexec sleep 60\n",
            pid_file.display()
        );
        std::fs::write(&stub, script).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (tx, _events) = mpsc::unbounded_channel();
        let views = Arc::new(ViewRoute::default());
        let home = dir.path().to_path_buf();
        let mut runner = Runner::new(
            stub,
            Agents::Fake {
                run: dir.path().join("x"),
                review: None,
            },
            None,
            tx,
            views,
            home,
        );
        runner.start(RunSpec {
            task: 1,
            run: 1,
            harness: Harness::Claude,
            dir: dir.path().to_path_buf(),
            repo_root: dir.path().to_path_buf(),
            message: "hi".to_string(),
            resume: None,
            snapshot: false,
            baseline: None,
            attach: None,
            moment: false,
            context: functor_app_core::agent_view::app_context(
                &functor_app_core::AppState::none(),
                ".",
            ),
        });
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file) {
                break text.trim().parse::<u32>().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        runner.stop(1, 1).await;
        let alive = || {
            let mut sys = System::new();
            let pid = Pid::from_u32(pid);
            let only = ProcessesToUpdate::Some(&[pid]);
            sys.refresh_processes_specifics(only, true, ProcessRefreshKind::nothing());
            sys.process(pid)
                .is_some_and(|p| p.status() != ProcessStatus::Zombie)
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while alive() {
            assert!(
                std::time::Instant::now() < deadline,
                "tool server {pid} outlived its run"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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
            serde_json::json!({"type": "tool_use_meta", "id": "t2", "parent_id": "t1"}),
            serde_json::json!({"type": "tool_result_meta", "id": "t2", "is_error": true}),
            serde_json::json!({"type": "usage_detail",
                "total": {"input": 10, "cache_read": 21431, "cache_creation": 14233, "output": 467, "reasoning_output": 120},
                "last": {"input": 2, "cache_read": 8512, "cache_creation": 119, "output": 3, "reasoning_output": null},
                "turns": 3, "cost_usd": 0.00287526, "api_ms": 4193, "context_window": 1000000}),
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
