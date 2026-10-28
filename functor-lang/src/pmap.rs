//! A persistent ordered map: a B+tree of `Rc` nodes with path copying.
//!
//! Cloning a [`PMap`] is O(1); an edit copies only the nodes on its root-to-
//! leaf path (`Rc::make_mut`: shared nodes are cloned, unique ones edited in
//! place), so old versions stay valid and share every untouched node.
//!
//! Key order comes from a caller-supplied, fallible comparison so a bounded
//! evaluator can charge (and refuse) each comparison. Edits are two-phase:
//! [`PMap::search`] does every comparison and records the path; the
//! infallible `*_at` edit then replays it. A refused comparison therefore
//! never leaves a half-edited map.
//!
//! Edits return work units: the summed final size (entries or children) of
//! every node they write. The count is the same whether a node was cloned or
//! edited in place, so fuel never depends on reference counts. A map that
//! fits one leaf charges exactly what a flat sorted vector would.

use std::cmp::Ordering;
use std::rc::Rc;

/// Max entries per leaf and children per internal node.
pub const CAP: usize = 16;
/// Min fill of a non-root node.
const MIN: usize = CAP / 2;

#[derive(Clone)]
enum Node<K, V> {
    Leaf(Vec<(K, V)>),
    /// `keys[i]` separates `children[i]` (all keys below it) from
    /// `children[i + 1]` (all keys at or above it). A separator can outlive
    /// its removed key until the node is next rewritten.
    Internal {
        keys: Vec<K>,
        children: Vec<Rc<Node<K, V>>>,
    },
}

impl<K, V> Node<K, V> {
    /// Entries in a leaf, children in an internal node.
    fn len(&self) -> usize {
        match self {
            Node::Leaf(entries) => entries.len(),
            Node::Internal { children, .. } => children.len(),
        }
    }
}

/// A persistent map ordered by the comparison its callers supply.
pub struct PMap<K, V> {
    root: Option<Rc<Node<K, V>>>,
    len: usize,
}

impl<K, V> Clone for PMap<K, V> {
    fn clone(&self) -> Self {
        PMap {
            root: self.root.clone(),
            len: self.len,
        }
    }
}

impl<K, V> Default for PMap<K, V> {
    fn default() -> Self {
        PMap { root: None, len: 0 }
    }
}

/// Where a key is (or would be): the child index taken at each internal
/// level, then the leaf slot. Valid only for the map it was searched in,
/// until that map is next edited.
pub struct Search {
    path: Vec<usize>,
    slot: Result<usize, usize>,
    /// Summed size of the nodes on the path.
    path_units: u64,
    /// The searched map's (root, len), to catch a stale or foreign search.
    origin: (*const (), usize),
}

impl Search {
    pub fn found(&self) -> bool {
        self.slot.is_ok()
    }

    /// Upper bound on [`PMap::insert_at`]'s units: each path node grows by
    /// at most one slot, plus a new two-child root. Lets callers refuse an
    /// edit before it allocates.
    pub fn insert_units_ceiling(&self) -> u64 {
        let depth = self.path.len() as u64 + 1;
        self.path_units.saturating_add(depth).saturating_add(2)
    }

    /// Upper bound on [`PMap::remove_at`]'s units: path nodes only shrink,
    /// plus one borrowed-from sibling (< `CAP`) per internal level.
    pub fn remove_units_ceiling(&self) -> u64 {
        let levels = self.path.len() as u64;
        self.path_units
            .saturating_add(levels.saturating_mul(CAP as u64 - 1))
    }
}

/// Binary search a leaf, exactly as over a flat sorted vector.
/// `cmp(stored)` orders a stored key against the probe.
fn search_leaf<K, V, E>(
    entries: &[(K, V)],
    cmp: &mut impl FnMut(&K) -> Result<Ordering, E>,
) -> Result<Result<usize, usize>, E> {
    let mut left = 0usize;
    let mut right = entries.len();
    while left < right {
        let mid = left + (right - left) / 2;
        match cmp(&entries[mid].0)? {
            Ordering::Less => left = mid + 1,
            Ordering::Greater => right = mid,
            Ordering::Equal => return Ok(Ok(mid)),
        }
    }
    Ok(Err(left))
}

