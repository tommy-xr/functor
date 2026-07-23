//! The EMBEDDED Functor Lang producer: the portable, in-memory sibling of the desktop
//! runner's `functor_lang_game.rs` and the web shell's `functor_lang_game.rs`, behind the
//! same `GameProducer` seam. Same load-time contract validation and per-frame
//! semantics — the MVU pair (subscriptions fold through `update` before
//! `tick`), the optional `physics` hook (tick → physics → draw), a bad frame
//! keeps the last good model/frame, per-frame errors dedupe — but with **no
//! shell assumptions at all**:
//!
//! - the `.fun` source arrives as strings (an embedded boot scene, a network
//!   push) — no filesystem, no fetch;
//! - no file-watch hot reload; the PUSH path (`reload_source`/`reload_project`)
//!   is the only reload, mirroring the desktop runner's `POST /reload-source`:
//!   parse → lower → check-as-warnings → `Session::load` →
//!   `functor_lang::rebind_value` on the held model;
//! - diagnostics go through the `log` facade (the shell owns the logger — on
//!   Quest that's android_logger; in a host test it's whatever the test
//!   installs), so the producer itself is target-agnostic: it compiles and
//!   runs on native, Android, and wasm alike.
//!
//! First consumer: the Quest shell (`functor-runtime-oculus`), whose tool APK
//! boots an embedded scene and then receives games over the network. The web
//! producer is this file's ancestor and can converge onto it later.

use functor_lang::project::SourceMap;
use functor_lang::{Session, Value};

use crate::functor_lang_prelude::{
    audio_scene_of, clear_audio_completions, clear_http_taggers, clear_preload_completions,
    contains_effect, frame_value, html_node_value, now_ms, take_ui_handlers, view_value, EffectLog,
    EffectRunner, EffectTree, FunctorHost, NetEventKind, RealEffects, UiHandler,
};
use crate::functor_lang_producer::{
    journal_arm, journal_swap, FrameCtx, JournalEntry, Reporter, SpanSource,
};
use crate::inspector::{build_trace_doc, inspector_sources, InspectorSource};
use crate::physics;
use crate::protocol::GameProducer;
use crate::timetravel::SceneRecorder;
use crate::ui::View;
use crate::webview::HtmlNode;
use crate::{Frame, FrameTime};

fn replay_status(history_replay: Option<(usize, f64)>) -> String {
    history_replay.map_or_else(String::new, |(frames, elapsed_ms)| {
        format!("; history recomputed from init ({frames} frames, {elapsed_ms:.2}ms)")
    })
}

/// The platform seam between the shared producer and its shell — the only place
/// the two shells genuinely differ (everything else is one shared body). A
/// native shell (Quest/host tests) installs the `log`-crate sink and has no
/// draw-error overlay; the web shell installs a console/event-sink bridge and
/// drives a DOM overlay. Passed to [`FunctorLangEmbeddedGame::create`] and held
/// for the producer's lifetime.
pub trait ProducerPlatform {
    /// One-time, process-global logging/trace/event sink setup. Run at the top
    /// of `create` before the first load so load errors surface.
    fn install_sinks(&self);
    /// Show (`Some(message)`) or hide (`None`) a draw-error overlay, deduped by
    /// the impl so a persistent error doesn't rewrite it every frame. Native:
    /// no-op (the native shells have no such overlay).
    fn set_draw_overlay(&mut self, error: Option<&str>);
    /// Called at the end of a successful reload (`swap_in`). The shell may have
    /// hidden the overlay out-of-band (the web push path hides it in JS), so
    /// this resets any dedupe shadow, letting the reloaded program's first draw
    /// re-show the overlay if it still errors. Native: no-op.
    fn on_reload(&mut self);
}

/// The native platform (Quest shell, host tests): routes diagnostics through
/// the `log` facade (whose backend the shell owns) and has no draw overlay.
pub struct NativePlatform;

impl ProducerPlatform for NativePlatform {
    fn install_sinks(&self) {
        // Route Functor Lang `Debug.log` traces through the runtime event stream
        // (whose sink the shell owns) — the desktop producer's rule.
        crate::functor_lang_prelude::install_debug_log_sink();
    }
    fn set_draw_overlay(&mut self, _error: Option<&str>) {}
    fn on_reload(&mut self) {}
}

