//! The hardware half of `Scene.instanced`: one `draw_elements_instanced` call
//! per recognized primitive template — or per mesh primitive of a recognized
//! rigid model template — in both the forward pass (unlit or lit, fogged,
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
use crate::model::Model;
use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::{VertexPositionTexture, VertexPositionTextureSkinned};
use crate::texture::RuntimeTexture;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::{DebugRenderMode, RenderContext, RenderPass};

use std::sync::Arc;

use super::instancing::{InstanceData, InstancedPrimitive, RecognizedPrimitive};
use super::MaterialDescription;

// The instance transform is `translate * rotate * scale` applied on top of
// the template's internal `local` matrix, all under the node's accumulated
// `world`. Shared by the forward and depth vertex shaders so the two passes'
// instance placement is provably identical.
const INSTANCE_TRANSFORM_GLSL: &str = r#"
        layout (location = 4) in vec3 instancePosition;
        layout (location = 5) in vec4 instanceRotation;
        layout (location = 6) in vec3 instanceScale;

        uniform mat4 world;
        uniform mat4 local;

        vec3 rotate(vec4 q, vec3 v) {
            return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
        }

        vec3 instanceLocal(vec3 pos) {
            vec4 lp = local * vec4(pos, 1.0);
            return rotate(instanceRotation, lp.xyz * instanceScale) + instancePosition;
        }
"#;

// Normals are covectors: the inverse-transpose of `R * S` is `R * S⁻¹`, hence
// rotate(normal / instanceScale); the template-internal part uses the
// precomputed `localNormalMatrix`, and the enclosing `world` uses
// `normalMatrix`. Tangents are ordinary directions and keep the plain
// matrices. A zero (or denormal) scale axis clamps to 1 for the NORMAL only —
// the CPU path's `normal_matrix` likewise falls back on a singular matrix, so
// a flattened copy shades sanely instead of dividing to NaN.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;
        layout (location = 7) in vec3 instanceTint;

        uniform mat3 normalMatrix;
        uniform mat3 localNormalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldPos;
        out vec3 worldNormal;
        out vec3 worldTangent;
        out vec3 tintColor;

        void main() {
            vec4 wp = world * vec4(instanceLocal(inPos), 1.0);
            texCoord = inTex;
            worldPos = wp.xyz;
            tintColor = instanceTint;
            vec3 ln = localNormalMatrix * inNormal;
            vec3 safeScale = mix(instanceScale, vec3(1.0),
                vec3(lessThan(abs(instanceScale), vec3(1e-8))));
            worldNormal = normalMatrix * rotate(instanceRotation, ln / safeScale);
            vec3 lt = mat3(local) * inTangent.xyz;
            worldTangent = mat3(world) * rotate(instanceRotation, lt * instanceScale);
            gl_Position = projection * view * wp;
        }
"#;

// Concatenated after `FOG_GLSL` + `lighting_glsl()` so the lit mode's light
// loop, shadow sampling, and uniforms match `LitMaterial` exactly.
const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 worldPos;
        in vec3 worldNormal;
        in vec3 worldTangent;
        in vec3 tintColor;

        uniform vec4 baseColor;
        uniform sampler2D texture1;
        uniform int useTexture;   // model albedo on unit 0 (glTF base color)
        uniform int materialMode; // 0 = unlit (color/emissive/model), 1 = lit
        uniform int debugMode;    // 0 = authored, 1 = normals, 2 = tangents

        void main() {
            if (debugMode == 1) {
                fragColor = vec4(normalize(worldNormal) * 0.5 + 0.5, 1.0);
            } else if (debugMode == 2) {
                fragColor = vec4(normalize(worldTangent) * 0.5 + 0.5, 1.0);
            } else {
                vec4 base = baseColor;
                if (useTexture == 1) {
                    base = base * texture(texture1, texCoord);
                }
                vec3 albedo = base.rgb * tintColor;
                vec3 shaded = albedo;
                if (materialMode == 1) {
                    vec3 n = normalize(worldNormal);
                    vec3 diffuseLight;
                    vec3 specularLight;
                    accumulateLights(n, worldPos, diffuseLight, specularLight);
                    shaded = albedo * diffuseLight + specularLight;
                }
                fragColor = vec4(applyFog(shaded, worldPos), base.a);
            }
        }
"#;

// Depth-pass twin: positions only (via the shared instance placement), packing
// depth with the same shared snippet as `DepthMaterial` so instanced copies
// land in the same RGBA8 shadow map.
const DEPTH_VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 view;
        uniform mat4 projection;

        void main() {
            gl_Position = projection * view * world * vec4(instanceLocal(inPos), 1.0);
        }
