//! The MLE producer (docs/mle.md Track C2): game logic written in `.mle`,
//! run by the real interpreter (`mle::Session`) with the Functor prelude
//! (`Scene.*` / `Camera.*` / `Frame.*` — see
//! `functor_runtime_common::mle_prelude`). This replaces the Milestone-0
//! throwaway spike (`mle_spike.rs`, deleted with this producer's arrival).
//!
//! Game contract (see the `mle-language` skill and `examples/mle-hello`):
//!
//! ```text
//! let init = { … }                       // the initial model (a value)
//! let tick = (model, dt, tts) => model'  // per-frame step
//! let draw = (model, tts) => Frame.create(camera, scene)
//! // optional MVU pair (C4b-2) — timer messages fold through update:
//! let update = (model, msg) => model'
//! let subscriptions = (model) => Sub.every(Time.seconds(1.0), Msg)
//! let physics = (model) => Physics.scene(gx, gy, gz, [body, …])  // OPTIONAL
//! let ui = (model) => Ui.column([…]) |> Ui.panel(Ui.topLeft())   // OPTIONAL HUD
//! ```
//!
//! Frame order with physics: tick → physics (reconcile + fixed-step the
//! singleton world) → draw, so `Physics.position`/`Physics.transformed` in
//! `draw` read the frame's stepped world. The world lives in this process's
//! registry, so like the model it survives hot reload.
//!
//! The model is a plain MLE value the host holds between frames — the
//! serializable-state seam hot-reload (C3) will swap sessions around.
//! Per-frame errors print and keep the previous model/frame (a bad frame
//! must not kill the session); load errors fail loud at startup.

use std::time::Instant;

use functor_runtime_common::mle_prelude::{
    audio_scene_of, clear_audio_completions, clear_http_taggers, frame_value, view_value,
    EffectLog, EffectRunner, EffectTree, FunctorHost, NetEventKind, RealEffects,
};
use functor_runtime_common::mle_producer::{FrameCtx, Reporter, SpanSource};
use functor_runtime_common::physics;
use functor_runtime_common::timetravel::SceneRecorder;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use mle::project::SourceMap;
use mle::{Session, Value};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::game::Game;

pub struct MleGame {
    path: String,
    /// Per-file mtimes of the WHOLE project (every sibling `.mle` — B8:
    /// file = module), so editing a non-entry module hot-reloads too; a
    /// file appearing or disappearing changes the stamp as well.
    stamp: Vec<(PathBuf, SystemTime)>,
    /// The last ENTRY source accepted over `reload_source`, kept so a
    /// sibling-file save reloads AROUND the pushed buffer instead of
    /// reverting the entry to disk. Cleared when the entry file itself
    /// changes on disk (last-write-wins, from either side — the existing
    /// push contract, now per file).
    pushed_entry: Option<String>,
    /// The lowered (merged) module the current session came from — kept so
    /// a reload can rebind model-stored closures (old module × new module).
    module: mle::ir::Module,
    session: Session,
    model: Value,
    has_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_subscriptions: bool,
    /// The previous frame's total-time, the left edge of the `(prev, tts]`
    /// window subscriptions fire over. `None` until the first frame has run
    /// (nothing fires on frame one — mirroring the F# executor). Producer
    /// state, not model state: it survives a hot reload, and the stateless
    /// time-grid semantics of `Sub.every` do the rest.
    prev_tts: Option<f64>,
    has_physics: bool,
    /// The game defines the optional `soundScape` entry point
    /// (`soundScape(model) -> AudioScene`, the continuous-audio hook). Absent =
    /// silence; unlike `subscriptions` it needs no `update`.
    has_soundscape: bool,
    /// The game defines the optional `ui` entry point (`ui(model) -> View`,
    /// the 2D HUD hook).
    has_ui: bool,
    /// The last successfully built HUD View, cached because `Game::ui` is a
    /// `&self` accessor — evaluated beside `draw` each frame. A bad `ui`
    /// keeps the last good view (the `last_frame` rule); a reload that drops
    /// the hook clears it.
    last_view: View,
    /// The last serialized soundscape (`soundScape model` → JSON), cached
    /// because `audio_scene_json` is a `&self` accessor — evaluated beside
    /// `draw` each frame so errors can `&mut`-dedupe. A bad frame keeps the
    /// last good scene; a reload that drops the hook resets it to silence.
    last_soundscape_json: String,
    /// Performs `Effect.*` commands — the real world in the runner; the
    /// drain logic itself is `mle_prelude::drain_effects` (tested there
    /// with fake/replay runners).
    effect_runner: RealEffects,
    /// The structured effect log (last `EFFECT_LOG_CAP` performed effects) —
    /// LLM-readable, and the input format for replay.
    effect_log: EffectLog,
    /// Physics queries deferred by the frame's pre-step drains, performed
    /// right after the physics step so their taggers answer against the
    /// fresh world ("commands apply at the step; queries answer after it").
    deferred_queries: Vec<EffectTree>,
    /// This frame's contact transitions, delivered post-step to the
    /// `Physics.events` taggers of the current `subscriptions(model)`.
    pending_events: Vec<functor_runtime_common::physics::PhysicsEvent>,
    /// The recorded physics drive (docs/physics.md Phase 6): the Timeline
    /// recorder + pause flag + fixed-step accumulator. The World stays in
    /// the registry; this owns the rewind machinery over it.
    physics_rt: physics::SteppedPhysics,
    /// Latest recorder status for the overlay: (fixed frame, paused, history).
    physics_status: (u64, bool, u64),
    /// The coupled time-travel recorder (docs/time-travel.md T1–T3): records the
    /// settled `model` + physics fixed-frame each rendered frame and seeks/
    /// rewinds them together. Shared with the web producer (one tested impl).
    recorder: SceneRecorder,
    /// Endpoint keys of the connections currently declared by
    /// `subscriptions` (`Sub.connect`/`Sub.listen`) — diffed each frame to
    /// open newly-declared ones and close dropped ones. The shell's
    /// connection manager owns the live sockets; this is just the reconcile
    /// key set (kept across hot reload, like the model — Connect is
    /// idempotent).
    live_conn_keys: std::collections::HashSet<String>,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// Per-frame error reporting (dedupe + stderr sink + project-span
    /// rendering) — shared with the web producer (`mle_producer::Reporter`).
    reporter: Reporter,
    // rolling per-frame eval cost, printed every STATS_EVERY frames (the C6
    // perf gate watches these). Physics is engine cost, not MLE eval cost, so
    // it gets its own counter — a heavy scene must not read as an interpreter
    // regression.
    frames: u64,
    tick_ns: u64,
    physics_ns: u64,
    draw_ns: u64,
}

const STATS_EVERY: u64 = 300;

/// A successfully loaded, contract-validated game project.
struct Loaded {
    sources: SourceMap,
    module: mle::ir::Module,
    session: Session,
    init: Value,
    /// The game defines the optional `input` entry point.
    has_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_subscriptions: bool,
    has_physics: bool,
    has_soundscape: bool,
    has_ui: bool,
}

