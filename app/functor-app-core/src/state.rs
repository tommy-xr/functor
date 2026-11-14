//! `AppState` and the pure `update` that steps it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::activity::{activities, Activity};
use crate::agent_view::{
    agent_view, app_context, auto_apply, prune_highlight, publish_review, refresh_contexts,
    step_clicked, AppContext, Highlight, ViewReply, ViewRequest,
};
use crate::changes::Change;
use crate::cli::CliProblem;
use crate::creation::{
    folder_name, CreateFailure, NewProject, NewProjectRequest, Step, Template, OPEN_STEP, TASK_STEP,
};
use crate::decision::{decisions, Decision};
use crate::devices::{self, on_link, DeviceState, Devices, LinkCommand, LinkEvent};
use crate::evidence::{self, Evidence, EvidenceState};
use crate::folders::{mark_missing, remember, Recent};
use crate::layout::{Layout, LayoutOp, Placement, Target, View};
use crate::panel::{client_msg, refresh_panels, Panel};
use crate::projects::{check_relative, default_project, project_dir, relative, Project};
use crate::protocol::{ClientMsg, ServerMsg};
use crate::queue::{self, Queue};
use crate::route::{self, By, Choice, Destination, Landed, Open, Place, RouteRequest, Routed};
use crate::review::{
    brief, default_model, review_harness, Review, ReviewEvent, ReviewSetting, ReviewSpec,
    ReviewState,
};
use crate::session::{Display, Pointer, Session, SessionEvent, SessionMode, SessionStatus};
use crate::tasks::{
    apply, derive_status, end_run, has_change, renamed, stop_run, title, AgentEvent, Harness,
    Item, Run, RunEvent, Scope, Spoken, Task, TaskStatus, Usage, UsageDetail,
};
use crate::transport::{SessionCall, Timeline, Transport, TransportCall};
use crate::voice::{self, Route, Stage, Utterance};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppState {
    /// Absolute path of the open folder's repo root (its nearest ancestor
    /// with a `.git`, else the folder); empty when no folder is open.
    pub repo_root: String,
    /// The folder the app opened, canonical; empty when none is.
    pub folder: String,
    /// Sorted by `path`.
    pub projects: Vec<Project>,
    /// The active project's `path`; `None` only when there are no projects.
    pub active: Option<String>,
    /// The active project's loadable `.fun`/`.funi` files and `functor.json`,
    /// sorted ([`crate::projects::project_files`]).
    pub files: Vec<String>,
    /// The repo's checkout; `None` outside git.
    pub git: Option<GitInfo>,
    /// The repo root is a git work tree, so tasks' changes are tracked
    /// (snapshot, diff, keep, revert). Off outside git.
    pub tracks_changes: bool,
    /// The active project's game session; `None` until one first starts.
    /// It runs as its project's game placement says (`Layout::game`).
    pub session: Option<Session>,
    /// Sessions run `--headless` (`functor-app --no-gpu`), so there are no frames.
    pub no_gpu: bool,
    /// This app turns speech into text, so the composer has a mic.
    pub speech: bool,
    /// Opening a project starts its game, paused on frame 0 (D37). Off for
    /// `--print` and `task`, which start one only when asked.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub start_on_open: bool,
    /// Queued tasks start by themselves when their turn comes (D30). Off
    /// for `--print` and `task`, which start no run unasked.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub auto_start: bool,
    /// Every project's tasks, by id; their transcripts travel separately.
    pub tasks: Vec<Task>,
    /// Per task id, what its agent is doing (D34), refreshed after every event.
    pub activity: BTreeMap<u64, Activity>,
    /// Per task id whose change waits, its decision page's size and verdict
    /// (D35), refreshed after every event.
    pub decisions: BTreeMap<u64, Decision>,
    /// Per project path with queued tasks or a reason a new one would
    /// wait, its queue (D30), refreshed after every event.
    pub queues: BTreeMap<String, Queue>,
    /// Decisions on where speech went that can still be moved (D29), with
    /// their lines and moves, refreshed after every event.
    pub routes: Vec<Routed>,
    /// Per project path, the chat's current task (`None`: a new one), the
    /// one the Tasks pane shows selected. Absent until a client first
    /// chooses one for the project; not kept across restarts.
    pub current_task: BTreeMap<String, Option<u64>>,
    /// The agents this machine can run, in the order the picker lists them.
    pub harnesses: Vec<Harness>,
    /// Per project path, the agent last picked for its new tasks.
    pub harness_choice: BTreeMap<String, Harness>,
    /// Per project path, how much of the user's own setup its runs read
    /// (D39); absent: all of it.
    pub scope_choice: BTreeMap<String, Scope>,
    /// Per project path, how its changes are reviewed (D31); absent: the
    /// default.
    pub review_settings: BTreeMap<String, ReviewSetting>,
    /// Project paths whose speech the model router places (D29; off by
    /// default: it adds 2–3 s and sometimes times out to the fixed rule).
    pub model_routes: BTreeSet<String>,
    /// The shell can run reviews (off for a fake agent without a review
    /// fixture, and for `task` without `--review`).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub can_review: bool,
    /// Per project path, its canvas layout; every project has one.
    pub layouts: BTreeMap<String, Layout>,
    /// Recent projects, most recent first; kept for the app, not the folder.
    pub recents: Vec<Recent>,
    /// The context window of each harness and model seen (`activity::window_key`),
    /// app-wide: it arrives only as a run ends, so is kept for the next.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub context_windows: BTreeMap<String, u64>,
    /// The window can show a native folder picker (`ClientMsg::PickFolder`).
    pub native_picker: bool,
    /// Why the last folder asked for did not open; cleared by the next try.
    pub open_error: Option<String>,
    /// The last folder asked for (`Command::OpenFolder`'s `id`): only its
    /// reading is used, so a slower, earlier one never lands after it.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub opening: u64,
    /// What `functor init --list --json` lists, starters first.
    pub templates: Vec<Template>,
    /// Why there are no templates (no `functor`, or one too old to list them).
    pub templates_error: Option<String>,
    /// What is wrong with the `functor` the app runs ([`CliProblem::message`]):
    /// while set, nothing is started with it (D40). `None`: nothing found wrong.
    pub cli_problem: Option<String>,
    /// The shell is checking `functor` again (Try again).
    pub cli_checking: bool,
    /// The new-project modal, while open.
    pub new_project: Option<NewProject>,
    /// Where the last project was created, the modal's next default.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub last_parent: Option<String>,
    /// A new project's first task, waiting on its game session (by id).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub first_task: Option<(u64, String, Harness)>,
    /// What the agent or a walkthrough step last pointed at (D25).
    pub highlight: Option<Highlight>,
    /// The page's canvas is too narrow for panes side by side, so they
    /// stack: the agent's views make no splits there.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub canvas_stacked: bool,
    /// A page shows a transport bar: the session is polled often and its
    /// input log read for marks.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub transport_watched: bool,
    /// The shell's clock (ms) when the event being stepped arrived, set
    /// before each `update`; the core only compares it.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub clock: u64,
    /// The shell's wall clock (ms since the Unix epoch) when the event
    /// being stepped arrived, set with `clock`; tasks' times. 0: unknown.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub now: u64,
    /// The last `Highlight::seq` given out; never reused.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub highlight_seq: u64,
    /// When (`clock`) the user last changed the layout.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub user_layout_at: Option<u64>,
    /// Per running task, the app context its tool server last got.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub contexts: BTreeMap<u64, AppContext>,
    /// Headsets, app-wide.
    pub devices: Devices,
    /// The session waiting for its headset's tunnel to start on it.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub headset_waits: Option<u64>,
    /// Per connected headset, the connection and panel it was last sent.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub panels: BTreeMap<String, (u64, Panel)>,
    /// Utterances from headsets whose runs are followed until the game
    /// reloads ([`voice::progress`]).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub voice: Vec<voice::Job>,
    /// The shell can ask a model where speech goes (D29); else the fixed rule decides.
    pub router: bool,
    /// Utterances waiting on the model router.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub routing: Vec<(Utterance, RouteRequest)>,
    /// The decisions behind `routes`; the last id given out.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub routed: Vec<Open>,
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub route_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GitInfo {
    /// `HEAD` when detached.
    pub branch: String,
    /// Abbreviated, as `git rev-parse --short` prints it.
    pub commit: String,
}

impl AppState {
    /// The state for `projects` under `root`, with the [`default_project`]
    /// for `start` (a directory under `root`) active. Files and git are the
    /// shell's to fill in.
    pub fn new(root: &Path, start: &Path, projects: Vec<Project>) -> Self {
        let paths: Vec<&str> = projects.iter().map(|p| p.path.as_str()).collect();
        let active = default_project(&paths, &relative(root, start)).map(str::to_string);
        let layouts = projects
            .iter()
            .map(|p| (p.path.clone(), Layout::initial()))
            .collect();
        Self {
            repo_root: root.to_string_lossy().into_owned(),
            folder: start.to_string_lossy().into_owned(),
            projects,
            active,
            files: Vec::new(),
            git: None,
            tracks_changes: false,
            session: None,
            no_gpu: false,
            speech: false,
            start_on_open: false,
            auto_start: false,
            tasks: Vec::new(),
            activity: BTreeMap::new(),
            decisions: BTreeMap::new(),
            queues: BTreeMap::new(),
            routes: Vec::new(),
            current_task: BTreeMap::new(),
            harnesses: Vec::new(),
            harness_choice: BTreeMap::new(),
            scope_choice: BTreeMap::new(),
            review_settings: BTreeMap::new(),
            model_routes: BTreeSet::new(),
            can_review: false,
            layouts,
            recents: Vec::new(),
            context_windows: BTreeMap::new(),
            native_picker: false,
            open_error: None,
            opening: 0,
            templates: Vec::new(),
            templates_error: None,
            cli_problem: None,
            cli_checking: false,
            new_project: None,
            last_parent: None,
            first_task: None,
            highlight: None,
            highlight_seq: 0,
            canvas_stacked: false,
            transport_watched: false,
            clock: 0,
            now: 0,
            user_layout_at: None,
            contexts: BTreeMap::new(),
            devices: Devices::default(),
            headset_waits: None,
            panels: BTreeMap::new(),
            voice: Vec::new(),
            router: false,
            routing: Vec::new(),
            routed: Vec::new(),
            route_seq: 0,
        }
    }

    /// No folder open.
    pub fn none() -> Self {
        Self::new(Path::new(""), Path::new(""), Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Client(ClientMsg),
    /// From the shell, about session `id`'s child.
    Session {
        id: u64,
        event: SessionEvent,
    },
    /// Project `path`'s files in `dir`, as `ListFiles` read them.
    Files {
        path: String,
        dir: PathBuf,
        files: Vec<String>,
    },
    /// The shell's reading of `folder` (as asked) for `OpenFolder`: the
    /// workspace as the shell loads it (repo root, projects, active project
    /// and its files, tasks, layouts, picks, git), or why it cannot be opened.
    Opened {
        id: u64,
        folder: String,
        result: Result<Box<AppState>, String>,
    },
    /// The folder chosen in the native picker; `None` if it was cancelled.
    /// `parent`: for the new project's parent folder, not to open.
    Picked {
        path: Option<String>,
        parent: bool,
    },
    /// `functor init --list --json`'s templates, or why there are none.
    Templates {
        result: Result<Vec<Template>, String>,
    },
    /// The shell's check of the `functor` it runs: what is wrong with it,
    /// or `None` when nothing is.
    CliChecked {
        problem: Option<CliProblem>,
    },
    /// The shell's `CreateProject`: the project's folder (with a `note`
    /// to show once it is open), or how it failed.
    Created {
        result: Result<String, CreateFailure>,
        note: Option<String>,
    },
    /// From the shell, about run `run` of task `task`.
    Run {
        task: u64,
        run: u64,
        event: RunEvent,
    },
    /// The saved tasks, read at startup; ids are unique.
    TasksLoaded {
        tasks: Vec<Task>,
    },
    /// The saved layouts by project path, read at startup after the tasks.
    LayoutsLoaded {
        layouts: BTreeMap<String, Layout>,
    },
    /// A client's `NewTask`, with the id the shell reserved for it (unique
    /// across app processes sharing a store), or why it could not.
    NewTask {
        id: Result<u64, String>,
        message: String,
        harness: Harness,
        /// This request's level (D39), else the project's.
        scope: Option<Scope>,
        /// The utterance it was said as, from a headset.
        voice: Option<Box<Utterance>>,
    },
    /// Task `task`'s change since `baseline`, as the shell's git computed it.
    Change {
        task: u64,
        baseline: String,
        change: Change,
    },
    /// The shell's revert of task `task`'s change since `baseline`.
    Reverted {
        task: u64,
        baseline: String,
        result: Result<(), String>,
    },
    /// Task `task`'s path evidence for the change measured as `tree`.
    Evidence {
        task: u64,
        tree: String,
        state: EvidenceState,
    },
    /// Run `run` of task `task`'s tool server asks to change the view
    /// (`POST /agent/view` with the run's secret) or, from its `review`,
    /// to report; answered with `ViewReply`.
    AgentView {
        task: u64,
        run: u64,
        review: bool,
        request: ViewRequest,
    },
    /// From the shell, about task `task`'s review of run `run`'s change.
    Review {
        task: u64,
        run: u64,
        event: ReviewEvent,
    },
    /// The model router's choice for `device`'s utterance `utterance`, or
    /// why there is none (a timeout, a failed call, an unreadable reply).
    Routed {
        device: String,
        utterance: u64,
        choice: Result<Choice, String>,
    },
    /// From the shell's device link.
    Link(LinkEvent),
    /// The app is exiting: stop the session and every run.
    Shutdown,
}

/// Why the user cannot open a folder now: [`can_open`], or a new project
/// being created (it opens itself).
fn can_open_now(state: &AppState) -> Result<(), String> {
    match &state.new_project {
        Some(np) if np.step.busy() => Err("a new project is being created".to_string()),
        _ => can_open(state),
    }
}

/// Why no other folder can be opened now: a run (or revert) of this
/// workspace's would carry on against the old one.
fn can_open(state: &AppState) -> Result<(), String> {
    match state.tasks.iter().find(|t| t.run.is_some() || t.reverting) {
        Some(t) => Err(format!(
            "task {} is running: stop it before opening another folder",
            t.id
        )),
        None => Ok(()),
    }
}

/// What the shell does after an `update`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// To the client whose message this answers.
    Reply(ServerMsg),
    /// To every connected client.
    Broadcast(ServerMsg),
    /// Spawn `functor -d <dir> run native` with a debug server on a free port.
    StartSession {
        id: u64,
        dir: PathBuf,
        display: Display,
    },
    /// Spawn `functor -d <dir> run vr --device-url <tunnel>`: the game on a
    /// paired headset, pushed through its tunnel, which is then the
    /// session's debug server. Stopping it returns the headset to its boot
    /// scene.
    StartHeadset {
        id: u64,
        dir: PathBuf,
        tunnel: String,
    },
    /// Kill session `id`'s child.
    StopSession { id: u64 },
    /// Check the `functor` the shell runs again (found anew), answering with
    /// `Event::CliChecked`.
    CheckCli,
    /// `GET /state?model=0` on session `id`'s debug server after the poll
    /// interval (shorter when `fast`), posted in order with its calls;
    /// `after` seeks were sent before it.
    PollState {
        id: u64,
        debug_url: String,
        fast: bool,
        after: u64,
    },
    /// Input or a transport call to session `id`'s debug server, in order.
    Call {
        id: u64,
        debug_url: String,
        call: SessionCall,
    },
    /// `GET /input-log?from=` on session `id`'s debug server after the
    /// marks interval, answering with `SessionEvent::InputLog`.
    ReadInputLog {
        id: u64,
        debug_url: String,
        from: u64,
    },
    /// Read project `path`'s files in `dir`, answering with `Event::Files`.
    ListFiles { path: String, dir: PathBuf },
    /// Read the folder `path` names as a workspace, answering with
    /// `Event::Opened` with the same `id`.
    OpenFolder { id: u64, path: String },
    /// The workspace read for request `id` is open: its git measures changes now.
    UseWorkspace { id: u64 },
    /// Show the native folder picker at `from`, answering with
    /// `Event::Picked` (for the new project's parent, with `parent`).
    PickFolder { from: Option<PathBuf>, parent: bool },
    /// List the templates, answering with `Event::Templates`.
    ListTemplates,
    /// Work out what `request` would do, answering the asking client with
    /// `ServerMsg::NewProjectPlan`.
    CheckNewProject { request: NewProjectRequest },
    /// Create `request`'s project in `folder` under its parent, answering
    /// with `Event::Created`.
    CreateProject {
        request: NewProjectRequest,
        folder: String,
    },
    /// Start a task as the chat's "new task" does: an id is reserved, then
    /// `Event::NewTask` (carrying `voice`).
    ReserveTask {
        message: String,
        harness: Harness,
        voice: Option<Box<Utterance>>,
    },
    /// List directories completing `path`, answering the asking client
    /// with `ServerMsg::Dirs`.
    ListDirs { path: String },
    /// Remember the recent projects, app-wide.
    SaveRecents { recents: Vec<Recent> },
    /// Keep the context windows seen (`AppState::context_windows`).
    SaveContextWindows { windows: BTreeMap<String, u64> },
    /// Start an agent run, answering with `Event::Run`s that end in `Ended`.
    StartRun(Box<RunSpec>),
    /// Cancel run `run` of task `task`, waiting until its processes are gone.
    StopRun { task: u64, run: u64 },
    /// Start a review (D31), answering with `Event::Review`s that end in
    /// `Ended`.
    StartReview(Box<ReviewSpec>),
    /// Cancel task `task`'s review of run `run`, without waiting: it edits
    /// nothing.
    StopReview { task: u64, run: u64 },
    /// Remember `setting` for the project at `dir`.
    SaveReview { dir: PathBuf, setting: ReviewSetting },
    /// Remember whether the model router places speech in the project at `dir`.
    SaveModelRoute { dir: PathBuf, on: bool },
    /// Write `task` (with its transcript) to the store for the project at `dir`.
    SaveTask { dir: PathBuf, task: Box<Task> },
    /// Remove task `task`'s file from the store for the project at `dir`;
    /// nothing else.
    DeleteTask { dir: PathBuf, task: u64 },
    /// Remember `harness` as the pick for new tasks in the project at `dir`.
    SaveHarness { dir: PathBuf, harness: Harness },
    /// Remember `scope` for the runs of the project at `dir` (D39).
    SaveScope { dir: PathBuf, scope: Scope },
    /// Remember `layout` for the project at `dir`; writes may be coalesced.
    SaveLayout { dir: PathBuf, layout: Box<Layout> },
    /// Read `path` (checked by [`check_relative`]) in project `project`'s
    /// `dir`, answering the asking client with `ServerMsg::File`.
    ReadFile {
        root: String,
        project: String,
        path: String,
        dir: PathBuf,
    },
    /// Compute task `task`'s change since `baseline`, answering with
    /// `Event::Change`; with `reply`, also its full diff to the asking
    /// client as `ServerMsg::Diff`.
    Diff {
        task: u64,
        baseline: String,
        reply: bool,
    },
    /// Put those of `paths` (the change the user reviewed) that differ from
    /// `baseline` back as they were there, answering with `Event::Reverted`.
    Revert {
        task: u64,
        baseline: String,
        paths: Vec<String>,
    },
    /// Take task `task`'s path evidence (D38): its moment replayed under
    /// project `project`'s files (repo-relative, at `dir`) in `baseline` and
    /// in `tree`, answering with `Event::Evidence`. A later one for the task
    /// replaces it.
    TakeEvidence {
        task: u64,
        dir: PathBuf,
        project: String,
        baseline: String,
        tree: String,
    },
    /// Remove these evidence files (`EvidenceState::Shown`'s).
    DropEvidence { files: Vec<String> },
    /// The answer to the `Event::AgentView` being stepped, for its caller.
    ViewReply(ViewReply),
    /// Give run `run` of task `task`'s tool server this context (`PUT
    /// /app-context`); writes may be coalesced.
    AppContext {
        task: u64,
        run: u64,
        context: AppContext,
    },
    /// Ask the model router where `device`'s utterance `utterance` goes,
    /// answering with `Event::Routed`.
    Route {
        device: String,
        utterance: u64,
        request: Box<RouteRequest>,
    },
    /// To the device link.
    Link(LinkCommand),
}

/// One agent run for the shell to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSpec {
    pub task: u64,
    pub run: u64,
    pub harness: Harness,
    /// How much of the user's own setup the run reads (D39).
    pub scope: Scope,
    /// The task's project directory: the agent's working directory.
    pub dir: PathBuf,
    /// What the agent may edit.
    pub repo_root: PathBuf,
    pub message: String,
    /// The agent session to continue; `None` starts one.
    pub resume: Option<String>,
    /// Snapshot the working tree before the agent starts, as the task's
    /// baseline: changes are tracked and the task has none yet.
    pub snapshot: bool,
    /// The task's baseline, when tracked: its change is measured against
    /// it as the run ends.
    pub baseline: Option<String>,
    /// The game session's debug server, when it runs this task's project.
    pub attach: Option<String>,
    /// Record the game's moment with the snapshot (`RunEvent::Moment`):
    /// sessions get a GPU, so its paths can be drawn (D38).
    pub moment: bool,
    pub context: AppContext,
}

pub fn update(mut state: AppState, event: Event) -> (AppState, Vec<Command>) {
    let mut commands = Vec::new();
    // A headset panel's action is the client message for the same choice.
    let (event, panel) = match event {
        Event::Link(LinkEvent::Panel { conn, action }) => match client_msg(&state, conn, action) {
            Ok(msg) => (Event::Client(msg), Some(conn)),
            Err(refused) => {
                commands.extend(refused.map(|reason| {
                    Command::Link(LinkCommand::PanelRefused { conn, reason })
                }));
                return (state, commands);
            }
        },
        event => (event, None),
    };
    let get_state = event == Event::Client(ClientMsg::GetState);
    // The exit starts no queued task; nor would a loader's step (a safeguard:
    // its state has auto-start off, and its commands are dropped).
    let starts = state.auto_start
        && !matches!(
            event,
            Event::Shutdown | Event::TasksLoaded { .. } | Event::LayoutsLoaded { .. }
        );
    let shown = transport_shown(&state);
    // A handler may move only the highlight (a step click on a shown tab).
    let highlighted = state.highlight.clone();
    let (changed, from_client) = match event {
        Event::Client(ClientMsg::GetState) => (false, true),
        Event::Client(ClientMsg::SetActiveProject { path }) => {
            let switch = state.active.as_ref() != Some(&path)
                && state.projects.iter().any(|p| p.path == path);
            if switch {
                commands.push(Command::ListFiles {
                    dir: project_dir(Path::new(&state.repo_root), &path),
                    path: path.clone(),
                });
                // Cleared, so the old project's files never show under the new name.
                state.files.clear();
                state.active = Some(path);
                remember_active(&mut state, &mut commands);
                // The session follows the active project.
                stop(&mut state, &mut commands);
                if starts_on_open(&state) {
                    start(&mut state, &mut commands);
                }
            }
            (switch, true)
        }
        // Input arrives at pointer rate, so it is never answered with the state.
        Event::Client(ClientMsg::Input { input }) => {
            if let Some(Session {
                id,
                status: SessionStatus::Running { debug_url, .. },
                ..
            }) = &state.session
            {
                commands.push(Command::Call {
                    id: *id,
                    debug_url: debug_url.clone(),
                    call: SessionCall::Input(input),
                });
            }
            return (state, commands);
        }
        // Shown at once and broadcast as `ServerMsg::Transport` (below).
        Event::Client(ClientMsg::Transport { call }) => {
            transport_call(&mut state, call, &mut commands);
            (false, false)
        }
        Event::Client(ClientMsg::WatchTransport { on }) => {
            // No bar shows any more (a page closed or lost its connection):
            // a peek one started cannot end there, so it ends here.
            if state.transport_watched && !on {
                transport_call(&mut state, TransportCall::Peek { on: false }, &mut commands);
            }
            state.transport_watched = on;
            read_marks(&mut state, &mut commands);
            (false, false)
        }
        Event::Client(ClientMsg::StartSession) => {
            let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
            (!live && start(&mut state, &mut commands), true)
        }
        Event::Client(ClientMsg::StopSession) => (stop(&mut state, &mut commands), true),
        Event::Client(ClientMsg::SetHarness { harness }) => {
            (set_harness(&mut state, harness, &mut commands), true)
        }
        Event::Client(ClientMsg::SetModelRoute { on }) => match state.active.clone() {
            Some(path) if state.model_routes.contains(&path) != on => {
                commands.push(Command::SaveModelRoute {
                    dir: project_dir(Path::new(&state.repo_root), &path),
                    on,
                });
                match on {
                    true => state.model_routes.insert(path),
                    false => state.model_routes.remove(&path),
                };
                (true, true)
            }
            _ => (false, true),
        },
        Event::Client(ClientMsg::SetScope { scope }) => {
            (set_scope(&mut state, scope, &mut commands), true)
        }
        Event::Client(ClientMsg::SetReview { setting }) => {
            (set_review(&mut state, setting, &mut commands), true)
        }
        Event::Client(ClientMsg::Layout { op }) => {
            let changed = layout_op(&mut state, op, &mut commands);
            if changed {
                state.user_layout_at = Some(state.clock);
            }
            (changed, true)
        }
        Event::Client(ClientMsg::WalkthroughStep { task, item, step }) => {
            match step_clicked(&mut state, task, item, step, &mut commands) {
                Ok(changed) => (changed, false),
                Err(reason) => (reject(&mut commands, reason), false),
            }
        }
        Event::Client(ClientMsg::ClearHighlight { seq }) => {
            let clear = state.highlight.as_ref().is_some_and(|h| h.seq == seq);
            if clear {
                state.highlight = None;
            }
            (clear, false)
        }
        Event::Client(ClientMsg::CanvasStacked { stacked }) => {
            state.canvas_stacked = stacked;
            (false, false)
        }
        Event::Client(ClientMsg::SelectTask { task, if_unset }) => {
            match select_task(&mut state, task, if_unset) {
                Ok(changed) => (changed, true),
                Err(reason) => (reject(&mut commands, reason), false),
            }
        }
        Event::Client(ClientMsg::RenameTask { task, title }) => {
            (rename_task(&mut state, task, &title, &mut commands), false)
        }
        Event::Client(ClientMsg::DeleteTask { task }) => {
            (delete_task(&mut state, task, &mut commands), false)
        }
        Event::AgentView {
            task,
            run,
            review,
            request,
        } => (
            agent_view(&mut state, task, run, review, request, &mut commands),
            false,
        ),
        Event::Link(LinkEvent::Utterance {
            conn,
            utterance,
            text,
            context,
        }) => (
            heard(&mut state, conn, utterance, text, context, &mut commands),
            false,
        ),
        Event::Review { task, run, event } => {
            (on_review(&mut state, task, run, event, &mut commands), false)
        }
        Event::Link(event) => {
            if let LinkEvent::Failed { reason }
            | LinkEvent::CablePaired {
                result: Err(reason),
                ..
            } = &event
            {
                commands.push(Command::Broadcast(ServerMsg::Rejected {
                    reason: reason.clone(),
                }));
            }
            let (changed, link) = on_link(&mut state.devices, event, state.now);
            commands.extend(link.into_iter().map(Command::Link));
            let started = start_headset(&mut state, &mut commands);
            (changed || started, false)
        }
        Event::Client(ClientMsg::AllowPairing { request }) => {
            match devices::allow(&mut state.devices, request, state.now) {
                Ok(command) => {
                    commands.push(Command::Link(command));
                    (true, false)
                }
                Err(reason) => (reject(&mut commands, reason), false),
            }
        }
        Event::Client(ClientMsg::DenyPairing { request }) => {
            match devices::deny(&mut state.devices, request) {
                Ok(command) => {
                    commands.push(Command::Link(command));
                    (true, false)
                }
                Err(reason) => (reject(&mut commands, reason), false),
            }
        }
        Event::Client(ClientMsg::PairByCable { serial, replace }) => {
            match devices::pair_by_cable(&mut state.devices, &serial, replace) {
                Ok(command) => {
                    commands.push(Command::Link(command));
                    (true, false)
                }
                Err(reason) => (reject(&mut commands, reason), false),
            }
        }
        Event::Client(ClientMsg::ForgetDevice { device }) => {
            match devices::forget(&state.devices, &device) {
                Ok(command) => commands.push(Command::Link(command)),
                Err(reason) => {
                    reject(&mut commands, reason);
                }
            }
            (false, false)
        }
        Event::Client(ClientMsg::GetFile { path }) => {
            get_file(&state, path, &mut commands);
            (false, false)
        }
        Event::Session { id, event } => {
            let changed = on_session(&mut state, id, event, &mut commands);
            release_first_task(&mut state, &mut commands);
            (changed, false)
        }
        Event::Client(ClientMsg::OpenFolder { path })
        | Event::Picked {
            path: Some(path),
            parent: false,
        } => {
            let id = state.opening + 1;
            let open = can_open_now(&state).map(|()| Command::OpenFolder { id, path });
            (try_open(&mut state, open, &mut commands), false)
        }
        Event::Picked { path: None, .. } => (false, false),
        Event::Picked {
            path: Some(path),
            parent: true,
        } => match &mut state.new_project {
            Some(np) if !np.step.busy() && np.parent != path => {
                np.parent = path;
                (true, false)
            }
            _ => (false, false),
        },
        Event::Client(ClientMsg::PickParent) => {
            match (&state.new_project, state.native_picker) {
                (Some(np), true) if !np.step.busy() => {
                    let from = Some(PathBuf::from(&np.parent)).filter(|p| p.is_absolute());
                    commands.push(Command::PickFolder { from, parent: true });
                }
                _ => {
                    reject(&mut commands, "there is no folder picker here".to_string());
                }
            }
            (false, false)
        }
        Event::Client(ClientMsg::ShowNewProject { here }) => {
            (show_new_project(&mut state, here, &mut commands), true)
        }
        Event::Client(ClientMsg::CloseNewProject) => match &state.new_project {
            Some(np) if np.step.busy() => (
                reject(&mut commands, "the project is being created".to_string()),
                false,
            ),
            _ => (state.new_project.take().is_some(), true),
        },
        Event::Client(ClientMsg::CheckNewProject { request }) => {
            commands.push(Command::CheckNewProject { request });
            (false, false)
        }
        Event::Client(ClientMsg::CreateProject { request, harness }) => (
            create_project(&mut state, request, harness, &mut commands),
            false,
        ),
        Event::Templates { result } => {
            let (templates, error) = match result {
                Ok(list) => (list, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            let changed = (&state.templates, &state.templates_error) != (&templates, &error);
            state.templates = templates;
            state.templates_error = error;
            (changed, false)
        }
        Event::Created { result, note } => {
            (created(&mut state, result, note, &mut commands), false)
        }
        Event::Client(ClientMsg::CheckCli) => {
            let check = !state.cli_checking;
            if check {
                state.cli_checking = true;
                commands.push(Command::CheckCli);
            }
            (check, true)
        }
        Event::CliChecked { problem } => (cli_checked(&mut state, problem, &mut commands), false),
        Event::Client(ClientMsg::PickFolder) => {
            if !state.native_picker {
                let reason = "there is no folder picker here: type the folder's path";
                return (state, vec![reject_cmd(reason)]);
            }
            let from = Some(PathBuf::from(&state.folder)).filter(|f| !f.as_os_str().is_empty());
            let pick = can_open_now(&state).map(|()| Command::PickFolder {
                from,
                parent: false,
            });
            (try_open(&mut state, pick, &mut commands), false)
        }
        // A reading superseded by a later request is dropped.
        Event::Opened { id, .. } if id != state.opening => (false, false),
        Event::Opened { id, folder, result } => {
            (opened(&mut state, id, folder, result, &mut commands), false)
        }
        Event::Client(ClientMsg::ListDirs { path }) => {
            commands.push(Command::ListDirs { path });
            (false, false)
        }
        Event::Client(ClientMsg::RemoveRecent { path }) => {
            let before = state.recents.len();
            state.recents.retain(|r| r.path != path);
            let removed = state.recents.len() != before;
            if removed {
                save_recents(&state, &mut commands);
            }
            (removed, true)
        }
        // A listing for a project switched away from (or another
        // workspace's of the same path) is dropped.
        Event::Files { path, dir, files } => {
            let root = Path::new(&state.repo_root);
            if state.active.as_ref() != Some(&path) || project_dir(root, &path) != dir {
                return (state, commands);
            }
            let listed = state.files != files;
            state.files = files;
            // Tabs of files that are gone close; an empty listing is an
            // unreadable directory (a project has its functor.json), not that.
            let pruned = !state.files.is_empty() && prune(&mut state, &path, &mut commands);
            (listed || pruned, false)
        }
        // Task messages that change nothing answer with `Rejected`, not the state.
        // Shells reserve an id first and send `Event::NewTask`.
        Event::Client(ClientMsg::NewTask { .. }) => (
            reject(&mut commands, "a new task needs an id".to_string()),
            false,
        ),
        Event::NewTask {
            id,
            harness,
            voice: Some(u),
            ..
        } => (
            voice_task(&mut state, id, harness, *u, &mut commands),
            false,
        ),
        Event::NewTask {
            id,
            message,
            harness,
            scope,
            voice: None,
        } => match id {
            Ok(id) => (
                new_task(&mut state, id, message, harness, scope, None, &mut commands),
                false,
            ),
            Err(e) => (
                reject(&mut commands, format!("cannot reserve a task id: {e}")),
                false,
            ),
        },
        Event::Client(ClientMsg::SendMessage { task, message }) => (
            send_message(&mut state, task, message, &mut commands),
            false,
        ),
        Event::Client(ClientMsg::StopTask { task }) => {
            (stop_task(&mut state, task, &mut commands), false)
        }
        Event::Client(ClientMsg::MoveQueued { task, to }) => {
            (move_queued(&mut state, task, to, &mut commands), false)
        }
        Event::Client(ClientMsg::StartQueued { task }) => {
            (start_now(&mut state, task, &mut commands), false)
        }
        Event::Client(ClientMsg::MoveUtterance { id, to }) => {
            (move_utterance(&mut state, id, to, &mut commands), false)
        }
        Event::Routed {
            device,
            utterance,
            choice,
        } => (
            routed(&mut state, &device, utterance, choice, &mut commands),
            false,
        ),
        Event::Client(ClientMsg::KeepChange { task }) => {
            (keep(&mut state, task, &mut commands), false)
        }
        Event::Client(ClientMsg::RevertChange { task, tree }) => {
            (revert(&mut state, task, &tree, &mut commands), false)
        }
        Event::Client(ClientMsg::GetDiff { task }) => {
            get_diff(&state, task, &mut commands);
            (false, false)
        }
        Event::Change {
            task,
            baseline,
            change,
        } => {
            let changed = on_change(&mut state, task, &baseline, change, &mut commands);
            (
                refresh_evidence(&mut state, task, &mut commands) || changed,
                false,
            )
        }
        Event::Evidence {
            task,
            tree,
            state: taken,
        } => (
            on_evidence(&mut state, task, tree, taken, &mut commands),
            false,
        ),
        Event::Reverted {
            task,
            baseline,
            result,
        } => (
            on_reverted(&mut state, task, baseline, result, &mut commands),
            false,
        ),
        Event::Client(ClientMsg::GetTranscript { task }) => {
            let reply = match state.tasks.iter().find(|t| t.id == task) {
                Some(t) => ServerMsg::Transcript {
                    task,
                    items: t.transcript.clone(),
                },
                None => no_task(task),
            };
            commands.push(Command::Reply(reply));
            (false, false)
        }
        Event::Run { task, run, event } => {
            let ended = matches!(event, RunEvent::Ended { .. });
            remember_window(&mut state, task, run, &event, &mut commands);
            let changed = on_run(&mut state, task, run, event, &mut commands);
            let shown = changed && ended && auto_apply(&mut state, task, run, &mut commands);
            let evidence = ended && refresh_evidence(&mut state, task, &mut commands);
            let review = changed && ended && start_review(&mut state, task, run, &mut commands);
            (changed || shown || evidence || review, false)
        }
        Event::TasksLoaded { tasks } => {
            state.tasks = tasks
                .into_iter()
                .map(|mut t| {
                    // A run (or revert) saved in flight died with the app.
                    t.run = None;
                    t.reverting = false;
                    if let Some(e) = t
                        .evidence
                        .as_mut()
                        .filter(|e| e.state == EvidenceState::Taking)
                    {
                        e.state = EvidenceState::Withheld {
                            reason: "the app closed before the paths were taken".to_string(),
                        };
                    }
                    if let Some(r) = t
                        .review
                        .as_mut()
                        .filter(|r| r.state == ReviewState::Preparing)
                    {
                        r.state = ReviewState::Failed {
                            reason: "the app closed before the review finished".to_string(),
                        };
                    }
                    t.status = derive_status(&t);
                    t
                })
                .collect();
            state.tasks.sort_by_key(|t| t.id);
            (true, false)
        }
        Event::LayoutsLoaded { layouts } => {
            for (path, layout) in layouts {
                if state.projects.iter().any(|p| p.path == path) {
                    state.layouts.insert(path.clone(), layout);
                    prune(&mut state, &path, &mut commands);
                }
            }
            (true, false)
        }
        Event::Shutdown => {
            stop(&mut state, &mut commands);
            let running: Vec<u64> = state
                .tasks
                .iter()
                .filter(|t| t.run.is_some())
                .map(|t| t.id)
                .collect();
            for id in running {
                stop_task(&mut state, id, &mut commands);
            }
            for task in &state.tasks {
                stop_review(task, &mut commands);
            }
            (false, false)
        }
    };
    let started = starts && start_ready(&mut state, &mut commands);
    voice::progress(&mut state, &mut commands);
    prune_highlight(&mut state);
    let activity = activities(&state.tasks, &state.context_windows);
    let decided = decisions(&state.tasks);
    let queues = queue::queues(&state.tasks, state.tracks_changes, state.active.as_deref());
    let mut open = std::mem::take(&mut state.routed);
    open.retain(|o| route::is_open(&state, o));
    state.routed = open;
    let routes = route::routes(&state);
    let changed = changed
        || started
        || state.highlight != highlighted
        || state.activity != activity
        || state.decisions != decided
        || state.queues != queues
        || state.routes != routes;
    state.activity = activity;
    state.decisions = decided;
    state.queues = queues;
    state.routes = routes;
    refresh_contexts(&mut state, &mut commands);
    refresh_panels(&mut state, &mut commands);
    // A client message that changed nothing still answers with the state, so
    // the sender sees what is active.
    let msg = || ServerMsg::State {
        state: Box::new(state.clone()),
    };
    if changed {
        // Before any transcript items, so a client knows their task first.
        let at = commands
            .iter()
            .position(|c| matches!(c, Command::Broadcast(ServerMsg::Item { .. })))
            .unwrap_or(commands.len());
        commands.insert(at, Command::Broadcast(msg()));
    } else if from_client {
        commands.push(Command::Reply(msg()));
    }
    send_transport(&state, shown, get_state, &mut commands);
    // The headset has no reply: a refusal goes to its panel.
    if let Some(conn) = panel {
        commands.retain_mut(|c| match c {
            Command::Reply(ServerMsg::Rejected { reason }) => {
                *c = Command::Link(LinkCommand::PanelRefused {
                    conn,
                    reason: std::mem::take(reason),
                });
                true
            }
            Command::Reply(_) => false,
            _ => true,
        });
    }
    (state, commands)
}

/// The active session's timeline, while it runs.
fn timeline(state: &AppState) -> Option<(u64, &Timeline)> {
    let session = state.session.as_ref()?;
    match &session.status {
        SessionStatus::Running { timeline, .. } => Some((session.id, timeline)),
        _ => None,
    }
}

/// What the clients were last sent: the transport, and whose marks at
/// which revision (`None`: no marks).
type Shown = (Option<Transport>, Option<(u64, u64)>);

fn transport_shown(state: &AppState) -> Shown {
    let timeline = timeline(state);
    (
        timeline.and_then(|(_, t)| t.clock),
        timeline
            .filter(|(_, t)| !t.marks.is_empty())
            .map(|(id, t)| (id, t.marks_rev)),
    )
}

/// Broadcasts the transport and marks where they changed since `before`; a
/// client asking for the state gets them too, when there are any. Apart from
/// the state, as they change every poll.
fn send_transport(state: &AppState, before: Shown, get_state: bool, commands: &mut Vec<Command>) {
    let now = transport_shown(state);
    let send = |msg| {
        if get_state {
            Command::Reply(msg)
        } else {
            Command::Broadcast(msg)
        }
    };
    if (get_state && now.0.is_some()) || (!get_state && now.0 != before.0) {
        commands.push(send(ServerMsg::Transport { transport: now.0 }));
    }
    if (get_state && now.1.is_some()) || (!get_state && now.1 != before.1) {
        let marks = timeline(state)
            .map(|(_, t)| t.marks.clone())
            .unwrap_or_default();
        commands.push(send(ServerMsg::Marks { marks }));
    }
}

/// Posts the bar's `call` to the running session, as its timeline allows.
fn transport_call(state: &mut AppState, call: TransportCall, commands: &mut Vec<Command>) {
    if let Some(Session {
        id,
        status:
            SessionStatus::Running {
                debug_url,
                timeline,
                ..
            },
        ..
    }) = &mut state.session
    {
        if let Some(call) = timeline.call(call) {
            commands.push(Command::Call {
                id: *id,
                debug_url: debug_url.clone(),
                call: SessionCall::Transport(call),
            });
        }
    }
}

/// Starts reading the session's input log for marks, if a page shows them
/// and no read is on its way.
fn read_marks(state: &mut AppState, commands: &mut Vec<Command>) {
    let watched = state.transport_watched;
    let Some(Session {
        id,
        status:
            SessionStatus::Running {
                debug_url,
                timeline,
                ..
            },
        ..
    }) = &mut state.session
    else {
        return;
    };
    if !watched || timeline.reading || !timeline.clock.is_some_and(|c| c.clock) {
        return;
    }
    timeline.reading = true;
    commands.push(Command::ReadInputLog {
        id: *id,
        debug_url: debug_url.clone(),
        from: timeline.next_read(),
    });
}

/// Starts opening a folder with `command`, or records why not in
/// `open_error`; whether the state changed.
fn try_open(
    state: &mut AppState,
    command: Result<Command, String>,
    commands: &mut Vec<Command>,
) -> bool {
    let error = match command {
        Ok(command) => {
            if let Command::OpenFolder { id, .. } = command {
                state.opening = id;
            }
            commands.push(command);
            None
        }
        Err(reason) => Some(reason),
    };
    let changed = state.open_error != error;
    state.open_error = error;
    changed
}

/// Swaps in the workspace the shell read for `folder`, stopping the
/// session; or records why it cannot in `open_error`.
fn opened(
    state: &mut AppState,
    id: u64,
    folder: String,
    result: Result<Box<AppState>, String>,
    commands: &mut Vec<Command>,
) -> bool {
    let new = match result {
        Ok(new) => new,
        Err(reason) => {
            state.open_error = Some(format!("cannot open {folder}: {reason}"));
            if mark_missing(&mut state.recents, &folder) {
                save_recents(state, commands);
            }
            not_opened(state, id);
            return true;
        }
    };
    // A run may have started while the folder was read.
    if let Err(reason) = can_open(state) {
        state.open_error = Some(reason);
        not_opened(state, id);
        return true;
    }
    stop(state, commands);
    // Their results would land on the old folder's tasks.
    for task in &state.tasks {
        stop_review(task, commands);
    }
    state.open_error = None;
    // Every workspace field, by name, so a new one is not left behind.
    let AppState {
        repo_root,
        folder,
        projects,
        active,
        files,
        git,
        tracks_changes,
        tasks,
        current_task,
        harness_choice,
        scope_choice,
        review_settings,
        model_routes,
        layouts,
        // Derived from `tasks` at the end of the step.
        activity: _,
        decisions: _,
        queues: _,
        routes: _,
        routing: _,
        routed: _,
        route_seq: _,
        router: _,
        session: _,
        no_gpu: _,
        start_on_open: _,
        auto_start: _,
        harnesses: _,
        can_review: _,
        recents: _,
        context_windows: _,
        native_picker: _,
        open_error: _,
        opening: _,
        templates: _,
        templates_error: _,
        cli_problem: _,
        cli_checking: _,
        new_project: _,
        last_parent: _,
        first_task: _,
        highlight: _,
        highlight_seq: _,
        canvas_stacked: _,
        transport_watched: _,
        clock: _,
        now: _,
        user_layout_at: _,
        contexts: _,
        devices: _,
        headset_waits: _,
        panels: _,
        voice: _,
        speech: _,
    } = *new;
    commands.push(Command::UseWorkspace { id });
    *state = AppState {
        repo_root,
        folder,
        projects,
        active,
        files,
        git,
        tracks_changes,
        tasks,
        current_task,
        harness_choice,
        scope_choice,
        review_settings,
        model_routes,
        layouts,
        // The old folder's highlight could name a project of the same path.
        highlight: None,
        ..std::mem::replace(state, AppState::none())
    };
    remember_active(state, commands);
    // Speech waiting on the router was said to the old folder's game.
    for (u, _) in std::mem::take(&mut state.routing) {
        let reason = "another folder was opened".to_string();
        voice::status(commands, &u.device, u.utterance, Stage::Failed(reason));
    }
    // A change left unmeasured (the app quit with its diff in flight) would
    // hold the queue until someone opened it: measured now.
    if state.tracks_changes {
        for task in state.tasks.iter().filter(|t| t.change.is_none()) {
            if let Some(baseline) = &task.baseline {
                commands.push(Command::Diff {
                    task: task.id,
                    baseline: baseline.clone(),
                    reply: false,
                });
            }
        }
    }
    // A project just created: its game starts, then its first task.
    let mut started = false;
    if let Some(NewProject {
        step:
            Step::Opening {
                id: made,
                task,
                note,
                ..
            },
        ..
    }) = &state.new_project
    {
        if *made == id {
            let task = task.clone();
            // Shown where folders' troubles are.
            state.open_error = note.clone();
            state.new_project = None;
            started = start(state, commands);
            // Once the game listens, so the agent's tools attach to it.
            let session = state.session.as_ref().map(|s| s.id);
            state.first_task = task.zip(session).map(|((m, h), id)| (id, m, h));
        }
    }
    if !started && starts_on_open(state) {
        start(state, commands);
    }
    true
}

/// Whether opening the active project starts its game: in the app (not
/// `--print` or `task`), shown in a pane or Mini or on a headset rather than
/// its own window.
fn starts_on_open(state: &AppState) -> bool {
    state.start_on_open
        && state
            .active
            .as_ref()
            .is_some_and(|p| placement(state, p).session_mode() != SessionMode::Window)
}

/// Starts a new project's first task once its game session is past
/// starting: listening (its tools attach to it), or ended or stopped (the
/// agent can still work on the files).
fn release_first_task(state: &mut AppState, commands: &mut Vec<Command>) {
    let Some((id, _, _)) = &state.first_task else {
        return;
    };
    let starting = state
        .session
        .as_ref()
        .is_some_and(|s| s.id == *id && s.status == SessionStatus::Starting);
    if !starting {
        let (_, message, harness) = state.first_task.take().expect("checked");
        commands.push(Command::ReserveTask {
            message,
            harness,
            voice: None,
        });
    }
}

/// A created project that open request `id` failed to open: the modal
/// says so (the reason is `open_error`).
fn not_opened(state: &mut AppState, id: u64) {
    let Some(np) = &mut state.new_project else {
        return;
    };
    if let Step::Opening {
        path,
        id: made,
        task,
        ..
    } = &np.step
    {
        if *made == id {
            let mut not_done = vec![OPEN_STEP.to_string()];
            let path = format!("{path}, with the template's files");
            if task.is_some() {
                not_done.push(TASK_STEP.to_string());
            }
            np.step = Step::Failed {
                reason: state.open_error.clone().unwrap_or_default(),
                done: vec![format!("Created {path}")],
                not_done,
            };
        }
    }
}

/// Opens the new-project modal: empty, or (`here`) for the open folder.
/// Its parent defaults to where the last project went, else beside the
/// open workspace, else the home folder. The templates are listed again.
fn show_new_project(state: &mut AppState, here: bool, commands: &mut Vec<Command>) -> bool {
    if state.new_project.is_some() {
        return false;
    }
    let folder = Path::new(&state.folder);
    let (name, parent) = match (here, folder.parent(), folder.file_name()) {
        // The folder itself, when its name is a project folder name as it is;
        // else a new folder in it.
        (true, Some(parent), Some(name))
            if folder_name(&name.to_string_lossy()).as_deref() == Ok(&*name.to_string_lossy()) =>
        {
            (name.to_string_lossy(), parent.to_string_lossy())
        }
        (true, _, _) if !state.folder.is_empty() => ("".into(), state.folder.as_str().into()),
        _ => {
            let beside = Path::new(&state.repo_root)
                .parent()
                .map(|p| p.to_string_lossy());
            let parent = state.last_parent.clone().map(Into::into).or(beside);
            ("".into(), parent.unwrap_or("~".into()))
        }
    };
    state.new_project = Some(NewProject {
        name: name.into_owned(),
        parent: parent.into_owned(),
        step: Step::Editing,
    });
    commands.push(Command::ListTemplates);
    true
}

/// Starts creating `request`'s project, or records why not in the modal.
fn create_project(
    state: &mut AppState,
    request: NewProjectRequest,
    harness: Harness,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(np) = &state.new_project else {
        return reject(commands, "the new-project window is not open".to_string());
    };
    if np.step.busy() {
        return reject(commands, "the project is being created".to_string());
    }
    let known = state.templates.iter().any(|t| t.name == request.template);
    let checked =
        folder_name(&request.name).and_then(|folder| match (&state.templates_error, known) {
            (Some(why), _) => Err(why.clone()),
            (None, false) => Err(format!("there is no template {:?}", request.template)),
            (None, true) => can_open(state).map(|()| folder),
        });
    let step = match checked {
        Ok(folder) => {
            let path = format!("{}/{folder}", request.parent.trim_end_matches('/'));
            let describe = request.describe.trim().to_string();
            let task = (!describe.is_empty()).then_some((describe, harness));
            state.last_parent = Some(request.parent.clone());
            commands.push(Command::CreateProject { request, folder });
            Step::Creating { path, task }
        }
        Err(reason) => Step::Failed {
            reason,
            done: Vec::new(),
            not_done: Vec::new(),
        },
    };
    let np = state.new_project.as_mut().expect("checked");
    let changed = np.step != step;
    np.step = step;
    changed
}

/// The shell checked its `functor`. Once a problem is gone, what it refused
/// starts again: the failed game session, the templates.
fn cli_checked(state: &mut AppState, problem: Option<CliProblem>, commands: &mut Vec<Command>) -> bool {
    let message = problem.map(|p| p.message());
    let fixed = state.cli_problem.is_some() && message.is_none();
    let changed = state.cli_checking || state.cli_problem != message;
    state.cli_checking = false;
    state.cli_problem = message;
    if fixed {
        let failed = state.session.as_ref().is_some_and(|s| matches!(s.status, SessionStatus::Failed { .. }));
        if failed {
            start(state, commands);
        }
        if state.templates_error.is_some() {
            commands.push(Command::ListTemplates);
        }
    }
    changed
}

/// The shell created the project (and opens it now) or failed.
fn created(
    state: &mut AppState,
    result: Result<String, CreateFailure>,
    note: Option<String>,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(np) = &mut state.new_project else {
        return false;
    };
    let Step::Creating { task, .. } = &np.step else {
        return false;
    };
    np.step = match result {
        Ok(path) => {
            let id = state.opening + 1;
            state.opening = id;
            state.open_error = None;
            commands.push(Command::OpenFolder {
                id,
                path: path.clone(),
            });
            Step::Opening {
                path,
                id,
                note,
                task: task.clone(),
            }
        }
        Err(CreateFailure {
            reason,
            done,
            not_done,
        }) => Step::Failed {
            reason,
            done,
            not_done,
        },
    };
    true
}

/// Puts the active project first in the recent projects.
fn remember_active(state: &mut AppState, commands: &mut Vec<Command>) {
    let Some(active) = &state.active else {
        return;
    };
    // Rebuilt from its parts, so one directory has one spelling: on Windows
    // `games/a` under `C:\r` joins as `C:\r\games/a`.
    let dir: PathBuf = project_dir(Path::new(&state.repo_root), active)
        .components()
        .collect();
    if remember(&mut state.recents, &dir.to_string_lossy()) {
        save_recents(state, commands);
    }
}

fn save_recents(state: &AppState, commands: &mut Vec<Command>) {
    commands.push(Command::SaveRecents {
        recents: state.recents.clone(),
    });
}

/// Stops the session, then starts a new one if it was live. An ended
/// session's outcome is cleared rather than carried over.
fn restart_live(state: &mut AppState, commands: &mut Vec<Command>) {
    let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
    stop(state, commands);
    if live {
        start(state, commands);
    }
}

/// Starts a session for the active project; false when there is none.
/// Placed in a pane, its Game tab opens in the focused pane if it is not
/// open: a live session in a pane always has its tab (see `layout_op`).
fn start(state: &mut AppState, commands: &mut Vec<Command>) -> bool {
    let Some(path) = state.active.clone() else {
        return false;
    };
    let mut layout = state.layouts.get(&path).cloned().unwrap_or_default();
    let placement = layout.game.placement;
    if placement.tabbed() && layout.find(&View::Game).is_none() {
        let open = LayoutOp::Open {
            view: View::Game,
            target: Target::Focused,
        };
        if layout.apply(open, |_| Ok(())) == Ok(true) {
            save_layout(state, &path, layout, commands);
        }
    }
    let id = state.session.as_ref().map_or(1, |s| s.id + 1);
    let dir = project_dir(Path::new(&state.repo_root), &path);
    state.session = Some(Session {
        id,
        pid: None,
        status: SessionStatus::Starting,
    });
    match Display::new(placement.session_mode(), state.no_gpu) {
        Some(display) => commands.push(Command::StartSession { id, dir, display }),
        None => {
            state.headset_waits = Some(id);
            start_headset(state, commands);
        }
    }
    true
}

/// Starts the session waiting on a headset once its tunnel is known; fails
/// it if the headset is not paired here or the link is off. Whether
/// anything changed.
fn start_headset(state: &mut AppState, commands: &mut Vec<Command>) -> bool {
    let (Some(id), Some(path)) = (state.headset_waits, state.active.clone()) else {
        return false;
    };
    let headset = state
        .layouts
        .get(&path)
        .and_then(|l| l.game.headset.clone());
    let card = headset.and_then(|h| state.devices.list.iter().find(|d| d.id == h && d.paired));
    let outcome = match (card.and_then(|d| d.tunnel.clone()), card) {
        _ if state.devices.started && state.devices.listening.is_none() => Err(
            "The device link is off in this app, so no headset can be reached. Choose Pane to run \
the game here."
                .to_string(),
        ),
        (Some(tunnel), _) => Ok(tunnel),
        // Its tunnel opens just after the link starts.
        (None, Some(_)) => return false,
        (None, None) if !state.devices.started => return false,
        (None, None) => Err(
            "That headset is no longer paired with this computer. Choose Pane to run the game here."
                .to_string(),
        ),
    };
    state.headset_waits = None;
    let Some(session) = state.session.as_mut().filter(|s| s.id == id) else {
        return false;
    };
    match outcome {
        Ok(tunnel) => commands.push(Command::StartHeadset {
            id,
            dir: project_dir(Path::new(&state.repo_root), &path),
            tunnel,
        }),
        Err(reason) => session.status = SessionStatus::Failed { reason },
    }
    true
}

/// Stops the session, killing a live child; false when already stopped.
fn stop(state: &mut AppState, commands: &mut Vec<Command>) -> bool {
    let Some(session) = &mut state.session else {
        return false;
    };
    if session.status == SessionStatus::Stopped {
        return false;
    }
    if session.status.is_live() {
        commands.push(Command::StopSession { id: session.id });
    }
    state.headset_waits = None;
    session.status = SessionStatus::Stopped;
    session.pid = None;
    true
}

/// Records `harness` as the active project's pick; false when it already is.
fn set_harness(state: &mut AppState, harness: Harness, commands: &mut Vec<Command>) -> bool {
    let Some(path) = &state.active else {
        return false;
    };
    if !state.harnesses.contains(&harness) {
        return reject(commands, unavailable(harness));
    }
    if state.harness_choice.get(path) == Some(&harness) {
        return false;
    }
    commands.push(Command::SaveHarness {
        dir: project_dir(Path::new(&state.repo_root), path),
        harness,
    });
    state.harness_choice.insert(path.clone(), harness);
    true
}

/// Records `scope` as the active project's level (D39); false when it
/// already is.
fn set_scope(state: &mut AppState, scope: Scope, commands: &mut Vec<Command>) -> bool {
    let Some(path) = &state.active else {
        return false;
    };
    if state.scope_choice.get(path).copied().unwrap_or_default() == scope {
        return false;
    }
    commands.push(Command::SaveScope {
        dir: project_dir(Path::new(&state.repo_root), path),
        scope,
    });
    state.scope_choice.insert(path.clone(), scope);
    true
}

/// Applies `op` to the active project's layout. A live game in a pane
/// keeps its tab: closing it is refused (Stop game, or another placement,
/// first). A placement whose session runs differently restarts it.
pub(crate) fn layout_op(state: &mut AppState, op: LayoutOp, commands: &mut Vec<Command>) -> bool {
    let Some(project) = state.active.clone() else {
        return false;
    };
    let before = state.layouts.get(&project).cloned().unwrap_or_default();
    let mut layout = before.clone();
    // The game goes to a headset only while it is connected, and named.
    match &op {
        LayoutOp::Place {
            placement: Placement::Headset,
        } => {
            return reject(
                commands,
                "choose which headset to run the game on".to_string(),
            )
        }
        LayoutOp::PlaceOnHeadset { device } if !connected(&state.devices, device) => {
            return reject(commands, "that headset is not connected".to_string())
        }
        _ => {}
    }
    let restore = op == LayoutOp::Restore;
    let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
    // A running game keeps its tab: "Close agent tabs" leaves it, unmarked.
    if op == LayoutOp::CloseAgentTabs && live {
        layout.agent.tabs.retain(|t| *t != View::Game);
    }
    match layout.apply(op, |view| check_view(state, &project, view)) {
        Ok(true) => {
            let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
            // Restoring keeps a running game's tab, in the pane the focus is
            // back in, when the snapshot has none.
            if restore
                && live
                && layout.game.placement.tabbed()
                && layout.find(&View::Game).is_none()
            {
                let game = LayoutOp::Open {
                    view: View::Game,
                    target: Target::Focused,
                };
                let _ = layout.apply(game, |_| Ok(()));
            }
            if live && layout.game.placement.tabbed() && layout.find(&View::Game).is_none() {
                let reason = "the game is running: stop it, or switch it to Mini, to close its tab";
                return reject(commands, reason.to_string());
            }
            // Another headset runs it elsewhere too.
            let runs = |l: &Layout| (l.game.placement.session_mode(), l.game.headset.clone());
            let restart = runs(&before) != runs(&layout);
            save_layout(state, &project, layout, commands);
            if restart {
                restart_live(state, commands);
            }
            // An ended run's reason was another placement's.
            if let Some(s) = state.session.as_mut().filter(|s| restart && !s.status.is_live()) {
                s.status = SessionStatus::Stopped;
            }
            true
        }
        Ok(false) => false,
        Err(reason) => reject(commands, reason),
    }
}

/// Paired device `id` is linked now.
pub(crate) fn connected(devices: &Devices, id: &str) -> bool {
    devices
        .list
        .iter()
        .any(|d| d.id == id && d.paired && matches!(d.state, DeviceState::Connected { .. }))
}

/// `project`'s game placement.
pub(crate) fn placement(state: &AppState, project: &str) -> Placement {
    state
        .layouts
        .get(project)
        .map(|l| l.game.placement)
        .unwrap_or_default()
}

/// Drops `project`'s tabs whose view no longer exists; whether any did.
fn prune(state: &mut AppState, project: &str, commands: &mut Vec<Command>) -> bool {
    let Some(before) = state.layouts.get(project) else {
        return false;
    };
    let mut layout = before.clone();
    layout.retain(|view| check_view(state, project, view).is_ok());
    let changed = &layout != before;
    if changed {
        save_layout(state, project, layout, commands);
    }
    changed
}

pub(crate) fn save_layout(
    state: &mut AppState,
    project: &str,
    layout: Layout,
    commands: &mut Vec<Command>,
) {
    commands.push(Command::SaveLayout {
        dir: project_dir(Path::new(&state.repo_root), project),
        layout: Box::new(layout.clone()),
    });
    state.layouts.insert(project.to_string(), layout);
}

/// Why `view` cannot be open in `project`'s layout. Files are checked
/// against the listing only for the active project, the one listed.
pub(crate) fn check_view(state: &AppState, project: &str, view: &View) -> Result<(), String> {
    match view {
        View::Game => Ok(()),
        View::File { path } => {
            check_relative(path)?;
            let listed = state.active.as_deref() != Some(project) || state.files.contains(path);
            match listed {
                true => Ok(()),
                false => Err(format!("{path} is not one of {project}'s files")),
            }
        }
        View::Changes { task } | View::Task { task } => {
            match state
                .tasks
                .iter()
                .any(|t| t.id == *task && t.project == project)
            {
                true => Ok(()),
                false => Err(format!("there is no task {task} in {project}")),
            }
        }
    }
}

/// Answers `GetFile`: the shell reads one of the active project's listed
/// files, others are answered with why not.
fn get_file(state: &AppState, path: String, commands: &mut Vec<Command>) {
    let Some(project) = state.active.clone() else {
        reject(
            commands,
            "there is no project to read a file in".to_string(),
        );
        return;
    };
    match check_view(state, &project, &View::File { path: path.clone() }) {
        Ok(()) => commands.push(Command::ReadFile {
            root: state.repo_root.clone(),
            dir: project_dir(Path::new(&state.repo_root), &project),
            project,
            path,
        }),
        Err(error) => commands.push(Command::Reply(ServerMsg::File {
            root: state.repo_root.clone(),
            project,
            path,
            text: None,
            error: Some(error),
        })),
    }
}

fn reject(commands: &mut Vec<Command>, reason: String) -> bool {
    commands.push(reject_cmd(&reason));
    false
}

fn reject_cmd(reason: &str) -> Command {
    Command::Reply(ServerMsg::Rejected {
        reason: reason.to_string(),
    })
}

fn no_task(id: u64) -> ServerMsg {
    ServerMsg::Rejected {
        reason: format!("there is no task {id}"),
    }
}

/// `now` as a task time: `None` when the shell gave no clock.
fn stamp(now: u64) -> Option<u64> {
    (now > 0).then_some(now)
}

/// Sets the active project's current task (with `if_unset`, only when it
/// has none); whether it changed. A task must be one of that project's.
fn select_task(state: &mut AppState, task: Option<u64>, if_unset: bool) -> Result<bool, String> {
    let Some(project) = state.active.clone() else {
        return Err("there is no project to choose a task in".to_string());
    };
    if if_unset && state.current_task.contains_key(&project) {
        return Ok(false);
    }
    if let Some(id) = task {
        if !state
            .tasks
            .iter()
            .any(|t| t.id == id && t.project == project)
        {
            return Err(format!("there is no task {id} in {project}"));
        }
    }
    Ok(state.current_task.insert(project, task) != Some(task))
}

/// Gives task `id` the title the user typed.
fn rename_task(state: &mut AppState, id: u64, title: &str, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    let title = match renamed(title) {
        Ok(title) => title,
        Err(reason) => return reject(commands, reason),
    };
    if task.title == title {
        return false;
    }
    task.title = title;
    commands.push(save(&root, task));
    true
}

/// Why task `id` cannot be deleted now: its run (or revert) would carry on
/// writing it, and its change would be left with no way to review it.
fn can_delete(state: &AppState, task: &Task) -> Result<(), String> {
    let id = task.id;
    if task.run.is_some() || task.reverting {
        return Err(format!(
            "task {id} is running: stop it before deleting the task"
        ));
    }
    if state.tracks_changes && has_change(task) {
        return Err(format!(
            "task {id} has a change to review: keep or revert it before deleting the task"
        ));
    }
    // Not measured yet (a stop's diff in flight, or one that failed): it may hold edits.
    if state.tracks_changes && task.baseline.is_some() && task.change.is_none() {
        return Err(format!(
            "task {id}'s change is not measured yet: open its changes, then try again"
        ));
    }
    Ok(())
}

/// Forgets task `id`: the shell removes its stored file (the conversation),
/// its tabs close, and a chat on it moves to a new task. Files in the
/// repo are never touched.
fn delete_task(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let Some(at) = state.tasks.iter().position(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    if let Err(reason) = can_delete(state, &state.tasks[at]) {
        return reject(commands, reason);
    }
    let task = state.tasks.remove(at);
    state.contexts.remove(&id);
    for current in state.current_task.values_mut() {
        if *current == Some(id) {
            *current = None;
        }
    }
    // Its Task and Changes tabs no longer check out, so they close.
    prune(state, &task.project, commands);
    drop_files(task.evidence, commands);
    commands.push(Command::DeleteTask {
        dir: project_dir(Path::new(&state.repo_root), &task.project),
        task: id,
    });
    true
}

/// Starts task `id` in the active project with `message` (reading `scope` of
/// the user's setup, else the project's level), or queues it while the
/// project is busy ([`queue::holds`], D30); refused where nothing starts
/// unasked.
fn new_task(
    state: &mut AppState,
    id: u64,
    message: String,
    harness: Harness,
    scope: Option<Scope>,
    spoken: Option<Spoken>,
    commands: &mut Vec<Command>,
) -> bool {
    match create_task(state, id, message, harness, scope, spoken, commands) {
        Ok(()) => true,
        Err(reason) => reject(commands, reason),
    }
}

/// [`new_task`], or why not.
fn create_task(
    state: &mut AppState,
    id: u64,
    message: String,
    harness: Harness,
    scope: Option<Scope>,
    spoken: Option<Spoken>,
    commands: &mut Vec<Command>,
) -> Result<(), String> {
    let Some(project) = state.active.clone() else {
        return Err("there is no project to start a task in".to_string());
    };
    if message.trim().is_empty() {
        return Err("the message is empty".to_string());
    }
    let wait = queue::holds(&state.tasks, state.tracks_changes, &project);
    if let (Some(wait), false) = (&wait, state.auto_start) {
        return Err(wait.reason());
    }
    state.current_task.insert(project.clone(), Some(id));
    let scope = scope.or_else(|| state.scope_choice.get(&project).copied());
    let mut task = Task {
        id,
        project,
        title: title(&message),
        status: TaskStatus::Queued,
        harness,
        scope: scope.unwrap_or_default().for_harness(harness),
        model: None,
        session_id: None,
        run: None,
        queued: Some(queue::last_place(&state.tasks)),
        baseline: None,
        change: None,
        reverting: false,
        moment: None,
        evidence: None,
        report: None,
        review: None,
        created_at: stamp(state.now),
        updated_at: None,
        transcript: vec![Item::User {
            text: message,
            spoken,
        }],
    };
    if wait.is_some() {
        commands.push(save(Path::new(&state.repo_root), &task));
        broadcast_items(commands, &task, vec![0]);
        state.tasks.push(task);
        return Ok(());
    }
    task.queued = None;
    state.tasks.push(task);
    start_run(state, id, commands);
    Ok(())
}

/// A message to task `id`: added to it while it is queued, a steer while
/// it runs, else a follow-up resuming its agent session.
fn send_message(
    state: &mut AppState,
    id: u64,
    message: String,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let now = stamp(state.now);
    let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    if message.trim().is_empty() {
        return reject(commands, "the message is empty".to_string());
    }
    if task.queued.is_some() {
        // Sent with the rest when its turn comes (D30).
        task.transcript.push(Item::User {
            text: message,
            spoken: None,
        });
        task.updated_at = now.or(task.updated_at);
        commands.push(save(&root, task));
        broadcast_items(commands, task, vec![task.transcript.len() - 1]);
        return true;
    }
    if task.run.is_some() {
        steer(state, id, message, None, commands);
        return true;
    }
    let project = task.project.clone();
    if let Err(reason) = can_run(state, &project, &message) {
        return reject(commands, reason);
    }
    say(state, id, message, None);
    start_run(state, id, commands);
    true
}

/// Why `message` cannot start a follow-up run in `project` now: one run
/// per project at a time, and none during a revert.
pub(crate) fn can_run(state: &AppState, project: &str, message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("the message is empty".to_string());
    }
    busy(state, project)
}

/// Why no run can start in `project` now: it runs one, or a revert runs.
fn busy(state: &AppState, project: &str) -> Result<(), String> {
    match state
        .tasks
        .iter()
        .find(|t| t.project == project && t.run.is_some())
    {
        Some(busy) => Err(format!(
            "task {} is running in {project}: one run per project at a time",
            busy.id
        )),
        // A revert writes anywhere in the repo, as an agent may.
        None => match state.tasks.iter().find(|t| t.reverting) {
            Some(t) => Err(format!("task {}'s change is being reverted", t.id)),
            None => Ok(()),
        },
    }
}

/// Adds the user's `message` to task `id`'s transcript.
fn say(state: &mut AppState, id: u64, message: String, spoken: Option<Spoken>) {
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("a known task");
    task.transcript.push(Item::User {
        text: message,
        spoken,
    });
}

/// Steers running task `id` with `message` (D29): the agent CLIs take no
/// input mid-run, so its run stops and the same conversation resumes with
/// it. The task keeps its baseline, so the change accumulates. A run cut
/// before its agent started ends with no `Stopped`: its messages go again
/// with this one ([`queue::prompt`]).
fn steer(
    state: &mut AppState,
    id: u64,
    message: String,
    spoken: Option<Spoken>,
    commands: &mut Vec<Command>,
) {
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("running");
    match task.run.take() {
        Some(run) if !run.started => {
            commands.push(Command::StopRun { task: id, run: run.id });
            // What it left (a snapshot only) is measured as the next run ends.
            task.change = None;
        }
        run => {
            task.run = run;
            stop_task(state, id, commands);
        }
    }
    say(state, id, message, spoken);
    start_run(state, id, commands);
}

/// Each free project's next queued task starts (D30); whether one did.
/// The active project's waits for its game to listen, so its agent attaches
/// to it and the game's moment is taken (as a new project's first task does).
fn start_ready(state: &mut AppState, commands: &mut Vec<Command>) -> bool {
    let starting = (state.session.as_ref()).is_some_and(|s| s.status == SessionStatus::Starting);
    let mut ready = queue::ready(&state.tasks, state.tracks_changes);
    ready.retain(|&id| {
        let task = state.tasks.iter().find(|t| t.id == id).expect("ready");
        !(starting && state.active.as_ref() == Some(&task.project))
    });
    for &id in &ready {
        let task = state.tasks.iter_mut().find(|t| t.id == id).expect("ready");
        task.queued = None;
        start_run(state, id, commands);
    }
    !ready.is_empty()
}

/// Starts queued task `id` now, ahead of its turn: the user's choice to
/// start it beside a change waiting for a decision, as a new task may be.
/// Refused while its project runs a task or a revert runs.
fn start_now(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    if task.queued.is_none() {
        return reject(commands, format!("task {id} is not queued"));
    }
    let project = task.project.clone();
    if let Err(reason) = busy(state, &project) {
        return reject(commands, reason);
    }
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("found");
    task.queued = None;
    start_run(state, id, commands);
    true
}

/// Moves queued task `id` to place `to` in its project's queue.
fn move_queued(state: &mut AppState, id: u64, to: usize, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    match queue::reorder(&mut state.tasks, id, to) {
        Ok(moved) => {
            for task in state.tasks.iter().filter(|t| moved.contains(&t.id)) {
                commands.push(save(&root, task));
            }
            !moved.is_empty()
        }
        Err(reason) => reject(commands, reason),
    }
}

/// Starts task `id`'s run for its last message (the user's), sending the
/// agent every message it has not seen ([`queue::prompt`]).
fn start_run(state: &mut AppState, id: u64, commands: &mut Vec<Command>) {
    let root = PathBuf::from(&state.repo_root);
    let project = &state
        .tasks
        .iter()
        .find(|t| t.id == id)
        .expect("a known task")
        .project;
    // The session is the active project's; another project's task gets none.
    let context = app_context(state, project);
    let attach = context.session.clone();
    state.contexts.insert(id, context.clone());
    let tracks = state.tracks_changes;
    let now = stamp(state.now);
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("a known task");
    task.updated_at = now.or(task.updated_at);
    let run = task.transcript.len() as u64 - 1;
    let prompt = queue::prompt(task);
    task.run = Some(Run {
        id: run,
        ..Default::default()
    });
    // The report speaks for the run that made it; this one presents its own.
    task.report = None;
    drop_review(task, commands);
    task.status = derive_status(task);
    commands.push(Command::StartRun(Box::new(RunSpec {
        task: id,
        run,
        harness: task.harness,
        scope: task.scope,
        dir: project_dir(&root, &task.project),
        repo_root: root.clone(),
        message: prompt,
        resume: task.session_id.clone(),
        snapshot: tracks && task.baseline.is_none(),
        baseline: task.baseline.clone().filter(|_| tracks),
        attach,
        moment: tracks && task.baseline.is_none() && !state.no_gpu,
        context,
    })));
    commands.push(save(&root, task));
    broadcast_items(commands, task, vec![run as usize]);
}

/// Stops task `id`'s run; rejected when it has none.
fn stop_task(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    let Some(run) = task.run.as_ref().map(|r| r.id) else {
        return reject(commands, format!("task {id} is not running"));
    };
    commands.push(Command::StopRun { task: id, run });
    task.updated_at = stamp(state.now).or(task.updated_at);
    let changed = stop_run(task);
    // A stopped run reports no change of its own: what it left is measured
    // now, and unknown until then.
    if let Some(baseline) = task.baseline.clone() {
        task.change = None;
        commands.push(Command::Diff {
            task: id,
            baseline,
            reply: false,
        });
    }
    commands.push(save(&root, task));
    broadcast_items(commands, task, changed);
    true
}

/// `conn`'s headset said `text`: it steers the task running for the game
/// on that headset, or starts one ([`voice::route`]). Whether a run started.
fn heard(
    state: &mut AppState,
    conn: u64,
    utterance: u64,
    text: String,
    context: Option<String>,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(device) = state.devices.list.iter().find(|d| d.conn == Some(conn)) else {
        return false;
    };
    let u = Utterance {
        device: device.id.clone(),
        utterance,
        spoken: Spoken {
            headset: device.name.clone(),
            context,
            route: None,
        },
        text,
        by: By::Rule,
        title: None,
        project: state.active.clone().unwrap_or_default(),
    };
    let rule = match rule_place(state, &u) {
        Ok(place) => place,
        Err(reason) => {
            voice::status(commands, &u.device, utterance, Stage::Failed(reason));
            return false;
        }
    };
    // With a choice to make, the model router decides, the rule its fallback.
    let model = state.router && state.model_routes.contains(&u.project);
    if let Some(request) = route::request(state, &u.project, &u.text).filter(|_| model) {
        commands.push(Command::Route {
            device: u.device.clone(),
            utterance,
            request: Box::new(request.clone()),
        });
        state.routing.push((u, request));
        return false;
    }
    land(state, u, rule, commands)
}

/// The fixed rule's place for `u` ([`voice::route`]), or why it has none.
fn rule_place(state: &AppState, u: &Utterance) -> Result<Place, String> {
    if state.active.as_ref() != Some(&u.project) {
        return Err("the game on the headset changed".to_string());
    }
    match voice::route(state, &u.device, &u.text) {
        Route::Nowhere(reason) => Err(reason),
        Route::Steer(id) => Ok(Place::Steer(id)),
        Route::New => Ok(Place::New(None)),
    }
}

/// The model router's `choice` for an utterance: its place if it was shown
/// that place and it is still one, else the fixed rule's.
fn routed(
    state: &mut AppState,
    device: &str,
    utterance: u64,
    choice: Result<Choice, String>,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(at) = (state.routing.iter())
        .position(|(u, _)| u.device == device && u.utterance == utterance)
    else {
        return false;
    };
    let (mut u, shown) = state.routing.remove(at);
    let rule = match rule_place(state, &u) {
        Ok(place) => place,
        Err(reason) => {
            voice::status(commands, &u.device, utterance, Stage::Failed(reason));
            return false;
        }
    };
    let now = route::request(state, &u.project, &u.text);
    let still = |c: &Choice| now.as_ref().is_some_and(|r| route::checked(c, r).is_some());
    let chosen = (choice.ok())
        .filter(|c| still(c))
        .and_then(|c| route::checked(&c, &shown));
    let place = match chosen {
        Some(place) => {
            u.by = By::Model;
            place
        }
        None => rule,
    };
    land(state, u, place, commands)
}

/// Puts utterance `u` in `place` and records the decision (D29): a steer
/// or an addition now, a new task once its id is reserved. Whether a run
/// started.
fn land(state: &mut AppState, mut u: Utterance, place: Place, commands: &mut Vec<Command>) -> bool {
    match place {
        Place::Steer(id) => {
            steer(state, id, u.text.clone(), Some(u.spoken.clone()), commands);
            let run = state.tasks.iter().find(|t| t.id == id).and_then(|t| t.run.as_ref());
            let landed = Landed::Steered {
                run: run.expect("steered").id,
            };
            record(state, &u, id, landed, commands);
            voice::follow(state, &u, id, commands);
            true
        }
        Place::Add(id) => {
            let now = stamp(state.now);
            let task = state.tasks.iter_mut().find(|t| t.id == id).expect("offered");
            task.transcript.push(Item::User {
                text: u.text.clone(),
                spoken: Some(u.spoken.clone()),
            });
            task.updated_at = now.or(task.updated_at);
            // Saved and broadcast with its line.
            record(state, &u, id, Landed::Added, commands);
            voice::follow(state, &u, id, commands);
            false
        }
        Place::New(title) => {
            u.title = title;
            let project = state.active.as_deref().unwrap_or_default();
            let pick = state.harness_choice.get(project).copied();
            match pick.or_else(|| state.harnesses.first().copied()) {
                Some(harness) => commands.push(Command::ReserveTask {
                    message: u.text.clone(),
                    harness,
                    voice: Some(Box::new(u)),
                }),
                None => {
                    let reason = "no agent is installed on the computer".to_string();
                    voice::status(commands, &u.device, u.utterance, Stage::Failed(reason));
                }
            }
            false
        }
    }
}

/// Records where `u` landed in task `task` (its last message): the line
/// on the message, and the decision while it can be moved.
fn record(state: &mut AppState, u: &Utterance, task: u64, landed: Landed, commands: &mut Vec<Command>) {
    let line = route::line(state, task, landed, u.by);
    let root = PathBuf::from(&state.repo_root);
    let t = state.tasks.iter_mut().find(|t| t.id == task).expect("landed");
    let item = t.transcript.len() - 1;
    if let Item::User {
        spoken: Some(spoken),
        ..
    } = &mut t.transcript[item]
    {
        spoken.route = Some(line);
    }
    commands.push(save(&root, t));
    broadcast_items(commands, t, vec![item]);
    state.route_seq += 1;
    state.routed.push(Open {
        id: state.route_seq,
        task,
        item,
        landed,
        by: u.by,
        device: u.device.clone(),
        utterance: u.utterance,
    });
}

/// Said to a task whose run was cut by a steer that is moved away before
/// its agent started.
pub const RESUMED: &str = "Carry on with what you were doing.";

/// Whether task `id`'s run has its agent started.
fn started(state: &AppState, id: u64) -> bool {
    let task = state.tasks.iter().find(|t| t.id == id);
    task.and_then(|t| t.run.as_ref()).is_some_and(|r| r.started)
}

/// Said to a steered task when its message is moved elsewhere.
pub const RETRACTED: &str = "That last message was not for this task: the user moved it to \
another. Ignore it and carry on with what you were doing before it.";

/// Moves decision `id`'s message to `to` (one of its moves): undone where
/// it is (a steered run stops and resumes without it; a queued task made
/// for it, or its place in one, goes), then put there as the user's choice.
fn move_utterance(state: &mut AppState, id: u64, to: Destination, commands: &mut Vec<Command>) -> bool {
    let Some(open) = state.routed.iter().find(|o| o.id == id && route::is_open(state, o)).cloned() else {
        return reject(commands, "that message can no longer be moved".to_string());
    };
    if !route::moves(state, &open).iter().any(|m| m.to == to) {
        return reject(commands, "that message cannot go there now".to_string());
    }
    state.routed.retain(|o| o.id != id);
    // Followed again where it lands, not failed where it was.
    (state.voice).retain(|j| !(j.device == open.device && j.utterance == open.utterance));
    let root = PathBuf::from(&state.repo_root);
    let task = state.tasks.iter().find(|t| t.id == open.task).expect("open");
    let Item::User {
        text,
        spoken: Some(spoken),
    } = task.transcript[open.item].clone()
    else {
        return reject(commands, "that message was not said aloud".to_string());
    };
    match open.landed {
        // Cut before its agent started: it never had the message, so the
        // message goes, and what it was given before runs again (or the
        // run it interrupted resumes).
        Landed::Steered { .. } if !started(state, open.task) => {
            let t = state.tasks.iter_mut().find(|t| t.id == open.task).expect("open");
            let run = t.run.take().expect("open").id;
            commands.push(Command::StopRun {
                task: open.task,
                run,
            });
            // A `Stopped` in its place: the next run's id is new, so a late
            // event of the cut one cannot land on it.
            t.transcript.truncate(open.item);
            t.transcript.push(Item::Stopped);
            t.status = derive_status(t);
            commands.push(Command::Broadcast(ServerMsg::Transcript {
                task: t.id,
                items: t.transcript.clone(),
            }));
            say(state, open.task, RESUMED.to_string(), None);
            start_run(state, open.task, commands);
        }
        Landed::Steered { .. } => {
            stop_task(state, open.task, commands);
            let line = route::moved(state, &to);
            let t = state.tasks.iter_mut().find(|t| t.id == open.task).expect("open");
            if let Item::User {
                spoken: Some(s), ..
            } = &mut t.transcript[open.item]
            {
                s.route = Some(line);
            }
            broadcast_items(commands, t, vec![open.item]);
            say(state, open.task, RETRACTED.to_string(), None);
            start_run(state, open.task, commands);
        }
        Landed::Queued => {
            delete_task(state, open.task, commands);
        }
        Landed::Added => {
            let t = state.tasks.iter_mut().find(|t| t.id == open.task).expect("open");
            t.transcript.truncate(open.item);
            commands.push(save(&root, t));
            commands.push(Command::Broadcast(ServerMsg::Transcript {
                task: t.id,
                items: t.transcript.clone(),
            }));
        }
        Landed::Started { .. } => unreachable!("a started task has no moves"),
    }
    let project = (state.tasks.iter().find(|t| t.id == open.task))
        .map_or_else(|| state.active.clone().unwrap_or_default(), |t| t.project.clone());
    let u = Utterance {
        project,
        device: open.device,
        utterance: open.utterance,
        text,
        spoken: Spoken {
            route: None,
            ..spoken
        },
        by: By::You,
        title: None,
    };
    let place = match to {
        Destination::Steer { task } => Place::Steer(task),
        Destination::New => Place::New(None),
        Destination::Queued { task } => Place::Add(task),
    };
    land(state, u, place, commands);
    true
}

/// A new task for utterance `u`, with the id reserved for it, while its
/// headset still runs the game; a steer if a task started meanwhile, as
/// the rule would have chosen then.
fn voice_task(
    state: &mut AppState,
    id: Result<u64, String>,
    harness: Harness,
    u: Utterance,
    commands: &mut Vec<Command>,
) -> bool {
    let started = match (id, voice::route(state, &u.device, &u.text)) {
        (Err(e), _) => Err(format!("cannot start a task: {e}")),
        // Its task goes in its game's project, or nowhere.
        _ if state.active.as_ref() != Some(&u.project) => {
            Err("the project changed before the task started".to_string())
        }
        // Moved by the user: where they put it, wherever the game is now.
        (_, Route::Nowhere(reason)) if u.by != By::You => Err(reason),
        // Chosen as a new task by the rule, which would steer it now.
        (_, Route::Steer(id)) if u.by == By::Rule => {
            steer(state, id, u.text.clone(), Some(u.spoken.clone()), commands);
            let run = state.tasks.iter().find(|t| t.id == id).and_then(|t| t.run.as_ref());
            Ok((id, Landed::Steered { run: run.expect("steered").id }))
        }
        (Ok(id), _) => {
            let spoken = Some(u.spoken.clone());
            // The project's level (D39).
            create_task(state, id, u.text.clone(), harness, None, spoken, commands).map(|()| {
                let t = state.tasks.iter_mut().find(|t| t.id == id).expect("created");
                if let Some(title) = &u.title {
                    t.title = title.clone();
                }
                let landed = match &t.run {
                    Some(run) => Landed::Started { run: run.id },
                    None => Landed::Queued,
                };
                (id, landed)
            })
        }
    };
    match started {
        Ok((id, landed)) => {
            record(state, &u, id, landed, commands);
            voice::follow(state, &u, id, commands);
            true
        }
        Err(reason) => {
            voice::status(commands, &u.device, u.utterance, Stage::Failed(reason));
            false
        }
    }
}

/// Task `id`, when its change can be kept or reverted now.
fn reviewable(state: &mut AppState, id: u64) -> Result<&mut Task, String> {
    if !state.tracks_changes {
        return Err("changes are tracked only in a git repository".to_string());
    }
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("there is no task {id}"))?;
    if task.run.is_some() {
        return Err(format!(
            "task {id} is still running: wait for it to finish or stop it"
        ));
    }
    if task.reverting {
        return Err(format!("task {id}'s change is being reverted"));
    }
    if !has_change(task) {
        return Err(format!("task {id} has no change to review"));
    }
    Ok(task)
}

/// Keeps task `id`'s change: the baseline is dropped and the task is done
/// until a follow-up, whose run takes a new one.
fn keep(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let now = stamp(state.now);
    let task = match reviewable(state, id) {
        Ok(task) => task,
        Err(reason) => return reject(commands, reason),
    };
    task.baseline = None;
    task.change = None;
    end_evidence(task, commands);
    close(&root, task, Item::Kept, now, commands);
    true
}

/// Starts reverting task `id`'s change, as far as the user reviewed it.
/// Refused while any run (an agent may edit anywhere in the repo) or other
/// revert is under way, and for a change too large to list in full.
fn revert(state: &mut AppState, id: u64, tree: &str, commands: &mut Vec<Command>) -> bool {
    let busy = state
        .tasks
        .iter()
        .find(|t| t.id != id && (t.run.is_some() || t.reverting));
    if let Some(busy) = busy {
        let reason = format!(
            "task {} is running or reverting: wait for it before reverting",
            busy.id
        );
        return reject(commands, reason);
    }
    let task = match reviewable(state, id) {
        Ok(task) => task,
        Err(reason) => return reject(commands, reason),
    };
    let change = task.change.as_ref().expect("reviewable");
    if change.tree != tree {
        let reason = format!("task {id}'s change has changed since you saw it: review it again");
        return reject(commands, reason);
    }
    if change.more > 0 {
        let reason =
            format!("task {id} changed more files than the app lists: revert them with git");
        return reject(commands, reason);
    }
    let paths = change.paths();
    task.reverting = true;
    // One preparing would read a tree half put back; a prepared one stays
    // with its report until the revert ends.
    if task.review.as_ref().is_some_and(|r| r.state == ReviewState::Preparing) {
        drop_review(task, commands);
    }
    commands.push(Command::Revert {
        task: id,
        baseline: task.baseline.clone().expect("reviewable"),
        paths,
    });
    true
}

/// Ends task `id`'s review with `item` (`Kept` or `Reverted`) at `now`.
fn close(root: &Path, task: &mut Task, item: Item, now: Option<u64>, commands: &mut Vec<Command>) {
    task.updated_at = now.or(task.updated_at);
    task.transcript.push(item);
    task.status = derive_status(task);
    commands.push(save(root, task));
    broadcast_items(commands, task, vec![task.transcript.len() - 1]);
}

/// Answers `GetDiff`: an empty diff when nothing is tracked for the task.
fn get_diff(state: &AppState, id: u64, commands: &mut Vec<Command>) {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return;
    };
    match (&task.baseline, state.tracks_changes) {
        (Some(baseline), true) => commands.push(Command::Diff {
            task: id,
            baseline: baseline.clone(),
            reply: true,
        }),
        _ => commands.push(Command::Reply(ServerMsg::Diff {
            task: id,
            tree: None,
            files: vec![],
            more: 0,
            error: None,
        })),
    }
}

/// Records a computed change; one measured from an older baseline is dropped.
fn on_change(
    state: &mut AppState,
    id: u64,
    baseline: &str,
    change: Change,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let Some(task) = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.baseline.as_deref() == Some(baseline))
    else {
        return false;
    };
    if task.change.as_ref() == Some(&change) {
        return false;
    }
    // Files changed after the run that reported on them: its report no
    // longer speaks for the change, nor does a review of it.
    if task.run.is_none() && task.change.as_ref().is_some_and(|c| c.tree != change.tree) {
        task.report = None;
        // One still preparing says why it stopped.
        stop_review(task, commands);
        match &mut task.review {
            Some(r) if r.state == ReviewState::Preparing => {
                r.state = ReviewState::Failed {
                    reason: "the files changed while the review ran".to_string(),
                };
                r.staged = None;
            }
            _ => task.review = None,
        }
    }
    task.change = Some(change);
    task.status = derive_status(task);
    commands.push(save(&root, task));
    true
}

