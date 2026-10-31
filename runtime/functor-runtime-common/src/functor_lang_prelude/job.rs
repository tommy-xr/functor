//! `Job.*` — run a top-level function on a job worker (see `job.funi` and
//! docs/time-travel.md, "Jobs"). The worker compiles the producer's project
//! itself (values are `Rc`, so nothing of main's session can cross), evaluates
//! only the defs the function reaches, calls it, and sends back a
//! [`Shareable`]. Job code runs like a top-level value: it may read files,
//! never physics or UI state.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use functor_lang::reuse::{CodeIndex, HostInput};
use functor_lang::Session;

use super::*;
use crate::jobs::{JobCtx, JobError, JobProgram, JobSpec, ReadSlot, RunCall, RunSpec, Shareable};
use crate::storage::ProjectReader;

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn4(
        "Job.run",
        "Job.run(key, f, input, tagger)",
        |key: String, f: Value, input: Value, tagger: Tagger| {
            FunctorLangEffect(EffectTree::JobRun { key, f, input, tagger: tagger.0 })
        },
    );
    reg.fn1("Job.cancel", "Job.cancel(key)", |key: String| {
        FunctorLangEffect(EffectTree::JobCancel { key })
    });
    reg.fn2("Job.report", "Job.report(fraction, value)", |fraction: f64, value: Value| {
        crate::jobs::report(fraction);
        value
    });
    reg.fn2("Job.progress", "Job.progress(key, tagger)", |key: String, tagger: Tagger| {
        FunctorLangSub(SubTree::JobProgress { key, tagger: tagger.0 })
    });
    reg.fn1("Job.describe", "Job.describe(error)", |error: Value| match &error {
        Value::Variant { ctor, args } => match (&**ctor, args.first()) {
            ("Job.Crashed", Some(Value::String(m))) => Ok(Value::String(format!("the job crashed: {m}").into())),
            (_, Some(Value::String(m))) => Ok(Value::String(m.clone())),
            _ => Err(format!("Job.describe: expected a Job.error, got {}", error.kind_name())),
        },
        other => Err(format!("Job.describe: expected a Job.error, got {}", other.kind_name())),
    });
}

/// A `Job.run`'s op spec, as the effect drain admits it: what to call, or
/// the error it is answered with next frame (a function that isn't
/// top-level, an input that can't cross, no program to compile).
pub(crate) fn spec(program: Option<&JobProgram>, key: String, f: &Value, input: &Value) -> JobSpec {
    let call = match program {
        None => Err(Shareable::error("Job.Unsupported", "Job.run: this host runs no jobs".to_string())),
        Some(program) => program
            .roots
            .name_of(f)
            .map_err(|why| Shareable::error("Job.Failed", format!("Job.run needs a top-level function; this is {why}")))
            .and_then(|name| {
                let input = Shareable::from_value(input)
                    .map_err(|why| Shareable::error("Job.NotShareable", format!("Job.run input: {why}")))?;
                Ok(RunCall { program: program.source.clone(), f: name.to_string(), input })
            }),
    };
    JobSpec::Run(Arc::new(RunSpec { key, call }))
}

/// A `Job.run` the pool couldn't finish (a full queue, a panic).
pub(crate) fn failed(e: JobError) -> Shareable {
    match e {
        JobError::Failed(m) => Shareable::error("Job.Failed", m),
        JobError::Crashed(m) => Shareable::error("Job.Crashed", m),
    }
}

/// One project revision compiled on this thread, with the dependencies of
/// each function a job called, evaluated.
struct Compiled {
    program: u64,
    module: functor_lang::ir::Module,
    sources: functor_lang::project::SourceMap,
    index: CodeIndex,
    prepared: HashMap<String, std::rc::Rc<Prepared>>,
}

/// A session holding `f`'s dependencies, and what evaluating them read.
struct Prepared {
    session: Session,
    reads: Option<Vec<HostInput>>,
}

#[cfg(test)]
thread_local! {
    /// Panic in the next job this thread runs (a test's crash).
    pub(crate) static PANIC_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// The latest revision this thread compiled: a reload's first job
    /// replaces it.
    static COMPILED: RefCell<Option<Compiled>> = const { RefCell::new(None) };
}

/// Run `spec` on this thread: `Result.Ok(output)` or `Result.Error` of a
/// `Job.error`. `reader` serves its file reads; `reads` receives them.
pub(crate) fn run(
    spec: &RunSpec,
    reader: Option<Arc<dyn ProjectReader>>,
    ctx: &JobCtx,
    reads: &ReadSlot,
) -> Shareable {
    let call = match &spec.call {
        Ok(call) => call,
        Err(refused) => return refused.clone(),
    };
    #[cfg(test)]
    if PANIC_NEXT.with(|panic| panic.replace(false)) {
        panic!("a test's panic inside the job");
    }
    let _reader = crate::storage::scope_reader(reader);
    COMPILED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let compiled = compiled(&mut cache, call)?;
        let prepared = compiled.prepare(&call.f, ctx, reads)?;
        let mut host = JobHost;
        host.begin_inputs();
        let out = prepared.session.call_cancellable(
            &call.f,
            vec![call.input.to_value()],
            &mut host,
            ctx.stop_flag(),
        );
        let called = host.end_inputs();
        let _ = reads.set(prepared.reads.clone().zip(called).map(|(mut a, b)| {
            a.extend(b);
            a
        }));
        let value = out.map_err(|e| Shareable::error("Job.Failed", compiled.sources.render(e.span.start, &e.message)))?;
        let value = Shareable::from_value(&value)
            .map_err(|why| Shareable::error("Job.NotShareable", format!("Job.run result: {why}")))?;
        Ok(Shareable::Variant("Result.Ok".into(), vec![value].into()))
    })
    .unwrap_or_else(|e| e)
}

