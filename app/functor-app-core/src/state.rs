//! `AppState` and the pure `update` that steps it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::changes::Change;
use crate::layout::{Layout, LayoutOp, Placement, Target, View};
use crate::projects::{check_relative, default_project, project_dir, relative, Project};
use crate::protocol::{ClientMsg, GameInput, ServerMsg};
use crate::session::{Display, Session, SessionEvent, SessionMode, SessionStatus};
use crate::tasks::{
    apply, derive_status, end_run, has_change, stop_run, title, Harness, Item, Run, RunEvent, Task,
    TaskStatus,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppState {
    /// Absolute path of the repo root.
    pub repo_root: String,
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
    /// Every project's tasks, by id; their transcripts travel separately.
    pub tasks: Vec<Task>,
    /// The agents this machine can run, in the order the picker lists them.
    pub harnesses: Vec<Harness>,
    /// Per project path, the agent last picked for its new tasks.
    pub harness_choice: BTreeMap<String, Harness>,
    /// Per project path, its canvas layout; every project has one.
    pub layouts: BTreeMap<String, Layout>,
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
            projects,
            active,
            files: Vec::new(),
            git: None,
            tracks_changes: false,
            session: None,
            no_gpu: false,
            tasks: Vec::new(),
            harnesses: Vec::new(),
            harness_choice: BTreeMap::new(),
            layouts,
        }
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
    /// Project `path`'s files, as `ListFiles` read them.
    Files {
        path: String,
        files: Vec<String>,
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
    /// The app is exiting: stop the session and every run.
    Shutdown,
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
    /// Kill session `id`'s child.
    StopSession { id: u64 },
    /// `GET /state` on session `id`'s debug server after the poll interval.
    PollState { id: u64, debug_url: String },
    /// `POST /input` to session `id`'s debug server, in order.
    Input {
        id: u64,
        debug_url: String,
        input: GameInput,
    },
    /// Read project `path`'s files in `dir`, answering with `Event::Files`.
    ListFiles { path: String, dir: PathBuf },
    /// Start an agent run, answering with `Event::Run`s that end in `Ended`.
    StartRun(Box<RunSpec>),
    /// Cancel run `run` of task `task`, waiting until its processes are gone.
    StopRun { task: u64, run: u64 },
    /// Write `task` (with its transcript) to the store for the project at `dir`.
    SaveTask { dir: PathBuf, task: Box<Task> },
    /// Remember `harness` as the pick for new tasks in the project at `dir`.
    SaveHarness { dir: PathBuf, harness: Harness },
    /// Remember `layout` for the project at `dir`; writes may be coalesced.
    SaveLayout { dir: PathBuf, layout: Box<Layout> },
    /// Read `path` (checked by [`check_relative`]) in project `project`'s
    /// `dir`, answering the asking client with `ServerMsg::File`.
    ReadFile {
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
}

/// One agent run for the shell to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSpec {
    pub task: u64,
    pub run: u64,
    pub harness: Harness,
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
    pub context: AppContext,
}

/// What the tool server's `app_context` tool reports to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppContext {
    /// The task's project, relative to the repo root.
    pub project: String,
    /// The game session's debug URL, when it runs this project.
    pub session: Option<String>,
    pub session_mode: SessionMode,
}

