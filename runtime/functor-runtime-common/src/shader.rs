use glow::*;

pub struct Shader {
    pub shader_id: glow::Shader,
}

pub enum ShaderType {
    Fragment,
    Vertex,
}

impl Shader {
    pub fn build(
        gl: &glow::Context,
        shader_type: ShaderType,
        shader_contents: &str,
        opengl_version: &str,
    ) -> Shader {
        let (gl_shader_type, gl_shader_description) = match shader_type {
            ShaderType::Fragment => (glow::FRAGMENT_SHADER, "FRAGMENT"),
            ShaderType::Vertex => (glow::VERTEX_SHADER, "VERTEX"),
        };

        let shader;
        unsafe {
            let shader_source = convert(shader_contents, opengl_version);
            shader = gl
                .create_shader(gl_shader_type)
                .expect("Cannot create shader");
            gl.shader_source(shader, &shader_source);
            gl.compile_shader(shader);

            if !gl.get_shader_compile_status(shader) {
                panic!(
                    "{}:{}",
                    gl_shader_description,
                    gl.get_shader_info_log(shader)
                );
            }
        }

        Shader { shader_id: shader }
    }
}

/**
 * convert converts an agnostic shader to either 320 es or 410
 */
fn convert(shader: &str, shader_version: &str) -> String {
    // Compatibility context for shader
    let preamble: &str = r#"
            #ifndef GL_ES
            #define highp
            #else
            precision mediump float;
            #endif
    "#;

    [shader_version, "\n", preamble, shader].join("\n")
}
