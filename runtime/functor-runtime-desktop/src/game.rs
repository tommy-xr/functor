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

    fn quit(&mut self);
}
