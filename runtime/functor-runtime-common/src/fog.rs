use cgmath::Vector3;
use serde::{Deserialize, Serialize};

use crate::shader_program::{ShaderProgram, UniformLocation};

/// Frame-level distance fog: fragments blend toward `color` with radial
/// world-space distance from the camera. Pure data in the `Frame` (the
/// `Light` serde style). The fog color doubles as the pass's clear color
/// (see [`clear_color`]), so distant geometry dissolves into the background
/// instead of silhouetting against it. Applies to every forward material —
/// including emissive (fog occludes glow). Depth passes and the
/// normals/tangents debug materials don't shade with fog (their shaders have
/// no fog block); the physics debug overlay shades normally, so fog applies.
///
/// Like `Light`, this accepts degenerate parameters (`far <= near`,
/// `density <= 0`) without judgement — validation with teaching errors is
/// the Functor Lang/game-surface layer's job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fog {
    /// `factor = clamp((far - d) / (far - near), 0, 1)`: fully clear at
    /// `near`, fully fog by `far` (world units).
    Linear { near: f32, far: f32, color: [f32; 3] },
    /// `factor = exp(-density * d)`: the classic atmospheric falloff.
    Exp { density: f32, color: [f32; 3] },
    /// Exponential height fog cut off by a linear end: the linear factor
    /// times `exp(-τ)`, where τ integrates
    /// `density * exp(-falloff * (y - base_y))` along the view ray. Valleys
    /// below `base_y` fill; peaks above it clear. Fully fogged by `far`.
    Height {
        density: f32,
        falloff: f32,
        base_y: f32,
        near: f32,
        far: f32,
        color: [f32; 3],
    },
}

impl Fog {
    pub fn linear(near: f32, far: f32, r: f32, g: f32, b: f32) -> Fog {
        Fog::Linear {
            near,
            far,
            color: [r, g, b],
        }
    }

