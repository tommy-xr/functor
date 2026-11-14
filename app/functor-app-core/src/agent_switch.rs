//! The agent switches the open project when the user asks (D42): its
//! `list_projects` and `open_project` requests, their gate, and the chat's
//! "Switched to … — asked by you" line with its Undo.
//!
//! A switch is the project picker's, so all that follows a manual one
//! follows it. The asking task stays in its project; its run is told to
//! answer and stop.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::projects::project_dir;
use crate::state::{can_open_now, switch_active, try_open, AppState, Command};
use crate::tasks::Task;
use crate::voice;

/// A switch is taken as asked for only this soon (ms) after the user's
/// message started the run.
pub const ASKED_WITHIN_MS: u64 = 10 * 60 * 1000;
/// Most projects `list_projects` names.
const MAX_LISTED: usize = 60;

/// The agent's switch, shown in the chat with an Undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentSwitch {
    /// The task that asked, and its title.
    pub task: u64,
    pub title: String,
    /// Its project, and that project's directory (absolute), for Undo.
    pub from: String,
    pub from_dir: String,
    /// The project opened (a path in the open folder) or, from another
    /// folder, its directory.
    pub to: String,
    /// Another folder, opened when the asking run ends.
    pub waiting: bool,
}

/// Where a path `list_projects` gave goes.
enum Dest {
    /// A project in the open folder.
    Here(String),
    /// A recent project's directory in another folder.
    Folder(String),
}

/// `project`'s directory as the recent projects spell it.
fn dir_of(state: &AppState, project: &str) -> String {
    let dir: PathBuf = project_dir(Path::new(&state.repo_root), project)
        .components()
        .collect();
    dir.to_string_lossy().into_owned()
}

/// Recent projects that are not in the open folder and are still there.
fn elsewhere(state: &AppState) -> impl Iterator<Item = &str> {
    let here: Vec<String> = state.projects.iter().map(|p| dir_of(state, &p.path)).collect();
    (state.recents.iter())
        .filter(move |r| !r.missing && !here.contains(&r.path))
        .map(|r| r.path.as_str())
}

/// Run `run` of task `id`, while it runs; never a review's.
fn caller(state: &AppState, id: u64, run: u64, review: bool) -> Result<&Task, String> {
    if review {
        return Err("a review does not switch projects".to_string());
    }
    (state.tasks.iter())
        .find(|t| t.id == id && t.run.as_ref().is_some_and(|r| r.id == run))
        .ok_or_else(|| "this run has ended".to_string())
}

/// `list_projects`: the open folder's projects, the open one marked, then
/// recent ones elsewhere. Changes nothing.
pub(crate) fn list(
    state: &AppState,
    id: u64,
    run: u64,
    review: bool,
) -> Result<(String, bool), String> {
    let task = caller(state, id, run, review)?;
    let mut out = format!(
        "Projects in {} (this task's is {}):\n",
        state.repo_root, task.project
    );
    for p in state.projects.iter().take(MAX_LISTED) {
        let open = match state.active.as_deref() == Some(p.path.as_str()) {
            true => " (open)",
            false => "",
        };
        let _ = writeln!(out, "{}{open}", p.path);
    }
    if let Some(more) = state.projects.len().checked_sub(MAX_LISTED).filter(|n| *n > 0) {
        let _ = writeln!(out, "… and {more} more");
    }
    let recent: Vec<&str> = elsewhere(state).collect();
    if !recent.is_empty() {
        let _ = writeln!(out, "Recent, in other folders:\n{}", recent.join("\n"));
    }
    out.push_str("Switch with `open_project` and one of these paths, only when the user asked to.");
    Ok((out, false))
}

