//! The Functor Lang producer for the web shell (docs/functor-lang.md Track C5): the wasm
//! sibling of the desktop runner's `functor_lang_game.rs`, behind the same
//! `GameProducer` seam. Same load-time contract validation and per-frame
//! semantics — the MVU pair (subscriptions fold through `update` before
//! `tick`), the optional `physics` hook (tick → physics → draw), a bad frame
//! keeps the last good model/frame, per-frame errors dedupe — but adapted to
//! the browser:
//!
//! - the `.fun` source arrives over HTTP (fetched by `run_async` from the dev
//!   server, which serves the project directory) instead of the filesystem;
//! - no file-watch hot reload (there is no filesystem to watch), but the
//!   PUSH path exists (docs/functor-lang.md D4): `reload_source` mirrors the desktop
//!   runner's `POST /reload-source` — parse → lower → check-as-warnings →
//!   `Session::load` → `functor_lang::rebind_value` on the held model — reachable
//!   from the page via the `functor_lang_set_source` wasm export in `lib.rs`;
//! - no per-frame perf stats (`std::time::Instant` panics on wasm; the C6
//!   perf gate measures natively);
//! - input events arrive from the page via the `functor_lang_*` wasm exports below,
//!   queued and drained by the frame loop each frame before `tick` (DOM
//!   handlers fire between rAF callbacks, never mid-frame).

use std::cell::RefCell;

use functor_lang::project::SourceMap;
use functor_lang::{Session, Value};
use functor_runtime_common::functor_lang_prelude::{
    audio_scene_of, clear_audio_completions, clear_http_taggers, contains_effect, frame_value,
    take_ui_handlers, view_value, EffectLog, EffectRunner, EffectTree, FunctorHost, NetEventKind,
    RealEffects, UiHandler,
};
use functor_runtime_common::functor_lang_producer::{
    journal_arm, journal_swap, FrameCtx, JournalEntry, Reporter, SpanSource,
};
use functor_runtime_common::inspector::{build_trace_doc, inspector_sources, InspectorSource};
use functor_runtime_common::physics;
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::timetravel::SceneRecorder;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use wasm_bindgen::prelude::*;

pub struct FunctorLangWebGame {
    path: String,
    /// The project's fetched source files (entry FIRST, then siblings) as
    /// `(path, source)` — the web's stand-in for the on-disk directory the
    /// desktop producer re-reads on reload. A push (`reload_source`) replaces
    /// only the ENTRY buffer; siblings keep their last-fetched text.
    sources: Vec<(String, String)>,
    /// The lowered module the current session came from — kept (like the
    /// desktop producer) so a pushed reload can rebind model-stored closures
    /// (old module × new module).
    module: functor_lang::ir::Module,
    session: Session,
    model: Value,
    has_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_subscriptions: bool,
    /// The previous frame's total-time, the left edge of the `(prev, tts]`
    /// window subscriptions fire over. `None` until the first frame has run
    /// (nothing fires on frame one — mirroring the F# executor and the
    /// desktop producer).
    prev_tts: Option<f64>,
    /// The shell's latest asset-loading snapshot (pushed each frame by the
    /// render loop) and the one the game last saw — the `Sub.assets` seam.
    asset_progress: Option<functor_runtime_common::asset::AssetProgress>,
    delivered_asset_progress: Option<functor_runtime_common::asset::AssetProgress>,
    has_physics: bool,
    /// The game defines the optional `soundScape` entry point
    /// (`soundScape(model) -> AudioScene`, the continuous-audio hook). Absent =
    /// silence; unlike `subscriptions` it needs no `update`.
    has_soundscape: bool,
    /// The last serialized soundscape (`soundScape model` → JSON), cached
    /// because `audio_scene_json` is a `&self` accessor — evaluated + deduped
    /// in `render` (the `ui` pattern), same as the desktop producer.
    last_soundscape_json: String,
    /// The game defines the optional `ui` entry point (`ui(model) -> View`,
    /// the 2D HUD hook).
    has_ui: bool,
    /// The last successfully built HUD View, cached because `ui()` is a
    /// `&self` accessor — evaluated beside `draw` each frame (the desktop
    /// producer's rule: a bad `ui` keeps the last good view).
    last_view: View,
    /// The interactive-widget handler table registered by the `ui(model)`
    /// evaluation that built `last_view` (docs/ui-interaction.md U2), kept in
    /// lockstep with it — same rules as the desktop producer.
    ui_handlers: Vec<UiHandler>,
    /// Performs `Effect.*` commands (B6). `RealEffects` is wasm-safe: its
    /// clock is `Date.now()` on this target.
    effect_runner: RealEffects,
    /// The structured effect log (bounded inside the drain).
    effect_log: EffectLog,
    /// Physics queries deferred by the frame's pre-step drains, performed
    /// right after the physics step so their taggers answer against the
    /// fresh world ("commands apply at the step; queries answer after it").
    deferred_queries: Vec<EffectTree>,
    /// This frame's contact transitions, delivered post-step to the
    /// `Physics.events` taggers of the current `subscriptions(model)`.
    pending_events: Vec<functor_runtime_common::physics::PhysicsEvent>,
    /// The recorded physics drive (docs/physics.md Phase 6): the Timeline
    /// recorder + fixed-step accumulator. The World stays in the registry;
    /// this owns the rewind machinery over it (driven by the shell scrubber).
    physics_rt: physics::SteppedPhysics,
    /// The physics world's fixed frame after the latest advance — what the
    /// coupled scene recorder stores per rendered frame.
    physics_frame: u64,
    /// The coupled time-travel recorder (docs/time-travel.md T1–T3), shared with
    /// the desktop producer (one tested impl): records the settled `model` +
    /// physics fixed-frame each rendered frame and seeks/rewinds them together.
    recorder: SceneRecorder,
    /// This frame's buffered input events (docs/time-travel.md T6b): appended in
    /// `key_event`/`mouse_move`/`mouse_wheel` beside the live `session.call`, and
    /// flushed into `recorder`'s input log by `record_frame` (plain data, so the
    /// log survives a reload). Shared shape with the desktop producer.
    input_buf: Vec<functor_runtime_common::RecordedInput>,
    /// Declared connection keys (`Sub.connect`/`Sub.listen`), reconciled each
    /// frame — see the desktop producer.
    live_conn_keys: std::collections::HashSet<String>,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// Per-frame error reporting (dedupe + browser-console sink + single-source
    /// span rendering) — shared with the desktop producer
    /// (`functor_lang_producer::Reporter`).
    reporter: Reporter,
    /// The DRAW error currently shown on the page's red overlay, if any (the
    /// web-only "blank screen" guard — a broken `draw` shows the message instead
    /// of a frozen canvas, React-HMR style). Tracked so the DOM is touched only
    /// on a transition (draw breaks / recovers), not every frame.
    overlay_error: Option<String>,
    /// The last real frame's replay journal (visual-debugger PR2b): one entry
    /// per model-updating call, swapped in from the thread-local journal at the
    /// end of each `tick`. A paused frame runs no `tick`, so this is preserved —
    /// the inspector reads the last real frame. Replayed through
    /// `Session::call_recorded` while paused. Mirrors the desktop producer.
    last_frame_journal: Vec<JournalEntry>,
    /// The lazily built + cached inspector-trace JSON for the current paused
    /// frame. Invalidated when the frame advances (`tick`), the paused frame
    /// changes (rewind/seek), or the program reloads.
    cached_trace: Option<String>,
    /// Per-file sha256 of the loaded `.fun` source, computed at load / reload
    /// (not per frame) — the wire contract's `sources`, and the per-file
    /// base→(file, local offset) map for binding spans.
    source_hashes: Vec<InspectorSource>,
}

