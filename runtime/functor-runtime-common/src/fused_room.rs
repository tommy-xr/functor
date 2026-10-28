//! The fused room (`Frame.withFusedRoom`): the environment depth read back to
//! the CPU every [`FUSE_EVERY`] frames, fused into a voxel volume off the
//! render thread ([`crate::fused_mesh`]), and its chunk meshes drawn as scene
//! geometry with the surface grid's lines.
//!
//! The readback never stalls the render thread: a pass packs the depth layer
//! into an RGBA8 target, `glReadPixels` copies it into a pixel-pack buffer,
//! and a later frame maps the buffer once its fence has signalled. Inline
//! runs (wasm, which has no threads, and desktop's deterministic
//! `--fixed-time`/`--input-script` captures) read it back and fuse at once,
//! so frame N always shows the same mesh.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use cgmath::Matrix4;
use glow::HasContext;

use crate::environment_depth::EnvironmentDepthView;
use crate::fused_mesh::{ChunkKey, DepthFrame, Volume, VOXEL};
use crate::material::Material;
use crate::runtime_settings::FusedMeshState;
use crate::scene3d::MeshData;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::surface_grid::{SurfaceGrid, GRID_LINES_GLSL};
use crate::{RenderContext, Scene3D, SceneObject, Shape};

/// Frames between depth readbacks: 30 fusions a second at 120 Hz.
pub const FUSE_EVERY: u64 = 4;
/// The readback's size: the headset's depth image, and desktop's stand-in
/// depth downsampled to it.
const READBACK_SIZE: i32 = 320;

/// Packs one depth layer into RGBA8: 24-bit window depth in RGB, most
/// significant byte first ([`DepthFrame::from_rgba8`] unpacks it).
const PACK_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec2 vUv;
        out vec4 fragColor;

        uniform highp sampler2DArray depthTexture;
        uniform float depthLayer;

        void main() {
            highp float d = textureLod(depthTexture, vec3(vUv, depthLayer), 0.0).r;
            highp float v = floor(clamp(d, 0.0, 1.0) * 16777215.0 + 0.5);
            highp float hi = floor(v / 65536.0);
            highp float mid = floor((v - hi * 65536.0) / 256.0);
            highp float lo = v - hi * 65536.0 - mid * 256.0;
            fragColor = vec4(hi, mid, lo, 255.0) / 255.0;
        }
"#;

const MESH_VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;
        uniform highp vec3 eye;
        // Metres toward the eye: over a drawn stand-in room, so the mesh
        // wins the depth test against the surface it was fused from.
        uniform highp float nudge;

        out highp vec3 worldPos;

        void main() {
            highp vec3 p = (world * vec4(inPos, 1.0)).xyz;
            worldPos = p;
            highp vec3 toEye = eye - p;
            p += toEye * min(nudge / max(length(toEye), 0.0001), 0.5);
            gl_Position = projection * view * vec4(p, 1.0);
        }
"#;

/// Every fragment writes depth (alpha 0 between lines), so nearer surfaces
/// hide the lines of farther ones.
const MESH_FRAGMENT_SHADER_SOURCE: &str = r#"
        in highp vec3 worldPos;
        out vec4 fragColor;

        uniform highp vec3 eye;
        uniform highp float cell;
        uniform vec3 color;

        void main() {
            float alpha = gridLines(worldPos, cell, distance(worldPos, eye));
            // `!(alpha > 0.0)` also catches a NaN from a degenerate normal.
            fragColor = functorOutput(vec4(functorSrgbToLinear(color), alpha > 0.0 ? alpha : 0.0));
        }
"#;

/// GL objects for packing and reading back one depth layer.
struct Readback {
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    pbo: glow::Buffer,
    program: ShaderProgram,
    depth_texture_loc: UniformLocation,
    depth_layer_loc: UniformLocation,
}

struct GridProgram {
    program: ShaderProgram,
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    eye_loc: UniformLocation,
    nudge_loc: UniformLocation,
    cell_loc: UniformLocation,
    color_loc: UniformLocation,
    output: crate::color_space::OutputEncodeUniform,
}

/// What the fusion thread hands back: chunk mesh updates since the last
/// drain, and the frames fused so far.
#[derive(Default)]
struct Fused {
    updates: Vec<(ChunkKey, Option<MeshData>)>,
    fused_frames: u64,
    top_eye_y: Option<f32>,
}

enum Fuser {
    Inline(Volume),
    Thread {
        // Holds one frame: a readback arriving while the thread is busy is
        // dropped rather than queued.
        frames: mpsc::SyncSender<(Vec<u8>, Matrix4<f32>)>,
        fused: Arc<Mutex<Fused>>,
    },
}

