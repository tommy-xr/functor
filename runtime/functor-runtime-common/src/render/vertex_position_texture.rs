use cgmath::{Vector2, Vector3, Vector4};
use std::mem::{offset_of, size_of};

use super::vertex::{BuiltInVertexChannel, Vertex, VertexAttribute, VertexAttributeType};

#[derive(Debug, Clone)]
#[repr(C)]
pub struct VertexPositionTexture {
    pub position: Vector3<f32>,
    pub uv: Vector2<f32>,
    pub normal: Vector3<f32>,
    /// Tangent for normal mapping: `xyz` is the surface tangent, `w` is the
    /// handedness (±1) of the bitangent (`cross(normal, tangent.xyz) * w`),
    /// matching glTF's `TANGENT` convention.
    pub tangent: Vector4<f32>,
}

impl VertexPositionTexture {
    /// Construct a vertex with a zeroed tangent placeholder. Call
    /// [`crate::geometry::compute_tangents`] over the assembled mesh to fill the
    /// tangents in from positions/uvs/normals.
    pub fn new(position: Vector3<f32>, uv: Vector2<f32>, normal: Vector3<f32>) -> Self {
        VertexPositionTexture {
            position,
            uv,
            normal,
            tangent: Vector4::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl Vertex for VertexPositionTexture {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTexture>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        // Order matters: the index in this Vec is the shader attribute
        // location (see `IndexedMesh::hydrate`). Position = 0, Uv = 1,
        // Normal = 2, Tangent = 3.
        let vec = vec![
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Position,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTexture, position),
                size: 3,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Uv,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTexture, uv),
                size: 2,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Normal,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTexture, normal),
                size: 3,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Tangent,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTexture, tangent),
                size: 4,
            },
        ];
        vec
    }
}