/// Brings task `id`'s path evidence up to date ([`evidence::next`]),
/// asking the shell for the captures when they are due.
fn refresh_evidence(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) else {
        return false;
    };
    let evidence::Next::Set { evidence, take } = evidence::next(task) else {
        return false;
    };
    if let (true, Some(baseline), Some(e)) = (take, &task.baseline, &evidence) {
        commands.push(Command::TakeEvidence {
            task: id,
            dir: project_dir(&root, &task.project),
            project: task.project.clone(),
            baseline: baseline.clone(),
            tree: e.tree.clone(),
        });
    }
    drop_files(std::mem::replace(&mut task.evidence, evidence), commands);
    commands.push(save(&root, task));
    true
}

/// Records the shell's captures, if they are still the ones asked for;
/// else their files go.
fn on_evidence(
    state: &mut AppState,
    id: u64,
    tree: String,
    taken: EvidenceState,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let evidence = Evidence { tree, state: taken };
    let asked = |e: &Evidence| e.tree == evidence.tree && e.state == EvidenceState::Taking;
    let Some(task) = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.evidence.as_ref().is_some_and(asked))
    else {
        drop_files(Some(evidence), commands);
        return false;
    };
    task.evidence = Some(evidence);
    commands.push(save(&root, task));
    true
}