/// The fused room's state on the render thread.
pub(crate) struct FusedRoom {
    readback: Readback,
    grid: Option<Rc<GridProgram>>,
    /// The readback in flight: its fence and the depth transform it read.
    pending: Option<(glow::Fence, Matrix4<f32>)>,
    last_capture: Option<u64>,
    fuser: Fuser,
    fused_frames: u64,
    /// The highest the depth camera has been ([`Volume::top_eye_y`]).
    top_eye_y: Option<f32>,
    meshes: BTreeMap<ChunkKey, Arc<MeshData>>,
    triangles: usize,
    /// `meshes` as one group, rebuilt when they change.
    scene: Option<Scene3D>,
    /// `meshes` for `Physics.fusedRoom`, rebuilt when they change.
    colliders: Option<Arc<crate::physics::RoomMeshes>>,
}

fn fuse_thread(frames: mpsc::Receiver<(Vec<u8>, Matrix4<f32>)>, fused: Arc<Mutex<Fused>>) {
    let mut volume = Volume::default();
    let mut busy = std::time::Duration::ZERO;
    // Ends when the render thread drops the sender.
    for (rgba, view_projection) in frames {
        let start = std::time::Instant::now();
        let size = READBACK_SIZE as usize;
        volume.integrate(&DepthFrame::from_rgba8(size, size, &rgba, view_projection));
        let updates = volume.extract_dirty();
        busy += start.elapsed();
        if volume.fused_frames % 120 == 0 {
            log::info!(
                "fused room: {} frames, {:.2} ms per fusion over the last 120",
                volume.fused_frames,
                busy.as_secs_f64() * 1000.0 / 120.0
            );
            busy = std::time::Duration::ZERO;
        }
        let mut out = fused.lock().expect("fusion state");
        out.updates.extend(updates);
        out.fused_frames = volume.fused_frames;
        out.top_eye_y = volume.top_eye_y;
    }
}

impl FusedRoom {
    pub(crate) fn new(gl: &glow::Context, shader_version: &str, inline: bool) -> FusedRoom {
        let fuser = if inline {
            Fuser::Inline(Volume::default())
        } else {
            let (frames, receiver) = mpsc::sync_channel(1);
            let fused = Arc::new(Mutex::new(Fused::default()));
            let thread_fused = fused.clone();
            std::thread::Builder::new()
                .name("functor-fusion".to_string())
                .spawn(move || fuse_thread(receiver, thread_fused))
                .expect("spawn the fusion thread");
            Fuser::Thread { frames, fused }
        };
        FusedRoom {
            readback: Readback::new(gl, shader_version),
            grid: None,
            pending: None,
            last_capture: None,
            fuser,
            fused_frames: 0,
            top_eye_y: None,
            meshes: BTreeMap::new(),
            triangles: 0,
            scene: None,
            colliders: None,
        }
    }

