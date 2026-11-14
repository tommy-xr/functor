//! The headset panel (D28): the shell draws it over any game, opened and
//! closed by a short press of the left menu button
//! ([`crate::link_overlay::Gesture::Panel`]). Sections Now, Talk, Tasks and
//! Game menu, a task's page with its decision (D35), and a task's
//! conversation (D41): what was said, read without taking the headset off.
//!
//! The right controller drives it: the thumbstick moves a focus ring, A
//! chooses, B goes back (closes, from the section list). Keep, Revert and
//! Stop each wait for a second A, so one stray press changes nothing. While
//! it is open the game gets poses but not those controls ([`game_view`]).
//! Pure: the shell owns the link, the clock and GL.

use std::cell::Cell;
use std::time::{Duration, Instant};

use cgmath::{vec3, Matrix4, One, Quaternion, Vector3};

use crate::debug_protocol::{PanelState, TaskStance, VoiceToast, WristLight};
use crate::input::XrInputSnapshot;
use crate::link_overlay::{toast_lines, Shape};
use crate::overlay_font::{self, fit, wrap};
use crate::TrackingPose;

/// The desktop's content for the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelData {
    pub project: Option<String>,
    /// The project's game is placed on this headset.
    pub placed: bool,
    /// The task in front of the user, one of `tasks`.
    pub now: Option<u64>,
    pub tasks: Vec<PanelTask>,
    /// Why queued tasks wait; empty when none are queued.
    pub waiting: String,
    /// Where this headset's latest utterance went, while it can be moved.
    pub route: Option<PanelRoute>,
    /// The current task's recent conversation.
    pub conversation: Option<ConversationData>,
    /// Where this headset's request in flight is (D43).
    pub progress: Option<ProgressLine>,
}

/// One request's step (D43): one line that moves through the steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressLine {
    pub step: ProgressStep,
    pub detail: String,
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressStep {
    Received,
    Transcribed,
    Processing,
    Agent,
    Outcome,
}

impl ProgressLine {
    /// "PROCESSING  Steered ..."; a failure names its step.
    fn text(&self) -> String {
        let step = match self.step {
            ProgressStep::Received => "RECEIVED",
            ProgressStep::Transcribed => "HEARD",
            ProgressStep::Processing => "PROCESSING",
            ProgressStep::Agent => "AGENT",
            ProgressStep::Outcome => "DONE",
        };
        match (self.failed, self.detail.is_empty()) {
            (true, _) => format!("FAILED AT {step}  {}", self.detail),
            (false, true) => step.to_string(),
            (false, false) => format!("{step}  {}", self.detail),
        }
    }
}

/// A task's recent conversation, as the desktop sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationData {
    pub task: u64,
    pub title: String,
    pub stance: TaskStance,
    pub activity: String,
    /// The agent waits on a question: its last line.
    pub question: bool,
    /// The index of `items[0]` among all the task's items; older ones stay
    /// on the desktop.
    pub first: u32,
    pub items: Vec<SaidLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaidLine {
    pub role: Role,
    pub spoken: bool,
    /// The desktop cut it.
    pub cut: bool,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    You,
    Agent,
    App,
}

/// The router's decision on an utterance (D29) and its moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRoute {
    pub id: u64,
    pub line: String,
    pub moves: Vec<(String, Destination)>,
}

/// Where a route can be moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Steer { task: u64 },
    New,
    Queued { task: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelTask {
    pub id: u64,
    pub title: String,
    pub stance: TaskStance,
    pub activity: String,
    pub decision: Option<PanelDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelDecision {
    pub tree: String,
    pub size: String,
    pub glance: bool,
    pub reason: String,
    pub summary: String,
    pub notes: Vec<String>,
}

/// What the user chose, for the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    Select { task: u64 },
    Keep { task: u64 },
    Revert { task: u64, tree: String },
    Stop { task: u64 },
    Move { id: u64, to: Destination },
}

/// The link as the header shows it (D27), each with its one fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    NotPaired,
    /// Paired, link down: away or unreachable look the same from here.
    NotConnected {
        desktop: String,
    },
    /// The desktop refused this app's link version.
    Outdated {
        desktop: String,
    },
    Connected {
        desktop: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Now,
    Talk,
    Tasks,
    GameMenu,
}

const SECTIONS: [Section; 4] = [
    Section::Now,
    Section::Talk,
    Section::Tasks,
    Section::GameMenu,
];

/// What a step asks of the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Action(PanelAction),
    /// Close, and hand the game one menu press.
    GameMenu,
    Close,
}

/// A focusable thing in the content column.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    /// Open the task's page.
    Page(u64),
    /// Open the task's conversation.
    Conversation(u64),
    /// The change's tree: a different change cancels a waiting confirm.
    Keep(u64, String),
    Revert(u64, String),
    Stop(u64),
    /// Move route `id`'s utterance to this destination.
    Move(u64, Destination),
}

/// How long a refusal from the desktop shows.
const NOTICE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    open: bool,
    section: Section,
    /// The task whose page shows, over the section.
    page: Option<u64>,
    /// The focused content item; `None` is the section list.
    focus: Option<usize>,
    /// The action waiting for its confirm press.
    confirm: Option<Item>,
    notice: Option<(String, Instant)>,
    stick: [i8; 2],
    a: bool,
    b: bool,
    /// Closed with the right hand still on its controls: they stay the
    /// panel's until let go.
    latched: bool,
    /// The task whose conversation shows, over the section or page.
    conversation: Option<u64>,
    /// Its first line in view, as the item it belongs to (an index among
    /// all the task's items, so new items do not move it) and a line within
    /// it; `None` follows the newest.
    scroll: Option<(u32, usize)>,
    /// Long agent messages shown whole.
    expanded: bool,
    /// How many of its lines fit, as last drawn.
    shown: Cell<usize>,
}

/// Lines a stick push scrolls the conversation.
const SCROLL: usize = 3;
/// An agent message's lines before "more".
const AGENT_LINES: usize = 6;

/// A stick axis as one press per push: past 0.6 fires once, back under 0.3
/// re-arms.
fn push(last: &mut i8, v: f32) -> i8 {
    let dir = match v {
        v if v > 0.6 => 1,
        v if v < -0.6 => -1,
        v if v.abs() < 0.3 => 0,
        _ => *last,
    };
    let fired = if dir != *last { dir } else { 0 };
    *last = dir;
    fired
}

fn running(stance: TaskStance) -> bool {
    matches!(stance, TaskStance::Thinking | TaskStance::Working)
}

