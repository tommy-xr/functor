use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, sync::Arc};

use glow::HasContext;

use cgmath::{vec3, Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use fable_library_rust::NativeArray_::Array;

use crate::{
    asset::{
        self,
        pipelines::{ModelPipeline, RawImagePipeline, TexturePipeline},
        AssetCache, AssetHandle, AssetPollState, BuiltAssetPipeline,
    },
    composite::{
        COMPOSITE_FRAGMENT_SHADER_SOURCE, COMPOSITE_VERTEX_SHADER_SOURCE, MAX_COMPOSITE,
    },
    geometry::{self, Geometry},
    material::{
        BasicMaterial, DepthMaterial, Material, NormalDebugMaterial, SkinnedDepthMaterial,
        SkinnedMaterial, SkinnedNormalDebugMaterial, SkinnedTangentDebugMaterial,
        TangentDebugMaterial,
    },
    math::Angle,
    model::{Model, Skeleton},
    render_target::{warn_line, RenderTargetBuffers, RenderTargetDescriptor},
    shader::{Shader, ShaderType},
    shader_program::{ShaderProgram, UniformLocation},
    skybox::{SkyboxDescription, SKYBOX_FRAGMENT_SHADER_SOURCE, SKYBOX_VERTEX_SHADER_SOURCE},
    texture::{RuntimeTexture, Texture2D, TextureData},
    DebugRenderMode, RenderContext, RenderPass,
};

mod material_description;
mod model_description;
mod texture_description;

pub use material_description::*;
pub use model_description::*;
pub use texture_description::*;

pub struct SceneContext {
    model_pipeline: Arc<BuiltAssetPipeline<Model>>,
    texture_pipeline: Arc<BuiltAssetPipeline<Texture2D>>,
    cube: RefCell<Box<dyn Geometry>>,
    cylinder: RefCell<Box<dyn Geometry>>,
    sphere: RefCell<Box<dyn Geometry>>,
    quad: RefCell<Box<dyn Geometry>>,
    plane: RefCell<Box<dyn Geometry>>,
    // Heightmaps are parameterized, so they're cached by a content hash (rows,
    // cols, heights) — static terrain builds its GL mesh once and reuses it.
    heightmaps: RefCell<HashMap<u64, Box<dyn Geometry>>>,
    // Render targets persist across frames/hot reloads, keyed by the target's
    // string id (the cross-frame identity). Buffers for ids a game stops
    // declaring are kept until exit — TODO: evict.
    render_targets: RefCell<HashMap<String, RenderTargetBuffers>>,
    render_target_warned: RefCell<HashSet<String>>,
    fallback_texture: RefCell<Option<glow::Texture>>,
    // Cubemap skyboxes, keyed by the joined six face paths. Like render
    // targets they persist across frames/hot reloads and are never evicted
    // (TODO). Faces decode through `raw_image_pipeline` (no GL hydration);
    // the cubemap uploads once when all six are ready.
    raw_image_pipeline: Arc<BuiltAssetPipeline<TextureData>>,
    skyboxes: RefCell<HashMap<String, SkyboxEntry>>,
    skybox_program: RefCell<Option<(ShaderProgram, SkyboxUniforms)>>,
    // The screen-space compositor's fullscreen-average program, built lazily on
    // first use and cached like the skybox program (docs/time-travel.md T5).
    composite_program: RefCell<Option<(ShaderProgram, CompositeUniforms)>>,
}

enum SkyboxEntry {
    /// Six pending face loads, in `SkyboxDescription::faces` order.
    Loading(Vec<Arc<AssetHandle<TextureData>>>),
    Ready(glow::Texture),
    /// A face failed to load or validate; warned once, never retried.
    Failed,
}

struct SkyboxUniforms {
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    skybox_loc: UniformLocation,
}

struct CompositeUniforms {
    /// `sampler2D uTex[MAX_COMPOSITE]` — one texture unit per input.
    tex_loc: UniformLocation,
    /// `float uWeight[MAX_COMPOSITE]` — the per-input blend weight.
    weight_loc: UniformLocation,
}

impl SceneContext {
    pub fn new() -> SceneContext {
        SceneContext {
            cube: RefCell::new(geometry::Cube::create()),
            sphere: RefCell::new(geometry::Sphere::create()),
            cylinder: RefCell::new(geometry::Cylinder::create()),
            quad: RefCell::new(geometry::Quad::create()),
            plane: RefCell::new(geometry::Plane::create()),
            heightmaps: RefCell::new(HashMap::new()),
            texture_pipeline: asset::build_pipeline(Box::new(TexturePipeline)),
            model_pipeline: asset::build_pipeline(Box::new(ModelPipeline)),
            render_targets: RefCell::new(HashMap::new()),
            render_target_warned: RefCell::new(HashSet::new()),
            fallback_texture: RefCell::new(None),
            raw_image_pipeline: asset::build_pipeline(Box::new(RawImagePipeline)),
            skyboxes: RefCell::new(HashMap::new()),
            skybox_program: RefCell::new(None),
            composite_program: RefCell::new(None),
        }
    }

    /// Create (or recreate, if the declared size changed) the buffers for a
    /// render target. Called for every declared target before any pass runs;
    /// `clear` is the target pass's background (its fog color when fogged).
    pub fn ensure_render_target(
        &self,
        gl: &glow::Context,
        desc: &RenderTargetDescriptor,
        clear: [f32; 3],
    ) {
        let mut targets = self.render_targets.borrow_mut();
        let stale = targets
            .get(&desc.id)
            .is_some_and(|b| (b.width, b.height) != (desc.width.max(1), desc.height.max(1)));
        if stale {
            targets.remove(&desc.id).unwrap().delete(gl);
        }
        targets
            .entry(desc.id.clone())
            .or_insert_with(|| RenderTargetBuffers::new(gl, desc.width, desc.height, clear));
    }

    /// The framebuffer + size a target pass renders into. Handles are `Copy` —
    /// the borrow is released before rendering starts.
    pub fn render_target_write(&self, id: &str) -> Option<(glow::Framebuffer, u32, u32)> {
        self.render_targets
            .borrow()
            .get(id)
            .map(|b| (b.write_fbo(), b.width, b.height))
    }

    /// Publish a finished target pass: readers now sample the new image.
    pub fn finish_render_target_write(&self, id: &str) {
        if let Some(buffers) = self.render_targets.borrow_mut().get_mut(id) {
            buffers.swap();
        }
    }

    /// The texture materials sample for a target id, if it exists.
    pub fn render_target_read_texture(&self, id: &str) -> Option<glow::Texture> {
        self.render_targets.borrow().get(id).map(|b| b.read_texture())
    }

    /// A 1x1 magenta texture bound when a material references a render target
    /// no frame declares — loud on screen, and `warn_once` says why.
    pub fn fallback_texture(&self, gl: &glow::Context) -> glow::Texture {
        let mut fallback = self.fallback_texture.borrow_mut();
        *fallback.get_or_insert_with(|| unsafe {
            let texture = gl.create_texture().expect("fallback texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&[255, 0, 255, 255])),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        })
    }

    /// Log `message` the first time `key` is seen (per-key, once per run) —
    /// render-loop warnings must not spam every frame.
    pub fn warn_once(&self, key: &str, message: &str) {
        if self.render_target_warned.borrow_mut().insert(key.to_string()) {
            warn_line(message);
        }
    }

    /// The cubemap for `desc`, once all six faces have loaded. `None` while
    /// faces are still loading (skip the draw — the clear color shows) or
    /// after a failure (warned once, never retried).
    fn skybox_texture(
        &self,
        gl: &glow::Context,
        asset_cache: &Arc<AssetCache>,
        desc: &SkyboxDescription,
    ) -> Option<glow::Texture> {
        let key = desc.faces().join("\n");
        let mut skyboxes = self.skyboxes.borrow_mut();
        let entry = skyboxes.entry(key.clone()).or_insert_with(|| {
            SkyboxEntry::Loading(
                desc.faces()
                    .iter()
                    .map(|path| {
                        asset_cache
                            .load_asset_with_pipeline(self.raw_image_pipeline.clone(), path)
                    })
                    .collect(),
            )
        });

        match entry {
            SkyboxEntry::Ready(texture) => Some(*texture),
            SkyboxEntry::Failed => None,
            SkyboxEntry::Loading(handles) => {
                // Poll EVERY handle each call: futures only advance when
                // polled (noop waker), and a wasm fetch doesn't even start
                // until its first poll — an early return on the first
                // pending face would serialize the six downloads.
                let mut faces: Vec<Arc<TextureData>> = Vec::with_capacity(6);
                let mut pending = false;
                let mut failed: Option<&str> = None;
                for (handle, path) in handles.iter().zip(desc.faces()) {
                    match handle.poll_state() {
                        AssetPollState::Loaded(data) => faces.push(data),
                        AssetPollState::Loading => pending = true,
                        AssetPollState::Failed => failed = Some(path),
                    }
                }
                if let Some(path) = failed {
                    let message = format!(
                        "[skybox] face \"{path}\" failed to load — skybox \
disabled for this set"
                    );
                    *entry = SkyboxEntry::Failed;
                    drop(skyboxes);
                    self.warn_once(&key, &message);
                    return None;
                }
                if pending {
                    return None;
                }
                // All six decoded: validate (square, uniform, non-empty —
                // a 0x0 face is the raw pipeline's undecodable sentinel).
                let (w, h) = (faces[0].width, faces[0].height);
                let valid =
                    w > 0 && w == h && faces.iter().all(|f| f.width == w && f.height == h);
                if !valid {
                    *entry = SkyboxEntry::Failed;
                    drop(skyboxes);
                    self.warn_once(
                        &key,
                        "[skybox] faces must all be square and the same size — \
skybox disabled for this set",
                    );
                    return None;
                }
                let texture = unsafe {
                    let texture = gl.create_texture().expect("skybox cubemap");
                    gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
                    for (i, face) in faces.iter().enumerate() {
                        gl.tex_image_2d(
                            glow::TEXTURE_CUBE_MAP_POSITIVE_X + i as u32,
                            0,
                            glow::RGBA8 as i32,
                            w as i32,
                            h as i32,
                            0,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(&face.bytes)),
                        );
                    }
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_MIN_FILTER,
                        glow::LINEAR as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_MAG_FILTER,
                        glow::LINEAR as i32,
                    );
                    // Single declared mip level: unambiguously complete (the
                    // ShadowMap/render-target macOS recipe).
                    gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, glow::TEXTURE_BASE_LEVEL, 0);
                    gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, glow::TEXTURE_MAX_LEVEL, 0);
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_WRAP_S,
                        glow::CLAMP_TO_EDGE as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_WRAP_T,
                        glow::CLAMP_TO_EDGE as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_WRAP_R,
                        glow::CLAMP_TO_EDGE as i32,
                    );
                    gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
                    texture
                };
                *entry = SkyboxEntry::Ready(texture);
                Some(texture)
            }
        }
    }

    /// Draw `desc`'s skybox: right after the pass's clear, before
    /// `Scene3D::render`. The unit cube is drawn from the inside (this
    /// engine never enables face culling), glued to the camera by a
    /// translation-stripped view, at NDC depth 1.0 (`gl_Position.xyww`) —
    /// LEQUAL lets it pass against the cleared depth, and `depth_mask(false)`
    /// keeps it from occluding anything. Skipped (clear color shows) while
    /// faces load or after a face failure.
    pub fn draw_skybox(
        &self,
        render_context: &RenderContext,
        desc: &SkyboxDescription,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
    ) {
        let gl = render_context.gl;
        let Some(texture) = self.skybox_texture(gl, &render_context.asset_cache, desc) else {
            return;
        };

        {
            let mut program = self.skybox_program.borrow_mut();
            if program.is_none() {
                let vertex = Shader::build(
                    gl,
                    ShaderType::Vertex,
                    SKYBOX_VERTEX_SHADER_SOURCE,
                    render_context.shader_version,
                );
                let fragment = Shader::build(
                    gl,
                    ShaderType::Fragment,
                    SKYBOX_FRAGMENT_SHADER_SOURCE,
                    render_context.shader_version,
                );
                let shader = ShaderProgram::link(gl, &vertex, &fragment);
                let uniforms = SkyboxUniforms {
                    view_loc: shader.get_uniform_location(gl, "view"),
                    projection_loc: shader.get_uniform_location(gl, "projection"),
                    skybox_loc: shader.get_uniform_location(gl, "skybox"),
                };
                *program = Some((shader, uniforms));
            }
        }

        // Strip the view translation so the box is centered on the camera.
        let mut view = *view_matrix;
        view.w = cgmath::vec4(0.0, 0.0, 0.0, 1.0);

        let program = self.skybox_program.borrow();
        let (shader, uniforms) = program.as_ref().expect("skybox program just initialized");
        unsafe {
            shader.use_program(gl);
            shader.set_uniform_matrix4(gl, &uniforms.view_loc, &view);
            shader.set_uniform_matrix4(gl, &uniforms.projection_loc, projection_matrix);
            shader.set_uniform_1i(gl, &uniforms.skybox_loc, 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));

            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
        }
        self.cube.borrow_mut().draw(gl);
        unsafe {
            gl.depth_mask(true);
            gl.depth_func(glow::LESS);
            gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        }
    }

    /// Composite `textures` onto the currently-bound framebuffer as a weighted
    /// average — the screen-space compositor pass (docs/time-travel.md T5). Each
    /// texture is a full offscreen render of one `Frame`; `weights[i]` scales
    /// input `i` (caller normalizes to sum 1 for an average). Draws a fullscreen
    /// quad with depth-testing off, over whatever the caller cleared, so it can
    /// land in the default framebuffer before the UI overlay (and thus in
    /// `--capture-frame` PNGs). Up to `MAX_COMPOSITE` inputs; extras are dropped
    /// by the caller. The averaging is exact and in-shader — no GL blend state.
    pub fn draw_composite(
        &self,
        gl: &glow::Context,
        shader_version: &str,
        textures: &[glow::Texture],
        weights: &[f32],
    ) {
        if textures.is_empty() {
            return;
        }

        {
            let mut program = self.composite_program.borrow_mut();
            if program.is_none() {
                let vertex = Shader::build(
                    gl,
                    ShaderType::Vertex,
                    COMPOSITE_VERTEX_SHADER_SOURCE,
                    shader_version,
                );
                let fragment = Shader::build(
                    gl,
                    ShaderType::Fragment,
                    COMPOSITE_FRAGMENT_SHADER_SOURCE,
                    shader_version,
                );
                let shader = ShaderProgram::link(gl, &vertex, &fragment);
                let uniforms = CompositeUniforms {
                    tex_loc: shader.get_uniform_location(gl, "uTex"),
                    weight_loc: shader.get_uniform_location(gl, "uWeight"),
                };
                *program = Some((shader, uniforms));
            }
        }

        // Build the full fixed-size uniform arrays: real inputs for the first
        // `k`, zero-weight padding for the rest (the shader unrolls to
        // MAX_COMPOSITE). Every sampler unit is bound to a valid texture — the
        // padding units reuse input 0, harmless since their weight is 0.
        let k = textures.len().min(weights.len()).min(MAX_COMPOSITE);
        let units: [i32; MAX_COMPOSITE] = std::array::from_fn(|i| i as i32);
        let mut weight_array = [0.0f32; MAX_COMPOSITE];
        weight_array[..k].copy_from_slice(&weights[..k]);

        let program = self.composite_program.borrow();
        let (shader, uniforms) = program
            .as_ref()
            .expect("composite program just initialized");
        unsafe {
            shader.use_program(gl);
            shader.set_uniform_1iv(gl, &uniforms.tex_loc, &units);
            shader.set_uniform_1fv(gl, &uniforms.weight_loc, &weight_array);
            for (i, unit) in units.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + *unit as u32);
                let texture = if i < k { textures[i] } else { textures[0] };
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            }
            // Fullscreen pass: no depth read/write wanted.
            gl.disable(glow::DEPTH_TEST);
        }
        self.quad.borrow_mut().draw(gl);
        unsafe {
            gl.enable(glow::DEPTH_TEST);
            // Leave the units clean so later passes don't see stale bindings.
            for unit in units.iter() {
                gl.active_texture(glow::TEXTURE0 + *unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.active_texture(glow::TEXTURE0);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Shape {
    Cube,
    Sphere,
    Cylinder,
    Quad,
    Plane,
    /// A subdivided XZ grid displaced by per-vertex heights (row-major,
    /// length `rows * cols`).
    Heightmap {
        rows: u32,
        cols: u32,
        heights: Vec<f32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneObject {
    Geometry(Shape),
    Model(ModelDescription),
    Material(MaterialDescription, Vec<Scene3D>),
    Group(Vec<Scene3D>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene3D {
    pub obj: SceneObject,
    #[serde(
        serialize_with = "serialize_matrix",
        deserialize_with = "deserialize_matrix"
    )]
    pub xform: Matrix4<f32>,
}

impl Scene3D {
    pub fn cube() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Cube),
            xform: Matrix4::identity(),
        }
    }

    pub fn sphere() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Sphere),
            xform: Matrix4::identity(),
        }
    }

    pub fn quad() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Quad),
            xform: Matrix4::identity(),
        }
    }

    pub fn plane() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Plane),
            xform: Matrix4::identity(),
        }
    }

    pub fn cylinder() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Cylinder),
            xform: Matrix4::identity(),
        }
    }

    pub fn heightmap(rows: i32, cols: i32, heights: Array<f32>) -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Heightmap {
                rows: rows.max(0) as u32,
                cols: cols.max(0) as u32,
                heights: heights.to_vec(),
            }),
            xform: Matrix4::identity(),
        }
    }

    pub fn material(material: MaterialDescription, items: Array<Scene3D>) -> Self {
        Scene3D {
            obj: SceneObject::Material(material, items.to_vec()),
            xform: Matrix4::identity(),
        }
    }

    pub fn model(model: ModelDescription) -> Self {
        Scene3D {
            obj: SceneObject::Model(model),
            xform: Matrix4::identity(),
        }
    }

    pub fn group(items: Array<Scene3D>) -> Self {
        Scene3D {
            obj: SceneObject::Group(items.to_vec()),
            xform: Matrix4::identity(),
        }
    }

    pub fn transform(self, xform: Matrix4<f32>) -> Self {
        Scene3D {
            xform: self.xform * xform,
            ..self
        }
    }

    pub fn scale_x(self, x: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(x, 1.0, 1.0))
    }
    pub fn scale_y(self, y: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(1.0, y, 1.0))
    }
    pub fn scale_z(self, z: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(1.0, 1.0, z))
    }

    pub fn translate_x(self, x: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(x, 0.0, 0.0)))
    }

    pub fn translate_y(self, y: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(0.0, y, 0.0)))
    }

    pub fn translate_z(self, z: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(0.0, 0.0, z)))
    }

    pub fn rotate_x(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_x(ang))
    }
    pub fn rotate_y(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_y(ang))
    }
    pub fn rotate_z(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_z(ang))
    }

    pub fn render(
        &self,
        render_context: &RenderContext,
        scene_context: &SceneContext,
        world_matrix: &Matrix4<f32>,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        current_material: &Box<dyn Material>,
    ) {
        let skinning_data = vec![];

        // A pass/mode can replace every node's own material with one shared
        // shader: the depth pass (filling a shadow map) uses DepthMaterial for
        // all geometry; the normals debug mode uses NormalDebugMaterial. The
        // depth override also keeps the lit shader from sampling the shadow map
        // while it is being written.
        let depth_pass = render_context.render_pass == RenderPass::DepthOnly;
        let override_material: Option<Box<dyn Material>> = if depth_pass {
            let mut m = DepthMaterial::create();
            m.initialize(render_context);
            Some(m)
        } else {
            match render_context.debug_render_mode {
                // Physics mode shades normally — its wireframes are a separate
                // overlay pass (`render_debug_lines`), not a material override.
                DebugRenderMode::Default | DebugRenderMode::Physics => None,
                DebugRenderMode::Normals => {
                    let mut m = NormalDebugMaterial::create();
                    m.initialize(render_context);
                    Some(m)
                }
                DebugRenderMode::Tangents => {
                    let mut m = TangentDebugMaterial::create();
                    m.initialize(render_context);
                    Some(m)
                }
            }
        };
        let geometry_material = override_material.as_ref().unwrap_or(current_material);

        match &self.obj {
            SceneObject::Model(model_description) => {
                match &model_description.handle {
                    ModelHandle::File(str) => {
                        let model: Arc<AssetHandle<Model>> = render_context
                            .asset_cache
                            .load_asset_with_pipeline(scene_context.model_pipeline.clone(), str);

                        let hydrated_model = model.get();

                        let matrix = world_matrix * self.xform;

                        // Skinned models pay for the joint-matrix uniform array;
                        // static models (no skeleton) render with the basic
                        // material instead. In a debug render mode, both swap to
                        // the matching diagnostic material (the skinned variant
                        // deforms the normal by the joint blend).
                        let is_skinned = hydrated_model.skeleton.get_joint_count() > 0;
                        let debug_override = !matches!(
                            render_context.debug_render_mode,
                            DebugRenderMode::Default | DebugRenderMode::Physics
                        );
                        // In the depth pass, draw the model with a depth material
                        // that still skins (so animated models cast a correctly
                        // deforming shadow), else the lit material or the matching
                        // diagnostic material (skinned variants deform the
                        // normal/tangent by the joint blend).
                        let mut model_material: Box<dyn Material> = match (depth_pass, is_skinned) {
                            (true, true) => SkinnedDepthMaterial::create(),
                            (true, false) => DepthMaterial::create(),
                            (false, _) => match render_context.debug_render_mode {
                                DebugRenderMode::Default | DebugRenderMode::Physics
                                    if is_skinned =>
                                {
                                    SkinnedMaterial::create()
                                }
                                DebugRenderMode::Default | DebugRenderMode::Physics => {
                                    BasicMaterial::create()
                                }
                                DebugRenderMode::Normals if is_skinned => {
                                    SkinnedNormalDebugMaterial::create()
                                }
                                DebugRenderMode::Normals => NormalDebugMaterial::create(),
                                DebugRenderMode::Tangents if is_skinned => {
                                    SkinnedTangentDebugMaterial::create()
                                }
                                DebugRenderMode::Tangents => TangentDebugMaterial::create(),
                            },
                        };
                        model_material.initialize(&render_context);

                        let animation_index = 0;

                        for mesh in hydrated_model.meshes.iter() {
                            // Go through selectors, and adjust
                            // let override_material_description = Some(MaterialDescription::Texture(
                            //     TextureDescription::File("vr_glove_color.jpg".to_string()),
                            // ));

                            let mut override_material_description: Option<&MaterialDescription> =
                                None;

                            let mut matrix = matrix * mesh.transform;

                            for (_selector, override_) in &model_description.overrides {
                                match override_ {
                                    MeshOverride::Material(material) => {
                                        override_material_description = Some(material);
                                    }
                                    MeshOverride::Transform(xform) => {
                                        matrix = matrix * xform;
                                    }
                                }
                            }

                            // A debug render mode or the depth pass overrides
                            // everything — ignore per-mesh material selectors so
                            // the whole model is drawn with the override material.
                            if debug_override || depth_pass {
                                override_material_description = None;
                            }

                            if let Some(material_description) = override_material_description {
                                let material =
                                    material_description.get(render_context, scene_context);

                                material.draw_opaque(
                                    &render_context,
                                    projection_matrix,
                                    view_matrix,
                                    &matrix,
                                    &[],
                                );
                            } else {
                                let joints = if is_skinned {
                                    match hydrated_model.animations.get(animation_index) {
                                        Some(animation) => {
                                            let time = render_context.frame_time.tts
                                                % animation.duration;
                                            let animated_skeleton = Skeleton::animate(
                                                &hydrated_model.skeleton,
                                                animation,
                                                time,
                                            );
                                            animated_skeleton.get_skinning_transforms()
                                        }
                                        None => vec![Matrix4::identity(); 50],
                                    }
                                } else {
                                    vec![]
                                };

                                // Bind textures
                                mesh.base_color_texture.bind(0, &render_context);
                                model_material.draw_opaque(
                                    &render_context,
                                    projection_matrix,
                                    view_matrix,
                                    &matrix,
                                    &joints,
                                );
                            };

                            // TODO: Bring back drawing
                            mesh.mesh.draw(&render_context.gl)
                        }
                    }
                }
            }

            SceneObject::Material(material_description, items) => {
                let material = material_description.get(render_context, scene_context);
                for item in items.into_iter() {
                    item.render(
                        &render_context,
                        &scene_context,
                        &world_matrix,
                        &projection_matrix,
                        &view_matrix,
                        &material,
                    )
                }
            }

            SceneObject::Group(items) => {
                let new_world_matrix = world_matrix * self.xform;
                for item in items.into_iter() {
                    item.render(
                        &render_context,
                        &scene_context,
                        &new_world_matrix,
                        &projection_matrix,
                        &view_matrix,
                        current_material,
                    )
                }
            }
            SceneObject::Geometry(Shape::Cube) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.cube.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Cylinder) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                scene_context.cylinder.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Sphere) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                scene_context.sphere.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Quad) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.quad.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Plane) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.plane.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Heightmap { rows, cols, heights }) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                // Build the grid mesh once per unique (rows, cols, heights) and
                // cache it; static terrain reuses the same GL buffers each frame.
                let key = heightmap_key(*rows, *cols, heights);
                scene_context
                    .heightmaps
                    .borrow_mut()
                    .entry(key)
                    .or_insert_with(|| {
                        geometry::Heightmap::create(*rows as usize, *cols as usize, heights)
                    });
                scene_context.heightmaps.borrow()[&key].draw(&render_context.gl);
            }
        }
    }
}

/// Content hash of a heightmap's parameters, used to cache its built GL mesh.
fn heightmap_key(rows: u32, cols: u32, heights: &[f32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    cols.hash(&mut hasher);
    for h in heights {
        h.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn serialize_matrix<S>(matrix: &Matrix4<f32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let array: [[f32; 4]; 4] = [
        [matrix.x[0], matrix.x[1], matrix.x[2], matrix.x[3]],
        [matrix.y[0], matrix.y[1], matrix.y[2], matrix.y[3]],
        [matrix.z[0], matrix.z[1], matrix.z[2], matrix.z[3]],
        [matrix.w[0], matrix.w[1], matrix.w[2], matrix.w[3]],
    ];
    array.serialize(serializer)
}

fn deserialize_matrix<'de, D>(deserializer: D) -> Result<Matrix4<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let array = <[[f32; 4]; 4]>::deserialize(deserializer)?;
    Ok(Matrix4::new(
        array[0][0],
        array[0][1],
        array[0][2],
        array[0][3],
        array[1][0],
        array[1][1],
        array[1][2],
        array[1][3],
        array[2][0],
        array[2][1],
        array[2][2],
        array[2][3],
        array[3][0],
        array[3][1],
        array[3][2],
        array[3][3],
    ))
}
