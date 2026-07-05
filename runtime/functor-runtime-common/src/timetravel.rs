//! The model half of the time-travel Timeline (docs/time-travel.md, T1).
//!
//! [`History`] is a bounded, per-frame snapshot ring of a cheaply-cloneable
//! state — in Functor that state is the MLE `model` (`mle::Value`). It is the
//! counterpart of the physics [`crate::physics::timeline::TimelineLog`], but
//! deliberately simpler:
//!
//! - **The physics `TimelineLog` keyframes + replays** because its snapshot (a
//!   whole serialized Rapier world) is expensive, so it stores one every N
//!   frames and re-steps forward to reconstruct the rest. That leans on
//!   determinism.
//! - **`History` snapshots every frame directly.** The MLE model is `Rc`-shared
//!   and immutable, so a clone is a handful of refcount bumps and adjacent
//!   frames structurally share every unchanged sub-tree (the
//!   `structural_sharing_*` test proves it). Because a scrub-back is a plain
//!   restore of a stored value — never a re-step — **scrubbing backward needs
//!   no determinism at all** (docs/time-travel.md, "The determinism boundary").
//!
//! ## The frame convention
//!
//! `record(f, state)` stores the *settled* state of frame `f` (in Functor: the
//! model after that frame's input/subscriptions/tick have folded in). `seek(f)`
//! returns exactly that stored state. Frames must be recorded consecutively;
//! `truncate_from` reopens an earlier frame for re-recording (rewind-then-branch,
//! mirroring `TimelineLog::truncate_from`).
//!
//! `Frame` is the same `u64` fixed-frame index the physics timeline uses; a
//! later increment unifies the two under one clock so a single `seek` restores
//! model *and* world together (docs/time-travel.md, "One frame, one clock").

use std::collections::VecDeque;

/// A fixed-step frame number — the same index space as
/// [`crate::physics::timeline::Frame`].
pub type Frame = u64;

/// Default rendered-frame retention (~15s at 60fps) — the window both game
/// shells size their model `History` to, matching the physics recorder. Shared
/// so the two shells can't silently diverge (docs/time-travel.md T1).
pub const DEFAULT_HISTORY_FRAMES: usize = 900;

/// A bounded per-frame snapshot ring of `T`. Records the settled state of each
/// consecutive frame and restores any frame still in the retained window. See
/// the module docs for why this is a plain ring, not a keyframe log.
pub struct History<T> {
    /// Frame number of `snapshots[0]`; valid once anything has been recorded.
    base: Frame,
    /// One snapshot per recorded frame, oldest first.
    snapshots: VecDeque<T>,
    /// The frame the *next* `record` must use — `None` until the first record.
    /// This is deliberately distinct from "`snapshots` is empty": after a
    /// `truncate_from` empties the ring, `next` stays `Some(branch_point)` so
    /// re-recording resumes at the right frame. Keying "next frame" off an
    /// empty `snapshots` instead would let a post-truncate `record` silently
    /// reset the base and resurrect a pruned frame (Codex xreview).
    next: Option<Frame>,
    /// Retain at most this many frames — the oldest is dropped when a `record`
    /// would exceed it. `None` = unbounded.
    capacity: Option<usize>,
}

impl<T: Clone> History<T> {
    /// Retain at most `capacity` frames (must be >= 1); older frames are pruned
    /// as new ones arrive. Functor sizes this to ~15s of history, matching the
    /// physics recorder's `HISTORY_FRAMES`.
    pub fn bounded(capacity: usize) -> History<T> {
        assert!(capacity >= 1, "history capacity must be at least 1");
        History {
            base: 0,
            snapshots: VecDeque::new(),
            next: None,
            capacity: Some(capacity),
        }
    }

    /// Retain every frame — for tests and short deterministic runs where the
    /// whole history fits in memory.
    pub fn unbounded() -> History<T> {
        History {
            base: 0,
            snapshots: VecDeque::new(),
            next: None,
            capacity: None,
        }
    }

