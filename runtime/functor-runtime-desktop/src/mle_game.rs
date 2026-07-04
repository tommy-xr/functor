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
    drain_effects, frame_value, physics_scene_value, split_model_effect, sub_messages_for_frame,
    EffectLog, FunctorHost, RealEffects,
};
use functor_runtime_common::physics;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use mle::{Session, Value};

use crate::game::Game;

pub struct MleGame {
    path: String,
    mtime: std::time::SystemTime,
    src: String,
    /// The lowered module the current session came from — kept so a reload
    /// can rebind model-stored closures (old module × new module).
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
    /// Performs `Effect.*` commands — the real world in the runner; the
    /// drain logic itself is `mle_prelude::drain_effects` (tested there
    /// with fake/replay runners).
    effect_runner: RealEffects,
    /// The structured effect log (last `EFFECT_LOG_CAP` performed effects) —
    /// LLM-readable, and the input format for replay.
    effect_log: EffectLog,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// The last per-frame error printed, to avoid flooding stderr at 60fps
    /// with the same message (a persistent error prints once until it
    /// changes).
    last_error: Option<String>,
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
const EFFECT_LOG_CAP: usize = 256;

/// A successfully loaded, contract-validated game module.
struct Loaded {
    src: String,
    module: mle::ir::Module,
    session: Session,
    init: Value,
    /// The game defines the optional `input` entry point.
    has_input: bool,
    has_mouse_move: bool,
    has_mouse_wheel: bool,
    has_subscriptions: bool,
    has_physics: bool,
}

/// Load, check, and contract-validate a game file. Errors come back as fully
/// rendered strings (`path:line:col: message`) so `create` can exit loud with
/// them and hot-reload can print-and-keep-running with the same text.
fn load_game(path: &str) -> Result<Loaded, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    load_source(path, src)
}

/// The source-shaped half of [`load_game`]: check and contract-validate
/// already-obtained source. `path` is only a label for error rendering — the
/// network reload path (`reload_source`) has no file behind the source.
fn load_source(path: &str, src: String) -> Result<Loaded, String> {
    let render = |stage: &str, span: mle::Span, message: &str| {
        let (line, col) = mle::line_col(&src, span.start);
        format!("cannot {stage} {path}:{line}:{col}: {message}")
    };
    let program = mle::parse(&src).map_err(|e| render("parse", e.span, &e.message))?;
    let module = mle::lower(program).map_err(|e| render("load", e.span, &e.message))?;
    // Type diagnostics are advisory in the dev loop: print, keep going.
    for diag in mle::check(&module) {
        let (line, col) = mle::line_col(&src, diag.span.start);
        eprintln!("warning: {path}:{line}:{col}: {}", diag.message);
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
    })
}

fn file_mtime(path: &str) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

