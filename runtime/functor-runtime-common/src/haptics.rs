//! Controller vibration the game asks for (`Effect.haptic`), for the shell to
//! perform.
//!
//! The effect drain offers each pulse here; the shell takes them once per
//! frame: with the snapshot ([`crate::sim_driver::SimSnapshot`]) on desktop
//! and Quest, through [`take_unactuated`] on web. Between two
//! takes a hand keeps one pulse: the strongest, ties going to the longer, and
//! the rest count as dropped. A dry run (fork, ghost) offers nothing, and a
//! seek or rewind restores frames without re-running their effects, so
//! neither actuates nor counts.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The longest pulse `Effect.haptic` accepts, in seconds.
pub const MAX_SECONDS: f64 = 5.0;

/// Which controller vibrates (`Xr.Left` / `Xr.Right`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "HapticHand"))]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    Left,
    Right,
}

/// One vibration: `amplitude` in `0..=1`, lasting `seconds` (0 = the
/// device's shortest pulse).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HapticPulse {
    pub hand: Hand,
    pub amplitude: f32,
    pub seconds: f32,
}

impl HapticPulse {
    /// Stronger, or as strong and longer.
    fn beats(&self, other: &HapticPulse) -> bool {
        (self.amplitude, self.seconds) > (other.amplitude, other.seconds)
    }
}

/// The pulses one frame performs: at most one per hand.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HapticFrame {
    pub left: Option<HapticPulse>,
    pub right: Option<HapticPulse>,
    /// Pulses that lost to a stronger (or as strong and longer) one.
    pub superseded: u32,
}

impl HapticFrame {
    /// Keep `pulse` if it beats its hand's current one.
    pub fn offer(&mut self, pulse: HapticPulse) {
        let slot = match pulse.hand {
            Hand::Left => &mut self.left,
            Hand::Right => &mut self.right,
        };
        match slot {
            Some(kept) if !pulse.beats(kept) => self.superseded += 1,
            Some(_) => {
                *slot = Some(pulse);
                self.superseded += 1;
            }
            None => *slot = Some(pulse),
        }
    }

    /// Fold in a frame the shell skipped, arbitrating per hand again.
    pub fn absorb(&mut self, older: HapticFrame) {
        self.superseded += older.superseded;
        for pulse in older.pulses() {
            self.offer(pulse);
        }
    }

    pub fn pulses(&self) -> impl Iterator<Item = HapticPulse> {
        self.left.into_iter().chain(self.right)
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.superseded == 0
    }
}

static PENDING: Mutex<HapticFrame> = Mutex::new(HapticFrame {
    left: None,
    right: None,
    superseded: 0,
});

/// Queue a pulse for this frame (the effect drain calls this).
pub fn offer(pulse: HapticPulse) {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).offer(pulse);
}

/// Take the frame's pulses (the snapshot calls this).
pub fn take() -> HapticFrame {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A pulse the shell took, as `GET /state` lists it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HapticRecord {
    pub hand: Hand,
    pub amplitude: f32,
    pub seconds: f32,
    /// `GET /state`'s `frame` when the shell took it.
    pub frame: u64,
}

/// Pulses since launch (`GET /state.haptics`): `sent` were handed to the
/// controller (counted without one on desktop and headless), `dropped` lost
/// their frame's arbitration or were refused (an unfocused headset session,
/// or one that stopped before they played). `recent` keeps the last few sent, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HapticCounts {
    pub sent: u64,
    pub dropped: u64,
    pub recent: Vec<HapticRecord>,
}

/// How many pulses [`HapticCounts::recent`] keeps.
const RECENT_PULSES: usize = 8;

static COUNTS: Mutex<HapticCounts> = Mutex::new(HapticCounts {
    sent: 0,
    dropped: 0,
    recent: Vec::new(),
});