/// A successfully loaded, contract-validated game module (the desktop
/// producer's `Loaded`, verbatim minus the file-shaped fields).
struct Loaded {
    sources: SourceMap,
    module: functor_lang::ir::Module,
    session: Session,
    init: Value,
    has_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_subscriptions: bool,
    has_physics: bool,
    has_soundscape: bool,
    has_ui: bool,
}

/// Load, check, and contract-validate a game PROJECT — the web counterpart of
/// the desktop `load_source`, shared by the page-load path (`create`) and the
/// editor push path (`reload_source`). `sources` is every fetched project file
/// as `(path, source)`, the ENTRY first, then siblings (`file = module`, so
/// `pieces.fun` is module `Pieces`). Errors come back as fully rendered strings
/// (`path:line:col: message`).
fn load_source(sources: &[(String, String)]) -> Result<Loaded, String> {
    let path = sources
        .first()
        .map(|(p, _)| p.clone())
        .unwrap_or_else(|| "game.fun".to_string());
    let pairs: Vec<(std::path::PathBuf, String)> = sources
        .iter()
        .map(|(p, s)| (std::path::PathBuf::from(p), s.clone()))
        .collect();
    // Link the entry with its siblings, injecting the host prelude `.funi`
    // interfaces so `Scene.*` (etc.) typecheck against real types — the exact
    // path the desktop producer runs (docs/functor-lang-interfaces.md). Check-time only; the
    // FunctorHost still provides the actual runtime values.
    let project =
        functor_lang::project::load_sources_with_prelude(pairs, &functor_prelude::modules())
            .map_err(|e| format!("cannot load {}", e.render()))?;
    let module = project.module;
    let source_map = project.sources;
    // Type diagnostics are advisory in the dev loop: warn, keep going
    // (the CLI's `build` is the strict gate). Spans render against whichever
    // project file they land in.
    for diag in functor_lang::check(&module) {
        web_sys::console::warn_1(
            &format!(
                "warning: {}",
                source_map.render(diag.span.start, &diag.message)
            )
            .into(),
        );
    }
    let session = Session::load(&module, &mut FunctorHost).map_err(|f| {
        format!(
            "cannot load {}",
            source_map.render(f.error.span.start, &f.error.message)
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
    if contains_effect(&init) {
        return Err(format!(
            "{path}: `init` contains an Effect value — Effects are commands, not data; \
return them beside the model as `(model, effect)`"
        ));
    }
    require_function(&path, &session, "tick", 3)?;
    require_function(&path, &session, "draw", 2)?;
    // `input` is optional (many games are non-interactive), but when
    // present it must honor the contract: (model, key, isDown) => model.
    let has_input = session.global("input").is_some();
    if has_input {
        require_function(&path, &session, "input", 3)?;
    }
    // Same deal for the mouse: `mouseMove(model, x, y)` in window pixels,
    // `mouseWheel(model, delta)`.
    let has_mouse_move = session.global("mouseMove").is_some();
    if has_mouse_move {
        require_function(&path, &session, "mouseMove", 3)?;
    }
    let has_mouse_wheel = session.global("mouseWheel").is_some();
    if has_mouse_wheel {
        require_function(&path, &session, "mouseWheel", 2)?;
    }
    // The MVU pair: `subscriptions(model)` declares timers whose fired
    // messages fold through `update(model, msg)` — so subscriptions
    // without an update have nowhere to deliver.
    let has_subscriptions = session.global("subscriptions").is_some();
    if has_subscriptions {
        require_function(&path, &session, "subscriptions", 1)?;
        if session.global("update").is_none() {
            return Err(format!(
                "{path}: `subscriptions` produces messages but there is no \
`let update = (model, msg) => …` to receive them"
            ));
        }
    }
    if session.global("update").is_some() {
        require_function(&path, &session, "update", 2)?;
    }
    // Optional physics: `physics(model) => Physics.scene(…)` declares the
    // bodies that should exist; the host reconciles + fixed-steps the
    // world after each tick (docs/physics.md). Rapier is pure Rust, so
    // the world runs in the browser exactly as it does natively.
    let has_physics = session.global("physics").is_some();
    if has_physics {
        require_function(&path, &session, "physics", 1)?;
    }
    // Optional soundscape: `soundScape(model)` returns an AudioScene (the
    // continuous, reconciled half of audio). No `update` requirement — the
    // scene is reconciled by the shell, not folded back as a message.
    let has_soundscape = session.global("soundScape").is_some();
    if has_soundscape {
        require_function(&path, &session, "soundScape", 1)?;
    }
    // Optional HUD: `ui(model)` returns a View (Ui.text / Ui.column /
    // Ui.panel), lowered to the shared text overlay — the F# `ui` hook.
    let has_ui = session.global("ui").is_some();
    if has_ui {
        require_function(&path, &session, "ui", 1)?;
    }
    Ok(Loaded {
        sources: source_map,
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

impl FunctorLangWebGame {
    /// Build the producer from the fetched project sources (entry FIRST, then
    /// siblings). Errors come back fully rendered for `run_async` to fail loud
    /// with (there is no keep-running fallback: a page load either gets a valid
    /// game or a console error).
    pub fn create(sources: Vec<(String, String)>) -> Result<FunctorLangWebGame, String> {
        // Route Functor Lang `Debug.log` traces to the browser console (once per process;
        // the process-global sink survives hot-reload). The web runtime has no
        // CLI event stream, so — unlike native, which forwards into the
        // region-aware log path — a trace goes straight to `console.log`, the
        // web equivalent of plain `functor-lang run`'s stdout.
        functor_lang::set_trace_sink(Box::new(|message| {
            web_sys::console::log_1(&JsValue::from_str(&message));
        }));
        // Route runtime events to the browser console too. Without a sink they
        // fall back to eprintln!, which goes nowhere in a browser — a failed
        // asset was completely invisible (it just rendered as the fallback).
        // AssetError is the load-bearing case: `Scene.model` on a missing or
        // bad URL now says so in the console, where a dev (or a headless test)
        // can see it.
        functor_runtime_common::events::set_sink(Box::new(|event| {
            use functor_runtime_common::events::RuntimeEvent as R;
            match event {
                R::AssetError { path, message } => {
                    let line = match path {
                        Some(path) => format!(
                            "[functor] asset '{path}' failed to load; using fallback: {message}"
                        ),
                        None => {
                            format!("[functor] asset failed to load; using fallback: {message}")
                        }
                    };
                    web_sys::console::error_1(&JsValue::from_str(&line));
                }
                R::HotReload { ok, message } => {
                    let line = format!("[functor] hot-reload: {message}");
                    if ok {
                        web_sys::console::log_1(&JsValue::from_str(&line));
                    } else {
                        web_sys::console::error_1(&JsValue::from_str(&line));
                    }
                }
                R::FunctorLangTrace { message } => {
                    web_sys::console::log_1(&JsValue::from_str(&message));
                }
                // CLI-stream concerns; quiet in the browser.
                R::Ready | R::FrameStats { .. } | R::CaptureWritten { .. } => {}
            }
        }));
        let path = sources
            .first()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| "game.fun".to_string());
        let loaded = load_source(&sources)?;
        web_sys::console::log_1(&format!("[functor-lang] loaded {path}").into());
        // Arm the paused-inspector journal on this (the only) wasm thread: from
        // now on every live model-updating call is journaled (a cheap Rc-clone
        // push). During play we NEVER arm the recorder or render Display — the
        // trace is built lazily only when paused (visual-debugger PR2b).
        journal_arm();
        let source_hashes = inspector_sources(&loaded.sources);
        Ok(FunctorLangWebGame {
            reporter: Reporter::new(SpanSource::Project(loaded.sources), report_to_console),
            overlay_error: None,
            last_frame_journal: Vec::new(),
            cached_trace: None,
            source_hashes,
            sources,
            path,
            module: loaded.module,
            session: loaded.session,
            model: loaded.init,
            has_input: loaded.has_input,
            has_mouse_move: loaded.has_mouse_move,
            has_mouse_wheel: loaded.has_mouse_wheel,
            has_subscriptions: loaded.has_subscriptions,
            prev_tts: None,
            effect_runner: RealEffects::new(),
            effect_log: EffectLog::new(),
            deferred_queries: Vec::new(),
            pending_events: Vec::new(),
            physics_rt: physics::SteppedPhysics::new(),
            physics_frame: 0,
            recorder: SceneRecorder::new(),
            input_buf: Vec::new(),
            live_conn_keys: std::collections::HashSet::new(),
            asset_progress: None,
            delivered_asset_progress: None,
            has_physics: loaded.has_physics,
            has_soundscape: loaded.has_soundscape,
            last_soundscape_json: empty_soundscape_json(),
            has_ui: loaded.has_ui,
            last_view: View::Empty,
            ui_handlers: Vec::new(),
            last_frame: empty_frame(),
        })
    }

    /// Swap in a freshly loaded program, KEEPING THE MODEL — the desktop
    /// producer's `swap_in`, verbatim. `init` from the new program is
    /// deliberately unused: state survives the edit, and closures stored in
    /// the model rebind to the edited code (B5 part 2, `functor_lang::rebind_value`).
    /// The physics world is deliberately KEPT too, like the model: it lives
    /// in this process's registry, so bodies stay where they are across the
    /// edit (removing the `physics` hook drops the world). `prev_tts` is kept
    /// as well: `Sub.every` fires on the global time grid, so timers tick
    /// right through a reload. Returns the number of stored closures rebound,
    /// for the status line.
    fn swap_in(&mut self, loaded: Loaded) -> usize {
        let live_model_was_safe = self.recorder.prepare_reload(
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
        );
        let (model, report) = functor_lang::rebind_value(&self.model, &self.module, &loaded.module);
        self.model = model;
        for warning in &report.warnings {
            web_sys::console::warn_1(&format!("[functor-lang] reload: {warning}").into());
        }
        // Recompute the inspector source hashes for the edited files, and drop
        // the journal + cached trace: they refer to the OLD program's spans and
        // execution (visual-debugger PR2b — reload clears both, like desktop).
        self.source_hashes = inspector_sources(&loaded.sources);
        self.last_frame_journal.clear();
        self.cached_trace = None;
        journal_swap(); // discard any partial current-frame journal
        self.reporter
            .set_source(SpanSource::Project(loaded.sources));
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
            // (the physics-world / `ui` rule).
            self.last_soundscape_json = empty_soundscape_json();
        }
        // A deferred query or in-flight HTTP request holds a tagger — a closure
        // into the OLD session; drop them rather than let them dangle. A
        // `playThen` completion message closes over the old session too.
        self.deferred_queries.clear();
        self.pending_events.clear();
        clear_http_taggers();
        clear_audio_completions();
        // The widget handler table holds msgs/taggers into the OLD session;
        // the next render's `ui(model)` rebuilds it against the new one.
        self.ui_handlers.clear();
        // Plain-data snapshots remain seekable under the new program. A model
        // history containing callable or opaque host values instead starts a new
        // generation anchored at this rebound live frame.
        self.recorder
            .finish_reload(&self.model, self.physics_frame, live_model_was_safe);
        self.has_ui = loaded.has_ui;
        if !self.has_ui {
            // Deleting the `ui` hook drops the HUD (the physics-world rule).
            self.last_view = View::Empty;
        }
        self.reporter.reset();
        // The push path (`functor_lang_set_source`) already hid the overlay in the DOM;
        // clear our shadow so the reloaded program's first draw re-shows it if
        // that program's `draw` still errors.
        self.overlay_error = None;
        report.rebound
    }

    /// Toggle the page's red draw-error overlay, touching the DOM only when the
    /// state actually changes (draw breaks with a new message, or recovers) so a
    /// persistent error doesn't rewrite the overlay every frame.
    fn set_draw_overlay(&mut self, error: Option<String>) {
        if self.overlay_error == error {
            return;
        }
        match &error {
            Some(message) => crate::show_error_overlay(message),
            None => crate::hide_error_overlay(),
        }
        self.overlay_error = error;
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
            physics_frame: &mut self.physics_frame,
            recorder: &mut self.recorder,
            effect_runner: &mut self.effect_runner as &mut dyn EffectRunner,
            effect_log: &mut self.effect_log,
            deferred_queries: &mut self.deferred_queries,
            pending_events: &mut self.pending_events,
            live_conn_keys: &mut self.live_conn_keys,
            prev_tts: &mut self.prev_tts,
            input_buf: &mut self.input_buf,
            has_physics: self.has_physics,
            has_subscriptions: self.has_subscriptions,
            asset_progress: self.asset_progress.clone(),
            delivered_asset_progress: &mut self.delivered_asset_progress,
            suppress_outbound: false,
            reporter: &mut self.reporter,
        }
    }
}

impl GameProducer for FunctorLangWebGame {
    // File-watch hot reload is native-only (docs/functor-lang.md C3) — there is no
    // filesystem here. The PUSH path below is the web's reload.
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn push_asset_progress(&mut self, progress: functor_runtime_common::asset::AssetProgress) {
        // Stored, not delivered here: the producer compares it against what
        // the game last saw during the frame's subscription phase.
        self.asset_progress = Some(progress);
    }

    fn reload_source(&mut self, source: &str) -> Result<String, String> {
        // The editor push path (docs/functor-lang.md D4), same semantics as the
        // desktop runner's `POST /reload-source`: model preserved, a broken
        // push keeps the old program (and the error goes back to the pusher,
        // who is looking at the source that caused it). No mtime bookkeeping
        // — the browser has no file watcher; pushes are the only reload.
        let started = js_sys::Date::now();
        // The push replaces the ENTRY buffer; siblings keep their last-fetched
        // text (the web has no filesystem to re-read, unlike the desktop
        // producer). A load failure leaves `self.sources` untouched.
        let mut sources = self.sources.clone();
        if let Some(entry) = sources.first_mut() {
            entry.1 = source.to_string();
        } else {
            sources.push((self.path.clone(), source.to_string()));
        }
        let loaded = load_source(&sources)?;
        self.sources = sources;
        let rebound = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        let status = format!(
            "reloaded {} from pushed source in {:.2}ms (model preserved{stored})",
            self.path,
            js_sys::Date::now() - started
        );
        web_sys::console::log_1(&format!("[functor-lang] {status}").into());
        Ok(status)
    }

    fn reload_project(&mut self, files: &[(String, String)]) -> Result<String, String> {
        // The multi-file push path (the web IDE): the pusher owns the WHOLE
        // file set, so — unlike `reload_source`, which swaps the entry and
        // keeps the last-fetched siblings — this replaces every module.
        // Entry first, then siblings; same keep-old-program-on-failure
        // semantics.
        if files.is_empty() {
            return Err("a pushed project needs at least the entry file".to_string());
        }
        let started = js_sys::Date::now();
        let loaded = load_source(files)?;
        self.sources = files.to_vec();
        self.path = files[0].0.clone();
        let rebound = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        let status = format!(
            "reloaded {} ({} file(s)) from pushed project in {:.2}ms (model preserved{stored})",
            self.path,
            files.len(),
            js_sys::Date::now() - started
        );
        web_sys::console::log_1(&format!("[functor-lang] {status}").into());
        Ok(status)
    }

    /// Coupled scene rewind — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T1), identical to the desktop producer.
    fn rewind_scene_to(&mut self, target: u64) -> Result<String, String> {
        let result = self.recorder.rewind_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
        );
        if result.is_ok() {
            self.deferred_queries.clear();
            self.pending_events.clear();
            // Model restored to `target`; drop orphaned buffered input so it can't
            // record into the branch (fixed-timestep 0-substep buffering, xreview).
            self.input_buf.clear();
            // The scrubbed frame is a historical one whose journal we didn't keep
            // — report it honestly as empty invocations (visual-debugger PR2b).
            self.last_frame_journal.clear();
            self.cached_trace = None;
        }
        result
    }

    fn seek_scene_to(&mut self, target: u64) -> Result<String, String> {
        let result = self.recorder.seek_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
        );
        if result.is_ok() {
            // Same as rewind: the model was restored, so buffered input since the
            // last recorded frame is orphaned and must not enter the branch.
            self.input_buf.clear();
            // The paused frame changed — clear the last-frame journal and cache
            // so the trace reflects the scrubbed frame (visual-debugger PR2b).
            self.last_frame_journal.clear();
            self.cached_trace = None;
        }
        result
    }

    fn current_scene_frame(&self) -> Option<u64> {
        self.recorder.current_scene_frame()
    }

    fn scene_frame_range(&self) -> Option<(u64, u64)> {
        self.recorder.scene_frame_range()
    }

    fn recorded_inputs_at(
        &self,
        rendered_frame: u64,
    ) -> Vec<functor_runtime_common::RecordedInput> {
        self.recorder.inputs_at(rendered_frame).to_vec()
    }

    fn scene_timeline_generation(&self) -> u64 {
        self.recorder.generation()
    }

    fn current_scene_tts(&self) -> Option<f64> {
        self.recorder.current_scene_frame_tts()
    }

    /// Forward-ghosting (docs/time-travel.md T6d) — delegated to the shared
    /// producer body (`functor_lang_producer::ghost_frames`), identical to the desktop
    /// producer. Makes the web producer's ghost half available; the web RENDER
    /// loop compositing them is a later slice.
    fn ghost_frames(
        &self,
        divisions: usize,
        dt: f32,
        start_tts: f64,
        script_inputs: Option<&[Vec<functor_runtime_common::RecordedInput>]>,
    ) -> Vec<(Frame, FrameTime)> {
        functor_runtime_common::functor_lang_producer::ghost_frames(
            &self.session,
            &self.model,
            &self.recorder,
            self.has_physics,
            self.has_subscriptions,
            self.prev_tts,
            &self.last_frame,
            divisions,
            dt,
            start_tts,
            script_inputs,
        )
    }

    fn tick(&mut self, frame_time: FrameTime) {
        // The whole MVU frame body lives in the shared `FrameCtx`
        // (docs/time-travel.md T6a). Web runs it as one call — unlike native it
        // has no per-frame perf timing to split it at the physics boundary.
        self.ctx().run_frame(frame_time);
        // A real frame ran: swap its journal into `last_frame_journal` (leaving
        // a fresh armed journal) and drop the cached trace (the frame advanced).
        // A paused frame never reaches here, so its last real frame is kept
        // (visual-debugger PR2b — mirrors the desktop producer).
        if let Some(journal) = journal_swap() {
            self.last_frame_journal = journal;
        }
        self.cached_trace = None;
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        // The optional `input` entry point: (model, key, isDown) => model.
        // Keys cross as the built-in `Key` module's variants (`Key.W`,
        // `Key.Up`, `Key.Num0`) — mirrors the desktop producer.
        if !self.has_input {
            return;
        }
        let Some(key_value) = functor_runtime_common::key_input_value(code) else {
            return; // unrecognized code / Key::Unknown — never delivered.
        };
        let args = vec![self.model.clone(), key_value, Value::Bool(is_down)];
        match self.session.call("input", args, &mut FunctorHost) {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error("input", &err),
        }
        // Buffer the raw event for the frame-indexed input log (T6b): flushed
        // into the recorder by `record_frame`, replayed by the forward-step.
        self.input_buf
            .push(functor_runtime_common::RecordedInput::Key { code, is_down });
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
        self.input_buf
            .push(functor_runtime_common::RecordedInput::MouseMove { x, y });
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
        self.input_buf
            .push(functor_runtime_common::RecordedInput::MouseWheel { delta });
    }

    fn ui_event(&mut self, event: functor_runtime_common::ui::UiEvent) {
        // No `ui` hook → no widgets to have interacted with; drop silently
        // (mirrors the has_input gates above).
        if !self.has_ui {
            return;
        }
        // The table is moved out for the call — `ctx()` borrows every other
        // producer field mutably — and restored after.
        let handlers = std::mem::take(&mut self.ui_handlers);
        self.ctx().deliver_ui_event(&handlers, &event);
        self.ui_handlers = handlers;
        // Buffer for the frame-indexed input log (T6b), like key events, so a
        // replay re-delivers the interaction.
        self.input_buf
            .push(functor_runtime_common::RecordedInput::UiEvent(event));
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
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
                Some(frame) => {
                    self.last_frame = frame.clone();
                    // A live draw clears the blank-screen overlay: the canvas is
                    // rendering again (a transient/first-frame error recovers).
                    self.set_draw_overlay(None);
                }
                None => {
                    let rendered = format!(
                        "[functor-lang] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    self.reporter.report_once(rendered.clone());
                    self.set_draw_overlay(Some(rendered));
                }
            },
            Err(err) => {
                let rendered = self.reporter.render_frame_error("draw", &err);
                self.reporter.report_once(rendered.clone());
                self.set_draw_overlay(Some(rendered));
            }
        }
        // The optional HUD, evaluated beside `draw` (same settled model) and
        // cached — `ui()` is a `&self` accessor, and errors need `&mut`
        // dedupe. A bad `ui` keeps the last good view (the last_frame rule).
        if self.has_ui {
            match self
                .session
                .call("ui", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(value) => match view_value(&value) {
                    Some(view) => {
                        self.last_view = view.clone();
                        // The evaluation registered this tree's widget handlers
                        // — adopt them in lockstep with the view they address.
                        self.ui_handlers = take_ui_handlers();
                    }
                    None => {
                        let _ = take_ui_handlers();
                        self.reporter.report_once(format!(
                            "[functor-lang] ui must return a View (Ui.text / Ui.column / Ui.panel), got {}",
                            value.kind_name()
                        ))
                    }
                },
                Err(err) => {
                    // A failed evaluation keeps the last good view AND its
                    // handlers; drop the partial table it registered.
                    let _ = take_ui_handlers();
                    self.reporter.frame_error("ui", &err)
                }
            }
        }
        // The optional soundscape, evaluated beside `draw` (same settled model)
        // and cached — `audio_scene_json` is a `&self` accessor, and errors
        // need `&mut` dedupe (the `ui` pattern, same as the desktop producer).
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
                        "[functor-lang] soundScape must return an AudioScene (AudioScene.create / \
AudioScene.empty), got {}",
                        value.kind_name()
                    )),
                },
                Err(err) => self.reporter.frame_error("soundScape", &err),
            }
        }
        // On failure this is the last good frame — a bad draw must not blank
        // the canvas.
        self.last_frame.clone()
    }

    fn ui(&self) -> View {
        self.last_view.clone()
    }

    fn state_debug(&self) -> String {
        self.model.to_string()
    }

    /// The paused-inspector trace (visual-debugger PR2b), same contract and
    /// caching as the desktop producer: the byte-stable stub while playing
    /// (no `frame`/`tts`), and a lazily built + cached full doc while paused —
    /// built only once per pause / paused-frame change (the cache is dropped on
    /// tick / rewind / seek / reload). Published to the page's poll loop via
    /// [`publish_inspector_trace`] each frame; the page relays a CHANGE to the
    /// VS Code live-preview as a `functor-inspector-trace` postMessage.
    fn inspector_trace(&mut self, paused: bool) -> String {
        if !paused {
            return build_trace_doc(false, 0, 0.0, &self.source_hashes, &[], None, &self.session);
        }
        if let Some(cached) = &self.cached_trace {
            return cached.clone();
        }
        let frame = self.recorder.current_scene_frame().unwrap_or(0);
        let tts = self.recorder.current_scene_frame_tts().unwrap_or(0.0);
        // Draw is pure and never journaled; the builder replays it once
        // against the frozen model so the render pass is inspectable too.
        let draw_args = vec![self.model.clone(), Value::Number(tts)];
        let json = build_trace_doc(
            true,
            frame,
            tts,
            &self.source_hashes,
            &self.last_frame_journal,
            Some(&draw_args),
            &self.session,
        );
        self.cached_trace = Some(json.clone());
        json
    }

    fn net_drain_commands(&self) -> String {
        // HttpRequest commands (Effect.httpGet/httpPost); the page's fetch host
        // performs them and returns the response via net_push_http_*.
        functor_runtime_common::net::drain_commands_json()
    }
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String) {
        self.ctx()
            .deliver_http_result(functor_runtime_common::net::HttpResult {
                token: token as u64,
                status: status as u16,
                body: body.into_bytes(),
                error: None,
            });
    }
    fn net_push_http_error(&mut self, token: i32, message: String) {
        self.ctx()
            .deliver_http_result(functor_runtime_common::net::HttpResult {
                token: token as u64,
                status: 0,
                body: Vec::new(),
                error: Some(message),
            });
    }
    fn audio_drain_commands(&self) -> String {
        // One-shot commands (Effect.play/playAt/playThen); the page's Web Audio
        // host plays them. playThen finishes are not reported on wasm yet — see
        // `deliver_audio_completion`.
        functor_runtime_common::audio::drain_commands_json()
    }
    fn audio_scene_json(&self) -> String {
        // The continuous soundscape, evaluated + cached in `render` (the `ui`
        // pattern) so this stays a cheap `&self` read.
        self.last_soundscape_json.clone()
    }
    fn net_drain_conn_commands(&self) -> String {
        functor_runtime_common::net::drain_conn_commands_json()
    }
    fn net_push_connected(&mut self, key: String, conn: i32) {
        self.ctx()
            .deliver_net_event(key, NetEventKind::Connected, conn, String::new());
    }
    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String) {
        self.ctx()
            .deliver_net_event(key, NetEventKind::Message, conn, text);
    }
    fn net_push_disconnected(&mut self, key: String, conn: i32) {
        self.ctx()
            .deliver_net_event(key, NetEventKind::Disconnected, conn, String::new());
    }
    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String) {
        self.ctx()
            .deliver_net_event(key, NetEventKind::Error, conn, message);
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

/// The web `Reporter` sink: per-frame problems go to the browser console.
fn report_to_console(message: &str) {
    web_sys::console::error_1(&message.into());
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

// --- Page → producer input bridge. ------------------------------------------
//
// The Functor Lang game lives *inside* this runtime, so the Functor Lang index page
// (index-functor-lang.html) calls the `functor_lang_*` exports below. Events queue here and the
// frame loop drains them into the producer before each tick.

// The page-input queue carries the SAME plain-data shape the recorder logs, so
// reuse the shared `RecordedInput` (T6b) rather than a parallel private enum —
// `drain_input` dispatches its variants unchanged.
use functor_runtime_common::RecordedInput as InputEvent;

thread_local! {
    static INPUT_QUEUE: RefCell<Vec<InputEvent>> = const { RefCell::new(Vec::new()) };
}

/// Far more events than one frame can produce; if the frame loop never starts
/// (a failed game load leaves the page's handlers wired but nothing
/// draining), the queue must not grow forever.
const INPUT_QUEUE_CAP: usize = 1024;

fn push_input(event: InputEvent) {
    INPUT_QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() < INPUT_QUEUE_CAP {
            q.push(event);
        }
    });
}