/// `call`'s revision, compiled on this thread (once per revision).
fn compiled<'c>(cache: &'c mut Option<Compiled>, call: &RunCall) -> Result<&'c mut Compiled, Shareable> {
    if cache.as_ref().is_none_or(|c| c.program != call.program.id) {
        *cache = None;
        let started = now_ms();
        let files = call.program.files.iter().map(|(p, s)| (std::path::PathBuf::from(p), s.clone()));
        let project = functor_lang::project::load_sources_with_bundled_modules(
            files.collect(),
            &functor_prelude::bundled_modules(),
        )
        .map_err(|e| Shareable::error("Job.Failed", format!("Job.run: the project doesn't load: {}", e.render())))?;
        log::info!(
            "[functor-lang] job worker compiled the project in {:.1} ms",
            now_ms() - started
        );
        let index = CodeIndex::new(&project.module);
        *cache = Some(Compiled {
            program: call.program.id,
            module: project.module,
            sources: project.sources,
            index,
            prepared: HashMap::new(),
        });
    }
    Ok(cache.as_mut().expect("compiled above"))
}

impl Compiled {
    /// A session with `f`'s dependencies evaluated (once per function, per
    /// revision: a data-file edit reloads, which is a new revision). On a
    /// failure, `reads` gets what evaluating them read, so a reload that
    /// fixes a file runs the job again.
    fn prepare(&mut self, f: &str, ctx: &JobCtx, reads: &ReadSlot) -> Result<std::rc::Rc<Prepared>, Shareable> {
        if let Some(prepared) = self.prepared.get(f) {
            return Ok(prepared.clone());
        }
        let deps = self.index.deps_of(f).ok_or_else(|| {
            Shareable::error("Job.Failed", format!("Job.run: `{f}` is no longer a top-level function"))
        })?;
        let started = now_ms();
        let mut host = JobHost;
        host.begin_inputs();
        let loaded = Session::load_cancellable(&self.module, &|name| deps.contains(name), &mut host, ctx.stop_flag());
        let deps_reads = host.end_inputs();
        let session = loaded.map_err(|failure| {
            let _ = reads.set(deps_reads.clone());
            let message = self.sources.render(failure.error.span.start, &failure.error.message);
            Shareable::error("Job.Failed", format!("Job.run: evaluating what `{f}` uses failed: {message}"))
        })?;
        let reads = deps_reads;
        log::info!(
            "[functor-lang] job worker evaluated `{f}`'s {} dependencies in {:.1} ms",
            deps.len(),
            now_ms() - started
        );
        let prepared = std::rc::Rc::new(Prepared { session, reads });
        self.prepared.insert(f.to_string(), prepared.clone());
        Ok(prepared)
    }
}

/// A measurement: compiling `call`'s revision and evaluating its
/// function's dependencies on this thread, in ms (cold each time).
#[cfg(test)]
pub(crate) fn spin_up(call: &RunCall) -> (f64, f64) {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx = JobCtx::new(stop);
    let mut cache = None;
    let started = now_ms();
    let compiled = compiled(&mut cache, call).ok().expect("compiles");
    let compile = now_ms() - started;
    let started = now_ms();
    compiled.prepare(&call.f, &ctx, &ReadSlot::default()).ok().expect("dependencies evaluate");
    (compile, now_ms() - started)
}

/// Whether every input a job read still reads the same through `reader`.
pub(crate) fn reads_current(reader: Option<Arc<dyn ProjectReader>>, inputs: &[HostInput]) -> bool {
    let _reader = crate::storage::scope_reader(reader);
    FunctorHost.inputs_current(inputs)
}

/// The prelude host, refusing reads job code can't make: physics and UI
/// state belong to the frame, which a worker isn't part of.
struct JobHost;

impl Host for JobHost {
    fn provides(&self, path: &str) -> bool {
        FunctorHost.provides(path)
    }

    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<HostReturn, RunError> {
        let before = read_counts();
        let result = FunctorHost.call(path, args, span);
        let live = ReadClass::ALL
            .into_iter()
            .find(|&class| class != ReadClass::File && read_count(class) != before[class as usize]);
        match live {
            Some(class) => Err(RunError {
                message: format!(
                    "`{path}` reads {class} state, which job code can't: a job runs off the \
frame, like a top-level value (files are fine) — read it in `update` and put it in the input"
                ),
                span,
            }),
            None => result,
        }
    }

    fn begin_inputs(&mut self) {
        FunctorHost.begin_inputs()
    }

    fn end_inputs(&mut self) -> Option<Vec<HostInput>> {
        FunctorHost.end_inputs()
    }

    fn inputs_current(&mut self, inputs: &[HostInput]) -> bool {
        FunctorHost.inputs_current(inputs)
    }
}
