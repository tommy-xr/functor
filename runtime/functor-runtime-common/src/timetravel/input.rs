use std::collections::BTreeMap;

use super::{retention_floor, Frame};
use crate::input::RecordedInput;

/// Sparse input history with O(events) storage rather than one heap-sized
/// `Vec` header per input-free frame. Edge-only games retain their whole
/// session so a reload can replay from `init`. Once a continuously sampled
/// snapshot appears, the log adopts the model ring's bounded horizon instead:
/// one coeffect per rendered frame must not grow for the lifetime of a process.
/// The contiguous frame range is tracked separately because fixed-step replay
/// still advances through empty frames.
pub(super) struct InputHistory {
    base: Option<Frame>,
    next: Option<Frame>,
    pub(super) events: BTreeMap<Frame, Vec<RecordedInput>>,
    has_sampled_snapshots: bool,
    /// Whether the currently loaded program's origin replay requirements are
    /// satisfied. Recomputed before each reload: a destructive rewind may
    /// legitimately discard a historical coeffect gap.
    pub(super) origin_replay_complete: bool,
}

impl InputHistory {
    pub(super) fn new() -> InputHistory {
        InputHistory {
            base: None,
            next: None,
            events: BTreeMap::new(),
            has_sampled_snapshots: false,
            origin_replay_complete: true,
        }
    }

    pub(super) fn record(&mut self, frame: Frame, inputs: Vec<RecordedInput>) {
        match self.next {
            None => self.base = Some(frame),
            Some(next) => assert_eq!(frame, next, "input frames must be recorded consecutively"),
        }
        if inputs
            .iter()
            .any(|input| matches!(input, RecordedInput::Snapshot(_)))
        {
            self.has_sampled_snapshots = true;
        }
        if !inputs.is_empty() {
            self.events.insert(frame, inputs);
        }
        self.next = Some(frame + 1);
        if self.has_sampled_snapshots {
            let retain_from = retention_floor(frame);
            if self.base.is_some_and(|base| retain_from > base) {
                self.events = self.events.split_off(&retain_from);
                self.base = Some(retain_from);
            }
        }
    }

    pub(super) fn recorded_range(&self) -> Option<(Frame, Frame)> {
        self.base
            .zip(self.next)
            .and_then(|(base, next)| (next > base).then_some((base, next - 1)))
    }

    pub(super) fn at(&self, frame: Frame) -> &[RecordedInput] {
        self.events.get(&frame).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(super) fn supports_origin_replay_through(&self, frame: Frame) -> bool {
        self.origin_replay_complete
            && (!self.has_sampled_snapshots
                || self
                    .recorded_range()
                    .is_some_and(|(lo, hi)| lo == 0 && hi >= frame))
    }

    fn has_snapshot_at(&self, frame: Frame) -> bool {
        self.at(frame)
            .iter()
            .any(|input| matches!(input, RecordedInput::Snapshot(_)))
    }

    pub(super) fn sampled_history_is_complete_from_origin(&self) -> bool {
        match self.recorded_range() {
            None => true,
            Some((0, hi)) => (0..=hi).all(|frame| self.has_snapshot_at(frame)),
            Some(_) => false,
        }
    }

    pub(super) fn truncate_from(&mut self, frame: Frame) {
        let Some(next) = self.next else { return };
        if frame >= next {
            return;
        }
        let base = self
            .base
            .expect("input history with a next frame has a base");
        assert!(
            frame >= base,
            "truncate_from({frame}) predates input history (base {base})"
        );
        self.events.split_off(&frame);
        self.next = Some(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timetravel::DEFAULT_HISTORY_FRAMES;

    #[test]
    fn sampled_input_history_uses_the_model_retention_horizon() {
        let mut history = InputHistory::new();
        for frame in 0..=(DEFAULT_HISTORY_FRAMES as u64 + 1) {
            history.record(
                frame,
                vec![RecordedInput::Snapshot(Box::new(
                    crate::InputSnapshot::default(),
                ))],
            );
        }

        assert_eq!(
            history.recorded_range(),
            Some((2, DEFAULT_HISTORY_FRAMES as u64 + 1))
        );
        assert!(history.at(0).is_empty());
        assert!(matches!(
            history.at(2),
            [RecordedInput::Snapshot(snapshot)]
                if snapshot.as_ref() == &crate::InputSnapshot::default()
        ));
    }
}
