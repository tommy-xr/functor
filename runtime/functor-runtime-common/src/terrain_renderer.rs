//! GPU terrain rendering: a finite quadtree selected on the CPU and drawn as
//! instances of one immutable 64×64 grid.
//!
//! The heightmap stays a 16-bit integer texture. Vertices sample it directly,
//! so changing LOD uploads only a small instance list; it never rebuilds or
//! uploads a world-sized mesh. Border skirts hide T-junctions between adjacent
//! LODs. Selection uses the frame's stable center camera, shared by both eyes.

use std::{collections::HashMap, sync::Arc};

use cgmath::{InnerSpace, Matrix, Matrix3, Matrix4, SquareMatrix, Vector3, Vector4};
use glow::HasContext;

use crate::{
    asset::pipelines::HeightmapData,
    fog::{FogUniforms, FOG_GLSL},
    light::{lighting_glsl, LightingUniforms},
    shader::{Shader, ShaderType},
    shader_program::{ShaderProgram, UniformLocation},
    DebugRenderMode, RenderContext, TerrainDescription, TerrainGrass,
};

const PATCH_QUADS: u32 = 64;
const MAX_LEVEL: u32 = 10;
const MAX_INSTANCES: usize = 8192;
const MAX_GRASS_INSTANCES: usize = 20_000;
/// First texture unit for the detail maps. Unit 0 stays the height texture.
const DETAIL_TEXTURE_UNIT0: i32 = 1;
/// The baked macro map sits above the four detail units.
const MACRO_TEXTURE_UNIT: i32 = 5;
/// Furthest the detail band may reach, in metres from the camera.
///
/// A cap, not a flat value. Detail stops being resolvable at a distance that
/// depends on the surface, not on how large the world is, so a 4 km map gains
/// nothing from carrying tiled detail out to a kilometre — that is fill rate
/// spent below a pixel. But the band must still shrink with the terrain: on a
/// 200 m island an absolute 400 m would put the entire world at full detail
/// permanently, which is why the fraction below also applies.
const DETAIL_FADE_MAX_END: f32 = 400.0;
/// The band is also bounded by a fraction of the terrain's longest side, which
/// is what governs on anything smaller than ~1.4 km.
const DETAIL_FADE_END_FRACTION: f32 = 0.28;
/// Detail holds at full strength within this fraction of the band's end.
const DETAIL_FADE_START_FRACTION: f32 = 0.15;

