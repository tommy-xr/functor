//! The agent drives the view (D25): its `show_view` and `present` requests,
//! walkthrough cards, the highlight they leave, and the app context its tool
//! server reads. The placement rules themselves are [`Layout::agent_show`].

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::layout::{LayoutOp, Placed, Placement, Target, View};
use crate::projects::check_relative;
use crate::session::{Session, SessionMode, SessionStatus};
use crate::state::{
    broadcast_items, check_view, layout_op, placement, save, save_layout, AppState, Command,
};
use crate::tasks::{has_change, Item};

/// Most view requests one run may make; later ones are refused.
pub const MAX_VIEW_REQUESTS: u32 = 20;
/// Most steps in a walkthrough.
pub const MAX_STEPS: usize = 8;
/// Longest step title and note, in characters.
const MAX_TITLE: usize = 80;
const MAX_NOTE: usize = 400;
/// A run's walkthrough opens by itself only if the user has not changed the
/// layout for this long (ms) before the run ended.
pub const AUTO_QUIET_MS: u64 = 3000;

/// What a view request or walkthrough step shows: one of the run's
/// project's files (`lines` `[first, last]`, 1-based), its task's changes
/// (scrolled to `path`), or the game. Paths are relative to the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewTarget {
    File {
        path: String,
        #[serde(default)]
        lines: Option<[u32; 2]>,
    },
    Changes {
        #[serde(default)]
        path: Option<String>,
    },
    Game,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub note: Option<String>,
    pub view: ViewTarget,
}

/// A request from a run's tool server (`POST /agent/view`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewRequest {
    Show { view: ViewTarget },
    Present { steps: Vec<Step> },
}

/// The answer the agent reads: what happened, or why nothing did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ViewReply {
    pub ok: bool,
    pub message: String,
}

/// What the agent or a walkthrough step last pointed at, until the user
/// scrolls it out of view or its tab stops being shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Highlight {
    pub project: String,
    pub view: View,
    /// A file's lines, `[first, last]`, 1-based.
    pub lines: Option<[u32; 2]>,
    /// A changes view's file, relative to the repo root.
    pub file: Option<String>,
    /// New per highlight, so showing the same again scrolls again.
    pub seq: u64,
}

