//! The agent switches the open project when the user asks (D42): its
//! `list_projects` and `open_project` requests, their gate, and the chat's
//! "Switched to … — asked by you" line with its Undo.
//!
//! A switch is the project picker's, so all that follows a manual one
//! follows it. The asking task stays in its project; its run is told to
//! answer and stop.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::state::{recent_dir, switch_active, AppState, Command};
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
    /// The task that asked, its title and its project.
    pub task: u64,
    pub title: String,
    pub from: String,
    /// The project the app switched to.
    pub to: String,
    /// `to`'s current task before (`None`: none chosen), back on Undo.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub shown: Option<Option<u64>>,
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
/// recent ones in other folders, which only the user opens. Changes nothing.
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
    let here: Vec<String> = (state.projects.iter()).map(|p| recent_dir(state, &p.path)).collect();
    let elsewhere: Vec<&str> = (state.recents.iter())
        .filter(|r| !r.missing && !here.contains(&r.path))
        .map(|r| r.path.as_str())
        .collect();
    if !elsewhere.is_empty() {
        let _ = writeln!(
            out,
            "Recent, in other folders (the user opens these from the project switcher):\n{}",
            elsewhere.join("\n")
        );
    }
    out.push_str("Switch with `open_project` and a project in this folder, only when the user asked to.");
    Ok((out, false))
}

/// `open_project`: switches the app to `path`, a project in the open
/// folder, when the gate allows. What happened, and whether the state
/// changed.
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
    // Another folder opens only when no task runs, and this one does.
    if !state.projects.iter().any(|p| p.path == path) {
        return Err(nothing(format!(
            "{path} is not a project in this folder: give one `list_projects` lists here, or ask \
the user to open another folder from the project switcher"
        )));
    }
    if path == from {
        return Err(nothing(format!("{path} is already open")));
    }
    let to = path.to_string();
    let name = (state.projects.iter().find(|p| p.path == to)).map_or(to.clone(), |p| p.name.clone());
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("found");
    task.run.as_mut().expect("running").switched = true;
    // The chat there starts a new task.
    let shown = state.current_task.insert(to.clone(), None);
    state.agent_switch = Some(AgentSwitch {
        task: id,
        title,
        from: from.clone(),
        to: to.clone(),
        shown,
    });
    switch_active(state, to.clone(), commands);
    voice::switched(state, id, &format!("Switched to {name}"), commands);
    Ok((
        format!(
            "The app now shows {to}. This task stays with {from}; say or type the next request to \
work in {to}. Tell the user so in one sentence and stop: your game tools no longer reach a game."
        ),
        true,
    ))
}

/// The user's Undo: back to the project the agent was asked from, showing
/// the task that asked.
pub(crate) fn undo(state: &mut AppState, commands: &mut Vec<Command>) -> Result<bool, String> {
    let switch = state.agent_switch.take().ok_or("there is no switch to undo")?;
    // `to`'s chat shows what it did, unless the user chose a task there since.
    if state.current_task.get(&switch.to) == Some(&None) {
        let exists = |id: &u64| state.tasks.iter().any(|t| t.id == *id);
        match switch.shown {
            None => state.current_task.remove(&switch.to),
            Some(shown) => state.current_task.insert(switch.to.clone(), shown.filter(exists)),
        };
    }
    switch_active(state, switch.from.clone(), commands);
    if state.tasks.iter().any(|t| t.id == switch.task) {
        state.current_task.insert(switch.from, Some(switch.task));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_view::{ViewReply, ViewRequest};
    use crate::devices::LinkCommand;
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
        // A's as this platform spells it, so it is known as this folder's.
        let a = recent_dir(&s, A);
        s.recents = [(a.as_str(), false), ("/other/kart", false), ("/gone", true)]
            .map(|(path, missing)| Recent {
                name: String::new(),
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
             Recent, in other folders (the user opens these from the project switcher):\n/other/kart\n\
             Switch with `open_project` and a project in this folder, only when the user asked to."
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
        assert!(refused(s.clone(), open("examples/c")).contains("not a project in this folder"));
        // Another folder opens only when no task runs: the user opens it.
        assert!(refused(s.clone(), open("/other/kart")).contains("from the project switcher"));
        assert!(refused(s.clone(), open(A)).contains("examples/a is already open"));

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
        // B's chat had no task chosen before, and has none again.
        assert_eq!(back.current_task.get(B), None);
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

    /// Asked for in the headset: the new project's game runs there, and the
    /// headset hears what happened, once, shown and (replies on) aloud.
    #[test]
    fn a_switch_on_the_headset_stays_there_and_says_so() {
        let mut s = voice_task_running();
        s.start_on_open = true;
        s.speak = true;
        let aloud = |commands: &[Command]| -> Vec<String> {
            let text = |c: &Command| match c {
                Command::Link(LinkCommand::Say { text, .. }) => Some(text.clone()),
                _ => None,
            };
            commands.iter().filter_map(text).collect()
        };
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
        assert_eq!(stages(&commands), [(1, Stage::Answered("Switched to b".into()))]);
        assert_eq!(aloud(&commands), ["Switched to b"]);
        assert!(!saves(&commands, B), "for the session only");
        let (s, commands) = update(s, agent(4, 0, finished()));
        assert_eq!((stages(&commands), aloud(&commands)), (vec![], vec![]));
        let (_, commands) = update(s, ended(4, 0));
        assert_eq!((stages(&commands), aloud(&commands)), (vec![], vec![]));
    }

    /// Whether `commands` save `project`'s layout.
    fn saves(commands: &[Command], project: &str) -> bool {
        (commands.iter()).any(|c| matches!(c, Command::SaveLayout { dir, .. } if dir.ends_with(project)))
    }

    /// The picker's own switch keeps the game on the headset too, for the
    /// session: the project's saved layout keeps its own placement.
    #[test]
    fn a_manual_switch_on_the_headset_stays_there_for_the_session() {
        let mut s = voice_ready();
        s.start_on_open = true;
        let (mut s, commands) = update(s, client(ClientMsg::SetActiveProject { path: B.into() }));
        assert_eq!(s.layouts[B].game.placement, Placement::Headset);
        assert!(commands.iter().any(|c| matches!(c, Command::StartHeadset { dir, .. } if dir.ends_with(B))));
        assert!(!saves(&commands, B));
        // A saved in a pane: a switch back from the headset carries it there
        // too, and leaves its saved layout as it is.
        let mut pane = s.layouts[A].clone();
        let place = LayoutOp::Place {
            placement: Placement::Pane,
        };
        assert_eq!(pane.apply(place, |_| Ok(())), Ok(true));
        s.layouts.insert(A.into(), pane);
        let (s, commands) = update(s, client(ClientMsg::SetActiveProject { path: A.into() }));
        assert_eq!(s.layouts[A].game.placement, Placement::Headset);
        assert!(commands.iter().any(|c| matches!(c, Command::StartHeadset { dir, .. } if dir.ends_with(A))));
        assert!(!saves(&commands, A));
    }
}