pub struct FunctorLangEmbeddedGame {
    path: String,
    /// The project's source files (entry FIRST, then siblings) as
    /// `(path, source)` — the in-memory stand-in for the on-disk directory the
    /// desktop producer re-reads on reload. A push (`reload_source`) replaces
    /// only the ENTRY buffer; siblings keep their last-pushed text.
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
    /// (nothing fires on frame one — mirroring the other producers).
    prev_tts: Option<f64>,
    /// The shell's latest asset-loading snapshot (pushed each frame by the
    /// render loop) and the one the game last saw — the `Sub.assets` seam.
    asset_progress: Option<crate::asset::AssetProgress>,
    delivered_asset_progress: Option<crate::asset::AssetProgress>,
    has_physics: bool,
    /// The game defines the optional `soundScape` entry point
    /// (`soundScape(model) -> AudioScene`, the continuous-audio hook). Absent =
    /// silence; unlike `subscriptions` it needs no `update`.
    has_soundscape: bool,
    /// The last serialized soundscape (`soundScape model` → JSON), cached
    /// because `audio_scene_json` is a `&self` accessor — evaluated + deduped
    /// in `render` (the `ui` pattern, same as the other producers).
    last_soundscape_json: String,
    /// The game defines the optional `ui` entry point (`ui(model) -> View`,
    /// the 2D HUD hook).
    has_ui: bool,
    /// The last successfully built HUD View, cached because `ui()` is a
    /// `&self` accessor — a bad `ui` keeps the last good view.
    last_view: View,
    /// The interactive-widget handler table registered by the `ui(model)`
    /// evaluation that built `last_view` (docs/ui-interaction.md U2), kept in
    /// lockstep with it.
    ui_handlers: Vec<UiHandler>,
    /// The game defines the optional `webview` entry point
    /// (`webview(model) -> Html.node`, the HTML/CSS overlay hook).
    has_webview: bool,
    /// The last successfully built webview tree, cached like `last_view`.
    last_webview: Option<HtmlNode>,
    /// The handler table for `last_webview` — the webview's own slot space,
    /// separate from `ui_handlers`. Same lockstep/reload rules.
    webview_handlers: Vec<UiHandler>,
    /// Performs `Effect.*` commands (B6). `RealEffects` is portable: its
    /// clock has a per-target implementation.
    effect_runner: RealEffects,
    /// The structured effect log (bounded inside the drain).
    effect_log: EffectLog,
    /// Physics queries deferred by the frame's pre-step drains, performed
    /// right after the physics step so their taggers answer against the
    /// fresh world ("commands apply at the step; queries answer after it").
    deferred_queries: Vec<EffectTree>,
    /// This frame's contact transitions, delivered post-step to the
    /// `Physics.events` taggers of the current `subscriptions(model)`.
    pending_events: Vec<crate::physics::PhysicsEvent>,
    /// The recorded physics drive (docs/physics.md Phase 6): the Timeline
    /// recorder + fixed-step accumulator. The World stays in the registry;
    /// this owns the rewind machinery over it.
    physics_rt: physics::SteppedPhysics,
    /// The physics world's fixed frame after the latest advance — what the
    /// coupled scene recorder stores per rendered frame.
    physics_frame: u64,
    /// The coupled time-travel recorder (docs/time-travel.md T1–T3), shared
    /// with the other producers (one tested impl).
    recorder: SceneRecorder,
    /// This frame's buffered input events (docs/time-travel.md T6b): appended
    /// beside the live `session.call`, flushed into `recorder`'s input log by
    /// `record_frame` (plain data, so the log survives a reload).
    input_buf: Vec<crate::RecordedInput>,
    /// Declared connection keys (`Sub.connect`/`Sub.listen`), reconciled each
    /// frame — see the desktop producer.
    live_conn_keys: std::collections::HashSet<String>,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// Per-frame error reporting (dedupe + `log` sink + single-source span
    /// rendering) — shared with the other producers
    /// (`functor_lang_producer::Reporter`).
    reporter: Reporter,
    /// The last real frame's replay journal (visual-debugger PR2b): one entry
    /// per model-updating call, swapped in from the thread-local journal at
    /// the end of each `tick`. Replayed through `Session::call_recorded`
    /// while paused.
    last_frame_journal: Vec<JournalEntry>,
    /// A window of recent frames' journals `(frame, entries)` — the recency
    /// gutter's coverage source. Survives rewind/seek; cleared on hot-reload
    /// (old program's spans).
    journal_ring: std::collections::VecDeque<(u64, Vec<JournalEntry>)>,
    /// The static could-run set, recomputed on load/reload.
    runnable: Vec<usize>,
    /// The lazily built + cached inspector-trace JSON for the current paused
    /// frame. Invalidated when the frame advances (`tick`), the paused frame
    /// changes (rewind/seek), or the program reloads.
    cached_trace: Option<String>,
    /// Per-file sha256 of the loaded `.fun` source, computed at load / reload
    /// (not per frame) — the wire contract's `sources`.
    source_hashes: Vec<InspectorSource>,
    /// The shell seam: installs the diagnostics sinks and drives the optional
    /// draw-error overlay. `NativePlatform` for the Quest shell / host tests;
    /// the web shell passes its own DOM-overlay platform.
    platform: Box<dyn ProducerPlatform>,
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
    has_webview: bool,
}

