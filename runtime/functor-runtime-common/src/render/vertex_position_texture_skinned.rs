use cgmath::{Vector2, Vector3, Vector4};
use std::mem::{offset_of, size_of};

use super::vertex::{BuiltInVertexChannel, Vertex, VertexAttribute, VertexAttributeType};

#[derive(Debug, Clone)]
#[repr(C)]
pub struct VertexPositionTextureSkinned {
    pub position: Vector3<f32>,
    pub uv: Vector2<f32>,
    pub joint_indices: Vector4<f32>,
    pub weights: Vector4<f32>,
}

impl Vertex for VertexPositionTextureSkinned {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTextureSkinned>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        let vec = vec![
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Position,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, position),
                size: 3,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Uv,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, uv),
                size: 2,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::JointIndices,
                // TODO: Can we switch this to byte instead? Or a packed representation?
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, joint_indices),
                size: 4,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::JointWeights,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, weights),
                size: 4,
            },
        ];
        vec
    }
}
