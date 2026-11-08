//! The fused room (`Frame.withFusedRoom`): the environment depth readbacks
//! ([`crate::depth_readback`]) fused into a voxel volume off the render
//! thread ([`crate::fused_mesh`]), and its chunk meshes drawn as scene
//! geometry with the surface grid's lines.
//!
//! Inline runs (wasm, which has no threads, and desktop's deterministic
//! `--fixed-time`/`--input-script` captures) read back and fuse at once, so
//! frame N always shows the same mesh.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use cgmath::Matrix4;
use glow::HasContext;

use crate::depth_readback::READBACK_SIZE;
use crate::fused_mesh::{ChunkKey, DepthFrame, Volume, VOXEL};
use crate::material::Material;
use crate::runtime_settings::FusedMeshState;
use crate::scene3d::MeshData;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::surface_grid::{SurfaceGrid, GRID_LINES_GLSL};
use crate::{RenderContext, Scene3D, SceneObject, Shape};

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
    grid: Option<Rc<GridProgram>>,
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
    pub(crate) fn new(inline: bool) -> FusedRoom {
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
            grid: None,
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
        if let Some(grid) = &self.grid {
            unsafe { gl.delete_program(grid.program.program_id) };
        }
    }

    /// Once per main pass: take the fusion thread's new meshes.
    pub(crate) fn drain(&mut self) {
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

    /// Fold one readback in: at once inline, else handed to the fusion
    /// thread (dropped while it is still busy with the last one).
    pub(crate) fn fuse(&mut self, rgba: Vec<u8>, view_projection: Matrix4<f32>) {
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
