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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use functor_lang::Value;

use super::{Frame, OpId};
use crate::asset::AssetProgress;
use crate::jobs::{JobKind, JobPort, JobSpec, ReadSlot, RunSpec, Shareable, Ticket};
use crate::functor_lang_prelude::NetEventKind;
use crate::net::HttpResult;

/// A transport's token space (each mints its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Http,
    Audio,
    Preload,
    Job,
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
    /// A job's result: what its continuation receives.
    Job { ticket: Ticket, value: Shareable },
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
            Arrival::Job { ticket, .. } => Some((Channel::Job, ticket.id)),
            Arrival::Net { .. } | Arrival::Assets(_) => None,
        }
    }

    /// Whether it may answer `pending`: the op on its channel and token,
    /// and for a job, the op's live attempt.
    fn answers(&self, pending: &Pending) -> bool {
        let Some((channel, token)) = self.token() else {
            return false;
        };
        let live_attempt = match (&pending.job, self) {
            (None, _) => true,
            (Some(job), Arrival::Job { ticket, .. }) => job.live() && job.attempt == ticket.attempt,
            (Some(_), _) => false,
        };
        pending.answer.is_none()
            && pending.channel == channel
            && pending.token == Some(token)
            && live_attempt
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
    /// A job op's request and run state.
    job: Option<JobOp>,
}

/// What a job op needs beyond a request: its spec (to dispatch it again),
/// which attempt is live, and whether the game cancelled it.
#[derive(Clone)]
struct JobOp {
    spec: JobSpec,
    attempt: u32,
    stop: Stop,
    cancelled: Cancelled,
    /// What the live `Job.run` attempt read, once it finished.
    reads: Option<ReadSlot>,
    /// A reload kept the attempt: its result is delivered only if what it
    /// read still reads the same.
    recheck: bool,
}

impl JobOp {
    fn live(&self) -> bool {
        self.cancelled == Cancelled::No
    }
}

enum InFlight {
    Provisional(usize),
    Committed(OpId),
}

/// When the game cancelled a job: in Δ (a seek undoes it), or on a frame
/// (a branch from before it restores the op).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cancelled {
    No,
    /// `was`: the later cancel it replaced (Δ cancelled, while scrubbed, an
    /// op the shown future cancels); undoing Δ puts it back.
    InDelta { was: Option<Frame> },
    At(Frame),
}

/// A running attempt's stop flag. Dropping the op stops its job; a clone
/// (a fork's copy) never owns it.
#[derive(Default)]
struct Stop(Option<Arc<AtomicBool>>);

impl Stop {
    /// Whether its attempt was dispatched here and not stopped.
    fn running(&self) -> bool {
        self.0.as_ref().is_some_and(|flag| !flag.load(Ordering::Relaxed))
    }

