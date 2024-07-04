use cgmath::{Vector2, Vector3};
use std::mem::{offset_of, size_of};

pub enum VertexAttributeType {
    Float,
    NormalizedFloat,
    Int,
}

pub enum BuiltInVertexChannel {
    Position,
    Uv,
    Normal,
    Binormal,
    Tangent,
    BoneIndices,
    // Not bound to any channel used for internal shaders.
    // May be an attribute used for custom shaders
    Custom,
}

pub struct VertexAttribute {
    pub attribute_type: VertexAttributeType,
    pub attribute_channel: BuiltInVertexChannel,
    pub offset: usize,
    pub size: i32,
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct VertexPositionTexture {
    pub position: Vector3<f32>,
    pub uv: Vector2<f32>,
}

pub trait Vertex {
    fn get_total_size() -> usize;
    fn get_vertex_attributes() -> Vec<VertexAttribute>;
}

impl Vertex for VertexPositionTexture {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTexture>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        vec![
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
        ]
    }
}