impl Panel {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens on Now, or closes.
    pub fn toggle(&mut self) {
        if self.open {
            return self.close();
        }
        *self = Panel {
            open: true,
            notice: self.notice.take(),
            stick: self.stick,
            a: self.a,
            b: self.b,
            ..Panel::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.latched = true;
        self.confirm = None;
    }

    /// The game does not see the panel's controls ([`game_view`]): while
    /// it is open, and after it closes until they are let go.
    pub fn hides_controls(&self) -> bool {
        self.open || self.latched
    }

    /// The user started talking: show the Talk section, unless the
    /// conversation shows (its stages show inline there).
    pub fn talk(&mut self) {
        if self.open && self.conversation.is_none() {
            self.section = Section::Talk;
            self.page = None;
            self.focus = None;
            self.confirm = None;
        }
    }

    /// A refusal shows at `now`.
    pub fn notice_shown(&self, now: Instant) -> bool {
        self.notice.as_ref().is_some_and(|(_, until)| now < *until)
    }

    /// The desktop refused the last action.
    pub fn refused(&mut self, reason: String, now: Instant) {
        self.notice = Some((reason, now + NOTICE));
    }

    /// One controller sample. `busy` while push-to-talk holds the right
    /// hand: the panel takes no input then.
    pub fn step(
        &mut self,
        xr: Option<&XrInputSnapshot>,
        data: Option<&PanelData>,
        busy: bool,
    ) -> Option<Out> {
        let (stick, a, b) = xr.map_or(([0.0; 2], false, false), |xr| {
            (
                xr.right.thumbstick,
                xr.right.primary_pressed,
                xr.right.secondary_pressed,
            )
        });
        let held = a || b || stick.iter().any(|v| v.abs() >= 0.3);
        let pressed = |was: &mut bool, is: bool| !std::mem::replace(was, is) && is;
        let (a, b) = (pressed(&mut self.a, a), pressed(&mut self.b, b));
        let [x, y] = [
            push(&mut self.stick[0], stick[0]),
            push(&mut self.stick[1], stick[1]),
        ];
        // A diagonal push is one move: up or down wins.
        let x = if y != 0 { 0 } else { x };
        if !self.open {
            self.latched &= held;
            return None;
        }
        let confirming = self.confirm.is_some();
        self.settle(data);
        if busy || (confirming && self.confirm.is_none()) {
            return None;
        }
        if let Some(item) = &self.confirm {
            let action = match item {
                Item::Keep(task, _) => PanelAction::Keep { task: *task },
                Item::Revert(task, tree) => PanelAction::Revert {
                    task: *task,
                    tree: tree.clone(),
                },
                Item::Stop(task) => PanelAction::Stop { task: *task },
                Item::Move(id, to) => PanelAction::Move { id: *id, to: *to },
                Item::Page(_) | Item::Conversation(_) => {
                    unreachable!("pages open without a confirm")
                }
            };
            if a || b {
                self.confirm = None;
            }
            return a.then_some(Out::Action(action));
        }
        if let Some(id) = self.conversation {
            self.scroll_step(data, id, a, b, y);
            return None;
        }
        // Back from a page to the list, on the task it opened.
        if b {
            if let Some(id) = self.page.take() {
                self.focus = self.items(data).iter().position(|i| *i == Item::Page(id));
                return None;
            }
        }
        let items = self.items(data);
        let Some(at) = self.focus else {
            // The section list: up is +y.
            if y != 0 {
                let i = SECTIONS
                    .iter()
                    .position(|s| *s == self.section)
                    .unwrap_or(0);
                let i = (i as i32 - i32::from(y)).clamp(0, SECTIONS.len() as i32 - 1);
                self.section = SECTIONS[i as usize];
                self.page = None;
            }
            if a && self.section == Section::GameMenu {
                return Some(Out::GameMenu);
            }
            if (a || x == 1) && !items.is_empty() {
                self.focus = Some(0);
            }
            return b.then_some(Out::Close);
        };
        if y != 0 {
            self.focus = Some((at as i32 - i32::from(y)).clamp(0, items.len() as i32 - 1) as usize);
        }
        if b || x == -1 {
            self.focus = None;
        } else if a {
            match items[self.focus.unwrap_or(at)].clone() {
                Item::Page(id) => {
                    self.page = Some(id);
                    self.focus = (!self.items(data).is_empty()).then_some(0);
                    return Some(Out::Action(PanelAction::Select { task: id }));
                }
                // The desktop sends the selected task's conversation.
                Item::Conversation(id) => {
                    self.conversation = Some(id);
                    self.scroll = None;
                    self.expanded = false;
                    return Some(Out::Action(PanelAction::Select { task: id }));
                }
                item => self.confirm = Some(item),
            }
        }
        None
    }

    /// On the conversation page: B goes back, A shows long messages whole
    /// or cut, the stick scrolls; scrolled to the end it follows again.
    fn scroll_step(&mut self, data: Option<&PanelData>, id: u64, a: bool, b: bool, y: i8) {
        if b {
            self.conversation = None;
            self.scroll = None;
            return;
        }
        self.expanded ^= a;
        let conv = data.and_then(|d| d.conversation.as_ref());
        let Some(c) = conv.filter(|c| c.task == id).filter(|_| y != 0) else {
            return;
        };
        let lines = conversation_lines(c, self.expanded);
        let shown = self.shown.get().max(1);
        let top = self.top(&lines, shown);
        let top = match y > 0 {
            true => top.saturating_sub(SCROLL),
            false => top + SCROLL,
        };
        self.scroll = (top + shown < lines.len()).then(|| {
            let item = lines[top].3;
            (
                item,
                top - lines.iter().position(|l| l.3 == item).unwrap_or(top),
            )
        });
    }

    /// The conversation's first line in view: the scroll's place, or the
    /// newest `shown` when following.
    fn top(&self, lines: &[Laid], shown: usize) -> usize {
        match self.scroll {
            None => lines.len().saturating_sub(shown),
            Some((item, offset)) => {
                // Its item scrolled out of the window: from the oldest left.
                let at = lines.iter().position(|l| l.3 >= item).unwrap_or(0);
                (at + offset).min(lines.len().saturating_sub(1))
            }
        }
    }

    /// Follows the desktop's latest: a page whose task is gone closes, the
    /// focus stays on an item, a confirm whose item is gone is dropped.
    fn settle(&mut self, data: Option<&PanelData>) {
        let tasks = data.map_or(&[][..], |d| &d.tasks[..]);
        if self
            .page
            .is_some_and(|id| !tasks.iter().any(|t| t.id == id))
        {
            self.page = None;
        }
        let items = self.items(data);
        self.focus = match self.focus {
            Some(_) if items.is_empty() => None,
            Some(i) => Some(i.min(items.len() - 1)),
            None => None,
        };
        if self.confirm.as_ref().is_some_and(|c| !items.contains(c)) {
            self.confirm = None;
        }
    }

    fn items(&self, data: Option<&PanelData>) -> Vec<Item> {
        let Some(data) = data else { return vec![] };
        let find = |id: u64| data.tasks.iter().find(|t| t.id == id);
        // A task's page and Now open its conversation first.
        if let Some(t) = self.page.and_then(find) {
            let rest = match (&t.decision, running(t.stance)) {
                (Some(d), _) => vec![
                    Item::Keep(t.id, d.tree.clone()),
                    Item::Revert(t.id, d.tree.clone()),
                ],
                (None, true) => vec![Item::Stop(t.id)],
                (None, false) => vec![],
            };
            return [vec![Item::Conversation(t.id)], rest].concat();
        }
        match self.section {
            Section::Now => match data.now.and_then(find) {
                Some(t) if t.decision.is_some() => {
                    vec![Item::Conversation(t.id), Item::Page(t.id)]
                }
                Some(t) if running(t.stance) => vec![Item::Conversation(t.id), Item::Stop(t.id)],
                Some(t) => vec![Item::Conversation(t.id)],
                None => vec![],
            },
            Section::Tasks => data.tasks.iter().map(|t| Item::Page(t.id)).collect(),
            Section::Talk => data
                .route
                .iter()
                .flat_map(|r| r.moves.iter().map(|(_, to)| Item::Move(r.id, *to)))
                .collect(),
            Section::GameMenu => vec![],
        }
    }

    /// `/state.link.panel`.
    pub fn debug(&self, data: Option<&PanelData>, wrist: Option<WristLight>) -> PanelState {
        let conversation = data.and_then(|d| d.conversation.as_ref());
        PanelState {
            open: self.open,
            section: match self.section {
                Section::Now => "now",
                Section::Talk => "talk",
                Section::Tasks => "tasks",
                Section::GameMenu => "game_menu",
            }
            .to_string(),
            page: self.page,
            focus: self.focus,
            confirm: self.confirm.as_ref().map(|c| {
                match c {
                    Item::Keep(..) => "keep",
                    Item::Revert(..) => "revert",
                    Item::Stop(_) => "stop",
                    Item::Move(..) => "move",
                    Item::Page(_) => "page",
                    Item::Conversation(_) => "conversation",
                }
                .to_string()
            }),
            now: data.and_then(|d| d.now),
            tasks: data.map_or(0, |d| d.tasks.len()),
            wrist,
            conversation: self.conversation,
            following: self.conversation.is_some() && self.scroll.is_none(),
            said: conversation.map_or(0, |c| c.items.len()),
            question: conversation.is_some_and(|c| c.question),
        }
    }
}

/// What the game samples while the panel is open: its controls (the right
/// thumbstick, A, B) and the menu button are the panel's; poses, triggers
/// and the rest stay the game's.
pub fn game_view(xr: &mut XrInputSnapshot) {
    xr.right.thumbstick = [0.0; 2];
    xr.right.primary_pressed = false;
    xr.right.secondary_pressed = false;
    xr.left.menu_pressed = false;
}

/// How far ahead the panel opens, and how far below the eyes.
const DISTANCE: f32 = 0.9;
const DROP: f32 = 0.12;

/// `pose` as a matrix: its space to its parent's.
pub fn pose_matrix(pose: TrackingPose) -> Matrix4<f32> {
    let [x, y, z, w] = pose.orientation;
    Matrix4::from_translation(pose.position.into()) * Matrix4::from(Quaternion::new(w, x, y, z))
}

/// Where the panel stands when it opens in front of `head` (both in the
/// same tracking space): upright, facing the head, level with its yaw.
pub fn placement(head: TrackingPose) -> Matrix4<f32> {
    let [x, y, z, w] = head.orientation;
    // Looking nearly straight up or down: it opens along -Z.
    let yaw = crate::camera::yaw_only(Quaternion::new(w, x, y, z)).unwrap_or(Quaternion::one());
    Matrix4::from_translation(head.position.into())
        * Matrix4::from(yaw)
        * Matrix4::from_translation(vec3(0.0, -DROP, -DISTANCE))
}

/// Where on the left grip the wrist light sits: the back of the hand, toward
/// the wrist (OpenXR's left grip: +X out of the back of the hand, +Z toward
/// the wrist).
const WRIST_AT: [f32; 3] = [0.03, 0.0, 0.08];

/// The wrist light's position in head space, from rig-local `head` and
/// left `grip` poses of one sample.
pub fn wrist_position(head: TrackingPose, grip: TrackingPose) -> Option<[f32; 3]> {
    let grip = grip.relative_to(head)?;
    let at = pose_matrix(grip) * Vector3::from(WRIST_AT).extend(1.0);
    Some([at.x, at.y, at.z])
}

/// The wrist light while the panel is closed: the task in front of the
/// user's stance, while linked.
pub fn wrist(data: Option<&PanelData>, linked: bool) -> Option<WristLight> {
    let data = data.filter(|_| linked)?;
    let now = data.tasks.iter().find(|t| Some(t.id) == data.now)?;
    Some(WristLight {
        stance: now.stance,
        pulsing: now.stance == TaskStance::Waiting,
    })
}

/// Side of the wrist light, in metres.
pub const WRIST: f32 = 0.016;

/// The wrist light's shapes, centred on the origin, and its opacity `t`
/// seconds in: a slow pulse for "waiting", steady otherwise.
pub fn wrist_shapes(light: WristLight, t: f32) -> (Vec<Shape>, f32) {
    let [r, g, b] = stance_color(light.stance);
    let edge = WRIST * 1.4;
    let shapes = vec![
        Shape::Rect {
            rect: [-edge / 2.0, -edge / 2.0, edge, edge],
            color: [BG[0], BG[1], BG[2], 0.85],
        },
        Shape::Rect {
            rect: [-WRIST / 2.0, -WRIST / 2.0, WRIST, WRIST],
            color: [r, g, b, 1.0],
        },
    ];
    let alpha = match light.pulsing {
        true => 0.35 + 0.65 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.2).cos()),
        false => 1.0,
    };
    (shapes, alpha)
}

