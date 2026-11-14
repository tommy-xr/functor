//! The headset panel on Quest ([`link_panel`]): its content arrives from
//! the desktop over the link ([`functor_device_link::panel`]), its actions
//! go back the same way, it stands in the world where it opened, and while
//! it is closed the wrist light shows the stance of the task in front of
//! the user.

use std::rc::Rc;
use std::time::Instant;

use cgmath::Matrix4;
use functor_device_link::client::{Client, View};
use functor_device_link::panel::{self as wire, Action, Update, ACTION_CHANNEL};
use functor_runtime_common::debug_protocol::{PanelState, TaskStance, VoiceToast, WristLight};
use functor_runtime_common::link_overlay::Shape;
use functor_runtime_common::link_panel::{
    self, Connection, ConversationData, Destination, Out, Panel, PanelAction, PanelData,
    PanelDecision, PanelRoute, PanelTask, ProgressLine, ProgressStep, Role, SaidLine,
};
use functor_runtime_common::XrInputSnapshot;

#[derive(Default)]
pub struct HeadsetPanel {
    pub nav: Panel,
    /// The desktop's latest, while linked.
    pub data: Option<PanelData>,
    /// Where it stands (stage space), from the first frame drawn open.
    pub anchor: Option<Matrix4<f32>>,
    /// Game menu chosen: one menu press for the game's next step.
    menu_tap: bool,
    /// Counts the desktop's updates, so the drawing knows its data changed.
    generation: u64,
    /// The open panel's shapes and what they were laid out from: laid out
    /// again only when that changes.
    drawn: Option<(Drawn, Rc<Vec<Shape>>)>,
}

#[derive(PartialEq)]
struct Drawn {
    nav: Panel,
    generation: u64,
    connection: Connection,
    toast: Option<VoiceToast>,
    notice: bool,
}

impl HeadsetPanel {
    /// A message on the panel's update channel.
    pub fn received(&mut self, bytes: &[u8], now: Instant) {
        match Update::decode(bytes) {
            Ok(Update::Panel(panel)) => {
                self.data = Some(data(panel));
                self.generation += 1;
            }
            Ok(Update::Refused { reason }) => self.nav.refused(reason, now),
            Err(e) => log::warn!("panel: an update we cannot read: {e}"),
        }
    }

    pub fn toggle(&mut self) {
        self.nav.toggle();
        self.anchor = None;
    }

    /// One frame's sample, while the link client runs.
    pub fn frame(&mut self, client: &Client, sample: Option<&XrInputSnapshot>, busy: bool) {
        if !client.linked() && self.data.take().is_some() {
            self.generation += 1;
        }
        match self.nav.step(sample, self.data.as_ref(), busy) {
            Some(Out::Action(action)) => {
                log::info!("panel: {action:?}");
                let sent = client.link().is_some_and(|link| {
                    client.send_channel(link, ACTION_CHANNEL, encode(action), false)
                });
                if !sent {
                    self.nav
                        .refused("not connected to the desktop".into(), Instant::now());
                }
            }
            Some(Out::GameMenu) => {
                self.nav.close();
                self.menu_tap = true;
            }
            Some(Out::Close) => self.nav.close(),
            None => {}
        }
    }

    /// What the game sees: with `owns_menu` (the overlay reads the sample
    /// the game gets) the menu button is the shell's, and a Game menu choice
    /// hands it one press on a frame that steps; while the panel is open, or
    /// its controls are still held after, so are they.
    pub fn game_view(&mut self, xr: &mut XrInputSnapshot, stepping: bool, owns_menu: bool) {
        if self.nav.hides_controls() {
            link_panel::game_view(xr);
        }
        if owns_menu {
            xr.left.menu_pressed = self.menu_tap && stepping;
        }
        self.menu_tap &= !stepping;
    }

    /// The open panel's shapes, laid out again only when what they show
    /// changed.
    pub fn shapes(
        &mut self,
        connection: Connection,
        toast: Option<VoiceToast>,
        now: Instant,
    ) -> Rc<Vec<Shape>> {
        let key = Drawn {
            nav: self.nav.clone(),
            generation: self.generation,
            connection,
            toast,
            notice: self.nav.notice_shown(now),
        };
        match &self.drawn {
            Some((drawn, shapes)) if *drawn == key => shapes.clone(),
            _ => {
                let shapes = Rc::new(self.nav.shapes(
                    self.data.as_ref(),
                    &key.connection,
                    key.toast.as_ref(),
                    now,
                ));
                self.drawn = Some((key, shapes.clone()));
                shapes
            }
        }
    }

