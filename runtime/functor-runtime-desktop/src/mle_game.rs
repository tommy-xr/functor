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
    session: Session,
    model: Value,
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
        let model = session.global("init").unwrap_or_else(|| {
            eprintln!("error: {path} has no top-level `let init = …`");
            std::process::exit(1);
        });
        println!("[mle] loaded {path}");
        MleGame {
            path: path.to_string(),
            session,
            model,
            frames: 0,
            tick_ns: 0,
            draw_ns: 0,
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
            Err(err) => eprintln!("[mle] tick error in {}: {}", self.path, err.message),
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
        let frame = match self.session.call("draw", args, &mut FunctorHost) {
            Ok(value) => match frame_value(&value) {
                Some(frame) => frame.clone(),
                None => {
                    eprintln!(
                        "[mle] draw must return Frame.create(camera, scene), got {}",
                        value.kind_name()
                    );
                    empty_frame()
                }
            },
            Err(err) => {
                eprintln!("[mle] draw error in {}: {}", self.path, err.message);
                empty_frame()
            }
        };
        self.draw_ns += started.elapsed().as_nanos() as u64;
        frame
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
