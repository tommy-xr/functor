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
//! - no hot reload (the browser reloads the whole page — like the F# bridge's
//!   `check_hot_reload` no-op);
//! - no per-frame perf stats (`std::time::Instant` panics on wasm; the C6
//!   perf gate measures natively);
//! - input events arrive from the page via the `mle_*` wasm exports below,
//!   queued and drained by the frame loop each frame before `tick` (DOM
//!   handlers fire between rAF callbacks, never mid-frame).

use std::cell::RefCell;

use functor_runtime_common::mle_prelude::{
    drain_effects, frame_value, physics_scene_value, split_model_effect, sub_messages_for_frame,
    EffectLog, FunctorHost, RealEffects,
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
    /// Performs `Effect.*` commands (B6). `RealEffects` is wasm-safe: its
    /// clock is `Date.now()` on this target.
    effect_runner: RealEffects,
    /// The structured effect log (bounded) — see the desktop producer.
    effect_log: EffectLog,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// The last per-frame error logged, to avoid flooding the console at
    /// 60fps with the same message (a persistent error logs once until it
    /// changes).
    last_error: Option<String>,
}

impl MleWebGame {
    /// Load, check, and contract-validate fetched game source — the web
    /// counterpart of the desktop `load_game`. Errors come back as fully
    /// rendered strings (`path:line:col: message`) for `run_async` to fail
    /// loud with (there is no keep-running fallback: a page load either gets
    /// a valid game or a console error).
    pub fn create(path: &str, src: String) -> Result<MleWebGame, String> {
        let render = |stage: &str, span: mle::Span, message: &str| {
            let (line, col) = mle::line_col(&src, span.start);
            format!("cannot {stage} {path}:{line}:{col}: {message}")
        };
        let program = mle::parse(&src).map_err(|e| render("parse", e.span, &e.message))?;
        let module = mle::lower(program).map_err(|e| render("load", e.span, &e.message))?;
        // Type diagnostics are advisory in the dev loop: warn, keep going
        // (the CLI's `build` is the strict gate).
        for diag in mle::check(&module) {
            let (line, col) = mle::line_col(&src, diag.span.start);
            web_sys::console::warn_1(
                &format!("warning: {path}:{line}:{col}: {}", diag.message).into(),
            );
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
        web_sys::console::log_1(&format!("[mle] loaded {path}").into());
        Ok(MleWebGame {
            path: path.to_string(),
            src,
            session,
            model: init,
            has_input,
            has_mouse_move,
            has_mouse_wheel,
            has_subscriptions,
            prev_tts: None,
            effect_runner: RealEffects::new(),
            effect_log: EffectLog::new(),
            has_physics,
            last_frame: empty_frame(),
            last_error: None,
        })
    }

    /// Fire subscription timers over `(prev_tts, tts]` and fold their
    /// messages through `update`, before this frame's `tick` — the message
    /// drain seam (docs/mle.md C4b-2), same semantics as the desktop
    /// producer. Errors report per message and processing continues.
    /// Take an entry point's return: split off any `(model, effect)` pair,
    /// adopt the model, and drain the effects to a fixed point through
    /// `update` (docs/mle.md B6) — mirrors the desktop producer.
    fn absorb(&mut self, returned: Value) {
        const EFFECT_LOG_CAP: usize = 256;
        let (model, effects) = split_model_effect(returned);
        self.model = model;
        let Some(effects) = effects else { return };
        if self.session.global("update").is_none() {
            self.log_once(
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
            self.log_once(message);
        }
        if self.effect_log.len() > EFFECT_LOG_CAP {
            let excess = self.effect_log.len() - EFFECT_LOG_CAP;
            self.effect_log.drain(..excess);
        }
    }

    fn pump_subscriptions(&mut self, tts: f64) {
        // Advance the window even without subscriptions (or on frame one),
        // so the edge is always sane.
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
                None => self.log_once(format!(
                    "[mle] physics must return Physics.scene(gx, gy, gz, [body, …]), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.frame_error("physics", &err),
        }
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
    // Hot reload is native-only (docs/mle.md C3); on web, reload the page.
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

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
        self.step_physics(frame_time.dts);
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
        // On failure this is the last good frame — a bad draw must not blank
        // the canvas.
        self.last_frame.clone()
    }

    fn ui(&self) -> View {
        View::empty()
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
