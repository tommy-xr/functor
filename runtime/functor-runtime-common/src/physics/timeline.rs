//! The rewind seam: `Simulatable` + `Timeline` (docs/physics.md, "Rewind").
//!
//! The **command log is the invariant** — server-authoritative prediction needs
//! it regardless of rewind strategy. The only thing that varies between
//! strategies is **snapshot cadence**, and therefore how `seek` reconstructs a
//! frame. So the whole design is two small traits: anything rewindable
//! implements [`Simulatable`]; the swappable strategy implements [`Timeline`].
//! The sim loop and (later) the netcode reconciler name only the traits.
//!
//! ## The frame convention
//!
//! `record(f, sim, cmds)` is called at the **start** of frame `f`, with `sim`
//! in the pre-step state and `cmds` the commands about to be applied; the
//! driver then calls `sim.step(cmds)`. `seek(f)` restores that same pre-step
//! state of frame `f`. Frames are recorded consecutively from the first
//! `record` call.
//!
//! The trait contract — `seek(K)` equals restoring a valid earlier snapshot
//! and re-stepping with the recorded commands — **is** the determinism
//! invariant (docs/physics.md); the strategy-equivalence golden in
//! `goldens.rs` asserts it byte-for-byte with the every-frame cadence as the
//! oracle.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{PhysicsCommand, PhysicsEvent, PhysicsScene, PhysicsSnapshot, World};

/// A fixed-step frame number.
pub type Frame = u64;

/// One frame's worth of input to a physics [`World`] step. This is what a
/// replay re-executes, so it must capture *everything* that can change the
/// world: the declared scene (whose history is also the insert/remove
/// history Rapier arena handles depend on) and the frame's fire-and-forget
/// commands (impulses/forces/teleports — Phase 3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    DeclareScene(PhysicsScene),
    Apply(PhysicsCommand),
}

/// Events produced by a step: contact transitions (docs/physics.md Phase 5).
pub use super::PhysicsEvent as Event;

/// Anything rewindable. Physics is the first impl; the whole game model
/// (serializable + input-driven) could be a second later.
pub trait Simulatable {
    /// Full serializable state — restoring it resumes bit-exact.
    type Snapshot;
    /// Per-frame inputs (see [`Command`] for physics).
    type Command;
    type Event;

    fn snapshot(&self) -> Self::Snapshot;
    fn restore(&mut self, s: &Self::Snapshot);
    /// Apply one frame's commands, then advance one fixed step. (The timestep
    /// is a fixed property of the sim — `FIXED_DT` for physics — not a
    /// parameter, so a variable dt can't sneak in through this seam.)
    fn step(&mut self, cmds: &[Self::Command]) -> Vec<Self::Event>;
}

impl Simulatable for World {
    type Snapshot = PhysicsSnapshot;
    type Command = Command;
    type Event = Event;

    fn snapshot(&self) -> PhysicsSnapshot {
        self.checkpoint()
    }

    fn restore(&mut self, snapshot: &PhysicsSnapshot) {
        self.restore_checkpoint(snapshot);
    }

    fn step(&mut self, cmds: &[Command]) -> Vec<PhysicsEvent> {
        // Per-frame event discipline mirrors `step_frame`: stale events from
        // an undrained prior step must not leak into this one.
        self.events_clear();
        for cmd in cmds {
            match cmd {
                Command::DeclareScene(scene) => self.reconcile(scene),
                Command::Apply(command) => self.queue_command(command.clone()),
            }
        }
        // Same per-frame command discipline as `step_frame`: queued commands
        // land after this frame's reconcile, forces last exactly one frame.
        // NOTE one live-vs-timeline difference: `step_frame` may run 0–8
        // substeps per rendered frame (commands carry over a zero-substep
        // frame), while this seam is exactly one fixed step per recorded
        // frame. Recording live play (Phase 6) must record commands against
        // the FIXED frame they actually applied on, not the rendered frame
        // they were issued on.
        self.apply_pending();
        self.step_fixed();
        self.clear_frame_forces();
        self.take_events()
    }
}

/// The SWAPPABLE part: how history is stored and how `seek` reconstructs a
/// frame. Strategy choice is runtime config; the loop names only this trait.
pub trait Timeline<S: Simulatable> {
    /// Record frame `frame` — `sim` pre-step, `cmds` about to be applied.
    ///
    /// Frames must be recorded consecutively (each call `= last + 1`).
    /// Recording after a `seek` (rewind-then-*branch*) additionally requires
    /// truncating the forward history — that lands with the pause/rewind
    /// culmination (Phase 6); until then, resume a seek by replaying
    /// [`Timeline::commands_since`], not by re-recording.
    fn record(&mut self, frame: Frame, sim: &S, cmds: &[S::Command]);
    /// Restore `sim` to the pre-step state of `frame`.
    ///
    /// Panics if `frame` was never recorded or has been pruned away.
    fn seek(&mut self, frame: Frame, sim: &mut S);
    /// The recorded commands for every frame `>= frame`, in frame order.
    ///
    /// Panics if `frame` predates the (possibly pruned) history — a silently
    /// clamped window would replay wrong-frame commands. `frame` one past the
    /// last recorded frame yields an empty slice.
    fn commands_since(&self, frame: Frame) -> &[Vec<S::Command>];
    /// Drop history before `frame` (memory bound / server-confirmed), keeping
    /// enough that every frame `>= frame` stays seekable. How much can
    /// actually be dropped is strategy-dependent: the floor is the newest
    /// snapshot at or before `frame` (for the replay-only cadence that is the
    /// first frame, so `prune` is a documented no-op).
    fn prune(&mut self, frame: Frame);
}