/// Load, check, and contract-validate a game project (B8: the entry plus
/// every sibling `.mle` file — file = module). Errors come back as fully
/// rendered strings (`path:line:col: message`) so `create` can exit loud with
/// them and hot-reload can print-and-keep-running with the same text.
fn load_game(path: &str) -> Result<Loaded, String> {
    load_project(path, None)
}

/// The source-shaped half of [`load_game`]: the pushed source stands in for
/// the ENTRY file (the network reload path, `reload_source`); sibling
/// modules still load from disk.
fn load_source(path: &str, src: String) -> Result<Loaded, String> {
    load_project(path, Some(src))
}

fn load_project(path: &str, entry_src: Option<String>) -> Result<Loaded, String> {
    let project = mle::project::load_with_entry_source(std::path::Path::new(path), entry_src)
        .map_err(|e| format!("cannot load {}", e.render()))?;
    // Type diagnostics are advisory in the dev loop: print, keep going.
    for diag in project.check() {
        eprintln!(
            "warning: {}",
            project.sources.render(diag.span.start, &diag.message)
        );
    }
    let module = project.module;
    let sources = project.sources;
    let session = Session::load(&module, &mut FunctorHost).map_err(|f| {
        format!(
            "cannot load {}",
            sources.render(f.error.span.start, &f.error.message)
        )
    })?;
    // The producer contract is knowable at load — fail here, not once per
    // frame: `init` must be a model VALUE, `tick`/`draw` functions of the
    // right arity.
    let init = session
        .global("init")
        .ok_or_else(|| format!("{path} has no top-level `let init = …`"))?;
    if matches!(
        init,
        Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_)
    ) {
        return Err(format!(
            "{path}: `init` must be a model value, not a function"
        ));
    }
    if functor_runtime_common::mle_prelude::contains_effect(&init) {
        return Err(format!(
            "{path}: `init` contains an Effect value — Effects are commands, not data; \
return them beside the model as `(model, effect)`"
        ));
    }
    require_function(path, &session, "tick", 3)?;
    require_function(path, &session, "draw", 2)?;
    // `input` is optional (many games are non-interactive), but when present
    // it must honor the contract: (model, key, isDown) => model.
    let has_input = session.global("input").is_some();
    if has_input {
        require_function(path, &session, "input", 3)?;
    }
    // Same deal for the mouse: `mouseMove(model, x, y)` in window pixels,
    // `mouseWheel(model, delta)`.
    let has_mouse_move = session.global("mouseMove").is_some();
    if has_mouse_move {
        require_function(path, &session, "mouseMove", 3)?;
    }
    let has_mouse_wheel = session.global("mouseWheel").is_some();
    if has_mouse_wheel {
        require_function(path, &session, "mouseWheel", 2)?;
    }
    // The MVU pair: `subscriptions(model)` declares timers whose fired
    // messages fold through `update(model, msg)` — so subscriptions without
    // an update have nowhere to deliver.
    let has_subscriptions = session.global("subscriptions").is_some();
    if has_subscriptions {
        require_function(path, &session, "subscriptions", 1)?;
        if session.global("update").is_none() {
            return Err(format!(
                "{path}: `subscriptions` produces messages but there is no \
`let update = (model, msg) => …` to receive them"
            ));
        }
    }
    if session.global("update").is_some() {
        require_function(path, &session, "update", 2)?;
    }
    // Optional physics: `physics(model) => Physics.scene(…)` declares the
    // bodies that should exist; the host reconciles + fixed-steps the world
    // after each tick (docs/physics.md).
    let has_physics = session.global("physics").is_some();
    if has_physics {
        require_function(path, &session, "physics", 1)?;
    }
    // Optional soundscape: `soundScape(model)` returns an AudioScene (the
    // continuous, reconciled half of audio). No `update` requirement — the
    // scene is reconciled by the shell, not folded back as a message.
    let has_soundscape = session.global("soundScape").is_some();
    if has_soundscape {
        require_function(path, &session, "soundScape", 1)?;
    }
    // Optional HUD: `ui(model)` returns a View (Ui.text / Ui.column /
    // Ui.panel), lowered to the shared text overlay — the F# `ui` hook.
    let has_ui = session.global("ui").is_some();
    if has_ui {
        require_function(path, &session, "ui", 1)?;
    }
    Ok(Loaded {
        sources,
        module,
        session,
        init,
        has_input,
        has_mouse_move,
        has_mouse_wheel,
        has_subscriptions,
        has_physics,
        has_soundscape,
        has_ui,
    })
}

/// Per-file mtimes of every `.mle` file in the entry's project, sorted by
/// path — the hot-reload change stamp. Any edited, added, or removed file
/// changes the stamp (a file we cannot stat contributes UNIX_EPOCH, so a
/// mid-save disappearing file still registers as a change).
/// The entry file's mtime within a stamp ([`project_files`] lists the
/// entry first).
fn entry_mtime(stamp: &[(PathBuf, SystemTime)]) -> Option<SystemTime> {
    stamp.first().map(|(_, mtime)| *mtime)
}

fn project_stamp(path: &str) -> Vec<(PathBuf, SystemTime)> {
    let files = mle::project::project_files(std::path::Path::new(path)).unwrap_or_default();
    files
        .into_iter()
        .map(|file| {
            let mtime = std::fs::metadata(&file)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (file, mtime)
        })
        .collect()
}

impl MleGame {
    pub fn create(path: &str) -> MleGame {
        // Stat BEFORE reading: an edit that lands mid-load then compares
        // unequal on the next frame and triggers a reload, instead of being
        // silently absorbed into a stale session.
        let stamp = project_stamp(path);
        let loaded = match load_game(path) {
            Ok(loaded) => loaded,
            Err(message) => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
        };
        println!("[mle] loaded {path}");
        MleGame {
            path: path.to_string(),
            stamp,
            pushed_entry: None,
            module: loaded.module,
            session: loaded.session,
            model: loaded.init,
            has_input: loaded.has_input,
            has_mouse_move: loaded.has_mouse_move,
            has_mouse_wheel: loaded.has_mouse_wheel,
            has_subscriptions: loaded.has_subscriptions,
            prev_tts: None,
            has_physics: loaded.has_physics,
            has_soundscape: loaded.has_soundscape,
            has_ui: loaded.has_ui,
            last_view: View::Empty,
            last_soundscape_json: empty_soundscape_json(),
            effect_runner: RealEffects::new(),
            effect_log: EffectLog::new(),
            deferred_queries: Vec::new(),
            pending_events: Vec::new(),
            physics_rt: physics::SteppedPhysics::new(),
            physics_status: (0, false, 0),
            recorder: SceneRecorder::new(),
            live_conn_keys: std::collections::HashSet::new(),
            last_frame: empty_frame(),
            reporter: Reporter::new(SpanSource::Project(loaded.sources), report_to_stderr),
            frames: 0,
            tick_ns: 0,
            physics_ns: 0,
            draw_ns: 0,
        }
    }

