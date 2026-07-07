//! The shared game clock owned by both runtime shells (docs/time-travel.md).
//!
//! It turns wall-clock frame deltas into the game's `tts` (total time) — the
//! value handed to `tick` / `subscriptions` / `draw`. Unlike a raw wall clock,
//! `tts` is a GAME clock:
//!
//! - **Live:** it ACCUMULATES the real frame delta (`game_time += real_delta`),
//!   so it equals wall-clock elapsed only as long as the game is never paused.
//! - **Paused** (scrubber pause / debug pin): it FREEZES (`dts = 0`, `tts`
//!   held). Resuming continues from `game_time`, NOT wall-clock — this is what
//!   kills the pause→resume jump (a pause of any wall-clock length costs the
//!   game clock nothing).
//! - **Rebase:** [`GameClock::rebase`] jumps `game_time` to an arbitrary time —
//!   used when a time-travel branch resumes from a scrubbed frame, so play
//!   continues from the scrubbed scene time rather than snapping to "now".
//!
//! `--fixed-time` / `?fixed-time` is an UNCONDITIONAL pin: every frame returns
//! `{ dts: 0, tts: <const> }`, bypassing accumulation, pause, step, and rebase.
//! This is the deterministic-capture / golden-image path and MUST stay
//! byte-identical, so it is checked first, ahead of every other control.

use crate::FrameTime;

/// The fixed model timestep, in seconds — the interval `tick` advances by under
/// [`GameClock::fixed_frames`]. It is `1/60`, matching BOTH the physics fixed
/// step (`physics::world::FIXED_DT`) — so the model and physics fixed-frames
/// advance in lockstep — AND the forward-ghost's `sub_dt`, so one recorded frame
/// IS exactly one forward-step fine step (the mapping the ghost replay assumes;
/// docs/time-travel.md). Recording at variable wall-clock dt breaks that mapping
/// and scrambles the strobe.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Spiral-of-death clamp: never run more than this many fixed model steps for a
/// single rendered frame. After a long stall (breakpoint, alt-tab, a heavy load
/// frame) the backlog is capped rather than replayed as a burst — mirrors
/// physics' `MAX_SUBSTEPS_PER_FRAME`.
const MAX_SUBSTEPS: usize = 8;

/// A frame-time source shared by the desktop and web shells. Constructed once
/// per session (seeded with the shell's fixed-time option) and called each
/// rendered frame via [`GameClock::fixed_frames`] (the fixed-timestep model
/// loop) — or the legacy one-tick [`GameClock::frame`].
pub struct GameClock {
    /// Accumulated game time in seconds — the `tts` handed to the game. Advances
    /// by the real frame delta while live; frozen while paused; jumps on rebase.
    game_time: f32,
    /// Unspent real time (seconds) not yet consumed by a whole [`FIXED_DT`]
    /// model step. [`GameClock::fixed_frames`] drains it in `FIXED_DT` chunks;
    /// the remainder carries to the next rendered frame. Untouched by the legacy
    /// [`GameClock::frame`] path.
    accumulator: f32,
    /// Scrubber / debug pause. While paused, `dts = 0` and `game_time` is
    /// frozen. A queued [`GameClock::step`] also sets this.
    paused: bool,
    /// A one-shot step (seconds) that advances `game_time` on the next frame,
    /// then holds. Set by Step / debug Advance (both imply pause).
    pending_step: Option<f32>,
    /// Unconditional pin (`--fixed-time` / `?fixed-time`). When set, every frame
    /// is `{ dts: 0, tts: <const> }` — no accumulation, no pause, no rebase.
    fixed_time: Option<f32>,
}

impl GameClock {
    /// A clock seeded with the shell's fixed-time option (`None` = live).
    pub fn new(fixed_time: Option<f32>) -> Self {
        GameClock {
            game_time: 0.0,
            accumulator: 0.0,
            paused: false,
            pending_step: None,
            fixed_time,
        }
    }

