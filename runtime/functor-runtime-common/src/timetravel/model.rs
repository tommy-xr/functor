use functor_lang::Value;

use super::{fnv1a, Frame, FrameEnd, History, Live, Participant, Restore};

/// Participant 0: the MVU `model`, one `Rc`-shared snapshot per frame.
pub(super) struct ModelParticipant {
    pub(super) model_history: History<Value>,
}

/// The model's digest: its `Display` text, as the debug protocol shows it.
fn digest(model: &Value) -> u64 {
    fnv1a(model.to_string().as_bytes())
}

impl Participant for ModelParticipant {
    fn name(&self) -> &'static str {
        "model"
    }

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

    fn recorded_digest(&self, frame: Frame) -> Option<u64> {
        self.model_history.get(frame).map(digest)
    }

    fn live_digest(&self, live: &Live) -> u64 {
        digest(live.model)
    }
}
