//! Tree-walking interpreter over the core IR — Track B3 of `docs/functor-lang.md`.
//!
//! ## Program semantics
//!
//! [`run`] evaluates a module's top-level defs **eagerly, in file order**,
//! into a global environment. Top-level names are mutually visible (see
//! [`crate::lower`]), but eager evaluation means a *top-level value
//! expression* may only demand a global defined above it — demanding a later
//! one is a "used before its definition" error. Code inside a lambda body
//! resolves globals at **call time** (late binding), so forward references
//! between functions work, and a swapped-in global rebinds every caller —
//! the hot-reload semantics the Functor Lang design picked (docs/functor-lang.md, Milestone 0).
//!
//! If the module defines `main` as a zero-parameter function, [`run`] calls
//! it and reports its result; otherwise the outcome is the list of top-level
//! bindings.
//!
//! ## Externals
//!
//! [`ExprKind::External`] names resolve against the builtin registry at
//! evaluation time ([`builtin`]): `List.map`/`filter`/`fold`/`maximum`,
//! `Map.get`/`insert`/`fromList`, `Text.concat`/`fromFloat`/`toBullets`,
//! `Math.clamp01`. An unregistered external is a spanned runtime error.
//!
//! ## Tracing
//!
//! With [`Tracing::On`], every call (closure or builtin) records an
//! enter/exit event pair with rendered argument and result values — the
//! LLM-readable execution story (`functor-lang trace`). Recording stops (with a
//! marker event) after [`MAX_TRACE_EVENTS`] so a hot loop can't produce an
//! unbounded transcript; evaluation itself continues.

use crate::ir::{
    BindingId, Def, ExpectDef, Expr, ExprKind, Module, Pattern, PatternKind, StringPart,
};
use crate::span::Span;
use crate::value::{canonicalize_map_entries, Closure, Env, MapKey, Value};
use crate::RunError;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;

/// Evaluation depth cap: pathological or unboundedly recursive Functor Lang code must
/// fail as a clean spanned error, not a host stack overflow. Counts *every*
/// nested `eval` entry (expression nesting and calls alike), so it bounds
/// host stack usage directly. Deep iteration belongs in the iterative builtins
/// (`List.map`/`fold`), not user-level recursion.
///
/// INVARIANT: the cap must trip *before* a default 2 MiB test-thread stack is
/// exhausted in a debug build, so `cargo test -p mle` needs no `RUST_MIN_STACK`
/// bump and the infinite-recursion test is a live guard on this budget
/// (`error_infinite_recursion_is_a_clean_error` runs on the default stack; a
/// per-frame regression shows up there as a stack overflow, not silently).
///
/// Measured 2026-07 (debug build, Apple Silicon): the deepest recursion costs
/// ≈11.8 KiB of stack per eval level — 128 levels need ≈1.35 MiB, leaving ~50%
/// frame-growth headroom under 2 MiB. 200 levels had grown past the budget
/// (≈2.2 MiB), aborting the whole test binary on a clean tree (#221).
const MAX_EVAL_DEPTH: usize = 128;

/// Trace-event cap; see the module doc.
pub const MAX_TRACE_EVENTS: usize = 10_000;

/// Recorder caps (see [`Recorder`] / [`Session::call_recorded`]). Distinct
/// *sites* per armed invocation — PER SITE CLASS (binders and references
/// budget separately, so a reference-heavy body can't starve the binder
/// record) — and total *events* (a loop re-hitting one site counts each
/// time). A site-class breach stops NEW sites of that class only; the event
/// breach stops recording entirely. Both flag `truncated`; evaluation itself
/// is never affected.
pub const MAX_RECORDED_SITES: usize = 1024;
pub const MAX_RECORDED_BINDINGS: usize = 100_000;

#[derive(Clone, Copy, PartialEq)]
pub enum Tracing {
    Off,
    On,
}

/// One recorded call boundary. `depth` is the call nesting at the event, for
/// indentation; values are pre-rendered so the trace owns no `Value`s.
pub enum TraceEvent {
    Enter {
        depth: usize,
        callee: String,
        args: Vec<String>,
    },
    Exit {
        depth: usize,
        result: String,
    },
    Truncated,
}

/// What `run` produced: `main`'s result, or (when there is no runnable
/// `main`) every top-level binding in file order.
pub enum RunOutcome {
    Main(Value),
    Bindings(Vec<(String, Value)>),
}

pub struct RunRecord {
    pub outcome: RunOutcome,
    pub trace: Vec<TraceEvent>,
}

/// A failed run: the error **plus the trace recorded up to it** — a failing
/// program is exactly when the execution story matters most, so the partial
/// trace is never discarded (`functor-lang trace` prints it before the diagnostic).
pub struct RunFailure {
    pub error: RunError,
    pub trace: Vec<TraceEvent>,
}

/// The record of one armed entry-point call (see [`Session::call_recorded`]):
/// the call's result and every binding-site value observed while it ran, as
/// pre-rendered `Display` text (the record owns no `Value`s). Framing above a
/// single invocation — entry provenance, ghost, index/count within a frame —
/// is the producer's job; this is one call's raw record.
pub struct RecordedInvocation {
    /// The called top-level name (`update`, `tick`, …).
    pub entry: String,
    /// `Display` of the returned value.
    pub result: String,
    /// Depth-limited one-line preview of the returned value
    /// ([`Value::preview`]) — what an editor shows inline.
    pub result_preview: String,
    /// One entry per distinct binding site reached, in first-seen order.
    pub bindings: Vec<RecordedBinding>,
    /// Sorted span starts of every expression evaluated during the call —
    /// the execution-coverage set (which lines / match arms / branches ran).
    pub coverage: Vec<usize>,
    /// The recorder hit a cap during this call — `bindings` is partial (but
    /// `result` is exact; recording never changes evaluation).
    pub truncated: bool,
}

/// How a recorded value renders in an editor: a `Primitive` is short and
/// complete (its preview IS its full rendering — show inline as-is); a
/// `Composite` has structure (show the depth-limited preview inline and the
/// full `value` on hover). See [`Value::is_primitive`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordedKind {
    Primitive,
    Composite,
}

/// What kind of site observed the value: a `Binder` (a `let` binding, a
/// lambda/function parameter, or a match-pattern variable) or a `Ref` — a
/// READ of a local or module-level name, so every line that *uses* a
/// variable carries its value too. Reference sites skip callable values (a
/// call's callee name is not data worth overlaying).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordedSite {
    Binder,
    Ref,
}

/// One site's last observed value: `Binder` sites (a `let` binding, a
/// lambda/function parameter, a match-pattern variable) plus `Ref` sites
/// (variable reads); nested calls' sites appear here too (flat — spans
/// disambiguate them). `value` is the LAST value seen at the site and
/// `count` how many times it was seen (a loop, e.g. `List.fold`, re-binds
/// and re-reads a site each iteration).
pub struct RecordedBinding {
    pub name: String,
    /// Byte range into the loaded source. NOTE: [`Span`] carries no file — a
    /// multi-file project loads as one concatenated source, so mapping this
    /// range back to (file, offset) is the producer's job (see the visual-
    /// debugger wire contract). For a `let` this is the `let [mut] name =`
    /// region (as `goto`/`hover` report binder names); for a param, match
    /// var, or reference it is the name's own span.
    pub span: Span,
    /// `Display` of the last value seen here.
    pub value: String,
    /// Depth-limited one-line preview of that value ([`Value::preview`]);
    /// equals `value` when `kind` is `Primitive`.
    pub preview: String,
    /// Whether `value` is short/complete or structured — the editor's
    /// inline-vs-hover policy input.
    pub kind: RecordedKind,
    /// Binder or reference — see [`RecordedSite`].
    pub site: RecordedSite,
    pub count: u32,
    /// For NUMERIC sites hit more than once (a loop), the observed range —
    /// the editor renders `= min…max (×N)` instead of just the last value.
    /// `None` for non-numeric values or single hits.
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// A recorder site key. Binder sites key by [`BindingId`] (unique per site
/// in a module; a loop re-enters the same site's id); reference sites key by
/// the reference expression's start offset — unique among references, since
/// no two name reads start at the same byte.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SiteKey {
    Binding(u32),
    Ref(usize),
}

/// The per-call site recorder — armed by [`Session::call_recorded`],
/// mirroring the [`Tracing`] mode: an `Option<Recorder>` on the interpreter,
/// checked cheaply at binding and reference sites only (a `None` test when
/// off). It keeps the last value + hit count per site. On a cap breach it
/// sets `truncated` and stops recording — evaluation continues untouched.
struct Recorder {
    bindings: Vec<RecordedBinding>,
    /// Site key → index into `bindings`.
    index: HashMap<SiteKey, usize>,
    /// Distinct sites so far, per class (see [`MAX_RECORDED_SITES`]).
    binder_sites: usize,
    ref_sites: usize,
    events: usize,
    truncated: bool,
    /// Span starts of every expression evaluated under this recorder — the
    /// execution-coverage set (which lines/arms actually ran). Distinct
    /// starts are static program positions, so the set is bounded by program
    /// size; [`MAX_RECORDED_BINDINGS`] caps it defensively anyway.
    coverage: std::collections::HashSet<usize>,
    /// When false, only coverage collects — the values pass (Display
    /// rendering, site bookkeeping) is skipped entirely. The cheap mode for
    /// replaying a whole window of frames just to know what ran.
    record_values: bool,
}

impl Recorder {
    fn new(record_values: bool) -> Recorder {
        Recorder {
            bindings: Vec::new(),
            index: HashMap::new(),
            binder_sites: 0,
            ref_sites: 0,
            events: 0,
            truncated: false,
            coverage: std::collections::HashSet::new(),
            record_values,
        }
    }

    /// Note an expression evaluation at `start` (its span start).
    fn cover(&mut self, start: usize) {
        if self.coverage.len() < MAX_RECORDED_BINDINGS {
            self.coverage.insert(start);
        }
    }

    /// The sorted coverage set.
    fn coverage_sorted(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.coverage.iter().copied().collect();
        out.sort_unstable();
        out
    }

    /// Record a value observed at a site. The event cap is a hard stop; a
    /// site-class cap only refuses NEW sites of that class — existing sites
    /// keep updating and the other class keeps recording, so a
    /// reference-heavy body can't starve the binder record. A no-op in
    /// coverage-only mode.
    fn record(&mut self, key: SiteKey, site: RecordedSite, name: &str, span: Span, value: &Value) {
        if !self.record_values {
            return;
        }
        if self.events >= MAX_RECORDED_BINDINGS {
            self.truncated = true;
            return;
        }
        self.events += 1;
        if let Some(&idx) = self.index.get(&key) {
            let slot = &mut self.bindings[idx];
            slot.value = value.to_string();
            slot.preview = value.preview();
            slot.kind = recorded_kind(value);
            slot.count += 1;
            // Numeric loop sites fold their range; a non-numeric value at a
            // previously-numeric site (a union-typed binder) drops it.
            if let Value::Number(n) = value {
                slot.min = Some(slot.min.map_or(*n, |m| m.min(*n)));
                slot.max = Some(slot.max.map_or(*n, |m| m.max(*n)));
            } else {
                slot.min = None;
                slot.max = None;
            }
            return;
        }
        let sites = match site {
            RecordedSite::Binder => &mut self.binder_sites,
            RecordedSite::Ref => &mut self.ref_sites,
        };
        if *sites >= MAX_RECORDED_SITES {
            self.truncated = true;
            return;
        }
        *sites += 1;
        self.index.insert(key, self.bindings.len());
        let n = match value {
            Value::Number(n) => Some(*n),
            _ => None,
        };
        self.bindings.push(RecordedBinding {
            name: name.to_string(),
            span,
            value: value.to_string(),
            preview: value.preview(),
            kind: recorded_kind(value),
            site,
            count: 1,
            min: n,
            max: n,
        });
    }
}

fn recorded_kind(value: &Value) -> RecordedKind {
    if value.is_primitive() {
        RecordedKind::Primitive
    } else {
        RecordedKind::Composite
    }
}

/// Host-provided externals: the embedding runtime (e.g. Functor's shells)
/// registers extra `Module.function` names beyond the built-in registry —
/// the capability seam from the Functor Lang design notes (Rust handlers provide what
/// the pure language can't). An [`ExprKind::External`] resolves against the
/// builtin registry first, then the host; host calls appear in the trace
/// like any other call. See `functor_runtime_common::functor_lang_prelude` for the
/// first real host.
pub trait Host {
    /// Does this host provide `path` (a joined qualified name, `Scene.cube`)?
    fn provides(&self, path: &str) -> bool;

    /// Perform a host call. Only invoked for paths `provides` accepted.
    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<Value, RunError>;
}

/// The hostless host: no externals beyond the builtin registry.
pub struct NoHost;

impl Host for NoHost {
    fn provides(&self, _path: &str) -> bool {
        false
    }

    fn call(&mut self, path: &str, _args: Vec<Value>, span: Span) -> Result<Value, RunError> {
        Err(RunError {
            message: format!("internal: NoHost cannot call `{path}`"),
            span,
        })
    }
}

/// Evaluate a lowered module (see the module doc for semantics).
pub fn run(module: &Module, tracing: Tracing) -> Result<RunRecord, RunFailure> {
    run_with_host(module, tracing, &mut NoHost)
}

/// [`run`], with host-provided externals (see [`Host`]).
pub fn run_with_host(
    module: &Module,
    tracing: Tracing,
    host: &mut dyn Host,
) -> Result<RunRecord, RunFailure> {
    let mut interp = Interp {
        globals: HashMap::new(),
        mut_slots: HashMap::new(),
        trace: Vec::new(),
        tracing,
        recorder: None,
        depth: 0,
        call_depth: 0,
        fuel: None,
        brand_ops: BrandOps::default(),
        host,
    };
    match interp.run_module(module) {
        Ok(outcome) => Ok(RunRecord {
            outcome,
            trace: interp.trace,
        }),
        Err(error) => Err(RunFailure {
            error,
            trace: interp.trace,
        }),
    }
}

/// The result of one `expect` test (see [`run_expects`]). The span is the
/// test's identity — there are no names; render `file:line` via the project
/// source map.
#[derive(Debug)]
pub struct ExpectReport {
    pub span: Span,
    pub outcome: ExpectOutcome,
}

#[derive(Debug)]
pub enum ExpectOutcome {
    Pass,
    /// Evaluated to `false`. When the expression's top level is a comparison,
    /// both sides' rendered values are carried for the report.
    Fail(Option<FailedCompare>),
    /// Evaluation failed (a spanned runtime error), or the expression did not
    /// produce a bool (unchecked code — `check` catches this statically).
    Error(RunError),
}

impl ExpectOutcome {
    /// The tooling-facing `(state, detail)` — ONE shared mapping for every
    /// editor gutter (the LSP and the web IDE): `pass` / `fail` /
    /// `unrunnable` (an engine-external call — the plain evaluator has no
    /// host) / `error`. An `Error`'s detail is the bare message; callers
    /// holding a `SourceMap` may prefer a located rendering.
    pub fn status(&self) -> (&'static str, Option<String>) {
        match self {
            ExpectOutcome::Pass => ("pass", None),
            ExpectOutcome::Fail(Some(cmp)) => (
                "fail",
                Some(format!(
                    "left {} right — left: {}, right: {}",
                    cmp.op, cmp.lhs, cmp.rhs
                )),
            ),
            ExpectOutcome::Fail(None) => ("fail", Some("expected true, got false".to_string())),
            ExpectOutcome::Error(error) if error.message.starts_with("unknown external") => (
                "unrunnable",
                Some(format!(
                    "{} — engine calls need the runtime; run `functor test` or the game",
                    error.message
                )),
            ),
            ExpectOutcome::Error(error) => ("error", Some(error.message.clone())),
        }
    }
}

/// The sides of a failed top-level comparison: `expect a == b` reports what
/// `a` and `b` actually were (rendered with [`Value`]'s deterministic
/// `Display`).
#[derive(Debug)]
pub struct FailedCompare {
    pub op: &'static str,
    pub lhs: String,
    pub rhs: String,
}

/// Evaluate a module's `expect` tests: top-level defs load first (exactly as
/// [`Session::load`] — initializers run, `main` is not called), then each
/// expect evaluates independently over the loaded globals. A def-load failure
/// is the returned `Err`; a failing or erroring expect is its own report and
/// never stops the remaining tests. (One `Interp` is shared across expects —
/// independence holds because `depth`/`call_depth` unwind on error, globals
/// are read-only after the load, and `mut_slots` entries are keyed by
/// per-expect-unique `BindingId`s, so a leaked slot is unreachable.)
pub fn run_expects(
    module: &Module,
    host: &mut dyn Host,
) -> Result<Vec<ExpectReport>, RunFailure> {
    run_expects_budgeted(module, host, None)
}

/// [`run_expects`] with an optional step budget — the live-tooling seam: an
/// editor evaluating on every (debounced) edit must never hang on a runaway
/// expect. `budget` is the max interpreter steps for EACH phase — the def
/// load, then every expect independently (the budget RESETS between expects,
/// so one runaway is blamed precisely and cannot starve the rest). Exceeding
/// it is a spanned [`ExpectOutcome::Error`] (or the returned `Err` when the
/// def load itself exceeds it). A step is one function call, or one
/// element/byte/entry a bulk list/text/map builtin materializes, scans, or
/// compares (see [`Fuel`] — growth builtins charge their OUTPUT, closing the
/// doubling-amplifier hole) — the honest runaway proxy for a loop-free
/// language, chosen over per-eval-node charging to keep the frame loop's
/// hot path branch-free. Total interpreter work is O(budget), so the budget
/// bounds wall-clock up to the per-step constant; pick it accordingly
/// (~10^6 is comfortably sub-second).
///
/// STACK CONTRACT for in-process embedding (the LSP / wasm IDE seam): a
/// budgeted run cannot build a value nested deeper than `budget` levels (each
/// level costs ≥ 1 step), but DROPPING a value that deep recurses to its
/// depth (`Value` deliberately has no iterative Drop — one was measured at
/// ~2x frame_bench). Run this on a worker thread whose stack covers the
/// budget — ~100 bytes per level, so `budget * 100` bytes reserved (e.g.
/// `std::thread::Builder::stack_size(256 << 20)` comfortably covers a 10^6
/// budget; reserved virtual stack commits lazily). Rendering and comparison
/// are depth-safe regardless (`Display`/`value_eq` are iterative).
pub fn run_expects_budgeted(
    module: &Module,
    host: &mut dyn Host,
    budget: Option<u64>,
) -> Result<Vec<ExpectReport>, RunFailure> {
    let mut interp = Interp {
        globals: HashMap::new(),
        mut_slots: HashMap::new(),
        trace: Vec::new(),
        tracing: Tracing::Off,
        recorder: None,
        depth: 0,
        call_depth: 0,
        fuel: budget.map(Fuel::new),
        brand_ops: BrandOps::default(),
        host,
    };
    if let Err(error) = interp
        .eval_defs(module)
        .and_then(|_| interp.load_brand_ops(module).map(|ops| interp.brand_ops = ops))
    {
        return Err(RunFailure {
            error,
            trace: interp.trace,
        });
    }
    Ok(module
        .expects
        .iter()
        .map(|expect| {
            interp.fuel = budget.map(Fuel::new);
            ExpectReport {
                span: expect.span,
                outcome: interp.eval_expect(expect),
            }
        })
        .collect())
}

/// A persistent interpreter session for embedding (the C2 producer): load a
/// module once, then call top-level functions per frame. Globals are
/// evaluated at load; each `call` runs with a fresh interpreter over the
/// session's globals (Rc-cheap clones), so per-frame state lives entirely in
/// the VALUES passed in and returned — the model stays data (hot-reload can
/// swap the session and keep the model; docs/functor-lang.md C3).
pub struct Session {
    globals: HashMap<String, Value>,
    brand_ops: BrandOps,
}

/// Declared unit operators, resolved to callable VALUES and keyed by the
/// runtime tag of the brand they act on ([`brand_tag`]) — one row of four
/// slots, `+` `-` `*` `/`, so dispatch is a BORROWED lookup that allocates
/// nothing on the operand path.
///
/// The checker resolves `90deg + 45deg` statically, but the interpreter must
/// dispatch too: `check` is not on every path (`functor-lang run` skips it),
/// and a value can reach an operator through a gradual `unknown` seam. Both
/// roads lead to exactly the call the declaration names — under a fake or
/// plain prelude the behaviour is the handwritten call's, precisely.
pub(crate) type BrandOps = Rc<HashMap<String, BrandOpRow>>;