impl MleGame {
    pub fn create(path: &str) -> MleGame {
        // Stat BEFORE reading: an edit that lands mid-load then compares
        // unequal on the next frame and triggers a reload, instead of being
        // silently absorbed into a stale session.
        let mtime = file_mtime(path);
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
            mtime,
            src: loaded.src,
            module: loaded.module,
            session: loaded.session,
            model: loaded.init,
            has_input: loaded.has_input,
            has_mouse_move: loaded.has_mouse_move,
            has_mouse_wheel: loaded.has_mouse_wheel,
            has_subscriptions: loaded.has_subscriptions,
            prev_tts: None,
            has_physics: loaded.has_physics,
            effect_runner: RealEffects::new(),
            effect_log: EffectLog::new(),
            last_frame: empty_frame(),
            last_error: None,
            frames: 0,
            tick_ns: 0,
            physics_ns: 0,
            draw_ns: 0,
        }
    }

    /// Print a per-frame problem once per distinct message — a 60fps loop must
    /// not flood stderr with one persistent bug.
    fn report_once(&mut self, rendered: String) {
        if self.last_error.as_deref() != Some(rendered.as_str()) {
            eprintln!("{rendered}");
            self.last_error = Some(rendered);
        }
    }

    /// Report a per-frame error with its source position (deduped).
    fn frame_error(&mut self, stage: &str, err: &mle::RunError) {
        let (line, col) = mle::line_col(&self.src, err.span.start);
        let rendered = format!(
            "[mle] {stage} error at {}:{line}:{col}: {}",
            self.path, err.message
        );
        self.report_once(rendered);
    }

    /// The frame's physics phase (docs/physics.md): ask the game what bodies
    /// should exist, reconcile the singleton world to match, and advance it in
    /// fixed substeps. Runs after `tick` so declarations come from the settled
    /// model, and before `render` so `Physics.position`/`Physics.transformed`
    /// in `draw` read the just-stepped world.
    fn step_physics(&mut self, dts: f32) {
        if !self.has_physics {
            return;
        }
        let args = vec![self.model.clone()];
        match self.session.call("physics", args, &mut FunctorHost) {
            Ok(value) => match physics_scene_value(&value) {
                Some(scene) => {
                    physics::with_world(physics::DEFAULT_WORLD, |w| {
                        w.reconcile(scene);
                        w.step_frame(dts);
                    });
                }
                None => self.report_once(format!(
                    "[mle] physics must return Physics.scene(gx, gy, gz, [body, …]), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.frame_error("physics", &err),
        }
    }

    /// Take an entry point's return: split off any `(model, effect)` pair,
    /// adopt the model, and drain the effects to a fixed point through
    /// `update` (docs/mle.md B6). Every producer path that runs game code
    /// funnels through here, so effects work uniformly from tick, input,
    /// mouse, and subscription messages.
    fn absorb(&mut self, returned: Value) {
        let (model, effects) = split_model_effect(returned);
        self.model = model;
        let Some(effects) = effects else { return };
        if self.session.global("update").is_none() {
            self.report_once(
                "[mle] effects returned but there is no `let update = (model, msg) => …` \
to receive their messages; dropping them"
                    .to_string(),
            );
            return;
        }
        let mut reports: Vec<String> = Vec::new();
        drain_effects(
            &self.session,
            &mut self.model,
            effects,
            &mut self.effect_runner,
            &mut self.effect_log,
            &mut |message| reports.push(message),
        );
        for message in reports {
            self.report_once(message);
        }
        if self.effect_log.len() > EFFECT_LOG_CAP {
            let excess = self.effect_log.len() - EFFECT_LOG_CAP;
            self.effect_log.drain(..excess);
        }
    }

    /// Fire subscription timers over `(prev_tts, tts]` and fold their
    /// messages through `update`, before this frame's `tick` — the message
    /// drain seam (docs/mle.md C4b-2; B6's effects will feed this same
    /// path). Subscriptions are recomputed from the current model each
    /// frame, so a model change can silence a timer. Errors report per
    /// message and processing continues — one bad message must not stall
    /// the rest, and dedupe keeps a persistent bug to one line.
    fn pump_subscriptions(&mut self, tts: f64) {
        // Advance the window even without subscriptions (or on frame one),
        // so a hot reload that ADDS subscriptions starts from a sane edge.
        let prev = self.prev_tts.replace(tts);
        if !self.has_subscriptions {
            return;
        }
        let Some(prev) = prev else {
            return;
        };
        let subs =
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => subs,
                Err(err) => return self.frame_error("subscriptions", &err),
            };
        let msgs = match sub_messages_for_frame(&subs, prev, tts) {
            Ok(msgs) => msgs,
            Err(message) => return self.report_once(format!("[mle] {message}")),
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
        self.last_error = None;
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
        // Poll the file's mtime (a stat per frame is ~free) and swap in a new
        // session on change. THE MODEL IS KEPT: it is a plain value the host
        // holds, so state survives the edit and all functions rebind — the
        // dev-loop payoff the language was built for (docs/mle.md C3).
        // Closures STORED IN THE MODEL rebind too (B5 part 2,
        // `mle::rebind`): they adopt the edited code with their captured env
        // carried over; one that can't be matched keeps its old body with a
        // loud warning. A broken edit prints and keeps the old program
        // running.
        let mtime = file_mtime(&self.path);
        if mtime == self.mtime {
            return;
        }
        self.mtime = mtime;
        let started = Instant::now();
        match load_game(&self.path) {
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
                self.report_once(format!(
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
        let loaded = load_source(&self.path, source.to_string())?;
        let rebound = self.swap_in(loaded);
        let stored = if rebound > 0 {
            format!("; {rebound} stored closure(s) rebound")
        } else {
            String::new()
        };
        // Absorb any disk mtime observed up to now: a save that landed
        // earlier this frame (after check_hot_reload ran, before this push
        // was serviced) is by definition older than the push and must not
        // revert it next frame. Saves after this instant still win.
        self.mtime = file_mtime(&self.path);
        let status = format!(
            "reloaded {} from pushed source in {:.2}ms (model preserved{stored})",
            self.path,
            started.elapsed().as_secs_f64() * 1000.0
        );
        println!("[mle] {status}");
        Ok(status)
    }

    fn tick(&mut self, frame_time: FrameTime) {
        let started = Instant::now();
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
        self.tick_ns += started.elapsed().as_nanos() as u64;
        let physics_started = Instant::now();
        self.step_physics(frame_time.dts);
        self.physics_ns += physics_started.elapsed().as_nanos() as u64;
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
        let started = Instant::now();
        let args = vec![self.model.clone(), Value::Number(frame_time.tts as f64)];
        match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => self.last_frame = frame.clone(),
                None => self.report_once(format!(
                    "[mle] draw must return Frame.create(camera, scene), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.frame_error("draw", &err),
        }
        self.draw_ns += started.elapsed().as_nanos() as u64;
        // On failure this is the last good frame — a bad draw must not blank
        // the screen.
        self.last_frame.clone()
    }

    fn ui(&self) -> View {
        View::empty()
    }

    fn state_debug(&self) -> String {
        self.model.to_string()
    }

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
        "[]".to_string()
    }
    fn net_push_connected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_message(&mut self, _key: String, _conn: i32, _text: String) {}
    fn net_push_disconnected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_error(&mut self, _key: String, _conn: i32, _message: String) {}
    fn audio_push_finished(&mut self, _token: i32) {}

    fn quit(&mut self) {}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `src` to a temp .mle file and return `load_game`'s error.
    fn load_err(name: &str, src: &str) -> String {
        let path =
            std::env::temp_dir().join(format!("mle-game-test-{}-{name}.mle", std::process::id()));
        std::fs::write(&path, src).expect("write temp game");
        let err = load_game(path.to_str().expect("utf-8 temp path"))
            .err()
            .expect("load should fail");
        let _ = std::fs::remove_file(&path);
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
}
