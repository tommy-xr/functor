use std::collections::BTreeMap;

use super::{retention_floor, Frame};
use crate::functor_lang_prelude::EffectValue;
use crate::input::RecordedInput;

/// One occurrence on the timeline: the `ordinal`-th effect op of rendered
/// `frame`, on the branch `epoch` (the recorder's generation). Ops queued
/// between frames are provisional until the frame records them, so the id is
/// stamped then: paused input's ops belong to whichever frame Δ becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpId {
    pub epoch: u64,
    pub frame: Frame,
    pub ordinal: u32,
}

/// One frame-log entry, in the order the frame produced it.
#[derive(Clone, Debug)]
pub enum Entry {
    Input(RecordedInput),
    /// A broker answer (`Effect.now`, `Effect.random`, `Persistence.load`):
    /// a replay hands `value` back at the matching effect call instead of
    /// asking the world, after checking `kind` and the `args` digest.
    Answer {
        op: OpId,
        kind: &'static str,
        args: u64,
        value: EffectValue,
    },
    /// An outbound intent (`Persistence.save`, HTTP, sends, sounds, preloads),
    /// recorded whether or not it was dispatched. A replay validates it and
    /// never dispatches it.
    Issued {
        op: OpId,
        kind: &'static str,
        args: u64,
    },
}

impl Entry {
    pub fn answer(kind: &'static str, args: u64, value: EffectValue) -> Entry {
        Entry::Answer {
            op: OpId::default(),
            kind,
            args,
            value,
        }
    }

    pub fn issued(kind: &'static str, args: u64) -> Entry {
        Entry::Issued {
            op: OpId::default(),
            kind,
            args,
        }
    }

    pub fn input(&self) -> Option<&RecordedInput> {
        match self {
            Entry::Input(input) => Some(input),
            _ => None,
        }
    }

    /// `(op, kind, args)` of an effect op; `None` for input.
    pub fn op(&self) -> Option<(OpId, &'static str, u64)> {
        match self {
            Entry::Input(_) => None,
            Entry::Answer { op, kind, args, .. } | Entry::Issued { op, kind, args } => {
                Some((*op, kind, *args))
            }
        }
    }

    fn op_mut(&mut self) -> Option<&mut OpId> {
        match self {
            Entry::Input(_) => None,
            Entry::Answer { op, .. } | Entry::Issued { op, .. } => Some(op),
        }
    }
}

/// FNV-1a digest of an effect op's arguments, over their `Debug` text: stable
/// within a build, which is all in-session replay needs.
pub fn args_digest(args: &impl std::fmt::Debug) -> u64 {
    fnv1a(format!("{args:?}").as_bytes())
}

/// FNV-1a: stable across toolchains, so digests and fuzz seeds reproduce.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The frame log (design-systems §2.4): each rendered frame's input and effect
/// ops, sparse, in O(entries) storage. Edge input stays session-long so a
/// reload can replay from `init`; once a continuously sampled snapshot appears,
/// input adopts the model ring's bounded horizon (one coeffect per frame must
/// not grow for the process lifetime). Effect ops always use that horizon:
/// nothing replays them from the origin. The contiguous frame range is tracked
/// separately because fixed-step replay still advances through empty frames.
pub(super) struct FrameLog {
    base: Option<Frame>,
    next: Option<Frame>,
    pub(super) events: BTreeMap<Frame, Vec<Entry>>,
    /// Frames below this hold input only.
    ops_floor: Frame,
    has_sampled_snapshots: bool,
    /// Whether the currently loaded program's origin replay requirements are
    /// satisfied. Recomputed before each reload: a destructive rewind may
    /// legitimately discard a historical coeffect gap.
    pub(super) origin_replay_complete: bool,
}

impl FrameLog {
    pub(super) fn new() -> FrameLog {
        FrameLog {
            base: None,
            next: None,
            events: BTreeMap::new(),
            ops_floor: 0,
            has_sampled_snapshots: false,
            origin_replay_complete: true,
        }
    }

