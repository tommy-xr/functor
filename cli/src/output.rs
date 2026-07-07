//! The CLI's output event stream — one source of truth, two renderers.
//!
//! Command logic emits typed [`Event`]s via [`emit`]; a single [`Renderer`],
//! selected once at startup ([`init`]), turns them into human text
//! ([`PlainRenderer`]) or newline-delimited JSON ([`JsonRenderer`]). Command
//! code never formats user-facing strings itself. See `docs/cli-output.md` for
//! the schema and the renderer-selection rule.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use colored::Colorize;
use serde::Serialize;

mod live;

/// Diagnostic severity (an MLE check error is `Error`; `Warning` is reserved
/// for the runtime's permissive dev-loop diagnostics routed later).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    // Reserved: the runtime's permissive dev-loop diagnostics route here in PR-2.
    #[allow(dead_code)]
    Warning,
}

/// The level of a free-form [`Event::Log`] line. `Debug`/`Info`/`Warn`/`Error`
/// mirror the `log` crate's levels (its `Trace` folds into `Debug`) and are
/// gated by verbosity (`-v`); `Trace` is a distinct, always-visible tier used
/// only for explicit MLE `Debug.log` traces (which are user intent, so they
/// show by default — see `docs/cli-output.md`). Serialized snake_case.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<log::Level> for LogLevel {
    fn from(level: log::Level) -> Self {
        match level {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Info => LogLevel::Info,
            // Trace is rare and folds into Debug — the CLI has no separate tier.
            log::Level::Debug | log::Level::Trace => LogLevel::Debug,
        }
    }
}

/// A user-visible thing the CLI wants to say. Serialized as
/// `{"type": "<snake_case>", …}` — the stable machine API (see
/// `docs/cli-output.md`). Only the PR-1 (CLI-side) variants live here; the
/// runtime-side ones (frame stats, capture, hot-reload, asset errors) land in
/// PR-2 with the runtime event-sink refactor.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A command began.
    CommandStarted {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<String>,
    },
    /// A command ended (also the exit-code carrier for machine consumers).
    CommandFinished { ok: bool, duration_ms: u64 },
    /// A `build` typecheck passed (`entry` plus `sibling_count` sibling modules).
    MleLoaded { entry: String, sibling_count: usize },
    /// An MLE check / load error, positioned in its file. `source_line` is the
    /// raw offending line (no caret baked in) so the human renderer can draw a
    /// rustc-style caret under `col`; machine consumers get the same structured
    /// fields and can render their own. Omitted from JSON when absent.
    Diagnostic {
        severity: Severity,
        #[serde(skip_serializing_if = "Option::is_none")]
        file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        col: Option<usize>,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_line: Option<String>,
    },
    /// The wasm dev server bound and is serving.
    ServerListening { url: String },
    /// Neutral status (a hot-reload hint, a push acknowledgement, …).
    Info { message: String },
    /// A non-fatal issue.
    Warning { message: String },
    /// A fatal error, emitted just before the process exits non-zero.
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
    /// The in-process runtime loaded and is about to render.
    RuntimeReady,
    /// A periodic frame-cost sample from the runtime's game loop. `frame_us` is
    /// the total (tick + physics + draw); `budget_pct` is that against a 60 fps
    /// budget.
    FrameStats {
        tick_us: f64,
        draw_us: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        frame_us: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        budget_pct: Option<f64>,
        over_n_frames: u32,
    },
    /// A `--capture-frame` PNG was written.
    CaptureWritten { path: String },
    /// A hot-reload settled (`ok` false = the edit was rejected; the old program
    /// keeps running).
    HotReload { ok: bool, message: String },
    /// An asset failed to load; the runtime serves the fallback asset.
    AssetError {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        message: String,
    },
    /// The wasm dev-server told a connected page to reload (not emitted by the
    /// native runtime; wired with the wasm serve path).
    #[allow(dead_code)]
    Reload,
    /// A free-form log line — any `log::{debug,info,warn,error}!` call in the CLI
    /// or the in-process runtime, funneled through the region-aware renderer so
    /// it never corrupts the live panel or the ndjson stream (see
    /// `docs/cli-output.md`).
    Log { level: LogLevel, message: String },
}

impl From<functor_runtime_common::events::RuntimeEvent> for Event {
    fn from(ev: functor_runtime_common::events::RuntimeEvent) -> Self {
        use functor_runtime_common::events::RuntimeEvent as R;
        match ev {
            R::Ready => Event::RuntimeReady,
            R::FrameStats {
                tick_us,
                draw_us,
                frame_us,
                budget_pct,
                over_n_frames,
            } => Event::FrameStats {
                tick_us,
                draw_us,
                frame_us: Some(frame_us),
                budget_pct: Some(budget_pct),
                over_n_frames,
            },
            R::CaptureWritten { path } => Event::CaptureWritten { path },
            R::HotReload { ok, message } => Event::HotReload { ok, message },
            R::AssetError { path, message } => Event::AssetError { path, message },
            // An MLE `Debug.log` trace: explicit user intent, so it's an
            // always-visible `Trace`-level log (not `-v`-gated like the `log`
            // facade). The message is already `"label: value"`.
            R::MleTrace { message } => Event::Log {
                level: LogLevel::Trace,
                message,
            },
        }
    }
}

