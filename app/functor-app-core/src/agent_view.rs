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
#[serde(tag = "kind", rename_all = "snake_case")]
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
pub struct WalkthroughStep {
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
    Present { steps: Vec<WalkthroughStep> },
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
fn check_steps(steps: &[WalkthroughStep]) -> Result<(), String> {
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
            return Err(format!("step {n}: a title is 1 to {MAX_TITLE} characters"));
        }
        if step
            .note
            .as_ref()
            .is_some_and(|n| n.chars().count() > MAX_NOTE)
        {
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
    let arranges = state.layouts.get(&project).is_none_or(|l| l.agent.arranges);
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
                format!(
                    "Added a walkthrough of {n} step{} to the chat; {then}.",
                    plural(n)
                ),
                false,
            ))
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
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
    let placed = layout.agent_show(&view, state.canvas_stacked, |v| {
        check_view(state, project, v)
    })?;
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
) -> Result<(&mut Vec<WalkthroughStep>, &mut Option<usize>), String> {
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
fn available(
    state: &AppState,
    project: &str,
    task: u64,
    target: &ViewTarget,
) -> Result<(), String> {
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
fn mark_current(
    state: &mut AppState,
    id: u64,
    item: usize,
    step: usize,
    commands: &mut Vec<Command>,
) {
    let root = Path::new(&state.repo_root).to_path_buf();
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("checked");
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
pub(crate) fn auto_apply(
    state: &mut AppState,
    id: u64,
    run: u64,
    commands: &mut Vec<Command>,
) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use crate::projects::{Entries, Project};
    use crate::protocol::{ClientMsg, ServerMsg};
    use crate::state::{update, Event};
    use crate::tasks::{AgentEvent, Harness, RunEvent};

    fn project(path: &str) -> Project {
        Project {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            entries: Entries::Single {
                file: "game.fun".to_string(),
            },
        }
    }

    const A: &str = "examples/a";

    /// Project a open (game.fun listed, a.fun the user's tab) with task 1
    /// running (run 0).
    fn running() -> AppState {
        let mut s = AppState {
            repo_root: "/repo".to_string(),
            projects: vec![project(A), project("examples/b")],
            active: Some(A.to_string()),
            files: ["a.fun", "functor.json", "game.fun"]
                .map(String::from)
                .to_vec(),
            harnesses: vec![Harness::Claude],
            tracks_changes: true,
            ..AppState::none()
        };
        s.layouts.insert(A.to_string(), user_layout());
        let new = Event::NewTask {
            id: Ok(1),
            message: "make it count by two".to_string(),
            harness: Harness::Claude,
        };
        update(s, new).0
    }

    /// One pane showing a.fun, behind it the game.
    fn user_layout() -> Layout {
        let mut l = Layout::initial();
        let open = LayoutOp::Open {
            view: file("a.fun"),
            target: Target::Focused,
        };
        l.apply(open, |_| Ok(())).unwrap();
        l
    }

    fn file(path: &str) -> View {
        View::File {
            path: path.to_string(),
        }
    }

    fn show(target: ViewTarget) -> Event {
        Event::AgentView {
            task: 1,
            run: 0,
            request: ViewRequest::Show { view: target },
        }
    }

    fn game_fun(lines: Option<[u32; 2]>) -> ViewTarget {
        ViewTarget::File {
            path: "game.fun".to_string(),
            lines,
        }
    }

    fn present(steps: Vec<WalkthroughStep>) -> Event {
        Event::AgentView {
            task: 1,
            run: 0,
            request: ViewRequest::Present { steps },
        }
    }

    fn step(title: &str, view: ViewTarget) -> WalkthroughStep {
        WalkthroughStep {
            title: title.to_string(),
            note: None,
            view,
        }
    }

    fn reply(commands: &[Command]) -> &ViewReply {
        let replies: Vec<&ViewReply> = commands
            .iter()
            .filter_map(|c| match c {
                Command::ViewReply(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 1, "{commands:?}");
        replies[0]
    }

    fn layout(s: &AppState) -> &Layout {
        &s.layouts[A]
    }

    fn ended(s: AppState) -> (AppState, Vec<Command>) {
        let s = update(
            s,
            agent_event(AgentEvent::Finished {
                ok: true,
                summary: None,
            }),
        )
        .0;
        update(
            s,
            Event::Run {
                task: 1,
                run: 0,
                event: RunEvent::Ended { error: None },
            },
        )
    }

    fn agent_event(event: AgentEvent) -> Event {
        Event::Run {
            task: 1,
            run: 0,
            event: RunEvent::Agent(event),
        }
    }

    fn walkthrough(s: &AppState) -> (usize, Vec<WalkthroughStep>, Option<usize>) {
        let t = s.tasks.iter().find(|t| t.id == 1).unwrap();
        t.transcript
            .iter()
            .enumerate()
            .find_map(|(i, item)| match item {
                Item::Walkthrough { steps, current } => Some((i, steps.clone(), *current)),
                _ => None,
            })
            .expect("a walkthrough")
    }

    #[test]
    fn show_view_opens_beside_the_user_and_highlights_the_lines() {
        let s = running();
        let (s, commands) = update(s, show(game_fun(Some([4, 5]))));
        let r = reply(&commands);
        assert!(r.ok, "{r:?}");
        assert_eq!(
            r.message,
            "Opened game.fun, lines 4–5 in the pane beside the user's."
        );
        assert_eq!(
            crate::print_layout(layout(&s)),
            "right 50% (>#1[game, *file:a.fun] | #3[*file:game.fun])"
        );
        assert_eq!(layout(&s).agent.tabs, [file("game.fun")]);
        let h = s.highlight.clone().unwrap();
        assert_eq!(
            (h.view, h.lines, h.project.as_str()),
            (file("game.fun"), Some([4, 5]), A)
        );
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::SaveLayout { .. })));
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::Broadcast(ServerMsg::State { .. }))));
    }

    #[test]
    fn a_bad_or_foreign_request_is_refused_with_a_reason_and_changes_nothing() {
        let s = running();
        let before = s.layouts.clone();
        for (target, why) in [
            (
                ViewTarget::File {
                    path: "../secret".into(),
                    lines: None,
                },
                "not a path inside",
            ),
            (
                ViewTarget::File {
                    path: "/etc/passwd".into(),
                    lines: None,
                },
                "not a path inside",
            ),
            (
                ViewTarget::File {
                    path: "notes.txt".into(),
                    lines: None,
                },
                "its files are: a.fun, functor.json, game.fun",
            ),
            (game_fun(Some([0, 2])), "1 <= first"),
            (game_fun(Some([5, 4])), "1 <= first"),
            (
                ViewTarget::Changes {
                    path: Some("../x".into()),
                },
                "not a path inside",
            ),
        ] {
            let (next, commands) = update(s.clone(), show(target.clone()));
            let r = reply(&commands);
            assert!(!r.ok && r.message.contains(why), "{target:?}: {r:?}");
            assert_eq!(next.layouts, before);
            assert_eq!(next.highlight, None);
        }
        // Another task's, an ended run's, or a run that never was.
        for (task, run) in [(2, 0), (1, 7)] {
            let request = ViewRequest::Show {
                view: ViewTarget::Game,
            };
            let (_, commands) = update(s.clone(), Event::AgentView { task, run, request });
            assert!(
                reply(&commands).message.contains("this run has ended"),
                "{commands:?}"
            );
        }
        // The user has another project open.
        let (b, _) = update(
            s.clone(),
            Event::Client(ClientMsg::SetActiveProject {
                path: "examples/b".into(),
            }),
        );
        let (b, commands) = update(b, show(game_fun(None)));
        assert!(reply(&commands)
            .message
            .contains("the user has examples/b open"));
        assert_eq!(b.layouts.get(A), before.get(A));
        // Outside git there are no changes to show.
        let mut untracked = s.clone();
        untracked.tracks_changes = false;
        let (_, commands) = update(untracked, show(ViewTarget::Changes { path: None }));
        assert!(reply(&commands)
            .message
            .contains("only in a git repository"));
    }

    #[test]
    fn a_run_may_make_only_so_many_view_requests() {
        let mut s = running();
        for _ in 0..MAX_VIEW_REQUESTS {
            let commands;
            (s, commands) = update(s, show(ViewTarget::Game));
            assert!(reply(&commands).ok);
        }
        let (s, commands) = update(s, show(game_fun(None)));
        let r = reply(&commands);
        assert!(!r.ok && r.message.contains("the most one run may"), "{r:?}");
        assert!(layout(&s).find(&file("game.fun")).is_none());
        // A walkthrough counts as one too.
        let (_, commands) = update(s, present(vec![step("t", ViewTarget::Game)]));
        assert!(!reply(&commands).ok);
    }

    #[test]
    fn with_the_setting_off_nothing_moves_but_the_card_is_added() {
        let s = running();
        let off = LayoutOp::AgentArranges { on: false };
        let (s, _) = update(s, Event::Client(ClientMsg::Layout { op: off }));
        let before = layout(&s).clone();
        let (s, commands) = update(s, show(game_fun(Some([1, 2]))));
        let r = reply(&commands);
        assert!(r.ok && r.message.contains("turned off"), "{r:?}");
        assert_eq!(layout(&s), &before);
        assert_eq!(s.highlight, None);
        let (s, commands) = update(s, present(vec![step("The rule", game_fun(Some([1, 2])))]));
        assert!(reply(&commands)
            .message
            .contains("opens only when they click it"));
        let (s, _) = ended(s);
        assert_eq!(walkthrough(&s).2, None, "not applied at the end");
        assert_eq!(layout(&s), &before);
    }

    #[test]
    fn present_adds_one_saved_card_and_refuses_a_bad_step_whole() {
        let s = running();
        let steps = vec![
            step("The new step", game_fun(Some([4, 5]))),
            step("The game", ViewTarget::Game),
        ];
        let (next, commands) = update(s.clone(), present(steps.clone()));
        assert!(reply(&commands).ok);
        assert_eq!(walkthrough(&next), (1, steps.clone(), None));
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::SaveTask { .. })));
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::Broadcast(ServerMsg::Item { index: 1, .. }))));
        assert_eq!(
            next.layouts, s.layouts,
            "nothing opens while the run goes on"
        );
        let long = "x".repeat(81);
        for (bad, why) in [
            (vec![], "1 to 8 steps"),
            (vec![step("t", ViewTarget::Game); 9], "1 to 8 steps"),
            (vec![step(" ", ViewTarget::Game)], "step 1: a title"),
            (vec![step(&long, ViewTarget::Game)], "step 1: a title"),
            (
                vec![
                    steps[0].clone(),
                    step(
                        "Gone",
                        ViewTarget::File {
                            path: "x.fun".into(),
                            lines: None,
                        },
                    ),
                ],
                "step 2: x.fun is not one of",
            ),
        ] {
            let (after, commands) = update(s.clone(), present(bad));
            let r = reply(&commands);
            assert!(!r.ok && r.message.contains(why), "{r:?}");
            assert_eq!(after.tasks[0].transcript.len(), 1, "no card");
        }
    }

    #[test]
    fn at_the_end_of_the_run_the_first_step_opens_once() {
        let s = running();
        let steps = vec![
            step("The rule", game_fun(Some([4, 5]))),
            step("The game", ViewTarget::Game),
        ];
        let (s, _) = update(s, present(steps));
        let (s, _) = ended(s);
        assert_eq!(walkthrough(&s).2, Some(0));
        assert_eq!(
            crate::print_layout(layout(&s)),
            "right 50% (>#1[game, *file:a.fun] | #3[*file:game.fun])"
        );
        assert_eq!(s.highlight.as_ref().unwrap().lines, Some([4, 5]));
        assert!(layout(&s).agent.restore.is_some());
        // "Restore my layout" puts the user's back in one step.
        let (s, _) = update(
            s,
            Event::Client(ClientMsg::Layout {
                op: LayoutOp::Restore,
            }),
        );
        assert_eq!(crate::print_layout(layout(&s)), ">#1[game, *file:a.fun]");
        assert_eq!(s.highlight, None, "its tab is gone");
        // The next run's end does not apply it again.
        let (s, _) = update(
            s,
            Event::Client(ClientMsg::SendMessage {
                task: 1,
                message: "and?".into(),
            }),
        );
        let run = s.tasks[0].run.as_ref().unwrap().id;
        let s = update(
            s,
            Event::Run {
                task: 1,
                run,
                event: RunEvent::Ended { error: None },
            },
        )
        .0;
        assert_eq!(crate::print_layout(layout(&s)), ">#1[game, *file:a.fun]");
    }

    #[test]
    fn the_end_of_run_step_waits_for_a_quiet_layout_and_an_untouched_card() {
        let steps = vec![step("The rule", game_fun(None))];
        // The user moved a tab a second before the run ended.
        let mut s = running();
        s.clock = 10_000;
        let (mut s, _) = update(
            s,
            Event::Client(ClientMsg::Layout {
                op: LayoutOp::Split {
                    pane: 1,
                    direction: crate::layout::Direction::Down,
                },
            }),
        );
        let (s2, _) = update(s.clone(), present(steps.clone()));
        let mut busy = s2.clone();
        busy.clock = 11_000;
        let (busy, _) = ended(busy);
        assert_eq!(walkthrough(&busy).2, None);
        // Three seconds later it is quiet enough.
        s = s2;
        s.clock = 10_000 + AUTO_QUIET_MS;
        let (quiet, _) = ended(s);
        assert_eq!(walkthrough(&quiet).2, Some(0));
        // A card the user already stepped through is theirs.
        let (s, _) = update(
            running(),
            present(vec![step("a", game_fun(None)), step("b", ViewTarget::Game)]),
        );
        let (s, _) = update(
            s,
            Event::Client(ClientMsg::WalkthroughStep {
                task: 1,
                item: 1,
                step: 1,
            }),
        );
        let mut s = s;
        s.clock = 60_000;
        let before = layout(&s).clone();
        let (s, _) = ended(s);
        assert_eq!(walkthrough(&s).2, Some(1));
        assert_eq!(layout(&s), &before);
    }

    #[test]
    fn a_clicked_step_opens_as_the_users_own_and_is_marked_current() {
        let (s, _) = update(
            running(),
            present(vec![
                step("a", game_fun(Some([2, 2]))),
                step("b", ViewTarget::Game),
            ]),
        );
        let (s, commands) = update(
            s,
            Event::Client(ClientMsg::WalkthroughStep {
                task: 1,
                item: 1,
                step: 0,
            }),
        );
        // Beside, focused there (a click is the user's), not marked as the agent's.
        assert_eq!(
            crate::print_layout(layout(&s)),
            "right 50% (#1[game, *file:a.fun] | >#3[*file:game.fun])"
        );
        assert!(layout(&s).agent.tabs.is_empty());
        assert_eq!(walkthrough(&s).2, Some(0));
        assert_eq!(s.highlight.as_ref().unwrap().lines, Some([2, 2]));
        assert!(commands
            .iter()
            .any(|c| matches!(c, Command::SaveTask { .. })));
        // The game step focuses its tab where it is.
        let (s, _) = update(
            s,
            Event::Client(ClientMsg::WalkthroughStep {
                task: 1,
                item: 1,
                step: 1,
            }),
        );
        assert_eq!(
            crate::print_layout(layout(&s)),
            "right 50% (>#1[*game, file:a.fun] | #3[*file:game.fun])"
        );
        // A step whose file is gone is refused, not applied.
        let mut gone = s.clone();
        gone.files.retain(|f| f != "game.fun");
        let (after, commands) = update(
            gone,
            Event::Client(ClientMsg::WalkthroughStep {
                task: 1,
                item: 1,
                step: 0,
            }),
        );
        assert!(
            matches!(&commands[..], [Command::Reply(ServerMsg::Rejected { .. })]),
            "{commands:?}"
        );
        assert_eq!(walkthrough(&after).2, Some(1));
        // No such item or step.
        for (item, step) in [(0, 0), (1, 5)] {
            let (_, commands) = update(
                s.clone(),
                Event::Client(ClientMsg::WalkthroughStep {
                    task: 1,
                    item,
                    step,
                }),
            );
            assert!(
                matches!(&commands[..], [Command::Reply(ServerMsg::Rejected { .. })]),
                "{commands:?}"
            );
        }
    }

    #[test]
    fn the_highlight_goes_when_its_tab_is_covered_or_scrolled_away() {
        let (s, _) = update(running(), show(game_fun(Some([4, 5]))));
        let seq = s.highlight.as_ref().unwrap().seq;
        // A stale clear leaves a newer highlight.
        let (s, _) = update(s, Event::Client(ClientMsg::ClearHighlight { seq: seq + 1 }));
        assert!(s.highlight.is_some());
        let (cleared, _) = update(s.clone(), Event::Client(ClientMsg::ClearHighlight { seq }));
        assert_eq!(cleared.highlight, None);
        // The user opens something over it.
        let open = LayoutOp::Open {
            view: View::Changes { task: 1 },
            target: Target::Pane { pane: 3 },
        };
        let (covered, _) = update(s, Event::Client(ClientMsg::Layout { op: open }));
        assert_eq!(covered.highlight, None);
    }

    #[test]
    fn the_tool_server_gets_the_context_again_when_the_layout_changes() {
        let s = running();
        assert_eq!(s.contexts[&1].layout.panes.len(), 1);
        let (s, commands) = update(s, show(game_fun(None)));
        let contexts: Vec<&AppContext> = commands
            .iter()
            .filter_map(|c| match c {
                Command::AppContext {
                    task: 1,
                    run: 0,
                    context,
                } => Some(context),
                _ => None,
            })
            .collect();
        assert_eq!(contexts.len(), 1, "{commands:?}");
        let panes = &contexts[0].layout.panes;
        assert_eq!(panes.len(), 2);
        assert!(panes[0].focused && !panes[1].focused);
        assert_eq!(panes[1].shown, Some(file("game.fun")));
        assert_eq!(contexts[0].layout.agent_tabs, [file("game.fun")]);
        // Nothing changed, nothing sent.
        let (s, commands) = update(s, Event::Client(ClientMsg::GetState));
        assert!(!commands
            .iter()
            .any(|c| matches!(c, Command::AppContext { .. })));
        // Switching projects and placements are news too.
        let place = LayoutOp::Place {
            placement: Placement::Mini,
        };
        let (s, commands) = update(s, Event::Client(ClientMsg::Layout { op: place }));
        assert!(commands.iter().any(|c| matches!(c, Command::AppContext { context, .. } if context.layout.game == Placement::Mini)));
        let (s, commands) = update(
            s,
            Event::Client(ClientMsg::SetActiveProject {
                path: "examples/b".into(),
            }),
        );
        assert!(commands.iter().any(|c| matches!(c, Command::AppContext { context, .. } if context.active_project.as_deref() == Some("examples/b"))));
        // A run that ends is forgotten.
        let (s, _) = ended(s);
        assert!(s.contexts.is_empty());
    }

    #[test]
    fn walkthroughs_persist_with_the_transcript() {
        let (s, _) = update(
            running(),
            present(vec![WalkthroughStep {
                title: "The rule".into(),
                note: Some("It counts by two.".into()),
                view: ViewTarget::Changes {
                    path: Some("game.fun".into()),
                },
            }]),
        );
        let task = s.tasks[0].clone();
        let back = crate::tasks::from_stored(&crate::tasks::to_stored(&task)).unwrap();
        assert_eq!(back.transcript, task.transcript);
        // Unknown ops, kinds and request fields are refused.
        for bad in [
            r#"{"op": "show", "view": {"kind": "game"}, "beside": true}"#,
            r#"{"op": "show", "view": {"kind": "task", "task": 1}}"#,
            r#"{"op": "close", "view": {"kind": "game"}}"#,
        ] {
            assert!(serde_json::from_str::<ViewRequest>(bad).is_err(), "{bad}");
        }
        let ok = r#"{"op": "present", "steps": [{"title": "t", "view": {"kind": "file", "path": "a.fun", "lines": [1, 2]}}]}"#;
        assert!(serde_json::from_str::<ViewRequest>(ok).is_ok());
    }

    #[test]
    fn a_stacked_canvas_gets_a_background_tab_and_a_mini_game_stays_put() {
        let (s, _) = update(
            running(),
            Event::Client(ClientMsg::CanvasStacked { stacked: true }),
        );
        let (s, commands) = update(s, show(game_fun(Some([1, 1]))));
        assert!(
            reply(&commands).message.contains("too narrow to split"),
            "{commands:?}"
        );
        assert_eq!(
            crate::print_layout(layout(&s)),
            ">#1[game, *file:a.fun, file:game.fun]"
        );
        let place = LayoutOp::Place {
            placement: Placement::Mini,
        };
        let (s, _) = update(s, Event::Client(ClientMsg::Layout { op: place }));
        let before = layout(&s).clone();
        let (s, commands) = update(s, show(ViewTarget::Game));
        assert!(reply(&commands)
            .message
            .contains("already in sight in its mini view"));
        assert_eq!(layout(&s), &before);
    }
}
