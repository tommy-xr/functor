use super::{
    indexed_mesh,
    mesh::{self, Mesh},
    IndexedMesh,
};

pub struct Cube;

impl Cube {
    pub fn create() -> IndexedMesh {
        // Vertices of a cube (position and texture coordinates)
        let vertices: Vec<f32> = vec![
            // Front face
            -0.5, -0.5, 0.5, 0.0, 0.0, 0.5, -0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, -0.5, 0.5,
            0.5, 0.0, 1.0, // Back face
            -0.5, -0.5, -0.5, 0.0, 0.0, 0.5, -0.5, -0.5, 1.0, 0.0, 0.5, 0.5, -0.5, 1.0, 1.0, -0.5,
            0.5, -0.5, 0.0, 1.0,
        ];

        // Indices of the cube (two triangles per face)
        let indices: Vec<u32> = vec![
            // Front face
            0, 1, 2, 2, 3, 0, // Back face
            4, 5, 6, 6, 7, 4, // Left face
            4, 0, 3, 3, 7, 4, // Right face
            1, 5, 6, 6, 2, 1, // Top face
            3, 2, 6, 6, 7, 3, // Bottom face
            4, 5, 1, 1, 0, 4,
        ];
        indexed_mesh::create(vertices, indices)
    }
}
