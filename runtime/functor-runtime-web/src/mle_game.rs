//! The MLE producer for the web shell (docs/mle.md Track C5): the wasm
//! sibling of the desktop runner's `mle_game.rs`, behind the same
//! `GameProducer` seam. Same load-time contract validation and per-frame
//! semantics — the MVU pair (subscriptions fold through `update` before
//! `tick`), the optional `physics` hook (tick → physics → draw), a bad frame
//! keeps the last good model/frame, per-frame errors dedupe — but adapted to
//! the browser:
//!
//! - the `.mle` source arrives over HTTP (fetched by `run_async` from the dev
//!   server, which serves the project directory) instead of the filesystem;
//! - no file-watch hot reload (there is no filesystem to watch), but the
//!   PUSH path exists (docs/mle.md D4): `reload_source` mirrors the desktop
//!   runner's `POST /reload-source` — parse → lower → check-as-warnings →
//!   `Session::load` → `mle::rebind_value` on the held model — reachable
//!   from the page via the `mle_set_source` wasm export in `lib.rs`;
//! - no per-frame perf stats (`std::time::Instant` panics on wasm; the C6
//!   perf gate measures natively);
//! - input events arrive from the page via the `mle_*` wasm exports below,
//!   queued and drained by the frame loop each frame before `tick` (DOM
//!   handlers fire between rAF callbacks, never mid-frame).

use std::cell::RefCell;

use functor_runtime_common::mle_prelude::{
    audio_scene_of, clear_audio_completions, clear_http_taggers, contains_effect, frame_value,
    view_value, EffectLog, EffectTree, FunctorHost, NetEventKind, RealEffects,
};
use functor_runtime_common::mle_producer::{FrameCtx, Reporter, SpanSource};
use functor_runtime_common::physics;
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::timetravel::SceneRecorder;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use mle::{Session, Value};
use wasm_bindgen::prelude::*;

pub struct MleWebGame {
    path: String,
    /// The lowered module the current session came from — kept (like the
    /// desktop producer) so a pushed reload can rebind model-stored closures
    /// (old module × new module).
    module: mle::ir::Module,
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
    /// recorder + pause flag + fixed-step accumulator. The World stays in
    /// the registry; this owns the rewind machinery over it.
    physics_rt: physics::SteppedPhysics,
    /// Latest recorder status for the overlay: (fixed frame, paused, history).
    physics_status: (u64, bool, u64),
    /// The coupled time-travel recorder (docs/time-travel.md T1–T3), shared with
    /// the desktop producer (one tested impl): records the settled `model` +
    /// physics fixed-frame each rendered frame and seeks/rewinds them together.
    recorder: SceneRecorder,
    /// Declared connection keys (`Sub.connect`/`Sub.listen`), reconciled each
    /// frame — see the desktop producer.
    live_conn_keys: std::collections::HashSet<String>,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// Per-frame error reporting (dedupe + browser-console sink + single-source
    /// span rendering) — shared with the desktop producer
    /// (`mle_producer::Reporter`).
    reporter: Reporter,
}

