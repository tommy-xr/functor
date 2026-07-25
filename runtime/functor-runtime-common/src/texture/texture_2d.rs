use glow::HasContext;

use crate::{
    asset::{RenderableAsset, RuntimeRenderableAsset},
    RenderContext,
};

use super::{PixelFormat, RuntimeTexture, TextureData};

pub struct Texture2D {
    ora: RuntimeRenderableAsset<TextureData>,
}

#[derive(Default)]
pub struct TextureOptions {
    pub wrap: bool,
    pub linear: bool,
    /// Build a mip chain and minify through it (trilinear when `linear`).
    ///
    /// Off by default: a mip chain is wrong for anything sampled at or above
    /// 1:1 — pixel-art sprites, UI, and the checkerboard fallback all want the
    /// level-0 texels they were authored as. Turn it on for textures that
    /// minify, where the alternative is aliasing.
    pub mipmap: bool,
}

/// Anisotropic taps to request when a mip chain is built.
///
/// Hardware commonly reports 16; the visible gain past 8 on terrain-angle
/// surfaces is slight and the bandwidth cost is not, which matters on Quest.
const ANISOTROPY_TAPS: f32 = 8.0;

/// The device's anisotropy limit, or `None` where the extension is absent.
///
/// Native and WebGL2 spell the extension differently, and glow enables it at
/// context creation on the web, so probing both names covers every target we
/// ship without per-backend code.
fn max_anisotropy(gl: &glow::Context) -> Option<f32> {
    let supported = gl
        .supported_extensions()
        .contains("GL_EXT_texture_filter_anisotropic")
        || gl
            .supported_extensions()
            .contains("EXT_texture_filter_anisotropic");
    supported.then(|| unsafe { gl.get_parameter_f32(glow::MAX_TEXTURE_MAX_ANISOTROPY) })
}

impl Texture2D {
    pub fn init_from_data(data: TextureData, opts: TextureOptions) -> Texture2D {
        Texture2D {
            ora: RuntimeRenderableAsset::new(data, opts),
        }
    }
}

/// The minification filter for `options` — a pure function so the mip/filter
/// pairing is checkable without a GL context.
fn min_filter_for(options: &TextureOptions) -> u32 {
    match (options.linear, options.mipmap) {
        (true, true) => glow::LINEAR_MIPMAP_LINEAR,
        (true, false) => glow::LINEAR,
        // A NEAREST texture that still minifies wants nearest levels, not a
        // blurred blend between them.
        (false, true) => glow::NEAREST_MIPMAP_NEAREST,
        (false, false) => glow::NEAREST,
    }
}

impl RuntimeTexture for Texture2D {
    fn bind(&self, index: u32, render_context: &RenderContext) {
        let texture = self.ora.get(render_context.gl);
        let gl = render_context.gl;
        unsafe {
            gl.active_texture(glow::TEXTURE0 + index);
            gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
        }
    }
}

impl RenderableAsset for TextureData {
    type HydratedType = glow::Texture;
    type OptionsType = TextureOptions;

    fn hydrate(
        &self,
        gl_context: &glow::Context,
        options: &Self::OptionsType,
    ) -> Self::HydratedType {
        unsafe {
            let gl = gl_context;
            let texture = gl.create_texture().expect("Texture to be created");
            crate::gpu_counters::gpu_counters().texture_created();
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            // Set texture parameters
            let wrap_val = if options.wrap {
                glow::REPEAT
            } else {
                glow::CLAMP_TO_EDGE
            };

            let filter = if options.linear {
                glow::LINEAR
            } else {
                glow::NEAREST
            };
            // Magnification never consults a mip chain, so only the
            // minification filter changes when mipmaps are on.
            let min_filter = min_filter_for(options);

            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap_val as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap_val as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min_filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);

            let format = match self.format {
                PixelFormat::RGB => glow::RGB,
                PixelFormat::RGBA => glow::RGBA,
            };

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                format as i32,
                self.width as i32,
                self.height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&self.bytes)),
            );
            crate::gpu_counters::gpu_counters().uploaded(self.bytes.len());

            // After the level-0 upload: the mip chain is derived from it.
            if options.mipmap {
                gl.generate_mipmap(glow::TEXTURE_2D);
                // Anisotropy only samples levels a mip chain provides, so it
                // is meaningless (and on some drivers ignored) without one.
                if let Some(device_max) = max_anisotropy(gl) {
                    gl.tex_parameter_f32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MAX_ANISOTROPY,
                        ANISOTROPY_TAPS.min(device_max),
                    );
                }
            }

            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(linear: bool, mipmap: bool) -> TextureOptions {
        TextureOptions {
            wrap: true,
            linear,
            mipmap,
        }
    }

    // The default must stay mip-free: sprites, UI, and the fallback
    // checkerboard are authored to be read at level 0.
    #[test]
    fn textures_are_not_mipmapped_unless_asked() {
        let default = TextureOptions::default();
        assert!(!default.mipmap);
        assert_eq!(min_filter_for(&default), glow::NEAREST);
    }

    #[test]
    fn mipmapped_textures_minify_through_the_chain() {
        assert_eq!(
            min_filter_for(&options(true, true)),
            glow::LINEAR_MIPMAP_LINEAR
        );
        assert_eq!(
            min_filter_for(&options(false, true)),
            glow::NEAREST_MIPMAP_NEAREST
        );
    }

    // Selecting a mip filter for a texture with no chain samples an
    // incomplete texture, which renders black on most drivers.
    #[test]
    fn textures_without_a_chain_never_select_a_mip_filter() {
        for linear in [true, false] {
            let filter = min_filter_for(&options(linear, false));
            assert!(
                filter == glow::LINEAR || filter == glow::NEAREST,
                "{filter:#x} samples mip levels that were never generated"
            );
        }
    }
}
