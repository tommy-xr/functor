//! The model half of the time-travel Timeline (docs/time-travel.md, T1).
//!
//! [`History`] is a bounded, per-frame snapshot ring of a cheaply-cloneable
//! state — in Functor that state is the Functor Lang `model` (`functor_lang::Value`). It is the
//! counterpart of the physics [`crate::physics::timeline::TimelineLog`], but
//! deliberately simpler:
//!
//! - **The physics `TimelineLog` keyframes + replays** because its snapshot (a
//!   whole serialized Rapier world) is expensive, so it stores one every N
//!   frames and re-steps forward to reconstruct the rest. That leans on
//!   determinism.
//! - **`History` snapshots every frame directly.** The Functor Lang model is `Rc`-shared
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

pub mod fuzz;
mod history;
mod log;
mod model;
mod physics;
mod requests;

use functor_lang::Value;

use crate::input::RecordedInput;
use crate::physics::{PhysicsCommand, SteppedPhysics};
use crate::FrameTime;

pub use history::History;
use log::FrameLog;
pub use log::{args_digest, fnv1a, Entry, OpId};
pub use requests::{Arrival, Channel, Delivered, Requests};
use model::ModelParticipant;
use physics::PhysicsParticipant;

/// A fixed-step frame number — the same index space as
/// [`crate::physics::timeline::Frame`].
pub type Frame = u64;

/// What a successful code reload did to the seekable scene history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadHistory {
    /// Every retained model was plain data, so the coupled history remains
    /// seekable under the new program.
    Preserved,
    /// Code-bearing/callable model values required a new history generation.
    /// The rebound live scene was seeded as its first safe snapshot.
    RestartedAt(Frame),
}

/// Default rendered-frame retention (~15s at 60fps) — the one window the
/// [`SceneRecorder`] prunes every participant to, matching the physics
/// recorder (docs/time-travel.md T1).
pub const DEFAULT_HISTORY_FRAMES: usize = 900;

/// The oldest frame the retention window keeps once `frame` is recorded.
fn retention_floor(frame: Frame) -> Frame {
    (frame + 1).saturating_sub(DEFAULT_HISTORY_FRAMES as u64)
}

/// The producer-owned live state a restore writes into. The producer still
/// owns the model and the world; the recorder borrows them per operation.
pub(crate) struct Live<'a> {
    pub model: &'a mut Value,
    pub physics: &'a mut SteppedPhysics,
    /// The world's current fixed frame, as the producer tracks it.
    pub physics_frame: &'a mut u64,
    pub has_physics: bool,
}

/// What a rendered frame ended with: every participant's record input.
pub(crate) struct FrameEnd<'a> {
    pub model: &'a Value,
    pub physics_frame: u64,
    /// Physics commands queued for the next step.
    pub in_flight: &'a [PhysicsCommand],
}

/// How a restore treats live work queued since the frame (paused input's
/// model change, queued physics commands).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restore {
    /// Non-destructive scrub: live state becomes the frame's, queued work is
    /// dropped, the future stays recorded.
    Seek,
    /// Debug rewind: as `Seek`, then the future is truncated.
    Rewind,
    /// Resume from a scrub: queued work is KEPT (the input log records its
    /// input on the branch's first frame); the future is truncated.
    Resume,
}

/// One piece of state on the rendered-frame timeline (a "system" in the design
/// docs). The [`SceneRecorder`] applies every operation to all participants
/// together, so they can never land on different frames.
pub(crate) trait Participant {
    /// Short name for reports (`"model"`, `"physics"`).
    fn name(&self) -> &'static str;
    /// Record how `frame` ended. Frames arrive consecutively.
    fn record(&mut self, frame: Frame, end: &FrameEnd);
    /// Rendered frames retained, inclusive; `None` before the first record.
    fn range(&self) -> Option<(Frame, Frame)>;
    /// Refuse restoring `frame` BEFORE any participant mutates, so a coupled
    /// restore is exact-or-refused.
    fn check_restore(&self, _frame: Frame, _live: &Live) -> Result<(), String> {
        Ok(())
    }
    /// Put `frame`'s recorded state into `live`. Only after every
    /// participant's `check_restore` passed.
    fn restore(&mut self, frame: Frame, how: Restore, live: &mut Live);
    /// Drop recorded frames at and after `frame` (branching away the future).
    fn truncate_from(&mut self, frame: Frame);
    /// Drop recorded frames before `floor` (the bounded window).
    fn prune(&mut self, floor: Frame);
    /// Drop all history: a reload starting a new generation.
    fn clear(&mut self);
    /// Digest of what `frame` recorded; `None` outside [`Self::range`].
    fn recorded_digest(&self, frame: Frame) -> Option<u64>;
    /// Digest of `live` in [`Self::recorded_digest`]'s terms. The two agree
    /// right after `record`, and after a `Seek` or `Rewind` restore — the
    /// fuzzer checks every participant against that.
    fn live_digest(&self, live: &Live) -> u64;
}

/// One participant's retained range and digests at the selected frame (the
/// fuzzer's per-participant checks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParticipantState {
    pub name: &'static str,
    pub range: Option<(Frame, Frame)>,
    pub recorded: Option<u64>,
    pub live: u64,
}

