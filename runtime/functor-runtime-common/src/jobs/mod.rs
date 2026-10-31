//! Jobs: heavy work off the frame thread, whose results come back as
//! recorded deliveries (docs/time-travel.md, "Jobs"). One process-wide pool
//! runs native tasks; each producer has its own mailbox, and a job reads
//! through the reader captured when it was dispatched (the process's, unless
//! the producer installs its own), never a later or global one.
//!
//! A job is dispatched when the effect drain issues it, and its result is an
//! `Arrival::Job` the producer delivers like an HTTP response: logged, so a
//! replay hands it back instead of running the job again. Every result
//! carries its [`Ticket`]; one whose attempt is no longer the op's is stale
//! and dropped.

pub mod load;
mod shareable;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

pub use shareable::{HostValue, Key, Shareable, NODE_CAP};

use crate::storage::ProjectReader;

/// Which dispatch of which op a result answers. `id` names the op for its
/// whole life; `attempt` bumps on each re-dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket {
    pub id: u64,
    pub attempt: u32,
}

/// A process-unique op id: results route by mailbox, so this only has to
/// tell a producer's ops apart.
pub(crate) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Why the pool produced no result of its own.
#[derive(Clone, Debug, PartialEq)]
pub enum JobError {
    /// The job couldn't run: the queue was full.
    Failed(String),
    /// The task panicked.
    Crashed(String),
}

/// What a running task can ask.
pub struct JobCtx {
    stop: Arc<AtomicBool>,
}

impl JobCtx {
    #[cfg(test)]
    pub(crate) fn new(stop: Arc<AtomicBool>) -> JobCtx {
        JobCtx { stop }
    }

    /// Whether the op was cancelled or discarded: a long task should return.
    pub fn cancelled(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// The flag behind [`Self::cancelled`], for an evaluation to poll.
    pub fn stop_flag(&self) -> &AtomicBool {
        &self.stop
    }
}

pub type Task = Box<dyn FnOnce(&JobCtx) -> Shareable + Send>;

/// An attempt's latest reported progress in 0..1, unset until its first
/// report. The worker overwrites it and the producer reads it once a frame,
/// so a job reporting in a tight loop can't flood anything.
#[derive(Clone, Debug)]
pub struct Progress(Arc<AtomicU64>);

/// No report yet: a NaN's bits, which [`Progress::set`] never stores.
const UNSET: u64 = u64::MAX;

impl Default for Progress {
    fn default() -> Progress {
        Progress(Arc::new(AtomicU64::new(UNSET)))
    }
}

impl Progress {
    /// Clamped to 0..1; a NaN is ignored.
    fn set(&self, fraction: f64) {
        if !fraction.is_nan() {
            self.0.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn latest(&self) -> Option<f64> {
        let bits = self.0.load(Ordering::Relaxed);
        (bits != UNSET).then(|| f64::from_bits(bits))
    }
}

thread_local! {
    /// Where `Job.report` writes while this thread runs a job.
    static REPORTING: RefCell<Option<Progress>> = const { RefCell::new(None) };
}

/// Run `f` with `Job.report` writing to `progress`.
fn reporting_to<R>(progress: &Progress, f: impl FnOnce() -> R) -> R {
    let outer = REPORTING.with(|slot| slot.replace(Some(progress.clone())));
    let out = f();
    REPORTING.with(|slot| *slot.borrow_mut() = outer);
    out
}

/// `Job.report`: the running job's progress is now `fraction`. Outside a
/// job, nothing.
pub fn report(fraction: f64) {
    REPORTING.with(|slot| {
        if let Some(progress) = &*slot.borrow() {
            progress.set(fraction);
        }
    });
}

/// Results waiting for their producer.
#[derive(Clone, Default)]
pub struct Mailbox(Arc<Mutex<Vec<(Ticket, Shareable)>>>);

impl Mailbox {
    fn post(&self, ticket: Ticket, value: Shareable) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push((ticket, value));
    }

    /// Every result posted so far, in posting order.
    pub fn take(&self) -> Vec<(Ticket, Shareable)> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Tickets posted and not yet taken (the fuzzer's hung-op check).
    pub(crate) fn tickets(&self) -> Vec<Ticket> {
        let posted = self.0.lock().unwrap_or_else(|e| e.into_inner());
        posted.iter().map(|(ticket, _)| *ticket).collect()
    }
}

/// One dispatch of an op: the task, how to report a failure in the op's
/// result type, its stop flag and where its result goes.
pub struct Job {
    pub ticket: Ticket,
    task: Task,
    fail: fn(JobError) -> Shareable,
    stop: Arc<AtomicBool>,
    progress: Progress,
    mailbox: Mailbox,
}

impl Job {
    /// Run on this thread and post the result, unless stopped first. A
    /// panic posts a `Crashed` failure instead; `true` if it panicked.
    pub fn run(self) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return false;
        }
        let ctx = JobCtx { stop: self.stop.clone() };
        let task = self.task;
        let ran = reporting_to(&self.progress, || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(&ctx)))
        });
        let (value, crashed) = match ran {
            Ok(value) => (value, false),
            Err(panic) => ((self.fail)(JobError::Crashed(panic_text(&panic))), true),
        };
        if !self.stop.load(Ordering::Relaxed) {
            self.mailbox.post(self.ticket, value);
        }
        crashed
    }

    /// Post a failure without running.
    fn refuse(self, error: JobError) {
        self.mailbox.post(self.ticket, (self.fail)(error));
    }
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the job panicked".to_string())
}

