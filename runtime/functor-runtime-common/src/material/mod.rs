use cgmath::Matrix4;

pub trait Material {
    fn initialize(&mut self, gl: &glow::Context, opengl_version: &str);
    fn draw_opaque(
        &self,
        gl: &glow::Context,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool;
    fn draw_transparent(
        &self,
        _gl: &glow::Context,
        _projection_matrix: &Matrix4<f32>,
        _view_matrix: &Matrix4<f32>,
        _world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        false
    }
}

mod basic_material;
mod color_material;

pub use basic_material::*;
pub use color_material::*;
