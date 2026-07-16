//! The `Debug.log` trace sink.
//!
//! `Debug.log(value, label)` (see [`crate::eval`]) is an Elm-style trace: it
//! logs `label: <value>` and returns `value` unchanged. Where that line *goes*
//! is a host decision, so the destination is a process-wide, settable sink:
//!
//! - **Plain `functor-lang run`** (no host installed): the default sink prints
//!   `label: value` to stdout — the interpreter has no renderer of its own.
//! - **Under the Functor host** (the runner): the shell installs a sink that
//!   forwards the line into the CLI's region-aware log path (an `Event::Log`),
//!   so a trace lands ABOVE the live telemetry panel / as an ndjson log event
//!   instead of corrupting stdout (see `docs/cli-output.md`).
//!
//! The `functor_lang` crate must not depend on the runtime/CLI, so this is a bare
//! `Fn(String)` callback the host owns — the same "runtime installs a sink"
//! shape as `functor_runtime_common::events`. It lives on the *process* (not any
//! interpreter `Session`), so it survives Functor Lang hot-reload's `Session` rebuild for
//! free. Unlike `events`' `OnceLock` it is *re-settable* (an `RwLock`) for one
//! reason: the test seam swaps in a per-test capturing sink. Host installs are
//! idempotent (a stateless closure), so overwriting is harmless.

use std::sync::RwLock;

type Sink = Box<dyn Fn(String) + Send + Sync>;

static SINK: RwLock<Option<Sink>> = RwLock::new(None);

/// Install the process-wide `Debug.log` trace sink. The host calls this once at
/// startup to route traces into its own logging path; a later call overwrites
/// the previous sink (host installs are idempotent, and the test seam swaps in a
/// capturing sink this way).
pub fn set_trace_sink(sink: Sink) {
    *SINK.write().unwrap() = Some(sink);
}

thread_local! {
    /// See [`suppress`] — true while a paused-inspector replay runs on this
    /// thread.
    static SUPPRESSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A scope with `Debug.log` emission suppressed on this thread — the paused
/// inspector's replay seam: a journaled call re-runs to record values, and its
/// `Debug.log` lines already emitted live at frame time, so re-emitting would
/// duplicate every line on each trace build. RAII so an early return can't
/// leave the thread muted.
pub fn suppress() -> SuppressGuard {
    SUPPRESSED.with(|s| s.set(true));
    SuppressGuard
}

pub struct SuppressGuard;

impl Drop for SuppressGuard {
    fn drop(&mut self) {
        SUPPRESSED.with(|s| s.set(false));
    }
}

/// Emit one already-formatted `Debug.log` line (`"label: value"`). With a sink
/// installed it routes there; otherwise — plain `functor-lang run`, tests, a bare
/// interpreter — it prints to stdout, the interpreter's only renderer. Silent
/// inside a [`suppress`] scope (inspector replay).
pub fn emit(message: String) {
    if SUPPRESSED.with(|s| s.get()) {
        return;
    }
    match SINK.read().unwrap().as_ref() {
        Some(sink) => sink(message),
        None => println!("{message}"),
    }
}