/// Where a producer's jobs run.
pub trait Dispatch {
    fn dispatch(&self, job: Job);
}

/// Tasks in the pool's queue or running, across every producer.
const QUEUE_CAP: usize = 256;

/// The process's job pool: a few worker threads draining one queue.
pub struct Pool {
    queue: mpsc::Sender<Job>,
    /// Workers that started: with none, jobs are refused, not left queued.
    workers: usize,
    /// Queued plus running tasks, stopped ones included until they return.
    in_flight: Arc<AtomicUsize>,
}

impl Pool {
    /// The shared pool, started on first use.
    pub fn shared() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| Pool::start(workers()))
    }

    fn start(workers: usize) -> Pool {
        let (queue, jobs) = mpsc::channel::<Job>();
        let jobs = Arc::new(Mutex::new(jobs));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut started = 0;
        for n in 0..workers {
            match spawn_worker(n, jobs.clone(), in_flight.clone()) {
                Ok(()) => started += 1,
                Err(e) => log::warn!("[functor-lang] couldn't start job worker {n}: {e}"),
            }
        }
        Pool { queue, workers: started, in_flight }
    }
}

/// A worker's stack: dropping a deeply nested value recurses, ~100 bytes a
/// level (functor-lang's stack contract). Reserved, committed as used.
const WORKER_STACK: usize = 256 << 20;

/// Start worker `n`. After a panic it hands over to a fresh thread: the
/// panic may have left this one's host state (caches, read records) half
/// updated.
fn spawn_worker(
    n: usize,
    jobs: Arc<Mutex<mpsc::Receiver<Job>>>,
    in_flight: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(format!("functor-job-{n}"))
        .stack_size(WORKER_STACK)
        .spawn(move || {
            lower_priority();
            loop {
                let next = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
                let Ok(job) = next else { return };
                let crashed = job.run();
                in_flight.fetch_sub(1, Ordering::Relaxed);
                if crashed {
                    if let Err(e) = spawn_worker(n, jobs, in_flight) {
                        log::warn!("[functor-lang] couldn't replace job worker {n}: {e}");
                    }
                    return;
                }
            }
        })
        .map(|_| ())
}

/// Background work: utility QoS on macOS, so jobs don't contend with the
/// frame's threads for performance cores.
fn lower_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: plain libc call on the calling thread; no pointers involved.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

impl Dispatch for Pool {
    fn dispatch(&self, job: Job) {
        if self.workers == 0 {
            return job.refuse(JobError::Failed("no job worker could start".to_string()));
        }
        if self.in_flight.fetch_add(1, Ordering::Relaxed) >= QUEUE_CAP {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            return job.refuse(JobError::Failed(format!(
                "the job queue is full ({QUEUE_CAP} jobs queued or running)"
            )));
        }
        if let Err(mpsc::SendError(job)) = self.queue.send(job) {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            job.refuse(JobError::Failed("the job pool has stopped".to_string()));
        }
    }
}