/// Deliver a keyboard event (`code` = `functor_runtime_common::Key` as i32).
#[wasm_bindgen]
pub fn functor_lang_key_event(code: i32, is_down: bool) {
    push_input(InputEvent::Key { code, is_down });
}

/// Deliver a mouse position in window pixels (the page accumulates pointer-lock
/// movement deltas, matching the desktop's absolute cursor position).
#[wasm_bindgen]
pub fn functor_lang_mouse_move(x: i32, y: i32) {
    push_input(InputEvent::MouseMove { x, y });
}

/// Deliver a mouse-wheel event (vertical scroll offset, ±1 per notch).
#[wasm_bindgen]
pub fn functor_lang_mouse_wheel(delta: i32) {
    push_input(InputEvent::MouseWheel { delta });
}

thread_local! {
    /// The page's UNLOCKED pointer over the canvas — `(pos in CSS px,
    /// primary button down, press latched since last sample)` — for the
    /// interactive game-UI overlay (docs/ui-interaction.md U3). Separate from
    /// the pointer-lock mouse-look path above: while locked there is no
    /// cursor to point at widgets with (`pos` is `None`). Level state plus a
    /// press LATCH: a mousedown+mouseup landing between two rAF frames would
    /// otherwise sample as never-pressed and the click would be lost — the
    /// latch keeps the sampled level down for one frame so egui sees the
    /// press edge (the release follows next frame).
    static UI_POINTER: std::cell::Cell<(Option<(f32, f32)>, bool, bool)> =
        const { std::cell::Cell::new((None, false, false)) };
}

