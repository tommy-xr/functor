//! Nav: weighted graph routing for agents (see `nav.funi`). A [`Graph`] is
//! immutable, built once from data; each [`Query`] carries the state that
//! changes (closed doors, link and node penalties), so a route depends only
//! on the graph and the query. Std-only, so it benchmarks standalone.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// A directed link between two nodes (list positions).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub from: u32,
    pub to: u32,
    /// Added to a route's cost; finite and ≥ 0.
    pub cost: f64,
    /// Who may use it: an agent whose bits share one with these.
    pub bits: u32,
    /// Closed to everyone while this door is in the query's `closed_doors`.
    pub door: Option<u32>,
}

pub struct Graph {
    at: Vec<[f32; 3]>,
    /// Links sorted by `from` (stable), so node n's are
    /// `links[first[n]..first[n + 1]]`.
    links: Vec<Link>,
    first: Vec<usize>,
    /// The least cost per unit length over every link, so `scale × distance`
    /// never overestimates and A* stays exact.
    scale: f64,
    revision: f64,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Nav.graph({} nodes, {} links, revision {})", self.at.len(), self.links.len(), self.revision)
    }
}

impl PartialEq for Graph {
    fn eq(&self, other: &Graph) -> bool {
        self.revision == other.revision
    }
}

/// A cost added while `now < until`; an infinite weight cuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Penalty {
    pub weight: f64,
    pub until: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub bits: u32,
    pub closed_doors: Vec<u32>,
    /// By directed node pair: every link from → to.
    pub links: Vec<((u32, u32), Penalty)>,
    /// Entering the node.
    pub nodes: Vec<(u32, Penalty)>,
    pub now: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Reached the node nearest `to`.
    Full,
    /// It can't be reached: ends at the reachable node nearest `to`.
    Partial,
    /// No node is nearer `to` than the start's (or the graph is empty).
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub outcome: Outcome,
    /// Start node to end node; empty when `Failed`.
    pub nodes: Vec<u32>,
    /// When the goal fell short only because of cuts: the latest `until`
    /// among the cuts on the route that lifting them opens.
    pub retry_at: Option<f64>,
}

/// Expansions between cancel checks.
const CANCEL_EVERY: usize = 1024;

impl Graph {
    /// Validate and index `links` over nodes at `at`. An error names the first
    /// bad link.
    pub fn new(at: Vec<[f32; 3]>, mut links: Vec<Link>) -> Result<Graph, String> {
        let n = at.len();
        for (i, link) in links.iter().enumerate() {
            for end in [link.from, link.to] {
                if end as usize >= n {
                    return Err(format!("link {i} names node {end}, but there are {n} nodes"));
                }
            }
            if !(link.cost.is_finite() && link.cost >= 0.0) {
                return Err(format!("link {i} costs {}; a cost must be finite and ≥ 0", link.cost));
            }
        }
        let mut scale = f64::INFINITY;
        for link in &links {
            let length = distance(at[link.from as usize], at[link.to as usize]);
            if length > 0.0 {
                scale = scale.min(link.cost / length);
            }
        }
        if !scale.is_finite() {
            scale = 0.0;
        }
        // Content identity, so a result can be matched to its graph.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&format!("{at:?}{links:?}"), &mut hasher);
        let revision = (std::hash::Hasher::finish(&hasher) & ((1 << 53) - 1)) as f64;
        links.sort_by_key(|link| link.from);
        let mut first = vec![0; n + 1];
        for link in &links {
            first[link.from as usize + 1] += 1;
        }
        for i in 0..n {
            first[i + 1] += first[i];
        }
        Ok(Graph { at, links, first, scale, revision })
    }

