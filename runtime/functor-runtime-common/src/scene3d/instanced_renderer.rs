//! The hardware half of `Scene.instanced`: one `draw_elements_instanced` call
//! per recognized template, in both the forward pass (unlit or lit, fogged,
//! tinted per instance) and the depth pass (so instances cast shadows exactly
//! like their stamped equivalent).
//!
//! Persistent GPU state is lazily created on first use — scenes that never
//! instance allocate nothing here. Per-primitive static meshes upload once;
//! each node re-uploads only its compact 52-byte-per-copy instance records.

use std::collections::HashMap;
use std::mem::{offset_of, size_of};

use cgmath::Matrix4;
use glow::HasContext;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::geometry::{
    cube_mesh_data, cylinder_mesh_data, plane_mesh_data, quad_mesh_data, sphere_mesh_data,
};
use crate::light::{lighting_glsl, LightingUniforms};
use crate::math::normal_matrix;
use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::VertexPositionTexture;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::{DebugRenderMode, RenderContext, RenderPass};

use super::instancing::{InstanceData, InstancedPrimitive, RecognizedTemplate};
use super::MaterialDescription;

// The instance transform is `translate * rotate * scale` applied on top of
// the template's internal `local` matrix, all under the node's accumulated
// `world`. Normals are covectors: the inverse-transpose of `R * S` is
// `R * S⁻¹`, hence rotate(normal / instanceScale); the template-internal part
// uses the precomputed `localNormalMatrix`, and the enclosing `world` uses
// `normalMatrix`. Tangents are ordinary directions and keep the plain
// matrices.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;
        layout (location = 4) in vec3 instancePosition;
        layout (location = 5) in vec4 instanceRotation;
        layout (location = 6) in vec3 instanceScale;
        layout (location = 7) in vec3 instanceTint;

        uniform mat4 world;
        uniform mat4 local;
        uniform mat3 normalMatrix;
        uniform mat3 localNormalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out vec3 worldPos;
        out vec3 worldNormal;
        out vec3 worldTangent;
        out vec3 tintColor;

        vec3 rotate(vec4 q, vec3 v) {
            return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
        }

        void main() {
            vec4 lp = local * vec4(inPos, 1.0);
            vec3 p = rotate(instanceRotation, lp.xyz * instanceScale) + instancePosition;
            vec4 wp = world * vec4(p, 1.0);
            worldPos = wp.xyz;
            tintColor = instanceTint;
            vec3 ln = localNormalMatrix * inNormal;
            worldNormal = normalMatrix * rotate(instanceRotation, ln / instanceScale);
            vec3 lt = mat3(local) * inTangent.xyz;
            worldTangent = mat3(world) * rotate(instanceRotation, lt * instanceScale);
            gl_Position = projection * view * wp;
        }
"#;

// Concatenated after `FOG_GLSL` + `lighting_glsl()` so the lit mode's light
// loop, shadow sampling, and uniforms match `LitMaterial` exactly.
const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 worldPos;
        in vec3 worldNormal;
        in vec3 worldTangent;
        in vec3 tintColor;

        uniform vec4 baseColor;
        uniform int materialMode; // 0 = unlit (color/emissive), 1 = lit
        uniform int debugMode;    // 0 = authored, 1 = normals, 2 = tangents

        void main() {
            if (debugMode == 1) {
                fragColor = vec4(normalize(worldNormal) * 0.5 + 0.5, 1.0);
            } else if (debugMode == 2) {
                fragColor = vec4(normalize(worldTangent) * 0.5 + 0.5, 1.0);
            } else {
                vec3 albedo = baseColor.rgb * tintColor;
                vec3 shaded = albedo;
                if (materialMode == 1) {
                    vec3 n = normalize(worldNormal);
                    vec3 diffuseLight;
                    vec3 specularLight;
                    accumulateLights(n, worldPos, diffuseLight, specularLight);
                    shaded = albedo * diffuseLight + specularLight;
                }
                fragColor = vec4(applyFog(shaded, worldPos), baseColor.a);
            }
        }
"#;

// Depth-pass twin: positions only, packing depth exactly like `DepthMaterial`
// so instanced copies land in the same RGBA8 shadow map.
const DEPTH_VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 4) in vec3 instancePosition;
        layout (location = 5) in vec4 instanceRotation;
        layout (location = 6) in vec3 instanceScale;

        uniform mat4 world;
        uniform mat4 local;
        uniform mat4 view;
        uniform mat4 projection;

        vec3 rotate(vec4 q, vec3 v) {
            return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
        }

        void main() {
            vec4 lp = local * vec4(inPos, 1.0);
            vec3 p = rotate(instanceRotation, lp.xyz * instanceScale) + instancePosition;
            gl_Position = projection * view * world * vec4(p, 1.0);
        }
"#;