/// The coupled time-travel recorder shared by both Functor Lang shells (docs/time-travel.md
/// T1–T3): the one owner of the rendered-frame clock. Each rendered frame it
/// records every [`Participant`] (the MVU `model`, the physics world) in
/// lockstep, and seeks/rewinds/commits them together. The producer keeps
/// ownership of the `model` and the [`SteppedPhysics`] and hands them in, so
/// this one implementation drives the desktop and web producers identically
/// (no drift).
///
/// A **scrub** ([`Self::seek_scene_to`]) is non-destructive so the draggable bar
/// can drag back and forth; the future is discarded only when play resumes from
/// the scrubbed point ([`Self::commit_scrub_if_resuming`]). Every coupled seek is **exact-or-refused** — it
/// never lands the model and world on different times.
pub struct SceneRecorder {
    model: ModelParticipant,
    physics: PhysicsParticipant,
    /// Participants beyond the model and physics (see [`Self::add_participant`]).
    others: Vec<Box<dyn Participant>>,
    /// The render clock `tts` (total time) of each rendered frame, recorded in
    /// LOCKSTEP with the participants. A scrubbed frame draws at its recorded
    /// `tts` so `tts`-driven visuals (orbiting lights, `sin(tts)` bobbing)
    /// rewind too, not just model/world state (docs/time-travel.md).
    tts_history: History<f64>,
    /// Exact per-frame `dts` / `tts`, retained for the current branch so a
    /// same-source reconstruction replays debugger steps and f32 clock rounding
    /// byte-identically even after the display-oriented `tts_history` prunes.
    clock_history: History<FrameTime>,
    /// The shell's one unrecorded bootstrap execution (`dts == 0`) before
    /// frame zero. It can still mutate a valid model, so reconstruction must
    /// replay it (and any input delivered before it) ahead of recorded frames.
    replay_prefix: Option<(FrameTime, Vec<Entry>)>,
    /// The frame log (docs/time-travel.md T6, "The event log"; design-systems
    /// §2.4): each rendered frame's input and effect ops, keyed in LOCKSTEP
    /// with the participants.
    log: FrameLog,
    /// The rendered-frame index the next snapshot records at; monotonic.
    rendered_frame: u64,
    /// The branch: bumps on every branch commit, and names ops (`OpId`).
    epoch: u64,
    /// In-flight ops and parked arrivals (design-systems §2.4).
    requests: Requests,
    /// Bumps whenever existing recorded frames may have been replaced or a
    /// reload starts a new snapshot generation. Timeline marker consumers use
    /// this to rebuild from authoritative input history instead of guessing a
    /// branch from range movement.
    generation: u64,
    /// Successful program swaps, including plain-data reloads that deliberately
    /// keep the same timeline generation. Preview caches key on this revision:
    /// a code edit changes extrapolation even when frame/range state does not.
    program_revision: u64,
    /// A plain-data reload preserved snapshots produced by the previous
    /// program. Input-only extrapolation rebuilds the retained model timeline
    /// from the input log under the new program instead of treating old-code
    /// snapshots as authoritative.
    counterfactual_replay: bool,
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
            model: ModelParticipant {
                model_history: History::unbounded(),
            },
            physics: PhysicsParticipant {
                world_frame_history: History::unbounded(),
            },
            others: Vec::new(),
            tts_history: History::unbounded(),
            clock_history: History::unbounded(),
            replay_prefix: None,
            log: FrameLog::new(),
            rendered_frame: 0,
            epoch: 0,
            requests: Requests::default(),
            generation: 0,
            program_revision: 0,
            counterfactual_replay: false,
            scrub_pos: None,
        }
    }

    /// Couple another participant to the timeline, before the first record:
    /// the seekable window is the intersection of every range.
    #[cfg(test)]
    fn add_participant(&mut self, participant: Box<dyn Participant>) {
        debug_assert_eq!(self.rendered_frame, 0, "add participants before recording");
        self.others.push(participant);
    }

    fn participants(&self) -> impl Iterator<Item = &dyn Participant> + '_ {
        [&self.model as &dyn Participant, &self.physics as &dyn Participant]
            .into_iter()
            .chain(self.others.iter().map(|p| p.as_ref()))
    }

    fn participants_mut(&mut self) -> impl Iterator<Item = &mut dyn Participant> + '_ {
        [
            &mut self.model as &mut dyn Participant,
            &mut self.physics as &mut dyn Participant,
        ]
        .into_iter()
        .chain(self.others.iter_mut().map(|p| p.as_mut() as &mut dyn Participant))
    }

    /// Every participant's [`ParticipantState`] at the selected frame.
    pub(crate) fn participant_states(&self, live: &Live) -> Vec<ParticipantState> {
        let frame = self.current_scene_frame();
        self.participants()
            .map(|p| ParticipantState {
                name: p.name(),
                range: p.range(),
                recorded: frame.and_then(|f| p.recorded_digest(f)),
                live: p.live_digest(live),
            })
            .collect()
    }

    /// The seekable window: the frames every participant retains.
    fn range(&self) -> Option<(Frame, Frame)> {
        let mut window: Option<(Frame, Frame)> = None;
        for range in self.participants().map(|p| p.range()) {
            let (lo, hi) = range?;
            window = Some(window.map_or((lo, hi), |(wlo, whi)| (wlo.max(lo), whi.min(hi))));
        }
        window.filter(|(lo, hi)| lo <= hi)
    }

    /// Clamp `target` to the window, then restore every participant to it —
    /// or refuse, touching nothing. Returns the restored frame.
    fn restore_all(
        &mut self,
        target: u64,
        how: Restore,
        live: &mut Live,
        verb: &str,
    ) -> Result<Frame, String> {
        let (lo, hi) = self
            .range()
            .ok_or_else(|| format!("{verb}: nothing recorded yet"))?;
        let frame = target.clamp(lo, hi);
        for participant in self.participants() {
            participant.check_restore(frame, live)?;
        }
        for participant in self.participants_mut() {
            participant.restore(frame, how, live);
        }
        if how != Restore::Resume {
            self.requests.drop_delta(self.rendered_frame);
        }
        Ok(frame)
    }

    /// Finish a successful hot reload after the producer has rebound its live
    /// model to the new module.
    ///
    /// Plain-data model snapshots carry no module IR, so their coupled model /
    /// world / time history remains valid and seekable under the new program.
    /// If any retained snapshot contains a callable or opaque host value, the
    /// old generation cannot cross the reload safely. In that case the rings are
    /// reset and immediately seeded at the current frame with the REBOUND live
    /// model, current world frame, and recorded game time. That anchor keeps the
    /// frame counter and transport stable while honestly making older frames
    /// unavailable. The plain-data input log always survives.
    pub fn finish_reload(
        &mut self,
        rebound_model: &Value,
        physics_fixed_frame: u64,
        live_model_was_safe: bool,
    ) -> ReloadHistory {
        self.program_revision = self.program_revision.wrapping_add(1);
        let current_frame = self.current_scene_frame();
        if live_model_was_safe && self.reload_history_is_safe() {
            // Snapshots stay as recorded. Input delivered while paused lives
            // in the rebound live model, which the next frame carries on from
            // (Resume keeps it); the input log records it on that frame.
            // Keep `scrub_pos`: reload is non-destructive, and Resume remains
            // the operation that branches away the recorded future.
            // Ordinary hot reload at the live tail preserves the live model;
            // only an explicitly scrubbed historical branch is counterfactual.
            self.counterfactual_replay = self
                .scrub_pos
                .zip(self.model.model_history.recorded_range())
                .is_some_and(|(selected, (_, hi))| {
                    selected < hi && self.log.supports_origin_replay_through(hi)
                });
            return ReloadHistory::Preserved;
        }

        self.counterfactual_replay = false;
        self.scrub_pos = None;
        let Some((frame, tts, in_flight)) = current_frame.map(|frame| {
            let tts = *self.tts_history.seek(frame);
            let in_flight = self.physics.world_frame_history.seek(frame).in_flight.clone();
            (frame, tts, in_flight)
        }) else {
            // An unsafe snapshot implies a non-empty history, so this is only a
            // defensive fallback if that invariant changes later.
            return ReloadHistory::Preserved;
        };
        self.tts_history = History::unbounded();
        self.generation = self.generation.wrapping_add(1);

        let anchor = FrameEnd {
            model: rebound_model,
            physics_frame: physics_fixed_frame,
            in_flight: &in_flight,
        };
        for participant in self.participants_mut() {
            participant.clear();
            participant.record(frame, &anchor);
        }
        self.tts_history.record(frame, &tts);
        self.requests.prune(frame + 1);
        ReloadHistory::RestartedAt(frame)
    }

    /// Whether every retained model snapshot is independent of the loaded
    /// module and can therefore remain seekable across a hot reload.
    pub fn reload_history_is_safe(&self) -> bool {
        Value::all_reload_safe(&self.model.model_history.snapshots)
    }

    /// Whether a reload now may restart history: a model snapshot, or the
    /// live `model`, holds code.
    pub fn reload_may_restart(&self, model: &Value) -> bool {
        !self.reload_history_is_safe() || !model.is_reload_safe_snapshot()
    }

    /// Classify and, when necessary, branch the coupled timeline before the
    /// producer rebinds its authoritative live model. The live model may have
    /// changed without a recorded frame while paused, so it participates in
    /// the safety decision independently of the snapshot ring.
    ///
    /// Branching keeps the authoritative live model (a [`Restore::Resume`]), so
    /// a paused update is not silently lost. Returns the live-model flag, to
    /// pass to [`Self::finish_reload`] after rebinding, and whether Δ was
    /// dropped (a connected scene returned to the newest frame instead): the
    /// producer then drops its input and queries since the frame too.
    pub fn prepare_reload(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
        prev_tts: &mut Option<f64>,
    ) -> (bool, bool) {
        let live_model_is_safe = model.is_reload_safe_snapshot();
        if !self.reload_may_restart(model) {
            return (live_model_is_safe, false);
        }
        let dropped =
            self.commit_scrub_before_reload(model, physics, physics_frame, has_physics, prev_tts);
        (model.is_reload_safe_snapshot(), dropped)
    }

    /// Monotonic revision for destructive branch/reload boundaries.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Monotonic successful-code-swap revision. Unlike [`Self::generation`], a
    /// safe plain-data reload bumps this too, because extrapolation changes even
    /// though the recorded frame range remains intact.
    pub fn program_revision(&self) -> u64 {
        self.program_revision
    }

    /// Configure origin replay for the program about to finish reloading.
    ///
    /// A program with `sampledInput` needs a sample for the zero-delta prefix
    /// and every recorded frame. A program without it can replay the same
    /// history without consuming those coeffects. Recompute from the current
    /// branch rather than preserving a past failure: rewinding before a gap
    /// makes the remaining origin-to-tail history complete again.
    pub fn configure_origin_replay_for_sampled_input(&mut self, required: bool) {
        let prefix_complete = self.replay_prefix.as_ref().is_none_or(|(_, entries)| {
            entries
                .iter()
                .any(|entry| matches!(entry, Entry::Input(RecordedInput::Snapshot(_))))
        });
        self.log.origin_replay_complete =
            !required || (prefix_complete && self.log.sampled_history_is_complete_from_origin());
    }

    /// Complete replay target after a safe reload. Sparse edge-input logs retain
    /// frame zero; sampled coeffect logs only enable this path while their
    /// bounded window still covers the session origin. Return a diagnostic
    /// instead of silently falling back if the sparse-log invariant is broken.
    pub fn counterfactual_replay_span(&self) -> Result<Option<(Frame, Frame, Frame)>, String> {
        if !self.counterfactual_replay {
            return Ok(None);
        }
        let (lo, hi) = self.model.model_history.recorded_range().ok_or_else(|| {
            "counterfactual replay unavailable: no model history is retained".to_string()
        })?;
        let (input_lo, input_hi) = self.log.recorded_range().ok_or_else(|| {
            "counterfactual replay unavailable: the session input log is empty".to_string()
        })?;
        if input_lo != 0 || input_hi < hi {
            return Err(format!(
                "counterfactual replay unavailable: input history covers frames \
                 {input_lo}..={input_hi}, not the required 0..={hi}"
            ));
        }
        let selected = self
            .current_scene_frame()
            .ok_or_else(|| "counterfactual replay unavailable: no selected frame".to_string())?;
        let (clock_lo, clock_hi) = self.clock_history.recorded_range().ok_or_else(|| {
            "counterfactual replay unavailable: the session clock log is empty".to_string()
        })?;
        if clock_lo != 0 || clock_hi < hi {
            return Err(format!(
                "counterfactual replay unavailable: clock history covers frames \
                 {clock_lo}..={clock_hi}, not the required 0..={hi}"
            ));
        }
        Ok(Some((lo, selected, hi)))
    }

    /// Replace the complete retained model timeline with models replayed under
    /// the current program, returning the rebuilt selected model — or
    /// `selected_override` unchanged when given (the live model holding input
    /// since the frame). The input,
    /// world-frame, and `tts` rings keep their existing frame alignment.
    pub fn materialize_counterfactual_history(
        &mut self,
        models: &[Value],
        selected_override: Option<&Value>,
    ) -> Result<Option<Value>, String> {
        let Some((lo, selected, hi)) = self.counterfactual_replay_span()? else {
            return Ok(None);
        };
        let retained = (hi - lo + 1) as usize;
        if models.len() != retained {
            return Err(format!(
                "counterfactual replay produced {} frames, expected {}",
                models.len(),
                retained
            ));
        }
        let (retained_lo, retained_hi) = self.model.model_history.recorded_range().ok_or_else(|| {
            "counterfactual replay cannot replace an empty model history".to_string()
        })?;
        if (retained_lo, retained_hi) != (lo, hi) {
            return Err(
                "counterfactual replay target changed while rebuilding history".to_string(),
            );
        }
        for (frame, rebuilt) in (lo..=hi).zip(models) {
            self.model.model_history.replace(frame, rebuilt);
        }
        self.counterfactual_replay = false;
        Ok(Some(
            selected_override
                .unwrap_or_else(|| self.model.model_history.seek(selected))
                .clone(),
        ))
    }

    /// Commit a non-destructive scrub before an UNSAFE reload replaces the
    /// snapshot generation. Safe plain-data reloads must not call this: they
    /// retain both the selected cursor and its recorded future until Resume.
    /// For an unsafe reload this keeps the live model the user is inspecting
    /// ([`Restore::Resume`]) and branches the physics timeline in lockstep
    /// before `scrub_pos` disappears.
    ///
    /// With a connection open that branch is refused (D4): the scene returns
    /// to the newest frame instead, dropping Δ (returns `true`).
    pub fn commit_scrub_before_reload(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
        prev_tts: &mut Option<f64>,
    ) -> bool {
        let Some(frame) = self.scrub_pos else { return false };
        let mut live = Live { model, physics, physics_frame, has_physics };
        if let Err(refused) = self.refuse_connected_branch(frame, "reload") {
            ::log::warn!("[functor-lang] {refused}; reloading at the newest frame");
            let newest = self.range().map_or(frame, |(_, hi)| hi);
            if let Ok(newest) = self.restore_all(newest, Restore::Seek, &mut live, "reload") {
                self.scrub_pos = Some(newest);
                return true;
            }
            return false;
        }
        let _ = self.branch_to(frame, &mut live, prev_tts, Restore::Resume);
        false
    }

    /// Rendered frame at which the next settled model will be recorded.
    pub fn next_frame(&self) -> u64 {
        self.rendered_frame
    }

    /// Record the settled `model`, the world's current fixed frame, and the
    /// render clock `tts` for this rendered frame. Call at the end of `tick`
    /// only when the sim advanced (`dts > 0`) — a paused frame would pile up
    /// frozen duplicates.
    pub fn record(&mut self, model: &Value, physics_fixed_frame: u64, tts: f64) {
        self.record_timed(
            model,
            physics_fixed_frame,
            Vec::new(),
            FrameTime {
                dts: 1.0 / 60.0,
                tts: tts as f32,
            },
        );
    }

    /// Production recording path: retain the exact clock values the model saw,
    /// and the physics commands still in flight at the end of the frame.
    pub fn record_timed(
        &mut self,
        model: &Value,
        physics_fixed_frame: u64,
        in_flight: Vec<PhysicsCommand>,
        frame_time: FrameTime,
    ) {
        let frame = self.rendered_frame;
        let end = FrameEnd {
            model,
            physics_frame: physics_fixed_frame,
            in_flight: &in_flight,
        };
        for participant in self.participants_mut() {
            participant.record(frame, &end);
        }
        self.tts_history.record(frame, &(frame_time.tts as f64));
        self.clock_history.record(frame, &frame_time);
        // One retention window for every participant and the display clock.
        let floor = retention_floor(frame);
        for participant in self.participants_mut() {
            participant.prune(floor);
        }
        self.tts_history.prune_before(floor);
        self.requests.prune(floor);
        self.rendered_frame += 1;
    }

    pub fn frame_time_at(&self, frame: Frame) -> Option<FrameTime> {
        self.clock_history.recorded_range().and_then(|(lo, hi)| {
            (frame >= lo && frame <= hi).then(|| *self.clock_history.seek(frame))
        })
    }

    /// Capture the shell's initial zero-delta settling execution (its input and
    /// effect ops). Later paused renders are not simulation history and remain
    /// deliberately discarded.
    pub fn record_replay_prefix(&mut self, frame_time: FrameTime, entries: Vec<Entry>) {
        self.requests
            .commit_unrecorded(self.epoch, self.rendered_frame);
        if self.rendered_frame == 0 && self.replay_prefix.is_none() {
            self.replay_prefix = Some((frame_time, entries));
        }
    }

    pub fn replay_prefix_len(&self) -> usize {
        usize::from(self.replay_prefix.is_some())
    }

    pub fn replay_frame_time_at_step(&self, step: usize) -> Option<FrameTime> {
        match (&self.replay_prefix, step) {
            (Some((frame_time, _)), 0) => Some(*frame_time),
            (Some(_), step) => self.frame_time_at((step - 1) as Frame),
            (None, step) => self.frame_time_at(step as Frame),
        }
    }

    /// The log entries of origin-replay step `step` (the prefix, then frame
    /// zero onward).
    pub fn replay_entries_at_step(&self, step: usize) -> &[Entry] {
        match (&self.replay_prefix, step) {
            (Some((_, entries)), 0) => entries,
            (Some(_), step) => self.logged((step - 1) as Frame).unwrap_or(&[]),
            (None, step) => self.logged(step as Frame).unwrap_or(&[]),
        }
    }

    /// Commit Δ (this rendered frame's input and effect ops) as the log of
    /// the CURRENT `rendered_frame` — the frame the matching [`Self::record`]
    /// lands on. It does NOT advance the clock; call it JUST BEFORE `record`
    /// (which does). An empty `Vec` is recorded for quiet frames, keeping the
    /// log consecutive with the model ring.
    pub fn record_entries(&mut self, entries: Vec<Entry>) {
        self.requests.commit(self.epoch, self.rendered_frame);
        self.log.record(self.rendered_frame, self.epoch, entries);
    }

    #[cfg(test)]
    pub(crate) fn record_inputs(&mut self, inputs: Vec<RecordedInput>) {
        self.record_entries(inputs.into_iter().map(Entry::Input).collect());
    }

    /// `frame`'s log; `None` outside the retained window (never recorded, or
    /// pruned).
    pub fn logged(&self, frame: Frame) -> Option<&[Entry]> {
        let (lo, hi) = self.log.recorded_range()?;
        (frame >= lo && frame <= hi).then(|| self.log.at(frame))
    }

    /// The recorded input events of `frame` — empty outside the retained
    /// window.
    pub fn inputs_at(&self, frame: u64) -> Vec<RecordedInput> {
        self.logged(frame)
            .unwrap_or(&[])
            .iter()
            .filter_map(Entry::input)
            .cloned()
            .collect()
    }

    /// The recorded inputs for frames `start..=newest`, contiguous, so the
    /// forward-step can replay them in order (division `i` gets index `i`). Empty
    /// if nothing at/after `start` is retained. Frames before the retained floor
    /// are skipped; `start` is clamped up to the floor.
    pub fn inputs_from(&self, start: u64) -> Vec<Vec<RecordedInput>> {
        match self.log.recorded_range() {
            Some((lo, hi)) => (start.max(lo)..=hi)
                .map(|f| self.log.inputs(f).cloned().collect())
                .collect(),
            None => Vec::new(),
        }
    }

    /// Most recent continuously sampled coeffect at or before `frame`, within
    /// the retained input horizon. Forward projection carries this level state
    /// through future ticks when there is no newer recorded sample.
    pub fn sampled_input_at_or_before(&self, frame: u64) -> Option<crate::InputSnapshot> {
        let (lo, hi) = self.log.recorded_range()?;
        let end = frame.min(hi);
        if end < lo {
            return None;
        }
        let mut snapshot = self
            .log
            .events
            .range(lo..=end)
            .rev()
            .find_map(|(_, events)| {
                events.iter().rev().find_map(|event| match event {
                    Entry::Input(RecordedInput::Snapshot(snapshot)) => {
                        Some(snapshot.as_ref().clone())
                    }
                    _ => None,
                })
            })?;
        // The sample at `frame` has already been delivered. Projection carries
        // continuous levels/poses, never those consumed transitions.
        snapshot.clear_edges();
        Some(snapshot)
    }

    /// The render clock `tts` to draw at WHILE SCRUBBING — the recorded `tts` of
    /// the scrubbed-to frame, so `tts`-driven visuals rewind with the model.
    /// `None` during live play: the producer then uses the real clock (no
    /// override). See docs/time-travel.md.
    pub fn scrub_render_tts(&self) -> Option<f64> {
        self.scrub_pos.map(|k| *self.tts_history.seek(k))
    }

    /// The frame the handle sits on (the scrubbed-to frame while dragging, else
    /// the newest recorded frame).
    pub fn current_scene_frame(&self) -> Option<u64> {
        self.scrub_pos.or_else(|| self.range().map(|(_, hi)| hi))
    }

    /// The recorded `tts` of the frame the scene currently sits on — the
    /// scrubbed frame while dragging, else the newest recorded frame. The shells
    /// read this to REBASE their [`crate::GameClock`] when a time-travel branch
    /// resumes, so play continues from the scene's time rather than wall-clock.
    /// Unlike [`Self::scrub_render_tts`] (scrub-only, for the paused render
    /// override) this ALSO resolves after a branch commit / rewind, when
    /// `scrub_pos` has been cleared — it then reports the newest frame's `tts`
    /// (the rewind target). `None` before anything is recorded.
    pub fn current_scene_frame_tts(&self) -> Option<f64> {
        self.current_scene_frame()
            .map(|f| *self.tts_history.seek(f))
    }

    /// The seekable window `(oldest, newest)` — the draggable range.
    pub fn scene_frame_range(&self) -> Option<(u64, u64)> {
        self.range()
    }

    /// A recorded frame's model, `tts`, and end-of-frame physics fixed frame —
    /// the three coupled rings, read WITHOUT seeking anything. `None` outside
    /// the recorded range (the rings clamp on `seek`, which would silently
    /// hand back the wrong frame). The backward preview
    /// (docs/time-travel.md T6e) reconstructs past frames from exactly this.
    pub fn recorded_frame(&self, frame: Frame) -> Option<(&Value, f64, u64)> {
        let (lo, hi) = self.range()?;
        (frame >= lo && frame <= hi).then(|| {
            (
                self.model.model_history.seek(frame),
                *self.tts_history.seek(frame),
                self.physics.world_frame_history.seek(frame).fixed,
            )
        })
    }

    /// If play resumes (`dts > 0`) while parked on an earlier frame, branch the
    /// timeline from there BEFORE the frame advances. Call at the top of `tick`.
    /// Returns `true` if a branch was committed, so the producer can drop any
    /// in-flight frame work that must not cross the branch (pending events —
    /// the reload discipline).
    ///
    /// Input injected while parked (debug `POST /input`) already updated the
    /// live `model`; the input log records it on the branch's first frame, so
    /// the branch keeps it ([`Restore::Resume`]) rather than restoring the
    /// scrubbed snapshot over it. Arrivals stay parked until the branch's
    /// first frame delivers them.
    ///
    /// `Err` refuses the branch (an open connection, D4): the scene stays
    /// scrubbed and the frame must not run.
    pub fn commit_scrub_if_resuming(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
        prev_tts: &mut Option<f64>,
    ) -> Result<bool, String> {
        let Some(k) = self.scrub_pos else {
            return Ok(false);
        };
        self.refuse_connected_branch(k, "resume")?;
        self.scrub_pos = None;
        let mut live = Live { model, physics, physics_frame, has_physics };
        let _ = self.branch_to(k, &mut live, prev_tts, Restore::Resume);
        self.counterfactual_replay = false;
        Ok(true)
    }

    /// Design-systems D4: a branch that cuts recorded frames is refused while
    /// a connection is open — the restored model may hold obsolete
    /// connection ids, and the peer's timeline does not rewind with ours.
    fn refuse_connected_branch(&self, target: u64, verb: &str) -> Result<(), String> {
        let open = self.requests.open_connections();
        let cuts = self
            .range()
            .is_some_and(|(lo, hi)| target.clamp(lo, hi) < hi);
        if open > 0 && cuts {
            return Err(format!(
                "{verb} from frame {target} refused: {open} connection(s) open. A connected \
game can scrub but not branch; seek to the newest frame to resume, or disconnect first"
            ));
        }
        Ok(())
    }

    /// Whether the scene sits on a scrubbed frame (arrivals park).
    pub fn is_scrubbed(&self) -> bool {
        self.scrub_pos.is_some()
    }

    /// The pending table and mailbox.
    pub(crate) fn requests(&self) -> &Requests {
        &self.requests
    }

    pub(crate) fn requests_mut(&mut self) -> &mut Requests {
        &mut self.requests
    }

    /// Deliver `arrival` in Δ (see [`Requests`]).
    pub fn deliver(&mut self, arrival: &Arrival) -> Delivered {
        self.requests
            .deliver(arrival, self.epoch, self.rendered_frame)
    }

    /// Track open connections from an arrival as it is pushed, parked or
    /// not (transport upkeep).
    pub fn note_arrival(&mut self, arrival: &Arrival) {
        if let Arrival::Net { key, kind, conn, .. } = arrival {
            self.requests.note_connection(key, *kind, *conn);
        }
    }

    /// Park an arrival until the scrub ends.
    pub fn park(&mut self, arrival: Arrival) {
        self.requests.park(arrival);
    }

    /// Take every parked arrival, oldest first.
    pub fn take_parked(&mut self) -> Vec<Arrival> {
        self.requests.take_mailbox()
    }

    /// A connection key the game closed.
    pub fn close_connection_key(&mut self, key: &str) {
        self.requests.close_key(key);
    }

    /// Rebind every in-flight continuation to a reloaded program; the
    /// cancellations, as messages.
    pub fn rebind_requests(
        &mut self,
        old: &functor_lang::ir::Module,
        new: &functor_lang::ir::Module,
    ) -> Vec<String> {
        self.requests.rebind(old, new)
    }

    /// A fork replaying from `frame` of `source`: the ops pending there.
    pub(crate) fn fork_requests_from(&mut self, source: &SceneRecorder, frame: Frame) {
        self.requests = source.requests.fork(frame);
    }

    /// Whether any frame after `frame` delivered an arrival.
    pub fn delivered_after(&self, frame: Frame) -> bool {
        self.log
            .events
            .range(frame + 1..)
            .any(|(_, entries)| entries.iter().any(|e| matches!(e, Entry::Delivery { .. })))
    }

    /// Non-destructive scrub for the draggable bar: restore `model` + world to
    /// `target` for DISPLAY without truncating, so the caller can seek back and
    /// forth. Exact-or-refused.
    pub fn seek_scene_to(
        &mut self,
        target: u64,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
    ) -> Result<String, String> {
        let mut live = Live { model, physics, physics_frame, has_physics };
        let frame = self.restore_all(target, Restore::Seek, &mut live, "seek")?;
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
        physics_frame: &mut u64,
        has_physics: bool,
        prev_tts: &mut Option<f64>,
    ) -> Result<String, String> {
        self.refuse_connected_branch(target, "rewind")?;
        let mut live = Live { model, physics, physics_frame, has_physics };
        self.branch_to(target, &mut live, prev_tts, Restore::Rewind)
    }

    /// The body of [`Self::rewind_scene_to`] and the scrub commits: restore
    /// every participant (`how`), then truncate the future everywhere.
    fn branch_to(
        &mut self,
        target: u64,
        live: &mut Live,
        prev_tts: &mut Option<f64>,
        how: Restore,
    ) -> Result<String, String> {
        let frame = self.restore_all(target, how, live, "rewind")?;
        // Rewind the SUBSCRIPTION-TIMER window with the model, or `Sub.every`
        // breaks across the branch. Timers fire over `(prev_tts, tts]`; leaving
        // `prev_tts` at the live tail while the clock rebases to this frame
        // makes that window empty or negative, so every timer stays silent
        // until the clock climbs back past where it used to be. (A branch that
        // moved time FORWARD would fire them as a burst instead, but that is
        // unreachable here: `frame` is clamped to the newest recorded frame,
        // which is where `prev_tts` already sits.)
        //
        // The forward-step/ghost path already guards this hazard explicitly
        // (`functor_lang_producer.rs`, "a scrubbed preview must not seed
        // subscription windows from the old live tail's prev_tts"); this is the
        // same rule for the destructive path, which never had it. [xreview]
        *prev_tts = Some(*self.tts_history.seek(frame));
        for participant in self.participants_mut() {
            participant.truncate_from(frame + 1);
        }
        self.tts_history.truncate_from(frame + 1);
        // Re-park the cut future's answers to ops still pending here, and its
        // disconnects.
        let cut = self.log.recorded_range().is_some_and(|(_, hi)| hi > frame);
        let disconnects = cut.then(|| {
            self.log
                .events
                .range(frame + 1..)
                .flat_map(|(_, entries)| entries)
                .filter_map(|entry| match entry {
                    Entry::Delivery { arrival, .. } if arrival.is_disconnect() => {
                        Some(arrival.clone())
                    }
                    _ => None,
                })
                .collect()
        });
        self.requests.truncate_from(frame + 1, disconnects);
        // Truncate the input log too: a destructive branch discards the old
        // future consistently across every ring (docs/time-travel.md T6b).
        self.log.truncate_from(frame + 1);
        self.clock_history.truncate_from(frame + 1);
        self.rendered_frame = frame + 1;
        self.generation = self.generation.wrapping_add(1);
        self.epoch += 1;
        self.scrub_pos = None;
        let clamped = if frame == target {
            String::new()
        } else {
            format!(" (requested {target}, clamped to the recorded window)")
        };
        Ok(format!("rewound scene to rendered frame {frame}{clamped}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_lang::{RunOutcome, Tracing};
    use std::rc::Rc;

    fn closure_model() -> Value {
        let module = functor_lang::lower(
            functor_lang::parse("let main = () => (x) => x + 1.0").expect("parse closure"),
        )
        .expect("lower closure");
        let record = match functor_lang::run(&module, Tracing::Off) {
            Ok(record) => record,
            Err(failure) => panic!("run closure: {}", failure.error.message),
        };
        match record.outcome {
            RunOutcome::Main(value) => value,
            _ => panic!("expected main value"),
        }
    }

    #[test]
    fn sampled_input_carry_clears_already_consumed_edges() {
        let mut recorder = SceneRecorder::new();
        recorder.record_inputs(vec![RecordedInput::Snapshot(Box::new(
            crate::InputSnapshot {
                held_keys: vec![crate::Key::Space],
                pressed_keys: vec![crate::Key::Space],
                mouse: crate::MouseSnapshot {
                    buttons: crate::MouseButtons {
                        left: true,
                        ..crate::MouseButtons::default()
                    },
                    pressed: crate::MouseButtons {
                        left: true,
                        ..crate::MouseButtons::default()
                    },
                    ..crate::MouseSnapshot::default()
                },
                ..crate::InputSnapshot::default()
            },
        ))]);
        recorder.record(&Value::Number(0.0), 0, 0.0);

        let carried = recorder
            .sampled_input_at_or_before(0)
            .expect("recorded sample");
        assert_eq!(carried.held_keys, vec![crate::Key::Space]);
        assert!(carried.mouse.buttons.left);
        assert!(carried.pressed_keys.is_empty());
        assert!(carried.released_keys.is_empty());
        assert_eq!(carried.mouse.pressed, crate::MouseButtons::default());
        assert_eq!(carried.mouse.released, crate::MouseButtons::default());
    }

    // --- the real payload: an functor_lang::Value model ---

    fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
        match v {
            Value::Record(fields) => &fields.iter().find(|(k, _)| k == name).expect("field").1,
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

    // --- the coupled recorder: tts rewinds with the model ---

    #[test]
    fn scrub_render_tts_returns_the_scrubbed_frames_recorded_time() {
        // Record frames with distinct render clocks; a physics-less scene means
        // the seek touches only the model + scrub position.
        let mut rec = SceneRecorder::new();
        for f in 0..5u64 {
            rec.record(&Value::Number(f as f64), 0, f as f64 * 10.0);
        }
        // Live play (not scrubbing): no override, the producer uses the real clock.
        assert_eq!(rec.scrub_render_tts(), None);

        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut frame = 0u64;
        // Non-destructively scrub back to frame 2 (has_physics = false).
        rec.seek_scene_to(2, &mut model, &mut physics, &mut frame, false)
            .expect("seek");
        // A scrubbed frame draws at its recorded tts (2 * 10.0), and the model
        // restored in lockstep.
        assert_eq!(rec.scrub_render_tts(), Some(20.0));
        assert_eq!(model.to_string(), Value::Number(2.0).to_string());

        // Scrub forward to frame 4: the override tracks the handle.
        rec.seek_scene_to(4, &mut model, &mut physics, &mut frame, false)
            .expect("seek");
        assert_eq!(rec.scrub_render_tts(), Some(40.0));
    }

    #[test]
    fn plain_data_reload_while_scrubbed_preserves_the_full_future_until_resume() {
        let mut rec = SceneRecorder::new();
        for frame in 0..5u64 {
            rec.record_inputs(Vec::new());
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(2, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");

        let generation = rec.generation();
        assert!(rec.reload_history_is_safe());
        assert_eq!(
            rec.finish_reload(&model, physics_frame, true),
            ReloadHistory::Preserved
        );
        assert_eq!(rec.scene_frame_range(), Some((0, 4)));
        assert_eq!(rec.current_scene_frame(), Some(2));
        assert_eq!(rec.scrub_render_tts(), Some(2.0));
        assert_eq!(rec.generation(), generation);
        assert_eq!(rec.next_frame(), 5, "reload must not branch the future");
        assert_eq!(rec.counterfactual_replay_span(), Ok(Some((0, 2, 4))));

        let rebuilt: Vec<Value> = (10..15).map(|n| Value::Number(n as f64)).collect();
        model = rec
            .materialize_counterfactual_history(&rebuilt, None)
            .expect("valid replay history")
            .expect("materialized history");

        assert_eq!(
            rec.commit_scrub_if_resuming(&mut model, &mut physics, &mut physics_frame, false, &mut None),
            Ok(true)
        );
        assert_eq!(model.to_string(), "12", "Resume adopts the rebuilt anchor");
        assert_eq!(rec.counterfactual_replay_span(), Ok(None));
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));
        assert_eq!(rec.next_frame(), 3);
        assert_ne!(rec.generation(), generation, "Resume commits the branch");
    }

    /// A branch must rewind the SUBSCRIPTION-TIMER window with the model.
    ///
    /// `Sub.every` fires over `(prev_tts, tts]`. Leaving `prev_tts` at the live
    /// tail across a rewind makes that window empty or negative, so timers stay
    /// silent until the clock climbs back past where it used to be — the bug
    /// this pins. The forward-step/ghost path already guarded it; the
    /// destructive path never did.
    #[test]
    fn a_branch_rewinds_the_subscription_timer_window_with_the_model() {
        let mut rec = SceneRecorder::new();
        for frame in 0..10u64 {
            rec.record_inputs(Vec::new());
            // tts advances a second per frame, so a stale window is unmistakable.
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;

        // The live tail: the game has been running to frame 9 (tts 9.0).
        let mut prev_tts = Some(9.0);
        rec.rewind_scene_to(3, &mut model, &mut physics, &mut physics_frame, false, &mut prev_tts)
            .expect("rewind");

        // The next frame's window is then (3.0, 3.0+dt] — forward and non-empty.
        // With the bug it was (9.0, 3.0+dt]: negative, so every timer was mute
        // for six seconds of game time.
        assert_eq!(
            prev_tts,
            Some(3.0),
            "prev_tts must follow the model back to the branch frame's recorded tts"
        );
    }

    /// The same rule on the path a resume actually takes: park, then resume.
    #[test]
    fn resuming_from_a_scrub_rewinds_the_timer_window_too() {
        let mut rec = SceneRecorder::new();
        for frame in 0..10u64 {
            rec.record_inputs(Vec::new());
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(4, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");

        let mut prev_tts = Some(9.0);
        assert_eq!(
            rec.commit_scrub_if_resuming(
                &mut model,
                &mut physics,
                &mut physics_frame,
                false,
                &mut prev_tts
            ),
            Ok(true)
        );
        assert_eq!(prev_tts, Some(4.0), "resume commits the branch and its window");
    }

    #[test]
    fn counterfactual_replay_requires_a_historical_scrub_but_not_model_frame_zero() {
        let mut live = SceneRecorder::new();
        for frame in 0..3u64 {
            live.record_inputs(Vec::new());
            live.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        assert_eq!(
            live.finish_reload(&Value::Number(2.0), 0, true),
            ReloadHistory::Preserved
        );
        assert!(
            live.counterfactual_replay_span() == Ok(None),
            "ordinary live-tail reload preserves the live model"
        );

        let mut tail = SceneRecorder::new();
        for frame in 0..3u64 {
            tail.record_inputs(Vec::new());
            tail.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut tail_model = Value::Number(0.0);
        let mut tail_physics = SteppedPhysics::new();
        let mut tail_physics_frame = 0;
        tail.seek_scene_to(
            2,
            &mut tail_model,
            &mut tail_physics,
            &mut tail_physics_frame,
            false,
        )
        .expect("seek live tail");
        tail.finish_reload(&tail_model, tail_physics_frame, true);
        assert!(
            tail.counterfactual_replay_span() == Ok(None),
            "a cursor explicitly parked at the newest frame is still the live tail"
        );

        let mut pruned = SceneRecorder::new();
        for frame in 0..=DEFAULT_HISTORY_FRAMES as u64 {
            pruned.record_inputs(Vec::new());
            pruned.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let (lo, _) = pruned.scene_frame_range().unwrap();
        assert!(lo > 0);
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        pruned
            .seek_scene_to(lo, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek pruned floor");
        assert_eq!(
            pruned.finish_reload(&model, physics_frame, true),
            ReloadHistory::Preserved
        );
        assert_eq!(
            pruned.counterfactual_replay_span(),
            Ok(Some((lo, lo, DEFAULT_HISTORY_FRAMES as u64))),
            "the session input log keeps frame zero after model pruning"
        );
    }

    #[test]
    fn sampled_input_falls_back_to_selected_snapshot_after_origin_ages_out() {
        let mut rec = SceneRecorder::new();
        for frame in 0..=DEFAULT_HISTORY_FRAMES as u64 {
            rec.record_inputs(vec![RecordedInput::Snapshot(Box::new(
                crate::InputSnapshot::default(),
            ))]);
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let (lo, hi) = rec.scene_frame_range().expect("retained model history");
        assert!(lo > 0);
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(lo, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek retained floor");

        assert_eq!(
            rec.finish_reload(&model, physics_frame, true),
            ReloadHistory::Preserved
        );
        assert_eq!(rec.scene_frame_range(), Some((lo, hi)));
        assert_eq!(
            rec.counterfactual_replay_span(),
            Ok(None),
            "a bounded coeffect prefix must not masquerade as origin replay"
        );
    }

    #[test]
    fn counterfactual_replay_reports_a_missing_session_input_log() {
        let mut rec = SceneRecorder::new();
        for frame in 0..3u64 {
            // Deliberately violate the producer invariant by omitting
            // `record_inputs`; production must report this instead of silently
            // retaining old-code snapshots.
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(1, &mut model, &mut physics, &mut physics_frame, false)
            .expect("scrub into history");
        rec.finish_reload(&model, physics_frame, true);

        let error = rec
            .counterfactual_replay_span()
            .expect_err("missing input history must be diagnosed");
        assert!(error.contains("session input log is empty"), "{error}");
    }

    #[test]
    fn preserved_reload_keeps_the_recorded_snapshot() {
        let mut rec = SceneRecorder::new();
        rec.record(&Value::Number(1.0), 0, 0.0);

        // Paused input may update the live model without recording a frame.
        // The log records it on the next frame, so frame 0 stays as recorded.
        let live_model = Value::Number(99.0);
        assert_eq!(
            rec.finish_reload(&live_model, 0, true),
            ReloadHistory::Preserved
        );

        let mut restored = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(0, &mut restored, &mut physics, &mut physics_frame, false)
            .expect("seek");
        assert_eq!(restored.to_string(), "1");
    }

    #[test]
    fn closure_history_restarts_at_the_rebound_current_frame() {
        let stored_closure = closure_model();
        let mut rec = SceneRecorder::new();
        for frame in 0..5u64 {
            rec.record(&stored_closure, 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(2, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");
        assert!(!rec.reload_history_is_safe());
        // The reload-while-scrubbed branch restores the timer window too — the
        // third path into `rewind_scene_to`, which the dedicated tests above
        // don't reach. [xreview]
        let mut prev_tts = Some(4.0);
        rec.commit_scrub_before_reload(
            &mut model,
            &mut physics,
            &mut physics_frame,
            false,
            &mut prev_tts,
        );
        assert_eq!(
            prev_tts,
            Some(2.0),
            "an unsafe reload while scrubbed branches, so its timer window rewinds too"
        );
        let generation = rec.generation();

        assert_eq!(
            rec.finish_reload(&model, physics_frame, false),
            ReloadHistory::RestartedAt(2)
        );
        assert_eq!(rec.scene_frame_range(), Some((2, 2)));
        assert_eq!(rec.current_scene_frame(), Some(2));
        assert_eq!(rec.current_scene_frame_tts(), Some(2.0));
        assert_eq!(rec.next_frame(), 3);
        assert_ne!(rec.generation(), generation);
        assert!(rec
            .seek_scene_to(1, &mut model, &mut physics, &mut physics_frame, false)
            .is_ok());
        assert_eq!(
            rec.current_scene_frame(),
            Some(2),
            "seek clamps to the new boundary"
        );
    }

    #[test]
    fn unsafe_values_only_in_the_discarded_future_keep_the_safe_prefix() {
        let stored_closure = closure_model();
        let mut rec = SceneRecorder::new();
        for frame in 0..3u64 {
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        for frame in 3..5u64 {
            rec.record(&stored_closure, 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(2, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");

        assert!(!rec.reload_history_is_safe());
        rec.commit_scrub_before_reload(&mut model, &mut physics, &mut physics_frame, false, &mut None);
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));
        assert!(rec.reload_history_is_safe());

        assert_eq!(
            rec.finish_reload(&model, physics_frame, true),
            ReloadHistory::Preserved
        );
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));
        assert_eq!(rec.current_scene_frame(), Some(2));
    }

    #[test]
    fn unsafe_authoritative_live_model_forces_a_boundary_without_losing_it() {
        let mut rec = SceneRecorder::new();
        for frame in 0..5u64 {
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(2, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");

        // Simulate a paused update that changed the authoritative model without
        // recording another frame. Preparing the reload may branch history,
        // but must not overwrite this live value with frame 2's snapshot.
        model = closure_model();
        let live_before = model.to_string();
        let (live_model_was_safe, _) =
            rec.prepare_reload(&mut model, &mut physics, &mut physics_frame, false, &mut None);
        assert!(!live_model_was_safe);
        assert_eq!(model.to_string(), live_before);
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));

        assert_eq!(
            rec.finish_reload(&model, physics_frame, live_model_was_safe),
            ReloadHistory::RestartedAt(2)
        );
        assert_eq!(rec.scene_frame_range(), Some((2, 2)));
    }

    #[test]
    fn first_class_constructor_history_starts_a_new_generation() {
        let constructor = Value::Ctor {
            name: Rc::from("Pair"),
            arity: 1,
        };
        let mut rec = SceneRecorder::new();
        rec.record(&constructor, 0, 0.0);
        let generation = rec.generation();

        assert_eq!(
            rec.finish_reload(&constructor, 0, false),
            ReloadHistory::RestartedAt(0)
        );
        assert_ne!(rec.generation(), generation);
        assert_eq!(rec.scene_frame_range(), Some((0, 0)));
    }

    // --- participants beyond the model and physics ---

    /// Records `frame * 10`; restores it into `live`; refuses `refuse`.
    struct Probe {
        history: Rc<std::cell::RefCell<History<u64>>>,
        live: Rc<std::cell::Cell<u64>>,
        refuse: Rc<std::cell::Cell<Option<Frame>>>,
    }

    impl Participant for Probe {
        fn name(&self) -> &'static str {
            "probe"
        }
        fn record(&mut self, frame: Frame, _end: &FrameEnd) {
            self.history.borrow_mut().record(frame, &(frame * 10));
        }
        fn range(&self) -> Option<(Frame, Frame)> {
            self.history.borrow().recorded_range()
        }
        fn check_restore(&self, frame: Frame, _live: &Live) -> Result<(), String> {
            match self.refuse.get() {
                Some(refused) if refused == frame => Err(format!("probe refuses {frame}")),
                _ => Ok(()),
            }
        }
        fn restore(&mut self, frame: Frame, how: Restore, _live: &mut Live) {
            if how != Restore::Resume {
                self.live.set(*self.history.borrow().seek(frame));
            }
        }
        fn truncate_from(&mut self, frame: Frame) {
            self.history.borrow_mut().truncate_from(frame);
        }
        fn prune(&mut self, floor: Frame) {
            self.history.borrow_mut().prune_before(floor);
        }
        fn clear(&mut self) {
            *self.history.borrow_mut() = History::unbounded();
        }
        fn recorded_digest(&self, frame: Frame) -> Option<u64> {
            self.history.borrow().get(frame).copied()
        }
        fn live_digest(&self, _live: &Live) -> u64 {
            self.live.get()
        }
    }

    #[test]
    fn an_added_participant_is_restored_truncated_and_pruned_with_the_rest() {
        let history = Rc::new(std::cell::RefCell::new(History::unbounded()));
        let live = Rc::new(std::cell::Cell::new(0));
        let refuse = Rc::new(std::cell::Cell::new(None));
        let mut rec = SceneRecorder::new();
        rec.add_participant(Box::new(Probe {
            history: history.clone(),
            live: live.clone(),
            refuse: refuse.clone(),
        }));
        let total = DEFAULT_HISTORY_FRAMES as u64 + 10;
        for frame in 0..total {
            rec.record_inputs(Vec::new());
            rec.record(&Value::Number(frame as f64), 0, frame as f64);
        }

        // Pruned with the rest: one window.
        assert_eq!(rec.scene_frame_range(), Some((10, total - 1)));
        assert_eq!(history.borrow().recorded_range(), Some((10, total - 1)));

        // Restored with the rest.
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        rec.seek_scene_to(15, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek");
        assert_eq!((model.to_string().as_str(), live.get()), ("15", 150));
        let states = rec.participant_states(&Live {
            model: &mut model,
            physics: &mut physics,
            physics_frame: &mut physics_frame,
            has_physics: false,
        });
        assert_eq!(
            states.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["model", "physics", "probe"]
        );
        for state in &states {
            assert_eq!(state.range, Some((10, total - 1)), "{state:?}");
            assert_eq!(state.recorded, Some(state.live), "{state:?}");
        }

        // One participant's refusal refuses the whole seek, touching nothing.
        refuse.set(Some(17));
        assert!(rec
            .seek_scene_to(17, &mut model, &mut physics, &mut physics_frame, false)
            .is_err());
        assert_eq!((model.to_string().as_str(), live.get()), ("15", 150));
        assert_eq!(rec.current_scene_frame(), Some(15));

        // Truncated with the rest.
        rec.rewind_scene_to(30, &mut model, &mut physics, &mut physics_frame, false, &mut None)
            .expect("rewind");
        assert_eq!((model.to_string().as_str(), live.get()), ("30", 300));
        assert_eq!(rec.scene_frame_range(), Some((10, 30)));
        assert_eq!(history.borrow().recorded_range(), Some((10, 30)));
    }

    // --- the input log: record → read back, and survive a reload (T6b) ---

    #[test]
    fn input_log_records_reads_back_and_survives_reload() {
        let mut rec = SceneRecorder::new();
        // Record three frames, keying inputs in lockstep with the model (the
        // order the producer's `record_frame` uses: inputs first, then model).
        let f0 = vec![RecordedInput::Key {
            code: 30,
            is_down: true,
        }];
        let f1: Vec<RecordedInput> = vec![];
        let f2 = vec![
            RecordedInput::MouseMove { x: 5, y: 7 },
            RecordedInput::MouseWheel { delta: -1 },
        ];
        for inputs in [&f0, &f1, &f2] {
            rec.record_inputs(inputs.clone());
            rec.record(&Value::Number(0.0), 0, 0.0);
        }

        // Round-trip: each frame reads back exactly what was recorded.
        assert_eq!(rec.inputs_at(0).len(), 1);
        assert!(matches!(
            rec.inputs_at(0)[0],
            RecordedInput::Key {
                code: 30,
                is_down: true
            }
        ));
        assert!(rec.inputs_at(1).is_empty());
        assert_eq!(rec.inputs_at(2).len(), 2);
        // A frame outside the recorded window reads back empty, not a panic.
        assert!(rec.inputs_at(99).is_empty());
        // `inputs_from` is contiguous from the start frame.
        assert_eq!(rec.inputs_from(1).len(), 2); // frames 1 and 2

        // A hot reload keeps both the plain-data model ring and input log.
        let generation = rec.generation();
        assert_eq!(
            rec.finish_reload(&Value::Number(0.0), 0, true),
            ReloadHistory::Preserved
        );
        assert_eq!(rec.generation(), generation);
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));
        assert_eq!(rec.inputs_at(0).len(), 1, "input log survives the reload");
        assert_eq!(rec.inputs_at(2).len(), 2, "input log survives the reload");
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
        // (docs/time-travel.md, "Why Functor Lang makes this nearly free").
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