const VERTEX_SHADER_SOURCE: &str = r#"
        // uv.xy plus 1 for a duplicated skirt vertex.
        layout (location = 0) in vec3 inGrid;
        // normalized heightmap offset.xy and scale.xy.
        layout (location = 1) in vec4 inPatch;

        uniform mat4 world;
        uniform mat3 normalMatrix;
        uniform mat4 view;
        uniform mat4 projection;
        uniform highp usampler2D heightmapTex;
        uniform vec3 terrainSize; // width, depth, height range
        uniform float minHeight;
        uniform float skirtDepth;

        out vec3 worldNormal;
        out vec3 worldTangent;
        out vec3 worldPos;
        out float localHeight;
        out vec2 terrainUv;

        float heightAt(vec2 uv) {
            ivec2 size = textureSize(heightmapTex, 0);
            ivec2 maxCoord = size - ivec2(1);
            vec2 p = clamp(uv, vec2(0.0), vec2(1.0)) * vec2(maxCoord);
            ivec2 a = ivec2(floor(p));
            ivec2 b = min(a + ivec2(1), maxCoord);
            vec2 f = fract(p);
            float h00 = float(texelFetch(heightmapTex, a, 0).r);
            float h10 = float(texelFetch(heightmapTex, ivec2(b.x, a.y), 0).r);
            float h01 = float(texelFetch(heightmapTex, ivec2(a.x, b.y), 0).r);
            float h11 = float(texelFetch(heightmapTex, b, 0).r);
            // Match Rapier's default heightfield diagonal exactly. Bilinear
            // interpolation would produce a different surface inside every
            // non-coplanar source cell.
            float sampleHeight = f.x + f.y <= 1.0
                ? h00 + f.x * (h10 - h00) + f.y * (h01 - h00)
                : h11 + (1.0 - f.x) * (h01 - h11)
                      + (1.0 - f.y) * (h10 - h11);
            float normalized = sampleHeight / 65535.0;
            return minHeight + normalized * terrainSize.z;
        }

        void main() {
            vec2 uv = inPatch.xy + inGrid.xy * inPatch.zw;
            terrainUv = uv;
            ivec2 texSize = textureSize(heightmapTex, 0);
            vec2 texel = 1.0 / vec2(max(texSize - ivec2(1), ivec2(1)));

            float h = heightAt(uv);
            float hL = heightAt(uv - vec2(texel.x, 0.0));
            float hR = heightAt(uv + vec2(texel.x, 0.0));
            float hD = heightAt(uv - vec2(0.0, texel.y));
            float hU = heightAt(uv + vec2(0.0, texel.y));
            float dx = terrainSize.x * texel.x;
            float dz = terrainSize.y * texel.y;
            float dhdx = (hR - hL) / max(2.0 * dx, 1e-5);
            float dhdz = (hU - hD) / max(2.0 * dz, 1e-5);

            vec3 localNormal = normalize(vec3(-dhdx, 1.0, -dhdz));
            vec3 localTangent = normalize(vec3(1.0, dhdx, 0.0));
            worldNormal = normalMatrix * localNormal;
            worldTangent = mat3(world) * localTangent;

            vec3 localPos = vec3(
                (uv.x - 0.5) * terrainSize.x,
                h - inGrid.z * skirtDepth,
                (uv.y - 0.5) * terrainSize.y);
            vec4 wp = world * vec4(localPos, 1.0);
            worldPos = wp.xyz;
            localHeight = h;
            gl_Position = projection * view * wp;
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec3 worldNormal;
        in vec3 worldTangent;
        in vec3 worldPos;
        in float localHeight;
        in vec2 terrainUv;

        uniform vec3 terrainColor;
        uniform int useLayers;
        uniform vec3 lowColor;
        uniform vec3 highColor;
        uniform vec3 rockColor;
        uniform vec3 snowColor;
        uniform float minTerrainHeight;
        uniform float maxTerrainHeight;
        uniform float snowHeight;
        uniform int debugMode; // 0=lit, 1=normals, 2=tangents
        uniform sampler2D macroTex;
        // How much of the baked occlusion to apply. Full strength reads as
        // painted-on shadow on flat ground; this keeps ridges and valleys
        // legible without the terrain looking pre-shaded.
        const float MACRO_STRENGTH = 0.55;
        uniform int useTextures;
        uniform sampler2D lowTex;
        uniform sampler2D highTex;
        uniform sampler2D rockTex;
        uniform sampler2D snowTex;
        uniform float detailTile;
        uniform float detailFadeStart;
        uniform float detailFadeEnd;
        // Explicitly this shader's own: fog owns fogCameraPos and does not set
        // it when fog is disabled, but the detail fade must work regardless.
        uniform vec3 detailCameraPos;
        // Each map's mean RGB, computed once on the CPU at load. Sampling the
        // top mip per fragment cost one fetch per map to recover a constant.
        uniform vec3 detailAverage[4];

        out vec4 fragColor;

        // One tile repeat is obvious across kilometres, so sample a second
        // time at an irrational multiple and average: the two lattices only
        // re-align at their product, which is far past the fade distance.
        //
        // `fade` resolves to the map's own top mip — a 1x1 average — instead
        // of to an authored color. That makes the distance transition
        // seamless for ANY map without asking the game to hand-match a band
        // color to its texture, which is what a fixed far color would require.
        // Returns STRUCTURE, not color: the sample divided by the map's own
        // average, supplied as a uniform. The ratio is ~1.0 on average, so
        // multiplying a band color by this adds the map's detail without
        // shifting its hue — a photographic albedo averages brown, and using
        // it directly would repaint a green hillside as dirt. It also makes
        // the distance fade exact: as `fade` reaches 0 the result is 1.0, i.e.
        // no change, so there is nothing to seam against.
        vec3 detail(sampler2D tex, vec2 uv, float fade, vec3 average) {
            vec3 near = mix(texture(tex, uv).rgb, texture(tex, uv * 0.137).rgb, 0.5);
            return mix(vec3(1.0), clamp(near / max(average, vec3(1e-3)), 0.0, 2.5), fade);
        }

        void main() {
            vec3 n = normalize(worldNormal);
            if (debugMode == 1) {
                fragColor = vec4(n * 0.5 + 0.5, 1.0);
                return;
            }
            if (debugMode == 2) {
                fragColor = vec4(normalize(worldTangent) * 0.5 + 0.5, 1.0);
                return;
            }

            vec3 diffuseLight;
            vec3 specularLight;
            accumulateLights(n, worldPos, diffuseLight, specularLight);
            vec3 albedo = terrainColor;
            if (useLayers == 1) {
                float heightRange = max(maxTerrainHeight - minTerrainHeight, 1e-4);
                float h = clamp((localHeight - minTerrainHeight) / heightRange, 0.0, 1.0);
                float bandWeight = smoothstep(0.15, 0.72, h);
                float slope = 1.0 - clamp(n.y, 0.0, 1.0);
                float rockWeight = smoothstep(0.30, 0.67, slope);
                float snowWeight = smoothstep(
                    snowHeight,
                    snowHeight + heightRange * 0.08,
                    localHeight) * (1.0 - rockWeight * 0.72);

                albedo = mix(lowColor, highColor, bandWeight);
                albedo = mix(albedo, rockColor, rockWeight);
                albedo = mix(albedo, snowColor, snowWeight);

                if (useTextures == 1) {
                    // Past the fade the detail is sub-pixel: sampling it only
                    // buys aliasing, so resolve to the flat band colors that
                    // the untextured terrain already renders correctly.
                    float fade = 1.0 - smoothstep(
                        detailFadeStart, detailFadeEnd, distance(worldPos, detailCameraPos));
                    // Past the band every detail() is exactly vec3(1.0), so
                    // the 8 fetches it takes to compute that are pure waste —
                    // and on a terrain vista most of the screen is past it.
                    // The branch is coherent (it tracks distance), which is
                    // what a tile-based GPU wants.
                    // Safe despite `texture()`'s implicit derivatives inside
                    // non-uniform control flow: the only quads that straddle
                    // this branch are ones where fade ~ 0, so a bad sample is
                    // multiplied by ~0 and clamped. Moving this threshold off
                    // zero would make that no longer true.
                    if (fade > 0.0) {
                        // The SAME weights as the colors above: texturing
                        // dresses the bands, it does not move them.
                        vec2 uv = worldPos.xz / detailTile;
                        vec3 structure = mix(
                            detail(lowTex, uv, fade, detailAverage[0]),
                            detail(highTex, uv, fade, detailAverage[1]),
                            bandWeight);
                        structure = mix(
                            structure, detail(rockTex, uv, fade, detailAverage[2]), rockWeight);
                        structure = mix(
                            structure, detail(snowTex, uv, fade, detailAverage[3]), snowWeight);
                        albedo *= structure;
                    }
                }
            }
            // Baked occlusion, unique per world position. Tiled detail maps
            // minify to their own average by construction, so past a few
            // hundred metres they contribute nothing; this is what still
            // distinguishes a valley from a ridge at four kilometres.
            //
            // Applied to the diffuse term only. Occlusion describes how much
            // sky a point can see, which is an ambient/diffuse property —
            // scaling the specular by it would dim highlights that come from a
            // direction the geometry does not block.
            // Squared before mixing. Horizon openness over real terrain is a
            // narrow band — most of a landscape sees most of the sky, so the
            // raw values cluster near 1.0 and mixing them straight barely
            // reads. Squaring spreads that band without moving its top.
            float sky = texture(macroTex, terrainUv).r;
            float occlusion = mix(1.0, sky * sky, MACRO_STRENGTH);
            vec3 shaded = albedo * diffuseLight * occlusion + specularLight;
            fragColor = vec4(applyFog(shaded, worldPos), 1.0);
        }
"#;

const GRASS_VERTEX_SHADER_SOURCE: &str = r#"
        // A small crossed-blade cluster, authored around the local origin.
        layout (location = 0) in vec3 inBlade;
        // Terrain-local x/z, random rotation, and height scale.
        layout (location = 1) in vec4 inGrass;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;
        uniform highp usampler2D heightmapTex;
        uniform vec3 terrainSize; // width, depth, height range
        uniform float minHeight;
        uniform float snowHeight;
        uniform float bladeHeight;
        uniform float time;

        out vec3 worldPos;
        out float shade;
        out float visibility;

        float heightAt(vec2 uv) {
            ivec2 size = textureSize(heightmapTex, 0);
            ivec2 maxCoord = size - ivec2(1);
            vec2 p = clamp(uv, vec2(0.0), vec2(1.0)) * vec2(maxCoord);
            ivec2 a = ivec2(floor(p));
            ivec2 b = min(a + ivec2(1), maxCoord);
            vec2 f = fract(p);
            float h00 = float(texelFetch(heightmapTex, a, 0).r);
            float h10 = float(texelFetch(heightmapTex, ivec2(b.x, a.y), 0).r);
            float h01 = float(texelFetch(heightmapTex, ivec2(a.x, b.y), 0).r);
            float h11 = float(texelFetch(heightmapTex, b, 0).r);
            float sampleHeight = f.x + f.y <= 1.0
                ? h00 + f.x * (h10 - h00) + f.y * (h01 - h00)
                : h11 + (1.0 - f.x) * (h01 - h11)
                      + (1.0 - f.y) * (h10 - h11);
            return minHeight + sampleHeight / 65535.0 * terrainSize.z;
        }

        void main() {
            vec2 uv = vec2(
                inGrass.x / terrainSize.x + 0.5,
                inGrass.y / terrainSize.y + 0.5);
            ivec2 texSize = textureSize(heightmapTex, 0);
            vec2 texel = 1.0 / vec2(max(texSize - ivec2(1), ivec2(1)));
            float h = heightAt(uv);
            float hL = heightAt(uv - vec2(texel.x, 0.0));
            float hR = heightAt(uv + vec2(texel.x, 0.0));
            float hD = heightAt(uv - vec2(0.0, texel.y));
            float hU = heightAt(uv + vec2(0.0, texel.y));
            float dx = terrainSize.x * texel.x;
            float dz = terrainSize.y * texel.y;
            vec3 normal = normalize(vec3(
                -(hR - hL) / max(2.0 * dx, 1e-5),
                1.0,
                -(hU - hD) / max(2.0 * dz, 1e-5)));

            float low = minHeight + terrainSize.z * 0.16;
            float aboveWater = smoothstep(low, low + terrainSize.z * 0.06, h);
            float belowSnow = 1.0 - smoothstep(
                snowHeight - terrainSize.z * 0.05,
                snowHeight,
                h);
            float flatEnough = smoothstep(0.68, 0.80, normal.y);
            visibility = aboveWater * belowSnow * flatEnough;

            float angle = inGrass.z;
            float c = cos(angle);
            float s = sin(angle);
            vec3 blade = inBlade * bladeHeight * inGrass.w * visibility;
            blade.xz = mat2(c, -s, s, c) * blade.xz;
            float wind = sin(time * 1.7 + inGrass.x * 0.019 + inGrass.y * 0.013);
            blade.xz += vec2(wind, wind * 0.37) * blade.y * blade.y * 0.16;

            vec3 localPos = vec3(inGrass.x, h, inGrass.y) + blade;
            vec4 wp = world * vec4(localPos, 1.0);
            worldPos = wp.xyz;
            shade = mix(0.68, 1.12, inBlade.y) * mix(0.86, 1.08, inGrass.w - 0.7);
            gl_Position = projection * view * wp;
        }
"#;

const GRASS_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec3 worldPos;
        in float shade;
        in float visibility;

        uniform vec3 grassColor;

        out vec4 fragColor;

        void main() {
            if (visibility < 0.08) {
                discard;
            }
            fragColor = vec4(applyFog(grassColor * shade, worldPos), 1.0);
        }
"#;

struct TerrainUniforms {
    world: UniformLocation,
    normal_matrix: UniformLocation,
    view: UniformLocation,
    projection: UniformLocation,
    heightmap: UniformLocation,
    terrain_size: UniformLocation,
    min_height: UniformLocation,
    skirt_depth: UniformLocation,
    color: UniformLocation,
    use_layers: UniformLocation,
    low_color: UniformLocation,
    high_color: UniformLocation,
    rock_color: UniformLocation,
    snow_color: UniformLocation,
    min_terrain_height: UniformLocation,
    max_terrain_height: UniformLocation,
    snow_height: UniformLocation,
    debug_mode: UniformLocation,
    macro_map: UniformLocation,
    use_textures: UniformLocation,
    detail_textures: [UniformLocation; 4],
    detail_tile: UniformLocation,
    detail_fade_start: UniformLocation,
    detail_fade_end: UniformLocation,
    detail_camera_pos: UniformLocation,
    detail_averages: [UniformLocation; 4],
    lighting: LightingUniforms,
    fog: FogUniforms,
}

struct TerrainProgram {
    program: ShaderProgram,
    uniforms: TerrainUniforms,
}

struct GrassUniforms {
    world: UniformLocation,
    view: UniformLocation,
    projection: UniformLocation,
    heightmap: UniformLocation,
    terrain_size: UniformLocation,
    min_height: UniformLocation,
    snow_height: UniformLocation,
    blade_height: UniformLocation,
    time: UniformLocation,
    color: UniformLocation,
    fog: FogUniforms,
}

struct GrassProgram {
    program: ShaderProgram,
    uniforms: GrassUniforms,
}

struct TerrainGeometry {
    key: TerrainInstanceKey,
    vao: glow::VertexArray,
    _grid_vbo: glow::Buffer,
    _index_ebo: glow::Buffer,
    instance_vbo: glow::Buffer,
    index_count: i32,
    last_used_frame: u64,
    cache_key: Option<PatchCacheKey>,
    scratch_instances: Vec<[f32; 4]>,
    uploaded_instances: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
struct TerrainInstanceKey {
    heightmap: String,
    terrain_width_bits: u32,
    terrain_depth_bits: u32,
    min_height_bits: u32,
    max_height_bits: u32,
    max_pixel_error_bits: u32,
    world_bits: [u32; 16],
}

impl TerrainInstanceKey {
    fn new(description: &TerrainDescription, world: &Matrix4<f32>) -> Self {
        let matrix: &[f32; 16] = world.as_ref();
        Self {
            heightmap: description.heightmap.clone(),
            terrain_width_bits: description.width.to_bits(),
            terrain_depth_bits: description.depth.to_bits(),
            min_height_bits: description.min_height.to_bits(),
            max_height_bits: description.max_height.to_bits(),
            max_pixel_error_bits: description.max_pixel_error.to_bits(),
            world_bits: std::array::from_fn(|index| matrix[index].to_bits()),
        }
    }

    fn matches(&self, description: &TerrainDescription, world: &Matrix4<f32>) -> bool {
        let matrix: &[f32; 16] = world.as_ref();
        self.heightmap == description.heightmap
            && self.terrain_width_bits == description.width.to_bits()
            && self.terrain_depth_bits == description.depth.to_bits()
            && self.min_height_bits == description.min_height.to_bits()
            && self.max_height_bits == description.max_height.to_bits()
            && self.max_pixel_error_bits == description.max_pixel_error.to_bits()
            && self
                .world_bits
                .iter()
                .zip(matrix)
                .all(|(cached, current)| *cached == current.to_bits())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PatchCacheKey {
    sample_width: u32,
    sample_height: u32,
    terrain_width: f32,
    terrain_depth: f32,
    min_height: f32,
    max_height: f32,
    max_pixel_error: f32,
    world: Matrix4<f32>,
    camera_pos: Vector3<f32>,
    view_projections: [Matrix4<f32>; 2],
    frustum_count: usize,
    projection_scale: f32,
    viewport_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct GrassCacheKey {
    camera_cell_x: i32,
    camera_cell_z: i32,
    terrain_width: f32,
    terrain_depth: f32,
    grass: TerrainGrass,
}

struct GrassGeometry {
    key: TerrainInstanceKey,
    vao: glow::VertexArray,
    _blade_vbo: glow::Buffer,
    instance_vbo: glow::Buffer,
    vertex_count: i32,
    last_used_frame: u64,
    cache_key: Option<GrassCacheKey>,
    scratch_instances: Vec<[f32; 4]>,
    uploaded_instances: Vec<[f32; 4]>,
}

struct HeightTexture {
    revision: u64,
    texture: glow::Texture,
    /// Baked occlusion, derived from the same samples and invalidated with
    /// them — a heightmap reload must not leave stale shadows behind.
    macro_texture: glow::Texture,
    last_used_frame: u64,
}

impl HeightTexture {
    /// Both textures are created together and must die together; splitting
    /// this across the two eviction sites leaked the macro map from one.
    fn delete(&self, gl: &glow::Context) {
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            gl.delete_texture(self.texture);
            gl.delete_texture(self.macro_texture);
        }
        counters.texture_deleted();
        counters.texture_deleted();
    }
}

#[derive(Default)]
pub(crate) struct TerrainRenderer {
    program: Option<TerrainProgram>,
    geometries: Vec<TerrainGeometry>,
    grass_program: Option<GrassProgram>,
    grass_geometries: Vec<GrassGeometry>,
    height_textures: HashMap<(String, [u32; 3]), HeightTexture>,
    current_frame: u64,
}

impl TerrainRenderer {
    pub(crate) fn begin_frame(&mut self, gl: &glow::Context, frame: u64) {
        if self.current_frame == frame {
            return;
        }
        self.current_frame = frame;
        self.height_textures.retain(|_, entry| {
            let recent = frame.wrapping_sub(entry.last_used_frame) <= 1;
            if !recent {
                entry.delete(gl);
            }
            recent
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        ctx: &RenderContext,
        description: &TerrainDescription,
        source: Arc<HeightmapData>,
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
        // The four detail maps' mean colors when the shell bound them to units
        // 1..4 for this draw, else `None`. Layers without textures — and a
        // textured terrain whose maps have not streamed in — render the flat
        // band colors.
        detail_bound: Option<[[f32; 3]; 4]>,
    ) {
        if source.width < 2 || source.height < 2 {
            return;
        }

        self.ensure_program(ctx);
        let patch_key = PatchCacheKey {
            sample_width: source.width,
            sample_height: source.height,
            terrain_width: description.width,
            terrain_depth: description.depth,
            min_height: description.min_height,
            max_height: description.max_height,
            max_pixel_error: description.max_pixel_error,
            world: *world,
            camera_pos: ctx.lod_camera_pos,
            view_projections: ctx.lod_view_projections,
            frustum_count: ctx.lod_frustum_count,
            projection_scale: ctx.lod_projection_scale,
            viewport_height: ctx.viewport_height,
        };
        let geometry_index = self.ensure_geometry(ctx.gl, description, world, &patch_key);
        let (height_texture, macro_texture) = self.ensure_height_texture(
            ctx.gl,
            &description.heightmap,
            source.clone(),
            description.width,
            description.depth,
            description.max_height - description.min_height,
        );
        let geometry = &mut self.geometries[geometry_index];
        let gl = ctx.gl;

        if geometry.cache_key.as_ref() != Some(&patch_key) {
            select_patches_into(
                description,
                source.width,
                source.height,
                world,
                ctx.lod_camera_pos,
                &ctx.lod_view_projections[..ctx.lod_frustum_count],
                ctx.lod_projection_scale,
                ctx.viewport_height,
                &mut geometry.scratch_instances,
            );
            if geometry.uploaded_instances != geometry.scratch_instances {
                unsafe {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(geometry.instance_vbo));
                    let bytes = slice_bytes(&geometry.scratch_instances);
                    gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
                    crate::gpu_counters::gpu_counters().uploaded(bytes.len());
                    gl.bind_buffer(glow::ARRAY_BUFFER, None);
                }
                geometry
                    .uploaded_instances
                    .clone_from(&geometry.scratch_instances);
            }
            geometry.cache_key = Some(patch_key);
        }
        if geometry.uploaded_instances.is_empty() {
            return;
        }

        let program = self.program.as_ref().expect("terrain program initialized");
        let normal_matrix = terrain_normal_matrix(world);
        unsafe {
            let p = &program.program;
            let u = &program.uniforms;
            p.use_program(gl);
            p.set_uniform_matrix4(gl, &u.world, world);
            p.set_uniform_matrix3(gl, &u.normal_matrix, &normal_matrix);
            p.set_uniform_matrix4(gl, &u.view, view);
            p.set_uniform_matrix4(gl, &u.projection, projection);
            p.set_uniform_1i(gl, &u.heightmap, 0);
            p.set_uniform_vec3(
                gl,
                &u.terrain_size,
                &Vector3::new(
                    description.width,
                    description.depth,
                    description.max_height - description.min_height,
                ),
            );
            let (use_layers, low, high, rock, snow, snow_height) = match description.layers.as_ref()
            {
                Some(layers) => (
                    1,
                    layers.low,
                    layers.high,
                    layers.rock,
                    layers.snow,
                    layers.snow_height,
                ),
                None => (
                    0,
                    description.color,
                    description.color,
                    description.color,
                    description.color,
                    description.max_height,
                ),
            };
            p.set_uniform_1i(gl, &u.use_layers, use_layers);
            for (location, color) in [
                (&u.low_color, low),
                (&u.high_color, high),
                (&u.rock_color, rock),
                (&u.snow_color, snow),
            ] {
                p.set_uniform_vec3(gl, location, &Vector3::new(color[0], color[1], color[2]));
            }
            p.set_uniform_1f(gl, &u.min_terrain_height, description.min_height);
            p.set_uniform_1f(gl, &u.max_terrain_height, description.max_height);
            p.set_uniform_1f(gl, &u.snow_height, snow_height);
            p.set_uniform_1f(gl, &u.min_height, description.min_height);
            p.set_uniform_1f(
                gl,
                &u.skirt_depth,
                ((description.max_height - description.min_height) * 0.03).max(2.0),
            );
            p.set_uniform_vec3(
                gl,
                &u.color,
                &Vector3::new(
                    description.color[0],
                    description.color[1],
                    description.color[2],
                ),
            );
            let debug_mode = match ctx.debug_render_mode {
                DebugRenderMode::Normals => 1,
                DebugRenderMode::Tangents => 2,
                DebugRenderMode::Default
                | DebugRenderMode::Transparent
                | DebugRenderMode::Physics => 0,
            };
            p.set_uniform_1i(gl, &u.debug_mode, debug_mode);
            gl.active_texture(glow::TEXTURE0 + MACRO_TEXTURE_UNIT as u32);
            gl.bind_texture(glow::TEXTURE_2D, Some(macro_texture));
            p.set_uniform_1i(gl, &u.macro_map, MACRO_TEXTURE_UNIT);
            let detail = description
                .textures
                .as_ref()
                .filter(|_| use_layers == 1 && detail_bound.is_some());
            p.set_uniform_1i(gl, &u.use_textures, detail.is_some() as i32);
            if let Some(textures) = detail {
                for (unit, location) in u.detail_textures.iter().enumerate() {
                    p.set_uniform_1i(gl, location, unit as i32 + DETAIL_TEXTURE_UNIT0);
                }
                p.set_uniform_1f(gl, &u.detail_tile, textures.tile_size);
                let span = description.width.max(description.depth);
                let fade_end = DETAIL_FADE_MAX_END.min(span * DETAIL_FADE_END_FRACTION);
                p.set_uniform_1f(gl, &u.detail_fade_start, fade_end * DETAIL_FADE_START_FRACTION);
                p.set_uniform_1f(gl, &u.detail_fade_end, fade_end);
                p.set_uniform_vec3(gl, &u.detail_camera_pos, &ctx.camera_pos);
                let averages = detail_bound.expect("checked by the filter above");
                for (location, average) in u.detail_averages.iter().zip(averages) {
                    p.set_uniform_vec3(
                        gl,
                        location,
                        &Vector3::new(average[0], average[1], average[2]),
                    );
                }
            }
            u.lighting.set(p, ctx, view);
            u.fog.set(p, gl, ctx.fog, &ctx.camera_pos);

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(height_texture));
            gl.bind_vertex_array(Some(geometry.vao));
            gl.draw_elements_instanced(
                glow::TRIANGLES,
                geometry.index_count,
                glow::UNSIGNED_SHORT,
                0,
                geometry.uploaded_instances.len() as i32,
            );
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        if matches!(
            ctx.debug_render_mode,
            DebugRenderMode::Default
                | DebugRenderMode::Transparent
                | DebugRenderMode::Physics
        ) {
            if let Some(grass) = description.grass.as_ref() {
                self.draw_grass(
                    ctx,
                    description,
                    grass,
                    height_texture,
                    world,
                    projection,
                    view,
                );
            }
        }
    }

    fn ensure_program(&mut self, ctx: &RenderContext) {
        if self.program.is_some() {
            return;
        }
        let vertex = Shader::build(
            ctx.gl,
            ShaderType::Vertex,
            VERTEX_SHADER_SOURCE,
            ctx.shader_version,
        );
        let fragment_source = format!(
            "{}\n{}\n{}",
            FOG_GLSL,
            lighting_glsl(),
            FRAGMENT_SHADER_SOURCE
        );
        let fragment = Shader::build(
            ctx.gl,
            ShaderType::Fragment,
            &fragment_source,
            ctx.shader_version,
        );
        let program = ShaderProgram::link(ctx.gl, &vertex, &fragment);
        let uniforms = TerrainUniforms {
            world: program.get_uniform_location(ctx.gl, "world"),
            normal_matrix: program.get_uniform_location(ctx.gl, "normalMatrix"),
            view: program.get_uniform_location(ctx.gl, "view"),
            projection: program.get_uniform_location(ctx.gl, "projection"),
            heightmap: program.get_uniform_location(ctx.gl, "heightmapTex"),
            terrain_size: program.get_uniform_location(ctx.gl, "terrainSize"),
            min_height: program.get_uniform_location(ctx.gl, "minHeight"),
            skirt_depth: program.get_uniform_location(ctx.gl, "skirtDepth"),
            color: program.get_uniform_location(ctx.gl, "terrainColor"),
            use_layers: program.get_uniform_location(ctx.gl, "useLayers"),
            low_color: program.get_uniform_location(ctx.gl, "lowColor"),
            high_color: program.get_uniform_location(ctx.gl, "highColor"),
            rock_color: program.get_uniform_location(ctx.gl, "rockColor"),
            snow_color: program.get_uniform_location(ctx.gl, "snowColor"),
            min_terrain_height: program.get_uniform_location(ctx.gl, "minTerrainHeight"),
            max_terrain_height: program.get_uniform_location(ctx.gl, "maxTerrainHeight"),
            snow_height: program.get_uniform_location(ctx.gl, "snowHeight"),
            debug_mode: program.get_uniform_location(ctx.gl, "debugMode"),
            macro_map: program.get_uniform_location(ctx.gl, "macroTex"),
            use_textures: program.get_uniform_location(ctx.gl, "useTextures"),
            detail_textures: [
                program.get_uniform_location(ctx.gl, "lowTex"),
                program.get_uniform_location(ctx.gl, "highTex"),
                program.get_uniform_location(ctx.gl, "rockTex"),
                program.get_uniform_location(ctx.gl, "snowTex"),
            ],
            detail_tile: program.get_uniform_location(ctx.gl, "detailTile"),
            detail_fade_start: program.get_uniform_location(ctx.gl, "detailFadeStart"),
            detail_fade_end: program.get_uniform_location(ctx.gl, "detailFadeEnd"),
            detail_camera_pos: program.get_uniform_location(ctx.gl, "detailCameraPos"),
            detail_averages: std::array::from_fn(|i| {
                program.get_uniform_location(ctx.gl, &format!("detailAverage[{i}]"))
            }),
            lighting: LightingUniforms::get(&program, ctx.gl),
            fog: FogUniforms::get(&program, ctx.gl),
        };
        self.program = Some(TerrainProgram { program, uniforms });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_grass(
        &mut self,
        ctx: &RenderContext,
        description: &TerrainDescription,
        grass: &TerrainGrass,
        height_texture: glow::Texture,
        world: &Matrix4<f32>,
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        self.ensure_grass_program(ctx);
        let cache_key = grass_cache_key(description, grass, world, ctx.lod_camera_pos);
        let geometry_index = self.ensure_grass_geometry(ctx.gl, description, world, &cache_key);
        let geometry = &mut self.grass_geometries[geometry_index];
        if geometry.cache_key.as_ref() != Some(&cache_key) {
            select_grass_instances_into(
                description,
                grass,
                &cache_key,
                &mut geometry.scratch_instances,
            );
            if geometry.uploaded_instances != geometry.scratch_instances {
                unsafe {
                    ctx.gl
                        .bind_buffer(glow::ARRAY_BUFFER, Some(geometry.instance_vbo));
                    let bytes = slice_bytes(&geometry.scratch_instances);
                    ctx.gl
                        .buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
                    crate::gpu_counters::gpu_counters().uploaded(bytes.len());
                    ctx.gl.bind_buffer(glow::ARRAY_BUFFER, None);
                }
                geometry
                    .uploaded_instances
                    .clone_from(&geometry.scratch_instances);
            }
            geometry.cache_key = Some(cache_key);
        }
        if geometry.uploaded_instances.is_empty() {
            return;
        }

        let program = self
            .grass_program
            .as_ref()
            .expect("grass program initialized");
        let gl = ctx.gl;
        let snow_height = description
            .layers
            .as_ref()
            .map(|layers| layers.snow_height)
            .unwrap_or(description.max_height + grass.blade_height);
        unsafe {
            let p = &program.program;
            let u = &program.uniforms;
            p.use_program(gl);
            p.set_uniform_matrix4(gl, &u.world, world);
            p.set_uniform_matrix4(gl, &u.view, view);
            p.set_uniform_matrix4(gl, &u.projection, projection);
            p.set_uniform_1i(gl, &u.heightmap, 0);
            p.set_uniform_vec3(
                gl,
                &u.terrain_size,
                &Vector3::new(
                    description.width,
                    description.depth,
                    description.max_height - description.min_height,
                ),
            );
            p.set_uniform_1f(gl, &u.min_height, description.min_height);
            p.set_uniform_1f(gl, &u.snow_height, snow_height);
            p.set_uniform_1f(gl, &u.blade_height, grass.blade_height);
            p.set_uniform_1f(gl, &u.time, ctx.frame_time.tts);
            p.set_uniform_vec3(
                gl,
                &u.color,
                &Vector3::new(grass.color[0], grass.color[1], grass.color[2]),
            );
            u.fog.set(p, gl, ctx.fog, &ctx.camera_pos);

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(height_texture));
            gl.bind_vertex_array(Some(geometry.vao));
            gl.draw_arrays_instanced(
                glow::TRIANGLES,
                0,
                geometry.vertex_count,
                geometry.uploaded_instances.len() as i32,
            );
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    fn ensure_grass_program(&mut self, ctx: &RenderContext) {
        if self.grass_program.is_some() {
            return;
        }
        let vertex = Shader::build(
            ctx.gl,
            ShaderType::Vertex,
            GRASS_VERTEX_SHADER_SOURCE,
            ctx.shader_version,
        );
        let fragment_source = format!("{}\n{}", FOG_GLSL, GRASS_FRAGMENT_SHADER_SOURCE);
        let fragment = Shader::build(
            ctx.gl,
            ShaderType::Fragment,
            &fragment_source,
            ctx.shader_version,
        );
        let program = ShaderProgram::link(ctx.gl, &vertex, &fragment);
        let uniforms = GrassUniforms {
            world: program.get_uniform_location(ctx.gl, "world"),
            view: program.get_uniform_location(ctx.gl, "view"),
            projection: program.get_uniform_location(ctx.gl, "projection"),
            heightmap: program.get_uniform_location(ctx.gl, "heightmapTex"),
            terrain_size: program.get_uniform_location(ctx.gl, "terrainSize"),
            min_height: program.get_uniform_location(ctx.gl, "minHeight"),
            snow_height: program.get_uniform_location(ctx.gl, "snowHeight"),
            blade_height: program.get_uniform_location(ctx.gl, "bladeHeight"),
            time: program.get_uniform_location(ctx.gl, "time"),
            color: program.get_uniform_location(ctx.gl, "grassColor"),
            fog: FogUniforms::get(&program, ctx.gl),
        };
        self.grass_program = Some(GrassProgram { program, uniforms });
    }

    fn ensure_geometry(
        &mut self,
        gl: &glow::Context,
        description: &TerrainDescription,
        world: &Matrix4<f32>,
        patch_key: &PatchCacheKey,
    ) -> usize {
        if let Some(index) = self.geometries.iter().position(|geometry| {
            geometry.key.matches(description, world)
                && geometry.cache_key.as_ref() == Some(patch_key)
        }) {
            self.geometries[index].last_used_frame = self.current_frame;
            return index;
        }
        let key = TerrainInstanceKey::new(description, world);
        if let Some(index) = self
            .geometries
            .iter()
            .position(|geometry| geometry.last_used_frame != self.current_frame)
        {
            let geometry = &mut self.geometries[index];
            geometry.key = key;
            geometry.last_used_frame = self.current_frame;
            geometry.cache_key = None;
            return index;
        }
        let (vertices, indices) = build_patch_mesh();
        unsafe {
            let counters = crate::gpu_counters::gpu_counters();
            let vao = gl.create_vertex_array().expect("terrain VAO");
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));

            let grid_vbo = gl.create_buffer().expect("terrain grid VBO");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_vbo));
            let vertex_bytes = slice_bytes(&vertices);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::STATIC_DRAW);
            counters.uploaded(vertex_bytes.len());
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);

            let instance_vbo = gl.create_buffer().expect("terrain instance VBO");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(instance_vbo));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                (MAX_INSTANCES * std::mem::size_of::<[f32; 4]>()) as i32,
                glow::DYNAMIC_DRAW,
            );
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 16, 0);
            gl.vertex_attrib_divisor(1, 1);

            let index_ebo = gl.create_buffer().expect("terrain index EBO");
            counters.buffer_created();
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_ebo));
            let index_bytes = slice_bytes(&indices);
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
            counters.uploaded(index_bytes.len());

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
            self.geometries.push(TerrainGeometry {
                key,
                vao,
                _grid_vbo: grid_vbo,
                _index_ebo: index_ebo,
                instance_vbo,
                index_count: indices.len() as i32,
                last_used_frame: self.current_frame,
                cache_key: None,
                scratch_instances: Vec::new(),
                uploaded_instances: Vec::new(),
            });
        }
        self.geometries.len() - 1
    }

    fn ensure_grass_geometry(
        &mut self,
        gl: &glow::Context,
        description: &TerrainDescription,
        world: &Matrix4<f32>,
        grass_key: &GrassCacheKey,
    ) -> usize {
        if let Some(index) = self.grass_geometries.iter().position(|geometry| {
            geometry.key.matches(description, world)
                && geometry.cache_key.as_ref() == Some(grass_key)
        }) {
            self.grass_geometries[index].last_used_frame = self.current_frame;
            return index;
        }
        let key = TerrainInstanceKey::new(description, world);
        if let Some(index) = self
            .grass_geometries
            .iter()
            .position(|geometry| geometry.last_used_frame != self.current_frame)
        {
            let geometry = &mut self.grass_geometries[index];
            geometry.key = key;
            geometry.last_used_frame = self.current_frame;
            geometry.cache_key = None;
            return index;
        }
        let vertices = build_grass_cluster();
        unsafe {
            let counters = crate::gpu_counters::gpu_counters();
            let vao = gl.create_vertex_array().expect("grass VAO");
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));

            let blade_vbo = gl.create_buffer().expect("grass blade VBO");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(blade_vbo));
            let vertex_bytes = slice_bytes(&vertices);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::STATIC_DRAW);
            counters.uploaded(vertex_bytes.len());
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);

            let instance_vbo = gl.create_buffer().expect("grass instance VBO");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(instance_vbo));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                (MAX_GRASS_INSTANCES * std::mem::size_of::<[f32; 4]>()) as i32,
                glow::DYNAMIC_DRAW,
            );
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 16, 0);
            gl.vertex_attrib_divisor(1, 1);

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            self.grass_geometries.push(GrassGeometry {
                key,
                vao,
                _blade_vbo: blade_vbo,
                instance_vbo,
                vertex_count: vertices.len() as i32,
                last_used_frame: self.current_frame,
                cache_key: None,
                scratch_instances: Vec::new(),
                uploaded_instances: Vec::new(),
            });
        }
        self.grass_geometries.len() - 1
    }

    fn ensure_height_texture(
        &mut self,
        gl: &glow::Context,
        locator: &str,
        source: Arc<HeightmapData>,
        terrain_width: f32,
        terrain_depth: f32,
        height_range: f32,
    ) -> (glow::Texture, glow::Texture) {
        // The occlusion bake depends on the world span and vertical range the
        // `.fun` declares, not just on the image — so those are part of the key,
        // not merely checked against it. Keying by locator alone and comparing
        // would make two terrains that share a heightmap at different scales
        // evict and rebake each other on every single draw.
        //
        // The cost is a duplicated height texture in that (rare) case, which is
        // the cheaper half of the trade: it is 2 MB, against a rebake per frame.
        //
        // Float bits, so the comparison is exact and the key stays hashable.
        let key = (
            locator.to_string(),
            [
                terrain_width.to_bits(),
                terrain_depth.to_bits(),
                height_range.to_bits(),
            ],
        );
        if let Some(entry) = self
            .height_textures
            .get_mut(&key)
            .filter(|entry| entry.revision == source.revision)
        {
            crate::gpu_counters::gpu_counters().cache_hit();
            entry.last_used_frame = self.current_frame;
            return (entry.texture, entry.macro_texture);
        }

        let counters = crate::gpu_counters::gpu_counters();
        counters.cache_miss();
        if let Some(stale) = self.height_textures.remove(&key) {
            stale.delete(gl);
        }

        let max_size = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(2) as u32;
        let resized;
        let (samples, width, height) = if source.width > max_size || source.height > max_size {
            let fitted = fit_heightmap(&source, max_size);
            eprintln!(
                "[terrain] \"{locator}\" is {}x{}, above this GPU's {}px texture limit; \
rendering a {}x{} height copy",
                source.width, source.height, max_size, fitted.1, fitted.2
            );
            resized = Some(fitted);
            let fitted = resized.as_ref().unwrap();
            (fitted.0.as_slice(), fitted.1, fitted.2)
        } else {
            resized = None;
            (source.samples.as_slice(), source.width, source.height)
        };
        let _keep_resized_alive = &resized;

        let texture = unsafe {
            let texture = gl.create_texture().expect("terrain height texture");
            counters.texture_created();
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            let bytes = slice_bytes(samples);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R16UI as i32,
                width as i32,
                height as i32,
                0,
                glow::RED_INTEGER,
                glow::UNSIGNED_SHORT,
                glow::PixelUnpackData::Slice(Some(bytes)),
            );
            counters.uploaded(bytes.len());
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
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        };
        // Baked from the same samples, on the same cache miss — so it can
        // never disagree with the height texture it accompanies.
        let (ao, macro_w, macro_h) = bake_macro_ao(
            samples,
            width,
            height,
            terrain_width,
            terrain_depth,
            height_range,
        );
        let macro_texture = unsafe {
            let macro_texture = gl.create_texture().expect("terrain macro texture");
            counters.texture_created();
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(macro_texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                macro_w as i32,
                macro_h as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&ao)),
            );
            counters.uploaded(ao.len());
            // Mipped and linear: this is sampled at every distance, and it is
            // the one terrain texture that must stay smooth as it minifies.
            gl.generate_mipmap(glow::TEXTURE_2D);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            macro_texture
        };
        self.height_textures.insert(
            key,
            HeightTexture {
                revision: source.revision,
                texture,
                macro_texture,
                last_used_frame: self.current_frame,
            },
        );
        (texture, macro_texture)
    }
}

