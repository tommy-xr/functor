//! The canvas layout (D20): a tree of splits whose leaves are panes of tabs,
//! each tab a [`View`]. A view is open at most once in the whole layout, so a
//! view names its tab. The game view's placement (D21) is part of it: only
//! [`Placement::Pane`] has a Game tab. [`Layout::apply`] is the one reducer
//! and [`Layout::normalize`] the one place the invariants are enforced.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::session::SessionMode;

/// What a tab shows. New view types need no layout work.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    Game,
    /// `path` is relative to the project directory.
    File {
        path: String,
    },
    /// The task's change: its diff and review.
    Changes {
        task: u64,
    },
    Task {
        task: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Pane {
    /// Unique among the layout's panes and splits.
    pub id: u64,
    /// In tab-bar order. Empty only for a pane a split just made.
    pub tabs: Vec<View>,
    /// Index of the shown tab in `tabs` (0 when empty).
    pub active: usize,
}

/// `Right`: the second child sits right of the first; `Down`: below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Right,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Pane(Pane),
    /// Always two children, so there is no empty split.
    Split {
        id: u64,
        direction: Direction,
        /// The first child's share, in thousandths ([`RATIO_MIN`]..=[`RATIO_MAX`]).
        ratio: u16,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Layout {
    /// `None`: an empty canvas.
    pub root: Option<Node>,
    /// The pane views open in by default; a pane of `root` exactly when
    /// there is one.
    pub focus: Option<u64>,
    /// Where the game view shows. Absent from layouts saved before it
    /// existed, which load as a pane.
    #[serde(default)]
    pub game: GamePlacement,
    /// What the agent arranged (D25). Absent from older layouts.
    #[serde(default)]
    pub agent: AgentLayout,
}

/// The agent's part of a layout (D25): kept with it, so it survives restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentLayout {
    /// "Let the agent arrange views". Off, its view requests change nothing.
    #[serde(default = "on")]
    pub arranges: bool,
    /// Open tabs the agent opened: marked, and closed by "Close agent tabs".
    #[serde(default)]
    pub tabs: Vec<View>,
    /// The layout before the first change of the agent's latest run that
    /// changed it: "Restore my layout" returns to it. Dropped once the
    /// layout is that again.
    #[serde(default)]
    pub restore: Option<Box<Layout>>,
    /// That run, as `[task, run]`: its later changes keep the snapshot.
    #[serde(default)]
    pub restore_run: Option<[u64; 2]>,
}

fn on() -> bool {
    true
}

impl Default for AgentLayout {
    fn default() -> Self {
        Self {
            arranges: true,
            tabs: Vec::new(),
            restore: None,
            restore_run: None,
        }
    }
}

/// Where [`Layout::agent_show`] put a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    /// Added to a pane beside the user's, and shown there.
    Opened,
    /// Already open beside the user's pane: shown there.
    Shown,
    /// The tab the user's pane shows: nothing to change.
    InView,
    /// Added (or left) behind the tab the user's pane shows: the canvas
    /// stacks its panes, so no split was made.
    Behind,
    /// The game is in its mini view or own window, already in sight.
    GameElsewhere(Placement),
}

/// Where the project's one game view shows (D21).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// A Game tab in a pane, streamed from a hidden session.
    #[default]
    Pane,
    /// Floating over the canvas, streamed from a hidden session; no Game tab.
    Mini,
    /// The session runs as a normal game window; no Game tab.
    Window,
}

impl Placement {
    /// How a session for this placement runs: switching between placements
    /// with different modes restarts a live session.
    pub fn session_mode(self) -> SessionMode {
        match self {
            Self::Pane | Self::Mini => SessionMode::Hidden,
            Self::Window => SessionMode::Window,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The mini view: the canvas corner it sits in and its width in CSS
/// pixels ([`MINI_MIN`]..=[`MINI_MAX`]); its height follows the game's
/// aspect ratio, and the page shrinks it to fit a smaller canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Mini {
    pub corner: Corner,
    pub width: u32,
}

impl Default for Mini {
    fn default() -> Self {
        Self {
            corner: Corner::BottomRight,
            width: 360,
        }
    }
}

pub const MINI_MIN: u32 = 280;
pub const MINI_MAX: u32 = 1200;

/// Where the Game tab was when it left its pane, so docking puts it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Dock {
    pub pane: u64,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GamePlacement {
    pub placement: Placement,
    pub mini: Mini,
    /// Set when the Game tab leaves a pane; taken when it comes back.
    pub dock: Option<Dock>,
}

/// Where [`LayoutOp::Open`] puts a view that is not open yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// The focused pane (a new one on an empty canvas).
    Focused,
    /// The pane after the focused one, or before it if it is last; a split
    /// right of the focused pane when it is the only one.
    Side,
    Pane {
        pane: u64,
    },
}

/// A change to the layout. Ops naming a pane or split that is gone (another
/// window changed the layout first) change nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutOp {
    /// Shows `view` and focuses its pane: where it is if already open, else
    /// added last in `target`.
    Open {
        view: View,
        target: Target,
    },
    /// Moves an open `view` to `pane` before tab `index` (or last), shown
    /// and focused there. A pane it leaves empty is removed.
    Move {
        view: View,
        pane: u64,
        index: usize,
    },
    /// Closes `view`'s tab; a pane it leaves empty is removed.
    Close {
        view: View,
    },
    /// Removes `pane` with its tabs; its sibling takes its space.
    ClosePane {
        pane: u64,
    },
    /// Adds an empty pane beside `pane`, halving its space, and focuses it.
    Split {
        pane: u64,
        direction: Direction,
    },
    Focus {
        pane: u64,
    },
    Resize {
        split: u64,
        ratio: u16,
    },
    /// Back to [`Layout::initial`]'s panes; the placement stays.
    Reset,
    /// Moves the game view. Back to a pane, the Game tab goes where it left
    /// from, or to the focused pane if that pane is gone.
    Place {
        placement: Placement,
    },
    /// The mini view's corner and width (clamped).
    SetMini {
        mini: Mini,
    },
    /// "Restore my layout": back to [`AgentLayout::restore`], the
    /// placement staying.
    Restore,
    /// Closes every tab the agent opened ([`AgentLayout::tabs`]).
    CloseAgentTabs,
    /// "Let the agent arrange views" on or off.
    AgentArranges {
        on: bool,
    },
}

pub const RATIO_MIN: u16 = 100;
pub const RATIO_MAX: u16 = 900;

impl Pane {
    /// Keeps the tabs `keep` accepts; the shown tab stays shown if kept,
    /// else the one now at its place (or the last) shows.
    fn retain_tabs(&mut self, keep: &mut impl FnMut(&View) -> bool) {
        let shown = self.tabs.get(self.active).cloned();
        let mut at = self.active;
        let mut i = 0;
        self.tabs.retain(|t| {
            let kept = keep(t);
            if !kept && i < self.active {
                at -= 1;
            }
            i += 1;
            kept
        });
        self.active = match shown {
            Some(v) if self.tabs.contains(&v) => self.tabs.iter().position(|t| *t == v).unwrap(),
            _ => at.min(self.tabs.len().saturating_sub(1)),
        };
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::initial()
    }
}

impl Layout {
    /// One pane with the Game tab: a project's layout until it has its own.
    pub fn initial() -> Self {
        Self {
            root: Some(Node::Pane(Pane {
                id: 1,
                tabs: vec![View::Game],
                active: 0,
            })),
            focus: Some(1),
            game: GamePlacement::default(),
            agent: AgentLayout::default(),
        }
    }