/// Deliver the unlocked pointer's canvas position (CSS px, e.g. `offsetX/Y`)
/// and primary-button state. Called by the page's mousemove/mousedown/mouseup
/// handlers while pointer lock is NOT engaged.
#[wasm_bindgen]
pub fn functor_lang_ui_pointer(x: f32, y: f32, primary_down: bool) {
    UI_POINTER.with(|p| {
        let (_, was_down, clicked) = p.get();
        // Latch the press EDGE (not the held level) so a sub-frame click
        // survives to the next sample without pinning held state forever.
        p.set((
            Some((x, y)),
            primary_down,
            clicked || (primary_down && !was_down),
        ));
    });
}

/// The pointer left the canvas (or pointer lock engaged). The page clears its
/// own button state on leave, so mirror it — a press begun off-canvas must
/// not replay as a click on re-entry (the bridge's swallow rule; a press held
/// across the leave is released by the bridge at its last position).
#[wasm_bindgen]
pub fn functor_lang_ui_pointer_leave() {
    UI_POINTER.with(|p| {
        let (_, _, clicked) = p.get();
        p.set((None, false, clicked));
    });
}

/// This frame's pointer for the overlay pass, scaled from the page's CSS px
/// to framebuffer px (`dpr` — the overlay runs at the device pixel ratio).
/// Consumes the press latch: a latched sub-frame click samples as down once.
pub fn ui_pointer_state(dpr: f32) -> functor_runtime_common::ui::PointerState {
    UI_POINTER.with(|p| {
        let (pos, primary_down, clicked) = p.get();
        p.set((pos, primary_down, false));
        functor_runtime_common::ui::PointerState {
            pos: pos.map(|(x, y)| (x * dpr, y * dpr)),
            primary_down: primary_down || clicked,
        }
    })
}

