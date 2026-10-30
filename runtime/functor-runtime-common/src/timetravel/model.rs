use functor_lang::Value;

use super::{Frame, FrameEnd, History, Live, Participant, Restore};

/// Participant 0: the MVU `model`, one `Rc`-shared snapshot per frame.
pub(super) struct ModelParticipant {
    pub(super) model_history: History<Value>,
}

impl Participant for ModelParticipant {
    fn record(&mut self, frame: Frame, end: &FrameEnd) {
        self.model_history.record(frame, end.model);
    }

    fn range(&self) -> Option<(Frame, Frame)> {
        self.model_history.recorded_range()
    }

    fn restore(&mut self, frame: Frame, how: Restore, live: &mut Live) {
        if how != Restore::Resume {
            *live.model = self.model_history.seek(frame).clone();
        }
    }

    fn truncate_from(&mut self, frame: Frame) {
        self.model_history.truncate_from(frame);
    }

    fn prune(&mut self, floor: Frame) {
        self.model_history.prune_before(floor);
    }

    fn clear(&mut self) {
        self.model_history = History::unbounded();
    }
}
