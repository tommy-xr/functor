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
/// the MLE/game-surface layer's job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fog {
    /// `factor = clamp((far - d) / (far - near), 0, 1)`: fully clear at
    /// `near`, fully fog by `far` (world units).
    Linear { near: f32, far: f32, color: [f32; 3] },
    /// `factor = exp(-density * d)`: the classic atmospheric falloff.
    Exp { density: f32, color: [f32; 3] },
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

    pub fn color(&self) -> [f32; 3] {
        match self {
            Fog::Linear { color, .. } | Fog::Exp { color, .. } => *color,
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
pub const FOG_GLSL: &str = r#"
        uniform int fogEnabled;      // 0 = off (the default frame state)
        uniform int fogMode;         // 0 = linear, 1 = exp
        uniform vec3 fogColor;
        uniform float fogNear;
        uniform float fogFar;
        uniform float fogDensity;
        uniform vec3 fogCameraPos;   // camera world position

        vec3 applyFog(vec3 color, vec3 worldPosition) {
            if (fogEnabled == 0) {
                return color;
            }
            float dist = distance(worldPosition, fogCameraPos);
            float factor;
            if (fogMode == 0) {
                factor = clamp((fogFar - dist) / max(fogFar - fogNear, 1e-4), 0.0, 1.0);
            } else {
                factor = exp(-fogDensity * dist);
            }
            return mix(fogColor, color, factor);
        }
"#;

/// The fog uniform locations of one forward shader program. Every material
/// whose fragment shader includes [`FOG_GLSL`] looks these up in
/// `initialize` and uploads via [`FogUniforms::set`] each draw. All seven
/// uniforms are statically referenced inside `applyFog`, so they stay active
/// and `get_uniform_location` never panics.
pub struct FogUniforms {
    enabled_loc: UniformLocation,
    mode_loc: UniformLocation,
    color_loc: UniformLocation,
    near_loc: UniformLocation,
    far_loc: UniformLocation,
    density_loc: UniformLocation,
    camera_pos_loc: UniformLocation,
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
            camera_pos_loc: shader.get_uniform_location(gl, "fogCameraPos"),
        }
    }

    /// Upload this draw's fog state. `None` sets only `fogEnabled = 0` (the
    /// other uniforms are dead behind the shader's early return).
    /// `camera_pos` is the camera's world position — frame-constant, so the
    /// renderer computes it once per pass (`RenderContext::camera_pos`).
    pub fn set(
        &self,
        p: &ShaderProgram,
        gl: &glow::Context,
        fog: Option<&Fog>,
        camera_pos: &Vector3<f32>,
    ) {
        let Some(fog) = fog else {
            p.set_uniform_1i(gl, &self.enabled_loc, 0);
            return;
        };
        p.set_uniform_1i(gl, &self.enabled_loc, 1);
        p.set_uniform_vec3(gl, &self.camera_pos_loc, camera_pos);
        let c = fog.color();
        p.set_uniform_vec3(gl, &self.color_loc, &Vector3::new(c[0], c[1], c[2]));
        match fog {
            Fog::Linear { near, far, .. } => {
                p.set_uniform_1i(gl, &self.mode_loc, 0);
                p.set_uniform_1f(gl, &self.near_loc, *near);
                p.set_uniform_1f(gl, &self.far_loc, *far);
                p.set_uniform_1f(gl, &self.density_loc, 0.0);
            }
            Fog::Exp { density, .. } => {
                p.set_uniform_1i(gl, &self.mode_loc, 1);
                p.set_uniform_1f(gl, &self.density_loc, *density);
                p.set_uniform_1f(gl, &self.near_loc, 0.0);
                p.set_uniform_1f(gl, &self.far_loc, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
