use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct FrameTime {
    // tts - total time in seconds
    pub tts: f32,

    // dts - delta time in seconds for this frame
    pub dts: f32,
}

/// Sleeps a loop to one iteration per `period` by deadline, so oversleeping
/// one iteration shortens the next sleep instead of lowering the rate. A late
/// iteration restarts the schedule rather than rushing to catch up.
pub struct FramePacer {
    period: Duration,
    deadline: Option<Instant>,
}

impl FramePacer {
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            deadline: None,
        }
    }

    /// How long to sleep at the end of the iteration that ends at `now`.
    pub fn delay(&mut self, now: Instant) -> Duration {
        let deadline = self
            .deadline
            .map_or(now, |last| last + self.period)
            .max(now);
        self.deadline = Some(deadline);
        deadline - now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_sleeps_to_each_deadline_and_restarts_after_a_late_frame() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(ms(10));
        assert_eq!(pacer.delay(t0), Duration::ZERO, "the first frame starts the schedule");
        assert_eq!(pacer.delay(t0 + ms(3)), ms(7));
        // Overslept to 10.5 ms: the next frame still ends on 20 ms.
        assert_eq!(pacer.delay(t0 + ms(13)), ms(7));
        assert_eq!(pacer.delay(t0 + ms(35)), Duration::ZERO, "late: no sleep");
        assert_eq!(pacer.delay(t0 + ms(38)), ms(7), "the schedule restarts at the late frame");
        let mut uncapped = FramePacer::new(Duration::ZERO);
        assert_eq!(uncapped.delay(t0), Duration::ZERO);
        assert_eq!(uncapped.delay(t0 + ms(1)), Duration::ZERO);
    }
}
