//! The headset panel's content (D28), derived from the state for each
//! connected headset and sent when it changes; and the panel's actions,
//! which become the client messages the desktop UI sends for the same
//! choices.

use crate::activity::Stance;
use crate::decision::{count, Depth, Kind};
use crate::devices::{DeviceState, LinkCommand};
use crate::layout::Placement;
use crate::protocol::ClientMsg;
use crate::queue::Wait;
use crate::route::{Destination, Move};
use crate::state::{AppState, Command};
use crate::tasks::{Task, TaskStatus};

/// Most tasks a panel lists; notes on its decision page.
pub const TASKS: usize = 6;
pub const NOTES: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    /// The open project; `None` with no folder open.
    pub project: Option<String>,
    /// Its game is placed on this headset.
    pub placed: bool,
    /// The task in front of the user: the running one, else the latest
    /// waiting on the user, else the latest.
    pub now: Option<u64>,
    /// `now` first, then those waiting on the user, then the latest.
    pub tasks: Vec<PanelTask>,
    /// While tasks are queued, why they wait: "Waiting for your decision
    /// on “…”".
    pub waiting: Option<String>,
    /// This headset's latest utterance's route, while it can be moved.
    pub route: Option<PanelRoute>,
}

/// Where the router put an utterance (D29), and where it can go instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRoute {
    pub id: u64,
    pub line: String,
    pub moves: Vec<Move>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelTask {
    pub id: u64,
    pub title: String,
    pub stance: Stance,
    /// In the project's queue (D30).
    pub queued: bool,
    /// Its running call's label, its error, or empty.
    pub activity: String,
    pub decision: Option<PanelDecision>,
}

/// A waiting change's first page (D35), as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelDecision {
    /// `Change::tree`, which a revert must name.
    pub tree: String,
    pub size: String,
    pub glance: bool,
    pub reason: String,
    pub summary: String,
    pub notes: Vec<String>,
}

/// What the headset's user chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    Select {
        task: u64,
    },
    Keep {
        task: u64,
    },
    Revert {
        task: u64,
        tree: String,
    },
    Stop {
        task: u64,
    },
    /// Move route `id` to `to`.
    Move {
        id: u64,
        to: Destination,
    },
}

/// The panel for headset `device`: the open project's tasks.
pub fn panel(state: &AppState, device: &str) -> Panel {
    let Some(project) = state.active.clone() else {
        return Panel::default();
    };
    let placed = state.layouts.get(&project).is_some_and(|l| {
        l.game.placement == Placement::Headset && l.game.headset.as_deref() == Some(device)
    });
    let stance = |t: &Task| state.activity.get(&t.id).map_or(Stance::Done, |a| a.stance);
    let latest = |t: &&Task| (t.updated_at.or(t.created_at), t.id);
    let mut tasks: Vec<&Task> = state
        .tasks
        .iter()
        .filter(|t| t.project == project)
        .collect();
    tasks.sort_by_key(|t| std::cmp::Reverse(latest(t)));
    let now = tasks
        .iter()
        .find(|t| t.run.is_some())
        .or_else(|| tasks.iter().find(|t| stance(t) == Stance::Waiting))
        .or(tasks.first())
        .map(|t| t.id);
    // Stable: within each group, the latest first.
    tasks.sort_by_key(|t| (Some(t.id) != now, stance(t) != Stance::Waiting));
    let tasks = tasks
        .into_iter()
        .take(TASKS)
        .map(|t| {
            let activity = state.activity.get(&t.id);
            PanelTask {
                id: t.id,
                title: t.title.clone(),
                stance: stance(t),
                queued: t.status == TaskStatus::Queued,
                activity: activity
                    .and_then(|a| a.calls.first().map(|c| &c.label).or(a.error.as_ref()))
                    .cloned()
                    .unwrap_or_default(),
                decision: decision(state, t),
            }
        })
        .collect();
    let title = |id: u64| {
        let t = state.tasks.iter().find(|t| t.id == id);
        t.map_or(format!("task {id}"), |t| t.title.clone())
    };
    let waiting = state
        .queues
        .get(&project)
        .filter(|q| !q.tasks.is_empty())
        .and_then(|q| q.wait.as_ref())
        .map(|w| match w {
            Wait::Decision { task } => format!(
                "Waiting for your decision on \u{201c}{}\u{201d}",
                title(*task)
            ),
            Wait::Running { task } => format!("Starts after \u{201c}{}\u{201d}", title(*task)),
            Wait::Reverting { task } => {
                format!("Waiting for \u{201c}{}\u{201d} to revert", title(*task))
            }
            Wait::Measuring { task } => format!(
                "Waiting for \u{201c}{}\u{201d}'s change to be measured",
                title(*task)
            ),
        });
    // The newest open decision on this headset's speech.
    let route = state
        .routed
        .iter()
        .rev()
        .filter(|o| o.device == device)
        .find_map(|o| state.routes.iter().find(|r| r.id == o.id))
        .map(|r| PanelRoute {
            id: r.id,
            line: r.line.clone(),
            moves: r.moves.clone(),
        });
    Panel {
        project: Some(project),
        placed,
        now,
        tasks,
        waiting,
        route,
    }
}

