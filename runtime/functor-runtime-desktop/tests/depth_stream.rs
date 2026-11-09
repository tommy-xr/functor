//! `Xr.streamDepth` end to end on desktop: the stand-in room's depth is read
//! back through the shared `render_frame` path and POSTed to an in-test HTTP
//! server, which decodes each `functor.depth-frame/1` and unprojects its
//! pixels by the documented formula onto the known room (a floor and a box).
//!
//! - frames arrive, `seq` increments, pixels land on the room within 5 mm;
//! - a slow server sees one POST at a time while `dropped` counts the rest;
//! - nothing uploads while not live (paused, scrubbed, replaying), and
//!   resuming starts a new stream from `seq` 0;
//! - a server that never answers times out as an error.
//!
//! Self-skipping like `passthrough_alpha` (needs a GL display; GLFW on the
//! main thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test depth_stream -- --ignored --nocapture
//! ```

mod sensor_server;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cgmath::{InnerSpace, Matrix4, SquareMatrix, Vector4};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::depth_frame::{decode, DepthFrameHeader, DepthFrameSource};
use functor_runtime_common::environment_depth::EnvironmentDepthSource;
use functor_runtime_common::math::Angle;
use functor_runtime_common::runtime_settings::SensorStreamSettings;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, DebugRenderMode, Frame, FrameTime, Scene3D, SceneContext, SceneObject,
    Viewport,
};
use glow::HasContext;
use sensor_server::Server;

/// The readback's size, so stand-in texels and streamed pixels coincide.
const SIZE: u32 = 320;
/// The box: x, z in ±0.4, y in 0..0.8, on a 10 m floor at y = 0.
const BOX_HALF: [f32; 3] = [0.4, 0.4, 0.4];
const BOX_CENTER: [f32; 3] = [0.0, 0.4, 0.0];
const TOLERANCE_M: f32 = 0.005;
const DEPTH: &str = functor_runtime_common::depth_frame::CONTENT_TYPE;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("depth_stream: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let harness = Harness::new();
    frames_decode_onto_the_room(&harness);
    println!("test frames_decode_onto_the_room ... ok");
    a_slow_server_sees_one_post_at_a_time(&harness);
    println!("test a_slow_server_sees_one_post_at_a_time ... ok");
    nothing_uploads_unless_live(&harness);
    println!("test nothing_uploads_unless_live ... ok");
    a_silent_server_times_out(&harness);
    println!("test a_silent_server_times_out ... ok");
}

