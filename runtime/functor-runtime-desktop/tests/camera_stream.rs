//! `Xr.streamCamera` end to end on desktop: the stand-in room is rendered as
//! the emulated camera after a `render_frame` main pass and POSTed to an
//! in-test HTTP server, which decodes each `functor.camera-frame/1` and
//! projects known world points (bright marker cubes) through the header's
//! pose and intrinsics by the documented formula:
//!
//! - each marker's blob centroid lands within 0.3 px of its projection;
//! - a red marker's chroma reads red, so the planes are in I420 order;
//! - the game's scene never shows in the camera image.
//!
//! Self-skipping like `depth_stream` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test camera_stream -- --ignored --nocapture
//! ```

mod sensor_server;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cgmath::{InnerSpace, Matrix4, SquareMatrix, Vector3, Vector4};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::camera_frame::{decode, CameraFrameHeader, CameraFrameSource};
use functor_runtime_common::camera_stream::{EMULATED_HEIGHT, EMULATED_INTRINSICS, EMULATED_WIDTH};
use functor_runtime_common::math::Angle;
use functor_runtime_common::runtime_settings::SensorStreamSettings;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, DebugRenderMode, Frame, FrameTime, MaterialDescription, Scene3D,
    SceneObject, Viewport,
};
use glow::HasContext;
use sensor_server::Server;

const CAMERA: &str = functor_runtime_common::camera_frame::CONTENT_TYPE;
const WINDOW: u32 = 320;
/// White marker cube centres; the red one; the game's own (unseen) cube.
const MARKERS: [[f32; 3]; 4] = [
    [0.0, 1.2, -0.5],
    [-0.6, 0.9, -0.3],
    [0.5, 1.5, -0.8],
    [0.3, 0.6, 0.2],
];
const RED: [f32; 3] = [-0.4, 1.6, -0.6];
const GAME_CUBE: [f32; 3] = [0.6, 1.0, 0.0];
const MARKER_SIZE: f32 = 0.06;
const TOLERANCE_PX: f32 = 0.3;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("camera_stream: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let harness = Harness::new();
    frames_project_onto_the_markers(&harness);
    println!("test frames_project_onto_the_markers ... ok");
}

fn frames_project_onto_the_markers(harness: &Harness) {
    let server = Server::start(CAMERA, Duration::ZERO, false);
    let settings = SensorStreamSettings {
        url: server.url.clone(),
        hz: 1000.0,
    };
    harness
        .scene_context
        .set_camera_stream(Some(&settings), true);
    let start = Instant::now();
    while server.bodies().len() < 3 {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for frames"
        );
        harness.render();
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = harness
        .scene_context
        .camera_stream_state()
        .expect("streaming");
    harness.scene_context.set_camera_stream(None, false);
    assert!(state.sent >= 3 && state.errors == 0, "{state:?}");
    assert_eq!(server.max_in_flight.load(Ordering::SeqCst), 1);

    let camera = Harness::camera();
    let mut worst = 0.0f32;
    for (i, body) in server.bodies().iter().enumerate() {
        let (header, yuv) = decode(body).expect("a valid camera frame");
        assert_eq!(header.seq, i as u64);
        check_header(&header, &camera);
        let (w, h) = (header.width as usize, header.height as usize);
        let (y_plane, chroma) = yuv.split_at(w * h);
        let (u_plane, v_plane) = chroma.split_at(w / 2 * h / 2);
        let world_to_camera = matrix(&header.camera_to_world).invert().unwrap();
        let project = |p: [f32; 3]| {
            let c = world_to_camera * Vector4::new(p[0], p[1], p[2], 1.0);
            header
                .intrinsics
                .project([c.x, c.y, c.z])
                .expect("in front of the camera")
        };
        for marker in MARKERS {
            let [u, v] = project(marker);
            let [cu, cv] = centroid(y_plane, w, h, [u, v], 128).expect("the marker is visible");
            worst = worst.max(((cu - u).powi(2) + (cv - v).powi(2)).sqrt());
        }
        // Red: luma 76, U below neutral, V high.
        let [u, v] = project(RED);
        let chroma_at = |plane: &[u8]| plane[(v as usize / 2) * (w / 2) + u as usize / 2];
        assert!((60..=90).contains(&y_plane[v.round() as usize * w + u.round() as usize]));
        assert!(chroma_at(u_plane) < 100 && chroma_at(v_plane) > 220);
        // The game's cube is not real: the backdrop shows where it would be.
        let [u, v] = project(GAME_CUBE);
        assert!(
            centroid(y_plane, w, h, [u, v], 128).is_none(),
            "the game scene shows"
        );
    }
    assert!(
        worst < TOLERANCE_PX,
        "a marker lands {worst} px off its projection"
    );
    println!(
        "  {} frames; worst marker {worst:.3} px off",
        server.bodies().len()
    );
}

