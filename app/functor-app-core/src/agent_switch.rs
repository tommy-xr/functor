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
