//! The request lifecycle (design-systems §2.4): the broker's pending table —
//! each in-flight op's continuation, by [`OpId`] — and the mailbox that parks
//! outside arrivals while the scene is scrubbed.
//!
//! An op's continuation (an HTTP tagger, a completion message) is committed
//! state: a seek leaves it, a branch keeps it if the op was issued on the
//! surviving prefix, a reload rebinds it. An arrival is delivered only if its
//! op is still pending on the current branch; the delivery is logged, so a
//! replay re-delivers it and a branch that cuts it re-parks it (the outside
//! world will not resend).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use functor_lang::Value;

use super::{Frame, OpId};
use crate::asset::AssetProgress;
use crate::functor_lang_prelude::NetEventKind;
use crate::net::HttpResult;

/// A transport's token space (each mints its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Http,
    Audio,
    Preload,
}

/// An outside arrival, as a shell pushes it — and as a frame log's
/// `Entry::Delivery` replays it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arrival {
    Http(HttpResult),
    AudioFinished(u64),
    PreloadSettled(u64),
    /// A `Sub.connect` / `Sub.listen` event: unsolicited, no pending op.
    Net {
        key: String,
        kind: NetEventKind,
        conn: i32,
        text: String,
    },
    /// A changed `Sub.assets` snapshot. Logged, never mailed: the frame
    /// reads the shell's latest snapshot.
    Assets(AssetProgress),
}

impl Arrival {
    /// An HTTP response, as the shells' `net_push_http_response` hands it.
    pub fn http_response(token: i32, status: i32, body: String) -> Arrival {
        Arrival::Http(HttpResult {
            token: token as u64,
            status: status as u16,
            body: body.into_bytes(),
            error: None,
        })
    }

    /// A transport failure for an HTTP request.
    pub fn http_error(token: i32, message: String) -> Arrival {
        Arrival::Http(HttpResult {
            token: token as u64,
            status: 0,
            body: Vec::new(),
            error: Some(message),
        })
    }

    pub fn net(key: String, kind: NetEventKind, conn: i32, text: String) -> Arrival {
        Arrival::Net { key, kind, conn, text }
    }

    pub fn is_disconnect(&self) -> bool {
        matches!(self, Arrival::Net { kind: NetEventKind::Disconnected, .. })
    }

    /// The op it answers, by transport token; `None` when unsolicited.
    pub fn token(&self) -> Option<(Channel, u64)> {
        match self {
            Arrival::Http(result) => Some((Channel::Http, result.token)),
            Arrival::AudioFinished(token) => Some((Channel::Audio, *token)),
            Arrival::PreloadSettled(token) => Some((Channel::Preload, *token)),
            Arrival::Net { .. } | Arrival::Assets(_) => None,
        }
    }
}

/// What delivering an arrival found.
pub enum Delivered {
    /// A connection event: no op to look up.
    Unsolicited,
    /// The pending op it answers, and that op's continuation.
    Op(OpId, Value),
    /// Its op is not pending on this branch (discarded, cancelled, or
    /// already answered): drop it.
    Dropped,
}

#[derive(Clone)]
struct Pending {
    channel: Channel,
    /// The transport's correlation id; `None` when never dispatched (replay).
    token: Option<u64>,
    /// The HTTP tagger, or the completion message.
    continuation: Value,
    issued: Frame,
    /// What answered it; `None` while in flight.
    answer: Option<Answer>,
}

/// The arrival that answered an op, kept so a branch or dropped Δ that
/// cuts the answer can park it again — even one delivered on an unlogged
/// zero-delta frame (a pinned clock).
#[derive(Clone)]
struct Answer {
    /// The frame whose Δ delivered it.
    frame: Frame,
    /// Delivery order, so re-parked arrivals keep it.
    seq: u64,
    arrival: Arrival,
}

/// Ops issued on an unrecorded zero-delta frame (the bootstrap, a pinned
/// clock) take this frame and a unique ordinal: their Δ is never logged.
const UNRECORDED: Frame = Frame::MAX;

