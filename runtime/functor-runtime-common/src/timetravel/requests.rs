//! The pending table (design-systems §2.4): each in-flight op's continuation
//! — an HTTP tagger, a `playThen`/`preloadThen` message — by [`OpId`], owned
//! by the recorder of one producer.
//!
//! It is committed state: a seek leaves it, a branch keeps the ops issued on
//! the surviving prefix, a reload rebinds their continuations. An arrival is
//! delivered only if its op is still pending.

use std::collections::BTreeMap;

use functor_lang::Value;

use super::{Frame, OpId};
use crate::net::HttpResult;

/// A transport's token space (each mints its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Http,
    Audio,
    Preload,
}

/// An answer from outside, as a shell pushes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arrival {
    Http(HttpResult),
    AudioFinished(u64),
    PreloadSettled(u64),
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

    /// The op it answers, by transport token.
    pub fn token(&self) -> (Channel, u64) {
        match self {
            Arrival::Http(result) => (Channel::Http, result.token),
            Arrival::AudioFinished(token) => (Channel::Audio, *token),
            Arrival::PreloadSettled(token) => (Channel::Preload, *token),
        }
    }
}

/// What delivering an arrival found.
pub enum Delivered {
    /// The pending op it answers, and that op's continuation.
    Op(OpId, Value),
    /// Its op is not pending (discarded, cancelled, or already answered):
    /// drop it.
    Dropped,
}

struct Pending {
    channel: Channel,
    /// The transport's correlation id; `None` when never dispatched (replay).
    token: Option<u64>,
    /// The HTTP tagger, or the completion message.
    continuation: Value,
    issued: Frame,
}

/// Ops issued on an unrecorded zero-delta frame (the bootstrap, a pinned
/// clock) take this frame and a unique ordinal: their Δ is never logged.
const UNRECORDED: Frame = Frame::MAX;

/// Pending audio and preload ops beyond this, per channel, evict the oldest:
/// their completions are best-effort (wasm reports no audio finishes), so
/// they need not drain.
const PENDING_CAP: usize = 4096;

