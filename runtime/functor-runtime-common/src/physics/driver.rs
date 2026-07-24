//! The recorded physics drive (docs/physics.md, Phase 6): per-fixed-frame
//! recording over the 1b `Timeline` seam, powering the shell-owned scene
//! rewind/seek (docs/time-travel.md T1/T3).
//!
//! [`SteppedPhysics`] replaces the drivers' direct `reconcile + step_frame`
//! call with per-fixed-frame recording: each substep is recorded through
//! [`TimelineLog`] with exactly the [`Command`]s that produce it (the frame's
//! declared scene on its first substep, plus any queued
//! [`super::PhysicsCommand`]s), then stepped through `Simulatable::step` —
//! the SAME path a `seek` replays, so live and replayed frames are
//! byte-identical by construction (the strategy-equivalence goldens' claim,
//! now load-bearing at runtime).
//!
//! Rewind/seek is driven by the SHELL (the scrubber's coupled scene restore,
//! via [`SteppedPhysics::rewind_to_frame`] / [`SteppedPhysics::seek_to_frame`]),
//! not by game code — the game-authored timeline-control effects
//! (`Physics.pause`/`rewindTo`/…) were removed when the whole-game scrubber
//! superseded them.

use super::{
    with_world, Command, PhysicsEvent, PhysicsScene, Simulatable, Timeline, TimelineLog, World,
    WorldId, DEFAULT_WORLD, FIXED_DT, MAX_SUBSTEPS_PER_FRAME,
};

/// Snapshot cadence for the live recorder: every half second at 60Hz. Seeks
/// replay at most 29 frames — imperceptible — while snapshots stay rare.
const KEYFRAME_INTERVAL: u64 = 30;

/// How much history the recorder keeps: 15 seconds at 60Hz. Pruned each
/// frame, so memory is bounded no matter how long the game runs.
const HISTORY_FRAMES: u64 = 900;

/// What one [`SteppedPhysics::advance`] did, for the driver to surface.
pub struct Advanced {
    /// Fixed substeps simulated this frame (0 when the accumulator hasn't
    /// reached a full step).
    pub steps: u32,
    /// The frame's contact transitions.
    pub events: Vec<PhysicsEvent>,
    /// Command problems to report (deduped by the driver).
    pub warnings: Vec<String>,
    /// The world's current fixed frame after this advance (what the coupled
    /// scene recorder stores per rendered frame).
    pub frame: u64,
}

/// The per-driver recorded physics drive. Owns the fixed-step accumulator and
/// the `TimelineLog` — the `World` itself stays in the registry (shared with
/// reads, wireframes, and hot reload).
pub struct SteppedPhysics {
    world: WorldId,
    timeline: TimelineLog<World>,
    accumulator: f32,
    /// Whether any frame has simulated yet. The first advance is floored to
    /// one step, so the world is reconciled+stepped before the first draw
    /// even when the first frame's dt is short — otherwise draw-time reads
    /// (`Physics.position`/`transformed`) would find an empty world.
    /// (Reconcile lives INSIDE the recorded step so replay reproduces it
    /// byte-exact, so we can't reconcile eagerly for draw — we step once
    /// instead.)
    started: bool,
}

impl Default for SteppedPhysics {
    fn default() -> SteppedPhysics {
        SteppedPhysics::new()
    }
}

impl SteppedPhysics {
    pub fn new() -> SteppedPhysics {
        SteppedPhysics {
            world: DEFAULT_WORLD,
            timeline: TimelineLog::keyframes(KEYFRAME_INTERVAL),
            accumulator: 0.0,
            started: false,
        }
    }

    /// Build a THROWAWAY dry-run recorder over an EXPLICIT world id, with a
    /// fresh `TimelineLog` — the dry-run forward-step (docs/time-travel.md T6b)
    /// points this at a throwaway world (a restored snapshot of
    /// [`DEFAULT_WORLD`]) so it can step the scene forward WITHOUT touching the
    /// live world/driver/timeline.
    ///
    /// The live driver's sub-frame state is deliberately NOT forked: `started`
    /// is false (so the first division floors to one fixed step — the `new`
    /// bootstrap) and `accumulator` is 0. So the projection matches a live
    /// continuation exactly only for a fork driven at a full `FIXED_DT` with no
    /// accumulator residue (the case the T6b test covers). Forking a
    /// sub-`FIXED_DT` accumulator phase, or carrying timeline history so the
    /// projection itself could be scrubbed, is follow-up work.
    pub fn for_world(world: WorldId) -> SteppedPhysics {
        SteppedPhysics {
            world,
            timeline: TimelineLog::keyframes(KEYFRAME_INTERVAL),
            accumulator: 0.0,
            started: false,
        }
    }

