//! The game view's transport (D13): the session's clock and recorded history
//! as its debug server reports them, the marks on that history, and the calls
//! the bar makes. Pure: the shell polls and posts, and feeds the answers in.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::protocol::GameInput;
use crate::session::PolledClock;

/// What the bar shows: `GET /state`'s `frame` and `clock`, with the bar's
/// own calls applied until the runtime has answered them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Transport {
    /// The runtime reports its clock (debug protocol v37): without it there
    /// is a frame to show but nothing to drive.
    pub clock: bool,
    /// Pinned: paused, stepped, or parked on a seek.
    pub paused: bool,
    /// The recorded steps behind the state shown (`/state.frame`).
    pub frame: u64,
    /// The seekable window `[oldest, newest]`, numbered as `frame`; `None`
    /// until a step is recorded.
    pub recorded: Option<[u64; 2]>,
    /// Bumps when recorded frames are replaced: a step or resume from a
    /// seeked frame branches, and so may a hot reload.
    pub generation: u64,
    /// The `id` of the last seek asked for (0: none), so the page that asked
    /// stops waiting for it.
    pub seek: u64,
}

/// An event on the timeline: an input edge or a hot reload, with the kind
/// and label the web timeline gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Mark {
    pub id: u64,
    pub frame: u64,
    /// `key-down`, `mouse-button-up`, `ui-input`, `reload-ok`, ….
    pub kind: String,
    pub label: String,
}

/// What the bar asks of the session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportCall {
    Pause,
    Resume,
    /// One frame, then hold.
    Step,
    /// Show recorded frame `frame`, paused, keeping the future. `id` is the
    /// asking page's, echoed as [`Transport::seek`].
    Seek {
        frame: u64,
        id: u64,
    },
}

/// One request to the session's debug server. They are posted in order, one
/// at a time, so a call never overtakes one sent before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionCall {
    Input(GameInput),
    Transport(TransportCall),
    /// `GET /state?model=0`, sent after `after` transport calls.
    Poll {
        after: u64,
    },
}

/// Queues `call`. A mouse move or seek still waiting at the back is replaced
/// by a newer one: only the latest pointer position or drag target matters.
pub fn enqueue(queue: &mut VecDeque<SessionCall>, call: SessionCall) {
    use SessionCall::{Input, Transport};
    let replaces = matches!(
        (queue.back(), &call),
        (
            Some(Input(GameInput::MouseMove { .. })),
            Input(GameInput::MouseMove { .. })
        ) | (
            Some(Transport(TransportCall::Seek { .. })),
            Transport(TransportCall::Seek { .. })
        )
    );
    if replaces {
        queue.pop_back();
    }
    queue.push_back(call);
}

/// A `GET /input-log` read: its generation and window, and each step's marks
/// as `(kind, label)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogPage {
    pub generation: u64,
    pub oldest: u64,
    pub newest: u64,
    pub steps: Vec<(u64, Vec<(String, String)>)>,
}

/// More marks than this drop the oldest (the web timeline's cap).
const MARK_CAP: usize = 4096;

/// A running session's transport: its clock and marks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    /// `None` until the first successful poll.
    pub clock: Option<Transport>,
    /// Transport calls sent this session. A poll sent before the latest is
    /// answered before that call lands, so its frame and pause are stale.
    pub calls: u64,
    /// In the recorded window, oldest first.
    pub marks: Vec<Mark>,
    /// Bumps when `marks` changes.
    pub marks_rev: u64,
    /// An input-log read is on its way.
    pub reading: bool,
    next_mark: u64,
    /// The next input-log frame to read; `None`: from the oldest.
    cursor: Option<u64>,
    /// The input-log generation the input marks belong to.
    log_generation: Option<u64>,
    /// The last program revision polled: a new one is a hot reload.
    revision: Option<u64>,
}

impl Timeline {
    /// A poll sent after `after` transport calls reported `frame` and `clock`.
    pub fn polled(&mut self, after: u64, frame: u64, clock: Option<PolledClock>) {
        let stale = after < self.calls;
        let Some(clock) = clock else {
            self.clock = Some(Transport {
                frame,
                ..Transport::default()
            });
            return;
        };
        let shown = self.clock.filter(|c| c.clock);
        let branched = shown.is_some_and(|c| c.generation != clock.generation);
        // A call not yet answered: keep showing what it asked for.
        let (frame, paused) = match shown {
            Some(c) if stale => (c.frame, c.paused),
            _ => (frame, clock.paused),
        };
        // A reload lands on the frame shown through the swap (as on the web);
        // the first poll is a baseline, not news.
        if self.revision.is_some_and(|r| r != clock.program_revision) {
            self.push(frame, "reload-ok", "hot reload".to_string());
        }
        self.revision = Some(clock.program_revision);
        self.clock = Some(Transport {
            clock: true,
            paused,
            frame,
            recorded: clock.recorded,
            generation: clock.generation,
            seek: shown.map_or(0, |c| c.seek),
        });
        self.prune(branched);
    }