    /// Record the settled state of `frame`. Frames must be recorded
    /// consecutively (each call `= last + 1`); the first recorded frame sets
    /// the base. When the ring is full the oldest frame is dropped, advancing
    /// the base.
    pub fn record(&mut self, frame: Frame, state: &T) {
        match self.next {
            None => self.base = frame,
            Some(next) => assert_eq!(
                frame, next,
                "history frames must be recorded consecutively"
            ),
        }
        self.snapshots.push_back(state.clone());
        if let Some(cap) = self.capacity {
            while self.snapshots.len() > cap {
                self.snapshots.pop_front();
                self.base += 1;
            }
        }
        self.next = Some(self.base + self.snapshots.len() as u64);
    }

    /// The stored state of `frame`. Panics if `frame` is outside the retained
    /// window — a silently clamped read would restore the wrong frame.
    pub fn seek(&self, frame: Frame) -> &T {
        let (oldest, newest) = self.recorded_range().expect("seek on an empty history");
        assert!(
            frame >= oldest && frame <= newest,
            "seek({frame}) outside recorded history [{oldest}, {newest}]"
        );
        &self.snapshots[(frame - self.base) as usize]
    }

    /// The most recently recorded state, or `None` if nothing is recorded yet.
    pub fn latest(&self) -> Option<&T> {
        self.snapshots.back()
    }

    /// The seekable range `(oldest, newest)` inclusive — `None` until something
    /// is recorded. `bounded` pruning moves the floor; `truncate_from` moves
    /// the ceiling.
    pub fn recorded_range(&self) -> Option<(Frame, Frame)> {
        (!self.snapshots.is_empty())
            .then(|| (self.base, self.base + self.snapshots.len() as u64 - 1))
    }

    /// Drop all recorded frames at and after `frame` — the record-after-seek
    /// truncation that rewind-then-*branch* needs (mirrors
    /// `TimelineLog::truncate_from`): after seeking back to `frame`,
    /// `truncate_from(frame)` makes `record(frame, …)` legal again and the old
    /// future is gone. A `frame` at or past the end is a no-op; one before the
    /// retained window panics (the branch point was pruned away).
    pub fn truncate_from(&mut self, frame: Frame) {
        let Some(next) = self.next else { return };
        if frame >= next {
            return;
        }
        assert!(
            frame >= self.base,
            "truncate_from({frame}) predates recorded history (base {})",
            self.base
        );
        self.snapshots.truncate((frame - self.base) as usize);
        // Even when this empties the ring (`frame == base`), the next legal
        // record frame is `frame` — hold it so re-recording branches cleanly
        // instead of resetting the base.
        self.next = Some(frame);
    }

    /// Number of frames currently retained.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mle::Value;
    use std::rc::Rc;

    // --- the generic ring, exercised over a trivial Clone state ---

    #[test]
    fn record_then_seek_returns_each_frame() {
        let mut h = History::unbounded();
        for f in 0..10 {
            h.record(f, &(f * 100));
        }
        assert_eq!(h.recorded_range(), Some((0, 9)));
        for f in 0..10 {
            assert_eq!(*h.seek(f), f * 100);
        }
        assert_eq!(h.latest(), Some(&900));
    }

    #[test]
    fn bounded_prunes_the_oldest_frames() {
        let mut h = History::bounded(5);
        for f in 0..10 {
            h.record(f, &f);
        }
        // Only the last 5 frames survive.
        assert_eq!(h.len(), 5);
        assert_eq!(h.recorded_range(), Some((5, 9)));
        assert_eq!(*h.seek(5), 5);
        assert_eq!(*h.seek(9), 9);
    }

    #[test]
    #[should_panic(expected = "outside recorded history")]
    fn seek_below_the_pruned_floor_panics() {
        let mut h = History::bounded(5);
        for f in 0..10 {
            h.record(f, &f);
        }
        h.seek(4); // pruned away
    }

    #[test]
    fn truncate_from_drops_the_future_and_reopens_recording() {
        let mut h = History::unbounded();
        for f in 0..10 {
            h.record(f, &(f as i64));
        }
        // Rewind to 6 and branch: the old 6..10 is discarded.
        h.truncate_from(6);
        assert_eq!(h.recorded_range(), Some((0, 5)));
        // Re-record from the branch point with different values.
        for f in 6..9 {
            h.record(f, &(-(f as i64)));
        }
        assert_eq!(h.recorded_range(), Some((0, 8)));
        assert_eq!(*h.seek(5), 5);
        assert_eq!(*h.seek(6), -6, "the branch overwrote the old future");
        assert_eq!(*h.seek(8), -8);
    }

