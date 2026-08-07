//! The EMBEDDED Functor Lang producer: the portable, in-memory sibling of the desktop
//! runner's `functor_lang_game.rs` and the web shell's `functor_lang_game.rs`, behind the
//! same `GameProducer` seam. Same load-time contract validation and per-frame
//! semantics — sampled input, then the MVU pair (subscriptions fold through
//! `update` before `tick`), the optional `physics` hook (tick → physics →
//! draw), a bad frame
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
    frame_value, html_node_value, now_ms, take_ui_handlers, view_value, EffectLog, EffectRunner,
    EffectTree, FunctorHost, NetEventKind, RealEffects, UiHandler,
};
use crate::functor_lang_producer::{
    journal_arm, journal_swap, rebase_connect_retry_deadlines, validate_contract,
    ConnectRetryState, EntryNames, EntryRole, FrameCtx, JournalEntry, Reporter, SpanSource,
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
    /// How the role names its entry bindings (same-file entries): a binding
    /// prefix or an inline `module` block. Kept because an inline-module role
    /// re-resolves against every (re)loaded project.
    role: EntryRole,
    /// The role's resolved entry-point names, from the CURRENT program —
    /// every canonical lookup and contract error goes through it.
    names: EntryNames,
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
    /// How many times [`Self::model`] has been replaced by game logic — the
    /// debug protocol's `model_revision`. Counted by `FrameCtx::absorb`, and
    /// deliberately NOT reset by a hot reload (which rebinds the model rather
    /// than replacing it), so it stays monotone for the life of the process.
    model_revision: u64,
    has_input: bool,
    has_sampled_input: bool,
    /// The shell samples physical state before a fixed simulation step. Hold
    /// that coeffect until the shared frame body has committed any pending
    /// scrub branch, then deliver it against the restored authoritative model.
    pending_sampled_input: Option<crate::InputSnapshot>,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_mouse_button: bool,
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
    connect_retries: std::collections::HashMap<String, ConnectRetryState>,
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
    /// The role's binding names, resolved against THIS load (an inline-module
    /// role's canonical path comes from the linked project).
    names: EntryNames,
    sources: SourceMap,
    module: functor_lang::ir::Module,
    session: Session,
    init: Value,
    has_input: bool,
    has_sampled_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_mouse_button: bool,
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
fn load_source(sources: &[(String, String)], role: &EntryRole) -> Result<Loaded, String> {
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
    // An inline-module role resolves against the linked project, so a reload
    // that renamed or deleted the block fails here — naming the block — rather
    // than reporting its every entry binding as missing.
    let names = role.resolve(&path, &project)?;
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
    // The producer contract (init a value, tick/draw functions of the right
    // arity, optional hooks well-shaped) is shared with the desktop producer
    // and the CLI's build gate — errors name the ROLE'S resolved bindings
    // (`serverTick`, not `tick`) via `names`.
    let contract = validate_contract(&path, &session, &names)?;
    Ok(Loaded {
        names,
        sources: source_map,
        module,
        session,
        init: contract.init,
        has_input: contract.has_input,
        has_sampled_input: contract.has_sampled_input,
        has_mouse_move: contract.has_mouse_move,
        has_mouse_wheel: contract.has_mouse_wheel,
        has_mouse_button: contract.has_mouse_button,
        has_subscriptions: contract.has_subscriptions,
        has_physics: contract.has_physics,
        has_soundscape: contract.has_soundscape,
        has_ui: contract.has_ui,
        has_webview: contract.has_webview,
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
        Self::create_for_role(sources, EntryRole::Prefix(String::new()), platform)
    }

    /// [`Self::create`] for one same-file ROLE: either a binding prefix
    /// (`"server"` → `serverInit`/`serverTick`/…, empty = the classic
    /// unprefixed contract) or an inline `module Server { … }` block, whose
    /// members ARE the role's contract (`Server.init`/`Server.tick`/…). The
    /// role is kept, so every (re)load re-resolves it against that program.
    pub fn create_for_role(
        sources: Vec<(String, String)>,
        role: EntryRole,
        platform: Box<dyn ProducerPlatform>,
    ) -> Result<FunctorLangEmbeddedGame, String> {
        // Install the shell's diagnostics sinks BEFORE the first load so load
        // errors surface (native: the `log` sink; web: console/event bridge).
        platform.install_sinks();
        let path = sources
            .first()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| "game.fun".to_string());
        let loaded = load_source(&sources, &role)?;
        log::info!("[functor-lang] loaded {path}");
        // Arm the paused-inspector journal on this thread: from now on every
        // live model-updating call is journaled (a cheap Rc-clone push).
        journal_arm();
        let source_hashes = inspector_sources(&loaded.sources);
        let runnable = functor_lang::coverage::runnable_offsets(&loaded.module);
        let mut game = FunctorLangEmbeddedGame {
            reporter: Reporter::new(SpanSource::Project(loaded.sources), report_to_log),
            last_frame_journal: Vec::new(),
            journal_ring: std::collections::VecDeque::new(),
            runnable,
            cached_trace: None,
            source_hashes,
            sources,
            path,
            role,
            names: loaded.names,
            module: loaded.module,
            session: loaded.session,
            model: loaded.init,
            model_revision: 0,
            has_input: loaded.has_input,
            has_sampled_input: loaded.has_sampled_input,
            pending_sampled_input: None,
            has_mouse_move: loaded.has_mouse_move,
            has_mouse_wheel: loaded.has_mouse_wheel,
            has_mouse_button: loaded.has_mouse_button,
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
            connect_retries: std::collections::HashMap::new(),
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
        };
        // Cold start (docs/physics.md): declare the initial world before the
        // first frame, so frame 1's physics reads answer instead of raising.
        // The world is a thread-local, so drop whatever an earlier producer on
        // this thread left behind first — otherwise a same-tag body from the
        // previous session would answer this session's priming reads.
        physics::remove_world(physics::DEFAULT_WORLD);
        game.ctx().prime_physics();
        Ok(game)
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
        let retry_tts_before_reload = self.prev_tts;
        let live_model_was_safe = self.recorder.prepare_reload(
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
            &mut self.prev_tts,
        );
        rebase_connect_retry_deadlines(
            &mut self.connect_retries,
            retry_tts_before_reload,
            self.prev_tts,
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
        self.names = loaded.names;
        self.module = loaded.module;
        self.session = loaded.session;
        self.has_input = loaded.has_input;
        self.has_sampled_input = loaded.has_sampled_input;
        self.pending_sampled_input = None;
        self.has_mouse_move = loaded.has_mouse_move;
        self.has_mouse_wheel = loaded.has_mouse_wheel;
        self.has_mouse_button = loaded.has_mouse_button;
        self.has_subscriptions = loaded.has_subscriptions;
        let had_physics = self.has_physics;
        self.has_physics = loaded.has_physics;
        if !self.has_physics {
            physics::remove_world(physics::DEFAULT_WORLD);
        } else if !had_physics {
            // The edit ADDED the hook: there is no surviving world to keep, so
            // this reload is a cold start for physics and must prime like one
            // (docs/physics.md). Without it the very next frame's reads face an
            // empty world — the cold-start hole this PR closes, reopened by the
            // most common way to reach it.
            self.ctx().prime_physics();
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
            .configure_origin_replay_for_sampled_input(self.has_sampled_input);
        self.recorder
            .finish_reload(&self.model, self.physics_frame, live_model_was_safe);
        let replay_started = now_ms();
        let history_replay = match crate::functor_lang_producer::materialize_counterfactual_history(
            &self.session,
            &self.names,
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

    /// Install a freshly loaded project as a NEW game. This is deliberately
    /// separate from `swap_in`: a device runtime boots a tiny placeholder
    /// program, so its first real project cannot preserve that unrelated
    /// model or runtime history.
    fn reset_in(&mut self, loaded: Loaded) {
        self.ctx().close_all_connections();
        physics::remove_world(physics::DEFAULT_WORLD);
        clear_http_taggers();
        clear_audio_completions();
        clear_preload_completions();

        self.source_hashes = inspector_sources(&loaded.sources);
        self.last_frame_journal.clear();
        self.journal_ring.clear();
        self.runnable = functor_lang::coverage::runnable_offsets(&loaded.module);
        self.cached_trace = None;
        journal_swap();
        self.reporter
            .set_source(SpanSource::Project(loaded.sources));
        self.names = loaded.names;
        self.module = loaded.module;
        self.session = loaded.session;
        self.model = loaded.init;
        self.has_input = loaded.has_input;
        self.has_sampled_input = loaded.has_sampled_input;
        self.pending_sampled_input = None;
        self.has_mouse_move = loaded.has_mouse_move;
        self.has_mouse_wheel = loaded.has_mouse_wheel;
        self.has_mouse_button = loaded.has_mouse_button;
        self.has_subscriptions = loaded.has_subscriptions;
        self.prev_tts = None;
        self.asset_progress = None;
        self.delivered_asset_progress = None;
        self.has_physics = loaded.has_physics;
        self.has_soundscape = loaded.has_soundscape;
        self.last_soundscape_json = empty_soundscape_json();
        self.has_ui = loaded.has_ui;
        self.last_view = View::Empty;
        self.ui_handlers.clear();
        self.has_webview = loaded.has_webview;
        self.last_webview = None;
        self.webview_handlers.clear();
        self.effect_runner = RealEffects::new();
        self.effect_log = EffectLog::new();
        self.deferred_queries.clear();
        self.pending_events.clear();
        self.physics_rt = physics::SteppedPhysics::new();
        self.physics_frame = 0;
        self.recorder = SceneRecorder::new();
        self.input_buf.clear();
        self.last_frame = empty_frame();
        self.reporter.reset();
        // A reset is a cold start: the model is `init` again and the world was
        // removed, so re-prime it (a hot reload deliberately does NOT — there
        // the world, like the model, survives).
        self.ctx().prime_physics();
        self.platform.on_reload();
    }

    /// Bundle this producer's per-frame state into the shared [`FrameCtx`]
    /// (docs/time-travel.md T6a) — the frame body and its helpers (`absorb`,
    /// `pump_subscriptions`, `step_physics`, `deliver_*`) live there, one copy
    /// for all shells. A cheap borrow-only view, rebuilt per call.
    fn ctx(&mut self) -> FrameCtx<'_> {
        FrameCtx {
            session: &self.session,
            names: &self.names,
            model: &mut self.model,
            model_revision: &mut self.model_revision,
            physics_rt: &mut self.physics_rt,
            physics_frame: &mut self.physics_frame,
            recorder: &mut self.recorder,
            effect_runner: &mut self.effect_runner as &mut dyn EffectRunner,
            effect_log: &mut self.effect_log,
            deferred_queries: &mut self.deferred_queries,
            pending_events: &mut self.pending_events,
            live_conn_keys: &mut self.live_conn_keys,
            connect_retries: &mut self.connect_retries,
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

    fn uses_captured_mouse_input(&self) -> bool {
        self.has_mouse_move || self.has_mouse_wheel || self.has_mouse_button
    }

    fn push_asset_progress(&mut self, progress: crate::asset::AssetProgress) {
        // Stored, not delivered here: the producer compares it against what
        // the game last saw during the frame's subscription phase.
        self.asset_progress = Some(progress);
    }

    fn project_sources(&self) -> Option<crate::debug_protocol::ProjectSources> {
        // Every source this producer has ever run arrived over the wire, so
        // its own buffers ARE the truth.
        Some(self.sources.clone())
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
        let loaded = load_source(&sources, &self.role)?;
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

    fn set_entry_role(&mut self, role: EntryRole) -> Option<EntryRole> {
        Some(std::mem::replace(&mut self.role, role))
    }

    fn entry_role(&self) -> Option<EntryRole> {
        Some(self.role.clone())
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
        let loaded = load_source(files, &self.role)?;
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

    fn load_project(&mut self, files: &[(String, String)]) -> Result<String, String> {
        if files.is_empty() {
            return Err("a pushed project needs at least the entry file".to_string());
        }
        let started = now_ms();
        let loaded = load_source(files, &self.role)?;
        self.sources = files.to_vec();
        self.path = files[0].0.clone();
        self.reset_in(loaded);
        let status = format!(
            "loaded {} ({} file(s)) from pushed project in {:.2}ms (model initialized)",
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
        let retry_tts_before_rewind = self.prev_tts;
        let result = self.recorder.rewind_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
            &mut self.prev_tts,
        );
        if result.is_ok() {
            rebase_connect_retry_deadlines(
                &mut self.connect_retries,
                retry_tts_before_rewind,
                self.prev_tts,
            );
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

    /// Backward-trailing (docs/time-travel.md T6e) — delegated to the shared
    /// producer body (`functor_lang_producer::history_frames`), identical to
    /// the other producers.
    fn history_frames(&self, divisions: usize, dt: f32) -> Vec<(Frame, FrameTime)> {
        crate::functor_lang_producer::history_frames(
            &self.session,
            &self.names,
            &self.recorder,
            &self.physics_rt,
            self.has_physics,
            divisions,
            dt,
        )
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
            &self.names,
            &self.model,
            &self.recorder,
            self.has_physics,
            self.has_subscriptions,
            self.prev_tts,
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
        let sampled_input = self.pending_sampled_input.take();
        self.ctx().run_frame(frame_time, sampled_input.as_ref());
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

    fn samples_input(&self) -> bool {
        self.has_sampled_input
    }

    fn sampled_input(&mut self, snapshot: &crate::InputSnapshot) {
        self.pending_sampled_input = Some(snapshot.clone());
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
        match self.session.call(self.names.input, args, &mut FunctorHost) {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error(self.names.input, &err),
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
        match self
            .session
            .call(self.names.mouse_move, args, &mut FunctorHost)
        {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error(self.names.mouse_move, &err),
        }
        self.input_buf
            .push(crate::RecordedInput::MouseMove { x, y });
    }

    fn mouse_wheel(&mut self, delta: i32) {
        if !self.has_mouse_wheel {
            return;
        }
        let args = vec![self.model.clone(), Value::Number(delta as f64)];
        match self
            .session
            .call(self.names.mouse_wheel, args, &mut FunctorHost)
        {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error(self.names.mouse_wheel, &err),
        }
        self.input_buf
            .push(crate::RecordedInput::MouseWheel { delta });
    }

    fn mouse_button(&mut self, button: i32, is_down: bool) {
        // The optional `mouseButton` entry point: (model, button, isDown) =>
        // model. Buttons cross as the built-in `Mouse` module's variants
        // (`Mouse.Left`) — mirrors the other producers.
        if !self.has_mouse_button {
            return;
        }
        let Some(button_value) = crate::mouse_button_input_value(button) else {
            return; // unrecognized code / MouseButton::Unknown — never delivered.
        };
        let args = vec![self.model.clone(), button_value, Value::Bool(is_down)];
        match self
            .session
            .call(self.names.mouse_button, args, &mut FunctorHost)
        {
            Ok(returned) => self.ctx().absorb(returned),
            Err(err) => self.reporter.frame_error(self.names.mouse_button, &err),
        }
        self.input_buf
            .push(crate::RecordedInput::MouseButton { button, is_down });
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
        match self.session.call(self.names.draw, args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => {
                    self.last_frame = frame.clone();
                    // A live draw clears any draw-error overlay: the shell is
                    // rendering again (a transient/first-frame error recovers).
                    self.platform.set_draw_overlay(None);
                }
                None => {
                    let rendered = format!(
                        "[functor-lang] {} must return Frame.create(camera, scene), got {}",
                        self.names.draw,
                        value.kind_name()
                    );
                    self.platform.set_draw_overlay(Some(&rendered));
                    self.reporter.report_once(rendered);
                }
            },
            Err(err) => {
                let rendered = self.reporter.render_frame_error(self.names.draw, &err);
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
                .call(self.names.ui, vec![self.model.clone()], &mut FunctorHost)
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
                            "[functor-lang] {} must return a View (Ui.text / Ui.column / Ui.panel), got {}",
                            self.names.ui,
                            value.kind_name()
                        ))
                    }
                },
                Err(err) => {
                    // A failed evaluation keeps the last good view AND its
                    // handlers; drop the partial table it registered.
                    let _ = take_ui_handlers();
                    self.reporter.frame_error(self.names.ui, &err)
                }
            }
        }
        // The optional webview, evaluated beside `draw` like `ui` — same
        // caching, same handler-adoption lockstep, its own handler table.
        if self.has_webview {
            match self.session.call(
                self.names.webview,
                vec![self.model.clone()],
                &mut FunctorHost,
            ) {
                Ok(value) => match html_node_value(&value) {
                    Some(node) => {
                        self.last_webview = Some(node.clone());
                        self.webview_handlers = take_ui_handlers();
                    }
                    None => {
                        let _ = take_ui_handlers();
                        self.reporter.report_once(format!(
                            "[functor-lang] {} must return an Html node (Html.div / Html.text / …), got {}",
                            self.names.webview,
                            value.kind_name()
                        ))
                    }
                },
                Err(err) => {
                    let _ = take_ui_handlers();
                    self.reporter.frame_error(self.names.webview, &err)
                }
            }
        }
        // The optional soundscape, evaluated beside `draw` (same settled
        // model) and cached — `audio_scene_json` is a `&self` accessor, and
        // errors need `&mut` dedupe (the `ui` pattern).
        if self.has_soundscape {
            match self.session.call(
                self.names.sound_scape,
                vec![self.model.clone()],
                &mut FunctorHost,
            ) {
                Ok(value) => match audio_scene_of(&value) {
                    Some(scene) => self.last_soundscape_json = crate::audio::scene_to_json(scene),
                    None => self.reporter.report_once(format!(
                        "[functor-lang] {} must return an AudioScene (AudioScene.create / \
AudioScene.empty), got {}",
                        self.names.sound_scape,
                        value.kind_name()
                    )),
                },
                Err(err) => self.reporter.frame_error(self.names.sound_scape, &err),
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

    fn state_json(&self) -> serde_json::Value {
        crate::functor_lang_prelude::value_to_json(&self.model)
    }

    fn model_revision(&self) -> u64 {
        self.model_revision
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
            Some((self.names.draw, &draw_args)),
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
    Camera3D.lookAt(Vec3.make(6.0, 4.0, -8.0), Vec3.make(0.0, 0.5, 0.0)),
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
        assert!(spun.contains("spin"), "model is the game's record: {spun}");

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

    #[test]
    fn a_prefixed_role_resolves_and_names_the_prefixed_contract() {
        // The boot scene as a `server` role (same-file entries): every entry
        // binding resolves through the prefix as camelCase.
        let server_boot = BOOT
            .replace("let init", "let serverInit")
            .replace("let tick", "let serverTick")
            .replace("let draw", "let serverDraw");
        let mut game = FunctorLangEmbeddedGame::create_for_role(
            vec![("game.fun".to_string(), server_boot)],
            EntryRole::Prefix("server".to_string()),
            Box::new(NativePlatform),
        )
        .expect("prefixed role loads");
        let ft = frame_time(0.016, 0.016);
        game.tick(ft.clone());
        let frame = game.render(ft);
        assert!(
            !matches!(&frame.scene.obj, crate::SceneObject::Group(children) if children.is_empty()),
            "serverDraw produced the game's scene, not the empty fallback"
        );
        assert!(
            game.state_debug().contains("spin"),
            "serverTick advanced the model"
        );

        // The prefixed sibling of the broken-push case below: a push missing
        // the role's tick names `serverTick`, never the canonical `tick`.
        let err = game
            .reload_source("let serverInit = { spin: 0.0 }")
            .expect_err("missing serverTick/serverDraw is a load error");
        assert!(
            err.contains("serverTick"),
            "the error names the prefixed contract: {err}"
        );

        // An UNPREFIXED program under the server role misses the contract —
        // and the error teaches the resolved name it looked for.
        let err = match FunctorLangEmbeddedGame::create_for_role(
            vec![("game.fun".to_string(), BOOT.to_string())],
            EntryRole::Prefix("server".to_string()),
            Box::new(NativePlatform),
        ) {
            Err(err) => err,
            Ok(_) => panic!("unprefixed bindings don't satisfy a prefixed role"),
        };
        assert!(err.contains("serverInit"), "{err}");
    }

    /// A `{ "file": …, "module": "Server" }` role runs the BLOCK's members as
    /// its contract on the EMBEDDED producer too (the web/device shells), and
    /// keeps re-resolving them on every pushed reload: an edit inside the
    /// block lands with the model preserved, while a push that removes the
    /// block fails loudly — naming it — and keeps the old program.
    #[test]
    fn a_module_role_resolves_and_hot_reloads_on_a_push() {
        let role_src = |probe: f64| {
            format!(
                "{BOOT}module Server {{\n\
                 let init = {{ n: 7.0 }}\n\
                 let tick = (m, dt, tts) => m\n\
                 let draw = (m, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), \
Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n\
                 let probe = {probe}.0\n\
                 }}\n"
            )
        };
        let mut game = FunctorLangEmbeddedGame::create_for_role(
            vec![("game.fun".to_string(), role_src(1.0))],
            EntryRole::Module("Server".to_string()),
            Box::new(NativePlatform),
        )
        .expect("module role loads");
        // The role's contract is the block's, not the file's top level: the
        // file's own `init` is `{ spin: 0.0 }`.
        assert_eq!(game.names.tick, "Server.tick");
        assert!(
            game.state_debug().contains("n: 7"),
            "the block's init is the role's model: {}",
            game.state_debug()
        );
        let ft = frame_time(0.016, 0.016);
        game.tick(ft.clone());
        let _ = game.render(ft);

        // A push re-resolves the block: the edit lands, the model survives.
        game.reload_source(&role_src(2.0)).expect("push reloads");
        assert_eq!(game.names.tick, "Server.tick");
        assert_eq!(
            game.session
                .global("Server.probe")
                .expect("probe")
                .to_string(),
            "2",
            "the edit must have landed"
        );
        assert!(game.state_debug().contains("n: 7"), "model preserved");

        // Deleting the block fails the reload naming it, keeping the program.
        let err = game
            .reload_source(BOOT)
            .expect_err("a role whose module vanished cannot load");
        assert!(
            err.contains("module Server"),
            "the error names the block: {err}"
        );
        assert_eq!(
            game.session
                .global("Server.probe")
                .expect("probe")
                .to_string(),
            "2",
            "the old program keeps running"
        );
    }

    /// An unknown role module is a load error naming the role's file and the
    /// blocks it DOES declare — the resolver's teaching error, reaching the
    /// embedded shells' boot path.
    #[test]
    fn an_unknown_role_module_lists_the_files_blocks() {
        let source = format!("{BOOT}module Server {{ let probe = 1.0 }}\n");
        let err = FunctorLangEmbeddedGame::create_for_role(
            vec![("game.fun".to_string(), source)],
            EntryRole::Module("Sever".to_string()),
            Box::new(NativePlatform),
        )
        .err()
        .expect("an unknown block cannot boot");
        assert!(
            err.contains("no inline `module Sever") && err.contains("it declares: Server"),
            "{err}"
        );
    }

    /// The DEVICE path (`functor run vr`): the APK boots its embedded scene
    /// under the plain contract and learns the role from the push itself.
    /// This is the whole wire loop minus HTTP — adopt the declared role, load,
    /// re-push under it, and reject a push that deletes the block.
    #[test]
    fn a_pushed_role_boots_re_resolves_and_survives_a_broken_push() {
        let role_src = |probe: f64| {
            format!(
                "let unrelated = 1.0\n\
                 module Server {{\n\
                 let init = {{ n: 7.0 }}\n\
                 let tick = (m, dt, tts) => m\n\
                 let draw = (m, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), \
Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n\
                 let probe = {probe}.0\n\
                 }}\n"
            )
        };
        // The APK's boot: the unprefixed contract, no role in sight.
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("boot.fun".to_string(), BOOT.to_string())],
            Box::new(NativePlatform),
        )
        .expect("boot scene loads");
        assert_eq!(game.names.tick, "tick");

        // `POST /load-project?module=Server`. The pushed file has NO top-level
        // contract, so reaching a loaded state at all proves the role resolved.
        let files = |probe: f64| vec![("game.fun".to_string(), role_src(probe))];
        crate::protocol::load_with_role(
            &mut game,
            Some(EntryRole::Module("Server".to_string())),
            |game| game.load_project(&files(1.0)),
        )
        .expect("the declared role boots on the push");
        assert_eq!(game.names.tick, "Server.tick");
        assert!(
            game.state_debug().contains("n: 7"),
            "{}",
            game.state_debug()
        );

        // A re-push re-resolves the same role and preserves the model.
        crate::protocol::reload_with_role(
            &mut game,
            Some(EntryRole::Module("Server".to_string())),
            |game| game.reload_project(&files(2.0)),
        )
        .expect("the re-push reloads");
        assert_eq!(game.names.tick, "Server.tick");
        assert_eq!(
            game.session
                .global("Server.probe")
                .expect("probe")
                .to_string(),
            "2",
            "the edit landed"
        );
        assert!(game.state_debug().contains("n: 7"), "model preserved");

        // A push that DELETES the block fails loudly naming it; the old
        // program AND its role keep running, so the next good push works.
        let err = crate::protocol::reload_with_role(
            &mut game,
            Some(EntryRole::Module("Server".to_string())),
            |game| game.reload_project(&[("game.fun".to_string(), "let unrelated = 1.0\n".into())]),
        )
        .expect_err("a role whose block vanished cannot load");
        assert!(err.contains("module Server"), "{err}");
        assert_eq!(game.names.tick, "Server.tick", "the role is intact");
        assert_eq!(
            game.session
                .global("Server.probe")
                .expect("probe")
                .to_string(),
            "2",
            "the old program keeps running"
        );
        crate::protocol::reload_with_role(&mut game, None, |game| game.reload_project(&files(3.0)))
            .expect("a role-less push runs the role already in force");
        assert_eq!(game.names.tick, "Server.tick");

        // A DIFFERENT role on the model-preserving route is refused: adopting
        // it would hand Server's model to the plain contract's `tick`. The
        // error names the route that does start a new game.
        let err = crate::protocol::reload_with_role(
            &mut game,
            Some(EntryRole::Prefix(String::new())),
            |game| game.reload_project(&files(4.0)),
        )
        .expect_err("a role CHANGE is not a model-preserving reload");
        assert!(err.contains("/load-project"), "{err}");
        assert_eq!(game.names.tick, "Server.tick", "the role is untouched");

        // The same change IS a load — a new game, its model from `init`.
        crate::protocol::load_with_role(
            &mut game,
            Some(EntryRole::Prefix(String::new())),
            |game| game.load_project(&[("boot.fun".to_string(), BOOT.to_string())]),
        )
        .expect("a role change loads as a new game");
        assert_eq!(game.names.tick, "tick");
    }

    /// A prefixed role's EFFECTS must fold through the role's own update
    /// (`serverUpdate`), not the canonical `update` — the drain would
    /// otherwise fail its call and silently drop every message.
    #[test]
    fn a_prefixed_role_drains_its_effects_through_its_own_update() {
        let source = "\
type Msg = | GotTime(t: Float)\n\
let serverInit = { ticks: 0.0, stamped: 0.0 }\n\
let serverUpdate = (m, msg) =>\n\
  match msg with\n\
  | GotTime(t) => { m with stamped: m.stamped + 1.0 }\n\
let serverTick = (m, dt, tts) =>\n\
  ({ m with ticks: m.ticks + dt }, Effect.now((t) => GotTime(t)))\n\
let serverDraw = (m, tts) =>\n\
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n";
        let mut game = FunctorLangEmbeddedGame::create_for_role(
            vec![("game.fun".to_string(), source.to_string())],
            EntryRole::Prefix("server".to_string()),
            Box::new(NativePlatform),
        )
        .expect("prefixed role loads");
        game.tick(frame_time(0.016, 0.016));
        let model = game.state_debug();
        assert!(
            model.contains("stamped: 1"),
            "serverTick's Effect.now result reached serverUpdate: {model}"
        );
    }

    #[test]
    fn loading_a_new_project_initializes_its_own_model() {
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("boot.fun".to_string(), BOOT.to_string())],
            Box::new(NativePlatform),
        )
        .expect("boot scene loads");
        game.tick(frame_time(0.016, 0.016));

        let project = BOOT
            .replace("spin:", "speed:")
            .replace("model.spin", "model.speed");
        let status = game
            .load_project(&[("game.fun".to_string(), project)])
            .expect("new project loads");

        assert!(status.contains("model initialized"), "{status}");
        let model = game.state_debug();
        assert!(model.contains("speed"), "{model}");
        assert!(!model.contains("spin"), "{model}");
        let ft = frame_time(0.032, 0.016);
        game.tick(ft.clone());
        let frame = game.render(ft);
        assert!(
            !matches!(&frame.scene.obj, crate::SceneObject::Group(children) if children.is_empty()),
            "new project renders with its initialized model"
        );
    }

    #[test]
    fn sampled_input_is_typed_delivered_and_recorded_per_tick() {
        let source = r#"
let init = {
  trigger: 0.0,
  held: 0.0,
  pressed: 0.0,
  released: 0.0,
  mousePressed: false,
  mouseReleased: false,
  mouseX: 0.0
}
let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.Some(xr) => {
      trigger: xr.right.trigger,
      held: List.length(snapshot.heldKeys),
      pressed: List.length(snapshot.pressedKeys),
      released: List.length(snapshot.releasedKeys),
      mousePressed: snapshot.mouse.pressed.left,
      mouseReleased: snapshot.mouse.released.right,
      mouseX: snapshot.mouse.x
    }
  | Option.None => model
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), source.to_string())],
            Box::new(NativePlatform),
        )
        .expect("sampled-input game loads");
        assert!(game.samples_input());

        let snapshot = crate::InputSnapshot {
            held_keys: vec![crate::Key::W, crate::Key::Space],
            pressed_keys: vec![crate::Key::Space],
            released_keys: vec![crate::Key::Enter],
            mouse: crate::MouseSnapshot {
                x: 42,
                y: 9,
                pressed: crate::MouseButtons {
                    left: true,
                    ..crate::MouseButtons::default()
                },
                released: crate::MouseButtons {
                    right: true,
                    ..crate::MouseButtons::default()
                },
                ..Default::default()
            },
            xr: Some(crate::XrInputSnapshot {
                right: crate::XrControllerSnapshot {
                    active: true,
                    trigger: 0.625,
                    ..crate::XrControllerSnapshot::default()
                },
                ..crate::XrInputSnapshot::default()
            }),
            ..crate::InputSnapshot::default()
        };
        game.sampled_input(&snapshot);
        game.tick(frame_time(1.0 / 60.0, 1.0 / 60.0));

        let model = game.state_debug();
        assert!(model.contains("trigger: 0.625"), "{model}");
        assert!(model.contains("held: 2"), "{model}");
        assert!(model.contains("pressed: 1"), "{model}");
        assert!(model.contains("released: 1"), "{model}");
        assert!(model.contains("mousePressed: true"), "{model}");
        assert!(model.contains("mouseReleased: true"), "{model}");
        assert!(model.contains("mouseX: 42"), "{model}");
        assert!(matches!(
            game.recorded_inputs_at(0).as_slice(),
            [crate::RecordedInput::Snapshot(recorded)] if recorded.as_ref() == &snapshot
        ));
    }

    #[test]
    fn sampled_input_is_applied_after_a_resuming_scrub_restores_the_model() {
        let source = r#"
let init = { sample: 0, ticks: 0.0 }
let sampledInput = (model, snapshot: Input.snapshot) =>
  { sample: snapshot.mouse.x, ticks: model.ticks }
let tick = (model, dt, tts) =>
  { sample: model.sample, ticks: model.ticks + 1.0 }
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), source.to_string())],
            Box::new(NativePlatform),
        )
        .expect("sampled-input game loads");
        let snapshot = |x| crate::InputSnapshot {
            mouse: crate::MouseSnapshot {
                x,
                y: 0,
                ..Default::default()
            },
            ..crate::InputSnapshot::default()
        };

        game.sampled_input(&snapshot(1));
        game.tick(frame_time(1.0 / 60.0, 1.0 / 60.0));
        game.sampled_input(&snapshot(2));
        game.tick(frame_time(2.0 / 60.0, 1.0 / 60.0));
        game.seek_scene_to(0).expect("frame zero remains seekable");

        // The queued live sample belongs to the new branch. It must land after
        // Resume restores frame zero, or that restoration silently erases it.
        game.sampled_input(&snapshot(10));
        game.tick(frame_time(2.0 / 60.0, 1.0 / 60.0));

        let model = game.state_debug();
        assert!(model.contains("sample: 10"), "{model}");
        assert!(model.contains("ticks: 2"), "{model}");
        assert!(matches!(
            game.recorded_inputs_at(1).as_slice(),
            [crate::RecordedInput::Snapshot(recorded)] if recorded.mouse.x == 10
        ));
    }

    #[test]
    fn reload_that_adds_sampled_input_keeps_selected_snapshot_semantics() {
        let old = r#"
let init = { n: 0.0, gate: 0.0 }
let tick = (model, dt, tts) =>
  { n: model.n + 1.0, gate: model.gate }
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let new = r#"
let init = { n: 0.0, gate: 0.0 }
let sampledInput = (model, snapshot: Input.snapshot) =>
  { n: model.n, gate: 1.0 }
let tick = (model, dt, tts) =>
  { n: model.n + model.gate, gate: model.gate }
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), old.to_string())],
            Box::new(NativePlatform),
        )
        .expect("edge-only game loads");
        for frame in 1..=3 {
            game.tick(frame_time(frame as f32 / 60.0, 1.0 / 60.0));
        }
        game.seek_scene_to(0).expect("frame zero is seekable");

        let status = game.reload_source(new).expect("sampled hook reloads");
        let model = game.state_debug();
        assert!(model.contains("n: 1"), "{model}");
        assert!(model.contains("gate: 0"), "{model}");
        assert!(!status.contains("history recomputed"), "{status}");

        // The missing historical coeffects are a property of the retained
        // timeline, not just this code swap. A second edit while still
        // scrubbed must not re-enable origin replay.
        let status = game.reload_source(new).expect("second reload succeeds");
        let model = game.state_debug();
        assert!(model.contains("n: 1"), "{model}");
        assert!(model.contains("gate: 0"), "{model}");
        assert!(!status.contains("history recomputed"), "{status}");

        // Recording a sampled frame does not fill the older gap. Seeking back
        // across that gap and reloading must keep selected-snapshot semantics.
        game.sampled_input(&crate::InputSnapshot::default());
        game.tick(frame_time(4.0 / 60.0, 1.0 / 60.0));
        game.seek_scene_to(0).expect("frame zero remains seekable");
        let status = game.reload_source(new).expect("later reload succeeds");
        let model = game.state_debug();
        assert!(model.contains("n: 1"), "{model}");
        assert!(model.contains("gate: 0"), "{model}");
        assert!(!status.contains("history recomputed"), "{status}");
    }

    #[test]
    fn removing_and_readding_sampled_input_reuses_complete_coeffects() {
        let sampled = r#"
let init = { sum: 0.0 }
let sampledInput = (model, snapshot: Input.snapshot) =>
  { sum: model.sum + snapshot.mouse.x }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let without_sampled = r#"
let init = { sum: 0.0 }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), sampled.to_string())],
            Box::new(NativePlatform),
        )
        .expect("sampled-input game loads");
        for (frame, x) in [1, 2, 3].into_iter().enumerate() {
            game.sampled_input(&crate::InputSnapshot {
                mouse: crate::MouseSnapshot {
                    x,
                    y: 0,
                    ..Default::default()
                },
                ..crate::InputSnapshot::default()
            });
            game.tick(frame_time((frame + 1) as f32 / 60.0, 1.0 / 60.0));
        }
        game.seek_scene_to(0).expect("frame zero is seekable");

        let removed = game
            .reload_source(without_sampled)
            .expect("removing the hook reloads");
        assert!(removed.contains("history recomputed"), "{removed}");
        let readded = game
            .reload_source(sampled)
            .expect("re-adding the hook reloads");
        assert!(readded.contains("history recomputed"), "{readded}");

        game.seek_scene_to(2)
            .expect("rebuilt future remains seekable");
        let model = game.state_debug();
        assert!(model.contains("sum: 6"), "{model}");
    }

    #[test]
    fn adding_sampled_input_before_the_first_step_keeps_origin_complete() {
        let old = r#"
let init = { sum: 0.0 }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let sampled = r#"
let init = { sum: 0.0 }
let sampledInput = (model, snapshot: Input.snapshot) =>
  { sum: model.sum + snapshot.mouse.x }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), old.to_string())],
            Box::new(NativePlatform),
        )
        .expect("edge-only game loads");
        game.reload_source(sampled)
            .expect("hook added before the first step");

        for (frame, x) in [1, 2].into_iter().enumerate() {
            game.sampled_input(&crate::InputSnapshot {
                mouse: crate::MouseSnapshot {
                    x,
                    y: 0,
                    ..Default::default()
                },
                ..crate::InputSnapshot::default()
            });
            game.tick(frame_time((frame + 1) as f32 / 60.0, 1.0 / 60.0));
        }
        game.seek_scene_to(0).expect("frame zero is seekable");
        let status = game.reload_source(sampled).expect("later reload succeeds");
        assert!(status.contains("history recomputed"), "{status}");
        game.seek_scene_to(1)
            .expect("rebuilt future remains seekable");
        let model = game.state_debug();
        assert!(model.contains("sum: 3"), "{model}");
    }

    #[test]
    fn rewinding_before_a_sample_gap_restores_origin_replay() {
        let sampled = r#"
let init = { sum: 0.0 }
let sampledInput = (model, snapshot: Input.snapshot) =>
  { sum: model.sum + snapshot.mouse.x }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let without_sampled = r#"
let init = { sum: 0.0 }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
"#;
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), sampled.to_string())],
            Box::new(NativePlatform),
        )
        .expect("sampled-input game loads");
        for (frame, x) in [1, 2, 3].into_iter().enumerate() {
            game.sampled_input(&crate::InputSnapshot {
                mouse: crate::MouseSnapshot {
                    x,
                    y: 0,
                    ..Default::default()
                },
                ..crate::InputSnapshot::default()
            });
            game.tick(frame_time((frame + 1) as f32 / 60.0, 1.0 / 60.0));
        }

        game.reload_source(without_sampled)
            .expect("removing the hook reloads");
        game.tick(frame_time(4.0 / 60.0, 1.0 / 60.0));
        game.reload_source(sampled)
            .expect("re-adding after the gap reloads");

        game.rewind_scene_to(2)
            .expect("rewind discards the unsampled frame");
        game.seek_scene_to(0).expect("frame zero is seekable");
        let status = game
            .reload_source(sampled)
            .expect("same-hook reload revalidates the current branch");
        assert!(status.contains("history recomputed"), "{status}");
        game.seek_scene_to(2)
            .expect("rebuilt future remains seekable");
        let model = game.state_debug();
        assert!(model.contains("sum: 6"), "{model}");
    }
}
