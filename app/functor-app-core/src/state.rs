//! `AppState` and the pure `update` that steps it.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::projects::{nearest_project, relative, Project};
use crate::protocol::{ClientMsg, ServerMsg};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppState {
    /// Absolute path of the repo root.
    pub repo_root: String,
    /// Sorted by `path`.
    pub projects: Vec<Project>,
    /// The active project's `path`; `None` only when there are no projects.
    pub active: Option<String>,
}

impl AppState {
    /// The state for `projects` under `root`, with the project nearest
    /// `start` (a directory under `root`) active.
    pub fn new(root: &Path, start: &Path, projects: Vec<Project>) -> Self {
        let paths: Vec<&str> = projects.iter().map(|p| p.path.as_str()).collect();
        let active = nearest_project(&paths, &relative(root, start)).map(str::to_string);
        Self {
            repo_root: root.to_string_lossy().into_owned(),
            projects,
            active,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Client(ClientMsg),
}

/// What the shell does after an `update`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// To the client whose message this answers.
    Reply(ServerMsg),
    /// To every connected client.
    Broadcast(ServerMsg),
}

pub fn update(mut state: AppState, event: Event) -> (AppState, Vec<Command>) {
    let changed = match event {
        Event::Client(ClientMsg::GetState) => false,
        Event::Client(ClientMsg::SetActiveProject { path }) => {
            let switch = state.active.as_ref() != Some(&path)
                && state.projects.iter().any(|p| p.path == path);
            if switch {
                state.active = Some(path);
            }
            switch
        }
    };
    // An unknown or unchanged switch still answers with the state, so the
    // sender sees what is active.
    let msg = ServerMsg::State {
        state: state.clone(),
    };
    let command = if changed {
        Command::Broadcast(msg)
    } else {
        Command::Reply(msg)
    };
    (state, vec![command])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::Entries;

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
        }
    }

    fn switch(path: &str) -> Event {
        Event::Client(ClientMsg::SetActiveProject {
            path: path.to_string(),
        })
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
    fn switching_to_a_known_project_broadcasts_it() {
        let (next, commands) = update(state(), switch("examples/b"));
        assert_eq!(next.active.as_deref(), Some("examples/b"));
        assert_eq!(
            commands,
            vec![Command::Broadcast(ServerMsg::State { state: next })]
        );
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
        let json = serde_json::to_value(ServerMsg::State { state: state() }).unwrap();
        assert_eq!(json["type"], "state");
        assert_eq!(json["state"]["active"], "examples/a");
    }
}
