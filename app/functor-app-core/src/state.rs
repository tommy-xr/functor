//! `AppState` and the pure `update` that steps it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::projects::{default_project, project_dir, relative, Project};
use crate::protocol::{ClientMsg, GameInput, ServerMsg};
use crate::session::{Display, Session, SessionEvent, SessionMode, SessionStatus};

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
        Event::Session { id, event } => (on_session(&mut state, id, event, &mut commands), false),
        // A listing for a project switched away from is dropped.
        Event::Files { path, files } => {
            let current = state.active.as_ref() == Some(&path) && state.files != files;
            if current {
                state.files = files;
            }
            (current, false)
        }
    };
    // A client message that changed nothing still answers with the state, so
    // the sender sees what is active.
    let msg = || ServerMsg::State {
        state: state.clone(),
    };
    if changed {
        commands.push(Command::Broadcast(msg()));
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
            state: state.clone(),
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
            vec![Command::Reply(ServerMsg::State { state: state() })]
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
                vec![Command::Reply(ServerMsg::State { state: state() })]
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
                vec![Command::Reply(ServerMsg::State { state: before })]
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
            vec![Command::Reply(ServerMsg::State { state: next })]
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
            vec![Command::Reply(ServerMsg::State { state: next })]
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
            state: with_session(1, Some(9), running(Some(12))),
        })
        .unwrap();
        assert_eq!(json["type"], "state");
        assert_eq!(json["state"]["active"], "examples/a");
        assert_eq!(json["state"]["session"]["status"]["kind"], "running");
        assert_eq!(json["state"]["session"]["status"]["frame"], 12);
    }
}