    /// Commit Δ as `frame`'s log, stamping its ops' ids.
    pub(super) fn record(&mut self, frame: Frame, epoch: u64, mut entries: Vec<Entry>) {
        match self.next {
            None => self.base = Some(frame),
            Some(next) => assert_eq!(frame, next, "input frames must be recorded consecutively"),
        }
        if entries
            .iter()
            .any(|entry| matches!(entry, Entry::Input(RecordedInput::Snapshot(_))))
        {
            self.has_sampled_snapshots = true;
        }
        for (ordinal, op) in entries.iter_mut().filter_map(Entry::op_mut).enumerate() {
            *op = OpId {
                epoch,
                frame,
                ordinal: ordinal as u32,
            };
        }
        if !entries.is_empty() {
            self.events.insert(frame, entries);
        }
        self.next = Some(frame + 1);
        let retain_from = retention_floor(frame);
        if self.has_sampled_snapshots {
            if self.base.is_some_and(|base| retain_from > base) {
                self.events = self.events.split_off(&retain_from);
                self.base = Some(retain_from);
            }
        } else if retain_from > self.ops_floor {
            let aged: Vec<Frame> = self
                .events
                .range(self.ops_floor..retain_from)
                .map(|(f, _)| *f)
                .collect();
            for frame in aged {
                let entries = self.events.get_mut(&frame).expect("key just listed");
                entries.retain(|entry| entry.input().is_some());
                if entries.is_empty() {
                    self.events.remove(&frame);
                }
            }
        }
        self.ops_floor = self.ops_floor.max(retain_from);
    }

    pub(super) fn recorded_range(&self) -> Option<(Frame, Frame)> {
        self.base
            .zip(self.next)
            .and_then(|(base, next)| (next > base).then_some((base, next - 1)))
    }

    pub(super) fn at(&self, frame: Frame) -> &[Entry] {
        self.events.get(&frame).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(super) fn inputs(&self, frame: Frame) -> impl Iterator<Item = &RecordedInput> {
        self.at(frame).iter().filter_map(Entry::input)
    }

    pub(super) fn supports_origin_replay_through(&self, frame: Frame) -> bool {
        self.origin_replay_complete
            && (!self.has_sampled_snapshots
                || self
                    .recorded_range()
                    .is_some_and(|(lo, hi)| lo == 0 && hi >= frame))
    }

    fn has_snapshot_at(&self, frame: Frame) -> bool {
        self.inputs(frame)
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

    fn snapshot() -> Entry {
        Entry::Input(RecordedInput::Snapshot(Box::new(
            crate::InputSnapshot::default(),
        )))
    }

    #[test]
    fn sampled_input_history_uses_the_model_retention_horizon() {
        let mut history = FrameLog::new();
        for frame in 0..=(DEFAULT_HISTORY_FRAMES as u64 + 1) {
            history.record(frame, 0, vec![snapshot()]);
        }

        assert_eq!(
            history.recorded_range(),
            Some((2, DEFAULT_HISTORY_FRAMES as u64 + 1))
        );
        assert!(history.at(0).is_empty());
        assert!(matches!(
            history.inputs(2).collect::<Vec<_>>().as_slice(),
            [RecordedInput::Snapshot(snapshot)]
                if snapshot.as_ref() == &crate::InputSnapshot::default()
        ));
    }

    #[test]
    fn ops_are_stamped_on_commit_and_age_out_while_edge_input_stays() {
        let key = || {
            Entry::Input(RecordedInput::Key {
                code: 30,
                is_down: true,
            })
        };
        let mut log = FrameLog::new();
        for frame in 0..=(DEFAULT_HISTORY_FRAMES as u64) {
            let random = Entry::answer("random", 0, EffectValue::Number(frame as f64));
            log.record(
                frame,
                7,
                vec![key(), random, Entry::issued("storage.save", 1)],
            );
        }
        let ops: Vec<_> = log.at(5).iter().filter_map(Entry::op).collect();
        assert_eq!(
            ops,
            vec![
                (
                    OpId {
                        epoch: 7,
                        frame: 5,
                        ordinal: 0
                    },
                    "random",
                    0
                ),
                (
                    OpId {
                        epoch: 7,
                        frame: 5,
                        ordinal: 1
                    },
                    "storage.save",
                    1
                ),
            ]
        );
        // Frame 0's ops left the window; its edge input did not.
        assert_eq!(
            log.recorded_range(),
            Some((0, DEFAULT_HISTORY_FRAMES as u64))
        );
        assert!(matches!(log.at(0), [Entry::Input(_)]));
    }
}
