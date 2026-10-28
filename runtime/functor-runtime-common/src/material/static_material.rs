use cgmath::Matrix4;

use crate::math::normal_matrix;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::light::{lighting_glsl, LightingUniforms};
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// One material group of a static data model (`Model.static`): the group's
// base texture (unit 0; white when `useTexture` is 0) shaded by the frame's
// lights with the shared light loop, no highlight (like glTF models). An
// `unlit` group, or any group in a frame without lights, draws its base
// fullbright, as static models do in `Frame.create`.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;

        uniform mat4 world;
        uniform mat3 normalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldNormal;
        out vec3 worldPos;

        void main() {
            texCoord = inTex;
            // Unit length at the vertex stage's highp (see LitMaterial).
            worldNormal = normalize(normalMatrix * inNormal);
            vec4 wp = world * vec4(inPos, 1.0);
            worldPos = wp.xyz;
            gl_Position = projection * view * wp;
        }
"#;

// Concatenated after `FOG_GLSL` + `lighting_glsl()`.
const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 worldNormal;
        in highp vec3 worldPos;

        uniform sampler2D texture1;
        uniform int useTexture;
        uniform int unlit;

        void main() {
            vec4 base = useTexture == 1 ? texture(texture1, texCoord) : vec4(1.0);
            vec3 color = base.rgb;
            if (unlit == 0 && numLights > 0) {
                vec3 diffuseLight;
                vec3 specularLight;
                accumulateLights(normalize(worldNormal), worldPos, diffuseLight, specularLight);
                color = base.rgb * diffuseLight;
            }
            fragColor = functorOutput(vec4(applyFog(color, worldPos), base.a));
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    normal_matrix_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
    unlit_loc: UniformLocation,
    lighting: LightingUniforms,
    fog: FogUniforms,
    output: crate::color_space::OutputEncodeUniform,
    pass: crate::PassUniforms,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

/// A static model's shader: `draw_opaque` sets the model's uniforms, then
/// [`StaticMaterial::set_group`] each group's (its texture bound to unit 0).
pub struct StaticMaterial;

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for StaticMaterial {
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
                let fragment_source =
                    format!("{}\n{}\n{}", FOG_GLSL, lighting_glsl(), FRAGMENT_SHADER_SOURCE);
                let fragment_shader =
                    Shader::build(ctx.gl, ShaderType::Fragment, &fragment_source, ctx.shader_version);
                let shader = ShaderProgram::link(ctx.gl, &vertex_shader, &fragment_shader);
                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    normal_matrix_loc: shader.get_uniform_location(ctx.gl, "normalMatrix"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
                    use_texture_loc: shader.get_uniform_location(ctx.gl, "useTexture"),
                    unlit_loc: shader.get_uniform_location(ctx.gl, "unlit"),
                    lighting: LightingUniforms::get(&shader, ctx.gl),
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
                    p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);
                    uniforms.lighting.set_pass(p, ctx, view_matrix);
                    uniforms.fog.set(p, ctx.gl, ctx.fog, &ctx.camera_pos);
                }
                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix3(
                    ctx.gl,
                    &uniforms.normal_matrix_loc,
                    &normal_matrix(world_matrix),
                );
                LightingUniforms::bind_shadow_map(ctx);
                uniforms.output.set(p, ctx.gl, ctx.output_srgb_encode, ctx.output_alpha());
            }
        }
        true
    }
}

impl StaticMaterial {
    /// The next group's state; call after `draw_opaque`, before its draw.
    pub fn set_group(&self, ctx: &RenderContext, use_texture: bool, unlit: bool) {
        unsafe {
            #[allow(static_mut_refs)]
            if let Some((p, uniforms)) = &SHADER_PROGRAM {
                p.set_uniform_1i(ctx.gl, &uniforms.use_texture_loc, use_texture as i32);
                p.set_uniform_1i(ctx.gl, &uniforms.unlit_loc, unlit as i32);
            }
        }
    }
}
