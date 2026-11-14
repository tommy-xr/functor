//! Messages between the UI and the core.

use serde::{Deserialize, Serialize};

use crate::changes::FileDiff;
use crate::creation::{NewProjectRequest, Plan};
use crate::layout::LayoutOp;
use crate::review::ReviewSetting;
use crate::tasks::{Harness, Item, Scope};
use crate::transport::{Mark, Transport, TransportCall};
use crate::AppState;

/// UI → core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    GetState,
    /// `path` is a project's path relative to the repo root.
    SetActiveProject {
        path: String,
    },
    /// Start a game session for the active project, unless one is live.
    StartSession,
    StopSession,
    /// Forwarded to a running session; dropped otherwise. Not answered.
    Input {
        input: GameInput,
    },
    /// The transport bar's call, posted in order with input; dropped
    /// without a running session. Answered by `ServerMsg::Transport`.
    Transport {
        call: TransportCall,
    },
    /// This page shows a transport bar, or no longer does: while any page
    /// does, the session is polled often and its marks are read.
    WatchTransport {
        on: bool,
    },
    /// Start a task in the active project with its first message; `scope`
    /// for this task, else the project's (D39). Queued while the project is
    /// busy (D30).
    NewTask {
        message: String,
        harness: Harness,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        scope: Option<Scope>,
    },
    /// A message to `task`: a follow-up continuing its agent session (and
    /// reading what it started with); while it runs, a steer (its run stops
    /// and resumes with it, D29); while it is queued, added to what it starts
    /// with (D30).
    SendMessage {
        task: u64,
        message: String,
    },
    /// Stop `task`'s run.
    StopTask {
        task: u64,
    },
    /// Move queued `task` to place `to` (0: next) in its project's queue
    /// (D30); past the end is last.
    MoveQueued {
        task: u64,
        to: usize,
    },
    /// Start queued `task` now, beside a change waiting for a decision
    /// (which holds only the queue's own starts); refused while a run or a
    /// revert holds its project.
    StartQueued {
        task: u64,
    },
    /// Answered with [`ServerMsg::Transcript`].
    GetTranscript {
        task: u64,
    },
    /// The agent new tasks in the active project start with; remembered
    /// per project.
    SetHarness {
        harness: Harness,
    },
    /// How much of the user's own setup the active project's runs read
    /// (D39); remembered per project.
    SetScope {
        scope: Scope,
    },
    /// How the active project's changes are reviewed (D31); remembered.
    SetReview {
        setting: ReviewSetting,
    },
    /// Answered with [`ServerMsg::Diff`]; also refreshes the task's change.
    GetDiff {
        task: u64,
    },
    /// Keep `task`'s change: its review closes, the files stay as they are.
    KeepChange {
        task: u64,
    },
    /// Put the files `task` changed back as they were before it started.
    /// `tree` is the `Change::tree` the user confirmed: a change measured
    /// differently since is refused, to be reviewed again.
    RevertChange {
        task: u64,
        tree: String,
    },
    /// Changes the active project's layout (`AppState::layouts`), the game
    /// view's placement included.
    Layout {
        op: LayoutOp,
    },
    /// Answered with [`ServerMsg::File`]: the text of `path`, relative to
    /// the active project's directory.
    GetFile {
        path: String,
    },
    /// Open `path` (absolute; `~` is the user's home) as the workspace,
    /// stopping the session; refused while an agent run is active.
    OpenFolder {
        path: String,
    },
    /// Choose a folder to open in the native picker (`AppState::native_picker`).
    PickFolder,
    /// Open the new-project modal; `here` fills it with the open folder.
    ShowNewProject {
        here: bool,
    },
    /// Close it; refused while a project is being created.
    CloseNewProject,
    /// Answered with [`ServerMsg::NewProjectPlan`]: what `request` would do.
    CheckNewProject {
        request: NewProjectRequest,
    },
    /// Choose the new project's parent folder in the native picker.
    PickParent,
    /// Create the project, open it, start its game, and start a task with
    /// `request.describe` (with `harness`) if it is not blank.
    CreateProject {
        request: NewProjectRequest,
        harness: Harness,
    },
    /// Answered with [`ServerMsg::Dirs`]: directories completing `path`.
    ListDirs {
        path: String,
    },
    /// Drop `path` from the recent projects.
    RemoveRecent {
        path: String,
    },
    /// Show step `step` of the walkthrough at transcript index `item` of
    /// `task` (a click on its card).
    WalkthroughStep {
        task: u64,
        item: usize,
        step: usize,
    },
    /// The user scrolled highlight `seq` (`AppState::highlight`) out of view.
    ClearHighlight {
        seq: u64,
    },
    /// The canvas now stacks its panes (too narrow side by side), or not.
    CanvasStacked {
        stacked: bool,
    },
    /// Make `task` (of the active project) the chat's current task; `None`
    /// is a new task (`AppState::current_task`). `if_unset`: only when the
    /// project has none yet (a page's default).
    SelectTask {
        task: Option<u64>,
        if_unset: bool,
    },
    /// Give `task` the title the user typed ([`crate::tasks::renamed`]).
    RenameTask {
        task: u64,
        title: String,
    },
    /// Forget `task`: its stored conversation is removed and its tabs close.
    /// The user's files are untouched; refused while it runs or has a
    /// change to review.
    DeleteTask {
        task: u64,
    },
    /// The user compared the codes: pair the device asking with `request`
    /// (`Devices::requests`); the secret goes once the headset confirms too.
    AllowPairing {
        request: u64,
    },
    /// The code is not theirs: refuse `request`, storing nothing.
    DenyPairing {
        request: u64,
    },
    /// Forget paired `device` (its id): its pairing secret is deleted on
    /// this side and its link closes.
    ForgetDevice {
        device: String,
    },
    /// Pair the headset on adb serial `serial`'s cable (`Devices::attached`),
    /// with no code. `replace`: the user agreed to replace its pairing with
    /// another desktop.
    PairByCable {
        serial: String,
        #[serde(default)]
        replace: bool,
    },
}

