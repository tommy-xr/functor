use cgmath::{vec4, Vector4};
use glow::HasContext;
use serde::{Deserialize, Serialize};

use crate::{
    material::{BasicMaterial, ColorMaterial, EmissiveMaterial, LitMaterial, Material},
    texture::RuntimeTexture,
    RenderContext, SceneContext, TextureDescription,
};

/// Texture filtering selected by a lowered sprite image.
///
/// This lives in the serializable material description rather than GPU state
/// so native/web frames and extrapolation copies carry the same sampling
/// choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpriteSampling {
    Linear,
    Nearest,
}

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
    /// A fullbright sprite image. `source_pixels`, when present, is a
    /// top-left-origin `[x, y, width, height]` rectangle in the source image.
    SpriteTexture {
        #[serde(
            serialize_with = "serialize_vec4",
            deserialize_with = "deserialize_vec4"
        )]
        color: Vector4<f32>,
        texture: TextureDescription,
        source_pixels: Option<[f32; 4]>,
        sampling: SpriteSampling,
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

    /// A fullbright sprite texture with optional atlas source pixels and an
    /// explicit sampling mode.
    pub fn sprite_texture_tinted(
        tex: TextureDescription,
        source_pixels: Option<[f32; 4]>,
        sampling: SpriteSampling,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) -> MaterialDescription {
        MaterialDescription::SpriteTexture {
            color: vec4(r, g, b, a),
            texture: tex,
            source_pixels,
            sampling,
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
                bind_texture_description(t, 0, SpriteSampling::Linear, context, scene_context);

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
            MaterialDescription::SpriteTexture {
                color,
                texture,
                source_pixels,
                sampling,
            } => {
                bind_texture_description(texture, 0, *sampling, context, scene_context);
                let mut material =
                    EmissiveMaterial::create_sprite(*color, source_pixels.map(Vector4::from));
                material.initialize(&context);
                material
            }
        }
    }
}

/// Bind an optional texture to unit `unit`; returns whether one was bound (the
/// shaders sample the texture only when their corresponding `use…` uniform is
/// set).
fn bind_optional_texture(
    texture: &Option<TextureDescription>,
    unit: u32,
    context: &RenderContext,
    scene_context: &SceneContext,
) -> bool {
    match texture {
        Some(t) => {
            bind_texture_description(t, unit, SpriteSampling::Linear, context, scene_context);
            true
        }
        None => false,
    }
}

/// Bind a texture description to unit `unit`: a file texture through the asset
/// pipeline, or a render target's read texture (last completed write). A target
/// id no frame declares binds a 1x1 magenta fallback and warns once.
fn bind_texture_description(
    texture: &TextureDescription,
    unit: u32,
    sampling: SpriteSampling,
    context: &RenderContext,
    scene_context: &SceneContext,
) {
    match texture {
        TextureDescription::File(file) => {
            let asset = context
                .asset_cache
                .load_asset_with_pipeline(scene_context.texture_pipeline.clone(), file);
            asset.get().bind(unit, context);
            set_bound_texture_wrap(unit, true, context);
            set_bound_texture_filter(unit, sampling, context);
        }
        TextureDescription::FileClamped(file) => {
            let asset = context
                .asset_cache
                .load_asset_with_pipeline(scene_context.texture_pipeline.clone(), file);
            asset.get().bind(unit, context);
            set_bound_texture_wrap(unit, false, context);
            set_bound_texture_filter(unit, sampling, context);
        }
        // `Asset.whilePending`: the first loaded chain entry binds while the
        // primary streams in (instead of the checkerboard fallback); a FAILED
        // primary keeps the fallback — failure is not pending.
        TextureDescription::FileWhilePending {
            file,
            while_pending,
        } => {
            let asset = context
                .asset_cache
                .load_asset_with_pipeline(scene_context.texture_pipeline.clone(), file);
            crate::asset::resolve_while_pending(
                &context.asset_cache,
                &scene_context.texture_pipeline,
                &asset,
                while_pending,
            )
            .bind(unit, context);
            set_bound_texture_wrap(unit, true, context);
            set_bound_texture_filter(unit, sampling, context);
        }
        TextureDescription::FileClampedWhilePending {
            file,
            while_pending,
        } => {
            let asset = context
                .asset_cache
                .load_asset_with_pipeline(scene_context.texture_pipeline.clone(), file);
            crate::asset::resolve_while_pending(
                &context.asset_cache,
                &scene_context.texture_pipeline,
                &asset,
                while_pending,
            )
            .bind(unit, context);
            set_bound_texture_wrap(unit, false, context);
            set_bound_texture_filter(unit, sampling, context);
        }
        TextureDescription::RenderTarget(id) => {
            // Select the unit BEFORE any lazy fallback creation: creating the
            // fallback binds/unbinds TEXTURE_2D on the active unit, which would
            // otherwise clobber a texture bound to a lower unit moments ago
            // (e.g. a Lit albedo on unit 0 while resolving a missing normal
            // map on unit 2).
            unsafe {
                context.gl.active_texture(glow::TEXTURE0 + unit);
            }
            let texture = scene_context
                .render_target_read_texture(id)
                .unwrap_or_else(|| {
                    scene_context.warn_once(
                        id,
                        &format!(
                            "[render-target] a material samples \"{id}\" but no \
Frame.withRenderTarget declares it — binding the magenta fallback"
                        ),
                    );
                    scene_context.fallback_texture(context.gl)
                });
            unsafe {
                context.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            }
        }
    }
}

/// File textures share cached GL objects, so every bind reasserts the caller's
/// wrapping mode rather than leaking a prior model/sprite use of that texture.
fn set_bound_texture_wrap(unit: u32, repeat: bool, context: &RenderContext) {
    let wrap = if repeat {
        glow::REPEAT
    } else {
        glow::CLAMP_TO_EDGE
    };
    unsafe {
        context.gl.active_texture(glow::TEXTURE0 + unit);
        context
            .gl
            .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap as i32);
        context
            .gl
            .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap as i32);
    }
}

/// File textures share cached GL objects, so every material bind establishes
/// its own filter rather than inheriting the previous sprite/model draw.
fn set_bound_texture_filter(unit: u32, sampling: SpriteSampling, context: &RenderContext) {
    let filter = match sampling {
        SpriteSampling::Linear => glow::LINEAR,
        SpriteSampling::Nearest => glow::NEAREST,
    };
    unsafe {
        context.gl.active_texture(glow::TEXTURE0 + unit);
        context
            .gl
            .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
        context
            .gl
            .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);
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
