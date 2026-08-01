use std::collections::{BTreeSet, HashMap, HashSet};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use glow::HasContext;

use cgmath::{vec3, Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use crate::{
    asset::{
        self,
        pipelines::{
            HeightmapData, HeightmapPipeline, ModelPipeline, RawImagePipeline, TexturePipeline,
        },
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
    terrain_renderer::TerrainRenderer,
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
    /// Terrain detail maps decode through their own pipeline so their reduced
    /// anisotropy and mean-color scan stay scoped to them.
    terrain_detail_pipeline: Arc<BuiltAssetPipeline<Texture2D>>,
    cube: RefCell<Box<dyn Geometry>>,
    cylinder: RefCell<Box<dyn Geometry>>,
    sphere: RefCell<Box<dyn Geometry>>,
    quad: RefCell<Box<dyn Geometry>>,
    plane: RefCell<Box<dyn Geometry>>,
    // One persistent mesh per grid size. Animated terrain (heights change every
    // frame) re-uploads its vertex buffer in place instead of rebuilding a fresh
    // GL mesh each frame; static terrain uploads exactly once. Keyed by
    // (rows, cols) — the stable identity of the mesh across frames.
    heightmaps: RefCell<HashMap<(u32, u32), geometry::HeightmapMesh>>,
    // One persistent 2D fill mesh per POINT COUNT, re-uploaded in place when the
    // points differ from what is loaded. Every `Sprite.circle` lowers to the same
    // unit ring plus a scale, so all circles share one mesh and upload nothing;
    // author-supplied polygons of the same vertex count share a mesh and each
    // re-uploads before its own draw.
    polygons: RefCell<HashMap<usize, geometry::PolygonMesh>>,
    heightmap_pipeline: Arc<BuiltAssetPipeline<HeightmapData>>,
    terrain_decode_residency: RefCell<TerrainDecodeResidency>,
    terrain_requests: RefCell<BTreeSet<crate::terrain::TerrainSource>>,
    terrain_renderer: RefCell<TerrainRenderer>,
    terrain_frame_serial: Cell<u64>,
    // Render targets persist across frames/hot reloads, keyed by the target's
    // string id (the cross-frame identity). Buffers for ids a game stops
    // declaring are kept until exit — TODO: evict.
    render_targets: RefCell<HashMap<String, RenderTargetBuffers>>,
    render_target_warned: RefCell<HashSet<String>>,
    fallback_texture: RefCell<Option<glow::Texture>>,
    // Compiled-in textures, hydrated on first bind and kept for the process
    // lifetime like the fallback texture: they have no locator to evict by and
    // there is at most one per variant.
    builtin_textures: RefCell<HashMap<BuiltinTexture, glow::Texture>>,
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
    // In-flight `Effect.preload` loads (B.5), polled each frame by
    // `drive_preloads` until they settle — asset futures advance only when
    // polled, and nothing else polls an asset `draw` isn't referencing yet.
    preloads: RefCell<Vec<PreloadEntry>>,
}

/// One in-flight `Effect.preload` target: the handle being driven plus every
/// `preloadThen` completion token waiting on its settlement. Deduped by
/// (kind, locator): re-preloading an in-flight asset merges into the existing
/// entry instead of growing the list — a game spamming preload of a stalled
/// URL each frame must not accumulate entries (or per-frame polls).
struct PreloadEntry {
    kind: crate::asset::preload::PreloadKind,
    locator: String,
    handle: PreloadHandle,
    tokens: Vec<u64>,
}

enum PreloadHandle {
    Model(Arc<AssetHandle<Model>>),
    Texture(Arc<AssetHandle<Texture2D>>),
}

/// First texture unit for terrain detail maps; must match the terrain
/// renderer's `DETAIL_TEXTURE_UNIT0` (unit 0 is the height texture).
const TERRAIN_DETAIL_UNIT0: u32 = 1;

/// Decoded terrain sources receive one unused shell frame of grace before
/// eviction. This retains ordinary frame-to-frame reuse without keeping every
/// level/hot-reload heightmap alive for the process lifetime.
#[derive(Default)]
struct TerrainDecodeResidency {
    epoch: u64,
    last_used: HashMap<String, u64>,
}

impl TerrainDecodeResidency {
    fn begin_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn mark<'a>(&mut self, locators: impl IntoIterator<Item = &'a str>) {
        for locator in locators {
            if let Some(last_used) = self.last_used.get_mut(locator) {
                *last_used = self.epoch;
            } else {
                self.last_used.insert(locator.to_string(), self.epoch);
            }
        }
    }

    fn mark_source(
        &mut self,
        primary: &str,
        while_pending: &[String],
        primary_is_loading: bool,
        mut is_unsettled: impl FnMut(&str) -> bool,
    ) {
        self.mark(std::iter::once(primary));
        self.mark(
            while_pending
                .iter()
                .filter(|locator| primary_is_loading || is_unsettled(locator))
                .map(String::as_str),
        );
    }

    fn mark_unsettled(
        &mut self,
        locators: &[String],
        mut is_unsettled: impl FnMut(&str) -> bool,
    ) {
        self.mark(
            locators
                .iter()
                .filter(|locator| is_unsettled(locator))
                .map(String::as_str),
        );
    }

    fn evict_stale(&mut self, mut evict: impl FnMut(&str)) {
        let epoch = self.epoch;
        self.last_used.retain(|locator, last_used| {
            let recent = epoch.wrapping_sub(*last_used) <= 1;
            if !recent {
                evict(locator);
            }
            recent
        });
    }
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
    /// Drop every cached decode of `path` so the next draw reloads it from
    /// disk — asset hot-reload (pair with `AssetCache::evict` for the bytes).
    /// A skybox using the path as a face rebuilds too (its cache key is the
    /// six face paths joined with '\n'). GPU objects hydrated from the old
    /// decode are not freed (renderables have no Drop yet) — a dev-loop leak
    /// bounded by save count, the same class as the render-target TODO above.
    pub fn evict_asset(&self, path: &str) {
        self.model_pipeline.evict(path);
        self.texture_pipeline.evict(path);
        self.terrain_detail_pipeline.evict(path);
        self.raw_image_pipeline.evict(path);
        self.heightmap_pipeline.evict(path);
        self.skyboxes
            .borrow_mut()
            .retain(|faces, _| !faces.split('\n').any(|face| face == path));
    }

    pub fn new() -> SceneContext {
        SceneContext {
            cube: RefCell::new(geometry::Cube::create()),
            sphere: RefCell::new(geometry::Sphere::create()),
            cylinder: RefCell::new(geometry::Cylinder::create()),
            quad: RefCell::new(geometry::Quad::create()),
            plane: RefCell::new(geometry::Plane::create()),
            heightmaps: RefCell::new(HashMap::new()),
            polygons: RefCell::new(HashMap::new()),
            heightmap_pipeline: asset::build_pipeline(Box::new(HeightmapPipeline)),
            terrain_decode_residency: RefCell::new(TerrainDecodeResidency::default()),
            terrain_requests: RefCell::new(BTreeSet::new()),
            terrain_renderer: RefCell::new(TerrainRenderer::default()),
            terrain_frame_serial: Cell::new(0),
            texture_pipeline: asset::build_pipeline(Box::new(TexturePipeline)),
            terrain_detail_pipeline: asset::build_pipeline(Box::new(
                crate::asset::pipelines::TerrainDetailPipeline,
            )),
            model_pipeline: asset::build_pipeline(Box::new(ModelPipeline)),
            render_targets: RefCell::new(HashMap::new()),
            render_target_warned: RefCell::new(HashSet::new()),
            fallback_texture: RefCell::new(None),
            builtin_textures: RefCell::new(HashMap::new()),
            raw_image_pipeline: asset::build_pipeline(Box::new(RawImagePipeline)),
            skyboxes: RefCell::new(HashMap::new()),
            skybox_program: RefCell::new(None),
            composite_program: RefCell::new(None),
            preloads: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn begin_terrain_frame(&self, gl: &glow::Context, external_frame: Option<u64>) {
        let frame = external_frame.unwrap_or_else(|| {
            let next = self.terrain_frame_serial.get().wrapping_add(1);
            self.terrain_frame_serial.set(next);
            next
        });
        self.terrain_renderer.borrow_mut().begin_frame(gl, frame);
    }

    /// The shell's per-frame preload step (B.5): turn this frame's
    /// `Effect.preload` commands (from `GameProducer::preload_drain_commands`)
    /// into cache loads, and POLL every in-flight preload until it settles —
    /// futures advance only when polled, and an unpolled preload would
    /// strand in `Sub.assets`' total (the `resolve_while_pending` liveness
    /// rule). Returns the `preloadThen` tokens that settled this frame
    /// (loaded or failed), for the shell to report via
    /// `GameProducer::preload_push_settled`. Once `draw` starts referencing
    /// a warmed asset it hits the same cached handle.
    pub fn drive_preloads(
        &self,
        asset_cache: &Arc<AssetCache>,
        commands: Vec<crate::asset::preload::PreloadCommand>,
    ) -> Vec<u64> {
        self.terrain_decode_residency.borrow_mut().begin_epoch();
        self.capture_terrain_requests();
        self.drive_terrain_requests(asset_cache);
        self.terrain_decode_residency
            .borrow_mut()
            .evict_stale(|locator| self.heightmap_pipeline.evict(locator));
        use crate::asset::preload::PreloadKind;
        let mut preloads = self.preloads.borrow_mut();
        for cmd in commands {
            if let Some(entry) = preloads
                .iter_mut()
                .find(|e| e.kind == cmd.kind && e.locator == cmd.locator)
            {
                // Already in flight: merge (the cache would hand back the
                // same handle anyway); just accumulate the completion token.
                if let Some(token) = cmd.token.filter(|token| !entry.tokens.contains(token)) {
                    entry.tokens.push(token);
                    if entry.tokens.len()
                        > crate::asset::preload::TOKENS_PER_TARGET_CAP
                    {
                        entry.tokens.remove(0);
                    }
                }
                continue;
            }
            let handle = match cmd.kind {
                PreloadKind::Model => PreloadHandle::Model(
                    asset_cache
                        .load_asset_with_pipeline(self.model_pipeline.clone(), &cmd.locator),
                ),
                PreloadKind::Texture => PreloadHandle::Texture(
                    asset_cache
                        .load_asset_with_pipeline(self.texture_pipeline.clone(), &cmd.locator),
                ),
            };
            preloads.push(PreloadEntry {
                kind: cmd.kind,
                locator: cmd.locator,
                handle,
                tokens: cmd.token.into_iter().collect(),
            });
        }
        let mut settled = Vec::new();
        preloads.retain_mut(|entry| {
            let still_loading = match &entry.handle {
                PreloadHandle::Model(handle) => {
                    matches!(handle.poll_state(), AssetPollState::Loading)
                }
                PreloadHandle::Texture(handle) => {
                    matches!(handle.poll_state(), AssetPollState::Loading)
                }
            };
            if still_loading {
                return true;
            }
            settled.append(&mut entry.tokens);
            false
        });
        settled
    }

    /// Claim terrain hydration requests emitted by the producer that just ran.
    /// Multi-producer shells call this immediately after each producer callback
    /// so the process-global request queue cannot be attributed to a different
    /// instance on the next frame. Single-producer shells can rely on
    /// [`Self::drive_preloads`] calling it for them.
    pub fn capture_terrain_requests(&self) {
        self.terrain_requests
            .borrow_mut()
            .extend(crate::terrain::take_heightmap_requests());
    }

    /// Clone the logical terrain hydration queue for a whole-shell snapshot.
    ///
    /// Asset handles remain cache-owned and warm across a seek; restoring this
    /// plain descriptor set prevents requests from an abandoned future from
    /// leaking into the new branch.
    pub fn snapshot_terrain_requests(&self) -> Vec<crate::terrain::TerrainSource> {
        self.terrain_requests.borrow().iter().cloned().collect()
    }

    /// Replace the logical terrain hydration queue from a whole-shell snapshot.
    pub fn restore_terrain_requests(&self, requests: Vec<crate::terrain::TerrainSource>) {
        *self.terrain_requests.borrow_mut() = requests.into_iter().collect();
    }

    /// Discard only the in-flight preload driver state before restoring a
    /// whole-shell snapshot. Decoded pipeline handles and terrain hydration
    /// remain warm; the snapshot's logical preload commands are re-submitted
    /// on the next frame.
    pub fn reset_preloads(&self) {
        self.preloads.borrow_mut().clear();
    }

    fn drive_terrain_requests(&self, asset_cache: &Arc<AssetCache>) {
        self.terrain_requests.borrow_mut().retain(|source| {
            let handle = asset_cache
                .load_asset_with_pipeline(self.heightmap_pipeline.clone(), &source.locator);
            self.terrain_decode_residency
                .borrow_mut()
                .mark_unsettled(&source.while_pending, |locator| {
                    asset_cache.is_unsettled(locator)
                });
            let resolved = crate::asset::resolve_while_pending_state(
                asset_cache,
                &self.heightmap_pipeline,
                &handle,
                &source.while_pending,
            );
            let primary_is_loading =
                matches!(&resolved, crate::asset::WhilePendingState::Loading(_));
            self.terrain_decode_residency.borrow_mut().mark_source(
                &source.locator,
                &source.while_pending,
                primary_is_loading,
                |locator| asset_cache.is_unsettled(locator),
            );
            match resolved {
                crate::asset::WhilePendingState::Loading(stand_in) => {
                    if let Some(data) = stand_in {
                        crate::terrain::publish_heightmap(source, data);
                    }
                    true
                }
                crate::asset::WhilePendingState::Loaded(data) => {
                    crate::terrain::publish_heightmap(source, data);
                    crate::asset::while_pending_chain_is_unsettled(
                        asset_cache,
                        &source.while_pending,
                    )
                }
                crate::asset::WhilePendingState::Failed => {
                    // Rendering uses the pipeline fallback after terminal
                    // failure; publish that same flat surface for collision.
                    let data = handle.fallback();
                    crate::terrain::publish_heightmap(source, data);
                    crate::asset::while_pending_chain_is_unsettled(
                        asset_cache,
                        &source.while_pending,
                    )
                }
            }
        });
    }

    fn draw_terrain(
        &self,
        render_context: &RenderContext,
        terrain: &crate::terrain::TerrainDescription,
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        let handle = render_context
            .asset_cache
            .load_asset_with_pipeline(self.heightmap_pipeline.clone(), &terrain.heightmap);
        // Snapshot residency before polling. A hot-reloaded stand-in can
        // decode and settle synchronously inside resolve_while_pending_state;
        // after that poll is_unsettled() is already false, so a post-poll
        // mark alone would let the new decoded handle escape eviction.
        self.terrain_decode_residency
            .borrow_mut()
            .mark_unsettled(&terrain.while_pending, |locator| {
                render_context.asset_cache.is_unsettled(locator)
            });
        let resolved = crate::asset::resolve_while_pending_state(
            &render_context.asset_cache,
            &self.heightmap_pipeline,
            &handle,
            &terrain.while_pending,
        );
        let source_descriptor = terrain.source();
        let primary_is_loading =
            matches!(&resolved, crate::asset::WhilePendingState::Loading(_));
        self.terrain_decode_residency.borrow_mut().mark_source(
            &terrain.heightmap,
            &terrain.while_pending,
            primary_is_loading,
            |locator| render_context.asset_cache.is_unsettled(locator),
        );
        let source = match resolved {
            crate::asset::WhilePendingState::Loaded(data) => {
                crate::terrain::publish_heightmap(&source_descriptor, data.clone());
                data
            }
            crate::asset::WhilePendingState::Loading(Some(data)) => {
                crate::terrain::publish_heightmap(&source_descriptor, data.clone());
                data
            }
            crate::asset::WhilePendingState::Loading(None) => handle.fallback(),
            crate::asset::WhilePendingState::Failed => {
                let fallback = handle.fallback();
                crate::terrain::publish_heightmap(&source_descriptor, fallback.clone());
                fallback
            }
        };
        // Drawing ordinarily polls again next frame. If the node disappears,
        // retain any source or placeholder already in flight so the shared
        // asset-progress gate cannot be stranded indefinitely.
        if primary_is_loading
            || crate::asset::while_pending_chain_is_unsettled(
                &render_context.asset_cache,
                &terrain.while_pending,
            )
        {
            crate::terrain::request_heightmap(source_descriptor);
        }
        // Detail maps stream like any other texture. Bind them before the
        // terrain program runs; a map still loading leaves the whole set
        // unbound so the terrain shows its flat band colors rather than a
        // checkerboard smeared across kilometres.
        let detail_bound = terrain.detail_textures().and_then(|maps| {
            let handles = maps.map(|map| {
                let asset = render_context
                    .asset_cache
                    .load_asset_with_pipeline(self.terrain_detail_pipeline.clone(), &map.locator);
                crate::asset::resolve_while_pending_state(
                    &render_context.asset_cache,
                    &self.terrain_detail_pipeline,
                    &asset,
                    &map.while_pending,
                )
            });
            let ready = handles.iter().all(|state| {
                matches!(
                    state,
                    crate::asset::WhilePendingState::Loaded(_)
                        | crate::asset::WhilePendingState::Loading(Some(_))
                )
            });
            if !ready {
                return None;
            }
            let mut averages = [[1.0f32; 3]; 4];
            for (index, state) in handles.iter().enumerate() {
                let texture = match state {
                    crate::asset::WhilePendingState::Loaded(texture)
                    | crate::asset::WhilePendingState::Loading(Some(texture)) => texture,
                    _ => unreachable!("checked above"),
                };
                texture.bind(TERRAIN_DETAIL_UNIT0 + index as u32, render_context);
                averages[index] = texture.average_color();
            }
            Some(averages)
        });
        self.terrain_renderer.borrow_mut().draw(
            render_context,
            terrain,
            source,
            world,
            projection,
            view,
            detail_bound,
        );
    }

    #[cfg(test)]
    pub(crate) fn preloads_in_flight(&self) -> usize {
        self.preloads.borrow().len()
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
            crate::gpu_counters::gpu_counters().texture_created();
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
            crate::gpu_counters::gpu_counters().uploaded(4);
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

    /// A texture compiled into the runtime, uploaded on first use and cached
    /// for the process lifetime. No asset cache, no IO, no fetch — so it is
    /// ready on the very first frame that draws it, on every target.
    ///
    /// Filtering is deliberately NOT set here: `bind_texture_description`
    /// reasserts wrap and filter on every bind (builtin textures share one GL
    /// object across draws, exactly like file textures), so a `Sprite.nearest`
    /// subtree and a `Sprite.linear` one can sample the same atlas in one
    /// frame.
    pub fn builtin_texture(&self, gl: &glow::Context, which: BuiltinTexture) -> glow::Texture {
        let mut textures = self.builtin_textures.borrow_mut();
        *textures.entry(which).or_insert_with(|| {
            let data = match which {
                BuiltinTexture::FontAtlas => crate::sprite_font::atlas_texture_data(),
            };
            unsafe {
                let texture = gl.create_texture().expect("builtin texture");
                crate::gpu_counters::gpu_counters().texture_created();
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    data.width as i32,
                    data.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&data.bytes)),
                );
                crate::gpu_counters::gpu_counters().uploaded(data.bytes.len());
                gl.bind_texture(glow::TEXTURE_2D, None);
                texture
            }
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
                    crate::gpu_counters::gpu_counters().texture_created();
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
                        crate::gpu_counters::gpu_counters().uploaded(face.bytes.len());
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
    /// A filled CONVEX polygon in the XY plane (z = 0), facing +Z: the points in
    /// order, used verbatim (not re-centered) and triangulated as a fan. The 2D
    /// fill behind `Sprite.polygon` and `Sprite.circle`.
    ///
    /// Convexity is a precondition established at construction — `Sprite.polygon`
    /// rejects a non-convex outline rather than let a fan fill it wrongly — so
    /// rendering does no validation.
    ConvexPolygon { points: Vec<[f32; 2]> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneObject {
    Geometry(Shape),
    Model(ModelDescription),
    Terrain(Box<crate::terrain::TerrainDescription>),
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

    pub fn model(model: ModelDescription) -> Self {
        Scene3D {
            obj: SceneObject::Model(model),
            xform: Matrix4::identity(),
        }
    }

    pub fn terrain(terrain: crate::terrain::TerrainDescription) -> Self {
        Scene3D {
            obj: SceneObject::Terrain(Box::new(terrain)),
            xform: Matrix4::identity(),
        }
    }

    /// Set the animation expression on every `Model` node in this subtree —
    /// `Scene.animate`'s semantics. Piping right after `Scene.model` targets
    /// that one model; applying over a group animates each model in it.
    pub fn with_animation(self, expr: crate::anim::AnimExpr) -> Self {
        let obj = match self.obj {
            SceneObject::Model(mut description) => {
                description.animation = Some(expr);
                SceneObject::Model(description)
            }
            SceneObject::Group(items) => SceneObject::Group(
                items
                    .into_iter()
                    .map(|item| item.with_animation(expr.clone()))
                    .collect(),
            ),
            SceneObject::Material(material, items) => SceneObject::Material(
                material,
                items
                    .into_iter()
                    .map(|item| item.with_animation(expr.clone()))
                    .collect(),
            ),
            leaf @ (SceneObject::Geometry(_) | SceneObject::Terrain(_)) => leaf,
        };
        Scene3D { obj, ..self }
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
                // Physics and Transparent preserve authored materials. Physics
                // adds a separate line pass; Transparent changes blending and
                // resets depth around the scene pass.
                DebugRenderMode::Default
                | DebugRenderMode::Transparent
                | DebugRenderMode::Physics => None,
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

                        // While the primary streams in, an `Asset.whilePending`
                        // chain renders its first loaded placeholder instead of
                        // the empty fallback (chainless models resolve exactly
                        // like the old `get()`).
                        let hydrated_model = crate::asset::resolve_while_pending(
                            &render_context.asset_cache,
                            &scene_context.model_pipeline,
                            &model,
                            &model_description.while_pending,
                        );

                        let matrix = world_matrix * self.xform;

                        // Skinned models pay for the joint-matrix uniform array;
                        // static models (no skeleton) render with the basic
                        // material instead. In a debug render mode, both swap to
                        // the matching diagnostic material (the skinned variant
                        // deforms the normal by the joint blend).
                        let is_skinned = hydrated_model.skeleton.get_joint_count() > 0;
                        let debug_override = !matches!(
                            render_context.debug_render_mode,
                            DebugRenderMode::Default
                                | DebugRenderMode::Transparent
                                | DebugRenderMode::Physics
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
                                DebugRenderMode::Default
                                | DebugRenderMode::Transparent
                                | DebugRenderMode::Physics
                                    if is_skinned =>
                                {
                                    SkinnedMaterial::create()
                                }
                                DebugRenderMode::Default
                                | DebugRenderMode::Transparent
                                | DebugRenderMode::Physics => {
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

                        // The pose depends only on the model + expression, so
                        // evaluate it once per model (a blend samples every
                        // clip in the expression) and share it across meshes.
                        let joints = if is_skinned {
                            match &model_description.animation {
                                // The declarative path: game code chose the
                                // pose (clip playheads, blend weights,
                                // per-joint rotations) — evaluate it. An
                                // unknown clip/joint name warns once; a
                                // missing clip contributes the bind pose, a
                                // missing joint is ignored.
                                Some(expr) => crate::anim::skinning_transforms(
                                    &hydrated_model,
                                    expr,
                                    &mut |warning| {
                                        let (kind, name, hint) = match warning {
                                            crate::anim::AnimWarning::MissingClip(name) => (
                                                "clip",
                                                name,
                                                "rendering the bind pose (functor inspect \
lists a model's clips)",
                                            ),
                                            crate::anim::AnimWarning::MissingJoint(name) => (
                                                "joint",
                                                name,
                                                "ignoring it (functor inspect lists a \
model's joints)",
                                            ),
                                        };
                                        scene_context.warn_once(
                                            &format!("anim-{kind}:{str}:{name}"),
                                            &format!(
                                                "[anim] model \"{str}\" has no {kind} \
named \"{name}\" — {hint}"
                                            ),
                                        );
                                    },
                                ),
                                // Zero-config default: the first clip
                                // auto-plays, looping on the game clock.
                                None => match hydrated_model.animations.get(animation_index) {
                                    Some(animation) => {
                                        let time =
                                            render_context.frame_time.tts % animation.duration;
                                        let animated_skeleton = Skeleton::animate(
                                            &hydrated_model.skeleton,
                                            animation,
                                            time,
                                        );
                                        animated_skeleton.get_skinning_transforms()
                                    }
                                    None => vec![Matrix4::identity(); 50],
                                },
                            }
                        } else {
                            vec![]
                        };

                        for mesh in hydrated_model.meshes.iter() {
                            // Go through selectors, and adjust
                            // let override_material_description = Some(MaterialDescription::Texture(
                            //     TextureDescription::File("vr_glove_color.jpg".to_string()),
                            // ));

                            let mut override_material_description: Option<&MaterialDescription> =
                                None;

                            // glTF 2.0: "When a mesh is skinned, the transform
                            // of the node that references the mesh MUST be
                            // ignored; only the joint transforms apply." The
                            // skeleton already carries the full chain up to
                            // the scene root, so a skinned mesh takes only the
                            // Scene-graph transform; static meshes keep their
                            // node transform. This matrix also feeds the
                            // depth/shadow pass (the material choice above),
                            // so shadows agree with the main pass.
                            let mut matrix = match is_skinned {
                                true => matrix,
                                false => matrix * mesh.transform,
                            };

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

            // The dedicated GPU terrain path is initialized and drawn here;
            // keeping it as its own scene leaf (rather than a giant
            // `Shape::Heightmap`) lets rendering choose LOD without changing
            // the pure scene description. Terrain receives the forward
            // pass's shadows but does not yet render into the shadow map.
            SceneObject::Terrain(terrain) if !depth_pass => {
                let xform = world_matrix * self.xform;
                scene_context.draw_terrain(
                    render_context,
                    terrain,
                    &xform,
                    projection_matrix,
                    view_matrix,
                );
            }
            SceneObject::Terrain(_) => {}

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
                // One persistent GL mesh per (rows, cols): build it on first sight,
                // then re-upload its vertices in place only when the heights change
                // (a no-op for static terrain). No per-frame VAO/VBO/EBO churn.
                let mut heightmaps = scene_context.heightmaps.borrow_mut();
                let counters = crate::gpu_counters::gpu_counters();
                let mesh = match heightmaps.entry((*rows, *cols)) {
                    std::collections::hash_map::Entry::Occupied(e) => {
                        counters.cache_hit();
                        e.into_mut()
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        counters.cache_miss();
                        e.insert(geometry::HeightmapMesh::create(
                            &render_context.gl,
                            *rows as usize,
                            *cols as usize,
                            heights,
                        ))
                    }
                };
                mesh.update(&render_context.gl, heights);
                mesh.draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::ConvexPolygon { points }) => {
                // A polygon needs at least a triangle; anything less is rejected
                // at construction, so a shorter list here means malformed
                // protocol data and is skipped rather than drawn as garbage.
                if points.len() < 3 {
                    return;
                }
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                // One persistent mesh per point count (see `PolygonMesh`).
                let mut polygons = scene_context.polygons.borrow_mut();
                let counters = crate::gpu_counters::gpu_counters();
                let mesh = match polygons.entry(points.len()) {
                    std::collections::hash_map::Entry::Occupied(e) => {
                        counters.cache_hit();
                        e.into_mut()
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        counters.cache_miss();
                        e.insert(geometry::PolygonMesh::create(&render_context.gl, points))
                    }
                };
                mesh.update(&render_context.gl, points);
                mesh.draw(&render_context.gl);
            }
        }
    }
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

#[cfg(test)]
mod preload_tests {
    use super::*;
    use crate::asset::preload::{PreloadCommand, PreloadKind};
    use crate::asset::AssetCache;

    fn temp_glb(name: &str) -> String {
        // A minimal valid-magic glb (the model pipeline falls back panic-free
        // on truncated content — decode result doesn't matter here, only that
        // the byte-load SETTLES).
        let path = std::env::temp_dir().join(format!(
            "functor-preload-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::write(&path, b"glTF").unwrap();
        path.to_string_lossy().to_string()
    }

    /// The driver loads queued commands through the pipelines, drives them
    /// to settlement (local reads settle on the first poll), reports settled
    /// preloadThen tokens exactly once, and counts the loads in the cache's
    /// progress (so `Sub.assets` sees preloads like any other load).
    #[test]
    fn drive_preloads_loads_settles_and_reports_tokens() {
        let ctx = SceneContext::new();
        let cache = Arc::new(AssetCache::new());
        let model = temp_glb("boss");
        let texture = temp_glb("wood");
        let missing = "preload/does-not-exist.glb".to_string();

        let settled = ctx.drive_preloads(
            &cache,
            vec![
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: model.clone(),
                    token: Some(7),
                },
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: missing.clone(),
                    token: Some(8),
                },
                PreloadCommand {
                    kind: PreloadKind::Texture,
                    locator: texture.clone(),
                    token: None,
                },
            ],
        );
        // Local reads settle on the first poll: the loaded model AND the
        // failed one both report (settled = loaded OR failed); the tokenless
        // texture settles silently.
        let mut sorted = settled.clone();
        sorted.sort();
        assert_eq!(sorted, vec![7, 8]);
        assert_eq!(ctx.preloads_in_flight(), 0, "nothing left in flight");

        // Preloads count in Sub.assets' snapshot like any load: 3 started,
        // 1 failed (progress keys by PATH, so distinct files are used).
        let progress = cache.progress();
        assert_eq!(progress.total, 3);
        assert_eq!(progress.failed.len(), 1);
        assert_eq!(progress.failed[0].0, missing);

        // A later frame with no commands is a cheap no-op.
        assert!(ctx.drive_preloads(&cache, vec![]).is_empty());

        for f in [model, texture] {
            let _ = std::fs::remove_file(&f);
        }
    }

    /// Duplicate commands for one (kind, locator) MERGE into a single
    /// in-flight entry (no unbounded growth from spamming a stalled URL),
    /// and every waiting token still reports on settlement.
    #[test]
    fn duplicate_preloads_merge_and_report_every_token() {
        let ctx = SceneContext::new();
        let cache = Arc::new(AssetCache::new());
        // A remote url with no fetcher installed stays Loading? No — it
        // fails fast ("remote assets are not supported"); use a missing
        // local path scheduled twice IN ONE BATCH so the merge happens
        // before the first poll settles it.
        let missing = "preload/dup-missing.glb".to_string();
        let settled = ctx.drive_preloads(
            &cache,
            vec![
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: missing.clone(),
                    token: Some(41),
                },
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: missing.clone(),
                    token: Some(42),
                },
                // Restoring a whole-shell snapshot may resubmit a token to an
                // entry that is still alive in the driver. It stays one-shot.
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: missing.clone(),
                    token: Some(42),
                },
                PreloadCommand {
                    kind: PreloadKind::Model,
                    locator: missing.clone(),
                    token: None,
                },
            ],
        );
        // One merged entry, settled on the first poll (missing file fails
        // fast), BOTH tokens delivered.
        let mut sorted = settled.clone();
        sorted.sort();
        assert_eq!(sorted, vec![41, 42]);
        assert_eq!(ctx.preloads_in_flight(), 0);
        // The path counted once in progress despite repeated commands.
        assert_eq!(cache.progress().total, 1);
    }

    #[test]
    fn terrain_request_snapshot_restore_discards_future_requests() {
        let ctx = SceneContext::new();
        let earlier = crate::terrain::TerrainSource {
            locator: "terrain/earlier.png".to_string(),
            while_pending: Vec::new(),
        };
        let future = crate::terrain::TerrainSource {
            locator: "terrain/future.png".to_string(),
            while_pending: Vec::new(),
        };

        crate::terrain::request_heightmap(earlier.clone());
        ctx.capture_terrain_requests();
        let snapshot = ctx.snapshot_terrain_requests();

        crate::terrain::request_heightmap(future);
        ctx.capture_terrain_requests();
        assert_eq!(ctx.snapshot_terrain_requests().len(), 2);

        ctx.restore_terrain_requests(snapshot);
        assert_eq!(ctx.snapshot_terrain_requests(), vec![earlier]);
    }

    #[test]
    fn terrain_decode_residency_evicts_sources_after_one_unused_epoch() {
        let mut residency = TerrainDecodeResidency::default();
        let mut evicted = Vec::new();

        residency.begin_epoch();
        residency.mark(["world-a.png", "proxy-a.png"]);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));
        assert!(evicted.is_empty());

        residency.begin_epoch();
        residency.mark(["world-b.png"]);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));
        assert!(evicted.is_empty(), "one unused epoch is retained as grace");

        residency.begin_epoch();
        residency.mark(["world-b.png"]);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));
        evicted.sort();
        assert_eq!(evicted, ["proxy-a.png", "world-a.png"]);
    }

    #[test]
    fn settled_terrain_stand_ins_expire_while_the_primary_stays_resident() {
        let mut residency = TerrainDecodeResidency::default();
        let pending = vec!["world-low.png".to_string()];
        let mut evicted = Vec::new();

        residency.begin_epoch();
        residency.mark_source("world.png", &pending, true, |_| false);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));

        residency.begin_epoch();
        residency.mark_source("world.png", &pending, false, |_| false);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));
        assert!(evicted.is_empty(), "stand-in keeps one epoch of grace");

        residency.begin_epoch();
        residency.mark_source("world.png", &pending, false, |_| false);
        residency.evict_stale(|locator| evicted.push(locator.to_string()));
        assert_eq!(evicted, ["world-low.png"]);
        assert!(residency.last_used.contains_key("world.png"));
    }

    #[test]
    fn synchronously_reloaded_terrain_stand_in_remains_evictable() {
        let ctx = SceneContext::new();
        let cache = Arc::new(AssetCache::new());
        let primary = temp_glb("terrain-residency-primary.png");
        let stand_in = temp_glb("terrain-residency-stand-in.png");
        let pending = vec![stand_in.clone()];
        let primary_handle =
            cache.load_asset_with_pipeline(ctx.heightmap_pipeline.clone(), &primary);
        assert!(matches!(
            primary_handle.poll_state(),
            AssetPollState::Loaded(_)
        ));
        let stand_in_handle =
            cache.load_asset_with_pipeline(ctx.heightmap_pipeline.clone(), &stand_in);
        assert!(matches!(
            stand_in_handle.poll_state(),
            AssetPollState::Loaded(_)
        ));

        // Let the original settled stand-in age out of decoded residency.
        {
            let mut residency = ctx.terrain_decode_residency.borrow_mut();
            residency.begin_epoch();
            residency.mark([primary.as_str(), stand_in.as_str()]);
            residency.evict_stale(|locator| ctx.heightmap_pipeline.evict(locator));
            residency.begin_epoch();
            residency.mark([primary.as_str()]);
            residency.evict_stale(|locator| ctx.heightmap_pipeline.evict(locator));
            residency.begin_epoch();
            residency.mark([primary.as_str()]);
            residency.evict_stale(|locator| ctx.heightmap_pipeline.evict(locator));
        }
        assert!(ctx.heightmap_pipeline.get_opt(&stand_in).is_none());

        // Hot reload makes the already-started stand-in unsettled. Its local
        // read then settles synchronously during resolution while the primary
        // remains loaded.
        cache.evict(&stand_in);
        ctx.heightmap_pipeline.evict(&stand_in);
        assert!(cache.is_unsettled(&stand_in));
        {
            let mut residency = ctx.terrain_decode_residency.borrow_mut();
            residency.mark_unsettled(&pending, |locator| cache.is_unsettled(locator));
        }
        assert!(matches!(
            crate::asset::resolve_while_pending_state(
                &cache,
                &ctx.heightmap_pipeline,
                &primary_handle,
                &pending,
            ),
            crate::asset::WhilePendingState::Loaded(_)
        ));
        assert!(!cache.is_unsettled(&stand_in));
        ctx.terrain_decode_residency.borrow_mut().mark_source(
            &primary,
            &pending,
            false,
            |locator| cache.is_unsettled(locator),
        );
        assert!(
            ctx.terrain_decode_residency
                .borrow()
                .last_used
                .contains_key(&stand_in),
            "the pre-poll mark retains a synchronously settled decode"
        );

        // The refreshed decode still receives one grace epoch, then expires.
        {
            let mut residency = ctx.terrain_decode_residency.borrow_mut();
            residency.begin_epoch();
            residency.mark([primary.as_str()]);
            residency.evict_stale(|locator| ctx.heightmap_pipeline.evict(locator));
            assert!(ctx.heightmap_pipeline.get_opt(&stand_in).is_some());
            residency.begin_epoch();
            residency.mark([primary.as_str()]);
            residency.evict_stale(|locator| ctx.heightmap_pipeline.evict(locator));
        }
        assert!(ctx.heightmap_pipeline.get_opt(&stand_in).is_none());

        for path in [primary, stand_in] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// A game that only ever declares `Physics.heightfield` — never a terrain
    /// in its scene — still hydrates collision: the physics declaration is
    /// what requests the heightmap, and the shell's terrain asset driver
    /// decodes and publishes it with no renderer in the loop. (Ported from
    /// the deleted netsim suite, which drove the same slice through a game
    /// instance; the fetch → decode → publish → heightfield chain here needs
    /// neither a producer nor a network.)
    #[test]
    fn physics_only_terrain_hydrates_without_a_renderer() {
        use crate::physics::{
            remove_world, with_world, Body, PhysicsScene, Shape, SteppedPhysics, DEFAULT_WORLD,
            FIXED_DT,
        };
        use crate::terrain::{TerrainGeometry, TerrainSource};

        remove_world(DEFAULT_WORLD);
        let path = std::env::temp_dir().join(format!(
            "functor-physics-only-terrain-{}.png",
            std::process::id()
        ));
        image::ImageBuffer::from_pixel(2, 2, image::Luma([u16::MAX]))
            .save(&path)
            .unwrap();
        let source = TerrainSource {
            locator: path.to_string_lossy().to_string(),
            while_pending: Vec::new(),
        };
        let scene = PhysicsScene::create(
            [0.0, -9.81, 0.0],
            vec![Body::fixed(
                "terrain".to_string(),
                Shape::Heightfield {
                    geometry: TerrainGeometry {
                        source: source.clone(),
                        width: 20.0,
                        depth: 20.0,
                        min_height: 0.0,
                        max_height: 10.0,
                    },
                    data: None,
                },
            )],
        );
        let hit_y = || {
            with_world(DEFAULT_WORLD, |world| {
                world
                    .raycast([0.0, 20.0, 0.0], [0.0, -1.0, 0.0], 30.0)
                    .map(|hit| hit.position[1])
            })
            .flatten()
        };

        let ctx = SceneContext::new();
        let cache = Arc::new(AssetCache::new());
        let mut physics = SteppedPhysics::new();

        // The first fixed frame declares the heightfield with no samples yet;
        // that declaration is the only thing that asks for the heightmap.
        physics.advance(&scene, FIXED_DT);
        assert!(hit_y().is_none(), "nothing to collide with before the load");

        // The shell's asset driver claims that request, decodes the file and
        // publishes it into the render/physics hydration bridge.
        ctx.drive_preloads(&cache, vec![]);
        assert!(
            crate::terrain::hydrated_heightmap(&source).is_some(),
            "the terrain driver published the decoded surface"
        );

        // The next frame builds the collider from the published samples.
        physics.advance(&scene, FIXED_DT);
        let hit = hit_y().expect("the hydrated heightfield collides");
        assert!(
            (hit - 10.0).abs() < 0.001,
            "the max-height sample reached physics: {hit}"
        );

        remove_world(DEFAULT_WORLD);
        let _ = std::fs::remove_file(&path);
    }
}
