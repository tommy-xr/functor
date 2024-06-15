use cgmath::Matrix4;

pub trait Material {
    fn initialize(&mut self, ctx: &RenderContext);
    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool;
    fn draw_transparent(
        &self,
        _ctx: &RenderContext,
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
#[allow(unused_imports)]
pub use color_material::*;

use crate::RenderContext;