/// Load, check, and contract-validate a game PROJECT — the in-memory
/// counterpart of the desktop `load_source`, shared by the boot path
/// (`create`) and the push path (`reload_source`/`reload_project`). `sources`
/// is every project file as `(path, source)`, the ENTRY first, then siblings
/// (`file = module`, so `pieces.fun` is module `Pieces`). Errors come back as
/// fully rendered strings (`path:line:col: message`).
fn load_source(sources: &[(String, String)]) -> Result<Loaded, String> {
    let path = sources
        .first()
        .map(|(p, _)| p.clone())
        .unwrap_or_else(|| "game.fun".to_string());
    let pairs: Vec<(std::path::PathBuf, String)> = sources
        .iter()
        .map(|(p, s)| (std::path::PathBuf::from(p), s.clone()))
        .collect();
    // Link the same executable `.fun` modules and host `.funi` interfaces as
    // the other producers.
    let project = functor_lang::project::load_sources_with_bundled_modules(
        pairs,
        &functor_prelude::bundled_modules(),
    )
    .map_err(|e| format!("cannot load {}", e.render()))?;
    let module = project.module;
    let source_map = project.sources;
    // Type diagnostics are advisory in the dev loop: warn, keep going
    // (the CLI's `build` is the strict gate).
    for diag in functor_lang::check(&module) {
        log::warn!(
            "warning: {}",
            source_map.render(diag.span.start, &diag.message)
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
    // world after each tick (docs/physics.md).
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
    // Ui.panel), lowered to the shared text overlay.
    let has_ui = session.global("ui").is_some();
    if has_ui {
        require_function(&path, &session, "ui", 1)?;
    }
    // Optional webview: `webview(model)` returns an Html node (Html.div /
    // Html.text / …), rendered by shells that have an HTML overlay.
    let has_webview = session.global("webview").is_some();
    if has_webview {
        require_function(&path, &session, "webview", 1)?;
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
        has_webview,
    })
}

impl FunctorLangEmbeddedGame {
    /// Build the producer from in-memory project sources (entry FIRST, then
    /// siblings). Errors come back fully rendered for the shell to fail loud
    /// with (a boot either gets a valid game or an error).
    pub fn create(
        sources: Vec<(String, String)>,
        platform: Box<dyn ProducerPlatform>,
    ) -> Result<FunctorLangEmbeddedGame, String> {
        // Install the shell's diagnostics sinks BEFORE the first load so load
        // errors surface (native: the `log` sink; web: console/event bridge).
        platform.install_sinks();
        let path = sources
            .first()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| "game.fun".to_string());
        let loaded = load_source(&sources)?;
        log::info!("[functor-lang] loaded {path}");
        // Arm the paused-inspector journal on this thread: from now on every
        // live model-updating call is journaled (a cheap Rc-clone push).
        journal_arm();
        let source_hashes = inspector_sources(&loaded.sources);
        let runnable = functor_lang::coverage::runnable_offsets(&loaded.module);
        Ok(FunctorLangEmbeddedGame {
            reporter: Reporter::new(SpanSource::Project(loaded.sources), report_to_log),
            last_frame_journal: Vec::new(),
            journal_ring: std::collections::VecDeque::new(),
            runnable,
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
            has_webview: loaded.has_webview,
            last_webview: None,
            webview_handlers: Vec::new(),
            last_frame: empty_frame(),
            platform,
        })
    }

    /// Swap in a freshly loaded program, KEEPING THE MODEL — the desktop
    /// producer's `swap_in`, verbatim. `init` from the new program is
    /// deliberately unused: state survives the edit, and closures stored in
    /// the model rebind to the edited code (B5 part 2,
    /// `functor_lang::rebind_value`). The physics world is deliberately KEPT
    /// too, like the model: it lives in this process's registry, so bodies
    /// stay where they are across the edit (removing the `physics` hook drops
    /// the world). `prev_tts` is kept as well: `Sub.every` fires on the global
    /// time grid, so timers tick right through a reload. Returns the number of
    /// stored closures rebound, for the status line.
    fn swap_in(&mut self, loaded: Loaded) -> (usize, Option<(usize, f64)>) {
        let live_model_was_safe = self.recorder.prepare_reload(
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
        );
        let (model, report) = functor_lang::rebind_value(&self.model, &self.module, &loaded.module);
        self.model = model;
        for warning in &report.warnings {
            log::warn!("[functor-lang] reload: {warning}");
        }
        // Recompute the inspector source hashes for the edited files, and drop
        // the journal + cached trace: they refer to the OLD program's spans
        // and execution (reload clears both, like the other producers).
        self.source_hashes = inspector_sources(&loaded.sources);
        self.last_frame_journal.clear();
        self.journal_ring.clear(); // old program's spans
        self.runnable = functor_lang::coverage::runnable_offsets(&loaded.module);
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
        // A deferred query or in-flight HTTP request holds a tagger — a
        // closure into the OLD session; drop them rather than let them dangle.
        // A `playThen` completion message closes over the old session too.
        self.deferred_queries.clear();
        self.pending_events.clear();
        clear_http_taggers();
        clear_audio_completions();
        clear_preload_completions();
        // The widget handler table holds msgs/taggers into the OLD session;
        // the next render's `ui(model)` rebuilds it against the new one.
        self.ui_handlers.clear();
        self.webview_handlers.clear();
        // Plain-data snapshots remain seekable under the new program. A model
        // history containing callable or opaque host values instead starts a
        // new generation anchored at this rebound live frame.
        self.recorder
            .finish_reload(&self.model, self.physics_frame, live_model_was_safe);
        let replay_started = now_ms();
        let history_replay = match crate::functor_lang_producer::materialize_counterfactual_history(
            &self.session,
            &mut self.model,
            &mut self.recorder,
            self.has_physics,
            self.has_subscriptions,
            !self.input_buf.is_empty(),
        ) {
            Ok(frames) => frames.map(|frames| (frames, now_ms() - replay_started)),
            Err(error) => {
                self.reporter.report_once(format!("[functor-lang] {error}"));
                None
            }
        };
        self.has_ui = loaded.has_ui;
        if !self.has_ui {
            // Deleting the `ui` hook drops the HUD (the physics-world rule).
            self.last_view = View::Empty;
        }
        self.has_webview = loaded.has_webview;
        if !self.has_webview {
            // Deleting the `webview` hook drops the overlay (the `ui` rule).
            self.last_webview = None;
        }
        self.reporter.reset();
        // The shell may have hidden the draw-error overlay out-of-band during
        // the reload; reset the platform's dedupe shadow so the reloaded
        // program's first draw re-shows it if that program's `draw` still errors.
        self.platform.on_reload();
        (report.rebound, history_replay)
    }

    /// Bundle this producer's per-frame state into the shared [`FrameCtx`]
    /// (docs/time-travel.md T6a) — the frame body and its helpers (`absorb`,
    /// `pump_subscriptions`, `step_physics`, `deliver_*`) live there, one copy
    /// for all shells. A cheap borrow-only view, rebuilt per call.
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

impl GameProducer for FunctorLangEmbeddedGame {
    // File-watch hot reload needs a filesystem; the PUSH path below is the
    // embedded producer's reload.
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn push_asset_progress(&mut self, progress: crate::asset::AssetProgress) {
        // Stored, not delivered here: the producer compares it against what
        // the game last saw during the frame's subscription phase.
        self.asset_progress = Some(progress);
    }

    fn reload_source(&mut self, source: &str) -> Result<String, String> {
        // The editor push path (docs/functor-lang.md D4), same semantics as the
        // desktop runner's `POST /reload-source`: model preserved, a broken
        // push keeps the old program (and the error goes back to the pusher,
        // who is looking at the source that caused it).
        let started = now_ms();
        // The push replaces the ENTRY buffer; siblings keep their last-pushed
        // text. A load failure leaves `self.sources` untouched.
        let mut sources = self.sources.clone();
        if let Some(entry) = sources.first_mut() {
            entry.1 = source.to_string();
        } else {
            sources.push((self.path.clone(), source.to_string()));
        }
        let loaded = load_source(&sources)?;
        self.sources = sources;
        let (rebound, history_replay) = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        let history = replay_status(history_replay);
        let status = format!(
            "reloaded {} from pushed source in {:.2}ms (model preserved{stored}{history})",
            self.path,
            now_ms() - started
        );
        log::info!("[functor-lang] {status}");
        Ok(status)
    }

    fn reload_project(&mut self, files: &[(String, String)]) -> Result<String, String> {
        // The multi-file push path: the pusher owns the WHOLE file set, so —
        // unlike `reload_source`, which swaps the entry and keeps the
        // last-pushed siblings — this replaces every module. Entry first,
        // then siblings; same keep-old-program-on-failure semantics.
        if files.is_empty() {
            return Err("a pushed project needs at least the entry file".to_string());
        }
        let started = now_ms();
        let loaded = load_source(files)?;
        self.sources = files.to_vec();
        self.path = files[0].0.clone();
        let (rebound, history_replay) = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        let history = replay_status(history_replay);
        let status = format!(
            "reloaded {} ({} file(s)) from pushed project in {:.2}ms \
(model preserved{stored}{history})",
            self.path,
            files.len(),
            now_ms() - started
        );
        log::info!("[functor-lang] {status}");
        Ok(status)
    }

    /// Coupled scene rewind — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T1), identical to the other producers.
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
            // The restored model predates the current loading snapshot —
            // redeliver it on the next frame (see before_physics).
            self.delivered_asset_progress = None;
            // Model restored to `target`; drop orphaned buffered input so it
            // can't record into the branch.
            self.input_buf.clear();
            // The scrubbed frame is a historical one whose journal we didn't
            // keep — report it honestly as empty invocations.
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
            // Same as rewind: the model was restored, so buffered input since
            // the last recorded frame is orphaned and must not enter the branch.
            self.input_buf.clear();
            // The paused frame changed — clear the last-frame journal and cache
            // so the trace reflects the scrubbed frame.
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

    fn recorded_inputs_at(&self, rendered_frame: u64) -> Vec<crate::RecordedInput> {
        self.recorder.inputs_at(rendered_frame).to_vec()
    }

    fn scene_timeline_generation(&self) -> u64 {
        self.recorder.generation()
    }

    fn scene_program_revision(&self) -> u64 {
        self.recorder.program_revision()
    }

    fn current_scene_tts(&self) -> Option<f64> {
        self.recorder.current_scene_frame_tts()
    }

    /// Forward-ghosting (docs/time-travel.md T6d) — delegated to the shared
    /// producer body (`functor_lang_producer::ghost_frames`), identical to the
    /// other producers.
    fn ghost_frames(
        &self,
        divisions: usize,
        dt: f32,
        start_tts: f64,
        script_inputs: Option<&[Vec<crate::RecordedInput>]>,
    ) -> Vec<(Frame, FrameTime)> {
        crate::functor_lang_producer::ghost_frames(
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
        // (docs/time-travel.md T6a), run as one call — like the web producer,
        // there is no per-frame perf timing to split it at the physics
        // boundary (the C6 perf gate measures on desktop).
        self.ctx().run_frame(frame_time);
        // A real frame ran: swap its journal into `last_frame_journal`
        // (leaving a fresh armed journal) and drop the cached trace (the frame
        // advanced). A paused frame never reaches here, so its last real frame
        // is kept.
        if let Some(journal) = journal_swap() {
            // The ring shares the frame's entries (Rc-cloned args — cheap);
            // coverage replays them lazily at pause time.
            let frame = self.recorder.current_scene_frame().unwrap_or(0);
            self.journal_ring.push_back((frame, journal.clone()));
            while self.journal_ring.len() > crate::inspector::COVERAGE_RING_FRAMES {
                self.journal_ring.pop_front();
            }
            self.last_frame_journal = journal;
        }
        self.cached_trace = None;
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        // The optional `input` entry point: (model, key, isDown) => model.
        // Keys cross as the built-in `Key` module's variants (`Key.W`,
        // `Key.Up`, `Key.Num0`) — mirrors the other producers.
        if !self.has_input {
            return;
        }
        let Some(key_value) = crate::key_input_value(code) else {
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
            .push(crate::RecordedInput::Key { code, is_down });
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
        self.input_buf.push(crate::RecordedInput::MouseMove { x, y });
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
        self.input_buf.push(crate::RecordedInput::MouseWheel { delta });
    }

    fn ui_event(&mut self, event: crate::ui::UiEvent) {
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
        self.input_buf.push(crate::RecordedInput::UiEvent(event));
    }

    fn webview_event(&mut self, event: crate::ui::UiEvent) {
        // The `ui_event` shape, against the webview's own handler table.
        if !self.has_webview {
            return;
        }
        let handlers = std::mem::take(&mut self.webview_handlers);
        self.ctx().deliver_ui_event(&handlers, &event);
        self.webview_handlers = handlers;
        // Its own variant, so replay resolves against the webview handler table.
        self.input_buf
            .push(crate::RecordedInput::WebviewEvent(event));
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        // While scrubbing, draw at the scrubbed frame's recorded `tts` so
        // `tts`-driven visuals rewind with the model; live play uses the real
        // clock (docs/time-travel.md).
        let tts = self
            .recorder
            .scrub_render_tts()
            .unwrap_or(frame_time.tts as f64);
        let args = vec![self.model.clone(), Value::Number(tts)];
        match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => {
                    self.last_frame = frame.clone();
                    // A live draw clears any draw-error overlay: the shell is
                    // rendering again (a transient/first-frame error recovers).
                    self.platform.set_draw_overlay(None);
                }
                None => {
                    let rendered = format!(
                        "[functor-lang] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    self.platform.set_draw_overlay(Some(&rendered));
                    self.reporter.report_once(rendered);
                }
            },
            Err(err) => {
                let rendered = self.reporter.render_frame_error("draw", &err);
                self.platform.set_draw_overlay(Some(&rendered));
                self.reporter.report_once(rendered);
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
        // The optional webview, evaluated beside `draw` like `ui` — same
        // caching, same handler-adoption lockstep, its own handler table.
        if self.has_webview {
            match self
                .session
                .call("webview", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(value) => match html_node_value(&value) {
                    Some(node) => {
                        self.last_webview = Some(node.clone());
                        self.webview_handlers = take_ui_handlers();
                    }
                    None => {
                        let _ = take_ui_handlers();
                        self.reporter.report_once(format!(
                            "[functor-lang] webview must return an Html node (Html.div / Html.text / …), got {}",
                            value.kind_name()
                        ))
                    }
                },
                Err(err) => {
                    let _ = take_ui_handlers();
                    self.reporter.frame_error("webview", &err)
                }
            }
        }
        // The optional soundscape, evaluated beside `draw` (same settled
        // model) and cached — `audio_scene_json` is a `&self` accessor, and
        // errors need `&mut` dedupe (the `ui` pattern).
        if self.has_soundscape {
            match self
                .session
                .call("soundScape", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(value) => match audio_scene_of(&value) {
                    Some(scene) => {
                        self.last_soundscape_json = crate::audio::scene_to_json(scene)
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
        // the screen.
        self.last_frame.clone()
    }

    fn ui(&self) -> View {
        self.last_view.clone()
    }

    fn webview(&self) -> Option<HtmlNode> {
        self.last_webview.clone()
    }

    fn state_debug(&self) -> String {
        self.model.to_string()
    }

    /// The paused-inspector trace (visual-debugger PR2b), same contract and
    /// caching as the other producers: the byte-stable stub while playing, and
    /// a lazily built + cached full doc while paused.
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
        let ring: Vec<(u64, Vec<JournalEntry>)> = self.journal_ring.iter().cloned().collect();
        let json = crate::inspector::build_trace_doc_with_coverage(
            true,
            frame,
            tts,
            &self.source_hashes,
            &self.last_frame_journal,
            Some(&draw_args),
            &ring,
            &self.runnable,
            &self.session,
        );
        self.cached_trace = Some(json.clone());
        json
    }

    fn net_drain_commands(&self) -> String {
        // HttpRequest commands (Effect.httpGet/httpPost); the shell performs
        // them (or drains-and-drops when it has no HTTP host yet).
        crate::net::drain_commands_json()
    }
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String) {
        self.ctx().deliver_http_result(crate::net::HttpResult {
            token: token as u64,
            status: status as u16,
            body: body.into_bytes(),
            error: None,
        });
    }
    fn net_push_http_error(&mut self, token: i32, message: String) {
        self.ctx().deliver_http_result(crate::net::HttpResult {
            token: token as u64,
            status: 0,
            body: Vec::new(),
            error: Some(message),
        });
    }
    fn audio_drain_commands(&self) -> String {
        // One-shot commands (Effect.play/playAt/playThen); the shell's audio
        // host plays them (or drains-and-drops without one).
        crate::audio::drain_commands_json()
    }
    fn audio_scene_json(&self) -> String {
        // The continuous soundscape, evaluated + cached in `render` (the `ui`
        // pattern) so this stays a cheap `&self` read.
        self.last_soundscape_json.clone()
    }
    fn net_drain_conn_commands(&self) -> String {
        crate::net::drain_conn_commands_json()
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

    // preload_drain_commands: the trait default drains the shared queue.
    fn preload_push_settled(&mut self, token: u64) {
        self.ctx().deliver_preload_completion(token);
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

/// The embedded `Reporter` sink: per-frame problems go through the `log`
/// facade (the shell owns the logger).
fn report_to_log(message: &str) {
    log::error!("{message}");
}

/// The silent soundscape's wire form — the default before/without a
/// `soundScape` hook (matches `AudioScene::default()` serialized).
fn empty_soundscape_json() -> String {
    "{\"sources\":[]}".to_string()
}

fn empty_frame() -> Frame {
    use cgmath::{Matrix4, SquareMatrix};
    Frame::new(
        crate::Camera::default(),
        crate::Scene3D {
            obj: crate::SceneObject::Group(vec![]),
            xform: Matrix4::identity(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT: &str = r#"
let init = { spin: 0.0 }

let tick = (model, dt, tts) => { spin: model.spin + dt }

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(6.0, 4.0, -8.0), Vec3.make(0.0, 0.5, 0.0)),
    Scene.group([
      Scene.cube() |> Scene.rotateY(Angle.radians(model.spin))
    ]),
    [Light.ambient(Color.rgb(0.2, 0.2, 0.2))])
"#;

    fn frame_time(tts: f32, dts: f32) -> FrameTime {
        FrameTime { tts, dts }
    }

    #[test]
    fn boots_ticks_renders_and_reloads_preserving_the_model() {
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), BOOT.to_string())],
            Box::new(NativePlatform),
        )
        .expect("boot scene loads");

        // A few frames advance the model and produce a real (non-empty) frame.
        for i in 1..=3 {
            let ft = frame_time(i as f32 * 0.016, 0.016);
            game.tick(ft.clone());
            let frame = game.render(ft);
            assert!(
                !matches!(&frame.scene.obj, crate::SceneObject::Group(children) if children.is_empty()),
                "draw produced the game's scene, not the empty fallback"
            );
        }
        let spun = game.state_debug();
        assert!(
            spun.contains("spin"),
            "model is the game's record: {spun}"
        );

        // Push an edited program: the model must survive (spin keeps its
        // accumulated value; only the code changed).
        let edited = BOOT.replace("model.spin + dt", "model.spin + dt + dt");
        let status = game.reload_source(&edited).expect("push reloads");
        assert!(
            status.contains("model preserved"),
            "reload status says so: {status}"
        );
        assert_eq!(
            game.state_debug(),
            spun,
            "the pushed reload preserved the model verbatim"
        );

        // A broken push keeps the old program running.
        let err = game
            .reload_source("let init = { spin: 0.0 }")
            .expect_err("missing tick/draw is a load error");
        assert!(err.contains("tick"), "the error names the contract: {err}");
        let ft = frame_time(0.1, 0.016);
        game.tick(ft.clone());
        let _ = game.render(ft); // still renders under the old program
    }
}