fn check_header(header: &CameraFrameHeader, camera: &Camera) {
    assert_eq!(header.source, CameraFrameSource::Emulated);
    assert_eq!(header.camera, "left");
    assert_eq!(
        (header.width, header.height),
        (EMULATED_WIDTH, EMULATED_HEIGHT)
    );
    assert_eq!(header.intrinsics, EMULATED_INTRINSICS);
    assert_eq!(header.distortion, None);
    assert_eq!(header.time.clock, "steady");
    assert!(header.tracking);
    // The camera sits at the eye, its +z (OpenCV forward) toward the target.
    let camera_to_world = matrix(&header.camera_to_world);
    let eye = camera_to_world.w.truncate();
    assert!(
        (eye - Vector3::from(camera.eye)).magnitude() < 1e-4,
        "{eye:?}"
    );
    let forward = (Vector3::from(camera.target) - Vector3::from(camera.eye)).normalize();
    assert!(camera_to_world.z.truncate().dot(forward) > 0.9999);
    // Head = the view: the OpenCV-to-GL flip.
    let flip = Matrix4::from_diagonal(Vector4::new(1.0, -1.0, -1.0, 1.0));
    assert_eq!(matrix(&header.camera_to_head), flip);
}

/// The centroid of the pixels brighter than `threshold` within 20 px of
/// `near`; `None` if there are none.
fn centroid(plane: &[u8], w: usize, h: usize, near: [f32; 2], threshold: u8) -> Option<[f32; 2]> {
    let (mut sum, mut n) = ([0.0f32; 2], 0.0f32);
    let (x0, y0) = (near[0].round() as i64, near[1].round() as i64);
    for y in (y0 - 20).max(0)..(y0 + 20).min(h as i64) {
        for x in (x0 - 20).max(0)..(x0 + 20).min(w as i64) {
            if plane[y as usize * w + x as usize] > threshold {
                sum[0] += x as f32;
                sum[1] += y as f32;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| [sum[0] / n, sum[1] / n])
}

fn matrix(columns: &[f32; 16]) -> Matrix4<f32> {
    let c = |i: usize| Vector4::new(columns[i], columns[i + 1], columns[i + 2], columns[i + 3]);
    Matrix4::from_cols(c(0), c(4), c(8), c(12))
}

fn emissive(r: f32, g: f32, b: f32, items: Vec<Scene3D>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::Emissive {
                color: Vector4::new(r, g, b, 1.0),
                texture: None,
            },
            items,
        ),
        xform: Matrix4::from_scale(1.0),
    }
}

fn cube_at(center: [f32; 3], size: f32) -> Scene3D {
    Scene3D::cube().transform(Matrix4::from_translation(center.into()) * Matrix4::from_scale(size))
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: functor_runtime_common::SceneContext,
    shadow_map: ShadowMap,
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
            .create_window(
                WINDOW,
                WINDOW,
                "camera-stream-test",
                glfw::WindowMode::Windowed,
            )
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };
        // The main pass's target: color and depth renderbuffers.
        let fbo = unsafe {
            let fbo = gl.create_framebuffer().expect("fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            for (format, attachment) in [
                (glow::RGBA8, glow::COLOR_ATTACHMENT0),
                (glow::DEPTH_COMPONENT24, glow::DEPTH_ATTACHMENT),
            ] {
                let buffer = gl.create_renderbuffer().expect("renderbuffer");
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(buffer));
                gl.renderbuffer_storage(glow::RENDERBUFFER, format, WINDOW as i32, WINDOW as i32);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    attachment,
                    glow::RENDERBUFFER,
                    Some(buffer),
                );
            }
            fbo
        };
        let shadow_map = ShadowMap::new(&gl, 16);
        Harness {
            gl,
            fbo,
            asset_cache: Arc::new(AssetCache::new()),
            scene_context: functor_runtime_common::SceneContext::new(),
            shadow_map,
            _window: window,
            _glfw: glfw,
        }
    }

    fn camera() -> Camera {
        Camera::look_at(
            [0.1, 1.5, 2.0],
            [0.0, 1.1, -0.5],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        )
    }

    /// One passthrough frame: a black room (floor, backdrop) holding the
    /// markers, and a game scene of one white cube.
    fn render(&self) {
        let white = MARKERS.iter().map(|m| cube_at(*m, MARKER_SIZE)).collect();
        let room = Scene3D {
            obj: SceneObject::Group(vec![
                emissive(
                    0.0,
                    0.0,
                    0.0,
                    vec![
                        Scene3D::plane().transform(Matrix4::from_scale(10.0)),
                        cube_at([0.0, 1.0, -4.0], 6.0),
                    ],
                ),
                emissive(1.0, 1.0, 1.0, white),
                emissive(1.0, 0.0, 0.0, vec![cube_at(RED, MARKER_SIZE)]),
            ]),
            xform: Matrix4::from_scale(1.0),
        };
        let game = emissive(1.0, 1.0, 1.0, vec![cube_at(GAME_CUBE, 0.2)]);
        let frame = Frame::with_stand_in_room(
            Frame::with_passthrough(Frame::new(Self::camera(), game)),
            room,
        );
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
            Viewport::new(WINDOW, WINDOW),
            DebugRenderMode::Default,
        );
        self.scene_context.stream_emulated_camera(
            &self.gl,
            "#version 410",
            self.asset_cache.clone(),
            &frame,
            &frame.camera,
            FrameTime { tts: 0.0, dts: 0.0 },
        );
        // What a swap would do: submit, so the readback's fence can signal.
        unsafe { self.gl.flush() };
    }
}