thread_local! {
    /// Keyboard events queued for a focused `Ui.textInput`
    /// (docs/ui-interaction.md U4). The page routes keydowns here (instead
    /// of the game key path) while [`functor_lang_ui_wants_keyboard`] reports
    /// true; the frame loop drains it into the overlay pass. Same cap
    /// rationale as [`INPUT_QUEUE`].
    static UI_KEY_QUEUE: RefCell<Vec<functor_runtime_common::ui::UiKeyboardEvent>> =
        const { RefCell::new(Vec::new()) };
    /// Whether the overlay wanted the keyboard after the LAST frame's pass —
    /// what the page's keydown handler polls to pick a route.
    static UI_WANTS_KEYBOARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Deliver a keydown for a focused text field. `key` is the DOM
/// `KeyboardEvent.key`: a single-char string is printable text; the named
/// editing keys map across; anything else is dropped (F-keys, media keys).
/// Returns whether the key was CONSUMED — the page only `preventDefault()`s
/// then, so browser chrome (F5, DevTools) keeps working while typing.
#[wasm_bindgen]
pub fn functor_lang_ui_key(key: &str) -> bool {
    use functor_runtime_common::ui::{UiEditKey, UiKeyboardEvent};
    let mut chars = key.chars();
    let event = match (chars.next(), chars.next()) {
        // Exactly one char → printable text ("a", "3", "`", …).
        (Some(c), None) => Some(UiKeyboardEvent::Char(c)),
        _ => match key {
            "Backspace" => Some(UiKeyboardEvent::Edit(UiEditKey::Backspace)),
            "Delete" => Some(UiKeyboardEvent::Edit(UiEditKey::Delete)),
            "ArrowLeft" => Some(UiKeyboardEvent::Edit(UiEditKey::Left)),
            "ArrowRight" => Some(UiKeyboardEvent::Edit(UiEditKey::Right)),
            "Home" => Some(UiKeyboardEvent::Edit(UiEditKey::Home)),
            "End" => Some(UiKeyboardEvent::Edit(UiEditKey::End)),
            "Enter" => Some(UiKeyboardEvent::Edit(UiEditKey::Enter)),
            "Escape" => Some(UiKeyboardEvent::Edit(UiEditKey::Escape)),
            _ => None,
        },
    };
    match event {
        Some(event) => {
            UI_KEY_QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                if q.len() < INPUT_QUEUE_CAP {
                    q.push(event);
                }
            });
            true
        }
        None => false,
    }
}