/// A successfully loaded, contract-validated game module (the desktop
/// producer's `Loaded`, verbatim minus the file-shaped fields).
struct Loaded {
    src: String,
    module: mle::ir::Module,
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

/// Load, check, and contract-validate game source — the web counterpart of
/// the desktop `load_source`, shared by the page-load path (`create`) and
/// the editor push path (`reload_source`). Errors come back as fully
/// rendered strings (`path:line:col: message`); `path` is only a label for
/// error rendering.
fn load_source(path: &str, src: String) -> Result<Loaded, String> {
    let render = |stage: &str, span: mle::Span, message: &str| {
        let (line, col) = mle::line_col(&src, span.start);
        format!("cannot {stage} {path}:{line}:{col}: {message}")
    };
    // Load through the single-source project path so the built-in `Net`
    // module is in scope (the web fetches ONE file — no siblings — but the
    // Net ADT must still resolve). Spans render against the entry source.
    let project = mle::project::load_single_source("Game", &src)
        .map_err(|e| format!("cannot load {path}:{}:{}: {}", e.line, e.col, e.message))?;
    let module = project.module;
    // Type diagnostics are advisory in the dev loop: warn, keep going
    // (the CLI's `build` is the strict gate).
    for diag in mle::check(&module) {
        let (line, col) = mle::line_col(&src, diag.span.start);
        web_sys::console::warn_1(&format!("warning: {path}:{line}:{col}: {}", diag.message).into());
    }
    let session = Session::load(&module, &mut FunctorHost)
        .map_err(|f| render("load", f.error.span, &f.error.message))?;
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
    require_function(path, &session, "tick", 3)?;
    require_function(path, &session, "draw", 2)?;
    // `input` is optional (many games are non-interactive), but when
    // present it must honor the contract: (model, key, isDown) => model.
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
    // messages fold through `update(model, msg)` — so subscriptions
    // without an update have nowhere to deliver.
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
    // bodies that should exist; the host reconciles + fixed-steps the
    // world after each tick (docs/physics.md). Rapier is pure Rust, so
    // the world runs in the browser exactly as it does natively.
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
        src,
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

impl MleWebGame {
    /// Build the producer from fetched game source. Errors come back fully
    /// rendered for `run_async` to fail loud with (there is no keep-running
    /// fallback: a page load either gets a valid game or a console error).
    pub fn create(path: &str, src: String) -> Result<MleWebGame, String> {
        let loaded = load_source(path, src)?;
        web_sys::console::log_1(&format!("[mle] loaded {path}").into());
        Ok(MleWebGame {
            reporter: Reporter::new(
                SpanSource::Single {
                    src: loaded.src,
                    path: path.to_string(),
                },
                report_to_console,
            ),
            path: path.to_string(),
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
            physics_status: (0, false, 0),
            recorder: SceneRecorder::new(),
            live_conn_keys: std::collections::HashSet::new(),
            has_physics: loaded.has_physics,
            has_soundscape: loaded.has_soundscape,
            last_soundscape_json: empty_soundscape_json(),
            has_ui: loaded.has_ui,
            last_view: View::Empty,
            last_frame: empty_frame(),
        })
    }

    /// Swap in a freshly loaded program, KEEPING THE MODEL — the desktop
    /// producer's `swap_in`, verbatim. `init` from the new program is
    /// deliberately unused: state survives the edit, and closures stored in
    /// the model rebind to the edited code (B5 part 2, `mle::rebind_value`).
    /// The physics world is deliberately KEPT too, like the model: it lives
    /// in this process's registry, so bodies stay where they are across the
    /// edit (removing the `physics` hook drops the world). `prev_tts` is kept
    /// as well: `Sub.every` fires on the global time grid, so timers tick
    /// right through a reload. Returns the number of stored closures rebound,
    /// for the status line.
    fn swap_in(&mut self, loaded: Loaded) -> usize {
        let (model, report) = mle::rebind_value(&self.model, &self.module, &loaded.module);
        self.model = model;
        for warning in &report.warnings {
            web_sys::console::warn_1(&format!("[mle] reload: {warning}").into());
        }
        self.reporter.set_source(SpanSource::Single {
            src: loaded.src,
            path: self.path.clone(),
        });
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
        // Reload is a model-history BOUNDARY (see the desktop producer): the
        // retained snapshots can hold old-module closures, so they can't cross
        // a reload; the recorder keeps its rendered-frame clock monotonic so
        // recording resumes consecutively.
        self.recorder.reset_on_reload();
        self.has_ui = loaded.has_ui;
        if !self.has_ui {
            // Deleting the `ui` hook drops the HUD (the physics-world rule).
            self.last_view = View::Empty;
        }
        self.reporter.reset();
        report.rebound
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
            effect_runner: &mut self.effect_runner,
            effect_log: &mut self.effect_log,
            deferred_queries: &mut self.deferred_queries,
            pending_events: &mut self.pending_events,
            live_conn_keys: &mut self.live_conn_keys,
            prev_tts: &mut self.prev_tts,
            has_physics: self.has_physics,
            has_subscriptions: self.has_subscriptions,
            reporter: &mut self.reporter,
        }
    }
}

impl GameProducer for MleWebGame {
    // File-watch hot reload is native-only (docs/mle.md C3) — there is no
    // filesystem here. The PUSH path below is the web's reload.
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn reload_source(&mut self, source: &str) -> Result<String, String> {
        // The editor push path (docs/mle.md D4), same semantics as the
        // desktop runner's `POST /reload-source`: model preserved, a broken
        // push keeps the old program (and the error goes back to the pusher,
        // who is looking at the source that caused it). No mtime bookkeeping
        // — the browser has no file watcher; pushes are the only reload.
        let started = js_sys::Date::now();
        let loaded = load_source(&self.path, source.to_string())?;
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
        web_sys::console::log_1(&format!("[mle] {status}").into());
        Ok(status)
    }

    /// Coupled scene rewind — delegated to the shared [`SceneRecorder`]
    /// (docs/time-travel.md T1), identical to the desktop producer.
    fn rewind_scene_to(&mut self, target: u64) -> Result<String, String> {
        let result = self.recorder.rewind_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_status,
            self.has_physics,
        );
        if result.is_ok() {
            self.deferred_queries.clear();
            self.pending_events.clear();
        }
        result
    }