    pub(crate) fn delete(self, gl: &glow::Context) {
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            if let Some((fence, _)) = self.pending {
                gl.delete_sync(fence);
            }
            gl.delete_framebuffer(self.readback.fbo);
            gl.delete_renderbuffer(self.readback.color);
            gl.delete_buffer(self.readback.pbo);
            counters.buffer_deleted();
            gl.delete_program(self.readback.program.program_id);
            if let Some(grid) = &self.grid {
                gl.delete_program(grid.program.program_id);
            }
        }
    }

    /// Once per main pass: collect a finished readback, start one every
    /// [`FUSE_EVERY`] frames from `view`, and take the fusion's new meshes.
    pub(crate) fn update(
        &mut self,
        gl: &glow::Context,
        quad: &mut dyn crate::geometry::Geometry,
        frame: u64,
        view: Option<EnvironmentDepthView>,
    ) {
        let inline = matches!(self.fuser, Fuser::Inline(_));
        if let Some(view) = view.filter(|_| {
            frame % FUSE_EVERY == 0 && self.last_capture != Some(frame) && self.pending.is_none()
        }) {
            self.last_capture = Some(frame);
            self.pending = Some((self.readback.capture(gl, quad, view), view.view_projection));
        }
        if let Some((fence, view_projection)) = self.pending.take() {
            // Inline: read now — mapping waits for the copy. Threaded: only
            // once the GPU is done, so the render thread never waits.
            let status = if inline {
                glow::ALREADY_SIGNALED
            } else {
                unsafe { gl.client_wait_sync(fence, 0, 0) }
            };
            if status == glow::TIMEOUT_EXPIRED {
                self.pending = Some((fence, view_projection));
            } else {
                // Signalled — or failed, which drops this readback.
                let rgba = (status != glow::WAIT_FAILED)
                    .then(|| self.readback.read(gl))
                    .flatten();
                unsafe { gl.delete_sync(fence) };
                if let Some(rgba) = rgba {
                    self.fuse(rgba, view_projection);
                }
            }
        }
        let updates = match &self.fuser {
            Fuser::Inline(_) => Vec::new(),
            Fuser::Thread { fused, .. } => {
                let mut fused = fused.lock().expect("fusion state");
                self.fused_frames = fused.fused_frames;
                self.top_eye_y = fused.top_eye_y;
                std::mem::take(&mut fused.updates)
            }
        };
        self.apply(updates);
    }

    fn fuse(&mut self, rgba: Vec<u8>, view_projection: Matrix4<f32>) {
        match &mut self.fuser {
            Fuser::Inline(volume) => {
                let size = READBACK_SIZE as usize;
                volume.integrate(&DepthFrame::from_rgba8(size, size, &rgba, view_projection));
                self.fused_frames = volume.fused_frames;
                self.top_eye_y = volume.top_eye_y;
                let updates = volume.extract_dirty();
                self.apply(updates);
            }
            Fuser::Thread { frames, .. } => {
                // Full (still fusing the last one) or gone: skip this one.
                let _ = frames.try_send((rgba, view_projection));
            }
        }
    }

    fn apply(&mut self, updates: Vec<(ChunkKey, Option<MeshData>)>) {
        if updates.is_empty() {
            return;
        }
        for (key, mesh) in updates {
            match mesh {
                Some(mesh) => self.meshes.insert(key, Arc::new(mesh)),
                None => self.meshes.remove(&key),
            };
        }
        self.triangles = self.meshes.values().map(|m| m.triangles.len()).sum();
        self.scene = None;
        self.colliders = None;
    }

    /// The meshes for `Physics.fusedRoom`: the same `Arc` until they change.
    pub(crate) fn room_meshes(&mut self) -> Arc<crate::physics::RoomMeshes> {
        let (meshes, top_eye_y) = (&self.meshes, self.top_eye_y);
        // A higher head can admit a floor: rebuild for it too.
        if self.colliders.as_ref().is_some_and(|c| c.top_eye_y != top_eye_y) {
            self.colliders = None;
        }
        self.colliders
            .get_or_insert_with(|| {
                Arc::new(crate::physics::RoomMeshes {
                    chunks: meshes.clone(),
                    top_eye_y,
                })
            })
            .clone()
    }

    /// `GET /state.xr.mesh`.
    pub(crate) fn state(&self) -> FusedMeshState {
        FusedMeshState {
            chunks: self.meshes.len(),
            triangles: self.triangles,
            fused_frames: self.fused_frames,
            voxel_m: VOXEL,
            ..Default::default()
        }
    }

    /// The meshes as one group (identity transforms: they are in world
    /// space) and the grid material to draw them with; `None` while empty.
    pub(crate) fn drawable(
        &mut self,
        render_context: &RenderContext,
        grid: SurfaceGrid,
        nudge: f32,
    ) -> Option<(&Scene3D, Box<dyn Material>)> {
        if self.meshes.is_empty() {
            return None;
        }
        let program = self
            .grid
            .get_or_insert_with(|| Rc::new(GridProgram::new(render_context)))
            .clone();
        let meshes = &self.meshes;
        let scene = self.scene.get_or_insert_with(|| {
            let mesh_node = |mesh: &Arc<MeshData>| Scene3D {
                obj: SceneObject::Geometry(Shape::Mesh { mesh: mesh.clone() }),
                xform: Matrix4::from_scale(1.0),
            };
            Scene3D {
                obj: SceneObject::Group(meshes.values().map(mesh_node).collect()),
                xform: Matrix4::from_scale(1.0),
            }
        });
        Some((
            scene,
            Box::new(GridMaterial {
                program,
                grid,
                nudge,
            }),
        ))
    }
}