    /// Byte-exact snapshot of this recorder's world (the determinism oracle
    /// the goldens assert) — the forward-step samples it after each stepped
    /// division. `None` when the world does not exist.
    pub fn snapshot_world(&self) -> Option<super::PhysicsSnapshot> {
        with_world(self.world, |w| w.checkpoint())
    }

    /// One rendered frame's worth of recorded physics: simulate whole fixed
    /// substeps from `real_dt`, recording each through the Timeline. The
    /// declared `scene` reconciles on the frame's first recorded substep —
    /// identical semantics to `World::step_frame`, but every fixed frame is
    /// now seekable.
    pub fn advance(&mut self, scene: &PhysicsScene, real_dt: f32) -> Advanced {
        let mut out = Advanced {
            steps: 0,
            events: Vec::new(),
            warnings: Vec::new(),
            frame: 0,
        };

        self.accumulator += real_dt.max(0.0);
        // Bootstrap: guarantee the very first frame simulates one step so
        // the world exists for the first draw. Floors, never adds — a
        // full-dt first frame (the tests) still yields exactly one step.
        if !self.started {
            self.accumulator = self.accumulator.max(FIXED_DT);
        }
        let simulate = self.accumulator >= FIXED_DT;
        if simulate {
            self.started = true;
        }

        if simulate {
            let whole = (self.accumulator / FIXED_DT) as u32;
            let steps = whole.min(MAX_SUBSTEPS_PER_FRAME);
            self.accumulator -= steps as f32 * FIXED_DT;
            if whole > MAX_SUBSTEPS_PER_FRAME {
                // Hitch: drop the whole-step backlog, keep the phase
                // (the same discipline as World::step_frame).
                self.accumulator %= FIXED_DT;
            }
            // Resolve terrain once at the live shell boundary, then move that
            // exact immutable scene into the command log. Replays must never
            // consult whatever asset revision happens to be loaded later.
            let mut scene = Some(scene.hydrated_heightfields());
            let (events, warnings) = with_world(self.world, |w| {
                let mut events = Vec::new();
                for i in 0..steps {
                    // The declared scene reconciles on the first substep; a
                    // repeat declaration is a no-op via the divergence cache,
                    // so recording it per-frame would also be correct — but
                    // one DeclareScene per rendered frame keeps the log lean.
                    let mut cmds: Vec<Command> = Vec::new();
                    if i == 0 {
                        cmds.push(Command::DeclareScene(
                            scene.take().expect("first fixed substep owns the scene"),
                        ));
                        for command in w.take_pending_commands() {
                            cmds.push(Command::Apply(command));
                        }
                    }
                    self.timeline.record(w.frame(), w, &cmds);
                    events.extend(w.step(&cmds));
                }
                (events, w.take_command_warnings())
            })
            .unwrap_or((Vec::new(), Vec::new()));
            out.steps = steps;
            out.events = events;
            out.warnings.extend(warnings);
            self.timeline
                .prune(self.current_frame().saturating_sub(HISTORY_FRAMES));
        }

        out.frame = self.current_frame();
        out
    }

    /// Rewind the recorded world to a fixed frame — the coupled scene-rewind
    /// path (docs/time-travel.md T1), called in-process by the shell when a
    /// time-travel branch commits. Returns any clamp/range warnings. The
    /// coupled caller checks [`Self::seekable_range`] first so this only ever
    /// runs on an exactly-restorable frame.
    pub fn rewind_to_frame(&mut self, frame: u64) -> Vec<String> {
        let mut warnings = Vec::new();
        self.rewind_to(frame, &mut warnings);
        warnings
    }

    /// Seek the recorded world to a fixed frame WITHOUT truncating the future
    /// (docs/time-travel.md T3, the draggable scrubber): unlike
    /// [`Self::rewind_to_frame`], the recorded history is left intact, so the
    /// caller can seek back and forth freely while paused. Branching (discarding
    /// the future) only happens later, when play resumes from a scrubbed point
    /// via `rewind_to_frame`. Returns clamp/range warnings.
    pub fn seek_to_frame(&mut self, frame: u64) -> Vec<String> {
        let mut warnings = Vec::new();
        let (lo, hi) = match self.recorded_range() {
            Some(range) => range,
            None => {
                warnings.push("physics seek: nothing recorded yet".to_string());
                return warnings;
            }
        };
        if hi == 0 {
            warnings.push("physics seek: no stepped frame recorded yet".to_string());
            return warnings;
        }
        let floor = if lo == 0 { 1 } else { lo };
        let target = frame.clamp(floor, hi);
        with_world(self.world, |w| {
            self.timeline.seek(target, w);
            // The replayed steps re-emit events/warnings nobody should re-observe.
            let _ = w.take_events();
            let _ = w.take_command_warnings();
        });
        warnings
    }

