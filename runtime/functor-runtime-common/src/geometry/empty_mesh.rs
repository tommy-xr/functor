use super::Geometry;

pub struct EmptyMesh;

impl Geometry for EmptyMesh {
    fn draw(&mut self, _gl: &glow::Context) {
        // do nothing, the mesh is empty
    }
}
