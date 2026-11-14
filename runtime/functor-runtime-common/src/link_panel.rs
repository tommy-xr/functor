//! The headset panel (D28): the shell draws it over any game, opened and
//! closed by a short press of the left menu button
//! ([`crate::link_overlay::Gesture::Panel`]). Sections Now, Talk, Tasks and
//! Game menu, and a task's page with its decision (D35).
//!
//! The right controller drives it: the thumbstick moves a focus ring, A
//! chooses, B goes back (closes, from the section list). Keep, Revert and
//! Stop each wait for a second A, so one stray press changes nothing. While
//! it is open the game gets poses but not those controls ([`game_view`]).
//! Pure: the shell owns the link, the clock and GL.

use std::time::{Duration, Instant};

use cgmath::{vec3, Matrix4, One, Quaternion, Vector3};

use crate::debug_protocol::{PanelState, TaskStance, VoiceToast, WristLight};
use crate::input::XrInputSnapshot;
use crate::link_overlay::{toast_lines, wrap, Shape};
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
    /// The change's tree: a different change cancels a waiting confirm.
    Keep(u64, String),
    Revert(u64, String),
    Stop(u64),
    /// Move route `id`'s utterance (its move at this index).
    Move(u64, Destination),
}

/// How long a refusal from the desktop shows.
const NOTICE: Duration = Duration::from_secs(5);

#[derive(Default)]
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
}

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

    /// The user started talking: show the Talk section.
    pub fn talk(&mut self) {
        if self.open {
            self.section = Section::Talk;
            self.page = None;
            self.focus = None;
            self.confirm = None;
        }
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
        self.settle(data);
        if busy {
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
                Item::Page(_) => unreachable!("pages open without a confirm"),
            };
            if a || b {
                self.confirm = None;
            }
            return a.then_some(Out::Action(action));
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
                item => self.confirm = Some(item),
            }
        }
        None
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
        if let Some(t) = self.page.and_then(find) {
            return match (&t.decision, running(t.stance)) {
                (Some(d), _) => vec![
                    Item::Keep(t.id, d.tree.clone()),
                    Item::Revert(t.id, d.tree.clone()),
                ],
                (None, true) => vec![Item::Stop(t.id)],
                (None, false) => vec![],
            };
        }
        match self.section {
            Section::Now => match data.now.and_then(find) {
                Some(t) if t.decision.is_some() => vec![Item::Page(t.id)],
                Some(t) if running(t.stance) => vec![Item::Stop(t.id)],
                _ => vec![],
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
                }
                .to_string()
            }),
            now: data.and_then(|d| d.now),
            tasks: data.map_or(0, |d| d.tasks.len()),
            wrist,
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

// Metres: about 0.74 m wide at 0.9 m (the mockup's proportions); a body
// glyph is about 0.85 degrees.
pub const WIDTH: f32 = 0.76;
pub const HEIGHT: f32 = 0.54;
const PAD: f32 = 0.024;
const HEADER: f32 = 0.05;
const NAV: f32 = 0.17;
const ROW: f32 = 0.045;
const TITLE: f32 = 0.018;
const BODY: f32 = 0.0135;
const SMALL: f32 = 0.011;
const LABEL: f32 = 0.0095;
const BUTTON: f32 = 0.042;
const RING: f32 = 0.0025;

/// Text in the font's ASCII: typographic marks folded, others `?`.
fn ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ' '..='~' => out.push(c),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2013}' | '\u{2014}' | '\u{2212}' | '\u{b7}' | '\u{2022}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2192}' => out.push_str("->"),
            c if c.is_whitespace() => out.push(' '),
            _ => out.push('?'),
        }
    }
    out
}