    /// This frame's [`FrameTime`], given the real wall-clock delta since the last
    /// frame. Fixed-time pins unconditionally (checked first, so golden captures
    /// stay byte-identical); a pending step advances once then holds; a pause
    /// freezes; otherwise the clock advances by `real_delta`.
    pub fn frame(&mut self, real_delta: f32) -> FrameTime {
        if let Some(t) = self.fixed_time {
            return FrameTime { dts: 0.0, tts: t };
        }
        if let Some(step) = self.pending_step.take() {
            self.game_time += step;
            return FrameTime {
                dts: step,
                tts: self.game_time,
            };
        }
        if self.paused {
            return FrameTime {
                dts: 0.0,
                tts: self.game_time,
            };
        }
        self.game_time += real_delta;
        FrameTime {
            dts: real_delta,
            tts: self.game_time,
        }
    }

    /// The fixed-timestep model loop: the zero-or-more [`FIXED_DT`] sub-frames to
    /// run `tick` for this rendered frame, given the real wall-clock delta since
    /// the last frame. Unlike [`Self::frame`] (one variable-dt tick per rendered
    /// frame) this decouples the model rate from the render rate — so the sim is
    /// deterministic and a recorded frame is exactly one forward-step fine step
    /// (docs/time-travel.md). The shell runs `tick` once per returned frame, in
    /// order, then renders ONCE at [`Self::current_tts`].
    ///
    /// - **Fixed-time** pins unconditionally (checked first): a single
    ///   `{ dts: 0, tts: <const> }` — the golden-capture path runs the body once
    ///   with `dts = 0`, byte-identical to [`Self::frame`].
    /// - **Pending step** (Step / debug Advance): a single `{ dts: step, … }`.
    /// - **Paused**: EMPTY — no model advance. The shell still renders the frozen
    ///   (or scrubbed) pose once at the held `tts`.
    /// - **Live**: accumulate `real_delta` and emit one frame per whole
    ///   [`FIXED_DT`] of backlog, clamped to [`MAX_SUBSTEPS`] so a stall doesn't
    ///   replay as a burst. The remainder carries to the next rendered frame, so
    ///   at >60fps most frames emit 0 and every ~Nth emits 1; at <60fps a frame
    ///   emits 2+ to keep the model caught up to wall-clock.
    pub fn fixed_frames(&mut self, real_delta: f32) -> Vec<FrameTime> {
        if let Some(t) = self.fixed_time {
            return vec![FrameTime { dts: 0.0, tts: t }];
        }
        if let Some(step) = self.pending_step.take() {
            self.game_time += step;
            return vec![FrameTime {
                dts: step,
                tts: self.game_time,
            }];
        }
        if self.paused {
            return Vec::new();
        }
        self.accumulator += real_delta;
        let max_backlog = FIXED_DT * MAX_SUBSTEPS as f32;
        if self.accumulator > max_backlog {
            self.accumulator = max_backlog;
        }
        let mut frames = Vec::new();
        while self.accumulator >= FIXED_DT {
            self.accumulator -= FIXED_DT;
            self.game_time += FIXED_DT;
            frames.push(FrameTime {
                dts: FIXED_DT,
                tts: self.game_time,
            });
        }
        frames
    }

    /// The current game time (`tts`) for rendering / hot-reload / capture — the
    /// value after this frame's fixed steps, or the frozen `game_time` while
    /// paused, or the pinned constant under `--fixed-time`. The shell renders at
    /// this `tts` so the drawn pose reflects the settled sim.
    pub fn current_tts(&self) -> f32 {
        self.fixed_time.unwrap_or(self.game_time)
    }

    /// Whether the scrubber/debug pause is engaged (Step also pauses).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Whether the clock is pinned this frame — paused OR fixed-time. The shells
    /// use this to freeze user input in deterministic/paused modes (matching the
    /// old `held_time.is_some()`).
    pub fn is_pinned(&self) -> bool {
        self.paused || self.fixed_time.is_some()
    }

