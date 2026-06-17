use cgmath::Matrix4;

use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// Depth-only material for the shadow pass: transforms positions by the light's
// matrices and writes nothing meaningful to color (the shadow FBO has no color
// attachment). Used to fill the shadow map; see `shadow.rs`.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        void main() {
            gl_Position = projection * view * world * vec4(inPos, 1.0);
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        // Pack a [0,1] depth into RGBA8 (24-bit precision) so the shadow map can
        // be a portable RGBA8 texture. The lit shader unpacks with the matching
        // dot product.
        vec4 packDepth(float depth) {
            vec4 enc = vec4(1.0, 255.0, 65025.0, 16581375.0) * depth;
            enc = fract(enc);
            enc -= enc.yzww * vec4(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 0.0);
            return enc;
        }

        void main() {
            fragColor = packDepth(gl_FragCoord.z);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct DepthMaterial;

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for DepthMaterial {
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

                let fragment_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Fragment,
                    FRAGMENT_SHADER_SOURCE,
                    ctx.shader_version,
                );

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
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
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);
                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
            }
        }

        true
    }
}

impl DepthMaterial {
    pub fn create() -> Box<dyn Material> {
        Box::new(DepthMaterial)
    }
}