/// The one [`Timeline`] implementation: a contiguous per-frame command log
/// plus snapshots at a fixed cadence. The doc's three strategies are
/// *cadences* of this single type, picked by constructor — they differ in
/// nothing else.
pub struct TimelineLog<S: Simulatable> {
    /// Snapshot every `interval` recorded frames (1 = every frame).
    interval: u64,
    /// Frame number of `commands[0]`; meaningless until `commands` is
    /// non-empty.
    base: Frame,
    commands: Vec<Vec<S::Command>>,
    keyframes: BTreeMap<Frame, S::Snapshot>,
}

impl<S: Simulatable> TimelineLog<S>
where
    S::Command: Clone,
{
    /// The default strategy (`KeyframeLog`): snapshot every `interval` frames
    /// + always log commands; `seek` restores the nearest keyframe ≤ frame and
    /// steps forward. Bounded memory *and* bounded seek.
    pub fn keyframes(interval: u64) -> TimelineLog<S> {
        assert!(interval > 0, "keyframe interval must be at least 1");
        TimelineLog {
            interval,
            base: 0,
            commands: Vec::new(),
            keyframes: BTreeMap::new(),
        }
    }

    /// `SnapshotRing`: a snapshot every frame — O(1) seek, heavy memory. The
    /// oracle in the strategy-equivalence golden.
    pub fn snapshot_ring() -> TimelineLog<S> {
        TimelineLog::keyframes(1)
    }

    /// `ReplayOnly`: one snapshot at the first recorded frame; `seek` replays
    /// from it. Lightest memory, leans hardest on determinism. `prune` is a
    /// no-op (the base snapshot is the only restore point).
    pub fn replay_only() -> TimelineLog<S> {
        TimelineLog::keyframes(u64::MAX)
    }

    /// Drop all recorded history at and after `frame` — the record-after-seek
    /// truncation rewind-then-BRANCH needs (docs/physics.md, the culmination):
    /// after `seek(f)`, `truncate_from(f)` makes `record(f, …)` legal again,
    /// and the old future is gone. Keyframes at or after `frame` go too (a
    /// re-recorded frame re-snapshots on its cadence).
    pub fn truncate_from(&mut self, frame: Frame) {
        let Some(next) = self.next_frame() else { return };
        if frame >= next {
            return;
        }
        assert!(
            frame >= self.base,
            "truncate_from({frame}) predates recorded history (base {})",
            self.base
        );
        self.commands.truncate((frame - self.base) as usize);
        self.keyframes.retain(|&k, _| k < frame);
    }

    /// The seekable range `(oldest, newest)` — `None` until something is
    /// recorded. Prune moves the floor; truncate moves the ceiling.
    pub fn recorded_range(&self) -> Option<(Frame, Frame)> {
        self.next_frame().map(|next| (self.base, next - 1))
    }

    fn next_frame(&self) -> Option<Frame> {
        (!self.commands.is_empty()).then(|| self.base + self.commands.len() as u64)
    }
}

