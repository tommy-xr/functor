//! The ink-style human renderer: a live region above stable scrollback.
//!
//! Selected only on an interactive TTY with color allowed (see
//! [`super::init`]); every other path — `--json`, `--quiet`, non-TTY / piped /
//! `CI` / `NO_COLOR` / `--no-color` — keeps the plain [`super::PlainRenderer`]
//! or [`super::JsonRenderer`], byte-for-byte unchanged. So the machine-facing
//! streams never see a spinner or a control char.
//!
//! Two behaviors, both driven by the same [`Event`] stream — no formatting is
//! duplicated: finished/one-shot lines reuse [`super::PlainRenderer::lines`] and
//! are committed to scrollback with [`MultiProgress::println`]; only the *live*
//! bits (an in-flight phase spinner, and the sticky `run native` telemetry
//! panel fed by `frame_stats`/`hot_reload`) are rendered here.
//!
//! 1. **`build` / `develop`:** the checking phase shows a braille spinner that
//!    resolves to a stable `✓ checked …` line in scrollback.
//! 2. **`run native`:** a sticky multi-line telemetry panel (fps / tick / draw /
//!    frame / a budget bar / last hot-reload) pinned at the bottom, above
//!    scrollback. It refreshes on each `frame_stats` event and stays *alive*
//!    between them via indicatif's cheap `enable_steady_tick` (spinner +
//!    elapsed) — no per-frame work is added to the game loop.

use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use super::{Event, PlainRenderer, Renderer};

/// One in-flight phase's frame-cost sample, kept so a hot-reload can redraw the
/// panel immediately without waiting for the next `frame_stats`.
#[derive(Clone, Copy)]
struct Stats {
    tick_us: f64,
    draw_us: f64,
    frame_us: Option<f64>,
    budget_pct: Option<f64>,
}

/// The sticky `run native` telemetry panel.
struct Panel {
    bar: ProgressBar,
    /// Wall-clock of the previous `frame_stats`, to measure real fps from the
    /// frame count between two samples.
    last_stats_at: Option<Instant>,
    fps: Option<f64>,
    stats: Option<Stats>,
    last_reload: Option<String>,
}

impl Panel {
    /// Rebuild the panel's message block from the latest stats + reload note.
    fn refresh(&self) {
        let mut lines: Vec<String> = Vec::new();
        match self.stats {
            Some(s) => {
                let fps = match self.fps {
                    Some(f) => format!("{f:.0}"),
                    None => "—".to_string(),
                };
                let mut row = format!(
                    "  fps {} · tick {} · draw {}",
                    fps.bold(),
                    format!("{:.0}µs", s.tick_us).cyan(),
                    format!("{:.0}µs", s.draw_us).cyan(),
                );
                if let Some(frame_us) = s.frame_us {
                    row.push_str(&format!(" · frame {}", format!("{frame_us:.0}µs").cyan()));
                }
                lines.push(row);
                if let Some(pct) = s.budget_pct {
                    lines.push(format!("  {}", budget_bar(pct)));
                }
            }
            None => lines.push(format!("  {}", "warming up…".dimmed())),
        }
        if let Some(reload) = &self.last_reload {
            lines.push(format!("  {} {}", super::g_reload().cyan(), reload));
        }
        self.bar.set_message(lines.join("\n"));
    }
}

/// A 20-cell budget bar for the frame's share of the 60 fps (16.7 ms) budget.
/// Green under budget, yellow tight, red over.
fn budget_bar(pct: f64) -> String {
    const CELLS: usize = 20;
    let filled = ((pct / 100.0) * CELLS as f64)
        .round()
        .clamp(0.0, CELLS as f64) as usize;
    let (full, empty) = if super::ascii() {
        ("#", "-")
    } else {
        ("█", "░")
    };
    let bar = format!("{}{}", full.repeat(filled), empty.repeat(CELLS - filled));
    let bar = if pct >= 100.0 {
        bar.red()
    } else if pct >= 70.0 {
        bar.yellow()
    } else {
        bar.green()
    };
    format!("[{bar}] {} of 16.7ms budget", format!("{pct:.0}%").dimmed())
}

struct Inner {
    /// The in-flight phase spinner (checking), if any.
    spinner: Option<ProgressBar>,
    /// The sticky telemetry panel (`run native`), if any.
    panel: Option<Panel>,
    /// The project's basename, for the panel header.
    project: Option<String>,
}

/// The ink-style renderer. `mp` (a cheap `Arc` handle) lives outside the lock so
/// [`cleanup`](Self::cleanup) can wipe the live region from a panic hook /
/// Ctrl-C handler without contending for `inner`.
pub struct LiveRenderer {
    mp: MultiProgress,
    inner: Mutex<Inner>,
}

impl LiveRenderer {
    pub fn new() -> Self {
        // Draw to stdout to keep human output on one fd (as documented). On a
        // non-terminal this target auto-degrades to a no-op, so it can never
        // leak control chars — though selection already gates us to a TTY.
        let mp = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());
        LiveRenderer {
            mp,
            inner: Mutex::new(Inner {
                spinner: None,
                panel: None,
                project: None,
            }),
        }
    }

    /// Commit finished/one-shot lines to stable scrollback (above the live
    /// bars). Reuses the plain renderer's formatting — the single source of
    /// truth for how an event reads.
    fn commit(&self, event: &Event) {
        for line in PlainRenderer::lines(event) {
            let _ = self.mp.println(line);
        }
    }

    fn finish_spinner(inner: &mut Inner) {
        if let Some(pb) = inner.spinner.take() {
            pb.finish_and_clear();
        }
    }

    fn clear_panel(inner: &mut Inner) {
        if let Some(panel) = inner.panel.take() {
            panel.bar.finish_and_clear();
        }
    }
}