/// Resolution of the baked macro map.
///
/// Deliberately coarser than the heightmap: occlusion from surrounding terrain
/// is a low-frequency signal, so baking it at source resolution costs time and
/// memory for detail that is not there. 256² over a 4 km world is one texel per
/// ~16 m, which resolves valleys and ridge shadowing without resolving noise.
///
/// Measured against 512²: the rendered frame differs by 0.2/255 on average
/// while the bake costs a quarter as much, so the extra resolution was paying
/// for nothing.
const MACRO_SIZE: u32 = 256;
/// Horizon directions sampled per texel. Eight leaves visible eight-pointed
/// star artifacts radiating from peaks — the rays are far enough apart that a
/// ridge occludes along one and not its neighbour.
const AO_DIRECTIONS: usize = 16;
/// Steps taken along each direction, in macro texels.
const AO_STEPS: usize = 16;
/// How far the search reaches, as a fraction of the terrain's longest side.
/// Beyond this, terrain occludes by being over the horizon rather than by
/// being nearby, which the fog already expresses.
const AO_REACH: f32 = 0.06;

/// Bake ambient occlusion from the heightfield.
///
/// For each texel this walks outward in `AO_DIRECTIONS` and tracks the highest
/// elevation angle any sample subtends — the horizon angle. A texel at the
/// bottom of a valley sees terrain rising all around it and darkens; a ridge
/// sees sky and stays bright.
///
/// This is what carries the FAR field. Tiled detail maps minify to their own
/// average by construction, so past a few hundred metres they contribute
/// nothing; occlusion is unique per world position and survives at any
/// distance. It is the reason distant terrain reads as landform rather than
/// as a smooth gradient.
///
/// The source is box-reduced to `MACRO_SIZE²` before marching. That is not an
/// optimisation detail: point-sampling one source texel per macro texel would
/// let a single noisy sample stand in for the 16-256 it represents, and it is
/// what makes the cost depend only on `MACRO_SIZE`, not on how large a
/// heightmap the game happens to ship.
///
/// `width_world`/`depth_world` are the terrain's world span. They are taken
/// separately because a rectangular terrain has different metres-per-texel on
/// each axis, and the horizon angle is only a real angle if the horizontal run
/// is measured in metres. The `world` matrix's own scale is NOT considered —
/// occlusion is baked in the terrain's local space.
/// Macro-map dimensions for a terrain of this world span.
///
/// The long axis gets `MACRO_SIZE`; the short axis is scaled so a macro texel
/// is SQUARE IN WORLD SPACE. That is what keeps the bake isotropic: on a square
/// grid stretched over a 4000x1000 terrain, a step of one texel is 15.6 m one
/// way and 3.9 m the other, so a fixed step count either overshoots features on
/// one axis or oversamples on the other.
fn macro_dims(width_world: f32, depth_world: f32) -> (u32, u32) {
    let long = width_world.max(depth_world);
    // Degenerate or non-finite spans fall back to the square grid rather than
    // producing a zero/NaN dimension.
    if !long.is_finite() || long <= 0.0 {
        return (MACRO_SIZE, MACRO_SIZE);
    }
    let scale = |v: f32| {
        ((v / long * MACRO_SIZE as f32).round() as u32).clamp(8, MACRO_SIZE)
    };
    (scale(width_world), scale(depth_world))
}