/// Workers beside the render and sim threads: Quest runs one native job at
/// a time (the platform reserves cores); desktop leaves two cores free.
fn workers() -> usize {
    if cfg!(target_os = "android") {
        return 1;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    cores.saturating_sub(2).clamp(1, 4)
}

/// Runs each job at dispatch, on the calling thread: the web (no threads).
/// The result still arrives through the mailbox, next frame, so it is
/// logged like any other.
pub struct Inline;

impl Dispatch for Inline {
    fn dispatch(&self, job: Job) {
        job.run();
    }
}

/// The shared pool, started on its first dispatch.
struct Shared;

impl Dispatch for Shared {
    fn dispatch(&self, job: Job) {
        Pool::shared().dispatch(job);
    }
}

/// Holds jobs until the test completes them, in any order (deterministic
/// lifecycle tests and the fuzzer).
#[derive(Default)]
pub struct FakeJobs {
    held: RefCell<Vec<Job>>,
    dispatched: RefCell<Vec<Ticket>>,
}

impl FakeJobs {
    /// Held tickets, oldest first.
    pub fn held(&self) -> Vec<Ticket> {
        self.held.borrow().iter().map(|job| job.ticket).collect()
    }

    /// Every ticket ever dispatched, in order.
    pub fn dispatched(&self) -> Vec<Ticket> {
        self.dispatched.borrow().clone()
    }

    /// Whether the held job `ticket` was stopped; `None` if it isn't held.
    pub fn stopped(&self, ticket: Ticket) -> Option<bool> {
        let held = self.held.borrow();
        let job = held.iter().find(|job| job.ticket == ticket)?;
        Some(job.stop.load(Ordering::Relaxed))
    }

    /// Report `fraction` from the held job `ticket`, as if it were running.
    /// `false` if it isn't held.
    pub fn report(&self, ticket: Ticket, fraction: f64) -> bool {
        let held = self.held.borrow();
        let Some(job) = held.iter().find(|job| job.ticket == ticket) else {
            return false;
        };
        job.progress.set(fraction);
        true
    }

    /// Run the held job `ticket` now, posting its result unless it was
    /// stopped. `false` if it isn't held.
    pub fn complete(&self, ticket: Ticket) -> bool {
        let mut held = self.held.borrow_mut();
        let Some(at) = held.iter().position(|job| job.ticket == ticket) else {
            return false;
        };
        let job = held.remove(at);
        drop(held);
        job.run();
        true
    }
}

impl Dispatch for FakeJobs {
    fn dispatch(&self, job: Job) {
        self.dispatched.borrow_mut().push(job.ticket);
        self.held.borrow_mut().push(job);
    }
}

/// What a job op asks for: enough to dispatch it again (a branch restoring
/// a cancelled op, a seek undoing a cancel, a reload).
#[derive(Clone, Debug, PartialEq)]
pub enum JobSpec {
    /// `File.load`: a project file's bytes.
    FileLoad { path: String },
    /// `Job.run`: a top-level function called on a worker.
    Run(Arc<RunSpec>),
}

/// Which API a job key belongs to: `Job.cancel(k)` cancels `Job.run`s,
/// `File.cancel(p)` `File.load`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    File,
    Run,
}

impl JobSpec {
    /// The same-key rule's key: one in-flight op per kind and key (design
    /// J2).
    pub fn key(&self) -> (JobKind, &str) {
        match self {
            JobSpec::FileLoad { path } => (JobKind::File, path),
            JobSpec::Run(run) => (JobKind::Run, &run.key),
        }
    }

    fn task(&self, reader: Option<Arc<dyn ProjectReader>>) -> (Task, fn(JobError) -> Shareable, Option<ReadSlot>) {
        match self {
            JobSpec::FileLoad { path } => {
                let path = path.clone();
                let task: Task = Box::new(move |_| {
                    crate::functor_lang_prelude::bytes::load_file(reader.as_deref(), &path)
                });
                let fail = |error: JobError| {
                    let (JobError::Failed(m) | JobError::Crashed(m)) = error;
                    crate::functor_lang_prelude::bytes::load_failed(m)
                };
                (task, fail, None)
            }
            JobSpec::Run(run) => {
                let (run, reads) = (run.clone(), ReadSlot::default());
                let slot = reads.clone();
                let task: Task = Box::new(move |ctx| {
                    crate::functor_lang_prelude::job::run(&run, reader, ctx, &slot)
                });
                (task, crate::functor_lang_prelude::job::failed, Some(reads))
            }
        }
    }
}

/// A `Job.run` op's request.
#[derive(Debug)]
pub struct RunSpec {
    pub key: String,
    /// What to call; `Err` holds the error result when it can't run at all
    /// (a function that isn't top-level, an input that can't cross).
    pub call: Result<RunCall, Shareable>,
}

impl PartialEq for RunSpec {
    fn eq(&self, other: &RunSpec) -> bool {
        std::ptr::eq(self, other)
    }
}

/// A call a worker can make: the project to compile, the top-level
/// function, and its input.
#[derive(Clone, Debug)]
pub struct RunCall {
    pub program: Arc<ProgramSource>,
    pub f: String,
    pub input: Shareable,
}

