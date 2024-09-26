mod skeleton;

use crate::{geometry::IndexedMesh, render::VertexPositionTexture, texture::Texture2D};
use cgmath::Matrix4;

pub use skeleton::*;

pub struct ModelMesh {
    // Material info
    pub base_color_texture: Texture2D,

    pub mesh: IndexedMesh<VertexPositionTexture>,

    pub transform: Matrix4<f32>,
}

pub struct Model {
    pub meshes: Vec<ModelMesh>,

    pub skeleton: Skeleton,
}