/// One brand's declared operators, indexed by [`op_slot`].
pub(crate) type BrandOpRow = [Option<Value>; 4];

/// Which slot of a [`BrandOpRow`] an operator owns — `None` for the
/// comparisons, which are not declarable.
fn op_slot(op: crate::ast::BinOp) -> Option<usize> {
    use crate::ast::BinOp;
    match op {
        BinOp::Add => Some(0),
        BinOp::Sub => Some(1),
        BinOp::Mul => Some(2),
        BinOp::Div => Some(3),
        _ => None,
    }
}

/// The operator a [`BrandOpRow`] slot holds — the inverse of [`op_slot`],
/// for the "declares `+`, `*`, but not `-`" teaching error.
fn slot_op(slot: usize) -> crate::ast::BinOp {
    use crate::ast::BinOp;
    [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div][slot]
}

impl Session {
    /// Evaluate a module's top-level defs (eagerly, in file order) into a
    /// session. Unlike [`run`], a `main` def is NOT called — loading a game
    /// must not execute anything beyond its initializers.
    pub fn load(module: &Module, host: &mut dyn Host) -> Result<Session, RunFailure> {
        let mut interp = Interp {
            globals: HashMap::new(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            recorder: None,
            depth: 0,
            call_depth: 0,
            fuel: None,
            brand_ops: BrandOps::default(),
            host,
        };
        let loaded = interp
            .eval_defs(module)
            .and_then(|_| interp.load_brand_ops(module));
        match loaded {
            Ok(brand_ops) => Ok(Session {
                globals: interp.globals,
                brand_ops,
            }),
            Err(error) => Err(RunFailure {
                error,
                trace: interp.trace,
            }),
        }
    }

    /// The value of a top-level def, if any.
    pub fn global(&self, name: &str) -> Option<Value> {
        self.globals.get(name).cloned()
    }

    /// Call the top-level function `name` with `args`. `span` 0..0 is used
    /// for errors with no better location (the caller is not Functor Lang code).
    pub fn call(
        &self,
        name: &str,
        args: Vec<Value>,
        host: &mut dyn Host,
    ) -> Result<Value, RunError> {
        let callee = self.globals.get(name).cloned().ok_or_else(|| RunError {
            message: format!("no top-level `let {name}` in the module"),
            span: Span::new(0, 0),
        })?;
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            recorder: None,
            depth: 0,
            call_depth: 0,
            fuel: None,
            brand_ops: self.brand_ops.clone(),
            host,
        };
        interp.call(callee, args, name.to_string(), Span::new(0, 0), None)
    }

    /// Apply a function VALUE (a closure the host is holding — e.g. an
    /// effect's tagger) with `args`. Same execution shape as [`Self::call`];
    /// the label names it in traces/errors.
    pub fn apply(
        &self,
        callee: Value,
        args: Vec<Value>,
        label: &str,
        host: &mut dyn Host,
    ) -> Result<Value, RunError> {
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            recorder: None,
            depth: 0,
            call_depth: 0,
            fuel: None,
            brand_ops: self.brand_ops.clone(),
            host,
        };
        interp.call(callee, args, label.to_string(), Span::new(0, 0), None)
    }

    /// Like [`Self::call`], but with the binding-site recorder armed: returns
    /// the call's result plus a [`RecordedInvocation`] of every `let` /
    /// parameter / match-variable value bound while it ran (including nested
    /// user calls). Recording is bounded (see [`MAX_RECORDED_SITES`] /
    /// [`MAX_RECORDED_BINDINGS`]) and never changes evaluation — the result is
    /// identical to [`Self::call`]; only observation is added. The seam the
    /// paused visual-debugger replays entry points through.
    pub fn call_recorded(
        &self,
        name: &str,
        args: Vec<Value>,
        host: &mut dyn Host,
    ) -> Result<(Value, RecordedInvocation), RunError> {
        let callee = self.globals.get(name).cloned().ok_or_else(|| RunError {
            message: format!("no top-level `let {name}` in the module"),
            span: Span::new(0, 0),
        })?;
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            recorder: Some(Recorder::new(true)),
            depth: 0,
            call_depth: 0,
            fuel: None,
            brand_ops: self.brand_ops.clone(),
            host,
        };
        let result = interp.call(callee, args, name.to_string(), Span::new(0, 0), None)?;
        let recorder = interp.recorder.expect("armed above");
        let coverage = recorder.coverage_sorted();
        let invocation = RecordedInvocation {
            entry: name.to_string(),
            result: result.to_string(),
            result_preview: result.preview(),
            bindings: recorder.bindings,
            coverage,
            truncated: recorder.truncated,
        };
        Ok((result, invocation))
    }

    /// Like [`Self::call_recorded`] but COVERAGE-ONLY: returns the sorted
    /// span starts of every expression the call evaluated, skipping the
    /// values pass entirely (no Display rendering, no site bookkeeping) —
    /// the cheap mode for replaying a whole window of frames just to learn
    /// which lines/arms ran.
    pub fn call_covered(
        &self,
        name: &str,
        args: Vec<Value>,
        host: &mut dyn Host,
    ) -> Result<(Value, Vec<usize>), RunError> {
        let callee = self.globals.get(name).cloned().ok_or_else(|| RunError {
            message: format!("no top-level `let {name}` in the module"),
            span: Span::new(0, 0),
        })?;
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            recorder: Some(Recorder::new(false)),
            depth: 0,
            call_depth: 0,
            fuel: None,
            brand_ops: self.brand_ops.clone(),
            host,
        };
        let result = interp.call(callee, args, name.to_string(), Span::new(0, 0), None)?;
        let recorder = interp.recorder.expect("armed above");
        Ok((result, recorder.coverage_sorted()))
    }
}

/// A step budget for bounded evaluation ([`run_expects_budgeted`] — the live
/// tooling seam: an editor evaluating on every edit must not hang on a
/// runaway expect). A step is one function CALL (closures, builtins, ctors —
/// [`Interp::call`]), one container CONSTRUCTED (list/tuple/record literals
/// and updates — what makes value depth ≤ budget, the stack contract's
/// arithmetic), or one element/byte a bulk builtin materializes or scans
/// (`List.range`'s count; `List.append`/`flatten` output elements;
/// `Text.concat`/`join`/`toBullets` output bytes; `reverse`/`maximum`/
/// `split` input size; Map key comparisons, copied entries, and materialized
/// iteration output). Charging GROWTH builtins by output is what makes
/// the budget honest — `(x) => List.append(x, x)` doubles per call, so a
/// per-call-only charge would allow exponential work under a linear budget.
/// The general per-node eval path is deliberately uncharged so the frame
/// loop stays branch-free. `limit` is kept for the error message.
#[derive(Clone, Copy)]
struct Fuel {
    remaining: u64,
    limit: u64,
}

impl Fuel {
    fn new(limit: u64) -> Fuel {
        Fuel {
            remaining: limit,
            limit,
        }
    }
}

fn step_budget_error(limit: u64, span: Span) -> RunError {
    RunError {
        message: format!(
            "evaluation exceeded its step budget ({limit} steps) — the embedding \
tool bounds test evaluation (a live editor re-runs on every edit); look for runaway recursion \
or an oversized computation"
        ),
        span,
    }
}

/// A formatter sink that refuses to materialize more bytes than the current
/// tooling budget permits. `Value::Display` is iterative, so this also keeps
/// rendering a shared, structurally large value bounded by the same fuel that
/// bounds its construction.
struct FuelWriter<'a> {
    out: &'a mut String,
    remaining: u64,
}

impl std::fmt::Write for FuelWriter<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let bytes = u64::try_from(text.len()).map_err(|_| std::fmt::Error)?;
        self.remaining = self.remaining.checked_sub(bytes).ok_or(std::fmt::Error)?;
        self.out.push_str(text);
        Ok(())
    }
}

struct Interp<'h> {
    globals: HashMap<String, Value>,
    /// Live `let mut` slots, keyed by binding, as a stack per binding (the
    /// same binding can be re-entered through indirect recursion). Lowering
    /// guarantees a slot is only touched within its `let`'s dynamic extent —
    /// mut bindings cannot be captured — so push/pop brackets are exact.
    mut_slots: HashMap<u32, Vec<Value>>,
    trace: Vec<TraceEvent>,
    tracing: Tracing,
    /// The binding-site recorder, armed only by [`Session::call_recorded`];
    /// `None` (the norm) means zero recording cost on the hot path.
    recorder: Option<Recorder>,
    depth: usize,
    call_depth: usize,
    /// The step budget, set only by [`run_expects_budgeted`]; `None` (the
    /// norm — every game-loop path) means zero cost beyond one branch per
    /// eval step, the same hot-path cost class as `recorder`.
    fuel: Option<Fuel>,
    /// Declared unit operators, keyed by an operand's runtime brand tag
    /// (see [`brand_tag`]). Empty for every program that declares none, and
    /// consulted ONLY when an arithmetic operand is not a number — so plain
    /// float math costs exactly what it did before.
    brand_ops: BrandOps,
    host: &'h mut dyn Host,
}

