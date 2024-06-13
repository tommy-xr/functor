use functor_runtime_common::{FrameTime, Scene3D};

pub trait Game {
    fn check_hot_reload(&mut self, frame_time: FrameTime);

    fn render(&mut self, frame_time: FrameTime) -> Scene3D;
}