    /// The fixed-frame range a rewind can restore EXACTLY: the practical floor
    /// (frame 0's pre-step is the empty pre-reconcile world, so 1 is the real
    /// floor) through the newest recorded frame. `None` until something has
    /// stepped. The coupled scene rewind (docs/time-travel.md T1) uses this to
    /// refuse rather than silently clamp — a clamp would land the world on a
    /// different frame than the model, desyncing the two.
    pub fn seekable_range(&self) -> Option<(u64, u64)> {
        self.recorded_range()
            .map(|(lo, hi)| (if lo == 0 { 1 } else { lo }, hi))
    }

    /// The world's current live fixed frame (== the next frame to record). A
    /// rendered frame whose recorded fixed frame equals this had no physics
    /// step after it, so its end-of-frame world IS the live world — the coupled
    /// rewind needs no physics seek for it.
    pub fn current_fixed_frame(&self) -> u64 {
        self.current_frame()
    }

    fn rewind_to(&mut self, frame: u64, warnings: &mut Vec<String>) {
        let (lo, hi) = match self.recorded_range() {
            Some(range) => range,
            None => {
                warnings.push("physics rewind: nothing recorded yet".to_string());
                return;
            }
        };
        if hi == 0 {
            // Only the empty frame-0 exists — nothing meaningful to seek to
            // (frame 0's pre-step state is the empty world, before any
            // reconcile, which draw-reads can't use).
            warnings.push("physics rewind: no stepped frame recorded yet".to_string());
            return;
        }
        // Pre-step of fixed frame 0 is the empty world, so the practical floor
        // is frame 1 (= the world after its first step).
        let floor = if lo == 0 { 1 } else { lo };
        let target = frame.clamp(floor, hi);
        with_world(self.world, |w| {
            // Commands queued THIS frame (before the rewind) survive the
            // restore: a rewind+command in one frame lands the command at the
            // branch's first step, rather than the snapshot silently dropping
            // it. (Two reviewers flagged the drop.)
            let carried = w.take_pending_commands();
            self.timeline.seek(target, w);
            // A seek re-simulates from a keyframe; the replayed steps emit
            // contact events and command warnings nobody should re-observe.
            let _ = w.take_events();
            let _ = w.take_command_warnings();
            for command in carried {
                w.queue_command(command);
            }
        });
        // The old future is gone: recording resumes (and BRANCHES) from here.
        self.timeline.truncate_from(target);
    }

    fn current_frame(&self) -> u64 {
        with_world(self.world, |w| w.frame()).unwrap_or(0)
    }

    /// The seekable fixed-frame range `(oldest, newest)`, if anything has
    /// been recorded.
    fn recorded_range(&self) -> Option<(u64, u64)> {
        self.timeline.recorded_range()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::asset::pipelines::HeightmapData;
    use crate::physics::{remove_world, Body, Shape};
    use crate::terrain::{TerrainGeometry, TerrainSource};

    fn scene_at(t: u64) -> PhysicsScene {
        PhysicsScene::create(
            [0.0, -9.81, 0.0],
            vec![
                Body::fixed(
                    "ground".to_string(),
                    Shape::Cuboid {
                        extents: [20.0, 0.4, 20.0],
                    },
                ),
                Body::dynamic(
                    "a".to_string(),
                    Shape::Cuboid {
                        extents: [1.0, 1.0, 1.0],
                    },
                )
                .at(if t < 30 { [0.0, 4.0, 0.0] } else { [0.5, 5.0, 0.0] }),
            ],
        )
    }

    fn snapshot() -> Vec<u8> {
        with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap()
    }

    fn fresh() -> SteppedPhysics {
        remove_world(DEFAULT_WORLD);
        SteppedPhysics::new()
    }

    #[test]
    fn rewind_restores_and_replay_is_byte_identical() {
        let mut sp = fresh();
        // Run A: 0..40 rendered frames (one substep each), snapshotting the
        // world state at frame 20 and the end.
        let mut snap_20 = Vec::new();
        for t in 0..40 {
            if t == 20 {
                snap_20 = snapshot();
            }
            sp.advance(&scene_at(t), FIXED_DT);
        }
        let end_a = snapshot();

        // Rewind to fixed frame 20 (pre-step state of 20 == post-step of 19
        // == what we snapshotted before advancing frame 20) — the shell's
        // coupled scene-rewind path.
        let warnings = sp.rewind_to_frame(20);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            snapshot() == snap_20,
            "rewind must restore the recorded frame byte-exact"
        );

        // Replay the same declared scenes: local determinism makes the branch
        // land byte-identical to run A.
        for t in 20..40 {
            sp.advance(&scene_at(t), FIXED_DT);
        }
        assert!(
            snapshot() == end_a,
            "replaying identical inputs must reproduce run A"
        );
    }

