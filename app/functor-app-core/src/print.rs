//! The `--print` text dump of the state.

use crate::projects::Entries;
use crate::{AppState, SessionStatus};

/// One line per field, then one per project in path order, the active one
/// marked `*`:
///
/// ```text
/// repo: /src/functor
/// active: examples/hello
/// session: running pid 4242 http://127.0.0.1:52011 frame 30
/// projects: 2
/// * examples/hello (hello): entry game.fun
///   examples/orbs (orbs): entries client=game.fun module=Client, server=game.fun module=Server
/// ```
pub fn print(state: &AppState) -> String {
    let active = state.active.as_deref();
    let mut lines = vec![
        format!("repo: {}", state.repo_root),
        format!("active: {}", active.unwrap_or("none")),
        format!("session: {}", session(state)),
        format!("projects: {}", state.projects.len()),
    ];
    for p in &state.projects {
        let mark = if Some(p.path.as_str()) == active {
            '*'
        } else {
            ' '
        };
        let entries = match &p.entries {
            Entries::Single { file } => format!("entry {file}"),
            Entries::Named { roles } => {
                let roles: Vec<String> = roles
                    .iter()
                    .map(|r| {
                        let module = r.module.as_ref().map(|m| format!(" module={m}"));
                        let prefix = r.prefix.as_ref().map(|p| format!(" prefix={p}"));
                        format!(
                            "{}={}{}{}",
                            r.name,
                            r.file,
                            module.unwrap_or_default(),
                            prefix.unwrap_or_default()
                        )
                    })
                    .collect();
                format!("entries {}", roles.join(", "))
            }
            Entries::Invalid { error } => format!("invalid: {error}"),
        };
        lines.push(format!("{mark} {} ({}): {entries}", p.path, p.name));
    }
    lines.join("\n") + "\n"
}

fn session(state: &AppState) -> String {
    let Some(session) = &state.session else {
        return "stopped".to_string();
    };
    let pid = session.pid.map(|p| format!(" pid {p}")).unwrap_or_default();
    match &session.status {
        SessionStatus::Stopped => "stopped".to_string(),
        SessionStatus::Starting => format!("starting{pid}"),
        SessionStatus::Running { debug_url, frame } => {
            let frame = frame.map_or("not polled".to_string(), |f| format!("frame {f}"));
            format!("running{pid} {debug_url} {frame}")
        }
        SessionStatus::Exited { reason } => format!("exited: {reason}"),
        SessionStatus::Failed { reason } => format!("failed: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{Project, Role};
    use crate::Session;

    #[test]
    fn prints_projects_with_the_active_one_marked() {
        let state = AppState {
            repo_root: "/src/functor".to_string(),
            projects: vec![
                Project {
                    name: "hello".into(),
                    path: "examples/hello".into(),
                    entries: Entries::Single {
                        file: "game.fun".into(),
                    },
                },
                Project {
                    name: "orbs".into(),
                    path: "examples/orbs".into(),
                    entries: Entries::Named {
                        roles: vec![
                            Role {
                                name: "client".into(),
                                file: "game.fun".into(),
                                module: Some("Client".into()),
                                prefix: None,
                            },
                            Role {
                                name: "server".into(),
                                file: "server.fun".into(),
                                module: None,
                                prefix: None,
                            },
                        ],
                    },
                },
                Project {
                    name: "old".into(),
                    path: "old".into(),
                    entries: Entries::Invalid {
                        error: "bad".into(),
                    },
                },
            ],
            active: Some("examples/hello".into()),
            session: Some(Session {
                id: 1,
                pid: Some(4242),
                status: SessionStatus::Running {
                    debug_url: "http://127.0.0.1:52011".into(),
                    frame: Some(30),
                },
            }),
        };
        assert_eq!(
            print(&state),
            "\
repo: /src/functor
active: examples/hello
session: running pid 4242 http://127.0.0.1:52011 frame 30
projects: 3
* examples/hello (hello): entry game.fun
  examples/orbs (orbs): entries client=game.fun module=Client, server=server.fun
  old (old): invalid: bad
"
        );
    }

    #[test]
    fn prints_none_without_projects() {
        let state = AppState {
            repo_root: "/tmp/empty".into(),
            projects: vec![],
            active: None,
            session: None,
        };
        assert_eq!(
            print(&state),
            "repo: /tmp/empty\nactive: none\nsession: stopped\nprojects: 0\n"
        );
    }

    #[test]
    fn prints_each_session_status() {
        let line = |pid, status| {
            let state = AppState {
                repo_root: "/r".into(),
                projects: vec![],
                active: None,
                session: Some(Session { id: 1, pid, status }),
            };
            print(&state).lines().nth(2).unwrap().to_string()
        };
        let reason = || "exit status: 1: error: bad".to_string();
        assert_eq!(line(None, SessionStatus::Stopped), "session: stopped");
        assert_eq!(
            line(Some(7), SessionStatus::Starting),
            "session: starting pid 7"
        );
        assert_eq!(
            line(
                Some(7),
                SessionStatus::Running {
                    debug_url: "http://127.0.0.1:1".into(),
                    frame: None
                }
            ),
            "session: running pid 7 http://127.0.0.1:1 not polled"
        );
        assert_eq!(
            line(None, SessionStatus::Exited { reason: reason() }),
            "session: exited: exit status: 1: error: bad"
        );
        assert_eq!(
            line(None, SessionStatus::Failed { reason: reason() }),
            "session: failed: exit status: 1: error: bad"
        );
    }
}