/// The child to descend into: the number of separators at or below the probe.
fn search_internal<K, E>(
    keys: &[K],
    cmp: &mut impl FnMut(&K) -> Result<Ordering, E>,
) -> Result<usize, E> {
    let mut left = 0usize;
    let mut right = keys.len();
    while left < right {
        let mid = left + (right - left) / 2;
        match cmp(&keys[mid])? {
            Ordering::Greater => right = mid,
            Ordering::Less | Ordering::Equal => left = mid + 1,
        }
    }
    Ok(left)
}

impl<K: Clone, V: Clone> PMap<K, V> {
    pub fn new() -> Self {
        PMap::default()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Build from entries already strictly sorted by key. Leaves and
    /// internal nodes are packed evenly (each at least half full). Returns
    /// the internal nodes' summed size as work units; leaves take ownership
    /// of the entries, so a single-leaf map costs nothing extra.
    pub fn from_sorted(entries: Vec<(K, V)>) -> (Self, u64) {
        let len = entries.len();
        if len == 0 {
            return (PMap::default(), 0);
        }
        // Each level is (smallest key under the node, node).
        let mut level: Vec<(K, Rc<Node<K, V>>)> = even_chunks(entries)
            .into_iter()
            .map(|chunk| (chunk[0].0.clone(), Rc::new(Node::Leaf(chunk))))
            .collect();
        let mut units = 0u64;
        while level.len() > 1 {
            level = even_chunks(level)
                .into_iter()
                .map(|group| {
                    units += group.len() as u64;
                    let mut keys = Vec::with_capacity(group.len());
                    let mut children = Vec::with_capacity(group.len());
                    for (key, child) in group {
                        keys.push(key);
                        children.push(child);
                    }
                    // The first child's min is the node's min, not a separator.
                    let min = keys.remove(0);
                    (min, Rc::new(Node::Internal { keys, children }))
                })
                .collect();
        }
        let root = level.pop().map(|(_, node)| node);
        (PMap { root, len }, units)
    }

    /// Find `key`'s position. `cmp(stored)` orders a stored key against the
    /// probe; its first error aborts the search.
    pub fn search<E>(&self, cmp: &mut impl FnMut(&K) -> Result<Ordering, E>) -> Result<Search, E> {
        let mut path = Vec::new();
        let mut path_units = 0u64;
        let origin = (self.root_ptr(), self.len);
        let Some(mut node) = self.root.as_deref() else {
            return Ok(Search {
                path,
                slot: Err(0),
                path_units,
                origin,
            });
        };
        loop {
            path_units += node.len() as u64;
            match node {
                Node::Internal { keys, children } => {
                    let index = search_internal(keys, cmp)?;
                    path.push(index);
                    node = &children[index];
                }
                Node::Leaf(entries) => {
                    let slot = search_leaf(entries, cmp)?;
                    return Ok(Search {
                        path,
                        slot,
                        path_units,
                        origin,
                    });
                }
            }
        }
    }

    /// Look a key up (no path recorded).
    pub fn get<E>(&self, cmp: &mut impl FnMut(&K) -> Result<Ordering, E>) -> Result<Option<&V>, E> {
        let Some(mut node) = self.root.as_deref() else {
            return Ok(None);
        };
        loop {
            match node {
                Node::Internal { keys, children } => node = &children[search_internal(keys, cmp)?],
                Node::Leaf(entries) => {
                    return Ok(search_leaf(entries, cmp)?.ok().map(|i| &entries[i].1));
                }
            }
        }
    }

    /// Identity of the root node (null when empty): equal pointers mean
    /// equal maps.
    pub fn root_ptr(&self) -> *const () {
        self.root
            .as_ref()
            .map_or(std::ptr::null(), |root| Rc::as_ptr(root) as *const ())
    }

    /// Insert at a [`Search`] position, replacing the value if the key was
    /// found. Returns work units (≤ `search.insert_units_ceiling()`).
    pub fn insert_at(&mut self, search: Search, key: K, value: V) -> u64 {
        debug_assert!(search.origin == (self.root_ptr(), self.len), "stale Search");
        let Some(root) = &mut self.root else {
            self.root = Some(Rc::new(Node::Leaf(vec![(key, value)])));
            self.len = 1;
            return 1;
        };
        if search.slot.is_err() {
            self.len += 1;
        }
        let mut units = 0u64;
        let split = insert_rec(root, &search.path, search.slot, key, value, &mut units);
        if let Some((separator, right)) = split {
            let left = self.root.take().expect("root present");
            self.root = Some(Rc::new(Node::Internal {
                keys: vec![separator],
                children: vec![left, right],
            }));
            units += 2;
        }
        units
    }

    /// Remove the entry a successful [`Search`] found. Returns the value and
    /// work units (≤ `search.remove_units_ceiling()`).
    pub fn remove_at(&mut self, search: Search) -> (V, u64) {
        debug_assert!(search.origin == (self.root_ptr(), self.len), "stale Search");
        let Ok(index) = search.slot else {
            panic!("remove_at needs a found key");
        };
        let root = self.root.as_mut().expect("a found key has a root");
        let mut units = 0u64;
        let value = remove_rec(root, &search.path, index, &mut units);
        units += root.len() as u64;
        self.len -= 1;
        // Collapse a root left with one child, or drop an empty leaf root.
        let collapse = match self.root.as_deref() {
            Some(Node::Internal { children, .. }) if children.len() == 1 => {
                Some(Some(children[0].clone()))
            }
            Some(Node::Leaf(entries)) if entries.is_empty() => Some(None),
            _ => None,
        };
        if let Some(root) = collapse {
            self.root = root;
        }
        (value, units)
    }

    /// Entries in key order, from either end.
    pub fn iter(&self) -> Iter<'_, K, V> {
        let mut iter = Iter {
            front: Cursor::default(),
            back: Cursor::default(),
            remaining: self.len,
        };
        if let Some(root) = self.root.as_deref() {
            iter.front.descend(root, true);
            iter.back.descend(root, false);
        }
        iter
    }
}