/// What the tool server's `app_context` tool reports to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppContext {
    /// The task's project, relative to the repo root.
    pub project: String,
    /// The project the user has open; views are shown only in the task's.
    pub active_project: Option<String>,
    /// The game session's debug URL, when it runs this project.
    pub session: Option<String>,
    pub session_mode: SessionMode,
    /// The task's project's canvas.
    pub layout: LayoutContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LayoutContext {
    /// In reading order; the user works in the `focused` one.
    pub panes: Vec<PaneContext>,
    pub game: Placement,
    /// Tabs the agent opened that are still open.
    pub agent_tabs: Vec<View>,
    /// "Let the agent arrange views": off, view requests change nothing.
    pub agent_arranges: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaneContext {
    pub tabs: Vec<View>,
    pub shown: Option<View>,
    pub focused: bool,
}

/// The context for a run in `project` now.
pub fn app_context(state: &AppState, project: &str) -> AppContext {
    let session = match &state.session {
        Some(Session {
            status: SessionStatus::Running { debug_url, .. },
            ..
        }) if state.active.as_deref() == Some(project) => Some(debug_url.clone()),
        _ => None,
    };
    let layout = state.layouts.get(project).cloned().unwrap_or_default();
    let panes = layout
        .panes()
        .into_iter()
        .map(|p| PaneContext {
            tabs: p.tabs.clone(),
            shown: p.tabs.get(p.active).cloned(),
            focused: layout.focus == Some(p.id),
        })
        .collect();
    AppContext {
        project: project.to_string(),
        active_project: state.active.clone(),
        session,
        session_mode: placement(state, project).session_mode(),
        layout: LayoutContext {
            panes,
            game: layout.game.placement,
            agent_tabs: layout.agent.tabs.clone(),
            agent_arranges: layout.agent.arranges,
        },
    }
}

/// For each running task whose context changed since its tool server last
/// got it, a `Command::AppContext`; forgets tasks no longer running.
pub(crate) fn refresh_contexts(state: &mut AppState, commands: &mut Vec<Command>) {
    let running: Vec<(u64, u64, String)> = state
        .tasks
        .iter()
        .filter_map(|t| Some((t.id, t.run.as_ref()?.id, t.project.clone())))
        .collect();
    state
        .contexts
        .retain(|id, _| running.iter().any(|(t, ..)| t == id));
    for (task, run, project) in running {
        let context = app_context(state, &project);
        if state.contexts.get(&task) != Some(&context) {
            state.contexts.insert(task, context.clone());
            commands.push(Command::AppContext { task, run, context });
        }
    }
}

/// The layout view `target` shows, for task `task`.
fn view_of(target: &ViewTarget, task: u64) -> View {
    match target {
        ViewTarget::File { path, .. } => View::File { path: path.clone() },
        ViewTarget::Changes { .. } => View::Changes { task },
        ViewTarget::Game => View::Game,
    }
}

/// `target` in words: "game.fun, lines 4–5", "the changes", "the game".
pub fn describe(target: &ViewTarget) -> String {
    match target {
        ViewTarget::File {
            path,
            lines: Some([a, b]),
        } if a == b => format!("{path}, line {a}"),
        ViewTarget::File {
            path,
            lines: Some([a, b]),
        } => format!("{path}, lines {a}–{b}"),
        ViewTarget::File { path, .. } => path.clone(),
        ViewTarget::Changes { path: Some(path) } => format!("the changes to {path}"),
        ViewTarget::Changes { path: None } => "the changes".to_string(),
        ViewTarget::Game => "the game".to_string(),
    }
}

/// Why `target` cannot be shown for task `task` in `project` now: the file
/// view's rules (project-relative, listed), a change needs git.
fn check_target(
    state: &AppState,
    project: &str,
    task: u64,
    target: &ViewTarget,
) -> Result<(), String> {
    match target {
        ViewTarget::File { path, lines } => {
            if let Some([a, b]) = lines {
                if *a == 0 || a > b {
                    return Err(format!(
                        "lines [{a}, {b}]: give [first, last] with 1 <= first <= last"
                    ));
                }
            }
            check_view(state, project, &View::File { path: path.clone() }).map_err(|e| {
                let listed: Vec<&str> = state.files.iter().take(30).map(String::as_str).collect();
                format!("{e}; its files are: {}", listed.join(", "))
            })
        }
        ViewTarget::Changes { path } => {
            if !state.tracks_changes {
                return Err(
                    "changes are tracked only in a git repository: show the file instead"
                        .to_string(),
                );
            }
            if let Some(path) = path {
                check_relative(path)?;
            }
            check_view(state, project, &View::Changes { task })
        }
        ViewTarget::Game => Ok(()),
    }
}

/// Why `steps` is not a walkthrough: 1 to [`MAX_STEPS`], short titles and notes.
fn check_steps(steps: &[Step]) -> Result<(), String> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!(
            "a walkthrough has 1 to {MAX_STEPS} steps, not {}",
            steps.len()
        ));
    }
    for (i, step) in steps.iter().enumerate() {
        let n = i + 1;
        let title = step.title.trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE {
            return Err(format!(
                "step {n}: a title is 1 to {MAX_TITLE} characters"
            ));
        }
        if step.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE) {
            return Err(format!("step {n}: a note is at most {MAX_NOTE} characters"));
        }
    }
    Ok(())
}

/// Answers run `run` of task `task`'s view request with a
/// `Command::ViewReply`; whether the state changed.
pub(crate) fn agent_view(
    state: &mut AppState,
    task: u64,
    run: u64,
    request: ViewRequest,
    commands: &mut Vec<Command>,
) -> bool {
    let (ok, message, changed) = match handle(state, task, run, request, commands) {
        Ok((message, changed)) => (true, message, changed),
        Err(message) => (false, message, false),
    };
    commands.push(Command::ViewReply(ViewReply { ok, message }));
    changed
}