/// Whether a `Ui.textInput` is focused — the page's keydown handler routes
/// keys to [`functor_lang_ui_key`] while this is true, and to the game's
/// key path otherwise (the focus gate, docs/ui-interaction.md U4).
#[wasm_bindgen]
pub fn functor_lang_ui_wants_keyboard() -> bool {
    UI_WANTS_KEYBOARD.with(|w| w.get())
}

/// Drain the focused-field key queue (the frame loop, before the overlay
/// pass). `deliver: false` (pinned clock) discards — typing is inert while
/// pinned, like all other window input.
pub fn drain_ui_keys(deliver: bool) -> Vec<functor_runtime_common::ui::UiKeyboardEvent> {
    let events = UI_KEY_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if deliver {
        events
    } else {
        Vec::new()
    }
}

/// Publish this frame's `wants_keyboard` for the page's keydown routing.
pub fn set_ui_wants_keyboard(wants: bool) {
    UI_WANTS_KEYBOARD.with(|w| w.set(wants));
}

/// Drain the queued page input into the producer, in arrival order. Called by
/// the frame loop before `tick`. Empty (and free) on the F# path — its page
/// never calls the `functor_lang_*` exports.
///
/// When `deliver` is false (the clock is paused), the queue is still drained but
/// its events are DISCARDED — never dispatched to the game. This mirrors the
/// desktop pinned-clock gate: while paused, NO input may reach the model, so the
/// frame-indexed input log has nothing unlogged to diverge forward-step replay.
/// Draining (rather than leaving the queue) also stops it bursting on resume.
pub fn drain_input(game: &mut dyn GameProducer, deliver: bool) {
    let events = INPUT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if !deliver {
        return;
    }
    for event in events {
        match event {
            InputEvent::Key { code, is_down } => game.key_event(code, is_down),
            InputEvent::MouseMove { x, y } => game.mouse_move(x, y),
            InputEvent::MouseWheel { delta } => game.mouse_wheel(delta),
            InputEvent::UiEvent(event) => game.ui_event(event),
        }
    }
}