fn bake_macro_ao(
    samples: &[u16],
    width: u32,
    height: u32,
    width_world: f32,
    depth_world: f32,
    height_range: f32,
) -> (Vec<u8>, u32, u32) {
    let (mw, mh) = macro_dims(width_world, depth_world);
    let open = || (vec![255u8; (mw * mh) as usize], mw, mh);

    // Ground with no relief occludes nothing, so there is nothing to bake. This
    // is a correctness shortcut that happens to pay for itself: the 2x2
    // placeholder served while the real heightmap streams is constant, and
    // without this it would pay the full fixed cost on every load before the
    // real map arrives and pays it again.
    //
    // Keyed on the SAMPLES rather than on the dimensions: a small heightmap is
    // still a heightmap, and `width < 4` would silently drop occlusion from a
    // real one.
    if width == 0 || height == 0 || samples.iter().all(|&s| s == samples[0]) {
        return open();
    }
    let metres_per_texel = width_world.max(depth_world) / MACRO_SIZE as f32;
    if !metres_per_texel.is_finite() || metres_per_texel <= 0.0 {
        return open();
    }

    // Average each macro texel's footprint in the source. Expressed as a
    // footprint rather than a scatter-add so it is also correct when the
    // source is SMALLER than the macro grid — scattering leaves those macro
    // texels with no contributor at all, and a zero-filled hole reads as a
    // pit that occludes everything around it.
    let (sw, sh) = (width as usize, height as usize);
    let (nw, nh) = (mw as usize, mh as usize);
    let mut grid = vec![0f32; nw * nh];
    for mz in 0..nh {
        let z0 = mz * sh / nh;
        let z1 = ((mz + 1) * sh / nh).max(z0 + 1);
        for mx in 0..nw {
            let x0 = mx * sw / nw;
            let x1 = ((mx + 1) * sw / nw).max(x0 + 1);
            let mut sum = 0f64;
            for z in z0..z1 {
                for x in x0..x1 {
                    sum += samples[z * sw + x] as f64;
                }
            }
            let mean = sum / ((z1 - z0) * (x1 - x0)) as f64;
            grid[mz * nw + mx] = (mean / 65535.0) as f32 * height_range;
        }
    }

    // Texels are square in world space, so the reach is one distance in texels
    // and the horizontal run is that distance times a single scale.
    let reach = (AO_REACH * width_world.max(depth_world) / metres_per_texel).max(2.0);

    // Hoisted: these are the same 16 directions for every texel.
    let dirs: Vec<(f32, f32)> = (0..AO_DIRECTIONS)
        .map(|dir| {
            let angle = dir as f32 / AO_DIRECTIONS as f32 * std::f32::consts::TAU;
            (angle.cos(), angle.sin())
        })
        .collect();

    let (iw, ih) = (nw as i32, nh as i32);
    let mut out = vec![0u8; nw * nh];
    for mz in 0..ih {
        for mx in 0..iw {
            let here = grid[(mz * iw + mx) as usize];
            let mut openness = 0.0f32;
            for &(dx, dz) in &dirs {
                let mut max_tan = 0.0f32;
                for step in 1..=AO_STEPS {
                    let t = step as f32 / AO_STEPS as f32 * reach;
                    let x = mx + (dx * t) as i32;
                    let z = mz + (dz * t) as i32;
                    // Rays STOP at the edge rather than clamping. A clamped ray
                    // slides along the border and keeps sampling it, so a peak
                    // in one corner would occlude the whole edge beside it.
                    if x < 0 || x >= iw || z < 0 || z >= ih {
                        break;
                    }
                    let rise = grid[(z * iw + x) as usize] - here;
                    if rise > 0.0 {
                        // Run converted to metres, so this is a real tangent.
                        max_tan = max_tan.max(rise / (t * metres_per_texel));
                    }
                }
                // atan gives the horizon angle; the visible sky fraction in
                // this direction is what is left above it.
                openness += 1.0 - (max_tan.atan() / std::f32::consts::FRAC_PI_2);
            }
            let ao = (openness / AO_DIRECTIONS as f32).clamp(0.0, 1.0);
            out[(mz * iw + mx) as usize] = (ao * 255.0) as u8;
        }
    }
    (out, mw, mh)
}