    fn signal(&self) {
        if let Some(flag) = &self.0 {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl Clone for Stop {
    fn clone(&self) -> Stop {
        Stop(None)
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.signal();
    }
}

/// Dispatch `job` (op `id`) again as its next attempt.
fn redispatch(jobs: &JobPort, id: u64, job: &mut JobOp) {
    job.attempt += 1;
    let (stop, reads) = jobs.start(&job.spec, Ticket { id, attempt: job.attempt });
    job.stop = Stop(Some(stop));
    job.reads = reads;
    job.recheck = false;
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
    /// Where this producer's jobs run and their results land.
    jobs: JobPort,
    /// See [`Self::set_view`].
    view: Option<Frame>,
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
            job: None,
        };
        self.insert(recorded, ordinal, pending);
    }

    fn insert(&mut self, recorded: Option<OpId>, ordinal: u32, pending: Pending) {
        match recorded {
            Some(op) => {
                self.ops.insert(op, Pending { issued: op.frame, ..pending });
            }
            None => self.provisional.push((ordinal, pending)),
        }
    }

    /// Whether a job with `key` is in flight on the working table: the
    /// committed ops and Δ's, in drain order (design J2: a same-key request
    /// is dropped while one is pending).
    pub fn job_in_flight(&self, kind: JobKind, key: &str) -> bool {
        self.find_in_flight(kind, key).is_some()
    }

    /// The job op with `key` in flight for Δ (at most one): Δ's own, else
    /// a committed one.
    fn find_in_flight(&self, kind: JobKind, key: &str) -> Option<InFlight> {
        let keyed = |p: &Pending| p.job.as_ref().is_some_and(|j| j.spec.key() == (kind, key));
        let live = |p: &Pending| keyed(p) && p.job.as_ref().is_some_and(JobOp::live);
        if let Some(at) = self.provisional.iter().position(|(_, p)| live(p)) {
            return Some(InFlight::Provisional(at));
        }
        let (op, _) = self.ops.iter().find(|(_, p)| keyed(p) && self.in_flight_now(p))?;
        Some(InFlight::Committed(*op))
    }

    /// Whether committed job op `p` is in flight for Δ: at the tail, or
    /// while scrubbed at the frame shown (Δ is that branch's first frame).
    fn in_flight_now(&self, p: &Pending) -> bool {
        let Some(job) = &p.job else { return false };
        match self.view {
            None => p.answer.is_none() && job.live(),
            Some(k) => {
                p.issued <= k
                    && p.answer.as_ref().is_none_or(|a| a.frame > k)
                    && match job.cancelled {
                        Cancelled::No => true,
                        Cancelled::InDelta { .. } => false,
                        Cancelled::At(at) => at > k,
                    }
            }
        }
    }

    /// The scrubbed frame (`None` at the tail), from the recorder.
    pub(super) fn set_view(&mut self, view: Option<Frame>) {
        self.view = view;
    }

    /// Register a job op like [`Self::issue`], dispatching it unless
    /// replaying (`dispatch` false: its answer comes from the log).
    pub fn issue_job(
        &mut self,
        recorded: Option<OpId>,
        ordinal: u32,
        spec: JobSpec,
        continuation: Value,
        dispatch: bool,
    ) {
        let (token, stop, reads) = if dispatch {
            let ticket = Ticket { id: crate::jobs::next_id(), attempt: 1 };
            let (stop, reads) = self.jobs.start(&spec, ticket);
            (Some(ticket.id), Stop(Some(stop)), reads)
        } else {
            (None, Stop::default(), None)
        };
        let job = JobOp { spec, attempt: 1, stop, cancelled: Cancelled::No, reads, recheck: false };
        let pending = Pending {
            channel: Channel::Job,
            token,
            continuation,
            issued: 0,
            answer: None,
            job: Some(job),
        };
        self.insert(recorded, ordinal, pending);
    }

    /// Cancel the in-flight job with `key`: no message is delivered for it.
    /// A committed op is kept, so a seek or branch from before the cancel
    /// restores (and re-dispatches) it.
    pub fn cancel_job(&mut self, kind: JobKind, key: &str) {
        match self.find_in_flight(kind, key) {
            Some(InFlight::Provisional(at)) => {
                self.provisional.remove(at);
            }
            Some(InFlight::Committed(op)) => {
                let job = self.ops.get_mut(&op).and_then(|p| p.job.as_mut()).expect("found");
                let was = match job.cancelled {
                    Cancelled::At(at) => Some(at),
                    _ => None,
                };
                // The worker stops when Δ commits: a seek that drops Δ first
                // finds it still running.
                job.cancelled = Cancelled::InDelta { was };
            }
            None => {}
        }
    }

    /// Set each job op's cancel to `undo(cancel)` where that is `Some`. An
    /// op back in flight whose result isn't here (answered, parked or
    /// finished) lost its worker, so [`Self::dispatch_restored`] runs it
    /// again.
    fn restore(&mut self, undo: impl Fn(Cancelled) -> Option<Cancelled>) {
        let mut here = self.jobs.mailbox().tickets();
        here.extend(self.parked_jobs());
        for pending in self.ops.values_mut() {
            let Some(job) = pending.job.as_mut() else { continue };
            let Some(cancelled) = undo(job.cancelled) else { continue };
            job.cancelled = cancelled;
            let arrived = pending
                .token
                .is_some_and(|id| here.contains(&Ticket { id, attempt: job.attempt }));
            if job.live() && pending.answer.is_none() && !arrived && !job.stop.running() {
                job.stop = Stop::default();
            }
        }
    }

    /// Dispatch each restored job op as a new attempt, so a stale result
    /// from an earlier one drops. Called once a seek or branch is done, so
    /// an op it then cuts never runs.
    pub(super) fn dispatch_restored(&mut self) {
        for pending in self.ops.values_mut().filter(|p| p.answer.is_none()) {
            let (Some(id), Some(job)) = (pending.token, pending.job.as_mut()) else { continue };
            // Never dispatched (a replay's op): its answer is the log's.
            if job.live() && job.stop.0.is_none() {
                redispatch(&self.jobs, id, job);
            }
        }
    }

    /// After a reload, for each `Job.run` still running: if the code its
    /// function reaches changed, stop it and run it again on the new program
    /// (a new attempt, so the old result drops); else keep it, re-checking
    /// its reads at delivery. A result that already arrived is delivered
    /// (re-checked too), like one already recorded.
    fn reload_jobs(&mut self, old: &functor_lang::ir::Module, new: &functor_lang::ir::Module) {
        use functor_lang::reuse::CodeIndex;
        let Some(program) = self.jobs.program().map(|p| p.source.clone()) else { return };
        let mut arrived = self.jobs.mailbox().tickets();
        arrived.extend(self.parked_jobs());
        let mut index: Option<(CodeIndex, CodeIndex)> = None;
        let pendings = self.ops.values_mut().chain(self.provisional.iter_mut().map(|(_, p)| p));
        for pending in pendings {
            let Some(job) = pending.job.as_mut() else { continue };
            let JobSpec::Run(run) = &job.spec else { continue };
            let Ok(call) = &run.call else { continue };
            let f = call.f.clone();
            let moved = RunSpec {
                key: run.key.clone(),
                call: Ok(crate::jobs::RunCall { program: program.clone(), ..call.clone() }),
            };
            job.spec = JobSpec::Run(std::sync::Arc::new(moved));
            let ticket = pending.token.map(|id| Ticket { id, attempt: job.attempt });
            let Some(ticket) = ticket.filter(|_| job.live() && pending.answer.is_none()) else {
                continue;
            };
            if !job.stop.running() {
                continue;
            }
            let (old, new) = index.get_or_insert_with(|| (CodeIndex::new(old), CodeIndex::new(new)));
            let same = old.code_key(&f).is_some() && old.code_key(&f) == new.code_key(&f);
            if same || arrived.contains(&ticket) {
                job.recheck = true;
            } else {
                job.stop.signal();
                redispatch(&self.jobs, ticket.id, job);
            }
        }
    }

    /// This producer's job dispatcher, mailbox and reader.
    pub fn jobs(&self) -> &JobPort {
        &self.jobs
    }

    pub fn jobs_mut(&mut self) -> &mut JobPort {
        &mut self.jobs
    }

    /// Δ committed as `frame` on branch `epoch`.
    pub(super) fn commit(&mut self, epoch: u64, frame: Frame) {
        for (ordinal, pending) in self.provisional.drain(..) {
            let op = OpId { epoch, frame, ordinal };
            self.ops.insert(op, Pending { issued: frame, ..pending });
        }
        self.stamp_cancels(frame);
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
        self.stamp_cancels(frame);
        self.delta.truncate(MAILBOX_CAP);
        self.evict_over_cap(true);
    }

    /// Δ's cancels happened on `frame`.
    fn stamp_cancels(&mut self, frame: Frame) {
        for job in self.ops.values_mut().filter_map(|p| p.job.as_mut()) {
            if let Cancelled::InDelta { .. } = job.cancelled {
                job.cancelled = Cancelled::At(frame);
                job.stop.signal();
            }
        }
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
            // Cancelled jobs, which only a prune would free.
            evict(&|p| p.job.as_ref().is_some_and(|j| !j.live()), &|p| p.issued);
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
        // Its cancels are undone, like its answers.
        self.restore(|c| match c {
            Cancelled::InDelta { was } => Some(was.map_or(Cancelled::No, Cancelled::At)),
            Cancelled::At(at) if at == frame => Some(Cancelled::No),
            _ => None,
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
        // A job cancelled in the cut future is in flight again; its worker
        // stopped, so it runs again from its spec.
        self.restore(|c| match c {
            Cancelled::At(at) if at >= frame => Some(Cancelled::No),
            Cancelled::InDelta { was: Some(at) } if at >= frame => {
                Some(Cancelled::InDelta { was: None })
            }
            _ => None,
        });
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
        self.ops.retain(|_, p| {
            let cancelled_before = p
                .job
                .as_ref()
                .is_some_and(|j| matches!(j.cancelled, Cancelled::At(at) if at < floor));
            p.answer.as_ref().is_none_or(|a| a.frame >= floor) && !cancelled_before
        });
    }

    /// Rebind every continuation to the new program (like the model), the
    /// answered ones too: a branch may re-park them. An op whose continuation
    /// cannot follow is cancelled; its arrival will drop.
    pub(super) fn rebind(
        &mut self,
        old: &functor_lang::ir::Module,
        new: &functor_lang::ir::Module,
    ) -> Vec<String> {
        self.reload_jobs(old, new);
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
        if arrival.token().is_none() {
            self.delta.push((seq, arrival.clone()));
            return Delivered::Unsolicited;
        }
        let answers = |p: &Pending| arrival.answers(p);
        if self.stale_after_reload(arrival) {
            return Delivered::Dropped;
        }
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

    /// Whether `arrival` answers a job a reload kept running whose reads no
    /// longer read the same: it runs again instead (unknown reads count as
    /// changed).
    fn stale_after_reload(&mut self, arrival: &Arrival) -> bool {
        let reader = self.jobs.reader();
        let pendings = self.ops.values_mut().chain(self.provisional.iter_mut().map(|(_, p)| p));
        let Some(pending) = pendings.into_iter().find(|p| arrival.answers(p)) else {
            return false;
        };
        let (Some(id), Some(job)) = (pending.token, pending.job.as_mut()) else { return false };
        if !std::mem::take(&mut job.recheck) {
            return false;
        }
        let current = match job.reads.as_ref().and_then(|slot| slot.get()) {
            Some(Some(inputs)) => crate::functor_lang_prelude::job::reads_current(reader, inputs),
            Some(None) => false,
            None => true,
        };
        if !current {
            redispatch(&self.jobs, id, job);
        }
        !current
    }

    /// A replay's re-delivery of recorded op `op`: its continuation, if it
    /// is pending on the fork.
    pub(crate) fn deliver_recorded(&mut self, op: OpId, arrival: &Arrival) -> Option<Value> {
        let pending = self
            .ops
            .get_mut(&op)
            .filter(|p| p.answer.is_none() && p.job.as_ref().is_none_or(JobOp::live))?;
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
                // Δ's cancel is after `frame`; one it replaced still counts.
                let cancelled_by = p.job.as_ref().is_some_and(|j| {
                    matches!(j.cancelled,
                        Cancelled::At(at) | Cancelled::InDelta { was: Some(at) } if at <= frame)
                });
                p.issued <= frame
                    && p.answer.as_ref().is_none_or(|a| a.frame > frame)
                    && !cancelled_by
            })
            .map(|(op, p)| {
                // Cancelled after `frame`: in flight there.
                let job = p.job.clone().map(|j| JobOp { cancelled: Cancelled::No, ..j });
                (*op, Pending { token: None, answer: None, job, ..p.clone() })
            })
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
            // A dropped answer takes its op (never a newer attempt's).
            if let Some(arrival) = self.mailbox.remove(at) {
                self.ops.retain(|_, p| !arrival.answers(p));
                self.provisional.retain(|(_, p)| !arrival.answers(p));
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
            .filter(|p| p.answer.is_none() && p.job.as_ref().is_none_or(JobOp::live))
            .filter_map(|p| Some((p.channel, p.token?)))
            .collect()
    }

    /// The live attempt of each job op in flight (the fuzzer's hung-op
    /// check).
    pub(crate) fn pending_jobs(&self) -> Vec<Ticket> {
        let ops = self.ops.values();
        let provisional = self.provisional.iter().map(|(_, p)| p);
        ops.chain(provisional)
            .filter(|p| p.answer.is_none())
            .filter_map(|p| {
                let job = p.job.as_ref().filter(|j| j.live())?;
                Some(Ticket { id: p.token?, attempt: job.attempt })
            })
            .collect()
    }

    /// Tickets of parked job results.
    pub(crate) fn parked_jobs(&self) -> Vec<Ticket> {
        self.mailbox
            .iter()
            .filter_map(|arrival| match arrival {
                Arrival::Job { ticket, .. } => Some(*ticket),
                _ => None,
            })
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
            || !self.jobs.mailbox().tickets().is_empty()
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

    // ── Jobs, over a fake dispatcher ────────────────────────────────────

    use crate::jobs::FakeJobs;
    use std::rc::Rc;

    /// An in-memory project: `path` reads as `text`.
    struct Files(&'static str, &'static str);

    impl crate::storage::ProjectReader for Files {
        fn read(&self, path: &str) -> Result<Arc<[u8]>, crate::storage::ProjectReadError> {
            if path == self.0 {
                Ok(Arc::from(self.1.as_bytes()))
            } else {
                Err(crate::storage::ProjectReadError::Unreadable("missing".into()))
            }
        }
        fn exists(&self, path: &str) -> bool {
            path == self.0
        }
    }

    fn with_fake(files: Files) -> (Requests, Rc<FakeJobs>) {
        let fake = Rc::new(FakeJobs::default());
        let mut requests = Requests::default();
        requests.jobs_mut().set_dispatch(fake.clone());
        requests.jobs_mut().set_reader(Some(Arc::new(files)));
        (requests, fake)
    }

    /// A branch keeping frames before `frame`, as the recorder runs it.
    fn branch(requests: &mut Requests, frame: Frame) {
        requests.truncate_from(frame, Some(Vec::new()));
        requests.dispatch_restored();
    }

    fn load(path: &str) -> JobSpec {
        JobSpec::FileLoad { path: path.into() }
    }

    fn issue_load(requests: &mut Requests, path: &str) {
        requests.issue_job(None, 0, load(path), tagger(0.0), true);
    }

    /// Complete every held job, then deliver what landed in the mailbox.
    fn finish(requests: &mut Requests, fake: &FakeJobs, frame: Frame) -> Vec<Delivered> {
        for ticket in fake.held() {
            fake.complete(ticket);
        }
        deliver_mail(requests, frame)
    }

    fn deliver_mail(requests: &mut Requests, frame: Frame) -> Vec<Delivered> {
        let mail = requests.jobs().mailbox().take();
        mail.into_iter()
            .map(|(ticket, value)| requests.deliver(&Arrival::Job { ticket, value }, 0, frame))
            .collect()
    }

    fn ok_text(value: &Shareable) -> String {
        let Shareable::Variant(ctor, args) = value else { panic!("{value:?}") };
        assert_eq!(&**ctor, "Result.Ok", "{value:?}");
        let Shareable::Host(crate::jobs::HostValue::Bytes(bytes)) = &args[0] else { panic!() };
        String::from_utf8(bytes.as_slice().to_vec()).unwrap()
    }

    #[test]
    fn a_job_result_answers_its_op_once() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        assert_eq!(fake.dispatched().len(), 1);
        fake.complete(fake.held()[0]);
        let mail = requests.jobs().mailbox().take();
        assert_eq!(ok_text(&mail[0].1), "hi");
        let arrival = Arrival::Job { ticket: mail[0].0, value: mail[0].1.clone() };
        assert!(matches!(requests.deliver(&arrival, 0, 1), Delivered::Op(..)));
        assert!(matches!(requests.deliver(&arrival, 0, 1), Delivered::Dropped));
        assert!(requests.pending_tokens().is_empty());
    }

    #[test]
    fn admission_reads_the_working_table_in_drain_order() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        // `run, run` in one drain: the second sees the first (provisional).
        issue_load(&mut requests, "a.txt");
        assert!(requests.job_in_flight(JobKind::File, "a.txt"));
        // `cancel, run`: the cancel takes the provisional op, so the run is
        // admitted.
        requests.cancel_job(JobKind::File, "a.txt");
        assert!(!requests.job_in_flight(JobKind::File, "a.txt"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        assert_eq!(fake.dispatched().len(), 2);
        assert_eq!(requests.pending_tokens().len(), 1);
        // The cancelled provisional op's job was stopped: it posts nothing.
        let delivered = finish(&mut requests, &fake, 1);
        assert_eq!(delivered.len(), 1);
        assert!(matches!(delivered[0], Delivered::Op(..)));
    }

    #[test]
    fn a_result_from_a_stale_attempt_drops() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        let first = fake.held()[0];
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 1);
        // A branch from frame 0 restores the op: attempt 2 runs.
        branch(&mut requests, 1);
        let attempts: Vec<u32> = fake.dispatched().iter().map(|t| t.attempt).collect();
        assert_eq!(attempts, vec![1, 2]);
        // Attempt 1 raced its stop flag and posted anyway: stale.
        let stale = Arrival::Job { ticket: first, value: Shareable::Bool(true) };
        assert!(matches!(requests.deliver(&stale, 0, 1), Delivered::Dropped));
        assert!(matches!(finish(&mut requests, &fake, 1)[..], [Delivered::Op(..)]));
    }

    #[test]
    fn mailbox_overflow_dropping_a_stale_result_keeps_the_live_attempt() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        let first = fake.held()[0];
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 1);
        branch(&mut requests, 1);
        requests.park(Arrival::Job { ticket: first, value: Shareable::Bool(true) });
        for token in 100..100 + MAILBOX_CAP as u64 {
            requests.park(http(token));
        }
        let live = Ticket { id: first.id, attempt: 2 };
        assert_eq!(requests.pending_jobs(), vec![live]);
    }