// --- Time-travel scrubber ↔ DOM bridge (docs/time-travel.md T3) -------------
//
// On web the scrubber is NATIVE DOM (index-functor-lang.html), not egui-in-canvas, so
// its widgets sit OUTSIDE the game canvas — their clicks never reach the canvas
// (no pointer-lock clash) and they render as accessible browser controls. The
// page calls the `functor_lang_scrub_*` write exports (queued here, applied by the frame
// loop, which owns the clock) and polls the read exports each frame; the loop
// publishes the current view state. The coupled-rewind LOGIC stays shared
// (`SceneRecorder`); only the UI surface differs from desktop.

/// A control from the DOM scrubber, applied by the frame loop.
pub enum ScrubControl {
    TogglePause,
    Step,
    SeekTo {
        frame: u64,
        request_id: u32,
    },
    /// Future-preview mode (docs/time-travel.md T6/T6d), pushed by the DOM
    /// preview `<select>` (PreviewMode wire index: 0 off / 1 trail / 2 strobe /
    /// 3 both / 4 ghost). The frame loop owns the preview state.
    SetPreview(u32),
    /// The ⚙ popover's shared forward window (seconds) + samples-per-second
    /// rate, pushed by the DOM inputs on change.
    SetPreviewConfig {
        window: f32,
        rate: usize,
    },
}

thread_local! {
    static SCRUB_CONTROLS: RefCell<Vec<ScrubControl>> = const { RefCell::new(Vec::new()) };
    /// Published each frame for the page's slider:
    /// `(frame, lo, hi, paused, history generation)`.
    /// `frame`/`lo`/`hi` are `-1.0` when nothing is recorded yet.
    static SCRUB_VIEW: RefCell<(f64, f64, f64, bool, u64)> =
        const { RefCell::new((-1.0, -1.0, -1.0, false, 0)) };
    /// Latest completed seek as `(request id, authoritative applied frame)`.
    /// The DOM uses this acknowledgement to retire optimistic handle state even
    /// when the runtime clamps or refuses a request.
    static SCRUB_SEEK_RESULT: RefCell<Option<(u32, f64)>> = const { RefCell::new(None) };
}

const SCRUB_CONTROLS_CAP: usize = 256;

#[derive(Clone)]
struct TimelineMarker {
    id: u64,
    frame: u64,
    kind: &'static str,
    label: String,
}

#[derive(Default)]
struct TimelineLog {
    markers: Vec<TimelineMarker>,
    next_id: u64,
    input_cursor: Option<u64>,
    input_generation: Option<u64>,
    cached_json: String,
    dirty: bool,
    revision: u32,
}

impl TimelineLog {
    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    fn push(&mut self, frame: u64, kind: &'static str, label: String) {
        self.markers.push(TimelineMarker {
            id: self.next_id,
            frame,
            kind,
            label,
        });
        self.next_id += 1;
        const CAP: usize = 4096;
        if self.markers.len() > CAP {
            self.markers.drain(..self.markers.len() - CAP);
        }
        self.changed();
    }

    fn retain_range(&mut self, lo: u64, hi: u64) {
        let old_len = self.markers.len();
        self.markers
            .retain(|marker| marker.frame >= lo && marker.frame <= hi);
        if self.markers.len() != old_len {
            self.changed();
        }
    }

    fn truncate_from(&mut self, frame: u64) {
        let old_len = self.markers.len();
        self.markers.retain(|marker| marker.frame < frame);
        if self.markers.len() != old_len {
            self.changed();
        }
    }

    fn reset_inputs(&mut self) {
        let old_len = self.markers.len();
        self.markers
            .retain(|marker| marker.kind.starts_with("reload-"));
        if self.markers.len() != old_len {
            self.changed();
        }
        self.input_cursor = None;
    }

    fn json(&mut self) -> &str {
        if self.dirty || self.cached_json.is_empty() {
            let markers: Vec<_> = self
                .markers
                .iter()
                .map(|marker| {
                    serde_json::json!({
                        "id": marker.id,
                        "frame": marker.frame,
                        "kind": marker.kind,
                        "label": marker.label,
                    })
                })
                .collect();
            self.cached_json = serde_json::to_string(&markers).unwrap_or_else(|_| "[]".to_string());
            self.dirty = false;
        }
        &self.cached_json
    }
}

thread_local! {
    static TIMELINE_LOG: RefCell<TimelineLog> = RefCell::new(TimelineLog::default());
}

fn input_marker(input: &InputEvent) -> (&'static str, String) {
    match input {
        InputEvent::Key { code, is_down } => {
            let name = functor_runtime_common::Key::from_i32(*code)
                .map(|key| key.name())
                .unwrap_or_else(|| format!("key {code}"));
            let edge = if *is_down { "down" } else { "up" };
            (
                if *is_down { "key-down" } else { "key-up" },
                format!("{name} {edge}"),
            )
        }
        InputEvent::MouseMove { x, y } => ("mouse-move", format!("mouse move ({x}, {y})")),
        InputEvent::MouseWheel { delta } => ("mouse-wheel", format!("mouse wheel {delta:+}")),
        InputEvent::UiEvent(event) => ("ui-input", format!("UI {event:?}")),
    }
}

/// Copy newly recorded inputs into the DOM timeline's compact marker log. The
/// producer's recorder is authoritative: inputs discarded while paused never
/// appear, and replayable inputs land on their exact rendered frame.
pub fn publish_timeline_inputs(game: &dyn GameProducer) {
    let Some((lo, hi)) = game.scene_frame_range() else {
        return;
    };
    TIMELINE_LOG.with(|log| {
        let mut log = log.borrow_mut();
        let generation = game.scene_timeline_generation();
        if log.input_generation != Some(generation) {
            // A branch can replace frames without making `hi` move backward
            // (for example, one frame from the old tail). Rebuild input markers
            // from the authoritative recorder instead of inferring from range.
            log.reset_inputs();
            log.input_generation = Some(generation);
        }
        if log.input_cursor.is_some_and(|cursor| cursor > hi) {
            // A resumed scrub rewrote `hi`, the first frame on the new branch.
            // Drop the discarded branch including its stale marker at `hi`,
            // then rescan that authoritative replacement frame below.
            log.truncate_from(hi);
            log.input_cursor = hi.checked_sub(1);
        }
        let start = log
            .input_cursor
            .map_or(lo, |cursor| cursor.saturating_add(1).max(lo));
        if start <= hi {
            for frame in start..=hi {
                let mut last_mouse_move = None;
                for input in game.recorded_inputs_at(frame) {
                    if matches!(&input, InputEvent::MouseMove { .. }) {
                        // Pointer-lock mouselook can emit several moves per
                        // rendered frame. One marker still says "input here"
                        // without multiplying bridge payload and DOM hits.
                        last_mouse_move = Some(input);
                        continue;
                    }
                    let (kind, label) = input_marker(&input);
                    log.push(frame, kind, label);
                }
                if let Some(input) = last_mouse_move {
                    let (kind, label) = input_marker(&input);
                    log.push(frame, kind, label);
                }
            }
        }
        log.input_cursor = Some(hi);
        log.retain_range(lo, hi);
    });
}