/// Split `items` into `ceil(n / CAP)` runs of near-equal length — each at
/// least `MIN` when there is more than one.
fn even_chunks<T>(items: Vec<T>) -> Vec<Vec<T>> {
    let count = items.len().div_ceil(CAP);
    let (base, extra) = (items.len() / count, items.len() % count);
    let mut iter = items.into_iter();
    (0..count)
        .map(|i| iter.by_ref().take(base + usize::from(i < extra)).collect())
        .collect()
}

/// Insert below `node`; returns `(separator, new right sibling)` if `node`
/// overflowed and split.
fn insert_rec<K: Clone, V: Clone>(
    node: &mut Rc<Node<K, V>>,
    path: &[usize],
    slot: Result<usize, usize>,
    key: K,
    value: V,
    units: &mut u64,
) -> Option<(K, Rc<Node<K, V>>)> {
    match Rc::make_mut(node) {
        Node::Leaf(entries) => {
            match slot {
                Ok(i) => entries[i].1 = value,
                Err(i) => entries.insert(i, (key, value)),
            }
            *units += entries.len() as u64;
            if entries.len() <= CAP {
                return None;
            }
            let right = entries.split_off(entries.len() / 2);
            Some((right[0].0.clone(), Rc::new(Node::Leaf(right))))
        }
        Node::Internal { keys, children } => {
            let i = path[0];
            if let Some((separator, right)) =
                insert_rec(&mut children[i], &path[1..], slot, key, value, units)
            {
                keys.insert(i, separator);
                children.insert(i + 1, right);
            }
            *units += children.len() as u64;
            if children.len() <= CAP {
                return None;
            }
            // 17 children → 8 left, 9 right; the middle key moves up.
            let mid = children.len() / 2;
            let right_children = children.split_off(mid);
            let mut right_keys = keys.split_off(mid - 1);
            let separator = right_keys.remove(0);
            Some((
                separator,
                Rc::new(Node::Internal {
                    keys: right_keys,
                    children: right_children,
                }),
            ))
        }
    }
}