/// Unanswered audio and preload ops beyond this, per channel, evict the
/// oldest: their completions are best-effort (wasm reports no audio
/// finishes), so they need not drain. Answered ops beyond it evict the oldest
/// too, but only under a pinned clock, which never prunes.
const PENDING_CAP: usize = 4096;

/// Parked arrivals beyond this drop the oldest: a long scrub under heavy
/// traffic must not grow without bound.
const MAILBOX_CAP: usize = 4096;

#[derive(Default)]
pub struct Requests {
    ops: BTreeMap<OpId, Pending>,
    /// Δ's ops by ordinal among its effect ops; ids are stamped on commit.
    provisional: Vec<(u32, Pending)>,
    /// Connection events Δ delivered, by delivery order: a dropped Δ
    /// re-parks them.
    delta: Vec<(u64, Arrival)>,
    mailbox: VecDeque<Arrival>,
    /// The next delivery's order.
    seq: u64,
    unrecorded: u32,
    /// Open connections as `(key, conn)`: a branch commit is refused while
    /// any is (design-systems D4).
    open: BTreeSet<(String, i32)>,
}

impl Requests {
    /// Register `continuation` for the op a drain just logged as Δ's
    /// `ordinal`-th effect op — or, replaying, for the recorded op.
    pub fn issue(
        &mut self,
        recorded: Option<OpId>,
        ordinal: u32,
        channel: Channel,
        token: Option<u64>,
        continuation: Value,
    ) {
        let pending = Pending {
            channel,
            token,
            continuation,
            issued: 0,
            answer: None,
        };
        match recorded {
            Some(op) => {
                self.ops.insert(op, Pending { issued: op.frame, ..pending });
            }
            None => self.provisional.push((ordinal, pending)),
        }
    }

    /// Δ committed as `frame` on branch `epoch`.
    pub(super) fn commit(&mut self, epoch: u64, frame: Frame) {
        for (ordinal, pending) in self.provisional.drain(..) {
            let op = OpId { epoch, frame, ordinal };
            self.ops.insert(op, Pending { issued: frame, ..pending });
        }
        self.delta.clear();
        self.evict_over_cap(false);
    }

    /// Δ discarded unlogged (a zero-delta frame): its ops stay in flight
    /// under unique ids. What it delivered stays answered, re-parkable.
    pub(super) fn commit_unrecorded(&mut self, epoch: u64, frame: Frame) {
        for (_, pending) in std::mem::take(&mut self.provisional) {
            let op = OpId { epoch, frame: UNRECORDED, ordinal: self.unrecorded };
            self.unrecorded = self.unrecorded.wrapping_add(1);
            self.ops.insert(op, Pending { issued: frame, ..pending });
        }
        self.delta.truncate(MAILBOX_CAP);
        self.evict_over_cap(true);
    }

    /// `answered`: also bound answered ops — only on unrecorded frames,
    /// where no window prunes them (on recorded ones a branch may need them).
    fn evict_over_cap(&mut self, answered: bool) {
        if self.ops.len() <= PENDING_CAP {
            return;
        }
        let mut evict = |keep: &dyn Fn(&Pending) -> bool, order: &dyn Fn(&Pending) -> u64| {
            let mut over: Vec<(u64, OpId)> = self
                .ops
                .iter()
                .filter(|(_, p)| keep(p))
                .map(|(op, p)| (order(p), *op))
                .collect();
            over.sort();
            for (_, op) in &over[..over.len().saturating_sub(PENDING_CAP)] {
                self.ops.remove(op);
            }
        };
        for channel in [Channel::Audio, Channel::Preload] {
            evict(&|p| p.channel == channel && p.answer.is_none(), &|p| p.issued);
        }
        if answered {
            evict(&|p| p.answer.is_some(), &|p| p.answer.as_ref().map_or(0, |a| a.seq));
        }
    }

