//! The `--print` text dump of the state.

use crate::projects::Entries;
use crate::{AppState, Display, Item, SessionStatus, Task, TaskStatus};

/// One line per field, then one per project in path order, the active one
/// marked `*`, then one per task:
///
/// ```text
/// repo: /src/functor
/// active: examples/hello
/// session: running pid 4242 http://127.0.0.1:52011 frame 30
/// mode: hidden
/// projects: 2
/// * examples/hello (hello): entry game.fun
///   examples/orbs (orbs): entries client=game.fun module=Client, server=game.fun module=Server
/// tasks: 1
///   #1 waiting_on_you examples/hello "Make the cube spin" (claude): 6 items, 2 tool calls
/// ```
pub fn print(state: &AppState) -> String {
    let active = state.active.as_deref();
    let mut lines = vec![
        format!("repo: {}", state.repo_root),
        format!("active: {}", active.unwrap_or("none")),
        format!("session: {}", session(state)),
        format!("mode: {}", mode(state)),
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
    if !state.tasks.is_empty() {
        lines.push(format!("tasks: {}", state.tasks.len()));
        lines.extend(state.tasks.iter().map(task));
    }
    lines.join("\n") + "\n"
}

fn task(t: &Task) -> String {
    let status = match &t.status {
        TaskStatus::InProgress => "in_progress".to_string(),
        TaskStatus::WaitingOnYou => "waiting_on_you".to_string(),
        TaskStatus::ReadyForReview => "ready_for_review".to_string(),
        TaskStatus::Failed { summary: None } => "failed".to_string(),
        TaskStatus::Failed { summary: Some(s) } => format!("failed ({})", clip(s)),
    };
    let model = t
        .model
        .as_ref()
        .map(|m| format!(" {m}"))
        .unwrap_or_default();
    let tools = t
        .transcript
        .iter()
        .filter(|i| matches!(i, Item::Tool { .. }))
        .count();
    format!(
        "  #{} {status} {} {:?} ({}{model}): {} items, {tools} tool calls",
        t.id,
        t.project,
        t.title,
        t.harness.name(),
        t.transcript.len(),
    )
}

/// One transcript item as a line, as `functor-app task` prints it:
///
/// ```text
/// you: What is the count?
/// tool: mcp__mis__get_state {"session":"s1"} -> {"frame":180,"model":{"count":3}}
/// agent: The game is on frame 180 and the count is 3.
/// usage: 2400 in, 75 out
/// finished: ok
/// ```
pub fn print_item(item: &Item) -> String {
    match item {
        Item::User { text } => format!("you: {text}"),
        Item::Agent { text, .. } => format!("agent: {text}"),
        Item::Tool {
            name,
            input,
            result,
        } => {
            let result = result.as_deref().map_or("(no result)".to_string(), clip);
            format!("tool: {name} {} -> {result}", clip(&input.to_string()))
        }
        Item::Usage { input, output } => format!("usage: {input} in, {output} out"),
        Item::Finished { ok: true, .. } => "finished: ok".to_string(),
        Item::Finished { ok: false, summary } => {
            format!("failed: {}", summary.as_deref().unwrap_or("no summary"))
        }
        Item::Stopped => "stopped".to_string(),
    }
}

/// Longest tool input or result shown, in characters.
const CLIP_CHARS: usize = 200;

/// `text` on one line, cut to [`CLIP_CHARS`] with `…`.
fn clip(text: &str) -> String {
    let line = text.replace('\n', "\\n");
    match line.char_indices().nth(CLIP_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// How the next (or the live) session runs: `hidden`, `window` or `headless`.
fn mode(state: &AppState) -> &'static str {
    match Display::new(state.session_mode, state.no_gpu) {
        Display::Headless => "headless",
        Display::Hidden => "hidden",
        Display::Window => "window",
    }
}

fn session(state: &AppState) -> String {
    let Some(session) = &state.session else {
        return "stopped".to_string();
    };
    let pid = session.pid.map(|p| format!(" pid {p}")).unwrap_or_default();
    match &session.status {
        SessionStatus::Stopped => "stopped".to_string(),
        SessionStatus::Starting => format!("starting{pid}"),
        SessionStatus::Running {
            debug_url, frame, ..
        } => {
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
    use crate::{Session, SessionMode};

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
            files: vec![],
            git: None,
            session: Some(Session {
                id: 1,
                pid: Some(4242),
                status: SessionStatus::Running {
                    debug_url: "http://127.0.0.1:52011".into(),
                    frame: Some(30),
                    surface: Some([800, 600]),
                },
            }),
            session_mode: SessionMode::Hidden,
            no_gpu: false,
            tasks: vec![],
            harnesses: vec![],
            harness_choice: Default::default(),
            next_task: 1,
        };
        assert_eq!(
            print(&state),
            "\
repo: /src/functor
active: examples/hello
session: running pid 4242 http://127.0.0.1:52011 frame 30
mode: hidden
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
            files: vec![],
            git: None,
            session: None,
            session_mode: SessionMode::Window,
            no_gpu: false,
            tasks: vec![],
            harnesses: vec![],
            harness_choice: Default::default(),
            next_task: 1,
        };
        assert_eq!(
            print(&state),
            "repo: /tmp/empty\nactive: none\nsession: stopped\nmode: window\nprojects: 0\n"
        );
        let no_gpu = AppState {
            no_gpu: true,
            ..state
        };
        assert_eq!(print(&no_gpu).lines().nth(3), Some("mode: headless"));
    }

    #[test]
    fn prints_each_session_status() {
        let line = |pid, status| {
            let state = AppState {
                repo_root: "/r".into(),
                projects: vec![],
                active: None,
                files: vec![],
                git: None,
                session: Some(Session { id: 1, pid, status }),
                session_mode: SessionMode::Hidden,
                no_gpu: false,
                tasks: vec![],
                harnesses: vec![],
                harness_choice: Default::default(),
                next_task: 1,
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
                    frame: None,
                    surface: None,
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

    #[test]
    fn prints_tasks_and_items() {
        use crate::tasks::{Harness, Task};
        let transcript = vec![
            Item::User {
                text: "What is the count?".into(),
            },
            Item::Tool {
                name: "mcp__mis__get_state".into(),
                input: serde_json::json!({"session": "s1"}),
                result: Some("{\"frame\":180,\n\"count\":3}".into()),
            },
            Item::Agent {
                text: "It is 3.".into(),
                partial: false,
            },
            Item::Usage {
                input: 2400,
                output: 75,
            },
            Item::Finished {
                ok: false,
                summary: Some("usage limit".into()),
            },
        ];
        let state = AppState {
            repo_root: "/r".into(),
            projects: vec![],
            active: None,
            files: vec![],
            git: None,
            session: None,
            session_mode: SessionMode::Hidden,
            no_gpu: false,
            tasks: vec![Task {
                id: 3,
                project: "examples/counter".into(),
                title: "What is the count?".into(),
                status: TaskStatus::Failed {
                    summary: Some("usage limit".into()),
                },
                harness: Harness::Claude,
                model: Some("claude-model".into()),
                session_id: None,
                run: None,
                transcript: transcript.clone(),
            }],
            harnesses: vec![],
            harness_choice: Default::default(),
            next_task: 4,
        };
        let text = print(&state);
        assert!(
            text.ends_with(
                "tasks: 1\n  #3 failed (usage limit) examples/counter \"What is the count?\" \
                 (claude claude-model): 5 items, 1 tool calls\n"
            ),
            "{text}"
        );
        let lines: Vec<String> = transcript.iter().map(print_item).collect();
        assert_eq!(
            lines,
            [
                "you: What is the count?",
                r#"tool: mcp__mis__get_state {"session":"s1"} -> {"frame":180,\n"count":3}"#,
                "agent: It is 3.",
                "usage: 2400 in, 75 out",
                "failed: usage limit",
            ]
        );
        let long = "x".repeat(300);
        assert_eq!(clip(&long).chars().count(), CLIP_CHARS + 1);
    }
}