// The desktop app's palette (sRGB).
const BG: [f32; 3] = [0.082, 0.102, 0.129];
const SELECTED: [f32; 3] = [0.165, 0.2, 0.251];
const SUNK: [f32; 3] = [0.063, 0.082, 0.106];
const TEXT: [f32; 3] = [0.906, 0.925, 0.949];
const MUTED: [f32; 3] = [0.682, 0.722, 0.769];
const FAINT: [f32; 3] = [0.553, 0.6, 0.659];
const ACCENT: [f32; 3] = [0.369, 0.902, 0.949];
const INK: [f32; 3] = [0.024, 0.125, 0.149];
const SOFT: [f32; 3] = [0.498, 0.914, 0.957];
const OK: [f32; 3] = [0.373, 0.831, 0.604];
const WARN: [f32; 3] = [0.949, 0.69, 0.369];
const DANGER: [f32; 3] = [0.949, 0.545, 0.51];

/// The stance colours the desktop's agent cards use.
pub fn stance_color(stance: TaskStance) -> [f32; 3] {
    match stance {
        TaskStance::Thinking | TaskStance::Working => SOFT,
        TaskStance::Waiting => WARN,
        TaskStance::Done => OK,
        TaskStance::Failed => DANGER,
        TaskStance::Queued => FAINT,
    }
}

fn stance_word(stance: TaskStance) -> &'static str {
    match stance {
        TaskStance::Thinking => "THINKING",
        TaskStance::Working => "WORKING",
        TaskStance::Waiting => "WAITING FOR YOU",
        TaskStance::Done => "DONE",
        TaskStance::Failed => "FAILED",
        TaskStance::Queued => "QUEUED",
    }
}

// Metres: about 0.74 m wide at 0.9 m (the mockup's proportions). Body
// capitals are about 0.8 degrees tall, its x-height about 0.6.
pub const WIDTH: f32 = 0.76;
pub const HEIGHT: f32 = 0.54;
const PAD: f32 = 0.024;
const HEADER: f32 = 0.05;
const NAV: f32 = 0.17;
const ROW: f32 = 0.045;
const TITLE: f32 = 0.018;
const BODY: f32 = 0.0135;
const SMALL: f32 = 0.011;
const BUTTON: f32 = 0.042;
const RING: f32 = 0.0025;

fn outline(out: &mut Vec<Shape>, [x, y, w, h]: [f32; 4], c: [f32; 3]) {
    let c = [c[0], c[1], c[2], 1.0];
    for rect in [
        [x, y, w, RING],
        [x, y + h - RING, w, RING],
        [x, y, RING, h],
        [x + w - RING, y, RING, h],
    ] {
        out.push(Shape::Rect { rect, color: c });
    }
}

fn fill(out: &mut Vec<Shape>, rect: [f32; 4], c: [f32; 3]) {
    out.push(Shape::Rect {
        rect,
        color: [c[0], c[1], c[2], 1.0],
    });
}

fn text(out: &mut Vec<Shape>, at: [f32; 2], size: f32, color: [f32; 3], s: &str) {
    out.push(Shape::Text {
        at,
        size,
        color,
        text: s.to_string(),
    });
}

/// The content column, laid out top down.
struct Col<'a> {
    out: &'a mut Vec<Shape>,
    x: f32,
    w: f32,
    y: f32,
}

impl Col<'_> {
    fn line(&mut self, s: &str, size: f32, color: [f32; 3]) {
        text(self.out, [self.x, self.y], size, color, &fit(s, size, self.w));
        self.y -= advance(size);
    }

    fn para(&mut self, s: &str, size: f32, color: [f32; 3], lines: usize) {
        for l in wrap(s, size, self.w, lines) {
            self.line(&l, size, color);
        }
    }

    fn gap(&mut self, h: f32) {
        self.y -= h;
    }
}

impl Panel {
    /// The open panel's shapes, centred on the origin. `toast` is push-to-talk's
    /// (shown in Talk); `now` ages the refusal notice.
    pub fn shapes(
        &self,
        data: Option<&PanelData>,
        connection: &Connection,
        toast: Option<&VoiceToast>,
        now: Instant,
    ) -> Vec<Shape> {
        let (w, h) = (WIDTH, HEIGHT);
        let (left, top) = (-w / 2.0, h / 2.0);
        let mut out = vec![Shape::Rect {
            rect: [left, -h / 2.0, w, h],
            color: [BG[0], BG[1], BG[2], 1.0],
        }];

        // Header: the app, the project, the link's state word.
        let y = top - PAD;
        text(&mut out, [left + PAD, y], BODY, ACCENT, "FUNCTOR");
        let (word, color) = match connection {
            Connection::NotPaired => ("NOT PAIRED".to_string(), FAINT),
            Connection::NotConnected { .. } => ("CANNOT REACH".to_string(), WARN),
            Connection::Outdated { .. } => ("NEEDS AN UPDATE".to_string(), WARN),
            Connection::Connected { desktop } => (fit(desktop, SMALL, 22.0 * SMALL), OK),
        };
        let wx = w / 2.0 - PAD - overlay_font::width(&word, SMALL);
        if let Some(project) = data.and_then(|d| d.project.as_deref()) {
            let x = left + PAD + overlay_font::width("FUNCTOR", BODY) + BODY;
            text(&mut out, [x, y], BODY, MUTED, &fit(project, BODY, wx - x - BODY));
        }
        text(&mut out, [wx, y], SMALL, color, &word);
        fill(&mut out, [left, top - HEADER, w, 0.002], SELECTED);

        // The section list.
        let mut y = top - HEADER - PAD / 2.0;
        let waiting = data.map_or(0, |d| {
            d.tasks
                .iter()
                .filter(|t| t.stance == TaskStance::Waiting)
                .count()
        });
        for section in SECTIONS {
            let rect = [left + PAD / 2.0, y - ROW, NAV, ROW];
            if section == self.section {
                fill(&mut out, rect, SELECTED);
                if self.focus.is_none() && self.confirm.is_none() && self.conversation.is_none() {
                    outline(&mut out, rect, ACCENT);
                }
            }
            let label = match section {
                Section::Now => "Now",
                Section::Talk => "Talk",
                Section::Tasks => "Tasks",
                Section::GameMenu => "Game menu",
            };
            let ty = y - (ROW - BODY) / 2.0;
            let color = if section == self.section { TEXT } else { MUTED };
            text(&mut out, [left + PAD, ty], BODY, color, label);
            if section == Section::Tasks && waiting > 0 {
                let n = waiting.to_string();
                text(&mut out, [left + NAV - 2.0 * BODY, ty], BODY, WARN, &n);
            }
            y -= ROW + 0.006;
        }

        // The content column, and the footer under it.
        let x = left + PAD + NAV + PAD;
        let cw = w / 2.0 - PAD - x;
        let bottom = -h / 2.0 + PAD + SMALL * 2.0;
        let mut col = Col {
            out: &mut out,
            x,
            w: cw,
            y: top - HEADER - PAD,
        };
        let hint = match (connection, data) {
            (Connection::Connected { .. }, Some(data)) => {
                self.content(&mut col, data, toast, bottom)
            }
            (Connection::Connected { .. }, None) => {
                col.line("WAITING FOR THE DESKTOP", SMALL, FAINT);
                col.para(
                    "The desktop app has not sent its tasks yet: it may need an update.",
                    BODY,
                    MUTED,
                    3,
                );
                None
            }
            (Connection::NotPaired, _) => {
                col.line("NOT PAIRED", SMALL, FAINT);
                col.para("Hold the menu button for a second to pair with the Functor app on your computer.", BODY, TEXT, 3);
                None
            }
            (Connection::NotConnected { desktop }, _) => {
                col.line("CANNOT REACH THE DESKTOP", SMALL, WARN);
                col.para(&format!("Open the Functor app on {desktop}, on the same Wi-Fi as this headset. It reconnects by itself."), BODY, TEXT, 4);
                None
            }
            (Connection::Outdated { desktop }, _) => {
                col.line("NEEDS AN UPDATE", SMALL, WARN);
                col.para(&format!("This headset app and the Functor app on {desktop} speak different versions. Update the older one."), BODY, TEXT, 4);
                None
            }
        };
        if let Some((reason, until)) = &self.notice {
            if now < *until {
                let mut note = Col {
                    out: &mut out,
                    x,
                    w: cw,
                    y: bottom + SMALL * 1.6,
                };
                note.line(&format!("Not done: {reason}"), SMALL, DANGER);
            }
        }
        let footer = hint.unwrap_or(match (self.focus, self.page) {
            (None, _) => "Stick move   A open   B close",
            (Some(_), _) => "Stick move   A choose   B back",
        });
        text(&mut out, [x, -h / 2.0 + PAD + SMALL], SMALL, FAINT, footer);
        out
    }

