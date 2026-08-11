//! sRGB ↔ linear conversion — the single home of Functor's gamma story.
//!
//! The pipeline is gamma-correct by construction:
//!
//! - **Color texture uploads use sRGB internal formats** (`SRGB8` /
//!   `SRGB8_ALPHA8`): file albedo textures, glTF base-color textures, builtin
//!   textures, sprite fonts, skybox faces. The hardware decodes to linear at
//!   sample time. Data textures (heightmaps, the terrain macro map, normal
//!   maps, packed-depth shadow maps) stay linear — their bytes are numbers,
//!   not colors.
//! - **Authored colors decode sRGB→linear CPU-side at the uniform-set/clear
//!   boundary** (material colors, light colors, fog color, clear colors,
//!   terrain layer/grass colors) — never in the serialized `Frame`/`Scene3D`,
//!   so serialization, `Frame.equals`, time-travel and replay are untouched.
//! - **Shading happens in linear space**, and the result is encoded back to
//!   sRGB exactly once per pixel:
//!   - render-target / composite-input attachments are `SRGB8_ALPHA8`, so the
//!     hardware encodes on write (desktop enables `GL_FRAMEBUFFER_SRGB` once
//!     at init to match the always-on ES/WebGL2 semantics);
//!   - non-sRGB output surfaces (the desktop GLFW backbuffer, the WebGL2
//!     canvas) get a shader epilogue instead: [`OUTPUT_ENCODE_GLSL`]'s
//!     `functorOutput`, gated by the `uOutputSrgbEncode` uniform.
//!
//! **The no-double-encoding invariant:** the encode decision is derived in ONE
//! place — the renderer sets [`crate::RenderContext::output_srgb_encode`] from
//! "is the current attachment sRGB?" (always false for render-target /
//! composite-input passes; the shell-declared
//! [`crate::SceneContext::set_output_colorspace`] for caller-framebuffer
//! passes). Effects and materials never decide encoding themselves.
//!
//! No tone mapping: the epilogue clamps and encodes, nothing more. It is the
//! seam where a tonemap later slots in.

use std::sync::OnceLock;

use cgmath::Vector4;

use crate::shader_program::{ShaderProgram, UniformLocation};

/// What the shell's output surface (the framebuffer `render_frame*` draws
/// into) stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputColorspace {
    /// A non-sRGB surface (desktop GLFW backbuffer, WebGL2 canvas): shaders
    /// encode linear→sRGB in the `functorOutput` epilogue.
    NonSrgb,
    /// An sRGB surface (the Quest eye swapchain's `SRGB8_ALPHA8`): the
    /// hardware encodes on write, so the epilogue passes through.
    Srgb,
}

/// The sRGB EOTF⁻¹: decode one sRGB-encoded channel to linear light.
/// (IEC 61966-2-1: linear below 0.04045, a 2.4 power curve above.)
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB OETF: encode one linear channel to sRGB.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Decode an authored `[r, g, b]` to linear.
pub fn srgb_to_linear3(c: [f32; 3]) -> [f32; 3] {
    [
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
    ]
}

/// Decode an authored RGBA color's RGB to linear; alpha is coverage, not
/// color, and passes through.
pub fn srgb_to_linear_vec4(c: Vector4<f32>) -> Vector4<f32> {
    Vector4::new(
        srgb_to_linear(c.x),
        srgb_to_linear(c.y),
        srgb_to_linear(c.z),
        c.w,
    )
}

/// Decode one sRGB byte to linear, through a lazily built 256-entry table —
/// for O(pixels) load-time scans ([`crate::texture::Texture2D`]'s
/// `average_color`).
pub fn srgb_u8_to_linear(byte: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))[byte as usize]
}

