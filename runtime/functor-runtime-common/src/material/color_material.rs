use cgmath::Matrix4;
use cgmath::Vector4;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec3 worldPos;

        void main() {
            worldPos = (world * vec4(inPos, 1.0)).xyz;
            gl_Position = projection * view * world * vec4(inPos, 1.0);
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 worldPos;

        uniform vec4 color;

        void main() {
            fragColor = vec4(applyFog(color.rgb, worldPos), color.a);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    color_loc: UniformLocation,
    fog: FogUniforms,
}

// TODO: We'll have to re-think this pattern
// Maybe we need a shader repository or something to pull from
static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct ColorMaterial(Vector4<f32>);

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for ColorMaterial {
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

                // fragment shader
                let fragment_source = format!("{}\n{}", FOG_GLSL, FRAGMENT_SHADER_SOURCE);
                let fragment_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Fragment,
                    &fragment_source,
                    ctx.shader_version,
                );
                // link shaders

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    color_loc: shader.get_uniform_location(ctx.gl, "color"),
                    fog: FogUniforms::get(&shader, ctx.gl),
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
            // TODO: Find another approach to do this - maybe a shader repository?
            #[allow(static_mut_refs)]
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
                p.set_uniform_vec4(ctx.gl, &uniforms.color_loc, &self.0);
                uniforms.fog.set(p, ctx.gl, ctx.fog, &ctx.camera_pos);
            }
        }

        true
    }
}

impl ColorMaterial {
    pub fn create(color: Vector4<f32>) -> Box<dyn Material> {
        Box::new(ColorMaterial(color))
    }
}
