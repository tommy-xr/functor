use cgmath::Matrix4;

use crate::{geometry::IndexedMesh, render::vertex::VertexPositionTexture, texture::Texture2D};

pub struct ModelMesh {
    // Material info
    pub base_color_texture: Texture2D,

    pub mesh: IndexedMesh<VertexPositionTexture>,

    pub transform: Matrix4<f32>,
}

pub struct Model {
    pub meshes: Vec<ModelMesh>,
}
