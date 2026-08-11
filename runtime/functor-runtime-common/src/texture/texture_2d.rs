use glow::HasContext;

use crate::{
    asset::{RenderableAsset, RuntimeRenderableAsset},
    RenderContext,
};

use super::{PixelFormat, RuntimeTexture, TextureData};

pub struct Texture2D {
    ora: RuntimeRenderableAsset<TextureData>,
    average: [f32; 3],
}

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
    /// Anisotropic taps to request, clamped to what the device supports. Only
    /// meaningful with `mipmap`, since anisotropy samples mip levels.
    ///
    /// Costs bandwidth in proportion to the taps, and it is charged per
    /// fragment at grazing angles — which is most of a terrain. Surfaces that
    /// are mostly grazing and mostly tiled want fewer taps than the default.
    pub anisotropy: f32,
    /// Compute the image's mean color at load (see [`Texture2D::average_color`]).
    ///
    /// Off by default: it is an O(pixels) scan that only terrain detail maps
    /// consume, and every sprite and UI texture would otherwise pay it.
    pub compute_average: bool,
    /// Upload with an sRGB internal format (`SRGB8` / `SRGB8_ALPHA8`) so the
    /// hardware decodes to linear at sample time (see `crate::color_space`).
    ///
    /// On for COLOR images (albedo, sprites, detail maps); off — the default,
    /// so no upload site becomes sRGB by accident — for DATA (normal maps,
    /// heightmaps, packed values), whose bytes are numbers, not colors.
    pub srgb: bool,
}

/// Taps for textures that do not say otherwise. Hardware commonly reports 16;
/// the visible gain past 8 is slight and the bandwidth cost is not.
pub const DEFAULT_ANISOTROPY: f32 = 8.0;

/// Taps for tiled terrain detail maps.
///
/// Terrain is overwhelmingly grazing-angle, so anisotropy is charged over
/// almost the whole surface. Measured on a Quest 3: 8 taps on these four maps
/// cost 8.9-13.6 ms of a 13.8 ms frame; 2 taps still missed 72 Hz. The
/// distance fade already limits how far the detail is asked to hold up.
pub const TERRAIN_DETAIL_ANISOTROPY: f32 = 1.0;

impl Default for TextureOptions {
    fn default() -> Self {
        Self {
            wrap: false,
            linear: false,
            mipmap: false,
            anisotropy: DEFAULT_ANISOTROPY,
            compute_average: false,
            srgb: false,
        }
    }
}

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
        let average = opts
            .compute_average
            .then(|| average_color(&data, opts.srgb))
            .unwrap_or([1.0; 3]);
        Texture2D {
            ora: RuntimeRenderableAsset::new(data, opts),
            average,
        }
    }

    /// The image's mean RGB, computed once at load, or white when
    /// `compute_average` was not requested.
    ///
    /// Terrain detail maps divide each sample by this to contribute structure
    /// rather than color; reading it from the texture's top mip instead would
    /// cost a fetch per map per fragment to recover a constant.
    ///
    /// Deliberately in the same space as the samples that divide by it, which
    /// is whatever `TextureOptions::srgb` makes `texture()` return: an sRGB
    /// upload's texels decode to LINEAR in hardware, so its mean is the mean
    /// of the decoded texels; a linear upload's mean is the mean of the raw
    /// bytes. The two must move together — an upload-format change that skips
    /// this divisor silently invalidates it.
    pub fn average_color(&self) -> [f32; 3] {
        self.average
    }
}