    /// Δ (built as `frame`) dropped by a seek or rewind: its ops go (late
    /// arrivals for them are dropped), and its deliveries are parked again.
    pub(super) fn drop_delta(&mut self, frame: Frame) {
        self.provisional.clear();
        // Ops Δ issued on an unrecorded (zero-delta) frame go with it too.
        self.ops.retain(|op, p| {
            !(op.frame == UNRECORDED && p.issued == frame && p.answer.is_none())
        });
        let mut again = self.take_answers(|answer| answer.frame == frame);
        again.append(&mut self.delta);
        again.sort_by_key(|(seq, _)| *seq);
        self.repark(again);
    }

    /// A branch keeping frames before `frame`: ops issued from it are
    /// discarded; ops answered from it are in flight again and their answers
    /// re-parked. `cut_disconnects`, when frames are cut, are the logged
    /// disconnects they held: re-parked too, since the restored model may
    /// still hold those connections. Other parked connection events are then
    /// dropped: a cutting branch is refused while a connection is open, so
    /// they belong to closed ones.
    pub(super) fn truncate_from(
        &mut self,
        frame: Frame,
        cut_disconnects: Option<Vec<Arrival>>,
    ) {
        self.ops.retain(|_, p| p.issued < frame);
        let mut again = self.take_answers(|answer| answer.frame >= frame);
        if let Some(disconnects) = cut_disconnects {
            self.mailbox
                .retain(|arrival| arrival.token().is_some() || arrival.is_disconnect());
            // Disconnects belong after the answers they followed; the order
            // among them is the log's.
            again.extend(disconnects.into_iter().map(|arrival| (u64::MAX, arrival)));
        }
        self.repark(again);
    }

    /// Un-answer the ops whose answer matches; their arrivals, in order.
    fn take_answers(&mut self, cut: impl Fn(&Answer) -> bool) -> Vec<(u64, Arrival)> {
        let mut again: Vec<(u64, Arrival)> = self
            .ops
            .values_mut()
            .filter(|p| p.answer.as_ref().is_some_and(&cut))
            .filter_map(|p| p.answer.take())
            .map(|answer| (answer.seq, answer.arrival))
            .collect();
        again.sort_by_key(|(seq, _)| *seq);
        again
    }

    /// Forget ops answered before `floor`: no seekable frame reaches them.
    pub(super) fn prune(&mut self, floor: Frame) {
        self.ops
            .retain(|_, p| p.answer.as_ref().is_none_or(|a| a.frame >= floor));
    }

    /// Rebind every continuation to the new program (like the model), the
    /// answered ones too: a branch may re-park them. An op whose continuation
    /// cannot follow is cancelled; its arrival will drop.
    pub(super) fn rebind(
        &mut self,
        old: &functor_lang::ir::Module,
        new: &functor_lang::ir::Module,
    ) -> Vec<String> {
        // One pass over every continuation: indexing the modules dominates.
        let all: Vec<Value> = self
            .ops
            .values()
            .chain(self.provisional.iter().map(|(_, p)| p))
            .map(|p| p.continuation.clone())
            .collect();
        let count = all.len();
        let (rebound, report) =
            functor_lang::rebind_value(&Value::List(std::rc::Rc::new(all)), old, new);
        if let (true, Value::List(values)) = (report.warnings.is_empty(), &rebound) {
            if values.len() == count {
                let pendings = self
                    .ops
                    .values_mut()
                    .chain(self.provisional.iter_mut().map(|(_, p)| p));
                for (pending, value) in pendings.zip(values.iter()) {
                    pending.continuation = value.clone();
                }
                return Vec::new();
            }
        }
        // Something cannot follow: rebind one by one to find what.
        let mut cancelled = Vec::new();
        let mut rebind = |pending: &mut Pending| {
            let (value, report) = functor_lang::rebind_value(&pending.continuation, old, new);
            pending.continuation = value;
            if !report.warnings.is_empty() {
                cancelled.push(format!(
                    "reload cancelled a {:?} request: {}",
                    pending.channel,
                    report.warnings.join("; ")
                ));
            }
            report.warnings.is_empty()
        };
        self.ops.retain(|_, p| rebind(p));
        self.provisional.retain_mut(|(_, p)| rebind(p));
        cancelled
    }

