//! The load job: a project's top-level values evaluated on a loader thread
//! while the frame thread keeps drawing (a loading view at startup). The
//! thread compiles the project itself, evaluates it reusing its own previous
//! load, and sends back every value that can cross ([`Shareable`]) with the
//! load's records. The frame thread compiles its own module and adopts them
//! ([`Session::load_adopting`]), evaluating only what couldn't cross
//! (functions, mostly) itself.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use functor_lang::eval::LoadWatch;
use functor_lang::ir::{ExprKind, Module};
use functor_lang::reuse::{Records, ReuseStats};
use functor_lang::{Host, RunFailure, Session};

use super::{panic_text, ProgramSource, Progress, Shareable};
use crate::functor_lang_prelude::{now_ms, FunctorHost};
use crate::storage::ProjectReader;

/// A def that couldn't cross and took longer than this on the loader warns:
/// the frame thread evaluates it again.
const SLOW_MAIN_ONLY_MS: f64 = 50.0;

/// What a load job produced: the values that crossed, and what the frame
/// thread needs to adopt them.
pub struct Evaluated {
    /// Each value def whose value could cross, by name.
    values: HashMap<String, Shareable>,
    records: Records,
    pub stats: ReuseStats,
    /// The project files the load read, by version (the data watch).
    pub file_reads: BTreeMap<String, Option<String>>,
    /// Value defs that couldn't cross, with what each took on the loader.
    main_only: Vec<(String, f64)>,
    /// What each def it evaluated took.
    took: HashMap<String, f64>,
}

impl Evaluated {
    /// `module`'s session, from these values: `module` must be the project
    /// the job compiled. Defs that couldn't cross evaluate here, on `host`.
    pub fn adopt(self, module: &Module, host: &mut dyn Host, costs: &mut Costs) -> Result<(Session, ReuseStats), RunFailure> {
        costs.merge(self.took);
        for (name, ms) in &self.main_only {
            if *ms > SLOW_MAIN_ONLY_MS {
                log::warn!(
                    "[functor-lang] `{name}` took {ms:.0} ms to load and evaluates again on the \
frame thread: its value holds a function or engine value that can't leave the loader — move its \
heavy data into a plain-data top-level value"
                );
            }
        }
        let values = self.values.into_iter().map(|(name, v)| (name, v.to_value())).collect();
        let session = Session::load_adopting(module, host, values, self.records)?;
        Ok((session, self.stats))
    }
}

/// How a load job ended.
pub enum Outcome {
    Evaluated(Box<Evaluated>),
    /// A def failed: the rendered error, and what the load read (creating a
    /// file it lacked can fix it).
    Failed { message: String, file_reads: BTreeMap<String, Option<String>> },
    /// The load read state only the frame thread has (the physics world, UI
    /// slots): load there instead. Why, for the log.
    Inline(String),
}

/// A load job's shared state: its stop flag, progress, and result.
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    progress: Progress,
    done: Mutex<Option<Outcome>>,
}

/// A load in flight. Dropping it stops the job.
pub struct LoadHandle {
    shared: Arc<Shared>,
}

impl LoadHandle {
    /// The latest progress in 0..1: defs started over all defs, or a
    /// `Job.report` from inside one, whichever came last.
    pub fn progress(&self) -> Option<f64> {
        self.shared.progress.latest()
    }

