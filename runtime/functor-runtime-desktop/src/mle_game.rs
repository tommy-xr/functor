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
//! ```
//!
//! The model is a plain MLE value the host holds between frames — the
//! serializable-state seam hot-reload (C3) will swap sessions around.
//! Per-frame errors print and keep the previous model/frame (a bad frame
//! must not kill the session); load errors fail loud at startup.

use std::time::Instant;

use functor_runtime_common::mle_prelude::{frame_value, FunctorHost};
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};
use mle::{Session, Value};

use crate::game::Game;

pub struct MleGame {
    path: String,
    src: String,
    session: Session,
    model: Value,
    /// The last successfully drawn frame, kept so a bad draw shows the last
    /// good picture instead of a blank.
    last_frame: Frame,
    /// The last per-frame error printed, to avoid flooding stderr at 60fps
    /// with the same message (a persistent error prints once until it
    /// changes).
    last_error: Option<String>,
    // rolling per-frame eval cost, printed every STATS_EVERY frames (the C6
    // perf gate watches these).
    frames: u64,
    tick_ns: u64,
    draw_ns: u64,
}

const STATS_EVERY: u64 = 300;

fn fail(path: &str, stage: &str, src: &str, span: mle::Span, message: &str) -> ! {
    let (line, col) = mle::line_col(src, span.start);
    eprintln!("error: cannot {stage} {path}:{line}:{col}: {message}");
    std::process::exit(1);
}

impl MleGame {
    pub fn create(path: &str) -> MleGame {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error: cannot read {path}: {e}");
            std::process::exit(1);
        });
        let program = match mle::parse(&src) {
            Ok(program) => program,
            Err(err) => fail(path, "parse", &src, err.span, &err.message),
        };
        let module = match mle::lower(program) {
            Ok(module) => module,
            Err(err) => fail(path, "load", &src, err.span, &err.message),
        };
        // Type diagnostics are advisory in the dev loop: print, keep going.
        for diag in mle::check(&module) {
            let (line, col) = mle::line_col(&src, diag.span.start);
            eprintln!("warning: {path}:{line}:{col}: {}", diag.message);
        }
        let session = match Session::load(&module, &mut FunctorHost) {
            Ok(session) => session,
            Err(failure) => fail(
                path,
                "load",
                &src,
                failure.error.span,
                &failure.error.message,
            ),
        };
        // The producer contract is knowable at load — fail loud here, not
        // once per frame: `init` must be a model VALUE, `tick`/`draw` must
        // be functions of the right arity.
        let model = session.global("init").unwrap_or_else(|| {
            eprintln!("error: {path} has no top-level `let init = …`");
            std::process::exit(1);
        });
        if matches!(
            model,
            Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_)
        ) {
            eprintln!("error: {path}: `init` must be a model value, not a function");
            std::process::exit(1);
        }
        require_function(path, &session, "tick", 3);
        require_function(path, &session, "draw", 2);
        println!("[mle] loaded {path}");
        MleGame {
            path: path.to_string(),
            src,
            session,
            model,
            last_frame: empty_frame(),
            last_error: None,
            frames: 0,
            tick_ns: 0,
            draw_ns: 0,
        }
    }

    /// Report a per-frame error with its source position, once per distinct
    /// message (a 60fps loop must not flood stderr with one persistent bug).
    fn frame_error(&mut self, stage: &str, err: &mle::RunError) {
        let (line, col) = mle::line_col(&self.src, err.span.start);
        let rendered = format!(
            "[mle] {stage} error at {}:{line}:{col}: {}",
            self.path, err.message
        );
        if self.last_error.as_deref() != Some(rendered.as_str()) {
            eprintln!("{rendered}");
            self.last_error = Some(rendered);
        }
    }

    fn report_stats(&mut self) {
        if self.frames > 0 && self.frames % STATS_EVERY == 0 {
            let tick_us = self.tick_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let draw_us = self.draw_ns as f64 / STATS_EVERY as f64 / 1000.0;
            println!(
                "[mle] avg over {STATS_EVERY} frames: tick {tick_us:.1}µs, draw {draw_us:.1}µs \
                 ({:.1}% of a 60fps budget)",
                (tick_us + draw_us) / 16_666.0 * 100.0
            );
            self.tick_ns = 0;
            self.draw_ns = 0;
        }
    }
}

impl Game for MleGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        // C3: file-watch → reparse → new Session, model preserved.
    }

    fn tick(&mut self, frame_time: FrameTime) {
        let started = Instant::now();
        let args = vec![
            self.model.clone(),
            Value::Number(frame_time.dts as f64),
            Value::Number(frame_time.tts as f64),
        ];
        match self.session.call("tick", args, &mut FunctorHost) {
            Ok(model) => self.model = model,
            Err(err) => self.frame_error("tick", &err),
        }
        self.tick_ns += started.elapsed().as_nanos() as u64;
        self.frames += 1;
        self.report_stats();
    }

    fn key_event(&mut self, _code: i32, _is_down: bool) {}
    fn mouse_move(&mut self, _x: i32, _y: i32) {}
    fn mouse_wheel(&mut self, _delta: i32) {}

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        let started = Instant::now();
        let args = vec![self.model.clone(), Value::Number(frame_time.tts as f64)];
        match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => self.last_frame = frame.clone(),
                None => {
                    let rendered = format!(
                        "[mle] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    if self.last_error.as_deref() != Some(rendered.as_str()) {
                        eprintln!("{rendered}");
                        self.last_error = Some(rendered);
                    }
                }
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

/// Exit loud at load if `name` is not a function of `arity` params — the
/// alternative is one error per frame, forever.
fn require_function(path: &str, session: &Session, name: &str, arity: usize) {
    match session.global(name) {
        Some(Value::Closure(closure)) if closure.params.len() == arity => {}
        Some(Value::Closure(closure)) => {
            eprintln!(
                "error: {path}: `{name}` must take {arity} parameter(s), takes {}",
                closure.params.len()
            );
            std::process::exit(1);
        }
        Some(other) => {
            eprintln!(
                "error: {path}: `{name}` must be a function, got {}",
                other.kind_name()
            );
            std::process::exit(1);
        }
        None => {
            eprintln!("error: {path} has no top-level `let {name} = …`");
            std::process::exit(1);
        }
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