/// One revision of a project, as a worker compiles it: the project's own
/// files, entry first (the bundled modules are the worker's own).
#[derive(Debug)]
pub struct ProgramSource {
    /// Unique per revision: a worker's compile cache key.
    pub id: u64,
    pub files: Vec<(String, String)>,
}

impl ProgramSource {
    pub fn new(files: Vec<(String, String)>) -> ProgramSource {
        ProgramSource { id: next_id(), files }
    }

    /// The project files `sources` linked (the bundled modules left out).
    pub fn of(sources: &functor_lang::project::SourceMap) -> ProgramSource {
        let files = sources
            .files()
            .iter()
            .filter(|file| !file.path.to_string_lossy().starts_with('<'))
            .map(|file| (file.path.to_string_lossy().into_owned(), file.src.clone()))
            .collect();
        ProgramSource::new(files)
    }
}

/// The project as a producer runs it: what a worker compiles, and which
/// closures are its top-level functions.
pub struct JobProgram {
    pub source: Arc<ProgramSource>,
    pub roots: functor_lang::reuse::Roots,
}

impl JobProgram {
    /// The loaded `module`, linked from `sources`.
    pub fn new(module: &functor_lang::ir::Module, sources: &functor_lang::project::SourceMap) -> JobProgram {
        JobProgram {
            source: Arc::new(ProgramSource::of(sources)),
            roots: functor_lang::reuse::Roots::of(module),
        }
    }
}

/// What a `Job.run` attempt read, set when it finishes: its file inputs,
/// or `None` when it read something no input describes. A reload that kept
/// the attempt running re-checks them before delivering its result.
pub type ReadSlot = Arc<OnceLock<Option<Vec<functor_lang::reuse::HostInput>>>>;

/// A producer's side of the pool: where its jobs run, its mailbox, the
/// reader its jobs use, and the program `Job.run` compiles.
pub struct JobPort {
    dispatch: std::rc::Rc<dyn Dispatch>,
    mailbox: Mailbox,
    /// `None`: the process's reader as of each dispatch.
    reader: Option<Arc<dyn ProjectReader>>,
    /// `None` until the producer installs its program: `Job.run` is
    /// unsupported.
    program: Option<std::rc::Rc<JobProgram>>,
}

impl Default for JobPort {
    fn default() -> JobPort {
        let dispatch: std::rc::Rc<dyn Dispatch> = if cfg!(target_arch = "wasm32") {
            std::rc::Rc::new(Inline)
        } else {
            std::rc::Rc::new(Shared)
        };
        JobPort { dispatch, mailbox: Mailbox::default(), reader: None, program: None }
    }
}

impl JobPort {
    /// Run this producer's jobs on `dispatch` (a test's [`FakeJobs`]).
    pub fn set_dispatch(&mut self, dispatch: std::rc::Rc<dyn Dispatch>) {
        self.dispatch = dispatch;
    }

    /// Read this producer's files through `reader`, not the process's.
    pub fn set_reader(&mut self, reader: Option<Arc<dyn ProjectReader>>) {
        self.reader = reader;
    }

    pub fn mailbox(&self) -> &Mailbox {
        &self.mailbox
    }

    /// Run `Job.run`s against this program from now on.
    pub fn set_program(&mut self, program: JobProgram) {
        self.program = Some(std::rc::Rc::new(program));
    }

    pub fn program(&self) -> Option<&JobProgram> {
        self.program.as_deref()
    }

    /// A fork's port: the same program (a replay's `Job.run` digest names
    /// its function), nothing in flight.
    pub(crate) fn fork(&self) -> JobPort {
        JobPort { program: self.program.clone(), ..JobPort::default() }
    }

    /// The reader a job dispatched now reads through.
    pub(crate) fn reader(&self) -> Option<Arc<dyn ProjectReader>> {
        self.reader.clone().or_else(crate::storage::project_reader)
    }

    /// Dispatch `spec` as `ticket`.
    pub(crate) fn start(&self, spec: &JobSpec, ticket: Ticket) -> Started {
        let (task, fail, reads) = spec.task(self.reader());
        let (stop, progress) = (Arc::new(AtomicBool::new(false)), Progress::default());
        self.dispatch.dispatch(Job {
            ticket,
            task,
            fail,
            stop: stop.clone(),
            progress: progress.clone(),
            mailbox: self.mailbox.clone(),
        });
        Started { stop, reads, progress }
    }
}

/// A dispatched attempt's handles: `stop` stops it, a `Job.run`'s `reads`
/// receives what it read, and `progress` is what it reported.
pub(crate) struct Started {
    pub stop: Arc<AtomicBool>,
    pub reads: Option<ReadSlot>,
    pub progress: Progress,
}