    #[test]
    fn a_result_that_finished_before_the_cancel_is_kept_when_the_cancel_is_undone() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        // Finished, not yet delivered, when the game cancels.
        fake.complete(fake.held()[0]);
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 1);
        branch(&mut requests, 1);
        assert_eq!(fake.dispatched().len(), 1, "not run again");
        assert!(matches!(deliver_mail(&mut requests, 1)[..], [Delivered::Op(..)]));
    }

    #[test]
    fn a_replacement_finishing_first_wins_and_the_old_attempt_drops() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        let first = fake.held()[0];
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 1);
        branch(&mut requests, 1);
        let second = fake.held()[1];
        fake.complete(second);
        assert!(matches!(deliver_mail(&mut requests, 1)[..], [Delivered::Op(..)]));
        // The old attempt was stopped; even run, it posts nothing.
        fake.complete(first);
        assert!(requests.jobs().mailbox().take().is_empty());
    }

    #[test]
    fn a_branch_before_a_cancel_redispatches_and_after_it_does_not() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        requests.commit(0, 1);
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 2);
        // Branch keeping frames before 3: the cancel survives.
        branch(&mut requests, 3);
        assert_eq!(fake.dispatched().len(), 1);
        assert!(requests.pending_tokens().is_empty());
        // Branch keeping frames before 2: the cancel is cut.
        branch(&mut requests, 2);
        assert_eq!(fake.dispatched().len(), 2);
        assert!(matches!(finish(&mut requests, &fake, 2)[..], [Delivered::Op(..)]));
    }

    #[test]
    fn while_scrubbed_admission_and_cancels_see_the_shown_frame() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 5);
        // Scrubbed at frame 3 the op is in flight: Δ there is the branch's.
        requests.set_view(Some(3));
        assert!(requests.job_in_flight(JobKind::File, "a.txt"));
        requests.cancel_job(JobKind::File, "a.txt");
        assert!(!requests.job_in_flight(JobKind::File, "a.txt"));
        // A seek drops Δ: the shown future's own cancel stands.
        requests.drop_delta(6);
        requests.dispatch_restored();
        requests.set_view(None);
        assert!(!requests.job_in_flight(JobKind::File, "a.txt"));
        assert_eq!(fake.dispatched().len(), 1);
    }

    #[test]
    fn a_seek_undoes_a_cancel_in_delta() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        requests.cancel_job(JobKind::File, "a.txt");
        requests.drop_delta(1);
        requests.dispatch_restored();
        assert_eq!(fake.dispatched().len(), 1, "its worker never stopped");
        assert!(matches!(finish(&mut requests, &fake, 1)[..], [Delivered::Op(..)]));
    }

    #[test]
    fn a_branch_reparks_a_cut_answer_without_redispatching() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        assert!(matches!(finish(&mut requests, &fake, 3)[..], [Delivered::Op(..)]));
        requests.commit(0, 3);
        branch(&mut requests, 2);
        assert_eq!(fake.dispatched().len(), 1, "never re-run");
        let parked = requests.take_mailbox();
        assert_eq!(parked.len(), 1);
        assert!(matches!(requests.deliver(&parked[0], 0, 2), Delivered::Op(..)));
    }

    #[test]
    fn a_cut_op_stops_its_job_and_a_replay_dispatches_nothing() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        requests.commit(0, 0);
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 1);
        branch(&mut requests, 1);
        assert!(finish(&mut requests, &fake, 1).is_empty(), "stopped: no result");
        // A replay registers the op without dispatching it.
        requests.issue_job(Some(OpId { epoch: 1, frame: 1, ordinal: 0 }), 0, load("a.txt"), tagger(0.0), false);
        assert_eq!(fake.dispatched().len(), 1);
    }

    #[test]
    fn a_fork_holds_jobs_in_flight_at_its_frame_and_stops_none() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "a.txt");
        requests.commit(0, 0);
        requests.cancel_job(JobKind::File, "a.txt");
        requests.commit(0, 2);
        let op = OpId { epoch: 0, frame: 0, ordinal: 0 };
        let value = Shareable::Bool(true);
        let arrival = Arrival::Job { ticket: Ticket { id: 0, attempt: 1 }, value };
        // At frame 1 the op was in flight (cancelled at 2); at frame 2 not.
        assert!(requests.fork(1).deliver_recorded(op, &arrival).is_some());
        assert!(requests.fork(2).deliver_recorded(op, &arrival).is_none());
        // Dropping the forks stopped nothing live: a branch restores the op
        // and its new attempt runs.
        branch(&mut requests, 1);
        assert!(matches!(finish(&mut requests, &fake, 1)[..], [Delivered::Op(..)]));
    }

    #[test]
    fn two_producers_read_their_own_files_and_get_their_own_results() {
        let (mut a, fake_a) = with_fake(Files("x.txt", "from a"));
        let (mut b, fake_b) = with_fake(Files("x.txt", "from b"));
        issue_load(&mut a, "x.txt");
        issue_load(&mut b, "x.txt");
        for fake in [&fake_a, &fake_b] {
            fake.complete(fake.held()[0]);
        }
        let (mail_a, mail_b) = (a.jobs().mailbox().take(), b.jobs().mailbox().take());
        assert_eq!(ok_text(&mail_a[0].1), "from a");
        assert_eq!(ok_text(&mail_b[0].1), "from b");
        // A's result doesn't answer B's op.
        let stray = Arrival::Job { ticket: mail_a[0].0, value: mail_a[0].1.clone() };
        assert!(matches!(b.deliver(&stray, 0, 0), Delivered::Dropped));
    }

    #[test]
    fn a_missing_file_is_a_missing_error() {
        let (mut requests, fake) = with_fake(Files("a.txt", "hi"));
        issue_load(&mut requests, "b.txt");
        fake.complete(fake.held()[0]);
        let mail = requests.jobs().mailbox().take();
        let Shareable::Variant(ctor, args) = &mail[0].1 else { panic!() };
        assert_eq!(&**ctor, "Result.Error");
        assert!(matches!(&args[0], Shareable::Variant(e, _) if &**e == "File.Missing"));
    }
}
