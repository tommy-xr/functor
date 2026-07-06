//! The CLI's output event stream — one source of truth, two renderers.
//!
//! Command logic emits typed [`Event`]s via [`emit`]; a single [`Renderer`],
//! selected once at startup ([`init`]), turns them into human text
//! ([`PlainRenderer`]) or newline-delimited JSON ([`JsonRenderer`]). Command
//! code never formats user-facing strings itself. See `docs/cli-output.md` for
//! the schema and the renderer-selection rule.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

use colored::Colorize;
use serde::Serialize;

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
    /// An MLE check / load error, positioned in its file.
    Diagnostic {
        severity: Severity,
        #[serde(skip_serializing_if = "Option::is_none")]
        file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        col: Option<usize>,
        message: String,
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
        }
    }
}

impl Event {
    /// Whether `--quiet` keeps this event (errors + final status only).
    fn is_essential(&self) -> bool {
        matches!(
            self,
            Event::Error { .. }
                | Event::Diagnostic { .. }
                | Event::CommandFinished { .. }
                | Event::AssetError { .. }
        )
    }
}

/// One source of truth, two presentations.
pub trait Renderer: Send + Sync {
    fn render(&self, event: &Event);
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
                let mut s = format!("{} {}", "▸".cyan(), command.bold());
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
                    vec![format!("{} done {}", "✓".green(), dur)]
                } else {
                    vec![format!("{} failed {}", "✗".red(), dur)]
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
                    "✓".green(),
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
                vec![format!("{label}: {}{message}", loc.dimmed())]
            }
            Event::ServerListening { url } => {
                vec![format!("{} serving {}", "◈".cyan(), url.underline())]
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
            Event::RuntimeReady => vec![format!("{} running", "▸".cyan())],
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
                vec![format!("{} captured {}", "✓".green(), path)]
            }
            Event::HotReload { ok, message } => {
                if *ok {
                    vec![format!("{} {message}", "↻".cyan())]
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
            Event::Reload => vec![format!("{} reload", "↻".cyan())],
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

static RENDERER: OnceLock<Box<dyn Renderer>> = OnceLock::new();

/// Select and install the process-wide renderer from the global flags +
/// environment. Called once at startup, before any command logic runs. See
/// `docs/cli-output.md` for the selection table.
pub fn init(json: bool, quiet: bool, no_color: bool) {
    // Color only on a real TTY, and never under NO_COLOR / --no-color / CI /
    // --json. Enforced globally so no code path can leak ANSI.
    let color = !json
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("CI").is_none()
        && !no_color;
    colored::control::set_override(color);

    let renderer: Box<dyn Renderer> = if json {
        Box::new(JsonRenderer)
    } else {
        Box::new(PlainRenderer { quiet })
    };
    let _ = RENDERER.set(renderer);
}

/// Emit an event to the installed renderer. A no-op if [`init`] was never
/// called (defensive — main always calls it first).
pub fn emit(event: Event) {
    if let Some(renderer) = RENDERER.get() {
        renderer.render(&event);
    }
}