    #[test]
    #[should_panic(expected = "recorded consecutively")]
    fn branch_to_empty_still_enforces_the_next_frame() {
        // Truncating exactly at the retained floor empties the ring; the next
        // record must still resume at the branch point, not silently reset the
        // base and resurrect a pruned frame (Codex xreview).
        let mut h = History::bounded(5);
        for f in 0..10 {
            h.record(f, &f); // retained 5..9
        }
        h.truncate_from(5); // legal branch point; empties the ring
        assert!(h.is_empty());
        assert_eq!(h.recorded_range(), None);
        h.record(0, &0); // frame 0 was pruned away — must panic, not be accepted
    }

    #[test]
    fn branch_to_empty_then_rerecord_at_the_branch_point() {
        let mut h = History::bounded(5);
        for f in 0..10 {
            h.record(f, &f);
        }
        h.truncate_from(5); // empties the ring, next legal frame is 5
        h.record(5, &555); // resumes at the branch point
        assert_eq!(h.recorded_range(), Some((5, 5)));
        assert_eq!(*h.seek(5), 555);
    }

    #[test]
    fn truncate_from_past_the_end_is_a_noop() {
        let mut h = History::unbounded();
        for f in 0..5 {
            h.record(f, &f);
        }
        h.truncate_from(5);
        h.truncate_from(99);
        assert_eq!(h.recorded_range(), Some((0, 4)));
    }

    #[test]
    #[should_panic(expected = "recorded consecutively")]
    fn non_consecutive_record_panics() {
        let mut h = History::unbounded();
        h.record(0, &0);
        h.record(2, &2);
    }

    // --- the real payload: an mle::Value model ---

    fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
        match v {
            Value::Record(fields) => {
                &fields.iter().find(|(k, _)| k == name).expect("field").1
            }
            _ => panic!("not a record"),
        }
    }

    /// Build `{ shared: <shared>, n: <n> }` — models sharing a `shared` field
    /// keep the same `Rc` allocation for it.
    fn model(shared: &Value, n: f64) -> Value {
        Value::Record(Rc::new(vec![
            ("shared".to_string(), shared.clone()),
            ("n".to_string(), Value::Number(n)),
        ]))
    }

    #[test]
    fn value_model_restores_exact() {
        let shared = Value::List(Rc::new(vec![Value::Number(1.0), Value::Number(2.0)]));
        let mut h = History::unbounded();
        // Simulate `{ model with n: n + 1 }` evolving each frame.
        for f in 0..8 {
            h.record(f, &model(&shared, f as f64));
        }
        // A scrub back returns the exact settled model of that frame (compared
        // via the canonical Display form — Value has no PartialEq).
        assert_eq!(h.seek(3).to_string(), model(&shared, 3.0).to_string());
        assert_eq!(h.seek(0).to_string(), model(&shared, 0.0).to_string());
    }

    #[test]
    fn structural_sharing_survives_snapshots() {
        // One `shared` sub-tree referenced by every frame's model.
        let shared = Value::List(Rc::new(vec![Value::Number(1.0), Value::Number(2.0)]));
        let mut h = History::unbounded();
        h.record(0, &model(&shared, 0.0));
        h.record(1, &model(&shared, 1.0));

        // The stored snapshots hold the *same* allocation for `shared`, not a
        // deep copy — this is why a 900-frame ring of a large model stays cheap
        // (docs/time-travel.md, "Why MLE makes this nearly free").
        let s0 = field(h.seek(0), "shared");
        let s1 = field(h.seek(1), "shared");
        match (s0, s1) {
            (Value::List(a), Value::List(b)) => {
                assert!(Rc::ptr_eq(a, b), "the shared sub-tree was deep-copied");
            }
            _ => panic!("shared field is not a list"),
        }
    }
}
