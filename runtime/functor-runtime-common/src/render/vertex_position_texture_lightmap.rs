use cgmath::{Vector2, Vector3};
use std::mem::{offset_of, size_of};

use super::vertex::{BuiltInVertexChannel, Vertex, VertexAttribute, VertexAttributeType};

/// A static model's vertex (40 B): `uv` samples the base texture, `uv2` the
/// lightmap. Position, uv and normal sit at locations 0–2 like the other
/// formats, so depth and debug shaders draw it unchanged.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct VertexPositionTextureLightmap {
    pub position: Vector3<f32>,
    pub uv: Vector2<f32>,
    pub normal: Vector3<f32>,
    pub uv2: Vector2<f32>,
}

impl Vertex for VertexPositionTextureLightmap {
    fn get_total_size() -> usize {
        size_of::<VertexPositionTextureLightmap>()
    }

    fn get_vertex_attributes() -> Vec<VertexAttribute> {
        let attribute = |attribute_channel, offset, size| VertexAttribute {
            attribute_type: VertexAttributeType::Float,
            attribute_channel,
            offset,
            size,
        };
        vec![
            attribute(
                BuiltInVertexChannel::Position,
                offset_of!(VertexPositionTextureLightmap, position),
                3,
            ),
            attribute(
                BuiltInVertexChannel::Uv,
                offset_of!(VertexPositionTextureLightmap, uv),
                2,
            ),
            attribute(
                BuiltInVertexChannel::Normal,
                offset_of!(VertexPositionTextureLightmap, normal),
                3,
            ),
            attribute(
                BuiltInVertexChannel::Custom,
                offset_of!(VertexPositionTextureLightmap, uv2),
                2,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn is_forty_bytes() {
        assert_eq!(std::mem::size_of::<super::VertexPositionTextureLightmap>(), 40);
    }
}