    /// The outcome, once the job has finished (taken once).
    pub fn take(&self) -> Option<Outcome> {
        self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl Drop for LoadHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

struct Request {
    source: Arc<ProgramSource>,
    reader: Option<Arc<dyn ProjectReader>>,
    shared: Arc<Shared>,
}

/// A producer's loader thread, which keeps its previous load to reuse from.
/// Started on the first load; it ends when this is dropped.
#[derive(Default)]
pub struct Loader {
    requests: Option<mpsc::Sender<Request>>,
}

impl Loader {
    /// Evaluate `source`'s top-level values on the loader, reading files
    /// through `reader`. `None` when the thread can't start (the web).
    pub fn start(&mut self, source: Arc<ProgramSource>, reader: Option<Arc<dyn ProjectReader>>) -> Option<LoadHandle> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let shared = Arc::new(Shared::default());
        let requests = match &self.requests {
            Some(requests) => requests,
            None => self.requests.insert(spawn()?),
        };
        // `serve` catches panics, so the thread outlives every request.
        requests.send(Request { source, reader, shared: shared.clone() }).ok()?;
        Some(LoadHandle { shared })
    }
}

fn spawn() -> Option<mpsc::Sender<Request>> {
    let (send, requests) = mpsc::channel::<Request>();
    let spawned = std::thread::Builder::new()
        .name("functor-load".to_string())
        .stack_size(super::WORKER_STACK)
        .spawn(move || {
            super::waited_on();
            serve(requests)
        });
    match spawned {
        Ok(_) => Some(send),
        Err(e) => {
            log::warn!("[functor-lang] couldn't start the loader thread: {e}");
            None
        }
    }
}

fn serve(requests: mpsc::Receiver<Request>) {
    let mut previous: Option<Session> = None;
    for request in requests {
        let shared = request.shared.clone();
        if shared.stop.load(Ordering::Relaxed) {
            continue;
        }
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::reporting_to(&shared.progress, || evaluate(&request, previous.as_ref()))
        }));
        let outcome = match ran {
            Ok((outcome, session)) => {
                if let Some(session) = session {
                    previous = Some(session);
                }
                outcome
            }
            Err(panic) => {
                previous = None;
                let message = format!("the loader crashed: {}", panic_text(&panic));
                Outcome::Failed { message, file_reads: BTreeMap::new() }
            }
        };
        *shared.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
    }
}

/// Times each def a load evaluates; reports defs started over all defs as
/// progress (the loader), or stops before a heavy one (a deferrable
/// reload).
struct Watch<'w> {
    progress: Option<&'w Progress>,
    /// With the last load's costs and records: stop before a heavy def.
    defer: Option<(&'w Costs, Option<&'w Records>)>,
    /// When the def being evaluated started.
    started: f64,
    took: HashMap<String, f64>,
    /// What this load has evaluated so far, in ms.
    spent: f64,
    heavy: Option<String>,
}

impl<'w> Watch<'w> {
    fn new(progress: Option<&'w Progress>, defer: Option<(&'w Costs, Option<&'w Records>)>) -> Watch<'w> {
        Watch { progress, defer, started: 0.0, took: HashMap::new(), spent: 0.0, heavy: None }
    }
}

impl LoadWatch for Watch<'_> {
    fn evaluating(&mut self, index: usize, total: usize, name: &str) -> bool {
        if let Some((costs, records)) = self.defer {
            let slow = costs.0.get(name).is_some_and(|ms| *ms > HEAVY_MS) || self.spent > HEAVY_MS;
            if slow || records.is_some_and(|records| records.read_files(name)) {
                self.heavy = Some(name.to_string());
                return false;
            }
        }
        if let Some(progress) = self.progress {
            progress.set(index as f64 / total.max(1) as f64);
        }
        self.started = now_ms();
        true
    }

    fn evaluated(&mut self, name: &str) {
        let took = now_ms() - self.started;
        self.spent += took;
        self.took.insert(name.to_string(), took);
    }
}

/// A def that took longer than this, if a reload must evaluate it again,
/// sends the reload to the loader.
pub const HEAVY_MS: f64 = 50.0;

/// Each def's last evaluation time, in ms.
#[derive(Default)]
pub struct Costs(HashMap<String, f64>);

impl Costs {
    fn merge(&mut self, took: HashMap<String, f64>) {
        self.0.extend(took);
    }
}

/// How a load on this thread ended.
pub enum Inline {
    Loaded(Session, ReuseStats),
    /// It would have evaluated this heavy def: the loader should.
    Heavy(String),
    Failed(RunFailure),
}

/// A host whose file inputs never check as current, so a deferrable load
/// reads no file on the frame thread: a value that read files evaluates
/// again, which defers it to the loader (which re-checks them).
struct Unchecked<'h>(&'h mut dyn Host);

impl Host for Unchecked<'_> {
    fn provides(&self, path: &str) -> bool {
        self.0.provides(path)
    }

    fn call(
        &mut self,
        path: &str,
        args: Vec<functor_lang::Value>,
        span: functor_lang::Span,
    ) -> Result<functor_lang::HostReturn, functor_lang::RunError> {
        self.0.call(path, args, span)
    }

    fn begin_inputs(&mut self) {
        self.0.begin_inputs()
    }

    fn end_inputs(&mut self) -> Option<Vec<functor_lang::reuse::HostInput>> {
        self.0.end_inputs()
    }

    fn inputs_current(&mut self, inputs: &[functor_lang::reuse::HostInput]) -> bool {
        inputs.is_empty()
    }
}

