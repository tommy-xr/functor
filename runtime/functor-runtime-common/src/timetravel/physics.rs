use super::{
    args_digest, Frame, FrameEnd, History, Live, Participant, Restore, DEFAULT_HISTORY_FRAMES,
};
use crate::physics::PhysicsCommand;

/// Where the physics world stood when a rendered frame ended: its fixed frame
/// and the commands queued after that frame's step (an impulse a raycast
/// answer returned). The physics timeline records those as the NEXT frame's
/// input, so restoring this frame must put them back in flight.
#[derive(Clone)]
pub(super) struct WorldEnd {
    pub(super) fixed: u64,
    pub(super) in_flight: Vec<PhysicsCommand>,
}

/// Participant 1: the physics world, coupled by where it ended each rendered
/// frame. Its own keyframe log lives in [`SteppedPhysics`](crate::physics::SteppedPhysics).
pub(super) struct PhysicsParticipant {
    pub(super) world_frame_history: History<WorldEnd>,
}

impl PhysicsParticipant {
    /// Resolve the physics fixed-frame to seek for rendered `frame` WITHOUT
    /// mutating: `Ok(None)` = no seek needed (no physics, or the frame's end-
    /// state is already the live append), `Ok(Some(fixed))` = exact seek,
    /// `Err` = pruned (refuse rather than desync). Compares against the newest
    /// RECORDED frame, not the live world frame — after a non-destructive scrub
    /// the world is parked mid-history, and using the live frame would skip the
    /// truncate on the branch commit and panic on the next (non-consecutive)
    /// record. The floor is [`SteppedPhysics::seekable_range`]'s (1, not 0).
    fn physics_seek_target(&self, frame: u64, live: &Live) -> Result<Option<u64>, String> {
        if !live.has_physics {
            return Ok(None);
        }
        let want = self.world_frame_history.seek(frame).fixed;
        match live.physics.seekable_range() {
            None => Ok(None),
            // The zero-substep tail: the frame ended at the live tail. No seek
            // if the world is still there; after a scrub parked it
            // mid-history, restore the tail, or the next record would skip the
            // frames in between.
            Some((_, hi)) if want > hi => {
                debug_assert_eq!(want, hi + 1, "a rendered frame ended past the live tail");
                Ok((live.physics.current_fixed_frame() != want).then_some(want))
            }
            Some((flo, hi)) if want >= flo && want <= hi => Ok(Some(want)),
            _ => Err(format!(
                "cannot seek to rendered frame {frame}: its physics frame {want} has \
                 been pruned from the {DEFAULT_HISTORY_FRAMES}-frame world history"
            )),
        }
    }
}

impl Participant for PhysicsParticipant {
    fn name(&self) -> &'static str {
        "physics"
    }

    fn record(&mut self, frame: Frame, end: &FrameEnd) {
        self.world_frame_history.record_owned(
            frame,
            WorldEnd {
                fixed: end.physics_frame,
                in_flight: end.in_flight.to_vec(),
            },
        );
    }

    fn range(&self) -> Option<(Frame, Frame)> {
        self.world_frame_history.recorded_range()
    }

    fn check_restore(&self, frame: Frame, live: &Live) -> Result<(), String> {
        self.physics_seek_target(frame, live).map(|_| ())
    }

    fn restore(&mut self, frame: Frame, how: Restore, live: &mut Live) {
        let target = self
            .physics_seek_target(frame, live)
            .expect("restore only after a passing check_restore");
        if let Some(fixed) = target {
            // Warnings are empty on every reachable coupled seek:
            // `physics_seek_target` already validated `fixed`.
            let _ = match how {
                Restore::Seek => live.physics.seek_to_frame(fixed),
                Restore::Rewind | Restore::Resume => live.physics.rewind_to_frame(fixed),
            };
            *live.physics_frame = live.physics.current_fixed_frame();
        }
        if live.has_physics && how != Restore::Resume {
            // Also drops any command queued since: the world shows `frame`.
            live.physics
                .set_pending_commands(&self.world_frame_history.seek(frame).in_flight);
        }
    }

    fn truncate_from(&mut self, frame: Frame) {
        self.world_frame_history.truncate_from(frame);
    }

    fn prune(&mut self, floor: Frame) {
        self.world_frame_history.prune_before(floor);
    }

    fn clear(&mut self) {
        self.world_frame_history = History::unbounded();
    }

    fn recorded_digest(&self, frame: Frame) -> Option<u64> {
        let end = self.world_frame_history.get(frame)?;
        Some(args_digest(&(end.fixed, &end.in_flight)))
    }

    fn live_digest(&self, live: &Live) -> u64 {
        // As the producer records it: no commands without the hook.
        let in_flight = match live.has_physics {
            true => live.physics.pending_commands(),
            false => Vec::new(),
        };
        args_digest(&(*live.physics_frame, &in_flight))
    }
}
