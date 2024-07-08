use cgmath::{vec2, vec3};

use crate::render::vertex::VertexPositionTexture;

use super::{
    indexed_mesh,
    mesh::{self, MeshData},
    Geometry, IndexedMeshData,
};

pub struct Cube;

impl Cube {
    pub fn create() -> Box<dyn Geometry> {
        let vertices = vec![
            // Front face
            VertexPositionTexture {
                position: vec3(-1.0, -1.0, 1.0),
                uv: vec2(0.0, 0.0),
            },
            VertexPositionTexture {
                position: vec3(1.0, -1.0, 1.0),
                uv: vec2(1.0, 0.0),
            },
            VertexPositionTexture {
                position: vec3(1.0, 1.0, 1.0),
                uv: vec2(1.0, 1.0),
            },
            VertexPositionTexture {
                position: vec3(-1.0, 1.0, 1.0),
                uv: vec2(0.0, 1.0),
            },
            // Back face
            VertexPositionTexture {
                position: vec3(-1.0, -1.0, -1.0),
                uv: vec2(1.0, 0.0),
            },
            VertexPositionTexture {
                position: vec3(1.0, -1.0, -1.0),
                uv: vec2(0.0, 0.0),
            },
            VertexPositionTexture {
                position: vec3(1.0, 1.0, -1.0),
                uv: vec2(0.0, 1.0),
            },
            VertexPositionTexture {
                position: vec3(-1.0, 1.0, -1.0),
                uv: vec2(1.0, 1.0),
            },
        ];

        let indices = vec![
            // Front face
            0, 1, 2, 2, 3, 0, // Top face
            3, 2, 6, 6, 7, 3, // Back face
            7, 6, 5, 5, 4, 7, // Bottom face
            4, 5, 1, 1, 0, 4, // Left face
            4, 0, 3, 3, 7, 4, // Right face
            1, 5, 6, 6, 2, 1,
        ];

        let verts = vertices
            .into_iter()
            .map(|v| VertexPositionTexture {
                position: v.position / 2.0,
                uv: v.uv,
            })
            .collect();

        Box::new(indexed_mesh::create(verts, indices))
    }
}