/// Remove below `node`. Each written node is counted once, at its final
/// size, by its parent (the root by [`PMap::remove_at`]).
fn remove_rec<K: Clone, V: Clone>(
    node: &mut Rc<Node<K, V>>,
    path: &[usize],
    index: usize,
    units: &mut u64,
) -> V {
    match Rc::make_mut(node) {
        Node::Leaf(entries) => entries.remove(index).1,
        Node::Internal { keys, children } => {
            let mut i = path[0];
            let value = remove_rec(&mut children[i], &path[1..], index, units);
            if children[i].len() < MIN {
                i = rebalance(keys, children, i, units);
            }
            *units += children[i].len() as u64;
            value
        }
    }
}

/// Refill underfull `children[i]`: borrow one from a sibling above `MIN`,
/// else merge with a sibling. Adds a borrowed-from sibling's size to
/// `units`; returns the refilled child's index.
fn rebalance<K: Clone, V: Clone>(
    keys: &mut Vec<K>,
    children: &mut Vec<Rc<Node<K, V>>>,
    i: usize,
    units: &mut u64,
) -> usize {
    if i > 0 && children[i - 1].len() > MIN {
        let (left, rest) = children.split_at_mut(i);
        let (left, child) = (Rc::make_mut(&mut left[i - 1]), Rc::make_mut(&mut rest[0]));
        *units += left.len() as u64 - 1;
        match (left, child) {
            (Node::Leaf(left), Node::Leaf(child)) => {
                let moved = left.pop().expect("above MIN");
                keys[i - 1] = moved.0.clone();
                child.insert(0, moved);
            }
            (
                Node::Internal {
                    keys: lk,
                    children: lc,
                },
                Node::Internal {
                    keys: ck,
                    children: cc,
                },
            ) => {
                cc.insert(0, lc.pop().expect("above MIN"));
                let up = lk.pop().expect("above MIN");
                ck.insert(0, std::mem::replace(&mut keys[i - 1], up));
            }
            _ => unreachable!("siblings share a level"),
        }
        i
    } else if i + 1 < children.len() && children[i + 1].len() > MIN {
        let (rest, right) = children.split_at_mut(i + 1);
        let (child, right) = (Rc::make_mut(&mut rest[i]), Rc::make_mut(&mut right[0]));
        *units += right.len() as u64 - 1;
        match (child, right) {
            (Node::Leaf(child), Node::Leaf(right)) => {
                child.push(right.remove(0));
                keys[i] = right[0].0.clone();
            }
            (
                Node::Internal {
                    keys: ck,
                    children: cc,
                },
                Node::Internal {
                    keys: rk,
                    children: rc,
                },
            ) => {
                cc.push(rc.remove(0));
                let up = rk.remove(0);
                ck.push(std::mem::replace(&mut keys[i], up));
            }
            _ => unreachable!("siblings share a level"),
        }
        i
    } else {
        // Merge the right node of the pair (i-1, i) or (i, i+1) into the left.
        let at = if i > 0 { i - 1 } else { i };
        let separator = keys.remove(at);
        let right = Rc::unwrap_or_clone(children.remove(at + 1));
        let left = Rc::make_mut(&mut children[at]);
        match (left, right) {
            (Node::Leaf(left), Node::Leaf(right)) => left.extend(right),
            (
                Node::Internal {
                    keys: lk,
                    children: lc,
                },
                Node::Internal {
                    keys: rk,
                    children: rc,
                },
            ) => {
                lk.push(separator);
                lk.extend(rk);
                lc.extend(rc);
            }
            _ => unreachable!("siblings share a level"),
        }
        at
    }
}

