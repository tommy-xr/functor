use cgmath::Matrix4;

use std::sync::OnceLock;

use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

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

        void main() {
            fragColor = vec4(texCoord.x, texCoord.y, 0.0, 1.0);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
}

// static SHADER_PROGRAM: OnceLock<(ShaderProgram, Uniforms)> = OnceLock::new();

pub struct BasicMaterial;

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for BasicMaterial {
    fn initialize(&mut self, ctx: &RenderContext) {}
    //     let _ = SHADER_PROGRAM.get_or_init(|| {
    //         // build and compile our shader program
    //         // ------------------------------------
    //         // vertex shader
    //         let vertex_shader =
    //             Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER_SOURCE, opengl_version);

    //         // fragment shader
    //         let fragment_shader = Shader::build(
    //             gl,
    //             ShaderType::Fragment,
    //             FRAGMENT_SHADER_SOURCE,
    //             opengl_version,
    //         );
    //         // link shaders

    //         let shader =
    //             crate::shader_program::ShaderProgram::link(&gl, &vertex_shader, &fragment_shader);

    //         let uniforms = Uniforms {
    //             world_loc: shader.get_uniform_location(gl, "world"),
    //             view_loc: shader.get_uniform_location(gl, "view"),
    //             projection_loc: shader.get_uniform_location(gl, "projection"),
    //         };

    //         (shader, uniforms)
    //     });
    // }

    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        // TODO:
        let gl = ctx.gl;
        // build and compile our shader program
        // ------------------------------------
        // vertex shader
        let vertex_shader = Shader::build(
            gl,
            ShaderType::Vertex,
            VERTEX_SHADER_SOURCE,
            ctx.shader_version,
        );

        // fragment shader
        let fragment_shader = Shader::build(
            gl,
            ShaderType::Fragment,
            FRAGMENT_SHADER_SOURCE,
            ctx.shader_version,
        );
        // link shaders

        let shader =
            crate::shader_program::ShaderProgram::link(&gl, &vertex_shader, &fragment_shader);

        let uniforms = Uniforms {
            world_loc: shader.get_uniform_location(gl, "world"),
            view_loc: shader.get_uniform_location(gl, "view"),
            projection_loc: shader.get_uniform_location(gl, "projection"),
        };

        // (shader, uniforms)

        // let (shader_program, uniforms) = SHADER_PROGRAM.get().expect("shader not compiled");
        let p = shader;
        p.use_program(gl);

        p.set_uniform_matrix4(gl, &uniforms.world_loc, world_matrix);
        p.set_uniform_matrix4(gl, &uniforms.view_loc, view_matrix);
        p.set_uniform_matrix4(gl, &uniforms.projection_loc, projection_matrix);

        true
    }
}

impl BasicMaterial {
    pub fn create() -> Box<dyn Material> {
        Box::new(BasicMaterial)
    }
}