    #[test]
    fn rewind_clamps_to_recorded_history() {
        let mut sp = fresh();
        // Nothing recorded yet: rewind warns instead of panicking.
        let warnings = sp.rewind_to_frame(3);
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        for t in 0..5 {
            sp.advance(&scene_at(t), FIXED_DT);
        }
        // Past the newest end: clamped, no warning. (Rewinding to the OLDEST
        // frame truncates the entire history — by design: the future is
        // discarded — so the far-future clamp is tested first.)
        let warnings = sp.rewind_to_frame(9_999);
        assert!(warnings.is_empty(), "{warnings:?}");
        // Below the recorded floor: clamped to it, no warning, no panic.
        let warnings = sp.rewind_to_frame(0);
        assert!(warnings.is_empty(), "{warnings:?}");
        remove_world(DEFAULT_WORLD);
    }

    #[test]
    fn command_queued_with_a_rewind_lands_on_the_branch() {
        let mut sp = fresh();
        for t in 0..20 {
            sp.advance(&scene_at(t), FIXED_DT);
        }
        // Same frame: queue an impulse, then rewind to 10. The impulse must
        // survive the seek and apply at the branch's first step.
        with_world(DEFAULT_WORLD, |w| {
            w.queue_command(crate::physics::PhysicsCommand::ApplyImpulse {
                tag: "a".to_string(),
                impulse: [5.0, 0.0, 0.0],
            })
        });
        let warnings = sp.rewind_to_frame(10);
        assert!(warnings.is_empty(), "{warnings:?}");
        let out = sp.advance(&scene_at(20), FIXED_DT);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        // The branch's first step applied the +x impulse.
        let vx = with_world(DEFAULT_WORLD, |w| w.body_velocity("a").unwrap()[0]).unwrap();
        assert!(vx > 0.0, "the same-frame impulse was dropped by the rewind: vx={vx}");
        remove_world(DEFAULT_WORLD);
    }

    #[test]
    fn recorded_commands_replay_through_a_seek() {
        let mut sp = fresh();
        // Impulse at fixed frame 10; snapshot the pre-step state of 20.
        let mut snap_20 = Vec::new();
        for t in 0..30 {
            if t == 10 {
                with_world(DEFAULT_WORLD, |w| {
                    w.queue_command(crate::physics::PhysicsCommand::ApplyImpulse {
                        tag: "a".to_string(),
                        impulse: [2.0, 3.0, 0.0],
                    })
                });
            }
            if t == 20 {
                snap_20 = snapshot();
            }
            sp.advance(&scene_at(t), FIXED_DT);
        }
        // Rewinding to 20 seeks: keyframe 0 restored, frames 0..19 replayed
        // FROM THE LOG — including frame 10's recorded Command::Apply. Landing
        // byte-identical proves commands replay. (Post-rewind the future is
        // truncated — a resumed run is a BRANCH and only re-runs what the
        // game issues again; that is the design, not a loss.)
        let warnings = sp.rewind_to_frame(20);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            snapshot() == snap_20,
            "seek must re-apply recorded commands to land byte-exact"
        );
        remove_world(DEFAULT_WORLD);
    }

    #[test]
    fn replay_uses_the_heightmap_revision_recorded_live() {
        let mut sp = fresh();
        let source = TerrainSource {
            locator: "timeline-heightfield.png".to_string(),
            while_pending: Vec::new(),
        };
        let scene = PhysicsScene::create(
            [0.0, -9.81, 0.0],
            vec![Body::fixed(
                "terrain".to_string(),
                Shape::Heightfield {
                    geometry: TerrainGeometry {
                        source: source.clone(),
                        width: 20.0,
                        depth: 20.0,
                        min_height: 0.0,
                        max_height: 10.0,
                    },
                    data: None,
                },
            )],
        );
        let samples = |sample, revision| {
            Arc::new(HeightmapData {
                samples: vec![sample; 4],
                width: 2,
                height: 2,
                revision,
            })
        };
        let hit_y = || {
            with_world(DEFAULT_WORLD, |world| {
                world
                    .raycast([0.0, 20.0, 0.0], [0.0, -1.0, 0.0], 30.0)
                    .expect("terrain ray hit")
                    .position[1]
            })
            .unwrap()
        };

        let first_samples = samples(16384, 1);
        crate::terrain::publish_heightmap(&source, first_samples.clone());
        sp.advance(&scene, FIXED_DT);
        let first_height = hit_y();

        let reloaded_samples = samples(49152, 2);
        crate::terrain::publish_heightmap(&source, reloaded_samples.clone());
        sp.advance(&scene, FIXED_DT);
        let reloaded_height = hit_y();
        assert!(
            reloaded_height > first_height + 4.0,
            "{first_height} -> {reloaded_height}"
        );

        let warnings = sp.seek_to_frame(1);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            (hit_y() - first_height).abs() < 0.001,
            "replay consulted the newer global heightmap revision"
        );
        remove_world(DEFAULT_WORLD);
    }
}