    /// Bundle this producer's per-frame state into the shared [`FrameCtx`]
    /// (docs/time-travel.md T6a) — the frame body and its helpers (`absorb`,
    /// `pump_subscriptions`, `step_physics`, `deliver_*`) live there, one copy
    /// for both shells. A cheap borrow-only view, rebuilt per call.
    fn ctx(&mut self) -> FrameCtx<'_> {
        FrameCtx {
            session: &self.session,
            model: &mut self.model,
            physics_rt: &mut self.physics_rt,
            physics_status: &mut self.physics_status,
            recorder: &mut self.recorder,
            effect_runner: &mut self.effect_runner as &mut dyn EffectRunner,
            effect_log: &mut self.effect_log,
            deferred_queries: &mut self.deferred_queries,
            pending_events: &mut self.pending_events,
            live_conn_keys: &mut self.live_conn_keys,
            prev_tts: &mut self.prev_tts,
            has_physics: self.has_physics,
            has_subscriptions: self.has_subscriptions,
            suppress_outbound: false,
            reporter: &mut self.reporter,
        }
    }

    /// Swap in a freshly loaded program, KEEPING THE MODEL — the shared tail
    /// of both reload paths (file watch and network push). `init` from the
    /// new program is deliberately unused: state survives the edit, and
    /// closures stored in the model rebind to the edited code (B5 part 2,
    /// `mle::rebind_value`). The physics world is deliberately KEPT too, like
    /// the model: it lives in this process's registry, so bodies stay where
    /// they are across the edit and the next frame's declaration re-diffs
    /// against them (removing the `physics` hook drops the world). `prev_tts`
    /// is kept as well: `Sub.every` fires on the global time grid, so timers
    /// tick right through a reload. Returns the number of stored closures
    /// rebound, for the status line.
    fn swap_in(&mut self, loaded: Loaded) -> usize {
        let (model, report) = mle::rebind_value(&self.model, &self.module, &loaded.module);
        self.model = model;
        for warning in &report.warnings {
            eprintln!("[mle] reload: {warning}");
        }
        self.reporter.set_source(SpanSource::Project(loaded.sources));
        self.module = loaded.module;
        self.session = loaded.session;
        self.has_input = loaded.has_input;
        self.has_mouse_move = loaded.has_mouse_move;
        self.has_mouse_wheel = loaded.has_mouse_wheel;
        self.has_subscriptions = loaded.has_subscriptions;
        self.has_physics = loaded.has_physics;
        if !self.has_physics {
            physics::remove_world(physics::DEFAULT_WORLD);
        }
        self.has_soundscape = loaded.has_soundscape;
        if !self.has_soundscape {
            // Deleting the `soundScape` hook drops the soundscape to silence
            // (the physics-world / `ui` rule); the shell reconciles the empty
            // scene next frame, stopping every live voice.
            self.last_soundscape_json = empty_soundscape_json();
        }
        self.has_ui = loaded.has_ui;
        if !self.has_ui {
            // Deleting the `ui` hook drops the HUD (the physics-world rule).
            self.last_view = View::Empty;
        }
        self.reporter.reset();
        // A deferred query or in-flight HTTP request holds a tagger — a closure
        // into the OLD session; drop them rather than let them dangle. A late
        // HTTP response for a dropped token arrives orphaned and is ignored.
        self.deferred_queries.clear();
        self.pending_events.clear();
        clear_http_taggers();
        // In-flight `playThen` completion messages close over the OLD session
        // too — drop them alongside the HTTP taggers (a late finish for a
        // dropped token arrives orphaned and is ignored).
        clear_audio_completions();
        // Reload is a model-history BOUNDARY: the retained snapshots can hold
        // closures bound to the old module, so — unlike the live model, which
        // `rebind_value` migrates above — they can't safely cross a reload. The
        // recorder keeps its rendered-frame clock monotonic so recording resumes
        // consecutively. (Rebinding snapshots to preserve rewind ACROSS an edit
        // is deferred to when that feature is built — docs/time-travel.md.)
        self.recorder.reset_on_reload();
        report.rebound
    }

    fn report_stats(&mut self) {
        if self.frames > 0 && self.frames % STATS_EVERY == 0 {
            let tick_us = self.tick_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let physics_us = self.physics_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let draw_us = self.draw_ns as f64 / STATS_EVERY as f64 / 1000.0;
            println!(
                "[mle] avg over {STATS_EVERY} frames: tick {tick_us:.1}µs, physics \
                 {physics_us:.1}µs, draw {draw_us:.1}µs ({:.1}% of a 60fps budget)",
                (tick_us + physics_us + draw_us) / 16_666.0 * 100.0
            );
            self.tick_ns = 0;
            self.physics_ns = 0;
            self.draw_ns = 0;
        }
    }

}

