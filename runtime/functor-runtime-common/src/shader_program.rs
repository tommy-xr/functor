use cgmath::{conv::array4x4, Matrix4, Vector3};
use glow::*;

pub struct ShaderProgram {
    pub program_id: NativeProgram,
}

pub struct UniformLocation {
    native_uniform_location: NativeUniformLocation,
}

use crate::shader::Shader;

impl ShaderProgram {
    pub fn link(
        gl: &glow::Context,
        vertex_shader: &Shader,
        fragment_shader: &Shader,
    ) -> ShaderProgram {
        unsafe {
            let mut success = 0;
            let program_id = gl.create_program().expect("Cannot create program");
            gl.attach_shader(program_id, vertex_shader.shader_id);
            gl.attach_shader(program_id, fragment_shader.shader_id);

            gl.link_program(program_id);

            if !gl.get_program_link_status(program_id) {
                panic!("{}", gl.get_program_info_log(program_id));
            }

            ShaderProgram { program_id }
        }
    }

    pub fn use_program(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program_id));
        }
    }

    pub fn set_uniform_vec3(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        matrix: &Vector3<f32>,
    ) {
        panic!("TODO: Set uniformation matrix");
    }

    pub fn set_uniform_matrix4(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        matrix: &Matrix4<f32>,
    ) {
        unsafe {
            let location = gl.get_uniform_location(self.program_id, name).unwrap();
            panic!("TODO: Set uniformation matrix");
            // let data = (&array4x4(*matrix) as *const [[f32; 4]; 4]) as *const f32;
            // gl.uniform_matrix_4_f32_slice(Some(location), false, data);
        }
    }
}