fn decision(state: &AppState, task: &Task) -> Option<PanelDecision> {
    let d = state.decisions.get(&task.id)?;
    let report = task.report.as_ref();
    let files = count(d.size.files, "file", "files");
    let mut size = vec![format!("+{} -{}", d.size.added, d.size.removed), files];
    size.extend(d.size.kinds.iter().map(|k| {
        match k {
            Kind::GameLogic => "game logic",
            Kind::Assets => "assets",
            Kind::Config => "config",
            Kind::Other => "other files",
        }
        .to_string()
    }));
    Some(PanelDecision {
        tree: task.change.as_ref()?.tree.clone(),
        size: size.join(", "),
        glance: d.depth == Depth::Glance,
        reason: d.reason.clone(),
        summary: report.and_then(|r| r.summary.clone()).unwrap_or_default(),
        notes: report
            .and_then(|r| r.notes.as_ref())
            .map(|n| n.iter().take(NOTES).cloned().collect())
            .unwrap_or_default(),
    })
}

/// For each connected headset whose panel changed since it was last sent,
/// or whose link is new (a relink may come before the old one drops), a
/// `LinkCommand::Panel`; forgets the rest.
pub(crate) fn refresh_panels(state: &mut AppState, commands: &mut Vec<Command>) {
    let connected: Vec<(String, u64)> = state
        .devices
        .list
        .iter()
        .filter(|d| matches!(d.state, DeviceState::Connected { .. }))
        .filter_map(|d| Some((d.id.clone(), d.conn?)))
        .collect();
    state
        .panels
        .retain(|id, _| connected.iter().any(|(c, _)| c == id));
    for (device, conn) in connected {
        let panel = panel(state, &device);
        let sent = state.panels.get(&device);
        if sent.is_none_or(|(c, p)| *c != conn || *p != panel) {
            state.panels.insert(device.clone(), (conn, panel.clone()));
            commands.push(Command::Link(LinkCommand::Panel {
                conn,
                panel: Box::new(panel),
            }));
        }
    }
}

/// The client message for `action` from connection `conn`; refused (with
/// why, to answer) unless `conn` is a connected headset's link and the
/// task is in its panel's project. An unlinked connection gets no answer.
pub(crate) fn client_msg(
    state: &AppState,
    conn: u64,
    action: PanelAction,
) -> Result<ClientMsg, Option<String>> {
    let Some(device) = state
        .devices
        .list
        .iter()
        .find(|d| d.conn == Some(conn) && matches!(d.state, DeviceState::Connected { .. }))
    else {
        return Err(None);
    };
    let (task, msg) = match action {
        // A route is the headset's own utterance's, as its panel shows it.
        PanelAction::Move { id, to } => {
            let own = state
                .routed
                .iter()
                .any(|o| o.id == id && o.device == device.id);
            if !own || !state.routes.iter().any(|r| r.id == id) {
                return Err(Some("that message can no longer be moved".to_string()));
            }
            return Ok(ClientMsg::MoveUtterance { id, to });
        }
        PanelAction::Select { task } => (
            task,
            ClientMsg::SelectTask {
                task: Some(task),
                if_unset: false,
            },
        ),
        PanelAction::Keep { task } => (task, ClientMsg::KeepChange { task }),
        PanelAction::Revert { task, tree } => (task, ClientMsg::RevertChange { task, tree }),
        PanelAction::Stop { task } => (task, ClientMsg::StopTask { task }),
    };
    if !state
        .tasks
        .iter()
        .any(|t| t.id == task && Some(&t.project) == state.active.as_ref())
    {
        return Err(Some(format!("there is no task {task} here")));
    }
    Ok(msg)
}
