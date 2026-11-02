//! `Terrain.materials`' layers on the GPU: every layer's albedo (and every
//! detail normal) packed into one texture array, so the shader selects a layer
//! by index — a texture coordinate — instead of branching between samplers.

use std::sync::Arc;

use glow::HasContext;

use crate::{
    asset::{AssetHandle, AssetPollState},
    terrain::TerrainMaterials,
    terrain_renderer::MaterialsBound,
    texture::TextureData,
};

/// Rec. 709 luma weights, for linear RGB (as in the shader).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

pub(super) enum MaterialsEntry {
    /// Every layer's albedo, then every detail normal, in layer order.
    Loading(Vec<Arc<AssetHandle<TextureData>>>),
    Ready(MaterialsBound),
    /// A map failed to load or the sizes disagree; warned once, never retried.
    Failed,
}

/// Every map a material set reads, albedos first — the cache key's parts.
pub(super) fn locators(materials: &TerrainMaterials) -> Vec<&str> {
    let albedo = materials.layers.iter().map(|l| l.albedo.locator.as_str());
    let normals = materials
        .layers
        .iter()
        .filter_map(|l| l.normal.as_ref().map(|n| n.locator.as_str()));
    albedo.chain(normals).collect()
}

/// Poll `handles`; once all have decoded, upload the arrays. `Ok(None)` while
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
            AssetPollState::Loaded(data) => maps.push(data),
            AssetPollState::Loading => pending = true,
            AssetPollState::Failed => return Err(format!("\"{locator}\" failed to load")),
        }
    }
    if pending {
        return Ok(None);
    }
    let (albedo, normals) = maps.split_at(materials.layers.len());
    for (maps, what) in [(albedo, "albedo"), (normals, "normal")] {
        // A 0×0 map is the raw pipeline's undecodable sentinel.
        let (w, h) = maps.first().map_or((1, 1), |m| (m.width, m.height));
        if w == 0 || maps.iter().any(|m| (m.width, m.height) != (w, h)) {
            return Err(format!("every layer's {what} map must decode at the same size"));
        }
    }
    let mut luma = [1.0; 4];
    let mut normal_slot = [-1.0; 4];
    let mut slot = 0.0;
    for (i, (layer, map)) in materials.layers.iter().zip(albedo).enumerate() {
        let mean = crate::texture::average_color(map, true);
        luma[i] = (0..3).map(|c| mean[c] * LUMA[c]).sum::<f32>().max(1e-3);
        if layer.normal.is_some() {
            normal_slot[i] = slot;
            slot += 1.0;
        }
    }
    unsafe {
        Ok(Some(MaterialsBound {
            albedo: upload_array(gl, albedo, true),
            normals: (!normals.is_empty()).then(|| upload_array(gl, normals, false)),
            luma,
            normal_slot,
        }))
    }
}

/// A mipped, repeating, anisotropy-1 RGBA8 array of same-sized `layers`
/// (sRGB storage for color, so samples decode to linear like `texture()`).
unsafe fn upload_array(gl: &glow::Context, layers: &[Arc<TextureData>], srgb: bool) -> glow::Texture {
    let (w, h) = (layers[0].width as i32, layers[0].height as i32);
    let texture = gl.create_texture().expect("terrain layer array");
    crate::gpu_counters::gpu_counters().texture_created();
    gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
    let internal = if srgb { glow::SRGB8_ALPHA8 } else { glow::RGBA8 };
    gl.tex_image_3d(
        glow::TEXTURE_2D_ARRAY,
        0,
        internal as i32,
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