/// Keyboard and mouse input in the debug server's `POST /input` shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameInput {
    /// `key` is a wire name: `a`–`z`, `0`–`9`, `up`, `space`, `enter`, ….
    Key {
        key: String,
        down: bool,
    },
    /// In the game's pointer surface ([`crate::SessionStatus::Running`]).
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
    },
    /// Positive scrolls up, as GLFW reports it.
    MouseWheel {
        delta: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Core → UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// The whole state; the UI replaces what it holds. Boxed: it is far
    /// larger than the other messages.
    State { state: Box<AppState> },
    /// `task`'s whole transcript, answering `GetTranscript`.
    Transcript { task: u64, items: Vec<Item> },
    /// Item `index` of `task`'s transcript, added (at the end) or replaced:
    /// a streamed text arrives as repeats of one index.
    Item { task: u64, index: usize, item: Item },
    /// `task`'s full diff, answering `GetDiff`: the summary's files (all of
    /// them, listed or not, are in `files` or counted in `more`), each with
    /// its lines unless omitted. `tree` is the snapshot it shows (the
    /// summary's `Change::tree`); `error` when it could not be computed.
    Diff {
        task: u64,
        tree: Option<String>,
        files: Vec<FileDiff>,
        more: u32,
        error: Option<String>,
    },
    /// `path` of `project` as it is on disk, answering `GetFile`: its
    /// `text`, or the `error` saying why not (outside the project, too
    /// large, not UTF-8). `root` is the workspace's (`AppState::repo_root`)
    /// when it was asked: another folder's project may share `project`.
    File {
        root: String,
        project: String,
        path: String,
        text: Option<String>,
        error: Option<String>,
    },
    /// Answering `ListDirs`: up to a few directories completing `path`
    /// (their paths, hidden ones left out), and why `path` cannot be
    /// opened, if it cannot.
    Dirs {
        path: String,
        dirs: Vec<String>,
        error: Option<String>,
    },
    /// Answering `CheckNewProject`: the `plan` for `request`, or the
    /// `error` saying why it cannot be made.
    NewProjectPlan {
        request: NewProjectRequest,
        plan: Option<Plan>,
        error: Option<String>,
    },
    /// A message was refused, saying why; nothing changed.
    Rejected { reason: String },
    /// The active session's clock as it changes; `None` while no session
    /// runs, or before it first answers.
    Transport { transport: Option<Transport> },
    /// The marks on the active session's recorded history, as they change.
    Marks { marks: Vec<Mark> },
}