#[cfg(test)]
mod load_tests;
#[cfg(test)]
mod run_tests;

/// Counts dispatches on their way to the shared pool (tests).
#[cfg(test)]
pub(crate) struct Counted(pub std::cell::Cell<usize>);

#[cfg(test)]
impl Dispatch for Counted {
    fn dispatch(&self, job: Job) {
        self.0.set(self.0.get() + 1);
        Pool::shared().dispatch(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(ticket: Ticket, mailbox: &Mailbox, task: Task) -> (Job, Arc<AtomicBool>) {
        let stop = Arc::new(AtomicBool::new(false));
        let fail = |e: JobError| Shareable::Text(Arc::from(format!("{e:?}").as_str()));
        let progress = Progress::default();
        (Job { ticket, task, fail, stop: stop.clone(), progress, mailbox: mailbox.clone() }, stop)
    }

    fn ticket(id: u64) -> Ticket {
        Ticket { id, attempt: 1 }
    }

    #[test]
    fn the_pool_runs_jobs_off_thread_and_posts_results() {
        let pool = Pool::start(2);
        let mailbox = Mailbox::default();
        let caller = std::thread::current().id();
        for id in 0..8 {
            let (job, _) = job(ticket(id), &mailbox, Box::new(move |_| {
                assert_ne!(std::thread::current().id(), caller);
                Shareable::Number(id as f64)
            }));
            pool.dispatch(job);
        }
        let mut got = Vec::new();
        while got.len() < 8 {
            got.extend(mailbox.take());
            std::thread::yield_now();
        }
        got.sort_by_key(|(t, _)| *t);
        assert_eq!(got[3], (ticket(3), Shareable::Number(3.0)));
    }

    #[test]
    fn a_panic_posts_a_crash_and_a_fresh_worker_takes_over() {
        let pool = Pool::start(1);
        let mailbox = Mailbox::default();
        let thread = || Shareable::Text(Arc::from(format!("{:?}", std::thread::current().id()).as_str()));
        pool.dispatch(job(ticket(1), &mailbox, Box::new(move |_| thread())).0);
        pool.dispatch(job(ticket(2), &mailbox, Box::new(|_| panic!("boom"))).0);
        pool.dispatch(job(ticket(3), &mailbox, Box::new(move |_| thread())).0);
        let mut got = Vec::new();
        while got.len() < 3 {
            got.extend(mailbox.take());
            std::thread::yield_now();
        }
        assert_eq!(got[1].1, Shareable::Text(Arc::from("Crashed(\"boom\")")));
        assert_ne!(got[0].1, got[2].1, "the panicked thread retired");
    }

    #[test]
    fn report_writes_the_running_jobs_progress_and_is_a_no_op_outside_one() {
        let fake = FakeJobs::default();
        let mailbox = Mailbox::default();
        let (mut running, _) = job(ticket(1), &mailbox, Box::new(|_| {
            report(2.0);
            report(f64::NAN);
            Shareable::Bool(true)
        }));
        let progress = Progress::default();
        running.progress = progress.clone();
        fake.dispatch(running);
        report(0.5);
        assert_eq!(progress.latest(), None, "outside a job");
        fake.complete(ticket(1));
        assert_eq!(progress.latest(), Some(1.0), "clamped; a NaN ignored");
        report(0.5);
        assert_eq!(progress.latest(), Some(1.0), "the job has returned");
    }

    #[test]
    fn a_stopped_job_posts_nothing() {
        let fake = FakeJobs::default();
        let mailbox = Mailbox::default();
        let (queued, stop) = job(ticket(1), &mailbox, Box::new(|_| Shareable::Bool(true)));
        fake.dispatch(queued);
        stop.store(true, Ordering::Relaxed);
        assert!(fake.complete(ticket(1)));
        assert!(mailbox.take().is_empty());
    }

    #[test]
    fn a_full_queue_refuses_with_a_failure() {
        let pool = Pool::start(1);
        let mailbox = Mailbox::default();
        // The first job holds the only worker, so the rest queue up.
        let (release, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        pool.dispatch(job(ticket(0), &mailbox, Box::new(move |_| {
            let _ = wait.lock().unwrap().recv();
            Shareable::Bool(true)
        })).0);
        for id in 1..=QUEUE_CAP as u64 {
            pool.dispatch(job(ticket(id), &mailbox, Box::new(|_| Shareable::Bool(true))).0);
        }
        let refused = mailbox.take();
        release.send(()).unwrap();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].0, ticket(QUEUE_CAP as u64));
        assert!(matches!(&refused[0].1, Shareable::Text(t) if t.contains("queue is full")));
    }
}