/// [`Session::load_reusing`] on this thread, timing each def into `costs`.
/// With `defer`, it reads no file and stops before evaluating a heavy def:
/// slow last time ([`HEAVY_MS`]), a reader of project files (a decode,
/// typically), or any def once the load has spent that long.
pub fn load_inline(
    module: &Module,
    host: &mut dyn Host,
    previous: Option<&Session>,
    types: HashMap<String, String>,
    costs: &mut Costs,
    defer: bool,
) -> Inline {
    let records = previous.and_then(Session::records);
    let mut watch = Watch::new(None, defer.then_some((&*costs, records)));
    let mut unchecked = Unchecked(&mut *host);
    let host: &mut dyn Host = if defer { &mut unchecked } else { unchecked.0 };
    let loaded = Session::load_watched(module, host, previous, types, None, &mut watch);
    let Watch { took, heavy, .. } = watch;
    match (loaded, heavy) {
        (Ok((session, stats)), _) => {
            costs.merge(took);
            Inline::Loaded(session, stats)
        }
        (Err(_), Some(def)) => Inline::Heavy(def),
        (Err(failure), None) => Inline::Failed(failure),
    }
}

/// Compile and evaluate on this thread; the new session, when it loaded,
/// becomes the next load's previous.
fn evaluate(request: &Request, previous: Option<&Session>) -> (Outcome, Option<Session>) {
    let _reader = crate::storage::scope_reader(request.reader.clone());
    let files = request.source.files.iter().map(|(p, s)| (std::path::PathBuf::from(p), s.clone()));
    let project = match functor_lang::project::load_sources_with_bundled_modules(
        files.collect(),
        &functor_prelude::bundled_modules(),
    ) {
        Ok(project) => project,
        Err(e) => {
            let message = format!("cannot load {}", e.render());
            return (Outcome::Failed { message, file_reads: BTreeMap::new() }, None);
        }
    };
    // Type diagnostics are advisory in the dev loop, as on the frame thread.
    let (diagnostics, types) = project.check_with_types();
    for diag in diagnostics {
        log::warn!("warning: {}", project.sources.render(diag.span.start, &diag.message));
    }
    let types = functor_lang::reuse::def_types(&project.module, &types);
    crate::functor_lang_prelude::bytes::take_file_reads();
    let live_before = crate::functor_lang_prelude::live_reads();
    let mut watch = Watch::new(Some(&request.shared.progress), None);
    let loaded = Session::load_watched(
        &project.module,
        &mut FunctorHost,
        previous,
        types,
        Some(&request.shared.stop),
        &mut watch,
    );
    let file_reads = crate::functor_lang_prelude::bytes::take_file_reads();
    let (session, stats) = match loaded {
        Ok(loaded) => loaded,
        Err(f) => {
            let message = project.sources.render(f.error.span.start, &f.error.message);
            let message = format!("cannot load {message}");
            return (Outcome::Failed { message, file_reads }, None);
        }
    };
    if crate::functor_lang_prelude::live_reads() != live_before {
        let why = "a top-level value read the physics world or UI slots at load".to_string();
        return (Outcome::Inline(why), None);
    }
    let mut values = HashMap::new();
    let mut main_only = Vec::new();
    for def in &project.module.defs {
        if matches!(def.value.kind, ExprKind::Lambda { .. }) {
            continue;
        }
        let value = session.global(&def.name).expect("a loaded def has a value");
        match Shareable::from_value(&value) {
            Ok(shared) => {
                values.insert(def.name.clone(), shared);
            }
            Err(_) => main_only.push((def.name.clone(), watch.took.get(&def.name).copied().unwrap_or(0.0))),
        }
    }
    let records = session.records().cloned().unwrap_or_default();
    let evaluated = Evaluated {
        values,
        records,
        stats,
        file_reads,
        main_only,
        took: watch.took,
    };
    (Outcome::Evaluated(Box::new(evaluated)), Some(session))
}