const DEPTH_FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        vec4 packDepth(float depth) {
            vec4 enc = vec4(1.0, 255.0, 65025.0, 16581375.0) * depth;
            enc = fract(enc);
            enc -= enc.yzww * vec4(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 0.0);
            return enc;
        }

        void main() {
            fragColor = packDepth(gl_FragCoord.z);
        }
"#;

struct ForwardUniforms {
    world: UniformLocation,
    local: UniformLocation,
    normal_matrix: UniformLocation,
    local_normal_matrix: UniformLocation,
    view: UniformLocation,
    projection: UniformLocation,
    base_color: UniformLocation,
    material_mode: UniformLocation,
    debug_mode: UniformLocation,
    lighting: LightingUniforms,
    fog: FogUniforms,
}

struct DepthUniforms {
    world: UniformLocation,
    local: UniformLocation,
    view: UniformLocation,
    projection: UniformLocation,
}

/// One primitive's persistent buffers: the static mesh plus that mesh's
/// per-instance buffer (a VAO's attribute pointers bind to specific buffers,
/// so each primitive owns its own instance buffer).
struct MeshBuffers {
    vao: glow::VertexArray,
    _vertex_buffer: glow::Buffer,
    index_buffer: glow::Buffer,
    instance_buffer: glow::Buffer,
    index_count: i32,
    instance_capacity: usize,
}

/// Persistent GPU state for `Scene.instanced`'s hardware path.
pub(super) struct InstancedRenderer {
    meshes: HashMap<InstancedPrimitive, MeshBuffers>,
    forward: ShaderProgram,
    forward_uniforms: ForwardUniforms,
    depth: ShaderProgram,
    depth_uniforms: DepthUniforms,
}

