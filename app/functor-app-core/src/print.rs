//! The `--print` text dump of the state.

use crate::changes::Change;
use crate::layout::{Corner, Direction, GamePlacement, Layout, Node, Placement, View};
use crate::projects::Entries;
use crate::{AppState, Display, Item, SessionStatus, Task, TaskStatus};

/// One line per field, then one per project in path order, the active one
/// marked `*`, then one per task:
///
/// ```text
/// repo: /src/functor
/// folder: /src/functor/examples/hello
/// active: examples/hello
/// session: running pid 4242 http://127.0.0.1:52011 frame 30
/// mode: hidden
/// placement: pane
/// layout: right 50% (>#1[*game] | #3[*file:game.fun, changes:1])
/// projects: 2
/// * examples/hello (hello): entry game.fun
///   examples/orbs (orbs): entries client=game.fun module=Client, server=game.fun module=Server
/// tasks: 1
///   #1 ready_for_review examples/hello "Make the cube spin" (claude): 6 items, 2 tool calls
///     change: 1 file, +2 -1
///       examples/hello/game.fun +2 -1
/// recents: 2
///   hello /src/functor/examples/hello
///   old /src/old (missing)
/// ```
///
/// Recents list the first [`RECENTS_SHOWN`].
pub fn print(state: &AppState) -> String {
    let active = state.active.as_deref();
    let mut lines = vec![
        format!("repo: {}", or_none(&state.repo_root)),
        format!("folder: {}", or_none(&state.folder)),
        format!("active: {}", active.unwrap_or("none")),
        format!("session: {}", session(state)),
        format!("mode: {}", mode(state)),
    ];
    if let Some(active) = active {
        let layout = state.layouts.get(active).cloned().unwrap_or_default();
        lines.push(format!("placement: {}", print_placement(&layout.game)));
        lines.push(format!("layout: {}", print_layout(&layout)));
    }
    lines.push(format!("projects: {}", state.projects.len()));
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
        for t in &state.tasks {
            lines.push(task(t));
            if let Some(change) = t.change.as_ref().filter(|c| !c.is_empty()) {
                lines.extend(print_change(change).lines().map(|l| format!("    {l}")));
            }
        }
    }
    if !state.recents.is_empty() {
        lines.push(format!("recents: {}", state.recents.len()));
        for r in state.recents.iter().take(RECENTS_SHOWN) {
            let missing = if r.missing { " (missing)" } else { "" };
            lines.push(format!("  {} {}{missing}", r.name, r.path));
        }
    }
    // Only once the link is on or a device is known.
    if state.devices.listening.is_some() || !state.devices.list.is_empty() {
        lines.extend(print_devices(&state.devices));
    }
    lines.join("\n") + "\n"
}

/// `headset:` with the device link's address, then a line per device:
///
/// ```text
/// headset: connected (link 0.0.0.0:53122)
///   Quest 3 0a1b2c3d: connected over wifi, battery 80%, build abc, running examples/hello, 12 ms; tunnel http://127.0.0.1:53130
///   Quest Pro 9f8e7d6c: asking to pair, code 482 913
/// ```
pub fn print_devices(devices: &crate::Devices) -> Vec<String> {
    use crate::devices::{DeviceState, Headset, LinkKind};
    let word = match devices.headset {
        Headset::NotPaired => "not paired",
        Headset::AskingToPair => "asking to pair",
        Headset::Away => "away",
        Headset::Connected => "connected",
        Headset::NeedsUpdate => "needs an update",
        Headset::CannotReach => "cannot reach",
    };
    let link = devices
        .listening
        .as_deref()
        .map_or("link off".to_string(), |a| format!("link {a}"));
    let mut lines = vec![format!("headset: {word} ({link})")];
    for d in &devices.list {
        let state = match &d.state {
            DeviceState::Away => "away".to_string(),
            DeviceState::Connected {
                link,
                battery,
                build,
                project,
                latency_ms,
                ..
            } => {
                let mut parts = vec![format!(
                    "connected over {}",
                    match link {
                        LinkKind::Wifi => "wifi",
                        LinkKind::Cable => "cable",
                    }
                )];
                parts.extend(battery.map(|b| format!("battery {b}%")));
                parts.extend(build.as_ref().map(|b| format!("build {b}")));
                parts.extend(project.as_ref().map(|p| format!("running {p}")));
                parts.extend(latency_ms.map(|ms| format!("{ms} ms")));
                parts.join(", ")
            }
            DeviceState::NeedsUpdate { reason, .. } => format!("needs an update: {reason}"),
            DeviceState::CannotReach { reason, .. } => format!("cannot reach: {reason}"),
        };
        let tunnel = d
            .tunnel
            .as_ref()
            .map(|t| format!("; tunnel {t}"))
            .unwrap_or_default();
        let short = &d.id[..d.id.len().min(8)];
        lines.push(format!("  {} {short}: {state}{tunnel}", d.name));
    }
    for r in &devices.requests {
        let short = &r.device[..r.device.len().min(8)];
        let (a, b) = r.code.split_at(r.code.len() / 2);
        let allowed = if r.allowed { ", allowed" } else { "" };
        lines.push(format!(
            "  {} {short}: asking to pair, code {a} {b}{allowed}",
            r.name
        ));
    }
    lines
}

