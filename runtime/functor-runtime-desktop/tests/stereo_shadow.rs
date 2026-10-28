//! Pins the stereo render cost of skinned models:
//!
//! - two `render_frame_with_projection` calls sharing a frame id (one per eye)
//!   render the main shadow map once: each skinned mesh draws 3× (shadow +
//!   two eyes), not 4×;
//! - the reused map is exact: the second eye matches a fresh render of the
//!   same eye with its own shadow pass, byte for byte;
//! - pass-constant uniforms (lights, fog, view, projection) upload once per
//!   program per pass: an extra skinned node adds only its per-draw uniforms.
//!
//! Self-skipping like `passthrough_alpha` (needs a GL display; GLFW on the
//! main thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test stereo_shadow -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, Light,
    MaterialDescription, ModelDescription, ModelHandle, Scene3D, SceneContext, SceneObject,
    Viewport,
};
use glow::HasContext;

// One eye; the target holds both side by side.
const EYE: u32 = 128;
const MODEL: &str = "Xbot.glb";

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("stereo_shadow: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    harness.warm(&scene_frame(1));

    // Mono call: shadow + forward per mesh. Stereo pair: one shadow, two eyes.
    let one = scene_frame(1);
    harness.render(&one, &[0]);
    let mono = gpu_counters().frame_skinned();
    harness.render(&one, &[0, 1]);
    let stereo = gpu_counters().frame_skinned();
    let meshes = mono.draws / 2;
    assert!(meshes > 0, "model never drew: {mono:?}");
    assert_eq!(
        stereo.draws,
        3 * meshes,
        "stereo pair re-rendered the shadow map"
    );
    assert_eq!(
        stereo.pose_evals, 1,
        "pose evaluated more than once per frame"
    );
    println!(
        "test stereo_pair_renders_one_shadow_pass ... ok ({} draws)",
        stereo.draws
    );

    // Exact reuse: the right eye after the left eye's map == the right eye
    // rendering its own map.
    let reused = harness.render(&one, &[0, 1])[1].clone();
    let fresh = harness.render(&one, &[1])[0].clone();
    assert!(
        reused == fresh,
        "reused shadow map differs from a fresh one"
    );
    println!("test reused_shadow_map_is_exact ... ok");

    // Each extra node adds only per-draw uniforms: world + joints in the
    // shadow pass, world + normalMatrix + joints + output per eye.
    harness.render(&one, &[0, 1]);
    let one_node = gpu_counters().frame_uniform_uploads();
    harness.render(&scene_frame(2), &[0, 1]);
    let two_nodes = gpu_counters().frame_uniform_uploads();
    println!("uniform uploads per stereo frame: 1 node {one_node}, 2 nodes {two_nodes}");
    assert_eq!(two_nodes - one_node, (2 + 2 * 4) * meshes);
    println!("test pass_constants_upload_once_per_pass ... ok");
}

/// `count` Xbots on a lit floor under a shadow-casting sun.
fn scene_frame(count: usize) -> Frame {
    let model = |i: usize| {
        Scene3D::model(ModelDescription {
            handle: ModelHandle::File(MODEL.to_string()),
            overrides: vec![],
            animation: None,
            while_pending: vec![],
        })
        .translate_x(i as f32 * 1.2 - 0.6)
    };
    let floor = Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::Lit {
                color: cgmath::vec4(0.8, 0.8, 0.8, 1.0),
                texture: None,
                normal_map: None,
            },
            vec![Scene3D::cube().scale_x(8.0).scale_y(0.02).scale_z(8.0)],
        ),
        xform: cgmath::Matrix4::from_scale(1.0),
    };
    let mut items: Vec<Scene3D> = (0..count).map(model).collect();
    items.push(floor);
    let camera = Camera::look_at(
        [0.0, 1.5, -4.0],
        [0.0, 0.9, 0.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    );
    let mut frame = Frame::new(
        camera,
        Scene3D {
            obj: SceneObject::Group(items),
            xform: cgmath::Matrix4::from_scale(1.0),
        },
    );
    frame.lights = vec![
        Light::ambient(0.2, 0.2, 0.2),
        Light::Directional {
            direction: [0.4, -1.0, 0.3],
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            casts_shadows: true,
        },
    ];
    frame
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    shadow_map: ShadowMap,
    frame_id: u64,
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
            .create_window(EYE, EYE, "stereo-shadow-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };

        let fbo = unsafe {
            let color = gl.create_texture().expect("texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_storage_2d(
                glow::TEXTURE_2D,
                1,
                glow::RGBA8,
                (2 * EYE) as i32,
                EYE as i32,
            );
            let depth = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                (2 * EYE) as i32,
                EYE as i32,
            );
            let fbo = gl.create_framebuffer().expect("fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            assert_eq!(
                gl.check_framebuffer_status(glow::FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE
            );
            fbo
        };

        let model = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/crowd/Xbot.glb");
        let asset_cache = Arc::new(AssetCache::new());
        asset_cache.replace_uploaded(MODEL, std::fs::read(model).expect("read Xbot.glb"));

        let shadow_map = ShadowMap::new(&gl, 512);
        Harness {
            gl,
            fbo,
            asset_cache,
            scene_context: SceneContext::new(),
            shadow_map,
            frame_id: 0,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render until the model has streamed in and draws.
    fn warm(&mut self, frame: &Frame) {
        for _ in 0..500 {
            self.render(frame, &[0]);
            if gpu_counters().frame_skinned().draws > 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("{MODEL} never loaded");
    }

    /// Render `eyes` (0 = left, 1 = right) as one frame id and read each
    /// eye's pixels back.
    fn render(&mut self, frame: &Frame, eyes: &[usize]) -> Vec<Vec<u8>> {
        self.frame_id += 1;
        let gl = &self.gl;
        let (left, right) = frame.camera.stereo_eyes(0.064);
        let cameras = [left, right];
        for &eye in eyes {
            let camera = &cameras[eye];
            let projection = camera.projection_matrix(1.0);
            let view_projection = projection * camera.view_matrix();
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
                gl.disable(glow::SCISSOR_TEST);
                gl.enable(glow::DEPTH_TEST);
            }
            render_frame_with_projection(
                gl,
                "#version 410",
                self.asset_cache.clone(),
                &self.scene_context,
                &self.shadow_map,
                frame,
                camera,
                &projection,
                &frame.camera,
                &[view_projection],
                projection.y.y.abs(),
                EYE as f32,
                self.frame_id,
                None,
                FrameTime { tts: 1.0, dts: 0.0 },
                Viewport::with_offset(eye as u32 * EYE, 0, EYE, EYE),
                DebugRenderMode::Default,
            );
        }
        eyes.iter()
            .map(|&eye| {
                let mut pixels = vec![0u8; (EYE * EYE * 4) as usize];
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
                    gl.read_pixels(
                        (eye as u32 * EYE) as i32,
                        0,
                        EYE as i32,
                        EYE as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                }
                pixels
            })
            .collect()
    }
}