impl Event {
    /// Whether `--quiet` keeps this event (errors + final status only). A log
    /// line survives `--quiet` only at warn/error — debug/info are suppressed.
    fn is_essential(&self) -> bool {
        match self {
            Event::Error { .. }
            | Event::Diagnostic { .. }
            | Event::CommandFinished { .. }
            | Event::AssetError { .. } => true,
            Event::Log { level, .. } => matches!(level, LogLevel::Warn | LogLevel::Error),
            _ => false,
        }
    }
}

/// One source of truth, two presentations.
pub trait Renderer: Send + Sync {
    fn render(&self, event: &Event);

    /// Restore the terminal (wipe any live region, show the cursor). A no-op for
    /// the plain/json renderers; the live renderer overrides it. Invoked from
    /// the panic hook and the Ctrl-C handler (see [`cleanup`]).
    fn cleanup(&self) {}
}

/// ndjson: one compact JSON object per line, flushed. No color, ever.
pub struct JsonRenderer;

impl Renderer for JsonRenderer {
    fn render(&self, event: &Event) {
        let mut out = std::io::stdout().lock();
        if serde_json::to_writer(&mut out, event).is_ok() {
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

/// Human-readable text. Color is governed globally by
/// `colored::control::set_override` (set in [`init`]), so this renders plain
/// on a pipe / `NO_COLOR` / `--no-color` with no ANSI leakage. No spinners or
/// animation yet — that's the PR-2 ink-style renderer.
pub struct PlainRenderer {
    pub quiet: bool,
}

impl Renderer for PlainRenderer {
    fn render(&self, event: &Event) {
        if self.quiet && !event.is_essential() {
            return;
        }
        for line in Self::lines(event) {
            println!("{line}");
        }
    }
}

impl PlainRenderer {
    fn lines(event: &Event) -> Vec<String> {
        match event {
            Event::CommandStarted {
                command,
                project,
                env,
            } => {
                let mut s = format!("{} {}", g_bullet().cyan(), command.bold());
                if let Some(project) = project {
                    s.push_str(&format!(" {}", project.dimmed()));
                }
                if let Some(env) = env {
                    s.push_str(&format!(" {}", format!("({env})").dimmed()));
                }
                vec![s]
            }
            Event::CommandFinished { ok, duration_ms } => {
                let dur = format_duration(*duration_ms).dimmed();
                if *ok {
                    vec![format!("{} done {}", g_ok().green(), dur)]
                } else {
                    vec![format!("{} failed {}", g_fail().red(), dur)]
                }
            }
            Event::MleLoaded {
                entry,
                sibling_count,
            } => {
                let siblings = match sibling_count {
                    0 => String::new(),
                    1 => " (+1 module)".to_string(),
                    n => format!(" (+{n} modules)"),
                };
                vec![format!(
                    "{} checked {}{}",
                    g_ok().green(),
                    entry,
                    siblings.dimmed()
                )]
            }
            Event::Diagnostic {
                severity,
                file,
                line,
                col,
                message,
                source_line,
            } => {
                let label = match severity {
                    Severity::Error => "error".red().bold(),
                    Severity::Warning => "warning".yellow().bold(),
                };
                let loc = match (file, line, col) {
                    (Some(f), Some(l), Some(c)) => format!("{f}:{l}:{c}: "),
                    (Some(f), _, _) => format!("{f}: "),
                    _ => String::new(),
                };
                let mut out = vec![format!("{label}: {}{message}", loc.dimmed())];
                // rustc-style source line + caret, when we resolved the line.
                if let (Some(src), Some(l), Some(c)) = (source_line, line, col) {
                    out.extend(caret_block(src, *l, *c, *severity));
                }
                out
            }
            Event::ServerListening { url } => {
                vec![format!("{} serving {}", g_serve().cyan(), url.underline())]
            }
            Event::Info { message } => vec![message.clone()],
            Event::Warning { message } => {
                vec![format!("{}: {message}", "warning".yellow().bold())]
            }
            Event::Error { message, hint } => {
                let mut out = vec![format!("{}: {message}", "error".red().bold())];
                if let Some(hint) = hint {
                    out.push(format!("{}: {hint}", "hint".cyan().bold()));
                }
                out
            }
            Event::RuntimeReady => vec![format!("{} running", g_bullet().cyan())],
            Event::FrameStats {
                tick_us,
                draw_us,
                frame_us,
                budget_pct,
                over_n_frames,
            } => {
                let mut s = format!("tick {tick_us:.1}µs, draw {draw_us:.1}µs");
                if let Some(frame_us) = frame_us {
                    s.push_str(&format!(", frame {frame_us:.1}µs"));
                }
                if let Some(budget_pct) = budget_pct {
                    s.push_str(&format!(" ({budget_pct:.1}% of 60fps)"));
                }
                vec![format!(
                    "{} {}",
                    format!("stats/{over_n_frames}").dimmed(),
                    s.dimmed()
                )]
            }
            Event::CaptureWritten { path } => {
                vec![format!("{} captured {}", g_ok().green(), path)]
            }
            Event::HotReload { ok, message } => {
                if *ok {
                    vec![format!("{} {message}", g_reload().cyan())]
                } else {
                    vec![format!("{}: {message}", "reload".yellow().bold())]
                }
            }
            Event::AssetError { path, message } => {
                let loc = match path {
                    Some(p) => format!("{p}: "),
                    None => String::new(),
                };
                vec![format!(
                    "{}: {}{message}",
                    "asset error".red().bold(),
                    loc.dimmed()
                )]
            }
            Event::Reload => vec![format!("{} reload", g_reload().cyan())],
            Event::Log { level, message } => {
                // ASCII-safe level tags (`[debug]` … already plain ASCII).
                let tag = match level {
                    LogLevel::Trace => "[trace]".magenta(),
                    LogLevel::Debug => "[debug]".dimmed(),
                    LogLevel::Info => "[info]".cyan(),
                    LogLevel::Warn => "[warn]".yellow().bold(),
                    LogLevel::Error => "[error]".red().bold(),
                };
                vec![format!("{tag} {message}")]
            }
        }
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("in {ms}ms")
    } else {
        format!("in {:.1}s", ms as f64 / 1000.0)
    }
}

/// A rustc-style source-line + caret block under column `col` (1-based). The
/// caret indent copies the source line's own prefix (tabs kept as tabs) so it
/// lands under the right glyph regardless of tab width.
fn caret_block(source_line: &str, line: usize, col: usize, severity: Severity) -> Vec<String> {
    let n = line.to_string();
    let gutter = " ".repeat(n.len());
    let bar = "|".dimmed();
    let indent: String = source_line
        .chars()
        .take(col.saturating_sub(1))
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let caret = match severity {
        Severity::Error => "^".red().bold(),
        Severity::Warning => "^".yellow().bold(),
    };
    vec![
        format!("{gutter} {bar}"),
        format!("{} {bar} {source_line}", n.dimmed()),
        format!("{gutter} {bar} {indent}{caret}"),
    ]
}

// ── ASCII fallback ───────────────────────────────────────────────────────────
// A single source of truth for the status glyphs. On a dumb / non-UTF-8 terminal
// (or under `--ascii`) they degrade to plain ASCII so output is never mojibake.
// Decided once in `init`; read here and by the live renderer.

static ASCII: AtomicBool = AtomicBool::new(false);

/// Whether the CLI is in ASCII-only mode (dumb terminal / non-UTF-8 locale /
/// `--ascii`). The live renderer reads this for its spinner + bar glyphs too.
pub(crate) fn ascii() -> bool {
    ASCII.load(Ordering::Relaxed)
}

pub(crate) fn g_bullet() -> &'static str {
    if ascii() {
        "->"
    } else {
        "▸"
    }
}
pub(crate) fn g_ok() -> &'static str {
    if ascii() {
        "[ok]"
    } else {
        "✓"
    }
}
pub(crate) fn g_fail() -> &'static str {
    if ascii() {
        "[x]"
    } else {
        "✗"
    }
}
pub(crate) fn g_serve() -> &'static str {
    if ascii() {
        "::"
    } else {
        "◈"
    }
}
pub(crate) fn g_reload() -> &'static str {
    if ascii() {
        "~"
    } else {
        "↻"
    }
}