"#;

const DEPTH_FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

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
    texture1: UniformLocation,
    use_texture: UniformLocation,
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

/// One model mesh primitive's instanced state: a SECOND VAO over the mesh's
/// own vertex/index buffers (locations 0-3; the skinned joint/weight
/// attributes at 4/5 are deliberately not enabled — instance channels live
/// there) plus this primitive's instance buffer.
struct ModelMeshBuffers {
    vao: glow::VertexArray,
    /// The mesh's own VBO the VAO reads vertices from — kept to detect a
    /// stale entry when an Arc address is reused by a different model.
    vertex_buffer: glow::Buffer,
    index_buffer: glow::Buffer,
    instance_buffer: glow::Buffer,
    index_count: i32,
    instance_capacity: usize,
}

/// Persistent GPU state for `Scene.instanced`'s hardware path.
pub(super) struct InstancedRenderer {
    meshes: HashMap<InstancedPrimitive, MeshBuffers>,
    // Keyed by the hydrated model's Arc pointer + mesh index. An asset evict/
    // hot-reload produces a NEW Arc with new GL buffers — usually a new
    // address too, but a dropped Arc's address CAN be reused by a later
    // model (ABA), so each draw validates the entry against the mesh's live
    // VBO/EBO handles and rebuilds on mismatch. Entries for dropped models
    // are otherwise retained (bounded by the models a session actually
    // instanced) like every other persistent cache.
    model_meshes: HashMap<(usize, usize), ModelMeshBuffers>,
    forward: ShaderProgram,
    forward_uniforms: ForwardUniforms,
    depth: ShaderProgram,
    depth_uniforms: DepthUniforms,
}