    /// The column for a linked desktop's `data`; a footer hint to show
    /// instead of the default.
    fn content(
        &self,
        col: &mut Col,
        data: &PanelData,
        toast: Option<&VoiceToast>,
        bottom: f32,
    ) -> Option<&'static str> {
        if let Some(id) = self.conversation {
            return self.conversation_of(col, data, id, toast, bottom);
        }
        let find = |id: u64| data.tasks.iter().find(|t| t.id == id);
        if let Some(t) = self.page.and_then(find) {
            self.conversation_row(col, data, t.id);
            return self.page_of(col, t, bottom);
        }
        match self.section {
            Section::Now => {
                col.line(
                    match data.placed {
                        true => "NOW  -  RUNNING IN THIS HEADSET",
                        false => "NOW  -  ITS GAME RUNS ON THE DESKTOP",
                    },
                    SMALL,
                    FAINT,
                );
                col.gap(0.004);
                let Some(t) = data.now.and_then(find) else {
                    col.line("Nothing yet.", TITLE, TEXT);
                    col.para(
                        "Hold the right thumbstick and say what to change.",
                        BODY,
                        MUTED,
                        2,
                    );
                    return None;
                };
                self.conversation_row(col, data, t.id);
                col.para(&t.title, TITLE, TEXT, 2);
                col.line(stance_word(t.stance), SMALL, stance_color(t.stance));
                if !t.activity.is_empty() {
                    col.para(&t.activity, BODY, MUTED, 2);
                }
                // Not on the running task it would name ("Starts after" it).
                if !data.waiting.is_empty() && !running(t.stance) {
                    col.para(&data.waiting, SMALL, WARN, 1);
                }
                if let Some(d) = &t.decision {
                    col.gap(0.008);
                    col.line(&d.size, BODY, TEXT);
                    verdict(col, d);
                }
                if let Some(Item::Stop(_)) = &self.confirm {
                    return confirm(col, "Stop this task?", bottom);
                }
                let label = match (&t.decision, running(t.stance)) {
                    (Some(_), _) => "Open the decision",
                    (None, true) => "Stop",
                    (None, false) => return None,
                };
                self.buttons(col, &[label], 1, bottom)
            }
            Section::Talk => {
                col.line("TALK", SMALL, FAINT);
                col.gap(0.004);
                col.para(
                    "Hold the right thumbstick to talk; let go to send. B cancels.",
                    BODY,
                    TEXT,
                    2,
                );
                if let Some(toast) = toast {
                    col.gap(0.008);
                    for l in toast_lines(toast) {
                        col.line(&l.text, l.size * 0.8, l.color);
                    }
                }
                // Where the request in flight is (D43).
                if let Some(p) = &data.progress {
                    col.gap(0.008);
                    col.line("WHERE IT IS", SMALL, FAINT);
                    col.para(&p.text(), BODY, if p.failed { DANGER } else { SOFT }, 2);
                }
                let Some(route) = &data.route else {
                    return None;
                };
                col.gap(0.008);
                col.line("WHERE IT WENT", SMALL, FAINT);
                col.para(&route.line, BODY, TEXT, 2);
                if let Some(Item::Move(_, to)) = &self.confirm {
                    let label = route
                        .moves
                        .iter()
                        .find(|(_, t)| t == to)
                        .map_or("", |(l, _)| l);
                    return confirm(col, &format!("{label}?"), bottom);
                }
                for (i, (label, _)) in route.moves.iter().enumerate() {
                    self.row(col, self.focus == Some(i), label, "", TEXT, "");
                }
                None
            }
            Section::Tasks => {
                col.line("TASKS", SMALL, FAINT);
                if !data.waiting.is_empty() {
                    col.line(&data.waiting, SMALL, WARN);
                }
                col.gap(0.004);
                if data.tasks.is_empty() {
                    col.para(
                        "No tasks yet. Hold the right thumbstick to ask for one.",
                        BODY,
                        MUTED,
                        2,
                    );
                }
                for (i, t) in data.tasks.iter().enumerate() {
                    let word = match t.stance {
                        TaskStance::Waiting => "NEEDS YOU",
                        s => stance_word(s),
                    };
                    let focus = self.focus == Some(i) && self.page.is_none();
                    self.row(
                        col,
                        focus,
                        &t.title,
                        word,
                        stance_color(t.stance),
                        &t.activity,
                    );
                }
                None
            }
            Section::GameMenu => {
                col.line("GAME MENU", SMALL, FAINT);
                col.gap(0.004);
                col.para(
                    "A closes this panel and opens the game's own menu.",
                    BODY,
                    TEXT,
                    2,
                );
                Some("A open the game's menu   B close")
            }
        }
    }

    /// A task's page: its decision (D35) with Keep and Revert, or its run
    /// with Stop.
    fn page_of(&self, col: &mut Col, t: &PanelTask, bottom: f32) -> Option<&'static str> {
        col.para(&t.title, TITLE, TEXT, 2);
        let stance = match t.decision {
            Some(_) => "READY FOR YOUR DECISION",
            None => stance_word(t.stance),
        };
        col.line(stance, SMALL, stance_color(t.stance));
        col.gap(0.006);
        let Some(d) = &t.decision else {
            if !t.activity.is_empty() {
                col.para(&t.activity, BODY, MUTED, 3);
            }
            if !running(t.stance) {
                return None;
            }
            if let Some(Item::Stop(_)) = &self.confirm {
                return confirm(col, "Stop this task?", bottom);
            }
            return self.buttons(col, &["Stop"], 1, bottom);
        };
        col.line("SIZE OF THE CHANGE", SMALL, FAINT);
        col.line(&d.size, BODY, TEXT);
        col.gap(0.004);
        verdict(col, d);
        if !d.summary.is_empty() {
            col.gap(0.004);
            col.line("SUMMARY", SMALL, FAINT);
            col.para(&d.summary, BODY, TEXT, 3);
        }
        if !d.notes.is_empty() {
            col.gap(0.004);
            col.line("THINGS TO NOTE", SMALL, FAINT);
            for note in &d.notes {
                col.line(&format!("- {note}"), SMALL, MUTED);
            }
        }
        match &self.confirm {
            Some(Item::Keep(..)) => confirm(col, "Keep this change?", bottom),
            Some(Item::Revert(..)) => {
                confirm(col, "Revert it? The files go back as they were.", bottom)
            }
            _ => self
                .buttons(col, &["Keep", "Revert"], 1, bottom)
                .or(Some("A choose   B back   Click & hold R stick: talk")),
        }
    }

    /// A list row: `title` with `word` at its right, `sub` under it.
    fn row(&self, col: &mut Col, focus: bool, title: &str, word: &str, color: [f32; 3], sub: &str) {
        let rect = [col.x, col.y - ROW, col.w, ROW];
        fill(col.out, rect, if focus { SELECTED } else { SUNK });
        if focus {
            outline(col.out, rect, ACCENT);
        }
        let inset = 0.01;
        let wx = col.x + col.w - inset - overlay_font::width(word, SMALL);
        let ty = col.y - 0.008;
        text(
            col.out,
            [col.x + inset, ty],
            BODY,
            TEXT,
            &fit(title, BODY, wx - col.x - 2.0 * inset),
        );
        text(col.out, [wx, ty], SMALL, color, word);
        let sy = ty - BODY - 0.006;
        text(
            col.out,
            [col.x + inset, sy],
            SMALL,
            FAINT,
            &fit(sub, SMALL, col.w - 2.0 * inset),
        );
        col.gap(ROW + 0.006);
    }

    /// The row that opens task `id`'s conversation (content item 0), with
    /// its newest line, or that the agent asks.
    fn conversation_row(&self, col: &mut Col, data: &PanelData, id: u64) {
        let conv = data.conversation.as_ref().filter(|c| c.task == id);
        let (word, newest) = match conv {
            Some(c) if c.question => ("ASKS YOU", c.items.last()),
            Some(c) => ("", c.items.last()),
            None => ("", None),
        };
        let sub = newest.map_or(String::new(), |l| l.text.clone());
        let focus = self.focus == Some(0);
        self.row(col, focus, "Conversation", word, WARN, &sub);
    }

    /// Task `id`'s conversation: what the agent asks first, then the
    /// messages (newest at the bottom, scrolled), then inline push-to-talk
    /// and what the agent is doing.
    fn conversation_of(
        &self,
        col: &mut Col,
        data: &PanelData,
        id: u64,
        toast: Option<&VoiceToast>,
        bottom: f32,
    ) -> Option<&'static str> {
        let hint = Some("Stick scroll  A more  B back  Hold stick: talk");
        let Some(c) = data.conversation.as_ref().filter(|c| c.task == id) else {
            col.line("CONVERSATION", SMALL, FAINT);
            col.para("Waiting for the desktop to send it...", BODY, MUTED, 2);
            return hint;
        };
        col.line(&c.title, TITLE, TEXT);
        col.line(stance_word(c.stance), SMALL, stance_color(c.stance));
        let asked = c.items.iter().rev().find(|i| i.role == Role::Agent);
        if let Some(q) = asked.filter(|_| c.question) {
            col.gap(0.004);
            col.line("THE AGENT ASKS", SMALL, WARN);
            col.para(&q.text, BODY, WARN, 3);
            col.line("Hold the right stick to answer", SMALL, TEXT);
        }
        col.gap(0.006);
        // Under the messages: push-to-talk's stages, then the work.
        let mut tail: Vec<(String, f32, [f32; 3])> = toast
            .map(|t| {
                (toast_lines(t).into_iter())
                    .map(|l| (l.text, l.size * 0.8, l.color))
                    .collect()
            })
            .unwrap_or_default();
        // The request in flight's step (D43), else what the agent does.
        match &data.progress {
            Some(p) => tail.push((p.text(), SMALL, if p.failed { DANGER } else { SOFT })),
            None if running(c.stance) => {
                let doing = match c.activity.is_empty() {
                    true => "Working...".to_string(),
                    false => format!("Working... {}", c.activity),
                };
                tail.push((doing, SMALL, SOFT));
            }
            None => {}
        }
        let floor = bottom + SMALL * 1.6 + tail.iter().map(|l| advance(l.1)).sum::<f32>();
        let lines = conversation_lines(c, self.expanded);
        let room = col.y - floor;
        // How many lines fit from `from`, walking `lines` in `order`.
        let fit = |order: &mut dyn Iterator<Item = &Laid>| {
            let mut used = 0.0;
            order
                .take_while(|l| {
                    used += advance(l.1);
                    used <= room
                })
                .count()
        };
        let newest = fit(&mut lines.iter().rev());
        let start = self.top(&lines, newest);
        let shown = fit(&mut lines[start..].iter());
        self.shown.set(newest);
        for (text, size, color, _) in &lines[start..start + shown] {
            col.line(text, *size, *color);
        }
        col.y = floor;
        for (text, size, color) in &tail {
            col.line(text, *size, *color);
        }
        hint
    }

    /// A row of buttons above the footer, the one focused (content item
    /// `first + i`) ringed.
    fn buttons(
        &self,
        col: &mut Col,
        labels: &[&str],
        first: usize,
        bottom: f32,
    ) -> Option<&'static str> {
        let gap = 0.012;
        let bw = (col.w - gap * (labels.len() - 1) as f32) / labels.len() as f32;
        let y = bottom + SMALL * 3.0;
        for (i, label) in labels.iter().enumerate() {
            let rect = [col.x + i as f32 * (bw + gap), y, bw, BUTTON];
            let primary = i == 0;
            fill(col.out, rect, if primary { ACCENT } else { SELECTED });
            if self.focus == Some(first + i) {
                // White, so it reads against the accent fill.
                outline(
                    col.out,
                    [
                        rect[0] - 0.004,
                        rect[1] - 0.004,
                        rect[2] + 0.008,
                        rect[3] + 0.008,
                    ],
                    TEXT,
                );
            }
            let tx = rect[0] + (bw - overlay_font::width(label, BODY)) / 2.0;
            let ty = y + (BUTTON + BODY) / 2.0;
            text(
                col.out,
                [tx, ty],
                BODY,
                if primary { INK } else { TEXT },
                label,
            );
        }
        None
    }
}