#[derive(Default)]
pub struct Requests {
    ops: BTreeMap<OpId, Pending>,
    /// Δ's ops by ordinal among its effect ops; ids are stamped on commit.
    provisional: Vec<(u32, Pending)>,
    unrecorded: u32,
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
        let pending = Pending { channel, token, continuation, issued: 0 };
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
        self.evict_over_cap();
    }

    /// Δ discarded unlogged (a zero-delta frame): its ops stay in flight
    /// under unique ids.
    pub(super) fn commit_unrecorded(&mut self, epoch: u64, frame: Frame) {
        for (_, pending) in std::mem::take(&mut self.provisional) {
            let op = OpId { epoch, frame: UNRECORDED, ordinal: self.unrecorded };
            self.unrecorded = self.unrecorded.wrapping_add(1);
            self.ops.insert(op, Pending { issued: frame, ..pending });
        }
        self.evict_over_cap();
    }

    fn evict_over_cap(&mut self) {
        if self.ops.len() <= PENDING_CAP {
            return;
        }
        for channel in [Channel::Audio, Channel::Preload] {
            let mut over: Vec<(Frame, OpId)> = self
                .ops
                .iter()
                .filter(|(_, p)| p.channel == channel)
                .map(|(op, p)| (p.issued, *op))
                .collect();
            over.sort();
            for (_, op) in &over[..over.len().saturating_sub(PENDING_CAP)] {
                self.ops.remove(op);
            }
        }
    }

    /// Δ dropped by a seek or rewind: its ops go, and late arrivals for them
    /// are dropped.
    pub(super) fn drop_delta(&mut self) {
        self.provisional.clear();
    }

    /// A branch keeping frames before `frame`: ops issued from it are
    /// discarded.
    pub(super) fn truncate_from(&mut self, frame: Frame) {
        self.ops.retain(|_, p| p.issued < frame);
    }

    /// Rebind every continuation to the new program (like the model). An op
    /// whose continuation cannot follow is cancelled; its arrival will drop.
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

    /// Deliver `arrival` in Δ (built as `frame` on branch `epoch`): its op
    /// leaves the table.
    pub(crate) fn deliver(&mut self, arrival: &Arrival, epoch: u64, frame: Frame) -> Delivered {
        let (channel, token) = arrival.token();
        let answers = |p: &Pending| p.channel == channel && p.token == Some(token);
        if let Some(op) = self.ops.iter().find(|(_, p)| answers(p)).map(|(op, _)| *op) {
            let pending = self.ops.remove(&op).expect("just found");
            return Delivered::Op(op, pending.continuation);
        }
        match self.provisional.iter().position(|(_, p)| answers(p)) {
            Some(at) => {
                let (ordinal, pending) = self.provisional.remove(at);
                Delivered::Op(OpId { epoch, frame, ordinal }, pending.continuation)
            }
            None => Delivered::Dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(token: u64) -> Arrival {
        Arrival::Http(HttpResult { token, status: 200, body: Vec::new(), error: None })
    }

    fn delivered(requests: &mut Requests, token: u64, frame: Frame) -> Option<OpId> {
        match requests.deliver(&http(token), 0, frame) {
            Delivered::Op(op, _) => Some(op),
            Delivered::Dropped => None,
        }
    }

    #[test]
    fn a_provisional_op_is_stamped_on_commit_and_answered_once() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(7), Value::Number(1.0));
        requests.commit(0, 3);
        assert_eq!(
            delivered(&mut requests, 7, 4),
            Some(OpId { epoch: 0, frame: 3, ordinal: 0 })
        );
        assert!(delivered(&mut requests, 7, 4).is_none(), "a second answer drops");
    }

    #[test]
    fn a_branch_drops_the_cut_ops_and_a_seek_drops_delta() {
        let mut requests = Requests::default();
        requests.issue(None, 0, Channel::Http, Some(1), Value::Number(1.0));
        requests.commit(0, 0);
        requests.issue(None, 0, Channel::Http, Some(2), Value::Number(2.0));
        requests.commit(0, 2);
        requests.issue(None, 0, Channel::Http, Some(3), Value::Number(3.0));
        requests.drop_delta();
        requests.truncate_from(1);
        assert!(delivered(&mut requests, 1, 1).is_some(), "kept by the branch");
        assert!(delivered(&mut requests, 2, 1).is_none(), "issued in the cut future");
        assert!(delivered(&mut requests, 3, 1).is_none(), "dropped with Δ");
    }

    #[test]
    fn pending_completions_past_the_cap_evict_the_oldest() {
        let mut requests = Requests::default();
        requests.issue(None, 1, Channel::Http, Some(1), Value::Number(0.0));
        for token in 1..=(PENDING_CAP as u64 + 10) {
            requests.issue(None, 0, Channel::Audio, Some(token), Value::Number(0.0));
            requests.commit(0, token);
        }
        assert_eq!(requests.ops.len(), PENDING_CAP + 1, "HTTP is not capped");
        let pending = |token| requests.ops.values().any(|p| p.token == Some(token));
        assert!(!pending(10), "the oldest went first");
    }

    #[test]
    fn unrecorded_ops_stay_in_flight_under_unique_ids() {
        let mut requests = Requests::default();
        for token in [1, 2] {
            requests.issue(None, 0, Channel::Preload, Some(token), Value::Number(0.0));
            requests.commit_unrecorded(0, 0);
        }
        let ids: Vec<OpId> = [1, 2]
            .map(|token| match requests.deliver(&Arrival::PreloadSettled(token), 0, 0) {
                Delivered::Op(op, _) => op,
                Delivered::Dropped => panic!("pending"),
            })
            .to_vec();
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|op| op.frame == UNRECORDED));
    }
}
