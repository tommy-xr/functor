use functor_runtime_common::{FrameTime, Scene3D};

pub trait Game {
    fn render(&mut self, frame_time: FrameTime) -> Scene3D;
}
