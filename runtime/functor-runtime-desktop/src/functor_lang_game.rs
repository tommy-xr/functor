//! The Functor Lang producer (docs/functor-lang.md Track C2): game logic written in `.fun`,
//! run by the real interpreter (`functor_lang::Session`) with the Functor prelude
//! (`Scene.*` / `Camera.*` / `Frame.*` — see
//! `functor_runtime_common::functor_lang_prelude`). This replaces the Milestone-0
//! throwaway spike (`functor_lang_spike.rs`, deleted with this producer's arrival).
//!
//! Game contract (see the `functor-lang` skill and `examples/hello-cubes`):
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
//! The model is a plain Functor Lang value the host holds between frames — the
//! serializable-state seam hot-reload (C3) will swap sessions around.
//! Per-frame errors print and keep the previous model/frame (a bad frame
//! must not kill the session); load errors fail loud at startup.

use std::time::Instant;

use functor_runtime_common::functor_lang_prelude::{
    audio_scene_of, clear_audio_completions, clear_http_taggers, frame_value, take_ui_handlers,
    view_value, EffectLog, EffectRunner, EffectTree, FunctorHost, NetEventKind, RealEffects,
    UiHandler,
};
use functor_runtime_common::events::{self, RuntimeEvent};
use functor_runtime_common::functor_lang_producer::{
    journal_arm, journal_push, journal_swap, FrameCtx, JournalEntry, Provenance, Reporter,
    SpanSource,
};
use functor_runtime_common::physics;
use functor_runtime_common::timetravel::SceneRecorder;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use functor_lang::project::SourceMap;
use functor_lang::{Session, Span, Value};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::game::Game;

pub struct FunctorLangGame {
    path: String,
    /// Per-file mtimes of the WHOLE project (every sibling `.fun` — B8:
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
    module: functor_lang::ir::Module,
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
    /// The interactive-widget handler table registered by the `ui(model)`
    /// evaluation that built `last_view` (docs/ui-interaction.md U2): a
    /// `UiEvent` the shell reports resolves its slot here. Kept in lockstep
    /// with `last_view` (updated only on a successful evaluation) and cleared
    /// on reload — its values may close over the old session.
    ui_handlers: Vec<UiHandler>,
    /// The last serialized soundscape (`soundScape model` → JSON), cached
    /// because `audio_scene_json` is a `&self` accessor — evaluated beside
    /// `draw` each frame so errors can `&mut`-dedupe. A bad frame keeps the
    /// last good scene; a reload that drops the hook resets it to silence.
    last_soundscape_json: String,
    /// Performs `Effect.*` commands — the real world in the runner; the
    /// drain logic itself is `functor_lang_prelude::drain_effects` (tested there
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
    /// recorder + fixed-step accumulator. The World stays in the registry;
    /// this owns the rewind machinery over it (driven by the shell scrubber).
    physics_rt: physics::SteppedPhysics,
    /// The physics world's fixed frame after the latest advance — what the
    /// coupled scene recorder stores per rendered frame.
    physics_frame: u64,
    /// The coupled time-travel recorder (docs/time-travel.md T1–T3): records the
    /// settled `model` + physics fixed-frame each rendered frame and seeks/
    /// rewinds them together. Shared with the web producer (one tested impl).
    recorder: SceneRecorder,
    /// This frame's buffered input events (docs/time-travel.md T6b): appended in
    /// `key_event`/`mouse_move`/`mouse_wheel` beside the live `session.call`, and
    /// flushed into `recorder`'s input log by `record_frame` (plain data, so the
    /// log survives a hot reload).
    input_buf: Vec<functor_runtime_common::RecordedInput>,
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
    /// rendering) — shared with the web producer (`functor_lang_producer::Reporter`).
    reporter: Reporter,
    /// The last real frame's replay journal (visual-debugger PR2): one entry
    /// per model-updating call, swapped in from the thread-local journal at the
    /// end of each `tick`. A paused frame runs no `tick`, so this is preserved
    /// — the inspector reads the last real frame. `GET /trace` replays each
    /// entry through `Session::call_recorded` while paused.
    last_frame_journal: Vec<JournalEntry>,
    /// The lazily built + cached `/trace` JSON for the current paused frame.
    /// Invalidated when the frame advances (`tick`), the paused frame changes
    /// (rewind/seek), or the program reloads.
    cached_trace: Option<String>,
    /// Per-file sha256 of the loaded `.fun` source, computed at load /
    /// hot-reload (not per frame) — the wire contract's `sources`, and the
    /// per-file base→(file, local offset) map for binding spans.
    source_hashes: Vec<InspectorSource>,
    // rolling per-frame eval cost, printed every STATS_EVERY frames (the C6
    // perf gate watches these). Physics is engine cost, not Functor Lang eval cost, so
    // it gets its own counter — a heavy scene must not read as an interpreter
    // regression.
    frames: u64,
    tick_ns: u64,
    physics_ns: u64,
    draw_ns: u64,
    // GL cost the shell measures around its render/swap calls and folds back via
    // `record_gl_timing` — the render pass and the vsync-blocking buffer swap.
    render_ns: u64,
    swap_ns: u64,
}

const STATS_EVERY: u64 = 300;

/// Round a microsecond figure to one decimal, matching the old stats line's
/// `{:.1}` precision so the reported numbers stay tidy across both renderers.
fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// A successfully loaded, contract-validated game project.
struct Loaded {
    sources: SourceMap,
    module: functor_lang::ir::Module,
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

/// One project source file's inspector metadata (visual-debugger PR2): its
/// wire name, its base offset in the project-wide span space, its length, and
/// the sha256 of the text the runtime loaded. Computed once per load, so a
/// binding span maps to `(file, local offset)` and the wire `sources` gates on
/// a content hash without re-reading files.
struct InspectorSource {
    file: String,
    base: usize,
    len: usize,
    hash: String,
}

/// Build the per-file inspector metadata from a loaded [`SourceMap`]: the REAL
/// project `.fun` files only (skip the injected prelude `.funi` interfaces and
/// the `<builtin>/Net.fun` module — a game binding never lands in them, and
/// they aren't editor documents the LSP gates on).
fn inspector_sources(sources: &SourceMap) -> Vec<InspectorSource> {
    use sha2::{Digest, Sha256};
    sources
        .files()
        .iter()
        .filter(|f| !f.interface && !f.path.to_string_lossy().starts_with('<'))
        .map(|f| {
            let file = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.to_string_lossy().into_owned());
            let hash = format!("{:x}", Sha256::digest(f.src.as_bytes()));
            InspectorSource {
                file,
                base: f.base,
                len: f.src.len(),
                hash,
            }
        })
        .collect()
}

