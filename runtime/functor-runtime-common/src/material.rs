use cgmath::Matrix4;

pub trait Material {
    fn initialize(&mut self, gl: &glow::Context, opengl_version: &str);
    fn draw_opaque(
        &self,
        gl: &glow::Context,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool;
    fn draw_transparent(
        &self,
        _gl: &glow::Context,
        _projection_matrix: &Matrix4<f32>,
        _view_matrix: &Matrix4<f32>,
        _world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        false
    }
}

pub mod color_material {
    use cgmath::Matrix4;
    use cgmath::Vector3;
    use once_cell::sync::OnceCell;

    use crate::shader_program::ShaderProgram;
    use crate::shader_program::UniformLocation;

    use super::Material;

    const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;
        uniform vec3 color;

        out vec3 vertexColor;

        void main() {
            vertexColor = color;
            gl_Position = projection * view * world * vec4(inPos, 1.0);
        }
"#;

    const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 vertexColor;

        void main() {
            fragColor = vec4(vertexColor.rgb, 1.0);
        }
"#;

    struct Uniforms {
        world_loc: UniformLocation,
        view_loc: UniformLocation,
        projection_loc: UniformLocation,
        color_loc: UniformLocation,
    }

    static SHADER_PROGRAM: OnceCell<(ShaderProgram, Uniforms)> = OnceCell::new();

    pub struct ColorMaterial {
        pub color: Vector3<f32>,
    }

    use crate::shader::Shader;
    use crate::shader::ShaderType;

    impl Material for ColorMaterial {
        fn initialize(&mut self, gl: &glow::Context, opengl_version: &str) {
            let _ = SHADER_PROGRAM.get_or_init(|| {
                // build and compile our shader program
                // ------------------------------------
                // vertex shader
                let vertex_shader =
                    Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER_SOURCE, opengl_version);

                // fragment shader
                let fragment_shader = Shader::build(
                    gl,
                    ShaderType::Fragment,
                    FRAGMENT_SHADER_SOURCE,
                    opengl_version,
                );
                // link shaders

                let shader = crate::shader_program::ShaderProgram::link(
                    &gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(gl, "world"),
                    view_loc: shader.get_uniform_location(gl, "view"),
                    projection_loc: shader.get_uniform_location(gl, "projection"),
                    color_loc: shader.get_uniform_location(gl, "color"),
                };

                (shader, uniforms)
            });
        }

        fn draw_opaque(
            &self,
            gl: &glow::Context,
            projection_matrix: &Matrix4<f32>,
            view_matrix: &Matrix4<f32>,
            world_matrix: &Matrix4<f32>,
            _skinning_data: &[Matrix4<f32>],
        ) -> bool {
            let (shader_program, uniforms) = SHADER_PROGRAM.get().expect("shader not compiled");
            let p = shader_program;
            p.use_program(gl);

            p.set_uniform_matrix4(gl, &uniforms.world_loc, world_matrix);
            p.set_uniform_matrix4(gl, &uniforms.view_loc, view_matrix);
            p.set_uniform_matrix4(gl, &uniforms.projection_loc, projection_matrix);

            p.set_uniform_vec3(gl, &uniforms.color_loc, &self.color);
            true
        }
    }

    impl ColorMaterial {
        pub fn create(color: Vector3<f32>) -> Box<dyn Material> {
            Box::new(ColorMaterial { color })
        }
    }
}