/// How many recent projects `print` lists.
pub const RECENTS_SHOWN: usize = 5;

fn or_none(path: &str) -> &str {
    if path.is_empty() {
        "none"
    } else {
        path
    }
}

/// A layout on one line: each pane as `#id[tabs]`, the focused one marked
/// `>` and its shown tab `*`; each split as `right|down N% (first | second)`,
/// N the first child's share; `empty` for no panes.
pub fn print_layout(layout: &Layout) -> String {
    fn node(n: &Node, focus: Option<u64>) -> String {
        match n {
            Node::Pane(p) => {
                let tabs: Vec<String> = p
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let mark = if i == p.active { "*" } else { "" };
                        let view = match v {
                            View::Game => "game".to_string(),
                            View::File { path } => format!("file:{path}"),
                            View::Changes { task } => format!("changes:{task}"),
                            View::Task { task } => format!("task:{task}"),
                        };
                        format!("{mark}{view}")
                    })
                    .collect();
                let mark = if focus == Some(p.id) { ">" } else { "" };
                format!("{mark}#{}[{}]", p.id, tabs.join(", "))
            }
            Node::Split {
                direction,
                ratio,
                first,
                second,
                ..
            } => {
                let dir = match direction {
                    Direction::Right => "right",
                    Direction::Down => "down",
                };
                let (a, b) = (node(first, focus), node(second, focus));
                format!("{dir} {}% ({a} | {b})", ratio / 10)
            }
        }
    }
    match &layout.root {
        Some(root) => node(root, layout.focus),
        None => "empty".to_string(),
    }
}

fn task(t: &Task) -> String {
    let status = match &t.status {
        TaskStatus::InProgress => "in_progress".to_string(),
        TaskStatus::WaitingOnYou => "waiting_on_you".to_string(),
        TaskStatus::ReadyForReview => "ready_for_review".to_string(),
        TaskStatus::Done => "done".to_string(),
        TaskStatus::Failed { summary: None } => "failed".to_string(),
        TaskStatus::Failed { summary: Some(s) } => format!("failed ({})", clip(s)),
        TaskStatus::Kept => "kept".to_string(),
        TaskStatus::Reverted => "reverted".to_string(),
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
        Item::Kept => "kept: the change".to_string(),
        Item::Reverted => "reverted: the change".to_string(),
        Item::Walkthrough { steps, .. } => {
            let titles: Vec<&str> = steps.iter().map(|s| s.title.as_str()).collect();
            format!("walkthrough: {}", titles.join(" → "))
        }
    }
}