fn build_grass_cluster() -> Vec<[f32; 3]> {
    let mut vertices = Vec::with_capacity(18);
    for angle in [
        0.0f32,
        std::f32::consts::FRAC_PI_3,
        std::f32::consts::FRAC_PI_3 * 2.0,
    ] {
        let (sin, cos) = angle.sin_cos();
        let left = [-cos * 0.09, 0.0, sin * 0.09];
        let top = [0.0, 1.0, 0.0];
        let right = [cos * 0.09, 0.0, -sin * 0.09];
        vertices.extend_from_slice(&[left, top, right, right, top, left]);
    }
    vertices
}

fn build_patch_mesh() -> (Vec<[f32; 3]>, Vec<u16>) {
    let side = PATCH_QUADS + 1;
    let mut vertices = Vec::with_capacity((side * side + side * 4) as usize);
    for z in 0..side {
        for x in 0..side {
            vertices.push([
                x as f32 / PATCH_QUADS as f32,
                z as f32 / PATCH_QUADS as f32,
                0.0,
            ]);
        }
    }

    let mut indices = Vec::with_capacity((PATCH_QUADS * PATCH_QUADS * 6 + side * 24) as usize);
    for z in 0..PATCH_QUADS {
        for x in 0..PATCH_QUADS {
            let a = (z * side + x) as u16;
            let b = a + 1;
            let c = ((z + 1) * side + x) as u16;
            let d = c + 1;
            // Counter-clockwise from +Y in the engine's XZ ground plane.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    // One ordered loop around the patch. Duplicated vertices get lowered in
    // the shader; triangles connect them to the main border as a vertical
    // skirt, masking coarse/fine T-junctions.
    let mut perimeter = Vec::with_capacity((PATCH_QUADS * 4) as usize);
    for x in 0..side {
        perimeter.push(x as u16);
    }
    for z in 1..side {
        perimeter.push((z * side + PATCH_QUADS) as u16);
    }
    for x in (0..PATCH_QUADS).rev() {
        perimeter.push((PATCH_QUADS * side + x) as u16);
    }
    for z in (1..PATCH_QUADS).rev() {
        perimeter.push((z * side) as u16);
    }
    let skirt_start = vertices.len() as u16;
    for main in &perimeter {
        let mut vertex = vertices[*main as usize];
        vertex[2] = 1.0;
        vertices.push(vertex);
    }
    for i in 0..perimeter.len() {
        let next = (i + 1) % perimeter.len();
        let a = perimeter[i];
        let b = perimeter[next];
        let sa = skirt_start + i as u16;
        let sb = skirt_start + next as u16;
        indices.extend_from_slice(&[a, sa, b, b, sa, sb]);
    }
    (vertices, indices)
}

fn grass_cache_key(
    description: &TerrainDescription,
    grass: &TerrainGrass,
    world: &Matrix4<f32>,
    camera_world: Vector3<f32>,
) -> GrassCacheKey {
    let inverse = world.invert().unwrap_or_else(Matrix4::identity);
    let local4 = inverse * Vector4::new(camera_world.x, camera_world.y, camera_world.z, 1.0);
    let camera_local = local4.truncate() / local4.w.max(1e-6);
    let camera_cell_x = (camera_local.x / grass.spacing).floor() as i32;
    let camera_cell_z = (camera_local.z / grass.spacing).floor() as i32;
    GrassCacheKey {
        camera_cell_x,
        camera_cell_z,
        terrain_width: description.width,
        terrain_depth: description.depth,
        grass: grass.clone(),
    }
}

fn select_grass_instances_into(
    description: &TerrainDescription,
    grass: &TerrainGrass,
    key: &GrassCacheKey,
    instances: &mut Vec<[f32; 4]>,
) {
    instances.clear();
    let radius_cells = (grass.distance / grass.spacing).ceil().max(0.0) as i32;
    let radius_cells_f64 = f64::from(radius_cells);
    let estimated = std::f64::consts::PI * radius_cells_f64 * radius_cells_f64;
    let stride = (estimated / MAX_GRASS_INSTANCES as f64)
        .sqrt()
        .ceil()
        .max(1.0) as i32;
    let anchor_x = key.camera_cell_x as f32 * grass.spacing;
    let anchor_z = key.camera_cell_z as f32 * grass.spacing;
    let half_width = description.width * 0.5;
    let half_depth = description.depth * 0.5;
    let radius_squared = grass.distance * grass.distance;
    let stride_f64 = f64::from(stride);
    let desired_capacity = ((estimated / (stride_f64 * stride_f64)).ceil() as usize)
        .min(MAX_GRASS_INSTANCES);
    if instances.capacity() < desired_capacity {
        instances.reserve(desired_capacity);
    }

    let min_cell_x = key.camera_cell_x.saturating_sub(radius_cells);
    let max_cell_x = key.camera_cell_x.saturating_add(radius_cells);
    let min_cell_z = key.camera_cell_z.saturating_sub(radius_cells);
    let max_cell_z = key.camera_cell_z.saturating_add(radius_cells);
    let stride_i64 = i64::from(stride);
    let min_cell_x_i64 = i64::from(min_cell_x);
    let min_cell_z_i64 = i64::from(min_cell_z);
    let first_cell_x = min_cell_x_i64 + (-min_cell_x_i64).rem_euclid(stride_i64);
    let first_cell_z = min_cell_z_i64 + (-min_cell_z_i64).rem_euclid(stride_i64);

    for cell_z in (first_cell_z..=i64::from(max_cell_z)).step_by(stride as usize) {
        for cell_x in (first_cell_x..=i64::from(max_cell_x)).step_by(stride as usize) {
            let cell_x = cell_x as i32;
            let cell_z = cell_z as i32;
            let hash = grass_hash(cell_x, cell_z);
            let jitter_x = hash_unit(hash) - 0.5;
            let jitter_z = hash_unit(hash.rotate_left(13)) - 0.5;
            let x = cell_x as f32 * grass.spacing + jitter_x * grass.spacing * 0.76;
            let z = cell_z as f32 * grass.spacing + jitter_z * grass.spacing * 0.76;
            let distance_squared = (x - anchor_x).powi(2) + (z - anchor_z).powi(2);
            if distance_squared > radius_squared
                || x < -half_width
                || x > half_width
                || z < -half_depth
                || z > half_depth
            {
                continue;
            }
            let rotation = hash_unit(hash.rotate_left(7)) * std::f32::consts::TAU;
            let scale = 0.75 + hash_unit(hash.rotate_left(23)) * 0.5;
            instances.push([x, z, rotation, scale]);
            if instances.len() == MAX_GRASS_INSTANCES {
                return;
            }
        }
    }
}

#[cfg(test)]
fn select_grass_instances(
    description: &TerrainDescription,
    grass: &TerrainGrass,
    world: &Matrix4<f32>,
    camera_world: Vector3<f32>,
) -> (GrassCacheKey, Vec<[f32; 4]>) {
    let key = grass_cache_key(description, grass, world, camera_world);
    let mut instances = Vec::new();
    select_grass_instances_into(description, grass, &key, &mut instances);
    (key, instances)
}

fn grass_hash(x: i32, z: i32) -> u32 {
    let mut value =
        (x as u32).wrapping_mul(0x9e37_79b1) ^ (z as u32).wrapping_mul(0x85eb_ca77) ^ 0xc2b2_ae3d;
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn hash_unit(hash: u32) -> f32 {
    (hash & 0x00ff_ffff) as f32 / 0x0100_0000 as f32
}

#[derive(Clone, Copy)]
struct Plane {
    normal: Vector3<f32>,
    d: f32,
}

impl Plane {
    fn normalized(v: Vector4<f32>) -> Self {
        let normal = v.truncate();
        let length = normal.magnitude().max(1e-6);
        Self {
            normal: normal / length,
            d: v.w / length,
        }
    }

    fn excludes_sphere(self, center: Vector3<f32>, radius: f32) -> bool {
        self.normal.dot(center) + self.d < -radius
    }
}

fn frustum_planes(m: &Matrix4<f32>) -> [Plane; 6] {
    // cgmath stores columns; assemble rows for the standard row4 ± rowN
    // extraction from a world→clip matrix.
    let r0 = Vector4::new(m.x.x, m.y.x, m.z.x, m.w.x);
    let r1 = Vector4::new(m.x.y, m.y.y, m.z.y, m.w.y);
    let r2 = Vector4::new(m.x.z, m.y.z, m.z.z, m.w.z);
    let r3 = Vector4::new(m.x.w, m.y.w, m.z.w, m.w.w);
    [
        Plane::normalized(r3 + r0),
        Plane::normalized(r3 - r0),
        Plane::normalized(r3 + r1),
        Plane::normalized(r3 - r1),
        Plane::normalized(r3 + r2),
        Plane::normalized(r3 - r2),
    ]
}

#[allow(clippy::too_many_arguments)]
fn select_patches_into(
    description: &TerrainDescription,
    sample_width: u32,
    sample_height: u32,
    world: &Matrix4<f32>,
    camera_world: Vector3<f32>,
    view_projections: &[Matrix4<f32>],
    projection_scale: f32,
    viewport_height: f32,
    patches: &mut Vec<[f32; 4]>,
) {
    patches.clear();
    let inverse = world.invert().unwrap_or_else(Matrix4::identity);
    let local4 = inverse * Vector4::new(camera_world.x, camera_world.y, camera_world.z, 1.0);
    let camera_local = local4.truncate() / local4.w.max(1e-6);
    let scales = [
        world.x.truncate().magnitude(),
        world.y.truncate().magnitude(),
        world.z.truncate().magnitude(),
    ];
    let min_scale = scales
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min)
        .max(1e-4);
    let max_scale = scales.iter().copied().fold(0.0, f32::max).max(1e-4);
    debug_assert!(!view_projections.is_empty());
    debug_assert!(view_projections.len() <= 2);
    let frusta = [
        frustum_planes(&view_projections[0]),
        frustum_planes(view_projections.get(1).unwrap_or(&view_projections[0])),
    ];
    let frusta = &frusta[..view_projections.len()];
    let max_level = max_quadtree_level(sample_width, sample_height);
    // A quadtree begins with one leaf; every split atomically replaces one
    // leaf with four (+3). Reserving split tokens up front guarantees the
    // traversal can never outgrow the fixed instance VBO, even while sibling
    // branches are still pending on the recursion stack.
    let mut remaining_splits = (MAX_INSTANCES - 1) / 3;

    #[allow(clippy::too_many_arguments)]
    fn visit(
        description: &TerrainDescription,
        world: &Matrix4<f32>,
        camera_local: Vector3<f32>,
        frusta: &[[Plane; 6]],
        min_scale: f32,
        max_scale: f32,
        projection_scale: f32,
        viewport_height: f32,
        max_level: u32,
        level: u32,
        u: f32,
        v: f32,
        scale: f32,
        remaining_splits: &mut usize,
        out: &mut Vec<[f32; 4]>,
    ) {
        let half_width = description.width * scale * 0.5;
        let half_depth = description.depth * scale * 0.5;
        let center_local = Vector3::new(
            (u + scale * 0.5 - 0.5) * description.width,
            (description.min_height + description.max_height) * 0.5,
            (v + scale * 0.5 - 0.5) * description.depth,
        );
        let center4 = world * Vector4::new(center_local.x, center_local.y, center_local.z, 1.0);
        let center_world = center4.truncate() / center4.w.max(1e-6);
        let half_height = (description.max_height - description.min_height) * 0.5;
        let radius =
            transformed_aabb_radius(world, Vector3::new(half_width, half_height, half_depth));
        if frusta.iter().all(|planes| {
            planes
                .iter()
                .any(|plane| plane.excludes_sphere(center_world, radius))
        }) {
            return;
        }

        let min = Vector3::new(
            center_local.x - half_width,
            description.min_height,
            center_local.z - half_depth,
        );
        let max = Vector3::new(
            center_local.x + half_width,
            description.max_height,
            center_local.z + half_depth,
        );
        let dx = (min.x - camera_local.x).max(0.0) + (camera_local.x - max.x).max(0.0);
        let dy = (min.y - camera_local.y).max(0.0) + (camera_local.y - max.y).max(0.0);
        let dz = (min.z - camera_local.z).max(0.0) + (camera_local.z - max.z).max(0.0);
        let distance = (dx * dx + dy * dy + dz * dz).sqrt() * min_scale;
        let vertex_spacing =
            description.width.max(description.depth) * scale * max_scale / PATCH_QUADS as f32;
        let projected_spacing =
            vertex_spacing * projection_scale * viewport_height * 0.5 / distance.max(0.5);
        let should_split = projected_spacing > description.max_pixel_error
            && level < max_level
            && *remaining_splits > 0;
        if should_split {
            *remaining_splits -= 1;
            let child = scale * 0.5;
            visit(
                description,
                world,
                camera_local,
                frusta,
                min_scale,
                max_scale,
                projection_scale,
                viewport_height,
                max_level,
                level + 1,
                u,
                v,
                child,
                remaining_splits,
                out,
            );
            visit(
                description,
                world,
                camera_local,
                frusta,
                min_scale,
                max_scale,
                projection_scale,
                viewport_height,
                max_level,
                level + 1,
                u + child,
                v,
                child,
                remaining_splits,
                out,
            );
            visit(
                description,
                world,
                camera_local,
                frusta,
                min_scale,
                max_scale,
                projection_scale,
                viewport_height,
                max_level,
                level + 1,
                u,
                v + child,
                child,
                remaining_splits,
                out,
            );
            visit(
                description,
                world,
                camera_local,
                frusta,
                min_scale,
                max_scale,
                projection_scale,
                viewport_height,
                max_level,
                level + 1,
                u + child,
                v + child,
                child,
                remaining_splits,
                out,
            );
        } else {
            out.push([u, v, scale, scale]);
        }
    }

    visit(
        description,
        world,
        camera_local,
        frusta,
        min_scale,
        max_scale,
        projection_scale,
        viewport_height,
        max_level,
        0,
        0.0,
        0.0,
        1.0,
        &mut remaining_splits,
        patches,
    );
    debug_assert!(patches.len() <= MAX_INSTANCES);
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn select_patches(
    description: &TerrainDescription,
    sample_width: u32,
    sample_height: u32,
    world: &Matrix4<f32>,
    camera_world: Vector3<f32>,
    view_projection: &Matrix4<f32>,
    projection_scale: f32,
    viewport_height: f32,
) -> Vec<[f32; 4]> {
    let mut patches = Vec::new();
    select_patches_into(
        description,
        sample_width,
        sample_height,
        world,
        camera_world,
        std::slice::from_ref(view_projection),
        projection_scale,
        viewport_height,
        &mut patches,
    );
    patches
}

fn max_quadtree_level(width: u32, height: u32) -> u32 {
    let cells = width.saturating_sub(1).max(height.saturating_sub(1));
    let mut covered = PATCH_QUADS;
    let mut level = 0;
    while covered < cells && level < MAX_LEVEL {
        covered = covered.saturating_mul(2);
        level += 1;
    }
    level
}

fn fit_heightmap(source: &HeightmapData, max_size: u32) -> (Vec<u16>, u32, u32) {
    let ratio = (max_size as f64 / source.width as f64).min(max_size as f64 / source.height as f64);
    let width = ((source.width as f64 * ratio).floor() as u32).max(2);
    let height = ((source.height as f64 * ratio).floor() as u32).max(2);
    let mut samples = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let source_y = y as u64 * (source.height - 1) as u64 / (height - 1) as u64;
        for x in 0..width {
            let source_x = x as u64 * (source.width - 1) as u64 / (width - 1) as u64;
            samples.push(source.samples[(source_y * source.width as u64 + source_x) as usize]);
        }
    }
    (samples, width, height)
}

fn terrain_normal_matrix(world: &Matrix4<f32>) -> Matrix3<f32> {
    let linear = Matrix3::from_cols(world.x.truncate(), world.y.truncate(), world.z.truncate());
    linear
        .invert()
        .map(|matrix| matrix.transpose())
        .unwrap_or_else(Matrix3::identity)
}

fn transformed_aabb_radius(world: &Matrix4<f32>, half_extents: Vector3<f32>) -> f32 {
    let x = world.x.truncate();
    let y = world.y.truncate();
    let z = world.z.truncate();
    let extents = Vector3::new(
        x.x.abs() * half_extents.x + y.x.abs() * half_extents.y + z.x.abs() * half_extents.z,
        x.y.abs() * half_extents.x + y.y.abs() * half_extents.y + z.y.abs() * half_extents.z,
        x.z.abs() * half_extents.x + y.z.abs() * half_extents.y + z.z.abs() * half_extents.z,
    );
    extents.magnitude()
}

fn slice_bytes<T>(slice: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cgmath::{perspective, vec3, Deg, EuclideanSpace, Matrix4, Point3};

    use super::*;

    fn terrain() -> TerrainDescription {
        TerrainDescription::heightmap(
            "world.png".to_string(),
            vec![],
            4000.0,
            4000.0,
            -80.0,
            520.0,
        )
    }

    /// A cone rising from a flat plain: the ring at its foot is enclosed by
    /// rising ground, the plain far from it sees open sky, and the summit sees
    /// the most of all.
    fn cone_heightmap() -> HeightmapData {
        const N: u32 = 256;
        let centre = N as f32 / 2.0;
        let samples = (0..N * N)
            .map(|i| {
                let (x, z) = ((i % N) as f32, (i / N) as f32);
                let r = ((x - centre).powi(2) + (z - centre).powi(2)).sqrt();
                let h = (1.0 - r / 40.0).max(0.0);
                (h * 65535.0) as u16
            })
            .collect();
        HeightmapData {
            samples,
            width: N,
            height: N,
            revision: 1,
        }
    }

    fn ao_at(map: &[u8], x: u32, z: u32) -> f32 {
        map[(z * MACRO_SIZE + x) as usize] as f32 / 255.0
    }

    /// Square terrains keep the full MACRO_SIZE grid, so tests that use one can
    /// index it directly.
    fn bake_square(samples: &[u16], w: u32, h: u32, world: f32, range: f32) -> Vec<u8> {
        let (map, mw, mh) = bake_macro_ao(samples, w, h, world, world, range);
        assert_eq!((mw, mh), (MACRO_SIZE, MACRO_SIZE));
        map
    }

    #[test]
    fn baked_occlusion_darkens_ground_enclosed_by_rising_terrain() {
        let hm = cone_heightmap();
        let map = bake_square(&hm.samples, hm.width, hm.height, 4000.0, 600.0);
        let mid = MACRO_SIZE / 2;
        // 40/256 of the map is the cone's radius, so the foot sits just past it.
        let foot = mid + (MACRO_SIZE as f32 * 45.0 / 256.0) as u32;
        let summit = ao_at(&map, mid, mid);
        let foot = ao_at(&map, foot, mid);
        let plain = ao_at(&map, MACRO_SIZE - 4, mid);

        assert!(
            foot < plain,
            "ground at the cone's foot should be occluded by it: {foot} vs open plain {plain}"
        );
        assert!(
            summit > foot,
            "the summit sees more sky than its foot: {summit} vs {foot}"
        );
        assert!(
            plain > 0.98,
            "flat ground far from any relief is essentially unoccluded, got {plain}"
        );
    }

    #[test]
    fn baked_occlusion_skips_ground_with_no_relief() {
        // The early-out keys on the SAMPLES, not the dimensions, so this pins
        // the 2x2 streaming placeholder and a flat 4 km plain with one rule.
        for (samples, w, h) in [
            (vec![0u16; 4], 2u32, 2u32),
            (vec![1234u16; 64 * 64], 64, 64),
        ] {
            let map = bake_square(&samples, w, h, 4000.0, 600.0);
            assert!(map.iter().all(|&v| v == 255), "{w}x{h} should be fully open");
        }
    }

    #[test]
    fn a_small_heightmap_still_gets_occlusion() {
        // Guarding the early-out on `width < 4` silently threw away occlusion
        // for real heightmaps this size. Relief is relief at any resolution.
        let samples: Vec<u16> = (0..9).map(|i| if i == 4 { 60000 } else { 0 }).collect();
        let map = bake_square(&samples, 3, 3, 4000.0, 600.0);
        assert!(
            map.iter().any(|&v| v < 250),
            "a 3x3 map with a peak in the middle occludes something"
        );
    }

    /// How far a pillar's occlusion reaches, in METRES, walking away from it
    /// along one axis.
    ///
    /// The pillar is square IN WORLD SPACE — sized in source texels per axis so
    /// that it is the same physical object however the terrain is proportioned.
    /// Its shadow must then extend the same distance whichever way you walk.
    fn pillar_shadow_metres(width_world: f32, depth_world: f32, along_x: bool) -> f32 {
        const N: u32 = 512;
        const PILLAR_M: f32 = 200.0;
        let half_x = (PILLAR_M * N as f32 / width_world / 2.0).max(1.0) as u32;
        let half_z = (PILLAR_M * N as f32 / depth_world / 2.0).max(1.0) as u32;
        let samples: Vec<u16> = (0..N * N)
            .map(|i| {
                let (x, z) = (i % N, i / N);
                if x.abs_diff(N / 2) <= half_x && z.abs_diff(N / 2) <= half_z {
                    65535
                } else {
                    0
                }
            })
            .collect();
        let (map, mw, mh) = bake_macro_ao(&samples, N, N, width_world, depth_world, 600.0);

        let (nw, nh) = (mw as usize, mh as usize);
        // Square in world space, so one scale covers both axes.
        let metres_per_texel = width_world.max(depth_world) / MACRO_SIZE as f32;
        let (cx, cz) = (nw / 2, nh / 2);
        // Start clear of the pillar itself: standing on it, nothing occludes.
        let start = (PILLAR_M / 2.0 / metres_per_texel).ceil() as usize + 1;
        let limit = if along_x { nw - cx } else { nh - cz };
        for step in start..limit {
            let v = if along_x {
                map[cz * nw + (cx + step)]
            } else {
                map[(cz + step) * nw + cx]
            };
            if v >= 254 {
                return (step - start) as f32 * metres_per_texel;
            }
        }
        f32::MAX
    }

    #[test]
    fn occlusion_reaches_the_same_distance_on_both_axes_of_a_rectangular_terrain() {
        // The same physical pillar on a 4000x1000 terrain must cast the same
        // shadow in metres along X as along Z. A macro grid whose texels are not
        // square in world space breaks this: a fixed step count then covers
        // 15.6 m one way and 3.9 m the other.
        //
        // Note this is NOT the same as baking the terrain rotated and comparing
        // — the whole computation is symmetric under transposing (x, width) with
        // (z, depth), so that version passes even when the axes disagree.
        let x = pillar_shadow_metres(4000.0, 1000.0, true);
        let z = pillar_shadow_metres(4000.0, 1000.0, false);
        assert!(
            x.is_finite() && z.is_finite() && x > 0.0,
            "the pillar should cast a measurable shadow: x={x} z={z}"
        );
        let ratio = x.max(z) / x.min(z);
        assert!(
            ratio < 1.3,
            "occlusion reach is anisotropic: {x} m along X vs {z} m along Z"
        );
    }

    #[test]
    fn rays_stop_at_the_edge_instead_of_sliding_along_it() {
        // A single spike in the corner, and a probe placed so that NO ray in the
        // 16-direction set reaches it without first leaving the map: the two
        // directions bracketing the spike either run straight past it or exit
        // through the left edge well short of it.
        //
        // Clamping a departed ray pins it to x=0 and lets it keep walking DOWN
        // that column into the spike, inventing occlusion from terrain the
        // probe cannot see. Terminating the ray does not.
        const N: u32 = 256;
        let samples: Vec<u16> = (0..N * N)
            .map(|i| if i == 0 { 65535 } else { 0 })
            .collect();
        let map = bake_square(&samples, N, N, 4000.0, 600.0);

        let n = MACRO_SIZE as usize;
        let probe = map[12 * n + 2];
        assert!(
            probe >= 254,
            "a ray that left the map slid along the edge into the corner spike: {probe}"
        );
    }

    #[test]
    fn a_source_larger_than_the_macro_grid_is_averaged_not_point_sampled() {
        // The reduce exists so a big heightmap is not represented by one
        // arbitrary sample per macro texel.
        const N: u32 = 1024;
        let centre = N as f32 / 2.0;
        let samples: Vec<u16> = (0..N * N)
            .map(|i| {
                let (x, z) = ((i % N) as f32, (i / N) as f32);
                let r = ((x - centre).powi(2) + (z - centre).powi(2)).sqrt();
                ((1.0 - r / 160.0).max(0.0) * 65535.0) as u16
            })
            .collect();
        let map = bake_square(&samples, N, N, 4000.0, 600.0);
        let mid = MACRO_SIZE / 2;
        let summit = ao_at(&map, mid, mid);
        let foot = ao_at(&map, mid + (MACRO_SIZE as f32 * 45.0 / 256.0) as u32, mid);
        assert!(summit > foot, "summit {summit} should out-see its foot {foot}");
        assert!(ao_at(&map, MACRO_SIZE - 4, mid) > 0.98, "the far plain is open");
    }

    #[test]
    fn patch_mesh_is_one_grid_plus_a_closed_skirt() {
        let (vertices, indices) = build_patch_mesh();
        let surface_vertices = ((PATCH_QUADS + 1) * (PATCH_QUADS + 1)) as usize;
        assert_eq!(
            vertices.len(),
            surface_vertices + (PATCH_QUADS * 4) as usize
        );
        assert_eq!(
            indices.len(),
            (PATCH_QUADS * PATCH_QUADS * 6 + PATCH_QUADS * 4 * 6) as usize
        );
        assert!(vertices[..surface_vertices]
            .iter()
            .all(|vertex| vertex[2] == 0.0));
        assert!(vertices[surface_vertices..]
            .iter()
            .all(|vertex| vertex[2] == 1.0));
    }

    #[test]
    fn grass_cluster_is_three_double_sided_blades() {
        assert_eq!(build_grass_cluster().len(), 18);
    }

    #[test]
    fn grass_selection_is_deterministic_camera_local_and_bounded() {
        let description = terrain();
        let grass = TerrainGrass {
            spacing: 8.0,
            distance: 300.0,
            blade_height: 2.5,
            color: [0.1, 0.3, 0.08],
        };
        let world = Matrix4::from_translation(vec3(200.0, 0.0, -100.0));
        let camera = vec3(220.0, 50.0, -80.0);
        let (key_a, instances_a) = select_grass_instances(&description, &grass, &world, camera);
        let (key_b, instances_b) = select_grass_instances(&description, &grass, &world, camera);
        assert_eq!(key_a, key_b);
        assert_eq!(instances_a, instances_b);
        assert!(!instances_a.is_empty());
        assert!(instances_a.len() <= MAX_GRASS_INSTANCES);
        assert!(instances_a.iter().all(|instance| {
            instance[0].abs() <= description.width * 0.5
                && instance[1].abs() <= description.depth * 0.5
                && instance[3] >= 0.75
                && instance[3] <= 1.25
        }));
    }

    #[test]
    fn extreme_grass_density_never_exceeds_the_instance_budget() {
        let description = terrain();
        let grass = TerrainGrass {
            spacing: 0.1,
            distance: 10_000.0,
            blade_height: 1.0,
            color: [0.1, 0.3, 0.08],
        };
        let (_, instances) = select_grass_instances(
            &description,
            &grass,
            &Matrix4::identity(),
            Vector3::new(0.0, 20.0, 0.0),
        );
        assert!(instances.len() <= MAX_GRASS_INSTANCES);
    }

    #[test]
    fn extreme_valid_grass_range_does_not_overflow_decimation_math() {
        let description = terrain();
        let grass = TerrainGrass {
            spacing: 1.0,
            distance: f32::MAX,
            blade_height: 1.0,
            color: [0.1, 0.3, 0.08],
        };
        let (_, instances) = select_grass_instances(
            &description,
            &grass,
            &Matrix4::identity(),
            Vector3::new(0.0, 20.0, 0.0),
        );
        assert!(instances.len() <= MAX_GRASS_INSTANCES);
    }

    #[test]
    fn decimated_grass_uses_a_world_stable_sampling_lattice() {
        let description = terrain();
        let grass = TerrainGrass {
            spacing: 0.5,
            distance: 100.0,
            blade_height: 1.0,
            color: [0.1, 0.3, 0.08],
        };
        let radius_cells = (grass.distance / grass.spacing).ceil() as i32;
        let estimated =
            std::f64::consts::PI * f64::from(radius_cells) * f64::from(radius_cells);
        let stride = (estimated / MAX_GRASS_INSTANCES as f64).sqrt().ceil() as i32;
        assert!(stride > 1, "test must exercise decimated selection");

        let (_, first) = select_grass_instances(
            &description,
            &grass,
            &Matrix4::identity(),
            Vector3::new(0.0, 20.0, 0.0),
        );
        let (_, moved_one_cell) = select_grass_instances(
            &description,
            &grass,
            &Matrix4::identity(),
            Vector3::new(grass.spacing, 20.0, 0.0),
        );
        let moved: HashSet<[u32; 4]> = moved_one_cell
            .iter()
            .map(|instance| instance.map(f32::to_bits))
            .collect();
        let retained = first
            .iter()
            .filter(|instance| moved.contains(&instance.map(f32::to_bits)))
            .count();

        assert!(
            retained * 10 > first.len() * 9,
            "moving one cell replaced too much grass: retained {retained}/{}",
            first.len()
        );
    }

    #[test]
    fn four_k_heightmap_needs_six_quadtree_levels() {
        assert_eq!(max_quadtree_level(4096, 4096), 6);
        assert_eq!(max_quadtree_level(4097, 4097), 6);
        assert_eq!(max_quadtree_level(65, 65), 0);
    }

    #[test]
    fn projected_error_refines_nearby_patches_and_stays_bounded() {
        let description = terrain();
        let eye = Point3::new(0.0, 100.0, -500.0);
        let view = Matrix4::look_at_rh(eye, Point3::new(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
        let projection = perspective(Deg(60.0), 1.0, 0.1, 6000.0);
        let patches = select_patches(
            &description,
            4096,
            4096,
            &Matrix4::identity(),
            eye.to_vec(),
            &(projection * view),
            projection.y.y,
            1600.0,
        );
        assert!(patches.len() > 1, "near terrain should refine");
        assert!(patches.len() <= MAX_INSTANCES);
        assert!(patches.iter().all(|patch| patch[2] >= 1.0 / 64.0));
    }

    #[test]
    fn stereo_culling_keeps_the_exact_union_of_displaced_eye_frusta() {
        let mut description = terrain();
        description.width = 200.0;
        description.depth = 200.0;
        description.min_height = 0.0;
        description.max_height = 2.0;
        description.max_pixel_error = 0.1;
        let projection = perspective(Deg(24.0), 1.0, 0.1, 500.0);
        let left_eye = Point3::new(-45.0, 12.0, -90.0);
        let right_eye = Point3::new(45.0, 12.0, -90.0);
        let left_view =
            Matrix4::look_at_rh(left_eye, Point3::new(-45.0, 0.0, 0.0), Vector3::unit_y());
        let right_view =
            Matrix4::look_at_rh(right_eye, Point3::new(45.0, 0.0, 0.0), Vector3::unit_y());
        let left_vp = projection * left_view;
        let right_vp = projection * right_view;
        let select = |frusta: &[Matrix4<f32>]| {
            let mut patches = Vec::new();
            select_patches_into(
                &description,
                257,
                257,
                &Matrix4::identity(),
                Vector3::new(0.0, 12.0, -90.0),
                frusta,
                projection.y.y,
                1600.0,
                &mut patches,
            );
            patches
        };
        let left = select(std::slice::from_ref(&left_vp));
        let right = select(std::slice::from_ref(&right_vp));
        let union = select(&[left_vp, right_vp]);

        assert!(union.len() > left.len(), "right-eye-only patches were lost");
        assert!(union.len() > right.len(), "left-eye-only patches were lost");
        assert!(left.iter().all(|patch| union.contains(patch)));
        assert!(right.iter().all(|patch| union.contains(patch)));
    }

    #[test]
    fn fully_refined_quadtree_never_exceeds_the_fixed_instance_buffer() {
        let mut description = terrain();
        description.max_pixel_error = f32::MIN_POSITIVE;
        let eye = Point3::new(0.0, 5000.0, 0.0);
        let view = Matrix4::look_at_rh(
            eye,
            Point3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        );
        let projection = perspective(Deg(120.0), 1.0, 0.1, 10_000.0);
        let patches = select_patches(
            &description,
            PATCH_QUADS * (1 << MAX_LEVEL) + 1,
            PATCH_QUADS * (1 << MAX_LEVEL) + 1,
            &Matrix4::identity(),
            eye.to_vec(),
            &(projection * view),
            projection.y.y,
            2048.0,
        );
        assert!(patches.len() <= MAX_INSTANCES);
        assert!(
            patches.len() > MAX_INSTANCES - 32,
            "test should exhaust the split budget, got {} patches",
            patches.len()
        );
    }

    #[test]
    fn terrain_normals_use_inverse_transpose_for_non_uniform_scale() {
        let world = Matrix4::from_nonuniform_scale(2.0, 1.0, 0.5);
        let local = Vector3::new(1.0, 1.0, 0.0).normalize();
        let actual = (terrain_normal_matrix(&world) * local).normalize();
        let expected = Vector3::new(0.5, 1.0, 0.0).normalize();
        assert!((actual - expected).magnitude() < 1e-6);
    }

    #[test]
    fn transformed_patch_bounds_stay_conservative_under_shear() {
        let world = Matrix4::new(
            1.0, 0.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        );
        assert_eq!(
            transformed_aabb_radius(&world, Vector3::new(1.0, 1.0, 1.0)),
            3.0
        );
    }

    #[test]
    fn oversized_sources_fit_the_gpu_limit_and_preserve_corners() {
        let source = HeightmapData {
            width: 4,
            height: 3,
            samples: (0..12).collect(),
            revision: 1,
        };
        let (samples, width, height) = fit_heightmap(&source, 3);
        assert_eq!((width, height), (3, 2));
        assert_eq!(samples.first(), Some(&0));
        assert_eq!(samples.last(), Some(&11));
    }
}