    /// Deliver `arrival` in Δ (built as `frame` on branch `epoch`).
    pub(crate) fn deliver(&mut self, arrival: &Arrival, epoch: u64, frame: Frame) -> Delivered {
        let seq = self.seq;
        self.seq += 1;
        let Some((channel, token)) = arrival.token() else {
            self.delta.push((seq, arrival.clone()));
            return Delivered::Unsolicited;
        };
        let answers = |p: &Pending| {
            p.answer.is_none() && p.channel == channel && p.token == Some(token)
        };
        let found = match self.ops.iter_mut().find(|(_, p)| answers(p)) {
            Some((op, pending)) => Some((*op, pending)),
            None => self
                .provisional
                .iter_mut()
                .find(|(_, p)| answers(p))
                .map(|(ordinal, pending)| (OpId { epoch, frame, ordinal: *ordinal }, pending)),
        };
        let Some((op, pending)) = found else {
            return Delivered::Dropped;
        };
        pending.answer = Some(Answer { frame, seq, arrival: arrival.clone() });
        Delivered::Op(op, pending.continuation.clone())
    }

    /// A replay's re-delivery of recorded op `op`: its continuation, if it
    /// is pending on the fork.
    pub(crate) fn deliver_recorded(&mut self, op: OpId, arrival: &Arrival) -> Option<Value> {
        let pending = self.ops.get_mut(&op).filter(|p| p.answer.is_none())?;
        // A fork never re-parks: the frame and order are unused.
        pending.answer = Some(Answer { frame: op.frame, seq: 0, arrival: arrival.clone() });
        Some(pending.continuation.clone())
    }

    /// The ops pending at the end of `frame`, for a fork replaying from it.
    pub(super) fn fork(&self, frame: Frame) -> Requests {
        let ops = self
            .ops
            .iter()
            .filter(|(_, p)| {
                p.issued <= frame && p.answer.as_ref().is_none_or(|a| a.frame > frame)
            })
            .map(|(op, p)| (*op, Pending { token: None, answer: None, ..p.clone() }))
            .collect();
        Requests { ops, ..Requests::default() }
    }

    /// Park an arrival. Over the cap the oldest connection event goes first;
    /// a dropped answer cancels its op, whose response then never comes (a
    /// typed `Cancelled` for its tagger is future work).
    pub(super) fn park(&mut self, arrival: Arrival) {
        self.mailbox.push_back(arrival);
        self.trim_mailbox();
    }

    fn trim_mailbox(&mut self) {
        if self.mailbox.len() > MAILBOX_CAP {
            log::warn!("[functor-lang] parked arrivals over {MAILBOX_CAP}: dropping the oldest");
        }
        while self.mailbox.len() > MAILBOX_CAP {
            let at = self.mailbox.iter().position(|a| a.token().is_none()).unwrap_or(0);
            if let Some((channel, token)) = self.mailbox.remove(at).and_then(|a| a.token()) {
                let answers = |p: &Pending| {
                    p.answer.is_none() && p.channel == channel && p.token == Some(token)
                };
                self.ops.retain(|_, p| !answers(p));
                self.provisional.retain(|(_, p)| !answers(p));
            }
        }
    }

    pub(super) fn take_mailbox(&mut self) -> Vec<Arrival> {
        self.mailbox.drain(..).collect()
    }

    fn repark(&mut self, arrivals: Vec<(u64, Arrival)>) {
        for (_, arrival) in arrivals.into_iter().rev() {
            self.mailbox.push_front(arrival);
        }
        self.trim_mailbox();
    }

    /// Track open connections as their events arrive (transport upkeep, not
    /// a game delivery: it runs even while the arrival is parked).
    pub(super) fn note_connection(&mut self, key: &str, kind: NetEventKind, conn: i32) {
        match kind {
            NetEventKind::Connected => {
                self.open.insert((key.to_string(), conn));
            }
            NetEventKind::Disconnected => {
                self.open.remove(&(key.to_string(), conn));
            }
            // An error alone does not close a connection (as for the retry
            // state): refusing a branch is the safe side.
            NetEventKind::Error | NetEventKind::Message => {}
        }
    }

