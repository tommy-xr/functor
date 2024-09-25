use cgmath::{Vector2, Vector3};
use std::mem::{offset_of, size_of};

pub enum VertexAttributeType {
    Float,
}

pub enum BuiltInVertexChannel {
    Position,
    Uv,
    Normal,
    Binormal,
    Tangent,
    JointIndices,
    JointWeights,
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

pub trait Vertex {
    fn get_total_size() -> usize;
    fn get_vertex_attributes() -> Vec<VertexAttribute>;
}