impl Game for MleGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        // Poll every project file's mtime (a few stats per frame is ~free)
        // and swap in a new session on change — editing a SIBLING module
        // hot-reloads exactly like editing the entry (B8). THE MODEL IS
        // KEPT: it is a plain value the host holds, so state survives the
        // edit and all functions rebind — the dev-loop payoff the language
        // was built for (docs/mle.md C3). Closures STORED IN THE MODEL
        // rebind too (B5 part 2, `mle::rebind`): they adopt the edited code
        // with their captured env carried over; one that can't be matched
        // keeps its old body with a loud warning. A broken edit prints and
        // keeps the old program running.
        let stamp = project_stamp(&self.path);
        if stamp == self.stamp {
            return;
        }
        // Disk wins for the ENTRY only when the entry file itself changed;
        // a sibling-only change reloads around a pushed entry buffer, so a
        // live-preview push isn't silently reverted by editing a sibling.
        let entry_changed = entry_mtime(&stamp) != entry_mtime(&self.stamp);
        self.stamp = stamp;
        if entry_changed {
            self.pushed_entry = None;
        }
        let started = Instant::now();
        let loaded = match &self.pushed_entry {
            Some(src) => load_source(&self.path, src.clone()),
            None => load_game(&self.path),
        };
        match loaded {
            Ok(loaded) => {
                let rebound = self.swap_in(loaded);
                let stored = if rebound > 0 {
                    format!("; {rebound} stored closure(s) rebound")
                } else {
                    String::new()
                };
                println!(
                    "[mle] hot-reloaded {} in {:.2}ms (model preserved{stored}; an edited \
`init` takes effect on restart)",
                    self.path,
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(message) => {
                self.reporter.report_once(format!(
                    "[mle] reload failed, keeping old program: {message}"
                ));
            }
        }
    }

    fn reload_source(&mut self, source: &str) -> Result<String, String> {
        // Same semantics as the file-watch path: model preserved, a broken
        // push keeps the old program (and the error goes back to the pusher,
        // who is looking at the source that caused it). The on-disk file is
        // untouched — a later save still wins via the mtime watcher
        // (last-write-wins, from either side).
        let started = Instant::now();
        // Stamp BEFORE reading sources (the same rule as `create`): a
        // sibling saved mid-load then compares unequal next frame and
        // triggers a reload (around the pushed entry), instead of its mtime
        // being absorbed while the session holds its stale content. Any save
        // already on disk here is either in this load or older than the
        // push — absorbing its mtime is correct either way.
        let stamp = project_stamp(&self.path);
        let loaded = load_source(&self.path, source.to_string())?;
        self.pushed_entry = Some(source.to_string());
        let rebound = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        self.stamp = stamp;
        let status = format!(
            "reloaded {} from pushed source in {:.2}ms (model preserved{stored})",
            self.path,
            started.elapsed().as_secs_f64() * 1000.0
        );
        println!("[mle] {status}");
        Ok(status)
    }

    /// Coupled scene rewind — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T1). Restores model + world to `target` and branches
    /// the future; exact-or-refused. After a successful branch, drop any
    /// in-flight frame work so it doesn't carry across (the reload discipline).
    fn rewind_scene_to(&mut self, target: u64) -> Result<String, String> {
        let result = self.recorder.rewind_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_status,
            self.has_physics,
        );
        if result.is_ok() {
            // No in-flight frame work should carry across the branch (matches
            // the reload discipline); between-frame callers have these empty.
            self.deferred_queries.clear();
            self.pending_events.clear();
            clear_http_taggers();
            clear_audio_completions();
        }
        result
    }

    fn current_scene_frame(&self) -> Option<u64> {
        self.recorder.current_scene_frame()
    }

    fn scene_frame_range(&self) -> Option<(u64, u64)> {
        self.recorder.scene_frame_range()
    }

    fn current_scene_tts(&self) -> Option<f64> {
        self.recorder.current_scene_frame_tts()
    }

    /// Non-destructive scrub — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T3): restore model + world for display without
    /// truncating, so the draggable bar can seek back and forth.
    fn seek_scene_to(&mut self, target: u64) -> Result<String, String> {
        self.recorder.seek_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_status,
            self.has_physics,
        )
    }

    fn tick(&mut self, frame_time: FrameTime) {
        // The frame body lives in the shared `FrameCtx` (docs/time-travel.md
        // T6a); native splits it at the physics boundary to keep the separate
        // `tick_ns` / `physics_ns` perf counters the C6 gate watches.
        let started = Instant::now();
        self.ctx().before_physics(frame_time);
        self.tick_ns += started.elapsed().as_nanos() as u64;
        let physics_started = Instant::now();
        self.ctx().physics_phase(frame_time);
        self.physics_ns += physics_started.elapsed().as_nanos() as u64;
        self.ctx().record_frame(frame_time);
        self.frames += 1;
        self.report_stats();
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        // The optional `input` entry point: (model, key, isDown) => model.
        // Keys cross as their canonical names ("W", "Up", "Space") — the same
        // spelling the debug server and SDK use.
        if !self.has_input {
            return;
        }
        let Some(key) = functor_runtime_common::Key::from_i32(code) else {
            return;
        };
        let args = vec![
            self.model.clone(),
            Value::String(std::rc::Rc::from(format!("{key:?}").as_str())),
            Value::Bool(is_down),
        ];
        match self.session.call("input", args, &mut FunctorHost) {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error("input", &err),
        }
    }
    fn mouse_move(&mut self, x: i32, y: i32) {
        if !self.has_mouse_move {
            return;
        }
        let args = vec![
            self.model.clone(),
            Value::Number(x as f64),
            Value::Number(y as f64),
        ];
        match self.session.call("mouseMove", args, &mut FunctorHost) {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error("mouseMove", &err),
        }
    }

    fn mouse_wheel(&mut self, delta: i32) {
        if !self.has_mouse_wheel {
            return;
        }
        let args = vec![self.model.clone(), Value::Number(delta as f64)];
        match self.session.call("mouseWheel", args, &mut FunctorHost) {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error("mouseWheel", &err),
        }
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        let started = Instant::now();
        // While scrubbing, draw at the scrubbed frame's recorded `tts` so
        // `tts`-driven visuals (orbiting lights, `sin(tts)` motion) rewind with
        // the model; live play uses the real clock (docs/time-travel.md).
        let tts = self
            .recorder
            .scrub_render_tts()
            .unwrap_or(frame_time.tts as f64);
        let args = vec![self.model.clone(), Value::Number(tts)];
        match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => self.last_frame = frame.clone(),
                None => self.reporter.report_once(format!(
                    "[mle] draw must return Frame.create(camera, scene), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.reporter.frame_error("draw", &err),
        }
        // The optional HUD, evaluated beside `draw` (same settled model) and
        // cached — `Game::ui` is a `&self` accessor, and errors need `&mut`
        // dedupe. A bad `ui` keeps the last good view (the last_frame rule).
        if self.has_ui {
            match self
                .session
                .call("ui", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(value) => match view_value(&value) {
                    Some(view) => self.last_view = view.clone(),
                    None => self.reporter.report_once(format!(
                        "[mle] ui must return a View (Ui.text / Ui.column / Ui.panel), got {}",
                        value.kind_name()
                    )),
                },
                Err(err) => self.reporter.frame_error("ui", &err),
            }
        }
        // The optional soundscape, evaluated beside `draw` (same settled model)
        // and cached — `audio_scene_json` is a `&self` accessor, and errors
        // need `&mut` dedupe. A bad frame keeps the last good scene (the
        // last_frame rule).
        if self.has_soundscape {
            match self
                .session
                .call("soundScape", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(value) => match audio_scene_of(&value) {
                    Some(scene) => {
                        self.last_soundscape_json =
                            functor_runtime_common::audio::scene_to_json(scene)
                    }
                    None => self.reporter.report_once(format!(
                        "[mle] soundScape must return an AudioScene (AudioScene.create / \
AudioScene.empty), got {}",
                        value.kind_name()
                    )),
                },
                Err(err) => self.reporter.frame_error("soundScape", &err),
            }
        }
        self.draw_ns += started.elapsed().as_nanos() as u64;
        // On failure this is the last good frame — a bad draw must not blank
        // the screen.
        self.last_frame.clone()
    }

    fn ui(&self) -> View {
        // The game's own `ui` view, plus a read-only recorder status line
        // while paused (docs/physics.md, the culmination) — shown only when
        // paused so live play stays clean. All physics CONTROL is via the
        // game's keyboard bindings (egui input isn't wired). Composing via a
        // column keeps both: an `Empty` game view (e.g. mle-physics has no
        // `ui` hook) renders nothing, leaving just the status.
        let (frame, paused, history) = self.physics_status;
        if !paused {
            return self.last_view.clone();
        }
        let status = View::Text {
            text: format!(
                "physics ⏸ frame {frame} · {history} recorded · Left/Right scrub · Space resume"
            ),
            color: [255, 255, 255],
            font: None,
        };
        View::Column(vec![self.last_view.clone(), status])
    }

    fn state_debug(&self) -> String {
        self.model.to_string()
    }

    fn net_drain_commands(&self) -> String {
        // HttpRequest commands (Effect.httpGet/httpPost), performed by the
        // shell's net_dispatch; the response returns via net_push_http_*.
        functor_runtime_common::net::drain_commands_json()
    }
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String) {
        self.ctx().deliver_http_result(functor_runtime_common::net::HttpResult {
            token: token as u64,
            status: status as u16,
            body: body.into_bytes(),
            error: None,
        });
    }
    fn net_push_http_error(&mut self, token: i32, message: String) {
        self.ctx().deliver_http_result(functor_runtime_common::net::HttpResult {
            token: token as u64,
            status: 0,
            body: Vec::new(),
            error: Some(message),
        });
    }
    fn audio_drain_commands(&self) -> String {
        // One-shot commands (Effect.play/playAt/playThen), performed by the
        // shell's audio device; a playThen finish returns via audio_push_finished.
        functor_runtime_common::audio::drain_commands_json()
    }
    fn audio_scene_json(&self) -> String {
        // The continuous soundscape (`soundScape model`), reconciled by the
        // shell against its live voices. Evaluated + cached in `render` (the
        // `ui` pattern) so this stays a cheap `&self` read.
        self.last_soundscape_json.clone()
    }
    fn net_drain_conn_commands(&self) -> String {
        functor_runtime_common::net::drain_conn_commands_json()
    }
    fn net_push_connected(&mut self, key: String, conn: i32) {
        self.ctx().deliver_net_event(key, NetEventKind::Connected, conn, String::new());
    }
    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String) {
        self.ctx().deliver_net_event(key, NetEventKind::Message, conn, text);
    }
    fn net_push_disconnected(&mut self, key: String, conn: i32) {
        self.ctx().deliver_net_event(key, NetEventKind::Disconnected, conn, String::new());
    }
    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String) {
        self.ctx().deliver_net_event(key, NetEventKind::Error, conn, message);
    }
    fn audio_push_finished(&mut self, token: i32) {
        self.ctx().deliver_audio_completion(token as u64);
    }

    fn quit(&mut self) {
        self.ctx().close_all_connections();
    }
}