/// Load, check, and contract-validate a game project (B8: the entry plus
/// every sibling `.fun` file — file = module). Errors come back as fully
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
    let entry = std::path::Path::new(path);
    // A pushed buffer (network reload) stands in for the entry file; siblings
    // still load from disk.
    let overrides = match entry_src {
        Some(src) => std::collections::HashMap::from([(entry.to_path_buf(), src)]),
        None => std::collections::HashMap::new(),
    };
    // Inject the host prelude `.funi` interfaces so `Scene.*` (etc.) typecheck
    // against real types (docs/functor-lang-interfaces.md). Check-time only — the FunctorHost still
    // provides the actual runtime values.
    let project = functor_lang::project::load_with_prelude(entry, &overrides, &functor_prelude::modules())
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
    if functor_runtime_common::functor_lang_prelude::contains_effect(&init) {
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

/// Per-file mtimes of every `.fun` file in the entry's project, sorted by
/// path — the hot-reload change stamp. Any edited, added, or removed file
/// changes the stamp (a file we cannot stat contributes UNIX_EPOCH, so a
/// mid-save disappearing file still registers as a change).
/// The entry file's mtime within a stamp ([`project_files`] lists the
/// entry first).
fn entry_mtime(stamp: &[(PathBuf, SystemTime)]) -> Option<SystemTime> {
    stamp.first().map(|(_, mtime)| *mtime)
}

fn project_stamp(path: &str) -> Vec<(PathBuf, SystemTime)> {
    let files = functor_lang::project::project_files(std::path::Path::new(path)).unwrap_or_default();
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

impl FunctorLangGame {
    pub fn create(path: &str) -> FunctorLangGame {
        // Route Functor Lang `Debug.log` traces into the region-aware event stream (once
        // per process; survives hot-reload's Session rebuild — the sink is
        // installed on the process, not the Session). See functor_lang_prelude.
        functor_runtime_common::functor_lang_prelude::install_debug_log_sink();
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
        // Arm the paused-inspector journal on this (the render) thread: from now
        // on every live model-updating call is journaled (a cheap Rc-clone push).
        // The web producer never arms it — its shared frame body pays only a
        // `None` check. See `functor_lang_producer`.
        journal_arm();
        let source_hashes = inspector_sources(&loaded.sources);
        FunctorLangGame {
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
            ui_handlers: Vec::new(),
            last_soundscape_json: empty_soundscape_json(),
            effect_runner: RealEffects::new(),
            effect_log: EffectLog::new(),
            deferred_queries: Vec::new(),
            pending_events: Vec::new(),
            physics_rt: physics::SteppedPhysics::new(),
            physics_frame: 0,
            recorder: SceneRecorder::new(),
            input_buf: Vec::new(),
            live_conn_keys: std::collections::HashSet::new(),
            last_frame: empty_frame(),
            reporter: Reporter::new(SpanSource::Project(loaded.sources), report_to_stderr),
            last_frame_journal: Vec::new(),
            cached_trace: None,
            source_hashes,
            frames: 0,
            tick_ns: 0,
            physics_ns: 0,
            draw_ns: 0,
            render_ns: 0,
            swap_ns: 0,
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
            suppress_outbound: false,
            reporter: &mut self.reporter,
        }
    }

    /// Swap in a freshly loaded program, KEEPING THE MODEL — the shared tail
    /// of both reload paths (file watch and network push). `init` from the
    /// new program is deliberately unused: state survives the edit, and
    /// closures stored in the model rebind to the edited code (B5 part 2,
    /// `functor_lang::rebind_value`). The physics world is deliberately KEPT too, like
    /// the model: it lives in this process's registry, so bodies stay where
    /// they are across the edit and the next frame's declaration re-diffs
    /// against them (removing the `physics` hook drops the world). `prev_tts`
    /// is kept as well: `Sub.every` fires on the global time grid, so timers
    /// tick right through a reload. Returns the number of stored closures
    /// rebound, for the status line.
    fn swap_in(&mut self, loaded: Loaded) -> usize {
        let (model, report) = functor_lang::rebind_value(&self.model, &self.module, &loaded.module);
        self.model = model;
        for warning in &report.warnings {
            eprintln!("[functor-lang] reload: {warning}");
        }
        // Recompute the inspector source hashes for the edited files, and drop
        // the journal + cached trace: they refer to the OLD program's spans and
        // execution (visual-debugger PR2 — hot-reload clears both).
        self.source_hashes = inspector_sources(&loaded.sources);
        self.last_frame_journal.clear();
        self.cached_trace = None;
        journal_swap(); // discard any partial current-frame journal
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
        // The widget handler table holds msgs/taggers into the OLD session;
        // the next render's `ui(model)` rebuilds it against the new one. A
        // click landing in the gap resolves an unknown slot and is dropped.
        self.ui_handlers.clear();
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
            let render_us = self.render_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let swap_us = self.swap_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let frame_us = tick_us + physics_us + draw_us;
            let counters = functor_runtime_common::gpu_counters::gpu_counters();
            let live = counters.live();
            let window = counters.take_window();
            events::emit(RuntimeEvent::FrameStats {
                tick_us: round1(tick_us),
                draw_us: round1(draw_us),
                render_us: round1(render_us),
                swap_us: round1(swap_us),
                frame_us: round1(frame_us),
                budget_pct: round1(frame_us / 16_666.0 * 100.0),
                over_n_frames: STATS_EVERY as u32,
                gpu_live_vaos: live.vaos,
                gpu_live_buffers: live.buffers,
                gpu_live_textures: live.textures,
                gpu_bytes_per_frame: round1(window.bytes_uploaded as f64 / STATS_EVERY as f64),
                gpu_cache_hits: window.cache_hits,
                gpu_cache_misses: window.cache_misses,
            });
            self.tick_ns = 0;
            self.physics_ns = 0;
            self.draw_ns = 0;
            self.render_ns = 0;
            self.swap_ns = 0;
        }
    }

    /// Map a project-wide binding span to `(file, local start, local end)` for
    /// the wire contract, using the per-file base offsets (visual-debugger PR2).
    /// `None` if the span falls outside a real project file (a prelude/builtin
    /// span — a game binding never does).
    fn span_to_file(&self, span: Span) -> Option<(String, usize, usize)> {
        let src = self
            .source_hashes
            .iter()
            .filter(|s| s.base <= span.start)
            .max_by_key(|s| s.base)?;
        if span.start > src.base + src.len {
            return None;
        }
        Some((
            src.file.clone(),
            span.start - src.base,
            span.end.saturating_sub(src.base),
        ))
    }

    /// Replay the last real frame's journal into the wire-contract
    /// `invocations` (visual-debugger PR2). Each journaled call is re-run through
    /// `Session::call_recorded` — entry points are pure functions of their args,
    /// so the record is exact, and effects are plain data (nothing performs), so
    /// replay is side-effect-free. `index`/`count` frame each call within its
    /// entry name; binding spans map to `(file, local offsets)`.
    fn build_invocations(&self) -> Vec<serde_json::Value> {
        use std::collections::HashMap;
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for e in &self.last_frame_journal {
            *counts.entry(e.entry).or_default() += 1;
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut out = Vec::with_capacity(self.last_frame_journal.len());
        for e in &self.last_frame_journal {
            let index = {
                let c = seen.entry(e.entry).or_default();
                let i = *c;
                *c += 1;
                i
            };
            // A call that succeeded live, replayed pure, should not fail — skip
            // it defensively rather than abort the whole trace if it somehow does.
            let (_discard, inv) =
                match self
                    .session
                    .call_recorded(e.entry, e.args.clone(), &mut FunctorHost)
                {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };
            let bindings: Vec<serde_json::Value> = inv
                .bindings
                .iter()
                .filter_map(|b| {
                    self.span_to_file(b.span).map(|(file, start, end)| {
                        serde_json::json!({
                            "name": b.name,
                            "file": file,
                            "start": start,
                            "end": end,
                            "value": b.value,
                            "count": b.count,
                        })
                    })
                })
                .collect();
            out.push(serde_json::json!({
                "entry": e.entry,
                "index": index,
                "count": counts[e.entry],
                "provenance": e.provenance.render(&e.args),
                "ghost": false,
                "result": inv.result,
                "truncated": inv.truncated,
                "bindings": bindings,
            }));
        }
        out
    }

    /// The wire `sources` array — `{ file, hash }` per loaded project file.
    fn sources_json(&self) -> Vec<serde_json::Value> {
        self.source_hashes
            .iter()
            .map(|s| serde_json::json!({ "file": s.file, "hash": s.hash }))
            .collect()
    }
}

impl Game for FunctorLangGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        // Poll every project file's mtime (a few stats per frame is ~free)
        // and swap in a new session on change — editing a SIBLING module
        // hot-reloads exactly like editing the entry (B8). THE MODEL IS
        // KEPT: it is a plain value the host holds, so state survives the
        // edit and all functions rebind — the dev-loop payoff the language
        // was built for (docs/functor-lang.md C3). Closures STORED IN THE MODEL
        // rebind too (B5 part 2, `functor_lang::rebind`): they adopt the edited code
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
                events::emit(RuntimeEvent::HotReload {
                    ok: true,
                    message: format!(
                        "hot-reloaded {} in {:.2}ms (model preserved{stored}; an edited \
`init` takes effect on restart)",
                        self.path,
                        started.elapsed().as_secs_f64() * 1000.0
                    ),
                });
            }
            Err(message) => {
                events::emit(RuntimeEvent::HotReload {
                    ok: false,
                    message: format!("reload failed, keeping old program: {message}"),
                });
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
        events::emit(RuntimeEvent::HotReload {
            ok: true,
            message: status.clone(),
        });
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
            &mut self.physics_frame,
            self.has_physics,
        );
        if result.is_ok() {
            // No in-flight frame work should carry across the branch (matches
            // the reload discipline); between-frame callers have these empty.
            self.deferred_queries.clear();
            self.pending_events.clear();
            clear_http_taggers();
            clear_audio_completions();
            // The paused frame moved to `target`, for which we hold no journal
            // (only the last real frame's) — drop the stale trace so the
            // inspector reports the rewound frame with no invocations (PR2).
            self.last_frame_journal.clear();
            self.cached_trace = None;
            // Drop any input buffered since the last recorded frame: the model was
            // just restored to `target`, so a stray live event (e.g. one buffered
            // on a 0-substep frame under the fixed-timestep loop) is now orphaned
            // and must not be recorded into the branch — it would diverge a
            // ghost/replay taken there (xreview).
            self.input_buf.clear();
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

    /// Forward-ghosting (docs/time-travel.md T6d): step the scene forward over a
    /// window of `divisions` divisions, each `dt` wide, from `start_tts` (a dry
    /// run over throwaway state — the live producer is untouched), then `draw`
    /// each stepped model at its division-boundary time and return the frames for
    /// the shell to composite. To keep velocity-integrated motion (mario's jump)
    /// faithful, each division is advanced in FINE `sub_dt = 1/60` sub-steps
    /// (`steps_per_division ≈ dt / sub_dt`) and sampled only at the boundary, so
    /// the strobe still has `divisions` frames but each is accurate integration.
    /// Division `div` draws at `tts = start_tts + (div+1)*steps_per_division*sub_dt`,
    /// matching the time `forward_step_scene` stepped the model to (the same f32
    /// arithmetic). Each frame's camera is overridden to the paused view
    /// (`last_frame.camera`) so only world motion smears. A draw that errors or
    /// doesn't return a Frame is skipped, so the result may be shorter than
    /// `divisions`.
    ///
    /// `script_inputs` selects the input source (docs/time-travel.md F2). When
    /// `Some`, the ghost forward-steps from `self.model` (the live anchor — K is
    /// NOT resolved from the recorder) replaying the caller-supplied SCRIPT slice,
    /// so the strobe is the *scripted* trajectory under the current code. When
    /// `None`, the T6d behavior: resolve K and replay the recorder's own log.
    fn ghost_frames(
        &self,
        divisions: usize,
        dt: f32,
        start_tts: f64,
        script_inputs: Option<&[Vec<functor_runtime_common::RecordedInput>]>,
    ) -> Vec<(Frame, FrameTime)> {
        // The body is shared (`functor_lang_producer::ghost_frames`) so both shells ghost
        // through one impl; this just hands it the producer's state.
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

    /// Non-destructive scrub — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T3): restore model + world for display without
    /// truncating, so the draggable bar can seek back and forth.
    fn seek_scene_to(&mut self, target: u64) -> Result<String, String> {
        let result = self.recorder.seek_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_frame,
            self.has_physics,
        );
        if result.is_ok() {
            // The scrubbed frame changed: drop the stale journal + cached trace
            // (we hold a journal only for the last real frame, not the scrubbed
            // target) — PR2, like `rewind_scene_to`.
            self.last_frame_journal.clear();
            self.cached_trace = None;
            // The model was restored to `target`; drop any input buffered since
            // the last recorded frame so a stray live event (buffered on a
            // 0-substep frame under the fixed-timestep loop) can't be recorded
            // into the resulting branch and diverge a ghost/replay (xreview).
            self.input_buf.clear();
        }
        result
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
        // A real frame ran: swap its journal into `last_frame_journal` (leaving
        // a fresh armed journal) and drop the cached trace (the frame advanced).
        // A paused frame never reaches here, so its last real frame is kept.
        if let Some(journal) = journal_swap() {
            self.last_frame_journal = journal;
        }
        self.cached_trace = None;
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
            Value::String(std::rc::Rc::from(key.name().as_str())),
            Value::Bool(is_down),
        ];
        journal_push("input", &args, Provenance::Input);
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
        journal_push("mouseMove", &args, Provenance::MouseMove);
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
        journal_push("mouseWheel", &args, Provenance::MouseWheel);
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
                    "[functor-lang] draw must return Frame.create(camera, scene), got {}",
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
                        "[functor-lang] soundScape must return an AudioScene (AudioScene.create / \
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

    fn record_gl_timing(&mut self, render_ns: u64, swap_ns: u64) {
        // The shell measured this frame's GL render + swap (it owns the GL
        // calls); fold them into the same rolling window `report_stats` averages
        // — same one-frame lag as `draw_ns`, which is likewise accumulated after
        // `tick` runs `report_stats`.
        self.render_ns += render_ns;
        self.swap_ns += swap_ns;
    }

    fn ui(&self) -> View {
        self.last_view.clone()
    }

    fn state_debug(&self) -> String {
        self.model.to_string()
    }

    /// The paused-inspector trace (visual-debugger PR2). When NOT paused, a
    /// cheap early-out: `paused: false` with empty invocations (frame/tts/
    /// sources still filled). When paused, lazily replay the last real frame's
    /// journal into the wire-contract `invocations` and CACHE the result until
    /// the frame advances (`tick`), the scrubbed frame changes, or the program
    /// reloads. `paused` is the shell's clock state (`GameClock::is_paused`).
    fn inspector_trace(&mut self, paused: bool) -> String {
        let frame = self.recorder.current_scene_frame().unwrap_or(0);
        let tts = self.recorder.current_scene_frame_tts().unwrap_or(0.0);
        if !paused {
            return serde_json::json!({
                "frame": frame,
                "tts": tts,
                "paused": false,
                "sources": self.sources_json(),
                "invocations": [],
            })
            .to_string();
        }
        if let Some(cached) = &self.cached_trace {
            return cached.clone();
        }
        let json = serde_json::json!({
            "frame": frame,
            "tts": tts,
            "paused": true,
            "sources": self.sources_json(),
            "invocations": self.build_invocations(),
        })
        .to_string();
        self.cached_trace = Some(json.clone());
        json
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
    /// live `FunctorLangGame` for the wsdemo port: declaring `Sub.connect`
    /// reconciles into a `Connect` command; a `Connected` event routes
    /// through the tagger → `update`, storing the id and replying with
    /// `Effect.send`; a `Message` event lands in the model.
    #[test]
    fn websocket_connect_send_receive() {
        let _guard = NET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use functor_runtime_common::net::{drain_conn_commands, ConnCommand};
        const ENDPOINT: &str = "ws://127.0.0.1:9001/echo";
        let dir = std::env::temp_dir().join(format!("functor-lang-net-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("game.fun"),
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

        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().unwrap());

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
        let dir = std::env::temp_dir().join(format!("functor-lang-net-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("game.fun"),
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
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().unwrap());

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

    /// Write `src` as `game.fun` in its own temp directory (a directory is
    /// a whole project since B8 — a shared temp dir would drag stray `.fun`
    /// files in as sibling modules) and return `load_game`'s error.
    fn load_err(name: &str, src: &str) -> String {
        let dir = std::env::temp_dir().join(format!("functor-lang-game-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let path = dir.join("game.fun");
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
    /// `config.fun` must reload around the pushed `game.fun`, and only an
    /// on-disk edit of the entry itself reverts to disk (last-write-wins,
    /// per file). [Codex Medium — B8 review]
    #[test]
    fn pushed_entry_survives_sibling_reloads() {
        let dir = std::env::temp_dir().join(format!("functor-lang-game-test-{}-push", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let entry = dir.join("game.fun");
        let disk_game = format!("{BASE}let probe = 1.0\n");
        std::fs::write(&entry, &disk_game).expect("write entry");
        std::fs::write(dir.join("config.fun"), "let k = 1.0\n").expect("write sibling");
        let mut game = FunctorLangGame::create(entry.to_str().expect("utf-8 path"));

        // Push an entry variant distinguishable from the disk one.
        let pushed = format!("{BASE}let probe = 2.0\n");
        game.reload_source(&pushed).expect("push should load");
        assert_eq!(
            game.session.global("probe").expect("probe").to_string(),
            "2"
        );

        // Edit the SIBLING: the reload must keep the pushed entry.
        std::thread::sleep(std::time::Duration::from_millis(20)); // distinct mtime
        std::fs::write(dir.join("config.fun"), "let k = 5.0\n").expect("edit sibling");
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
            std::env::temp_dir().join(format!("functor-lang-game-test-{}-history", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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
            .join(format!("functor-lang-game-test-{}-history-reload", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        let src = "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n";
        std::fs::write(dir.join("game.fun"), src).expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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
            .join(format!("functor-lang-game-test-{}-coupled", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.fixed(\"ground\", Physics.box(20.0, 0.4, 20.0)) |> Physics.at(0.0, -0.2, 0.0),\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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
    /// driven by the render clock `tts` (like `examples/lighting`'s orbiting
    /// lights) must, WHILE SCRUBBING, render at the scrubbed frame's RECORDED
    /// tts — not the live "now" clock. Here the camera eye tracks tts, so the
    /// returned `Frame` exposes which tts `draw` actually ran at. Exercises the
    /// real production render path (`render` → `current_scene_tts` override).
    #[test]
    fn scrubbed_frame_renders_at_its_recorded_tts() {
        physics::remove_world(physics::DEFAULT_WORLD);
        let dir =
            std::env::temp_dir().join(format!("functor-lang-game-test-{}-tts", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        // eye.x == tts, so the drawn Frame reveals the tts `draw` ran at.
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(tts, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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
        let dir = std::env::temp_dir().join(format!("functor-lang-game-test-{}-scrub", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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
            std::env::temp_dir().join(format!("functor-lang-game-test-{}-latest", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => { m with n: m.n + 1.0 }\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 8.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

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

    /// The deterministic SUB-STEPPED headless forward-step (docs/time-travel.md
    /// T6b / F1): from a fork point, `forward_step_scene` steps the whole scene
    /// forward at a fine `sub_dt = 1/60` (`STEPS_PER_DIV` sub-ticks per division)
    /// and its DIVISION-BOUNDARY snapshots reproduce EXACTLY the sequence a fresh
    /// 1/60 live continuation produces — model (`Value` via `to_string`) and
    /// physics world (snapshot bytes) both byte-equal — WITHOUT touching the live
    /// producer state. The game is pure (no `Now` / unseeded `Random`): a ball
    /// falls onto a slab and a contact counter folds through `update`, so both
    /// the model and the world genuinely evolve and stay coupled. A game reading
    /// wall-clock `Now` / unseeded `Random` would NOT match — the determinism
    /// boundary; a `tts`-driven / seeded game does, since the forward-step
    /// supplies `tts`.
    #[test]
    fn forward_step_is_deterministic_and_non_destructive() {
        // The physics registry is a per-thread thread-local shared by every
        // physics test on this thread — start from an empty world.
        physics::remove_world(physics::DEFAULT_WORLD);

        let dir = std::env::temp_dir().join(format!("functor-lang-fwd-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
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
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

        // Fine sub-step at 1/60 (= FIXED_DT, so each fine tick is one physics
        // step); the forward-step snapshots every STEPS_PER_DIV fine ticks.
        const SUB_DT: f32 = physics::FIXED_DT;
        const K: usize = 45;
        const DIVISIONS: usize = 5;
        const STEPS_PER_DIV: usize = 5;
        const N: usize = DIVISIONS * STEPS_PER_DIV; // total fine ticks in the window

        // Drive K frames to the fork point.
        let mut tts = 0.0f32;
        for _ in 0..K {
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
        }

        // Capture the fork state + a baseline of the live producer state.
        let fork_model = game.model.clone();
        let fork_prev_tts = game.prev_tts;
        let fork_tts = tts;
        let live_model_before = game.model.to_string();
        let live_world_before = physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot());
        let live_frame_before = game.physics_frame;

        // Forward-step DIVISIONS divisions (STEPS_PER_DIV fine ticks each) from
        // the fork — a dry run over throwaway state.
        let forward = functor_runtime_common::functor_lang_producer::forward_step_scene(
            &game.session,
            &fork_model,
            game.has_physics,
            game.has_subscriptions,
            fork_prev_tts,
            fork_tts,
            SUB_DT,
            DIVISIONS,
            STEPS_PER_DIV,
            &[],
        );

        // The live producer state is UNCHANGED by the forward-step.
        assert_eq!(game.model.to_string(), live_model_before, "model untouched");
        assert_eq!(
            physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot()),
            live_world_before,
            "live world untouched"
        );
        assert_eq!(
            game.physics_frame, live_frame_before,
            "live fixed frame untouched"
        );

        // The live continuation at 1/60: the ground truth the sub-stepped
        // forward-step must match at its division boundaries.
        let mut live: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(N);
        for _ in 0..N {
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
            let world = physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot());
            live.push((game.model.to_string(), world));
        }

        assert_eq!(forward.len(), DIVISIONS, "division count");
        // The scene genuinely evolves: the world moves across the window and the
        // ball lands (a Contact folds `hits` up through `update`).
        assert_ne!(live[0].1, live[N - 1].1, "world should move over the window");
        assert!(
            game.model.to_string().contains("hits: ")
                && !game.model.to_string().contains("hits: 0"),
            "the ball should have landed within the window: {}",
            game.model.to_string()
        );
        // Each forward DIVISION-BOUNDARY snapshot matches the live 1/60 frame at
        // fine step (div+1)*STEPS_PER_DIV — proving the ARC is accurate at fine dt.
        for (div, (fwd_m, fwd_w)) in forward.iter().enumerate() {
            let live_idx = (div + 1) * STEPS_PER_DIV - 1;
            let (live_m, live_w) = &live[live_idx];
            assert_eq!(fwd_m.to_string(), *live_m, "model diverged at division {div}");
            assert_eq!(fwd_w, live_w, "world diverged at division {div}");
        }

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The frame-indexed input log payoff (docs/time-travel.md T6b): forward-
    /// stepping the REAL `examples/mario` game while REPLAYING its recorded input
    /// log reproduces a scripted jump exactly, and the projected character clears
    /// the chasm. Mario has no `physics` hook (`has_physics = false`), so the
    /// projection is exact — the whole state forward-steps in `tick`, driven only
    /// by the replayed inputs. This is the "record a jump, replay it forward"
    /// demo, runtime-verified headlessly.
    #[test]
    fn mario_forward_step_replays_recorded_jump_and_clears_chasm() {
        use functor_runtime_common::Key;

        fn field(v: &Value, name: &str) -> f64 {
            match v {
                Value::Record(fields) => match &fields.iter().find(|(k, _)| k == name).unwrap().1 {
                    Value::Number(x) => *x,
                    Value::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    other => panic!("{name} is not a number/bool: {other}"),
                },
                _ => panic!("model is not a record"),
            }
        }

        let mario = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/mario/game.fun");
        let mut game = FunctorLangGame::create(mario);
        assert!(!game.has_physics, "mario must have no physics hook");

        const SUB_DT: f32 = 1.0 / 60.0; // fine sub-step (one rendered frame)
        const K: usize = 10; // pre-jump fork point (still running left of the edge)
        const DIVISIONS: usize = 15;
        const STEPS_PER_DIV: usize = 5;
        const N: usize = DIVISIONS * STEPS_PER_DIV; // 75 fine frames: run to the edge, jump, land, run

        // Hold Right from the very first frame (a single key-down, then held).
        game.key_event(Key::Right as i32, true);

        // Phase 1: drive K frames of running to the fork point (pre-jump).
        let mut tts = 0.0f32;
        for _ in 0..K {
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
        }
        let fork_model = game.model.clone();
        let fork_prev_tts = game.prev_tts;
        let fork_tts = tts;
        // The fork is pre-jump: grounded, running, still well left of the chasm.
        assert_eq!(field(&fork_model, "grounded"), 1.0, "fork must be grounded");
        assert!(field(&fork_model, "x") < -3.0, "fork must be left of the edge");

        // Phase 2: the live continuation — run to the edge and JUMP at the last
        // grounded frame before walking off (the optimal launch), then land. This
        // fills the recorder's input log for frames K.. and is the ground truth.
        let mut live: Vec<(f64, f64)> = Vec::with_capacity(N);
        let mut jumped = false;
        for _ in 0..N {
            // Jump just before walking off the left platform (mirrors a player
            // timing the jump at the edge). One press, gated on grounded.
            if !jumped
                && field(&game.model, "grounded") == 1.0
                && field(&game.model, "x") + (8.0 * SUB_DT as f64) >= -3.0
            {
                game.key_event(Key::Up as i32, true);
                jumped = true;
            }
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
            live.push((field(&game.model, "x"), field(&game.model, "y")));
        }
        assert!(jumped, "the scripted jump must have fired");

        // The recorded input log for the continuation frames (K..), replayed by
        // the forward-step. `current_scene_frame` is the newest recorded frame
        // (K + N - 1); ghosting resolves K from it, so replay frames K.. .
        let inputs = game.recorder.inputs_from(K as u64);
        assert_eq!(inputs.len(), N, "one input entry per continuation frame");
        // The jump was recorded as a single Up key-down somewhere in the window.
        let up_events: usize = inputs
            .iter()
            .flatten()
            .filter(|e| matches!(e, functor_runtime_common::RecordedInput::Key { code, is_down }
                if *code == Key::Up as i32 && *is_down))
            .count();
        assert_eq!(up_events, 1, "exactly one recorded jump");

        // Forward-step from the PRE-JUMP fork, replaying the recorded inputs,
        // SUB-STEPPED at 1/60 (STEPS_PER_DIV fine ticks per division snapshot).
        let forward = functor_runtime_common::functor_lang_producer::forward_step_scene(
            &game.session,
            &fork_model,
            game.has_physics,
            game.has_subscriptions,
            fork_prev_tts,
            fork_tts,
            SUB_DT,
            DIVISIONS,
            STEPS_PER_DIV,
            &inputs,
        );
        assert_eq!(forward.len(), DIVISIONS, "division count");

        // (a) Each forward DIVISION-BOUNDARY snapshot matches the live 1/60 frame
        // at fine step (div+1)*STEPS_PER_DIV — the replayed jump reproduces the
        // recorded arc EXACTLY at fine dt (velocity-integrated jump projected
        // faithfully, not coarsely).
        for (div, (fwd_m, _w)) in forward.iter().enumerate() {
            let live_idx = (div + 1) * STEPS_PER_DIV - 1;
            let (lx, ly) = live[live_idx];
            assert!(
                (field(fwd_m, "x") - lx).abs() < 1e-9,
                "x diverged at division {div}: {} vs {lx}",
                field(fwd_m, "x")
            );
            assert!(
                (field(fwd_m, "y") - ly).abs() < 1e-9,
                "y diverged at division {div}: {} vs {ly}",
                field(fwd_m, "y")
            );
        }

        // (b) The character CLEARS the chasm: it lands on the RIGHT platform
        // (x past chasmHalf 3.0 and inside rightEdge 11.0), grounded — not fallen
        // into the gap and respawned.
        let (final_x, _final_y) = *live.last().unwrap();
        assert!(
            final_x > 3.0 && final_x < 11.0,
            "character should have cleared the chasm and landed on the right platform, final x = {final_x}"
        );
        assert_eq!(
            field(&game.model, "grounded"),
            1.0,
            "character should be grounded on the right platform"
        );
    }

    /// The INTERACTIVE scrubber+ghost path end-to-end (docs/time-travel.md T6d):
    /// record a live run+jump at the fixed 1/60 step, SCRUB back to a pre-jump
    /// frame, then resolve the fork exactly as `functor_lang_producer::ghost_frames` does —
    /// `k = current_scene_frame()`, `inputs_from(k + 1)`, forward-step from the
    /// scrubbed `model` — and assert the strobe's per-division models reproduce the
    /// RECORDED future exactly. Unlike `mario_forward_step_...` (which hand-picks a
    /// fork model + input slice) this exercises `seek_scene_to` +
    /// `current_scene_frame` + `inputs_from` integration — the alignment the live
    /// scrubber+ghost actually depends on.
    #[test]
    fn mario_interactive_ghost_from_scrubbed_frame_matches_recorded_future() {
        use functor_runtime_common::Key;

        fn field(v: &Value, name: &str) -> f64 {
            match v {
                Value::Record(fields) => match &fields.iter().find(|(k, _)| k == name).unwrap().1 {
                    Value::Number(x) => *x,
                    Value::Bool(b) => (*b as i32) as f64,
                    other => panic!("{name} is not a number/bool: {other}"),
                },
                _ => panic!("model is not a record"),
            }
        }

        let mario = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/mario/game.fun");
        let mut game = FunctorLangGame::create(mario);

        const SUB_DT: f32 = 1.0 / 60.0;
        const N: usize = 90; // run to the edge, jump, land, run on the right

        // Live play at the fixed step: hold Right, jump at the edge, recording the
        // settled (x, y) of every frame as ground truth. `game.tick` fills the
        // recorder exactly as the shell's fixed-timestep loop now does.
        game.key_event(Key::Right as i32, true);
        let mut recorded: Vec<(f64, f64)> = Vec::with_capacity(N);
        let mut tts = 0.0f32;
        let mut jumped = false;
        for _ in 0..N {
            if !jumped
                && field(&game.model, "grounded") == 1.0
                && field(&game.model, "x") + (8.0 * SUB_DT as f64) >= -3.0
            {
                game.key_event(Key::Up as i32, true);
                jumped = true;
            }
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
            recorded.push((field(&game.model, "x"), field(&game.model, "y")));
        }
        assert!(jumped, "the scripted jump must have fired");

        // Scrub back to a PRE-JUMP frame and confirm the scene sits on it.
        const S: u64 = 3;
        game.seek_scene_to(S).expect("seek to S");
        assert_eq!(game.current_scene_frame(), Some(S), "scrubbed to frame S");
        assert!(
            (field(&game.model, "x") - recorded[S as usize].0).abs() < 1e-9,
            "scrubbed model must be the recorded frame S"
        );
        assert_eq!(field(&game.model, "grounded"), 1.0, "S is pre-jump / grounded");

        // Resolve the fork EXACTLY as `ghost_frames(script_inputs = None)` does.
        let k = game.current_scene_frame().unwrap();
        let inputs = game.recorder.inputs_from(k + 1);
        let start_tts = game.recorder.current_scene_frame_tts().unwrap() as f32;

        const DIVISIONS: usize = 8;
        const STEPS_PER_DIV: usize = 5;
        let forward = functor_runtime_common::functor_lang_producer::forward_step_scene(
            &game.session,
            &game.model, // the scrubbed model = seek(k)
            game.has_physics,
            game.has_subscriptions,
            game.prev_tts,
            start_tts,
            SUB_DT,
            DIVISIONS,
            STEPS_PER_DIV,
            &inputs,
        );
        assert_eq!(forward.len(), DIVISIONS, "division count");

        // Each division boundary reproduces recorded frame S + (div+1)*STEPS — the
        // strobe retraces the true recorded arc (exact at fixed dt).
        let mut peak_y = f64::MIN;
        for (div, (m, _w)) in forward.iter().enumerate() {
            peak_y = peak_y.max(field(m, "y"));
            let f = S as usize + (div + 1) * STEPS_PER_DIV;
            if f >= N {
                break; // beyond the recorded window the step coasts; stop comparing
            }
            let (rx, ry) = recorded[f];
            assert!(
                (field(m, "x") - rx).abs() < 1e-9,
                "ghost x diverged at division {div} (frame {f}): {} vs {rx}",
                field(m, "x")
            );
            assert!(
                (field(m, "y") - ry).abs() < 1e-9,
                "ghost y diverged at division {div} (frame {f}): {} vs {ry}",
                field(m, "y")
            );
        }
        // The strobe visibly shows the JUMP: some division rose above the ground.
        assert!(peak_y > 0.5, "the ghost should show the jump arc, peak y = {peak_y}");
    }

    /// Regression (xreview): under the fixed-timestep loop a live input can be
    /// buffered on a 0-substep render frame — `key_event` buffers it, but no
    /// `tick` runs that frame, so `record_frame` never drains it. If the user
    /// then SCRUBS, the model is restored to the recorded frame while the buffered
    /// event is left orphaned; recording it into the resulting branch on resume
    /// would diverge a ghost/replay. `seek_scene_to` (and `rewind_scene_to`) must
    /// drop the buffer when they restore the model.
    #[test]
    fn scrub_drops_input_buffered_on_a_zero_substep_frame() {
        use functor_runtime_common::Key;

        let mario = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/mario/game.fun");
        let mut game = FunctorLangGame::create(mario);

        // Record a few frames of history at the fixed step (each tick records).
        let sub_dt = 1.0 / 60.0;
        for i in 0..5u32 {
            game.tick(FrameTime {
                tts: (i + 1) as f32 * sub_dt,
                dts: sub_dt,
            });
        }
        assert!(game.current_scene_frame().is_some(), "history recorded");

        // A live input on a 0-SUBSTEP frame: buffered, but no tick drains it.
        game.key_event(Key::Up as i32, true);
        assert!(
            !game.input_buf.is_empty(),
            "input must be buffered when no tick follows it"
        );

        // Scrub back — the model is restored, so the buffered event is orphaned
        // and must be dropped (the fix), not recorded into the branch.
        game.seek_scene_to(1).expect("seek to an earlier frame");
        assert!(
            game.input_buf.is_empty(),
            "seek must drop input orphaned by the model restore"
        );
    }

    /// Ghosting a PHYSICS game end-to-end (the world-scoped host,
    /// docs/time-travel.md T6b): the ghost's `draw` calls resolve
    /// `Physics.transformed` against each division's PROJECTED world — the
    /// strobe shows the ball falling through its future poses, not N copies of
    /// the paused pose — and a scripted input whose handler issues a physics
    /// COMMAND (`Physics.applyImpulse`) kicks the projected ball, altering the
    /// ghost trajectory. The live world stays byte-identical throughout.
    #[test]
    fn ghost_frames_project_physics_poses_and_replay_commands() {
        use functor_runtime_common::{Key, RecordedInput};

        physics::remove_world(physics::DEFAULT_WORLD);

        let dir =
            std::env::temp_dir().join(format!("functor-lang-ghost-phys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp project dir");
        std::fs::write(
            dir.join("game.fun"),
            "let init = { n: 0.0 }\n\
             let tick = (m, dt, tts) => m\n\
             let input = (m, key, isDown) =>\n\
               match key with\n\
               | \"K\" =>\n\
                 (match isDown with\n\
                  | true => (m, Physics.applyImpulse(\"ball\", 6.0, 0.0, 0.0))\n\
                  | false => m)\n\
               | _ => m\n\
             let physics = (m) => Physics.scene(0.0, -9.81, 0.0, [\n\
             \x20 Physics.fixed(\"ground\", Physics.box(10.0, 0.4, 10.0)) |> Physics.at(0.0, -0.2, 0.0),\n\
             \x20 Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 4.0, 0.0)])\n\
             let draw = (m, tts) => Frame.create(\n\
               Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),\n\
               Scene.sphere() |> Physics.transformed(\"ball\"))\n",
        )
        .expect("write game");
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().expect("utf-8 path"));

        // Drive to a fork point mid-fall (ticks record history; render seeds
        // `last_frame`, whose camera the ghost freezes to).
        const SUB_DT: f32 = 1.0 / 60.0;
        const K: usize = 10;
        let mut tts = 0.0f32;
        for _ in 0..K {
            tts += SUB_DT;
            game.tick(FrameTime { tts, dts: SUB_DT });
            game.render(FrameTime { tts, dts: SUB_DT });
        }
        let paused_y = game.last_frame.scene.xform.w.y;
        assert!(
            paused_y < 4.0 && paused_y > 3.0,
            "fork mid-fall, y = {paused_y}"
        );
        let live_world_before = physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot());

        // The strobe over ~1s in 4 divisions: each ghost frame's draw must
        // read that division's projected world — falling, then resting.
        const DIVISIONS: usize = 4;
        const DT: f32 = 0.25;
        let ghosts = game.ghost_frames(DIVISIONS, DT, tts as f64, None);
        assert_eq!(ghosts.len(), DIVISIONS, "one frame per division");
        // Each frame is paired with its division-boundary time (the compositor
        // renders it at that time, so render-time animation advances).
        for (i, (_, ft)) in ghosts.iter().enumerate() {
            let expected = tts + (i as f32 + 1.0) * DT;
            assert!(
                (ft.tts - expected).abs() < 1e-4,
                "division {i} time: {} vs {expected}",
                ft.tts
            );
            assert_eq!(ft.dts, 0.0, "a ghost frame is a still of the future");
        }
        let ys: Vec<f32> = ghosts.iter().map(|(f, _)| f.scene.xform.w.y).collect();
        assert!(
            ys[0] < paused_y - 0.5,
            "division 0 must have fallen past the paused pose: {ys:?} vs {paused_y}"
        );
        for pair in ys.windows(2) {
            // Allow a small settle-bounce near the slab (restitution), but the
            // strobe must never show the ball climbing back up.
            assert!(
                pair[1] <= pair[0] + 0.1,
                "the ghost ball must keep falling/settling: {ys:?}"
            );
        }
        let rest_y = *ys.last().unwrap();
        assert!(
            (0.3..0.7).contains(&rest_y),
            "the ghost ball should come to rest on the slab: {ys:?}"
        );

        // A scripted kick (K down at the first fine step) must alter the ghost:
        // its handler returns Physics.applyImpulse, which applies to the
        // PROJECTED world — the kicked ghost drifts in +x, the coast ghost
        // stays on the fall line.
        let steps = DIVISIONS * ((DT / SUB_DT).round() as usize).max(1);
        let mut script: Vec<Vec<RecordedInput>> = vec![Vec::new(); steps];
        script[0].push(RecordedInput::Key {
            code: Key::K as i32,
            is_down: true,
        });
        let kicked = game.ghost_frames(DIVISIONS, DT, tts as f64, Some(&script));
        assert_eq!(kicked.len(), DIVISIONS);
        let coast_x = ghosts.last().unwrap().0.scene.xform.w.x;
        let kicked_x = kicked.last().unwrap().0.scene.xform.w.x;
        assert!(coast_x.abs() < 1e-3, "coast ghost stays centered: {coast_x}");
        assert!(
            kicked_x > 1.0,
            "the replayed kick must move the projected ball: {kicked_x}"
        );

        // The live producer and world are untouched by all of the above.
        assert_eq!(
            physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot()),
            live_world_before,
            "live world untouched by ghosting"
        );
        assert!(
            (game.last_frame.scene.xform.w.y - paused_y).abs() < 1e-6,
            "live frame untouched by ghosting"
        );

        physics::remove_world(physics::DEFAULT_WORLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Paused-inspector trace (visual-debugger PR2) -----------------------

    /// A per-second subscription whose `update` fires an `Effect.now` — one
    /// timer firing drives TWO `update`s in a frame (the message + its effect
    /// result), so the trace shows `update` count > 1.
    const INSPECTOR_GAME: &str = "\
        type Msg = | Tick | GotTime(t: Float)\n\
        type Model = { ticks: Float, lastTime: Float }\n\
        let init = { ticks: 0.0, lastTime: 0.0 }\n\
        let update = (m: Model, msg: Msg) =>\n\
          match msg with\n\
          | Tick => ({ m with ticks: m.ticks + 1.0 }, Effect.now((t) => GotTime(t)))\n\
          | GotTime(t) => { m with lastTime: t }\n\
        let subscriptions = (m: Model) => Sub.every(Time.seconds(1.0), Tick)\n\
        let tick = (m: Model, dt: Float, tts: Float) => m\n\
        let draw = (m: Model, tts: Float) =>\n\
          Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), Scene.cube())\n";

    #[test]
    fn inspector_trace_replays_the_paused_frame_and_is_empty_while_playing() {
        let dir = std::env::temp_dir().join(format!("functor-inspector-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.fun"), INSPECTOR_GAME).unwrap();
        let mut game = FunctorLangGame::create(dir.join("game.fun").to_str().unwrap());

        // Frame 1 seeds prev_tts (nothing fires on frame one); frame 2 crosses
        // the 1s boundary → the timer fires.
        game.tick(FrameTime { dts: 0.9, tts: 0.9 });
        game.tick(FrameTime { dts: 0.2, tts: 1.1 });

        // NOT paused: the cheap early-out — no replay, empty invocations, but
        // frame/tts/sources still filled. (Proves the recorder is only armed on
        // the paused path — `build_invocations` never runs here.)
        let playing: serde_json::Value =
            serde_json::from_str(&game.inspector_trace(false)).unwrap();
        assert_eq!(playing["paused"], serde_json::json!(false));
        assert_eq!(playing["invocations"].as_array().unwrap().len(), 0);
        assert!(!playing["sources"].as_array().unwrap().is_empty());

        // PAUSED: the last real frame replays into invocations.
        let paused: serde_json::Value = serde_json::from_str(&game.inspector_trace(true)).unwrap();
        assert_eq!(paused["paused"], serde_json::json!(true));
        let invs = paused["invocations"].as_array().unwrap();
        let updates: Vec<_> = invs.iter().filter(|i| i["entry"] == "update").collect();
        assert_eq!(updates.len(), 2, "update count > 1: {invs:#?}");
        assert_eq!(updates[0]["count"], serde_json::json!(2));
        assert_eq!(updates[0]["index"], serde_json::json!(0));
        assert_eq!(
            updates[0]["provenance"],
            serde_json::json!("subscription: Tick")
        );
        assert!(updates[1]["provenance"]
            .as_str()
            .unwrap()
            .starts_with("effect result: GotTime("));
        assert_eq!(updates[0]["ghost"], serde_json::json!(false));

        // Bindings map to the entry file with LOCAL byte offsets + values.
        let bindings = updates[0]["bindings"].as_array().unwrap();
        assert!(!bindings.is_empty());
        assert!(bindings.iter().all(|b| b["file"] == "game.fun"));
        assert!(bindings
            .iter()
            .all(|b| b["start"].as_u64().is_some() && b["value"].is_string()));

        // The `tick` invocation is present with its dt provenance.
        let tick = invs.iter().find(|i| i["entry"] == "tick").unwrap();
        assert_eq!(tick["provenance"], serde_json::json!("tick dt=0.2"));

        // sources: one entry per project file, each a 64-hex sha256.
        assert_eq!(paused["sources"][0]["file"], serde_json::json!("game.fun"));
        assert_eq!(paused["sources"][0]["hash"].as_str().unwrap().len(), 64);

        // A second /trace while still paused is served from cache (identical).
        assert_eq!(game.inspector_trace(true), game.inspector_trace(true));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
