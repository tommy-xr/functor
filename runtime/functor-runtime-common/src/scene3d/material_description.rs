use cgmath::{vec3, vec4, Matrix4, SquareMatrix, Vector4};
use serde::{Deserialize, Serialize};

use crate::{
    material::{ColorMaterial, Material},
    RenderContext,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaterialDescription {
    #[serde(
        serialize_with = "serialize_vec4",
        deserialize_with = "deserialize_vec4"
    )]
    Color(Vector4<f32>),
}

impl MaterialDescription {
    // TODO: Use color type
    pub fn color(r: f32, g: f32, b: f32, a: f32) -> MaterialDescription {
        MaterialDescription::Color(vec4(r, g, b, a))
    }
}

impl MaterialDescription {
    pub fn get(&self, context: &RenderContext) -> Box<dyn Material> {
        match self {
            MaterialDescription::Color(c) => {
                // TODO: Load from cache of assets
                let mut color_material = ColorMaterial::create(*c);
                color_material.initialize(&context);
                color_material
            }
        }
    }
}

fn serialize_vec4<S>(v: &Vector4<f32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let array: [f32; 4] = [v.x, v.y, v.z, v.w];
    array.serialize(serializer)
}

fn deserialize_vec4<'de, D>(deserializer: D) -> Result<Vector4<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let array = <[f32; 4]>::deserialize(deserializer)?;
    Ok(Vector4::new(array[0], array[1], array[2], array[3]))
}
