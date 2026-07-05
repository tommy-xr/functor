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

use mle::Value;

use crate::physics::SteppedPhysics;

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

/// The coupled time-travel recorder shared by both MLE shells (docs/time-travel.md
/// T1–T3): records the MVU `model` and the physics fixed-frame in lockstep each
/// rendered frame, and seeks/rewinds them together. It owns the recording rings
/// and the scrub state; the producer keeps ownership of the `model` and the
/// [`SteppedPhysics`] and hands them in, so this one implementation drives the
/// desktop and web producers identically (no drift).
///
/// The master clock is the RENDERED frame; the physics world couples via
/// `world_frame_history` (the fixed frame each rendered frame ended at). A
/// **scrub** ([`Self::seek_scene_to`]) is non-destructive so the draggable bar
/// can drag back and forth; the future is discarded only when play resumes from
/// the scrubbed point ([`Self::commit_scrub_if_resuming`], which branches via
/// [`Self::rewind_scene_to`]). Every coupled seek is **exact-or-refused** — it
/// never lands the model and world on different times.
pub struct SceneRecorder {
    model_history: History<Value>,
    /// The physics fixed-frame reached at the end of each rendered frame,
    /// recorded in LOCKSTEP with `model_history`.
    world_frame_history: History<u64>,
    /// The rendered-frame index the next snapshot records at; monotonic.
    rendered_frame: u64,
    /// While dragging: the frame the scrubber has non-destructively seeked to.
    /// `Some` = "scrubbing" (future intact); committed on resume.
    scrub_pos: Option<u64>,
}

impl Default for SceneRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneRecorder {
    pub fn new() -> SceneRecorder {
        SceneRecorder {
            model_history: History::bounded(DEFAULT_HISTORY_FRAMES),
            world_frame_history: History::bounded(DEFAULT_HISTORY_FRAMES),
            rendered_frame: 0,
            scrub_pos: None,
        }
    }

    /// Hot-reload boundary: drop the rings (their snapshots can hold old-module
    /// closures) but keep `rendered_frame` monotonic so recording resumes
    /// consecutively, and clear any scrub.
    pub fn reset_on_reload(&mut self) {
        self.model_history = History::bounded(DEFAULT_HISTORY_FRAMES);
        self.world_frame_history = History::bounded(DEFAULT_HISTORY_FRAMES);
        self.scrub_pos = None;
    }

    /// Record the settled `model` and the world's current fixed frame for this
    /// rendered frame. Call at the end of `tick` only when the sim advanced
    /// (`dts > 0`) — a paused frame would pile up frozen duplicates.
    pub fn record(&mut self, model: &Value, physics_fixed_frame: u64) {
        self.model_history.record(self.rendered_frame, model);
        self.world_frame_history
            .record(self.rendered_frame, &physics_fixed_frame);
        self.rendered_frame += 1;
    }

    /// The frame the handle sits on (the scrubbed-to frame while dragging, else
    /// the newest recorded frame).
    pub fn current_scene_frame(&self) -> Option<u64> {
        self.scrub_pos
            .or_else(|| self.model_history.recorded_range().map(|(_, hi)| hi))
    }

    /// The seekable window `(oldest, newest)` — the draggable range.
    pub fn scene_frame_range(&self) -> Option<(u64, u64)> {
        self.model_history.recorded_range()
    }

    /// If play resumes (`dts > 0`) while parked on an earlier frame, branch the
    /// timeline from there BEFORE the frame advances. Call at the top of `tick`.
    pub fn commit_scrub_if_resuming(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_status: &mut (u64, bool, u64),
        has_physics: bool,
    ) {
        if let Some(k) = self.scrub_pos.take() {
            let _ = self.rewind_scene_to(k, model, physics, physics_status, has_physics);
        }
    }

    /// Non-destructive scrub for the draggable bar: restore `model` + world to
    /// `target` for DISPLAY without truncating, so the caller can seek back and
    /// forth. Exact-or-refused.
    pub fn seek_scene_to(
        &mut self,
        target: u64,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_status: &mut (u64, bool, u64),
        has_physics: bool,
    ) -> Result<String, String> {
        let (lo, hi) = self
            .model_history
            .recorded_range()
            .ok_or_else(|| "seek: nothing recorded yet".to_string())?;
        let frame = target.clamp(lo, hi);
        let physics_target = self.physics_seek_target(frame, physics, has_physics)?;
        *model = self.model_history.seek(frame).clone();
        if let Some(fixed) = physics_target {
            let _ = physics.seek_to_frame(fixed);
            physics_status.0 = physics.current_fixed_frame();
        }
        self.scrub_pos = Some(frame);
        Ok(format!("scrubbed to rendered frame {frame}"))
    }

    /// Coupled rewind: restore `model` + world to `target` and BRANCH the
    /// recorded future from there (`rendered_frame` resets to `target + 1`).
    /// Exact-or-refused: verifies the physics frame is restorable BEFORE
    /// mutating, returning `Err` (touching nothing) if it was pruned.
    pub fn rewind_scene_to(
        &mut self,
        target: u64,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_status: &mut (u64, bool, u64),
        has_physics: bool,
    ) -> Result<String, String> {
        let (lo, hi) = self
            .model_history
            .recorded_range()
            .ok_or_else(|| "rewind: nothing recorded yet".to_string())?;
        let frame = target.clamp(lo, hi);
        let physics_target = self.physics_seek_target(frame, physics, has_physics)?;
        *model = self.model_history.seek(frame).clone();
        if let Some(fixed) = physics_target {
            let _ = physics.rewind_to_frame(fixed);
            physics_status.0 = physics.current_fixed_frame();
        }
        self.model_history.truncate_from(frame + 1);
        self.world_frame_history.truncate_from(frame + 1);
        self.rendered_frame = frame + 1;
        self.scrub_pos = None;
        let clamped = if frame == target {
            String::new()
        } else {
            format!(" (requested {target}, clamped to the recorded window)")
        };
        Ok(format!("rewound scene to rendered frame {frame}{clamped}"))
    }

    /// Resolve the physics fixed-frame to seek for rendered `frame` WITHOUT
    /// mutating: `Ok(None)` = no seek needed (no physics, or the frame's end-
    /// state is already the live append), `Ok(Some(fixed))` = exact seek,
    /// `Err` = pruned (refuse rather than desync). Compares against the newest
    /// RECORDED frame, not the live world frame — after a non-destructive scrub
    /// the world is parked mid-history, and using the live frame would skip the
    /// truncate on the branch commit and panic on the next (non-consecutive)
    /// record.
    fn physics_seek_target(
        &self,
        frame: u64,
        physics: &SteppedPhysics,
        has_physics: bool,
    ) -> Result<Option<u64>, String> {
        if !has_physics {
            return Ok(None);
        }
        let want = *self.world_frame_history.seek(frame);
        match physics.seekable_range() {
            None => Ok(None),
            Some((_, hi)) if want > hi => Ok(None),
            Some((flo, hi)) if want >= flo && want <= hi => Ok(Some(want)),
            _ => Err(format!(
                "cannot seek to rendered frame {frame}: its physics frame {want} has \
                 been pruned from the {DEFAULT_HISTORY_FRAMES}-frame world history"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
