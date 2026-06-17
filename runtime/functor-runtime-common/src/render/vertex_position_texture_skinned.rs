use cgmath::{Vector2, Vector3, Vector4};
use std::mem::{offset_of, size_of};

use super::vertex::{BuiltInVertexChannel, Vertex, VertexAttribute, VertexAttributeType};

#[derive(Debug, Clone)]
#[repr(C)]
pub struct VertexPositionTextureSkinned {
    pub position: Vector3<f32>,
    pub uv: Vector2<f32>,
    pub normal: Vector3<f32>,
    /// Tangent for normal mapping (`xyz` + handedness `w`), glTF `TANGENT`
    /// convention. Sits at location 3 (matching `VertexPositionTexture`), so
    /// joints/weights follow at 4/5.
    pub tangent: Vector4<f32>,
    pub joint_indices: Vector4<f32>,
    pub weights: Vector4<f32>,
}

impl Vertex for VertexPositionTextureSkinned {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTextureSkinned>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        // Order is the shader attribute location. Normal sits at location 2 and
        // tangent at 3 to match `VertexPositionTexture` (same channel = same
        // location in both formats); joints/weights follow at 4/5.
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
                attribute_channel: BuiltInVertexChannel::Normal,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, normal),
                size: 3,
            },
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Tangent,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, tangent),
                size: 4,
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
