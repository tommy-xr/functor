use cgmath::{vec2, vec3};

use crate::render::VertexPositionTexture;

use super::{Geometry, IndexedMesh};

/// A unit square in the XY plane (z = 0), centered at the origin, facing +Z.
/// The atom for sprites / billboards / UI; size it with `Transform.scale`.
pub struct Quad;

impl Quad {
    pub fn create() -> Box<dyn Geometry> {
        // Faces +Z (XY plane), so every vertex shares that normal.
        let normal = vec3(0.0, 0.0, 1.0);
        let vertices = vec![
            VertexPositionTexture {
                position: vec3(-0.5, -0.5, 0.0),
                uv: vec2(0.0, 0.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(0.5, -0.5, 0.0),
                uv: vec2(1.0, 0.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(0.5, 0.5, 0.0),
                uv: vec2(1.0, 1.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(-0.5, 0.5, 0.0),
                uv: vec2(0.0, 1.0),
                normal,
            },
        ];
        let indices = vec![0, 1, 2, 2, 3, 0];
        Box::new(IndexedMesh::create(vertices, indices))
    }
}