impl InstancedRenderer {
    pub(super) fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let forward_vertex_source =
            format!("{}\n{}", INSTANCE_TRANSFORM_GLSL, VERTEX_SHADER_SOURCE);
        let vertex_shader = Shader::build(
            gl,
            ShaderType::Vertex,
            &forward_vertex_source,
            shader_version,
        );
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
            texture1: forward.get_uniform_location(gl, "texture1"),
            use_texture: forward.get_uniform_location(gl, "useTexture"),
            material_mode: forward.get_uniform_location(gl, "materialMode"),
            debug_mode: forward.get_uniform_location(gl, "debugMode"),
            lighting: LightingUniforms::get(&forward, gl),
            fog: FogUniforms::get(&forward, gl),
        };

        let depth_vertex_source = format!(
            "{}\n{}",
            INSTANCE_TRANSFORM_GLSL, DEPTH_VERTEX_SHADER_SOURCE
        );
        let depth_vertex =
            Shader::build(gl, ShaderType::Vertex, &depth_vertex_source, shader_version);
        let depth_fragment_source = format!(
            "{}\n{}",
            crate::material::depth_material::PACK_DEPTH_GLSL,
            DEPTH_FRAGMENT_SHADER_SOURCE
        );
        let depth_fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            &depth_fragment_source,
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
            model_meshes: HashMap::new(),
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
            let vertex_bytes = crate::terrain_renderer::slice_bytes(&vertices);
            let index_bytes = crate::terrain_renderer::slice_bytes(&indices);

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

    /// Set the depth program's uniforms for one instanced draw.
    fn set_depth_uniforms(
        &self,
        ctx: &RenderContext,
        world: &Matrix4<f32>,
        local: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let u = &self.depth_uniforms;
        self.depth.use_program(ctx.gl);
        self.depth.set_uniform_matrix4(ctx.gl, &u.world, world);
        self.depth.set_uniform_matrix4(ctx.gl, &u.local, local);
        self.depth.set_uniform_matrix4(ctx.gl, &u.view, view);
        self.depth
            .set_uniform_matrix4(ctx.gl, &u.projection, projection);
    }

    /// Set the forward program's uniforms for one instanced draw.
    #[allow(clippy::too_many_arguments)]
    fn set_forward_uniforms(
        &self,
        ctx: &RenderContext,
        world: &Matrix4<f32>,
        local: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
        color: &cgmath::Vector4<f32>,
        use_texture: bool,
        lit: bool,
    ) {
        let u = &self.forward_uniforms;
        let p = &self.forward;
        p.use_program(ctx.gl);
        p.set_uniform_matrix4(ctx.gl, &u.world, world);
        p.set_uniform_matrix4(ctx.gl, &u.local, local);
        p.set_uniform_matrix3(ctx.gl, &u.normal_matrix, &normal_matrix(world));
        p.set_uniform_matrix3(ctx.gl, &u.local_normal_matrix, &normal_matrix(local));
        p.set_uniform_matrix4(ctx.gl, &u.view, view);
        p.set_uniform_matrix4(ctx.gl, &u.projection, projection);
        p.set_uniform_vec4(ctx.gl, &u.base_color, color);
        p.set_uniform_1i(ctx.gl, &u.texture1, 0);
        p.set_uniform_1i(ctx.gl, &u.use_texture, use_texture as i32);
        p.set_uniform_1i(ctx.gl, &u.material_mode, lit as i32);
        let debug_mode = match ctx.debug_render_mode {
            DebugRenderMode::Normals => 1,
            DebugRenderMode::Tangents => 2,
            DebugRenderMode::Default | DebugRenderMode::Transparent | DebugRenderMode::Physics => 0,
        };
        p.set_uniform_1i(ctx.gl, &u.debug_mode, debug_mode);
        u.lighting.set(p, ctx, view);
        u.fog.set(p, ctx.gl, ctx.fog, &ctx.camera_pos);
    }

    /// Draw one recognized PRIMITIVE instanced node — in the depth pass with
    /// the instanced depth program, otherwise with the forward program (unlit
    /// or lit per the template's material, or a normals/tangents diagnostic).
    pub(super) fn draw_primitive(
        &mut self,
        ctx: &RenderContext,
        template: &RecognizedPrimitive<'_>,
        instances: &[InstanceData],
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let depth_pass = ctx.render_pass == RenderPass::DepthOnly;
        let bytes = crate::terrain_renderer::slice_bytes(instances);
        // Scope the mutable mesh borrow: upload the instance records, then
        // carry only the Copy GL handles into the uniform/draw phase.
        let (index_buffer, index_count) = {
            let mesh = self.mesh(ctx.gl, template.primitive);
            unsafe {
                ctx.gl.bind_vertex_array(Some(mesh.vao));
                ctx.gl
                    .bind_buffer(glow::ARRAY_BUFFER, Some(mesh.instance_buffer));
                // Deliberately monotonic: capacity follows the high-water
                // mark for the process lifetime (like every other persistent
                // renderer cache here) rather than shrinking per frame.
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
        if depth_pass {
            self.set_depth_uniforms(ctx, world, &template.local, projection, view);
        } else {
            let (color, lit) = match template.material {
                MaterialDescription::Lit { color, .. } => (*color, true),
                MaterialDescription::Color(color)
                | MaterialDescription::Emissive { color, .. } => (*color, false),
                // Recognition never accepts these; the arm exists only so
                // the match stays exhaustive if the enum grows.
                MaterialDescription::Texture(_) | MaterialDescription::SpriteTexture { .. } => {
                    (cgmath::vec4(1.0, 1.0, 1.0, 1.0), false)
                }
            };
            self.set_forward_uniforms(
                ctx,
                world,
                &template.local,
                projection,
                view,
                &color,
                false,
                lit,
            );
        }
        unsafe {
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

    /// Draw one recognized rigid-MODEL instanced node: one instanced call per
    /// mesh primitive, each with its glTF base-color texture on unit 0 and
    /// its node transform composed into `local` — matching the unlit textured
    /// shading (and shadow casting) of the model's stamped equivalent.
    ///
    /// The caller has already resolved the model and verified it is rigid
    /// (no skeleton); an unloaded/failed model resolves to zero meshes and
    /// draws nothing here.
    pub(super) fn draw_model(
        &mut self,
        ctx: &RenderContext,
        model: &Arc<Model>,
        local: &Matrix4<f32>,
        instances: &[InstanceData],
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let depth_pass = ctx.render_pass == RenderPass::DepthOnly;
        let bytes = crate::terrain_renderer::slice_bytes(instances);
        let key_base = Arc::as_ptr(model) as usize;
        for (mesh_index, mesh) in model.meshes.iter().enumerate() {
            let handles = mesh.mesh.buffers(ctx.gl);
            let key = (key_base, mesh_index);
            // ABA guard: a dropped model Arc's address can be reused by a
            // DIFFERENT model later. The mesh's live GL handles are the
            // ground truth — a cached entry whose buffers don't match is
            // stale (its VAO reads deleted/foreign buffers); rebuild it.
            if let Some(entry) = self.model_meshes.get(&key) {
                if entry.vertex_buffer != handles.vbo || entry.index_buffer != handles.ebo {
                    let stale = self.model_meshes.remove(&key).expect("checked above");
                    let counters = crate::gpu_counters::gpu_counters();
                    counters.vao_deleted();
                    counters.buffer_deleted();
                    unsafe {
                        ctx.gl.delete_vertex_array(stale.vao);
                        ctx.gl.delete_buffer(stale.instance_buffer);
                    }
                }
            }
            // Scope the mutable cache borrow, carrying out only Copy handles.
            let (index_buffer, index_count) = {
                let entry = self
                    .model_meshes
                    .entry(key)
                    .or_insert_with(|| unsafe {
                        let vao = ctx.gl.create_vertex_array().expect("model instanced VAO");
                        let instance_buffer =
                            ctx.gl.create_buffer().expect("model instance data");
                        let counters = crate::gpu_counters::gpu_counters();
                        counters.vao_created();
                        counters.buffer_created();

                        ctx.gl.bind_vertex_array(Some(vao));
                        ctx.gl.bind_buffer(glow::ARRAY_BUFFER, Some(handles.vbo));
                        let stride = VertexPositionTextureSkinned::get_total_size() as i32;
                        // Locations 0-3 (position/uv/normal/tangent) from the
                        // mesh's own VBO; the skinned joints/weights at 4/5
                        // stay DISABLED — those locations carry the instance
                        // channels instead.
                        for (location, attribute) in VertexPositionTextureSkinned::get_vertex_attributes()
                            .iter()
                            .take(4)
                            .enumerate()
                        {
                            ctx.gl.enable_vertex_attrib_array(location as u32);
                            match attribute.attribute_type {
                                VertexAttributeType::Float => ctx.gl.vertex_attrib_pointer_f32(
                                    location as u32,
                                    attribute.size,
                                    glow::FLOAT,
                                    false,
                                    stride,
                                    attribute.offset as i32,
                                ),
                            }
                        }

                        ctx.gl.bind_buffer(glow::ARRAY_BUFFER, Some(instance_buffer));
                        let instance_stride = size_of::<InstanceData>() as i32;
                        // No tint attribute (location 7): a bare model
                        // template has no material colors for the tint to
                        // multiply, so the STAMP ignores it — the hardware
                        // path must match. The generic attribute value is
                        // pinned to white at draw time instead.
                        for (location, size, offset) in [
                            (4, 3, offset_of!(InstanceData, position)),
                            (5, 4, offset_of!(InstanceData, rotation)),
                            (6, 3, offset_of!(InstanceData, scale)),
                        ] {
                            ctx.gl.enable_vertex_attrib_array(location);
                            ctx.gl.vertex_attrib_pointer_f32(
                                location,
                                size,
                                glow::FLOAT,
                                false,
                                instance_stride,
                                offset as i32,
                            );
                            ctx.gl.vertex_attrib_divisor(location, 1);
                        }

                        ctx.gl.bind_vertex_array(None);
                        ctx.gl.bind_buffer(glow::ARRAY_BUFFER, None);

                        ModelMeshBuffers {
                            vao,
                            vertex_buffer: handles.vbo,
                            index_buffer: handles.ebo,
                            instance_buffer,
                            index_count: handles.index_count,
                            instance_capacity: 0,
                        }
                    });
                unsafe {
                    ctx.gl.bind_vertex_array(Some(entry.vao));
                    ctx.gl
                        .bind_buffer(glow::ARRAY_BUFFER, Some(entry.instance_buffer));
                    if bytes.len() > entry.instance_capacity {
                        ctx.gl
                            .buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::DYNAMIC_DRAW);
                        entry.instance_capacity = bytes.len();
                    } else {
                        ctx.gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
                    }
                }
                crate::gpu_counters::gpu_counters().uploaded(bytes.len());
                (entry.index_buffer, entry.index_count)
            };

            // Location 7's array is disabled in the model VAO — pin the
            // generic tint value to white (identity), matching the stamp's
            // tint-ignoring semantics for bare model templates. Generic
            // attribute values are context state, so set it per draw.
            unsafe {
                ctx.gl.vertex_attrib_3_f32(7, 1.0, 1.0, 1.0);
            }

            // glTF: a static mesh composes its NODE transform under the
            // template-internal transform (skinned models never reach here).
            let mesh_local = local * mesh.transform;
            if depth_pass {
                self.set_depth_uniforms(ctx, world, &mesh_local, projection, view);
            } else {
                mesh.base_color_texture.bind(0, ctx);
                self.set_forward_uniforms(
                    ctx,
                    world,
                    &mesh_local,
                    projection,
                    view,
                    &cgmath::vec4(1.0, 1.0, 1.0, 1.0),
                    true,
                    false,
                );
            }
            unsafe {
                ctx.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
                ctx.gl.draw_elements_instanced(
                    glow::TRIANGLES,
                    index_count,
                    glow::UNSIGNED_INT,
                    0,
                    instances.len() as i32,
                );
            }
        }
        unsafe {
            ctx.gl.bind_vertex_array(None);
        }
    }
}
