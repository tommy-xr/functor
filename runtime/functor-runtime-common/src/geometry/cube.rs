use cgmath::{vec2, vec3, Vector3};

use crate::render::VertexPositionTexture;

use super::{Geometry, IndexedMesh};

pub struct Cube;

impl Cube {
    pub fn create() -> Box<dyn Geometry> {
        // 24 vertices (4 per face) so each face carries its own flat normal —
        // shared corner vertices could only hold averaged/radial normals, which
        // would smooth-shade the cube under lighting. Extents are -0.5..0.5.
        let faces: [(Vector3<f32>, [Vector3<f32>; 4]); 6] = [
            // +Z (front)
            (
                vec3(0.0, 0.0, 1.0),
                [
                    vec3(-0.5, -0.5, 0.5),
                    vec3(0.5, -0.5, 0.5),
                    vec3(0.5, 0.5, 0.5),
                    vec3(-0.5, 0.5, 0.5),
                ],
            ),
            // -Z (back)
            (
                vec3(0.0, 0.0, -1.0),
                [
                    vec3(0.5, -0.5, -0.5),
                    vec3(-0.5, -0.5, -0.5),
                    vec3(-0.5, 0.5, -0.5),
                    vec3(0.5, 0.5, -0.5),
                ],
            ),
            // +X (right)
            (
                vec3(1.0, 0.0, 0.0),
                [
                    vec3(0.5, -0.5, 0.5),
                    vec3(0.5, -0.5, -0.5),
                    vec3(0.5, 0.5, -0.5),
                    vec3(0.5, 0.5, 0.5),
                ],
            ),
            // -X (left)
            (
                vec3(-1.0, 0.0, 0.0),
                [
                    vec3(-0.5, -0.5, -0.5),
                    vec3(-0.5, -0.5, 0.5),
                    vec3(-0.5, 0.5, 0.5),
                    vec3(-0.5, 0.5, -0.5),
                ],
            ),
            // +Y (top)
            (
                vec3(0.0, 1.0, 0.0),
                [
                    vec3(-0.5, 0.5, 0.5),
                    vec3(0.5, 0.5, 0.5),
                    vec3(0.5, 0.5, -0.5),
                    vec3(-0.5, 0.5, -0.5),
                ],
            ),
            // -Y (bottom)
            (
                vec3(0.0, -1.0, 0.0),
                [
                    vec3(-0.5, -0.5, -0.5),
                    vec3(0.5, -0.5, -0.5),
                    vec3(0.5, -0.5, 0.5),
                    vec3(-0.5, -0.5, 0.5),
                ],
            ),
        ];

        let uvs = [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)];

        let mut vertices: Vec<VertexPositionTexture> = Vec::with_capacity(24);
        let mut indices: Vec<u32> = Vec::with_capacity(36);

        for (normal, corners) in faces {
            let base = vertices.len() as u32;
            for (corner, uv) in corners.iter().zip(uvs.iter()) {
                vertices.push(VertexPositionTexture {
                    position: *corner,
                    uv: *uv,
                    normal,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }

        Box::new(IndexedMesh::create(vertices, indices))
    }
}
