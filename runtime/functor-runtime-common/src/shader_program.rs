use std::slice;

use cgmath::{conv::array4x4, Matrix3, Matrix4, Vector3, Vector4};
use glow::*;

pub struct ShaderProgram {
    pub program_id: Program,
}

pub struct UniformLocation {
    native_uniform_location: glow::UniformLocation,
}

use crate::shader::Shader;

impl ShaderProgram {
    pub fn link(
        gl: &glow::Context,
        vertex_shader: &Shader,
        fragment_shader: &Shader,
    ) -> ShaderProgram {
        unsafe {
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

    pub fn get_uniform_location(&self, gl: &glow::Context, uniform_name: &str) -> UniformLocation {
        unsafe {
            let native_uniform_location = gl
                .get_uniform_location(self.program_id, uniform_name)
                .expect(&format!("Cannot get uniform location: {}", &uniform_name));
            UniformLocation {
                native_uniform_location,
            }
        }
    }

    pub fn set_uniform_1i(&self, gl: &glow::Context, uniform_location: &UniformLocation, i: i32) {
        unsafe {
            gl.uniform_1_i32(Some(&uniform_location.native_uniform_location), i);
        }
    }

    /// Upload an `int[]` uniform (the slice sets `array[0..len]`).
    pub fn set_uniform_1iv(&self, gl: &glow::Context, uniform_location: &UniformLocation, v: &[i32]) {
        unsafe {
            gl.uniform_1_i32_slice(Some(&uniform_location.native_uniform_location), v);
        }
    }

    pub fn set_uniform_1f(&self, gl: &glow::Context, uniform_location: &UniformLocation, f: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&uniform_location.native_uniform_location), f);
        }
    }

    /// Upload a `float[]` uniform.
    pub fn set_uniform_1fv(&self, gl: &glow::Context, uniform_location: &UniformLocation, v: &[f32]) {
        unsafe {
            gl.uniform_1_f32_slice(Some(&uniform_location.native_uniform_location), v);
        }
    }

    /// Upload a `vec3[]` uniform from a flattened slice (length = 3 × count).
    pub fn set_uniform_vec3v(&self, gl: &glow::Context, uniform_location: &UniformLocation, v: &[f32]) {
        unsafe {
            gl.uniform_3_f32_slice(Some(&uniform_location.native_uniform_location), v);
        }
    }

    #[allow(dead_code)]
    pub fn set_uniform_vec3(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        vec: &Vector3<f32>,
    ) {
        unsafe {
            gl.uniform_3_f32_slice(
                Some(&uniform_location.native_uniform_location),
                &[vec.x, vec.y, vec.z],
            )
        }
    }

    #[allow(dead_code)]
    pub fn set_uniform_vec4(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        vec: &Vector4<f32>,
    ) {
        unsafe {
            gl.uniform_4_f32_slice(
                Some(&uniform_location.native_uniform_location),
                &[vec.x, vec.y, vec.z, vec.w],
            )
        }
    }

    pub fn set_uniform_matrix4(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        matrix: &Matrix4<f32>,
    ) {
        unsafe {
            let data = (&array4x4(*matrix) as *const [[f32; 4]; 4]) as *const f32;
            let raw = slice::from_raw_parts(data, 16);
            gl.uniform_matrix_4_f32_slice(
                Some(&uniform_location.native_uniform_location),
                false,
                raw,
            );
        }
    }

    pub fn set_uniform_matrix3(
        &self,
        gl: &glow::Context,
        uniform_location: &UniformLocation,
        matrix: &Matrix3<f32>,
    ) {
        let raw = [
            matrix.x.x, matrix.x.y, matrix.x.z, matrix.y.x, matrix.y.y, matrix.y.z, matrix.z.x,
            matrix.z.y, matrix.z.z,
        ];
        unsafe {
            gl.uniform_matrix_3_f32_slice(
                Some(&uniform_location.native_uniform_location),
                false,
                &raw,
            );
        }
    }

    pub fn set_uniform_matrix4fv(
        &self,
        gl: &glow::Context,
        location: &UniformLocation,
        values: &[f32],
    ) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(Some(&location.native_uniform_location), false, values)
        }
    }
}