/// One end of an [`Iter`]: the internal nodes above the current leaf (with
/// the child index taken) and the leaf with the next position to yield.
struct Cursor<'a, K, V> {
    stack: Vec<(&'a [Rc<Node<K, V>>], usize)>,
    leaf: &'a [(K, V)],
    /// Front: next index to yield. Back: one past it.
    pos: usize,
}

impl<K, V> Default for Cursor<'_, K, V> {
    fn default() -> Self {
        Cursor {
            stack: Vec::new(),
            leaf: &[],
            pos: 0,
        }
    }
}

impl<'a, K, V> Cursor<'a, K, V> {
    /// Walk to the first (`front`) or last leaf under `node`.
    fn descend(&mut self, mut node: &'a Node<K, V>, front: bool) {
        loop {
            match node {
                Node::Internal { children, .. } => {
                    let i = if front { 0 } else { children.len() - 1 };
                    self.stack.push((children, i));
                    node = &children[i];
                }
                Node::Leaf(entries) => {
                    self.leaf = entries;
                    self.pos = if front { 0 } else { entries.len() };
                    return;
                }
            }
        }
    }

    /// Move to the next (`front`) or previous leaf.
    fn step_leaf(&mut self, front: bool) {
        while let Some((children, i)) = self.stack.pop() {
            let next = if front {
                (i + 1 < children.len()).then_some(i + 1)
            } else {
                i.checked_sub(1)
            };
            if let Some(next) = next {
                self.stack.push((children, next));
                self.descend(&children[next], front);
                return;
            }
        }
    }
}