/// Mean RGB over every texel, in the space `texture()` will return the texels
/// in (`srgb` = decode each byte sRGB→linear first — matching the hardware
/// decode of an `SRGB8_*` upload). Ignores alpha: these are albedo maps, and
/// a transparent texel's color still contributes to what the eye averages.
fn average_color(data: &TextureData, srgb: bool) -> [f32; 3] {
    let stride = match data.format {
        PixelFormat::RGB => 3,
        PixelFormat::RGBA => 4,
    };
    let texels = data.bytes.len() / stride;
    if texels == 0 {
        return [1.0, 1.0, 1.0];
    }
    if srgb {
        let mut totals = [0f64; 3];
        for texel in data.bytes.chunks_exact(stride) {
            for channel in 0..3 {
                totals[channel] += crate::color_space::srgb_u8_to_linear(texel[channel]) as f64;
            }
        }
        std::array::from_fn(|channel| (totals[channel] / texels as f64) as f32)
    } else {
        let mut totals = [0u64; 3];
        for texel in data.bytes.chunks_exact(stride) {
            for channel in 0..3 {
                totals[channel] += texel[channel] as u64;
            }
        }
        std::array::from_fn(|channel| totals[channel] as f32 / texels as f32 / 255.0)
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
            // Color textures store sRGB so the hardware decodes to linear at
            // sample time; data textures keep the raw (linear) format. See
            // `crate::color_space` and `TextureOptions::srgb`.
            let internal_format = match (self.format, options.srgb) {
                (PixelFormat::RGB, true) => glow::SRGB8,
                (PixelFormat::RGBA, true) => glow::SRGB8_ALPHA8,
                (PixelFormat::RGB, false) => glow::RGB,
                (PixelFormat::RGBA, false) => glow::RGBA,
            };

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_format as i32,
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
                // 1.0 is the GL default, so skip the probe and the set when
                // nothing is being asked for.
                if options.anisotropy > 1.0 {
                    if let Some(device_max) = max_anisotropy(gl) {
                        gl.tex_parameter_f32(
                            glow::TEXTURE_2D,
                            glow::TEXTURE_MAX_ANISOTROPY,
                            options.anisotropy.min(device_max),
                        );
                    }
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
            ..TextureOptions::default()
        }
    }

    fn rgba(texels: &[[u8; 4]]) -> TextureData {
        TextureData {
            bytes: texels.iter().flatten().copied().collect(),
            width: texels.len() as u32,
            height: 1,
            format: PixelFormat::RGBA,
        }
    }

    #[test]
    fn average_color_is_the_mean_over_texels() {
        // Half black, half white: the mean is the midpoint, and alpha is
        // deliberately ignored.
        let data = rgba(&[[0, 0, 0, 0], [255, 255, 255, 255]]);
        let average = average_color(&data, false);
        for channel in average {
            assert!((channel - 0.5).abs() < 0.01, "{average:?}");
        }
    }

    // An sRGB upload's texels decode to linear in hardware, so its mean must
    // be the mean of the DECODED texels — mid-gray 128 decodes to ~0.216, not
    // 0.502 (the divisor invariant documented on `average_color`).
    #[test]
    fn average_color_of_srgb_texture_is_in_linear_space() {
        let data = rgba(&[[128, 128, 128, 255]]);
        let average = average_color(&data, true);
        let expected = crate::color_space::srgb_u8_to_linear(128);
        for channel in average {
            assert!((channel - expected).abs() < 1e-6, "{average:?}");
        }
    }

    #[test]
    fn average_color_reads_three_byte_texels_at_the_right_stride() {
        let data = TextureData {
            bytes: vec![255, 0, 0, 0, 0, 255],
            width: 2,
            height: 1,
            format: PixelFormat::RGB,
        };
        let average = average_color(&data, false);
        assert!((average[0] - 0.5).abs() < 0.01, "{average:?}");
        assert!(average[1].abs() < 0.01, "{average:?}");
        assert!((average[2] - 0.5).abs() < 0.01, "{average:?}");
    }

    // The mean is only paid for when a consumer asks for it.
    #[test]
    fn textures_without_compute_average_report_white() {
        let opaque_black = rgba(&[[0, 0, 0, 255]]);
        let texture = Texture2D::init_from_data(opaque_black, TextureOptions::default());
        assert_eq!(texture.average_color(), [1.0, 1.0, 1.0]);
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