    /// Toggle the pause state. Resuming continues from `game_time` (NOT wall
    /// clock) — this is what kills the pause→resume jump.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.pending_step = None;
    }

    /// Engage the pause (idempotent), dropping any queued step. Used by the
    /// scrubber's seek/step controls, which park on a frame.
    pub fn pause(&mut self) {
        self.paused = true;
        self.pending_step = None;
    }

    /// Return to live wall-clock accumulation, continuing from the current
    /// `game_time` (debug `POST /time` Resume).
    pub fn resume(&mut self) {
        self.paused = false;
        self.pending_step = None;
    }

    /// Pause and queue a one-frame step of `dt` seconds (Step / debug Advance).
    pub fn step(&mut self, dt: f32) {
        self.paused = true;
        self.pending_step = Some(dt);
    }

    /// Debug `POST /time` Set: pin the clock to `tts` (pause + rebase).
    pub fn set(&mut self, tts: f32) {
        self.paused = true;
        self.pending_step = None;
        self.game_time = tts;
    }

    /// Debug `POST /time` Advance: step one frame by `dts` (implies pause).
    pub fn advance(&mut self, dts: f32) {
        self.step(dts);
    }

    /// Rebase the clock so play continues from `tts` — used when a time-travel
    /// branch resumes from a scrubbed frame's recorded time (docs/time-travel.md).
    pub fn rebase(&mut self, tts: f32) {
        self.game_time = tts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_accumulates_real_delta() {
        let mut clock = GameClock::new(None);
        let a = clock.frame(0.1);
        assert_eq!(a.dts, 0.1);
        assert_eq!(a.tts, 0.1);
        let b = clock.frame(0.2);
        assert_eq!(b.dts, 0.2);
        assert!((b.tts - 0.3).abs() < 1e-6);
    }

    #[test]
    fn fixed_time_is_an_unconditional_pin() {
        let mut clock = GameClock::new(Some(2.0));
        // Every frame is the constant, regardless of delta, pause, or step.
        assert_eq!(clock.frame(0.1).tts, 2.0);
        assert_eq!(clock.frame(0.1).dts, 0.0);
        clock.pause();
        clock.step(1.0 / 60.0);
        clock.rebase(9.0);
        let f = clock.frame(0.5);
        assert_eq!(f.tts, 2.0);
        assert_eq!(f.dts, 0.0);
    }

    #[test]
    fn pause_then_resume_does_not_jump() {
        let mut clock = GameClock::new(None);
        // Advance to t = 1.0 while live.
        for _ in 0..10 {
            clock.frame(0.1);
        }
        assert!((clock.frame(0.0).tts - 1.0).abs() < 1e-6);
        clock.pause();
        // Wall clock keeps ticking while paused (large deltas), but tts freezes.
        for _ in 0..100 {
            let f = clock.frame(1.0);
            assert_eq!(f.dts, 0.0);
            assert!((f.tts - 1.0).abs() < 1e-6);
        }
        // On resume, tts continues from the freeze point, NOT wall-clock (which
        // advanced ~100s while paused).
        clock.toggle_pause();
        let f = clock.frame(0.1);
        assert_eq!(f.dts, 0.1);
        assert!((f.tts - 1.1).abs() < 1e-6, "resumed at {}", f.tts);
    }

    #[test]
    fn step_advances_one_frame_then_holds() {
        let mut clock = GameClock::new(None);
        clock.frame(0.5); // t = 0.5
        clock.step(1.0 / 60.0);
        let a = clock.frame(1.0); // real delta ignored — a single step
        assert!((a.dts - 1.0 / 60.0).abs() < 1e-6);
        assert!((a.tts - (0.5 + 1.0 / 60.0)).abs() < 1e-6);
        // ...then holds (paused) until the next control.
        let b = clock.frame(1.0);
        assert_eq!(b.dts, 0.0);
        assert!((b.tts - a.tts).abs() < 1e-6);
    }

    #[test]
    fn rebase_continues_from_the_branch_time() {
        let mut clock = GameClock::new(None);
        for _ in 0..50 {
            clock.frame(0.1); // t ≈ 5.0 (wall-clock "now")
        }
        // A time-travel branch resumes from an early recorded frame (tts ≈ 1.0).
        clock.rebase(1.0);
        let f = clock.frame(0.1);
        assert_eq!(f.dts, 0.1);
        assert!((f.tts - 1.1).abs() < 1e-6, "rebased play at {}", f.tts);
    }

    // --- fixed_frames (the fixed-timestep model loop) ---

    #[test]
    fn fixed_frames_emit_whole_steps_at_fixed_dt() {
        let mut clock = GameClock::new(None);
        // One render frame worth 2.5 fixed steps of real time → 2 steps emitted,
        // 0.5 * FIXED_DT carried.
        let frames = clock.fixed_frames(FIXED_DT * 2.5);
        assert_eq!(frames.len(), 2);
        assert!((frames[0].dts - FIXED_DT).abs() < 1e-6);
        assert!((frames[0].tts - FIXED_DT).abs() < 1e-6);
        assert!((frames[1].tts - 2.0 * FIXED_DT).abs() < 1e-6);
        // The carried 0.5 step + another 0.6 crosses one whole step next frame.
        let next = clock.fixed_frames(FIXED_DT * 0.6);
        assert_eq!(next.len(), 1);
        assert!((next[0].tts - 3.0 * FIXED_DT).abs() < 1e-6);
        assert!((clock.current_tts() - 3.0 * FIXED_DT).abs() < 1e-6);
    }

    #[test]
    fn fixed_frames_above_60fps_mostly_zero_then_one() {
        // At 144fps each render frame is ~0.4 of a fixed step: most frames emit
        // 0, and a step lands roughly every ~2.4 frames. Over many frames the
        // game time tracks wall-clock elapsed to within one fixed step.
        let mut clock = GameClock::new(None);
        let dt = 1.0 / 144.0;
        let mut total = 0usize;
        for _ in 0..144 {
            total += clock.fixed_frames(dt).len();
        }
        // ~1s of wall-clock → ~60 fixed steps.
        assert!((59..=61).contains(&total), "emitted {total} steps in 1s");
        // Game time tracks wall-clock to within a fixed step (plus f32 slop from
        // summing 144 deltas).
        assert!((clock.current_tts() - 1.0).abs() < 2.0 * FIXED_DT);
    }

    #[test]
    fn fixed_frames_below_60fps_catches_up_with_multiple_steps() {
        // At 30fps each render frame is 2 fixed steps → 2 emitted per frame.
        let mut clock = GameClock::new(None);
        let frames = clock.fixed_frames(1.0 / 30.0);
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn fixed_frames_clamps_the_backlog_after_a_stall() {
        // A 10s stall must not replay as 600 steps — capped at MAX_SUBSTEPS.
        let mut clock = GameClock::new(None);
        let frames = clock.fixed_frames(10.0);
        assert_eq!(frames.len(), MAX_SUBSTEPS);
    }

    #[test]
    fn fixed_frames_paused_emits_nothing_and_holds_tts() {
        let mut clock = GameClock::new(None);
        clock.fixed_frames(FIXED_DT * 3.0); // advance to 3 * FIXED_DT
        let held = clock.current_tts();
        clock.pause();
        for _ in 0..100 {
            assert!(clock.fixed_frames(1.0).is_empty());
        }
        // Paused: no steps, tts frozen — the shell renders the held pose.
        assert!((clock.current_tts() - held).abs() < 1e-6);
    }

    #[test]
    fn fixed_frames_fixed_time_is_a_single_pinned_step() {
        let mut clock = GameClock::new(Some(2.0));
        let frames = clock.fixed_frames(0.1);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].dts, 0.0);
        assert_eq!(frames[0].tts, 2.0);
        assert_eq!(clock.current_tts(), 2.0);
    }

    #[test]
    fn fixed_frames_step_advances_one_then_holds() {
        let mut clock = GameClock::new(None);
        clock.fixed_frames(FIXED_DT); // t = FIXED_DT
        clock.step(1.0 / 60.0);
        let a = clock.fixed_frames(1.0); // real delta ignored — one step
        assert_eq!(a.len(), 1);
        assert!((a[0].dts - 1.0 / 60.0).abs() < 1e-6);
        // ...then holds (paused).
        assert!(clock.fixed_frames(1.0).is_empty());
    }
}
