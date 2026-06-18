use functor_runtime_common::{Frame, FrameTime};

pub trait Game {
    fn check_hot_reload(&mut self, frame_time: FrameTime);

    fn tick(&mut self, frame_time: FrameTime);

    /// Deliver a keyboard event. `code` is a functor_runtime_common::Key as i32.
    fn key_event(&mut self, code: i32, is_down: bool);

    /// Deliver a mouse-move event in window pixel coordinates.
    fn mouse_move(&mut self, x: i32, y: i32);

    /// Deliver a mouse-wheel event (vertical scroll offset).
    fn mouse_wheel(&mut self, delta: i32);

    fn render(&mut self, frame_time: FrameTime) -> Frame;

    /// A pretty-printed (Rust `Debug`) view of the live game model, for
    /// introspection. Produced by the game dylib's `emit_state_debug` export;
    /// works for any game because Fable derives `Debug` on every generated type.
    fn state_debug(&self) -> String;

    /// Take the networking commands the game has queued this frame (a JSON array
    /// of `functor_runtime_common::net::NetCommand`), via the dylib's
    /// `net_drain_commands_json` export. The host performs the I/O and reports
    /// results back with `net_push_http_response` / `net_push_http_error`.
    fn net_drain_commands(&self) -> String;

    /// Deliver a completed HTTP response into the game's async inbox.
    fn net_push_http_response(&mut self, token: i32, status: i32, body: String);

    /// Deliver a transport-level failure for a request into the async inbox.
    fn net_push_http_error(&mut self, token: i32, message: String);

    /// Take the audio commands the game queued this frame (a JSON array of
    /// `functor_runtime_common::audio::AudioCommand`), via the dylib's
    /// `audio_drain_commands_json` export. The host plays them on its own device.
    fn audio_drain_commands(&self) -> String;

    fn quit(&mut self);
}