    pub fn revision(&self) -> f64 {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn at(&self, node: u32) -> [f32; 3] {
        self.at[node as usize]
    }

    /// The node nearest `p`; the lowest id on a tie.
    fn nearest(&self, p: [f32; 3]) -> Option<u32> {
        let mut best: Option<(f64, u32)> = None;
        for (i, at) in self.at.iter().enumerate() {
            let d = distance(*at, p);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, i as u32));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Route `query` from the node nearest `from` to the node nearest `to`,
    /// polling `cancelled` every 1,024 expansions: `None` once it says so.
    pub fn find(&self, query: &Query, cancelled: &dyn Fn() -> bool) -> Option<Route> {
        let failed = Route { outcome: Outcome::Failed, nodes: Vec::new(), retry_at: None };
        let (Some(start), Some(goal)) = (self.nearest(query.from), self.nearest(query.to)) else {
            return Some(failed);
        };
        let costs = Costs::of(query);
        let search = self.search(start, goal, query.bits, &costs, cancelled)?;
        if search.reached(goal) {
            let nodes = search.path(goal);
            return Some(Route { outcome: Outcome::Full, nodes, retry_at: None });
        }
        // Partial: the reachable node strictly nearer `to` than the start.
        let near = |n: u32| distance(self.at[n as usize], query.to);
        let mut best = (near(start), start);
        for &n in &search.settled {
            if near(n) < best.0 {
                best = (near(n), n);
            }
        }
        let retry_at = self.retry_at(start, goal, query.bits, costs, cancelled)?;
        if best.1 == start {
            return Some(Route { retry_at, ..failed });
        }
        Some(Route { outcome: Outcome::Partial, nodes: search.path(best.1), retry_at })
    }

    /// For a goal the cuts kept out of reach: re-route with only the doors,
    /// and wait for the latest cut on that route. `Some(None)` when there
    /// are no cuts or the goal is out of reach regardless.
    fn retry_at(
        &self,
        start: u32,
        goal: u32,
        bits: u32,
        costs: Costs,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Option<f64>> {
        let cut = |extra: &Extra| extra.cut_until;
        if !costs.links.values().chain(costs.nodes.values()).any(|e| cut(e).is_some()) {
            return Some(None);
        }
        let open = Costs { doors: costs.doors.clone(), ..Costs::default() };
        let search = self.search(start, goal, bits, &open, cancelled)?;
        if !search.reached(goal) {
            return Some(None);
        }
        let path = search.path(goal);
        let links = path.windows(2).filter_map(|w| costs.links.get(&(w[0], w[1])).and_then(cut));
        let nodes = path[1..].iter().filter_map(|n| costs.nodes.get(n).and_then(cut));
        Some(links.chain(nodes).reduce(f64::max))
    }

    /// A* from `start` toward `goal`, stopping when it settles; on a miss,
    /// every reachable node is settled. `None` when cancelled.
    fn search(
        &self,
        start: u32,
        goal: u32,
        bits: u32,
        costs: &Costs,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Search> {
        let n = self.at.len();
        let goal_at = self.at[goal as usize];
        let h = |node: u32| self.scale * distance(self.at[node as usize], goal_at);
        let mut g = vec![f64::INFINITY; n];
        let mut parent = vec![u32::MAX; n];
        let mut closed = vec![false; n];
        let mut settled = Vec::new();
        let mut open = BinaryHeap::new();
        g[start as usize] = 0.0;
        open.push(Open { f: h(start), node: start });
        while let Some(Open { node, .. }) = open.pop() {
            if std::mem::replace(&mut closed[node as usize], true) {
                continue;
            }
            settled.push(node);
            if settled.len() % CANCEL_EVERY == 0 && cancelled() {
                return None;
            }
            if node == goal {
                break;
            }
            let base = g[node as usize];
            for link in &self.links[self.first[node as usize]..self.first[node as usize + 1]] {
                if link.bits & bits == 0 || link.door.is_some_and(|d| costs.doors.contains(&d)) {
                    continue;
                }
                let extras = [costs.links.get(&(node, link.to)), costs.nodes.get(&link.to)];
                let extras = extras.into_iter().flatten();
                if extras.clone().any(|e| e.cut_until.is_some()) {
                    continue;
                }
                let extra: f64 = extras.map(|e| e.weight).sum();
                let next = base + link.cost + extra;
                if next < g[link.to as usize] {
                    g[link.to as usize] = next;
                    parent[link.to as usize] = node;
                    open.push(Open { f: next + h(link.to), node: link.to });
                }
            }
        }
        Some(Search { parent, settled })
    }
}

struct Search {
    parent: Vec<u32>,
    /// In settling order.
    settled: Vec<u32>,
}

impl Search {
    fn reached(&self, goal: u32) -> bool {
        self.settled.last() == Some(&goal)
    }

    fn path(&self, end: u32) -> Vec<u32> {
        let mut path = vec![end];
        while let Some(&p) = self.parent.get(*path.last().expect("non-empty") as usize) {
            if p == u32::MAX {
                break;
            }
            path.push(p);
        }
        path.reverse();
        path
    }
}

/// A query's penalties now in force, summed per link pair and node.
#[derive(Default)]
struct Costs {
    doors: HashSet<u32>,
    links: HashMap<(u32, u32), Extra>,
    nodes: HashMap<u32, Extra>,
}

#[derive(Default)]
struct Extra {
    weight: f64,
    /// Set when cut: the latest `until` of its cuts.
    cut_until: Option<f64>,
}

impl Extra {
    fn add(&mut self, p: Penalty) {
        if p.weight.is_infinite() {
            self.cut_until = Some(self.cut_until.map_or(p.until, |u| u.max(p.until)));
        } else {
            self.weight += p.weight;
        }
    }
}

impl Costs {
    fn of(query: &Query) -> Costs {
        let mut costs = Costs { doors: query.closed_doors.iter().copied().collect(), ..Costs::default() };
        let live = |p: &Penalty| query.now < p.until;
        for (pair, p) in query.links.iter().filter(|(_, p)| live(p)) {
            costs.links.entry(*pair).or_default().add(*p);
        }
        for (node, p) in query.nodes.iter().filter(|(_, p)| live(p)) {
            costs.nodes.entry(*node).or_default().add(*p);
        }
        costs
    }
}

/// An open-set entry: least `f` first, then the lowest node id, so equal
/// costs break the same way every run.
#[derive(PartialEq)]
struct Open {
    f: f64,
    node: u32,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Open) -> Ordering {
        other.f.total_cmp(&self.f).then(other.node.cmp(&self.node))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Open) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let d = |i: usize| f64::from(a[i]) - f64::from(b[i]);
    (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × w` grid (x right, z down, 1 m apart), 4-neighbour links both
    /// ways at cost 1, except where `wall(x, z)`.
    fn grid(w: u32, wall: impl Fn(u32, u32) -> bool) -> Graph {
        let at = (0..w * w).map(|i| [(i % w) as f32, 0.0, (i / w) as f32]).collect();
        let mut links = Vec::new();
        for i in 0..w * w {
            let (x, z) = (i % w, i / w);
            for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx >= w as i32 || nz >= w as i32 {
                    continue;
                }
                let j = nz as u32 * w + nx as u32;
                if !wall(x, z) && !wall(nx as u32, nz as u32) {
                    links.push(Link { from: i, to: j, cost: 1.0, bits: 1, door: None });
                }
            }
        }
        Graph::new(at, links).unwrap()
    }

    fn query(from: [f32; 3], to: [f32; 3]) -> Query {
        Query { from, to, bits: 1, closed_doors: Vec::new(), links: Vec::new(), nodes: Vec::new(), now: 0.0 }
    }

    fn never() -> bool {
        false
    }

    /// A line 0 — 1 — 2 with a costly direct link 0 → 2.
    fn line() -> Graph {
        let at = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let link = |from, to, cost| Link { from, to, cost, bits: 1, door: None };
        Graph::new(at, vec![link(0, 1, 1.0), link(1, 2, 1.0), link(0, 2, 5.0)]).unwrap()
    }

    #[test]
    fn the_cheapest_route_wins_over_the_fewest_links() {
        let route = line().find(&query([0.0; 3], [2.0, 0.0, 0.0]), &never).unwrap();
        assert_eq!(route, Route { outcome: Outcome::Full, nodes: vec![0, 1, 2], retry_at: None });
    }

    #[test]
    fn endpoints_snap_to_the_nearest_node() {
        let route = line().find(&query([-3.0, 1.0, 0.0], [1.4, 0.0, 0.2]), &never).unwrap();
        assert_eq!(route.nodes, vec![0, 1]);
    }

    #[test]
    fn a_wall_is_routed_around() {
        // A wall at x = 2 for z < 4 on a 5 × 5 grid: (0,0) → (4,0) goes down
        // and back up.
        let graph = grid(5, |x, z| x == 2 && z < 4);
        let route = graph.find(&query([0.0; 3], [4.0, 0.0, 0.0]), &never).unwrap();
        assert_eq!(route.outcome, Outcome::Full);
        assert_eq!(route.nodes.len(), 13);
        assert!(route.nodes.contains(&22), "through the gap at (2, 4)");
    }

    #[test]
    fn equal_costs_break_ties_by_node_id() {
        // Two equal routes around a square: 0 → 1 → 3 and 0 → 2 → 3.
        let at = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
        let link = |from, to| Link { from, to, cost: 1.0, bits: 1, door: None };
        let graph = Graph::new(at, vec![link(0, 2), link(0, 1), link(2, 3), link(1, 3)]).unwrap();
        let route = graph.find(&query([0.0; 3], [1.0, 0.0, 1.0]), &never).unwrap();
        assert_eq!(route.nodes, vec![0, 1, 3]);
    }

    #[test]
    fn bits_doors_and_penalties_come_from_the_query() {
        let graph = line();
        let mut q = query([0.0; 3], [2.0, 0.0, 0.0]);
        // A costly link: the direct one is now cheaper.
        q.links = vec![((1, 2), Penalty { weight: 10.0, until: 5.0 })];
        assert_eq!(graph.find(&q, &never).unwrap().nodes, vec![0, 2]);
        // Expired: back through 1.
        q.now = 5.0;
        assert_eq!(graph.find(&q, &never).unwrap().nodes, vec![0, 1, 2]);
        // A costly node, the same way.
        q.now = 0.0;
        q.links.clear();
        q.nodes = vec![(1, Penalty { weight: 4.0, until: 1.0 })];
        assert_eq!(graph.find(&q, &never).unwrap().nodes, vec![0, 2]);
        // No shared bit: nothing is usable.
        let mut q = query([0.0; 3], [2.0, 0.0, 0.0]);
        q.bits = 2;
        assert_eq!(graph.find(&q, &never).unwrap().outcome, Outcome::Failed);
        // A closed door.
        let at = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let door = Graph::new(at, vec![Link { from: 0, to: 1, cost: 1.0, bits: 1, door: Some(7) }]).unwrap();
        let mut q = query([0.0; 3], [1.0, 0.0, 0.0]);
        assert_eq!(door.find(&q, &never).unwrap().outcome, Outcome::Full);
        q.closed_doors = vec![7];
        assert_eq!(door.find(&q, &never).unwrap().outcome, Outcome::Failed);
    }

    #[test]
    fn an_unreachable_goal_routes_to_the_nearest_reachable_node() {
        // A wall across the grid at x = 2.
        let graph = grid(5, |x, _| x == 2);
        let route = graph.find(&query([0.0, 0.0, 2.0], [4.0, 0.0, 2.0]), &never).unwrap();
        assert_eq!(route.outcome, Outcome::Partial);
        assert_eq!(route.nodes, vec![10, 11]);
        assert_eq!(route.retry_at, None, "no cut to wait for");
        // Already as near as it gets.
        let route = graph.find(&query([1.0, 0.0, 2.0], [4.0, 0.0, 2.0]), &never).unwrap();
        assert_eq!(route, Route { outcome: Outcome::Failed, nodes: vec![], retry_at: None });
    }

    #[test]
    fn a_goal_cut_off_retries_when_the_latest_cut_on_its_route_lifts() {
        let graph = line();
        let mut q = query([0.0; 3], [2.0, 0.0, 0.0]);
        let cut = |until| Penalty { weight: f64::INFINITY, until };
        q.links = vec![((1, 2), cut(4.0)), ((0, 2), cut(9.0)), ((0, 2), cut(6.0))];
        let route = graph.find(&q, &never).unwrap();
        assert_eq!(route.outcome, Outcome::Partial);
        assert_eq!(route.nodes, vec![0, 1]);
        // Uncut, the cheapest route is 0 → 1 → 2: wait for 1 → 2's cut.
        assert_eq!(route.retry_at, Some(4.0));
        // A cut node is out of reach the same way.
        q.links.clear();
        q.nodes = vec![(2, cut(3.0))];
        assert_eq!(graph.find(&q, &never).unwrap().retry_at, Some(3.0));
        // A cut that has lifted is no cut.
        q.now = 3.0;
        assert_eq!(graph.find(&q, &never).unwrap().outcome, Outcome::Full);
    }

    #[test]
    fn a_cancelled_search_stops_within_a_thousand_expansions() {
        let graph = grid(100, |_, _| false);
        let q = query([0.0; 3], [99.0, 0.0, 99.0]);
        let polls = std::cell::Cell::new(0);
        let cancel = || {
            polls.set(polls.get() + 1);
            true
        };
        assert_eq!(graph.find(&q, &cancel), None);
        assert_eq!(polls.get(), 1, "the first poll, after 1,024 of 10,000 nodes, stops it");
        assert_eq!(graph.find(&q, &never).unwrap().nodes.len(), 199);
    }

    #[test]
    fn bad_links_are_refused() {
        let at = vec![[0.0; 3]; 2];
        let link = |to, cost| Link { from: 0, to, cost, bits: 1, door: None };
        assert_eq!(
            Graph::new(at.clone(), vec![link(1, 1.0), link(2, 1.0)]).unwrap_err(),
            "link 1 names node 2, but there are 2 nodes"
        );
        assert!(Graph::new(at.clone(), vec![link(1, -1.0)]).unwrap_err().contains("≥ 0"));
        assert!(Graph::new(at, vec![link(1, f64::NAN)]).is_err());
    }

    #[test]
    fn the_revision_is_the_content() {
        assert_eq!(line().revision(), line().revision());
        assert_ne!(line().revision(), grid(2, |_, _| false).revision());
        assert_eq!(line().revision().fract(), 0.0);
    }

    /// `rustc -O --edition 2021 --test src/nav.rs -o /tmp/nav && /tmp/nav
    /// --ignored --nocapture`: A* corner to corner on a 45 × 45 grid (2,025
    /// nodes), the interpreter benchmark's graph.
    #[test]
    #[ignore]
    fn bench_corner_to_corner_on_2k_nodes() {
        let graph = grid(45, |_, _| false);
        let q = query([0.0; 3], [44.0, 0.0, 44.0]);
        let runs = 200;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            assert_eq!(graph.find(&q, &never).unwrap().nodes.len(), 89);
        }
        let open = started.elapsed() / runs;
        // With a wall the search has to fill most of the grid.
        let walled = grid(45, |x, z| x == 22 && z < 44);
        let started = std::time::Instant::now();
        for _ in 0..runs {
            assert_eq!(walled.find(&q, &never).unwrap().outcome, Outcome::Full);
        }
        println!("2,025 nodes: open {open:?}, walled {:?} per query", started.elapsed() / runs);
    }
}
