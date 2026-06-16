use cgmath::{vec4, Vector4};
use serde::{Deserialize, Serialize};

use crate::{
    material::{BasicMaterial, ColorMaterial, EmissiveMaterial, Material},
    texture::RuntimeTexture,
    RenderContext, SceneContext, TextureDescription,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaterialDescription {
    #[serde(
        serialize_with = "serialize_vec4",
        deserialize_with = "deserialize_vec4"
    )]
    Color(Vector4<f32>),
    Texture(TextureDescription),
    /// A self-lit surface: a constant `color`, optionally modulated by a texture
    /// (neon signage). Rendered fullbright — unaffected by lighting.
    Emissive {
        #[serde(
            serialize_with = "serialize_vec4",
            deserialize_with = "deserialize_vec4"
        )]
        color: Vector4<f32>,
        texture: Option<TextureDescription>,
    },
}

impl MaterialDescription {
    // TODO: Use color type
    pub fn color(r: f32, g: f32, b: f32, a: f32) -> MaterialDescription {
        MaterialDescription::Color(vec4(r, g, b, a))
    }

    pub fn texture(tex: TextureDescription) -> MaterialDescription {
        MaterialDescription::Texture(tex)
    }

    /// A solid self-lit color (neon / UI), no texture.
    pub fn emissive(r: f32, g: f32, b: f32, a: f32) -> MaterialDescription {
        MaterialDescription::Emissive {
            color: vec4(r, g, b, a),
            texture: None,
        }
    }

    /// A self-lit texture, emitted at full brightness (white tint).
    pub fn emissive_texture(tex: TextureDescription) -> MaterialDescription {
        MaterialDescription::Emissive {
            color: vec4(1.0, 1.0, 1.0, 1.0),
            texture: Some(tex),
        }
    }
}

impl MaterialDescription {
    pub fn get(&self, context: &RenderContext, scene_context: &SceneContext) -> Box<dyn Material> {
        match self {
            MaterialDescription::Color(c) => {
                // TODO: Load from cache of assets
                let mut color_material = ColorMaterial::create(*c);
                color_material.initialize(&context);
                color_material
            }
            MaterialDescription::Texture(t) => {
                match t {
                    TextureDescription::File(file) => {
                        let asset = context.asset_cache.load_asset_with_pipeline(
                            scene_context.texture_pipeline.clone(),
                            &file,
                        );

                        asset.get().bind(0, context);
                    }
                };

                let mut basic_material = BasicMaterial::create();
                basic_material.initialize(&context);
                basic_material
            }
            MaterialDescription::Emissive { color, texture } => {
                // Bind the texture to unit 0 if present; the shader samples it
                // only when `use_texture` is set.
                let use_texture = match texture {
                    Some(TextureDescription::File(file)) => {
                        let asset = context.asset_cache.load_asset_with_pipeline(
                            scene_context.texture_pipeline.clone(),
                            file,
                        );
                        asset.get().bind(0, context);
                        true
                    }
                    None => false,
                };

                let mut emissive_material = EmissiveMaterial::create(*color, use_texture);
                emissive_material.initialize(&context);
                emissive_material
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