/// Decide ASCII-only mode: an explicit `--ascii`, a dumb `TERM`, or a locale
/// (`LC_ALL` / `LC_CTYPE` / `LANG`, first one set wins) that is set but is not
/// UTF-8. An unset locale is treated as UTF-8, so the default stays unchanged.
fn detect_ascii(force_ascii: bool) -> bool {
    if force_ascii {
        return true;
    }
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        return true;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    match locale {
        Some(v) => !v.to_ascii_lowercase().replace('-', "").contains("utf8"),
        None => false,
    }
}

static RENDERER: OnceLock<Box<dyn Renderer>> = OnceLock::new();
static LIVE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether a log record belongs to Functor's own crates (its `target` defaults
/// to the module path, so `functor_runtime_common::…`, `functor_cli`, `mle::…`).
/// We scope to these so `-v` means "Functor's debug logs" — not the debug/trace
/// firehose of transitive deps (notify/mio/tokio/hyper/glow/egui/gltf all use
/// `log`), and so a noisy dependency warning never lands in normal CLI output.
fn is_functor_target(target: &str) -> bool {
    target.starts_with("functor") || target == "mle" || target.starts_with("mle::")
}

/// A `log::Log` that funnels every Functor `log::{debug,info,warn,error}!` call
/// (from the CLI or the in-process runtime) into an [`Event::Log`], so free-form
/// logs travel the same region-aware renderer as everything else. Level is gated
/// cheaply by `log`'s global `max_level` (set in [`init`]); target scoping keeps
/// dependency logs out.
struct EventLogger;

