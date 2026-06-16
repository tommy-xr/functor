use cgmath::Matrix4;
use cgmath::Vector4;

use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// Self-lit ("emissive") surface: emits a constant color, optionally modulated
// by a texture (neon signage). Rendered fullbright — no lighting term — so once
// real lights land this stays bright while lit surfaces are shaded around it.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;

        void main() {
            texCoord = inTex;
            gl_Position = projection * view * world * vec4(inPos, 1.0);
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;

        uniform vec4 emissiveColor;
        uniform sampler2D texture1;
        uniform int useTexture;

        void main() {
            if (useTexture == 1) {
                fragColor = texture(texture1, texCoord) * emissiveColor;
            } else {
                fragColor = emissiveColor;
            }
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    emissive_color_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct EmissiveMaterial {
    color: Vector4<f32>,
    use_texture: bool,
}

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for EmissiveMaterial {
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
                    emissive_color_loc: shader.get_uniform_location(ctx.gl, "emissiveColor"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
                    use_texture_loc: shader.get_uniform_location(ctx.gl, "useTexture"),
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
                p.set_uniform_vec4(ctx.gl, &uniforms.emissive_color_loc, &self.color);
                p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);
                p.set_uniform_1i(ctx.gl, &uniforms.use_texture_loc, self.use_texture as i32);
            }
        }

        true
    }
}

impl EmissiveMaterial {
    /// `use_texture` expects a texture already bound to unit 0 (the caller binds
    /// it); the sampled texel is multiplied by `color` as a tint.
    pub fn create(color: Vector4<f32>, use_texture: bool) -> Box<dyn Material> {
        Box::new(EmissiveMaterial { color, use_texture })
    }
}