    pub fn wrist(&self, linked: bool) -> Option<WristLight> {
        link_panel::wrist(self.data.as_ref(), linked && !self.nav.is_open())
    }

    pub fn debug(&self, linked: bool) -> PanelState {
        self.nav.debug(self.data.as_ref(), self.wrist(linked))
    }
}

/// The header's state from the client's view.
pub fn connection(view: &View) -> Connection {
    let desktop = view.paired.clone();
    match (desktop, view.linked, view.outdated) {
        (None, _, _) => Connection::NotPaired,
        (Some(desktop), true, _) => Connection::Connected { desktop },
        (Some(desktop), false, true) => Connection::Outdated { desktop },
        (Some(desktop), false, false) => Connection::NotConnected { desktop },
    }
}

fn data(p: wire::Panel) -> PanelData {
    PanelData {
        project: p.project,
        placed: p.placed,
        now: p.now,
        tasks: p
            .tasks
            .into_iter()
            .map(|t| PanelTask {
                id: t.id,
                title: t.title,
                stance: stance(t.stance),
                activity: t.activity,
                decision: t.decision.map(|d| PanelDecision {
                    tree: d.tree,
                    size: d.size,
                    glance: d.glance,
                    reason: d.reason,
                    summary: d.summary,
                    notes: d.notes,
                }),
            })
            .collect(),
        waiting: p.waiting,
        route: p.route.map(|r| PanelRoute {
            id: r.id,
            line: r.line,
            moves: (r.moves.into_iter())
                .map(|m| {
                    let to = match m.to {
                        wire::Destination::Steer { task } => Destination::Steer { task },
                        wire::Destination::New => Destination::New,
                        wire::Destination::Queued { task } => Destination::Queued { task },
                    };
                    (m.label, to)
                })
                .collect(),
        }),
        conversation: p.conversation.map(|c| ConversationData {
            task: c.task,
            title: c.title,
            stance: stance(c.stance),
            activity: c.activity,
            question: c.question,
            first: c.first,
            items: (c.items.into_iter())
                .map(|i| SaidLine {
                    role: match i.role {
                        wire::Role::You => Role::You,
                        wire::Role::Agent => Role::Agent,
                        wire::Role::App => Role::App,
                    },
                    spoken: i.spoken,
                    cut: i.cut,
                    text: i.text,
                })
                .collect(),
        }),
        progress: p.progress.map(|g| ProgressLine {
            step: match g.step {
                wire::Step::Received => ProgressStep::Received,
                wire::Step::Transcribed => ProgressStep::Transcribed,
                wire::Step::Processing => ProgressStep::Processing,
                wire::Step::Agent => ProgressStep::Agent,
                wire::Step::Outcome => ProgressStep::Outcome,
            },
            detail: g.detail,
            failed: g.failed,
        }),
    }
}

fn stance(s: wire::Stance) -> TaskStance {
    match s {
        wire::Stance::Thinking => TaskStance::Thinking,
        wire::Stance::Working => TaskStance::Working,
        wire::Stance::Waiting => TaskStance::Waiting,
        wire::Stance::Done => TaskStance::Done,
        wire::Stance::Failed => TaskStance::Failed,
        wire::Stance::Queued => TaskStance::Queued,
    }
}

fn encode(action: PanelAction) -> Vec<u8> {
    match action {
        PanelAction::Select { task } => Action::Select { task },
        PanelAction::Keep { task } => Action::Keep { task },
        PanelAction::Revert { task, tree } => Action::Revert { task, tree },
        PanelAction::Stop { task } => Action::Stop { task },
        PanelAction::Move { id, to } => Action::Move {
            id,
            to: match to {
                Destination::Steer { task } => wire::Destination::Steer { task },
                Destination::New => wire::Destination::New,
                Destination::Queued { task } => wire::Destination::Queued { task },
            },
        },
    }
    .encode()
}