/// `text` in at most `n` characters, cut with `...`.
fn fit(text: &str, n: usize) -> String {
    match text.chars().count() {
        len if len <= n => text.to_string(),
        _ => text.chars().take(n.saturating_sub(3)).collect::<String>() + "...",
    }
}

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
        text: ascii(s),
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
        let n = (self.w / size) as usize;
        text(self.out, [self.x, self.y], size, color, &fit(&ascii(s), n));
        self.y -= size * 1.5 + 0.002;
    }

    fn para(&mut self, s: &str, size: f32, color: [f32; 3], lines: usize) {
        for l in wrap(&ascii(s), (self.w / size) as usize, lines) {
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
        if let Some(project) = data.and_then(|d| d.project.as_deref()) {
            let x = left + PAD + 9.0 * BODY;
            text(&mut out, [x, y], BODY, MUTED, &fit(project, 24));
        }
        let (word, color) = match connection {
            Connection::NotPaired => ("NOT PAIRED".to_string(), FAINT),
            Connection::NotConnected { .. } => ("CANNOT REACH".to_string(), WARN),
            Connection::Outdated { .. } => ("NEEDS AN UPDATE".to_string(), WARN),
            Connection::Connected { desktop } => (fit(&ascii(desktop), 22), OK),
        };
        let wx = w / 2.0 - PAD - word.chars().count() as f32 * SMALL;
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
                if self.focus.is_none() && self.confirm.is_none() {
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
        let items = self.items(data);
        let focused = self.focus.and_then(|i| items.get(i));
        let hint = match (connection, data) {
            (Connection::Connected { .. }, Some(data)) => {
                self.content(&mut col, data, toast, focused, bottom)
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
        focused: Option<&Item>,
        bottom: f32,
    ) -> Option<&'static str> {
        let find = |id: u64| data.tasks.iter().find(|t| t.id == id);
        if let Some(t) = self.page.and_then(find) {
            return self.page_of(col, t, focused, bottom);
        }
        match self.section {
            Section::Now => {
                col.line(
                    match data.placed {
                        true => "NOW  -  RUNNING IN THIS HEADSET",
                        false => "NOW  -  ITS GAME RUNS ON THE DESKTOP",
                    },
                    LABEL,
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
                col.para(&t.title, TITLE, TEXT, 2);
                col.line(stance_word(t.stance), SMALL, stance_color(t.stance));
                if !t.activity.is_empty() {
                    col.para(&t.activity, BODY, MUTED, 2);
                }
                if !data.waiting.is_empty() {
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
                self.buttons(col, &[label], focused.is_some(), bottom)
            }
            Section::Talk => {
                col.line("TALK", LABEL, FAINT);
                col.gap(0.004);
                col.para(
                    "Hold the right thumbstick to talk; let go to send.",
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
                let Some(route) = &data.route else {
                    return None;
                };
                col.gap(0.008);
                col.line("WHERE IT WENT", LABEL, FAINT);
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
                col.line("TASKS", LABEL, FAINT);
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
                col.line("GAME MENU", LABEL, FAINT);
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
    fn page_of(
        &self,
        col: &mut Col,
        t: &PanelTask,
        focused: Option<&Item>,
        bottom: f32,
    ) -> Option<&'static str> {
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
            return self.buttons(col, &["Stop"], focused.is_some(), bottom);
        };
        col.line("SIZE OF THE CHANGE", LABEL, FAINT);
        col.line(&d.size, BODY, TEXT);
        col.gap(0.004);
        verdict(col, d);
        if !d.summary.is_empty() {
            col.gap(0.004);
            col.line("SUMMARY", LABEL, FAINT);
            col.para(&d.summary, BODY, TEXT, 3);
        }
        if !d.notes.is_empty() {
            col.gap(0.004);
            col.line("THINGS TO NOTE", LABEL, FAINT);
            for note in &d.notes {
                col.line(&format!("- {note}"), SMALL, MUTED);
            }
        }
        match &self.confirm {
            Some(Item::Keep(..)) => confirm(col, "Keep this change?", bottom),
            Some(Item::Revert(..)) => {
                confirm(col, "Revert it? The files go back as they were.", bottom)
            }
            _ => {
                let focus = focused.is_some();
                self.buttons(col, &["Keep", "Revert"], focus, bottom)
                    .or(Some("A choose   B back   Click & hold R stick: talk"))
            }
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
        let wx = col.x + col.w - inset - word.len() as f32 * SMALL;
        let title_n = ((wx - col.x - 2.0 * inset) / BODY) as usize;
        let ty = col.y - 0.008;
        text(
            col.out,
            [col.x + inset, ty],
            BODY,
            TEXT,
            &fit(&ascii(title), title_n),
        );
        text(col.out, [wx, ty], SMALL, color, word);
        let n = ((col.w - 2.0 * inset) / SMALL) as usize;
        let sy = ty - BODY - 0.006;
        text(
            col.out,
            [col.x + inset, sy],
            SMALL,
            FAINT,
            &fit(&ascii(sub), n),
        );
        col.gap(ROW + 0.006);
    }

    /// A row of buttons above the footer, the focused one ringed.
    fn buttons(
        &self,
        col: &mut Col,
        labels: &[&str],
        focus: bool,
        bottom: f32,
    ) -> Option<&'static str> {
        let gap = 0.012;
        let bw = (col.w - gap * (labels.len() - 1) as f32) / labels.len() as f32;
        let y = bottom + SMALL * 3.0;
        for (i, label) in labels.iter().enumerate() {
            let rect = [col.x + i as f32 * (bw + gap), y, bw, BUTTON];
            let primary = i == 0;
            fill(col.out, rect, if primary { ACCENT } else { SELECTED });
            if focus && self.focus == Some(i) {
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
            let tx = rect[0] + (bw - label.len() as f32 * BODY) / 2.0;
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
        assert_eq!((p.page, p.focus), (Some(1), Some(0)));
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
        assert_eq!(
            press(&mut p, &d, P::A),
            Some(Out::Action(PanelAction::Select { task: 1 }))
        );
        assert_eq!(p.page, Some(1));
        // The desktop kept it meanwhile: the page has no decision, nor the
        // confirm that was waiting.
        press(&mut p, &d, P::A);
        assert!(p.confirm.is_some());
        d.tasks[0].decision = None;
        d.tasks[0].stance = TaskStance::Done;
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!((p.confirm.clone(), p.focus), (None, None));
        // B from that page, focus or not: back to where it opened.
        assert_eq!(press(&mut p, &d, P::B), None);
        assert_eq!((p.page, p.is_open()), (None, true));
        // Running: Stop, confirmed.
        let mut p = Panel::default();
        p.toggle();
        d.now = Some(2);
        press(&mut p, &d, P::A);
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
        press(&mut p, &d, P::A);
        press(&mut p, &d, P::A);
        assert_eq!(p.debug(Some(&d), None).confirm.as_deref(), Some("keep"));
        d.tasks[0].decision.as_mut().unwrap().tree = "tree-9".into();
        // That press keeps nothing; it asks again, for the new change.
        assert_eq!(press(&mut p, &d, P::A), None);
        assert_eq!(p.confirm, Some(Item::Keep(1, "tree-9".into())));

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
    fn every_view_fits_inside_the_panel_in_the_fonts_characters() {
        let d = data();
        let mut long = d.clone();
        long.tasks[0].title = "é".repeat(300);
        long.tasks[0].activity = "\u{201c}x\u{201d} ".repeat(100);
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
                        assert!(text.chars().all(|c| (' '..='~').contains(&c)), "{text}");
                        let right = at[0] + text.chars().count() as f32 * size;
                        assert!(at[0] >= -hw && right <= hw, "{text}: {right}");
                        assert!(at[1] <= hh && at[1] - size >= -hh, "{text}: {}", at[1]);
                    }
                }
            }
        }
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
        let font = crate::sprite_font::atlas_texture_data();
        let scale = 1600.0;
        let (w, h) = ((WIDTH * scale) as usize, (HEIGHT * scale) as usize);
        let paint = |shapes: &[Shape]| {
            let mut px = vec![[30u8, 24, 40]; w * h];
            let mut blend = |x: f32, y: f32, c: [f32; 3], a: f32| {
                let ix = (x * scale + w as f32 / 2.0) as usize;
                let iy = (h as f32 / 2.0 - y * scale) as usize;
                if ix < w && iy < h {
                    let p = &mut px[iy * w + ix];
                    for k in 0..3 {
                        p[k] = (p[k] as f32 * (1.0 - a) + c[k] * 255.0 * a) as u8;
                    }
                }
            };
            for s in shapes {
                match s {
                    Shape::Rect {
                        rect: [x, y, rw, rh],
                        color,
                    } => {
                        for j in 0..(rh * scale).ceil() as usize {
                            for i in 0..(rw * scale).ceil() as usize {
                                let c = [color[0], color[1], color[2]];
                                blend(x + i as f32 / scale, y + j as f32 / scale, c, color[3]);
                            }
                        }
                    }
                    Shape::Text {
                        at,
                        size,
                        color,
                        text,
                    } => {
                        let n = (size * scale).ceil() as usize;
                        for (k, ch) in text.chars().enumerate() {
                            let Some((cx, cy, ..)) = crate::sprite_font::glyph_cell(ch) else {
                                continue;
                            };
                            for j in 0..n {
                                for i in 0..n {
                                    let gx = cx as usize + i * 8 / n;
                                    let gy = cy as usize + j * 8 / n;
                                    if font.bytes[(gy * font.width as usize + gx) * 4 + 3] > 0 {
                                        let x = at[0] + k as f32 * size + i as f32 / scale;
                                        blend(x, at[1] - j as f32 / scale, *color, 1.0);
                                    }
                                }
                            }
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
    }
}
