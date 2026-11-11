//! `AppState` and the pure `update` that steps it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::projects::{default_project, project_dir, relative, Project};
use crate::protocol::{ClientMsg, GameInput, ServerMsg};
use crate::session::{Display, Session, SessionEvent, SessionMode, SessionStatus};
use crate::tasks::{
    apply, derive_status, end_run, stop_run, title, Harness, Item, Run, RunEvent, Task, TaskStatus,
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
    /// The active project's game session; `None` until one first starts.
    pub session: Option<Session>,
    /// Where sessions run; a change restarts a live one.
    pub session_mode: SessionMode,
    /// Sessions run `--headless` (`functor-app --no-gpu`), so there are no frames.
    pub no_gpu: bool,
    /// Every project's tasks, by id; their transcripts travel separately.
    pub tasks: Vec<Task>,
    /// The agents this machine can run, in the order the picker lists them.
    pub harnesses: Vec<Harness>,
    /// Per project path, the agent last picked for its new tasks.
    pub harness_choice: BTreeMap<String, Harness>,
    /// The next new task's id: above every id on disk, readable or not, so
    /// no new task overwrites a file the store skipped.
    #[serde(skip)]
    pub next_task: u64,
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
        Self {
            repo_root: root.to_string_lossy().into_owned(),
            projects,
            active,
            files: Vec::new(),
            git: None,
            session: None,
            session_mode: SessionMode::default(),
            no_gpu: false,
            tasks: Vec::new(),
            harnesses: Vec::new(),
            harness_choice: BTreeMap::new(),
            next_task: 1,
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
    /// The saved tasks, read at startup; ids are unique. `highest` is the
    /// highest id of any task file, including ones that could not be read.
    TasksLoaded {
        tasks: Vec<Task>,
        highest: u64,
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
        Event::Client(ClientMsg::SetSessionMode { mode }) => {
            let switch = state.session_mode != mode;
            if switch {
                state.session_mode = mode;
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
        Event::Session { id, event } => (on_session(&mut state, id, event, &mut commands), false),
        // A listing for a project switched away from is dropped.
        Event::Files { path, files } => {
            let current = state.active.as_ref() == Some(&path) && state.files != files;
            if current {
                state.files = files;
            }
            (current, false)
        }
        // Task messages that change nothing answer with `Rejected`, not the state.
        Event::Client(ClientMsg::NewTask { message, harness }) => {
            (new_task(&mut state, message, harness, &mut commands), false)
        }
        Event::Client(ClientMsg::SendMessage { task, message }) => (
            send_message(&mut state, task, message, &mut commands),
            false,
        ),
        Event::Client(ClientMsg::StopTask { task }) => {
            (stop_task(&mut state, task, &mut commands), false)
        }
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
        Event::TasksLoaded { tasks, highest } => {
            state.tasks = tasks
                .into_iter()
                .map(|mut t| {
                    // A run saved in flight died with the app.
                    t.run = None;
                    t.status = derive_status(&t);
                    t
                })
                .collect();
            state.tasks.sort_by_key(|t| t.id);
            let loaded = state.tasks.last().map_or(0, |t| t.id);
            state.next_task = highest.max(loaded) + 1;
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
fn start(state: &mut AppState, commands: &mut Vec<Command>) -> bool {
    let Some(path) = &state.active else {
        return false;
    };
    let id = state.session.as_ref().map_or(1, |s| s.id + 1);
    let dir = project_dir(Path::new(&state.repo_root), path);
    let display = Display::new(state.session_mode, state.no_gpu);
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

fn reject(commands: &mut Vec<Command>, reason: String) -> bool {
    commands.push(Command::Reply(ServerMsg::Rejected { reason }));
    false
}

fn no_task(id: u64) -> ServerMsg {
    ServerMsg::Rejected {
        reason: format!("there is no task {id}"),
    }
}

/// Starts a task in the active project with `message`.
fn new_task(
    state: &mut AppState,
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
    let id = state.next_task;
    state.next_task += 1;
    state.tasks.push(Task {
        id,
        project,
        title: title(&message),
        status: TaskStatus::InProgress,
        harness,
        model: None,
        session_id: None,
        run: None,
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
        None => Ok(()),
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
    let session_mode = state.session_mode;
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
    commands.push(save(&root, task));
    broadcast_items(commands, task, changed);
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
            session: None,
            session_mode: SessionMode::Hidden,
            no_gpu: false,
            tasks: vec![],
            harnesses: vec![Harness::Claude],
            harness_choice: BTreeMap::new(),
            next_task: 1,
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

    #[test]
    fn the_mode_and_gpu_choose_the_display() {
        for (mode, no_gpu, display) in [
            (SessionMode::Hidden, false, Display::Hidden),
            (SessionMode::Window, false, Display::Window),
            (SessionMode::Hidden, true, Display::Headless),
            (SessionMode::Window, true, Display::Headless),
        ] {
            let before = AppState {
                session_mode: mode,
                no_gpu,
                ..state()
            };
            let (_, commands) = update(before, client(ClientMsg::StartSession));
            assert_eq!(commands[0], start_cmd(1, "examples/a", display));
        }
    }

    #[test]
    fn switching_mode_restarts_a_live_session_in_the_new_mode() {
        let window = || ClientMsg::SetSessionMode {
            mode: SessionMode::Window,
        };
        for status in [SessionStatus::Starting, running(Some(12))] {
            let (next, commands) = update(with_session(1, Some(9), status), client(window()));
            assert_eq!(next.session_mode, SessionMode::Window);
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
                    Command::StopSession { id: 1 },
                    start_cmd(2, "examples/a", Display::Window),
                    broadcast(&next)
                ]
            );
        }
        // Without a live session the mode is only recorded, for the next start.
        let (next, commands) = update(state(), client(window()));
        assert_eq!(next.session_mode, SessionMode::Window);
        assert_eq!(next.session, None);
        assert_eq!(commands, vec![broadcast(&next)]);
        // The current mode again only replies.
        let (again, commands) = update(next.clone(), client(window()));
        assert_eq!(again, next);
        assert_eq!(
            commands,
            vec![Command::Reply(ServerMsg::State {
                state: Box::new(next)
            })]
        );
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
            serde_json::from_str(r#"{"type":"set_session_mode","mode":"window"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::SetSessionMode {
                mode: SessionMode::Window
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

    fn new_task(message: &str) -> Event {
        client(ClientMsg::NewTask {
            message: message.to_string(),
            harness: Harness::Claude,
        })
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
        let (next, commands) = update(s.clone(), new_task("second"));
        assert_eq!(next, s);
        assert_eq!(
            commands,
            rejected("task 1 is running in examples/a: one run per project at a time")
        );
        // Another project may run alongside it.
        let (b, _) = update(s, switch("examples/b"));
        let (b, commands) = update(b, new_task("in b"));
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
    fn loaded_tasks_lose_their_runs_and_new_ids_follow_them() {
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
                highest: 3,
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
        let (s, _) = update(s, new_task("next"));
        assert_eq!(s.tasks.last().unwrap().id, 5);
        // An unreadable file's id is never reused.
        let (s, _) = update(
            state(),
            Event::TasksLoaded {
                tasks: vec![],
                highest: 7,
            },
        );
        let (s, _) = update(s, new_task("next"));
        assert_eq!(s.tasks.last().unwrap().id, 8);
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
}
