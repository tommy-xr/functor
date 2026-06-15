use cgmath::{vec2, vec3};

use crate::render::VertexPositionTexture;

use super::{Geometry, IndexedMesh};

/// A unit square in the XZ plane (y = 0), centered at the origin — the ground.
/// Y-up convention, so this lies flat; size it with `Transform.scale`.
pub struct Plane;

impl Plane {
    pub fn create() -> Box<dyn Geometry> {
        // Lies in the XZ ground plane, so the normal points straight up (+Y).
        let normal = vec3(0.0, 1.0, 0.0);
        let vertices = vec![
            VertexPositionTexture {
                position: vec3(-0.5, 0.0, -0.5),
                uv: vec2(0.0, 0.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(0.5, 0.0, -0.5),
                uv: vec2(1.0, 0.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(0.5, 0.0, 0.5),
                uv: vec2(1.0, 1.0),
                normal,
            },
            VertexPositionTexture {
                position: vec3(-0.5, 0.0, 0.5),
                uv: vec2(0.0, 1.0),
                normal,
            },
        ];
        let indices = vec![0, 1, 2, 2, 3, 0];
        Box::new(IndexedMesh::create(vertices, indices))
    }
}