pub fn update(mut state: AppState, event: Event) -> (AppState, Vec<Command>) {
    let mut commands = Vec::new();
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
                // The session follows the active project.
                restart_live(&mut state, &mut commands);
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
                commands.push(Command::Input {
                    id: *id,
                    debug_url: debug_url.clone(),
                    input,
                });
            }
            return (state, commands);
        }
        Event::Client(ClientMsg::StartSession) => {
            let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
            (!live && start(&mut state, &mut commands), true)
        }
        Event::Client(ClientMsg::StopSession) => (stop(&mut state, &mut commands), true),
        Event::Client(ClientMsg::SetHarness { harness }) => {
            (set_harness(&mut state, harness, &mut commands), true)
        }
        Event::Client(ClientMsg::Layout { op }) => (layout_op(&mut state, op, &mut commands), true),
        Event::Client(ClientMsg::GetFile { path }) => {
            get_file(&state, path, &mut commands);
            (false, false)
        }
        Event::Session { id, event } => (on_session(&mut state, id, event, &mut commands), false),
        // A listing for a project switched away from is dropped.
        Event::Files { path, files } => {
            if state.active.as_ref() != Some(&path) {
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
            message,
            harness,
        } => match id {
            Ok(id) => (
                new_task(&mut state, id, message, harness, &mut commands),
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
        } => (
            on_change(&mut state, task, &baseline, change, &mut commands),
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
            (on_run(&mut state, task, run, event, &mut commands), false)
        }
        Event::TasksLoaded { tasks } => {
            state.tasks = tasks
                .into_iter()
                .map(|mut t| {
                    // A run (or revert) saved in flight died with the app.
                    t.run = None;
                    t.reverting = false;
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
            (false, false)
        }
    };
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
    (state, commands)
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
    if placement == Placement::Pane && layout.find(&View::Game).is_none() {
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
    let display = Display::new(placement.session_mode(), state.no_gpu);
    commands.push(Command::StartSession { id, dir, display });
    state.session = Some(Session {
        id,
        pid: None,
        status: SessionStatus::Starting,
    });
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
        let name = harness.name();
        return reject(commands, format!("{name} is not available on this machine"));
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

/// Applies `op` to the active project's layout. A live game in a pane
/// keeps its tab: closing it is refused (Stop game, or another placement,
/// first). A placement whose session runs differently restarts it.
fn layout_op(state: &mut AppState, op: LayoutOp, commands: &mut Vec<Command>) -> bool {
    let Some(project) = state.active.clone() else {
        return false;
    };
    let before = state.layouts.get(&project).cloned().unwrap_or_default();
    let mut layout = before.clone();
    match layout.apply(op, |view| check_view(state, &project, view)) {
        Ok(true) => {
            let live = state.session.as_ref().is_some_and(|s| s.status.is_live());
            if live
                && layout.game.placement == Placement::Pane
                && before.find(&View::Game).is_some()
                && layout.find(&View::Game).is_none()
            {
                let reason = "the game is running: stop it, or switch it to Mini, to close its tab";
                return reject(commands, reason.to_string());
            }
            let restart =
                before.game.placement.session_mode() != layout.game.placement.session_mode();
            save_layout(state, &project, layout, commands);
            if restart {
                restart_live(state, commands);
            }
            true
        }
        Ok(false) => false,
        Err(reason) => reject(commands, reason),
    }
}

/// `project`'s game placement.
fn placement(state: &AppState, project: &str) -> Placement {
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

fn save_layout(state: &mut AppState, project: &str, layout: Layout, commands: &mut Vec<Command>) {
    commands.push(Command::SaveLayout {
        dir: project_dir(Path::new(&state.repo_root), project),
        layout: Box::new(layout.clone()),
    });
    state.layouts.insert(project.to_string(), layout);
}

/// Why `view` cannot be open in `project`'s layout. Files are checked
/// against the listing only for the active project, the one listed.
fn check_view(state: &AppState, project: &str, view: &View) -> Result<(), String> {
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
            dir: project_dir(Path::new(&state.repo_root), &project),
            project,
            path,
        }),
        Err(error) => commands.push(Command::Reply(ServerMsg::File {
            project,
            path,
            text: None,
            error: Some(error),
        })),
    }
}

fn reject(commands: &mut Vec<Command>, reason: String) -> bool {
    commands.push(Command::Reply(ServerMsg::Rejected { reason }));
    false
}

fn no_task(id: u64) -> ServerMsg {
    ServerMsg::Rejected {
        reason: format!("there is no task {id}"),
    }
}

/// Starts task `id` in the active project with `message`.
fn new_task(
    state: &mut AppState,
    id: u64,
    message: String,
    harness: Harness,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(project) = state.active.clone() else {
        return reject(
            commands,
            "there is no project to start a task in".to_string(),
        );
    };
    if let Err(reason) = can_run(state, &project, None, &message) {
        return reject(commands, reason);
    }
    state.tasks.push(Task {
        id,
        project,
        title: title(&message),
        status: TaskStatus::InProgress,
        harness,
        model: None,
        session_id: None,
        run: None,
        baseline: None,
        change: None,
        reverting: false,
        transcript: Vec::new(),
    });
    start_run(state, id, message, commands);
    true
}

/// A follow-up on task `id`, resuming its agent session.
fn send_message(
    state: &mut AppState,
    id: u64,
    message: String,
    commands: &mut Vec<Command>,
) -> bool {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        commands.push(Command::Reply(no_task(id)));
        return false;
    };
    if let Err(reason) = can_run(state, &task.project, Some(id), &message) {
        return reject(commands, reason);
    }
    start_run(state, id, message, commands);
    true
}

/// Why `message` cannot start a run in `project` now; `task` for a follow-up.
/// v1 runs one task per project at a time and queues nothing.
fn can_run(
    state: &AppState,
    project: &str,
    task: Option<u64>,
    message: &str,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("the message is empty".to_string());
    }
    match state
        .tasks
        .iter()
        .find(|t| t.project == project && t.run.is_some())
    {
        Some(busy) if Some(busy.id) == task => Err(format!(
            "task {} is still running: wait for it to finish or stop it",
            busy.id
        )),
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

/// Adds `message` to task `id` and starts its run.
fn start_run(state: &mut AppState, id: u64, message: String, commands: &mut Vec<Command>) {
    let root = PathBuf::from(&state.repo_root);
    let project = &state
        .tasks
        .iter()
        .find(|t| t.id == id)
        .expect("a known task")
        .project;
    // The session is the active project's; another project's task gets none.
    let attach = match &state.session {
        Some(Session {
            status: SessionStatus::Running { debug_url, .. },
            ..
        }) if state.active.as_ref() == Some(project) => Some(debug_url.clone()),
        _ => None,
    };
    let session_mode = placement(state, project).session_mode();
    let tracks = state.tracks_changes;
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("a known task");
    let run = task.transcript.len() as u64;
    task.transcript.push(Item::User {
        text: message.clone(),
    });
    task.run = Some(Run {
        id: run,
        usage: None,
    });
    task.status = derive_status(task);
    commands.push(Command::StartRun(Box::new(RunSpec {
        task: id,
        run,
        harness: task.harness,
        dir: project_dir(&root, &task.project),
        repo_root: root.clone(),
        message,
        resume: task.session_id.clone(),
        snapshot: tracks && task.baseline.is_none(),
        baseline: task.baseline.clone().filter(|_| tracks),
        attach: attach.clone(),
        context: AppContext {
            project: task.project.clone(),
            session: attach,
            session_mode,
        },
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
    let changed = stop_run(task);
    // A stopped run reports no change of its own: what it left is measured now.
    if let Some(baseline) = task.baseline.clone() {
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
    let task = match reviewable(state, id) {
        Ok(task) => task,
        Err(reason) => return reject(commands, reason),
    };
    task.baseline = None;
    task.change = None;
    close(&root, task, Item::Kept, commands);
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
    commands.push(Command::Revert {
        task: id,
        baseline: task.baseline.clone().expect("reviewable"),
        paths,
    });
    true
}

/// Ends task `id`'s review with `item` (`Kept` or `Reverted`).
fn close(root: &Path, task: &mut Task, item: Item, commands: &mut Vec<Command>) {
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
    task.change = Some(change);
    task.status = derive_status(task);
    commands.push(save(&root, task));
    true
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
            close(&root, task, Item::Reverted, commands);
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
        RunEvent::Agent(event) => {
            let before = (task.session_id.clone(), task.model.clone());
            let changed = apply(task, event);
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
            let changed = end_run(task, error);
            commands.push(save(&root, task));
            broadcast_items(commands, task, changed);
            true
        }
    }
}

fn save(root: &Path, task: &Task) -> Command {
    Command::SaveTask {
        dir: project_dir(root, &task.project),
        task: Box::new(task.clone()),
    }
}

fn broadcast_items(commands: &mut Vec<Command>, task: &Task, changed: Vec<usize>) {
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
            });
            session.status = SessionStatus::Running {
                debug_url,
                frame: None,
                surface: None,
            };
        }
        (
            SessionEvent::Polled(result),
            SessionStatus::Running {
                debug_url,
                frame,
                surface,
            },
        ) => {
            // Keep polling through a failed request; the child's exit ends it.
            commands.push(Command::PollState {
                id,
                debug_url: debug_url.clone(),
            });
            match result {
                Ok(polled) if (*frame, *surface) != (Some(polled.frame), polled.surface) => {
                    *frame = Some(polled.frame);
                    *surface = polled.surface;
                }
                _ => return false,
            }
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
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::Entries;
    use crate::session::Polled;

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
            tasks: vec![],
            harnesses: vec![Harness::Claude, Harness::Codex],
            harness_choice: BTreeMap::new(),
            layouts: BTreeMap::new(),
        }
    }

    fn with_session(id: u64, pid: Option<u32>, status: SessionStatus) -> AppState {
        AppState {
            session: Some(Session { id, pid, status }),
            ..state()
        }
    }

    fn running(frame: Option<u64>) -> SessionStatus {
        SessionStatus::Running {
            debug_url: URL.to_string(),
            frame,
            surface: frame.map(|_| SURFACE),
        }
    }

    const SURFACE: [u32; 2] = [800, 600];

    fn polled(frame: u64) -> SessionEvent {
        SessionEvent::Polled(Ok(Polled {
            frame,
            surface: Some(SURFACE),
        }))
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

    fn files(path: &str, files: &[&str]) -> Event {
        Event::Files {
            path: path.to_string(),
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
        assert_eq!(commands, vec![list_b(), broadcast(&next)]);
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
        assert_eq!(commands, vec![poll(1), broadcast(&next)]);
        // The same poll, or a failed request, changes nothing but still polls.
        for event in [
            polled(12),
            SessionEvent::Polled(Err("connection refused".to_string())),
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
        assert_eq!(commands, vec![broadcast(&next)]);
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
        for status in [SessionStatus::Starting, running(Some(12))] {
            let (next, commands) = update(
                with_session(1, Some(9), status),
                client(ClientMsg::StopSession),
            );
            assert_eq!(next, with_session(1, None, SessionStatus::Stopped));
            assert_eq!(
                commands,
                vec![Command::StopSession { id: 1 }, broadcast(&next)]
            );
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
        let before = with_session(1, Some(9), running(Some(12)));
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
                Command::StopSession { id: 1 },
                start_cmd(2, "examples/b", Display::Hidden),
                broadcast(&next)
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
            assert_eq!(commands, vec![list_b(), broadcast(&next)]);
        }
    }

    #[test]
    fn a_new_surface_is_recorded() {
        let before = with_session(1, Some(9), running(Some(12)));
        let resized = SessionEvent::Polled(Ok(Polled {
            frame: 12,
            surface: Some([1024, 768]),
        }));
        let (next, commands) = update(before, session(1, resized));
        let Some(SessionStatus::Running { surface, .. }) = next.session.as_ref().map(|s| &s.status)
        else {
            panic!("{next:?}");
        };
        assert_eq!(*surface, Some([1024, 768]));
        assert_eq!(commands, vec![poll(1), broadcast(&next)]);
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
            for status in [SessionStatus::Starting, running(Some(12))] {
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
                assert_eq!(
                    commands,
                    vec![
                        save_layout_cmd("examples/a", &next.layouts["examples/a"]),
                        Command::StopSession { id: 1 },
                        start_cmd(2, "examples/a", display),
                        broadcast(&next)
                    ],
                    "{from:?} -> {to:?}"
                );
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
        };
        s.layouts.insert("examples/b".to_string(), file_only);
        let (next, _) = update(s, switch("examples/b"));
        assert_eq!(
            crate::print_layout(&next.layouts["examples/b"]),
            ">#1[file:game.fun, *game]"
        );
        // In Mini, none opens.
        let mut s = with_session(1, Some(9), running(Some(12)));
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
            vec![Command::Input {
                id: 3,
                debug_url: URL.to_string(),
                input: key()
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
        assert_eq!(json["state"]["session"]["status"]["frame"], 12);
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

    /// A task that ran once and finished: waiting on the user.
    fn finished_task() -> AppState {
        let (s, _) = update(state(), new_task("What is the count?"));
        let (s, _) = update(s, agent(1, 0, started("sess-1")));
        let (s, _) = update(s, agent(1, 0, finished()));
        update(s, ended(1, 0)).0
    }

    #[test]
    fn a_new_task_starts_a_run_in_the_active_project() {
        let (next, commands) = update(state(), new_task("  What is the count?\nAnd why?"));
        let t = task(&next, 1);
        assert_eq!(t.title, "What is the count?");
        assert_eq!(t.project, "examples/a");
        assert_eq!(t.status, TaskStatus::InProgress);
        let user = Item::User {
            text: "  What is the count?\nAnd why?".to_string(),
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
                    context: AppContext {
                        project: "examples/a".to_string(),
                        session: None,
                        session_mode: SessionMode::Hidden,
                    },
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
    fn agent_events_stream_items_and_ended_waits_on_the_user() {
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
        assert_eq!(t.status, TaskStatus::WaitingOnYou);
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

    #[test]
    fn messages_are_rejected_while_a_run_is_active() {
        let (s, _) = update(state(), new_task("first"));
        let (next, commands) = update(s.clone(), send(1, "more"));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            rejected("task 1 is still running: wait for it to finish or stop it")
        );
        let (next, commands) = update(s.clone(), nth_task(2, "second"));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            rejected("task 1 is running in examples/a: one run per project at a time")
        );
        // Another project may run alongside it.
        let (b, _) = update(s, switch("examples/b"));
        let (b, commands) = update(b, nth_task(2, "in b"));
        assert_eq!(task(&b, 2).project, "examples/b");
        assert!(matches!(commands[0], Command::StartRun(_)), "{commands:?}");
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
        });
        cut.run = Some(Run { id: 3, usage: None });
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
        assert_eq!(task(&s, 4).status, TaskStatus::WaitingOnYou);
    }

    #[test]
    fn a_new_task_takes_the_shells_id_or_is_rejected() {
        let (s, _) = update(state(), nth_task(7, "hi"));
        assert_eq!(s.tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [7]);
        let failed = Event::NewTask {
            id: Err("disk full".to_string()),
            message: "hi".to_string(),
            harness: Harness::Claude,
        };
        let (next, commands) = update(state(), failed);
        assert_eq!(next, state());
        assert_eq!(commands, rejected("cannot reserve a task id: disk full"));
        let bare = client(ClientMsg::NewTask {
            message: "hi".to_string(),
            harness: Harness::Claude,
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
        assert_eq!(kinds, ["stop_session", "stop_run", "save", "broadcast"]);
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
                harness: Harness::Codex
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
        assert_eq!(
            json["state"]["tasks"][0]["status"]["kind"],
            "waiting_on_you"
        );
        assert!(json["state"]["tasks"][0].get("transcript").is_none());
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
    fn a_finished_run_with_a_change_is_ready_for_review() {
        let s = reviewed(one_file(2));
        let t = task(&s, 1);
        assert_eq!(t.status, TaskStatus::ReadyForReview);
        assert_eq!(t.baseline.as_deref(), Some("tree-1"));
        assert_eq!(t.change, Some(one_file(2)));
        // No change: waiting on the user, as without git.
        let s = reviewed(Change::default());
        assert_eq!(task(&s, 1).status, TaskStatus::WaitingOnYou);
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
        // An emptied change (the user undid it by hand) waits on the user.
        let (next, _) = update(s, change(1, "tree-1", Change::default()));
        assert_eq!(task(&next, 1).status, TaskStatus::WaitingOnYou);
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
        let (_, commands) = update(s.clone(), nth_task(2, "other"));
        assert_eq!(commands, rejected("task 1's change is being reverted"));

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
        let untracked = AppState {
            tracks_changes: false,
            ..reviewed(one_file(2))
        };
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
        let s = AppState {
            tasks: vec![other],
            ..state()
        };
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
        };
        // Another project's files are not listed yet, so its file tabs stay.
        let theirs = Layout {
            root: Some(pane(1, vec![file("b.fun")])),
            focus: Some(1),
            game: Default::default(),
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
}
