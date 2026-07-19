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

use std::collections::{BTreeMap, VecDeque};

use functor_lang::Value;

use crate::FrameTime;
use crate::input::RecordedInput;
use crate::physics::SteppedPhysics;

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
            Some(next) => assert_eq!(frame, next, "history frames must be recorded consecutively"),
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

    /// Replace one retained snapshot without changing the window or its next
    /// frame. Reload uses this to make the current snapshot authoritative when
    /// paused input changed the live model between recorded frames.
    pub fn replace(&mut self, frame: Frame, state: &T) {
        let (oldest, newest) = self.recorded_range().expect("replace in an empty history");
        assert!(
            frame >= oldest && frame <= newest,
            "replace({frame}) outside recorded history [{oldest}, {newest}]"
        );
        self.snapshots[(frame - self.base) as usize] = state.clone();
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

/// Session-long input history with O(events) storage rather than one heap-sized
/// `Vec` header per input-free frame. The contiguous frame range is tracked
/// separately because fixed-step replay still advances through empty frames.
struct InputHistory {
    base: Option<Frame>,
    next: Option<Frame>,
    events: BTreeMap<Frame, Vec<RecordedInput>>,
}

impl InputHistory {
    fn new() -> InputHistory {
        InputHistory {
            base: None,
            next: None,
            events: BTreeMap::new(),
        }
    }

    fn record(&mut self, frame: Frame, inputs: Vec<RecordedInput>) {
        match self.next {
            None => self.base = Some(frame),
            Some(next) => assert_eq!(frame, next, "input frames must be recorded consecutively"),
        }
        if !inputs.is_empty() {
            self.events.insert(frame, inputs);
        }
        self.next = Some(frame + 1);
    }

    fn recorded_range(&self) -> Option<(Frame, Frame)> {
        self.base.zip(self.next).and_then(|(base, next)| {
            (next > base).then_some((base, next - 1))
        })
    }

    fn at(&self, frame: Frame) -> &[RecordedInput] {
        self.events.get(&frame).map(Vec::as_slice).unwrap_or(&[])
    }

    fn truncate_from(&mut self, frame: Frame) {
        let Some(next) = self.next else { return };
        if frame >= next {
            return;
        }
        let base = self.base.expect("input history with a next frame has a base");
        assert!(
            frame >= base,
            "truncate_from({frame}) predates input history (base {base})"
        );
        self.events.split_off(&frame);
        self.next = Some(frame);
    }
}

/// The coupled time-travel recorder shared by both Functor Lang shells (docs/time-travel.md
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
    /// The render clock `tts` (total time) of each rendered frame, recorded in
    /// LOCKSTEP with `model_history`. A scrubbed frame draws at its recorded
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
    replay_prefix: Option<(FrameTime, Vec<RecordedInput>)>,
    /// The frame-indexed input log (docs/time-travel.md T6, "The event log"):
    /// each rendered frame's recorded input events, keyed in LOCKSTEP with
    /// `model_history`. Unlike the other three rings this is PLAIN DATA
    /// (`RecordedInput`) and is retained from frame zero for the whole current
    /// branch. That lets a scrubbed hot reload replay from the edited program's
    /// `init` even after the much larger model/world snapshots have pruned their
    /// first ~15-second window.
    input_history: InputHistory,
    /// The rendered-frame index the next snapshot records at; monotonic.
    rendered_frame: u64,
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
            model_history: History::bounded(DEFAULT_HISTORY_FRAMES),
            world_frame_history: History::bounded(DEFAULT_HISTORY_FRAMES),
            tts_history: History::bounded(DEFAULT_HISTORY_FRAMES),
            clock_history: History::unbounded(),
            replay_prefix: None,
            input_history: InputHistory::new(),
            rendered_frame: 0,
            generation: 0,
            program_revision: 0,
            counterfactual_replay: false,
            scrub_pos: None,
        }
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
            // Input can update the live model while paused, without producing
            // a new recorded frame. Make the snapshot at the visible cursor
            // match that authoritative live state before retaining history.
            // Keep `scrub_pos`: reload is non-destructive, and Resume remains
            // the operation that branches away the recorded future.
            if let Some(frame) = current_frame {
                self.model_history.replace(frame, rebound_model);
                self.world_frame_history
                    .replace(frame, &physics_fixed_frame);
            }
            // Ordinary hot reload at the live tail preserves the live model;
            // only an explicitly scrubbed historical branch is counterfactual.
            self.counterfactual_replay = self
                .scrub_pos
                .zip(self.model_history.recorded_range())
                .is_some_and(|(selected, (_, hi))| selected < hi);
            return ReloadHistory::Preserved;
        }

        self.counterfactual_replay = false;
        self.scrub_pos = None;
        let Some((frame, tts)) = current_frame.map(|frame| {
            let tts = *self.tts_history.seek(frame);
            (frame, tts)
        }) else {
            // An unsafe snapshot implies a non-empty history, so this is only a
            // defensive fallback if that invariant changes later.
            return ReloadHistory::Preserved;
        };
        self.model_history = History::bounded(DEFAULT_HISTORY_FRAMES);
        self.world_frame_history = History::bounded(DEFAULT_HISTORY_FRAMES);
        self.tts_history = History::bounded(DEFAULT_HISTORY_FRAMES);
        self.generation = self.generation.wrapping_add(1);

        self.model_history.record(frame, rebound_model);
        self.world_frame_history.record(frame, &physics_fixed_frame);
        self.tts_history.record(frame, &tts);
        ReloadHistory::RestartedAt(frame)
    }

    /// Whether every retained model snapshot is independent of the loaded
    /// module and can therefore remain seekable across a hot reload.
    pub fn reload_history_is_safe(&self) -> bool {
        self.model_history
            .snapshots
            .iter()
            .all(Value::is_reload_safe_snapshot)
    }

    /// Classify and, when necessary, branch the coupled timeline before the
    /// producer rebinds its authoritative live model. The live model may have
    /// changed without a recorded frame while paused, so it participates in
    /// the safety decision independently of the snapshot ring.
    ///
    /// Branching normally restores the selected snapshot into `model`; retain
    /// and restore the authoritative live value around that operation so a
    /// paused update is not silently lost. The returned live-model flag must be
    /// passed to [`Self::finish_reload`] after rebinding.
    pub fn prepare_reload(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
    ) -> bool {
        let live_model_is_safe = model.is_reload_safe_snapshot();
        if !self.reload_history_is_safe() || !live_model_is_safe {
            let authoritative_model = model.clone();
            self.commit_scrub_before_reload(model, physics, physics_frame, has_physics);
            *model = authoritative_model;
        }
        live_model_is_safe
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

    /// Complete replay target after a safe reload. Model snapshots may have
    /// pruned frame zero; the small plain-data input log deliberately does not.
    /// Return a diagnostic instead of silently falling back to old-data
    /// semantics if that session-origin invariant is ever broken.
    pub fn counterfactual_replay_span(
        &self,
    ) -> Result<Option<(Frame, Frame, Frame)>, String> {
        if !self.counterfactual_replay {
            return Ok(None);
        }
        let (lo, hi) = self.model_history.recorded_range().ok_or_else(|| {
            "counterfactual replay unavailable: no model history is retained".to_string()
        })?;
        let (input_lo, input_hi) = self.input_history.recorded_range().ok_or_else(|| {
            "counterfactual replay unavailable: the session input log is empty".to_string()
        })?;
        if input_lo != 0 || input_hi < hi {
            return Err(format!(
                "counterfactual replay unavailable: input history covers frames \
                 {input_lo}..={input_hi}, not the required 0..={hi}"
            ));
        }
        let selected = self.current_scene_frame().ok_or_else(|| {
            "counterfactual replay unavailable: no selected frame".to_string()
        })?;
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
    /// the current program, returning the rebuilt selected model. The input,
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
        let (retained_lo, retained_hi) = self.model_history.recorded_range().ok_or_else(|| {
            "counterfactual replay cannot replace an empty model history".to_string()
        })?;
        if (retained_lo, retained_hi) != (lo, hi) {
            return Err("counterfactual replay target changed while rebuilding history".to_string());
        }
        for (frame, rebuilt) in (lo..=hi).zip(models) {
            self.model_history.replace(
                frame,
                if frame == selected {
                    selected_override.unwrap_or(rebuilt)
                } else {
                    rebuilt
                },
            );
        }
        self.counterfactual_replay = false;
        Ok(Some(self.model_history.seek(selected).clone()))
    }

    /// Commit a non-destructive scrub before an UNSAFE reload replaces the
    /// snapshot generation. Safe plain-data reloads must not call this: they
    /// retain both the selected cursor and its recorded future until Resume.
    /// For an unsafe reload this preserves the state the user is inspecting
    /// and branches the physics timeline in lockstep before `scrub_pos`
    /// disappears.
    pub fn commit_scrub_before_reload(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
    ) {
        if let Some(frame) = self.scrub_pos {
            let _ = self.rewind_scene_to(frame, model, physics, physics_frame, has_physics);
        }
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
            FrameTime {
                dts: 1.0 / 60.0,
                tts: tts as f32,
            },
        );
    }

    /// Production recording path: retain the exact clock values the model saw.
    pub fn record_timed(
        &mut self,
        model: &Value,
        physics_fixed_frame: u64,
        frame_time: FrameTime,
    ) {
        self.model_history.record(self.rendered_frame, model);
        self.world_frame_history
            .record(self.rendered_frame, &physics_fixed_frame);
        self.tts_history
            .record(self.rendered_frame, &(frame_time.tts as f64));
        self.clock_history.record(self.rendered_frame, &frame_time);
        self.rendered_frame += 1;
    }

    pub fn frame_time_at(&self, frame: Frame) -> Option<FrameTime> {
        self.clock_history.recorded_range().and_then(|(lo, hi)| {
            (frame >= lo && frame <= hi).then(|| *self.clock_history.seek(frame))
        })
    }

    /// Capture the shell's initial zero-delta settling execution. Later paused
    /// renders are not simulation history and remain deliberately discarded.
    pub fn record_replay_prefix(
        &mut self,
        frame_time: FrameTime,
        inputs: Vec<RecordedInput>,
    ) {
        if self.rendered_frame == 0 && self.replay_prefix.is_none() {
            self.replay_prefix = Some((frame_time, inputs));
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

    pub fn replay_inputs_at_step(&self, step: usize) -> &[RecordedInput] {
        match (&self.replay_prefix, step) {
            (Some((_, inputs)), 0) => inputs,
            (Some(_), step) => self.inputs_at((step - 1) as Frame),
            (None, step) => self.inputs_at(step as Frame),
        }
    }

    /// Record this rendered frame's input events, keyed by the CURRENT
    /// `rendered_frame` — so it lands on the same frame as the matching
    /// [`Self::record`]. It does NOT advance the clock; call it JUST BEFORE
    /// `record` (which does), so both key off the same frame. An empty `Vec` is
    /// recorded for input-free frames, keeping the ring consecutive with the
    /// model ring.
    pub fn record_inputs(&mut self, inputs: Vec<RecordedInput>) {
        self.input_history.record(self.rendered_frame, inputs);
    }

    /// The recorded input events of `frame` — empty if that frame is outside the
    /// retained input window (never recorded, or pruned).
    pub fn inputs_at(&self, frame: u64) -> &[RecordedInput] {
        match self.input_history.recorded_range() {
            Some((lo, hi)) if frame >= lo && frame <= hi => self.input_history.at(frame),
            _ => &[],
        }
    }

    /// The recorded inputs for frames `start..=newest`, contiguous, so the
    /// forward-step can replay them in order (division `i` gets index `i`). Empty
    /// if nothing at/after `start` is retained. Frames before the retained floor
    /// are skipped; `start` is clamped up to the floor.
    pub fn inputs_from(&self, start: u64) -> Vec<Vec<RecordedInput>> {
        match self.input_history.recorded_range() {
            Some((lo, hi)) => (start.max(lo)..=hi)
                .map(|f| self.input_history.at(f).to_vec())
                .collect(),
            None => Vec::new(),
        }
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
        self.scrub_pos
            .or_else(|| self.model_history.recorded_range().map(|(_, hi)| hi))
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
        self.model_history.recorded_range()
    }

    /// If play resumes (`dts > 0`) while parked on an earlier frame, branch the
    /// timeline from there BEFORE the frame advances. Call at the top of `tick`.
    /// Returns `true` if a branch was committed, so the producer can drop any
    /// in-flight frame work that must not cross the branch (deferred queries /
    /// pending events — the reload discipline).
    pub fn commit_scrub_if_resuming(
        &mut self,
        model: &mut Value,
        physics: &mut SteppedPhysics,
        physics_frame: &mut u64,
        has_physics: bool,
    ) -> bool {
        if let Some(k) = self.scrub_pos.take() {
            let _ = self.rewind_scene_to(k, model, physics, physics_frame, has_physics);
            self.counterfactual_replay = false;
            true
        } else {
            false
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
        physics_frame: &mut u64,
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
            // Warnings are empty on every reachable coupled seek: `physics_seek_
            // target` already validated `fixed` against the seekable range.
            let _ = physics.seek_to_frame(fixed);
            *physics_frame = physics.current_fixed_frame();
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
        physics_frame: &mut u64,
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
            *physics_frame = physics.current_fixed_frame();
        }
        self.model_history.truncate_from(frame + 1);
        self.world_frame_history.truncate_from(frame + 1);
        self.tts_history.truncate_from(frame + 1);
        // Truncate the input log too: a destructive branch discards the old
        // future consistently across all four rings (docs/time-travel.md T6b).
        self.input_history.truncate_from(frame + 1);
        self.clock_history.truncate_from(frame + 1);
        self.rendered_frame = frame + 1;
        self.generation = self.generation.wrapping_add(1);
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

        assert!(rec.commit_scrub_if_resuming(&mut model, &mut physics, &mut physics_frame, false));
        assert_eq!(model.to_string(), "12", "Resume adopts the rebuilt anchor");
        assert_eq!(rec.counterfactual_replay_span(), Ok(None));
        assert_eq!(rec.scene_frame_range(), Some((0, 2)));
        assert_eq!(rec.next_frame(), 3);
        assert_ne!(rec.generation(), generation, "Resume commits the branch");
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
    fn preserved_reload_refreshes_the_current_snapshot_from_the_live_model() {
        let mut rec = SceneRecorder::new();
        rec.record(&Value::Number(1.0), 0, 0.0);

        // Paused input may update the live model without advancing/recording a
        // frame. Reload must not leave frame 0 pointing at the stale value.
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
        assert_eq!(restored.to_string(), live_model.to_string());
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
        rec.commit_scrub_before_reload(&mut model, &mut physics, &mut physics_frame, false);
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
        rec.commit_scrub_before_reload(&mut model, &mut physics, &mut physics_frame, false);
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
        let live_model_was_safe =
            rec.prepare_reload(&mut model, &mut physics, &mut physics_frame, false);
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