    /// The bar's `call`, shown until the runtime answers: the call to post
    /// (a seek clamped to the window), or `None` when it cannot be made (no
    /// clock, or a seek with nothing recorded).
    pub fn call(&mut self, call: TransportCall) -> Option<TransportCall> {
        let clock = self.clock.as_mut().filter(|c| c.clock)?;
        let call = match call {
            TransportCall::Pause | TransportCall::Step => {
                clock.paused = true;
                call
            }
            TransportCall::Resume => {
                clock.paused = false;
                call
            }
            TransportCall::Seek { frame, id } => {
                let [oldest, newest] = clock.recorded?;
                clock.frame = frame.clamp(oldest, newest);
                clock.paused = true;
                clock.seek = id;
                TransportCall::Seek {
                    frame: clock.frame,
                    id,
                }
            }
        };
        self.calls += 1;
        Some(call)
    }

    /// Where the next input-log read starts.
    pub fn next_read(&self) -> u64 {
        self.cursor.unwrap_or(0)
    }

    /// An input-log read's marks. A new generation replaced recorded frames,
    /// so their input marks go and the log is read again from the oldest.
    pub fn read(&mut self, page: LogPage) {
        if self.log_generation != Some(page.generation) {
            let before = self.marks.len();
            self.marks.retain(|m| m.kind.starts_with("reload"));
            if self.marks.len() != before {
                self.marks_rev += 1;
            }
            self.cursor = None;
            self.log_generation = Some(page.generation);
        }
        let from = self.next_read();
        for (frame, marks) in page.steps.into_iter().filter(|(f, _)| *f >= from) {
            for (kind, label) in marks {
                self.push(frame, &kind, label);
            }
        }
        // `newest` is 0 while nothing is recorded.
        if page.newest > 0 {
            self.cursor = Some(page.newest + 1);
        }
        self.prune(false);
    }

    fn push(&mut self, frame: u64, kind: &str, label: String) {
        self.next_mark += 1;
        let mark = Mark {
            id: self.next_mark,
            frame,
            kind: kind.to_string(),
            label,
        };
        // Reads arrive in frame order; a reload mark may land among them.
        let at = self.marks.partition_point(|m| m.frame <= frame);
        self.marks.insert(at, mark);
        if self.marks.len() > MARK_CAP {
            self.marks.remove(0);
        }
        self.marks_rev += 1;
    }

