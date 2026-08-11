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
        let (gl_shader_type, gl_shader_description) = match &shader_type {
            ShaderType::Fragment => (glow::FRAGMENT_SHADER, "FRAGMENT"),
            ShaderType::Vertex => (glow::VERTEX_SHADER, "VERTEX"),
        };

        let shader;
        unsafe {
            let shader_source = convert(shader_contents, opengl_version, &shader_type);
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
 *
 * Fragment shaders additionally get the shared color-space helpers
 * ([`crate::color_space::OUTPUT_ENCODE_GLSL`]): `functorOutput`, the
 * linear→sRGB output epilogue gated by `uOutputSrgbEncode`, and
 * `functorSrgbToLinear` for decoding sRGB-authored inputs. Shaders that never
 * reference them (depth/debug) have them optimized out.
 */
fn convert(shader: &str, shader_version: &str, shader_type: &ShaderType) -> String {
    // Compatibility context for shader
    let preamble: &str = r#"
            #ifndef GL_ES
            #define highp
            #else
            precision mediump float;
            #endif
    "#;

    let color_space = match shader_type {
        ShaderType::Fragment => crate::color_space::OUTPUT_ENCODE_GLSL,
        ShaderType::Vertex => "",
    };

    [shader_version, "\n", preamble, color_space, shader].join("\n")
}
