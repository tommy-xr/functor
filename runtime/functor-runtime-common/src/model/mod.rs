mod data;
mod skeleton;

use crate::{
    animation::Animation, geometry::IndexedMesh, render::VertexPositionTextureSkinned,
    texture::Texture2D,
};
use cgmath::Matrix4;

pub use data::*;
pub use skeleton::*;

/// The joint palette size of every skinned shader (`#define MAX_JOINTS`).
pub const MAX_JOINTS: usize = 200;

pub struct ModelMesh {
    // Material info
    pub base_color_texture: Texture2D,

    pub mesh: IndexedMesh<VertexPositionTextureSkinned>,

    pub transform: Matrix4<f32>,
}

pub struct Model {
    pub meshes: Vec<ModelMesh>,

    pub skeleton: Skeleton,

    pub animations: Vec<Animation>,
}

impl Model {
    /// Free the GPU objects hydrated so far; the model must not draw again.
    pub(crate) fn delete(&self, gl: &glow::Context) {
        for mesh in &self.meshes {
            mesh.mesh.delete(gl);
            mesh.base_color_texture.delete(gl);
        }
    }
}

#[cfg(test)]
mod tests {
    /// The shaders hard-code the palette size; keep them in step with
    /// `MAX_JOINTS`, which `Model.skinned` validates against.
    #[test]
    fn skinned_shaders_declare_max_joints() {
        let define = format!("#define MAX_JOINTS {}", super::MAX_JOINTS);
        for (file, source) in [
            ("skinned_material", include_str!("../material/skinned_material.rs")),
            ("skinned_depth_material", include_str!("../material/skinned_depth_material.rs")),
            (
                "skinned_normal_debug_material",
                include_str!("../material/skinned_normal_debug_material.rs"),
            ),
            (
                "skinned_tangent_debug_material",
                include_str!("../material/skinned_tangent_debug_material.rs"),
            ),
            ("instanced_renderer", include_str!("../scene3d/instanced_renderer.rs")),
        ] {
            assert!(source.contains(&define), "{file} must declare `{define}`");
        }
    }
}
