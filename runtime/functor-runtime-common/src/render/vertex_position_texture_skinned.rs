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
    /// glTF `COLOR_0` (linear RGBA), multiplied into the base colour; white
    /// when the mesh has none.
    pub color: Vector4<f32>,
}

/// `color`'s attribute location in a mesh's own VAO (its attribute index).
pub const VERTEX_COLOR_LOCATION: u32 = 6;

/// Pin the colour attribute's generic value to white, which a mesh without
/// a colour array reads in the shaders shared with models. Per draw: a draw
/// with the array enabled may leave the generic value unspecified.
pub(crate) fn pin_white_vertex_color(gl: &glow::Context) {
    use glow::HasContext;
    unsafe { gl.vertex_attrib_4_f32(VERTEX_COLOR_LOCATION, 1.0, 1.0, 1.0, 1.0) };
}

impl Vertex for VertexPositionTextureSkinned {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTextureSkinned>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        // Order is the shader attribute location. Normal sits at location 2 and
        // tangent at 3 to match `VertexPositionTexture` (same channel = same
        // location in both formats); joints/weights follow at 4/5, colour at 6.
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
            VertexAttribute {
                attribute_channel: BuiltInVertexChannel::Color,
                attribute_type: VertexAttributeType::Float,
                offset: offset_of!(VertexPositionTextureSkinned, color),
                size: 4,
            },
        ];
        vec
    }
}
