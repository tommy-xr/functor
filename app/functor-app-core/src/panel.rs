//! The headset panel's content (D28), derived from the state for each
//! connected headset and sent when it changes; and the panel's actions,
//! which become the client messages the desktop UI sends for the same
//! choices.

use crate::activity::Stance;
use crate::decision::{count, Depth, Kind};
use crate::devices::{DeviceState, LinkCommand};
use crate::layout::Placement;
use crate::protocol::ClientMsg;
use crate::state::{AppState, Command};
use crate::tasks::Task;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelTask {
    pub id: u64,
    pub title: String,
    pub stance: Stance,
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
    Select { task: u64 },
    Keep { task: u64 },
    Revert { task: u64, tree: String },
    Stop { task: u64 },
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
                activity: activity
                    .and_then(|a| a.calls.first().map(|c| &c.label).or(a.error.as_ref()))
                    .cloned()
                    .unwrap_or_default(),
                decision: decision(state, t),
            }
        })
        .collect();
    Panel {
        project: Some(project),
        placed,
        now,
        tasks,
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
    let linked = state
        .devices
        .list
        .iter()
        .any(|d| d.conn == Some(conn) && matches!(d.state, DeviceState::Connected { .. }));
    if !linked {
        return Err(None);
    }
    let task = match &action {
        PanelAction::Select { task }
        | PanelAction::Keep { task }
        | PanelAction::Revert { task, .. }
        | PanelAction::Stop { task } => *task,
    };
    if !state
        .tasks
        .iter()
        .any(|t| t.id == task && Some(&t.project) == state.active.as_ref())
    {
        return Err(Some(format!("there is no task {task} here")));
    }
    let msg = match action {
        PanelAction::Select { task } => ClientMsg::SelectTask {
            task: Some(task),
            if_unset: false,
        },
        PanelAction::Keep { task } => ClientMsg::KeepChange { task },
        PanelAction::Revert { task, tree } => ClientMsg::RevertChange { task, tree },
        PanelAction::Stop { task } => ClientMsg::StopTask { task },
    };
    Ok(msg)
}
