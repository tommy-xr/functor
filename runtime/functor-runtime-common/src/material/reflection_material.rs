use cgmath::{Matrix4, Vector3, Vector4};

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::math::normal_matrix;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::RenderContext;

use super::Material;

/// The sky reflected about a surface, scaled by Schlick's fresnel:
/// `base + (1 − base)(1 − |n·v|)⁵`. The sky is the frame's gradient (or its
/// background colour twice), read along the reflected view ray. Needs
/// `FOG_GLSL` (its `skyGradient`) first. Shared with the instanced program.
pub(crate) const REFLECTION_GLSL: &str = r#"
        uniform vec3 reflectHorizon; // linear
        uniform vec3 reflectZenith;  // linear
        uniform highp vec3 reflectEye;
        uniform float reflectBase;

        vec3 skyReflection(vec3 normal, highp vec3 worldPosition) {
            highp vec3 v = normalize(reflectEye - worldPosition);
            vec3 n = normalize(normal);
            float nv = dot(n, v);
            float f = reflectBase
                + (1.0 - reflectBase) * pow(clamp(1.0 - abs(nv), 0.0, 1.0), 5.0);
            // Panes are two-sided: reflect about the face the eye sees.
            vec3 r = reflect(-v, nv < 0.0 ? -n : n);
            return skyGradient(reflectHorizon, reflectZenith, r) * f;
        }
"#;

/// The [`REFLECTION_GLSL`] uniforms of one program.
pub(crate) struct ReflectionUniforms {
    horizon: UniformLocation,
    zenith: UniformLocation,
    eye: UniformLocation,
    base: UniformLocation,
}

impl ReflectionUniforms {
    pub(crate) fn get(shader: &ShaderProgram, gl: &glow::Context) -> Self {
        Self {
            horizon: shader.get_uniform_location(gl, "reflectHorizon"),
            zenith: shader.get_uniform_location(gl, "reflectZenith"),
            eye: shader.get_uniform_location(gl, "reflectEye"),
            base: shader.get_uniform_location(gl, "reflectBase"),
        }
    }

    /// The pass's sky (authored, decoded here) and eye, and this draw's base.
    pub(crate) fn set(&self, p: &ShaderProgram, ctx: &RenderContext, base: f32) {
        let (horizon, zenith) = ctx.reflection_sky;
        let h = crate::color_space::srgb_to_linear3(horizon);
        let z = crate::color_space::srgb_to_linear3(zenith);
        p.set_uniform_vec3(ctx.gl, &self.horizon, &Vector3::new(h[0], h[1], h[2]));
        p.set_uniform_vec3(ctx.gl, &self.zenith, &Vector3::new(z[0], z[1], z[2]));
        p.set_uniform_vec3(ctx.gl, &self.eye, &ctx.camera_pos);
        p.set_uniform_1f(ctx.gl, &self.base, base);
    }
}

const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 2) in vec3 inNormal;

        uniform mat4 world;
        uniform mat3 normalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out highp vec3 worldPos;
        out vec3 worldNormal;

        void main() {
            vec4 wp = world * vec4(inPos, 1.0);
            worldPos = wp.xyz;
            worldNormal = normalize(normalMatrix * inNormal);
            gl_Position = projection * view * wp;
        }
"#;

// Additive: the term added to what is behind. Fog fades it toward the blend
// pass's fog colour, black, so a far pane adds nothing.
const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in highp vec3 worldPos;
        in vec3 worldNormal;

        uniform vec4 reflectColor; // linear

        void main() {
            vec3 c = reflectColor.rgb * skyReflection(worldNormal, worldPos);
            fragColor = functorOutput(vec4(applyFog(c, worldPos), 1.0));
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    normal_matrix_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    color_loc: UniformLocation,
    reflection: ReflectionUniforms,
    fog: FogUniforms,
    output: crate::color_space::OutputEncodeUniform,
    pass: crate::PassUniforms,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

/// `Scene.reflection`: an additive sky reflection with a fresnel term. Drawn
/// only in the additive blend pass, which sets the blend equation.
pub struct ReflectionMaterial {
    color: Vector4<f32>,
    base: f32,
}

impl Material for ReflectionMaterial {
    fn initialize(&mut self, ctx: &RenderContext) {
        unsafe {
            #[allow(static_mut_refs)]
            if SHADER_PROGRAM.is_none() {
                let vertex_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Vertex,
                    VERTEX_SHADER_SOURCE,
                    ctx.shader_version,
                );
                let fragment_source = format!(
                    "{}\n{}\n{}",
                    FOG_GLSL, REFLECTION_GLSL, FRAGMENT_SHADER_SOURCE
                );
                let fragment_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Fragment,
                    &fragment_source,
                    ctx.shader_version,
                );
                let shader = ShaderProgram::link(ctx.gl, &vertex_shader, &fragment_shader);
                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    normal_matrix_loc: shader.get_uniform_location(ctx.gl, "normalMatrix"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    color_loc: shader.get_uniform_location(ctx.gl, "reflectColor"),
                    reflection: ReflectionUniforms::get(&shader, ctx.gl),
                    fog: FogUniforms::get(&shader, ctx.gl),
                    output: crate::color_space::OutputEncodeUniform::get(&shader, ctx.gl),
                    pass: Default::default(),
                };
                SHADER_PROGRAM = Some((shader, uniforms));
            }
        }
    }

    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        unsafe {
            #[allow(static_mut_refs)]
            if let Some((p, uniforms)) = &SHADER_PROGRAM {
                p.use_program(ctx.gl);
                if uniforms.pass.begin(ctx) {
                    p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                    p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
                    uniforms
                        .fog
                        .set(p, ctx.gl, ctx.fog, ctx.sky_gradient, &ctx.camera_pos);
                }
                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix3(
                    ctx.gl,
                    &uniforms.normal_matrix_loc,
                    &normal_matrix(world_matrix),
                );
                // Authored color: decode sRGB→linear at the uniform boundary.
                p.set_uniform_vec4(
                    ctx.gl,
                    &uniforms.color_loc,
                    &crate::color_space::srgb_to_linear_vec4(self.color),
                );
                uniforms.reflection.set(p, ctx, self.base);
                uniforms
                    .output
                    .set(p, ctx.gl, ctx.output_srgb_encode, ctx.output_alpha());
            }
        }
        true
    }
}

impl ReflectionMaterial {
    pub fn create(color: Vector4<f32>, base: f32) -> Box<dyn Material> {
        Box::new(ReflectionMaterial { color, base })
    }
}
