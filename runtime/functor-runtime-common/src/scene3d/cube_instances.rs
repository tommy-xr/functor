use std::mem::{offset_of, size_of};

use cgmath::Matrix4;
use glow::HasContext;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::geometry::cube_mesh;
use crate::math::normal_matrix;
use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::VertexPositionTexture;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::{DebugRenderMode, RenderContext};

use super::CubeInstance;

const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;
        layout (location = 4) in vec3 instancePosition;
        layout (location = 5) in vec3 instanceScale;
        layout (location = 6) in vec3 instanceColor;

        uniform mat4 world;
        uniform mat3 normalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out vec3 worldPos;
        out vec3 color;
        out vec3 worldNormal;
        out vec3 worldTangent;

        void main() {
            vec3 local = inPos * instanceScale + instancePosition;
            vec4 wp = world * vec4(local, 1.0);
            worldPos = wp.xyz;
            color = instanceColor;
            // Applying the inverse instance scale before the enclosing
            // transform is the inverse-transpose normal transform for the
            // complete `world * scale` matrix.
            worldNormal = normalMatrix * (inNormal / instanceScale);
            worldTangent = mat3(world) * (inTangent.xyz * instanceScale);
            gl_Position = projection * view * wp;
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 worldPos;
        in vec3 color;
        in vec3 worldNormal;
        in vec3 worldTangent;

        uniform int debugMode; // 0 = authored, 1 = normals, 2 = tangents

        void main() {
            if (debugMode == 1) {
                fragColor = vec4(normalize(worldNormal) * 0.5 + 0.5, 1.0);
            } else if (debugMode == 2) {
                fragColor = vec4(normalize(worldTangent) * 0.5 + 0.5, 1.0);
            } else {
                fragColor = vec4(applyFog(color, worldPos), 1.0);
            }
        }
"#;

struct Uniforms {
    world: UniformLocation,
    normal_matrix: UniformLocation,
    view: UniformLocation,
    projection: UniformLocation,
    fog: FogUniforms,
    debug_mode: UniformLocation,
}

/// Persistent GPU state for `Scene.cubeInstances`. The static unit-cube mesh
/// uploads once; the compact instance buffer is replaced in place per node and
/// submitted with one `draw_elements_instanced` call.
pub(super) struct CubeInstanceRenderer {
    vao: glow::VertexArray,
    _vertex_buffer: glow::Buffer,
    index_buffer: glow::Buffer,
    instance_buffer: glow::Buffer,
    index_count: i32,
    instance_capacity: usize,
    shader: ShaderProgram,
    uniforms: Uniforms,
}

impl CubeInstanceRenderer {
    pub(super) fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let (vertices, indices) = cube_mesh();
        let vertex_bytes = as_bytes(&vertices);
        let index_bytes = as_bytes(&indices);

        unsafe {
            let vao = gl.create_vertex_array().expect("cube instance VAO");
            let vertex_buffer = gl.create_buffer().expect("cube instance vertices");
            let index_buffer = gl.create_buffer().expect("cube instance indices");
            let instance_buffer = gl.create_buffer().expect("cube instance data");
            let counters = crate::gpu_counters::gpu_counters();
            counters.vao_created();
            counters.buffer_created();
            counters.buffer_created();
            counters.buffer_created();

            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::STATIC_DRAW);
            counters.uploaded(vertex_bytes.len());
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
            counters.uploaded(index_bytes.len());

            let stride = VertexPositionTexture::get_total_size() as i32;
            for (location, attribute) in VertexPositionTexture::get_vertex_attributes()
                .iter()
                .enumerate()
            {
                gl.enable_vertex_attrib_array(location as u32);
                match attribute.attribute_type {
                    VertexAttributeType::Float => gl.vertex_attrib_pointer_f32(
                        location as u32,
                        attribute.size,
                        glow::FLOAT,
                        false,
                        stride,
                        attribute.offset as i32,
                    ),
                }
            }

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(instance_buffer));
            let instance_stride = size_of::<CubeInstance>() as i32;
            for (location, offset) in [
                (4, offset_of!(CubeInstance, position)),
                (5, offset_of!(CubeInstance, scale)),
                (6, offset_of!(CubeInstance, color)),
            ] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    3,
                    glow::FLOAT,
                    false,
                    instance_stride,
                    offset as i32,
                );
                gl.vertex_attrib_divisor(location, 1);
            }

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            let vertex_shader =
                Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER_SOURCE, shader_version);
            let fragment_shader = Shader::build(
                gl,
                ShaderType::Fragment,
                &format!("{}\n{}", FOG_GLSL, FRAGMENT_SHADER_SOURCE),
                shader_version,
            );
            let shader = ShaderProgram::link(gl, &vertex_shader, &fragment_shader);
            let uniforms = Uniforms {
                world: shader.get_uniform_location(gl, "world"),
                normal_matrix: shader.get_uniform_location(gl, "normalMatrix"),
                view: shader.get_uniform_location(gl, "view"),
                projection: shader.get_uniform_location(gl, "projection"),
                fog: FogUniforms::get(&shader, gl),
                debug_mode: shader.get_uniform_location(gl, "debugMode"),
            };

            Self {
                vao,
                _vertex_buffer: vertex_buffer,
                index_buffer,
                instance_buffer,
                index_count: indices.len() as i32,
                instance_capacity: 0,
                shader,
                uniforms,
            }
        }
    }

    pub(super) fn draw(
        &mut self,
        ctx: &RenderContext,
        instances: &[CubeInstance],
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let bytes = as_bytes(instances);
        unsafe {
            ctx.gl.bind_vertex_array(Some(self.vao));
            ctx.gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_buffer));
            if bytes.len() > self.instance_capacity {
                ctx.gl
                    .buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::DYNAMIC_DRAW);
                self.instance_capacity = bytes.len();
            } else {
                ctx.gl
                    .buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            }
            crate::gpu_counters::gpu_counters().uploaded(bytes.len());

            self.shader.use_program(ctx.gl);
            self.shader
                .set_uniform_matrix4(ctx.gl, &self.uniforms.world, world);
            self.shader.set_uniform_matrix3(
                ctx.gl,
                &self.uniforms.normal_matrix,
                &normal_matrix(world),
            );
            self.shader
                .set_uniform_matrix4(ctx.gl, &self.uniforms.view, view);
            self.shader
                .set_uniform_matrix4(ctx.gl, &self.uniforms.projection, projection);
            self.uniforms
                .fog
                .set(&self.shader, ctx.gl, ctx.fog, &ctx.camera_pos);
            let debug_mode = match ctx.debug_render_mode {
                DebugRenderMode::Normals => 1,
                DebugRenderMode::Tangents => 2,
                DebugRenderMode::Default
                | DebugRenderMode::Transparent
                | DebugRenderMode::Physics => 0,
            };
            self.shader
                .set_uniform_1i(ctx.gl, &self.uniforms.debug_mode, debug_mode);

            ctx.gl
                .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.index_buffer));
            ctx.gl.draw_elements_instanced(
                glow::TRIANGLES,
                self.index_count,
                glow::UNSIGNED_INT,
                0,
                instances.len() as i32,
            );
        }
    }
}

fn as_bytes<T>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}