/// Double-ended, exact-size iterator over a [`PMap`]'s entries in key order.
pub struct Iter<'a, K, V> {
    front: Cursor<'a, K, V>,
    back: Cursor<'a, K, V>,
    remaining: usize,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = &'a (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if self.front.pos == self.front.leaf.len() {
            self.front.step_leaf(true);
        }
        self.front.pos += 1;
        Some(&self.front.leaf[self.front.pos - 1])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> DoubleEndedIterator for Iter<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if self.back.pos == 0 {
            self.back.step_leaf(false);
        }
        self.back.pos -= 1;
        Some(&self.back.leaf[self.back.pos])
    }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::convert::Infallible;

    type Map = PMap<i64, i64>;

    fn by(probe: i64) -> impl FnMut(&i64) -> Result<Ordering, Infallible> {
        move |stored: &i64| Ok(stored.cmp(&probe))
    }

    fn get(map: &Map, key: i64) -> Option<i64> {
        map.get(&mut by(key)).unwrap().copied()
    }

    fn insert(map: &mut Map, key: i64, value: i64) -> u64 {
        let search = map.search(&mut by(key)).unwrap();
        let ceiling = search.insert_units_ceiling();
        let units = map.insert_at(search, key, value);
        assert!(units <= ceiling, "insert units {units} > ceiling {ceiling}");
        units
    }

    fn remove(map: &mut Map, key: i64) -> Option<i64> {
        let search = map.search(&mut by(key)).unwrap();
        if !search.found() {
            return None;
        }
        let ceiling = search.remove_units_ceiling();
        let (value, units) = map.remove_at(search);
        assert!(units <= ceiling, "remove units {units} > ceiling {ceiling}");
        Some(value)
    }

    fn build(keys: impl IntoIterator<Item = i64>) -> Map {
        let mut map = Map::new();
        for k in keys {
            insert(&mut map, k, k * 10);
        }
        map
    }

    fn depth(map: &Map) -> usize {
        let mut depth = 0;
        let mut node = map.root.as_deref();
        while let Some(n) = node {
            depth += 1;
            node = match n {
                Node::Internal { children, .. } => Some(&children[0]),
                Node::Leaf(_) => None,
            };
        }
        depth
    }

    /// Sorted keys, separator bounds, fill bounds, uniform leaf depth, and
    /// `len`, checked over the whole tree.
    fn check_invariants(map: &Map) {
        fn walk(
            node: &Node<i64, i64>,
            is_root: bool,
            lo: Option<i64>,
            hi: Option<i64>,
            depth: usize,
            leaf_depth: &mut Option<usize>,
            count: &mut usize,
        ) {
            let n = node.len();
            assert!(n <= CAP, "node over capacity: {n}");
            match node {
                Node::Leaf(entries) => {
                    assert!(is_root || n >= MIN, "leaf under MIN: {n}");
                    assert!(!entries.is_empty(), "empty leaf");
                    assert_eq!(*leaf_depth.get_or_insert(depth), depth, "uneven depth");
                    for pair in entries.windows(2) {
                        assert!(pair[0].0 < pair[1].0, "leaf unsorted");
                    }
                    for (k, _) in entries {
                        assert!(lo.is_none_or(|lo| *k >= lo), "key {k} below bound");
                        assert!(hi.is_none_or(|hi| *k < hi), "key {k} above bound");
                    }
                    *count += n;
                }
                Node::Internal { keys, children } => {
                    assert!(
                        n >= if is_root { 2 } else { MIN },
                        "internal under fill: {n}"
                    );
                    assert_eq!(keys.len() + 1, n, "keys/children mismatch");
                    for pair in keys.windows(2) {
                        assert!(pair[0] < pair[1], "separators unsorted");
                    }
                    for (i, child) in children.iter().enumerate() {
                        let clo = if i == 0 { lo } else { Some(keys[i - 1]) };
                        let chi = if i == keys.len() { hi } else { Some(keys[i]) };
                        walk(child, false, clo, chi, depth + 1, leaf_depth, count);
                    }
                }
            }
        }
        let mut count = 0;
        if let Some(root) = map.root.as_deref() {
            walk(root, true, None, None, 0, &mut None, &mut count);
        }
        assert_eq!(count, map.len(), "len mismatch");
    }

    fn assert_matches(map: &Map, model: &BTreeMap<i64, i64>) {
        check_invariants(map);
        let forward: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
        let expected: Vec<_> = model.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(forward, expected);
        let backward: Vec<_> = map.iter().rev().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(backward, expected.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(map.iter().len(), model.len());
    }

    /// splitmix64: a dependency-free seeded generator.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn empty_map() {
        let map = Map::new();
        assert!(map.is_empty());
        assert_eq!(get(&map, 1), None);
        assert_eq!(map.iter().next(), None);
        assert_eq!(map.iter().next_back(), None);
        let (built, units) = Map::from_sorted(Vec::new());
        assert!(built.is_empty());
        assert_eq!(units, 0);
    }

    #[test]
    fn single_leaf_units_match_a_flat_vector() {
        // Inserting into a one-leaf map writes exactly the new vector.
        let mut map = Map::new();
        let units: Vec<u64> = (0..4).map(|k| insert(&mut map, k, k)).collect();
        assert_eq!(units, [1, 2, 3, 4]);
        assert_eq!(insert(&mut map, 2, 99), 4, "replace rewrites the leaf");
        let search = map.search(&mut by(0)).unwrap();
        assert_eq!(map.remove_at(search).1, 3);
    }

    #[test]
    fn leaf_split_then_root_split() {
        let mut map = build(0..CAP as i64);
        assert_eq!(depth(&map), 1);
        insert(&mut map, CAP as i64, 0);
        assert_eq!(depth(&map), 2, "17th entry splits the root leaf");
        check_invariants(&map);
        // Enough sequential inserts to overflow the internal root.
        let map = build(0..400);
        assert_eq!(depth(&map), 3);
        check_invariants(&map);
        assert_eq!(get(&map, 399), Some(3990));
    }

    #[test]
    fn borrow_merge_and_root_collapse() {
        let mut map = build(0..400);
        let mut model: BTreeMap<i64, i64> = (0..400).map(|k| (k, k * 10)).collect();
        // Removing from the front forces borrows from the right, then merges.
        for k in 0..200 {
            assert_eq!(remove(&mut map, k), model.remove(&k));
            assert_matches(&map, &model);
        }
        // From the back: borrows from the left.
        for k in (300..400).rev() {
            assert_eq!(remove(&mut map, k), model.remove(&k));
            assert_matches(&map, &model);
        }
        // Drain fully: the root collapses level by level to empty.
        let mut depths = vec![depth(&map)];
        for k in 200..300 {
            remove(&mut map, k);
            model.remove(&k);
            assert_matches(&map, &model);
            depths.push(depth(&map));
        }
        assert!(depths.windows(2).all(|d| d[1] <= d[0]));
        assert_eq!(depths.last(), Some(&0));
        assert!(map.is_empty());
        assert_eq!(remove(&mut map, 5), None);
    }

    #[test]
    fn from_sorted_packs_evenly() {
        for n in [1, 15, 16, 17, 33, 256, 257, 1500, 6800] {
            let entries: Vec<(i64, i64)> = (0..n).map(|k| (k, k)).collect();
            let model: BTreeMap<i64, i64> = entries.iter().copied().collect();
            let (map, units) = Map::from_sorted(entries);
            assert_matches(&map, &model);
            assert_eq!(units == 0, n <= CAP as i64, "only multi-leaf builds charge");
        }
    }

    #[test]
    fn remove_units_count_each_written_node_at_its_final_size() {
        let sorted = |n: i64| Map::from_sorted((0..n).map(|k| (k, k)).collect()).0;
        // Leaves 9 + 8. Removing 16 borrows from the left: 8 + 8 + root 2.
        let mut map = sorted(17);
        let search = map.search(&mut by(16)).unwrap();
        assert_eq!(map.remove_at(search).1, 18);
        // Leaves 8 + 8. Removing 0 merges into one leaf of 15 + root 1.
        let mut map = sorted(16);
        insert(&mut map, 16, 16); // split: leaves 8 + 9
        remove(&mut map, 16); // borrow-free: leaves 8 + 8
        let search = map.search(&mut by(0)).unwrap();
        assert_eq!(map.remove_at(search).1, 16);
        assert_eq!(depth(&map), 1, "the one-child root collapsed");
    }

    #[test]
    fn edits_on_bulk_built_trees() {
        // Bulk-built nodes sit at or near MIN, so the first removes go
        // straight to borrows and merges.
        let mut rng = Rng(7);
        for n in [17, 257, 1500] {
            let entries: Vec<(i64, i64)> = (0..n).map(|k| (k * 2, k)).collect();
            let mut model: BTreeMap<i64, i64> = entries.iter().copied().collect();
            let mut map = Map::from_sorted(entries).0;
            for _ in 0..2000 {
                let key = rng.below(n as u64 * 2) as i64;
                if rng.below(3) == 0 {
                    insert(&mut map, key, -key);
                    model.insert(key, -key);
                } else {
                    assert_eq!(remove(&mut map, key), model.remove(&key));
                }
                check_invariants(&map);
            }
            assert_matches(&map, &model);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "stale Search")]
    fn a_stale_search_is_caught() {
        let mut map = build(0..10);
        let search = map.search(&mut by(50)).unwrap();
        insert(&mut map, 20, 0);
        map.insert_at(search, 50, 0);
    }

    #[test]
    fn double_ended_iteration_meets_in_the_middle() {
        let map = build(0..100);
        let mut iter = map.iter();
        let mut seen = Vec::new();
        for step in 0.. {
            let item = if step % 3 == 0 {
                iter.next_back()
            } else {
                iter.next()
            };
            match item {
                Some((k, _)) => seen.push(*k),
                None => break,
            }
            assert_eq!(iter.len(), 100 - seen.len());
        }
        seen.sort();
        assert_eq!(seen, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn failed_comparison_leaves_the_map_untouched() {
        let mut map = build(0..100);
        let before: Vec<_> = map.iter().cloned().collect();
        let mut calls = 0;
        let refused = map.search(&mut |stored: &i64| {
            calls += 1;
            if calls > 3 {
                Err("out of fuel")
            } else {
                Ok(stored.cmp(&50))
            }
        });
        assert!(refused.is_err());
        assert_eq!(map.iter().cloned().collect::<Vec<_>>(), before);
        insert(&mut map, 1000, 1);
        assert_eq!(map.len(), 101);
    }

    #[test]
    fn edits_never_change_older_versions() {
        let base = build(0..300);
        let snapshot: Vec<_> = base.iter().cloned().collect();
        let mut edited = base.clone();
        insert(&mut edited, 150, -1);
        insert(&mut edited, 1000, -1);
        for k in 0..100 {
            remove(&mut edited, k);
        }
        assert_eq!(base.iter().cloned().collect::<Vec<_>>(), snapshot);
        check_invariants(&base);
        check_invariants(&edited);
    }

    #[test]
    fn unique_maps_edit_in_place() {
        // A uniquely owned leaf is edited in place: the root Rc is reused.
        let mut map = build(0..10);
        let before = Rc::as_ptr(map.root.as_ref().unwrap());
        insert(&mut map, 3, 7);
        assert_eq!(Rc::as_ptr(map.root.as_ref().unwrap()), before);
        // A shared one is copied, and the old version keeps its node.
        let old = map.clone();
        insert(&mut map, 4, 7);
        assert_ne!(Rc::as_ptr(map.root.as_ref().unwrap()), before);
        assert_eq!(Rc::as_ptr(old.root.as_ref().unwrap()), before);
    }

    /// 100k seeded random ops against `BTreeMap`. Every 1k ops both maps are
    /// snapshotted; later ops (including edits to old snapshots) must never
    /// change what an earlier snapshot holds.
    #[test]
    fn differential_against_btreemap_with_retained_versions() {
        let mut rng = Rng(0x5eed_f00d);
        let mut map = Map::new();
        let mut model = BTreeMap::new();
        let mut versions: Vec<(Map, BTreeMap<i64, i64>)> = Vec::new();
        let mut max_depth = 0;
        for op in 0..100_000u64 {
            // Key range drifts so the map grows, churns, and shrinks.
            let range = 64 + (op / 50) % 3000;
            let key = rng.below(range) as i64;
            match rng.below(10) {
                0..=4 => {
                    let value = rng.next() as i64;
                    insert(&mut map, key, value);
                    model.insert(key, value);
                }
                5..=7 => assert_eq!(remove(&mut map, key), model.remove(&key)),
                8 => assert_eq!(get(&map, key), model.get(&key).copied()),
                _ => {
                    // Branch from an old version; its original must survive.
                    if let Some((old, old_model)) = versions.last() {
                        let (mut branch, mut branch_model) = (old.clone(), old_model.clone());
                        insert(&mut branch, key, -1);
                        branch_model.insert(key, -1);
                        remove(&mut branch, key / 2);
                        branch_model.remove(&(key / 2));
                        assert_matches(&branch, &branch_model);
                    }
                }
            }
            if op % 100 == 0 {
                assert_matches(&map, &model);
                max_depth = max_depth.max(depth(&map));
            }
            if op % 1000 == 0 {
                versions.push((map.clone(), model.clone()));
            }
        }
        assert_matches(&map, &model);
        assert!(
            max_depth >= 3,
            "the workload must reach internal splits and merges"
        );
        for (old, old_model) in &versions {
            assert_matches(old, old_model);
        }
    }
}