fn handle(
    state: &mut AppState,
    id: u64,
    run: u64,
    request: ViewRequest,
    commands: &mut Vec<Command>,
) -> Result<(String, bool), String> {
    let active = state.active.clone();
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.run.as_ref().is_some_and(|r| r.id == run))
        .ok_or("this run has ended: nothing can be shown for it")?;
    let project = task.project.clone();
    if active.as_deref() != Some(project.as_str()) {
        let other = active.as_deref().unwrap_or("nothing");
        return Err(format!(
            "the user has {other} open, not {project}: nothing was shown"
        ));
    }
    // Refused requests count too, so a confused agent cannot flood the app.
    let views = &mut task.run.as_mut().expect("found").views;
    if *views >= MAX_VIEW_REQUESTS {
        return Err(format!(
            "this run has made {MAX_VIEW_REQUESTS} view requests, the most one run may: nothing was shown"
        ));
    }
    *views += 1;
    let arranges = state
        .layouts
        .get(&project)
        .is_none_or(|l| l.agent.arranges);
    match request {
        ViewRequest::Show { view } => {
            check_target(state, &project, id, &view)?;
            if !arranges {
                return Ok((off(&view), false));
            }
            let message = agent_show(state, &project, id, &view, commands)?;
            Ok((message, true))
        }
        ViewRequest::Present { steps } => {
            check_steps(&steps)?;
            for (i, step) in steps.iter().enumerate() {
                check_target(state, &project, id, &step.view)
                    .map_err(|e| format!("step {}: {e}; nothing was added", i + 1))?;
            }
            let n = steps.len();
            let root = Path::new(&state.repo_root).to_path_buf();
            let task = state.tasks.iter_mut().find(|t| t.id == id).expect("found");
            task.transcript.push(Item::Walkthrough {
                steps,
                current: None,
            });
            commands.push(save(&root, task));
            broadcast_items(commands, task, vec![task.transcript.len() - 1]);
            let then = match arranges {
                true => "its first step opens beside the user's work when this run ends",
                false => "the user has turned off \"Let the agent arrange views\", so it opens only when they click it",
            };
            Ok((
                format!("Added a walkthrough of {n} step{} to the chat; {then}.", plural(n)),
                false,
            ))
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn off(view: &ViewTarget) -> String {
    format!(
        "The user has turned off \"Let the agent arrange views\" for this project: {} was not opened.",
        describe(view)
    )
}

/// Shows `target` under the agent's rules and highlights it; what happened.
fn agent_show(
    state: &mut AppState,
    project: &str,
    task: u64,
    target: &ViewTarget,
    commands: &mut Vec<Command>,
) -> Result<String, String> {
    let view = view_of(target, task);
    let mut layout = state.layouts.get(project).cloned().unwrap_or_default();
    let before = layout.clone();
    let placed = layout.agent_show(&view, state.canvas_stacked, |v| check_view(state, project, v))?;
    if layout != before {
        save_layout(state, project, layout, commands);
    }
    let what = describe(target);
    let message = match placed {
        Placed::Opened => format!("Opened {what} in the pane beside the user's."),
        Placed::Shown => format!("Showed {what} in the pane beside the user's."),
        Placed::InView => format!("{what} is the tab the user is looking at; it is highlighted."),
        Placed::Behind => {
            return Ok(format!(
                "The user's canvas is too narrow to split, so {what} is a tab behind the one they are reading; it was not brought forward."
            ))
        }
        Placed::GameElsewhere(placement) => {
            let where_ = match placement {
                Placement::Mini => "in its mini view",
                _ => "in its own window",
            };
            return Ok(format!(
                "The game is already in sight {where_}; nothing was moved."
            ));
        }
    };
    highlight(state, project, &view, target);
    Ok(message)
}

/// Points the highlight at `target`'s lines or changed file, when it has one.
fn highlight(state: &mut AppState, project: &str, view: &View, target: &ViewTarget) {
    let (lines, file) = match target {
        ViewTarget::File { lines, .. } => (*lines, None),
        ViewTarget::Changes { path } => {
            let repo = |p: &String| match project {
                "." => p.clone(),
                _ => format!("{project}/{p}"),
            };
            (None, path.as_ref().map(repo))
        }
        ViewTarget::Game => (None, None),
    };
    if lines.is_none() && file.is_none() {
        state.highlight = None;
        return;
    }
    let seq = state.highlight.as_ref().map_or(0, |h| h.seq) + 1;
    state.highlight = Some(Highlight {
        project: project.to_string(),
        view: view.clone(),
        lines,
        file,
        seq,
    });
}

/// Drops the highlight once its tab is not shown in the active project's
/// canvas (closed, or the user opened something over it).
pub(crate) fn prune_highlight(state: &mut AppState) {
    let Some(h) = &state.highlight else {
        return;
    };
    let shown = state.active.as_ref() == Some(&h.project)
        && state.layouts.get(&h.project).is_some_and(|l| {
            l.panes()
                .iter()
                .any(|p| p.tabs.get(p.active) == Some(&h.view))
        });
    if !shown {
        state.highlight = None;
    }
}

/// The walkthrough at `item` of task `id`, mutably, with its project.
fn walkthrough(
    state: &mut AppState,
    id: u64,
    item: usize,
) -> Result<(&mut Vec<Step>, &mut Option<usize>), String> {
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("there is no task {id}"))?;
    match task.transcript.get_mut(item) {
        Some(Item::Walkthrough { steps, current }) => Ok((steps, current)),
        _ => Err(format!("task {id} has no walkthrough at {item}")),
    }
}

/// The user clicked step `step` of the walkthrough at `item`: its view
/// opens as "View diff" opens one (beside, or focused where it is), then
/// is highlighted and marked current. Whether the state changed.
pub(crate) fn step_clicked(
    state: &mut AppState,
    id: u64,
    item: usize,
    step: usize,
    commands: &mut Vec<Command>,
) -> Result<bool, String> {
    let project = state
        .tasks
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.project.clone())
        .ok_or_else(|| format!("there is no task {id}"))?;
    if state.active.as_deref() != Some(project.as_str()) {
        return Err(format!("task {id} is in another project"));
    }
    let (steps, _) = walkthrough(state, id, item)?;
    let target = steps
        .get(step)
        .ok_or_else(|| format!("the walkthrough has no step {}", step + 1))?
        .view
        .clone();
    available(state, &project, id, &target)?;
    let view = view_of(&target, id);
    let in_sight = view == View::Game && placement(state, &project) != Placement::Pane;
    let mut changed = false;
    if !in_sight {
        let open = LayoutOp::Open {
            view: view.clone(),
            target: Target::Side,
        };
        changed = layout_op(state, open, commands);
        highlight(state, &project, &view, &target);
    }
    state.user_layout_at = Some(state.clock);
    mark_current(state, id, item, step, commands);
    Ok(changed)
}