    /// Every pane, left to right and top to bottom.
    pub fn panes(&self) -> Vec<&Pane> {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Pane>) {
            match node {
                Node::Pane(p) => out.push(p),
                Node::Split { first, second, .. } => {
                    walk(first, out);
                    walk(second, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            walk(root, &mut out);
        }
        out
    }

    /// The pane `view` is open in, and its tab index.
    pub fn find(&self, view: &View) -> Option<(u64, usize)> {
        self.panes()
            .into_iter()
            .find_map(|p| Some((p.id, p.tabs.iter().position(|t| t == view)?)))
    }

    /// Applies `op` and restores the invariants; whether the layout changed.
    /// `valid` says why a view cannot be opened (a file outside the project,
    /// a task that does not exist).
    pub fn apply(
        &mut self,
        op: LayoutOp,
        valid: impl Fn(&View) -> Result<(), String>,
    ) -> Result<bool, String> {
        let before = self.clone();
        match op {
            LayoutOp::Open { view, target } => {
                // An open view is only shown, so a listing not in yet cannot refuse it.
                if self.find(&view).is_none() {
                    valid(&view)?;
                }
                // Opening the game brings it into a pane, at `target`, or the
                // focused pane if that one is gone: never placed with no tab.
                let target = match target {
                    Target::Pane { pane }
                        if view == View::Game && self.pane_mut(pane).is_none() =>
                    {
                        Target::Focused
                    }
                    target => target,
                };
                if view == View::Game {
                    self.game.placement = Placement::Pane;
                    self.game.dock = None;
                }
                self.open(view, target);
            }
            LayoutOp::Move { view, pane, index } => self.move_view(&view, pane, index),
            LayoutOp::Close { view } => {
                if let Some((pane, i)) = self.find(&view) {
                    self.take_tab(pane, i);
                }
            }
            LayoutOp::ClosePane { pane } => self.remove_pane(pane),
            LayoutOp::Split { pane, direction } => {
                if let Some(new) = self.split(pane, direction) {
                    self.focus = Some(new);
                }
            }
            LayoutOp::Focus { pane } => {
                if self.pane_mut(pane).is_some() {
                    self.focus = Some(pane);
                }
            }
            LayoutOp::Resize { split, ratio } => {
                if let Some(Node::Split { ratio: r, .. }) = self.node_mut(split) {
                    *r = ratio;
                }
            }
            LayoutOp::Reset => {
                *self = Self {
                    game: self.game,
                    agent: std::mem::take(&mut self.agent),
                    ..Self::initial()
                }
            }
            LayoutOp::Place { placement } => self.place(placement),
            LayoutOp::SetMini { mini } => self.game.mini = mini,
            LayoutOp::Restore => {
                if let Some(before) = self.agent.restore.take() {
                    self.root = before.root;
                    self.focus = before.focus;
                    self.agent.tabs.clear();
                    self.agent.restore_run = None;
                }
            }
            LayoutOp::CloseAgentTabs => {
                for view in std::mem::take(&mut self.agent.tabs) {
                    if let Some((pane, i)) = self.find(&view) {
                        self.take_tab(pane, i);
                    }
                }
            }
            LayoutOp::AgentArranges { on } => self.agent.arranges = on,
        }
        self.normalize();
        Ok(*self != before)
    }

    /// Drops the tabs `keep` refuses (a deleted file, a removed task); a pane
    /// that leaves empty is removed.
    pub fn retain(&mut self, keep: impl Fn(&View) -> bool) {
        self.retain_dyn(&keep);
    }

    fn retain_dyn(&mut self, keep: &dyn Fn(&View) -> bool) {
        let emptied: Vec<u64> = self
            .panes()
            .into_iter()
            .filter(|p| !p.tabs.is_empty() && !p.tabs.iter().any(keep))
            .map(|p| p.id)
            .collect();
        for id in emptied {
            self.remove_pane(id);
        }
        self.for_each_pane(|p| p.retain_tabs(&mut |t| keep(t)));
        // "Restore my layout" never brings a dropped tab back.
        if let Some(before) = &mut self.agent.restore {
            before.retain_dyn(keep);
        }
        self.normalize();
    }

    /// The invariants, for a layout from any source (disk included): each
    /// view open once (so at most one Game tab, and none unless the game's
    /// placement is a pane), unique ids, each pane's `active` in range,
    /// ratios and the mini width clamped, and the focus on a pane.
    pub fn normalize(&mut self) {
        let mut views = HashSet::new();
        let mut ids = HashSet::new();
        let mut next = self.next_id();
        fn walk(
            node: &mut Node,
            views: &mut HashSet<View>,
            ids: &mut HashSet<u64>,
            next: &mut u64,
        ) {
            let id = match node {
                Node::Pane(p) => {
                    p.retain_tabs(&mut |t| views.insert(t.clone()));
                    &mut p.id
                }
                Node::Split {
                    id,
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    *ratio = (*ratio).clamp(RATIO_MIN, RATIO_MAX);
                    walk(first, views, ids, next);
                    walk(second, views, ids, next);
                    id
                }
            };
            if !ids.insert(*id) {
                *id = *next;
                ids.insert(*next);
                *next += 1;
            }
        }
        if let Some(root) = &mut self.root {
            walk(root, &mut views, &mut ids, &mut next);
        }
        // The Game tab leaving for Mini or Own window remembers its place.
        if self.game.placement != Placement::Pane {
            if let Some((pane, index)) = self.find(&View::Game) {
                self.game.dock = Some(Dock { pane, index });
                self.take_tab(pane, index);
            }
        }
        // A dock whose pane is gone is dropped, so a later pane given its id
        // is never mistaken for it.
        if let Some(dock) = self.game.dock {
            if self.pane_mut(dock.pane).is_none() {
                self.game.dock = None;
            }
        }
        self.game.mini.width = self.game.mini.width.clamp(MINI_MIN, MINI_MAX);
        let panes = self.panes();
        if !panes.iter().any(|p| Some(p.id) == self.focus) {
            self.focus = panes.first().map(|p| p.id);
        }
        // The agent's marks name open tabs only; a snapshot that is the
        // layout again has nothing left to restore.
        let mut tabs = std::mem::take(&mut self.agent.tabs);
        let mut seen = HashSet::new();
        tabs.retain(|t| self.find(t).is_some() && seen.insert(t.clone()));
        self.agent.tabs = tabs;
        if let Some(before) = &mut self.agent.restore {
            before.agent = AgentLayout::default();
            before.normalize();
            if (&before.root, before.focus) == (&self.root, self.focus) {
                self.agent.restore = None;
                self.agent.restore_run = None;
            }
        }
    }

    /// Shows `view` for the agent without taking the wheel (D25): never in
    /// the focused pane over the tab it shows, never moving the focus. An
    /// open view is shown where it is, or moved out from behind the user's
    /// tab; a new one goes to the pane after (else before) the focused one,
    /// made by a split right when there is none — unless the canvas is
    /// `stacked`, where it goes behind the user's tab instead. The first
    /// change of each run (`[task, run]`) snapshots the layout; tabs it adds
    /// are marked. `valid` says why a view cannot be opened.
    pub fn agent_show(
        &mut self,
        view: &View,
        stacked: bool,
        run: [u64; 2],
        valid: impl Fn(&View) -> Result<(), String>,
    ) -> Result<Placed, String> {
        if *view == View::Game && self.game.placement != Placement::Pane {
            return Ok(Placed::GameElsewhere(self.game.placement));
        }
        let found = self.find(view);
        if found.is_none() {
            valid(view)?;
        }
        // A snapshot from an earlier run would undo the user's work since.
        if self.agent.restore.is_none() || self.agent.restore_run != Some(run) {
            let mut before = self.clone();
            before.agent = AgentLayout::default();
            self.agent.restore = Some(Box::new(before));
            self.agent.restore_run = Some(run);
        }
        let user = self.focus;
        let placed = match found {
            Some((pane, i)) if Some(pane) != user => {
                self.pane_mut(pane).expect("found").active = i;
                Placed::Shown
            }
            Some((_, i)) if self.focused_shows(i) => Placed::InView,
            // Behind the user's tab: out to a pane of its own, if there can be one.
            Some((pane, i)) => match self.side_pane(stacked) {
                Some(to) => {
                    // Never the pane's last tab: the user's shown one stays.
                    self.take_tab(pane, i);
                    self.add_shown(to, view.clone());
                    Placed::Shown
                }
                None => Placed::Behind,
            },
            None => {
                let placed = match self.side_pane(stacked) {
                    Some(to) => {
                        self.add_shown(to, view.clone());
                        Placed::Opened
                    }
                    // An empty pane (one a split left) shows nothing to keep.
                    None => {
                        let focus = user.expect("no pane for the agent only beside the user's");
                        let p = self.pane_mut(focus).expect("focused");
                        p.tabs.push(view.clone());
                        match p.tabs.len() {
                            1 => Placed::Opened,
                            _ => Placed::Behind,
                        }
                    }
                };
                self.agent.tabs.push(view.clone());
                placed
            }
        };
        self.normalize();
        Ok(placed)
    }

    /// Whether the focused pane shows its tab `i`.
    fn focused_shows(&self, i: usize) -> bool {
        self.panes()
            .into_iter()
            .any(|p| Some(p.id) == self.focus && p.active == i)
    }

    /// The pane "to the side": the focused pane's neighbour, a new one split
    /// right of it, or on an empty canvas the first pane. `None` when the
    /// canvas is `stacked` and the focused pane is the only one.
    fn side_pane(&mut self, stacked: bool) -> Option<u64> {
        let Some(focus) = self.focus else {
            return Some(self.add_root());
        };
        match self.neighbour(focus) {
            Some(pane) => Some(pane),
            None if stacked => None,
            None => self.split(focus, Direction::Right),
        }
    }

    /// Adds `view` last in `pane` and shows it there; the focus stays.
    fn add_shown(&mut self, pane: u64, view: View) {
        let p = self.pane_mut(pane).expect("a pane of the layout");
        p.tabs.push(view);
        p.active = p.tabs.len() - 1;
    }

    fn open(&mut self, view: View, target: Target) {
        if let Some((pane, i)) = self.find(&view) {
            // Asked for in a named pane ("open here"): the tab moves there.
            // A view is open once, so anywhere else it is only shown.
            match target {
                Target::Pane { pane: to } if to != pane && self.pane_mut(to).is_some() => {
                    self.move_view(&view, to, usize::MAX)
                }
                _ => self.show(pane, i),
            }
            return;
        }
        let pane = match target {
            Target::Pane { pane } if self.pane_mut(pane).is_some() => pane,
            Target::Pane { .. } => return,
            Target::Focused => match self.focus {
                Some(pane) => pane,
                None => self.add_root(),
            },
            Target::Side => self.side_pane(false).expect("a side pane unless stacked"),
        };
        let p = self.pane_mut(pane).expect("a pane of the layout");
        p.tabs.push(view);
        let last = p.tabs.len() - 1;
        self.show(pane, last);
    }

    /// Leaving a pane, `normalize` takes the Game tab out; coming back, it
    /// goes to its dock if that pane is still there.
    fn place(&mut self, to: Placement) {
        if self.game.placement == to {
            return;
        }
        self.game.placement = to;
        if to != Placement::Pane {
            return;
        }
        match self.game.dock.take() {
            Some(Dock { pane, index }) if self.pane_mut(pane).is_some() => {
                self.open(View::Game, Target::Pane { pane });
                self.move_view(&View::Game, pane, index);
            }
            _ => self.open(View::Game, Target::Focused),
        }
    }

    fn move_view(&mut self, view: &View, to: u64, index: usize) {
        let Some((from, i)) = self.find(view) else {
            return;
        };
        if self.pane_mut(to).is_none() {
            return;
        }
        let p = self.pane_mut(from).expect("found");
        let view = p.tabs.remove(i);
        let left_empty = p.tabs.is_empty() && from != to;
        if p.active > i {
            p.active -= 1;
        }
        let p = self.pane_mut(to).expect("checked");
        // `index` counts the tabs as they were, the moved one included.
        let index = if from == to && index > i {
            index - 1
        } else {
            index
        };
        let at = index.min(p.tabs.len());
        p.tabs.insert(at, view);
        self.show(to, at);
        if left_empty {
            self.remove_pane(from);
        }
    }

    /// Removes tab `i` of `pane`, and the pane when it was its last tab.
    fn take_tab(&mut self, pane: u64, i: usize) {
        let p = self.pane_mut(pane).expect("found");
        p.tabs.remove(i);
        if p.tabs.is_empty() {
            return self.remove_pane(pane);
        }
        // The tab after it shows, or the one before when it was last.
        if p.active > i || p.active == p.tabs.len() {
            p.active -= 1;
        }
    }

    fn show(&mut self, pane: u64, i: usize) {
        self.pane_mut(pane).expect("a pane of the layout").active = i;
        self.focus = Some(pane);
    }

    /// The pane after `pane` in reading order, else the one before it.
    fn neighbour(&self, pane: u64) -> Option<u64> {
        let ids: Vec<u64> = self.panes().iter().map(|p| p.id).collect();
        let i = ids.iter().position(|&id| id == pane)?;
        ids.get(i + 1)
            .or(i.checked_sub(1).and_then(|j| ids.get(j)))
            .copied()
    }

    fn add_root(&mut self) -> u64 {
        let id = self.next_id();
        self.root = Some(Node::Pane(Pane {
            id,
            tabs: vec![],
            active: 0,
        }));
        id
    }

    /// Splits `pane`, putting a new empty pane after it; the new pane's id.
    fn split(&mut self, pane: u64, direction: Direction) -> Option<u64> {
        let (split, new) = (self.next_id(), self.next_id() + 1);
        let node = self.node_mut(pane).filter(|n| matches!(n, Node::Pane(_)))?;
        let fresh = Node::Pane(Pane {
            id: new,
            tabs: vec![],
            active: 0,
        });
        let old = std::mem::replace(node, fresh.clone());
        *node = Node::Split {
            id: split,
            direction,
            ratio: 500,
            first: Box::new(old),
            second: Box::new(fresh),
        };
        Some(new)
    }

    /// Removes `pane`; its sibling takes the parent split's place, and the
    /// focus moves to its neighbour if it was on it.
    fn remove_pane(&mut self, pane: u64) {
        if self.focus == Some(pane) {
            self.focus = self.neighbour(pane);
        }
        fn without(node: Node, pane: u64) -> Option<Node> {
            match node {
                Node::Pane(p) if p.id == pane => None,
                Node::Pane(_) => Some(node),
                Node::Split {
                    id,
                    direction,
                    ratio,
                    first,
                    second,
                } => match (without(*first, pane), without(*second, pane)) {
                    (Some(first), Some(second)) => Some(Node::Split {
                        id,
                        direction,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    (one, other) => one.or(other),
                },
            }
        }
        self.root = self.root.take().and_then(|n| without(n, pane));
    }

    fn next_id(&self) -> u64 {
        fn max(node: &Node) -> u64 {
            match node {
                Node::Pane(p) => p.id,
                Node::Split {
                    id, first, second, ..
                } => (*id).max(max(first)).max(max(second)),
            }
        }
        self.root.as_ref().map_or(0, max) + 1
    }

    /// The pane or split with `id`.
    fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        fn find(node: &mut Node, id: u64) -> Option<&mut Node> {
            let hit = match node {
                Node::Pane(p) => p.id == id,
                Node::Split { id: s, .. } => *s == id,
            };
            if hit {
                return Some(node);
            }
            match node {
                Node::Pane(_) => None,
                Node::Split { first, second, .. } => find(first, id).or_else(|| find(second, id)),
            }
        }
        find(self.root.as_mut()?, id)
    }

    fn pane_mut(&mut self, id: u64) -> Option<&mut Pane> {
        match self.node_mut(id)? {
            Node::Pane(p) => Some(p),
            Node::Split { .. } => None,
        }
    }

    fn for_each_pane(&mut self, mut f: impl FnMut(&mut Pane)) {
        fn walk(node: &mut Node, f: &mut impl FnMut(&mut Pane)) {
            match node {
                Node::Pane(p) => f(p),
                Node::Split { first, second, .. } => {
                    walk(first, f);
                    walk(second, f);
                }
            }
        }
        if let Some(root) = &mut self.root {
            walk(root, &mut f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> View {
        View::File {
            path: path.to_string(),
        }
    }

    fn pane(id: u64, tabs: &[View], active: usize) -> Node {
        Node::Pane(Pane {
            id,
            tabs: tabs.to_vec(),
            active,
        })
    }

    fn split(id: u64, direction: Direction, first: Node, second: Node) -> Node {
        Node::Split {
            id,
            direction,
            ratio: 500,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn layout(root: Node, focus: u64) -> Layout {
        Layout {
            root: Some(root),
            focus: Some(focus),
            game: GamePlacement::default(),
            agent: Default::default(),
        }
    }

    fn ok(_: &View) -> Result<(), String> {
        Ok(())
    }

    /// Applies `op` with every view valid; whether it changed `l`.
    fn apply(l: &mut Layout, op: LayoutOp) -> bool {
        l.apply(op, ok).unwrap()
    }

    fn open(view: View, target: Target) -> LayoutOp {
        LayoutOp::Open { view, target }
    }

    /// Game and a.fun side by side, a.fun's pane focused.
    fn two() -> Layout {
        layout(
            split(
                3,
                Direction::Right,
                pane(1, &[View::Game], 0),
                pane(2, &[file("a.fun")], 0),
            ),
            2,
        )
    }

    #[test]
    fn the_initial_layout_is_one_pane_with_the_game() {
        assert_eq!(Layout::initial(), layout(pane(1, &[View::Game], 0), 1));
        assert_eq!(Layout::default(), Layout::initial());
    }

    #[test]
    fn open_adds_last_in_the_focused_pane_and_shows_it() {
        let mut l = Layout::initial();
        assert!(apply(&mut l, open(file("a.fun"), Target::Focused)));
        assert!(apply(&mut l, open(file("b.fun"), Target::Focused)));
        assert_eq!(
            l,
            layout(pane(1, &[View::Game, file("a.fun"), file("b.fun")], 2), 1)
        );
    }

    #[test]
    fn opening_an_open_view_focuses_it_where_it_is() {
        let mut l = two();
        for target in [Target::Focused, Target::Side, Target::Pane { pane: 1 }] {
            let mut l2 = l.clone();
            assert!(apply(&mut l2, open(View::Game, target)));
            // Shown and focused in pane 1, never a second Game tab.
            assert_eq!(l2.focus, Some(1));
            assert_eq!(l2.find(&View::Game), Some((1, 0)));
            assert_eq!(l2.panes().iter().map(|p| p.tabs.len()).sum::<usize>(), 2);
        }
        // Already shown and focused: nothing changes.
        assert!(!apply(&mut l, open(file("a.fun"), Target::Focused)));
    }

    #[test]
    fn opening_an_open_view_in_another_pane_moves_it_there() {
        let mut l = two();
        assert!(apply(&mut l, open(file("b.fun"), Target::Pane { pane: 1 })));
        let (to, _) = l.find(&file("a.fun")).unwrap();
        let from = l.find(&View::Game).unwrap().0;
        assert_ne!(from, to);
        assert!(apply(
            &mut l,
            open(file("b.fun"), Target::Pane { pane: to })
        ));
        // One tab for it, last in the named pane, shown and focused.
        assert_eq!(l.find(&file("b.fun")), Some((to, 1)));
        assert_eq!(l.focus, Some(to));
        assert_eq!(l.panes().iter().map(|p| p.tabs.len()).sum::<usize>(), 3);
        // The last tab of a pane leaves no empty pane behind.
        assert!(apply(&mut l, open(View::Game, Target::Pane { pane: to })));
        assert_eq!(l.panes().len(), 1);
    }

    #[test]
    fn open_to_the_side_uses_the_neighbour_or_splits_right() {
        // One pane: a split right, the view in the new pane.
        let mut l = Layout::initial();
        assert!(apply(&mut l, open(file("a.fun"), Target::Side)));
        let game = |tabs: &[View], active| pane(1, tabs, active);
        assert_eq!(
            l,
            layout(
                split(
                    2,
                    Direction::Right,
                    game(&[View::Game], 0),
                    pane(3, &[file("a.fun")], 0)
                ),
                3
            )
        );
        // From the last pane, the side is the one before it.
        assert!(apply(&mut l, open(file("b.fun"), Target::Side)));
        assert_eq!(
            l,
            layout(
                split(
                    2,
                    Direction::Right,
                    game(&[View::Game, file("b.fun")], 1),
                    pane(3, &[file("a.fun")], 0)
                ),
                1
            )
        );
        // From the first, the one after it.
        assert!(apply(&mut l, open(file("c.fun"), Target::Side)));
        assert_eq!(l.find(&file("c.fun")), Some((3, 1)));
        assert_eq!(l.focus, Some(3));
    }

    #[test]
    fn opening_on_an_empty_canvas_makes_a_pane() {
        for target in [Target::Focused, Target::Side] {
            let mut l = Layout {
                root: None,
                focus: None,
                game: GamePlacement::default(),
                agent: Default::default(),
            };
            assert!(apply(&mut l, open(View::Game, target)));
            assert_eq!(l, Layout::initial());
        }
    }

    #[test]
    fn open_into_a_named_pane() {
        let mut l = two();
        assert!(apply(&mut l, open(file("b.fun"), Target::Pane { pane: 1 })));
        assert_eq!(l.find(&file("b.fun")), Some((1, 1)));
        assert_eq!(l.focus, Some(1));
        // A pane that is gone (or a split's id): nothing.
        for pane in [9, 3] {
            assert!(!apply(&mut l, open(file("c.fun"), Target::Pane { pane })));
        }
    }

    #[test]
    fn an_invalid_view_is_refused_and_changes_nothing() {
        let mut l = two();
        let refuse = |_: &View| Err("no such file".to_string());
        assert_eq!(
            l.apply(open(file("x.fun"), Target::Focused), refuse),
            Err("no such file".to_string())
        );
        assert_eq!(l, two());
    }

    #[test]
    fn move_between_panes_shows_and_focuses_it_there() {
        let mut l = layout(
            split(
                3,
                Direction::Right,
                pane(1, &[View::Game, file("b.fun")], 0),
                pane(2, &[file("a.fun")], 0),
            ),
            1,
        );
        assert!(apply(
            &mut l,
            LayoutOp::Move {
                view: View::Game,
                pane: 2,
                index: 0
            }
        ));
        assert_eq!(
            l,
            layout(
                split(
                    3,
                    Direction::Right,
                    pane(1, &[file("b.fun")], 0),
                    pane(2, &[View::Game, file("a.fun")], 0)
                ),
                2
            )
        );
        // The pane it leaves empty goes; its sibling takes the space.
        assert!(apply(
            &mut l,
            LayoutOp::Move {
                view: file("b.fun"),
                pane: 2,
                index: 99
            }
        ));
        assert_eq!(
            l,
            layout(pane(2, &[View::Game, file("a.fun"), file("b.fun")], 2), 2)
        );
        // A view that is not open, or a pane that is gone: nothing.
        assert!(!apply(
            &mut l,
            LayoutOp::Move {
                view: file("z.fun"),
                pane: 2,
                index: 0
            }
        ));
        assert!(!apply(
            &mut l,
            LayoutOp::Move {
                view: View::Game,
                pane: 7,
                index: 0
            }
        ));
    }

    #[test]
    fn move_within_a_pane_reorders_before_the_given_tab() {
        let tabs = || vec![file("a"), file("b"), file("c"), file("d")];
        let order = |l: &Layout| l.panes()[0].tabs.clone();
        let mut l = layout(pane(1, &tabs(), 0), 1);
        // "Before c" (index 2) for a: a lands between b and c.
        assert!(apply(
            &mut l,
            LayoutOp::Move {
                view: file("a"),
                pane: 1,
                index: 2
            }
        ));
        assert_eq!(order(&l), [file("b"), file("a"), file("c"), file("d")]);
        assert_eq!(l.panes()[0].active, 1);
        // Leftwards, and to the end.
        assert!(apply(
            &mut l,
            LayoutOp::Move {
                view: file("d"),
                pane: 1,
                index: 0
            }
        ));
        assert_eq!(order(&l), [file("d"), file("b"), file("a"), file("c")]);
        assert!(apply(
            &mut l,
            LayoutOp::Move {
                view: file("d"),
                pane: 1,
                index: 4
            }
        ));
        assert_eq!(order(&l), [file("b"), file("a"), file("c"), file("d")]);
        // Onto itself: shown, nothing else.
        assert!(!apply(
            &mut l,
            LayoutOp::Move {
                view: file("d"),
                pane: 1,
                index: 3
            }
        ));
        // Its only tab onto its own pane keeps the pane.
        let mut one = Layout::initial();
        assert!(!apply(
            &mut one,
            LayoutOp::Move {
                view: View::Game,
                pane: 1,
                index: 0
            }
        ));
        assert_eq!(one, Layout::initial());
    }

    #[test]
    fn split_adds_an_empty_focused_pane_after_it() {
        let mut l = Layout::initial();
        assert!(apply(
            &mut l,
            LayoutOp::Split {
                pane: 1,
                direction: Direction::Right
            }
        ));
        assert_eq!(
            l,
            layout(
                split(
                    2,
                    Direction::Right,
                    pane(1, &[View::Game], 0),
                    pane(3, &[], 0)
                ),
                3
            )
        );
        assert!(apply(
            &mut l,
            LayoutOp::Split {
                pane: 3,
                direction: Direction::Down
            }
        ));
        assert_eq!(
            l,
            layout(
                split(
                    2,
                    Direction::Right,
                    pane(1, &[View::Game], 0),
                    split(4, Direction::Down, pane(3, &[], 0), pane(5, &[], 0))
                ),
                5
            )
        );
        // A view opens in the new, focused pane.
        assert!(apply(&mut l, open(file("a.fun"), Target::Focused)));
        assert_eq!(l.find(&file("a.fun")), Some((5, 0)));
        // A split's id or a gone pane: nothing.
        for pane in [2, 9] {
            assert!(!apply(
                &mut l,
                LayoutOp::Split {
                    pane,
                    direction: Direction::Down
                }
            ));
        }
    }

    #[test]
    fn closing_a_tab_shows_its_neighbour() {
        let tabs = [file("a"), file("b"), file("c")];
        for (active, close, shown) in [
            // The shown tab: the one after it, or before it when last.
            (1, "b", 1),
            (2, "c", 1),
            // Another tab: the shown one stays shown.
            (2, "a", 1),
            (0, "c", 0),
        ] {
            let mut l = layout(pane(1, &tabs, active), 1);
            assert!(apply(&mut l, LayoutOp::Close { view: file(close) }));
            assert_eq!(l.panes()[0].tabs.len(), 2);
            assert_eq!(
                l.panes()[0].active,
                shown,
                "active {active}, closing {close}"
            );
        }
        // A view that is not open: nothing.
        let mut l = two();
        assert!(!apply(
            &mut l,
            LayoutOp::Close {
                view: file("z.fun")
            }
        ));
    }

    #[test]
    fn closing_a_panes_last_tab_removes_it() {
        let mut l = two();
        assert!(apply(
            &mut l,
            LayoutOp::Close {
                view: file("a.fun")
            }
        ));
        // The sibling takes the space and the focus.
        assert_eq!(l, layout(pane(1, &[View::Game], 0), 1));
        assert!(apply(&mut l, LayoutOp::Close { view: View::Game }));
        assert_eq!(
            l,
            Layout {
                root: None,
                focus: None,
                game: GamePlacement::default(),
                agent: Default::default(),
            }
        );
    }

    #[test]
    fn close_pane_removes_it_with_its_tabs() {
        let mut l = two();
        assert!(apply(&mut l, LayoutOp::ClosePane { pane: 1 }));
        assert_eq!(l, layout(pane(2, &[file("a.fun")], 0), 2));
        assert!(!apply(&mut l, LayoutOp::ClosePane { pane: 1 }));
        // An unfocused pane's removal keeps the focus where it was.
        let mut l = layout(
            split(
                5,
                Direction::Right,
                pane(1, &[View::Game], 0),
                split(
                    4,
                    Direction::Down,
                    pane(2, &[file("a")], 0),
                    pane(3, &[], 0),
                ),
            ),
            1,
        );
        assert!(apply(&mut l, LayoutOp::ClosePane { pane: 3 }));
        assert_eq!(l, two_with_focus(1));
    }

    fn two_with_focus(focus: u64) -> Layout {
        let mut l = layout(
            split(
                5,
                Direction::Right,
                pane(1, &[View::Game], 0),
                pane(2, &[file("a")], 0),
            ),
            focus,
        );
        l.normalize();
        l
    }

    #[test]
    fn resize_sets_a_splits_ratio_within_bounds() {
        let mut l = two();
        let ratio = |l: &Layout| match &l.root {
            Some(Node::Split { ratio, .. }) => *ratio,
            other => panic!("{other:?}"),
        };
        assert!(apply(
            &mut l,
            LayoutOp::Resize {
                split: 3,
                ratio: 620
            }
        ));
        assert_eq!(ratio(&l), 620);
        assert!(apply(&mut l, LayoutOp::Resize { split: 3, ratio: 0 }));
        assert_eq!(ratio(&l), RATIO_MIN);
        assert!(apply(
            &mut l,
            LayoutOp::Resize {
                split: 3,
                ratio: 5000
            }
        ));
        assert_eq!(ratio(&l), RATIO_MAX);
        // A pane's id or a gone split: nothing.
        for split in [1, 9] {
            assert!(!apply(&mut l, LayoutOp::Resize { split, ratio: 500 }));
        }
    }

    #[test]
    fn focus_and_reset() {
        let mut l = two();
        assert!(apply(&mut l, LayoutOp::Focus { pane: 1 }));
        assert_eq!(l.focus, Some(1));
        for pane in [1, 3, 9] {
            assert!(!apply(&mut l, LayoutOp::Focus { pane }));
        }
        assert!(apply(&mut l, LayoutOp::Reset));
        assert_eq!(l, Layout::initial());
        assert!(!apply(&mut l, LayoutOp::Reset));
    }

    #[test]
    fn normalize_restores_every_invariant() {
        // Two Game tabs, a repeated id, an active past the end, a bad ratio
        // and a focus on no pane, as a hand-edited file might hold.
        let mut l = Layout {
            root: Some(Node::Split {
                id: 1,
                direction: Direction::Down,
                ratio: 1,
                first: Box::new(pane(1, &[View::Game, file("a"), View::Game], 7)),
                second: Box::new(pane(2, &[View::Game, file("a"), file("b")], 0)),
            }),
            focus: Some(42),
            game: GamePlacement::default(),
            agent: Default::default(),
        };
        l.normalize();
        assert_eq!(
            l,
            Layout {
                root: Some(Node::Split {
                    id: 3,
                    direction: Direction::Down,
                    ratio: RATIO_MIN,
                    first: Box::new(pane(1, &[View::Game, file("a")], 1)),
                    second: Box::new(pane(2, &[file("b")], 0)),
                }),
                focus: Some(1),
                game: GamePlacement::default(),
                agent: Default::default(),
            }
        );
        check(&l);
        // An empty canvas keeps no focus.
        let mut empty = Layout {
            root: None,
            focus: Some(1),
            game: GamePlacement::default(),
            agent: Default::default(),
        };
        empty.normalize();
        assert_eq!(empty.focus, None);
    }

    #[test]
    fn retain_drops_stale_tabs_and_the_panes_they_empty() {
        let mut l = layout(
            split(
                5,
                Direction::Right,
                pane(1, &[View::Game, View::Task { task: 9 }], 1),
                split(
                    4,
                    Direction::Down,
                    pane(2, &[View::Changes { task: 9 }], 0),
                    pane(3, &[], 0),
                ),
            ),
            2,
        );
        l.retain(|v| !matches!(v, View::Task { task: 9 } | View::Changes { task: 9 }));
        // Pane 3 was empty already, so it stays; pane 2 held only stale tabs.
        assert_eq!(
            l,
            layout(
                split(
                    5,
                    Direction::Right,
                    pane(1, &[View::Game], 0),
                    pane(3, &[], 0)
                ),
                3
            )
        );
        check(&l);
    }

    #[test]
    fn dropping_an_earlier_tab_keeps_the_shown_one_shown() {
        let mut l = layout(pane(1, &[file("a"), View::Game, file("b")], 1), 1);
        l.retain(|v| *v != file("a"));
        assert_eq!(l, layout(pane(1, &[View::Game, file("b")], 0), 1));
        // A repeat before the shown tab, dropped by `normalize`, likewise.
        let mut l = layout(pane(1, &[file("a"), file("a"), file("b")], 2), 1);
        l.normalize();
        assert_eq!(l, layout(pane(1, &[file("a"), file("b")], 1), 1));
        // The shown tab itself dropped: the one now at its place shows.
        let mut l = layout(pane(1, &[file("a"), file("b"), file("c")], 1), 1);
        l.retain(|v| *v != file("b"));
        assert_eq!(l, layout(pane(1, &[file("a"), file("c")], 1), 1));
    }

    /// Every invariant `normalize` promises, checked directly.
    fn check(l: &Layout) {
        let panes = l.panes();
        let mut views = HashSet::new();
        let mut ids = HashSet::new();
        for p in &panes {
            assert!(ids.insert(p.id), "repeated id {}: {l:?}", p.id);
            for v in &p.tabs {
                assert!(views.insert(v.clone()), "{v:?} open twice: {l:?}");
            }
            assert!(p.active < p.tabs.len().max(1), "{l:?}");
        }
        fn splits(node: &Node, ids: &mut HashSet<u64>) {
            if let Node::Split {
                id,
                ratio,
                first,
                second,
                ..
            } = node
            {
                assert!(ids.insert(*id), "repeated id {id}");
                assert!((RATIO_MIN..=RATIO_MAX).contains(ratio));
                splits(first, ids);
                splits(second, ids);
            }
        }
        if let Some(root) = &l.root {
            splits(root, &mut ids);
        }
        // Agent marks name open tabs only.
        assert!(l.agent.tabs.iter().all(|t| l.find(t).is_some()), "{l:?}");
        match l.focus {
            Some(f) => assert!(panes.iter().any(|p| p.id == f), "{l:?}"),
            None => assert!(l.root.is_none(), "{l:?}"),
        }
        // A Game tab only in a pane placement.
        if l.game.placement != Placement::Pane {
            assert_eq!(l.find(&View::Game), None, "{l:?}");
        }
        assert!((MINI_MIN..=MINI_MAX).contains(&l.game.mini.width));
    }

    /// Long random op sequences (a fixed-seed generator) keep the invariants.
    #[test]
    fn random_ops_keep_the_invariants() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let views = [
            View::Game,
            file("a"),
            file("b"),
            file("c"),
            View::Changes { task: 1 },
            View::Task { task: 1 },
        ];
        for _ in 0..200 {
            let mut l = Layout::initial();
            for _ in 0..60 {
                let view = views[next(views.len() as u64) as usize].clone();
                let id = next(12);
                // The agent's moves mixed in, each checked never to take the wheel.
                match next(15) {
                    11 | 12 => {
                        agent(&mut l, view, next(2) == 0);
                        check(&l);
                        continue;
                    }
                    13 => {
                        apply(&mut l, LayoutOp::Restore);
                        check(&l);
                        continue;
                    }
                    14 => {
                        apply(&mut l, LayoutOp::CloseAgentTabs);
                        check(&l);
                        continue;
                    }
                    _ => {}
                }
                let op = match next(11) {
                    0 => open(view, Target::Focused),
                    1 => open(view, Target::Side),
                    2 => open(view, Target::Pane { pane: id }),
                    3 => LayoutOp::Move {
                        view,
                        pane: id,
                        index: next(5) as usize,
                    },
                    4 => LayoutOp::Close { view },
                    5 => LayoutOp::Split {
                        pane: id,
                        direction: if next(2) == 0 {
                            Direction::Right
                        } else {
                            Direction::Down
                        },
                    },
                    6 => LayoutOp::Focus { pane: id },
                    7 => LayoutOp::Resize {
                        split: id,
                        ratio: next(1200) as u16,
                    },
                    8 => LayoutOp::Place {
                        placement: [Placement::Pane, Placement::Mini, Placement::Window]
                            [next(3) as usize],
                    },
                    9 => LayoutOp::SetMini {
                        mini: Mini {
                            corner: Corner::TopLeft,
                            width: next(2000) as u32,
                        },
                    },
                    _ => LayoutOp::ClosePane { pane: id },
                };
                apply(&mut l, op);
                check(&l);
            }
        }
    }

    fn place(placement: Placement) -> LayoutOp {
        LayoutOp::Place { placement }
    }

    #[test]
    fn mini_takes_the_game_tab_out_and_dock_puts_it_back() {
        let mut l = layout(
            split(
                3,
                Direction::Right,
                pane(1, &[file("b.fun"), View::Game, file("c.fun")], 1),
                pane(2, &[file("a.fun")], 0),
            ),
            1,
        );
        let before = l.clone();
        assert!(apply(&mut l, place(Placement::Mini)));
        assert_eq!(l.find(&View::Game), None);
        assert_eq!(l.game.dock, Some(Dock { pane: 1, index: 1 }));
        // The tab after it shows.
        assert_eq!(l.panes()[0].tabs[l.panes()[0].active], file("c.fun"));
        // Panes and splits change underneath; the game stays out.
        assert!(apply(&mut l, open(file("d.fun"), Target::Focused)));
        assert!(apply(
            &mut l,
            LayoutOp::Split {
                pane: 2,
                direction: Direction::Down
            }
        ));
        assert_eq!(l.find(&View::Game), None);
        assert_eq!(l.game.placement, Placement::Mini);
        // Docked: the same pane and place, shown and focused.
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l.find(&View::Game), Some((1, 1)));
        assert_eq!(l.focus, Some(1));
        assert_eq!(l.game.dock, None);
        assert_eq!(
            l.panes()[0].tabs,
            before.panes()[0].tabs[..2]
                .iter()
                .cloned()
                .chain([file("c.fun"), file("d.fun")])
                .collect::<Vec<_>>()
        );
        // The same placement again: nothing.
        assert!(!apply(&mut l, place(Placement::Pane)));
    }

    #[test]
    fn docking_to_a_pane_that_is_gone_uses_the_focused_one() {
        // The Game tab alone in its pane: the pane goes with it.
        let mut l = two();
        assert!(apply(&mut l, place(Placement::Mini)));
        assert_eq!(
            l,
            Layout {
                game: l.game,
                ..layout(pane(2, &[file("a.fun")], 0), 2)
            }
        );
        // Its pane is gone, so is the dock.
        assert_eq!(l.game.dock, None);
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l.find(&View::Game), Some((2, 1)));
        // A pane made later with the gone pane's id is not taken for it.
        let mut l = layout(
            split(
                5,
                Direction::Right,
                pane(1, &[file("a.fun")], 0),
                pane(3, &[View::Game], 0),
            ),
            3,
        );
        assert!(apply(&mut l, place(Placement::Mini)));
        assert!(apply(
            &mut l,
            LayoutOp::Split {
                pane: 1,
                direction: Direction::Down
            }
        ));
        assert_eq!(l.panes()[1].id, 3);
        assert!(apply(&mut l, LayoutOp::Focus { pane: 1 }));
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l.find(&View::Game), Some((1, 1)));
        // And on an empty canvas, a new pane.
        let mut l = Layout::initial();
        assert!(apply(&mut l, place(Placement::Mini)));
        assert_eq!(l.root, None);
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l, Layout::initial());
    }

    #[test]
    fn own_window_has_no_game_tab_and_comes_back_like_mini() {
        let mut l = two();
        assert!(apply(&mut l, place(Placement::Window)));
        assert_eq!(l.find(&View::Game), None);
        // Window to Mini keeps the dock from the pane it left.
        let mut l = layout(pane(1, &[View::Game, file("a.fun")], 1), 1);
        assert!(apply(&mut l, place(Placement::Window)));
        assert!(apply(&mut l, place(Placement::Mini)));
        assert_eq!(l.game.dock, Some(Dock { pane: 1, index: 0 }));
        assert!(apply(&mut l, place(Placement::Window)));
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l.find(&View::Game), Some((1, 0)));
    }

    #[test]
    fn opening_the_game_brings_it_into_a_pane_at_the_target() {
        for from in [Placement::Mini, Placement::Window] {
            let mut l = two();
            assert!(apply(&mut l, place(from)));
            assert!(apply(&mut l, open(View::Game, Target::Pane { pane: 2 })));
            assert_eq!(l.game.placement, Placement::Pane);
            assert_eq!(l.game.dock, None);
            assert_eq!(l.find(&View::Game), Some((2, 1)));
            // Into a pane that is gone: the focused one, never no tab.
            let mut l = two();
            assert!(apply(&mut l, place(from)));
            assert!(apply(&mut l, open(View::Game, Target::Pane { pane: 9 })));
            assert_eq!(l.game.placement, Placement::Pane);
            assert_eq!(l.find(&View::Game), Some((2, 1)));
        }
    }

    #[test]
    fn reset_keeps_the_placement() {
        let mut l = two();
        assert!(apply(&mut l, place(Placement::Mini)));
        assert!(apply(&mut l, LayoutOp::Reset));
        // The initial pane held only the game, which is in Mini: an empty canvas.
        assert_eq!(l.root, None);
        assert_eq!(l.game.placement, Placement::Mini);
        assert!(apply(&mut l, place(Placement::Pane)));
        assert_eq!(l, Layout::initial());
    }

    #[test]
    fn the_mini_width_is_clamped() {
        let mut l = Layout::initial();
        for (width, kept) in [(10, MINI_MIN), (480, 480), (99_999, MINI_MAX)] {
            let mini = Mini {
                corner: Corner::TopLeft,
                width,
            };
            apply(&mut l, LayoutOp::SetMini { mini });
            assert_eq!(
                l.game.mini,
                Mini {
                    width: kept,
                    ..mini
                }
            );
        }
        // Not a placement change: the Game tab stays.
        assert_eq!(l.find(&View::Game), Some((1, 0)));
    }

    #[test]
    fn layouts_saved_before_placements_load_as_a_pane() {
        // What `layout.json` held before placements: the root and focus only.
        let old = r#"{"root":{"kind":"pane","id":1,"tabs":[{"kind":"game"},{"kind":"file","path":"game.fun"}],"active":1},"focus":1}"#;
        let l: Layout = serde_json::from_str(old).unwrap();
        assert_eq!(l.game, GamePlacement::default());
        assert_eq!(l.game.placement, Placement::Pane);
        assert_eq!(
            l.game.mini,
            Mini {
                corner: Corner::BottomRight,
                width: 360
            }
        );
        assert_eq!(l.find(&View::Game), Some((1, 0)));
        // A saved Mini with a Game tab (edited by hand) loses the tab, keeping its place.
        let mut mini: Layout = serde_json::from_str(&old.replace(
            r#""focus":1}"#,
            r#""focus":1,"game":{"placement":"mini","mini":{"corner":"top_left","width":5},"dock":null}}"#,
        ))
        .unwrap();
        mini.normalize();
        assert_eq!(mini.find(&View::Game), None);
        assert_eq!(mini.game.dock, Some(Dock { pane: 1, index: 0 }));
        assert_eq!(mini.game.mini.width, MINI_MIN);
    }

    #[test]
    fn layouts_round_trip_through_json() {
        let l = two();
        let json = serde_json::to_value(&l).unwrap();
        assert_eq!(json["root"]["kind"], "split");
        assert_eq!(json["root"]["direction"], "right");
        assert_eq!(json["root"]["second"]["kind"], "pane");
        assert_eq!(
            json["root"]["second"]["tabs"][0],
            serde_json::json!({"kind": "file", "path": "a.fun"})
        );
        assert_eq!(serde_json::from_value::<Layout>(json).unwrap(), l);
        let op: LayoutOp = serde_json::from_str(
            r#"{"type":"open","view":{"kind":"changes","task":3},"target":{"kind":"side"}}"#,
        )
        .unwrap();
        assert_eq!(op, open(View::Changes { task: 3 }, Target::Side));
    }

    /// The user's tab in `l`'s focused pane, and the focus: what an agent
    /// change must never move.
    fn wheel(l: &Layout) -> (Option<u64>, Option<View>) {
        let shown = l
            .panes()
            .into_iter()
            .find(|p| Some(p.id) == l.focus)
            .and_then(|p| p.tabs.get(p.active).cloned());
        (l.focus, shown)
    }

    fn agent(l: &mut Layout, view: View, stacked: bool) -> Placed {
        let before = wheel(l);
        let tabs = l.panes().iter().map(|p| p.tabs.len()).sum::<usize>();
        let placed = l.agent_show(&view, stacked, [1, 0], ok).unwrap();
        // On an empty canvas there is no focus to keep: its first pane takes
        // it; in an empty pane, no tab to keep.
        if before.0.is_some() {
            assert_eq!(l.focus, before.0, "the agent moved the user's focus");
        }
        if before.1.is_some() {
            assert_eq!(wheel(l).1, before.1, "the agent changed the user's tab");
        }
        let after = l.panes().iter().map(|p| p.tabs.len()).sum::<usize>();
        assert!(after >= tabs, "the agent closed a tab");
        placed
    }

    #[test]
    fn with_one_pane_the_agent_splits_right_and_leaves_the_focus() {
        let mut l = layout(pane(1, &[View::Game, file("a.fun")], 1), 1);
        let user = l.clone();
        assert_eq!(agent(&mut l, file("b.fun"), false), Placed::Opened);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[game, *file:a.fun] | #3[*file:b.fun])"
        );
        assert_eq!(l.agent.tabs, [file("b.fun")]);
        let mut restore = user;
        restore.agent = AgentLayout::default();
        assert_eq!(l.agent.restore.as_deref(), Some(&restore));
    }

    #[test]
    fn stacked_with_one_pane_it_opens_behind_the_users_tab() {
        let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
        assert_eq!(agent(&mut l, file("b.fun"), true), Placed::Behind);
        assert_eq!(crate::print_layout(&l), ">#1[*file:a.fun, file:b.fun]");
        assert_eq!(l.agent.tabs, [file("b.fun")]);
        // A view already behind the user's tab stays there.
        let mut l = layout(pane(1, &[file("a.fun"), file("b.fun")], 0), 1);
        assert_eq!(agent(&mut l, file("b.fun"), true), Placed::Behind);
        assert_eq!(crate::print_layout(&l), ">#1[*file:a.fun, file:b.fun]");
        assert!(l.agent.tabs.is_empty(), "the user's tab is not the agent's");
    }

    #[test]
    fn stacked_with_two_panes_it_uses_the_other_one() {
        let mut l = layout(
            split(
                3,
                Direction::Right,
                pane(1, &[file("a.fun")], 0),
                pane(2, &[View::Game], 0),
            ),
            1,
        );
        assert_eq!(agent(&mut l, file("b.fun"), true), Placed::Opened);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #2[game, *file:b.fun])"
        );
    }