    /// A connection key closed by the game (undeclared, reload, quit).
    pub(super) fn close_key(&mut self, key: &str) {
        self.open.retain(|(open, _)| open != key);
    }

    pub(super) fn open_connections(&self) -> usize {
        self.open.len()
    }

    /// Tokens of ops pending on this branch's tail (the fuzzer's hung-op check).
    pub(crate) fn pending_tokens(&self) -> Vec<(Channel, u64)> {
        let ops = self.ops.values();
        let provisional = self.provisional.iter().map(|(_, p)| p);
        ops.chain(provisional)
            .filter(|p| p.answer.is_none())
            .filter_map(|p| Some((p.channel, p.token?)))
            .collect()
    }

    /// Tokens of parked arrivals.
    pub(crate) fn parked_tokens(&self) -> Vec<(Channel, u64)> {
        self.mailbox.iter().filter_map(Arrival::token).collect()
    }

    /// Whether a branch at `frame` would re-park or deliver anything: an op
    /// answered after it, a connection event in Δ, or a parked arrival.
    pub(crate) fn arrivals_after(&self, frame: Frame) -> bool {
        !self.delta.is_empty()
            || !self.mailbox.is_empty()
            || self.ops.values().any(|p| p.answer.as_ref().is_some_and(|a| a.frame > frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(token: u64) -> Arrival {
        Arrival::Http(HttpResult { token, status: 200, body: Vec::new(), error: None })
    }

    fn message() -> Arrival {
        Arrival::Net { key: "ws".into(), kind: NetEventKind::Message, conn: 1, text: String::new() }
    }

    fn tagger(n: f64) -> Value {
        Value::Number(n)
    }

    fn delivered(requests: &mut Requests, token: u64, frame: Frame) -> Option<OpId> {
        match requests.deliver(&http(token), 0, frame) {
            Delivered::Op(op, _) => Some(op),
            _ => None,
        }
    }

    #[test]
    fn a_provisional_op_is_stamped_on_commit_and_answered_once() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(7), tagger(1.0));
        // Answered in its own Δ: the id is the one the commit will stamp.
        let op = delivered(&mut requests, 7, 0).expect("pending");
        assert_eq!(op, OpId { epoch: 0, frame: 0, ordinal: 0 });
        requests.commit(0, 0);
        assert!(delivered(&mut requests, 7, 1).is_none(), "a second answer drops");
        assert!(requests.pending_tokens().is_empty());
    }

    #[test]
    fn a_branch_drops_the_cut_ops_and_reparks_the_cut_answers() {
        let mut requests = Requests::default();
        // Frame 0 issues token 1; frame 2 issues token 2; frame 3 answers both.
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.commit(0, 0);
        requests.commit(0, 1);
        requests.issue(None, 0, Channel::Http, Some(2), tagger(2.0));
        requests.commit(0, 2);
        delivered(&mut requests, 1, 3).unwrap();
        delivered(&mut requests, 2, 3).unwrap();
        requests.commit(0, 3);
        // A connection event parked while scrubbed belongs to the cut
        // timeline's connection.
        requests.park(message());

        // Branch at frame 1: token 2 was issued in the cut future.
        requests.truncate_from(2, Some(Vec::new()));
        assert_eq!(requests.pending_tokens(), vec![(Channel::Http, 1)]);
        assert_eq!(requests.take_mailbox(), vec![http(1)]);
        assert!(delivered(&mut requests, 1, 2).is_some(), "the re-parked answer lands");
        assert!(delivered(&mut requests, 2, 2).is_none(), "the cut op's answer drops");
    }

    #[test]
    fn a_dropped_delta_reparks_its_deliveries_in_order_and_discards_its_ops() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.commit(0, 0);
        // Paused at the tail: Δ (frame 1) issues token 2, then receives a
        // message and token 1.
        requests.issue(None, 0, Channel::Http, Some(2), tagger(2.0));
        assert!(matches!(requests.deliver(&message(), 0, 1), Delivered::Unsolicited));
        assert!(delivered(&mut requests, 1, 1).is_some());
        requests.drop_delta(1);
        assert_eq!(requests.pending_tokens(), vec![(Channel::Http, 1)]);
        assert_eq!(requests.take_mailbox(), vec![message(), http(1)]);
    }

