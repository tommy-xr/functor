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

/// A frame-time source shared by the desktop and web shells. Constructed once
/// per session (seeded with the shell's fixed-time option) and called each
/// frame via [`GameClock::frame`].
pub struct GameClock {
    /// Accumulated game time in seconds — the `tts` handed to the game. Advances
    /// by the real frame delta while live; frozen while paused; jumps on rebase.
    game_time: f32,
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
}