    #[test]
    fn the_agent_uses_the_pane_after_the_focused_one_else_before() {
        let two = |focus| {
            layout(
                split(
                    3,
                    Direction::Right,
                    pane(1, &[file("a.fun")], 0),
                    pane(2, &[View::Game], 0),
                ),
                focus,
            )
        };
        let mut l = two(1);
        agent(&mut l, file("c.fun"), false);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #2[game, *file:c.fun])"
        );
        let mut l = two(2);
        agent(&mut l, file("c.fun"), false);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (#1[file:a.fun, *file:c.fun] | >#2[*game])"
        );
    }

    #[test]
    fn an_open_view_is_shown_where_it_is_or_brought_out_from_behind_the_users_tab() {
        // In the other pane, behind another tab: shown there.
        let mut l = layout(
            split(
                3,
                Direction::Right,
                pane(1, &[file("a.fun")], 0),
                pane(2, &[View::Game, file("b.fun")], 0),
            ),
            1,
        );
        assert_eq!(agent(&mut l, file("b.fun"), false), Placed::Shown);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #2[game, *file:b.fun])"
        );
        assert!(l.agent.tabs.is_empty());
        // The tab the user is looking at: nothing changes, nothing to restore.
        let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
        let user = l.clone();
        assert_eq!(agent(&mut l, file("a.fun"), false), Placed::InView);
        assert_eq!(l, user);
        // Behind the user's tab: out to a pane beside it.
        let mut l = layout(pane(1, &[file("a.fun"), View::Game], 0), 1);
        assert_eq!(agent(&mut l, View::Game, false), Placed::Shown);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #3[*game])"
        );
    }

    #[test]
    fn a_game_in_mini_or_its_own_window_is_left_where_it_is() {
        for placement in [Placement::Mini, Placement::Window] {
            let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
            l.game.placement = placement;
            let user = l.clone();
            assert_eq!(
                agent(&mut l, View::Game, false),
                Placed::GameElsewhere(placement)
            );
            assert_eq!(l, user);
        }
        let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
        assert_eq!(agent(&mut l, View::Game, false), Placed::Opened);
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #3[*game])"
        );
    }

    #[test]
    fn on_an_empty_canvas_the_agent_opens_a_pane() {
        let mut l = Layout {
            root: None,
            focus: None,
            game: GamePlacement::default(),
            agent: Default::default(),
        };
        assert_eq!(
            l.agent_show(&file("a.fun"), false, [1, 0], ok),
            Ok(Placed::Opened)
        );
        assert_eq!(crate::print_layout(&l), ">#1[*file:a.fun]");
    }

    #[test]
    fn a_refused_view_changes_nothing() {
        let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
        let user = l.clone();
        let refuse = |_: &View| Err("not listed".to_string());
        assert_eq!(
            l.agent_show(&file("x.fun"), false, [1, 0], refuse),
            Err("not listed".to_string())
        );
        assert_eq!(l, user);
        // An open view needs no check.
        assert_eq!(
            l.agent_show(&file("a.fun"), false, [1, 0], refuse),
            Ok(Placed::InView)
        );
    }

    #[test]
    fn restore_puts_back_the_layout_from_before_the_agents_first_change() {
        let mut l = layout(pane(1, &[View::Game, file("a.fun")], 1), 1);
        let user = l.clone();
        agent(&mut l, file("b.fun"), false);
        agent(&mut l, file("c.fun"), false);
        agent(&mut l, View::Game, false);
        // The snapshot is still the one from before the first change.
        assert_eq!(l.agent.restore.as_ref().unwrap().root, user.root);
        assert!(apply(
            &mut l,
            LayoutOp::Place {
                placement: Placement::Mini
            }
        ));
        assert!(apply(&mut l, LayoutOp::Restore));
        // The Game tab left with the placement, which the restore keeps.
        assert_eq!(crate::print_layout(&l), ">#1[*file:a.fun]");
        assert_eq!(l.game.placement, Placement::Mini);
        assert_eq!(l.agent, AgentLayout::default());
        assert!(
            !apply(&mut l, LayoutOp::Restore),
            "restoring twice does nothing"
        );
        // Undoing the agent's change by hand leaves nothing to restore.
        let mut l = layout(pane(1, &[file("a.fun")], 0), 1);
        agent(&mut l, file("b.fun"), false);
        assert!(apply(
            &mut l,
            LayoutOp::Close {
                view: file("b.fun")
            }
        ));
        assert!(apply(&mut l, LayoutOp::ClosePane { pane: 3 }) || l.panes().len() == 1);
        assert_eq!(l.agent.restore, None, "{}", crate::print_layout(&l));
    }

    #[test]
    fn close_agent_tabs_closes_only_the_agents() {
        let mut l = layout(
            split(
                3,
                Direction::Right,
                pane(1, &[file("a.fun")], 0),
                pane(2, &[View::Game], 0),
            ),
            1,
        );
        agent(&mut l, file("b.fun"), false);
        agent(&mut l, file("c.fun"), false);
        assert_eq!(l.agent.tabs, [file("b.fun"), file("c.fun")]);
        assert!(apply(&mut l, LayoutOp::CloseAgentTabs));
        assert_eq!(
            crate::print_layout(&l),
            "right 50% (>#1[*file:a.fun] | #2[*game])"
        );
        assert!(l.agent.tabs.is_empty());
        // A mark goes with its tab: closed by the user, it is forgotten.
        agent(&mut l, file("b.fun"), false);
        assert!(apply(
            &mut l,
            LayoutOp::Close {
                view: file("b.fun")
            }
        ));
        assert!(l.agent.tabs.is_empty());
    }

    #[test]
    fn the_setting_survives_reset_and_old_layouts_have_it_on() {
        let mut l = Layout::initial();
        assert!(l.agent.arranges);
        assert!(apply(&mut l, LayoutOp::AgentArranges { on: false }));
        assert!(apply(&mut l, open(file("a.fun"), Target::Side)));
        assert!(apply(&mut l, LayoutOp::Reset));
        assert!(!l.agent.arranges);
        let old: Layout = serde_json::from_str(r#"{"root": null, "focus": null}"#).unwrap();
        assert_eq!(old.agent, AgentLayout::default());
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), l);
    }
}