/// The review is over: the moment, the paths and the report go with the
/// baseline.
fn end_evidence(task: &mut Task, commands: &mut Vec<Command>) {
    task.moment = None;
    task.report = None;
    drop_review(task, commands);
    drop_files(task.evidence.take(), commands);
}

fn unavailable(harness: Harness) -> String {
    format!("{} is not available on this machine", harness.name())
}

/// Records `setting` as the active project's review setting.
fn set_review(state: &mut AppState, setting: ReviewSetting, commands: &mut Vec<Command>) -> bool {
    let Some(path) = state.active.clone() else {
        return false;
    };
    let setting = match setting.checked() {
        Ok(setting) => setting,
        Err(reason) => return reject(commands, reason),
    };
    if let Err(reason) = review_harness(setting.harness) {
        return reject(commands, reason.to_string());
    }
    if let Some(h) = setting.harness.filter(|h| !state.harnesses.contains(h)) {
        return reject(commands, unavailable(h));
    }
    if state.review_settings.get(&path) == Some(&setting) {
        return false;
    }
    commands.push(Command::SaveReview {
        dir: project_dir(Path::new(&state.repo_root), &path),
        setting: setting.clone(),
    });
    state.review_settings.insert(path, setting);
    true
}

/// Starts task `id`'s review of run `run`'s change (D31), once the run ended
/// ok with a change for the user and the project's setting is on. A harness
/// that is not installed fails it at once, saying so.
fn start_review(state: &mut AppState, id: u64, run: u64, commands: &mut Vec<Command>) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        return false;
    };
    let setting = (state.review_settings.get(&task.project).cloned()).unwrap_or_default();
    let ready = task.status == TaskStatus::ReadyForReview && state.tracks_changes;
    let (Some(baseline), Some(change)) = (&task.baseline, &task.change) else {
        return false;
    };
    if !state.can_review || !setting.on || !ready {
        return false;
    }
    // Only Claude's can be held read-only (`review_harness`), whatever the task's.
    let harness = Harness::Claude;
    let model = setting.model.or(default_model(harness).map(String::from));
    let spec = ReviewSpec {
        task: id,
        run,
        harness,
        model: model.clone(),
        instructions: setting.instructions,
        dir: project_dir(&root, &task.project),
        repo_root: root.clone(),
        baseline: baseline.clone(),
        tree: change.tree.clone(),
        brief: brief(task),
    };
    let review_state = match (!state.harnesses.contains(&harness)).then(|| unavailable(harness)) {
        None => {
            commands.push(Command::StartReview(Box::new(spec)));
            ReviewState::Preparing
        }
        Some(reason) => ReviewState::Failed { reason },
    };
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("found");
    task.review = Some(Review {
        run,
        harness,
        model,
        state: review_state,
        usage: None,
        detail: None,
        staged: None,
    });
    commands.push(save(&root, task));
    true
}

/// Applies a review's event; true when the task's summary changed.
fn on_review(
    state: &mut AppState,
    id: u64,
    run: u64,
    event: ReviewEvent,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    // A cancelled or superseded review's late events find none preparing.
    let Some(task) = state.tasks.iter_mut().find(|t| {
        t.id == id && t.review.as_ref().is_some_and(|r| r.preparing(run))
    }) else {
        return false;
    };
    let review = task.review.as_mut().expect("found");
    match event {
        ReviewEvent::Agent(AgentEvent::Started { model, .. }) => {
            review.model = model.or(review.model.take());
            true
        }
        // Kept with the task, sent with the next change.
        ReviewEvent::Agent(AgentEvent::Usage { input, output }) => {
            review.usage = Some(Usage { input, output });
            false
        }
        ReviewEvent::Agent(AgentEvent::UsageDetail(detail)) => {
            review.detail = Some(detail);
            false
        }
        ReviewEvent::Agent(_) => false,
        // The shell measured the files after it: a change since its start
        // has already failed it (`on_change`), so its report stands.
        ReviewEvent::Ended { error } => {
            // A failure (a cut, the agent's, an unmeasured tree) voids its report.
            let state = match error {
                Some(reason) => ReviewState::Failed { reason },
                None if publish_review(task, commands) => ReviewState::Prepared,
                None => ReviewState::Failed {
                    reason: "the review ended without a report".to_string(),
                },
            };
            let review = task.review.as_mut().expect("found");
            review.state = state;
            review.staged = None;
            commands.push(save(&root, task));
            true
        }
    }
}

/// Cancels `task`'s review while it prepares; the shell drops its results.
fn stop_review(task: &Task, commands: &mut Vec<Command>) {
    if let Some(Review {
        run,
        state: ReviewState::Preparing,
        ..
    }) = &task.review
    {
        commands.push(Command::StopReview {
            task: task.id,
            run: *run,
        });
    }
}

/// `task`'s review is over, its change decided or replaced.
fn drop_review(task: &mut Task, commands: &mut Vec<Command>) {
    stop_review(task, commands);
    task.review = None;
}

fn drop_files(evidence: Option<Evidence>, commands: &mut Vec<Command>) {
    let files = evidence.map(|e| e.files()).unwrap_or_default();
    if !files.is_empty() {
        commands.push(Command::DropEvidence { files });
    }
}

/// A finished revert closes the task; a failed one says why and measures
/// what is left.
fn on_reverted(
    state: &mut AppState,
    id: u64,
    baseline: String,
    result: Result<(), String>,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let now = stamp(state.now);
    let Some(task) = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.reverting && t.baseline.as_ref() == Some(&baseline))
    else {
        return false;
    };
    task.reverting = false;
    match result {
        Ok(()) => {
            task.baseline = None;
            task.change = None;
            end_evidence(task, commands);
            close(&root, task, Item::Reverted, now, commands);
        }
        Err(reason) => {
            commands.push(Command::Broadcast(ServerMsg::Rejected {
                reason: format!("cannot revert task {id}'s change: {reason}"),
            }));
            commands.push(Command::Diff {
                task: id,
                baseline,
                reply: false,
            });
        }
    }
    true
}

/// Applies a run's event; true when the task's summary changed.
fn on_run(
    state: &mut AppState,
    id: u64,
    run: u64,
    event: RunEvent,
    commands: &mut Vec<Command>,
) -> bool {
    let root = PathBuf::from(&state.repo_root);
    let now = stamp(state.now);
    // A stopped run's late events find no run of that id.
    let Some(task) = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.run.as_ref().is_some_and(|r| r.id == run))
    else {
        return false;
    };
    match event {
        RunEvent::Baseline { tree } => {
            task.baseline = Some(tree);
            task.change = None;
            commands.push(save(&root, task));
            true
        }
        RunEvent::Moment(moment) => {
            task.moment = Some(moment);
            commands.push(save(&root, task));
            true
        }
        RunEvent::Agent(event) => {
            let before = (task.session_id.clone(), task.model.clone());
            let changed = apply(task, event);
            if !changed.is_empty() {
                task.updated_at = now.or(task.updated_at);
            }
            let summary = (task.session_id.clone(), task.model.clone()) != before;
            // Saved as each item completes (not per streamed piece), so a
            // crash keeps the run so far and the session to resume.
            let settled = changed
                .iter()
                .any(|&i| !matches!(task.transcript[i], Item::Agent { partial: true, .. }));
            if summary || settled {
                commands.push(save(&root, task));
            }
            broadcast_items(commands, task, changed);
            summary
        }
        RunEvent::Ended { error } => {
            task.updated_at = now.or(task.updated_at);
            let changed = end_run(task, error);
            commands.push(save(&root, task));
            broadcast_items(commands, task, changed);
            true
        }
    }
}

/// Keeps the context window a run reports (Claude's, as it ends) for its
/// harness and model's later runs, and saves it when new.
fn remember_window(
    state: &mut AppState,
    id: u64,
    run: u64,
    event: &RunEvent,
    commands: &mut Vec<Command>,
) {
    let RunEvent::Agent(AgentEvent::UsageDetail(UsageDetail {
        context_window: Some(window),
        ..
    })) = event
    else {
        return;
    };
    let task = (state.tasks.iter()).find(|t| t.id == id && t.run.as_ref().is_some_and(|r| r.id == run));
    let Some(model) = task.and_then(|t| Some((t.harness, t.model.as_deref()?))) else {
        return;
    };
    let key = crate::activity::window_key(model.0, model.1);
    if state.context_windows.insert(key, *window) != Some(*window) {
        let windows = state.context_windows.clone();
        commands.push(Command::SaveContextWindows { windows });
    }
}

pub(crate) fn save(root: &Path, task: &Task) -> Command {
    Command::SaveTask {
        dir: project_dir(root, &task.project),
        task: Box::new(task.clone()),
    }
}

pub(crate) fn broadcast_items(commands: &mut Vec<Command>, task: &Task, changed: Vec<usize>) {
    for index in changed {
        commands.push(Command::Broadcast(ServerMsg::Item {
            task: task.id,
            index,
            item: task.transcript[index].clone(),
        }));
    }
}

