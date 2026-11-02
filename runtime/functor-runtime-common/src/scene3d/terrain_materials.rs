//! `Terrain.materials`' layers on the GPU: every layer's albedo packed into
//! one texture array, so the shader selects a layer by index — a texture
//! coordinate — instead of branching between samplers.

use std::sync::Arc;

use glow::HasContext;

use crate::{
    asset::{AssetHandle, AssetPollState},
    terrain::TerrainMaterials,
    terrain_renderer::MaterialsBound,
    texture::TextureData,
};

pub(super) enum MaterialsEntry {
    /// Every layer's albedo, in layer order.
    Loading(Vec<Arc<AssetHandle<TextureData>>>),
    Ready(MaterialsBound),
    /// A map failed to load or the sizes disagree; warned once, never retried.
    Failed,
}

/// Every layer's albedo locator, in layer order (also the cache key's parts).
pub(super) fn locators(materials: &TerrainMaterials) -> Vec<&str> {
    materials.layers.iter().map(|l| l.albedo.locator.as_str()).collect()
}

/// Poll `handles`; once all have decoded, upload the array. `Ok(None)` while
/// loading, `Err` (a warning) once a map fails or the sizes disagree.
pub(super) fn poll(
    gl: &glow::Context,
    materials: &TerrainMaterials,
    handles: &[Arc<AssetHandle<TextureData>>],
) -> Result<Option<MaterialsBound>, String> {
    // Poll every handle each call: a future advances only when polled.
    let mut maps = Vec::with_capacity(handles.len());
    let mut pending = false;
    for (handle, locator) in handles.iter().zip(locators(materials)) {
        match handle.poll_state() {
            // A 0×0 map is the raw pipeline's undecodable sentinel.
            AssetPollState::Loaded(data) if data.width == 0 => {
                return Err(format!("\"{locator}\" could not be decoded"));
            }
            AssetPollState::Loaded(data) => maps.push(data),
            AssetPollState::Loading => pending = true,
            AssetPollState::Failed => return Err(format!("\"{locator}\" failed to load")),
        }
    }
    if pending {
        return Ok(None);
    }
    let size = (maps[0].width, maps[0].height);
    if maps.iter().any(|m| (m.width, m.height) != size) {
        return Err("every layer's albedo map must decode at the same size".to_string());
    }
    let mut luma = [1.0; 4];
    for (i, map) in maps.iter().enumerate() {
        // The shader's divisor: the same Rec. 709 weights as its `LUMA`.
        let mean = crate::texture::average_color(map, true);
        luma[i] = crate::color_space::luminance(mean).max(1e-3);
    }
    Ok(Some(MaterialsBound {
        albedo: unsafe { upload_array(gl, &maps) },
        luma,
    }))
}

/// A mipped, repeating, anisotropy-1 sRGB array of same-sized `layers`, so
/// samples decode to linear like `texture()` on any color map.
unsafe fn upload_array(gl: &glow::Context, layers: &[Arc<TextureData>]) -> glow::Texture {
    let (w, h) = (layers[0].width as i32, layers[0].height as i32);
    let texture = gl.create_texture().expect("terrain layer array");
    crate::gpu_counters::gpu_counters().texture_created();
    gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
    gl.tex_image_3d(
        glow::TEXTURE_2D_ARRAY,
        0,
        glow::SRGB8_ALPHA8 as i32,
        w,
        h,
        layers.len() as i32,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(None),
    );
    for (i, layer) in layers.iter().enumerate() {
        gl.tex_sub_image_3d(
            glow::TEXTURE_2D_ARRAY,
            0,
            0,
            0,
            i as i32,
            w,
            h,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&layer.bytes)),
        );
        crate::gpu_counters::gpu_counters().uploaded(layer.bytes.len());
    }
    gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
    let params = [
        (glow::TEXTURE_WRAP_S, glow::REPEAT),
        (glow::TEXTURE_WRAP_T, glow::REPEAT),
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
    ];
    for (name, value) in params {
        gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, name, value as i32);
    }
    // Anisotropy stays at the GL default of 1, like every terrain map.
    gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
    texture
}
