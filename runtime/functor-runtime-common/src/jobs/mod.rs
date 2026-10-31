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
    /// Whether the op was cancelled or discarded: a long task should return.
    pub fn cancelled(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

pub type Task = Box<dyn FnOnce(&JobCtx) -> Shareable + Send>;

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
    mailbox: Mailbox,
}

impl Job {
    /// Run on this thread and post the result, unless stopped first. A
    /// panic posts a `Crashed` failure instead.
    pub fn run(self) {
        if self.stop.load(Ordering::Relaxed) {
            return;
        }
        let ctx = JobCtx { stop: self.stop.clone() };
        let task = self.task;
        let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(&ctx)))
            .unwrap_or_else(|panic| (self.fail)(JobError::Crashed(panic_text(&panic))));
        if !self.stop.load(Ordering::Relaxed) {
            self.mailbox.post(self.ticket, value);
        }
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
            let (jobs, in_flight) = (jobs.clone(), in_flight.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("functor-job-{n}"))
                .spawn(move || loop {
                    let next = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
                    let Ok(job) = next else { return };
                    job.run();
                    in_flight.fetch_sub(1, Ordering::Relaxed);
                });
            match spawned {
                Ok(_) => started += 1,
                Err(e) => log::warn!("[functor-lang] couldn't start job worker {n}: {e}"),
            }
        }
        Pool { queue, workers: started, in_flight }
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
/// a cancelled op, a seek undoing a cancel).
#[derive(Clone, Debug, PartialEq)]
pub enum JobSpec {
    /// `File.load`: a project file's bytes.
    FileLoad { path: String },
}

impl JobSpec {
    /// The same-key rule's key: one in-flight op per key (design J2).
    pub fn key(&self) -> &str {
        match self {
            JobSpec::FileLoad { path } => path,
        }
    }

    fn task(&self, reader: Option<Arc<dyn ProjectReader>>) -> (Task, fn(JobError) -> Shareable) {
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
                (task, fail)
            }
        }
    }
}

/// A producer's side of the pool: where its jobs run, its mailbox, and the
/// reader its file jobs use.
pub struct JobPort {
    dispatch: std::rc::Rc<dyn Dispatch>,
    mailbox: Mailbox,
    /// `None`: the process's reader as of each dispatch.
    reader: Option<Arc<dyn ProjectReader>>,
}

impl Default for JobPort {
    fn default() -> JobPort {
        let dispatch: std::rc::Rc<dyn Dispatch> = if cfg!(target_arch = "wasm32") {
            std::rc::Rc::new(Inline)
        } else {
            std::rc::Rc::new(Shared)
        };
        JobPort { dispatch, mailbox: Mailbox::default(), reader: None }
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

    /// Dispatch `spec` as `ticket`; the returned flag stops it.
    pub(crate) fn start(&self, spec: &JobSpec, ticket: Ticket) -> Arc<AtomicBool> {
        let reader = self.reader.clone().or_else(crate::storage::project_reader);
        let (task, fail) = spec.task(reader);
        let stop = Arc::new(AtomicBool::new(false));
        self.dispatch.dispatch(Job {
            ticket,
            task,
            fail,
            stop: stop.clone(),
            mailbox: self.mailbox.clone(),
        });
        stop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(ticket: Ticket, mailbox: &Mailbox, task: Task) -> (Job, Arc<AtomicBool>) {
        let stop = Arc::new(AtomicBool::new(false));
        let fail = |e: JobError| Shareable::Text(Arc::from(format!("{e:?}").as_str()));
        (Job { ticket, task, fail, stop: stop.clone(), mailbox: mailbox.clone() }, stop)
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
    fn a_panic_posts_a_crash_and_the_worker_lives_on() {
        let pool = Pool::start(1);
        let mailbox = Mailbox::default();
        pool.dispatch(job(ticket(1), &mailbox, Box::new(|_| panic!("boom"))).0);
        pool.dispatch(job(ticket(2), &mailbox, Box::new(|_| Shareable::Bool(true))).0);
        let mut got = Vec::new();
        while got.len() < 2 {
            got.extend(mailbox.take());
            std::thread::yield_now();
        }
        assert_eq!(got[0].1, Shareable::Text(Arc::from("Crashed(\"boom\")")));
        assert_eq!(got[1].1, Shareable::Bool(true));
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
