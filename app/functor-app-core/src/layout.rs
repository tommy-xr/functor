//! The canvas layout (D20): a tree of splits whose leaves are panes of tabs,
//! each tab a [`View`]. A view is open at most once in the whole layout, so a
//! view names its tab. [`Layout::apply`] is the one reducer and
//! [`Layout::normalize`] the one place the invariants are enforced.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

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
    /// Back to [`Layout::initial`].
    Reset,
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
                valid(&view)?;
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
            LayoutOp::Reset => *self = Self::initial(),
        }
        self.normalize();
        Ok(*self != before)
    }

    /// Drops the tabs `keep` refuses (a deleted file, a removed task); a pane
    /// that leaves empty is removed.
    pub fn retain(&mut self, keep: impl Fn(&View) -> bool) {
        let emptied: Vec<u64> = self
            .panes()
            .into_iter()
            .filter(|p| !p.tabs.is_empty() && !p.tabs.iter().any(&keep))
            .map(|p| p.id)
            .collect();
        for id in emptied {
            self.remove_pane(id);
        }
        self.for_each_pane(|p| p.retain_tabs(&mut |t| keep(t)));
        self.normalize();
    }

    /// The invariants, for a layout from any source (disk included): each
    /// view open once (so at most one Game tab), unique ids, each pane's
    /// `active` in range, ratios clamped, and the focus on a pane.
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
        let panes = self.panes();
        if !panes.iter().any(|p| Some(p.id) == self.focus) {
            self.focus = panes.first().map(|p| p.id);
        }
    }

    fn open(&mut self, view: View, target: Target) {
        if let Some((pane, i)) = self.find(&view) {
            self.show(pane, i);
            return;
        }
        let pane = match target {
            Target::Pane { pane } if self.pane_mut(pane).is_some() => pane,
            Target::Pane { .. } => return,
            Target::Focused => match self.focus {
                Some(pane) => pane,
                None => self.add_root(),
            },
            Target::Side => match self.focus {
                Some(focus) => self
                    .neighbour(focus)
                    .or_else(|| self.split(focus, Direction::Right))
                    .expect("the focused pane exists"),
                None => self.add_root(),
            },
        };
        let p = self.pane_mut(pane).expect("a pane of the layout");
        p.tabs.push(view);
        let last = p.tabs.len() - 1;
        self.show(pane, last);
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
        for target in [Target::Focused, Target::Side, Target::Pane { pane: 2 }] {
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
                focus: None
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
            }
        );
        check(&l);
        // An empty canvas keeps no focus.
        let mut empty = Layout {
            root: None,
            focus: Some(1),
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
        match l.focus {
            Some(f) => assert!(panes.iter().any(|p| p.id == f), "{l:?}"),
            None => assert!(l.root.is_none(), "{l:?}"),
        }
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
                let op = match next(9) {
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
                    _ => LayoutOp::ClosePane { pane: id },
                };
                apply(&mut l, op);
                check(&l);
            }
        }
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
}