fn frames_decode_onto_the_room(harness: &Harness) {
    let server = Server::start(DEPTH, Duration::ZERO, false);
    harness.stream(&server, true);
    harness.render_until(|| server.bodies().len() >= 10, "10 frames");
    harness.stop();
    let frames: Vec<(DepthFrameHeader, Vec<u8>)> = server
        .bodies()
        .iter()
        .map(|body| {
            let (header, payload) = decode(body).expect("a valid depth frame");
            (header, payload.to_vec())
        })
        .collect();

    let camera = Harness::camera();
    let (mut floor, mut boxed, mut worst) = (0, 0, 0.0f32);
    for (i, (header, payload)) in frames.iter().enumerate() {
        assert_eq!(header.stream, frames[0].0.stream);
        assert!(i == 0 || header.seq > frames[i - 1].0.seq, "seq increments");
        assert_eq!(header.source, DepthFrameSource::Emulated);
        assert_eq!((header.width, header.height), (SIZE, SIZE));
        assert_eq!(header.time.clock, "steady");
        assert!((header.near - camera.near).abs() < 1e-4);
        assert!((header.far.unwrap() - camera.far).abs() / camera.far < 1e-3);
        // The pose is the camera's, and view_projection agrees with it.
        let camera_to_world = matrix(&header.camera_to_world);
        let eye = camera_to_world * Vector4::new(0.0, 0.0, 0.0, 1.0);
        for (a, b) in [eye.x, eye.y, eye.z].iter().zip(camera.eye) {
            assert!((a - b).abs() < 1e-4, "eye {eye:?} vs {:?}", camera.eye);
        }
        let composed = matrix(&header.projection) * camera_to_world.invert().unwrap();
        let view_projection = matrix(&header.view_projection);
        for c in 0..4 {
            assert!((composed[c] - view_projection[c]).magnitude() < 1e-4);
        }

        // The documented decode, pixel by pixel.
        let inverse = view_projection.invert().unwrap();
        for y in 0..SIZE as usize {
            for x in 0..SIZE as usize {
                let p = &payload[(y * SIZE as usize + x) * 4..][..4];
                assert_eq!(p[3], 255);
                let packed = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                if packed == 16_777_215 {
                    continue;
                }
                let d = packed as f32 / 16_777_215.0;
                let ndc = Vector4::new(
                    (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0,
                    (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0,
                    2.0 * d - 1.0,
                    1.0,
                );
                let world = inverse * ndc;
                let world = [world.x / world.w, world.y / world.w, world.z / world.w];
                let (to_floor, to_box) = (world[1].abs(), box_distance(world));
                worst = worst.max(to_floor.min(to_box));
                if to_box < TOLERANCE_M {
                    boxed += 1;
                } else if to_floor < TOLERANCE_M {
                    floor += 1;
                }
            }
        }
    }
    assert!(worst < TOLERANCE_M, "a pixel lands {worst} m off the room");
    assert!(floor > 0 && boxed > 0, "floor {floor}, box {boxed}");
    println!("  {} frames; worst pixel {:.2} mm off the room", frames.len(), worst * 1000.0);
}

fn a_slow_server_sees_one_post_at_a_time(harness: &Harness) {
    let server = Server::start(DEPTH, Duration::from_millis(150), false);
    harness.stream(&server, true);
    harness.render_until(|| harness.state().dropped >= 5 && server.bodies().len() >= 3, "drops");
    let state = harness.state();
    // New streams while a POST is in flight still share one connection.
    for hz in [500.0, 400.0, 300.0] {
        harness.stream_at(&server, hz, true);
        for _ in 0..20 {
            harness.render();
        }
    }
    harness.stop();
    // Let the last POST land.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(server.max_in_flight.load(Ordering::SeqCst), 1);
    let bodies = server.bodies();
    let first = decode(&bodies[0]).unwrap().0.stream;
    let seqs: Vec<u64> = bodies
        .iter()
        .map(|b| decode(b).unwrap().0)
        .filter(|h| h.stream == first)
        .map(|h| h.seq)
        .collect();
    // Every gap in seq is a frame dropped from the one waiting slot.
    let gaps: u64 = seqs.windows(2).map(|w| w[1] - w[0] - 1).sum::<u64>() + seqs[0];
    assert!(gaps <= state.dropped, "gaps {gaps}, dropped {}", state.dropped);
    assert!(state.dropped >= 5 && state.errors == 0, "{state:?}");
}

fn nothing_uploads_unless_live(harness: &Harness) {
    let server = Server::start(DEPTH, Duration::ZERO, false);
    harness.stream(&server, false);
    for _ in 0..200 {
        harness.render();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.bodies().len(), 0, "uploaded while not live");
    assert_eq!(harness.state().sent, 0);
    // Live, then not, then live again: a new stream from seq 0.
    harness.stream(&server, true);
    harness.render_until(|| !server.bodies().is_empty(), "a first frame");
    harness.stream(&server, false);
    std::thread::sleep(Duration::from_millis(200));
    let before = server.bodies().len();
    for _ in 0..200 {
        harness.render();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.bodies().len(), before, "uploaded while paused");
    harness.stream(&server, true);
    harness.render_until(|| server.bodies().len() > before, "a resumed frame");
    harness.stop();
    let first = decode(&server.bodies()[0]).unwrap().0;
    let resumed = decode(&server.bodies()[before]).unwrap().0;
    assert_ne!(first.stream, resumed.stream);
    // Seq restarts at 0; a frame replaced before the sender woke is a drop.
    let dropped = harness.last_state.lock().unwrap().as_ref().unwrap().dropped;
    assert!(resumed.seq <= dropped, "resumed at seq {}, {dropped} dropped", resumed.seq);
}

fn a_silent_server_times_out(harness: &Harness) {
    let server = Server::start(DEPTH, Duration::ZERO, true);
    harness.stream(&server, true);
    harness.render_until(|| harness.state().errors >= 1, "a timeout");
    harness.stop();
    assert_eq!(harness.last_state.lock().unwrap().as_ref().unwrap().sent, 0);
}

/// Distance from `p` to the box's surface.
fn box_distance(p: [f32; 3]) -> f32 {
    let q: Vec<f32> = (0..3).map(|a| (p[a] - BOX_CENTER[a]).abs() - BOX_HALF[a]).collect();
    let outside = q.iter().map(|c| c.max(0.0).powi(2)).sum::<f32>().sqrt();
    let inside = q.iter().cloned().fold(f32::MIN, f32::max).min(0.0);
    (outside + inside).abs()
}

fn matrix(columns: &[f32; 16]) -> Matrix4<f32> {
    let c = |i: usize| Vector4::new(columns[i], columns[i + 1], columns[i + 2], columns[i + 3]);
    Matrix4::from_cols(c(0), c(4), c(8), c(12))
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    shadow_map: ShadowMap,
    last_state: Mutex<Option<functor_runtime_common::runtime_settings::SensorStreamState>>,
    _window: glfw::PWindow,
    _glfw: glfw::Glfw,
}

impl Harness {
    fn new() -> Harness {
        use glfw::Context as _;
        let mut glfw = glfw::init(glfw::fail_on_errors).expect("init GLFW");
        glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
        glfw.window_hint(glfw::WindowHint::Visible(false));
        let (mut window, _events) = glfw
            .create_window(SIZE, SIZE, "depth-stream-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };
        let fbo = unsafe {
            let color = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, SIZE as i32, SIZE as i32);
            let depth = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                SIZE as i32,
                SIZE as i32,
            );
            let fbo = gl.create_framebuffer().expect("fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            fbo
        };
        let shadow_map = ShadowMap::new(&gl, 16);
        Harness {
            gl,
            fbo,
            asset_cache: Arc::new(AssetCache::new()),
            scene_context: SceneContext::new(),
            shadow_map,
            last_state: Mutex::new(None),
            _window: window,
            _glfw: glfw,
        }
    }

    fn camera() -> Camera {
        Camera::look_at(
            [0.3, 1.6, 2.5],
            [0.0, 0.3, 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        )
    }

    /// Stream to `server` as fast as the readbacks allow.
    fn stream(&self, server: &Server, live: bool) {
        self.stream_at(server, 1000.0, live);
    }

    fn stream_at(&self, server: &Server, hz: f32, live: bool) {
        let settings = SensorStreamSettings {
            url: server.url.clone(),
            hz,
        };
        self.scene_context.set_depth_stream(Some(&settings), live);
    }

    /// Stop streaming, keeping the final counters.
    fn stop(&self) {
        *self.last_state.lock().unwrap() = Some(self.state());
        self.scene_context.set_depth_stream(None, false);
    }

    fn state(&self) -> functor_runtime_common::runtime_settings::SensorStreamState {
        self.scene_context.depth_stream_state().expect("streaming")
    }

    /// One passthrough frame of the stand-in room.
    fn render(&self) {
        let room = Scene3D {
            obj: SceneObject::Group(vec![
                Scene3D::plane().transform(Matrix4::from_scale(10.0)),
                Scene3D::cube().transform(
                    Matrix4::from_translation(BOX_CENTER.into())
                        * Matrix4::from_nonuniform_scale(
                            BOX_HALF[0] * 2.0,
                            BOX_HALF[1] * 2.0,
                            BOX_HALF[2] * 2.0,
                        ),
                ),
            ]),
            xform: Matrix4::from_scale(1.0),
        };
        let empty = Scene3D {
            obj: SceneObject::Group(vec![]),
            xform: Matrix4::from_scale(1.0),
        };
        let frame = Frame::with_stand_in_room(
            Frame::with_passthrough(Frame::new(Self::camera(), empty)),
            room,
        );
        self.scene_context
            .set_environment_depth_source(EnvironmentDepthSource::StandIn);
        unsafe { self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo)) };
        render_frame(
            &self.gl,
            "#version 410",
            self.asset_cache.clone(),
            &self.scene_context,
            &self.shadow_map,
            &frame,
            &frame.camera,
            FrameTime { tts: 0.0, dts: 0.0 },
            Viewport::new(SIZE, SIZE),
            DebugRenderMode::Default,
        );
        // What a swap would do: submit, so the readback's fence can signal.
        unsafe { self.gl.flush() };
    }

    fn render_until(&self, done: impl Fn() -> bool, what: &str) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
            self.render();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
