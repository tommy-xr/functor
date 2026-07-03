//! A frame-replay producer (docs/mle.md Track A3): plays back recorded
//! [`Frame`]s from a JSON file instead of running any game logic. The file is
//! either a single serialized `Frame` or a JSON array of them — the exact wire
//! format `GET /scene` emits — so a `/scene` capture can be replayed verbatim.
//! A trivial second `GameProducer` impl proving the seam is producer-agnostic.

use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};

use crate::game::Game;

/// Playback rate for multi-frame recordings: the rendered frame is
/// `floor(tts * PLAYBACK_FPS) % len` — 10 recorded frames per second of game
/// time, wrapping at the end. (A single-frame recording shows that frame
/// forever.)
const PLAYBACK_FPS: f32 = 10.0;

pub struct ReplayGame {
    path: String,
    frames: Vec<Frame>,
    /// Index of the most recently rendered frame, surfaced by `state_debug`.
    index: usize,
}

fn load_error(path: &str, message: String) -> ! {
    eprintln!("error: cannot replay {path}: {message}");
    std::process::exit(1);
}

impl ReplayGame {
    pub fn create(path: &str) -> ReplayGame {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| load_error(path, e.to_string()));
        // A recording is either a single Frame or an array of them. Pick the
        // parse by the file's shape so a malformed recording reports the
        // error from the parse that was actually intended (a bad frame inside
        // an array must not surface as "a sequence is not a Frame").
        let frames: Vec<Frame> = if src.trim_start().starts_with('[') {
            serde_json::from_str::<Vec<Frame>>(&src)
                .unwrap_or_else(|e| load_error(path, format!("bad Frame array: {e}")))
        } else {
            vec![serde_json::from_str::<Frame>(&src)
                .unwrap_or_else(|e| load_error(path, format!("bad Frame: {e}")))]
        };
        if frames.is_empty() {
            load_error(path, "the recording contains no frames".to_string());
        }
        println!("[replay] loaded {} frame(s) from {path}", frames.len());
        ReplayGame {
            path: path.to_string(),
            frames,
            index: 0,
        }
    }
}

impl Game for ReplayGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn tick(&mut self, _frame_time: FrameTime) {}

    fn key_event(&mut self, _code: i32, _is_down: bool) {}
    fn mouse_move(&mut self, _x: i32, _y: i32) {}
    fn mouse_wheel(&mut self, _delta: i32) {}

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        self.index = (frame_time.tts * PLAYBACK_FPS).floor().max(0.0) as usize % self.frames.len();
        self.frames[self.index].clone()
    }

    fn ui(&self) -> View {
        View::empty()
    }

    fn state_debug(&self) -> String {
        format!(
            "<replay of {} frame(s) from {}; current index {}>",
            self.frames.len(),
            self.path,
            self.index
        )
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