/// `open_project`: switches the app to `path` (one `list_projects` gave)
/// when the gate allows; another folder opens when the run ends. What
/// happened, and whether the state changed.
pub(crate) fn open(
    state: &mut AppState,
    id: u64,
    run: u64,
    review: bool,
    path: &str,
    commands: &mut Vec<Command>,
) -> Result<(String, bool), String> {
    let nothing = |e: String| format!("{e}: nothing was switched");
    let task = caller(state, id, run, review).map_err(nothing)?;
    let (from, title) = (task.project.clone(), task.title.clone());
    let r = task.run.as_ref().expect("running");
    if r.switched {
        return Err(nothing("this run has switched the project once, the most it may".into()));
    }
    if state.clock.saturating_sub(r.started_at) > ASKED_WITHIN_MS {
        let minutes = ASKED_WITHIN_MS / 60_000;
        return Err(nothing(format!(
            "the user's request is more than {minutes} minutes old; switch only when they ask again"
        )));
    }
    if state.active.as_deref() != Some(from.as_str()) {
        let other = state.active.as_deref().unwrap_or("nothing");
        return Err(nothing(format!("the user has {other} open, not {from}")));
    }
    if !state.layouts.get(&from).is_none_or(|l| l.agent.arranges) {
        return Err(nothing(
            "the user has turned off \"Let the agent arrange views\" for this project".into(),
        ));
    }
    if state.new_project.as_ref().is_some_and(|np| np.step.busy()) {
        return Err(nothing("a new project is being created".into()));
    }
    let dest = resolve(state, path).map_err(nothing)?;
    let from_dir = dir_of(state, &from);
    let (to, waiting) = match &dest {
        Dest::Here(to) if *to == from => return Err(nothing(format!("{to} is already open"))),
        Dest::Here(to) => (to.clone(), false),
        Dest::Folder(dir) => {
            let busy = (state.tasks.iter()).find(|t| t.id != id && (t.run.is_some() || t.reverting));
            if let Some(t) = busy {
                return Err(nothing(format!(
                    "task {} is running, and another folder opens only when no task runs",
                    t.id
                )));
            }
            (dir.clone(), true)
        }
    };
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("found");
    task.run.as_mut().expect("running").switched = true;
    state.agent_switch = Some(AgentSwitch {
        task: id,
        title,
        from: from.clone(),
        from_dir,
        to: to.clone(),
        waiting,
    });
    let tell = "Tell the user so in one sentence and stop: your game tools no longer reach a game.";
    let (said, message) = match waiting {
        false => {
            switch_active(state, to.clone(), commands);
            // The chat there starts a new task.
            state.current_task.insert(to.clone(), None);
            (
                format!("Switched to {to}"),
                format!(
                    "The app now shows {to}. This task stays with {from}; say or type the next request to work in {to}. {tell}"
                ),
            )
        }
        true => {
            let name = Path::new(&to).file_name().map_or(to.clone(), |n| n.to_string_lossy().into());
            (
                format!("Opening {name}"),
                format!(
                    "The app opens {to} when this run ends. This task stays with {from}; say or type the next request to work in {name}. {tell}"
                ),
            )
        }
    };
    voice::switched(state, id, &said, commands);
    Ok((message, true))
}

/// What `path` names: a project here, or a recent one elsewhere.
fn resolve(state: &AppState, path: &str) -> Result<Dest, String> {
    if state.projects.iter().any(|p| p.path == path) {
        return Ok(Dest::Here(path.to_string()));
    }
    // A recent project in the open folder is one of its projects.
    if let Some(p) = state.projects.iter().find(|p| dir_of(state, &p.path) == path) {
        return Ok(Dest::Here(p.path.clone()));
    }
    match elsewhere(state).any(|r| r == path) {
        true => Ok(Dest::Folder(path.to_string())),
        false => Err(format!(
            "{path} is not a path `list_projects` gives: call it and use one of those"
        )),
    }
}

/// Task `id`'s run ended: the other folder its switch waits on opens, if
/// it can now. Whether the state changed.
pub(crate) fn run_ended(state: &mut AppState, id: u64, commands: &mut Vec<Command>) -> bool {
    let Some(switch) = (state.agent_switch.as_mut()).filter(|s| s.waiting && s.task == id) else {
        return false;
    };
    switch.waiting = false;
    let path = switch.to.clone();
    let open = can_open_now(state).map(|()| Command::OpenFolder {
        id: state.opening + 1,
        path,
    });
    // Refused: the reason shows where folders' troubles do.
    if open.is_err() {
        state.agent_switch = None;
    }
    try_open(state, open, commands);
    true
}