impl InstancedRenderer {
    pub(super) fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let vertex_shader =
            Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER_SOURCE, shader_version);
        let fragment_source = format!(
            "{}\n{}\n{}",
            FOG_GLSL,
            lighting_glsl(),
            FRAGMENT_SHADER_SOURCE
        );
        let fragment_shader =
            Shader::build(gl, ShaderType::Fragment, &fragment_source, shader_version);
        let forward = ShaderProgram::link(gl, &vertex_shader, &fragment_shader);
        let forward_uniforms = ForwardUniforms {
            world: forward.get_uniform_location(gl, "world"),
            local: forward.get_uniform_location(gl, "local"),
            normal_matrix: forward.get_uniform_location(gl, "normalMatrix"),
            local_normal_matrix: forward.get_uniform_location(gl, "localNormalMatrix"),
            view: forward.get_uniform_location(gl, "view"),
            projection: forward.get_uniform_location(gl, "projection"),
            base_color: forward.get_uniform_location(gl, "baseColor"),
            material_mode: forward.get_uniform_location(gl, "materialMode"),
            debug_mode: forward.get_uniform_location(gl, "debugMode"),
            lighting: LightingUniforms::get(&forward, gl),
            fog: FogUniforms::get(&forward, gl),
        };

        let depth_vertex = Shader::build(
            gl,
            ShaderType::Vertex,
            DEPTH_VERTEX_SHADER_SOURCE,
            shader_version,
        );
        let depth_fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            DEPTH_FRAGMENT_SHADER_SOURCE,
            shader_version,
        );
        let depth = ShaderProgram::link(gl, &depth_vertex, &depth_fragment);
        let depth_uniforms = DepthUniforms {
            world: depth.get_uniform_location(gl, "world"),
            local: depth.get_uniform_location(gl, "local"),
            view: depth.get_uniform_location(gl, "view"),
            projection: depth.get_uniform_location(gl, "projection"),
        };

        Self {
            meshes: HashMap::new(),
            forward,
            forward_uniforms,
            depth,
            depth_uniforms,
        }
    }

    fn mesh(&mut self, gl: &glow::Context, primitive: InstancedPrimitive) -> &mut MeshBuffers {
        self.meshes.entry(primitive).or_insert_with(|| {
            let (vertices, indices) = match primitive {
                InstancedPrimitive::Cube => cube_mesh_data(),
                InstancedPrimitive::Sphere => sphere_mesh_data(),
                InstancedPrimitive::Cylinder => cylinder_mesh_data(),
                InstancedPrimitive::Quad => quad_mesh_data(),
                InstancedPrimitive::Plane => plane_mesh_data(),
            };
            let vertex_bytes = as_bytes(&vertices);
            let index_bytes = as_bytes(&indices);

            unsafe {
                let vao = gl.create_vertex_array().expect("instanced VAO");
                let vertex_buffer = gl.create_buffer().expect("instanced vertices");
                let index_buffer = gl.create_buffer().expect("instanced indices");
                let instance_buffer = gl.create_buffer().expect("instance data");
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
                let instance_stride = size_of::<InstanceData>() as i32;
                for (location, size, offset) in [
                    (4, 3, offset_of!(InstanceData, position)),
                    (5, 4, offset_of!(InstanceData, rotation)),
                    (6, 3, offset_of!(InstanceData, scale)),
                    (7, 3, offset_of!(InstanceData, tint)),
                ] {
                    gl.enable_vertex_attrib_array(location);
                    gl.vertex_attrib_pointer_f32(
                        location,
                        size,
                        glow::FLOAT,
                        false,
                        instance_stride,
                        offset as i32,
                    );
                    gl.vertex_attrib_divisor(location, 1);
                }

                gl.bind_vertex_array(None);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);

                MeshBuffers {
                    vao,
                    _vertex_buffer: vertex_buffer,
                    index_buffer,
                    instance_buffer,
                    index_count: indices.len() as i32,
                    instance_capacity: 0,
                }
            }
        })
    }

    /// Draw one recognized instanced node — in the depth pass with the
    /// instanced depth program, otherwise with the forward program (unlit or
    /// lit per the template's material, or a normals/tangents diagnostic).
    pub(super) fn draw(
        &mut self,
        ctx: &RenderContext,
        template: &RecognizedTemplate<'_>,
        instances: &[InstanceData],
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let depth_pass = ctx.render_pass == RenderPass::DepthOnly;
        let bytes = as_bytes(instances);
        // Scope the mutable mesh borrow: upload the instance records, then
        // carry only the Copy GL handles into the uniform/draw phase.
        let (index_buffer, index_count) = {
            let mesh = self.mesh(ctx.gl, template.primitive);
            unsafe {
                ctx.gl.bind_vertex_array(Some(mesh.vao));
                ctx.gl
                    .bind_buffer(glow::ARRAY_BUFFER, Some(mesh.instance_buffer));
                if bytes.len() > mesh.instance_capacity {
                    ctx.gl
                        .buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::DYNAMIC_DRAW);
                    mesh.instance_capacity = bytes.len();
                } else {
                    ctx.gl
                        .buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
                }
            }
            crate::gpu_counters::gpu_counters().uploaded(bytes.len());
            (mesh.index_buffer, mesh.index_count)
        };
        unsafe {
            if depth_pass {
                let u = &self.depth_uniforms;
                self.depth.use_program(ctx.gl);
                self.depth.set_uniform_matrix4(ctx.gl, &u.world, world);
                self.depth
                    .set_uniform_matrix4(ctx.gl, &u.local, &template.local);
                self.depth.set_uniform_matrix4(ctx.gl, &u.view, view);
                self.depth
                    .set_uniform_matrix4(ctx.gl, &u.projection, projection);
            } else {
                let (color, lit) = match template.material {
                    MaterialDescription::Lit { color, .. } => (*color, true),
                    MaterialDescription::Color(color)
                    | MaterialDescription::Emissive { color, .. }
                    | MaterialDescription::SpriteTexture { color, .. } => (*color, false),
                    // Recognition never accepts a bare `Texture` material.
                    MaterialDescription::Texture(_) => (cgmath::vec4(1.0, 1.0, 1.0, 1.0), false),
                };
                let u = &self.forward_uniforms;
                let p = &self.forward;
                p.use_program(ctx.gl);
                p.set_uniform_matrix4(ctx.gl, &u.world, world);
                p.set_uniform_matrix4(ctx.gl, &u.local, &template.local);
                p.set_uniform_matrix3(ctx.gl, &u.normal_matrix, &normal_matrix(world));
                p.set_uniform_matrix3(
                    ctx.gl,
                    &u.local_normal_matrix,
                    &normal_matrix(&template.local),
                );
                p.set_uniform_matrix4(ctx.gl, &u.view, view);
                p.set_uniform_matrix4(ctx.gl, &u.projection, projection);
                p.set_uniform_vec4(ctx.gl, &u.base_color, &color);
                p.set_uniform_1i(ctx.gl, &u.material_mode, lit as i32);
                let debug_mode = match ctx.debug_render_mode {
                    DebugRenderMode::Normals => 1,
                    DebugRenderMode::Tangents => 2,
                    DebugRenderMode::Default
                    | DebugRenderMode::Transparent
                    | DebugRenderMode::Physics => 0,
                };
                p.set_uniform_1i(ctx.gl, &u.debug_mode, debug_mode);
                u.lighting.set(p, ctx, view);
                u.fog.set(p, ctx.gl, ctx.fog, &ctx.camera_pos);
            }

            ctx.gl
                .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            ctx.gl.draw_elements_instanced(
                glow::TRIANGLES,
                index_count,
                glow::UNSIGNED_INT,
                0,
                instances.len() as i32,
            );
            ctx.gl.bind_vertex_array(None);
        }
    }
}

fn as_bytes<T>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}