    #[test]
    fn an_answer_on_an_unrecorded_frame_reparks_on_a_branch() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.commit(0, 0);
        // A pinned clock: zero-delta frames deliver, and their Δ is unlogged.
        delivered(&mut requests, 1, 1).unwrap();
        requests.commit_unrecorded(0, 1);
        requests.truncate_from(1, Some(Vec::new()));
        assert_eq!(requests.take_mailbox(), vec![http(1)]);
    }

    #[test]
    fn a_fork_holds_the_ops_pending_at_its_frame() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.commit(0, 0);
        let op = delivered(&mut requests, 1, 2).unwrap();
        let mut fork = requests.fork(1);
        assert!(fork.deliver_recorded(op, &http(1)).is_some());
        assert!(fork.deliver_recorded(op, &http(1)).is_none(), "re-delivered once");
        assert!(requests.fork(2).deliver_recorded(op, &http(1)).is_none(), "answered by frame 2");
    }

    #[test]
    fn prune_forgets_only_answered_ops() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.issue(None, 1, Channel::Audio, Some(1), tagger(2.0));
        requests.commit(0, 0);
        delivered(&mut requests, 1, 1).unwrap();
        requests.prune(5);
        assert_eq!(requests.pending_tokens(), vec![(Channel::Audio, 1)]);
    }

    #[test]
    fn resuming_without_a_cut_keeps_parked_connection_events() {
        let mut requests = Requests::default();
        requests.commit(0, 0);
        requests.park(message());
        requests.truncate_from(1, None);
        assert_eq!(requests.take_mailbox(), vec![message()]);
    }

    #[test]
    fn a_cutting_branch_reparks_the_cut_disconnects_and_keeps_parked_ones() {
        let mut requests = Requests::default();
        let disconnect = |conn| Arrival::Net {
            key: "ws".into(),
            kind: NetEventKind::Disconnected,
            conn,
            text: String::new(),
        };
        requests.park(message());
        requests.park(disconnect(2));
        requests.truncate_from(1, Some(vec![disconnect(1)]));
        assert_eq!(requests.take_mailbox(), vec![disconnect(1), disconnect(2)]);
    }

    #[test]
    fn mailbox_overflow_drops_connection_events_before_answers() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), tagger(1.0));
        requests.park(http(1));
        for _ in 1..MAILBOX_CAP {
            requests.park(message());
        }
        requests.park(http(2));
        assert_eq!(requests.parked_tokens(), vec![(Channel::Http, 1), (Channel::Http, 2)]);
    }

    #[test]
    fn unanswered_completions_past_the_cap_evict_the_oldest() {
        let mut requests = Requests::default();
        requests.issue(None, 1, Channel::Http, Some(1), tagger(0.0));
        for token in 1..=(PENDING_CAP as u64 + 10) {
            requests.issue(None, 0, Channel::Audio, Some(token), tagger(0.0));
            requests.commit(0, token);
        }
        let tokens = requests.pending_tokens();
        assert_eq!(tokens.len(), PENDING_CAP + 1, "HTTP is not capped");
        assert!(tokens.contains(&(Channel::Http, 1)));
        assert!(!tokens.contains(&(Channel::Audio, 10)), "the oldest went first");
    }

    #[test]
    fn unrecorded_ops_stay_in_flight_under_unique_ids() {
        let mut requests = Requests::default();
        for token in [1, 2] {
            requests.issue(None, 0, Channel::Preload, Some(token), tagger(0.0));
            requests.commit_unrecorded(0, 0);
        }
        let ids: Vec<OpId> = [1, 2]
            .map(|token| match requests.deliver(&Arrival::PreloadSettled(token), 0, 0) {
                Delivered::Op(op, _) => op,
                _ => panic!("pending"),
            })
            .to_vec();
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|op| op.frame == UNRECORDED));
    }
}