/// The width of the content column.
const CONTENT_W: f32 = WIDTH - 3.0 * PAD - NAV;

/// How far one line of `size` moves the column down.
fn advance(size: f32) -> f32 {
    size * 1.5 + 0.002
}

/// A laid-out conversation line: text, size, color, and the index (among
/// all the task's items) of the item it belongs to.
type Laid = (String, f32, [f32; 3], u32);
/// A blank line's size between messages.
const SPACE: f32 = 0.005;

/// `c` laid out for the conversation page, oldest first: each message under
/// who said it (spoken lines in the accent), app lines small; an agent
/// message past [`AGENT_LINES`] is cut unless `expanded`. A question the
/// agent waits on shows above them instead.
fn conversation_lines(c: &ConversationData, expanded: bool) -> Vec<Laid> {
    let mut out: Vec<Laid> = Vec::new();
    if c.first > 0 {
        let earlier = "Earlier messages are on the desktop.".into();
        out.push((earlier, SMALL, FAINT, c.first));
    }
    let asked = c
        .question
        .then(|| c.items.iter().rposition(|i| i.role == Role::Agent))
        .flatten();
    for (i, item) in c.items.iter().enumerate() {
        if Some(i) == asked {
            continue;
        }
        let at = c.first + i as u32;
        let text = &item.text;
        let mut push = |text: String, size: f32, color: [f32; 3]| out.push((text, size, color, at));
        // A little air above each message's header.
        if item.role != Role::App && (i > 0 || c.first > 0) {
            push(String::new(), SPACE, FAINT);
        }
        match item.role {
            Role::You => {
                let (head, color) = match item.spoken {
                    true => ("YOU, SPOKEN", SOFT),
                    false => ("YOU", MUTED),
                };
                push(head.into(), SMALL, color);
                let body = if item.spoken { SOFT } else { TEXT };
                for l in wrap(text, BODY, CONTENT_W, 40) {
                    push(l, BODY, body);
                }
            }
            Role::Agent => {
                push("AGENT".into(), SMALL, FAINT);
                let lines = wrap(text, BODY, CONTENT_W, 40);
                let shown = if expanded { lines.len() } else { AGENT_LINES };
                let more = lines.len() > shown;
                for l in lines.into_iter().take(shown) {
                    push(l, BODY, TEXT);
                }
                if more {
                    push("... more (A)".into(), SMALL, FAINT);
                } else if item.cut {
                    push("... the rest is on the desktop".into(), SMALL, FAINT);
                }
            }
            Role::App => {
                for l in wrap(&format!("- {text}"), SMALL, CONTENT_W, 2) {
                    push(l, SMALL, FAINT);
                }
            }
        }
    }
    out
}

/// "Approve at a glance" or "look into this", and why.
fn verdict(col: &mut Col, d: &PanelDecision) {
    match d.glance {
        true => col.line("APPROVE AT A GLANCE", SMALL, OK),
        false => col.line("LOOK INTO THIS", SMALL, WARN),
    }
    col.para(&d.reason, BODY, MUTED, 2);
}

