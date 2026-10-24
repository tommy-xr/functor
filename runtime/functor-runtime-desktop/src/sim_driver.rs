//! The shell's one handle on the game producer.
//!
//! Every producer call goes through [`SimDriver`] as an ordered message:
//! `post` (and the named input methods) for fire-and-forget input and events,
//! `call` for queries that need an answer, and [`SimDriver::frame`] for the
//! per-frame [`SimSnapshot`] the render half consumes. The shell keeps the
//! clock and the fixed-step decomposition, so the producer sees the same call
//! stream however it is driven. Closures are `Send + 'static` so the producer
//! can later live on its own thread without changing a call site.

use std::sync::Arc;

use functor_runtime_common::audio::{AudioCommand, AudioScene};
use functor_runtime_common::net::{ConnCommand, NetCommand};
use functor_runtime_common::physics::DebugLine;
use functor_runtime_common::ui::{UiEvent, View};
use functor_runtime_common::{Frame, FrameTime, InputSnapshot};

use crate::game::Game;

/// What the shell wants back from one [`SimDriver::frame`] beyond the frame
/// itself. Optional parts cost producer work, so the shell asks only for what
/// it will show.
pub(crate) struct FrameRequest {
    pub time: FrameTime,
    pub ui: bool,
    pub webview: bool,
    pub audio_scene: bool,
    pub physics_lines: bool,
    /// False under the embedder transport: connection commands stay queued
    /// for the debug server's `GET /net/outbound`.
    pub conn_commands: bool,
}

/// Producer capabilities the shell routes input by. Read from the latest
/// snapshot, so a hot reload that adds or removes a hook takes effect one
/// frame later.
#[derive(Clone, Copy, Default)]
pub(crate) struct SimCaps {
    pub samples_input: bool,
    pub uses_captured_mouse_input: bool,
}

/// Everything the render half needs from one sim frame: the drawn frame, the
/// effects it queued, and the timeline/capability reads the shell's chrome
/// shows. Plain data; nothing here points back into the producer.
pub(crate) struct SimSnapshot {
    /// The time the frame was drawn at, which render-time animation samples.
    pub time: FrameTime,
    pub frame: Arc<Frame>,
    pub ui: View,
    pub webview_html: Option<String>,
    pub audio_commands: Vec<AudioCommand>,
    pub audio_scene: Option<AudioScene>,
    pub net_commands: Vec<NetCommand>,
    pub conn_commands: Vec<ConnCommand>,
    pub preload_commands: Vec<functor_runtime_common::asset::preload::PreloadCommand>,
    pub physics_lines: Vec<DebugLine>,
    pub scene_frame: Option<u64>,
    pub scene_range: Option<(u64, u64)>,
    pub program_revision: u64,
    pub timeline_generation: u64,
    pub caps: SimCaps,
}

pub(crate) struct SimDriver {
    game: Box<dyn Game>,
}

impl SimDriver {
    /// Drive `game` on the calling thread.
    pub fn inline(game: Box<dyn Game>) -> Self {
        Self { game }
    }

    /// Queue a call; it runs after everything queued before it.
    pub fn post(&mut self, f: impl FnOnce(&mut dyn Game) + Send + 'static) {
        f(&mut *self.game);
    }

    /// Run a call after everything queued before it and return its result.
    pub fn call<R: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut dyn Game) -> R + Send + 'static,
    ) -> R {
        f(&mut *self.game)
    }

    pub fn caps(&mut self) -> SimCaps {
        self.call(|g| caps(g))
    }

    /// Draw the frame and collect what the shell consumes from it.
    pub fn frame(&mut self, req: FrameRequest) -> SimSnapshot {
        self.call(move |g| snapshot(g, req))
    }

    pub fn key_event(&mut self, code: i32, is_down: bool) {
        self.post(move |g| g.key_event(code, is_down));
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) {
        self.post(move |g| g.mouse_move(x, y));
    }

    pub fn mouse_wheel(&mut self, delta: i32) {
        self.post(move |g| g.mouse_wheel(delta));
    }

    pub fn mouse_button(&mut self, button: i32, is_down: bool) {
        self.post(move |g| g.mouse_button(button, is_down));
    }

    pub fn ui_event(&mut self, event: UiEvent) {
        self.post(move |g| g.ui_event(event));
    }

    pub fn webview_event(&mut self, event: UiEvent) {
        self.post(move |g| g.webview_event(event));
    }

    pub fn sampled_input(&mut self, snapshot: InputSnapshot) {
        self.post(move |g| g.sampled_input(&snapshot));
    }

    pub fn tick(&mut self, frame_time: FrameTime) {
        self.post(move |g| g.tick(frame_time));
    }

    pub fn quit(mut self) {
        self.game.quit();
    }
}

fn caps(game: &dyn Game) -> SimCaps {
    SimCaps {
        samples_input: game.samples_input(),
        uses_captured_mouse_input: game.uses_captured_mouse_input(),
    }
}

/// Parse a drained JSON command array; a malformed drain logs and drops.
fn parse_drain<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Vec<T> {
    if json == "[]" {
        return Vec::new();
    }
    serde_json::from_str(json).unwrap_or_else(|e| {
        eprintln!("[{what}] bad commands json: {e}");
        Vec::new()
    })
}

fn snapshot(game: &mut dyn Game, req: FrameRequest) -> SimSnapshot {
    let frame = Arc::new(game.render(req.time.clone()));
    let audio_scene = req.audio_scene.then(|| {
        serde_json::from_str::<AudioScene>(&game.audio_scene_json())
            .map_err(|e| eprintln!("[audio] bad scene json: {e}"))
            .ok()
    });
    SimSnapshot {
        time: req.time,
        frame,
        ui: if req.ui { game.ui() } else { View::Empty },
        webview_html: req
            .webview
            .then(|| game.webview().map(|node| node.to_html()))
            .flatten(),
        audio_commands: parse_drain(&game.audio_drain_commands(), "audio"),
        audio_scene: audio_scene.flatten(),
        net_commands: parse_drain(&game.net_drain_commands(), "net"),
        conn_commands: if req.conn_commands {
            parse_drain(&game.net_drain_conn_commands(), "net")
        } else {
            Vec::new()
        },
        preload_commands: serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default(),
        physics_lines: if req.physics_lines {
            functor_runtime_common::physics::with_world(
                functor_runtime_common::physics::DEFAULT_WORLD,
                |w| w.debug_lines(),
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        },
        scene_frame: game.current_scene_frame(),
        scene_range: game.scene_frame_range(),
        program_revision: game.scene_program_revision(),
        timeline_generation: game.scene_timeline_generation(),
        caps: caps(game),
    }
}
