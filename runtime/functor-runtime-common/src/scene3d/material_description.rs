use cgmath::{vec4, Vector4};
use serde::{Deserialize, Serialize};

use crate::{
    material::{BasicMaterial, ColorMaterial, EmissiveMaterial, LitMaterial, Material},
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
    /// A diffuse-lit surface: albedo `color` (optionally modulated by a texture)
    /// shaded by the frame's lights (Lambert + specular), optionally with a
    /// tangent-space `normal_map` perturbing the surface normal.
    Lit {
        #[serde(
            serialize_with = "serialize_vec4",
            deserialize_with = "deserialize_vec4"
        )]
        color: Vector4<f32>,
        texture: Option<TextureDescription>,
        #[serde(default)]
        normal_map: Option<TextureDescription>,
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

    /// A diffuse-lit solid color.
    pub fn lit(r: f32, g: f32, b: f32, a: f32) -> MaterialDescription {
        MaterialDescription::Lit {
            color: vec4(r, g, b, a),
            texture: None,
            normal_map: None,
        }
    }

    /// A diffuse-lit texture (white albedo tint).
    pub fn lit_texture(tex: TextureDescription) -> MaterialDescription {
        MaterialDescription::Lit {
            color: vec4(1.0, 1.0, 1.0, 1.0),
            texture: Some(tex),
            normal_map: None,
        }
    }

    /// A diffuse-lit surface with a tangent-space normal map perturbing the
    /// lighting. `color` is the albedo tint (no albedo texture); `normal_map` is
    /// the normal-map texture.
    pub fn lit_normal_mapped(
        r: f32,
        g: f32,
        b: f32,
        a: f32,
        normal_map: TextureDescription,
    ) -> MaterialDescription {
        MaterialDescription::Lit {
            color: vec4(r, g, b, a),
            texture: None,
            normal_map: Some(normal_map),
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
                let use_texture = bind_optional_texture(texture, 0, context, scene_context);
                let mut emissive_material = EmissiveMaterial::create(*color, use_texture);
                emissive_material.initialize(&context);
                emissive_material
            }
            MaterialDescription::Lit {
                color,
                texture,
                normal_map,
            } => {
                // Albedo on unit 0, the normal map on unit 2 (the shadow map
                // uses unit 1).
                let use_texture = bind_optional_texture(texture, 0, context, scene_context);
                let use_normal_map = bind_optional_texture(normal_map, 2, context, scene_context);
                let mut lit_material = LitMaterial::create(*color, use_texture, use_normal_map);
                lit_material.initialize(&context);
                lit_material
            }
        }
    }
}

/// Bind an optional albedo texture to unit 0; returns whether one was bound (the
/// shaders sample the texture only when their corresponding `use…` uniform is
/// set. Binds to texture unit `unit`.
fn bind_optional_texture(
    texture: &Option<TextureDescription>,
    unit: u32,
    context: &RenderContext,
    scene_context: &SceneContext,
) -> bool {
    match texture {
        Some(TextureDescription::File(file)) => {
            let asset = context
                .asset_cache
                .load_asset_with_pipeline(scene_context.texture_pipeline.clone(), file);
            asset.get().bind(unit, context);
            true
        }
        None => false,
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