/// Injected into every fragment shader's preamble (`shader::convert`).
///
/// - `functorSrgbToLinear` decodes sRGB-authored *inputs* that reach the
///   shader undecoded (the instanced renderer's per-instance tint attribute,
///   which multiplies its raw base color to match the CPU stamp's
///   `decode(base * tint)` exactly).
/// - `functorOutput` is the output epilogue: linear→sRGB encode when
///   `uOutputSrgbEncode` is 1 (non-sRGB output surface), passthrough when 0
///   (sRGB attachment — the hardware encodes). Every color-writing fragment
///   shader routes its final `fragColor` through it; the uniform is set from
///   `RenderContext::output_srgb_encode` (see the module doc's invariant).
///   The clamp is not a tonemap — it mirrors the clamp a UNORM write always
///   did; a tonemap would slot in right before the encode.
pub const OUTPUT_ENCODE_GLSL: &str = r#"
        uniform int uOutputSrgbEncode;

        vec3 functorSrgbToLinear(vec3 c) {
            vec3 lo = c / 12.92;
            vec3 hi = pow((c + 0.055) / 1.055, vec3(2.4));
            return mix(hi, lo, vec3(lessThanEqual(c, vec3(0.04045))));
        }

        vec4 functorOutput(vec4 color) {
            if (uOutputSrgbEncode == 1) {
                vec3 c = clamp(color.rgb, 0.0, 1.0);
                vec3 lo = c * 12.92;
                vec3 hi = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
                vec3 encoded = mix(hi, lo, vec3(lessThanEqual(c, vec3(0.0031308))));
                return vec4(encoded, color.a);
            }
            return color;
        }
"#;

/// The `uOutputSrgbEncode` uniform location of one color-writing shader
/// program (the `FogUniforms` pattern). Programs whose fragment shader routes
/// its output through `functorOutput` look this up in `initialize` and upload
/// via [`OutputEncodeUniform::set`] each draw; depth/debug programs never
/// reference the uniform (it is optimized out), so they must not look it up.
pub struct OutputEncodeUniform {
    loc: UniformLocation,
}

impl OutputEncodeUniform {
    pub fn get(shader: &ShaderProgram, gl: &glow::Context) -> OutputEncodeUniform {
        OutputEncodeUniform {
            loc: shader.get_uniform_location(gl, "uOutputSrgbEncode"),
        }
    }

    pub fn set(&self, p: &ShaderProgram, gl: &glow::Context, encode: bool) {
        p.set_uniform_1i(gl, &self.loc, encode as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        // Mid-gray: the canonical sRGB checkpoint.
        assert!((srgb_to_linear(0.5) - 0.214_041_14).abs() < 1e-6);
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(0.214_041_14) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn round_trips() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!((linear_to_srgb(srgb_to_linear(x)) - x).abs() < 1e-5, "{x}");
            assert!((srgb_to_linear(linear_to_srgb(x)) - x).abs() < 1e-5, "{x}");
        }
    }

    #[test]
    fn piecewise_boundary_is_continuous() {
        let below = srgb_to_linear(0.04045 - 1e-6);
        let above = srgb_to_linear(0.04045 + 1e-6);
        assert!((below - above).abs() < 1e-5);
        let below = linear_to_srgb(0.0031308 - 1e-7);
        let above = linear_to_srgb(0.0031308 + 1e-7);
        assert!((below - above).abs() < 1e-5);
    }

    #[test]
    fn u8_lut_matches_the_float_path() {
        for byte in [0u8, 1, 12, 64, 128, 200, 255] {
            let direct = srgb_to_linear(byte as f32 / 255.0);
            assert!((srgb_u8_to_linear(byte) - direct).abs() < 1e-6, "{byte}");
        }
    }

    #[test]
    fn vec4_decodes_rgb_and_passes_alpha_through() {
        let c = srgb_to_linear_vec4(Vector4::new(0.5, 0.0, 1.0, 0.25));
        assert!((c.x - 0.214_041_14).abs() < 1e-6);
        assert_eq!(c.y, 0.0);
        assert!((c.z - 1.0).abs() < 1e-6);
        assert_eq!(c.w, 0.25);
    }

    #[test]
    fn glsl_and_rust_agree_on_the_constants() {
        // The GLSL snippet duplicates the piecewise constants; pin them so a
        // drive-by edit to one side cannot silently diverge.
        for needle in ["0.04045", "12.92", "2.4", "0.055", "0.0031308"] {
            assert!(OUTPUT_ENCODE_GLSL.contains(needle), "{needle}");
        }
    }
}
