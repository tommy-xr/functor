use cgmath::Matrix4;

pub trait Material {
    fn has_initialized(&self) -> bool;
    fn initialize(&mut self, is_opengl_es: bool, storage: &Box<dyn crate::file_system::Storage>);
    fn draw_opaque(
        &self,
        projection_matrix: &EngineRenderContext,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool;
    fn draw_transparent(
        &self,
        projection_matrix: &EngineRenderContext,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool {
        false
    }
}