/// The user's Undo: back to the project the agent was asked from, showing
/// the task that asked; or, from another folder, that project's folder.
/// One still waiting to open is cancelled.
pub(crate) fn undo(state: &mut AppState, commands: &mut Vec<Command>) -> Result<bool, String> {
    let switch = state.agent_switch.take().ok_or("there is no switch to undo")?;
    if switch.waiting {
        return Ok(true);
    }
    let here = (state.projects.iter())
        .any(|p| p.path == switch.from && dir_of(state, &p.path) == switch.from_dir);
    if here {
        switch_active(state, switch.from.clone(), commands);
        if state.tasks.iter().any(|t| t.id == switch.task) {
            state.current_task.insert(switch.from, Some(switch.task));
        }
        return Ok(true);
    }
    let open = can_open_now(state).map(|()| Command::OpenFolder {
        id: state.opening + 1,
        path: switch.from_dir,
    });
    try_open(state, open, commands);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_view::{ViewReply, ViewRequest};
    use crate::folders::Recent;
    use crate::layout::{Layout, LayoutOp, Placement};
    use crate::protocol::ClientMsg;
    use crate::session::Display;
    use crate::state::tests::{
        agent, client, ended, finished, stages, state, task, voice_ready, voice_task_running,
    };
    use crate::state::{update, Event};
    use crate::tasks::Harness;
    use crate::voice::Stage;

    const A: &str = "examples/a";
    const B: &str = "examples/b";

    /// Examples/a open with task 1 running (run 0); a recent project in
    /// another folder and one gone.
    fn asking() -> AppState {
        let mut s = state();
        s.start_on_open = true;
        s.recents = [("/repo/examples/a", false), ("/other/kart", false), ("/gone", true)]
            .map(|(path, missing)| Recent {
                name: path.rsplit('/').next().unwrap().to_string(),
                path: path.to_string(),
                missing,
            })
            .to_vec();
        let new = Event::NewTask {
            id: Ok(1),
            message: "switch to b".to_string(),
            harness: Harness::Claude,
            voice: None,
            scope: None,
        };
        update(s, new).0
    }

    fn request(task: u64, review: bool, request: ViewRequest) -> Event {
        Event::AgentView {
            task,
            run: 0,
            review,
            request,
        }
    }

    fn open(path: &str) -> Event {
        let path = path.to_string();
        request(1, false, ViewRequest::OpenProject { path })
    }

    fn reply(commands: &[Command]) -> ViewReply {
        let mut replies = commands.iter().filter_map(|c| match c {
            Command::ViewReply(r) => Some(r.clone()),
            _ => None,
        });
        replies.next().expect("a reply")
    }

    /// The reply to `event` on `s`, which must refuse and change nothing.
    fn refused(s: AppState, event: Event) -> String {
        let (after, commands) = update(s.clone(), event);
        let r = reply(&commands);
        assert!(!r.ok, "{r:?}");
        assert_eq!((after.active, after.agent_switch), (s.active, None));
        r.message
    }

    #[test]
    fn the_list_names_the_folders_projects_and_recent_ones_elsewhere() {
        let (_, commands) = update(asking(), request(1, false, ViewRequest::ListProjects));
        let r = reply(&commands);
        assert!(r.ok, "{r:?}");
        assert_eq!(
            r.message,
            "Projects in /repo (this task's is examples/a):\nexamples/a (open)\nexamples/b\n\
             Recent, in other folders:\n/other/kart\n\
             Switch with `open_project` and one of these paths, only when the user asked to."
        );
    }

    /// As the picker switches: the old game stops, the new one starts; the
    /// chat there starts a new task; the line names the asking task.
    #[test]
    fn a_switch_asked_for_is_the_pickers() {
        let (s, commands) = update(asking(), open(B));
        let r = reply(&commands);
        assert!(r.ok, "{r:?}");
        assert!(
            r.message.starts_with(
                "The app now shows examples/b. This task stays with examples/a; say or type the next request to work in examples/b."
            ),
            "{}",
            r.message
        );
        assert_eq!(s.active.as_deref(), Some(B));
        assert!(commands.iter().any(|c| matches!(c, Command::ListFiles { path, .. } if path == B)));
        assert!(commands.iter().any(
            |c| matches!(c, Command::StartSession { dir, display: Display::Hidden, .. } if dir.ends_with(B))
        ));
        assert_eq!(s.current_task.get(B), Some(&None));
        let switch = s.agent_switch.clone().unwrap();
        assert_eq!(
            (switch.task, switch.title.as_str(), switch.from.as_str(), switch.to.as_str()),
            (1, "switch to b", A, B)
        );
        assert!(!switch.waiting);
        // Once per run.
        assert!(refused(update(s, client(ClientMsg::DismissSwitch)).0, open(A)).contains("once"));
    }

    #[test]
    fn the_gate_refuses_what_the_user_did_not_ask_for_now() {
        let s = asking();
        let review = request(1, true, ViewRequest::OpenProject { path: B.into() });
        assert!(refused(s.clone(), review).contains("a review does not switch"));
        let gone = Event::AgentView {
            task: 1,
            run: 9,
            review: false,
            request: ViewRequest::OpenProject { path: B.into() },
        };
        assert!(refused(s.clone(), gone).contains("this run has ended"));
        assert!(refused(s.clone(), open("examples/c")).contains("not a path `list_projects` gives"));
        assert!(refused(s.clone(), open("/gone")).contains("not a path"));
        assert!(refused(s.clone(), open(A)).contains("examples/a is already open"));
        // A recent spelling of a project here is that project.
        assert!(refused(s.clone(), open("/repo/examples/a")).contains("already open"));

        let stale = AppState {
            clock: ASKED_WITHIN_MS + 1,
            ..s.clone()
        };
        assert!(refused(stale, open(B)).contains("more than 10 minutes old"));

        let mut off = s.clone();
        let mut layout = Layout::initial();
        layout.agent.arranges = false;
        off.layouts.insert(A.into(), layout);
        assert!(refused(off, open(B)).contains("turned off"));

        // The user has another project open: the task's is not in sight.
        let away = AppState {
            active: Some(B.into()),
            ..s
        };
        assert!(refused(away, open(B)).contains("the user has examples/b open, not examples/a"));
    }

    #[test]
    fn undo_goes_back_to_the_asking_task_and_dismiss_only_hides_the_line() {
        let (s, _) = update(asking(), open(B));
        let (back, commands) = update(s.clone(), client(ClientMsg::UndoSwitch));
        assert_eq!(back.active.as_deref(), Some(A));
        assert_eq!(back.current_task.get(A), Some(&Some(1)));
        assert_eq!(back.agent_switch, None);
        assert!(commands.iter().any(|c| matches!(c, Command::StopSession { .. })));

        let (hidden, _) = update(s.clone(), client(ClientMsg::DismissSwitch));
        assert_eq!((hidden.active.as_deref(), hidden.agent_switch), (Some(B), None));

        // The user's own switch, or their next request, takes it too.
        let (mine, _) = update(s.clone(), client(ClientMsg::SetActiveProject { path: A.into() }));
        assert_eq!(mine.agent_switch, None);
        let next = Event::NewTask {
            id: Ok(2),
            message: "make it red".into(),
            harness: Harness::Claude,
            voice: None,
            scope: None,
        };
        let (s, _) = update(update(s, ended(1, 0)).0, next);
        assert_eq!((task(&s, 2).project.as_str(), &s.agent_switch), (B, &None));
    }

    /// Another folder opens as the asking run ends, if nothing else runs.
    #[test]
    fn a_recent_folder_opens_when_the_run_ends() {
        let (s, commands) = update(asking(), open("/other/kart"));
        let r = reply(&commands);
        assert!(r.ok && r.message.starts_with("The app opens /other/kart when this run ends."), "{r:?}");
        assert!(!commands.iter().any(|c| matches!(c, Command::OpenFolder { .. })));
        assert_eq!(s.active.as_deref(), Some(A));
        let (s, _) = update(s, agent(1, 0, finished()));
        let (s, commands) = update(s, ended(1, 0));
        let opens: Vec<&String> = (commands.iter())
            .filter_map(|c| match c {
                Command::OpenFolder { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(opens, ["/other/kart"]);
        assert!(!s.agent_switch.unwrap().waiting);

        // Not while another task runs: the folder would change under it.
        let mut busy = asking();
        busy.tasks[0].project = B.into();
        busy.tasks.push(Task {
            id: 2,
            project: A.into(),
            ..busy.tasks[0].clone()
        });
        let ask = request(2, false, ViewRequest::OpenProject { path: "/other/kart".into() });
        assert!(refused(busy, ask).contains("task 1 is running"));
    }

    /// Asked for in the headset: the new project's game runs there, and the
    /// headset hears what happened, once.
    #[test]
    fn a_switch_on_the_headset_stays_there_and_says_so() {
        let mut s = voice_task_running();
        s.start_on_open = true;
        let old = s.session.as_ref().unwrap().id;
        let (s, commands) = update(s, request(4, false, ViewRequest::OpenProject { path: B.into() }));
        assert!(reply(&commands).ok);
        let game = &s.layouts[B].game;
        assert_eq!((game.placement, game.headset.as_deref()), (Placement::Headset, Some("aa")));
        assert!(commands.contains(&Command::StopSession { id: old }));
        assert!(commands.iter().any(|c| matches!(
            c,
            Command::StartHeadset { dir, tunnel, .. } if dir.ends_with(B) && tunnel == "http://127.0.0.1:6000"
        )));
        assert_eq!(stages(&commands), [(1, Stage::Done("Switched to examples/b".into()))]);
        let (s, commands) = update(s, agent(4, 0, finished()));
        assert_eq!(stages(&commands), []);
        let (_, commands) = update(s, ended(4, 0));
        assert_eq!(stages(&commands), []);
    }

    /// The picker's own switch keeps the game on the headset too.
    #[test]
    fn a_manual_switch_on_the_headset_stays_there() {
        let mut s = voice_ready();
        s.start_on_open = true;
        let (s, commands) = update(s, client(ClientMsg::SetActiveProject { path: B.into() }));
        assert_eq!(s.layouts[B].game.placement, Placement::Headset);
        assert!(commands.iter().any(|c| matches!(c, Command::StartHeadset { dir, .. } if dir.ends_with(B))));
        // Back to a pane there, a switch back keeps a's own placement.
        let pane = client(ClientMsg::Layout {
            op: LayoutOp::Place {
                placement: Placement::Pane,
            },
        });
        let (s, _) = update(s, pane);
        let (s, commands) = update(s, client(ClientMsg::SetActiveProject { path: A.into() }));
        assert_eq!(s.layouts[A].game.placement, Placement::Headset);
        assert!(commands.iter().any(|c| matches!(c, Command::StartHeadset { .. })));
    }
}