fn on_session(
    state: &mut AppState,
    id: u64,
    event: SessionEvent,
    commands: &mut Vec<Command>,
) -> bool {
    let fast = state.transport_watched;
    let Some(session) = state.session.as_mut().filter(|s| s.id == id) else {
        return false;
    };
    match (event, &mut session.status) {
        (SessionEvent::Spawned { pid }, SessionStatus::Starting) => {
            session.pid = Some(pid);
        }
        (SessionEvent::Listening { debug_url }, SessionStatus::Starting) => {
            commands.push(Command::PollState {
                id,
                debug_url: debug_url.clone(),
                fast,
                after: 0,
            });
            session.status = SessionStatus::Running {
                debug_url,
                surface: None,
                pointer: Pointer::Visible,
                timeline: Box::default(),
            };
        }
        (
            SessionEvent::Polled { after, result },
            SessionStatus::Running {
                debug_url,
                timeline,
                ..
            },
        ) => {
            // Keep polling through a failed request; the child's exit ends it.
            commands.push(Command::PollState {
                id,
                debug_url: debug_url.clone(),
                fast,
                after: timeline.seeks,
            });
            let Ok(polled) = result else {
                return false;
            };
            timeline.polled(after, polled.frame, polled.clock);
            read_marks(state, commands);
            // The clock goes as `ServerMsg::Transport`; only a new surface or
            // pointer is state.
            return match &mut state.session.as_mut().map(|s| &mut s.status) {
                Some(SessionStatus::Running {
                    surface, pointer, ..
                }) if (*surface, *pointer) != (polled.surface, polled.pointer) => {
                    (*surface, *pointer) = (polled.surface, polled.pointer);
                    true
                }
                _ => false,
            };
        }
        (SessionEvent::InputLog(result), SessionStatus::Running { timeline, .. }) => {
            timeline.reading = false;
            if let Ok(page) = result {
                timeline.read(page);
            }
            read_marks(state, commands);
            return false;
        }
        (SessionEvent::Exited { reason }, SessionStatus::Running { .. }) => {
            session.status = SessionStatus::Exited { reason };
            session.pid = None;
        }
        (
            SessionEvent::Exited { reason } | SessionEvent::StartFailed { reason },
            SessionStatus::Starting,
        ) => {
            session.status = SessionStatus::Failed { reason };
            session.pid = None;
        }
        // A fix to a game that failed to load starts it again (D37).
        (SessionEvent::SourceChanged, SessionStatus::Failed { .. }) => {
            return start(state, commands);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::Entries;
    use crate::protocol::GameInput;
    use crate::session::{Polled, PolledClock};
    use crate::transport::{LogPage, Mark, TransportCall};

    fn project(path: &str) -> Project {
        Project {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            entries: Entries::Single {
                file: "game.fun".to_string(),
            },
        }
    }

    fn state() -> AppState {
        AppState {
            repo_root: "/repo".to_string(),
            projects: vec![project("examples/a"), project("examples/b")],
            active: Some("examples/a".to_string()),
            files: vec!["functor.json".to_string(), "game.fun".to_string()],
            git: None,
            tracks_changes: false,
            session: None,
            no_gpu: false,
            auto_start: true,
            tasks: vec![],
            harnesses: vec![Harness::Claude, Harness::Codex],
            harness_choice: BTreeMap::new(),
            layouts: BTreeMap::new(),
            ..AppState::none()
        }
    }

    fn with_session(id: u64, pid: Option<u32>, status: SessionStatus) -> AppState {
        AppState {
            session: Some(Session { id, pid, status }),
            ..state()
        }
    }

    /// Running, polled once at `frame` (paused, nothing recorded) unless `None`.
    fn running(frame: Option<u64>) -> SessionStatus {
        let mut timeline = Timeline::default();
        if let Some(frame) = frame {
            timeline.polled(0, frame, Some(PolledClock::default()));
        }
        SessionStatus::Running {
            debug_url: URL.to_string(),
            surface: frame.map(|_| SURFACE),
            pointer: Pointer::Visible,
            timeline: Box::new(timeline),
        }
    }

    const SURFACE: [u32; 2] = [800, 600];

    fn polled(frame: u64) -> SessionEvent {
        polled_clock(frame, PolledClock::default())
    }

    fn polled_clock(frame: u64, clock: PolledClock) -> SessionEvent {
        SessionEvent::Polled {
            after: 0,
            result: Ok(Polled {
                frame,
                surface: Some(SURFACE),
                pointer: Pointer::Visible,
                clock: Some(clock),
            }),
        }
    }

    /// The transport `frame` shows after a poll with `clock`.
    fn shown(frame: u64, clock: PolledClock) -> Transport {
        Transport {
            clock: true,
            paused: clock.paused,
            frame,
            recorded: clock.recorded,
            generation: clock.generation,
            seek: 0,
            extrapolate: false,
            revision: clock.program_revision,
        }
    }

    fn transport(transport: Option<Transport>) -> Command {
        Command::Broadcast(ServerMsg::Transport { transport })
    }

    fn marks(marks: Vec<Mark>) -> Command {
        Command::Broadcast(ServerMsg::Marks { marks })
    }

    fn start_cmd(id: u64, project: &str, display: Display) -> Command {
        Command::StartSession {
            id,
            dir: PathBuf::from(format!("/repo/{project}")),
            display,
        }
    }

    const URL: &str = "http://127.0.0.1:52011";

    fn client(msg: ClientMsg) -> Event {
        Event::Client(msg)
    }

    fn session(id: u64, event: SessionEvent) -> Event {
        Event::Session { id, event }
    }

    fn broadcast(state: &AppState) -> Command {
        Command::Broadcast(ServerMsg::State {
            state: Box::new(state.clone()),
        })
    }

    fn poll(id: u64) -> Command {
        Command::PollState {
            id,
            debug_url: URL.to_string(),
            fast: false,
            after: 0,
        }
    }

    fn switch(path: &str) -> Event {
        Event::Client(ClientMsg::SetActiveProject {
            path: path.to_string(),
        })
    }

    fn list_b() -> Command {
        Command::ListFiles {
            path: "examples/b".to_string(),
            dir: PathBuf::from("/repo/examples/b"),
        }
    }

    /// `path` as this OS spells it (`\repo\examples\b` on Windows).
    fn native(path: &str) -> String {
        Path::new(path)
            .components()
            .collect::<PathBuf>()
            .to_string_lossy()
            .into_owned()
    }

    fn recents_b() -> Command {
        Command::SaveRecents {
            recents: vec![Recent {
                name: "b".into(),
                path: native("/repo/examples/b"),
                missing: false,
            }],
        }
    }

    fn files(path: &str, files: &[&str]) -> Event {
        Event::Files {
            path: path.to_string(),
            dir: PathBuf::from(format!("/repo/{path}")),
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn get_state_replies_with_the_state_unchanged() {
        let (next, commands) = update(state(), Event::Client(ClientMsg::GetState));
        assert_eq!(next, state());
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(state())
            })]
        );
    }

    #[test]
    fn switching_to_a_known_project_broadcasts_it_and_lists_its_files() {
        let (next, commands) = update(state(), switch("examples/b"));
        assert_eq!(next.active.as_deref(), Some("examples/b"));
        assert!(next.files.is_empty());
        // It is the most recent project now.
        assert_eq!(next.recents[0].path, native("/repo/examples/b"));
        assert_eq!(commands, vec![list_b(), recents_b(), broadcast(&next)]);
    }

    #[test]
    fn the_active_projects_files_are_stored_and_broadcast() {
        let (switched, _) = update(state(), switch("examples/b"));
        let (next, commands) = update(switched, files("examples/b", &["b.fun"]));
        assert_eq!(next.files, ["b.fun"]);
        assert_eq!(commands, vec![broadcast(&next)]);
        // The same listing again changes nothing.
        let (again, commands) = update(next.clone(), files("examples/b", &["b.fun"]));
        assert_eq!((again, commands), (next, vec![]));
    }

    #[test]
    fn files_for_another_project_are_dropped() {
        let (next, commands) = update(state(), files("examples/b", &["b.fun"]));
        assert_eq!((next, commands), (state(), vec![]));
    }

    #[test]
    fn switching_to_an_unknown_or_current_project_only_replies() {
        for path in ["examples/missing", "examples/a"] {
            let (next, commands) = update(state(), switch(path));
            assert_eq!(next, state());
            assert_eq!(
                commands,
                vec![Command::Reply(ServerMsg::State {
                    state: Box::new(state())
                })]
            );
        }
    }

    #[test]
    fn start_session_spawns_for_the_active_project() {
        let (next, commands) = update(state(), client(ClientMsg::StartSession));
        assert_eq!(next, with_session(1, None, SessionStatus::Starting));
        assert_eq!(
            commands,
            vec![
                start_cmd(1, "examples/a", Display::Hidden),
                broadcast(&next)
            ]
        );
        // A later start takes the next id.
        let stopped = with_session(4, None, SessionStatus::Stopped);
        let (next, _) = update(stopped, client(ClientMsg::StartSession));
        assert_eq!(next.session.unwrap().id, 5);
    }

    #[test]
    fn a_source_change_restarts_only_a_game_that_failed_to_load() {
        let reason = || "error: game.fun:1:1".to_string();
        let failed = with_session(3, None, SessionStatus::Failed { reason: reason() });
        let (next, commands) = update(failed, session(3, SessionEvent::SourceChanged));
        assert_eq!(next, with_session(4, None, SessionStatus::Starting));
        assert_eq!(
            commands,
            vec![start_cmd(4, "examples/a", Display::Hidden), broadcast(&next)]
        );
        // Ended after running, stopped, or a stale session: no restart.
        for before in [
            with_session(3, None, SessionStatus::Exited { reason: reason() }),
            with_session(3, None, SessionStatus::Stopped),
            with_session(4, None, SessionStatus::Failed { reason: reason() }),
        ] {
            let (next, commands) = update(before.clone(), session(3, SessionEvent::SourceChanged));
            assert_eq!(next, before);
            assert!(!commands.iter().any(|c| matches!(c, Command::StartSession { .. })));
        }
    }

    fn too_old() -> crate::cli::CliProblem {
        crate::cli::CliProblem {
            path: "~/.functor/bin/functor".to_string(),
            version: Some("0.1.2".to_string()),
            cause: crate::cli::CliCause::TooOld,
            bundled: false,
        }
    }

    #[test]
    fn a_cli_problem_is_kept_until_a_check_finds_none() {
        let checked = |state, problem| update(state, Event::CliChecked { problem });
        let (bad, commands) = checked(state(), Some(too_old()));
        assert_eq!(bad.cli_problem, Some(too_old().message()));
        assert_eq!(commands, vec![broadcast(&bad)]);
        // Try again: one check at a time.
        let (checking, commands) = update(bad.clone(), client(ClientMsg::CheckCli));
        assert!(checking.cli_checking);
        assert_eq!(commands, vec![Command::CheckCli, broadcast(&checking)]);
        let (again, commands) = update(checking.clone(), client(ClientMsg::CheckCli));
        assert_eq!(again, checking);
        assert!(!commands.contains(&Command::CheckCli), "{commands:?}");
        // Still too old: the same message, no longer checking.
        let (still, _) = checked(checking.clone(), Some(too_old()));
        assert_eq!(still, bad);
        // Fixed: the message goes, and nothing failed to start meanwhile.
        let (fixed, commands) = checked(checking, None);
        assert_eq!(fixed.cli_problem, None);
        assert!(!fixed.cli_checking);
        assert_eq!(commands, vec![broadcast(&fixed)]);
    }

    #[test]
    fn a_fixed_cli_starts_what_it_refused() {
        let refused = AppState {
            cli_problem: Some(too_old().message()),
            templates_error: Some(too_old().message()),
            ..with_session(2, None, SessionStatus::Failed { reason: too_old().message() })
        };
        let (next, commands) = update(refused, Event::CliChecked { problem: None });
        assert_eq!(next.session.as_ref().unwrap().status, SessionStatus::Starting);
        assert_eq!(
            commands,
            vec![
                start_cmd(3, "examples/a", Display::Hidden),
                Command::ListTemplates,
                broadcast(&next)
            ]
        );
        // A stopped game stays stopped.
        let stopped = AppState {
            cli_problem: Some(too_old().message()),
            ..with_session(2, None, SessionStatus::Stopped)
        };
        let (next, commands) = update(stopped, Event::CliChecked { problem: None });
        assert_eq!(next.session.unwrap().status, SessionStatus::Stopped);
        assert!(!commands.iter().any(|c| matches!(c, Command::StartSession { .. })));
    }

    #[test]
    fn start_session_while_live_or_without_a_project_only_replies() {
        for before in [
            with_session(1, None, SessionStatus::Starting),
            with_session(1, Some(9), running(Some(3))),
            AppState {
                projects: vec![],
                active: None,
                ..state()
            },
        ] {
            let (next, commands) = update(before.clone(), client(ClientMsg::StartSession));
            assert_eq!(next, before);
            assert_eq!(
                commands,
                vec![Command::Reply(ServerMsg::State {
                    state: Box::new(before)
                })]
            );
        }
    }

    #[test]
    fn spawned_records_the_pid() {
        let before = with_session(1, None, SessionStatus::Starting);
        let (next, commands) = update(before, session(1, SessionEvent::Spawned { pid: 9 }));
        assert_eq!(next, with_session(1, Some(9), SessionStatus::Starting));
        assert_eq!(commands, vec![broadcast(&next)]);
    }

    #[test]
    fn listening_runs_and_starts_polling() {
        let before = with_session(1, Some(9), SessionStatus::Starting);
        let listening = SessionEvent::Listening {
            debug_url: URL.to_string(),
        };
        let (next, commands) = update(before, session(1, listening));
        assert_eq!(next, with_session(1, Some(9), running(None)));
        assert_eq!(commands, vec![poll(1), broadcast(&next)]);
    }

    #[test]
    fn a_poll_records_the_frame_and_polls_again() {
        let before = with_session(1, Some(9), running(None));
        let (next, commands) = update(before, session(1, polled(12)));
        assert_eq!(next, with_session(1, Some(9), running(Some(12))));
        // A first surface is state; the clock goes apart from it.
        let clock = Some(shown(12, PolledClock::default()));
        assert_eq!(commands, vec![poll(1), broadcast(&next), transport(clock)]);
        // The same poll, or a failed request, changes nothing but still polls.
        for event in [
            polled(12),
            SessionEvent::Polled {
                after: 0,
                result: Err("connection refused".to_string()),
            },
        ] {
            let (again, commands) = update(next.clone(), session(1, event));
            assert_eq!(again, next);
            assert_eq!(commands, vec![poll(1)]);
        }
    }

    #[test]
    fn exit_while_running_is_exited() {
        let before = with_session(1, Some(9), running(Some(12)));
        let exited = SessionEvent::Exited {
            reason: "signal: 9".to_string(),
        };
        let (next, commands) = update(before, session(1, exited));
        let status = SessionStatus::Exited {
            reason: "signal: 9".to_string(),
        };
        assert_eq!(next, with_session(1, None, status));
        // The transport goes with the session.
        assert_eq!(commands, vec![broadcast(&next), transport(None)]);
    }

    #[test]
    fn exit_or_spawn_failure_while_starting_is_failed() {
        let reason = || "exit status: 1: error: game.fun:3: expected `=`".to_string();
        for event in [
            SessionEvent::Exited { reason: reason() },
            SessionEvent::StartFailed { reason: reason() },
        ] {
            let before = with_session(1, Some(9), SessionStatus::Starting);
            let (next, commands) = update(before, session(1, event));
            let failed = SessionStatus::Failed { reason: reason() };
            assert_eq!(next, with_session(1, None, failed));
            assert_eq!(commands, vec![broadcast(&next)]);
        }
    }

    #[test]
    fn events_for_another_session_are_dropped() {
        let before = with_session(2, Some(9), running(Some(12)));
        for event in [
            SessionEvent::Exited {
                reason: "signal: 9".to_string(),
            },
            polled(40),
        ] {
            let (next, commands) = update(before.clone(), session(1, event));
            assert_eq!(next, before);
            assert_eq!(commands, vec![]);
        }
        // Nor does a stopped session's late exit or poll do anything.
        let stopped = with_session(2, None, SessionStatus::Stopped);
        let (next, commands) = update(stopped.clone(), session(2, polled(40)));
        assert_eq!((next, commands), (stopped, vec![]));
    }

    #[test]
    fn stop_kills_a_live_session() {
        for (status, hidden) in [
            (SessionStatus::Starting, vec![]),
            (running(Some(12)), vec![transport(None)]),
        ] {
            let (next, commands) = update(
                with_session(1, Some(9), status),
                client(ClientMsg::StopSession),
            );
            assert_eq!(next, with_session(1, None, SessionStatus::Stopped));
            let expected = [
                vec![Command::StopSession { id: 1 }, broadcast(&next)],
                hidden,
            ];
            assert_eq!(commands, expected.concat());
        }
        // An exited session just clears; a stopped one only replies.
        let exited = SessionStatus::Exited {
            reason: "x".to_string(),
        };
        let (next, commands) = update(
            with_session(1, None, exited),
            client(ClientMsg::StopSession),
        );
        assert_eq!(next, with_session(1, None, SessionStatus::Stopped));
        assert_eq!(commands, vec![broadcast(&next)]);
        let (next, commands) = update(next.clone(), client(ClientMsg::StopSession));
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(next)
            })]
        );
    }

    #[test]
    fn switching_projects_restarts_a_live_session() {
        let mut before = with_session(1, Some(9), running(Some(12)));
        before.start_on_open = true;
        let (next, commands) = update(before, switch("examples/b"));
        assert_eq!(next.active.as_deref(), Some("examples/b"));
        assert_eq!(
            next.session,
            Some(Session {
                id: 2,
                pid: None,
                status: SessionStatus::Starting
            })
        );
        assert_eq!(
            commands,
            vec![
                list_b(),
                recents_b(),
                Command::StopSession { id: 1 },
                start_cmd(2, "examples/b", Display::Hidden),
                broadcast(&next),
                transport(None)
            ]
        );
    }

    #[test]
    fn switching_projects_without_a_live_session_starts_none() {
        let failed = SessionStatus::Failed {
            reason: "x".to_string(),
        };
        for before in [state(), with_session(1, None, failed)] {
            let had_session = before.session.is_some();
            let (next, commands) = update(before, switch("examples/b"));
            assert_eq!(next.active.as_deref(), Some("examples/b"));
            // The old project's outcome is cleared, not carried over.
            let expected = had_session.then_some(Session {
                id: 1,
                pid: None,
                status: SessionStatus::Stopped,
            });
            assert_eq!(next.session, expected);
            assert_eq!(commands, vec![list_b(), recents_b(), broadcast(&next)]);
        }
    }

    #[test]
    fn a_new_surface_is_recorded() {
        let before = with_session(1, Some(9), running(Some(12)));
        let resized = SessionEvent::Polled {
            after: 0,
            result: Ok(Polled {
                frame: 12,
                surface: Some([1024, 768]),
                pointer: Pointer::Captured,
                clock: Some(PolledClock::default()),
            }),
        };
        let (next, commands) = update(before, session(1, resized));
        let Some(SessionStatus::Running {
            surface, pointer, ..
        }) = next.session.as_ref().map(|s| &s.status)
        else {
            panic!("{next:?}");
        };
        assert_eq!(*surface, Some([1024, 768]));
        assert_eq!(*pointer, Pointer::Captured);
        assert_eq!(commands, vec![poll(1), broadcast(&next)]);
    }

    /// Session 1, running: polled at frame `newest` with frames 1–`newest`
    /// recorded, playing, at program revision `revision`.
    fn recording(newest: u64, revision: u64) -> AppState {
        let before = with_session(1, Some(9), running(None));
        let (s, _) = update(
            before,
            session(1, polled_clock(newest, recorded(newest, revision))),
        );
        s
    }

    fn recorded(newest: u64, revision: u64) -> PolledClock {
        PolledClock {
            recorded: Some([1, newest]),
            program_revision: revision,
            ..PolledClock::default()
        }
    }

    fn bar(call: TransportCall) -> Event {
        client(ClientMsg::Transport { call })
    }

    fn posted(call: TransportCall) -> Command {
        Command::Call {
            id: 1,
            debug_url: URL.to_string(),
            call: SessionCall::Transport(call),
        }
    }

    fn read(from: u64) -> Command {
        Command::ReadInputLog {
            id: 1,
            debug_url: URL.to_string(),
            from,
        }
    }

    fn watch(on: bool) -> Event {
        client(ClientMsg::WatchTransport { on })
    }

    #[test]
    fn transport_calls_are_posted_and_a_seek_shows_at_once() {
        let s = recording(100, 0);
        let playing = shown(100, recorded(100, 0));
        // A pause is posted; it shows when a poll reports it.
        let (s, commands) = update(s, bar(TransportCall::Pause));
        assert_eq!(commands, vec![posted(TransportCall::Pause)]);
        // A seek shows at once, clamped to the window, with its page's id.
        let (s, commands) = update(s, bar(TransportCall::Seek { frame: 900, id: 3 }));
        let sought = TransportCall::Seek { frame: 100, id: 3 };
        let at = |frame| Transport {
            frame,
            seek: 3,
            ..playing
        };
        assert_eq!(commands, vec![posted(sought), transport(Some(at(100)))]);
        let (s, commands) = update(s, bar(TransportCall::Seek { frame: 40, id: 4 }));
        assert_eq!(
            commands[0],
            posted(TransportCall::Seek { frame: 40, id: 4 })
        );
        // A poll sent before those seeks does not undo them; its clock is news.
        let paused = PolledClock {
            paused: true,
            ..recorded(103, 0)
        };
        let (s, commands) = update(s, session(1, polled_clock(103, paused)));
        let ahead = Transport {
            frame: 40,
            seek: 4,
            paused: true,
            recorded: Some([1, 103]),
            ..playing
        };
        assert_eq!(
            commands,
            vec![
                Command::PollState {
                    id: 1,
                    debug_url: URL.to_string(),
                    fast: false,
                    after: 2,
                },
                transport(Some(ahead))
            ]
        );
        // One sent after them is the runtime's word.
        let answer = SessionEvent::Polled {
            after: 2,
            result: Ok(Polled {
                frame: 41,
                surface: Some(SURFACE),
                pointer: Pointer::Visible,
                clock: Some(paused),
            }),
        };
        let (_, commands) = update(s, session(1, answer));
        let word = Transport { frame: 41, ..ahead };
        assert_eq!(commands[1], transport(Some(word)));
    }

    #[test]
    fn transport_calls_need_a_running_session_with_a_clock() {
        let old_runtime = {
            let polled = SessionEvent::Polled {
                after: 0,
                result: Ok(Polled {
                    frame: 7,
                    surface: Some(SURFACE),
                    pointer: Pointer::Visible,
                    clock: None,
                }),
            };
            update(with_session(1, Some(9), running(None)), session(1, polled)).0
        };
        for before in [
            state(),
            with_session(1, Some(9), SessionStatus::Starting),
            with_session(1, Some(9), running(None)),
            old_runtime,
        ] {
            let (next, commands) = update(before.clone(), bar(TransportCall::Step));
            assert_eq!((next, commands), (before, vec![]));
        }
    }

    #[test]
    fn a_watched_transport_polls_fast_and_reads_marks() {
        let s = recording(10, 0);
        let (s, commands) = update(s, watch(true));
        assert_eq!(commands, vec![read(0)]);
        // One read at a time.
        let (s, commands) = update(s, watch(true));
        assert_eq!(commands, vec![]);
        let (s, commands) = update(s, session(1, polled_clock(10, recorded(10, 0))));
        assert_eq!(
            commands,
            vec![Command::PollState {
                id: 1,
                debug_url: URL.to_string(),
                fast: true,
                after: 0,
            }]
        );
        // A read's marks go to every page, and the next read follows.
        let page = LogPage {
            generation: 0,
            oldest: 1,
            newest: 10,
            steps: vec![(4, vec![("key-down".into(), "W down".into())])],
        };
        let (s, commands) = update(s, session(1, SessionEvent::InputLog(Ok(page))));
        let w = Mark {
            id: 1,
            frame: 4,
            kind: "key-down".into(),
            label: "W down".into(),
        };
        assert_eq!(commands, vec![read(11), marks(vec![w.clone()])]);
        // A new page gets the transport and marks apart from the state.
        let (s, commands) = update(s, client(ClientMsg::GetState));
        assert_eq!(
            commands[1..],
            [
                Command::Reply(ServerMsg::Transport {
                    transport: Some(shown(10, recorded(10, 0)))
                }),
                Command::Reply(ServerMsg::Marks { marks: vec![w] })
            ]
        );
        // Unwatched, the read on its way is the last, and the switch is posted
        // again: a peek no page can end is over.
        let (s, commands) = update(s, watch(false));
        assert_eq!(
            commands,
            vec![posted(TransportCall::Extrapolate { on: false })]
        );
        let failed = SessionEvent::InputLog(Err("connection refused".into()));
        let (_, commands) = update(s, session(1, failed));
        assert_eq!(commands, vec![]);
    }

    #[test]
    fn a_hot_reload_marks_the_frame_shown_and_a_new_session_starts_bare() {
        let s = recording(10, 0);
        let (s, commands) = update(s, session(1, polled_clock(12, recorded(12, 1))));
        let reload = Mark {
            id: 1,
            frame: 12,
            kind: "reload-ok".into(),
            label: "hot reload".into(),
        };
        assert_eq!(commands[2], marks(vec![reload]));
        // Stopped: no transport and no marks; restarted, none until polled.
        let (s, commands) = update(s, client(ClientMsg::StopSession));
        assert_eq!(commands[2..], [transport(None), marks(vec![])]);
        let (s, _) = update(s, client(ClientMsg::StartSession));
        let listening = SessionEvent::Listening {
            debug_url: URL.to_string(),
        };
        let (s, commands) = update(s, session(2, listening));
        assert!(!commands.iter().any(|c| matches!(
            c,
            Command::Broadcast(ServerMsg::Transport { .. } | ServerMsg::Marks { .. })
        )));
        let Some(SessionStatus::Running { timeline, .. }) = s.session.map(|s| s.status) else {
            panic!("running");
        };
        assert_eq!(*timeline, Timeline::default());
    }

    /// `s` with the active project's game at `placement`.
    fn placed(mut s: AppState, placement: Placement) -> AppState {
        let mut layout = Layout::initial();
        layout.game.placement = placement;
        layout.normalize();
        s.layouts.insert("examples/a".to_string(), layout);
        s
    }

    fn place(placement: Placement) -> Event {
        client(ClientMsg::Layout {
            op: LayoutOp::Place { placement },
        })
    }

    #[test]
    fn the_placement_and_gpu_choose_the_display() {
        for (placement, no_gpu, display) in [
            (Placement::Pane, false, Display::Hidden),
            (Placement::Mini, false, Display::Hidden),
            (Placement::Window, false, Display::Window),
            (Placement::Pane, true, Display::Headless),
            (Placement::Window, true, Display::Headless),
        ] {
            let before = AppState {
                no_gpu,
                ..placed(state(), placement)
            };
            let (_, commands) = update(before, client(ClientMsg::StartSession));
            assert_eq!(commands[0], start_cmd(1, "examples/a", display));
        }
    }

    #[test]
    fn to_or_from_own_window_restarts_a_live_session_in_the_new_mode() {
        for (from, to, display) in [
            (Placement::Pane, Placement::Window, Display::Window),
            (Placement::Mini, Placement::Window, Display::Window),
            (Placement::Window, Placement::Pane, Display::Hidden),
            (Placement::Window, Placement::Mini, Display::Hidden),
        ] {
            for (status, hidden) in [
                (SessionStatus::Starting, vec![]),
                (running(Some(12)), vec![transport(None)]),
            ] {
                let before = placed(with_session(1, Some(9), status), from);
                let (next, commands) = update(before, place(to));
                assert_eq!(next.layouts["examples/a"].game.placement, to);
                assert_eq!(
                    next.session,
                    Some(Session {
                        id: 2,
                        pid: None,
                        status: SessionStatus::Starting
                    })
                );
                let expected = vec![
                    save_layout_cmd("examples/a", &next.layouts["examples/a"]),
                    Command::StopSession { id: 1 },
                    start_cmd(2, "examples/a", display),
                    broadcast(&next),
                ];
                assert_eq!(commands, [expected, hidden].concat(), "{from:?} -> {to:?}");
            }
        }
        // Without a live session the placement is only recorded, for the next start.
        let (next, commands) = update(state(), place(Placement::Window));
        assert_eq!(next.layouts["examples/a"].game.placement, Placement::Window);
        assert_eq!(next.session, None);
        assert_eq!(
            commands,
            vec![
                save_layout_cmd("examples/a", &next.layouts["examples/a"]),
                broadcast(&next)
            ]
        );
        // The current placement again only replies.
        let (again, commands) = update(next.clone(), place(Placement::Window));
        assert_eq!(again, next);
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(next)
            })]
        );
    }

    #[test]
    fn pane_and_mini_switch_without_a_restart() {
        let live = with_session(1, Some(9), running(Some(12)));
        let (mini, commands) = update(live.clone(), place(Placement::Mini));
        assert_eq!(mini.session, live.session);
        assert_eq!(mini.layouts["examples/a"].find(&View::Game), None);
        assert_eq!(
            commands,
            vec![
                save_layout_cmd("examples/a", &mini.layouts["examples/a"]),
                broadcast(&mini)
            ]
        );
        // Docked: back where it was, the same session.
        let (pane, commands) = update(mini, place(Placement::Pane));
        assert_eq!(pane.session, live.session);
        assert_eq!(pane.layouts["examples/a"], Layout::initial());
        assert!(!commands.iter().any(|c| matches!(
            c,
            Command::StopSession { .. } | Command::StartSession { .. }
        )));
    }

    #[test]
    fn a_live_game_in_a_pane_keeps_its_tab() {
        let live = with_session(1, Some(9), running(Some(12)));
        let reason = "the game is running: stop it, or switch it to Mini, to close its tab";
        for op in [
            LayoutOp::Close { view: View::Game },
            LayoutOp::ClosePane { pane: 1 },
        ] {
            let (next, commands) = update(live.clone(), client(ClientMsg::Layout { op }));
            assert_eq!(next, live);
            // Refused, with the state for the page to show what is open.
            assert_eq!(commands[..1], rejected(reason)[..]);
        }
        // Stopped (or ended), it closes like any tab, and Start game opens it again.
        let (stopped, _) = update(live, client(ClientMsg::StopSession));
        let close = client(ClientMsg::Layout {
            op: LayoutOp::Close { view: View::Game },
        });
        let (closed, _) = update(stopped, close);
        assert_eq!(closed.layouts["examples/a"].root, None);
        let (started, commands) = update(closed, client(ClientMsg::StartSession));
        assert_eq!(started.layouts["examples/a"], Layout::initial());
        assert_eq!(
            commands[..2],
            [
                save_layout_cmd("examples/a", &Layout::initial()),
                start_cmd(2, "examples/a", Display::Hidden)
            ]
        );
    }

    #[test]
    fn a_session_restarted_for_another_project_opens_its_game_tab() {
        // examples/b's saved layout has no Game tab; its session starts in a pane.
        let mut s = with_session(1, Some(9), running(Some(12)));
        s.start_on_open = true;
        let file_only = Layout {
            root: Some(crate::layout::Node::Pane(crate::layout::Pane {
                id: 1,
                tabs: vec![View::File {
                    path: "game.fun".to_string(),
                }],
                active: 0,
            })),
            focus: Some(1),
            game: Default::default(),
            agent: Default::default(),
        };
        s.layouts.insert("examples/b".to_string(), file_only);
        let (next, _) = update(s, switch("examples/b"));
        assert_eq!(
            crate::print_layout(&next.layouts["examples/b"]),
            ">#1[file:game.fun, *game]"
        );
        // In Mini, none opens.
        let mut s = with_session(1, Some(9), running(Some(12)));
        s.start_on_open = true;
        let mut mini = Layout::initial();
        mini.game.placement = Placement::Mini;
        mini.normalize();
        s.layouts.insert("examples/b".to_string(), mini.clone());
        let (next, _) = update(s, switch("examples/b"));
        assert_eq!(next.layouts["examples/b"], mini);
    }

    #[test]
    fn input_reaches_only_a_running_session_and_is_not_answered() {
        let key = || GameInput::Key {
            key: "w".to_string(),
            down: true,
        };
        let input = || client(ClientMsg::Input { input: key() });
        let before = with_session(3, Some(9), running(Some(12)));
        let (next, commands) = update(before.clone(), input());
        assert_eq!(next, before);
        assert_eq!(
            commands,
            vec![Command::Call {
                id: 3,
                debug_url: URL.to_string(),
                call: SessionCall::Input(key())
            }]
        );
        let failed = SessionStatus::Failed {
            reason: "x".to_string(),
        };
        for before in [
            state(),
            with_session(3, Some(9), SessionStatus::Starting),
            with_session(3, None, SessionStatus::Stopped),
            with_session(3, None, failed),
        ] {
            let (next, commands) = update(before.clone(), input());
            assert_eq!((next, commands), (before, vec![]));
        }
    }

    #[test]
    fn new_activates_the_project_nearest_start() {
        let s = AppState::new(
            Path::new("/repo"),
            Path::new("/repo/examples/b/assets"),
            vec![project("examples/a"), project("examples/b")],
        );
        assert_eq!(s.active.as_deref(), Some("examples/b"));
    }

    #[test]
    fn wire_messages_are_type_tagged() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"set_active_project","path":"examples/b"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::SetActiveProject {
                path: "examples/b".to_string()
            }
        );
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"start_session"}"#).unwrap();
        assert_eq!(msg, ClientMsg::StartSession);
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"layout","op":{"type":"place","placement":"window"}}"#)
                .unwrap();
        assert_eq!(
            msg,
            ClientMsg::Layout {
                op: LayoutOp::Place {
                    placement: Placement::Window
                }
            }
        );
        // Input is already in `POST /input`'s shape.
        let input = |json: &str| match serde_json::from_str(json).unwrap() {
            ClientMsg::Input { input } => serde_json::to_value(input).unwrap(),
            other => panic!("{other:?}"),
        };
        for wire in [
            serde_json::json!({"type": "key", "key": "w", "down": true}),
            serde_json::json!({"type": "mouse_move", "x": 10, "y": 20}),
            serde_json::json!({"type": "mouse_button", "button": "right", "down": false}),
            serde_json::json!({"type": "mouse_wheel", "delta": -1}),
        ] {
            let msg = serde_json::json!({"type": "input", "input": wire});
            assert_eq!(input(&msg.to_string()), wire);
        }
        let json = serde_json::to_value(ServerMsg::State {
            state: Box::new(with_session(1, Some(9), running(Some(12)))),
        })
        .unwrap();
        assert_eq!(json["type"], "state");
        assert_eq!(json["state"]["active"], "examples/a");
        assert_eq!(json["state"]["session"]["status"]["kind"], "running");
        // The clock travels apart from the state.
        assert_eq!(
            json["state"]["session"]["status"]["timeline"],
            serde_json::Value::Null
        );
        let json = serde_json::to_value(ServerMsg::Transport {
            transport: Some(shown(12, PolledClock::default())),
        })
        .unwrap();
        assert_eq!(json["type"], "transport");
        assert_eq!(json["transport"]["frame"], 12);
        let call = |json: serde_json::Value| -> ClientMsg { serde_json::from_value(json).unwrap() };
        assert_eq!(
            call(
                serde_json::json!({"type": "transport", "call": {"type": "seek", "frame": 40, "id": 2}})
            ),
            ClientMsg::Transport {
                call: TransportCall::Seek { frame: 40, id: 2 }
            }
        );
        assert_eq!(
            call(serde_json::json!({"type": "watch_transport", "on": true})),
            ClientMsg::WatchTransport { on: true }
        );
    }

    // Tasks.

    use crate::tasks::{AgentEvent, INTERRUPTED};

    /// A new task, given id 1.
    fn new_task(message: &str) -> Event {
        nth_task(1, message)
    }

    fn nth_task(id: u64, message: &str) -> Event {
        Event::NewTask {
            id: Ok(id),
            message: message.to_string(),
            harness: Harness::Claude,
            voice: None,
            scope: Default::default(),
        }
    }

    fn send(task: u64, message: &str) -> Event {
        client(ClientMsg::SendMessage {
            task,
            message: message.to_string(),
        })
    }

    fn agent(task: u64, run: u64, event: AgentEvent) -> Event {
        Event::Run {
            task,
            run,
            event: RunEvent::Agent(event),
        }
    }

    fn ended(task: u64, run: u64) -> Event {
        Event::Run {
            task,
            run,
            event: RunEvent::Ended { error: None },
        }
    }

    fn started(session: &str) -> AgentEvent {
        AgentEvent::Started {
            session: Some(session.to_string()),
            model: Some("claude-model".to_string()),
        }
    }

    fn finished() -> AgentEvent {
        AgentEvent::Finished {
            ok: true,
            summary: Some("done".to_string()),
        }
    }

    fn rejected(reason: &str) -> Vec<Command> {
        vec![Command::Reply(ServerMsg::Rejected {
            reason: reason.to_string(),
        })]
    }

    fn task(state: &AppState, id: u64) -> &Task {
        state.tasks.iter().find(|t| t.id == id).unwrap()
    }

    fn spec(commands: &[Command]) -> &RunSpec {
        commands
            .iter()
            .find_map(|c| match c {
                Command::StartRun(spec) => Some(&**spec),
                _ => None,
            })
            .unwrap()
    }

    /// `s` with its tasks' activity derived, as every step leaves it.
    fn derived(mut s: AppState) -> AppState {
        s.activity = activities(&s.tasks, &s.context_windows);
        s.decisions = decisions(&s.tasks);
        s.queues = queue::queues(&s.tasks, s.tracks_changes, s.active.as_deref());
        s
    }

    /// A task that ran once and finished: done.
    fn finished_task() -> AppState {
        let (s, _) = update(state(), new_task("What is the count?"));
        let (s, _) = update(s, agent(1, 0, started("sess-1")));
        let (s, _) = update(s, agent(1, 0, finished()));
        update(s, ended(1, 0)).0
    }

    #[test]
    fn a_new_task_starts_a_run_in_the_active_project() {
        let (next, commands) = update(state(), new_task("  What is the count?\nAnd why?"));
        let s0 = state();
        let t = task(&next, 1);
        assert_eq!(t.title, "What is the count?");
        assert_eq!(t.project, "examples/a");
        assert_eq!(t.status, TaskStatus::InProgress);
        let user = Item::User {
            text: "  What is the count?\nAnd why?".to_string(),
            spoken: None,
        };
        assert_eq!(t.transcript, std::slice::from_ref(&user));
        assert_eq!(
            commands,
            vec![
                Command::StartRun(Box::new(RunSpec {
                    task: 1,
                    run: 0,
                    harness: Harness::Claude,
                    dir: PathBuf::from("/repo/examples/a"),
                    repo_root: PathBuf::from("/repo"),
                    message: "  What is the count?\nAnd why?".to_string(),
                    resume: None,
                    snapshot: false,
                    baseline: None,
                    attach: None,
                    moment: false,
                    context: crate::agent_view::app_context(&s0, "examples/a"),
                    scope: Default::default(),
                })),
                Command::SaveTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: Box::new(t.clone()),
                },
                // The state goes before the task's first item.
                broadcast(&next),
                Command::Broadcast(ServerMsg::Item {
                    task: 1,
                    index: 0,
                    item: user
                }),
            ]
        );
    }

    #[test]
    fn a_call_starting_or_ending_resends_the_state_with_its_activity() {
        use crate::activity::{Call, CallKind, Stance};
        let (s, _) = update(state(), new_task("hi"));
        assert_eq!(s.activity[&1].stance, Stance::Thinking);
        let read = AgentEvent::ToolUse {
            name: "Read".to_string(),
            input: serde_json::json!({"file_path": "game.fun"}),
        };
        let (s, commands) = update(s, agent(1, 0, read));
        assert_eq!(s.activity[&1].stance, Stance::Working);
        assert_eq!(
            s.activity[&1].calls,
            [Call {
                kind: CallKind::Files,
                label: "reading game.fun".to_string()
            }]
        );
        assert!(commands.contains(&broadcast(&s)), "{commands:?}");
        // Streamed text leaves the activity as it is: no state.
        let (s, _) = update(
            s,
            agent(
                1,
                0,
                AgentEvent::ToolResult {
                    content: "1".into(),
                },
            ),
        );
        let delta = AgentEvent::TextDelta { text: "Th".into() };
        let (s, commands) = update(s, agent(1, 0, delta));
        assert!(!commands.contains(&broadcast(&s)), "{commands:?}");
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, ended(1, 0));
        assert_eq!(s.activity[&1].stance, Stance::Done);
    }

    /// A model's first run has no context bar until it ends, with the
    /// window; that window is kept (and saved) for its later runs.
    #[test]
    fn the_context_window_arrives_at_a_runs_end_and_is_kept() {
        use crate::activity::ContextFill;
        use crate::tasks::TokenUsage;
        let detail = |cache_read, window| {
            let last = TokenUsage {
                cache_read,
                ..TokenUsage::default()
            };
            AgentEvent::UsageDetail(UsageDetail {
                total: last,
                last: Some(last),
                context_window: window,
                ..UsageDetail::default()
            })
        };
        let started = AgentEvent::Started {
            session: Some("s".into()),
            model: Some("m".into()),
        };
        let (s, _) = update(state(), new_task("hi"));
        let (s, _) = update(s, agent(1, 0, started));
        let (s, _) = update(s, agent(1, 0, detail(800, None)));
        assert_eq!(s.activity[&1].context, None);
        let (s, commands) = update(s, agent(1, 0, detail(900, Some(1000))));
        let windows = BTreeMap::from([("claude/m".to_string(), 1000)]);
        assert!(commands.contains(&Command::SaveContextWindows { windows: windows.clone() }));
        let fill = Some(ContextFill { used: 900, window: 1000 });
        assert_eq!(s.activity[&1].context, fill);
        // The same window again is not saved again.
        let (s, commands) = update(s, agent(1, 0, detail(950, Some(1000))));
        assert!(!commands.iter().any(|c| matches!(c, Command::SaveContextWindows { .. })));
        let usage = AgentEvent::Usage {
            input: 950,
            output: 1,
        };
        let (s, _) = update(s, agent(1, 0, usage));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, ended(1, 0));
        // After the run, the last use stands; a follow-up's first event
        // shows it at once, before any usage of its own.
        assert_eq!(s.activity[&1].context, Some(ContextFill { used: 950, window: 1000 }));
        let (s, _) = update(s, send(1, "again"));
        assert_eq!(s.activity[&1].context, Some(ContextFill { used: 950, window: 1000 }));
        assert_eq!(s.context_windows, windows);
    }

    /// D39: the project's level by default, a new task's own when it gives
    /// one, kept with the task; Codex always reads all of the user's setup.
    #[test]
    fn a_tasks_scope_is_its_own_else_the_projects_and_never_less_for_codex() {
        let task_with = |harness, scope| Event::NewTask {
            id: Ok(1),
            message: "hi".into(),
            harness,
            scope,
            voice: None,
        };
        // The project's default, saved; setting it again changes nothing.
        let (s, commands) = update(state(), client(ClientMsg::SetScope { scope: Scope::Isolated }));
        assert!(commands.contains(&Command::SaveScope {
            dir: PathBuf::from("/repo/examples/a"),
            scope: Scope::Isolated
        }));
        let (s, commands) = update(s, client(ClientMsg::SetScope { scope: Scope::Isolated }));
        assert!(!commands.iter().any(|c| matches!(c, Command::SaveScope { .. })));
        let (s2, commands) = update(s.clone(), task_with(Harness::Claude, None));
        assert_eq!(spec(&commands).scope, Scope::Isolated);
        assert_eq!(task(&s2, 1).scope, Scope::Isolated);
        // A task's own level, kept by its follow-ups (they resume its session).
        let (s2, commands) = update(s.clone(), task_with(Harness::Claude, Some(Scope::Project)));
        assert_eq!(spec(&commands).scope, Scope::Project);
        let (s2, _) = update(s2, agent(1, 0, finished()));
        let (s2, _) = update(s2, ended(1, 0));
        let (s2, commands) = update(s2, send(1, "more"));
        assert_eq!(spec(&commands).scope, Scope::Project);
        assert_eq!(task(&s2, 1).scope, Scope::Project);
        // Codex: all of it, whatever the project or the request says.
        let (s3, commands) = update(s, task_with(Harness::Codex, Some(Scope::Isolated)));
        assert_eq!(spec(&commands).scope, Scope::UserAndProject);
        assert_eq!(task(&s3, 1).scope, Scope::UserAndProject);
    }

    #[test]
    fn a_run_attaches_the_active_projects_running_session() {
        let (next, commands) = update(with_session(1, Some(9), running(Some(3))), new_task("hi"));
        let first = spec(&commands);
        assert_eq!(first.attach.as_deref(), Some(URL));
        assert_eq!(first.context.session.as_deref(), Some(URL));
        // A follow-up on it after a switch to another project gets none.
        let (next, _) = update(next, agent(1, 0, finished()));
        let (next, _) = update(next, ended(1, 0));
        let (next, _) = update(next, switch("examples/b"));
        let (_, commands) = update(next, send(1, "again"));
        let again = spec(&commands);
        assert_eq!(again.attach, None);
        assert_eq!(again.context.project, "examples/a");
        assert_eq!(again.dir, PathBuf::from("/repo/examples/a"));
    }

    #[test]
    fn agent_events_stream_items_and_an_ok_end_is_done() {
        let (s, _) = update(state(), new_task("hi"));
        // Started changes the summary: saved (for a resume after a crash)
        // and the state is broadcast.
        let (s, commands) = update(s, agent(1, 0, started("sess-1")));
        let t = task(&s, 1);
        assert_eq!(t.session_id.as_deref(), Some("sess-1"));
        let saved = Command::SaveTask {
            dir: PathBuf::from("/repo/examples/a"),
            task: Box::new(t.clone()),
        };
        assert_eq!(commands, vec![saved, broadcast(&s)]);
        // A delta only sends its item, unsaved.
        let delta = AgentEvent::TextDelta {
            text: "Hel".to_string(),
        };
        let (s, commands) = update(s, agent(1, 0, delta));
        let item = Item::Agent {
            text: "Hel".to_string(),
            partial: true,
        };
        assert_eq!(
            commands,
            vec![Command::Broadcast(ServerMsg::Item {
                task: 1,
                index: 1,
                item
            })]
        );
        let (s, _) = update(s, agent(1, 0, finished()));
        assert_eq!(task(&s, 1).status, TaskStatus::InProgress);
        let (s, commands) = update(s, ended(1, 0));
        let t = task(&s, 1);
        assert_eq!(t.status, TaskStatus::Done);
        assert_eq!(t.run, None);
        assert_eq!(
            commands,
            vec![
                Command::SaveTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: Box::new(t.clone()),
                },
                broadcast(&s),
                // Its text never got a whole `text`: it is settled as it stands.
                Command::Broadcast(ServerMsg::Item {
                    task: 1,
                    index: 1,
                    item: Item::Agent {
                        text: "Hel".to_string(),
                        partial: false,
                    },
                }),
            ]
        );
    }

    #[test]
    fn a_follow_up_resumes_the_stored_session() {
        let (next, commands) = update(finished_task(), send(1, "And now?"));
        let follow_up = spec(&commands);
        assert_eq!(follow_up.resume.as_deref(), Some("sess-1"));
        // The run is named by its user message's index.
        assert_eq!(follow_up.run, 2);
        assert_eq!(task(&next, 1).status, TaskStatus::InProgress);
        // Events for the old run are dropped.
        let (again, commands) = update(next.clone(), agent(1, 0, finished()));
        assert_eq!((again, commands), (next, vec![]));
    }

    fn queue_of<'a>(s: &'a AppState, project: &str) -> Option<&'a Queue> {
        s.queues.get(project)
    }

    /// A message to the running task steers it (D29): the run stops and
    /// the session resumes with it. A new task in the project waits its
    /// turn (D30); a follow-up on another task is refused; another project
    /// runs alongside.
    #[test]
    fn a_busy_project_steers_its_run_and_queues_new_tasks() {
        let (s, _) = update(state(), new_task("first"));
        let (s, _) = update(s, agent(1, 0, started("sess-1")));
        let (s, commands) = update(s, send(1, "more"));
        let stop = commands
            .iter()
            .position(|c| *c == Command::StopRun { task: 1, run: 0 });
        let start = commands
            .iter()
            .position(|c| matches!(c, Command::StartRun(_)));
        assert!(stop.is_some() && stop < start, "{commands:?}");
        let run = spec(&commands);
        assert_eq!(
            (run.run, run.message.as_str(), run.resume.as_deref()),
            (2, "more", Some("sess-1"))
        );
        assert_eq!(task(&s, 1).transcript[1], Item::Stopped);

        let (s, commands) = update(s, nth_task(2, "second"));
        assert!(!commands.iter().any(|c| matches!(c, Command::StartRun(_))));
        assert_eq!(task(&s, 2).status, TaskStatus::Queued);
        assert_eq!(s.current_task["examples/a"], Some(2));
        assert_eq!(
            queue_of(&s, "examples/a"),
            Some(&Queue {
                tasks: vec![2],
                wait: Some(queue::Wait::Running { task: 1 }),
                holds: true,
            })
        );
        // A queued task has no agent, so no card.
        assert!(!s.activity.contains_key(&2));
        // Queued with the level picked, kept until it starts (D39).
        let isolated = Event::NewTask {
            id: Ok(3),
            message: "third".to_string(),
            harness: Harness::Claude,
            scope: Some(crate::tasks::Scope::Isolated),
            voice: None,
        };
        let (s, commands) = update(s, isolated);
        assert_eq!(task(&s, 3).scope, crate::tasks::Scope::Isolated);
        assert_eq!(queue_of(&s, "examples/a").unwrap().tasks, [2, 3]);
        assert!(matches!(&commands[..], [Command::SaveTask { .. }, _, _]));

        // Said to a queued task: kept for its start, not run.
        let (s, commands) = update(s, send(3, "and fourth"));
        assert_eq!(commands.len(), 3, "{commands:?}");
        assert_eq!(task(&s, 3).transcript.len(), 2);
        // The next one starts as the run ends; a follow-up on the first waits.
        let (s, _) = update(s, agent(1, 2, finished()));
        let (done, _) = update(s.clone(), ended(1, 2));
        assert!(task(&done, 2).run.is_some());
        let (_, commands) = update(done, send(1, "again"));
        assert_eq!(
            commands,
            rejected("task 2 is running in examples/a: one run per project at a time")
        );

        let (b, _) = update(s, switch("examples/b"));
        let (b, commands) = update(b, nth_task(4, "in b"));
        assert_eq!(task(&b, 4).project, "examples/b");
        assert!(matches!(commands[0], Command::StartRun(_)), "{commands:?}");
    }

    /// A queued task starts by itself when the run ends (ok, failed or
    /// stopped), given its request and everything said to it since, in order.
    #[test]
    fn a_queued_task_starts_when_the_run_ends_with_every_message() {
        let busy = || {
            let (s, _) = update(state(), new_task("first"));
            let (s, _) = update(s, nth_task(2, "make it red"));
            update(s, send(2, "no, blue")).0
        };
        for end in [ended(1, 0), client(ClientMsg::StopTask { task: 1 })] {
            let (s, commands) = update(busy(), end);
            let run = spec(&commands);
            assert_eq!((run.task, run.run), (2, 1));
            assert_eq!(
                run.message,
                format!("make it red\n\n{}\n\nno, blue", queue::LATER)
            );
            assert_eq!(run.resume, None);
            let t = task(&s, 2);
            assert_eq!((t.queued, &t.status), (None, &TaskStatus::InProgress));
            assert_eq!(queue_of(&s, "examples/a").unwrap().tasks, Vec::<u64>::new());
        }
        // Its events then land on that run.
        let (s, _) = update(busy(), ended(1, 0));
        let (s, _) = update(s, agent(2, 1, finished()));
        let (s, _) = update(s, ended(2, 1));
        assert_eq!(task(&s, 2).status, TaskStatus::Done);
    }

    /// A change waiting for Keep or Revert holds the queue: a second task's
    /// edits would land in its change. The queue says whose, and starts
    /// once it is kept or reverted; an empty change holds nothing.
    #[test]
    fn a_change_awaiting_a_decision_holds_the_queue_until_kept_or_reverted() {
        let (s, _) = update(tracked(), new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, nth_task(2, "then bounce"));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert!(!commands.iter().any(|c| matches!(c, Command::StartRun(_))));
        assert_eq!(
            queue_of(&s, "examples/a").unwrap().wait,
            Some(queue::Wait::Decision { task: 1 })
        );
        // A new task goes behind it; one the user starts now runs beside
        // the change, as a new task with nothing queued does.
        let (behind, _) = update(s.clone(), nth_task(3, "and spin"));
        assert_eq!(queue_of(&behind, "examples/a").unwrap().tasks, [2, 3]);
        let (now, commands) = update(behind, client(ClientMsg::StartQueued { task: 3 }));
        assert_eq!(spec(&commands).task, 3);
        let (_, commands) = update(now, client(ClientMsg::StartQueued { task: 2 }));
        assert_eq!(
            commands,
            rejected("task 3 is running in examples/a: one run per project at a time")
        );
        let (_, commands) = update(reviewed(one_file(2)), nth_task(2, "beside"));
        assert_eq!(spec(&commands).task, 2);

        let (kept, commands) = update(s.clone(), keep(1));
        let run = spec(&commands);
        assert_eq!((run.task, run.snapshot), (2, true));
        assert!(task(&kept, 2).run.is_some());

        let (reverting, _) = update(s, revert(1));
        assert_eq!(
            queue_of(&reverting, "examples/a").unwrap().wait,
            Some(queue::Wait::Reverting { task: 1 })
        );
        let (_, commands) = update(reverting, reverted(1, "tree-1", Ok(())));
        assert_eq!(spec(&commands).task, 2);

        // Stopped: held while its change is measured, then free if empty.
        let (s, _) = update(tracked(), new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, nth_task(2, "then bounce"));
        let (s, _) = update(s, client(ClientMsg::StopTask { task: 1 }));
        assert_eq!(
            queue_of(&s, "examples/a").unwrap().wait,
            Some(queue::Wait::Measuring { task: 1 })
        );
        let (_, commands) = update(s, change(1, "tree-1", Change::default()));
        assert_eq!(spec(&commands).task, 2);
    }

    /// The queue is reordered and pruned by the user; "run next" is a move
    /// to the front; each project keeps its own line.
    #[test]
    fn a_queue_is_reordered_and_pruned_and_kept_per_project() {
        let (s, _) = update(state(), new_task("first"));
        let (s, _) = update(s, nth_task(2, "two"));
        let (s, _) = update(s, nth_task(3, "three"));
        let (s, _) = update(s, nth_task(4, "four"));
        let to = |task, to| client(ClientMsg::MoveQueued { task, to });
        let (s, commands) = update(s, to(4, 0));
        assert_eq!(queue_of(&s, "examples/a").unwrap().tasks, [4, 2, 3]);
        let saved: Vec<u64> = commands
            .iter()
            .filter_map(|c| match c {
                Command::SaveTask { task, .. } => Some(task.id),
                _ => None,
            })
            .collect();
        assert_eq!(saved, [2, 3, 4]);
        let (s, commands) = update(s, to(1, 0));
        assert_eq!(commands, rejected("task 1 is not queued"));
        let (s, _) = update(s, client(ClientMsg::DeleteTask { task: 2 }));
        assert_eq!(queue_of(&s, "examples/a").unwrap().tasks, [4, 3]);

        // Switched away: a's queue still starts in its turn.
        let (s, _) = update(s, switch("examples/b"));
        let (s, commands) = update(s, ended(1, 0));
        assert_eq!(spec(&commands).task, 4);
        assert_eq!(task(&s, 4).project, "examples/a");
    }

    /// A restart keeps the queue (it is saved with each task); the loader's
    /// own step starts nothing, and the next step starts the head of a free
    /// project. Without auto-start (`task`, `--print`) nothing is queued.
    #[test]
    fn a_restart_keeps_the_queue_and_only_the_app_starts_it() {
        let (s, _) = update(state(), new_task("first"));
        let (s, commands) = update(s, nth_task(2, "two"));
        let Some(Command::SaveTask { task: saved, .. }) = commands.first() else {
            panic!("{commands:?}");
        };
        let stored = crate::tasks::from_stored(&crate::tasks::to_stored(saved)).unwrap();
        assert_eq!(stored.queued, task(&s, 2).queued);
        let loaded = Event::TasksLoaded {
            tasks: vec![task(&s, 1).clone(), stored],
        };
        let (s, commands) = update(state(), loaded);
        assert!(!commands.iter().any(|c| matches!(c, Command::StartRun(_))));
        assert_eq!(task(&s, 2).status, TaskStatus::Queued);
        // The active project's waits for its game to listen.
        let mut waiting = s.clone();
        waiting.session = Some(Session {
            id: 7,
            pid: None,
            status: SessionStatus::Starting,
        });
        let (waiting, commands) = update(waiting, client(ClientMsg::GetState));
        assert!(!commands.iter().any(|c| matches!(c, Command::StartRun(_))));
        let listening = SessionEvent::Listening {
            debug_url: "http://127.0.0.1:6000".into(),
        };
        let (_, commands) = update(waiting, session(7, listening));
        assert_eq!(spec(&commands).attach.as_deref(), Some("http://127.0.0.1:6000"));
        let (s, commands) = update(s, client(ClientMsg::GetState));
        assert_eq!(spec(&commands).task, 2);
        assert!(task(&s, 2).run.is_some());

        // A change left unmeasured when the app quit is measured on open,
        // so it does not hold the queue until someone looks.
        let mut saved = task(&finished_task(), 1).clone();
        saved.baseline = Some("tree-1".to_string());
        let opened = Event::Opened {
            id: 0,
            folder: "/other/".into(),
            result: Ok(Box::new(AppState {
                tracks_changes: true,
                tasks: vec![saved],
                ..other()
            })),
        };
        let (_, commands) = update(state(), opened);
        assert!(commands.contains(&Command::Diff {
            task: 1,
            baseline: "tree-1".to_string(),
            reply: false,
        }));

        let manual = AppState {
            auto_start: false,
            ..state()
        };
        let (s, _) = update(manual, new_task("first"));
        let (_, commands) = update(s, nth_task(2, "two"));
        assert_eq!(
            commands,
            rejected("task 1 is running: one run per project at a time")
        );
    }

    #[test]
    fn bad_task_messages_are_rejected() {
        let s = finished_task();
        for (event, reason) in [
            (send(7, "hi"), "there is no task 7"),
            (send(1, "  \n"), "the message is empty"),
            (new_task(""), "the message is empty"),
            (
                client(ClientMsg::StopTask { task: 1 }),
                "task 1 is not running",
            ),
            (
                client(ClientMsg::GetTranscript { task: 7 }),
                "there is no task 7",
            ),
        ] {
            let (next, commands) = update(s.clone(), event);
            assert_eq!(next, s);
            assert_eq!(commands, rejected(reason));
        }
        let none = AppState {
            projects: vec![],
            active: None,
            ..state()
        };
        let (_, commands) = update(none, new_task("hi"));
        assert_eq!(commands, rejected("there is no project to start a task in"));
    }

    #[test]
    fn stop_cancels_the_run_and_waits_on_the_user() {
        let (s, _) = update(state(), new_task("hi"));
        let (s, commands) = update(s, client(ClientMsg::StopTask { task: 1 }));
        let t = task(&s, 1);
        assert_eq!(t.status, TaskStatus::WaitingOnYou);
        assert_eq!(t.transcript.last(), Some(&Item::Stopped));
        assert_eq!(
            commands,
            vec![
                Command::StopRun { task: 1, run: 0 },
                Command::SaveTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: Box::new(t.clone()),
                },
                broadcast(&s),
                Command::Broadcast(ServerMsg::Item {
                    task: 1,
                    index: 1,
                    item: Item::Stopped
                }),
            ]
        );
        // The cancelled run's own end is dropped.
        let (next, commands) = update(s.clone(), ended(1, 0));
        assert_eq!((next, commands), (s, vec![]));
    }

    #[test]
    fn a_run_that_could_not_start_fails_with_the_error() {
        let (s, _) = update(state(), new_task("hi"));
        let error = Event::Run {
            task: 1,
            run: 0,
            event: RunEvent::Ended {
                error: Some("cannot run functor: not found".to_string()),
            },
        };
        let (s, _) = update(s, error);
        assert_eq!(
            task(&s, 1).status,
            TaskStatus::Failed {
                summary: Some("cannot run functor: not found".to_string())
            }
        );
    }

    #[test]
    fn get_transcript_replies_with_the_items() {
        let s = finished_task();
        let (next, commands) = update(s.clone(), client(ClientMsg::GetTranscript { task: 1 }));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::Transcript {
                task: 1,
                items: task(&s, 1).transcript.clone()
            })]
        );
    }

    #[test]
    fn loaded_tasks_lose_their_runs() {
        let mut saved = task(&finished_task(), 1).clone();
        saved.id = 4;
        // Saved mid-run, before the app died.
        let mut cut = saved.clone();
        cut.id = 2;
        cut.transcript.push(Item::User {
            text: "more".to_string(),
            spoken: None,
        });
        cut.run = Some(Run {
            id: 3,
            ..Default::default()
        });
        cut.status = TaskStatus::InProgress;
        let (s, _) = update(
            state(),
            Event::TasksLoaded {
                tasks: vec![saved, cut],
            },
        );
        assert_eq!(s.tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(task(&s, 2).run, None);
        assert_eq!(
            task(&s, 2).status,
            TaskStatus::Failed {
                summary: Some(INTERRUPTED.to_string())
            }
        );
        assert_eq!(task(&s, 4).status, TaskStatus::Done);
    }

    #[test]
    fn a_new_task_takes_the_shells_id_or_is_rejected() {
        let (s, _) = update(state(), nth_task(7, "hi"));
        assert_eq!(s.tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [7]);
        let failed = Event::NewTask {
            id: Err("disk full".to_string()),
            message: "hi".to_string(),
            harness: Harness::Claude,
            voice: None,
            scope: Default::default(),
        };
        let (next, commands) = update(state(), failed);
        assert_eq!(next, state());
        assert_eq!(commands, rejected("cannot reserve a task id: disk full"));
        let bare = client(ClientMsg::NewTask {
            message: "hi".to_string(),
            harness: Harness::Claude,
            scope: Default::default(),
        });
        let (next, commands) = update(state(), bare);
        assert_eq!(next, state());
        assert_eq!(commands, rejected("a new task needs an id"));
    }

    #[test]
    fn shutdown_stops_the_session_and_every_run() {
        let before = with_session(1, Some(9), running(Some(3)));
        let (s, _) = update(before, new_task("hi"));
        let (s, commands) = update(s, Event::Shutdown);
        assert_eq!(task(&s, 1).transcript.last(), Some(&Item::Stopped));
        let kinds: Vec<&str> = commands
            .iter()
            .map(|c| match c {
                Command::StopSession { .. } => "stop_session",
                Command::StopRun { .. } => "stop_run",
                Command::SaveTask { .. } => "save",
                Command::Broadcast(_) => "broadcast",
                other => panic!("{other:?}"),
            })
            .collect();
        // The state (the stopped task's activity), its item, and the
        // transport hidden.
        assert_eq!(
            kinds,
            [
                "stop_session",
                "stop_run",
                "save",
                "broadcast",
                "broadcast",
                "broadcast"
            ]
        );
    }

    #[test]
    fn the_harness_pick_is_remembered_per_project() {
        let pick = |harness| client(ClientMsg::SetHarness { harness });
        let (s, commands) = update(state(), pick(Harness::Codex));
        assert_eq!(s.harness_choice["examples/a"], Harness::Codex);
        assert_eq!(
            commands,
            vec![
                Command::SaveHarness {
                    dir: PathBuf::from("/repo/examples/a"),
                    harness: Harness::Codex
                },
                broadcast(&s)
            ]
        );
        // The same pick again changes nothing; another project keeps its own.
        let (s, commands) = update(s, pick(Harness::Codex));
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(s.clone())
            })]
        );
        let (s, _) = update(s, switch("examples/b"));
        let (s, _) = update(s, pick(Harness::Claude));
        assert_eq!(
            s.harness_choice,
            BTreeMap::from([
                ("examples/a".to_string(), Harness::Codex),
                ("examples/b".to_string(), Harness::Claude)
            ])
        );
        // An agent this machine does not have is refused.
        let only_claude = AppState {
            harnesses: vec![Harness::Claude],
            ..state()
        };
        let (next, commands) = update(only_claude.clone(), pick(Harness::Codex));
        assert_eq!(next, only_claude);
        assert_eq!(
            commands,
            vec![
                Command::Reply(ServerMsg::Rejected {
                    reason: "codex is not available on this machine".to_string()
                }),
                Command::Reply(ServerMsg::State {
                    state: Box::new(only_claude)
                })
            ]
        );
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"set_harness","harness":"codex"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::SetHarness {
                harness: Harness::Codex
            }
        );
    }

    #[test]
    fn task_messages_are_type_tagged() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"new_task","message":"hi","harness":"codex"}"#)
                .unwrap();
        assert_eq!(
            msg,
            ClientMsg::NewTask {
                message: "hi".to_string(),
                harness: Harness::Codex,
                scope: Default::default(),
            }
        );
        let json = serde_json::to_value(ServerMsg::Item {
            task: 1,
            index: 2,
            item: Item::Agent {
                text: "Hel".to_string(),
                partial: true,
            },
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "item", "task": 1, "index": 2,
                "item": {"kind": "agent", "text": "Hel", "partial": true}})
        );
        // The state carries task summaries, not transcripts.
        let json = serde_json::to_value(ServerMsg::State {
            state: Box::new(finished_task()),
        })
        .unwrap();
        assert_eq!(json["state"]["tasks"][0]["status"]["kind"], "done");
        assert!(json["state"]["tasks"][0].get("transcript").is_none());
    }

    // The Tasks view: the current task, rename, delete, times.

    fn select(task: Option<u64>) -> Event {
        client(ClientMsg::SelectTask {
            task,
            if_unset: false,
        })
    }

    fn at(now: u64, s: AppState) -> AppState {
        AppState { now, ..s }
    }

    #[test]
    fn the_current_task_is_one_per_project_and_a_new_task_takes_it() {
        let (s, _) = update(state(), new_task("one"));
        assert_eq!(s.current_task.get("examples/a"), Some(&Some(1)));
        // Back to a new task, then task 1 again: each answered with the state.
        let (s, commands) = update(s, select(None));
        assert_eq!(s.current_task["examples/a"], None);
        assert_eq!(commands, vec![broadcast(&s)]);
        let (s, _) = update(s, select(Some(1)));
        let (again, commands) = update(s.clone(), select(Some(1)));
        assert_eq!(again, s);
        assert!(matches!(
            &commands[..],
            [Command::Reply(ServerMsg::State { .. })]
        ));
        // Another project's task, or none, is refused.
        let (s, _) = update(s, ended(1, 0));
        let (s, _) = update(s, switch("examples/b"));
        let (s, commands) = update(s, select(Some(1)));
        assert_eq!(commands, rejected("there is no task 1 in examples/b"));
        assert_eq!(s.current_task.get("examples/b"), None);
        assert_eq!(s.current_task["examples/a"], Some(1));
        let (s, commands) = update(s, select(Some(9)));
        assert_eq!(commands, rejected("there is no task 9 in examples/b"));
        // A page's default applies only where nothing is chosen yet.
        let default = |task| {
            client(ClientMsg::SelectTask {
                task,
                if_unset: true,
            })
        };
        let (s, _) = update(s, switch("examples/a"));
        let (s, _) = update(s, default(None));
        assert_eq!(s.current_task["examples/a"], Some(1));
        let (s, _) = update(s, switch("examples/b"));
        let (s, _) = update(s, default(None));
        assert_eq!(s.current_task["examples/b"], None);
    }

    #[test]
    fn a_rename_is_saved_and_a_bad_title_refused() {
        let s = finished_task();
        let rename = |title: &str| {
            client(ClientMsg::RenameTask {
                task: 1,
                title: title.to_string(),
            })
        };
        let (s, commands) = update(s, rename("  Count  check "));
        let t = task(&s, 1);
        assert_eq!(t.title, "Count check");
        assert_eq!(
            commands,
            vec![
                Command::SaveTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: Box::new(t.clone()),
                },
                broadcast(&s),
            ]
        );
        let (s, commands) = update(s, rename(" "));
        assert_eq!(commands, rejected("a task's title cannot be empty"));
        assert_eq!(task(&s, 1).title, "Count check");
        let (_, commands) = update(
            s,
            client(ClientMsg::RenameTask {
                task: 5,
                title: "x".to_string(),
            }),
        );
        assert_eq!(commands, rejected("there is no task 5"));
    }

    #[test]
    fn delete_forgets_the_task_closes_its_tabs_and_removes_only_its_file() {
        let (s, _) = update(state(), nth_task(1, "one"));
        let (s, _) = update(s, ended(1, 0));
        let (s, _) = update(s, nth_task(2, "two"));
        let (s, _) = update(s, ended(2, 0));
        // Task 1's page and changes beside the game, task 2's page too.
        let open = |view| {
            op(LayoutOp::Open {
                view,
                target: Target::Side,
            })
        };
        let (s, _) = update(s, open(View::Task { task: 1 }));
        let (s, _) = update(s, open(View::Changes { task: 1 }));
        let (s, _) = update(s, open(View::Task { task: 2 }));
        let (s, _) = update(s, select(Some(1)));
        let (s, commands) = update(s, client(ClientMsg::DeleteTask { task: 1 }));
        assert_eq!(s.tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [2]);
        assert_eq!(s.current_task["examples/a"], None);
        let layout = &s.layouts["examples/a"];
        assert_eq!(
            crate::print_layout(layout),
            "right 50% (#1[*game] | >#3[*task:2])"
        );
        assert_eq!(
            commands,
            vec![
                Command::SaveLayout {
                    dir: PathBuf::from("/repo/examples/a"),
                    layout: Box::new(layout.clone()),
                },
                Command::DeleteTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: 1,
                },
                broadcast(&s),
            ]
        );
        let (_, commands) = update(s, client(ClientMsg::DeleteTask { task: 1 }));
        assert_eq!(commands, rejected("there is no task 1"));
    }

    #[test]
    fn delete_is_refused_while_running_or_with_a_change_to_review() {
        let delete = || client(ClientMsg::DeleteTask { task: 1 });
        let (s, _) = update(state(), new_task("hi"));
        let (s, commands) = update(s, delete());
        assert_eq!(
            commands,
            rejected("task 1 is running: stop it before deleting the task")
        );
        assert_eq!(s.tasks.len(), 1);
        let s = reviewed(one_file(2));
        let (s, commands) = update(s, delete());
        assert_eq!(
            commands,
            rejected("task 1 has a change to review: keep or revert it before deleting the task")
        );
        // A stopped run's change is unknown until measured: no delete before.
        let (s2, _) = update(tracked(), new_task("hi"));
        let (s2, _) = update(s2, baseline(1, 0, "tree-1"));
        let (s2, _) = update(s2, client(ClientMsg::StopTask { task: 1 }));
        let (s2, commands) = update(s2, delete());
        assert_eq!(
            commands,
            rejected("task 1's change is not measured yet: open its changes, then try again")
        );
        let (s2, _) = update(s2, change(1, "tree-1", Change::default()));
        let (s2, _) = update(s2, delete());
        assert!(s2.tasks.is_empty());
        // Once kept it goes, leaving the files as they are (no revert).
        let (s, _) = update(s, keep(1));
        let (s, commands) = update(s, delete());
        assert!(s.tasks.is_empty());
        assert!(commands
            .iter()
            .all(|c| !matches!(c, Command::Revert { .. })));
        assert!(commands.contains(&Command::DeleteTask {
            dir: PathBuf::from("/repo/examples/a"),
            task: 1,
        }));
    }

    #[test]
    fn a_deleted_tab_is_not_brought_back_by_restore() {
        let (s, _) = update(state(), new_task("hi"));
        let (s, _) = update(s, ended(1, 0));
        let open = op(LayoutOp::Open {
            view: View::Task { task: 1 },
            target: Target::Side,
        });
        let (mut s, _) = update(s, open);
        // As if the agent had arranged views since: the snapshot holds the page.
        let layout = s.layouts.get_mut("examples/a").unwrap();
        layout.agent.restore = Some(Box::new(layout.clone()));
        let _ = layout.apply(LayoutOp::Close { view: View::Game }, |_| Ok(()));
        let (s, _) = update(s, client(ClientMsg::DeleteTask { task: 1 }));
        let layout = &s.layouts["examples/a"];
        let before = layout.agent.restore.as_deref().expect("a snapshot");
        assert_eq!(crate::print_layout(before), ">#1[*game]");
    }

    #[test]
    fn task_times_follow_the_shells_clock() {
        let (s, _) = update(at(1_000, state()), new_task("hi"));
        let t = task(&s, 1);
        assert_eq!((t.created_at, t.updated_at), (Some(1_000), Some(1_000)));
        // A run's events and its end, then a review, move the last activity.
        let (s, _) = update(at(2_000, s), agent(1, 0, started("s")));
        assert_eq!(task(&s, 1).updated_at, Some(1_000), "no item, no activity");
        let (s, _) = update(at(3_000, s), agent(1, 0, finished()));
        assert_eq!(task(&s, 1).updated_at, Some(3_000));
        let (s, _) = update(at(4_000, s), ended(1, 0));
        let t = task(&s, 1);
        assert_eq!((t.created_at, t.updated_at), (Some(1_000), Some(4_000)));
        // A rename or a selection is not activity.
        let rename = client(ClientMsg::RenameTask {
            task: 1,
            title: "x".to_string(),
        });
        let (s, _) = update(at(5_000, s), rename);
        let (s, _) = update(at(6_000, s), select(None));
        assert_eq!(task(&s, 1).updated_at, Some(4_000));
        let (s, _) = update(at(7_000, s), send(1, "again"));
        assert_eq!(task(&s, 1).updated_at, Some(7_000));
        let (s, _) = update(at(8_000, s), client(ClientMsg::StopTask { task: 1 }));
        assert_eq!(task(&s, 1).updated_at, Some(8_000));
        // A keep, and a finished revert.
        let s = reviewed(one_file(2));
        assert_eq!(task(&s, 1).updated_at, None, "no clock, no time");
        let (s, _) = update(at(9_000, s), keep(1));
        assert_eq!(task(&s, 1).updated_at, Some(9_000));
        let s = reviewed(one_file(2));
        let (s, _) = update(s, revert(1));
        let (s, _) = update(at(10_000, s), reverted(1, "tree-1", Ok(())));
        assert_eq!(task(&s, 1).updated_at, Some(10_000));
    }

    #[test]
    fn task_view_messages_are_type_tagged() {
        for (json, msg) in [
            (
                r#"{"type":"select_task","task":null,"if_unset":false}"#,
                ClientMsg::SelectTask {
                    task: None,
                    if_unset: false,
                },
            ),
            (
                r#"{"type":"select_task","task":3,"if_unset":true}"#,
                ClientMsg::SelectTask {
                    task: Some(3),
                    if_unset: true,
                },
            ),
            (
                r#"{"type":"rename_task","task":3,"title":"x"}"#,
                ClientMsg::RenameTask {
                    task: 3,
                    title: "x".to_string(),
                },
            ),
            (
                r#"{"type":"delete_task","task":3}"#,
                ClientMsg::DeleteTask { task: 3 },
            ),
        ] {
            assert_eq!(serde_json::from_str::<ClientMsg>(json).unwrap(), msg);
        }
    }

    // Changes.

    use crate::changes::FileChange;

    fn tracked() -> AppState {
        AppState {
            tracks_changes: true,
            ..state()
        }
    }

    fn baseline(task: u64, run: u64, tree: &str) -> Event {
        Event::Run {
            task,
            run,
            event: RunEvent::Baseline {
                tree: tree.to_string(),
            },
        }
    }

    fn one_file(added: u32) -> Change {
        Change::from_files(
            vec![FileChange {
                path: "examples/a/game.fun".to_string(),
                from: None,
                added,
                removed: 1,
                binary: false,
                large: false,
            }],
            format!("now-{added}"),
        )
    }

    fn change(task: u64, tree: &str, change: Change) -> Event {
        Event::Change {
            task,
            baseline: tree.to_string(),
            change,
        }
    }

    /// Task 1's first run, through its baseline, a change and its end.
    fn reviewed(with: Change) -> AppState {
        let (s, commands) = update(tracked(), new_task("Make it spin"));
        assert!(spec(&commands).snapshot);
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", with));
        update(s, ended(1, 0)).0
    }

    fn keep(task: u64) -> Event {
        client(ClientMsg::KeepChange { task })
    }

    fn revert(task: u64) -> Event {
        client(ClientMsg::RevertChange {
            task,
            tree: "now-2".to_string(),
        })
    }

    fn reverted(task: u64, tree: &str, result: Result<(), String>) -> Event {
        Event::Reverted {
            task,
            baseline: tree.to_string(),
            result,
        }
    }

    #[test]
    fn the_report_holds_until_the_files_change_after_the_run_or_the_review_ends() {
        let mut s = reviewed(one_file(2));
        s.tasks[0].report = Some(crate::decision::Report {
            summary: Some("One value changed.".into()),
            ..Default::default()
        });
        // Measured again, the same: it stands, and so does the decision on it.
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        assert!(task(&s, 1).report.is_some());
        assert!(s.decisions.contains_key(&1));
        let (kept, _) = update(s.clone(), keep(1));
        assert_eq!(task(&kept, 1).report, None, "the review is over");
        assert!(kept.decisions.is_empty());
        // A later edit (the user's, by hand) is not what the agent reported on.
        let (s, _) = update(s, change(1, "tree-1", one_file(3)));
        assert_eq!(task(&s, 1).report, None);
        assert_eq!(s.decisions[&1].reason, "The agent gave no report.");
    }

    // Reviews (D31).

    /// Task 1's first run, ended with `with` where reviews run; the end's
    /// commands. Its agent reported first, when `agent_report`.
    fn review_of(with: Change, agent_report: bool) -> (AppState, Vec<Command>) {
        let s = AppState {
            can_review: true,
            ..tracked()
        };
        let (s, _) = update(s, new_task("Make it spin"));
        let (mut s, _) = update(s, baseline(1, 0, "tree-1"));
        if agent_report {
            s.tasks[0].report = Some(crate::decision::Report {
                summary: Some("I made it spin.".into()),
                notes: Some(vec![]),
                ..Default::default()
            });
        }
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", with));
        update(s, ended(1, 0))
    }

    fn review_specs(commands: &[Command]) -> Vec<&ReviewSpec> {
        (commands.iter())
            .filter_map(|c| match c {
                Command::StartReview(spec) => Some(&**spec),
                _ => None,
            })
            .collect()
    }

    fn review_stops(commands: &[Command]) -> usize {
        (commands.iter())
            .filter(|c| matches!(c, Command::StopReview { task: 1, run: 0 }))
            .count()
    }

    fn reviewed_event(event: ReviewEvent) -> Event {
        Event::Review {
            task: 1,
            run: 0,
            event,
        }
    }

    fn review_ended(error: Option<&str>) -> Event {
        let error = error.map(String::from);
        reviewed_event(ReviewEvent::Ended { error })
    }

    /// The review's `report` (or, not `review`, the working run's).
    fn report_from(review: bool) -> Event {
        Event::AgentView {
            task: 1,
            run: 0,
            review,
            request: crate::agent_view::ViewRequest::Report {
                summary: "The cube spins at 2 turns per second. Check the speed in the game."
                    .into(),
                notes: vec!["Nobody tried it with a gamepad.".into()],
                evidence: vec![],
                steps: vec![],
            },
        }
    }

    fn view_reply(commands: &[Command]) -> &crate::agent_view::ViewReply {
        (commands.iter())
            .find_map(|c| match c {
                Command::ViewReply(reply) => Some(reply),
                _ => None,
            })
            .expect("a reply")
    }

    fn review_state(s: &AppState) -> Option<&ReviewState> {
        task(s, 1).review.as_ref().map(|r| &r.state)
    }

    #[test]
    fn a_change_for_the_user_starts_one_review_whose_report_replaces_the_agents() {
        let (s, commands) = review_of(one_file(2), true);
        let specs = review_specs(&commands);
        assert_eq!(specs.len(), 1, "{commands:?}");
        let spec = specs[0];
        assert_eq!((spec.task, spec.run, spec.harness), (1, 0, Harness::Claude));
        assert_eq!(spec.model.as_deref(), Some("sonnet"), "a cheaper model");
        assert_eq!((spec.baseline.as_str(), spec.tree.as_str()), ("tree-1", "now-2"));
        assert_eq!(spec.dir, PathBuf::from("/repo/examples/a"));
        assert!(spec.brief.contains("> Make it spin"), "{}", spec.brief);
        assert!(spec.brief.contains("Summary: I made it spin."), "{}", spec.brief);
        assert_eq!(review_state(&s), Some(&ReviewState::Preparing));
        // While it prepares, the page stands on the agent's report.
        assert_eq!(
            s.decisions[&1].reason,
            "No check was reported and no path was compared."
        );

        // Only the review reports; its report waits for its end (the files
        // measured unchanged), then replaces the agent's.
        let (same, commands) = update(s.clone(), report_from(false));
        assert!(!view_reply(&commands).ok);
        assert_eq!(task(&same, 1).report, task(&s, 1).report);
        let (s, commands) = update(s, report_from(true));
        let reply = view_reply(&commands);
        assert!(reply.ok, "{reply:?}");
        assert!(!task(&s, 1).report.as_ref().unwrap().reviewed, "held");
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, review_ended(None));
        assert_eq!(review_state(&s), Some(&ReviewState::Prepared));
        assert!(commands.iter().any(|c| matches!(c, Command::SaveTask { .. })));
        let report = task(&s, 1).report.clone().expect("the review's");
        assert!(report.reviewed);
        assert_eq!(report.notes, Some(vec!["Nobody tried it with a gamepad.".into()]));
        assert_eq!(s.decisions[&1].reason, "The review notes 1 thing.");
        assert!(s.decisions[&1].style.is_empty());
        // Late events and reports find no review preparing.
        let (late, commands) = update(s.clone(), report_from(true));
        assert!(!view_reply(&commands).ok);
        assert_eq!(late.tasks, s.tasks);
        // It comes back from the store as it was.
        let back = crate::tasks::from_stored(&crate::tasks::to_stored(task(&s, 1))).unwrap();
        assert_eq!(back.review, task(&s, 1).review);
    }

    #[test]
    fn no_review_without_a_change_or_when_off_and_a_missing_agent_says_so() {
        // An answer: no change.
        let (s, commands) = review_of(Change::from_files(vec![], "tree-1".into()), false);
        assert!(review_specs(&commands).is_empty());
        assert_eq!(task(&s, 1).review, None);
        // Turned off: the agent's own report is the page's.
        let off = ReviewSetting {
            on: false,
            ..Default::default()
        };
        let (mut s, _) = update(
            AppState {
                can_review: true,
                ..tracked()
            },
            client(ClientMsg::SetReview {
                setting: off.clone(),
            }),
        );
        assert_eq!(s.review_settings["examples/a"], off);
        s.can_review = true;
        let (s, _) = update(s, new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert!(review_specs(&commands).is_empty());
        assert_eq!(task(&s, 1).review, None);
        // Claude not installed fails it at once, with the reason.
        let s = AppState {
            can_review: true,
            harnesses: vec![Harness::Codex],
            ..tracked()
        };
        let (s, _) = update(s, new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert!(review_specs(&commands).is_empty());
        let reason = "claude is not available on this machine".to_string();
        assert_eq!(review_state(&s), Some(&ReviewState::Failed { reason }));
        // Codex cannot be held read-only: refused as a setting, and a stored
        // one is reviewed on Claude anyway.
        let codex = ReviewSetting {
            harness: Some(Harness::Codex),
            ..Default::default()
        };
        let set = client(ClientMsg::SetReview {
            setting: codex.clone(),
        });
        let (_, commands) = update(s, set);
        assert_eq!(commands[..1], rejected(crate::review::NO_CODEX));
        let mut s = AppState {
            can_review: true,
            ..tracked()
        };
        s.review_settings.insert("examples/a".into(), codex);
        let (s, _) = update(s, new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert_eq!(review_specs(&commands)[0].harness, Harness::Claude);
        assert_eq!(review_state(&s), Some(&ReviewState::Preparing));
    }

    /// A Codex task's change is reviewed on Claude, and a check the agent
    /// reported failed stays, so the verdict still leads with it.
    #[test]
    fn a_codex_tasks_review_runs_on_claude_and_keeps_failed_checks() {
        let s = AppState {
            can_review: true,
            ..tracked()
        };
        let codex_task = Event::NewTask {
            id: Ok(1),
            message: "Make it spin".into(),
            harness: Harness::Codex,
            voice: None,
            scope: Default::default(),
        };
        let (s, _) = update(s, codex_task);
        let (mut s, _) = update(s, baseline(1, 0, "tree-1"));
        s.tasks[0].report = Some(crate::decision::Report {
            summary: Some("Done.".into()),
            notes: Some(vec![]),
            evidence: vec![crate::decision::Check {
                ran: "functor test".into(),
                result: "1 of 3 expects pass".into(),
                ok: false,
            }],
            reviewed: false,
        });
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert_eq!(review_specs(&commands)[0].harness, Harness::Claude);
        let (s, _) = update(s, report_from(true));
        let (s, _) = update(s, review_ended(None));
        let report = task(&s, 1).report.clone().unwrap();
        assert!(report.reviewed);
        assert_eq!(report.evidence[0].ran, "functor test");
        assert_eq!(s.decisions[&1].reason, "A check failed: functor test.");
    }

    #[test]
    fn a_failed_or_cut_review_leaves_the_page_as_it_was() {
        // Cut after it reported: its report goes with it.
        let (s, _) = update(review_of(one_file(2), true).0, report_from(true));
        let (s, _) = update(s, review_ended(Some("cut: over 300000 input tokens")));
        let reason = "cut: over 300000 input tokens".to_string();
        assert_eq!(review_state(&s), Some(&ReviewState::Failed { reason }));
        let report = task(&s, 1).report.as_ref().expect("the agent's stays");
        assert!(!report.reviewed);
        // One that ended without reporting says so.
        let (s, _) = update(review_of(one_file(2), false).0, review_ended(None));
        let reason = "the review ended without a report".to_string();
        assert_eq!(review_state(&s), Some(&ReviewState::Failed { reason }));
        assert_eq!(s.decisions[&1].reason, "The agent gave no report.");
        // Usage is kept; a reload marks one in flight as cut by the close.
        let (s, _) = review_of(one_file(2), false);
        let usage = AgentEvent::Usage {
            input: 41_000,
            output: 900,
        };
        let (s, _) = update(s, reviewed_event(ReviewEvent::Agent(usage)));
        let review = task(&s, 1).review.clone().unwrap();
        assert_eq!(review.usage.map(|u| u.input), Some(41_000));
        let tasks = s.tasks.clone();
        let (s, _) = update(s, Event::TasksLoaded { tasks });
        let reason = "the app closed before the review finished".to_string();
        assert_eq!(review_state(&s), Some(&ReviewState::Failed { reason }));
    }

    #[test]
    fn keep_revert_a_follow_up_or_an_edit_end_the_review() {
        // Keep works at once, mid-review, and stops it.
        let (s, _) = review_of(one_file(2), true);
        let (kept, commands) = update(s.clone(), keep(1));
        assert_eq!(task(&kept, 1).status, TaskStatus::Kept);
        assert_eq!(review_stops(&commands), 1);
        assert_eq!(task(&kept, 1).review, None);
        // So does Revert, as it starts writing.
        let (s2, commands) = update(s.clone(), revert(1));
        assert!(task(&s2, 1).reverting, "{commands:?}");
        assert_eq!(review_stops(&commands), 1);
        assert_eq!(task(&s2, 1).review, None);
        // A follow-up supersedes it.
        let (s2, commands) = update(s.clone(), send(1, "Slower"));
        assert_eq!(review_stops(&commands), 1);
        assert_eq!(task(&s2, 1).review, None);
        // Files changed while it ran (by hand, or by the review itself):
        // stopped, its report void, and it says why.
        let (s2, _) = update(s.clone(), report_from(true));
        let (s2, commands) = update(s2, change(1, "tree-1", one_file(3)));
        assert_eq!(review_stops(&commands), 1);
        assert_eq!(task(&s2, 1).report, None);
        let reason = "the files changed while the review ran".to_string();
        assert_eq!(review_state(&s2), Some(&ReviewState::Failed { reason }));
        // Late events of the stopped review change nothing.
        let (late, _) = update(s2.clone(), review_ended(None));
        assert_eq!(late.tasks, s2.tasks);
    }

    // Path evidence (D38).

    fn moment(task: u64, moment: crate::evidence::Moment) -> Event {
        Event::Run {
            task,
            run: 0,
            event: RunEvent::Moment(moment),
        }
    }

    fn taken(tree: &str, state: EvidenceState) -> Event {
        Event::Evidence {
            task: 1,
            tree: tree.to_string(),
            state,
        }
    }

    fn captures(name: &str) -> EvidenceState {
        EvidenceState::Shown {
            before: format!("{name}-before.png"),
            after: format!("{name}-after.png"),
        }
    }

    fn takes(commands: &[Command]) -> Vec<&Command> {
        commands
            .iter()
            .filter(|c| matches!(c, Command::TakeEvidence { .. }))
            .collect()
    }

    fn drops(commands: &[Command]) -> Vec<Vec<String>> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::DropEvidence { files } => Some(files.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn paths_are_taken_once_the_run_ends_for_each_measured_change() {
        let (s, commands) = update(tracked(), new_task("Make it spin"));
        assert!(spec(&commands).moment, "asked with the snapshot");
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let at = crate::evidence::Moment::Taken { frame: 5, steps: 9 };
        let (s, _) = update(s, moment(1, at.clone()));
        assert_eq!(task(&s, 1).moment, Some(at));
        let (s, _) = update(s, agent(1, 0, finished()));
        // Measured while the run edits: not yet.
        let (s, commands) = update(s, change(1, "tree-1", one_file(2)));
        assert!(takes(&commands).is_empty(), "{commands:?}");
        let (s, commands) = update(s, ended(1, 0));
        assert_eq!(
            takes(&commands),
            [&Command::TakeEvidence {
                task: 1,
                dir: PathBuf::from("/repo/examples/a"),
                project: "examples/a".to_string(),
                baseline: "tree-1".to_string(),
                tree: "now-2".to_string(),
            }]
        );
        assert_eq!(
            task(&s, 1).evidence.as_ref().map(|e| &e.state),
            Some(&EvidenceState::Taking)
        );
        // Measured again unchanged: still once.
        let (s, commands) = update(s, change(1, "tree-1", one_file(2)));
        assert!(takes(&commands).is_empty());

        // An answer for an older change is dropped, files and all.
        let (s, commands) = update(s, taken("now-1", captures("old")));
        assert_eq!(drops(&commands), [["old-before.png", "old-after.png"]]);
        assert_eq!(
            task(&s, 1).evidence.as_ref().unwrap().state,
            EvidenceState::Taking
        );
        let (s, commands) = update(s, taken("now-2", captures("a")));
        assert_eq!(task(&s, 1).evidence.as_ref().unwrap().state, captures("a"));
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::SaveTask { .. })));

        // A new measurement takes them again; the old captures go.
        let (s, commands) = update(s, change(1, "tree-1", one_file(5)));
        assert_eq!(takes(&commands).len(), 1);
        assert_eq!(drops(&commands), [["a-before.png", "a-after.png"]]);
        let (s, _) = update(s, taken("now-5", captures("b")));

        // Keeping ends the review: the moment and the captures go.
        let (s, commands) = update(s, keep(1));
        assert_eq!(drops(&commands), [["b-before.png", "b-after.png"]]);
        assert_eq!((&task(&s, 1).moment, &task(&s, 1).evidence), (&None, &None));
    }

    #[test]
    fn without_a_moment_the_page_says_why_and_without_a_gpu_none_is_asked() {
        let (s, _) = update(tracked(), new_task("Make it spin"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let reason = "the game was not running when the task started".to_string();
        let missing = crate::evidence::Moment::Missing {
            reason: reason.clone(),
        };
        let (s, _) = update(s, moment(1, missing));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let (s, commands) = update(s, ended(1, 0));
        assert!(takes(&commands).is_empty());
        assert_eq!(
            task(&s, 1).evidence.as_ref().unwrap().state,
            EvidenceState::Withheld { reason }
        );

        let no_gpu = AppState {
            no_gpu: true,
            ..tracked()
        };
        let (_, commands) = update(no_gpu, new_task("Make it spin"));
        assert!(!spec(&commands).moment);
    }

    #[test]
    fn paths_being_taken_when_the_app_stopped_say_so_on_load() {
        let s = reviewed(one_file(2));
        let mut saved = task(&s, 1).clone();
        saved.evidence = Some(Evidence {
            tree: "now-2".to_string(),
            state: EvidenceState::Taking,
        });
        let (s, _) = update(state(), Event::TasksLoaded { tasks: vec![saved] });
        assert_eq!(
            task(&s, 1).evidence.as_ref().unwrap().state,
            EvidenceState::Withheld {
                reason: "the app closed before the paths were taken".to_string()
            }
        );
    }

    #[test]
    fn a_finished_run_with_a_change_is_ready_for_review() {
        let s = reviewed(one_file(2));
        let t = task(&s, 1);
        assert_eq!(t.status, TaskStatus::ReadyForReview);
        assert_eq!(t.baseline.as_deref(), Some("tree-1"));
        assert_eq!(t.change, Some(one_file(2)));
        // No change: done, as without git.
        let s = reviewed(Change::default());
        assert_eq!(task(&s, 1).status, TaskStatus::Done);
        // A failed run with a change stays failed.
        let (s, _) = update(tracked(), new_task("x"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (s, _) = update(s, change(1, "tree-1", one_file(2)));
        let error = Event::Run {
            task: 1,
            run: 0,
            event: RunEvent::Ended {
                error: Some("broke".to_string()),
            },
        };
        let (s, _) = update(s, error);
        assert!(matches!(task(&s, 1).status, TaskStatus::Failed { .. }));
    }

    #[test]
    fn a_change_is_recorded_once_and_only_for_the_current_baseline() {
        let s = reviewed(one_file(2));
        let (next, commands) = update(s.clone(), change(1, "tree-1", one_file(5)));
        assert_eq!(task(&next, 1).change, Some(one_file(5)));
        assert!(
            matches!(commands[0], Command::SaveTask { .. }),
            "{commands:?}"
        );
        assert_eq!(commands.last(), Some(&broadcast(&next)));
        // The same again, or one from another baseline, changes nothing.
        for event in [
            change(1, "tree-1", one_file(2)),
            change(1, "tree-0", one_file(9)),
            change(7, "tree-1", one_file(9)),
        ] {
            let (again, commands) = update(s.clone(), event);
            assert_eq!((again, commands), (s.clone(), vec![]));
        }
        // An emptied change (the user undid it by hand) leaves it done.
        let (next, _) = update(s, change(1, "tree-1", Change::default()));
        assert_eq!(task(&next, 1).status, TaskStatus::Done);
    }

    #[test]
    fn keep_closes_the_review_and_a_follow_up_takes_a_new_baseline() {
        let (s, commands) = update(reviewed(one_file(2)), keep(1));
        let t = task(&s, 1);
        assert_eq!(t.status, TaskStatus::Kept);
        assert_eq!((t.baseline.as_ref(), t.change.as_ref()), (None, None));
        assert_eq!(t.transcript.last(), Some(&Item::Kept));
        let index = t.transcript.len() - 1;
        assert_eq!(
            commands,
            vec![
                Command::SaveTask {
                    dir: PathBuf::from("/repo/examples/a"),
                    task: Box::new(t.clone()),
                },
                broadcast(&s),
                Command::Broadcast(ServerMsg::Item {
                    task: 1,
                    index,
                    item: Item::Kept
                }),
            ]
        );
        // A follow-up reopens it, snapshotting afresh.
        let (s, commands) = update(s, send(1, "And faster"));
        assert!(spec(&commands).snapshot);
        assert_eq!(task(&s, 1).status, TaskStatus::InProgress);
        let run = spec(&commands).run;
        let (s, _) = update(s, baseline(1, run, "tree-2"));
        assert_eq!(task(&s, 1).baseline.as_deref(), Some("tree-2"));
        // While a baseline stands, a follow-up does not take another.
        let (s, _) = update(s, agent(1, run, finished()));
        let (s, _) = update(s, change(1, "tree-2", one_file(1)));
        let (s, _) = update(s, ended(1, run));
        assert_eq!(task(&s, 1).status, TaskStatus::ReadyForReview);
        let (_, commands) = update(s, send(1, "More"));
        assert!(!spec(&commands).snapshot);
        // Untracked, no run snapshots.
        let (_, commands) = update(state(), new_task("x"));
        assert!(!spec(&commands).snapshot);
    }

    #[test]
    fn revert_asks_the_shell_and_closes_the_review_when_done() {
        let (s, commands) = update(reviewed(one_file(2)), revert(1));
        assert!(task(&s, 1).reverting);
        assert_eq!(
            commands,
            vec![
                Command::Revert {
                    task: 1,
                    baseline: "tree-1".to_string(),
                    paths: vec!["examples/a/game.fun".to_string()],
                },
                broadcast(&s)
            ]
        );
        // Nothing else may start meanwhile.
        let (_, commands) = update(s.clone(), revert(1));
        assert_eq!(commands, rejected("task 1's change is being reverted"));
        let (_, commands) = update(s.clone(), send(1, "more"));
        assert_eq!(commands, rejected("task 1's change is being reverted"));
        // A new task waits for it.
        let (queued, _) = update(s.clone(), nth_task(2, "other"));
        assert_eq!(task(&queued, 2).status, TaskStatus::Queued);

        let (done, commands) = update(s.clone(), reverted(1, "tree-1", Ok(())));
        let t = task(&done, 1);
        assert_eq!(t.status, TaskStatus::Reverted);
        assert!(!t.reverting);
        assert_eq!((t.baseline.as_ref(), t.change.as_ref()), (None, None));
        assert_eq!(t.transcript.last(), Some(&Item::Reverted));
        assert!(
            matches!(commands[0], Command::SaveTask { .. }),
            "{commands:?}"
        );

        // A failed revert says why and measures what is left.
        let (failed, commands) = update(s, reverted(1, "tree-1", Err("gone".to_string())));
        let t = task(&failed, 1);
        assert!(!t.reverting);
        assert_eq!(t.status, TaskStatus::ReadyForReview);
        assert_eq!(
            commands,
            vec![
                Command::Broadcast(ServerMsg::Rejected {
                    reason: "cannot revert task 1's change: gone".to_string()
                }),
                Command::Diff {
                    task: 1,
                    baseline: "tree-1".to_string(),
                    reply: false
                },
                broadcast(&failed),
            ]
        );
        // A late answer for no revert in flight is dropped.
        let (again, commands) = update(failed.clone(), reverted(1, "tree-1", Ok(())));
        assert_eq!((again, commands), (failed, vec![]));
    }

    #[test]
    fn keep_and_revert_are_rejected_without_a_reviewable_change() {
        // Running: the change is not settled.
        let (running, _) = update(tracked(), new_task("x"));
        let (running, _) = update(running, baseline(1, 0, "tree-1"));
        let (running, _) = update(running, change(1, "tree-1", one_file(2)));
        let untracked = derived(AppState {
            tracks_changes: false,
            ..reviewed(one_file(2))
        });
        for (s, reason) in [
            (
                running,
                "task 1 is still running: wait for it to finish or stop it",
            ),
            (
                reviewed(Change::default()),
                "task 1 has no change to review",
            ),
            (
                update(reviewed(one_file(2)), keep(1)).0,
                "task 1 has no change to review",
            ),
            (untracked, "changes are tracked only in a git repository"),
            (tracked(), "there is no task 1"),
        ] {
            for event in [keep(1), revert(1)] {
                let (next, commands) = update(s.clone(), event);
                assert_eq!(next, s);
                assert_eq!(commands, rejected(reason));
            }
        }
        // Another task running, even in another project: an agent may edit
        // anywhere in the repo, so no revert goes on under it.
        let (s, _) = update(reviewed(one_file(2)), switch("examples/b"));
        let (s, _) = update(s, nth_task(2, "second"));
        let (next, commands) = update(s.clone(), revert(1));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            rejected("task 2 is running or reverting: wait for it before reverting")
        );
        // Keeping touches no files, so it may.
        let (next, _) = update(s, keep(1));
        assert_eq!(task(&next, 1).status, TaskStatus::Kept);
        // A change too large to list in full is not reverted blind.
        let mut large = one_file(2);
        large.more = 3;
        let (s, commands) = update(reviewed(large), revert(1));
        assert!(!task(&s, 1).reverting);
        assert_eq!(
            commands,
            rejected("task 1 changed more files than the app lists: revert them with git")
        );
        // Measured again (to `now-3`) after the user confirmed `now-2`.
        let (s, commands) = update(reviewed(one_file(3)), revert(1));
        assert!(!task(&s, 1).reverting);
        assert_eq!(
            commands,
            rejected("task 1's change has changed since you saw it: review it again")
        );
    }

    #[test]
    fn get_diff_asks_the_shell_or_answers_empty() {
        let s = reviewed(one_file(2));
        let (next, commands) = update(s.clone(), client(ClientMsg::GetDiff { task: 1 }));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            vec![Command::Diff {
                task: 1,
                baseline: "tree-1".to_string(),
                reply: true
            }]
        );
        let empty = vec![Command::Reply(ServerMsg::Diff {
            task: 1,
            tree: None,
            files: vec![],
            more: 0,
            error: None,
        })];
        let (_, commands) = update(finished_task(), client(ClientMsg::GetDiff { task: 1 }));
        assert_eq!(commands, empty);
        let (_, commands) = update(state(), client(ClientMsg::GetDiff { task: 1 }));
        assert_eq!(commands, rejected("there is no task 1"));
    }

    #[test]
    fn stopping_a_run_with_a_baseline_measures_its_change() {
        let (s, _) = update(tracked(), new_task("x"));
        let (s, _) = update(s, baseline(1, 0, "tree-1"));
        let (_, commands) = update(s, client(ClientMsg::StopTask { task: 1 }));
        assert_eq!(
            commands[1],
            Command::Diff {
                task: 1,
                baseline: "tree-1".to_string(),
                reply: false
            }
        );
    }

    #[test]
    fn a_loaded_task_is_not_left_reverting() {
        let mut saved = task(&reviewed(one_file(2)), 1).clone();
        saved.reverting = true;
        let (s, _) = update(tracked(), Event::TasksLoaded { tasks: vec![saved] });
        let t = task(&s, 1);
        assert!(!t.reverting);
        assert_eq!(t.status, TaskStatus::ReadyForReview);
    }

    #[test]
    fn change_messages_are_type_tagged() {
        for (json, msg) in [
            (
                r#"{"type":"get_diff","task":1}"#,
                ClientMsg::GetDiff { task: 1 },
            ),
            (
                r#"{"type":"keep_change","task":1}"#,
                ClientMsg::KeepChange { task: 1 },
            ),
            (
                r#"{"type":"revert_change","task":1,"tree":"t"}"#,
                ClientMsg::RevertChange {
                    task: 1,
                    tree: "t".to_string(),
                },
            ),
        ] {
            assert_eq!(serde_json::from_str::<ClientMsg>(json).unwrap(), msg);
        }
        let json = serde_json::to_value(ServerMsg::State {
            state: Box::new(reviewed(one_file(2))),
        })
        .unwrap();
        let t = &json["state"]["tasks"][0];
        assert_eq!(t["status"]["kind"], "ready_for_review");
        assert_eq!(t["change"]["files"][0]["path"], "examples/a/game.fun");
        assert_eq!(t["change"]["files"][0]["added"], 2);
    }

    // Layouts.

    use crate::layout::{Direction, Node, Pane, Target};

    fn op(op: LayoutOp) -> Event {
        client(ClientMsg::Layout { op })
    }

    fn open_file(path: &str, target: Target) -> Event {
        op(LayoutOp::Open {
            view: View::File {
                path: path.to_string(),
            },
            target,
        })
    }

    fn save_layout_cmd(project: &str, layout: &Layout) -> Command {
        Command::SaveLayout {
            dir: PathBuf::from(format!("/repo/{project}")),
            layout: Box::new(layout.clone()),
        }
    }

    #[test]
    fn a_new_state_gives_every_project_the_initial_layout() {
        let s = AppState::new(
            Path::new("/repo"),
            Path::new("/repo"),
            vec![project("examples/a"), project("examples/b")],
        );
        assert_eq!(s.layouts.len(), 2);
        assert!(s.layouts.values().all(|l| *l == Layout::initial()));
    }

    #[test]
    fn a_layout_op_changes_the_active_projects_layout_saves_and_broadcasts() {
        let (next, commands) = update(state(), open_file("game.fun", Target::Side));
        let layout = &next.layouts["examples/a"];
        assert_eq!(
            crate::print_layout(layout),
            "right 50% (#1[*game] | >#3[*file:game.fun])"
        );
        assert_eq!(
            commands,
            vec![save_layout_cmd("examples/a", layout), broadcast(&next)]
        );
        // The other project's layout is its own.
        let (b, _) = update(next.clone(), switch("examples/b"));
        assert_eq!(b.layouts.get("examples/b"), None);
        assert_eq!(b.layouts["examples/a"], *layout);
        // An op that changes nothing only replies.
        let (again, commands) = update(next.clone(), open_file("game.fun", Target::Side));
        assert_eq!(again, next);
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(next)
            })]
        );
    }

    #[test]
    fn a_view_that_cannot_open_is_rejected() {
        let other = Task {
            project: "examples/b".to_string(),
            ..task(&finished_task(), 1).clone()
        };
        let s = derived(AppState {
            tasks: vec![other],
            ..state()
        });
        for (event, reason) in [
            (
                open_file("missing.fun", Target::Focused),
                "missing.fun is not one of examples/a's files",
            ),
            (
                open_file("../x.fun", Target::Focused),
                "\"../x.fun\" is not a path inside the project",
            ),
            (
                op(LayoutOp::Open {
                    view: View::Changes { task: 1 },
                    target: Target::Side,
                }),
                "there is no task 1 in examples/a",
            ),
        ] {
            let (next, commands) = update(s.clone(), event);
            assert_eq!(next, s);
            assert_eq!(
                commands,
                vec![
                    Command::Reply(ServerMsg::Rejected {
                        reason: reason.to_string()
                    }),
                    Command::Reply(ServerMsg::State {
                        state: Box::new(s.clone())
                    })
                ]
            );
        }
        // A task of the project opens.
        let (next, _) = update(
            finished_task(),
            op(LayoutOp::Open {
                view: View::Task { task: 1 },
                target: Target::Focused,
            }),
        );
        assert_eq!(
            crate::print_layout(&next.layouts["examples/a"]),
            ">#1[game, *task:1]"
        );
    }

    #[test]
    fn loaded_layouts_are_normalized_and_lose_stale_tabs() {
        let s = finished_task();
        let pane = |id, tabs: Vec<View>| {
            Node::Pane(Pane {
                id,
                tabs,
                active: 0,
            })
        };
        let file = |p: &str| View::File {
            path: p.to_string(),
        };
        let saved = Layout {
            root: Some(Node::Split {
                id: 3,
                direction: Direction::Right,
                ratio: 500,
                // A second Game tab, a gone task, a file no longer there.
                first: Box::new(pane(
                    1,
                    vec![
                        View::Game,
                        View::Changes { task: 1 },
                        View::Task { task: 9 },
                    ],
                )),
                second: Box::new(pane(
                    2,
                    vec![View::Game, file("gone.fun"), file("game.fun")],
                )),
            }),
            focus: Some(2),
            game: Default::default(),
            agent: Default::default(),
        };
        // Another project's files are not listed yet, so its file tabs stay.
        let theirs = Layout {
            root: Some(pane(1, vec![file("b.fun")])),
            focus: Some(1),
            game: Default::default(),
            agent: Default::default(),
        };
        let layouts = BTreeMap::from([
            ("examples/a".to_string(), saved),
            ("examples/b".to_string(), theirs.clone()),
            ("examples/gone".to_string(), Layout::initial()),
        ]);
        let (next, _) = update(s, Event::LayoutsLoaded { layouts });
        assert_eq!(
            crate::print_layout(&next.layouts["examples/a"]),
            "right 50% (#1[*game, changes:1] | >#2[*file:game.fun])"
        );
        assert_eq!(next.layouts["examples/b"], theirs);
        assert!(!next.layouts.contains_key("examples/gone"));
        // Once b is active and listed without b.fun, its tab closes and is saved.
        let (b, _) = update(next, switch("examples/b"));
        let (b, commands) = update(b, files("examples/b", &["c.fun"]));
        assert_eq!(
            b.layouts["examples/b"],
            Layout {
                root: None,
                focus: None,
                game: Default::default(),
                agent: Default::default(),
            }
        );
        assert_eq!(
            commands,
            vec![
                save_layout_cmd("examples/b", &b.layouts["examples/b"]),
                broadcast(&b)
            ]
        );
    }

    #[test]
    fn get_file_asks_the_shell_for_a_well_formed_path() {
        let (next, commands) = update(
            state(),
            client(ClientMsg::GetFile {
                path: "game.fun".into(),
            }),
        );
        assert_eq!(next, state());
        assert_eq!(
            commands,
            vec![Command::ReadFile {
                root: "/repo".to_string(),
                project: "examples/a".to_string(),
                path: "game.fun".to_string(),
                dir: PathBuf::from("/repo/examples/a"),
            }]
        );
        let (_, commands) = update(
            state(),
            client(ClientMsg::GetFile {
                path: "/etc/passwd".into(),
            }),
        );
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::File {
                root: "/repo".to_string(),
                project: "examples/a".to_string(),
                path: "/etc/passwd".to_string(),
                text: None,
                error: Some("\"/etc/passwd\" is not a path inside the project".to_string()),
            })]
        );
    }

    #[test]
    fn get_file_reads_only_listed_files() {
        let (_, commands) = update(
            state(),
            client(ClientMsg::GetFile {
                path: ".env".into(),
            }),
        );
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::File {
                root: "/repo".to_string(),
                project: "examples/a".to_string(),
                path: ".env".to_string(),
                text: None,
                error: Some(".env is not one of examples/a's files".to_string()),
            })]
        );
    }

    #[test]
    fn an_empty_listing_closes_no_file_tabs_and_open_tabs_still_show() {
        let (s, _) = update(state(), open_file("game.fun", Target::Side));
        let (s, _) = update(s, op(LayoutOp::Focus { pane: 1 }));
        let before = s.layouts["examples/a"].clone();
        // An unreadable directory lists nothing: the tab stays.
        let (s, _) = update(s, files("examples/a", &[]));
        assert_eq!(s.layouts["examples/a"], before);
        // Showing the open tab needs no listing.
        let (s, _) = update(s, open_file("game.fun", Target::Focused));
        assert_eq!(
            crate::print_layout(&s.layouts["examples/a"]),
            "right 50% (#1[*game] | >#3[*file:game.fun])"
        );
    }

    #[test]
    fn layout_messages_are_type_tagged() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"layout","op":{"type":"move","view":{"kind":"game"},"pane":2,"index":0}}"#,
        )
        .unwrap();
        assert_eq!(
            msg,
            ClientMsg::Layout {
                op: LayoutOp::Move {
                    view: View::Game,
                    pane: 2,
                    index: 0
                }
            }
        );
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"get_file","path":"game.fun"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::GetFile {
                path: "game.fun".into()
            }
        );
        let json = serde_json::to_value(ServerMsg::State {
            state: Box::new(AppState::new(
                Path::new("/r"),
                Path::new("/r"),
                vec![project("a")],
            )),
        })
        .unwrap();
        assert_eq!(
            json["state"]["layouts"]["a"]["root"]["tabs"][0]["kind"],
            "game"
        );
    }

    // Opening folders.

    fn open(path: &str) -> Event {
        client(ClientMsg::OpenFolder {
            path: path.to_string(),
        })
    }

    /// The workspace the shell reads for `/other`: one project at its root,
    /// with a saved task, layout and agent pick.
    fn other() -> AppState {
        let mut new = AppState::new(Path::new("/other"), Path::new("/other"), vec![project(".")]);
        new.files = vec!["functor.json".into()];
        new.tracks_changes = true;
        new.tasks = vec![Task {
            project: ".".into(),
            ..task(&finished_task(), 1).clone()
        }];
        new.harness_choice.insert(".".into(), Harness::Codex);
        new
    }

    #[test]
    fn opening_a_folder_asks_the_shell_to_read_it() {
        let (next, commands) = update(state(), open("/other"));
        assert_eq!(
            next,
            AppState {
                opening: 1,
                ..state()
            }
        );
        assert_eq!(
            commands,
            vec![Command::OpenFolder {
                id: 1,
                path: "/other".into()
            }]
        );
        // The picker's choice opens the same way; a cancel does nothing.
        let picked = Event::Picked {
            parent: false,
            path: Some("/other".into()),
        };
        assert_eq!(update(state(), picked).1, commands);
        assert_eq!(
            update(
                state(),
                Event::Picked {
                    path: None,
                    parent: false
                }
            )
            .1,
            vec![]
        );
    }

    #[test]
    fn no_folder_opens_while_a_run_or_revert_is_active() {
        let (running, _) = update(state(), new_task("hi"));
        let reason = "task 1 is running: stop it before opening another folder";
        let refused = AppState {
            open_error: Some(reason.into()),
            ..running.clone()
        };
        let (next, commands) = update(running.clone(), open("/other"));
        assert_eq!(next, refused);
        assert_eq!(commands, vec![broadcast(&refused)]);
        // Nor when a run started while the folder was read.
        let read = Event::Opened {
            id: 0,
            folder: "/other".into(),
            result: Ok(Box::new(other())),
        };
        let (next, commands) = update(running.clone(), read);
        assert_eq!(next, refused);
        assert_eq!(commands, vec![broadcast(&refused)]);
        let mut reverting = finished_task();
        reverting.tasks[0].reverting = true;
        let (next, _) = update(reverting, open("/other"));
        assert_eq!(next.open_error.as_deref(), Some(reason));
        // Stopped, it opens, and the reason is gone.
        let (stopped, _) = update(refused, client(ClientMsg::StopTask { task: 1 }));
        let (next, commands) = update(stopped, open("/other"));
        assert_eq!(next.open_error, None);
        assert!(matches!(
            commands[..],
            [Command::OpenFolder { .. }, Command::Broadcast(_)]
        ));
    }

    #[test]
    fn an_opened_folder_replaces_the_workspace_and_stops_the_session() {
        let mut before = with_session(1, Some(9), running(Some(12)));
        before.no_gpu = true;
        before.recents = vec![Recent {
            name: "a".into(),
            path: "/repo/examples/a".into(),
            missing: false,
        }];
        let read = Event::Opened {
            id: 0,
            folder: "/other/".into(),
            result: Ok(Box::new(other())),
        };
        let (next, commands) = update(before.clone(), read);
        let expected = AppState {
            session: Some(Session {
                id: 1,
                pid: None,
                status: SessionStatus::Stopped,
            }),
            no_gpu: true,
            auto_start: true,
            harnesses: before.harnesses.clone(),
            recents: vec![
                Recent {
                    name: "other".into(),
                    path: native("/other"),
                    missing: false,
                },
                before.recents[0].clone(),
            ],
            ..other()
        };
        assert_eq!(next, derived(expected.clone()));
        assert_eq!(
            commands,
            vec![
                Command::StopSession { id: 1 },
                Command::UseWorkspace { id: 0 },
                Command::SaveRecents {
                    recents: expected.recents.clone()
                },
                broadcast(&next),
                transport(None),
            ]
        );
        // The next session is the new project's, with a new id.
        let (_, commands) = update(next, client(ClientMsg::StartSession));
        assert_eq!(
            commands[0],
            Command::StartSession {
                id: 2,
                dir: PathBuf::from("/other"),
                display: Display::Headless,
            }
        );
    }

    #[test]
    fn in_the_app_opening_or_switching_starts_the_game_unless_it_has_its_own_window() {
        let starts = |commands: &[Command]| {
            commands
                .iter()
                .filter_map(|c| match c {
                    Command::StartSession { dir, display, .. } => Some((dir.clone(), *display)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let read = |placement| {
            let mut other = other();
            other.layouts.get_mut(".").unwrap().game.placement = placement;
            Event::Opened {
                id: 0,
                folder: "/other".into(),
                result: Ok(Box::new(other)),
            }
        };
        let mut app = state();
        app.start_on_open = true;
        for (placement, expected) in [
            (Placement::Pane, vec![(PathBuf::from("/other"), Display::Hidden)]),
            (Placement::Mini, vec![(PathBuf::from("/other"), Display::Hidden)]),
            (Placement::Window, vec![]),
        ] {
            let (next, commands) = update(app.clone(), read(placement));
            assert_eq!(starts(&commands), expected, "{placement:?}");
            assert_eq!(next.session.is_some(), !expected.is_empty());
        }
        // `--print` and `task` start none.
        assert_eq!(starts(&update(state(), read(Placement::Pane)).1), vec![]);
        // Switching starts the new project's, though none was live.
        let (next, commands) = update(app, switch("examples/b"));
        assert_eq!(
            starts(&commands),
            vec![(PathBuf::from("/repo/examples/b"), Display::Hidden)]
        );
        assert_eq!(next.session.unwrap().status, SessionStatus::Starting);
        // Nor to a project in its own window, though one was live.
        let mut live = with_session(1, Some(9), running(Some(12)));
        live.start_on_open = true;
        let mut window = Layout::initial();
        window.game.placement = Placement::Window;
        live.layouts.insert("examples/b".into(), window);
        assert_eq!(starts(&update(live, switch("examples/b")).1), vec![]);
    }

    #[test]
    fn a_folder_that_cannot_be_opened_says_why_and_marks_its_recent_missing() {
        let mut before = state();
        before.recents = vec![Recent {
            name: "gone".into(),
            path: "/gone".into(),
            missing: false,
        }];
        let failed = |folder: &str| Event::Opened {
            id: 0,
            folder: folder.into(),
            result: Err("it does not exist".into()),
        };
        let (next, commands) = update(before.clone(), failed("/gone"));
        assert!(next.recents[0].missing);
        assert_eq!(
            next.open_error.as_deref(),
            Some("cannot open /gone: it does not exist")
        );
        assert_eq!(
            commands,
            vec![
                Command::SaveRecents {
                    recents: next.recents.clone()
                },
                broadcast(&next),
            ]
        );
        // Not a recent: only the reason.
        let (other, commands) = update(before.clone(), failed("/x"));
        assert_eq!(other.recents, before.recents);
        assert_eq!(
            other.open_error.as_deref(),
            Some("cannot open /x: it does not exist")
        );
        assert_eq!(commands, vec![broadcast(&other)]);
        // A recent can be removed.
        let (removed, commands) = update(
            next,
            client(ClientMsg::RemoveRecent {
                path: "/gone".into(),
            }),
        );
        assert!(removed.recents.is_empty());
        assert_eq!(
            commands,
            vec![
                Command::SaveRecents { recents: vec![] },
                broadcast(&removed)
            ]
        );
    }

    #[test]
    fn only_the_last_folder_asked_for_opens() {
        let (s, _) = update(state(), open("/first"));
        let (s, commands) = update(s, open("/other"));
        assert_eq!(
            commands,
            vec![Command::OpenFolder {
                id: 2,
                path: "/other".into()
            }]
        );
        // The first one's reading, arriving late, is dropped.
        let first = |id| Event::Opened {
            id,
            folder: "/first".into(),
            result: Ok(Box::new(AppState::new(
                Path::new("/first"),
                Path::new("/first"),
                vec![],
            ))),
        };
        let (s, commands) = update(s.clone(), first(1));
        assert_eq!((s.repo_root.as_str(), commands), ("/repo", vec![]));
        let (s, commands) = update(
            s,
            Event::Opened {
                id: 2,
                folder: "/other".into(),
                result: Ok(Box::new(other())),
            },
        );
        assert_eq!(s.repo_root, "/other");
        assert!(commands.contains(&Command::UseWorkspace { id: 2 }));
    }

    #[test]
    fn the_native_picker_opens_only_where_there_is_one() {
        let (_, commands) = update(state(), client(ClientMsg::PickFolder));
        assert_eq!(
            commands,
            rejected("there is no folder picker here: type the folder's path")
        );
        let window = AppState {
            native_picker: true,
            folder: "/repo/examples/a".into(),
            ..state()
        };
        let (_, commands) = update(window.clone(), client(ClientMsg::PickFolder));
        assert_eq!(
            commands,
            vec![Command::PickFolder {
                from: Some(PathBuf::from("/repo/examples/a")),
                parent: false,
            }]
        );
        let (busy, _) = update(window, new_task("hi"));
        let (next, commands) = update(busy, client(ClientMsg::PickFolder));
        assert_eq!(
            next.open_error.as_deref(),
            Some("task 1 is running: stop it before opening another folder")
        );
        assert_eq!(commands, vec![broadcast(&next)]);
    }

    #[test]
    fn a_listing_from_another_workspace_is_dropped() {
        let stale = Event::Files {
            path: "examples/a".into(),
            dir: PathBuf::from("/old/examples/a"),
            files: vec!["x.fun".into()],
        };
        let (next, commands) = update(state(), stale);
        assert_eq!(next, state());
        assert!(commands.is_empty());
    }

    #[test]
    fn with_no_folder_open_nothing_is_active() {
        let none = AppState::none();
        assert_eq!((none.repo_root.as_str(), none.folder.as_str()), ("", ""));
        assert_eq!(none.active, None);
        let (_, commands) = update(none, client(ClientMsg::StartSession));
        assert!(matches!(commands[..], [Command::Reply(_)]));
    }

    // New projects.

    use crate::creation::TemplateKind;

    fn templates() -> Vec<Template> {
        ["3d", "tetris"]
            .map(|name| Template {
                name: name.into(),
                description: "d".into(),
                tags: vec![],
                kind: TemplateKind::Starter,
            })
            .to_vec()
    }

    /// `state()` with the folder open at `/repo/examples/a` and templates listed.
    fn listed() -> AppState {
        AppState {
            folder: "/repo/examples/a".into(),
            templates: templates(),
            ..state()
        }
    }

    fn ask(name: &str, describe: &str) -> NewProjectRequest {
        NewProjectRequest {
            name: name.into(),
            parent: "/games".into(),
            template: "3d".into(),
            git: true,
            describe: describe.into(),
        }
    }

    fn show(here: bool) -> Event {
        client(ClientMsg::ShowNewProject { here })
    }

    fn create(name: &str, describe: &str) -> Event {
        client(ClientMsg::CreateProject {
            request: ask(name, describe),
            harness: Harness::Codex,
        })
    }

    fn step(state: &AppState) -> &Step {
        &state.new_project.as_ref().unwrap().step
    }

    #[test]
    fn the_modal_opens_beside_the_workspace_or_for_the_open_folder() {
        let (s, commands) = update(listed(), show(false));
        let np = s.new_project.clone().unwrap();
        assert_eq!((np.name.as_str(), np.parent.as_str()), ("", "/"));
        assert_eq!(np.step, Step::Editing);
        assert_eq!(commands, vec![Command::ListTemplates, broadcast(&s)]);
        let (s, _) = update(listed(), show(true));
        let np = s.new_project.unwrap();
        assert_eq!(
            (np.name.as_str(), np.parent.as_str()),
            ("a", "/repo/examples")
        );
        // No folder open: the home folder.
        let (s, _) = update(AppState::none(), show(false));
        assert_eq!(s.new_project.unwrap().parent, "~");
        // After a project was made, where it went.
        let (s, _) = update(
            AppState {
                last_parent: Some("/games".into()),
                ..listed()
            },
            show(false),
        );
        assert_eq!(s.new_project.unwrap().parent, "/games");
    }

    #[test]
    fn a_project_is_created_opened_started_and_given_its_task() {
        let (s, _) = update(listed(), show(false));
        let (s, commands) = update(s, create("Space Race", "a racing game"));
        assert_eq!(
            step(&s),
            &Step::Creating {
                path: "/games/Space-Race".into(),
                task: Some(("a racing game".into(), Harness::Codex)),
            }
        );
        assert_eq!(
            commands,
            vec![
                Command::CreateProject {
                    request: ask("Space Race", "a racing game"),
                    folder: "Space-Race".into(),
                },
                broadcast(&s),
            ]
        );
        assert_eq!(s.last_parent.as_deref(), Some("/games"));
        // A second create while this one runs is refused.
        let (_, commands) = update(s.clone(), create("Other", ""));
        assert_eq!(commands, rejected("the project is being created"));
        let (_, commands) = update(s.clone(), client(ClientMsg::CloseNewProject));
        assert_eq!(commands, rejected("the project is being created"));

        let made = Event::Created {
            result: Ok("/games/Space-Race".into()),
            note: Some("git is not installed".into()),
        };
        let (s, commands) = update(s, made);
        assert!(matches!(step(&s), Step::Opening { id: 1, .. }));
        assert_eq!(
            commands,
            vec![
                Command::OpenFolder {
                    id: 1,
                    path: "/games/Space-Race".into()
                },
                broadcast(&s),
            ]
        );
        let new = AppState::new(
            Path::new("/games/Space-Race"),
            Path::new("/games/Space-Race"),
            vec![project(".")],
        );
        let opened = Event::Opened {
            id: 1,
            folder: "/games/Space-Race".into(),
            result: Ok(Box::new(new)),
        };
        let (s, commands) = update(s, opened);
        assert_eq!(s.new_project, None);
        assert_eq!(s.repo_root, "/games/Space-Race");
        // The creation's note is said once it is open.
        assert_eq!(s.open_error.as_deref(), Some("git is not installed"));
        assert!(commands.contains(&Command::StartSession {
            id: 1,
            dir: PathBuf::from("/games/Space-Race"),
            display: Display::Hidden,
        }));
        // The task waits for the game, so the agent's tools attach to it.
        let reserve = Command::ReserveTask {
            message: "a racing game".into(),
            harness: Harness::Codex,
            voice: None,
        };
        assert!(!commands.contains(&reserve));
        let (s, commands) = update(s, session(1, SessionEvent::Spawned { pid: 7 }));
        assert!(!commands.contains(&reserve));
        let listening = SessionEvent::Listening {
            debug_url: URL.into(),
        };
        let (s, commands) = update(s, session(1, listening));
        assert!(commands.contains(&reserve));
        assert_eq!(s.first_task, None);
        // The reserved id starts the task as the chat's would.
        let (s, _) = update(s, nth_task(5, "a racing game"));
        assert_eq!(task(&s, 5).project, ".");
    }

    #[test]
    fn creation_that_cannot_start_says_why_in_the_modal() {
        let failed = |reason: &str| Step::Failed {
            reason: reason.into(),
            done: vec![],
            not_done: vec![],
        };
        let (s, _) = update(listed(), show(false));
        let (next, _) = update(s.clone(), create("a/b", ""));
        assert!(matches!(step(&next), Step::Failed { reason, .. } if reason.starts_with("'/'")));
        let mut unknown = s.clone();
        unknown.templates.clear();
        unknown.templates_error = Some("functor cannot list templates".into());
        let (next, _) = update(unknown, create("ok", ""));
        assert_eq!(step(&next), &failed("functor cannot list templates"));
        let mut other = s.clone();
        other.templates.remove(0);
        let (next, _) = update(other, create("ok", ""));
        assert_eq!(step(&next), &failed("there is no template \"3d\""));
        let (busy, _) = update(s, new_task("hi"));
        let (next, commands) = update(busy, create("ok", ""));
        assert_eq!(
            step(&next),
            &failed("task 1 is running: stop it before opening another folder")
        );
        assert!(!commands
            .iter()
            .any(|c| matches!(c, Command::CreateProject { .. })));
        // Closed, there is nothing to create in.
        let (_, commands) = update(listed(), create("ok", ""));
        assert_eq!(commands, rejected("the new-project window is not open"));
    }

    #[test]
    fn failures_after_starting_say_what_was_and_was_not_done() {
        let (s, _) = update(listed(), show(false));
        let (creating, _) = update(s, create("Space", "describe"));
        let failure = CreateFailure {
            reason: "functor init failed".into(),
            done: vec!["Created /games/Space".into()],
            not_done: vec!["Add the 3d template's files".into()],
        };
        let (next, _) = update(
            creating.clone(),
            Event::Created {
                result: Err(failure.clone()),
                note: None,
            },
        );
        assert_eq!(
            step(&next),
            &Step::Failed {
                reason: failure.reason,
                done: failure.done,
                not_done: failure.not_done,
            }
        );
        // Created, but it does not open.
        let (opening, _) = update(
            creating,
            Event::Created {
                result: Ok("/games/Space".into()),
                note: Some("git is not installed".into()),
            },
        );
        let (next, commands) = update(
            opening,
            Event::Opened {
                id: 1,
                folder: "/games/Space".into(),
                result: Err("it does not exist".into()),
            },
        );
        assert_eq!(
            step(&next),
            &Step::Failed {
                reason: "cannot open /games/Space: it does not exist".into(),
                done: vec!["Created /games/Space, with the template's files".into()],
                not_done: vec![
                    "Open it and start the game".into(),
                    "Start a task with your description".into()
                ],
            }
        );
        assert!(!commands
            .iter()
            .any(|c| matches!(c, Command::StartSession { .. })));
        // Then it can be closed.
        let (closed, _) = update(next, client(ClientMsg::CloseNewProject));
        assert_eq!(closed.new_project, None);
    }

    #[test]
    fn nothing_else_opens_while_a_project_is_being_created() {
        let (s, _) = update(listed(), show(false));
        let (s, _) = update(s, create("Space", ""));
        let reason = Some("a new project is being created".to_string());
        let (next, commands) = update(s.clone(), open("/other"));
        assert_eq!(next.open_error, reason);
        assert!(!commands
            .iter()
            .any(|c| matches!(c, Command::OpenFolder { .. })));
        let (next, _) = update(
            AppState {
                native_picker: true,
                ..s
            },
            client(ClientMsg::PickFolder),
        );
        assert_eq!(next.open_error, reason);
        // Its own open goes ahead.
        let made = Event::Created {
            result: Ok("/games/Space".into()),
            note: None,
        };
        let (_, commands) = update(next, made);
        assert!(commands.contains(&Command::OpenFolder {
            id: 1,
            path: "/games/Space".into()
        }));
    }

    #[test]
    fn here_is_the_open_folder_itself_or_a_new_folder_in_it() {
        let named = |folder: &str| {
            let s = AppState {
                folder: folder.into(),
                ..listed()
            };
            let np = update(s, show(true)).0.new_project.unwrap();
            (np.name, np.parent)
        };
        assert_eq!(named("/games/space"), ("space".into(), "/games".into()));
        // Not a name a project folder could have as it is: a folder in it.
        assert_eq!(
            named("/games/my game"),
            ("".into(), "/games/my game".into())
        );
        // Already open: the modal keeps what was typed.
        let (s, _) = update(listed(), show(false));
        let (_, commands) = update(s, show(true));
        assert!(matches!(commands[..], [Command::Reply(_)]));
    }

    #[test]
    fn a_picked_parent_fills_the_modal() {
        let window = AppState {
            native_picker: true,
            ..listed()
        };
        let (s, _) = update(window, show(false));
        let (_, commands) = update(s.clone(), client(ClientMsg::PickParent));
        assert_eq!(
            commands,
            vec![Command::PickFolder {
                // `/` is not absolute on Windows: the picker starts anywhere.
                from: Some(PathBuf::from("/")).filter(|p| p.is_absolute()),
                parent: true
            }]
        );
        let picked = Event::Picked {
            path: Some("/games".into()),
            parent: true,
        };
        let (s, _) = update(s, picked);
        assert_eq!(s.new_project.unwrap().parent, "/games");
        let (_, commands) = update(listed(), client(ClientMsg::PickParent));
        assert_eq!(commands, rejected("there is no folder picker here"));
    }

    #[test]
    fn pairing_and_forgetting_go_through_the_link() {
        use crate::devices::{Headset, LinkCommand, LinkEvent, Paired};
        let link = |e| Event::Link(e);
        let (s, commands) = update(
            state(),
            link(LinkEvent::PairRequest {
                request: 4,
                device: "ab".into(),
                name: "Quest 3".into(),
                app_version: "0.1.0".into(),
                code: "123456".into(),
                expires_at: 1_000,
            }),
        );
        // Broadcast: every page shows the prompt.
        assert!(matches!(commands[..], [Command::Broadcast(_)]));
        assert_eq!(s.devices.headset, Headset::AskingToPair);
        let (_, commands) = update(s.clone(), client(ClientMsg::AllowPairing { request: 4 }));
        assert!(commands.contains(&Command::Link(LinkCommand::Allow { request: 4 })));
        let (_, commands) = update(s.clone(), client(ClientMsg::AllowPairing { request: 5 }));
        assert_eq!(commands, rejected("that pairing request has ended"));
        // Expired by the core's wall clock.
        let late = AppState {
            now: 1_000,
            ..s.clone()
        };
        let (_, commands) = update(late, client(ClientMsg::AllowPairing { request: 4 }));
        assert_eq!(commands, rejected("that pairing request has ended"));
        let (denied, commands) = update(s.clone(), client(ClientMsg::DenyPairing { request: 4 }));
        assert_eq!(commands[0], Command::Link(LinkCommand::Deny { request: 4 }));
        assert!(denied.devices.list.is_empty());
        let device = Paired {
            id: "ab".into(),
            name: "Quest 3".into(),
            first_seen: 1,
            last_seen: 1,
        };
        let (s, _) = update(s, link(LinkEvent::Paired { request: 4, device }));
        assert_eq!(s.devices.headset, Headset::Away);
        let (s, commands) = update(
            s,
            client(ClientMsg::ForgetDevice {
                device: "ab".into(),
            }),
        );
        assert_eq!(
            commands[0],
            Command::Link(LinkCommand::Forget {
                device: "ab".into()
            })
        );
        let (s, _) = update(
            s,
            link(LinkEvent::Forgotten {
                device: "ab".into(),
            }),
        );
        assert_eq!(s.devices.headset, Headset::NotPaired);
    }

    /// `s` with headsets `ids` paired, their tunnels open at
    /// `http://127.0.0.1:600<i>` and those in `linked` connected.
    fn with_headsets(s: AppState, ids: &[&str], linked: &[&str]) -> AppState {
        use crate::devices::{LinkEvent, LinkKind, Paired};
        let paired = ids
            .iter()
            .map(|id| Paired {
                id: id.to_string(),
                name: format!("Quest {id}"),
                first_seen: 1,
                last_seen: 1,
            })
            .collect();
        let listening = Some("0.0.0.0:7000".to_string());
        let (mut s, _) = update(s, Event::Link(LinkEvent::Started { listening, paired }));
        for (i, id) in ids.iter().enumerate() {
            let url = format!("http://127.0.0.1:600{i}");
            let device = id.to_string();
            s = update(s, Event::Link(LinkEvent::TunnelOpened { device, url })).0;
        }
        for (conn, id) in linked.iter().enumerate() {
            let connected = LinkEvent::Connected {
                conn: conn as u64,
                device: id.to_string(),
                name: format!("Quest {id}"),
                app_version: "0.0.0".into(),
                link: LinkKind::Wifi,
            };
            s = update(s, Event::Link(connected)).0;
        }
        s
    }

    fn on_headset(device: &str) -> Event {
        client(ClientMsg::Layout {
            op: LayoutOp::PlaceOnHeadset {
                device: device.into(),
            },
        })
    }

    fn start_headset(id: u64, tunnel: &str) -> Command {
        Command::StartHeadset {
            id,
            dir: PathBuf::from("/repo/examples/a"),
            tunnel: tunnel.into(),
        }
    }

    /// Headset is a choice only for a connected headset, named; choosing it
    /// moves a live game there (D5: the desktop session ends), keeps the
    /// Game tab, and choosing Pane brings it back here, paused on frame 0.
    #[test]
    fn the_game_moves_to_a_connected_headset_and_back() {
        let live = with_headsets(
            with_session(1, Some(42), running(Some(3))),
            &["aa", "bb"],
            &["aa"],
        );
        let place = |placement| {
            client(ClientMsg::Layout {
                op: LayoutOp::Place { placement },
            })
        };
        let (_, commands) = update(live.clone(), place(Placement::Headset));
        assert_eq!(
            commands[0],
            rejected("choose which headset to run the game on")[0]
        );
        for away in ["bb", "cc"] {
            let (_, commands) = update(live.clone(), on_headset(away));
            assert_eq!(
                commands[0],
                rejected("that headset is not connected")[0],
                "{away}"
            );
        }

        let (there, commands) = update(live, on_headset("aa"));
        let game = &there.layouts["examples/a"].game;
        assert_eq!(
            (game.placement, game.headset.as_deref()),
            (Placement::Headset, Some("aa"))
        );
        assert!(
            there.layouts["examples/a"].find(&View::Game).is_some(),
            "the tab stays"
        );
        let starts: Vec<_> = commands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Command::StopSession { .. } | Command::StartHeadset { .. }
                )
            })
            .collect();
        assert_eq!(
            starts,
            [
                &Command::StopSession { id: 1 },
                &start_headset(2, "http://127.0.0.1:6000")
            ]
        );
        assert_eq!(
            there.session.as_ref().unwrap().status,
            SessionStatus::Starting
        );
        assert_eq!(there.headset_waits, None);

        // Its run prints the tunnel as its debug server: polled there.
        let listening = SessionEvent::Listening {
            debug_url: "http://127.0.0.1:6000".into(),
        };
        let (there, _) = update(there, session(2, listening));
        assert!(matches!(
            &there.session.as_ref().unwrap().status,
            SessionStatus::Running { debug_url, .. } if debug_url == "http://127.0.0.1:6000"
        ));

        let (back, commands) = update(there, place(Placement::Pane));
        assert_eq!(back.layouts["examples/a"].game.headset, None);
        let starts: Vec<_> = commands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Command::StopSession { .. } | Command::StartSession { .. }
                )
            })
            .collect();
        assert_eq!(
            starts,
            [
                &Command::StopSession { id: 2 },
                &start_cmd(3, "examples/a", Display::Hidden)
            ]
        );
        assert_eq!(Display::Hidden.flags(), ["--hidden", "--paused"]);
    }

    /// With two connected, each is a choice; moving between them restarts.
    #[test]
    fn another_headset_runs_it_afresh() {
        let s = with_headsets(
            with_session(1, Some(42), running(Some(3))),
            &["aa", "bb"],
            &["aa", "bb"],
        );
        let (s, _) = update(s, on_headset("aa"));
        let (s, commands) = update(s, on_headset("bb"));
        assert_eq!(s.layouts["examples/a"].game.headset.as_deref(), Some("bb"));
        assert!(commands.contains(&start_headset(3, "http://127.0.0.1:6001")));
        // The same one again changes nothing.
        let (_, commands) = update(s, on_headset("bb"));
        assert!(!commands
            .iter()
            .any(|c| matches!(c, Command::StartHeadset { .. })));
    }

    /// A project placed on a headset starts there once its tunnel is known,
    /// whether or not it is linked (the run waits for it); one no longer
    /// paired fails, saying how to run it here.
    #[test]
    fn a_saved_headset_placement_waits_for_its_tunnel() {
        let placed = |headset: &str| {
            let mut s = state();
            let layout = s
                .layouts
                .entry("examples/a".into())
                .or_insert_with(Layout::initial);
            layout.game.placement = Placement::Headset;
            layout.game.headset = Some(headset.into());
            s
        };
        // Before the link starts: waiting, nothing spawned.
        let (s, commands) = update(placed("aa"), client(ClientMsg::StartSession));
        assert!(!commands.iter().any(|c| matches!(
            c,
            Command::StartSession { .. } | Command::StartHeadset { .. }
        )));
        assert_eq!(s.headset_waits, Some(1));
        assert_eq!(s.session.as_ref().unwrap().status, SessionStatus::Starting);
        // Paired, its tunnel opens: started there, though it is away.
        use crate::devices::LinkEvent;
        let (s, _) = update(
            s,
            Event::Link(LinkEvent::Started {
                listening: Some("0.0.0.0:7000".into()),
                paired: vec![crate::devices::Paired {
                    id: "aa".into(),
                    name: "Quest aa".into(),
                    first_seen: 1,
                    last_seen: 1,
                }],
            }),
        );
        assert_eq!(s.headset_waits, Some(1), "no tunnel yet");
        let opened = LinkEvent::TunnelOpened {
            device: "aa".into(),
            url: "http://127.0.0.1:6000".into(),
        };
        let (s, commands) = update(s, Event::Link(opened));
        assert!(commands.contains(&start_headset(1, "http://127.0.0.1:6000")));
        assert_eq!(s.headset_waits, None);
        assert_eq!(s.session.as_ref().unwrap().status, SessionStatus::Starting);

        // The link off: no headset can be reached.
        let (s, _) = update(placed("aa"), client(ClientMsg::StartSession));
        let off = LinkEvent::Started {
            listening: None,
            paired: vec![],
        };
        let (s, _) = update(s, Event::Link(off));
        assert!(matches!(
            &s.session.as_ref().unwrap().status,
            SessionStatus::Failed { reason } if reason.contains("link is off")
        ));

        // Forgotten: the link has started and does not list it.
        let s = with_headsets(
            update(placed("zz"), client(ClientMsg::StartSession)).0,
            &["aa"],
            &[],
        );
        assert_eq!(s.headset_waits, None);
        assert!(matches!(
            &s.session.as_ref().unwrap().status,
            SessionStatus::Failed { reason } if reason.contains("no longer paired")
        ));
    }

    /// Examples/a's game on headset "aa" (conn 0), polled at program
    /// revision 4.
    fn voice_ready() -> AppState {
        let live = with_session(1, Some(42), running(Some(3)));
        let s = with_headsets(live, &["aa"], &["aa"]);
        let (s, _) = update(s, on_headset("aa"));
        let listening = SessionEvent::Listening {
            debug_url: "http://127.0.0.1:6000".into(),
        };
        let id = s.session.as_ref().unwrap().id;
        let (s, _) = update(s, session(id, listening));
        reloaded_to(s, 4)
    }

    /// `s` after a poll reporting program revision `revision`.
    fn reloaded_to(s: AppState, revision: u64) -> AppState {
        let id = s.session.as_ref().unwrap().id;
        let clock = PolledClock {
            program_revision: revision,
            ..PolledClock::default()
        };
        update(s, session(id, polled_clock(8, clock))).0
    }

    fn said(text: &str, utterance: u64) -> Event {
        Event::Link(LinkEvent::Utterance {
            conn: 0,
            utterance,
            text: text.to_string(),
            context: Some("at frame 7".to_string()),
        })
    }

    fn stages(commands: &[Command]) -> Vec<(u64, Stage)> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Link(LinkCommand::Voice {
                    device,
                    utterance,
                    stage,
                }) if device == "aa" => Some((*utterance, stage.clone())),
                _ => None,
            })
            .collect()
    }

    fn spoken() -> Spoken {
        Spoken {
            headset: "Quest aa".to_string(),
            context: Some("at frame 7".to_string()),
            route: None,
        }
    }

    /// `s` after utterance 1, "go", started task 4.
    fn voice_task_running() -> AppState {
        let (s, commands) = update(voice_ready(), said("go", 1));
        let Some(Command::ReserveTask { voice, .. }) = commands
            .into_iter()
            .find(|c| matches!(c, Command::ReserveTask { .. }))
        else {
            panic!("reserved");
        };
        let reserved = Event::NewTask {
            id: Ok(4),
            message: "go".to_string(),
            harness: Harness::Claude,
            voice,
            scope: Default::default(),
        };
        update(s, reserved).0
    }

    /// Said with nothing running: a new task in the game's project, its
    /// message marked spoken, the agent told where; `Working`, then
    /// `Reloaded` once the run ends ok and the game has reloaded.
    #[test]
    fn an_utterance_starts_a_task_and_reports_until_the_game_reloads() {
        let s = voice_ready();
        assert_eq!(voice::route(&s, "aa", "make it red"), Route::New);
        let (s, commands) = update(s, said("make it red", 5));
        let Some(Command::ReserveTask {
            message,
            harness,
            voice: Some(u),
        }) = commands
            .iter()
            .find(|c| matches!(c, Command::ReserveTask { .. }))
        else {
            panic!("{commands:?}");
        };
        assert_eq!(
            (message.as_str(), *harness),
            ("make it red", Harness::Claude)
        );
        assert_eq!((u.device.as_str(), u.utterance), ("aa", 5));
        let reserved = Event::NewTask {
            id: Ok(9),
            message: message.clone(),
            harness: *harness,
            voice: Some(u.clone()),
            scope: Default::default(),
        };
        let (s, commands) = update(s, reserved);
        assert_eq!(
            task(&s, 9).transcript,
            [Item::User {
                text: "make it red".to_string(),
                spoken: Some(Spoken { route: Some("Started a new task".into()), ..spoken() }),
            }]
        );
        let run = spec(&commands);
        assert!(run
            .message
            .starts_with("(Said aloud in the headset (Quest aa) at frame 7."));
        assert!(run.message.ends_with("\n\nmake it red"));
        assert_eq!(stages(&commands), [(5, Stage::Working)]);

        // A reload mid-run is remembered; the outcome waits for the end.
        let s = reloaded_to(s, 5);
        let (s, commands) = update(s, agent(9, 0, finished()));
        assert_eq!(stages(&commands), []);
        let (s, commands) = update(s, ended(9, 0));
        assert_eq!(stages(&commands), [(5, Stage::Reloaded)]);
        assert!(s.voice.is_empty());
    }

    /// A run that ends ok before its push lands is `Reloaded` when it does.
    #[test]
    fn a_reload_after_the_run_ends_still_counts() {
        let (s, _) = update(voice_task_running(), agent(4, 0, finished()));
        let (s, commands) = update(s, ended(4, 0));
        assert_eq!(stages(&commands), []);
        let id = s.session.as_ref().unwrap().id;
        let clock = PolledClock {
            program_revision: 6,
            ..PolledClock::default()
        };
        let (_, commands) = update(s, session(id, polled_clock(9, clock)));
        assert_eq!(stages(&commands), [(1, Stage::Reloaded)]);
    }

    /// Said while a task runs for the game: that task is stopped and
    /// resumed with it (D29's steer), and only the new utterance is followed.
    /// A run cut before its agent started (no session yet, or steered twice
    /// quickly) leaves no `Stopped`: the next run gets its message too.
    #[test]
    fn an_utterance_steers_the_running_task() {
        let (s, _) = update(voice_ready(), nth_task(3, "make a ball"));
        let (fresh, commands) = update(s.clone(), said("bigger", 5));
        assert_eq!(stages(&commands), [(5, Stage::Working)]);
        let run = spec(&commands);
        assert_eq!(run.resume, None);
        assert!(run.message.starts_with("make a ball\n\n"), "{}", run.message);
        assert!(run.message.ends_with("\n\nbigger"), "{}", run.message);
        assert_eq!(task(&fresh, 3).run.as_ref().map(|r| r.id), Some(1));
        assert!(commands.contains(&Command::StopRun { task: 3, run: 0 }));
        let (s, _) = update(s, agent(3, 0, started("sess-1")));
        // Steered, then steered again before that run's agent started.
        let (quick, _) = update(s.clone(), said("red", 8));
        let (_, commands) = update(quick, said("and round", 9));
        let run = spec(&commands);
        assert_eq!(run.resume.as_deref(), Some("sess-1"));
        assert!(run.message.contains("\n\nred\n\n"), "{}", run.message);
        assert!(run.message.ends_with("\n\nand round"), "{}", run.message);
        assert_eq!(voice::route(&s, "aa", "bigger"), Route::Steer(3));
        let (s, commands) = update(s, said("bigger", 6));
        let stop = commands
            .iter()
            .position(|c| *c == Command::StopRun { task: 3, run: 0 });
        let start = commands
            .iter()
            .position(|c| matches!(c, Command::StartRun(_)));
        assert!(stop.is_some() && stop < start, "{commands:?}");
        assert_eq!(spec(&commands).resume.as_deref(), Some("sess-1"));
        assert_eq!(stages(&commands), [(6, Stage::Working)]);
        let t = task(&s, 3);
        assert_eq!(t.transcript[1], Item::Stopped);
        assert_eq!(
            t.transcript[2],
            Item::User {
                text: "bigger".to_string(),
                spoken: Some(Spoken { route: Some("Steered “make a ball”".into()), ..spoken() }),
            }
        );

        // Said again: the first utterance is superseded, silently.
        let (s, commands) = update(s, said("and blue", 7));
        assert_eq!(stages(&commands), [(7, Stage::Working)]);
        assert_eq!(s.voice.len(), 1);
    }

    /// Task 3, "make a ball", running with a session, and the model router on.
    fn routing() -> AppState {
        let (s, _) = update(voice_ready(), nth_task(3, "make a ball"));
        let (mut s, _) = update(s, agent(3, 0, started("sess-1")));
        s.router = true;
        s.model_routes.insert("examples/a".to_string());
        s
    }

    /// `s` after the model router answers `choice` for utterance `n`.
    fn answered(s: AppState, n: u64, choice: Result<Choice, String>) -> (AppState, Vec<Command>) {
        let routed = Event::Routed {
            device: "aa".to_string(),
            utterance: n,
            choice,
        };
        update(s, routed)
    }

    /// `commands`' reserved utterance given task id `id`.
    fn reserved(s: AppState, commands: Vec<Command>, id: u64) -> (AppState, Vec<Command>) {
        let Some(Command::ReserveTask { voice, message, .. }) = commands
            .into_iter()
            .find(|c| matches!(c, Command::ReserveTask { .. }))
        else {
            panic!("reserved");
        };
        let event = Event::NewTask {
            id: Ok(id),
            message,
            harness: Harness::Claude,
            scope: None,
            voice,
        };
        update(s, event)
    }

    fn route_of(s: &AppState, task: u64, item: usize) -> Option<String> {
        match &self::task(s, task).transcript[item] {
            Item::User {
                spoken: Some(sp), ..
            } => sp.route.clone(),
            _ => None,
        }
    }

    /// With a choice to make, the model router is asked (nothing happens
    /// until it answers); its choice of a place it was shown is taken, with
    /// its title; anything else (an error or timeout, a task it was not
    /// shown, unclear) falls back to the rule. With nothing to choose
    /// between, it is not asked.
    #[test]
    fn the_model_router_decides_when_there_is_a_choice() {
        let (idle, commands) = update(AppState { router: true, model_routes: ["examples/a".to_string()].into(), ..voice_ready() }, said("go", 1));
        assert!(!commands.iter().any(|c| matches!(c, Command::Route { .. })));
        assert!(idle.routing.is_empty());
        // Off for the project (the default): the rule decides at once.
        let (_, commands) = update(routing(), client(ClientMsg::SetModelRoute { on: false }));
        assert!(commands.iter().any(|c| matches!(c, Command::SaveModelRoute { on: false, .. })));
        let off = AppState {
            model_routes: BTreeSet::new(),
            ..routing()
        };
        let (_, commands) = update(off, said("bigger", 2));
        assert_eq!(spec(&commands).task, 3);

        let (s, commands) = update(routing(), said("add a pause menu", 5));
        let Some(Command::Route { request, .. }) = commands.iter().find(|c| matches!(c, Command::Route { .. })) else {
            panic!("{commands:?}");
        };
        let running = request.running.as_ref().unwrap();
        assert_eq!((running.id, running.title.as_str()), (3, "make a ball"));
        assert_eq!(running.last_instruction, "make a ball");
        assert!(!commands.iter().any(|c| matches!(c, Command::StartRun(_))));

        let new = Ok(Choice::NewTask {
            title: "Add a pause menu".into(),
        });
        let (q, commands) = answered(s.clone(), 5, new);
        let (q, _) = reserved(q, commands, 6);
        let t = task(&q, 6);
        assert_eq!((t.title.as_str(), &t.status), ("Add a pause menu", &TaskStatus::Queued));
        assert_eq!(route_of(&q, 6, 0).as_deref(), Some("Queued behind “make a ball”"));
        let r = &q.routes[0];
        assert_eq!((r.task, r.item, r.by), (6, 0, By::Model));
        assert_eq!(
            r.moves,
            [route::Move {
                to: Destination::Steer { task: 3 },
                label: "Steer “make a ball” instead".into(),
            }]
        );

        // Fallbacks: the rule steers task 3.
        for choice in [
            Err("the router took over 3 s".to_string()),
            Ok(Choice::Steer { task: 99 }),
            Ok(Choice::AddToQueued { task: 3 }),
            Ok(Choice::Unclear),
        ] {
            let (f, commands) = answered(s.clone(), 5, choice);
            assert_eq!(spec(&commands).task, 3);
            assert_eq!(route_of(&f, 3, 2).as_deref(), Some("Steered “make a ball”"));
            assert_eq!(f.routes[0].by, By::Rule);
        }
        // A task queued while the model was asked was not shown to it: the rule.
        let (later, _) = update(s.clone(), nth_task(9, "typed meanwhile"));
        let (later, commands) = answered(later, 5, Ok(Choice::AddToQueued { task: 9 }));
        assert_eq!(spec(&commands).task, 3);
        assert_eq!(task(&later, 9).transcript.len(), 1);
        // The project changed meanwhile: it goes nowhere, and the headset hears why.
        let (moved, _) = update(s.clone(), switch("examples/b"));
        let (_, commands) = answered(moved, 5, Ok(Choice::Steer { task: 3 }));
        assert!(matches!(&stages(&commands)[..], [(5, Stage::Failed(_))]), "{commands:?}");
        // A late answer for an utterance no longer waiting changes nothing.
        let (_, commands) = answered(s.clone(), 4, Ok(Choice::Unclear));
        assert!(commands.is_empty(), "{commands:?}");
    }

    /// Moving a steer: the run stops and resumes told to ignore it (what it
    /// changed meanwhile stays, and the message says so); the utterance
    /// becomes a new task, queued behind it. A queued task made for an
    /// utterance moves to a steer and is gone. Once the steered run ends,
    /// it can no longer move.
    #[test]
    fn a_decision_moves_while_it_can_be_undone() {
        let (s, _) = update(routing(), said("bigger", 5));
        let (s, commands) = answered(s, 5, Ok(Choice::Steer { task: 3 }));
        assert_eq!(spec(&commands).task, 3);
        let id = s.routes[0].id;
        let labels: Vec<&str> = s.routes[0].moves.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["Make it a new task"]);

        // Moved before the steered run's agent started: it never had the
        // message, which goes; the interrupted run resumes.
        let early = client(ClientMsg::MoveUtterance {
            id,
            to: Destination::New,
        });
        let (cut, commands) = update(s.clone(), early);
        assert!(commands.contains(&Command::StopRun { task: 3, run: 2 }));
        assert_eq!(task(&cut, 3).transcript.len(), 4);
        assert_eq!(task(&cut, 3).run.as_ref().map(|r| r.id), Some(3), "not the cut run's id");
        assert_eq!(spec(&commands).message, RESUMED);
        assert_eq!(stages(&commands), []);
        let (s, _) = update(s, agent(3, 2, started("sess-1")));

        let to_new = client(ClientMsg::MoveUtterance {
            id,
            to: Destination::New,
        });
        let (moved, commands) = update(s.clone(), to_new.clone());
        assert!(commands.contains(&Command::StopRun { task: 3, run: 2 }));
        let run = spec(&commands);
        assert_eq!((run.message.as_str(), run.resume.as_deref()), (RETRACTED, Some("sess-1")));
        assert_eq!(route_of(&moved, 3, 2).as_deref(), Some("Moved to a new task."));
        let (moved, commands) = reserved(moved, commands, 7);
        assert_eq!(task(&moved, 7).status, TaskStatus::Queued);
        assert_eq!(
            route_of(&moved, 7, 0).as_deref(),
            Some("Moved: queued behind “make a ball”")
        );
        assert_eq!(stages(&commands), []);
        // ...and back: the queued task goes, task 3 is steered with it.
        let back = moved.routes.iter().find(|r| r.task == 7).unwrap();
        let to_steer = client(ClientMsg::MoveUtterance {
            id: back.id,
            to: Destination::Steer { task: 3 },
        });
        let (again, commands) = update(moved, to_steer);
        assert!(again.tasks.iter().all(|t| t.id != 7));
        assert!(commands.iter().any(|c| matches!(c, Command::DeleteTask { task: 7, .. })));
        assert_eq!(spec(&commands).task, 3);
        assert_eq!(stages(&commands), [(5, Stage::Working)]);

        // In a git repo the line says the run's edits stay.
        let mut tracked_s = s.clone();
        tracked_s.tracks_changes = true;
        let (moved, _) = update(tracked_s, to_new.clone());
        assert!(route_of(&moved, 3, 2).unwrap().ends_with("stays in this task’s change."));

        // The steered run ended: closed.
        let (s, _) = update(s, agent(3, 2, finished()));
        let (s, _) = update(s, ended(3, 2));
        assert!(s.routes.is_empty());
        let (_, commands) = update(s, to_new);
        assert_eq!(commands, rejected("that message can no longer be moved"));
    }

    /// Added to a queued task, it can be split out as its own task: taken
    /// off the queued one (its transcript resent) while nothing followed it.
    #[test]
    fn an_addition_to_a_queued_task_moves_out() {
        let (s, _) = update(routing(), nth_task(4, "make it red"));
        let (s, _) = update(s, said("no, blue", 5));
        let (s, _) = answered(s, 5, Ok(Choice::AddToQueued { task: 4 }));
        assert_eq!(task(&s, 4).transcript.len(), 2);
        assert_eq!(route_of(&s, 4, 1).as_deref(), Some("Added to “make it red”, queued"));
        let r = &s.routes[0];
        let labels: Vec<&str> = r.moves.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["Steer “make a ball” instead", "Make it a new task"]);
        let to_new = client(ClientMsg::MoveUtterance {
            id: r.id,
            to: Destination::New,
        });
        let (out, commands) = update(s.clone(), to_new);
        assert_eq!(task(&out, 4).transcript.len(), 1);
        assert!(commands.contains(&Command::Broadcast(ServerMsg::Transcript {
            task: 4,
            items: task(&out, 4).transcript.clone(),
        })));
        let (out, _) = reserved(out, commands, 8);
        assert_eq!(queue::order(&out.tasks, "examples/a"), [4, 8]);
        // Typed after it: no longer last, so it stays.
        let (s, _) = update(s, send(4, "and big"));
        assert!(s.routes.is_empty());
    }

    /// A task that started while the utterance's id was reserved is steered
    /// with it, as the rule would have chosen then; the id goes unused.
    #[test]
    fn a_task_started_meanwhile_is_steered() {
        let (s, commands) = update(voice_ready(), said("go", 1));
        let Some(Command::ReserveTask { voice, .. }) = commands
            .into_iter()
            .find(|c| matches!(c, Command::ReserveTask { .. }))
        else {
            panic!("reserved");
        };
        let (s, _) = update(s, nth_task(3, "make a ball"));
        let reserved = Event::NewTask {
            id: Ok(4),
            message: "go".to_string(),
            harness: Harness::Claude,
            scope: None,
            voice,
        };
        let (s, commands) = update(s, reserved);
        assert_eq!(s.tasks.len(), 1);
        assert_eq!(spec(&commands).task, 3);
        assert_eq!(stages(&commands), [(1, Stage::Working)]);
    }

    /// Nothing to change: no game from the app on that headset, or a
    /// stopped one; an unknown connection is ignored.
    #[test]
    fn an_utterance_with_no_game_on_its_headset_fails_plainly() {
        let s = with_headsets(
            with_session(1, Some(42), running(Some(3))),
            &["aa"],
            &["aa"],
        );
        let (s, commands) = update(s, said("make it red", 1));
        let [(1, Stage::Failed(reason))] = &stages(&commands)[..] else {
            panic!("{commands:?}");
        };
        assert!(
            reason.contains("not running a game from the Functor app"),
            "{reason}"
        );
        assert!(s.tasks.is_empty());

        let mut stopped = voice_ready();
        stopped.session.as_mut().unwrap().status = SessionStatus::Stopped;
        let (_, commands) = update(stopped, said("make it red", 2));
        assert!(matches!(&stages(&commands)[..], [(2, Stage::Failed(_))]));

        let stranger = Event::Link(LinkEvent::Utterance {
            conn: 99,
            utterance: 3,
            text: "hi".to_string(),
            context: None,
        });
        let (_, commands) = update(voice_ready(), stranger);
        assert!(stages(&commands).is_empty());
    }

    /// A run that fails, is stopped, or ends ok without the game reloading
    /// in time is `Failed`; so is the game leaving the headset.
    #[test]
    fn a_followed_run_that_does_not_reload_fails() {
        let error = Event::Run {
            task: 4,
            run: 0,
            event: RunEvent::Ended {
                error: Some("boom".to_string()),
            },
        };
        let (_, commands) = update(voice_task_running(), error);
        assert_eq!(stages(&commands), [(1, Stage::Failed("boom".to_string()))]);

        let stop = client(ClientMsg::StopTask { task: 4 });
        let (_, commands) = update(voice_task_running(), stop);
        assert_eq!(
            stages(&commands),
            [(1, Stage::Failed("stopped".to_string()))]
        );

        let (s, _) = update(voice_task_running(), agent(4, 0, finished()));
        let (s, commands) = update(AppState { clock: 1_000, ..s }, ended(4, 0));
        assert!(stages(&commands).is_empty(), "waits for the push");
        let late = AppState {
            clock: 1_000 + voice::RELOAD_GRACE_MS,
            ..s
        };
        let (_, commands) = update(late, Event::Link(LinkEvent::Tick));
        assert!(
            matches!(&stages(&commands)[..], [(1, Stage::Failed(_))]),
            "{commands:?}"
        );

        let reason = "the game stopped on the headset".to_string();
        let gone = [(1, Stage::Failed(reason))];
        let (_, commands) = update(voice_task_running(), client(ClientMsg::StopSession));
        assert_eq!(stages(&commands), gone);
        // Another project's game on the same headset, started in the same
        // step: its reloads are not this run's.
        let mut s = voice_task_running();
        let placed = s.layouts["examples/a"].clone();
        s.layouts.insert("examples/b".to_string(), placed);
        s.start_on_open = true;
        let (s, commands) = update(s, switch("examples/b"));
        assert!(s.session.as_ref().is_some_and(|x| x.status.is_live()));
        assert_eq!(stages(&commands), gone);
    }

    /// A task whose review is preparing has no run of its own to steer:
    /// what is said starts a new task.
    #[test]
    fn a_review_is_never_steered() {
        let (s, _) = update(voice_task_running(), agent(4, 0, finished()));
        let (mut s, _) = update(s, ended(4, 0));
        s.tasks[0].review = Some(Review {
            run: 0,
            harness: Harness::Claude,
            model: None,
            state: ReviewState::Preparing,
            usage: None,
            detail: None,
            staged: None,
        });
        assert_eq!(voice::route(&s, "aa", "and blue"), Route::New);
    }

    /// An answer that changed nothing is said at once, not after the wait
    /// for a reload; said during a revert, it queues a task behind it,
    /// `Working` once that starts.
    #[test]
    fn an_answer_without_a_change_and_a_revert_are_reported() {
        let mut s = voice_task_running();
        s.tracks_changes = true;
        let baseline = Event::Run {
            task: 4,
            run: 0,
            event: RunEvent::Baseline {
                tree: "t0".to_string(),
            },
        };
        let (s, _) = update(s, baseline);
        let (s, _) = update(s, agent(4, 0, finished()));
        let none = Event::Change {
            task: 4,
            baseline: "t0".to_string(),
            change: Change::default(),
        };
        let (s, _) = update(s, none);
        let (_, commands) = update(s, ended(4, 0));
        assert!(
            matches!(&stages(&commands)[..], [(1, Stage::Failed(r))] if r.contains("without changing")),
            "{commands:?}"
        );

        let mut s = voice_ready();
        s.tracks_changes = true;
        s.tasks.push(Task {
            reverting: true,
            run: None,
            baseline: Some("t0".to_string()),
            change: Some(one_file(1)),
            ..task(&voice_task_running(), 4).clone()
        });
        assert_eq!(voice::route(&s, "aa", "go"), Route::New);
        let (s, commands) = update(s, said("go", 2));
        let Some(Command::ReserveTask { voice, .. }) = commands
            .into_iter()
            .find(|c| matches!(c, Command::ReserveTask { .. }))
        else {
            panic!("reserved");
        };
        let reserved = Event::NewTask {
            id: Ok(5),
            message: "go".to_string(),
            harness: Harness::Claude,
            scope: None,
            voice,
        };
        let (s, commands) = update(s, reserved);
        assert_eq!(task(&s, 5).status, TaskStatus::Queued);
        assert_eq!(stages(&commands), []);
        let (_, commands) = update(s, reverted(4, "t0", Ok(())));
        assert_eq!(spec(&commands).task, 5);
        assert_eq!(stages(&commands), [(2, Stage::Working)]);
    }

    // The headset panel.

    fn panels(commands: &[Command]) -> Vec<&crate::panel::Panel> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Link(LinkCommand::Panel { panel, .. }) => Some(&**panel),
                _ => None,
            })
            .collect()
    }

    fn from_panel(conn: u64, action: crate::panel::PanelAction) -> Event {
        Event::Link(LinkEvent::Panel { conn, action })
    }

    #[test]
    fn a_linked_headset_is_sent_its_panel_on_link_and_on_change_only() {
        use crate::activity::Stance;
        use crate::devices::LinkKind;
        let s = reviewed(one_file(2));
        let connected = |conn| {
            Event::Link(LinkEvent::Connected {
                conn,
                device: "aa".into(),
                name: "Quest aa".into(),
                app_version: "0.0.0".into(),
                link: LinkKind::Wifi,
            })
        };
        let s = with_headsets(s, &["aa"], &[]);
        assert!(s.panels.is_empty(), "not linked: nothing sent");
        let (s, commands) = update(s, connected(3));
        let sent = panels(&commands);
        assert_eq!(sent.len(), 1);
        let panel = sent[0];
        assert_eq!(panel.project.as_deref(), Some("examples/a"));
        assert!(!panel.placed);
        assert_eq!(panel.now, Some(1));
        let t = &panel.tasks[0];
        assert_eq!(
            (t.title.as_str(), t.stance),
            ("Make it spin", Stance::Waiting)
        );
        let d = t.decision.as_ref().unwrap();
        assert_eq!(d.tree, "now-2");
        assert_eq!(d.size, "+2 -1, 1 file, game logic");
        assert!(!d.glance && !d.reason.is_empty());

        // Nothing changed: nothing sent.
        let (s, commands) = update(s, Event::Link(LinkEvent::Tick));
        assert!(panels(&commands).is_empty());
        // Kept on the desktop: the panel follows.
        let (s, commands) = update(s, keep(1));
        let t = &panels(&commands)[0].tasks[0];
        assert_eq!((t.stance, t.decision.is_none()), (Stance::Done, true));
        // Gone and back: sent again.
        let gone = Event::Link(LinkEvent::Disconnected {
            conn: 3,
            lost: None,
        });
        let (s, _) = update(s, gone);
        assert!(s.panels.is_empty());
        let (s, commands) = update(s, connected(4));
        assert_eq!(panels(&commands).len(), 1);
        // Back on a new link before the old one drops: sent again.
        let (s, commands) = update(s, connected(5));
        assert_eq!(panels(&commands).len(), 1);
        let late = Event::Link(LinkEvent::Disconnected {
            conn: 4,
            lost: None,
        });
        let (_, commands) = update(s, late);
        assert!(panels(&commands).is_empty());
    }

    #[test]
    fn panel_actions_are_the_client_messages_from_a_linked_headset_only() {
        use crate::panel::PanelAction;
        let s = with_headsets(reviewed(one_file(2)), &["aa"], &["aa"]);
        let refused = |conn: u64, reason: &str| {
            Command::Link(LinkCommand::PanelRefused {
                conn,
                reason: reason.into(),
            })
        };

        // Keep is the desktop's Keep, with no reply to a client.
        let (kept, commands) = update(s.clone(), from_panel(0, PanelAction::Keep { task: 1 }));
        let (desktop, _) = update(s.clone(), keep(1));
        assert_eq!(task(&kept, 1).status, task(&desktop, 1).status);
        assert_eq!(task(&kept, 1).status, TaskStatus::Kept);
        assert!(!commands.iter().any(|c| matches!(c, Command::Reply(_))));

        // A revert names the change the user saw.
        let stale = PanelAction::Revert {
            task: 1,
            tree: "now-1".into(),
        };
        let (_, commands) = update(s.clone(), from_panel(0, stale));
        assert!(commands.contains(&refused(
            0,
            "task 1's change has changed since you saw it: review it again"
        )));
        let seen = PanelAction::Revert {
            task: 1,
            tree: "now-2".into(),
        };
        let (reverting, _) = update(s.clone(), from_panel(0, seen));
        assert!(task(&reverting, 1).reverting);

        // Select is the chat's current task; Stop a run's stop.
        let (selected, _) = update(s.clone(), from_panel(0, PanelAction::Select { task: 1 }));
        assert_eq!(selected.current_task["examples/a"], Some(1));
        let (_, commands) = update(s.clone(), from_panel(0, PanelAction::Stop { task: 1 }));
        assert!(commands.contains(&refused(0, "task 1 is not running")));

        // Not a linked headset's connection: dropped, nothing changes.
        let (after, commands) = update(s.clone(), from_panel(7, PanelAction::Keep { task: 1 }));
        assert!(commands.is_empty());
        assert_eq!(after, s);
        // A task outside the open project is not the panel's.
        let mut other = s.clone();
        other.tasks[0].project = "examples/b".into();
        let (after, commands) = update(other.clone(), from_panel(0, PanelAction::Keep { task: 1 }));
        assert_eq!(commands, vec![refused(0, "there is no task 1 here")]);
        assert_eq!(after, other);
    }

    /// The panel shows this headset's latest route with its moves, and a
    /// move from it is the desktop's move; a queued task says why it waits.
    #[test]
    fn the_panel_carries_the_route_its_moves_and_the_queue() {
        use crate::panel::{panel, PanelAction};
        let (s, _) = update(routing(), said("bigger", 5));
        let (s, _) = answered(s, 5, Ok(Choice::Steer { task: 3 }));
        let (s, _) = update(s, agent(3, 2, started("sess-1")));
        let route = panel(&s, "aa").route.unwrap();
        assert_eq!(route.id, s.routes[0].id);
        assert_eq!(route.line, s.routes[0].line);
        assert_eq!(route.moves, s.routes[0].moves);
        assert!(panel(&s, "bb").route.is_none(), "another headset's speech");

        let to = route.moves[0].to.clone();
        let (by_panel, commands) = update(s.clone(), from_panel(0, PanelAction::Move { id: route.id, to: to.clone() }));
        let (by_desktop, _) = update(s.clone(), client(ClientMsg::MoveUtterance { id: route.id, to: to.clone() }));
        assert!(commands.contains(&Command::StopRun { task: 3, run: 2 }));
        assert_eq!(by_panel.tasks, by_desktop.tasks);
        let (moved, commands) = reserved(by_panel, commands, 7);
        let p = panel(&moved, "aa");
        let queued = p.tasks.iter().find(|t| t.id == 7).unwrap();
        assert!(queued.queued);
        assert!(p.waiting.unwrap().starts_with("Starts after"), "{commands:?}");

        let stale = PanelAction::Move { id: 999, to };
        let (_, commands) = update(s, from_panel(0, stale));
        assert!(commands.iter().any(|c| matches!(
            c,
            Command::Link(LinkCommand::PanelRefused { conn: 0, .. })
        )));
    }
}
