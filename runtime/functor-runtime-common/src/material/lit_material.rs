use cgmath::Matrix4;
use cgmath::Vector4;

use crate::light::resolve_lighting;
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// Diffuse-lit surface: albedo (a color, optionally modulated by a texture) shaded
// by ambient + a single directional ("sun") light via Lambert. Reads the frame's
// lights from `RenderContext`. Needs the vertex normal (attribute location 2).
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldNormal;

        void main() {
            texCoord = inTex;
            worldNormal = mat3(world) * inNormal;
            gl_Position = projection * view * world * vec4(inPos, 1.0);
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 worldNormal;

        uniform vec4 baseColor;
        uniform sampler2D texture1;
        uniform int useTexture;

        uniform vec3 ambientColor;
        uniform vec3 lightDir;   // direction the light travels (a "sun" ray)
        uniform vec3 lightColor; // already multiplied by intensity

        void main() {
            vec3 n = normalize(worldNormal);
            float ndotl = max(dot(n, -normalize(lightDir)), 0.0);
            vec3 lighting = ambientColor + lightColor * ndotl;

            vec4 albedo = baseColor;
            if (useTexture == 1) {
                albedo = texture(texture1, texCoord) * baseColor;
            }
            fragColor = vec4(albedo.rgb * lighting, albedo.a);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    base_color_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
    ambient_color_loc: UniformLocation,
    light_dir_loc: UniformLocation,
    light_color_loc: UniformLocation,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct LitMaterial {
    color: Vector4<f32>,
    use_texture: bool,
}

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for LitMaterial {
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
                    base_color_loc: shader.get_uniform_location(ctx.gl, "baseColor"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
                    use_texture_loc: shader.get_uniform_location(ctx.gl, "useTexture"),
                    ambient_color_loc: shader.get_uniform_location(ctx.gl, "ambientColor"),
                    light_dir_loc: shader.get_uniform_location(ctx.gl, "lightDir"),
                    light_color_loc: shader.get_uniform_location(ctx.gl, "lightColor"),
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
        let lighting = resolve_lighting(ctx.lights);
        unsafe {
            #[allow(static_mut_refs)]
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
                p.set_uniform_vec4(ctx.gl, &uniforms.base_color_loc, &self.color);
                p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);
                p.set_uniform_1i(ctx.gl, &uniforms.use_texture_loc, self.use_texture as i32);
                p.set_uniform_vec3(ctx.gl, &uniforms.ambient_color_loc, &lighting.ambient);
                p.set_uniform_vec3(ctx.gl, &uniforms.light_dir_loc, &lighting.directional_dir);
                p.set_uniform_vec3(ctx.gl, &uniforms.light_color_loc, &lighting.directional_color);
            }
        }

        true
    }
}

impl LitMaterial {
    /// `use_texture` expects a texture bound to unit 0 (the caller binds it); the
    /// sampled texel is multiplied by `color` as a tint/albedo.
    pub fn create(color: Vector4<f32>, use_texture: bool) -> Box<dyn Material> {
        Box::new(LitMaterial { color, use_texture })
    }
}