impl Renderer for LiveRenderer {
    fn render(&self, event: &Event) {
        let mut inner = self.inner.lock().unwrap();
        match event {
            // The routed commands (build/run/develop) all typecheck first: show
            // a checking spinner that resolves on `mle_loaded`. Other commands
            // (push/init) have no build phase — just commit their line.
            Event::CommandStarted {
                command, project, ..
            } => {
                inner.project = project.as_ref().map(|p| basename(p));
                if matches!(command.as_str(), "build" | "run" | "develop") {
                    let pb = self.mp.add(phase_spinner());
                    pb.set_message(format!(
                        "checking {}",
                        inner.project.as_deref().unwrap_or("project")
                    ));
                    pb.enable_steady_tick(Duration::from_millis(80));
                    inner.spinner = Some(pb);
                } else {
                    self.commit(event);
                }
            }
            // Checking resolved: drop the spinner, commit the stable ✓ line.
            Event::MleLoaded { .. } => {
                Self::finish_spinner(&mut inner);
                self.commit(event);
            }
            // Native runtime is up: retire any spinner, raise the sticky panel.
            Event::RuntimeReady => {
                Self::finish_spinner(&mut inner);
                let bar = self.mp.add(ProgressBar::new_spinner());
                bar.set_style(panel_style());
                bar.set_prefix(inner.project.clone().unwrap_or_default());
                bar.enable_steady_tick(Duration::from_millis(120));
                let panel = Panel {
                    bar,
                    last_stats_at: None,
                    fps: None,
                    stats: None,
                    last_reload: None,
                };
                panel.refresh();
                inner.panel = Some(panel);
            }
            Event::FrameStats {
                tick_us,
                draw_us,
                frame_us,
                budget_pct,
                over_n_frames,
            } => {
                if let Some(panel) = inner.panel.as_mut() {
                    let now = Instant::now();
                    if let Some(prev) = panel.last_stats_at {
                        let secs = now.duration_since(prev).as_secs_f64();
                        if secs > 0.0 {
                            panel.fps = Some(*over_n_frames as f64 / secs);
                        }
                    }
                    panel.last_stats_at = Some(now);
                    panel.stats = Some(Stats {
                        tick_us: *tick_us,
                        draw_us: *draw_us,
                        frame_us: *frame_us,
                        budget_pct: *budget_pct,
                    });
                    panel.refresh();
                } else {
                    // No panel (e.g. stats without a prior ready) — fall back to
                    // a scrollback line rather than dropping the sample.
                    self.commit(event);
                }
            }
            // A hot-reload: record it on the panel (shown immediately) AND keep
            // it in scrollback history.
            Event::HotReload { message, .. } => {
                if let Some(panel) = inner.panel.as_mut() {
                    panel.last_reload = Some(message.clone());
                    panel.refresh();
                }
                self.commit(event);
            }
            // A failed check/load or a fatal error: abandon the spinner, tear
            // the panel down, and commit the diagnostic.
            Event::Diagnostic { .. } => {
                Self::finish_spinner(&mut inner);
                self.commit(event);
            }
            Event::Error { .. } => {
                Self::finish_spinner(&mut inner);
                Self::clear_panel(&mut inner);
                self.commit(event);
            }
            Event::CommandFinished { .. } => {
                Self::finish_spinner(&mut inner);
                Self::clear_panel(&mut inner);
                self.commit(event);
            }
            // Everything else is a plain scrollback line (serving, info,
            // warning, capture, asset error, reload).
            _ => self.commit(event),
        }
    }

    fn cleanup(&self) {
        // Called from the panic hook / Ctrl-C handler: wipe the live region and
        // (belt-and-suspenders) restore the cursor, so the terminal is never
        // left mangled. No `inner` lock is taken (MultiProgress is internally
        // synchronized), so this is safe even mid-render. Write the show-cursor
        // escape to STDOUT — the same fd the live region draws to — so it lands
        // on the terminal even when stderr is redirected, and never leaks a
        // control byte into a captured stderr. Do it FIRST, so recovery does not
        // depend on `clear()` returning.
        {
            let mut out = std::io::stdout().lock();
            let _ = write!(out, "\x1b[?25h");
            let _ = out.flush();
        }
        let _ = self.mp.clear();
    }
}

/// A braille phase spinner (⠋⠙⠹…) in cyan — ASCII (`|/-\`) on a dumb terminal.
fn phase_spinner() -> ProgressBar {
    let ticks = if super::ascii() {
        "|/-\\ "
    } else {
        "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "
    };
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_chars(ticks),
    );
    pb
}

/// The sticky telemetry panel: a header line (animated spinner + elapsed) over a
/// multi-line `{msg}` block of stats.
fn panel_style() -> ProgressStyle {
    let ticks = if super::ascii() {
        "|/-\\|"
    } else {
        "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⠋"
    };
    ProgressStyle::with_template("{spinner:.green.bold} running {prefix:.bold} · {elapsed}\n{msg}")
        .unwrap()
        .tick_chars(ticks)
}

/// Last path component, for a compact project label.
fn basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}