/// Record a reload boundary at the scene frame that remained current through
/// the swap. Failures mark the attempted boundary without changing the program.
pub fn publish_timeline_reload(frame: u64, ok: bool, message: &str) {
    TIMELINE_LOG.with(|log| {
        log.borrow_mut().push(
            frame,
            if ok { "reload-ok" } else { "reload-error" },
            if ok {
                "hot reload".to_string()
            } else {
                format!("reload failed: {message}")
            },
        );
    });
}

/// Runtime → page: JSON array of `{id, frame, kind, label}` markers.
#[wasm_bindgen]
pub fn functor_lang_timeline_events() -> String {
    TIMELINE_LOG.with(|log| log.borrow_mut().json().to_string())
}

/// Runtime → page: cheap marker-log revision. The DOM fetches the JSON only
/// when this changes instead of cloning it over the WASM boundary every rAF.
#[wasm_bindgen]
pub fn functor_lang_timeline_events_gen() -> u32 {
    TIMELINE_LOG.with(|log| log.borrow().revision)
}

fn push_scrub(control: ScrubControl) {
    SCRUB_CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() < SCRUB_CONTROLS_CAP {
            c.push(control);
        }
    });
}

/// Drain the queued scrubber controls; the frame loop applies them (it owns the
/// clock pin and the game).
pub fn take_scrub_controls() -> Vec<ScrubControl> {
    SCRUB_CONTROLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Publish this frame's scrubber state for the page to poll.
pub fn publish_scrub_view(
    frame: Option<u64>,
    range: Option<(u64, u64)>,
    paused: bool,
    generation: u64,
) {
    let f = frame.map(|f| f as f64).unwrap_or(-1.0);
    let (lo, hi) = range
        .map(|(l, h)| (l as f64, h as f64))
        .unwrap_or((-1.0, -1.0));
    SCRUB_VIEW.with(|v| *v.borrow_mut() = (f, lo, hi, paused, generation));
}

/// Page → runtime: toggle pause (pin/unpin the clock).
#[wasm_bindgen]
pub fn functor_lang_scrub_toggle_pause() {
    push_scrub(ScrubControl::TogglePause);
}

/// Page → runtime: advance exactly one frame, then hold.
#[wasm_bindgen]
pub fn functor_lang_scrub_step() {
    push_scrub(ScrubControl::Step);
}

/// Page → runtime: set the future-preview mode (the DOM preview `<select>`;
/// 0 off / 1 trail / 2 strobe / 3 both / 4 ghost — `PreviewMode::from_index`).
#[wasm_bindgen]
pub fn functor_lang_scrub_set_preview(mode: u32) {
    push_scrub(ScrubControl::SetPreview(mode));
}

/// Page → runtime: set the preview's shared forward window (seconds) and
/// samples-per-second rate (the ⚙ popover; JS owns the inputs and pushes on
/// change).
#[wasm_bindgen]
pub fn functor_lang_scrub_set_preview_config(window: f32, rate: usize) {
    push_scrub(ScrubControl::SetPreviewConfig { window, rate });
}

/// Page → runtime: non-destructively scrub to a rendered frame (slider drag).
#[wasm_bindgen]
pub fn functor_lang_seek_scene(frame: f64, request_id: u32) {
    if frame >= 0.0 {
        push_scrub(ScrubControl::SeekTo {
            frame: frame as u64,
            request_id,
        });
    }
}

/// Publish a completed seek's authoritative frame for the DOM's optimistic
/// state reconciler. Kept separate from [`SCRUB_VIEW`] so ordinary playback
/// publications do not masquerade as seek acknowledgements.
pub fn publish_scrub_seek_result(request_id: u32, frame: Option<u64>) {
    SCRUB_SEEK_RESULT.with(|result| {
        *result.borrow_mut() = Some((request_id, frame.map_or(-1.0, |frame| frame as f64)));
    });
}

/// Runtime → page: latest `[requestId, appliedFrame]`, or `[]` before any seek.
#[wasm_bindgen]
pub fn functor_lang_scrub_seek_result() -> Vec<f64> {
    SCRUB_SEEK_RESULT.with(|result| {
        result
            .borrow()
            .map(|(request_id, frame)| vec![request_id as f64, frame])
            .unwrap_or_default()
    })
}

/// Runtime → page: the current handle frame (`-1` if nothing recorded).
#[wasm_bindgen]
pub fn functor_lang_scene_frame() -> f64 {
    SCRUB_VIEW.with(|v| v.borrow().0)
}

/// Runtime → page: the seekable window as `[lo, hi]`, or `[]` if empty.
#[wasm_bindgen]
pub fn functor_lang_scene_range() -> Vec<f64> {
    let (_, lo, hi, _, _) = SCRUB_VIEW.with(|v| *v.borrow());
    if lo < 0.0 {
        vec![]
    } else {
        vec![lo, hi]
    }
}

/// Runtime → page: whether the clock is currently pinned.
#[wasm_bindgen]
pub fn functor_lang_scrub_paused() -> bool {
    SCRUB_VIEW.with(|v| v.borrow().3)
}

/// Runtime → page: current seekable-history generation.
#[wasm_bindgen]
pub fn functor_lang_scene_generation() -> f64 {
    SCRUB_VIEW.with(|v| v.borrow().4 as f64)
}

// --- Paused-scene inspector ↔ DOM bridge (visual-debugger PR2b) --------------
//
// The desktop shell serves the inspector trace over `GET /trace`; the web shell
// has no debug HTTP server, so it uses the SAME poll pattern as the scrubber
// above. Each frame the loop publishes the current trace doc via
// [`publish_inspector_trace`]; a GENERATION counter bumps only when the doc
// CONTENT changes — which, given the producer's caching, happens only on a
// pause-state change or a paused-frame change (step/seek), never generally
// during play. The page polls the counter and, on a change, reads the doc and
// relays it to the VS Code live-preview as a `functor-inspector-trace`
// postMessage (which the extension already forwards to the LSP).

thread_local! {
    /// `(generation, doc json)` — published each frame by the loop, read by the
    /// page's poll exports. The generation increments ONLY when the doc bytes
    /// change, so the page posts a trace on pause / paused-frame change, not
    /// every frame.
    static INSPECTOR_TRACE: RefCell<(u32, String)> = const { RefCell::new((0, String::new())) };
}

/// Publish this frame's inspector trace for the page to poll. Cheap: the doc is
/// the producer's cached string while paused (rebuilt only on a pause/frame
/// change) and the byte-stable stub while playing, so the equality check here
/// is a plain string compare — the generation bumps only on a real change.
pub fn publish_inspector_trace(doc: String) {
    INSPECTOR_TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if t.1 != doc {
            t.0 = t.0.wrapping_add(1);
            t.1 = doc;
        }
    });
}

/// Runtime → page: the inspector-trace generation. The page polls this each
/// frame and reads [`functor_lang_inspector_trace`] only when it changes.
#[wasm_bindgen]
pub fn functor_lang_inspector_trace_gen() -> u32 {
    INSPECTOR_TRACE.with(|t| t.borrow().0)
}

/// Runtime → page: the current inspector-trace wire JSON (the paused full doc,
/// or the byte-stable playing stub).
#[wasm_bindgen]
pub fn functor_lang_inspector_trace() -> String {
    INSPECTOR_TRACE.with(|t| t.borrow().1.clone())
}