impl log::Log for EventLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // Global `max_level` gates the level; we additionally scope to Functor's
        // own crates so a `-v` firehose from deps never floods the renderer.
        is_functor_target(metadata.target())
    }

    fn log(&self, record: &log::Record) {
        // `log!` checks `enabled` first, but be defensive against a record with a
        // custom target that skips it.
        if !is_functor_target(record.target()) {
            return;
        }
        emit(Event::Log {
            level: record.level().into(),
            message: record.args().to_string(),
        });
    }

    fn flush(&self) {}
}

/// Pick the log level: an explicit `RUST_LOG` (a bare level — `debug`, `warn`,
/// …) wins; else `-v/--verbose` opens debug; else the quiet default (warn/error
/// only), which keeps the CLI silent unless something needs attention.
fn log_level(verbose: bool) -> log::LevelFilter {
    if let Some(filter) = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| v.trim().parse::<log::LevelFilter>().ok())
    {
        return filter;
    }
    if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    }
}

/// Select and install the process-wide renderer from the global flags +
/// environment. Called once at startup, before any command logic runs. See
/// `docs/cli-output.md` for the selection table.
pub fn init(json: bool, quiet: bool, no_color: bool, ascii: bool, verbose: bool) {
    // Color only on a real TTY, and never under NO_COLOR / --no-color / CI /
    // --json. Enforced globally so no code path can leak ANSI.
    let color = !json
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("CI").is_none()
        && !no_color;
    colored::control::set_override(color);

    // ASCII glyph fallback — decided once, alongside color. `--json` is pure
    // structured data (no glyphs), so leave it on the default there.
    ASCII.store(!json && detect_ascii(ascii), Ordering::Relaxed);

    // The live (ink-style) renderer activates ONLY on an interactive, color-
    // allowed TTY, and never under --quiet (which keeps its exact minimal plain
    // output). Every machine-facing path — --json, --quiet, non-TTY, CI,
    // NO_COLOR, --no-color — keeps the plain/json renderer, byte-for-byte.
    let renderer: Box<dyn Renderer> = if json {
        Box::new(JsonRenderer)
    } else if quiet {
        Box::new(PlainRenderer { quiet })
    } else if color {
        LIVE_ACTIVE.store(true, Ordering::Relaxed);
        install_terminal_guard();
        Box::new(live::LiveRenderer::new())
    } else {
        Box::new(PlainRenderer { quiet })
    };
    let _ = RENDERER.set(renderer);

    // Install the `log` facade AFTER the renderer, so any `log!` — from the CLI
    // or the in-process runtime — becomes an `Event::Log` routed through the
    // region-aware renderer. `max_level` gates cheaply, so a suppressed
    // `log::debug!` on the hot path costs almost nothing.
    log::set_max_level(log_level(verbose));
    let _ = log::set_boxed_logger(Box::new(EventLogger));
}

/// True when the live renderer is installed. `main` uses this to arm a Ctrl-C
/// handler that restores the terminal, without changing the (unchanged) signal
/// behavior of the plain/json paths.
pub fn live_active() -> bool {
    LIVE_ACTIVE.load(Ordering::Relaxed)
}

/// Wipe the live region and restore the cursor. Idempotent and safe to call
/// from a panic hook or a signal handler; a no-op for the plain/json renderers.
pub fn cleanup() {
    if let Some(renderer) = RENDERER.get() {
        renderer.cleanup();
    }
}

/// Chain a panic hook that restores the terminal before the default hook runs,
/// so a panic mid-render never leaves a stuck live region / hidden cursor.
fn install_terminal_guard() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup();
        previous(info);
    }));
}

/// Emit an event to the installed renderer. A no-op if [`init`] was never
/// called (defensive — main always calls it first).
pub fn emit(event: Event) {
    if let Some(renderer) = RENDERER.get() {
        renderer.render(&event);
    }
}