/// The confirm prompt where the buttons were.
fn confirm(col: &mut Col, question: &str, bottom: f32) -> Option<&'static str> {
    col.y = bottom + SMALL * 3.0 + BUTTON;
    col.line(question, BODY, WARN);
    col.line("A  Yes        B  No", BODY, TEXT);
    Some("A yes   B no")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u64, stance: TaskStance, decision: bool) -> PanelTask {
        PanelTask {
            id,
            title: format!("Task {id}"),
            stance,
            activity: "reading game.fun".into(),
            decision: decision.then(|| PanelDecision {
                tree: format!("tree-{id}"),
                size: "+1 -1, 1 file, game logic".into(),
                glance: true,
                reason: "1 changed line in one file.".into(),
                summary: "The agent changed one value.".into(),
                notes: vec!["Not tried on the left hand.".into()],
            }),
        }
    }

    fn data() -> PanelData {
        PanelData {
            project: Some("vr-lab".into()),
            placed: true,
            now: Some(1),
            tasks: vec![
                task(1, TaskStance::Waiting, true),
                task(2, TaskStance::Working, false),
                task(3, TaskStance::Queued, false),
            ],
            waiting: "Waiting for your decision on \"Task 1\"".into(),
            route: Some(PanelRoute {
                id: 9,
                line: "Queued behind \"Task 2\"".into(),
                moves: vec![
                    (
                        "Steer \"Task 2\" instead".into(),
                        Destination::Steer { task: 2 },
                    ),
                    ("Make it a new task".into(), Destination::New),
                ],
            }),
            conversation: Some(conversation(1, 4)),
            progress: Some(ProgressLine {
                step: ProgressStep::Agent,
                detail: "working: editing game.fun".into(),
                failed: false,
            }),
        }
    }

    /// Task `task`'s conversation: the spoken request, the route, and `n`
    /// agent messages, `msg 0`..
    fn conversation(task: u64, n: usize) -> ConversationData {
        let said = |role, spoken, text: &str| SaidLine {
            role,
            spoken,
            cut: false,
            text: text.into(),
        };
        let mut items = vec![
            said(Role::You, true, "make the ball red"),
            said(Role::App, false, "Steered \"Task 1\""),
        ];
        items.extend((0..n).map(|i| said(Role::Agent, false, &format!("msg {i}"))));
        ConversationData {
            task,
            title: format!("Task {task}"),
            stance: TaskStance::Working,
            activity: "editing game.fun".into(),
            question: false,
            first: 0,
            items,
        }
    }

    /// Presses as samples: a control held for one sample, then let go.
    #[derive(Clone, Copy)]
    enum P {
        Up,
        Down,
        Right,
        Left,
        A,
        B,
    }

    fn press(panel: &mut Panel, data: &PanelData, p: P) -> Option<Out> {
        let mut xr = XrInputSnapshot::default();
        match p {
            P::Up => xr.right.thumbstick = [0.0, 1.0],
            P::Down => xr.right.thumbstick = [0.0, -1.0],
            P::Right => xr.right.thumbstick = [1.0, 0.0],
            P::Left => xr.right.thumbstick = [-1.0, 0.0],
            P::A => xr.right.primary_pressed = true,
            P::B => xr.right.secondary_pressed = true,
        }
        let out = panel.step(Some(&xr), Some(data), false);
        let rest = panel.step(Some(&XrInputSnapshot::default()), Some(data), false);
        assert_eq!(rest, None, "a release does nothing");
        out
    }

    #[test]
    fn tasks_open_a_decision_page_and_keep_takes_a_confirm() {
        let d = data();
        let mut p = Panel::default();
        assert_eq!(press(&mut p, &d, P::A), None, "closed: nothing");
        p.toggle();
        assert_eq!(p.debug(Some(&d), None).section, "now");
        press(&mut p, &d, P::Down);
        press(&mut p, &d, P::Down);
        assert_eq!(p.section, Section::Tasks);
        assert_eq!(press(&mut p, &d, P::Right), None);
        assert_eq!(p.focus, Some(0));
        // Past the end: it stays on the last.
        for _ in 0..3 {
            press(&mut p, &d, P::Down);
        }
        assert_eq!(p.focus, Some(2));
        press(&mut p, &d, P::Up);
        press(&mut p, &d, P::Up);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Select { task: 1 }))
        );
        assert_eq!((p.page, p.focus), (Some(1), Some(0)), "on its conversation");
        press(&mut p, &d, P::Down);
        // Keep waits for its confirm; B says no.
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(p.debug(Some(&d), None).confirm.as_deref(), Some("keep"));
        assert_eq!(press(&mut p, &d, P::B), None);
        assert_eq!((p.confirm.clone(), p.page), (None, Some(1)));
        // Revert names the change shown, after its confirm.
        press(&mut p, &d, P::Down);
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Revert {
                task: 1,
                tree: "tree-1".into()
            }))
        );
        press(&mut p, &d, P::Up);
        press(&mut p, &d, P::A);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Keep { task: 1 }))
        );
        // B: back to the list, on the task; again: the sections; again: closed.
        press(&mut p, &d, P::B);
        assert_eq!((p.page, p.focus), (None, Some(0)));
        press(&mut p, &d, P::B);
        assert_eq!(p.focus, None);
        assert_eq!(press(&mut p, &d, P::B), Some(Out::Close));
    }

    #[test]
    fn now_offers_its_decision_or_stop_and_game_menu_hands_over() {
        let mut d = data();
        let mut p = Panel::default();
        p.toggle();
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::Down);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Select { task: 1 }))
        );
        assert_eq!(p.page, Some(1));
        // The desktop kept it meanwhile: the page has no decision, nor the
        // confirm that was waiting, and that press does nothing else.
        press(&mut p, &d, P::Down);
        press(&mut p, &d, P::A);
        assert!(p.confirm.is_some());
        d.tasks[0].decision = None;
        d.tasks[0].stance = TaskStance::Done;
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(
            (p.confirm.clone(), p.focus, p.conversation),
            (None, Some(0), None)
        );
        // B from that page, focus or not: back to where it opened.
        assert_eq!(press(&mut p, &d, P::B), None);
        assert_eq!((p.page, p.is_open()), (None, true));
        // Running: Stop, confirmed.
        let mut p = Panel::default();
        p.toggle();
        d.now = Some(2);
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::Down);
        press(&mut p, &d, P::A);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Stop { task: 2 }))
        );
        // The game's menu, from the last section.
        press(&mut p, &d, P::Left);
        for _ in 0..5 {
            press(&mut p, &d, P::Down);
        }
        assert_eq!(press(&mut p, &d, P::A), Some(Out::GameMenu));
    }

    #[test]
    fn a_changed_change_cancels_its_keep_and_closing_keeps_the_hand_until_let_go() {
        let mut d = data();
        let mut p = Panel::default();
        p.toggle();
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::Down);
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::Down);
        press(&mut p, &d, P::A);
        assert_eq!(p.debug(Some(&d), None).confirm.as_deref(), Some("keep"));
        d.tasks[0].decision.as_mut().unwrap().tree = "tree-9".into();
        // That press keeps nothing; the new change needs its own Keep.
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(p.confirm, None);

        // Closed by B: B stays the panel's while held, then the game's.
        let mut b = XrInputSnapshot::default();
        b.right.secondary_pressed = true;
        p.focus = None;
        p.page = None;
        assert_eq!(p.step(Some(&b), Some(&d), false), Some(Out::Close));
        p.close();
        assert!(p.hides_controls());
        p.step(Some(&b), Some(&d), false);
        assert!(p.hides_controls(), "still held");
        p.step(Some(&XrInputSnapshot::default()), Some(&d), false);
        assert!(!p.hides_controls());
    }

    #[test]
    fn the_conversation_opens_from_now_follows_new_text_and_scrolls() {
        let mut d = data();
        d.conversation = Some(conversation(1, 30));
        let mut p = Panel::default();
        p.toggle();
        press(&mut p, &d, P::A);
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Select { task: 1 })),
            "the desktop sends the selected task's"
        );
        let state = |p: &Panel, d: &PanelData| p.debug(Some(d), None);
        assert_eq!(state(&p, &d).conversation, Some(1));
        assert!(state(&p, &d).following);
        // Talking keeps the page: its stages show inline.
        p.talk();
        assert_eq!(p.conversation, Some(1));
        // What the page shows first, as drawn.
        let first_line = |p: &Panel, d: &PanelData| {
            let lines = conversation_lines(d.conversation.as_ref().unwrap(), p.expanded);
            let connected = Connection::Connected {
                desktop: "Mac".into(),
            };
            p.shapes(Some(d), &connected, None, Instant::now());
            let shown = p.shown.get();
            assert!(
                shown > 0 && shown < lines.len(),
                "{shown} of {}",
                lines.len()
            );
            lines[p.top(&lines, shown)].0.clone()
        };
        // Up: older text, which new text no longer moves, also as the
        // window drops its oldest item.
        let _ = first_line(&p, &d);
        press(&mut p, &d, P::Up);
        assert!(!state(&p, &d).following);
        let held = first_line(&p, &d);
        let c = d.conversation.as_mut().unwrap();
        c.items.remove(0);
        c.first += 1;
        c.items.push(SaidLine {
            role: Role::Agent,
            spoken: false,
            cut: false,
            text: "new".into(),
        });
        assert_eq!(first_line(&p, &d), held);
        press(&mut p, &d, P::A);
        assert!(p.expanded, "A shows long messages whole");
        // Down to the end: it follows again.
        for _ in 0..20 {
            press(&mut p, &d, P::Down);
        }
        assert!(state(&p, &d).following);
        // B: back to Now, on the conversation's row.
        assert_eq!(press(&mut p, &d, P::B), None);
        assert_eq!(
            (p.conversation, p.focus, p.is_open()),
            (None, Some(0), true)
        );
    }

    #[test]
    fn a_question_is_shown_first_and_long_messages_are_cut_with_more() {
        let mut d = data();
        let c = d.conversation.as_mut().unwrap();
        c.question = true;
        c.items.push(SaidLine {
            role: Role::Agent,
            spoken: false,
            cut: true,
            text: "A long answer. ".repeat(30),
        });
        c.items.push(SaidLine {
            role: Role::Agent,
            spoken: false,
            cut: false,
            text: "Left or right hip?".into(),
        });
        let mut p = Panel::default();
        p.toggle();
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::A);
        assert!(p.debug(Some(&d), None).question);
        let shapes = p.shapes(
            Some(&d),
            &Connection::Connected {
                desktop: "Mac".into(),
            },
            None,
            Instant::now(),
        );
        let texts: Vec<&str> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let at = |t: &str| {
            texts
                .iter()
                .position(|x| *x == t)
                .unwrap_or_else(|| panic!("{t}: {texts:?}"))
        };
        assert!(at("THE AGENT ASKS") < at("AGENT"), "asked first");
        assert!(texts.contains(&"... more (A)"));
        // The request's step (D43) at the bottom; without one, the work.
        assert!(texts.contains(&"AGENT  working: editing game.fun"));
        d.progress = None;
        let shapes = p.shapes(
            Some(&d),
            &Connection::Connected {
                desktop: "Mac".into(),
            },
            None,
            Instant::now(),
        );
        assert!(shapes.iter().any(
            |s| matches!(s, Shape::Text { text, .. } if text == "Working... editing game.fun")
        ));
        let lines = conversation_lines(d.conversation.as_ref().unwrap(), true);
        assert!(lines
            .iter()
            .any(|l| l.0 == "... the rest is on the desktop"));
    }

    #[test]
    fn talk_offers_the_routes_moves_each_with_a_confirm() {
        let d = data();
        let mut p = Panel::default();
        p.toggle();
        press(&mut p, &d, P::Down);
        assert_eq!(p.section, Section::Talk);
        press(&mut p, &d, P::Right);
        press(&mut p, &d, P::Down);
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(p.debug(Some(&d), None).confirm.as_deref(), Some("move"));
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Move {
                id: 9,
                to: Destination::New
            }))
        );
        // The route moved: its moves go, and so does the focus.
        let mut gone = d.clone();
        gone.route = None;
        press(&mut p, &gone, P::Up);
        assert_eq!(p.focus, None);
    }

    #[test]
    fn talking_holds_the_panel_and_a_held_stick_moves_once() {
        let d = data();
        let mut p = Panel::default();
        p.toggle();
        let mut xr = XrInputSnapshot::default();
        xr.right.secondary_pressed = true;
        assert_eq!(p.step(Some(&xr), Some(&d), true), None, "B is talk's");
        assert!(p.is_open());
        let mut down = XrInputSnapshot::default();
        down.right.thumbstick = [0.0, -0.9];
        p.step(Some(&down), Some(&d), false);
        down.right.thumbstick = [0.0, -0.5];
        p.step(Some(&down), Some(&d), false);
        down.right.thumbstick = [0.0, -0.9];
        p.step(Some(&down), Some(&d), false);
        assert_eq!(p.section, Section::Talk, "one move until re-armed");
        p.talk();
        assert_eq!(p.section, Section::Talk);
        p.toggle();
        assert!(!p.is_open());
    }

    #[test]
    fn it_opens_ahead_of_the_head_level_and_facing_it_and_the_light_rides_the_wrist() {
        let half = 90f32.to_radians() / 2.0;
        // Facing -X (yaw 90 degrees left), tilted a little.
        let head = TrackingPose {
            position: [1.0, 1.6, 0.0],
            orientation: [0.05, half.sin(), 0.0, half.cos()],
        };
        let m = placement(head);
        let centre = m * cgmath::vec4(0.0, 0.0, 0.0, 1.0);
        assert!((centre.x - (1.0 - DISTANCE)).abs() < 0.03, "{centre:?}");
        assert!((centre.y - (1.6 - DROP)).abs() < 1e-4 && centre.z.abs() < 0.03);
        // Its face (+Z) points back at the head; its up stays up.
        let normal = m * cgmath::vec4(0.0, 0.0, 1.0, 0.0);
        assert!(normal.x > 0.99 && normal.y.abs() < 1e-4, "{normal:?}");

        let still = TrackingPose {
            position: [0.0, 1.6, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let grip = TrackingPose {
            position: [-0.2, 1.2, -0.3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let at = wrist_position(still, grip).unwrap();
        let want = [-0.2 + WRIST_AT[0], -0.4, -0.3 + WRIST_AT[2]];
        assert!(
            at.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5),
            "{at:?}"
        );
    }

    #[test]
    fn the_wrist_light_is_the_nows_stance_and_pulses_only_waiting() {
        let d = data();
        assert_eq!(wrist(Some(&d), false), None, "off unlinked");
        assert_eq!(wrist(None, true), None);
        let light = wrist(Some(&d), true).unwrap();
        assert_eq!((light.stance, light.pulsing), (TaskStance::Waiting, true));
        let (_, a0) = wrist_shapes(light, 0.0);
        let (_, a1) = wrist_shapes(light, 0.6);
        assert!(a0 > 0.9 && a1 < 0.5, "{a0} {a1}");
        let working = WristLight {
            stance: TaskStance::Working,
            pulsing: false,
        };
        assert_eq!(wrist_shapes(working, 0.6).1, 1.0);
    }

    #[test]
    fn every_view_fits_inside_the_panel_in_the_fonts_glyphs() {
        let d = data();
        let mut long = d.clone();
        long.tasks[0].title = "é".repeat(300);
        long.tasks[0].activity = "\u{201c}x\u{201d} ".repeat(100);
        let c = long.conversation.as_mut().unwrap();
        c.question = true;
        c.title = "\u{e9}".repeat(300);
        c.activity = "y".repeat(300);
        for item in &mut c.items {
            item.text = "word ".repeat(120);
        }
        let toast = VoiceToast {
            stage: crate::debug_protocol::VoiceStage::Heard,
            transcript: Some("word ".repeat(80)),
        };
        let now = Instant::now();
        let conns = [
            Connection::NotPaired,
            Connection::NotConnected {
                desktop: "Mac".into(),
            },
            Connection::Outdated {
                desktop: "Mac".into(),
            },
            Connection::Connected {
                desktop: "Mac".into(),
            },
        ];
        let mut views = Vec::new();
        for section in SECTIONS {
            for page in [None, Some(1), Some(2)] {
                for confirm in [
                    None,
                    Some(Item::Keep(1, "tree-1".into())),
                    Some(Item::Stop(2)),
                ] {
                    let mut p = Panel {
                        open: true,
                        section,
                        page,
                        focus: Some(0),
                        confirm,
                        notice: Some(("x".repeat(400), now + NOTICE)),
                        conversation: (section == Section::Talk).then_some(1),
                        scroll: (page == Some(2)).then_some((2, 1)),
                        ..Panel::default()
                    };
                    for data in [&d, &long] {
                        for c in &conns {
                            views.push(p.shapes(Some(data), c, Some(&toast), now));
                        }
                    }
                    p.focus = None;
                    views.push(p.shapes(None, &conns[3], None, now));
                }
            }
        }
        let (hw, hh) = (WIDTH / 2.0 + 1e-4, HEIGHT / 2.0 + 1e-4);
        for shapes in views {
            for s in shapes {
                match s {
                    Shape::Rect {
                        rect: [x, y, w, h], ..
                    } => {
                        assert!(
                            x >= -hw && x + w <= hw && y >= -hh && y + h <= hh,
                            "{x} {y} {w} {h}"
                        )
                    }
                    Shape::Text { at, size, text, .. } => {
                        assert!(text.chars().all(overlay_font::has_glyph), "{text}");
                        let right = at[0] + overlay_font::width(&text, size);
                        assert!(at[0] >= -hw && right <= hw, "{text}: {right}");
                        assert!(at[1] <= hh && at[1] - size >= -hh, "{text}: {}", at[1]);
                    }
                }
            }
        }
    }

    /// The overlay's layout as text (each rect, each line with its measured
    /// width) against `link_panel.layout.golden`; `UPDATE_GOLDENS=1`
    /// rewrites it. Metrics, not pixels: it catches a font or layout change.
    #[test]
    fn the_layout_matches_its_golden() {
        use crate::link_overlay::{card, lines};
        use std::fmt::Write;
        let d = data();
        let mac = Connection::Connected {
            desktop: "MacBook-Air".into(),
        };
        let code = crate::debug_protocol::PairingState::Code {
            desktop: "MacBook-Air".into(),
            code: "482913".into(),
            confirmed: false,
        };
        let heard = VoiceToast {
            stage: crate::debug_protocol::VoiceStage::Heard,
            transcript: Some("make the ball bounce a little higher off the left wall, please".into()),
        };
        let page = Panel {
            open: true,
            section: Section::Tasks,
            page: Some(1),
            focus: Some(0),
            ..Panel::default()
        };
        let views = [
            ("pairing", card(&lines(&code))),
            ("toast", card(&toast_lines(&heard))),
            ("decision", page.shapes(Some(&d), &mac, None, Instant::now())),
        ];
        let mut out = String::new();
        for (name, shapes) in views {
            writeln!(out, "== {name}").unwrap();
            for shape in shapes {
                match shape {
                    Shape::Rect { rect: [x, y, w, h], .. } => {
                        writeln!(out, "rect {x:.4} {y:.4} {w:.4} {h:.4}").unwrap()
                    }
                    Shape::Text { at, size, text, .. } => {
                        let w = overlay_font::width(&text, size);
                        writeln!(out, "text {:.4} {:.4} {size:.4} w={w:.4} {text:?}", at[0], at[1])
                            .unwrap()
                    }
                }
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/link_panel.layout.golden");
        if std::env::var_os("UPDATE_GOLDENS").is_some() {
            std::fs::write(path, &out).unwrap();
        }
        let want = std::fs::read_to_string(path).unwrap_or_default();
        assert!(out == want, "layout changed (UPDATE_GOLDENS=1 to accept):\n{out}");
    }

    /// The panel's views as PNGs in `FUNCTOR_PANEL_PNGS`, painted on the CPU
    /// as the overlay paints them (a layout check without a headset):
    /// `cargo test -p functor_runtime_common --lib link_panel -- --ignored`.
    #[test]
    #[ignore]
    fn paints_each_view_to_png() {
        let Some(dir) = std::env::var_os("FUNCTOR_PANEL_PNGS") else {
            return;
        };
        let atlas = image::load_from_memory(overlay_font::ATLAS_PNG).unwrap().to_rgb8();
        let scale = 1600.0;
        let (w, h) = ((WIDTH * scale) as usize, (HEIGHT * scale) as usize);
        // The field's median at atlas pixel `(u, v)`, bilinear like the GPU.
        let field = |u: f32, v: f32| {
            let (x, y) = (u - 0.5, v - 0.5);
            let (x0, y0) = (x.floor(), y.floor());
            let tap = |dx: f32, dy: f32| {
                let px = (x0 + dx).clamp(0.0, atlas.width() as f32 - 1.0) as u32;
                let py = (y0 + dy).clamp(0.0, atlas.height() as f32 - 1.0) as u32;
                atlas.get_pixel(px, py).0.map(|c| c as f32 / 255.0)
            };
            let (fx, fy) = (x - x0, y - y0);
            let mut c = [0.0; 3];
            for (dx, dy, k) in [
                (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
                (1.0, 0.0, fx * (1.0 - fy)),
                (0.0, 1.0, (1.0 - fx) * fy),
                (1.0, 1.0, fx * fy),
            ] {
                let t = tap(dx, dy);
                (0..3).for_each(|i| c[i] += t[i] * k);
            }
            c[0].min(c[1]).max(c[0].max(c[1]).min(c[2]))
        };
        // The overlay's own mesh, one quad (six vertices) at a time.
        let paint = |shapes: &[Shape]| {
            let mut px = vec![[30u8, 24, 40]; w * h];
            let mesh = crate::link_overlay::mesh(shapes);
            for quad in mesh.chunks(6 * 8) {
                let (lo, hi) = (&quad[..8], &quad[2 * 8..3 * 8]);
                let srgb = |c: f32| crate::color_space::linear_to_srgb(c) * 255.0;
                let color = [srgb(lo[4]), srgb(lo[5]), srgb(lo[6])];
                let [x0, y0] = [lo[0] * scale + w as f32 / 2.0, h as f32 / 2.0 - hi[1] * scale];
                let [x1, y1] = [hi[0] * scale + w as f32 / 2.0, h as f32 / 2.0 - lo[1] * scale];
                // Atlas texels per pixel, and the uv at the quad's top-left.
                let texels = (hi[2] - lo[2]) * atlas.width() as f32 / (x1 - x0);
                let (u0, v0) = (lo[2] * atlas.width() as f32, hi[3] * atlas.height() as f32);
                let range = (overlay_font::RANGE / texels).max(1.0);
                for iy in y0.max(0.0) as usize..(y1.ceil() as usize).min(h) {
                    for ix in x0.max(0.0) as usize..(x1.ceil() as usize).min(w) {
                        let (fx, fy) = (ix as f32 + 0.5 - x0, iy as f32 + 0.5 - y0);
                        if fx < 0.0 || fy < 0.0 || fx > x1 - x0 || fy > y1 - y0 {
                            continue;
                        }
                        let a = match lo[2] < 0.0 {
                            true => lo[7],
                            false => {
                                let m = field(u0 + fx * texels, v0 + fy * texels);
                                ((m - 0.5) * range + 0.5).clamp(0.0, 1.0)
                            }
                        };
                        let p = &mut px[iy * w + ix];
                        for k in 0..3 {
                            p[k] = (p[k] as f32 * (1.0 - a) + color[k] * a) as u8;
                        }
                    }
                }
            }
            let rgba: Vec<u8> = px.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            crate::frame_capture::encode_rgba_png(w as u32, h as u32, &rgba).unwrap()
        };
        let d = data();
        let mac = Connection::Connected {
            desktop: "MacBook-Air".into(),
        };
        let away = Connection::NotConnected {
            desktop: "MacBook-Air".into(),
        };
        let toast = VoiceToast {
            stage: crate::debug_protocol::VoiceStage::Listening {
                level: 7,
                seconds_left: None,
            },
            transcript: None,
        };
        let at = |section, page, focus, confirm| Panel {
            open: true,
            section,
            page,
            focus,
            confirm,
            ..Panel::default()
        };
        let keep = Some(Item::Keep(1, "tree-1".into()));
        let views = [
            ("now", at(Section::Now, None, None, None), &mac),
            ("talk", at(Section::Talk, None, None, None), &mac),
            ("tasks", at(Section::Tasks, None, Some(1), None), &mac),
            ("decision", at(Section::Tasks, Some(1), Some(0), None), &mac),
            ("confirm", at(Section::Tasks, Some(1), Some(0), keep), &mac),
            ("cannot-reach", at(Section::Now, None, None, None), &away),
            (
                "not-paired",
                at(Section::Now, None, None, None),
                &Connection::NotPaired,
            ),
        ];
        for (name, panel, connection) in views {
            let shapes = panel.shapes(Some(&d), connection, Some(&toast), Instant::now());
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            std::fs::write(path, paint(&shapes)).unwrap();
        }
        // The conversation: following, and with a question waiting.
        let mut asks = d.clone();
        let c = asks.conversation.as_mut().unwrap();
        c.items.push(SaidLine {
            role: Role::Agent,
            spoken: false,
            cut: false,
            text: "Should the ball stay red after it bounces, or flash back to white?".into(),
        });
        c.question = true;
        let page = Panel {
            open: true,
            conversation: Some(1),
            ..Panel::default()
        };
        for (name, data) in [("conversation", &d), ("question", &asks)] {
            let shapes = page.shapes(Some(data), &mac, Some(&toast), Instant::now());
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            std::fs::write(path, paint(&shapes)).unwrap();
        }
    }
}
