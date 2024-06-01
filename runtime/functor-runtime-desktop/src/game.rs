use functor_runtime_common::Scene3D;

pub trait Game {
    fn render(&mut self) -> Scene3D;
}