/// Count a taken frame. `perform` actuates one pulse and says whether the
/// device took it; `frame` is `GET /state`'s `frame`.
pub fn record(taken: &HapticFrame, frame: u64, mut perform: impl FnMut(&HapticPulse) -> bool) {
    if taken.is_empty() {
        return;
    }
    let mut counts = COUNTS.lock().unwrap_or_else(|e| e.into_inner());
    counts.dropped += u64::from(taken.superseded);
    for pulse in taken.pulses() {
        if !perform(&pulse) {
            counts.dropped += 1;
            continue;
        }
        counts.sent += 1;
        if counts.recent.len() == RECENT_PULSES {
            counts.recent.remove(0);
        }
        counts.recent.push(HapticRecord {
            hand: pulse.hand,
            amplitude: pulse.amplitude,
            seconds: pulse.seconds,
            frame,
        });
    }
}

/// Take the frame's pulses and count each as `sent`, for a shell with no
/// actuator that takes no snapshot (web); `frame` is `GET /state`'s `frame`.
pub fn take_unactuated(frame: u64) {
    record(&take(), frame, |_| true);
}

/// The counts so far (see [`HapticCounts`]).
pub fn counts() -> HapticCounts {
    COUNTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Serializes tests that touch the process-global queue.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn pulse(hand: Hand, amplitude: f32, seconds: f32) -> HapticPulse {
        HapticPulse {
            hand,
            amplitude,
            seconds,
        }
    }

    #[test]
    fn strongest_pulse_per_hand_wins_ties_to_the_longer() {
        let mut frame = HapticFrame::default();
        frame.offer(pulse(Hand::Left, 0.3, 0.5));
        frame.offer(pulse(Hand::Left, 0.8, 0.02));
        frame.offer(pulse(Hand::Left, 0.8, 0.01));
        frame.offer(pulse(Hand::Left, 0.8, 0.04));
        frame.offer(pulse(Hand::Right, 0.1, 0.1));
        assert_eq!(frame.left, Some(pulse(Hand::Left, 0.8, 0.04)));
        assert_eq!(frame.right, Some(pulse(Hand::Right, 0.1, 0.1)));
        assert_eq!(frame.superseded, 3);
    }

    #[test]
    fn absorbing_a_skipped_frame_arbitrates_again() {
        let mut older = HapticFrame::default();
        older.offer(pulse(Hand::Right, 0.9, 0.1));
        let mut newer = HapticFrame::default();
        newer.offer(pulse(Hand::Right, 0.2, 0.1));
        newer.absorb(older);
        assert_eq!(newer.right, Some(pulse(Hand::Right, 0.9, 0.1)));
        assert_eq!(newer.superseded, 1);
    }

    #[test]
    fn take_unactuated_drains_the_queue_and_counts_it_sent() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take();
        let before = counts();
        offer(pulse(Hand::Right, 0.6, 0.03));
        offer(pulse(Hand::Right, 0.2, 0.03));
        take_unactuated(4);
        assert!(take().is_empty(), "drained");
        let after = counts();
        assert_eq!(after.sent, before.sent + 1);
        assert_eq!(after.dropped, before.dropped + 1, "the superseded one");
        assert_eq!(after.recent.last().map(|r| r.frame), Some(4));
    }

    #[test]
    fn record_counts_sent_refused_and_superseded() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = counts();
        let mut frame = HapticFrame::default();
        frame.offer(pulse(Hand::Left, 0.5, 0.1));
        frame.offer(pulse(Hand::Left, 0.4, 0.1));
        frame.offer(pulse(Hand::Right, 0.5, 0.1));
        record(&frame, 7, |p| p.hand == Hand::Left);
        let after = counts();
        assert_eq!(after.sent, before.sent + 1);
        assert_eq!(after.dropped, before.dropped + 2, "one superseded, one refused");
        assert_eq!(
            after.recent.last(),
            Some(&HapticRecord {
                hand: Hand::Left,
                amplitude: 0.5,
                seconds: 0.1,
                frame: 7
            })
        );
    }
}