    fn seek_scene_to(&mut self, target: u64) -> Result<String, String> {
        self.recorder.seek_scene_to(
            target,
            &mut self.model,
            &mut self.physics_rt,
            &mut self.physics_status,
            self.has_physics,
        )
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

    fn tick(&mut self, frame_time: FrameTime) {
        // The whole MVU frame body lives in the shared `FrameCtx`
        // (docs/time-travel.md T6a). Web runs it as one call — unlike native it
        // has no per-frame perf timing to split it at the physics boundary.
        self.ctx().run_frame(frame_time);
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        // The optional `input` entry point: (model, key, isDown) => model.
        // Keys cross as their canonical names ("W", "Up", "Space") — the same
        // spelling the desktop producer and SDK use.
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
                None => {
                    let rendered = format!(
                        "[mle] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    self.reporter.report_once(rendered);
                }
            },
            Err(err) => self.reporter.frame_error("draw", &err),
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
                        "[mle] soundScape must return an AudioScene (AudioScene.create / \
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

    fn net_drain_commands(&self) -> String {
        // HttpRequest commands (Effect.httpGet/httpPost); the page's fetch host
        // performs them and returns the response via net_push_http_*.
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
// The MLE game lives *inside* this runtime, so the MLE index page
// (index-mle.html) calls the `mle_*` exports below. Events queue here and the
// frame loop drains them into the producer before each tick.

enum InputEvent {
    Key { code: i32, is_down: bool },
    MouseMove { x: i32, y: i32 },
    MouseWheel { delta: i32 },
}

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
pub fn mle_key_event(code: i32, is_down: bool) {
    push_input(InputEvent::Key { code, is_down });
}

/// Deliver a mouse position in window pixels (the page accumulates pointer-lock
/// movement deltas, matching the desktop's absolute cursor position).
#[wasm_bindgen]
pub fn mle_mouse_move(x: i32, y: i32) {
    push_input(InputEvent::MouseMove { x, y });
}

/// Deliver a mouse-wheel event (vertical scroll offset, ±1 per notch).
#[wasm_bindgen]
pub fn mle_mouse_wheel(delta: i32) {
    push_input(InputEvent::MouseWheel { delta });
}

/// Drain the queued page input into the producer, in arrival order. Called by
/// the frame loop before `tick`. Empty (and free) on the F# path — its page
/// never calls the `mle_*` exports.
pub fn drain_input(game: &mut dyn GameProducer) {
    let events = INPUT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for event in events {
        match event {
            InputEvent::Key { code, is_down } => game.key_event(code, is_down),
            InputEvent::MouseMove { x, y } => game.mouse_move(x, y),
            InputEvent::MouseWheel { delta } => game.mouse_wheel(delta),
        }
    }
}

// --- Time-travel scrubber ↔ DOM bridge (docs/time-travel.md T3) -------------
//
// On web the scrubber is NATIVE DOM (index-mle.html), not egui-in-canvas, so
// its widgets sit OUTSIDE the game canvas — their clicks never reach the canvas
// (no pointer-lock clash) and they render as accessible browser controls. The
// page calls the `mle_scrub_*` write exports (queued here, applied by the frame
// loop, which owns the clock) and polls the read exports each frame; the loop
// publishes the current view state. The coupled-rewind LOGIC stays shared
// (`SceneRecorder`); only the UI surface differs from desktop.

/// A control from the DOM scrubber, applied by the frame loop.
pub enum ScrubControl {
    TogglePause,
    Step,
    SeekTo(u64),
}

thread_local! {
    static SCRUB_CONTROLS: RefCell<Vec<ScrubControl>> = const { RefCell::new(Vec::new()) };
    /// Published each frame for the page's slider: `(frame, lo, hi, paused)`.
    /// `frame`/`lo`/`hi` are `-1.0` when nothing is recorded yet.
    static SCRUB_VIEW: RefCell<(f64, f64, f64, bool)> =
        const { RefCell::new((-1.0, -1.0, -1.0, false)) };
}

const SCRUB_CONTROLS_CAP: usize = 256;

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
pub fn publish_scrub_view(frame: Option<u64>, range: Option<(u64, u64)>, paused: bool) {
    let f = frame.map(|f| f as f64).unwrap_or(-1.0);
    let (lo, hi) = range
        .map(|(l, h)| (l as f64, h as f64))
        .unwrap_or((-1.0, -1.0));
    SCRUB_VIEW.with(|v| *v.borrow_mut() = (f, lo, hi, paused));
}

/// Page → runtime: toggle pause (pin/unpin the clock).
#[wasm_bindgen]
pub fn mle_scrub_toggle_pause() {
    push_scrub(ScrubControl::TogglePause);
}

/// Page → runtime: advance exactly one frame, then hold.
#[wasm_bindgen]
pub fn mle_scrub_step() {
    push_scrub(ScrubControl::Step);
}

/// Page → runtime: non-destructively scrub to a rendered frame (slider drag).
#[wasm_bindgen]
pub fn mle_seek_scene(frame: f64) {
    if frame >= 0.0 {
        push_scrub(ScrubControl::SeekTo(frame as u64));
    }
}

/// Runtime → page: the current handle frame (`-1` if nothing recorded).
#[wasm_bindgen]
pub fn mle_scene_frame() -> f64 {
    SCRUB_VIEW.with(|v| v.borrow().0)
}

/// Runtime → page: the seekable window as `[lo, hi]`, or `[]` if empty.
#[wasm_bindgen]
pub fn mle_scene_range() -> Vec<f64> {
    let (_, lo, hi, _) = SCRUB_VIEW.with(|v| *v.borrow());
    if lo < 0.0 {
        vec![]
    } else {
        vec![lo, hi]
    }
}

/// Runtime → page: whether the clock is currently pinned.
#[wasm_bindgen]
pub fn mle_scrub_paused() -> bool {
    SCRUB_VIEW.with(|v| v.borrow().3)
}
