mod skeleton;

use crate::{
    animation::Animation, geometry::IndexedMesh, render::VertexPositionTextureSkinned,
    texture::Texture2D,
};
use cgmath::Matrix4;

pub use skeleton::*;

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