    pub fn exp(density: f32, r: f32, g: f32, b: f32) -> Fog {
        Fog::Exp {
            density,
            color: [r, g, b],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn height(
        density: f32,
        falloff: f32,
        base_y: f32,
        near: f32,
        far: f32,
        r: f32,
        g: f32,
        b: f32,
    ) -> Fog {
        Fog::Height {
            density,
            falloff,
            base_y,
            near,
            far,
            color: [r, g, b],
        }
    }

    pub fn color(&self) -> [f32; 3] {
        match self {
            Fog::Linear { color, .. } | Fog::Exp { color, .. } | Fog::Height { color, .. } => {
                *color
            }
        }
    }

    /// The same fog with its color zeroed: what terrain LOD depends on.
    pub fn without_color(&self) -> Fog {
        match *self {
            Fog::Linear { near, far, .. } => Fog::linear(near, far, 0.0, 0.0, 0.0),
            Fog::Exp { density, .. } => Fog::exp(density, 0.0, 0.0, 0.0),
            Fog::Height {
                density,
                falloff,
                base_y,
                near,
                far,
                ..
            } => Fog::height(density, falloff, base_y, near, far, 0.0, 0.0, 0.0),
        }
    }

    /// Where the fog becomes total: past this distance every surface renders
    /// as exactly the fog color, however bright. Linear and height fog have
    /// one; exponential fog never reaches zero visibility.
    pub fn end(&self) -> Option<f32> {
        match *self {
            Fog::Linear { near, far, .. } | Fog::Height { near, far, .. }
                if far > near && far.is_finite() =>
            {
                Some(far)
            }
            _ => None,
        }
    }

    /// How much of a surface's own color survives at `distance`: the
    /// shader's `applyFog` factor, clamped to 0..1 (NaN reads as no fog).
    /// Terrain LOD scales its screen-space error by it. Height fog depends on
    /// the ray's heights too; this is its linear end term alone, an upper
    /// bound, so LOD never coarsens past what the fog hides.
    pub fn visibility(&self, distance: f32) -> f32 {
        let factor = match self {
            Fog::Linear { near, far, .. } | Fog::Height { near, far, .. } => {
                (far - distance) / (far - near).max(1e-4)
            }
            Fog::Exp { density, .. } => (-density * distance).exp(),
        };
        if factor.is_nan() {
            1.0
        } else {
            factor.clamp(0.0, 1.0)
        }
    }
}

/// The clear color for a forward pass: the fog color when fog is set (so the
/// background IS the fog at infinity), else the engine default.
pub fn clear_color(fog: Option<&Fog>) -> [f32; 3] {
    fog.map(Fog::color).unwrap_or([0.1, 0.2, 0.3])
}

/// Prepended to every forward fragment shader (via `format!` at build time).
/// `applyFog` early-returns the color unchanged when disabled, so a fog-less
/// frame renders bit-identically to the pre-fog engine — the golden-image
/// contract. Distance is radial world-space (not view-z): consistent across
/// materials and rotation-invariant.
///
/// The distance math is `highp`: GLES fragment shaders default to mediump
/// (fp16 on Adreno), where `distance()` overflows past ~256 m and every
/// fragment beyond reads as fully fogged.
pub const FOG_GLSL: &str = r#"
        uniform int fogEnabled;      // 0 = off (the default frame state)
        uniform int fogMode;         // 0 = linear, 1 = exp, 2 = height
        uniform vec3 fogColor;       // the sky's horizon, under a sky gradient
        uniform highp float fogNear;
        uniform highp float fogFar;
        uniform highp float fogDensity; // height: the density at fogBaseY
        uniform highp float fogFalloff;
        uniform highp float fogBaseY;
        uniform highp vec3 fogCameraPos;   // camera world position
        uniform int fogSky;          // 1: fog takes the sky gradient's color
        uniform vec3 fogSkyZenith;

        // The sky gradient (`Frame.withSkyGradient`): horizon at elevation 0
        // and below, blending linearly in dir.y to zenith straight up. The sky
        // pass and the fog share it, so fully fogged geometry takes exactly
        // the sky color behind it.
        vec3 skyGradient(vec3 horizon, vec3 zenith, highp vec3 dir) {
            return mix(horizon, zenith, clamp(normalize(dir).y, 0.0, 1.0));
        }

        // How much of a surface's own color survives at worldPosition; 1.0
        // with fog off, exactly 0.0 at and past linear/height fog's far.
        float fogVisibility(highp vec3 worldPosition) {
            if (fogEnabled == 0) {
                return 1.0;
            }
            highp float dist = distance(worldPosition, fogCameraPos);
            if (fogMode == 1) {
                return exp(-fogDensity * dist);
            }
            highp float ramp = clamp((fogFar - dist) / max(fogFar - fogNear, 1e-4), 0.0, 1.0);
            if (fogMode == 0 || ramp <= 0.0) {
                return ramp;
            }
            // Height: the optical depth of density * exp(-falloff * height)
            // along the ray is the density at its lower (denser) end, times
            // dist * (1 - exp(-x)) / x, with x the falloff times the ray's
            // rise. Both factors stay finite (the first capped at e^80);
            // near-level rays take the series 1 - x/2.
            highp float lowest = min(worldPosition.y, fogCameraPos.y) - fogBaseY;
            highp float densest = fogDensity * exp(min(-fogFalloff * lowest, 80.0));
            highp float x = fogFalloff * abs(worldPosition.y - fogCameraPos.y);
            highp float shape = x > 1e-3 ? (1.0 - exp(-x)) / x : 1.0 - 0.5 * x;
            return ramp * exp(-densest * dist * shape);
        }

        // The color fog blends toward: under a sky gradient, the sky behind
        // the fragment, so fully fogged geometry vanishes into it.
        vec3 fogColorAt(highp vec3 worldPosition) {
            if (fogSky == 0) {
                return fogColor;
            }
            return skyGradient(fogColor, fogSkyZenith, worldPosition - fogCameraPos);
        }

        vec3 applyFog(vec3 color, highp vec3 worldPosition) {
            if (fogEnabled == 0) {
                return color;
            }
            return mix(fogColorAt(worldPosition), color, fogVisibility(worldPosition));
        }
"#;

/// The fog uniform locations of one forward shader program. Every material
/// whose fragment shader includes [`FOG_GLSL`] looks these up in
/// `initialize` and uploads via [`FogUniforms::set`] each draw. Every uniform
/// is statically referenced inside `applyFog`, so they stay active and
/// `get_uniform_location` never panics.
pub struct FogUniforms {
    enabled_loc: UniformLocation,
    mode_loc: UniformLocation,
    color_loc: UniformLocation,
    near_loc: UniformLocation,
    far_loc: UniformLocation,
    density_loc: UniformLocation,
    falloff_loc: UniformLocation,
    base_y_loc: UniformLocation,
    camera_pos_loc: UniformLocation,
    sky_loc: UniformLocation,
    sky_zenith_loc: UniformLocation,
}

impl FogUniforms {
    pub fn get(shader: &ShaderProgram, gl: &glow::Context) -> FogUniforms {
        FogUniforms {
            enabled_loc: shader.get_uniform_location(gl, "fogEnabled"),
            mode_loc: shader.get_uniform_location(gl, "fogMode"),
            color_loc: shader.get_uniform_location(gl, "fogColor"),
            near_loc: shader.get_uniform_location(gl, "fogNear"),
            far_loc: shader.get_uniform_location(gl, "fogFar"),
            density_loc: shader.get_uniform_location(gl, "fogDensity"),
            falloff_loc: shader.get_uniform_location(gl, "fogFalloff"),
            base_y_loc: shader.get_uniform_location(gl, "fogBaseY"),
            camera_pos_loc: shader.get_uniform_location(gl, "fogCameraPos"),
            sky_loc: shader.get_uniform_location(gl, "fogSky"),
            sky_zenith_loc: shader.get_uniform_location(gl, "fogSkyZenith"),
        }
    }

    /// Upload this draw's fog state. `None` sets only `fogEnabled = 0` (the
    /// other uniforms are dead behind the shader's early return).
    /// `camera_pos` is the camera's world position — frame-constant, so the
    /// renderer computes it once per pass (`RenderContext::camera_pos`).
    /// `sky` is the drawn sky gradient's `(horizon, zenith)`, if any
    /// (`RenderContext::sky_gradient`): fog then blends toward that sky, its
    /// horizon replacing the fog's own color.
    pub fn set(
        &self,
        p: &ShaderProgram,
        gl: &glow::Context,
        fog: Option<&Fog>,
        sky: Option<([f32; 3], [f32; 3])>,
        camera_pos: &Vector3<f32>,
    ) {
        let Some(fog) = fog else {
            p.set_uniform_1i(gl, &self.enabled_loc, 0);
            return;
        };
        p.set_uniform_1i(gl, &self.enabled_loc, 1);
        p.set_uniform_vec3(gl, &self.camera_pos_loc, camera_pos);
        // Authored color: decode sRGB→linear at the uniform boundary (the
        // serialized Fog keeps its authored value). The clear color decodes
        // the same way where an sRGB attachment is cleared, so fully fogged
        // geometry still dissolves exactly into the background.
        let (horizon, zenith) = sky.unwrap_or((fog.color(), [0.0; 3]));
        let c = crate::color_space::srgb_to_linear3(horizon);
        p.set_uniform_vec3(gl, &self.color_loc, &Vector3::new(c[0], c[1], c[2]));
        p.set_uniform_1i(gl, &self.sky_loc, sky.is_some() as i32);
        let z = crate::color_space::srgb_to_linear3(zenith);
        p.set_uniform_vec3(gl, &self.sky_zenith_loc, &Vector3::new(z[0], z[1], z[2]));
        let (mode, near, far, density, falloff, base_y) = match *fog {
            Fog::Linear { near, far, .. } => (0, near, far, 0.0, 0.0, 0.0),
            Fog::Exp { density, .. } => (1, 0.0, 0.0, density, 0.0, 0.0),
            Fog::Height {
                density,
                falloff,
                base_y,
                near,
                far,
                ..
            } => (2, near, far, density, falloff, base_y),
        };
        p.set_uniform_1i(gl, &self.mode_loc, mode);
        p.set_uniform_1f(gl, &self.near_loc, near);
        p.set_uniform_1f(gl, &self.far_loc, far);
        p.set_uniform_1f(gl, &self.density_loc, density);
        p.set_uniform_1f(gl, &self.falloff_loc, falloff);
        p.set_uniform_1f(gl, &self.base_y_loc, base_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_matches_the_shader_factor() {
        let linear = Fog::linear(100.0, 300.0, 0.0, 0.0, 0.0);
        assert_eq!(linear.visibility(50.0), 1.0);
        assert_eq!(linear.visibility(200.0), 0.5);
        assert_eq!(linear.visibility(400.0), 0.0);
        let exp = Fog::exp(0.01, 0.0, 0.0, 0.0);
        assert!((exp.visibility(100.0) - (-1.0f32).exp()).abs() < 1e-6);
        assert_eq!(Fog::exp(-1.0, 0.0, 0.0, 0.0).visibility(10.0), 1.0);
        assert_eq!(Fog::exp(f32::NAN, 0.0, 0.0, 0.0).visibility(10.0), 1.0);
    }

    #[test]
    fn only_linear_fog_has_an_end() {
        let linear = Fog::linear(20.0, 200.0, 0.0, 0.0, 0.0);
        assert_eq!(linear.end(), Some(200.0));
        assert_eq!(linear.visibility(200.0), 0.0);
        assert_eq!(Fog::linear(200.0, 20.0, 0.0, 0.0, 0.0).end(), None);
        assert_eq!(Fog::exp(0.01, 0.0, 0.0, 0.0).end(), None);
    }

    #[test]
    fn height_fog_ends_like_linear_fog() {
        let fog = Fog::height(0.02, 0.05, 0.0, 100.0, 300.0, 0.0, 0.0, 0.0);
        assert_eq!(fog.end(), Some(300.0));
        // LOD's distance-only bound: the linear end term.
        assert_eq!(fog.visibility(50.0), 1.0);
        assert_eq!(fog.visibility(200.0), 0.5);
        assert_eq!(fog.visibility(300.0), 0.0);
        assert_eq!(fog.without_color().color(), [0.0; 3]);
    }

    #[test]
    fn clear_color_defaults_without_fog() {
        assert_eq!(clear_color(None), [0.1, 0.2, 0.3]);
    }

    #[test]
    fn clear_color_is_the_fog_color() {
        let fog = Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7);
        assert_eq!(clear_color(Some(&fog)), [0.5, 0.6, 0.7]);
        let fog = Fog::exp(0.08, 0.2, 0.3, 0.4);
        assert_eq!(clear_color(Some(&fog)), [0.2, 0.3, 0.4]);
    }

    #[test]
    fn constructors_map_fields() {
        assert_eq!(
            Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7),
            Fog::Linear {
                near: 4.0,
                far: 30.0,
                color: [0.5, 0.6, 0.7]
            }
        );
        assert_eq!(
            Fog::exp(0.08, 0.5, 0.6, 0.7),
            Fog::Exp {
                density: 0.08,
                color: [0.5, 0.6, 0.7]
            }
        );
    }
}