/// `name` must be a function of `arity` params — a contract violation is
/// reportable at load, and the alternative is one error per frame, forever.
fn require_function(path: &str, session: &Session, name: &str, arity: usize) -> Result<(), String> {
    match session.global(name) {
        Some(Value::Closure(closure)) if closure.params.len() == arity => Ok(()),
        Some(Value::Closure(closure)) => Err(format!(
            "{path}: `{name}` must take {arity} parameter(s), takes {}",
            closure.params.len()
        )),
        Some(other) => Err(format!(
            "{path}: `{name}` must be a function, got {}",
            other.kind_name()
        )),
        None => Err(format!("{path} has no top-level `let {name} = …`")),
    }
}

/// The desktop `Reporter` sink: per-frame problems go to stderr.
fn report_to_stderr(message: &str) {
    eprintln!("{message}");
}

/// The silent soundscape's wire form — the default before/without a
/// `soundScape` hook (matches `AudioScene::default()` serialized).
fn empty_soundscape_json() -> String {
    "{\"sources\":[]}".to_string()
}

fn empty_frame() -> Frame {
    use cgmath::{Matrix4, SquareMatrix};
    Frame::new(
        functor_runtime_common::Camera::default(),
        functor_runtime_common::Scene3D {
            obj: functor_runtime_common::SceneObject::Group(vec![]),
            xform: Matrix4::identity(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The net conn-command queue is process-global, so the two net tests
    // below must not run concurrently — serialize them.
    static NET_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The whole networking path, headless — no socket, no GL. Drives a
    /// live `MleGame` for the wsdemo port: declaring `Sub.connect`
    /// reconciles into a `Connect` command; a `Connected` event routes
    /// through the tagger → `update`, storing the id and replying with
    /// `Effect.send`; a `Message` event lands in the model.
    #[test]
    fn websocket_connect_send_receive() {
        let _guard = NET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use functor_runtime_common::net::{drain_conn_commands, ConnCommand};
        const ENDPOINT: &str = "ws://127.0.0.1:9001/echo";
        let dir = std::env::temp_dir().join(format!("mle-net-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("game.mle"),
            "type Conn = | NoConn | Conn(id: Float)\n\
             type Model = { conn: Conn, last: String }\n\
             type Msg = | Ws(ev: Net.NetEvent)\n\
             let init = { conn: NoConn, last: \"\" }\n\
             let update = (m: Model, msg: Msg) =>\n\
               match msg with\n\
               | Ws(ev) =>\n\
                 (match ev with\n\
                  | Net.Connected(id) => ({ m with conn: Conn(id) }, Effect.send(id, \"hello\"))\n\
                  | Net.Message(id, text) => { m with last: text }\n\
                  | Net.Disconnected(id) => { m with conn: NoConn }\n\
                  | Net.Error(id, e) => { m with last: e })\n\
             let subscriptions = (m: Model) => Sub.connect(\"ws://127.0.0.1:9001/echo\", Ws)\n\
             let tick = (m: Model, dt: Float, tts: Float) => m\n\
             let draw = (m: Model, tts: Float) =>\n\
               Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .unwrap();
        let _ = drain_conn_commands(); // clear the shared queue

        let mut game = MleGame::create(dir.join("game.mle").to_str().unwrap());

        // 1. Declaring the connection queues a Connect on the first frame.
        game.tick(FrameTime {
            tts: 0.0,
            dts: 0.016,
        });
        let cmds = drain_conn_commands();
        // Exactly one Connect (declared once) — the dedupe guard. [Codex M]
        let connects = cmds
            .iter()
            .filter(|c| matches!(c, ConnCommand::Connect { key, .. } if key == ENDPOINT))
            .count();
        assert_eq!(connects, 1, "expected exactly one Connect, got {cmds:?}");

        // 2. A Connected event → the game stores the id and replies with send.
        game.net_push_connected(ENDPOINT.to_string(), 5);
        let cmds = drain_conn_commands();
        assert!(
            cmds.iter().any(
                |c| matches!(c, ConnCommand::Send { conn, payload } if *conn == 5 && payload == b"hello")
            ),
            "expected Send(5, hello), got {cmds:?}"
        );

        // 3. A Message event lands in the model.
        game.net_push_conn_message(ENDPOINT.to_string(), 5, "echo".to_string());
        assert!(
            game.state_debug().contains("echo"),
            "model should hold the message: {}",
            game.state_debug()
        );
    }

    /// The server (Sub.listen) path with a CLOSURE tagger: listening queues
    /// a Listen command, a client Connected event greets THAT client by id,
    /// and a Message is echoed back to its sender.
    #[test]
    fn websocket_server_listen_greet_echo() {
        let _guard = NET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use functor_runtime_common::net::{drain_conn_commands, ConnCommand};
        const BIND: &str = "127.0.0.1:9001";
        let dir = std::env::temp_dir().join(format!("mle-net-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("game.mle"),
            "type Model = { clients: Float, last: String }\n\
             type Msg = | Joined(id: Float) | Got(id: Float, text: String) | Left(id: Float)\n\
             let toMsg = (ev: Net.NetEvent): Msg =>\n\
               match ev with\n\
               | Net.Connected(id) => Joined(id)\n\
               | Net.Message(id, text) => Got(id, text)\n\
               | Net.Disconnected(id) => Left(id)\n\
               | Net.Error(id, e) => Left(id)\n\
             let init = { clients: 0.0, last: \"\" }\n\
             let update = (m: Model, msg: Msg) =>\n\
               match msg with\n\
               | Joined(id) => ({ m with clients: m.clients + 1.0 }, Effect.send(id, \"welcome\"))\n\
               | Got(id, text) => ({ m with last: text }, Effect.send(id, text))\n\
               | Left(id) => { m with clients: m.clients - 1.0 }\n\
             let subscriptions = (m: Model) => Sub.listen(\"127.0.0.1:9001\", toMsg)\n\
             let tick = (m: Model, dt: Float, tts: Float) => m\n\
             let draw = (m: Model, tts: Float) =>\n\
               Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .unwrap();
        let _ = drain_conn_commands();
        let mut game = MleGame::create(dir.join("game.mle").to_str().unwrap());

        game.tick(FrameTime {
            tts: 0.0,
            dts: 0.016,
        });
        let cmds = drain_conn_commands();
        assert!(
            cmds.iter()
                .any(|c| matches!(c, ConnCommand::Listen { key, .. } if key == BIND)),
            "expected a Listen for {BIND}, got {cmds:?}"
        );

        // Two clients connect; each is greeted by its OWN id.
        game.net_push_connected(BIND.to_string(), 11);
        game.net_push_connected(BIND.to_string(), 22);
        let cmds = drain_conn_commands();
        assert!(cmds.iter().any(
            |c| matches!(c, ConnCommand::Send { conn: 11, payload } if payload == b"welcome")
        ));
        assert!(cmds.iter().any(
            |c| matches!(c, ConnCommand::Send { conn: 22, payload } if payload == b"welcome")
        ));
        assert!(
            game.state_debug().contains("clients: 2"),
            "{}",
            game.state_debug()
        );

        // A message from client 22 is echoed back to 22.
        game.net_push_conn_message(BIND.to_string(), 22, "ping".to_string());
        let cmds = drain_conn_commands();
        assert!(cmds
            .iter()
            .any(|c| matches!(c, ConnCommand::Send { conn: 22, payload } if payload == b"ping")));
    }

    /// Write `src` as `game.mle` in its own temp directory (a directory is
    /// a whole project since B8 — a shared temp dir would drag stray `.mle`
    /// files in as sibling modules) and return `load_game`'s error.
    fn load_err(name: &str, src: &str) -> String {
        let dir = std::env::temp_dir().join(format!("mle-game-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let path = dir.join("game.mle");
        std::fs::write(&path, src).expect("write temp game");
        let err = load_game(path.to_str().expect("utf-8 temp path"))
            .err()
            .expect("load should fail");
        let _ = std::fs::remove_dir_all(&dir);
        err
    }

    const BASE: &str = "let init = { n: 0.0 }\n\
         let tick = (m, dt, tts) => m\n\
         let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n";

    /// Subscriptions produce messages; without an `update` they have nowhere
    /// to go — a load error, not a per-frame one.
    #[test]
    fn subscriptions_require_update() {
        let err = load_err(
            "subs-no-update",
            &format!("{BASE}let subscriptions = (m) => Sub.none()\n"),
        );
        assert!(
            err.contains("no `let update = (model, msg) => …` to receive them"),
            "unexpected error: {err}"
        );
    }

    /// Effects are commands, not data: an Effect inside `init` would make
    /// the pair sniff ambiguous — rejected at load. [Codex H — B6 review]
    #[test]
    fn init_containing_an_effect_is_rejected() {
        let err = load_err(
            "init-effect",
            "let init = (0.0, Effect.none())
             let tick = (m, dt, tts) => m
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
",
        );
        assert!(
            err.contains("`init` contains an Effect value"),
            "unexpected error: {err}"
        );
    }

    /// A pushed entry buffer survives a SIBLING-file reload: editing
    /// `config.mle` must reload around the pushed `game.mle`, and only an
    /// on-disk edit of the entry itself reverts to disk (last-write-wins,
    /// per file). [Codex Medium — B8 review]
    #[test]
    fn pushed_entry_survives_sibling_reloads() {
        let dir = std::env::temp_dir().join(format!("mle-game-test-{}-push", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let entry = dir.join("game.mle");
        let disk_game = format!("{BASE}let probe = 1.0\n");
        std::fs::write(&entry, &disk_game).expect("write entry");
        std::fs::write(dir.join("config.mle"), "let k = 1.0\n").expect("write sibling");
        let mut game = MleGame::create(entry.to_str().expect("utf-8 path"));

        // Push an entry variant distinguishable from the disk one.
        let pushed = format!("{BASE}let probe = 2.0\n");
        game.reload_source(&pushed).expect("push should load");
        assert_eq!(
            game.session.global("probe").expect("probe").to_string(),
            "2"
        );

        // Edit the SIBLING: the reload must keep the pushed entry.
        std::thread::sleep(std::time::Duration::from_millis(20)); // distinct mtime
        std::fs::write(dir.join("config.mle"), "let k = 5.0\n").expect("edit sibling");
        game.check_hot_reload(FrameTime { tts: 0.0, dts: 0.0 });
        assert_eq!(
            game.session.global("probe").expect("probe").to_string(),
            "2",
            "a sibling edit must not revert the pushed entry"
        );
        assert_eq!(
            game.session.global("Config.k").expect("k").to_string(),
            "5",
            "the sibling edit itself must have landed"
        );

        // Edit the ENTRY on disk: disk wins, the push is dropped.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&entry, format!("{BASE}let probe = 3.0\n")).expect("edit entry");
        game.check_hot_reload(FrameTime { tts: 0.0, dts: 0.0 });
        assert_eq!(
            game.session.global("probe").expect("probe").to_string(),
            "3",
            "an on-disk entry edit wins over the stale push"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The optional `ui` HUD hook is arity-validated at load like every
    /// other entry point: `ui(model)`, one parameter.
    #[test]
    fn ui_arity_is_validated() {
        let err = load_err(
            "ui-arity",
            &format!("{BASE}let ui = (m, tts) => Ui.text(\"hud\")\n"),
        );
        assert!(
            err.contains("`ui` must take 1 parameter(s), takes 2"),
            "unexpected error: {err}"
        );
        let err = load_err("ui-not-fn", &format!("{BASE}let ui = 3.0\n"));
        assert!(
            err.contains("`ui` must be a function, got a number"),
            "unexpected error: {err}"
        );
    }

    /// The MVU pair is arity-validated at load like every other entry point.
    #[test]
    fn update_arity_is_validated() {
        let err = load_err(
            "update-arity",
            &format!("{BASE}let update = (m) => m\nlet subscriptions = (m) => Sub.none()\n"),
        );
        assert!(
            err.contains("`update` must take 2 parameter(s), takes 1"),
            "unexpected error: {err}"
        );
    }

    /// Recording wiring (docs/time-travel.md T1): each rendered frame's settled
    /// model lands in `model_history`, keyed by the rendered-frame clock, with
    /// the live model left untouched. Drives the real MVU loop headlessly.
    #[test]
    fn model_history_records_each_rendered_frame() {
        fn n_of(v: &Value) -> f64 {
            match v {
                Value::Record(fields) => {
                    match &fields.iter().find(|(k, _)| k == "n").expect("n field").1 {
                        Value::Number(x) => *x,
                        other => panic!("n is not a number: {other}"),
                    }
                }
                other => panic!("model is not a record: {other}"),
            }
        }

        let dir =
            std::env::temp_dir().join(format!("mle-game-test-{}-history", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.mle"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        // Nothing recorded until the first frame runs.
        assert_eq!(game.recorder.scene_frame_range(), None);

        for _ in 0..5 {
            game.tick(FrameTime { tts: 0.0, dts: 0.016 });
        }

        // Five rendered frames, indexed 0..4; recording left the live model
        // untouched (n incremented once per tick).
        assert_eq!(game.recorder.scene_frame_range(), Some((0, 4)));
        assert_eq!(n_of(&game.model), 5.0);
        // Each frame holds that frame's settled model — seeking is exact.
        game.seek_scene_to(0).expect("seek 0");
        assert_eq!(n_of(&game.model), 1.0);
        game.seek_scene_to(4).expect("seek 4");
        assert_eq!(n_of(&game.model), 5.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Hot reload is a model-history boundary (xreview): the ring is reset so
    /// it never retains old-module snapshots, while `rendered_frame` stays
    /// monotonic so recording resumes CONSECUTIVELY after the reload (a stale
    /// non-consecutive record would panic in `History::record`).
    #[test]
    fn hot_reload_resets_history_and_recording_resumes() {
        let dir = std::env::temp_dir()
            .join(format!("mle-game-test-{}-history-reload", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let src = "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n";
        std::fs::write(dir.join("game.mle"), src).expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        for _ in 0..3 {
            game.tick(FrameTime { tts: 0.0, dts: 0.016 });
        }
        assert_eq!(game.recorder.scene_frame_range(), Some((0, 2)));

        // Push a fresh (compatible) source: the model is rebound and KEPT, but
        // the history ring is reset.
        game.reload_source(src).expect("reload should succeed");
        assert_eq!(
            game.recorder.scene_frame_range(),
            None,
            "reload must reset the model history"
        );

        // Recording resumes at the current (monotonic) rendered frame — the
        // fresh ring re-bases there, so no non-consecutive panic.
        game.tick(FrameTime { tts: 0.0, dts: 0.016 });
        assert_eq!(game.recorder.scene_frame_range(), Some((3, 3)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The coupled seek (docs/time-travel.md T1): `rewind_scene_to` restores the
    /// MVU model AND the physics world together to an earlier rendered frame.
    /// Driven headlessly at `dt = FIXED_DT` (one physics step per rendered
    /// frame, so the rendered and fixed clocks stay aligned).
    #[test]
    fn rewind_scene_restores_model_and_physics_together() {
        fn n_of(v: &Value) -> f64 {
            match v {
                Value::Record(fields) => match &fields.iter().find(|(k, _)| k == "n").unwrap().1 {
                    Value::Number(x) => *x,
                    _ => panic!("n not a number"),
                },
                _ => panic!("not a record"),
            }
        }
        fn ball_y() -> f32 {
            physics::with_world(physics::DEFAULT_WORLD, |w| {
                w.body_transform("ball").map(|(pos, _)| pos[1])
            })
            .flatten()
            .expect("ball transform")
        }

        // Isolate the physics world from any prior test on this thread.
        physics::remove_world(physics::DEFAULT_WORLD);

        let dir = std::env::temp_dir()
            .join(format!("mle-game-test-{}-coupled", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.mle"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.fixed(\"ground\", Physics.box(20.0, 0.4, 20.0)) |> Physics.at(0.0, -0.2, 0.0),\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        let dt = FrameTime { tts: 0.0, dts: physics::FIXED_DT };
        // Frames 0..3 (4 ticks): the ball falls under gravity, n counts up.
        for _ in 0..4 {
            game.tick(dt.clone());
        }
        let y_at_3 = ball_y();
        // Frames 4..9 (6 more ticks): ball falls further, n reaches 10.
        for _ in 0..6 {
            game.tick(dt.clone());
        }
        let y_at_9 = ball_y();
        assert_eq!(n_of(&game.model), 10.0);
        assert!(y_at_3 > y_at_9, "ball should have fallen further by frame 9");

        // Rewind the WHOLE scene to rendered frame 3.
        let status = game.rewind_scene_to(3).expect("rewind should succeed");
        assert!(status.contains("frame 3"), "unexpected status: {status}");

        // Model restored to frame 3 (n == 4), physics world restored to the
        // ball's frame-3 pose — byte-exact, so it matches y_at_3 and NOT y_at_9.
        assert_eq!(n_of(&game.model), 4.0, "model did not rewind");
        let y_after = ball_y();
        assert!(
            (y_after - y_at_3).abs() < 1e-5,
            "physics did not rewind to frame 3: {y_after} vs {y_at_3}"
        );
        assert!(
            (y_after - y_at_9).abs() > 1e-4,
            "physics still at frame 9 after rewind"
        );
        // Both rings branched from the seek point (range truncated to 0..3;
        // recording resumes at 4).
        assert_eq!(game.recorder.scene_frame_range(), Some((0, 3)));

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end tts rewind (docs/time-travel.md): a game whose `draw` is
    /// driven by the render clock `tts` (like `examples/mle-lighting`'s orbiting
    /// lights) must, WHILE SCRUBBING, render at the scrubbed frame's RECORDED
    /// tts — not the live "now" clock. Here the camera eye tracks tts, so the
    /// returned `Frame` exposes which tts `draw` actually ran at. Exercises the
    /// real production render path (`render` → `current_scene_tts` override).
    #[test]
    fn scrubbed_frame_renders_at_its_recorded_tts() {
        physics::remove_world(physics::DEFAULT_WORLD);
        let dir =
            std::env::temp_dir().join(format!("mle-game-test-{}-tts", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        // eye.x == tts, so the drawn Frame reveals the tts `draw` ran at.
        std::fs::write(
            dir.join("game.mle"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(tts, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        // Five frames with an advancing render clock: frame i records tts = i+1.
        for i in 0..5u64 {
            game.tick(FrameTime {
                tts: (i + 1) as f32,
                dts: 1.0,
            });
        }
        assert_eq!(game.scene_frame_range(), Some((0, 4)));

        // Live (not scrubbing): render draws at the real clock — eye.x == 42.0.
        let live = game.render(FrameTime { tts: 42.0, dts: 1.0 });
        assert_eq!(live.camera.eye[0], 42.0, "live render uses the real clock");

        // Scrub back to frame 1 (recorded tts = 2.0). Even though render is
        // handed a bogus live tts, `draw` must run at the RECORDED tts, so the
        // tts-driven camera rewinds to eye.x == 2.0 — the bug this fixes.
        game.seek_scene_to(1).expect("seek 1");
        let scrubbed = game.render(FrameTime { tts: 99.0, dts: 0.0 });
        assert_eq!(
            scrubbed.camera.eye[0], 2.0,
            "scrubbed frame must render at its recorded tts, not the live clock"
        );

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The draggable scrubber (docs/time-travel.md T3): `seek_scene_to` is
    /// NON-destructive — you can seek back and forth freely (the range stays
    /// intact) — and the future is branched only when play RESUMES from the
    /// scrubbed frame.
    #[test]
    fn scrub_is_non_destructive_then_branches_on_resume() {
        fn n_of(v: &Value) -> f64 {
            match v {
                Value::Record(fields) => match &fields.iter().find(|(k, _)| k == "n").unwrap().1 {
                    Value::Number(x) => *x,
                    _ => panic!("n not a number"),
                },
                _ => panic!("not a record"),
            }
        }
        physics::remove_world(physics::DEFAULT_WORLD);
        let dir = std::env::temp_dir().join(format!("mle-game-test-{}-scrub", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.mle"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        let dt = FrameTime { tts: 0.0, dts: physics::FIXED_DT };
        for _ in 0..10 {
            game.tick(dt.clone());
        }
        assert_eq!(game.scene_frame_range(), Some((0, 9)));

        // Scrub back, forward, back — the window never shrinks (non-destructive),
        // and the model follows the handle.
        game.seek_scene_to(3).expect("seek 3");
        assert_eq!(n_of(&game.model), 4.0);
        assert_eq!(game.current_scene_frame(), Some(3));
        assert_eq!(game.scene_frame_range(), Some((0, 9)), "seek must not truncate");
        game.seek_scene_to(7).expect("seek 7");
        assert_eq!(n_of(&game.model), 8.0, "can scrub FORWARD again (non-destructive)");
        assert_eq!(game.scene_frame_range(), Some((0, 9)));
        game.seek_scene_to(2).expect("seek 2");
        assert_eq!(n_of(&game.model), 3.0);

        // Resume (dts > 0): the branch commits from frame 2 — the future after 2
        // is discarded, and recording continues at frame 3.
        game.tick(dt.clone());
        assert_eq!(game.current_scene_frame(), Some(3), "no longer scrubbing");
        assert_eq!(game.scene_frame_range(), Some((0, 3)), "future branched away");
        assert_eq!(n_of(&game.model), 4.0, "model advanced from the scrubbed frame");

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rewinding to the LATEST rendered frame is a no-op that must not desync:
    /// that frame's recorded fixed frame equals the live world frame (no step
    /// happened after it), so the world needs no seek and the model is already
    /// current (exercises the `PhysicsSeek::None` guard — the coupled off-by-one
    /// both xreview engines flagged).
    #[test]
    fn rewind_to_latest_frame_does_not_desync() {
        fn ball_y() -> f32 {
            physics::with_world(physics::DEFAULT_WORLD, |w| {
                w.body_transform("ball").map(|(pos, _)| pos[1])
            })
            .flatten()
            .expect("ball transform")
        }

        physics::remove_world(physics::DEFAULT_WORLD);
        let dir =
            std::env::temp_dir().join(format!("mle-game-test-{}-latest", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.mle"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        let dt = FrameTime { tts: 0.0, dts: physics::FIXED_DT };
        for _ in 0..8 {
            game.tick(dt.clone());
        }
        let y_before = ball_y();

        // Latest recorded frame is 7 (0..7).
        let status = game.rewind_scene_to(7).expect("rewind to latest should succeed");
        assert!(status.contains("frame 7"), "unexpected status: {status}");
        // World untouched (no physics seek), model still current.
        assert!((ball_y() - y_before).abs() < 1e-6, "latest-frame rewind moved the world");
        assert_eq!(game.recorder.scene_frame_range(), Some((0, 7)));

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The deterministic headless forward-step (docs/time-travel.md T6b): from a
    /// fork point, `forward_step_scene` steps the whole scene forward N frames
    /// and produces EXACTLY the sequence a live run produces — model (`Value`
    /// via `to_string`) and physics world (snapshot bytes) both byte-equal —
    /// WITHOUT touching the live producer state. The game is pure (no `Now` /
    /// unseeded `Random`): a ball falls onto a slab and a contact counter folds
    /// through `update`, so both the model and the world genuinely evolve and
    /// stay coupled. A game reading wall-clock `Now` / unseeded `Random` would
    /// NOT match — the determinism boundary; a `tts`-driven / seeded game does,
    /// since the forward-step supplies `tts`.
    #[test]
    fn forward_step_is_deterministic_and_non_destructive() {
        // The physics registry is a per-thread thread-local shared by every
        // physics test on this thread — start from an empty world.
        physics::remove_world(physics::DEFAULT_WORLD);

        let dir = std::env::temp_dir().join(format!("mle-fwd-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.mle"),
            "type Msg = | Contact(ev: e)\n\
             let init = { n: 0.0, hits: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.fixed(\"ground\", Physics.box(10.0, 0.4, 10.0)) |> Physics.at(0.0, -0.2, 0.0),\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 4.0, 0.0)])\n\
             let subscriptions = (m) => Physics.events(Contact)\n\
             let update = (m, msg) =>\n\
               match msg with\n\
               | Contact(e) => (match e.started with | true => { m with hits: m.hits + 1.0 } | false => m)\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = MleGame::create(dir.join("game.mle").to_str().expect("utf-8 path"));

        const DT: f32 = physics::FIXED_DT;
        const K: usize = 45;
        const N: usize = 25;

        // Drive K frames to the fork point.
        let mut tts = 0.0f32;
        for _ in 0..K {
            tts += DT;
            game.tick(FrameTime { tts, dts: DT });
        }

        // Capture the fork state + a baseline of the live producer state.
        let fork_model = game.model.clone();
        let fork_prev_tts = game.prev_tts;
        let fork_tts = tts;
        let live_model_before = game.model.to_string();
        let live_world_before = physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot());
        let live_frame_before = game.physics_status.0;

        // Forward-step N frames from the fork — a dry run over throwaway state.
        let forward = functor_runtime_common::mle_producer::forward_step_scene(
            &game.session,
            &fork_model,
            game.has_physics,
            game.has_subscriptions,
            fork_prev_tts,
            fork_tts,
            DT,
            N,
        );

        // The live producer state is UNCHANGED by the forward-step.
        assert_eq!(game.model.to_string(), live_model_before, "model untouched");
        assert_eq!(
            physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot()),
            live_world_before,
            "live world untouched"
        );
        assert_eq!(
            game.physics_status.0, live_frame_before,
            "live fixed frame untouched"
        );

        // The live continuation: the ground truth the forward-step must match.
        let mut live: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(N);
        for _ in 0..N {
            tts += DT;
            game.tick(FrameTime { tts, dts: DT });
            let world = physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot());
            live.push((game.model.to_string(), world));
        }

        assert_eq!(forward.len(), live.len(), "division count");
        // The scene genuinely evolves: the world moves across the window and the
        // ball lands (a Contact folds `hits` up through `update`).
        assert_ne!(live[0].1, live[N - 1].1, "world should move over the window");
        assert!(
            game.model.to_string().contains("hits: ")
                && !game.model.to_string().contains("hits: 0"),
            "the ball should have landed within the window: {}",
            game.model.to_string()
        );
        for (i, ((fwd_m, fwd_w), (live_m, live_w))) in forward.iter().zip(live.iter()).enumerate() {
            assert_eq!(fwd_m.to_string(), *live_m, "model diverged at division {i}");
            assert_eq!(fwd_w, live_w, "world diverged at division {i}");
        }

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