/// Why a walkthrough step cannot be shown now: its file is gone, or its
/// task's change is (the card shows it as unavailable).
fn available(state: &AppState, project: &str, task: u64, target: &ViewTarget) -> Result<(), String> {
    match target {
        ViewTarget::Changes { .. } => {
            let t = state.tasks.iter().find(|t| t.id == task);
            match t.is_some_and(has_change) && state.tracks_changes {
                true => Ok(()),
                false => Err("this step's change is no longer there".to_string()),
            }
        }
        target => check_target(state, project, task, target),
    }
}

/// Marks step `step` current, saving and broadcasting the item.
fn mark_current(state: &mut AppState, id: u64, item: usize, step: usize, commands: &mut Vec<Command>) {
    let root = Path::new(&state.repo_root).to_path_buf();
    let task = state.tasks.iter_mut().find(|t| t.id == id).expect("checked");
    if let Some(Item::Walkthrough { current, .. }) = task.transcript.get_mut(item) {
        *current = Some(step);
    }
    commands.push(save(&root, task));
    broadcast_items(commands, task, vec![item]);
}

/// Run `run` of task `id` ended: the first step of its last walkthrough
/// opens under the agent's rules, once, if no step of it was shown yet, the
/// setting is on, its project is open and the user has not changed the
/// layout in the [`AUTO_QUIET_MS`] before. Whether the state changed.
pub(crate) fn auto_apply(state: &mut AppState, id: u64, run: u64, commands: &mut Vec<Command>) -> bool {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        return false;
    };
    let project = task.project.clone();
    let last = task
        .transcript
        .iter()
        .enumerate()
        .skip(run as usize + 1)
        .rev()
        .find_map(|(i, item)| match item {
            Item::Walkthrough { steps, current } => Some((i, steps.first().cloned(), *current)),
            _ => None,
        });
    let Some((item, Some(first), None)) = last else {
        return false;
    };
    let quiet = state
        .user_layout_at
        .is_none_or(|at| state.clock.saturating_sub(at) >= AUTO_QUIET_MS);
    let arranges = state.layouts.get(&project).is_none_or(|l| l.agent.arranges);
    if !quiet || !arranges || state.active.as_deref() != Some(project.as_str()) {
        return false;
    }
    if available(state, &project, id, &first.view).is_err() {
        return false;
    }
    if agent_show(state, &project, id, &first.view, commands).is_err() {
        return false;
    }
    mark_current(state, id, item, 0, commands);
    true
}