/// A change summary, as `functor-app task` prints it at the end:
///
/// ```text
/// change: 3 files, +4 -1
///   examples/counter/game.fun +3 -1
///   examples/counter/old.fun -> examples/counter/new name.fun +1 -0
///   examples/counter/icon.png (binary)
/// ```
pub fn print_change(change: &Change) -> String {
    if change.is_empty() {
        return "change: none\n".to_string();
    }
    let n = change.files.len() + change.more as usize;
    let added: u32 = change.files.iter().map(|f| f.added).sum();
    let removed: u32 = change.files.iter().map(|f| f.removed).sum();
    let plural = if n == 1 { "" } else { "s" };
    let mut out = format!("change: {n} file{plural}, +{added} -{removed}\n");
    for f in &change.files {
        let path = match &f.from {
            Some(from) => format!("{from} -> {}", f.path),
            None => f.path.clone(),
        };
        let counts = match f.binary {
            true => "(binary)".to_string(),
            false => format!("+{} -{}", f.added, f.removed),
        };
        out.push_str(&format!("  {path} {counts}\n"));
    }
    if change.more > 0 {
        out.push_str(&format!("  … and {} more\n", change.more));
    }
    out
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

/// The game view's placement: `pane`, `window`, or `mini` with its corner
/// and width, `mini bottom_right 360`.
fn print_placement(game: &GamePlacement) -> String {
    let corner = match game.mini.corner {
        Corner::TopLeft => "top_left",
        Corner::TopRight => "top_right",
        Corner::BottomLeft => "bottom_left",
        Corner::BottomRight => "bottom_right",
    };
    match game.placement {
        Placement::Pane => "pane".to_string(),
        Placement::Window => "window".to_string(),
        Placement::Mini => format!("mini {corner} {}", game.mini.width),
    }
}

/// How the next (or the live) session runs: `hidden`, `window` or `headless`.
fn mode(state: &AppState) -> &'static str {
    let placement = state
        .active
        .as_deref()
        .map(|a| crate::state::placement(state, a))
        .unwrap_or_default();
    match Display::new(placement.session_mode(), state.no_gpu) {
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
    use crate::changes::FileChange;
    use crate::projects::{Project, Role};
    use crate::Session;

    #[test]
    fn prints_a_change_with_renames_binaries_and_the_rest_counted() {
        let file = |path: &str, added, removed| FileChange {
            path: path.into(),
            from: None,
            added,
            removed,
            binary: false,
            large: false,
        };
        let change = Change {
            files: vec![
                file("a/game.fun", 3, 1),
                FileChange {
                    from: Some("a/old.fun".into()),
                    ..file("a/new name.fun", 1, 0)
                },
                FileChange {
                    binary: true,
                    ..file("a/icon.png", 0, 0)
                },
            ],
            more: 2,
            tree: "t".into(),
        };
        assert_eq!(
            print_change(&change),
            "change: 5 files, +4 -1\n  a/game.fun +3 -1\n  a/old.fun -> a/new name.fun +1 -0\n  \
             a/icon.png (binary)\n  … and 2 more\n"
        );
        assert_eq!(print_change(&Change::default()), "change: none\n");
    }

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
            tracks_changes: false,
            session: Some(Session {
                id: 1,
                pid: Some(4242),
                status: SessionStatus::Running {
                    debug_url: "http://127.0.0.1:52011".into(),
                    frame: Some(30),
                    surface: Some([800, 600]),
                },
            }),
            no_gpu: false,
            tasks: vec![],
            harnesses: vec![],
            harness_choice: Default::default(),
            layouts: [("examples/hello".to_string(), split_layout())].into(),
            folder: "/src/functor/examples/hello".to_string(),
            recents: vec![
                crate::Recent {
                    name: "hello".into(),
                    path: "/src/functor/examples/hello".into(),
                    missing: false,
                },
                crate::Recent {
                    name: "old".into(),
                    path: "/src/old".into(),
                    missing: true,
                },
            ],
            native_picker: false,
            open_error: None,
            opening: 0,
            templates: vec![],
            templates_error: None,
            new_project: None,
            last_parent: None,
            first_task: None,
            ..AppState::none()
        };
        assert_eq!(
            print(&state),
            "\
repo: /src/functor
folder: /src/functor/examples/hello
active: examples/hello
session: running pid 4242 http://127.0.0.1:52011 frame 30
mode: hidden
placement: pane
layout: right 60% (#1[game, *file:game.fun] | down 50% (>#2[*changes:2, task:2] | #5[]))
projects: 3
* examples/hello (hello): entry game.fun
  examples/orbs (orbs): entries client=game.fun module=Client, server=server.fun
  old (old): invalid: bad
recents: 2
  hello /src/functor/examples/hello
  old /src/old (missing)
"
        );
    }

    fn split_layout() -> Layout {
        use crate::layout::Pane;
        let pane = |id, tabs: Vec<View>, active| Node::Pane(Pane { id, tabs, active });
        Layout {
            root: Some(Node::Split {
                id: 4,
                direction: Direction::Right,
                ratio: 600,
                first: Box::new(pane(
                    1,
                    vec![
                        View::Game,
                        View::File {
                            path: "game.fun".into(),
                        },
                    ],
                    1,
                )),
                second: Box::new(Node::Split {
                    id: 6,
                    direction: Direction::Down,
                    ratio: 500,
                    first: Box::new(pane(
                        2,
                        vec![View::Changes { task: 2 }, View::Task { task: 2 }],
                        0,
                    )),
                    second: Box::new(pane(5, vec![], 0)),
                }),
            }),
            focus: Some(2),
            game: GamePlacement::default(),
            agent: Default::default(),
        }
    }

    #[test]
    fn prints_layouts() {
        assert_eq!(print_layout(&Layout::initial()), ">#1[*game]");
        assert_eq!(
            print_layout(&Layout {
                root: None,
                focus: None,
                game: GamePlacement::default(),
                agent: Default::default(),
            }),
            "empty"
        );
    }

    #[test]
    fn prints_the_placement() {
        let mut game = GamePlacement::default();
        assert_eq!(print_placement(&game), "pane");
        game.placement = Placement::Window;
        assert_eq!(print_placement(&game), "window");
        game.placement = Placement::Mini;
        game.mini.corner = Corner::TopLeft;
        game.mini.width = 480;
        assert_eq!(print_placement(&game), "mini top_left 480");
    }

    #[test]
    fn prints_none_without_projects() {
        let state = AppState {
            repo_root: "/tmp/empty".into(),
            projects: vec![],
            active: None,
            files: vec![],
            git: None,
            tracks_changes: false,
            session: None,
            no_gpu: false,
            tasks: vec![],
            harnesses: vec![],
            harness_choice: Default::default(),
            layouts: Default::default(),
            ..AppState::none()
        };
        assert_eq!(
            print(&state),
            "repo: /tmp/empty\nfolder: none\nactive: none\nsession: stopped\nmode: hidden\nprojects: 0\n"
        );
        let no_gpu = AppState {
            no_gpu: true,
            ..state
        };
        assert_eq!(print(&no_gpu).lines().nth(4), Some("mode: headless"));
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
                tracks_changes: false,
                session: Some(Session { id: 1, pid, status }),
                no_gpu: false,
                tasks: vec![],
                harnesses: vec![],
                harness_choice: Default::default(),
                layouts: Default::default(),
                ..AppState::none()
            };
            print(&state).lines().nth(3).unwrap().to_string()
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
            tracks_changes: false,
            session: None,
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
                baseline: Some("tree".into()),
                change: Some(Change::from_files(
                    vec![FileChange {
                        path: "examples/counter/game.fun".into(),
                        from: None,
                        added: 2,
                        removed: 1,
                        binary: false,
                        large: false,
                    }],
                    "t".into(),
                )),
                reverting: false,
                created_at: None,
                updated_at: None,
                transcript: transcript.clone(),
            }],
            harnesses: vec![],
            harness_choice: Default::default(),
            layouts: Default::default(),
            ..AppState::none()
        };
        let text = print(&state);
        assert!(
            text.ends_with(
                "tasks: 1\n  #3 failed (usage limit) examples/counter \"What is the count?\" \
                 (claude claude-model): 5 items, 1 tool calls\n    \
                 change: 1 file, +2 -1\n      examples/counter/game.fun +2 -1\n"
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
        assert_eq!(print_item(&Item::Kept), "kept: the change");
        assert_eq!(print_item(&Item::Reverted), "reverted: the change");
        let long = "x".repeat(300);
        assert_eq!(clip(&long).chars().count(), CLIP_CHARS + 1);
    }

    #[test]
    fn prints_devices_and_their_states() {
        use crate::devices::{on_link, LinkEvent, LinkKind, Paired};
        let mut d = crate::Devices::default();
        let paired = vec![Paired {
            id: "0a1b2c3d4e5f".into(),
            name: "Quest 3".into(),
            first_seen: 1,
            last_seen: 1,
        }];
        let listening = Some("0.0.0.0:53122".into());
        on_link(&mut d, LinkEvent::Started { listening, paired }, 0);
        let tunnel = LinkEvent::TunnelOpened {
            device: "0a1b2c3d4e5f".into(),
            url: "http://127.0.0.1:53130".into(),
        };
        on_link(&mut d, tunnel, 0);
        let connected = LinkEvent::Connected {
            conn: 1,
            device: "0a1b2c3d4e5f".into(),
            name: "Quest 3".into(),
            app_version: "0.1.0".into(),
            link: LinkKind::Wifi,
        };
        on_link(&mut d, connected, 0);
        let status = LinkEvent::Status {
            conn: 1,
            battery: Some(80),
            charging: false,
            build: "abc".into(),
            project: Some("examples/hello".into()),
        };
        on_link(&mut d, status, 0);
        on_link(&mut d, LinkEvent::Latency { conn: 1, ms: 12 }, 0);
        let ask = LinkEvent::PairRequest {
            request: 2,
            device: "9f8e7d6c5b".into(),
            name: "Quest Pro".into(),
            app_version: "0.1.0".into(),
            code: "482913".into(),
            expires_at: 9,
        };
        on_link(&mut d, ask, 0);
        assert_eq!(
            print_devices(&d).join("\n"),
            "headset: asking to pair (link 0.0.0.0:53122)\n  \
             Quest 3 0a1b2c3d: connected over wifi, battery 80%, build abc, running examples/hello, 12 ms; tunnel http://127.0.0.1:53130\n  \
             Quest Pro 9f8e7d6c: asking to pair, code 482 913"
        );
    }
}
