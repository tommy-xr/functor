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
    contains_effect, deliver_physics_events, drain_effects, frame_value, needs_update,
    net_conn_subs, net_event_value, perform_deferred_queries, physics_event_taggers,
    physics_scene_value, split_model_effect, sub_messages_for_frame, view_value, EffectLog,
    EffectTree, FunctorHost, NetEventKind, RealEffects,
};
use functor_runtime_common::physics;
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use mle::{Session, Value};
use wasm_bindgen::prelude::*;

pub struct MleWebGame {
    path: String,
    src: String,
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
    /// Declared connection keys (`Sub.connect`/`Sub.listen`), reconciled each
    /// frame — see the desktop producer.
    live_conn_keys: std::collections::HashSet<String>,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// The last per-frame error logged, to avoid flooding the console at
    /// 60fps with the same message (a persistent error logs once until it
    /// changes).
    last_error: Option<String>,
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
            path: path.to_string(),
            src: loaded.src,
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
            live_conn_keys: std::collections::HashSet::new(),
            has_physics: loaded.has_physics,
            has_ui: loaded.has_ui,
            last_view: View::Empty,
            last_frame: empty_frame(),
            last_error: None,
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
        self.src = loaded.src;
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
        // A deferred query holds a tagger — a closure into the OLD session;
        // drop them rather than let them dangle.
        self.deferred_queries.clear();
        self.pending_events.clear();
        self.has_ui = loaded.has_ui;
        if !self.has_ui {
            // Deleting the `ui` hook drops the HUD (the physics-world rule).
            self.last_view = View::Empty;
        }
        self.last_error = None;
        report.rebound
    }

    /// Fire subscription timers over `(prev_tts, tts]` and fold their
    /// messages through `update`, before this frame's `tick` — the message
    /// drain seam (docs/mle.md C4b-2), same semantics as the desktop
    /// producer. Errors report per message and processing continues.
    /// Take an entry point's return: split off any `(model, effect)` pair,
    /// adopt the model, and drain the effects to a fixed point through
    /// `update` (docs/mle.md B6) — mirrors the desktop producer.
    fn absorb(&mut self, returned: Value) {
        let (model, effects) = split_model_effect(returned);
        self.model = model;
        // Effects are commands, not data — one stored in the model would
        // make the pair sniff ambiguous on a later return.
        if contains_effect(&self.model) {
            self.log_once(
                "[mle] the model contains an Effect value — Effects are commands, \
not data; return them beside the model as `(model, effect)` instead of storing them"
                    .to_string(),
            );
        }
        let Some(effects) = effects else { return };
        // Only MESSAGE-producing effects need an `update` to receive them —
        // tagger-less physics commands must not be dropped over a missing
        // hook (mirrors the desktop producer).
        if needs_update(&effects) && self.session.global("update").is_none() {
            self.log_once(
                "[mle] effects returned but there is no `let update = (model, msg) => …` \
to receive their messages; dropping them"
                    .to_string(),
            );
            return;
        }
        let mut reports: Vec<String> = Vec::new();
        let deferred = drain_effects(
            &self.session,
            &mut self.model,
            effects,
            &mut self.effect_runner,
            &mut self.effect_log,
            &mut |message| reports.push(message),
        );
        // Physics queries wait for the post-step drain (end of `tick`), so
        // their taggers answer against THIS frame's stepped world.
        self.deferred_queries.extend(deferred);
        for message in reports {
            self.log_once(message);
        }
    }

    /// Close every connection this producer still has open (a reload that
    /// dropped `subscriptions`, or shutdown). CloseKey is queued for each;
    /// the live set is cleared.
    fn close_all_connections(&mut self) {
        use functor_runtime_common::net::{push_conn_command, ConnCommand};
        for key in std::mem::take(&mut self.live_conn_keys) {
            push_conn_command(ConnCommand::CloseKey { key });
        }
    }

    /// Open connections newly declared this frame and close dropped ones —
    /// see the desktop producer's `reconcile_connections`.
    fn reconcile_connections(&mut self, subs: &Value) {
        use functor_runtime_common::net::{push_conn_command, ConnCommand};
        let conns = match net_conn_subs(subs) {
            Ok(conns) => conns,
            Err(message) => return self.log_once(format!("[mle] {message}")),
        };
        // Dedupe by key (first declaration wins its listen/connect role) so
        // a key is opened at most once even if declared twice in one frame.
        let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
        for conn in &conns {
            if !declared.insert(conn.key.clone()) {
                continue; // already handled this key this frame
            }
            if !self.live_conn_keys.contains(&conn.key) {
                push_conn_command(if conn.listen {
                    ConnCommand::Listen {
                        key: conn.key.clone(),
                        addr: conn.key.clone(),
                    }
                } else {
                    ConnCommand::Connect {
                        key: conn.key.clone(),
                        url: conn.key.clone(),
                    }
                });
            }
        }
        for key in &self.live_conn_keys {
            if !declared.contains(key) {
                push_conn_command(ConnCommand::CloseKey { key: key.clone() });
            }
        }
        self.live_conn_keys = declared;
    }

    /// Route one inbound connection event to the matching key's fresh
    /// tagger and fold through `update` — see the desktop producer.
    fn deliver_net_event(&mut self, key: String, kind: NetEventKind, conn: i32, text: String) {
        if !self.has_subscriptions {
            return;
        }
        let subs =
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => subs,
                Err(err) => return self.frame_error("subscriptions", &err),
            };
        let conns = match net_conn_subs(&subs) {
            Ok(conns) => conns,
            Err(message) => return self.log_once(format!("[mle] {message}")),
        };
        let Some(sub) = conns.into_iter().find(|c| c.key == key) else {
            return;
        };
        let value = net_event_value(kind, conn as u64, &text).to_mle();
        let msg = match self
            .session
            .apply(sub.tagger, vec![value], "net event", &mut FunctorHost)
        {
            Ok(msg) => msg,
            Err(err) => return self.frame_error("net event", &err),
        };
        match self
            .session
            .call("update", vec![self.model.clone(), msg], &mut FunctorHost)
        {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.frame_error("update", &err),
        }
    }

    fn pump_subscriptions(&mut self, tts: f64) {
        // Advance the window even without subscriptions (or on frame one),
        // so the edge is always sane.
        let prev = self.prev_tts.replace(tts);
        if !self.has_subscriptions {
            // No subscriptions must not leave a previous program's
            // connections open (a hot reload that dropped them).
            if !self.live_conn_keys.is_empty() {
                self.close_all_connections();
            }
            return;
        }
        let subs =
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => subs,
                Err(err) => return self.frame_error("subscriptions", &err),
            };
        self.reconcile_connections(&subs);
        let Some(prev) = prev else {
            return;
        };
        let msgs = match sub_messages_for_frame(&subs, prev, tts) {
            Ok(msgs) => msgs,
            Err(message) => return self.log_once(format!("[mle] {message}")),
        };
        for msg in msgs {
            match self
                .session
                .call("update", vec![self.model.clone(), msg], &mut FunctorHost)
            {
                Ok(returned) => self.absorb(returned),
                Err(err) => self.frame_error("update", &err),
            }
        }
    }

    /// The frame's physics phase (docs/physics.md): ask the game what bodies
    /// should exist, reconcile the singleton world to match, and advance it
    /// in fixed substeps. Runs after `tick` so declarations come from the
    /// settled model, and before `render` so `Physics.position`/
    /// `Physics.transformed` in `draw` read the just-stepped world.
    /// Returns the number of fixed substeps taken (0 when there is no
    /// `physics` hook, the hook errored, or the accumulator hasn't reached a
    /// full step yet).
    fn step_physics(&mut self, dts: f32) -> u32 {
        if !self.has_physics {
            return 0;
        }
        let args = vec![self.model.clone()];
        match self.session.call("physics", args, &mut FunctorHost) {
            Ok(value) => match physics_scene_value(&value) {
                Some(scene) => {
                    // The recorded drive (Phase 6): every fixed frame goes
                    // through the Timeline, so pause/rewind/replay work.
                    let advanced = self.physics_rt.advance(scene, dts);
                    self.pending_events = advanced.events;
                    self.physics_status = advanced.status;
                    let steps = advanced.steps;
                    let warnings = advanced.warnings;
                    // Command effects apply asynchronously (queued at perform
                    // time, applied at the step), so their problems — unknown
                    // tag, queue overflow — surface here, deduped.
                    for warning in warnings {
                        self.log_once(format!("[mle] {warning}"));
                    }
                    return steps;
                }
                None => self.log_once(format!(
                    "[mle] physics must return Physics.scene(gx, gy, gz, [body, …]), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.frame_error("physics", &err),
        }
        0
    }

    /// Report a per-frame error with its source position, once per distinct
    /// message (a 60fps loop must not flood the console with one persistent
    /// bug).
    fn frame_error(&mut self, stage: &str, err: &mle::RunError) {
        let (line, col) = mle::line_col(&self.src, err.span.start);
        let rendered = format!(
            "[mle] {stage} error at {}:{line}:{col}: {}",
            self.path, err.message
        );
        self.log_once(rendered);
    }

    fn log_once(&mut self, rendered: String) {
        if self.last_error.as_deref() != Some(rendered.as_str()) {
            web_sys::console::error_1(&rendered.as_str().into());
            self.last_error = Some(rendered);
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

    fn tick(&mut self, frame_time: FrameTime) {
        // Subscriptions first, so `tick` sees a model that has absorbed this
        // frame's messages (the F# executor's ordering).
        self.pump_subscriptions(frame_time.tts as f64);
        let args = vec![
            self.model.clone(),
            Value::Number(frame_time.dts as f64),
            Value::Number(frame_time.tts as f64),
        ];
        match self.session.call("tick", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.frame_error("tick", &err),
        }
        let physics_steps = self.step_physics(frame_time.dts);
        // Post-step query drain: deferred raycasts answer against the world
        // just stepped; their messages fold through `update` before `draw`.
        // On a ZERO-substep frame (the accumulator short of FIXED_DT — normal
        // right after load and at >60fps) queries stay deferred, like pending
        // commands, so they never answer against a world that hasn't
        // simulated. Games without a physics hook answer immediately (the
        // lazily-created empty world gives sane misses).
        // A query answers once the world has EVER stepped: normally this
        // frame's steps, but also while PAUSED (frozen mid-flight, frame > 0)
        // and on a short zero-substep frame — so a raycast fired while paused
        // answers against the frozen world instead of deferring forever.
        let world_ready = physics_steps > 0 || !self.has_physics || self.physics_status.0 > 0;
        if world_ready && !self.deferred_queries.is_empty() {
            let deferred = std::mem::take(&mut self.deferred_queries);
            let mut reports: Vec<String> = Vec::new();
            perform_deferred_queries(
                &self.session,
                &mut self.model,
                deferred,
                &mut self.effect_runner,
                &mut self.effect_log,
                &mut |message| reports.push(message),
            );
            for message in reports {
                self.log_once(message);
            }
        }
        // Collision events (docs/physics.md Phase 5): this frame's contact
        // transitions, delivered to the `Physics.events` taggers of the
        // CURRENT model's subscriptions — post-step, alongside query answers.
        let events = std::mem::take(&mut self.pending_events);
        if !events.is_empty() && self.has_subscriptions {
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => match physics_event_taggers(&subs) {
                    Ok(taggers) if !taggers.is_empty() => {
                        let mut reports: Vec<String> = Vec::new();
                        deliver_physics_events(
                            &self.session,
                            &mut self.model,
                            &taggers,
                            &events,
                            &mut self.effect_runner,
                            &mut self.effect_log,
                            &mut |message| reports.push(message),
                        );
                        for message in reports {
                            self.log_once(message);
                        }
                    }
                    Ok(_) => {}
                    Err(message) => self.log_once(format!("[mle] {message}")),
                },
                Err(err) => self.frame_error("subscriptions", &err),
            }
        }
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
            Ok(returned) => self.absorb(returned),
            Err(err) => self.frame_error("input", &err),
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
            Ok(returned) => self.absorb(returned),
            Err(err) => self.frame_error("mouseMove", &err),
        }
    }

    fn mouse_wheel(&mut self, delta: i32) {
        if !self.has_mouse_wheel {
            return;
        }
        let args = vec![self.model.clone(), Value::Number(delta as f64)];
        match self.session.call("mouseWheel", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.frame_error("mouseWheel", &err),
        }
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        let args = vec![self.model.clone(), Value::Number(frame_time.tts as f64)];
        match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => self.last_frame = frame.clone(),
                None => {
                    let rendered = format!(
                        "[mle] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    self.log_once(rendered);
                }
            },
            Err(err) => self.frame_error("draw", &err),
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
                    None => self.log_once(format!(
                        "[mle] ui must return a View (Ui.text / Ui.column / Ui.panel), got {}",
                        value.kind_name()
                    )),
                },
                Err(err) => self.frame_error("ui", &err),
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

    // MLE has no effects yet (docs/mle.md Track B): nothing to drain, nothing
    // pushed back.
    fn net_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn net_push_http_response(&mut self, _token: i32, _status: i32, _body: String) {}
    fn net_push_http_error(&mut self, _token: i32, _message: String) {}
    fn audio_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn audio_scene_json(&self) -> String {
        "{\"sources\":[]}".to_string()
    }
    fn net_drain_conn_commands(&self) -> String {
        functor_runtime_common::net::drain_conn_commands_json()
    }
    fn net_push_connected(&mut self, key: String, conn: i32) {
        self.deliver_net_event(key, NetEventKind::Connected, conn, String::new());
    }
    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String) {
        self.deliver_net_event(key, NetEventKind::Message, conn, text);
    }
    fn net_push_disconnected(&mut self, key: String, conn: i32) {
        self.deliver_net_event(key, NetEventKind::Disconnected, conn, String::new());
    }
    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String) {
        self.deliver_net_event(key, NetEventKind::Error, conn, message);
    }
    fn audio_push_finished(&mut self, _token: i32) {}

    fn quit(&mut self) {
        self.close_all_connections();
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
// The F# path feeds input straight into the game wasm module (index.html calls
// `key_event_wasm` etc. on it); the MLE game lives *inside* this runtime, so
// the MLE index page calls the `mle_*` exports below instead. Events queue
// here and the frame loop drains them into the producer before each tick.

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

/// Deliver a keyboard event (`code` = `functor_runtime_common::Key` as i32,
/// the same wire mapping index.html uses for the F# path).
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