    /// Drops marks older than the recorded window, and after a branch those
    /// past its end: their frames were replaced. (A read can be newer than
    /// the last poll, so otherwise marks past the end stay.)
    fn prune(&mut self, branched: bool) {
        let Some([oldest, newest]) = self.clock.and_then(|c| c.recorded) else {
            return;
        };
        let before = self.marks.len();
        self.marks
            .retain(|m| m.frame >= oldest && (!branched || m.frame <= newest));
        if self.marks.len() != before {
            self.marks_rev += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(
        paused: bool,
        recorded: [u64; 2],
        generation: u64,
        revision: u64,
    ) -> Option<PolledClock> {
        Some(PolledClock {
            paused,
            recorded: Some(recorded),
            generation,
            program_revision: revision,
        })
    }

    fn page(generation: u64, oldest: u64, newest: u64, steps: &[(u64, &str)]) -> LogPage {
        LogPage {
            generation,
            oldest,
            newest,
            steps: steps
                .iter()
                .map(|(f, label)| (*f, vec![("key-down".to_string(), label.to_string())]))
                .collect(),
        }
    }

    fn labels(t: &Timeline) -> Vec<(u64, &str)> {
        t.marks
            .iter()
            .map(|m| (m.frame, m.label.as_str()))
            .collect()
    }

    fn seek(frame: u64, id: u64) -> TransportCall {
        TransportCall::Seek { frame, id }
    }

    #[test]
    fn a_poll_shows_the_runtimes_clock() {
        let mut t = Timeline::default();
        t.polled(0, 30, clock(false, [1, 30], 0, 0));
        let shown = t.clock.unwrap();
        assert!(shown.clock && !shown.paused);
        assert_eq!((shown.frame, shown.recorded), (30, Some([1, 30])));
        // A runtime without a clock still shows its frame, with nothing to drive.
        t.polled(0, 31, None);
        assert_eq!(
            t.clock,
            Some(Transport {
                frame: 31,
                ..Transport::default()
            })
        );
        assert_eq!(t.call(TransportCall::Pause), None);
        assert_eq!(t.calls, 0);
    }

    #[test]
    fn calls_show_at_once_and_stale_polls_do_not_undo_them() {
        let mut t = Timeline::default();
        t.polled(0, 100, clock(false, [1, 100], 0, 0));
        assert_eq!(t.call(seek(40, 7)), Some(seek(40, 7)));
        let shown = t.clock.unwrap();
        assert_eq!((shown.frame, shown.paused, shown.seek), (40, true, 7));
        // A poll sent before the seek still reports the old frame, running.
        t.polled(0, 103, clock(false, [1, 103], 0, 0));
        let shown = t.clock.unwrap();
        assert_eq!((shown.frame, shown.paused), (40, true));
        // ...but its window is news.
        assert_eq!(shown.recorded, Some([1, 103]));
        // A poll sent after it is the runtime's word.
        t.polled(1, 41, clock(true, [1, 103], 0, 0));
        assert_eq!((t.clock.unwrap().frame, t.clock.unwrap().seek), (41, 7));
        // Resume shows as running; step and pause as paused.
        assert!(t.call(TransportCall::Resume).is_some());
        assert!(!t.clock.unwrap().paused);
        assert!(t.call(TransportCall::Step).is_some());
        assert!(t.clock.unwrap().paused);
        assert_eq!(t.calls, 3);
    }

    #[test]
    fn a_seek_is_clamped_to_the_window_and_refused_without_one() {
        let mut t = Timeline::default();
        t.polled(0, 0, Some(PolledClock::default()));
        assert_eq!(t.call(seek(5, 1)), None, "nothing recorded");
        t.polled(0, 50, clock(true, [10, 50], 0, 0));
        assert_eq!(t.call(seek(2, 2)), Some(seek(10, 2)));
        assert_eq!(t.clock.unwrap().frame, 10);
        assert_eq!(t.call(seek(90, 3)), Some(seek(50, 3)));
        assert_eq!(t.clock.unwrap().frame, 50);
    }

    #[test]
    fn seeks_waiting_to_be_posted_collapse_to_the_latest() {
        let mut queue = VecDeque::new();
        let s = |frame| SessionCall::Transport(seek(frame, frame));
        let key = SessionCall::Input(GameInput::Key {
            key: "w".into(),
            down: true,
        });
        for call in [
            s(1),
            s(2),
            key.clone(),
            s(3),
            SessionCall::Poll { after: 3 },
            s(4),
            s(5),
        ] {
            enqueue(&mut queue, call);
        }
        // Never across another call: the key lands on the frame sought before it.
        assert_eq!(
            Vec::from(queue),
            [s(2), key, s(3), SessionCall::Poll { after: 3 }, s(5)]
        );
    }

    #[test]
    fn queued_mouse_moves_collapse_to_the_latest() {
        let moved = |x| SessionCall::Input(GameInput::MouseMove { x, y: 0 });
        let key = |down| {
            SessionCall::Input(GameInput::Key {
                key: "w".to_string(),
                down,
            })
        };
        let mut queue = VecDeque::new();
        for call in [
            moved(1),
            moved(2),
            key(true),
            moved(3),
            moved(4),
            key(false),
        ] {
            enqueue(&mut queue, call);
        }
        // A move before a key stays, so the key lands where the pointer was.
        assert_eq!(
            Vec::from(queue),
            [moved(2), key(true), moved(4), key(false)]
        );
    }

    #[test]
    fn input_marks_are_read_once_and_pruned_to_the_window() {
        let mut t = Timeline::default();
        t.polled(0, 10, clock(false, [1, 10], 0, 0));
        assert_eq!(t.next_read(), 0);
        t.read(page(0, 1, 10, &[(3, "W down"), (8, "W up")]));
        assert_eq!(labels(&t), [(3, "W down"), (8, "W up")]);
        assert_eq!(t.next_read(), 11);
        // A later read overlapping what was read adds only the new steps.
        t.read(page(0, 1, 12, &[(8, "W up"), (12, "A down")]));
        assert_eq!(labels(&t), [(3, "W down"), (8, "W up"), (12, "A down")]);
        // The window rolls forward: older marks go.
        t.polled(0, 20, clock(false, [5, 20], 0, 0));
        assert_eq!(labels(&t), [(8, "W up"), (12, "A down")]);
        let ids: Vec<u64> = t.marks.iter().map(|m| m.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    }

    #[test]
    fn a_branch_rereads_the_input_marks_and_keeps_reloads() {
        let mut t = Timeline::default();
        t.polled(0, 10, clock(false, [1, 10], 0, 4));
        t.read(page(0, 1, 10, &[(3, "W down"), (8, "W up")]));
        // A hot reload: a mark where the scene was, after the baseline revision.
        t.polled(0, 10, clock(true, [1, 10], 0, 5));
        assert_eq!(labels(&t), [(3, "W down"), (8, "W up"), (10, "hot reload")]);
        assert_eq!(t.marks.last().unwrap().kind, "reload-ok");
        // Seek to 5 and resume: a new generation replaced frames 6 on.
        t.polled(0, 9, clock(false, [1, 9], 1, 5));
        assert_eq!(labels(&t), [(3, "W down"), (8, "W up")]);
        t.read(page(1, 1, 9, &[(3, "W down"), (7, "S down")]));
        assert_eq!(labels(&t), [(3, "W down"), (7, "S down")]);
    }

    #[test]
    fn a_rev_bump_says_the_marks_changed() {
        let mut t = Timeline::default();
        t.polled(0, 10, clock(false, [1, 10], 0, 0));
        let rev = t.marks_rev;
        t.read(page(0, 1, 10, &[]));
        assert_eq!(t.marks_rev, rev, "nothing new");
        t.read(page(0, 1, 11, &[(11, "W down")]));
        assert_ne!(t.marks_rev, rev);
    }
}