impl Interp<'_> {
    /// Evaluate every top-level def, eagerly, in file order (no `main` call —
    /// that is [`Interp::run_module`]'s CLI behavior, not a load semantic).
    fn eval_defs(&mut self, module: &Module) -> Result<Vec<(String, Value)>, RunError> {
        let mut bindings = Vec::new();
        for def in &module.defs {
            let value = self.eval(&def.value, &Env::empty())?;
            self.globals.insert(def.name.clone(), value.clone());
            bindings.push((def.name.clone(), value));
        }
        Ok(bindings)
    }

    /// Evaluate one `expect`. A top-level comparison is decomposed — the two
    /// sides evaluate separately — so a failure reports both actual values.
    fn eval_expect(&mut self, expect: &ExpectDef) -> ExpectOutcome {
        use crate::ast::BinOp;
        let env = Env::empty();
        if let ExprKind::Binary {
            op:
                op @ (BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge),
            lhs,
            rhs,
        } = &expect.expr.kind
        {
            let l = match self.eval(lhs, &env) {
                Ok(v) => v,
                Err(e) => return ExpectOutcome::Error(e),
            };
            let r = match self.eval(rhs, &env) {
                Ok(v) => v,
                Err(e) => return ExpectOutcome::Error(e),
            };
            return match self.binary_op(*op, l.clone(), r.clone(), expect.expr.span) {
                Ok(Value::Bool(true)) => ExpectOutcome::Pass,
                // Comparisons only produce bools, so anything else is false.
                Ok(_) => ExpectOutcome::Fail(Some(FailedCompare {
                    op: op.symbol(),
                    lhs: l.to_string(),
                    rhs: r.to_string(),
                })),
                // e.g. `==` on functions — a runtime error, not a plain fail.
                Err(e) => ExpectOutcome::Error(e),
            };
        }
        match self.eval(&expect.expr, &env) {
            Ok(Value::Bool(true)) => ExpectOutcome::Pass,
            Ok(Value::Bool(false)) => ExpectOutcome::Fail(None),
            Ok(other) => ExpectOutcome::Error(RunError {
                message: format!(
                    "an `expect` must evaluate to a bool, got {other} — write a comparison \
(`expect actual == expected`)"
                ),
                span: expect.expr.span,
            }),
            Err(e) => ExpectOutcome::Error(e),
        }
    }

    fn run_module(&mut self, module: &Module) -> Result<RunOutcome, RunError> {
        let bindings = self.eval_defs(module)?;
        self.brand_ops = self.load_brand_ops(module)?;
        match module.defs.iter().find(|def| def.name == "main") {
            Some(main_def) => Ok(RunOutcome::Main(self.call_main(main_def)?)),
            None => Ok(RunOutcome::Bindings(bindings)),
        }
    }

    /// Resolve every `unit <suffix> (<op>) = <impl>` declaration into the
    /// dispatch table, once, after the module's defs exist.
    ///
    /// A brand's runtime identity is taken from a value it actually BUILDS:
    /// the suffix's own constructor is applied to `1.0` and the result's tag
    /// ([`brand_tag`]) becomes the key. That works for every unit target
    /// shape — a host function (`Angle.degrees`), a variant constructor
    /// (`Px`), or an ordinary Functor Lang function — with no type
    /// information, which the interpreter does not have.
    fn load_brand_ops(&mut self, module: &Module) -> Result<BrandOps, RunError> {
        if module.unit_ops.is_empty() {
            return Ok(BrandOps::default());
        }
        let mut tags: HashMap<&str, String> = HashMap::new();
        let mut ops: HashMap<String, BrandOpRow> = HashMap::new();
        for unit_op in &module.unit_ops {
            let tag = match tags.get(unit_op.suffix.as_str()) {
                Some(tag) => tag.clone(),
                None => {
                    let Some(unit) = module
                        .units
                        .iter()
                        .find(|unit| unit.suffix == unit_op.suffix)
                    else {
                        // Lowering refuses an operator on an undeclared
                        // suffix, so this is unreachable in a loaded project.
                        continue;
                    };
                    let ctor = self.eval(&unit.target, &Env::empty())?;
                    let probe = self.call(
                        ctor,
                        vec![Value::Number(1.0)],
                        unit.suffix.clone(),
                        unit_op.span,
                        None,
                    )?;
                    let Some(tag) = brand_tag(&probe).map(str::to_string) else {
                        return Err(RunError {
                            message: format!(
                                "`unit {} ({})` needs a unit whose values carry a runtime tag — \
`{}` builds {}, which nothing can dispatch on (use a constructor, or a host value)",
                                unit_op.suffix,
                                unit_op.op.symbol(),
                                unit_op.suffix,
                                probe.kind_name()
                            ),
                            span: unit_op.span,
                        });
                    };
                    tags.insert(unit_op.suffix.as_str(), tag.clone());
                    tag
                }
            };
            let implementation = self.eval(&unit_op.target, &Env::empty())?;
            if let Some(slot) = op_slot(unit_op.op) {
                ops.entry(tag).or_default()[slot] = Some(implementation);
            }
        }
        Ok(Rc::new(ops))
    }

    fn call_main(&mut self, def: &Def) -> Result<Value, RunError> {
        let main = self.globals.get("main").expect("just defined").clone();
        match &main {
            Value::Closure(closure) if closure.params.is_empty() => {
                self.call(main.clone(), vec![], "main".to_string(), def.span, None)
            }
            // Builtins, host functions, and unapplied constructors take
            // arguments by definition, so they get the same error as a
            // parameterful closure rather than printing as a value.
            Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_) | Value::Ctor { .. } => {
                Err(RunError {
                    message: "`main` must take no parameters to be runnable".to_string(),
                    span: def.span,
                })
            }
            // A non-function `main` is just a value; report it directly.
            _ => Ok(main),
        }
    }

    /// Consume `units` of the step budget, when one is set — a cheap `None`
    /// test otherwise, and never on the per-eval-node path. The language has
    /// no loops, so any unbounded computation must recurse or iterate
    /// through builtin closure calls — both pass through [`Self::call`] —
    /// or move bulk data through a list/text builtin, which charges the
    /// size it materializes or scans (see [`Fuel`]).
    fn charge(&mut self, units: u64, span: Span) -> Result<(), RunError> {
        if let Some(fuel) = &mut self.fuel {
            match fuel.remaining.checked_sub(units) {
                Some(rest) => fuel.remaining = rest,
                None => return Err(step_budget_error(fuel.limit, span)),
            }
        }
        Ok(())
    }

    /// Refuse work whose conservative upper bound cannot fit, without
    /// consuming that bound. The operation charges its actual work afterward,
    /// so successful realistic inputs are not double-charged.
    fn preflight_charge(&self, units: u64, span: Span) -> Result<(), RunError> {
        if let Some(fuel) = self.fuel {
            if fuel.remaining < units {
                return Err(step_budget_error(fuel.limit, span));
            }
        }
        Ok(())
    }

    /// Binary-search a canonical Map vector, charging before each key
    /// comparison. Precharging matters for strings: their shared prefix can
    /// be arbitrarily long, so charging only after `str::cmp` would let the
    /// comparison itself escape the evaluator's bound.
    fn map_search(
        &mut self,
        entries: &[(MapKey, Value)],
        key: &MapKey,
        span: Span,
    ) -> Result<Result<usize, usize>, RunError> {
        let mut left = 0usize;
        let mut right = entries.len();
        while left < right {
            let mid = left + (right - left) / 2;
            let candidate = &entries[mid].0;
            self.charge(candidate.comparison_units(key), span)?;
            match candidate.compare(key) {
                Ordering::Less => left = mid + 1,
                Ordering::Greater => right = mid,
                Ordering::Equal => return Ok(Ok(mid)),
            }
        }
        Ok(Err(left))
    }

    fn eval(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            self.depth -= 1;
            return Err(RunError {
                message: format!(
                    "evaluation nested too deeply (exceeded the depth cap of {MAX_EVAL_DEPTH}): \
deep recursion, or deeply nested expressions. Deep list iteration belongs in the builtin list \
functions (List.fold/map/filter/any/all/length/…), which loop in the interpreter and consume no \
evaluation depth."
                ),
                span: expr.span,
            });
        }
        // Coverage: one armed-check per evaluation (a `None` test when off —
        // the same hot-path cost class as the binding-site hook).
        if let Some(recorder) = &mut self.recorder {
            recorder.cover(expr.span.start);
        }
        let result = self.eval_inner(expr, env);
        self.depth -= 1;
        result
    }

    /// Record a value bound at `binding`'s site, when the recorder is armed —
    /// a cheap `None` check otherwise (the binding-site hot-path cost). See
    /// [`Recorder`].
    fn record_binding(&mut self, binding: BindingId, name: &str, span: Span, value: &Value) {
        if let Some(recorder) = &mut self.recorder {
            recorder.record(SiteKey::Binding(binding.0), RecordedSite::Binder, name, span, value);
        }
    }

    /// Record a value READ at a reference site (a `Local`/`Global` name use),
    /// when the recorder is armed — the same cheap `None` check otherwise.
    /// Callable values are skipped: a call's callee name (`helper(m)`,
    /// `xs |> sum`) is not data worth overlaying, and skipping it halves the
    /// noise at the source.
    fn record_ref(&mut self, name: &str, span: Span, value: &Value) {
        if let Some(recorder) = &mut self.recorder {
            if matches!(
                value,
                Value::Closure(_)
                    | Value::Ctor { .. }
                    | Value::Partial(_)
                    | Value::Builtin(_)
                    | Value::HostFn(_)
            ) {
                return;
            }
            recorder.record(SiteKey::Ref(span.start), RecordedSite::Ref, name, span, value);
        }
    }

    fn eval_interpolated(
        &mut self,
        parts: &[StringPart],
        env: &Env,
        span: Span,
    ) -> Result<Value, RunError> {
        let mut out = String::new();
        for part in parts {
            match part {
                StringPart::Text(text) => {
                    self.charge(text.len() as u64, span)?;
                    out.push_str(text);
                }
                StringPart::Expr(part) => match self.eval(part, env)? {
                    Value::String(text) => {
                        self.charge(text.len() as u64, span)?;
                        out.push_str(&text);
                    }
                    value => {
                        self.append_interpolated_value(&mut out, &value, span)?;
                    }
                },
            }
        }
        Ok(Value::String(Rc::from(out)))
    }

    fn append_interpolated_value(
        &mut self,
        out: &mut String,
        value: &Value,
        span: Span,
    ) -> Result<(), RunError> {
        let Some(fuel) = self.fuel else {
            // Writing directly avoids the extra allocation that `to_string`
            // would impose on every unbudgeted game-loop interpolation.
            write!(out, "{value}").expect("writing to a String cannot fail");
            return Ok(());
        };
        let mut writer = FuelWriter {
            out,
            remaining: fuel.remaining,
        };
        if write!(&mut writer, "{value}").is_err() {
            return Err(step_budget_error(fuel.limit, span));
        }
        self.fuel.as_mut().expect("fuel was present").remaining = writer.remaining;
        Ok(())
    }

    fn eval_inner(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        match &expr.kind {
            ExprKind::Number(n) => Ok(Value::Number(*n)),
            ExprKind::String(s) => Ok(Value::String(Rc::from(s.as_str()))),
            ExprKind::InterpolatedString(parts) => self.eval_interpolated(parts, env, expr.span),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Local { binding, name } => {
                let value = env.lookup(*binding).ok_or_else(|| RunError {
                    // Unreachable if lowering is correct; fail loud rather than UB.
                    message: format!("internal: unbound local `{name}`"),
                    span: expr.span,
                })?;
                self.record_ref(name, expr.span, &value);
                Ok(value)
            }
            ExprKind::Global(name) => {
                let value = self.globals.get(name).cloned().ok_or_else(|| RunError {
                    message: format!("global `{name}` used before its definition"),
                    span: expr.span,
                })?;
                self.record_ref(name, expr.span, &value);
                Ok(value)
            }
            ExprKind::External(path) => {
                let joined = path.join(".");
                match builtin(path) {
                    // `Math.pi` is a constant, not a callable — resolve it
                    // straight to its value (every other builtin is a function).
                    Some(Builtin::MathPi) => Ok(Value::Number(std::f64::consts::PI)),
                    Some(b) => Ok(Value::Builtin(b)),
                    None if self.host.provides(&joined) => {
                        Ok(Value::HostFn(Rc::from(joined.as_str())))
                    }
                    // `#[cold]` helper: the message formatting must not
                    // enlarge this hot frame (eval recursion sits near the
                    // stack budget at the 128-depth cap — the call_curried
                    // rule; adding the formatting inline here overflowed the
                    // deep-recursion test's 2MB stack in debug).
                    None => Err(unknown_external_error(path, joined, expr.span)),
                }
            }
            // Container CONSTRUCTION charges one step (the depth invariant
            // behind run_expects_budgeted's stack contract: every level of a
            // value's nesting was constructed once, so value depth ≤ budget.
            // Without this, a nested literal in a fold body builds up to
            // MAX_EVAL_DEPTH levels per single call charge). `fuel: None`
            // (every game-loop path) makes each a one-branch no-op.
            ExprKind::Record(fields) => {
                self.charge(1, expr.span)?;
                let mut out = Vec::with_capacity(fields.len());
                for field in fields {
                    out.push((field.name.clone(), self.eval(&field.value, env)?));
                }
                Ok(Value::Record(Rc::new(out)))
            }
            ExprKind::Tuple(items) => {
                self.charge(1, expr.span)?;
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval(item, env)?);
                }
                Ok(Value::Tuple(Rc::new(values)))
            }
            ExprKind::List(items) => {
                self.charge(1, expr.span)?;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                Ok(Value::List(Rc::new(out)))
            }
            ExprKind::ListCons { items, tail } => {
                self.charge(1, expr.span)?;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                match self.eval(tail, env)? {
                    Value::List(rest) => {
                        out.extend(rest.iter().cloned());
                        Ok(Value::List(Rc::new(out)))
                    }
                    other => Err(RunError {
                        message: format!(
                            "`..` spreads a list, but the tail is {}",
                            other.kind_name()
                        ),
                        span: tail.span,
                    }),
                }
            }
            ExprKind::RecordUpdate { base, fields } => {
                self.charge(1, expr.span)?;
                let base_value = self.eval(base, env)?;
                let Value::Record(base_fields) = &base_value else {
                    return Err(RunError {
                        message: format!(
                            "`with` update on {}, not a record",
                            base_value.kind_name()
                        ),
                        span: expr.span,
                    });
                };
                let mut out = base_fields.as_ref().clone();
                for field in fields {
                    // Validate the target BEFORE evaluating the replacement:
                    // with host externals the RHS can have effects, and an
                    // invalid update must reject without running them.
                    if !out.iter().any(|(name, _)| *name == field.name) {
                        return Err(RunError {
                            message: format!("record has no field `{}` to update", field.name),
                            span: field.span,
                        });
                    }
                    let value = self.eval(&field.value, env)?;
                    let slot = out
                        .iter_mut()
                        .find(|(name, _)| *name == field.name)
                        .expect("checked above");
                    slot.1 = value;
                }
                Ok(Value::Record(Rc::new(out)))
            }
            ExprKind::LocalMut { binding, name } => {
                let value = self
                    .mut_slots
                    .get(&binding.0)
                    .and_then(|stack| stack.last())
                    .cloned()
                    .ok_or_else(|| RunError {
                        // Unreachable if lowering is correct; fail loud.
                        message: format!("internal: dead mut slot `{name}`"),
                        span: expr.span,
                    })?;
                self.record_ref(name, expr.span, &value);
                Ok(value)
            }
            ExprKind::Let {
                binding,
                name,
                mutable,
                value,
                body,
                ..
            } => {
                // The `let [mut] name =` region, as goto/hover report binders.
                let binder_span = Span::new(expr.span.start, value.span.start);
                let value = self.eval(value, env)?;
                self.record_binding(*binding, name, binder_span, &value);
                if *mutable {
                    self.mut_slots.entry(binding.0).or_default().push(value);
                    let result = self.eval(body, env);
                    self.mut_slots
                        .get_mut(&binding.0)
                        .expect("pushed above")
                        .pop();
                    result
                } else {
                    let child = env.child(vec![(*binding, value)]);
                    self.eval(body, &child)
                }
            }
            ExprKind::Assign {
                binding,
                name,
                value,
                rest,
            } => {
                let value = self.eval(value, env)?;
                match self
                    .mut_slots
                    .get_mut(&binding.0)
                    .and_then(|stack| stack.last_mut())
                {
                    Some(slot) => *slot = value,
                    None => {
                        return Err(RunError {
                            message: format!("internal: dead mut slot `{name}`"),
                            span: expr.span,
                        })
                    }
                }
                self.eval(rest, env)
            }
            ExprKind::FieldAccess { object, field } => {
                let object_value = self.eval(object, env)?;
                match &object_value {
                    Value::Record(fields) => fields
                        .iter()
                        .find(|(name, _)| name == field)
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| RunError {
                            message: format!("record has no field `{field}`"),
                            span: expr.span,
                        }),
                    other => Err(RunError {
                        message: format!("`.{field}` on {}, not a record", other.kind_name()),
                        span: expr.span,
                    }),
                }
            }
            ExprKind::Lambda { params, body, .. } => Ok(Value::Closure(Rc::new(Closure {
                params: params.clone(),
                body: body.clone(),
                env: env.clone(),
                expr_id: expr.id,
            }))),
            ExprKind::Call { callee, args } => {
                let callee_value = self.eval(callee, env)?;
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.eval(arg, env)?);
                }
                self.call(
                    callee_value,
                    arg_values,
                    callee_label(callee),
                    expr.span,
                    None,
                )
            }
            ExprKind::Binary { .. } => {
                // Left-assoc chains (`a + b + c + …`) nest down the lhs, and
                // the parser builds them iteratively (no depth guard), so
                // evaluate the lhs spine iteratively too — a flat 500-term sum
                // must not consume eval depth (or host stack) per term.
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ExprKind::Binary { op, lhs, rhs } = &leaf.kind {
                    spine.push((*op, rhs, leaf.span));
                    leaf = lhs;
                }
                let mut acc = self.eval(leaf, env)?;
                for (op, rhs, span) in spine.into_iter().rev() {
                    let rhs_value = self.eval(rhs, env)?;
                    acc = self.binary_op(op, acc, rhs_value, span)?;
                }
                Ok(acc)
            }
            // Short-circuit: an operand is evaluated only when the accumulator
            // so far doesn't already decide the result (`false && _` is false,
            // `true || _` is true). Both operands must be bools. Left-assoc
            // chains nest down the lhs and the parser builds them iteratively
            // (no depth guard), so walk the spine iteratively too — like the
            // Binary arm above, a flat `a && b && …` chain must not consume
            // host stack per term. Each spine node carries its own op, so a
            // mixed `a && b || c` folds correctly.
            ExprKind::Logical { .. } => {
                use crate::ast::LogicalOp;
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ExprKind::Logical { op, lhs, rhs } = &leaf.kind {
                    spine.push((*op, rhs.as_ref()));
                    leaf = lhs;
                }
                let mut acc = as_bool(&self.eval(leaf, env)?, leaf.span)?;
                for (op, rhs) in spine.into_iter().rev() {
                    let decided = match op {
                        LogicalOp::And => !acc,
                        LogicalOp::Or => acc,
                    };
                    if !decided {
                        acc = as_bool(&self.eval(rhs, env)?, rhs.span)?;
                    }
                }
                Ok(Value::Bool(acc))
            }
            ExprKind::Neg(inner) => match self.eval(inner, env)? {
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(RunError {
                    message: format!("cannot negate {}", other.kind_name()),
                    span: expr.span,
                }),
            },
            ExprKind::Not(inner) => {
                Ok(Value::Bool(!as_bool(&self.eval(inner, env)?, inner.span)?))
            }
            // Only the TAKEN branch is evaluated: test each condition (which
            // must be a bool) in order and evaluate the first matching branch.
            // `else if` chains nest down `else_branch`, so descend that spine
            // ITERATIVELY — a long chain must consume no host stack per link.
            ExprKind::If { .. } => {
                let mut node = expr;
                loop {
                    match &node.kind {
                        ExprKind::If {
                            cond,
                            then_branch,
                            else_branch,
                        } => {
                            let taken = match self.eval(cond, env)? {
                                Value::Bool(b) => b,
                                other => {
                                    return Err(RunError {
                                        message: format!(
                                            "`if` condition needs a bool, got {}",
                                            other.kind_name()
                                        ),
                                        span: cond.span,
                                    })
                                }
                            };
                            if taken {
                                return self.eval(then_branch, env);
                            }
                            node = else_branch;
                        }
                        // The final (non-`if`) else branch.
                        _ => return self.eval(node, env),
                    }
                }
            }
            // A nullary constructor used bare IS the variant value; a
            // parameterful one is a callable constructor (so `List.map(xs,
            // Circle)` works like any function argument).
            ExprKind::Ctor { name, arity } => {
                if *arity == 0 {
                    Ok(Value::Variant {
                        ctor: Rc::from(name.as_str()),
                        args: Rc::new(Vec::new()),
                    })
                } else {
                    Ok(Value::Ctor {
                        name: Rc::from(name.as_str()),
                        arity: *arity,
                    })
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let value = self.eval(scrutinee, env)?;
                for arm in arms {
                    let mut vars = Vec::new();
                    if match_pattern(&arm.pattern, &value, &mut vars) {
                        if self.recorder.is_some() {
                            let mut sites = Vec::new();
                            pattern_binder_sites(&arm.pattern, &mut sites);
                            for (binding, bound) in &vars {
                                if let Some(&(_, name, span)) =
                                    sites.iter().find(|(b, _, _)| b == binding)
                                {
                                    self.record_binding(*binding, name, span, bound);
                                }
                            }
                        }
                        let child = env.child(vars);
                        return self.eval(&arm.body, &child);
                    }
                }
                Err(RunError {
                    message: format!("no pattern matched {value}"),
                    span: expr.span,
                })
            }
        }
    }

    fn binary_op(
        &mut self,
        op: crate::ast::BinOp,
        lhs: Value,
        rhs: Value,
        span: Span,
    ) -> Result<Value, RunError> {
        use crate::ast::BinOp;
        match op {
            BinOp::Add => self.arith(op, lhs, rhs, span, |a, b| a + b),
            BinOp::Sub => self.arith(op, lhs, rhs, span, |a, b| a - b),
            BinOp::Mul => self.arith(op, lhs, rhs, span, |a, b| a * b),
            // Division follows IEEE-754 (x/0 is ±inf/NaN, printed as
            // `inf`/`NaN`) — one number type, no checked division.
            BinOp::Div => self.arith(op, lhs, rhs, span, |a, b| a / b),
            BinOp::Lt => self.compare(lhs, rhs, span, |a, b| a < b),
            BinOp::Gt => self.compare(lhs, rhs, span, |a, b| a > b),
            // IEEE ordering, like `<`/`>`: every comparison with NaN is false.
            BinOp::Le => self.compare(lhs, rhs, span, |a, b| a <= b),
            BinOp::Ge => self.compare(lhs, rhs, span, |a, b| a >= b),
            // `!=` is the exact negation of `==`: the SAME structural walk,
            // so it errors on functions/host values on exactly the inputs
            // `==` does (and `NaN != NaN` is true, since `NaN == NaN` is
            // false — the IEEE behaviour `<`/`<=` already have).
            BinOp::Eq | BinOp::Ne => {
                // Charge AFTER the walk (the count isn't known up front):
                // one comparison can overshoot the budget by at most the
                // value's size — itself budget-bounded to build — so total
                // work stays O(budget).
                let mut compared = 0u64;
                let negated = matches!(op, BinOp::Ne);
                let eq = value_eq(&lhs, &rhs, span, &mut compared, op.symbol())?;
                self.charge(compared, span)?;
                Ok(Value::Bool(eq != negated))
            }
        }
    }

    fn arith(
        &mut self,
        op: crate::ast::BinOp,
        lhs: Value,
        rhs: Value,
        span: Span,
        arith: fn(f64, f64) -> f64,
    ) -> Result<Value, RunError> {
        match (lhs, rhs) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(arith(a, b))),
            // Not two numbers: a branded operand may still have a declared
            // implementation for this operator.
            (a, b) => self.brand_arith(op, a, b, span),
        }
    }

    /// Arithmetic where at least one operand is not a number — the runtime
    /// half of unit operators. The implementation is exactly the one the
    /// `unit … (<op>) = …` declaration names, called with the operands in the
    /// declared order.
    fn brand_arith(
        &mut self,
        op: crate::ast::BinOp,
        lhs: Value,
        rhs: Value,
        span: Span,
    ) -> Result<Value, RunError> {
        use crate::ast::BinOp;
        let Some(slot) = op_slot(op) else {
            unreachable!("only arithmetic reaches brand dispatch")
        };
        let declared = |ops: &BrandOps, value: &Value| {
            brand_tag(value)
                .and_then(|tag| ops.get(tag))
                .and_then(|row| row[slot].clone())
        };
        // The declared form takes the branded value FIRST. Scaling commutes,
        // so `2.0 * 45deg` is the same call with its arguments swapped —
        // matching what the checker resolves (`/` deliberately does not).
        let call = match declared(&self.brand_ops, &lhs) {
            Some(implementation) => Some((implementation, lhs.clone(), rhs.clone())),
            None if matches!(op, BinOp::Mul) && matches!(lhs, Value::Number(_)) => {
                declared(&self.brand_ops, &rhs)
                    .map(|implementation| (implementation, rhs.clone(), lhs.clone()))
            }
            None => None,
        };
        match call {
            Some((implementation, first, second)) => self.call(
                implementation,
                vec![first, second],
                format!("({})", op.symbol()),
                span,
                None,
            ),
            None => Err(RunError {
                message: format!(
                    "arithmetic needs numbers, got {} and {}{}",
                    lhs.kind_name(),
                    rhs.kind_name(),
                    self.brand_arith_hint(op, &lhs, &rhs)
                ),
                span,
            }),
        }
    }

    /// What a failed branded arithmetic adds to its error: which operators
    /// the brand DOES declare, or how to declare one — the same teaching the
    /// checker gives, for the paths that reach the interpreter first.
    fn brand_arith_hint(&self, op: crate::ast::BinOp, lhs: &Value, rhs: &Value) -> String {
        let Some(tag) = brand_tag(lhs).or_else(|| brand_tag(rhs)) else {
            return String::new();
        };
        let declared: Vec<&str> = self
            .brand_ops
            .get(tag)
            .into_iter()
            .flat_map(|row| {
                row.iter()
                    .enumerate()
                    .filter(|(_, implementation)| implementation.is_some())
                    .map(|(slot, _)| slot_op(slot).symbol())
            })
            .collect();
        if declared.is_empty() {
            return format!(
                " — `{tag}` declares no arithmetic; declare it with `unit <suffix> ({}) = …`",
                op.symbol()
            );
        }
        format!(
            " — `{tag}` declares {}, but not `{}`",
            declared
                .iter()
                .map(|symbol| format!("`{symbol}`"))
                .collect::<Vec<_>>()
                .join(", "),
            op.symbol()
        )
    }

    fn compare(
        &mut self,
        lhs: Value,
        rhs: Value,
        span: Span,
        op: fn(f64, f64) -> bool,
    ) -> Result<Value, RunError> {
        match (lhs, rhs) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(op(a, b))),
            (a, b) => Err(RunError {
                message: format!(
                    "comparison needs numbers, got {} and {}",
                    a.kind_name(),
                    b.kind_name()
                ),
                span,
            }),
        }
    }

    /// Call a value. `label` is the callee's source-level name for the trace
    /// (`report`, `List.map`, `<lambda>`); `span` is the call site, used for
    /// errors raised by the call itself (arity, not-callable). `via` is set
    /// when a builtin is invoking its function argument, so an arity error
    /// blames "the function passed to List.map", not the builtin.
    fn call(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        label: String,
        span: Span,
        via: Option<&'static str>,
    ) -> Result<Value, RunError> {
        // Currying: a cheap arity pre-check gates the three curried cases. The
        // SATURATED case (`args.len() == arity` — every direct `f(a, b)` and
        // every pipe) falls straight through to the original dispatch below
        // with NO extra allocation and NO extra stack frame. The partial /
        // over-application / partial-unwrap cases are handled by the `#[cold]`
        // [`Self::call_curried`] so their locals never enlarge this hot frame
        // (the per-recursion stack cost feeds the eval-depth cap's budget, so
        // keeping this frame lean matters).
        match callee_arity(&callee) {
            Some(arity) if args.len() == arity => {}
            Some(arity) => return self.call_curried(callee, args, Some(arity), label, span, via),
            // A partial is unwrapped and re-dispatched on its underlying callee.
            None if matches!(callee, Value::Partial(_)) => {
                return self.call_curried(callee, args, None, label, span, via)
            }
            // Host fn (arity unknown) — treated as saturated; the host
            // validates its own arg count.
            None => {}
        }
        // ---- Saturated (or host) dispatch — the original body, unchanged ----
        // The step budget charges per CALL, not per eval node: the language
        // has no loops, so unbounded work must pass through here (recursion,
        // builtin per-element closure calls) or through `List.range`'s bulk
        // charge — and keeping the per-node eval path branch-free costs the
        // frame loop nothing (frame_bench-verified; a per-eval charge was a
        // measured ~3% wall-clock regression).
        self.charge(1, span)?;
        self.trace_enter(&label, &args);
        self.call_depth += 1;
        let result = match &callee {
            Value::Closure(closure) => {
                if closure.params.len() != args.len() {
                    let who = match via {
                        Some(builtin) => format!("the function passed to {builtin}"),
                        None => format!("`{label}`"),
                    };
                    Err(RunError {
                        message: format!(
                            "{who} takes {} argument(s), got {}",
                            closure.params.len(),
                            args.len()
                        ),
                        span,
                    })
                } else {
                    let vars: Vec<(BindingId, Value)> =
                        closure.params.iter().map(|p| p.binding).zip(args).collect();
                    if self.recorder.is_some() {
                        for (param, (_, value)) in closure.params.iter().zip(vars.iter()) {
                            self.record_binding(param.binding, &param.name, param.span, value);
                        }
                    }
                    let env = closure.env.child(vars);
                    let body = closure.body.clone();
                    self.eval(&body, &env)
                }
            }
            Value::Ctor { name, arity } => {
                // Saturated by the arity gate above: every mismatch went to
                // `call_curried`, which refuses it (constructors apply fully).
                debug_assert_eq!(args.len(), *arity, "ctor mismatch reaches call_curried");
                Ok(Value::Variant {
                    ctor: name.clone(),
                    args: Rc::new(args),
                })
            }
            Value::Builtin(b) => self.call_builtin(*b, args, span),
            Value::HostFn(path) => self.host.call(path, args, span),
            other => Err(RunError {
                message: format!("cannot call {}", other.kind_name()),
                span,
            }),
        };
        self.call_depth -= 1;
        if let Ok(value) = &result {
            self.trace_exit(value);
        }
        result
    }

    /// Currying's COLD paths — under-application, over-application, and
    /// partial-unwrap. Kept out of [`Self::call`]'s hot frame (`#[cold]`) so
    /// the saturated path pays nothing for them. `arity` is the callee's known
    /// arity (`None` only for a `Partial` being unwrapped). Each branch that
    /// dispatches re-enters `self.call`, which re-runs the arity gate and does
    /// its own tracing, so the outer call neither traces nor bumps
    /// `call_depth` for these cases.
    #[cold]
    #[inline(never)]
    fn call_curried(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        arity: Option<usize>,
        label: String,
        span: Span,
        via: Option<&'static str>,
    ) -> Result<Value, RunError> {
        // A partial absorbs the new args and re-dispatches on its underlying
        // callable with the combined argument list.
        if let Value::Partial(partial) = &callee {
            let mut combined = partial.applied.clone();
            combined.extend(args);
            return self.call(partial.callee.clone(), combined, label, span, via);
        }
        let arity = arity.expect("non-partial cold path carries its arity");
        // Constructors do NOT curry: applying one with the wrong argument
        // count is an error right here, under- and over-application alike.
        // (A bare `Circle` reference is still a first-class function value —
        // only CALLING one with the wrong count is refused. The checker
        // catches direct call syntax; this is the gradual-seam backstop.)
        if let Value::Ctor { name, .. } = &callee {
            return Err(RunError {
                message: ctor_arity_message(name, arity, args.len(), via),
                span,
            });
        }
        if args.len() < arity {
            // Under-applied → capture what we have as a partial. (The count
            // still needed is derived from the callee's arity at display time,
            // so we store only the callee and the args supplied so far.)
            return Ok(Value::Partial(Rc::new(crate::value::Partial {
                callee,
                applied: args,
            })));
        }
        // Over-applied → saturate with the first `arity` args, then apply the
        // leftover args to the result (which must itself be callable).
        let mut rest = args;
        let now = rest.drain(..arity).collect();
        let saturated = self.call(callee, now, label.clone(), span, via)?;
        self.call(saturated, rest, label, span, None)
    }

    fn trace_enter(&mut self, callee: &str, args: &[Value]) {
        if self.tracing == Tracing::Off {
            return;
        }
        if self.trace.len() >= MAX_TRACE_EVENTS {
            if !matches!(self.trace.last(), Some(TraceEvent::Truncated)) {
                self.trace.push(TraceEvent::Truncated);
            }
            return;
        }
        self.trace.push(TraceEvent::Enter {
            depth: self.call_depth,
            callee: callee.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        });
    }

    fn trace_exit(&mut self, result: &Value) {
        if self.tracing == Tracing::Off {
            return;
        }
        if self.trace.len() >= MAX_TRACE_EVENTS {
            // Same marker as trace_enter: without it, a transcript whose last
            // recorded event is an Enter would read like a hang.
            if !matches!(self.trace.last(), Some(TraceEvent::Truncated)) {
                self.trace.push(TraceEvent::Truncated);
            }
            return;
        }
        self.trace.push(TraceEvent::Exit {
            depth: self.call_depth,
            result: result.to_string(),
        });
    }

    fn call_builtin(
        &mut self,
        b: Builtin,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RunError> {
        let err = |message: String| Err(RunError { message, span });
        match b {
            // Subject-LAST — `List.map(fn, list)`, so `list |> List.map(fn)`
            // threads the list in as the final argument.
            Builtin::ListMap => match args.as_slice() {
                [f, Value::List(items)] => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        out.push(self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?);
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.map(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST — `List.filter(fn, list)`.
            Builtin::ListFilter => match args.as_slice() {
                [f, Value::List(items)] => {
                    let mut out = Vec::new();
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::Bool(true) => out.push(item.clone()),
                            Value::Bool(false) => {}
                            other => {
                                return err(format!(
                                    "List.filter predicate must return a bool, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.filter(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST like map/filter, so it composes with `|>` (the piped
            // value is APPENDED as the final argument — see crate::lower):
            // `list |> List.fold(fn, init)` == `List.fold(fn, init, list)`.
            Builtin::ListFold => match args.as_slice() {
                [f, init, Value::List(items)] => {
                    let mut acc = init.clone();
                    for (i, item) in items.iter().enumerate() {
                        acc = self.call(
                            f.clone(),
                            vec![acc, item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?;
                    }
                    Ok(acc)
                }
                _ => err(
                    "List.fold(fn, init, list) expects a function, an initial value, and a list"
                        .to_string(),
                ),
            },
            // NaN handling follows Rust's `f64::max` (IEEE maximumNumber):
            // NaN elements are ignored unless every element is NaN.
            // `List.range(n)` -> [0, 1, …, n-1] as Floats; n truncates. The
            // count must be finite and sane: Functor Lang numbers permit `inf`
            // (IEEE division), and `inf as usize` would ask the allocator
            // for usize::MAX elements — a process-killing panic, not a
            // recoverable frame error.
            Builtin::ListRange => match args.as_slice() {
                [Value::Number(n)] if n.is_finite() && *n <= 1_000_000.0 => {
                    let count = n.max(0.0) as usize;
                    // The one bulk builtin with NO per-element eval: charge
                    // its element count so a budget can't be sidestepped by
                    // allocating in one step. (List.grid pays per cell via
                    // its closure calls.)
                    self.charge(count as u64, span)?;
                    Ok(Value::List(Rc::new(
                        (0..count).map(|i| Value::Number(i as f64)).collect(),
                    )))
                }
                [Value::Number(n)] => err(format!(
                    "List.range needs a finite count up to 1000000, got {n}"
                )),
                _ => err("List.range(n) expects one number".to_string()),
            },
            // Tabulate a rows×cols grid by calling `f(row, col)` for each cell
            // (both 0-based) — `[[f(0,0), f(0,1), …], …]`. It is the engine's
            // tight-loop form of a procedural heightmap
            // (`Scene.heightmap(List.grid(height, rows, cols))`); vs a nested
            // `List.map(…)` it skips allocating the two range lists
            // and the outer-map closure each frame (both interpret `f` per cell,
            // and both loop iteratively so eval depth never accumulates).
            // Subject-LAST: the callback comes FIRST — `List.grid(fn, rows, cols)`.
            Builtin::ListGrid => match args.as_slice() {
                [f, Value::Number(rows), Value::Number(cols)]
                    if is_function(f)
                        && rows.is_finite()
                        && cols.is_finite()
                        && rows.fract() == 0.0
                        && cols.fract() == 0.0
                        && *rows >= 0.0
                        && *cols >= 0.0
                        // Bound TOTAL cells (not just each dim): a per-cell
                        // closure call makes this the interpreter's heaviest
                        // loop, so cap it like `List.range`.
                        && *rows * *cols <= 1_000_000.0 =>
                {
                    let (rows, cols) = (*rows as usize, *cols as usize);
                    let mut grid = Vec::with_capacity(rows);
                    for r in 0..rows {
                        let mut row = Vec::with_capacity(cols);
                        for c in 0..cols {
                            row.push(self.call(
                                f.clone(),
                                vec![Value::Number(r as f64), Value::Number(c as f64)],
                                format!("{}[{r}][{c}]", builtin_name(b)),
                                span,
                                Some(builtin_name(b)),
                            )?);
                        }
                        grid.push(Value::List(Rc::new(row)));
                    }
                    Ok(Value::List(Rc::new(grid)))
                }
                [f, Value::Number(_), Value::Number(_)] if is_function(f) => err(
                    "List.grid(fn, rows, cols) needs whole, non-negative counts with at \
most 1000000 cells"
                        .to_string(),
                ),
                _ => err("List.grid(fn, rows, cols) expects a function and two numbers".to_string()),
            },
            Builtin::ListMaximum => match args.as_slice() {
                [Value::List(items)] => {
                    // O(n) scan: charge n (the List.reverse rule).
                    self.charge(items.len() as u64, span)?;
                    let mut best: Option<f64> = None;
                    for item in items.iter() {
                        match item {
                            Value::Number(n) => {
                                best = Some(best.map_or(*n, |b: f64| b.max(*n)));
                            }
                            other => {
                                return err(format!(
                                    "List.maximum expects numbers, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    // PARTIAL, like `nth`/`head`/`last`/`find`: an empty list
                    // is `Option.None`, not an error.
                    Ok(option_value(best.map(Value::Number)))
                }
                _ => err("List.maximum(list) expects one list".to_string()),
            },
            // `List.minimum` is `List.maximum`'s sibling, Option-shaped like
            // every partial accessor (`List.head`/`nth`/`find`): an empty
            // list is `Option.None`, not an error.
            //
            // NaN follows `f64::min`, exactly like `List.maximum`'s `f64::max`:
            // a NaN element is ignored while any real number is present.
            Builtin::ListMinimum => match args.as_slice() {
                [Value::List(items)] => {
                    // O(n) scan: charge n (the List.maximum rule).
                    self.charge(items.len() as u64, span)?;
                    let mut best: Option<f64> = None;
                    for item in items.iter() {
                        match item {
                            Value::Number(n) => {
                                best = Some(best.map_or(*n, |b: f64| b.min(*n)));
                            }
                            other => {
                                return err(format!(
                                    "List.minimum expects numbers, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(option_value(best.map(Value::Number)))
                }
                _ => err("List.minimum(list) expects one list".to_string()),
            },
            // The iterative list helpers a game reaches for — hand-rolling
            // these recursively blows the eval-depth cap around n≈60, so they
            // loop in Rust and consume no evaluation depth (see MAX_EVAL_DEPTH).
            Builtin::ListLength => match args.as_slice() {
                [Value::List(items)] => Ok(Value::Number(items.len() as f64)),
                _ => err("List.length(list) expects one list".to_string()),
            },
            // Subject-LAST — `xs |> List.append(ys)` is `List.append(ys, xs)`
            // and yields `xs` followed by `ys` (the piped list stays the head).
            Builtin::ListAppend => match args.as_slice() {
                [Value::List(other), Value::List(items)] => {
                    // GROWTH builtin: charge the materialized output size,
                    // else `d = (x) => List.append(x, x)` doubles per unit
                    // charge — exponential work under a linear budget (the
                    // review probe: 26 nestings = 134M elements, seconds of
                    // wall-clock, ~56 charges).
                    self.charge((items.len() + other.len()) as u64, span)?;
                    let mut out = Vec::with_capacity(items.len() + other.len());
                    out.extend(items.iter().cloned());
                    out.extend(other.iter().cloned());
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.append(other, list) expects two lists".to_string()),
            },
            // Concatenate a list of lists one level deep (`List<List<'a>>` ->
            // `List<'a>`). A non-list element is an error, not a silent no-op.
            Builtin::ListFlatten => match args.as_slice() {
                [Value::List(items)] => {
                    let mut out = Vec::new();
                    for item in items.iter() {
                        match item {
                            Value::List(inner) => {
                                // Growth builtin: charge per materialized
                                // element (the List.append rule) — `[x, x]
                                // |> List.flatten` doubles too.
                                self.charge(inner.len() as u64, span)?;
                                out.extend(inner.iter().cloned())
                            }
                            other => {
                                return err(format!(
                                    "List.flatten expects a list of lists, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.flatten(list) expects one list of lists".to_string()),
            },
            // Subject-LAST — `xs |> List.any(pred)`. True when the predicate
            // holds for at least one element; short-circuits on the first hit.
            Builtin::ListAny => match args.as_slice() {
                [f, Value::List(items)] => {
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::Bool(true) => return Ok(Value::Bool(true)),
                            Value::Bool(false) => {}
                            other => {
                                return err(format!(
                                    "List.any predicate must return a bool, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::Bool(false))
                }
                _ => err("List.any(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST — `xs |> List.all(pred)`. True when the predicate
            // holds for every element; short-circuits on the first miss (an
            // empty list is vacuously true).
            Builtin::ListAll => match args.as_slice() {
                [f, Value::List(items)] => {
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::Bool(true) => {}
                            Value::Bool(false) => return Ok(Value::Bool(false)),
                            other => {
                                return err(format!(
                                    "List.all predicate must return a bool, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::Bool(true))
                }
                _ => err("List.all(fn, list) expects a function and a list".to_string()),
            },
            Builtin::ListReverse => match args.as_slice() {
                [Value::List(items)] => {
                    // O(n) copy of already-paid elements: charge n so a
                    // chain of unit-cost reverses can't go budget-quadratic.
                    self.charge(items.len() as u64, span)?;
                    let mut out: Vec<Value> = items.iter().cloned().collect();
                    out.reverse();
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.reverse(list) expects one list".to_string()),
            },
            Builtin::ListIsEmpty => match args.as_slice() {
                [Value::List(items)] => Ok(Value::Bool(items.is_empty())),
                _ => err("List.isEmpty(list) expects one list".to_string()),
            },
            // The PARTIAL list accessors — `nth`/`head`/`last`/`find` all
            // answer with `Option.t` (`Option.Some(x)` / `Option.None`)
            // rather than raising or inventing a sentinel. Absence is an
            // ordinary, matchable value here: game code that used to carry a
            // `-1.0` / `9999.0` "nothing found" marker can hold an
            // `Option.None` instead, and the checker forces it to be handled.
            //
            // (`List.maximum` is the same shape — see its arm above.)
            //
            // Subject-LAST — `List.nth(index, list)`, so `xs |> List.nth(2)`.
            // An index outside the list is absence (`Option.None`); an index
            // that isn't a whole finite number is a BUG in the caller, so it
            // errors rather than quietly reading as "not found".
            Builtin::ListNth => match args.as_slice() {
                [Value::Number(i), Value::List(items)] if i.is_finite() && i.fract() == 0.0 => {
                    Ok(option_value(
                        (*i >= 0.0)
                            .then(|| items.get(*i as usize).cloned())
                            .flatten(),
                    ))
                }
                [Value::Number(i), Value::List(_)] => err(format!(
                    "List.nth needs a whole, finite index, got {i}"
                )),
                _ => err("List.nth(index, list) expects a number and a list".to_string()),
            },
            Builtin::ListHead => match args.as_slice() {
                [Value::List(items)] => Ok(option_value(items.first().cloned())),
                _ => err("List.head(list) expects one list".to_string()),
            },
            Builtin::ListLast => match args.as_slice() {
                [Value::List(items)] => Ok(option_value(items.last().cloned())),
                _ => err("List.last(list) expects one list".to_string()),
            },
            // Subject-LAST — `xs |> List.find(pred)`. The first element the
            // predicate accepts, as `Option.Some`; `Option.None` when none
            // match. Short-circuits on the first hit (like `List.any`).
            Builtin::ListFind => match args.as_slice() {
                [f, Value::List(items)] => {
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::Bool(true) => return Ok(option_value(Some(item.clone()))),
                            Value::Bool(false) => {}
                            other => {
                                return err(format!(
                                    "List.find predicate must return a bool, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(option_value(None))
                }
                _ => err("List.find(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST — `xs |> List.indexedMap(fn)`. Like `List.map` but
            // the callback also receives the 0-based index FIRST:
            // `fn(index, element)`. This is the builtin that retires the
            // quadratic hand-rolled idiom (a `List.fold` that re-scanned the
            // list to recover each element's position).
            Builtin::ListIndexedMap => match args.as_slice() {
                [f, Value::List(items)] => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        out.push(self.call(
                            f.clone(),
                            vec![Value::Number(i as f64), item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?);
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.indexedMap(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST — `xs |> List.sortBy(key)`. Ascending by the
            // NUMBER the key function returns, and STABLE: equal keys keep
            // their original relative order, so a sort is reproducible frame
            // to frame (a game sorting sprites by depth must not shimmer).
            //
            // The key is evaluated exactly ONCE per element (a decorate–
            // sort–undecorate pass), not once per comparison: interpreting a
            // closure O(n log n) times would make sorting the dominant cost
            // of a frame. So it is n key calls + O(n log n) f64 comparisons.
            //
            // Ordering is [`sort_key_cmp`]: ascending, with NaN keys last.
            // It deliberately does NOT use `f64::total_cmp`, which orders by
            // the SIGN BIT and so puts a negative NaN first and a positive
            // NaN last — and the sign of a COMPUTED NaN is not specified by
            // IEEE 754. `0.0 / 0.0` is `-NaN` on x86 (`divsd` returns the
            // "QNaN floating-point indefinite") but `+NaN` on aarch64, so a
            // total_cmp sort put the same list in different orders on Linux
            // and macOS. In an engine that promises deterministic replay,
            // that is a correctness bug, not a curiosity.
            Builtin::ListSortBy => match args.as_slice() {
                [f, Value::List(items)] => {
                    // O(n log n) over already-paid elements, plus the n key
                    // calls (each charged by `call`): charge n, the
                    // `List.reverse` rule for a whole-list pass.
                    self.charge(items.len() as u64, span)?;
                    let mut keyed: Vec<(f64, Value)> = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let key = self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?;
                        match key {
                            Value::Number(n) => keyed.push((n, item.clone())),
                            other => {
                                return err(format!(
                                    "List.sortBy key must return a number, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    // `sort_by` is a stable merge sort — the guarantee above.
                    keyed.sort_by(|(a, _), (b, _)| sort_key_cmp(*a, *b));
                    Ok(Value::List(Rc::new(
                        keyed.into_iter().map(|(_, v)| v).collect(),
                    )))
                }
                _ => err("List.sortBy(fn, list) expects a function and a list".to_string()),
            },
            // Subject-LAST — `xs |> List.zip(ys)` is `List.zip(ys, xs)` and
            // pairs them as `(x, y)`: the PIPED list stays the first tuple
            // slot, matching `List.append`'s "the piped list stays the head".
            // Stops at the shorter list (no padding, no error) — the standard
            // truncating zip.
            Builtin::ListZip => match args.as_slice() {
                [Value::List(other), Value::List(items)] => {
                    let n = items.len().min(other.len());
                    // Materializes n tuples holding 2n cloned cells — charge
                    // the CELLS, per `List.append`'s "charge what you
                    // materialize" rule, not the tuple count.
                    self.charge(2 * n as u64, span)?;
                    let pairs: Vec<Value> = items
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| Value::Tuple(Rc::new(vec![a.clone(), b.clone()])))
                        .collect();
                    Ok(Value::List(Rc::new(pairs)))
                }
                _ => err("List.zip(other, list) expects two lists".to_string()),
            },
            // Subject-LAST — `xs |> List.take(3)` / `xs |> List.drop(3)`.
            // The count SATURATES rather than erroring: taking more than the
            // list holds yields the whole list, dropping more yields `[]`,
            // and a negative count behaves like 0. Windowing at a list's edge
            // is normal (a scrolling HUD, the last N scores), so it must not
            // need a guard at every call site.
            Builtin::ListTake | Builtin::ListDrop => match args.as_slice() {
                [Value::Number(n), Value::List(items)] => {
                    // `f64::max` returns the non-NaN side (IEEE maximumNumber),
                    // so a NaN count folds to 0 here with no special case, and
                    // `min(len)` bounds ±inf — the `as usize` can never
                    // saturate to an absurd length.
                    let k = n.max(0.0).min(items.len() as f64) as usize;
                    // O(k) copy of already-paid elements (the reverse rule).
                    // Charge BEFORE cloning: an exhausted budget must stop the
                    // copy, not merely report it afterwards.
                    let kept = if b == Builtin::ListTake {
                        k
                    } else {
                        items.len() - k
                    };
                    self.charge(kept as u64, span)?;
                    let out: Vec<Value> = if b == Builtin::ListTake {
                        items[..k].to_vec()
                    } else {
                        items[k..].to_vec()
                    };
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err(format!(
                    "{}(count, list) expects a number and a list",
                    builtin_name(b)
                )),
            },
            // The total counterpart to `List.maximum`: an empty list sums to
            // 0.0, so there is nothing partial about it and no Option.
            Builtin::ListSum => match args.as_slice() {
                [Value::List(items)] => {
                    // O(n) scan: charge n (the List.maximum rule).
                    self.charge(items.len() as u64, span)?;
                    let mut total = 0.0;
                    for item in items.iter() {
                        match item {
                            Value::Number(n) => total += n,
                            other => {
                                return err(format!(
                                    "List.sum expects numbers, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::Number(total))
                }
                _ => err("List.sum(list) expects one list".to_string()),
            },
            // Subject-LAST — `xs |> List.concatMap(fn)`. Map then flatten one
            // level, in a single pass (`fn` must return a list per element).
            Builtin::ListConcatMap => match args.as_slice() {
                [f, Value::List(items)] => {
                    let mut out = Vec::new();
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::List(inner) => {
                                // GROWTH builtin: charge the materialized
                                // output (the List.flatten rule) — a callback
                                // returning `[x, x]` doubles per unit charge.
                                self.charge(inner.len() as u64, span)?;
                                out.extend(inner.iter().cloned());
                            }
                            other => {
                                return err(format!(
                                    "List.concatMap function must return a list, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.concatMap(fn, list) expects a function and a list".to_string()),
            },
            Builtin::MapEmpty => match args.as_slice() {
                [] => Ok(Value::Map(Rc::new(Vec::new()))),
                _ => unreachable!("builtin arity checked before dispatch"),
            },
            // Maps are immutable sorted vectors. Lookup is binary search;
            // updates copy one canonical vector and preserve key order.
            // Subject-LAST throughout, so `map |> Map.get(key)` works.
            Builtin::MapGet => match args.as_slice() {
                [key, Value::Map(entries)] => {
                    let key = map_key_from_value(key, builtin_name(b))
                        .map_err(|message| RunError { message, span })?;
                    let found = self.map_search(entries, &key, span)?;
                    Ok(option_value(
                        found.ok().map(|index| entries[index].1.clone()),
                    ))
                }
                _ => err("Map.get(key, map) expects a key and a map".to_string()),
            },
            Builtin::MapInsert => match args.as_slice() {
                [key, value, Value::Map(entries)] => {
                    let key = map_key_from_value(key, builtin_name(b))
                        .map_err(|message| RunError { message, span })?;
                    let found = self.map_search(entries, &key, span)?;
                    let out_len = if found.is_ok() {
                        entries.len()
                    } else {
                        entries.len().saturating_add(1)
                    };
                    // Pre-charge the immutable copy so a low-budget tool
                    // cannot allocate it before discovering it is over fuel.
                    self.charge(out_len as u64, span)?;
                    let mut out = entries.as_ref().clone();
                    match found {
                        Ok(index) => out[index].1 = value.clone(),
                        Err(index) => out.insert(index, (key, value.clone())),
                    }
                    Ok(Value::Map(Rc::new(out)))
                }
                _ => {
                    err("Map.insert(key, value, map) expects a key, a value, and a map".to_string())
                }
            },
            Builtin::MapRemove => match args.as_slice() {
                [key, Value::Map(entries)] => {
                    let key = map_key_from_value(key, builtin_name(b))
                        .map_err(|message| RunError { message, span })?;
                    let found = self.map_search(entries, &key, span)?;
                    let Ok(index) = found else {
                        return Ok(Value::Map(entries.clone()));
                    };
                    self.charge(entries.len().saturating_sub(1) as u64, span)?;
                    let mut out = entries.as_ref().clone();
                    out.remove(index);
                    Ok(Value::Map(Rc::new(out)))
                }
                _ => err("Map.remove(key, map) expects a key and a map".to_string()),
            },
            Builtin::MapMember => match args.as_slice() {
                [key, Value::Map(entries)] => {
                    let key = map_key_from_value(key, builtin_name(b))
                        .map_err(|message| RunError { message, span })?;
                    let found = self.map_search(entries, &key, span)?;
                    Ok(Value::Bool(found.is_ok()))
                }
                _ => err("Map.member(key, map) expects a key and a map".to_string()),
            },
            Builtin::MapValues => match args.as_slice() {
                [Value::Map(entries)] => {
                    self.charge(entries.len() as u64, span)?;
                    Ok(Value::List(Rc::new(
                        entries.iter().map(|(_, value)| value.clone()).collect(),
                    )))
                }
                _ => err("Map.values(map) expects one map".to_string()),
            },
            Builtin::MapToList => match args.as_slice() {
                [Value::Map(entries)] => {
                    // Each entry materializes a two-cell tuple.
                    self.charge((entries.len() as u64).saturating_mul(2), span)?;
                    Ok(Value::List(Rc::new(
                        entries
                            .iter()
                            .map(|(key, value)| {
                                Value::Tuple(Rc::new(vec![key.to_value(), value.clone()]))
                            })
                            .collect(),
                    )))
                }
                _ => err("Map.toList(map) expects one map".to_string()),
            },
            Builtin::MapFromList => match args.as_slice() {
                [Value::List(items)] => {
                    // Charge entry cloning up front. After validating the
                    // keys, refuse an over-budget sort before its first
                    // comparison. The preflight is conservative; actual
                    // comparison work is charged afterward.
                    self.charge(items.len() as u64, span)?;
                    let mut sorted = Vec::with_capacity(items.len());
                    let mut max_comparison_units = 1u64;
                    for (index, item) in items.iter().enumerate() {
                        let Value::Tuple(pair) = item else {
                            return err(format!(
                                "Map.fromList(entries) expects (key, value) tuples; entry {index} is {}",
                                item.kind_name()
                            ));
                        };
                        let [key, value] = pair.as_slice() else {
                            return err(format!(
                                "Map.fromList(entries) expects 2-tuples; entry {index} has {} items",
                                pair.len()
                            ));
                        };
                        let key = map_key_from_value(key, builtin_name(b)).map_err(|message| {
                            RunError {
                                message: format!("{message} at entry {index}"),
                                span,
                            }
                        })?;
                        max_comparison_units =
                            max_comparison_units.max(key.comparison_unit_ceiling());
                        sorted.push((key, value.clone()));
                    }
                    let comparison_count_ceiling = stable_sort_comparison_ceiling(items.len())
                        .saturating_add(
                            u64::try_from(items.len().saturating_sub(1))
                                .unwrap_or(u64::MAX),
                        );
                    let comparison_work_ceiling =
                        comparison_count_ceiling.saturating_mul(max_comparison_units);
                    self.preflight_charge(comparison_work_ceiling, span)?;
                    let (out, comparison_work) = canonicalize_map_entries(sorted);
                    debug_assert!(comparison_work <= comparison_work_ceiling);
                    self.charge(comparison_work, span)?;
                    Ok(Value::Map(Rc::new(out)))
                }
                _ => {
                    err("Map.fromList(entries) expects one list of (key, value) tuples".to_string())
                }
            },
            Builtin::TextConcat => match args.as_slice() {
                [Value::String(a), Value::String(b)] => {
                    // Growth builtin: charge output BYTES (the List.append
                    // rule for strings — `Text.concat(s, s)` doubles).
                    self.charge((a.len() + b.len()) as u64, span)?;
                    Ok(Value::String(Rc::from(format!("{a}{b}").as_str())))
                }
                _ => err("Text.concat(a, b) expects two strings".to_string()),
            },
            Builtin::TextFromFloat => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::String(Rc::from(n.to_string().as_str()))),
                _ => err("Text.fromFloat(n) expects one number".to_string()),
            },
            // Fixed-decimal formatting (the F# `sprintf "%.1f"` shape a HUD
            // needs): `Text.fixed(3.14159, 1.0)` is `"3.1"`, and 0 decimals
            // formats whole numbers with no point (`Text.fixed(42.0, 0.0)` is
            // `"42"` — the `%d` shape too). Rust's `{:.prec}` rounding.
            Builtin::TextFixed => match args.as_slice() {
                [Value::Number(n), Value::Number(d)]
                    if *d >= 0.0 && d.fract() == 0.0 && *d <= 12.0 =>
                {
                    Ok(Value::String(Rc::from(
                        format!("{:.*}", *d as usize, n).as_str(),
                    )))
                }
                [Value::Number(_), Value::Number(d)] => err(format!(
                    "Text.fixed needs a whole number of decimals between 0 and 12, got {d}"
                )),
                _ => err("Text.fixed(n, decimals) expects two numbers".to_string()),
            },
            Builtin::TextToBullets => match args.as_slice() {
                [Value::List(items)] => {
                    let mut lines = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match item {
                            Value::String(s) => {
                                // Growth builtin (string concatenation):
                                // charge output bytes, like Text.concat.
                                self.charge(s.len() as u64 + 3, span)?;
                                lines.push(format!("- {s}"))
                            }
                            other => {
                                return err(format!(
                                    "Text.toBullets expects strings, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::String(Rc::from(lines.join("\n").as_str())))
                }
                _ => err("Text.toBullets(list) expects one list of strings".to_string()),
            },
            // The wire-protocol string trio the multiplayer ports need (the
            // F# `String.Split` / `String.concat` / `parseInt` shapes).
            // Subject-LAST — `Text.split(sep, s)`, so `s |> Text.split(sep)`.
            Builtin::TextSplit => match args.as_slice() {
                [Value::String(sep), Value::String(s)] => {
                    if sep.is_empty() {
                        return err("Text.split needs a non-empty separator".to_string());
                    }
                    // O(input) scan + re-materialized pieces: charge input
                    // bytes (no growth — output total ≈ input).
                    self.charge(s.len() as u64, span)?;
                    let parts: Vec<Value> =
                        s.split(sep.as_ref()).map(|p| Value::String(Rc::from(p))).collect();
                    Ok(Value::List(Rc::new(parts)))
                }
                _ => err("Text.split(sep, s) expects two strings".to_string()),
            },
            // Subject-LAST — `Text.join(sep, list)`, so `list |> Text.join(sep)`.
            Builtin::TextJoin => match args.as_slice() {
                [Value::String(sep), Value::List(items)] => {
                    let mut parts = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match item {
                            Value::String(s) => {
                                // Growth builtin (string concatenation):
                                // charge output bytes, like Text.concat.
                                self.charge((s.len() + sep.len()) as u64, span)?;
                                parts.push(s.to_string())
                            }
                            other => {
                                return err(format!(
                                    "Text.join expects strings, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::String(Rc::from(parts.join(sep.as_ref()).as_str())))
                }
                _ => err("Text.join(sep, list) expects a string and a list of strings".to_string()),
            },
            // Parse a number out of a (possibly space-padded) string,
            // defaulting to 0.0 on failure — mirrors the F# ports'
            // `trim().parse().unwrap_or(0)`, so a malformed wire field
            // degrades to 0 rather than raising. `f64::from_str` also parses
            // "nan"/"inf", but a non-finite result is exactly the "garbage"
            // this neutralizes (and the engine boundary rejects non-finite
            // numbers), so those degrade to 0 too.
            Builtin::TextParseFloat => match args.as_slice() {
                [Value::String(s)] => {
                    // O(input) scan: charge input bytes (the Text.split rule).
                    self.charge(s.len() as u64, span)?;
                    Ok(Value::Number(
                        s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).unwrap_or(0.0),
                    ))
                }
                _ => err("Text.parseFloat(s) expects one string".to_string()),
            },
            // Text.length / Text.chars count and split by UNICODE SCALAR
            // VALUE (Rust `char`), not by byte and not by grapheme cluster.
            // So "héllo" is 5 and "日本" is 2 — but a combining accent
            // ("e" + U+0301) counts as 2, and an emoji ZWJ sequence splits
            // into its parts. That is the honest, cheap definition; there is
            // no char type in the language, so `Text.chars` hands back
            // one-character STRINGS, which is what a glyph-per-character HUD
            // wants to feed straight into `List.map`.
            //
            // The two agree by construction:
            // `Text.length(s) == List.length(Text.chars(s))`.
            Builtin::TextLength => match args.as_slice() {
                [Value::String(s)] => {
                    // O(input) scan: charge input bytes (the Text.split rule).
                    self.charge(s.len() as u64, span)?;
                    Ok(Value::Number(s.chars().count() as f64))
                }
                _ => err("Text.length(s) expects one string".to_string()),
            },
            Builtin::TextChars => match args.as_slice() {
                [Value::String(s)] => {
                    // O(input) scan + re-materialized pieces: charge input
                    // bytes (the Text.split rule — output total ≈ input).
                    self.charge(s.len() as u64, span)?;
                    let chars: Vec<Value> = s
                        .chars()
                        .map(|c| {
                            let mut buf = [0u8; 4];
                            Value::String(Rc::from(c.encode_utf8(&mut buf) as &str))
                        })
                        .collect();
                    Ok(Value::List(Rc::new(chars)))
                }
                _ => err("Text.chars(s) expects one string".to_string()),
            },
            // Unicode-aware case mapping (Rust's `to_uppercase`), so it is
            // not limited to ASCII. Note the length can CHANGE — "ß"
            // uppercases to "SS" — which is correct, not a bug.
            Builtin::TextToUpper | Builtin::TextToLower => match args.as_slice() {
                [Value::String(s)] => {
                    // The O(input) scan is charged up front; the mapping can
                    // GROW the string ("ß" -> "SS"), so the output is charged
                    // too — but at its EXACT size, not a blanket 3x
                    // worst-case, which would bill ordinary ASCII text triple
                    // and exhaust a budget that the real work fits inside.
                    self.charge(s.len() as u64, span)?;
                    let out = if b == Builtin::TextToUpper {
                        s.to_uppercase()
                    } else {
                        s.to_lowercase()
                    };
                    self.charge(out.len() as u64, span)?;
                    Ok(Value::String(Rc::from(out.as_str())))
                }
                _ => err(format!("{}(s) expects one string", builtin_name(b))),
            },
            // Subject-LAST — `Text.contains(needle, s)`, so
            // `s |> Text.contains("ab")`. The empty needle is contained in
            // every string (Rust/`str::contains` semantics).
            Builtin::TextContains => match args.as_slice() {
                [Value::String(needle), Value::String(s)] => {
                    // O(input) scan: charge input bytes (the Text.split rule).
                    self.charge(s.len() as u64, span)?;
                    Ok(Value::Bool(s.contains(needle.as_ref())))
                }
                _ => err("Text.contains(needle, s) expects two strings".to_string()),
            },
            // Subject-LAST — `Text.replace(from, to, s)`, so
            // `s |> Text.replace("a", "b")`. Replaces EVERY occurrence,
            // scanning left to right and never re-examining what it just
            // wrote (so `Text.replace("a", "aa", s)` terminates). An empty
            // `from` is rejected, like `Text.split`'s empty separator: it
            // would otherwise match at every boundary.
            Builtin::TextReplace => match args.as_slice() {
                [Value::String(from), Value::String(to), Value::String(s)] => {
                    if from.is_empty() {
                        return err("Text.replace needs a non-empty string to replace".to_string());
                    }
                    // Charge BEFORE allocating, in two parts, so neither a
                    // huge scan nor a huge output can be paid for after the
                    // fact:
                    //   1. the O(input) scan — even a SHRINKING replacement
                    //      reads every byte, and charging only the output
                    //      would let `Text.replace(big, "", s)` scan for
                    //      free;
                    //   2. the materialized output, which this GROWTH builtin
                    //      can make far larger than the input (`"a" -> "aaaa"`
                    //      quadruples per unit charge under the input-only
                    //      rule — the `List.append` doubling attack).
                    // The count is another O(input) pass but no allocation,
                    // and it makes the output size exact without building it.
                    self.charge(s.len() as u64, span)?;
                    let hits = s.matches(from.as_ref()).count();
                    let out_len = (s.len() + hits.saturating_mul(to.len()))
                        .saturating_sub(hits.saturating_mul(from.len()));
                    self.charge(out_len as u64, span)?;
                    let out = s.replace(from.as_ref(), to.as_ref());
                    Ok(Value::String(Rc::from(out.as_str())))
                }
                _ => err("Text.replace(from, to, s) expects three strings".to_string()),
            },
            // Strip leading and trailing WHITESPACE (Unicode's definition —
            // spaces, tabs, newlines), the same trim `Text.parseFloat` does.
            Builtin::TextTrim => match args.as_slice() {
                [Value::String(s)] => {
                    // O(input) scan: charge input bytes (the Text.split rule).
                    self.charge(s.len() as u64, span)?;
                    Ok(Value::String(Rc::from(s.trim())))
                }
                _ => err("Text.trim(s) expects one string".to_string()),
            },
            Builtin::MathClamp01 => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.clamp(0.0, 1.0))),
                _ => err("Math.clamp01(n) expects one number".to_string()),
            },
            // Subject-LAST — `Math.clamp(low, high, n)`, so the pipeline form
            // `n |> Math.clamp(0.0, 10.0)` reads as "clamp n into 0..10".
            // `Math.clamp01(n)` is exactly `Math.clamp(0.0, 1.0, n)` and is
            // kept as the ergonomic shorthand.
            //
            // An inverted range (`low > high`) is a caller bug — `f64::clamp`
            // would PANIC on it, and silently swapping the bounds would hide
            // the mistake — so it is a teaching error.
            Builtin::MathClamp => match args.as_slice() {
                [Value::Number(low), Value::Number(high), Value::Number(n)] if low <= high => {
                    Ok(Value::Number(n.clamp(*low, *high)))
                }
                [Value::Number(low), Value::Number(high), Value::Number(_)] => err(format!(
                    "Math.clamp(low, high, n) needs low <= high, got low {low} and high {high}"
                )),
                _ => err("Math.clamp(low, high, n) expects three numbers".to_string()),
            },
            // `Math.lerp(target, t, from)` = `from + (target - from) * t` —
            // UNCLAMPED, like `Vec3.lerp`: `t` outside [0, 1] extrapolates,
            // which is what an overshooting ease or a spring wants. Clamp the
            // parameter first (`t |> Math.clamp01 |> …`) for the bounded form.
            //
            // The argument order MIRRORS `Vec3.lerp(target, t, from)` exactly,
            // so the subject is the START value and the two pipe identically:
            // `x |> Math.lerp(target, t)` beside `pos |> Vec3.lerp(target, t)`.
            // That costs `mix`/`std::lerp` familiarity and buys the thing that
            // actually bites — carrying the vector habit to scalars can no
            // longer typecheck clean while meaning something else.
            Builtin::MathLerp => match args.as_slice() {
                // `from + (target - from) * t`, the standard form — but it is
                // exact only at `t == 0`: `from + (target - from)` rounds off
                // `target` for ~9% of random float pairs. `t == 1` is therefore
                // answered with `target` itself, so both endpoints land
                // EXACTLY (the `std::lerp` rule) and an ease that runs to
                // completion actually arrives.
                [Value::Number(target), Value::Number(t), Value::Number(_)] if *t == 1.0 => {
                    Ok(Value::Number(*target))
                }
                [Value::Number(target), Value::Number(t), Value::Number(from)] => {
                    Ok(Value::Number(from + (target - from) * t))
                }
                _ => err("Math.lerp(target, t, from) expects three numbers".to_string()),
            },
            // The standard (GLSL/Hermite) smoothstep: 0 below `edge0`, 1 above
            // `edge1`, and a smooth `3t² - 2t³` ramp between — CLAMPED, unlike
            // `Math.lerp` (an easing curve that overshoots its own edges is
            // never what a caller means).
            //
            // The range must be finite and ascending. A zero-width, inverted,
            // or NaN range is the obvious caller bug, but an infinite (or
            // merely astronomically wide) one is the same bug wearing a
            // disguise: `edge1 - edge0` overflows, the division answers NaN or
            // ±inf, and the "flat 0 / flat 1" contract quietly stops holding.
            // Both are a teaching error, exactly as an inverted range is for
            // `Math.clamp`.
            Builtin::MathSmoothstep => match args.as_slice() {
                [Value::Number(edge0), Value::Number(edge1), Value::Number(x)]
                    if edge0 < edge1 && (edge1 - edge0).is_finite() =>
                {
                    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
                    Ok(Value::Number(t * t * (3.0 - 2.0 * t)))
                }
                [Value::Number(edge0), Value::Number(edge1), Value::Number(_)] => err(format!(
                    "Math.smoothstep(edge0, edge1, x) needs a finite range with edge0 < edge1, \
got edge0 {edge0} and edge1 {edge1}"
                )),
                _ => err("Math.smoothstep(edge0, edge1, x) expects three numbers".to_string()),
            },
            // `Math.round` rounds HALF AWAY FROM ZERO (Rust's `f64::round`),
            // not to even: 0.5 -> 1, 2.5 -> 3, -0.5 -> -1, -2.5 -> -3. That
            // is the rule people predict when they round a score or a grid
            // coordinate, and it is symmetric about zero, so rounding a
            // position and its mirror image stays consistent.
            Builtin::MathRound => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.round())),
                _ => err("Math.round(n) expects one number".to_string()),
            },
            // `Math.sign` answers -1 / 0 / 1 — and crucially 0 FOR ZERO,
            // unlike Rust's `signum`, which calls -0.0 negative and answers
            // 1.0 for +0.0. A game asking "which way is this moving?" about
            // something at rest wants "neither". NaN has no sign, so it
            // propagates as NaN rather than collapsing to 0.
            Builtin::MathSign => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(if n.is_nan() {
                    f64::NAN
                } else if *n > 0.0 {
                    1.0
                } else if *n < 0.0 {
                    -1.0
                } else {
                    0.0
                })),
                _ => err("Math.sign(n) expects one number".to_string()),
            },
            // `Math.asin` / `Math.acos` return NaN OUTSIDE [-1, 1] rather
            // than clamping, so a genuinely wrong input stays visible. The
            // common trap is feeding in a dot product that floating-point
            // error pushed a hair past 1.0; the fix is to say so explicitly:
            // `dot |> Math.clamp(-1.0, 1.0) |> Math.acos`.
            //
            // `Math.log` is the NATURAL logarithm (base e), the inverse of
            // `Math.exp`; it is NaN below 0 and -inf at 0.
            Builtin::MathTan
            | Builtin::MathAsin
            | Builtin::MathAcos
            | Builtin::MathAtan
            | Builtin::MathCeil
            | Builtin::MathLog
            | Builtin::MathExp => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(match b {
                    Builtin::MathTan => n.tan(),
                    Builtin::MathAsin => n.asin(),
                    Builtin::MathAcos => n.acos(),
                    Builtin::MathAtan => n.atan(),
                    Builtin::MathCeil => n.ceil(),
                    Builtin::MathLog => n.ln(),
                    _ => n.exp(),
                })),
                _ => err(format!("{}(n) expects one number", builtin_name(b))),
            },
            Builtin::MathSin => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.sin())),
                _ => err("Math.sin(n) expects one number".to_string()),
            },
            Builtin::MathCos => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.cos())),
                _ => err("Math.cos(n) expects one number".to_string()),
            },
            Builtin::MathSqrt => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.sqrt())),
                _ => err("Math.sqrt(n) expects one number".to_string()),
            },
            Builtin::MathAbs => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.abs())),
                _ => err("Math.abs(n) expects one number".to_string()),
            },
            Builtin::MathFloor => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.floor())),
                _ => err("Math.floor(n) expects one number".to_string()),
            },
            // atan2(y, x) — the full-circle angle, following the standard
            // math argument order (y first).
            Builtin::MathAtan2 => match args.as_slice() {
                [Value::Number(y), Value::Number(x)] => Ok(Value::Number(y.atan2(*x))),
                _ => err("Math.atan2(y, x) expects two numbers".to_string()),
            },
            // Euclidean remainder: the result is always NON-NEGATIVE (in
            // `[0, abs(b))`), so negative inputs wrap positively
            // (`Math.mod(-1.0, 8.0)` == 7.0) — the wraparound games want.
            // `b == 0.0` yields NaN (IEEE); the engine boundary rejects
            // non-finite numbers.
            Builtin::MathMod => match args.as_slice() {
                [Value::Number(a), Value::Number(b)] => Ok(Value::Number(a.rem_euclid(*b))),
                _ => err("Math.mod(a, b) expects two numbers".to_string()),
            },
            Builtin::MathMin => match args.as_slice() {
                [Value::Number(a), Value::Number(b)] => Ok(Value::Number(a.min(*b))),
                _ => err("Math.min(a, b) expects two numbers".to_string()),
            },
            Builtin::MathMax => match args.as_slice() {
                [Value::Number(a), Value::Number(b)] => Ok(Value::Number(a.max(*b))),
                _ => err("Math.max(a, b) expects two numbers".to_string()),
            },
            // pow(base, exp) == base ^ exp (standard math argument order).
            Builtin::MathPow => match args.as_slice() {
                [Value::Number(base), Value::Number(exp)] => Ok(Value::Number(base.powf(*exp))),
                _ => err("Math.pow(base, exp) expects two numbers".to_string()),
            },
            // `Math.pi` is a constant resolved directly to its value in `eval`;
            // it is never a callable, so reaching here means it was applied.
            Builtin::MathPi => err("Math.pi is a constant, not a function".to_string()),
            // Pure seeded PRNG: `Random.step(seed) => (value, nextSeed)` with
            // value in [0, 1). Deterministic (same seed → same stream) with no
            // effect round-trip, so it fits the functional core and runs
            // headlessly. Thread `nextSeed` through the model to advance.
            Builtin::RandomStep => match args.as_slice() {
                [Value::Number(seed)] if seed.is_finite() => {
                    let (value, next) = random_step(*seed);
                    Ok(Value::Tuple(Rc::new(vec![
                        Value::Number(value),
                        Value::Number(next),
                    ])))
                }
                _ => err("Random.step(seed) expects one finite number".to_string()),
            },
            // `Random.seed(n)` — make a Seed from any finite number by
            // hashing its BITS (see `seed_counter`); the branded entry point
            // to the PRNG. At runtime a Seed is a plain number (the brand is
            // check-time only), which keeps seeds plain data for time-travel
            // snapshots, hot-reload preservation, and `/state`.
            Builtin::RandomSeed => match args.as_slice() {
                [Value::Number(n)] if n.is_finite() => Ok(Value::Number(seed_counter(*n) as f64)),
                _ => err("Random.seed(n) expects one finite number".to_string()),
            },
            // `Random.fork(i, seed)` — the seed of decorrelated child stream
            // `i`: the typed form of the old `baseSeed + i` per-entity idiom.
            // Subject-LAST, so it pipes: `model.seed |> Random.fork(i)`.
            Builtin::RandomFork => match args.as_slice() {
                [Value::Number(i), Value::Number(seed)] if i.is_finite() && seed.is_finite() => {
                    Ok(Value::Number(fork_seed(*i, *seed)))
                }
                _ => err("Random.fork(i, seed) expects two finite numbers".to_string()),
            },
            // Convenience: `Random.range(lo, hi, seed) => (value, nextSeed)`
            // with value in [lo, hi) for lo <= hi (one `Random.step` draw
            // rescaled).
            Builtin::RandomRange => match args.as_slice() {
                [Value::Number(lo), Value::Number(hi), Value::Number(seed)]
                    if lo.is_finite() && hi.is_finite() && seed.is_finite() =>
                {
                    let (u, next) = random_step(*seed);
                    // Lerp form `lo(1-u) + hi·u` rather than `lo + u(hi-lo)`:
                    // it stays finite even for extreme finite bounds (the
                    // `hi - lo` difference could overflow to ±inf) and is still
                    // < hi for u in [0, 1) when lo < hi.
                    Ok(Value::Tuple(Rc::new(vec![
                        Value::Number(lo * (1.0 - u) + hi * u),
                        Value::Number(next),
                    ])))
                }
                _ => err("Random.range(lo, hi, seed) expects three finite numbers".to_string()),
            },
            // Elm-style trace (`Debug.log : String -> a -> a`): log
            // `label: <subject>` through the process-wide trace sink and return
            // the SUBJECT unchanged — an impure observability escape hatch that
            // never touches the model/sim (so a game with vs without a
            // `Debug.log` produces byte-identical state). Label-first /
            // subject-LAST, so it reads naturally standalone
            // (`Debug.log("hp", m.hp)`) AND threads in a pipe
            // (`m.hp |> Debug.log("hp")`). The subject renders with the
            // interpreter's own `Value` display — the same text as
            // `functor-lang run`/`trace` — for any type. The sink decides where the line
            // goes (stdout under plain `functor-lang run`, the region-aware log path
            // under the host); see `crate::trace`.
            Builtin::DebugLog => match args.as_slice() {
                [Value::String(label), subject] => {
                    crate::trace::emit(format!("{label}: {subject}"));
                    Ok(subject.clone())
                }
                _ => err(
                    "Debug.log(label, subject) expects a string label and a value".to_string(),
                ),
            },
        }
    }
}

/// A value's BRAND identity for unit-operator dispatch: a variant's
/// constructor, or an opaque host value's type name (`Angle`, `Duration`).
/// Everything else — numbers, strings, records, collections — carries no tag
/// an operator could be declared against, so it never dispatches.
pub(crate) fn brand_tag(value: &Value) -> Option<&str> {
    match value {
        Value::Variant { ctor, .. } => Some(ctor),
        Value::HostData(host) => Some(host.type_name()),
        _ => None,
    }
}

/// Coerce a value to a bool for the boolean operators (`&&`, `||`, `not`),
/// erroring at `span` on anything else.
fn as_bool(value: &Value, span: Span) -> Result<bool, RunError> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(RunError {
            message: format!("boolean operator needs a bool, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Does `pattern` match `value`? Appends each pattern variable's binding on
/// the way (bindings are only used if the whole pattern matched — a pattern
/// either fully matches or its arm is skipped, and sub-patterns are leaves,
/// so a partial append can never observe a half-match). Pure: literal
/// patterns compare primitively, so no function-equality error can arise.
fn match_pattern(pattern: &Pattern, value: &Value, vars: &mut Vec<(BindingId, Value)>) -> bool {
    match &pattern.kind {
        PatternKind::Wildcard => true,
        PatternKind::Var { binding, .. } => {
            vars.push((*binding, value.clone()));
            true
        }
        PatternKind::Ctor { name, args } => match value {
            // Same constructor AND same arg count: lowering fixes the
            // pattern's arity to the declaration, but the VALUE may come
            // from a host (`Session::call`) — mismatched args are a
            // non-match, not UB.
            Value::Variant { ctor, args: vals } if ctor.as_ref() == name => {
                args.len() == vals.len()
                    && args
                        .iter()
                        .zip(vals.iter())
                        .all(|(p, v)| match_pattern(p, v, vars))
            }
            _ => false,
        },
        // Arity must match exactly — a 2-pattern against a 3-tuple is a
        // non-match, like a mismatched ctor.
        PatternKind::Tuple(args) => match value {
            Value::Tuple(vals) => {
                args.len() == vals.len()
                    && args
                        .iter()
                        .zip(vals.iter())
                        .all(|(p, v)| match_pattern(p, v, vars))
            }
            _ => false,
        },
        // `[a, b]` needs an exact-length list; `[h, ..t]` needs at least
        // `items.len()` and binds `t` to the remainder.
        PatternKind::List { items, tail } => match value {
            Value::List(vals) => {
                let long_enough = match tail {
                    Some(_) => vals.len() >= items.len(),
                    None => vals.len() == items.len(),
                };
                if !long_enough {
                    return false;
                }
                if !items
                    .iter()
                    .zip(vals.iter())
                    .all(|(p, v)| match_pattern(p, v, vars))
                {
                    return false;
                }
                if let Some(tail) = tail {
                    let rest = Value::List(Rc::new(vals[items.len()..].to_vec()));
                    match_pattern(tail, &rest, vars)
                } else {
                    true
                }
            }
            _ => false,
        },
        PatternKind::Number(n) => matches!(value, Value::Number(v) if v == n),
        PatternKind::Bool(b) => matches!(value, Value::Bool(v) if v == b),
        PatternKind::String(s) => matches!(value, Value::String(v) if v.as_ref() == s),
    }
}

/// Each pattern variable's `(binding, name, span)` — the recorder's per-site
/// key/label/location for match binders (the `Var` pattern's own span is the
/// name's span), mirroring `goto::pattern_binders`'s traversal.
fn pattern_binder_sites<'a>(pattern: &'a Pattern, out: &mut Vec<(BindingId, &'a str, Span)>) {
    match &pattern.kind {
        PatternKind::Var { binding, name } => out.push((*binding, name, pattern.span)),
        PatternKind::Ctor { args, .. } | PatternKind::Tuple(args) => {
            for arg in args {
                pattern_binder_sites(arg, out);
            }
        }
        PatternKind::List { items, tail } => {
            for item in items {
                pattern_binder_sites(item, out);
            }
            if let Some(tail) = tail {
                pattern_binder_sites(tail, out);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// Structural equality for `==`. Functions have no equality — comparing them
/// is a runtime error rather than a silent `false`.
/// `compared` counts the value nodes visited — the Eq arm charges it
/// against the step budget, since a single `==` on a large value is O(size)
/// work that no call-path charge sees (review probe: repeated big-list
/// compares did ~9x10^8 uncharged comparisons under a 10^6 budget).
/// Iterative (explicit pair worklist), NOT recursive: value depth is
/// user-controlled (an iteratively-built `[acc]` nest goes far past any
/// recursion limit), and this runs inside editor/tooling processes where a
/// stack overflow is a host crash. Pairs push in reverse so comparison
/// order stays left-to-right (first mismatch/error is the leftmost).
fn value_eq(
    a: &Value,
    b: &Value,
    span: Span,
    compared: &mut u64,
    // The operator being evaluated (`==` or `!=`) — errors name the one the
    // source actually wrote.
    op: &str,
) -> Result<bool, RunError> {
    /// Pending work. `MissingField` stands in for a record field whose name
    /// the other record lacks — pushed IN POSITION so the not-equal verdict
    /// lands in field order (an earlier field's function-comparison error
    /// still fires first, exactly like the old interleaved walk).
    enum Work<'a> {
        Pair(&'a Value, &'a Value),
        Key(&'a MapKey, &'a MapKey),
        MissingField,
    }
    let mut work: Vec<Work> = vec![Work::Pair(a, b)];
    while let Some(item) = work.pop() {
        let (a, b) = match item {
            Work::Pair(a, b) => (a, b),
            Work::Key(a, b) => {
                *compared += 1;
                if a != b {
                    return Ok(false);
                }
                continue;
            }
            Work::MissingField => return Ok(false),
        };
        *compared += 1;
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                if x != y {
                    return Ok(false);
                }
            }
            (Value::String(x), Value::String(y)) => {
                if x != y {
                    return Ok(false);
                }
            }
            (Value::Bool(x), Value::Bool(y)) => {
                if x != y {
                    return Ok(false);
                }
            }
            // Structural, element-wise; arity difference is simply unequal.
            (Value::List(xs), Value::List(ys)) | (Value::Tuple(xs), Value::Tuple(ys)) => {
                if xs.len() != ys.len() {
                    return Ok(false);
                }
                work.extend(xs.iter().zip(ys.iter()).rev().map(|(x, y)| Work::Pair(x, y)));
            }
            (Value::Map(xs), Value::Map(ys)) => {
                if xs.len() != ys.len() {
                    return Ok(false);
                }
                for ((xk, xv), (yk, yv)) in xs.iter().zip(ys.iter()).rev() {
                    work.push(Work::Pair(xv, yv));
                    work.push(Work::Key(xk, yk));
                }
            }
            (Value::Record(xs), Value::Record(ys)) => {
                if xs.len() != ys.len() {
                    return Ok(false);
                }
                for (name, x) in xs.iter().rev() {
                    match ys.iter().find(|(n, _)| n == name) {
                        Some((_, y)) => work.push(Work::Pair(x, y)),
                        None => work.push(Work::MissingField),
                    }
                }
            }
            // Structural: same constructor, equal args (a function argument
            // still raises the function-comparison error below).
            (Value::Variant { ctor: xc, args: xs }, Value::Variant { ctor: yc, args: ys }) => {
                if xc != yc || xs.len() != ys.len() {
                    return Ok(false);
                }
                work.extend(xs.iter().zip(ys.iter()).rev().map(|(x, y)| Work::Pair(x, y)));
            }
            // An unapplied constructor is a function value — no equality.
            (
                Value::Closure(_)
                | Value::Partial(_)
                | Value::Builtin(_)
                | Value::HostFn(_)
                | Value::Ctor { .. },
                _,
            )
            | (
                _,
                Value::Closure(_)
                | Value::Partial(_)
                | Value::Builtin(_)
                | Value::HostFn(_)
                | Value::Ctor { .. },
            ) => {
                return Err(RunError {
                    message: format!("functions cannot be compared with `{op}`"),
                    span,
                })
            }
            (Value::HostData(_), _) | (_, Value::HostData(_)) => {
                return Err(RunError {
                    message: format!("host values cannot be compared with `{op}`"),
                    span,
                })
            }
            // Different kinds are simply unequal (structural, not typed — B4
            // adds the typechecker that would reject this statically).
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// The trace label for a call site, from its callee's IR shape. Also used by
/// the typechecker ([`crate::types`]) so its call diagnostics name callees
/// the same way.
pub(crate) fn callee_label(callee: &Expr) -> String {
    match &callee.kind {
        ExprKind::Global(name) => name.clone(),
        ExprKind::Local { name, .. } => name.clone(),
        ExprKind::External(path) => path.join("."),
        ExprKind::Ctor { name, .. } => name.clone(),
        _ => "<lambda>".to_string(),
    }
}

/// The ordering `List.sortBy` imposes on its float keys: **ascending, with
/// every NaN key last**.
///
/// It exists because neither obvious choice is acceptable:
///
///   * `f64::total_cmp` is a total order, but it orders by the SIGN BIT, so
///     a negative NaN sorts first and a positive NaN sorts last. IEEE 754
///     does not specify the sign of a COMPUTED NaN — `0.0 / 0.0` is `-NaN`
///     on x86 and `+NaN` on aarch64 — so the same program sorted the same
///     list differently on Linux and macOS. For an engine whose replay is
///     supposed to be deterministic, that is a bug.
///   * `partial_cmp(..).unwrap_or(Equal)` is an INCONSISTENT comparator when
///     NaN is present (NaN compares Equal to everything while the non-NaN
///     elements still order among themselves), which lets a merge sort
///     scramble unrelated elements.
///
/// So NaN is normalized: all NaNs compare Equal to each other and Greater
/// than every real number, whatever their sign bit. Because they compare
/// Equal, the stable sort keeps NaN-keyed elements in their input order too.
///
/// `-0.0` and `+0.0` compare EQUAL here (they are numerically equal, and
/// `total_cmp` would have split them) — so stability, not the sign of zero,
/// decides their order.
fn sort_key_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        // Neither is NaN, so `partial_cmp` is always `Some`.
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// Validate and canonicalize one public Map key. `-0.0` becomes `0.0` so
/// language equality, lookup, and ordering all agree; non-finite floats are
/// refused because NaN has no equality-compatible ordering and cannot cross
/// the plain-data wire.
fn map_key_from_value(value: &Value, operation: &str) -> Result<MapKey, String> {
    match value {
        Value::Bool(value) => Ok(MapKey::Bool(*value)),
        Value::Number(value) if value.is_finite() => Ok(MapKey::Number(*value + 0.0)),
        Value::Number(value) => Err(format!(
            "{operation} keys must be finite; got {value} (NaN/Infinity cannot be map keys)"
        )),
        Value::String(value) => Ok(MapKey::String(value.clone())),
        other => Err(format!(
            "{operation} keys must be bools, finite floats, or strings; got {}",
            other.kind_name()
        )),
    }
}

/// Conservative comparison ceiling for Rust's stable O(n log n) slice sort.
///
/// `Map.fromList` uses this only as a non-consuming preflight; it charges the
/// actual comparison work afterward. The factor of two leaves headroom for
/// run detection/merge bookkeeping while keeping realistic editor budgets
/// proportional to the work they permit.
fn stable_sort_comparison_ceiling(len: usize) -> u64 {
    let n = u64::try_from(len).unwrap_or(u64::MAX);
    if n < 2 {
        return 0;
    }
    let levels = u64::from(u64::BITS - (n - 1).leading_zeros());
    n.saturating_mul(levels).saturating_mul(2)
}

/// Wrap a "maybe absent" result as the language's `Option.t` — the shared
/// answer of every PARTIAL builtin (`List.nth`/`head`/`last`/`find`/`maximum`
/// and `Map.get`).
///
/// The constructor names are the QUALIFIED ones the `Option` module's
/// declarations lower to (`crate::project`'s bundled `stdlib/option.fun`), so
/// a value built here is indistinguishable from one a `.fun` file wrote as
/// `Option.Some(x)` — the same `match`, the same `Option.map`/`defaultValue`,
/// structural equality between the two. Nullary constructors are variants
/// with no arguments, so `None` is a `Variant`, never a `Ctor`.
fn option_value(v: Option<Value>) -> Value {
    match v {
        Some(v) => Value::Variant {
            ctor: Rc::from("Option.Some"),
            args: Rc::new(vec![v]),
        },
        None => Value::Variant {
            ctor: Rc::from("Option.None"),
            args: Rc::new(Vec::new()),
        },
    }
}

/// The trace label for a builtin invoking its function argument on element
/// `i` (`List.map[2]`).
fn element_label(b: Builtin, i: usize) -> String {
    format!("{}[{i}]", builtin_name(b))
}

/// Is `v` something `self.call` can apply? (Used to validate a callback
/// argument up front, before a loop, rather than mid-iteration.)
fn is_function(v: &Value) -> bool {
    matches!(
        v,
        Value::Closure(_)
            | Value::Partial(_)
            | Value::Builtin(_)
            | Value::HostFn(_)
            | Value::Ctor { .. }
    )
}

/// The statically-known arity of a callable, or `None` when it isn't known
/// here (host functions — the host validates its own arg count; partials are
/// unwrapped before this is consulted). Drives partial vs saturated vs
/// over-application in [`Interp::call`].
fn callee_arity(v: &Value) -> Option<usize> {
    match v {
        Value::Closure(c) => Some(c.params.len()),
        Value::Ctor { arity, .. } => Some(*arity),
        Value::Builtin(b) => Some(builtin_arity(*b)),
        _ => None,
    }
}

/// The wrong-argument-count message for a constructor application, shared by
/// the checker ([`crate::types`], where `via` is always `None`) and the
/// interpreter so the two can never word it differently. Constructors do not
/// curry: unlike a function, under-applying one is an error at the call rather
/// than a partial (OCaml/F# do the same), so the message says how to stage
/// arguments on purpose.
pub(crate) fn ctor_arity_message(
    name: &str,
    arity: usize,
    got: usize,
    via: Option<&str>,
) -> String {
    let who = match via {
        Some(builtin) => format!("the constructor `{name}` passed to {builtin}"),
        None => format!("`{name}`"),
    };
    let mut message =
        format!("{who} takes {arity} argument(s), got {got} — constructors apply fully");
    // The staging hint only helps at a call the author wrote: a builtin that
    // under-applied a ctor callback needs a different function, not a lambda
    // around the same missing value.
    if got < arity && via.is_none() {
        message.push_str("; wrap it in a lambda to stage arguments (e.g. `(x) => ");
        message.push_str(name);
        message.push_str("(…, x)`)");
    }
    message
}

/// A builtin's argument count — currying needs each builtin's arity to decide
/// partial vs saturated vs over-application. Must stay in sync with
/// [`Interp::call_builtin`] (and [`crate::types::builtin_signature`]).
pub fn builtin_arity(b: Builtin) -> usize {
    match b {
        Builtin::ListFold
        | Builtin::ListGrid
        | Builtin::RandomRange
        | Builtin::TextReplace
        | Builtin::MathClamp
        | Builtin::MathLerp
        | Builtin::MathSmoothstep
        | Builtin::MapInsert => 3,
        Builtin::ListMap
        | Builtin::ListFilter
        | Builtin::ListAppend
        | Builtin::ListAny
        | Builtin::ListAll
        | Builtin::ListNth
        | Builtin::ListFind
        | Builtin::ListIndexedMap
        | Builtin::ListSortBy
        | Builtin::ListZip
        | Builtin::ListTake
        | Builtin::ListDrop
        | Builtin::ListConcatMap
        | Builtin::TextConcat
        | Builtin::TextFixed
        | Builtin::TextSplit
        | Builtin::TextJoin
        | Builtin::TextContains
        | Builtin::MathAtan2
        | Builtin::MathMod
        | Builtin::MathMin
        | Builtin::MathMax
        | Builtin::MathPow
        | Builtin::RandomFork
        | Builtin::DebugLog
        | Builtin::MapGet
        | Builtin::MapRemove
        | Builtin::MapMember => 2,
        Builtin::ListRange
        | Builtin::ListMaximum
        | Builtin::ListMinimum
        | Builtin::ListLength
        | Builtin::ListFlatten
        | Builtin::ListReverse
        | Builtin::ListIsEmpty
        | Builtin::ListHead
        | Builtin::ListLast
        | Builtin::ListSum
        | Builtin::TextFromFloat
        | Builtin::TextToBullets
        | Builtin::TextParseFloat
        | Builtin::TextLength
        | Builtin::TextChars
        | Builtin::TextToUpper
        | Builtin::TextToLower
        | Builtin::TextTrim
        | Builtin::MathClamp01
        | Builtin::MathSin
        | Builtin::MathCos
        | Builtin::MathSqrt
        | Builtin::MathAbs
        | Builtin::MathFloor
        | Builtin::MathTan
        | Builtin::MathAsin
        | Builtin::MathAcos
        | Builtin::MathAtan
        | Builtin::MathRound
        | Builtin::MathCeil
        | Builtin::MathLog
        | Builtin::MathExp
        | Builtin::MathSign
        | Builtin::MapValues
        | Builtin::MapToList
        | Builtin::MapFromList => 1,
        // `Math.pi` resolves straight to a number in `eval` (it's a constant,
        // never a callable value), so this arity is never consulted.
        Builtin::MathPi | Builtin::MapEmpty => 0,
        Builtin::RandomSeed | Builtin::RandomStep => 1,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Builtin {
    ListMap,
    ListFilter,
    ListFold,
    ListRange,
    ListGrid,
    MathSin,
    MathCos,
    MathSqrt,
    MathAbs,
    MathFloor,
    MathAtan2,
    MathMod,
    MathMin,
    MathMax,
    MathPow,
    MathPi,
    ListMaximum,
    ListMinimum,
    ListLength,
    ListAppend,
    ListFlatten,
    ListAny,
    ListAll,
    ListReverse,
    ListIsEmpty,
    ListNth,
    ListHead,
    ListLast,
    ListFind,
    ListIndexedMap,
    ListSortBy,
    ListZip,
    ListTake,
    ListDrop,
    ListSum,
    ListConcatMap,
    MapEmpty,
    MapGet,
    MapInsert,
    MapRemove,
    MapMember,
    MapValues,
    MapToList,
    MapFromList,
    TextConcat,
    TextFromFloat,
    TextFixed,
    TextToBullets,
    TextSplit,
    TextJoin,
    TextParseFloat,
    TextLength,
    TextChars,
    TextToUpper,
    TextToLower,
    TextContains,
    TextReplace,
    TextTrim,
    MathClamp01,
    MathClamp,
    MathLerp,
    MathSmoothstep,
    MathTan,
    MathAsin,
    MathAcos,
    MathAtan,
    MathRound,
    MathCeil,
    MathLog,
    MathExp,
    MathSign,
    RandomSeed,
    RandomStep,
    RandomRange,
    RandomFork,
    DebugLog,
}

/// Pure seeded PRNG — the splitmix64 finalizer over a 52-bit Weyl counter.
///
/// Functor Lang numbers are f64, so the seed we hand back to the language has
/// to round-trip *exactly*: we keep the counter in `[0, 2^52)` (a comfortable
/// margin inside the 2^53 exact-integer range of f64, so `nextSeed` survives
/// the trip through the language's number type without rounding) and do all
/// mixing in `u64`, which is bit-identical on native and wasm. Each step
/// advances the counter by an odd golden-ratio gamma (a large stride, *not*
/// +1) and runs the result through the splitmix64 finalizer. Adjacent seeds
/// are different *phases* of the one full-period cycle; the large stride means
/// their draw sequences share no element within any game-length prefix (the
/// residues collide only after ~2^51 steps), and the finalizer decorrelates
/// their outputs — together that's the anti-correlation property the sin-hash
/// noise lacked.
///
/// Returns `(value, nextSeed)` with `value` in `[0, 1)`.
fn random_step(seed: f64) -> (f64, f64) {
    // Odd 52-bit golden-ratio gamma — a large stride with full period 2^52.
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7;

    // Fold whatever came in down to the counter range. Values WE produced are
    // already non-negative integers in `[0, 2^52)`, for which `rem_euclid` is
    // the identity — so the threaded-seed round-trip is exact. A caller-chosen
    // seed is reduced mod 2^52: `rem_euclid` (unlike a saturating `as u64`
    // cast, which maps every negative to 0) wraps negatives to DISTINCT
    // counters. Fractional seeds truncate — the counter is integral.
    let counter = seed.rem_euclid(TWO52) as u64 & MASK52;
    let next = counter.wrapping_add(GAMMA) & MASK52;

    // The finalizer avalanches the (52-bit) Weyl state into 64 bits.
    let z = mix64(next);

    // Top 53 bits → a double in [0, 1) (the standard splitmix64→f64 mapping).
    let value = (z >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0); // 2^53
    (value, next as f64)
}

const MASK52: u64 = (1u64 << 52) - 1;
const TWO52: f64 = 4_503_599_627_370_496.0; // 2^52

/// The splitmix64 finalizer — shared by [`random_step`], [`seed_counter`],
/// and [`fork_seed`].
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `Random.seed(n)` — fold any finite f64 into a counter by hashing its BIT
/// PATTERN, not its integer value. `random_step`'s raw fold truncates
/// fractional seeds, so every seed in `[0, 1)` — exactly what the documented
/// `Effect.random` seeding idiom delivers — collapsed to counter 0 (the same
/// stream every run). Hashing the bits gives every distinct float a
/// decorrelated starting point.
fn seed_counter(n: f64) -> u64 {
    // `+ 0.0` normalizes -0.0 to +0.0 so the two equal zeros seed alike.
    mix64((n + 0.0).to_bits()) & MASK52
}

/// `Random.fork(i, seed)` — the seed of child stream `i`, hash-combined so
/// ANY float index names a distinct decorrelated stream (no truncation edge
/// cases). The typed replacement for the old `baseSeed + i` arithmetic,
/// which an opaque seed no longer permits.
fn fork_seed(i: f64, seed: f64) -> f64 {
    // Offsetting the index bits domain-separates the combine: mix64(0) == 0,
    // so without it the all-zero corner `Random.fork(0.0, Random.seed(0.0))`
    // would fix-point back to the parent seed.
    const FORK_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
    let counter = seed.rem_euclid(TWO52) as u64 & MASK52;
    let index = mix64((i + 0.0).to_bits().wrapping_add(FORK_GAMMA));
    (mix64(counter ^ index) & MASK52) as f64
}

/// Resolve an [`ExprKind::External`] path against the registry.
/// The error for an external that resolved nowhere. A typo'd member of a
/// BUILTIN namespace (`List.fooo`) is a user error everywhere — it must not
/// read (or classify — see [`ExpectOutcome::status`]) like a host external
/// that merely isn't available in this embedding. `#[cold]`: keeps the
/// formatting locals out of `eval_inner`'s recursion frame.
#[cold]
#[inline(never)]
fn unknown_external_error(path: &[String], joined: String, span: Span) -> RunError {
    let message = match path.first() {
        Some(head) if BUILTIN_NAMESPACES.contains(&head.as_str()) => {
            format!("`{head}` has no builtin `{}`", path[1..].join("."))
        }
        _ => format!("unknown external `{joined}`"),
    };
    RunError { message, span }
}

/// The namespaces the BUILTIN registry owns: an unknown member of one of
/// these is a typo (a plain error), never a host-provided external — the
/// distinction the unknown-external error message (and through it the
/// expect gutter's `unrunnable` classification) rests on.
pub const BUILTIN_NAMESPACES: &[&str] = &["List", "Map", "Text", "Math", "Random", "Debug"];

/// The complete builtin registry as a list. Hand-listed because [`Builtin`] is
/// not iterable — keep in sync with the enum (`builtins_list_is_exhaustive`
/// fails to COMPILE otherwise, and `all_builtins_round_trip_through_dispatch`
/// fails if an entry stops dispatching). Lives next to
/// [`builtin`] so the members the CHECKER knows about (for its
/// unknown-member diagnostic and its suggestions) and the ones the
/// interpreter DISPATCHES come from one place.
pub const ALL_BUILTINS: [Builtin; 76] = [
    Builtin::ListMap,
    Builtin::ListFilter,
    Builtin::ListFold,
    Builtin::ListRange,
    Builtin::ListGrid,
    Builtin::ListMaximum,
    Builtin::ListMinimum,
    Builtin::ListLength,
    Builtin::ListAppend,
    Builtin::ListFlatten,
    Builtin::ListAny,
    Builtin::ListAll,
    Builtin::ListReverse,
    Builtin::ListIsEmpty,
    Builtin::ListNth,
    Builtin::ListHead,
    Builtin::ListLast,
    Builtin::ListFind,
    Builtin::ListIndexedMap,
    Builtin::ListSortBy,
    Builtin::ListZip,
    Builtin::ListTake,
    Builtin::ListDrop,
    Builtin::ListSum,
    Builtin::ListConcatMap,
    Builtin::MapEmpty,
    Builtin::MapGet,
    Builtin::MapInsert,
    Builtin::MapRemove,
    Builtin::MapMember,
    Builtin::MapValues,
    Builtin::MapToList,
    Builtin::MapFromList,
    Builtin::MathSin,
    Builtin::MathCos,
    Builtin::MathSqrt,
    Builtin::MathAbs,
    Builtin::MathFloor,
    Builtin::MathAtan2,
    Builtin::MathMod,
    Builtin::MathMin,
    Builtin::MathMax,
    Builtin::MathPow,
    Builtin::MathPi,
    Builtin::TextConcat,
    Builtin::TextFromFloat,
    Builtin::TextFixed,
    Builtin::TextToBullets,
    Builtin::TextSplit,
    Builtin::TextJoin,
    Builtin::TextParseFloat,
    Builtin::TextLength,
    Builtin::TextChars,
    Builtin::TextToUpper,
    Builtin::TextToLower,
    Builtin::TextContains,
    Builtin::TextReplace,
    Builtin::TextTrim,
    Builtin::MathClamp01,
    Builtin::MathClamp,
    Builtin::MathLerp,
    Builtin::MathSmoothstep,
    Builtin::MathTan,
    Builtin::MathAsin,
    Builtin::MathAcos,
    Builtin::MathAtan,
    Builtin::MathRound,
    Builtin::MathCeil,
    Builtin::MathLog,
    Builtin::MathExp,
    Builtin::MathSign,
    Builtin::RandomSeed,
    Builtin::RandomStep,
    Builtin::RandomRange,
    Builtin::RandomFork,
    Builtin::DebugLog,
];

/// The member names a builtin namespace owns (`List` → `map`, `filter`, …),
/// sorted, derived from [`ALL_BUILTINS`].
pub fn builtin_members(namespace: &str) -> Vec<&'static str> {
    let prefix = format!("{namespace}.");
    let mut members: Vec<&'static str> = ALL_BUILTINS
        .iter()
        .filter_map(|b| builtin_name(*b).strip_prefix(&prefix))
        .collect();
    members.sort_unstable();
    members
}

pub fn builtin(path: &[String]) -> Option<Builtin> {
    let joined = path.join(".");
    Some(match joined.as_str() {
        "List.map" => Builtin::ListMap,
        "List.filter" => Builtin::ListFilter,
        "List.fold" => Builtin::ListFold,
        "List.range" => Builtin::ListRange,
        "List.grid" => Builtin::ListGrid,
        "List.maximum" => Builtin::ListMaximum,
        "List.minimum" => Builtin::ListMinimum,
        "List.length" => Builtin::ListLength,
        "List.append" => Builtin::ListAppend,
        "List.flatten" => Builtin::ListFlatten,
        "List.any" => Builtin::ListAny,
        "List.all" => Builtin::ListAll,
        "List.reverse" => Builtin::ListReverse,
        "List.isEmpty" => Builtin::ListIsEmpty,
        "List.nth" => Builtin::ListNth,
        "List.head" => Builtin::ListHead,
        "List.last" => Builtin::ListLast,
        "List.find" => Builtin::ListFind,
        "List.indexedMap" => Builtin::ListIndexedMap,
        "List.sortBy" => Builtin::ListSortBy,
        "List.zip" => Builtin::ListZip,
        "List.take" => Builtin::ListTake,
        "List.drop" => Builtin::ListDrop,
        "List.sum" => Builtin::ListSum,
        "List.concatMap" => Builtin::ListConcatMap,
        "Map.empty" => Builtin::MapEmpty,
        "Map.get" => Builtin::MapGet,
        "Map.insert" => Builtin::MapInsert,
        "Map.remove" => Builtin::MapRemove,
        "Map.member" => Builtin::MapMember,
        "Map.values" => Builtin::MapValues,
        "Map.toList" => Builtin::MapToList,
        "Map.fromList" => Builtin::MapFromList,
        "Text.concat" => Builtin::TextConcat,
        "Text.fromFloat" => Builtin::TextFromFloat,
        "Text.fixed" => Builtin::TextFixed,
        "Text.toBullets" => Builtin::TextToBullets,
        "Text.split" => Builtin::TextSplit,
        "Text.join" => Builtin::TextJoin,
        "Text.parseFloat" => Builtin::TextParseFloat,
        "Text.length" => Builtin::TextLength,
        "Text.chars" => Builtin::TextChars,
        "Text.toUpper" => Builtin::TextToUpper,
        "Text.toLower" => Builtin::TextToLower,
        "Text.contains" => Builtin::TextContains,
        "Text.replace" => Builtin::TextReplace,
        "Text.trim" => Builtin::TextTrim,
        "Math.clamp01" => Builtin::MathClamp01,
        "Math.clamp" => Builtin::MathClamp,
        "Math.lerp" => Builtin::MathLerp,
        "Math.smoothstep" => Builtin::MathSmoothstep,
        "Math.tan" => Builtin::MathTan,
        "Math.asin" => Builtin::MathAsin,
        "Math.acos" => Builtin::MathAcos,
        "Math.atan" => Builtin::MathAtan,
        "Math.round" => Builtin::MathRound,
        "Math.ceil" => Builtin::MathCeil,
        "Math.log" => Builtin::MathLog,
        "Math.exp" => Builtin::MathExp,
        "Math.sign" => Builtin::MathSign,
        "Math.sin" => Builtin::MathSin,
        "Math.cos" => Builtin::MathCos,
        "Math.sqrt" => Builtin::MathSqrt,
        "Math.abs" => Builtin::MathAbs,
        "Math.floor" => Builtin::MathFloor,
        "Math.atan2" => Builtin::MathAtan2,
        "Math.mod" => Builtin::MathMod,
        "Math.min" => Builtin::MathMin,
        "Math.max" => Builtin::MathMax,
        "Math.pow" => Builtin::MathPow,
        "Math.pi" => Builtin::MathPi,
        "Random.seed" => Builtin::RandomSeed,
        "Random.step" => Builtin::RandomStep,
        "Random.range" => Builtin::RandomRange,
        "Random.fork" => Builtin::RandomFork,
        "Debug.log" => Builtin::DebugLog,
        _ => return None,
    })
}

/// The registry name of a builtin (`List.map`), for display.
pub fn builtin_name(b: Builtin) -> &'static str {
    match b {
        Builtin::ListMap => "List.map",
        Builtin::ListFilter => "List.filter",
        Builtin::ListFold => "List.fold",
        Builtin::ListRange => "List.range",
        Builtin::ListGrid => "List.grid",
        Builtin::ListMaximum => "List.maximum",
        Builtin::ListMinimum => "List.minimum",
        Builtin::ListLength => "List.length",
        Builtin::ListAppend => "List.append",
        Builtin::ListFlatten => "List.flatten",
        Builtin::ListAny => "List.any",
        Builtin::ListAll => "List.all",
        Builtin::ListReverse => "List.reverse",
        Builtin::ListIsEmpty => "List.isEmpty",
        Builtin::ListNth => "List.nth",
        Builtin::ListHead => "List.head",
        Builtin::ListLast => "List.last",
        Builtin::ListFind => "List.find",
        Builtin::ListIndexedMap => "List.indexedMap",
        Builtin::ListSortBy => "List.sortBy",
        Builtin::ListZip => "List.zip",
        Builtin::ListTake => "List.take",
        Builtin::ListDrop => "List.drop",
        Builtin::ListSum => "List.sum",
        Builtin::ListConcatMap => "List.concatMap",
        Builtin::MapEmpty => "Map.empty",
        Builtin::MapGet => "Map.get",
        Builtin::MapInsert => "Map.insert",
        Builtin::MapRemove => "Map.remove",
        Builtin::MapMember => "Map.member",
        Builtin::MapValues => "Map.values",
        Builtin::MapToList => "Map.toList",
        Builtin::MapFromList => "Map.fromList",
        Builtin::TextConcat => "Text.concat",
        Builtin::TextFromFloat => "Text.fromFloat",
        Builtin::TextFixed => "Text.fixed",
        Builtin::TextToBullets => "Text.toBullets",
        Builtin::TextSplit => "Text.split",
        Builtin::TextJoin => "Text.join",
        Builtin::TextParseFloat => "Text.parseFloat",
        Builtin::TextLength => "Text.length",
        Builtin::TextChars => "Text.chars",
        Builtin::TextToUpper => "Text.toUpper",
        Builtin::TextToLower => "Text.toLower",
        Builtin::TextContains => "Text.contains",
        Builtin::TextReplace => "Text.replace",
        Builtin::TextTrim => "Text.trim",
        Builtin::MathClamp01 => "Math.clamp01",
        Builtin::MathClamp => "Math.clamp",
        Builtin::MathLerp => "Math.lerp",
        Builtin::MathSmoothstep => "Math.smoothstep",
        Builtin::MathTan => "Math.tan",
        Builtin::MathAsin => "Math.asin",
        Builtin::MathAcos => "Math.acos",
        Builtin::MathAtan => "Math.atan",
        Builtin::MathRound => "Math.round",
        Builtin::MathCeil => "Math.ceil",
        Builtin::MathLog => "Math.log",
        Builtin::MathExp => "Math.exp",
        Builtin::MathSign => "Math.sign",
        Builtin::MathSin => "Math.sin",
        Builtin::MathCos => "Math.cos",
        Builtin::MathSqrt => "Math.sqrt",
        Builtin::MathAbs => "Math.abs",
        Builtin::MathFloor => "Math.floor",
        Builtin::MathAtan2 => "Math.atan2",
        Builtin::MathMod => "Math.mod",
        Builtin::MathMin => "Math.min",
        Builtin::MathMax => "Math.max",
        Builtin::MathPow => "Math.pow",
        Builtin::MathPi => "Math.pi",
        Builtin::RandomSeed => "Random.seed",
        Builtin::RandomStep => "Random.step",
        Builtin::RandomRange => "Random.range",
        Builtin::RandomFork => "Random.fork",
        Builtin::DebugLog => "Debug.log",
    }
}

/// Render a trace as indented enter/exit lines (the `functor-lang trace` output):
///
/// ```text
/// > report([12, 3.5, 40])
///   > List.filter([12, 3.5, 40], <fn(score)>)
///   < [12, 40]
/// < "- score: 12"
/// ```
pub fn render_trace(events: &[TraceEvent]) -> String {
    let mut out = String::new();
    for event in events {
        match event {
            TraceEvent::Enter {
                depth,
                callee,
                args,
            } => {
                out.push_str(&"  ".repeat(*depth));
                out.push_str(&format!("> {callee}({})\n", args.join(", ")));
            }
            TraceEvent::Exit { depth, result } => {
                out.push_str(&"  ".repeat(*depth));
                out.push_str(&format!("< {result}\n"));
            }
            TraceEvent::Truncated => {
                out.push_str("… trace truncated …\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod sort_key_tests {
    use super::{sort_key_cmp, stable_sort_comparison_ceiling};
    use std::cmp::Ordering;

    /// THE CROSS-PLATFORM LOCK. `List.sortBy`'s order must not depend on a
    /// NaN's sign bit — the exact bug that made CI green on macOS (aarch64,
    /// where `0.0 / 0.0` is `+NaN`) and red on Linux (x86, `-NaN`), because
    /// `f64::total_cmp` sorts by that bit.
    ///
    /// This test builds BOTH signed NaNs explicitly from their bit patterns,
    /// so it fails on every platform if the sign ever leaks back into the
    /// ordering — rather than depending on whatever `0.0 / 0.0` happens to
    /// produce on the machine running it.
    #[test]
    fn nan_keys_sort_last_regardless_of_sign_bit() {
        let pos_nan = f64::from_bits(0x7ff8_0000_0000_0000);
        let neg_nan = f64::from_bits(0xfff8_0000_0000_0000);
        assert!(pos_nan.is_nan() && neg_nan.is_nan());
        assert!(neg_nan.is_sign_negative() && !pos_nan.is_sign_negative());

        for nan in [pos_nan, neg_nan] {
            // Greater than everything real, including the infinities.
            for other in [0.0, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY] {
                assert_eq!(sort_key_cmp(nan, other), Ordering::Greater, "{nan} vs {other}");
                assert_eq!(sort_key_cmp(other, nan), Ordering::Less, "{other} vs {nan}");
            }
            // Both NaNs sort to the same place as each other.
            assert_eq!(sort_key_cmp(nan, pos_nan), Ordering::Equal);
            assert_eq!(sort_key_cmp(nan, neg_nan), Ordering::Equal);
        }

        // A whole sort agrees for either NaN sign — the CI failure, pinned.
        for nan in [pos_nan, neg_nan] {
            let mut v = vec![3.0, nan, 1.0, 2.0];
            v.sort_by(|a, b| sort_key_cmp(*a, *b));
            assert_eq!(&v[..3], &[1.0, 2.0, 3.0]);
            assert!(v[3].is_nan(), "the NaN must land last, got {v:?}");
        }
    }

    /// `-0.0` and `+0.0` are numerically equal, so the sort must treat them
    /// as ties and let STABILITY order them — `total_cmp` would have split
    /// them, silently reordering elements whose keys are `==` in the
    /// language.
    #[test]
    fn signed_zeros_are_ties() {
        assert_eq!(sort_key_cmp(-0.0, 0.0), Ordering::Equal);
        assert_eq!(sort_key_cmp(0.0, -0.0), Ordering::Equal);
    }

    /// Ordinary ordering is untouched.
    #[test]
    fn real_numbers_order_ascending() {
        assert_eq!(sort_key_cmp(1.0, 2.0), Ordering::Less);
        assert_eq!(sort_key_cmp(2.0, 1.0), Ordering::Greater);
        assert_eq!(sort_key_cmp(2.0, 2.0), Ordering::Equal);
        assert_eq!(sort_key_cmp(f64::NEG_INFINITY, f64::INFINITY), Ordering::Less);
    }

    #[test]
    fn map_sort_preflight_is_zero_for_trivial_inputs_and_n_log_n_afterward() {
        assert_eq!(stable_sort_comparison_ceiling(0), 0);
        assert_eq!(stable_sort_comparison_ceiling(1), 0);
        assert_eq!(stable_sort_comparison_ceiling(2), 4);
        assert_eq!(stable_sort_comparison_ceiling(8), 48);
        assert_eq!(stable_sort_comparison_ceiling(1000), 20_000);
    }
}

#[cfg(test)]
mod builtin_registry_tests {
    use super::{builtin, builtin_members, builtin_name, Builtin, ALL_BUILTINS, BUILTIN_NAMESPACES};
    use std::collections::BTreeSet;

    /// The DRIFT LOCK between the interpreter's dispatch table and the member
    /// set the checker reports (`types::unknown_builtin_member`): every listed
    /// builtin's name must resolve back through [`builtin`] to itself, and
    /// must live in a namespace [`BUILTIN_NAMESPACES`] declares — otherwise a
    /// real builtin could be reported as a typo, or a namespace's member list
    /// could be incomplete.
    #[test]
    fn all_builtins_round_trip_through_dispatch() {
        for b in ALL_BUILTINS {
            let name = builtin_name(b);
            let path: Vec<String> = name.split('.').map(str::to_string).collect();
            assert_eq!(path.len(), 2, "{name} should be `Namespace.member`");
            assert_eq!(builtin(&path), Some(b), "{name} does not dispatch to itself");
            assert!(
                BUILTIN_NAMESPACES.contains(&path[0].as_str()),
                "{name}: `{}` is missing from BUILTIN_NAMESPACES",
                path[0]
            );
        }
    }

    // THE DRIFT LOCK, half one: this match is exhaustive over `Builtin`, so
    // adding a variant fails to COMPILE here until `ALL_BUILTINS` offers it
    // too. That matters because `ALL_BUILTINS` is what the CHECKER reports as
    // a namespace's member set — a builtin missing from it would be rejected
    // as an unknown member even though the interpreter dispatches it.
    // (Half two is `all_builtins_round_trip_through_dispatch`, below.)
    #[test]
    fn builtins_list_is_exhaustive() {
        for &b in &ALL_BUILTINS {
            match b {
                Builtin::ListMap
                | Builtin::ListFilter
                | Builtin::ListFold
                | Builtin::ListRange
                | Builtin::ListGrid
                | Builtin::ListMaximum
                | Builtin::ListMinimum
                | Builtin::ListLength
                | Builtin::ListAppend
                | Builtin::ListFlatten
                | Builtin::ListAny
                | Builtin::ListAll
                | Builtin::ListReverse
                | Builtin::ListIsEmpty
                | Builtin::ListNth
                | Builtin::ListHead
                | Builtin::ListLast
                | Builtin::ListFind
                | Builtin::ListIndexedMap
                | Builtin::ListSortBy
                | Builtin::ListZip
                | Builtin::ListTake
                | Builtin::ListDrop
                | Builtin::ListSum
                | Builtin::ListConcatMap
                | Builtin::MapEmpty
                | Builtin::MapGet
                | Builtin::MapInsert
                | Builtin::MapRemove
                | Builtin::MapMember
                | Builtin::MapValues
                | Builtin::MapToList
                | Builtin::MapFromList
                | Builtin::MathSin
                | Builtin::MathCos
                | Builtin::MathSqrt
                | Builtin::MathAbs
                | Builtin::MathFloor
                | Builtin::MathAtan2
                | Builtin::MathMod
                | Builtin::MathMin
                | Builtin::MathMax
                | Builtin::MathPow
                | Builtin::MathPi
                | Builtin::MathClamp01
                | Builtin::MathClamp
                | Builtin::MathLerp
                | Builtin::MathSmoothstep
                | Builtin::MathTan
                | Builtin::MathAsin
                | Builtin::MathAcos
                | Builtin::MathAtan
                | Builtin::MathRound
                | Builtin::MathCeil
                | Builtin::MathLog
                | Builtin::MathExp
                | Builtin::MathSign
                | Builtin::TextConcat
                | Builtin::TextFromFloat
                | Builtin::TextFixed
                | Builtin::TextToBullets
                | Builtin::TextSplit
                | Builtin::TextJoin
                | Builtin::TextParseFloat
                | Builtin::TextLength
                | Builtin::TextChars
                | Builtin::TextToUpper
                | Builtin::TextToLower
                | Builtin::TextContains
                | Builtin::TextReplace
                | Builtin::TextTrim
                | Builtin::RandomSeed
                | Builtin::RandomStep
                | Builtin::RandomRange
                | Builtin::RandomFork
                | Builtin::DebugLog => {}
            }
        }
        assert_eq!(ALL_BUILTINS.len(), 76, "ALL_BUILTINS must list every Builtin");

        // The length check alone would accept a DUPLICATE entry standing in
        // for a missing one.
        let unique: BTreeSet<&str> = ALL_BUILTINS.iter().map(|b| builtin_name(*b)).collect();
        assert_eq!(
            unique.len(),
            ALL_BUILTINS.len(),
            "ALL_BUILTINS has a duplicate entry masking a missing builtin"
        );
    }

    #[test]
    fn every_namespace_has_members() {
        for namespace in BUILTIN_NAMESPACES {
            assert!(
                !builtin_members(namespace).is_empty(),
                "`{namespace}` is declared a builtin namespace but owns nothing"
            );
        }
    }
}

#[cfg(test)]
mod deep_value_tests {
    use super::{value_eq, FuelWriter, Span, Value};
    use std::fmt::Write;
    use std::rc::Rc;

    /// A list nested `depth` levels: `[[…[0]…]]`.
    fn nest(depth: usize) -> Value {
        let mut v = Value::Number(0.0);
        for _ in 0..depth {
            v = Value::List(Rc::new(vec![v]));
        }
        v
    }

    /// Display and value_eq must be ITERATIVE: on a deliberately SMALL stack
    /// (512KB), walking a 200k-deep value recursively would overflow (~100 B
    /// per frame ⇒ ~20MB), while the worklist versions stay flat. The built
    /// values are `mem::forget`-leaked because `Value`'s drop glue IS
    /// recursive by design (see the NOTE in `value.rs` and the stack
    /// contract on `run_expects_budgeted`) — this pin is exactly about the
    /// walks that must NOT share that constraint.
    #[test]
    fn display_and_eq_are_depth_safe_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let deep = nest(200_000);
                let text = deep.to_string();
                assert!(text.starts_with("[[[[") && text.len() > 400_000);
                let mut compared = 0u64;
                let same = value_eq(&deep, &deep.clone(), Span::new(0, 0), &mut compared, "==")
                    .expect("comparable");
                assert!(same);
                assert!(compared > 200_000);
                std::mem::forget(deep);
            })
            .expect("spawn small-stack worker")
            .join()
            .expect("deep display/eq must complete on a small stack");
    }

    #[test]
    fn fuel_writer_stops_structural_rendering_at_its_byte_limit() {
        let shared = Value::String(Rc::from("x".repeat(1024)));
        let value = Value::List(Rc::new(vec![shared; 100]));
        let mut out = String::new();
        let mut writer = FuelWriter {
            out: &mut out,
            remaining: 32,
        };
        assert!(write!(&mut writer, "{value}").is_err());
        assert!(out.len() <= 32, "rendered {} bytes past the cap", out.len());
    }
}

#[cfg(test)]
mod random_tests {
    use super::random_step;

    /// Draw `n` values starting from `seed`, threading the returned seed.
    fn stream(seed: f64, n: usize) -> Vec<f64> {
        let mut s = seed;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let (v, next) = random_step(s);
            out.push(v);
            s = next;
        }
        out
    }

    #[test]
    fn deterministic_same_seed_same_stream() {
        assert_eq!(stream(42.0, 64), stream(42.0, 64));
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(stream(1.0, 32), stream(2.0, 32));
    }

    #[test]
    fn values_in_unit_interval() {
        for &v in &stream(7.0, 10_000) {
            assert!((0.0..1.0).contains(&v), "value {v} out of [0,1)");
        }
    }

    #[test]
    fn successive_draws_differ() {
        let s = stream(123.0, 1000);
        // No two consecutive draws collide (a stuck/low-period generator would).
        for pair in s.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        // And the stream isn't dominated by repeats overall.
        let mut sorted = s.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.dedup();
        assert!(sorted.len() > 990, "too many repeats: {} unique", sorted.len());
    }

    #[test]
    fn caller_seeds_do_not_collapse() {
        // Regression: a saturating `as u64` cast maps every negative seed to 0,
        // collapsing them onto one stream. `rem_euclid` folding keeps them
        // distinct — negative, zero, and in-range integer seeds each differ.
        let seeds = [-100.0, -2.0, -1.0, 0.0, 1.0, 2.0, 42.0, 12345.0];
        let firsts: Vec<f64> = seeds.iter().map(|&s| random_step(s).0).collect();
        let mut sorted = firsts.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.dedup();
        assert_eq!(sorted.len(), seeds.len(), "distinct seeds collapsed: {firsts:?}");
        // And in-range integer seeds are the identity fold (not remapped).
        assert_eq!(random_step(5.0), random_step(5.0));
    }

    #[test]
    fn seed_round_trips_exactly_as_f64() {
        // The returned seed must be an integer exactly representable as f64, so
        // it survives being held in the (f64) model without drift.
        let mut s = 999.0;
        for _ in 0..100 {
            let (_, next) = random_step(s);
            assert_eq!(next, next.trunc(), "seed {next} is not an integer");
            assert!(next >= 0.0 && next < (1u64 << 52) as f64);
            s = next;
        }
    }

    /// The original bug: streams seeded from adjacent integers were visibly
    /// correlated. Draw many parallel streams `f(i)` for adjacent `i` and
    /// assert their first draws show no linear correlation.
    #[test]
    fn adjacent_seeds_uncorrelated() {
        let n = 4096;
        // First draw of each adjacent-seed stream.
        let xs: Vec<f64> = (0..n).map(|i| random_step(i as f64).0).collect();
        // Pearson correlation between consecutive first-draws (x[i], x[i+1]) —
        // the "shifted stream" footgun would spike this toward ±1.
        let pairs: Vec<(f64, f64)> = xs.windows(2).map(|w| (w[0], w[1])).collect();
        let corr = pearson(&pairs);
        assert!(corr.abs() < 0.05, "adjacent-seed correlation too high: {corr}");

        // Also: no adjacent-seed streams share an element within the first few
        // draws (the true overlap bug — stream i+1 == stream i shifted by one).
        for i in 0..64 {
            let a = stream(i as f64, 8);
            let b = stream((i + 1) as f64, 8);
            for &va in &a {
                assert!(
                    !b.contains(&va),
                    "adjacent seeds {i}/{} share a value {va}",
                    i + 1
                );
            }
        }
    }

    /// `Random.seed` hashes the float's BITS: fractional seeds — notably the
    /// `Effect.random` idiom's [0, 1) output, which the raw counter fold
    /// truncates to 0 — must land on distinct counters.
    #[test]
    fn seed_counter_spreads_fractional_seeds() {
        let seeds = [0.0, 0.42, 0.84, 0.999, 1.0, 5.0, -0.42, -5.0];
        let counters: Vec<u64> = seeds.iter().map(|&s| super::seed_counter(s)).collect();
        let mut sorted = counters.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seeds.len(), "seed counters collapsed: {counters:?}");
        // -0.0 == +0.0, so the two equal zeros must seed alike…
        assert_eq!(super::seed_counter(-0.0), super::seed_counter(0.0));
        // …and every counter round-trips exactly through the language's f64s.
        for &c in &counters {
            assert_eq!(c as f64 as u64, c);
            assert!(c < 1u64 << 52);
        }
    }

    /// The all-zero corner: `mix64(0) == 0`, so `Random.seed(0.0)` is counter
    /// 0 — and an undecorated hash-combine would make `fork(0.0, that)`
    /// fix-point back to the parent. The FORK_GAMMA offset breaks the cycle.
    #[test]
    fn fork_of_zero_seed_leaves_the_parent() {
        let zero = super::seed_counter(0.0) as f64;
        let child = super::fork_seed(0.0, zero);
        assert_ne!(child, zero, "fork(0, seed(0)) must not echo the parent");
        // And the child's stream diverges from the parent's.
        assert_ne!(random_step(child).0, random_step(zero).0);
    }

    /// `Random.fork(i, seed)` — distinct, deterministic, in-range child seeds
    /// for distinct stream indices (including fractional ones — no
    /// truncation collisions like `fork(0.5) == fork(0.0)`).
    #[test]
    fn fork_names_distinct_child_streams() {
        let seed = super::seed_counter(5.0) as f64;
        let indices: Vec<f64> = (0..64).map(|i| i as f64).chain([0.5, 1.5, -1.0]).collect();
        let children: Vec<f64> = indices.iter().map(|&i| super::fork_seed(i, seed)).collect();
        let mut sorted = children.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.dedup();
        assert_eq!(sorted.len(), children.len(), "fork child seeds collided");
        for (&i, &c) in indices.iter().zip(&children) {
            assert_eq!(c, super::fork_seed(i, seed), "fork is deterministic");
            assert!(c >= 0.0 && c < (1u64 << 52) as f64 && c == c.trunc());
        }
    }

    fn pearson(pairs: &[(f64, f64)]) -> f64 {
        let n = pairs.len() as f64;
        let (sx, sy): (f64, f64) = pairs
            .iter()
            .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
        let (mx, my) = (sx / n, sy / n);
        let mut cov = 0.0;
        let mut vx = 0.0;
        let mut vy = 0.0;
        for (x, y) in pairs {
            let (dx, dy) = (x - mx, y - my);
            cov += dx * dy;
            vx += dx * dx;
            vy += dy * dy;
        }
        cov / (vx.sqrt() * vy.sqrt())
    }
}