impl<S: Simulatable> Timeline<S> for TimelineLog<S>
where
    S::Command: Clone,
{
    fn record(&mut self, frame: Frame, sim: &S, cmds: &[S::Command]) {
        match self.next_frame() {
            None => self.base = frame,
            Some(next) => assert_eq!(
                frame, next,
                "timeline frames must be recorded consecutively"
            ),
        }
        if (frame - self.base) % self.interval == 0 {
            self.keyframes.insert(frame, sim.snapshot());
        }
        self.commands.push(cmds.to_vec());
    }

    fn seek(&mut self, frame: Frame, sim: &mut S) {
        let next = self.next_frame().expect("seek on an empty timeline");
        assert!(
            frame >= self.base && frame < next,
            "seek({frame}) outside recorded history [{}, {})",
            self.base,
            next
        );
        // Restore the nearest keyframe at or before `frame`, then re-step with
        // the recorded commands. Determinism makes this land bit-exact.
        let (&kf, snapshot) = self
            .keyframes
            .range(..=frame)
            .next_back()
            .expect("no keyframe at or below a recorded frame (pruned?)");
        sim.restore(snapshot);
        for f in kf..frame {
            sim.step(&self.commands[(f - self.base) as usize]);
        }
    }

    fn commands_since(&self, frame: Frame) -> &[Vec<S::Command>] {
        let Some(next) = self.next_frame() else {
            return &[]; // nothing recorded yet
        };
        assert!(
            frame >= self.base && frame <= next,
            "commands_since({frame}) outside recorded history [{}, {}]",
            self.base,
            next
        );
        &self.commands[(frame - self.base) as usize..]
    }

    fn prune(&mut self, frame: Frame) {
        // The new floor is the greatest keyframe <= frame, so `frame` itself
        // (and everything after) stays seekable.
        let Some((&floor, _)) = self.keyframes.range(..=frame).next_back() else {
            return;
        };
        self.keyframes = self.keyframes.split_off(&floor);
        self.commands.drain(..(floor - self.base) as usize);
        self.base = floor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{Body, Shape, DEFAULT_GRAVITY};

    /// A tiny scripted world: one falling crate, teleported at frame 5.
    fn cmds_at(frame: Frame) -> Vec<Command> {
        let pos = if frame < 5 {
            [0.0, 5.0, 0.0]
        } else {
            [2.0, 8.0, 0.0]
        };
        let body = Body::dynamic(
            "a".to_string(),
            Shape::Cuboid {
                extents: [1.0, 1.0, 1.0],
            },
        )
        .at(pos);
        vec![Command::DeclareScene(PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![body],
        ))]
    }

    fn drive<T: Timeline<World>>(tl: &mut T, sim: &mut World, from: Frame, to: Frame) {
        for f in from..to {
            let cmds = cmds_at(f);
            tl.record(f, sim, &cmds);
            sim.step(&cmds);
        }
    }

    #[test]
    fn seek_restores_a_recorded_frame_bit_exact() {
        let mut tl = TimelineLog::keyframes(4);
        let mut sim = World::new(DEFAULT_GRAVITY);
        let mut at_7 = None;
        for f in 0..20 {
            if f == 7 {
                at_7 = Some(Simulatable::snapshot(&sim));
            }
            let cmds = cmds_at(f);
            tl.record(f, &sim, &cmds);
            sim.step(&cmds);
        }

        // 7 is not a keyframe (cadence 4): seek restores keyframe 4 and
        // re-steps 4..7.
        tl.seek(7, &mut sim);
        assert!(
            Simulatable::snapshot(&sim) == at_7.expect("captured frame 7"),
            "seek(7) diverged"
        );
    }

    #[test]
    fn seek_then_resim_forward_matches_the_original_run() {
        let mut tl = TimelineLog::keyframes(8);
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 20);
        let live_end = Simulatable::snapshot(&sim);

        // Rewind to 10, then re-step with the recorded commands (the netcode
        // reconcile shape: seek K, replay commands K..now).
        tl.seek(10, &mut sim);
        for cmds in tl.commands_since(10).to_vec() {
            sim.step(&cmds);
        }
        assert!(
            Simulatable::snapshot(&sim) == live_end,
            "seek + command replay diverged from the live run"
        );
    }

    #[test]
    fn prune_keeps_later_frames_seekable() {
        let mut tl = TimelineLog::keyframes(4);
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 20);
        let at_10 = {
            let mut probe = World::new(DEFAULT_GRAVITY);
            tl.seek(10, &mut probe);
            Simulatable::snapshot(&probe)
        };

        tl.prune(10);
        let mut probe = World::new(DEFAULT_GRAVITY);
        tl.seek(10, &mut probe);
        assert!(Simulatable::snapshot(&probe) == at_10, "prune broke seek(10)");
        // The floor keyframe is 8, so frames 8..20 (12 of them) remain.
        assert_eq!(tl.commands_since(8).len(), 12);
    }

    #[test]
    fn commands_since_one_past_the_end_is_empty() {
        let mut tl = TimelineLog::keyframes(4);
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 8);
        assert!(tl.commands_since(8).is_empty());
        assert_eq!(tl.commands_since(6).len(), 2);
    }

    #[test]
    #[should_panic(expected = "outside recorded history")]
    fn commands_since_before_pruned_history_panics() {
        let mut tl = TimelineLog::keyframes(4);
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 8);
        tl.prune(6); // floor = keyframe 4
        tl.commands_since(2);
    }

    #[test]
    fn replay_only_prune_is_a_no_op() {
        let mut tl = TimelineLog::replay_only();
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 10);
        tl.prune(8);
        // The base snapshot is the only restore point, so nothing was dropped
        // and every frame stays seekable.
        assert_eq!(tl.commands_since(0).len(), 10);
        let mut probe = World::new(DEFAULT_GRAVITY);
        tl.seek(9, &mut probe);
    }

    #[test]
    #[should_panic(expected = "outside recorded history")]
    fn seek_before_recorded_history_panics() {
        let mut tl = TimelineLog::keyframes(4);
        let mut sim = World::new(DEFAULT_GRAVITY);
        drive(&mut tl, &mut sim, 0, 8);
        tl.prune(6);
        tl.seek(2, &mut sim);
    }

    #[test]
    #[should_panic(expected = "recorded consecutively")]
    fn non_consecutive_record_panics() {
        let mut tl = TimelineLog::snapshot_ring();
        let sim = World::new(DEFAULT_GRAVITY);
        tl.record(0, &sim, &cmds_at(0));
        tl.record(2, &sim, &cmds_at(2));
    }
}
