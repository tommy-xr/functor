//! Runtime → shell event sink.
//!
//! The in-process desktop runtime (frame loop, hot-reload, asset load) used to
//! `println!` its status straight to stdout, which corrupts the CLI's ndjson
//! stream and bypasses its renderer. Instead it now emits a typed
//! [`RuntimeEvent`] through a process-wide sink the shell installs once at
//! startup ([`set_sink`]).
//!
//! **Dependency direction.** This type + sink live in the runtime crate; the
//! `cli` crate (which depends on the runtime, never the reverse) installs an
//! adapter that maps each `RuntimeEvent` onto its own `output::Event` and
//! renders it. The runtime stays oblivious to the CLI — it only knows how to
//! emit.
//!
//! **Non-blocking.** [`emit`] is a cheap `OnceLock` load plus a call to the
//! installed `Fn` — no lock is taken here. The frame loop never emits on the
//! hot path: only [`RuntimeEvent::FrameStats`] (every ~300 frames), one-shot
//! lifecycle events, and error/reload notices flow through, so per-frame
//! `tick`/`draw` cost is unaffected.

use std::sync::OnceLock;

/// A user-visible thing the runtime wants to say — the runtime-side mirror of
/// the CLI's `output::Event`. The shell's sink maps these onto its own event
/// schema (see `docs/cli-output.md`).
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// The runtime is loaded and about to render its first frame.
    Ready,
    /// A periodic frame-cost sample, averaged over `over_n_frames` frames.
    /// `frame_us` is the total (`tick + physics + draw`); `budget_pct` is that
    /// total against a 60 fps (16.666 ms) budget. `render_us` / `swap_us` are the
    /// shell-measured GL cost — the scene render pass and the buffer swap (vsync
    /// blocking) — reported alongside `draw_us` so interpreter cost and GL cost
    /// can be told apart. They are not folded into `frame_us` (swap's vsync block
    /// would peg `budget_pct`).
    FrameStats {
        tick_us: f64,
        draw_us: f64,
        render_us: f64,
        swap_us: f64,
        frame_us: f64,
        budget_pct: f64,
        over_n_frames: u32,
    },
    /// A `--capture-frame` PNG was written to `path`.
    CaptureWritten { path: String },
    /// A hot-reload attempt settled (`ok` false on a rejected edit; the old
    /// program keeps running).
    HotReload { ok: bool, message: String },
    /// An asset failed to load; the runtime serves the fallback asset.
    AssetError {
        path: Option<String>,
        message: String,
    },
    /// An Functor Lang `Debug.log(value, label)` trace — the already-formatted
    /// `"label: value"` line. Unlike the `-v`-gated `log` facade, this is
    /// EXPLICIT user intent, so the shell shows it by default (the CLI maps it
    /// to an always-visible `Event::Log`; see `docs/cli-output.md`). Fired only
    /// where a game places a `Debug.log`; a default game emits none.
    FunctorLangTrace { message: String },
}

type Sink = Box<dyn Fn(RuntimeEvent) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the process-wide runtime event sink. Called once by the shell before
/// the runtime starts; a second call is ignored (the first wins).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Emit a runtime event to the installed sink. With no sink installed (wasm,
/// tests, or a bare runtime), errors fall back to stderr and routine notices are
/// dropped — so nothing corrupts a caller that never opted in.
pub fn emit(event: RuntimeEvent) {
    if let Some(sink) = SINK.get() {
        sink(event);
    } else if let RuntimeEvent::AssetError { path, message } = event {
        match path {
            Some(path) => eprintln!("Failed to load asset '{path}', using fallback: {message}"),
            None => eprintln!("Failed to load asset, using fallback: {message}"),
        }
    }
}