impl Readback {
    fn new(gl: &glow::Context, shader_version: &str) -> Readback {
        let vertex = Shader::build(
            gl,
            ShaderType::Vertex,
            crate::composite::COMPOSITE_VERTEX_SHADER_SOURCE,
            shader_version,
        );
        let fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            PACK_FRAGMENT_SHADER_SOURCE,
            shader_version,
        );
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        unsafe {
            let color = gl.create_renderbuffer().expect("fused room readback color");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::RGBA8,
                READBACK_SIZE,
                READBACK_SIZE,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            let previous = crate::renderer::ambient_framebuffer(gl);
            let fbo = gl.create_framebuffer().expect("fused room readback fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);

            let pbo = gl.create_buffer().expect("fused room readback buffer");
            crate::gpu_counters::gpu_counters().buffer_created();
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pbo));
            gl.buffer_data_size(
                glow::PIXEL_PACK_BUFFER,
                READBACK_SIZE * READBACK_SIZE * 4,
                glow::STREAM_READ,
            );
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            Readback {
                fbo,
                color,
                pbo,
                depth_texture_loc: program.get_uniform_location(gl, "depthTexture"),
                depth_layer_loc: program.get_uniform_location(gl, "depthLayer"),
                program,
            }
        }
    }

    /// Pack `view`'s layer and start copying it into the pixel-pack buffer;
    /// the fence signals when the copy is done. Restores the framebuffer;
    /// the caller resets the viewport.
    fn capture(
        &self,
        gl: &glow::Context,
        quad: &mut dyn crate::geometry::Geometry,
        view: EnvironmentDepthView,
    ) -> glow::Fence {
        unsafe {
            let previous = crate::renderer::ambient_framebuffer(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, READBACK_SIZE, READBACK_SIZE);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            self.program.use_program(gl);
            self.program.set_uniform_1i(gl, &self.depth_texture_loc, 0);
            self.program
                .set_uniform_1f(gl, &self.depth_layer_loc, view.layer as f32);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(view.texture));
            quad.draw(gl);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(self.pbo));
            gl.read_pixels(
                0,
                0,
                READBACK_SIZE,
                READBACK_SIZE,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::BufferOffset(0),
            );
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            let fence = gl
                .fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
                .expect("fused room readback fence");
            gl.enable(glow::DEPTH_TEST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            fence
        }
    }

    /// The pixel-pack buffer's bytes; `None` if it could not be mapped.
    fn read(&self, gl: &glow::Context) -> Option<Vec<u8>> {
        let len = (READBACK_SIZE * READBACK_SIZE * 4) as usize;
        let mut rgba = Some(vec![0; len]);
        unsafe {
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(self.pbo));
            // WebGL 2 has no mapping; its read is a copy.
            #[cfg(target_arch = "wasm32")]
            if let Some(rgba) = &mut rgba {
                gl.get_buffer_sub_data(glow::PIXEL_PACK_BUFFER, 0, rgba);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mapped =
                    gl.map_buffer_range(glow::PIXEL_PACK_BUFFER, 0, len as i32, glow::MAP_READ_BIT);
                match rgba.as_mut().filter(|_| !mapped.is_null()) {
                    Some(rgba) => {
                        rgba.copy_from_slice(std::slice::from_raw_parts(mapped, len));
                        gl.unmap_buffer(glow::PIXEL_PACK_BUFFER);
                    }
                    None => rgba = None,
                }
            }
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        }
        rgba
    }
}

impl GridProgram {
    fn new(ctx: &RenderContext) -> GridProgram {
        let gl = ctx.gl;
        let vertex = Shader::build(
            gl,
            ShaderType::Vertex,
            MESH_VERTEX_SHADER_SOURCE,
            ctx.shader_version,
        );
        let fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            &format!("{GRID_LINES_GLSL}\n{MESH_FRAGMENT_SHADER_SOURCE}"),
            ctx.shader_version,
        );
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        GridProgram {
            world_loc: program.get_uniform_location(gl, "world"),
            view_loc: program.get_uniform_location(gl, "view"),
            projection_loc: program.get_uniform_location(gl, "projection"),
            eye_loc: program.get_uniform_location(gl, "eye"),
            nudge_loc: program.get_uniform_location(gl, "nudge"),
            cell_loc: program.get_uniform_location(gl, "cell"),
            color_loc: program.get_uniform_location(gl, "color"),
            output: crate::color_space::OutputEncodeUniform::get(&program, gl),
            program,
        }
    }
}

/// The surface grid's lines on the fused mesh, blended straight-alpha (the
/// caller sets the blend state, as for the surface grid's pass).
struct GridMaterial {
    program: Rc<GridProgram>,
    grid: SurfaceGrid,
    nudge: f32,
}

impl Material for GridMaterial {
    fn initialize(&mut self, _ctx: &RenderContext) {}

    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        let (gl, p) = (ctx.gl, &self.program);
        p.program.use_program(gl);
        p.program
            .set_uniform_matrix4(gl, &p.world_loc, world_matrix);
        p.program.set_uniform_matrix4(gl, &p.view_loc, view_matrix);
        p.program
            .set_uniform_matrix4(gl, &p.projection_loc, projection_matrix);
        p.program.set_uniform_vec3(gl, &p.eye_loc, &ctx.camera_pos);
        p.program.set_uniform_1f(gl, &p.nudge_loc, self.nudge);
        p.program.set_uniform_1f(gl, &p.cell_loc, self.grid.cell);
        let [r, g, b] = self.grid.color;
        p.program
            .set_uniform_vec3(gl, &p.color_loc, &cgmath::vec3(r, g, b));
        p.output.set(
            &p.program,
            gl,
            ctx.output_srgb_encode,
            crate::color_space::OutputAlpha::Blend,
        );
        true
    }
}
