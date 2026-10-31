//! Automatic frustum culling, end to end on GL:
//!
//! - objects behind the camera draw nothing and count as culled; turning
//!   back draws them again with no upload (culled nodes stay resident);
//! - so does a lightmapped `Model.static`, keeping its pixel textures;
//! - a ring of cubes culls about 3/4, byte-identical to culling off;
//! - stereo culls per eye: an object only the left eye sees draws there only;
//! - a caster outside the view still shadows the visible floor.
//!
//! Self-skipping like `stereo_shadow` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test frustum_cull -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::{vec4, Matrix4, SquareMatrix};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::{gpu_counters, CullCounts};
use functor_runtime_common::math::Angle;
use functor_runtime_common::model::{GroupSource, LightmapSource, ModelData, ModelSource};
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::texture::{ColorSpace, PixelData, PixelEncoding};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData, Light,
    MaterialDescription, MeshData, ModelDescription, ModelHandle, Scene3D, SceneContext,
    SceneObject, Shape, TextureDescription, Viewport,
};
use glow::HasContext;

const EYE: u32 = 128;
const MODEL: &str = "Xbot.glb";

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("frustum_cull: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    behind_the_camera_culls_and_turning_back_uploads_nothing(&mut harness);
    a_static_model_behind_the_camera_culls_and_keeps_its_textures(&mut harness);
    a_ring_culls_most_and_matches_culling_off(&mut harness);
    stereo_culls_per_eye(&mut harness);
    an_offscreen_caster_still_casts_its_shadow(&mut harness);
}

fn behind_the_camera_culls_and_turning_back_uploads_nothing(h: &mut Harness) {
    // 8 Xbots, 8 meshes (one `Arc` each), 8 cubes and an instanced row, all
    // at z = 6; no shadow light, so every test is the eye's.
    let mut items = Vec::new();
    for i in 0..8 {
        let x = i as f32 * 1.5 - 5.25;
        items.push(xbot().translate_x(x).translate_z(6.0));
        items.push(lit(mesh().translate_x(x).translate_y(2.5).translate_z(6.0)));
        items.push(lit(Scene3D::cube()
            .translate_x(x)
            .translate_y(-1.0)
            .translate_z(6.0)));
    }
    let row: Vec<InstanceData> = (0..8)
        .map(|i| InstanceData::at([i as f32 * 1.5 - 5.25, 4.0, 6.0]))
        .collect();
    items.push(Scene3D::instanced(lit(scaled(Scene3D::sphere(), 0.3)), row));
    let scene = group(items);
    let toward = frame(
        Camera::look_at([0.0, 1.0, -6.0], [0.0, 1.0, 6.0], UP, fov()),
        scene.clone(),
        false,
    );
    let away = frame(
        Camera::look_at([0.0, 1.0, -6.0], [0.0, 1.0, -18.0], UP, fov()),
        scene,
        false,
    );
    h.warm(&toward);

    let before = h.render(&toward, &[0], true);
    let tested = gpu_counters().frame_cull().tested;
    // Each Xbot mesh, each mesh, each cube, and the instanced node once.
    assert!(tested > 8 * 3, "{tested}");
    assert_eq!(gpu_counters().frame_cull().culled, 0, "all in view");
    let live = gpu_counters().live();
    gpu_counters().take_window();

    for _ in 0..10 {
        let pixels = h.render(&away, &[0], true);
        assert!(blank(&pixels[0]), "something drew behind the camera");
        assert_eq!(
            gpu_counters().frame_cull(),
            CullCounts {
                tested,
                culled: tested
            }
        );
        assert_eq!(
            gpu_counters().frame_skinned().draws,
            0,
            "skinned meshes drew"
        );
    }
    let after = h.render(&toward, &[0], true);
    let window = gpu_counters().take_window();
    assert_eq!(
        window.bytes_uploaded, 0,
        "turning back re-uploaded: {window:?}"
    );
    let now = gpu_counters().live();
    assert_eq!(
        (now.buffers, now.vaos, now.textures),
        (live.buffers, live.vaos, live.textures)
    );
    assert!(after == before, "turning back changed the image");
    println!("test behind_the_camera_culls_and_turning_back_uploads_nothing ... ok");
}

fn a_static_model_behind_the_camera_culls_and_keeps_its_textures(h: &mut Harness) {
    let scene = static_quad([[-1.0, 0.0, 6.0], [1.0, 0.0, 6.0], [1.0, 2.0, 6.0], [-1.0, 2.0, 6.0]]);
    let toward = frame(
        Camera::look_at([0.0, 1.0, -6.0], [0.0, 1.0, 6.0], UP, fov()),
        scene.clone(),
        false,
    );
    let away = frame(
        Camera::look_at([0.0, 1.0, -6.0], [0.0, 1.0, -18.0], UP, fov()),
        scene,
        false,
    );
    // A few frames first, so the previous test's resources are released.
    for _ in 0..3 {
        h.render(&toward, &[0], true);
    }
    let before = h.render(&toward, &[0], true);
    let one = |culled| CullCounts { tested: 1, culled };
    assert_eq!(gpu_counters().frame_cull(), one(0));
    assert!(!blank(&before[0]), "the static model drew nothing");
    let live = gpu_counters().live();
    gpu_counters().take_window();
    for _ in 0..10 {
        let pixels = h.render(&away, &[0], true);
        assert!(blank(&pixels[0]), "the static model drew behind the camera");
        assert_eq!(gpu_counters().frame_cull(), one(1));
    }
    let after = h.render(&toward, &[0], true);
    let window = gpu_counters().take_window();
    assert_eq!(window.bytes_uploaded, 0, "turning back re-uploaded: {window:?}");
    let now = gpu_counters().live();
    assert_eq!(
        (now.buffers, now.vaos, now.textures),
        (live.buffers, live.vaos, live.textures)
    );
    assert!(after == before, "turning back changed the image");
    println!("test a_static_model_behind_the_camera_culls_and_keeps_its_textures ... ok");
}

fn a_ring_culls_most_and_matches_culling_off(h: &mut Harness) {
    let cubes: Vec<Scene3D> = (0..1000)
        .map(|i| {
            let a = i as f32 / 1000.0 * std::f32::consts::TAU;
            scaled(Scene3D::cube(), 0.3)
                .translate_x(10.0 * a.cos())
                .translate_y(0.15)
                .translate_z(10.0 * a.sin())
        })
        .collect();
    let mut items = vec![lit(group(cubes))];
    items.push(lit(Scene3D::cube()
        .scale_x(24.0)
        .scale_y(0.02)
        .scale_z(24.0)));
    let ring = frame(
        Camera::look_at([0.0, 2.0, 0.0], [0.0, 0.0, 10.0], UP, fov()),
        group(items),
        true,
    );
    let off = h.render(&ring, &[0], false);
    let on = h.render(&ring, &[0], true);
    let counts = gpu_counters().frame_cull();
    assert!(on == off, "culling changed the image");
    // The shadow pass (a 28 m box) keeps all 1,001; the eye culls most of
    // the ring. A 60° view over a ring around the eye sees about 1/4.
    let eye_culled = counts.culled as f32 / 1000.0;
    assert_eq!(counts.tested, 2 * 1001);
    assert!((0.6..0.9).contains(&eye_culled), "{counts:?}");
    println!(
        "test a_ring_culls_most_and_matches_culling_off ... ok ({:.0}% of the ring culled)",
        eye_culled * 100.0
    );
}

fn stereo_culls_per_eye(h: &mut Harness) {
    // Wide eyes so the band only one eye sees is easy to hit.
    let ipd = 0.5;
    let head = Camera::look_at([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], UP, fov());
    let (left, right) = head.stereo_eyes(ipd);
    let ndc_x = |camera: &Camera, x: f32| {
        let clip = camera.projection_matrix(1.0) * camera.view_matrix() * vec4(x, 0.0, 2.0, 1.0);
        clip.x / clip.w
    };
    // A 0.1 cube at depth 2 spans ±0.05 NDC; keep 0.1 clear of each edge.
    let x = (-300..300)
        .map(|i| i as f32 * 0.01)
        .find(|&x| ndc_x(&left, x).abs() < 0.9 && ndc_x(&right, x).abs() > 1.1)
        .expect("a position only the left eye sees");
    let scene = lit(scaled(Scene3D::cube(), 0.1).translate_x(x).translate_z(2.0));
    let mut stereo = frame(head, scene, false);
    stereo.lights = vec![Light::ambient(1.0, 1.0, 1.0)];
    h.ipd = ipd;
    let on = h.render(&stereo, &[0, 1], true);
    assert_eq!(
        gpu_counters().frame_cull(),
        CullCounts {
            tested: 2,
            culled: 1
        }
    );
    let off = h.render(&stereo, &[0, 1], false);
    assert!(on == off, "culling changed a stereo eye");
    assert!(!blank(&on[0]), "the left eye lost its cube");
    assert!(blank(&on[1]), "the right eye drew the cube");
    h.ipd = 0.064;
    println!("test stereo_culls_per_eye ... ok (x = {x:.2})");
}

fn an_offscreen_caster_still_casts_its_shadow(h: &mut Harness) {
    let camera = Camera::look_at([0.0, 3.0, -6.0], [0.0, 0.0, 0.0], UP, fov());
    let floor = lit(Scene3D::cube().scale_x(20.0).scale_y(0.02).scale_z(20.0));
    // Above the top edge of the view; the light is nearly straight down.
    let caster = lit(scaled(Scene3D::cube(), 2.0).translate_y(8.0));
    let with = frame(camera.clone(), group(vec![floor.clone(), caster]), true);
    let without = frame(camera, group(vec![floor]), true);
    let shadowed = h.render(&with, &[0], true);
    let counts = gpu_counters().frame_cull();
    // Shadow pass: floor + caster drawn. Eye: floor drawn, caster culled.
    assert_eq!(
        counts,
        CullCounts {
            tested: 4,
            culled: 1
        }
    );
    assert!(
        shadowed == h.render(&with, &[0], false),
        "culling changed the shadow"
    );
    assert!(
        shadowed != h.render(&without, &[0], true),
        "the caster cast no shadow"
    );
    println!("test an_offscreen_caster_still_casts_its_shadow ... ok");
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// Whether every pixel is the clear color.
fn blank(pixels: &[u8]) -> bool {
    pixels.chunks(4).all(|p| p == &pixels[..4])
}

fn scaled(scene: Scene3D, s: f32) -> Scene3D {
    scene.scale_x(s).scale_y(s).scale_z(s)
}

fn fov() -> Angle {
    Angle::from_degrees(60.0)
}

fn group(items: Vec<Scene3D>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(items),
        xform: Matrix4::identity(),
    }
}

fn lit(scene: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::Lit {
                color: vec4(0.8, 0.6, 0.4, 1.0),
                texture: None,
                normal_map: None,
            },
            vec![scene],
        ),
        xform: Matrix4::identity(),
    }
}

fn xbot() -> Scene3D {
    Scene3D::model(ModelDescription {
        handle: ModelHandle::File(MODEL.to_string()),
        overrides: vec![],
        animation: None,
        while_pending: vec![],
        parts: Vec::new(),
    })
}

/// A `Model.static` quad on `corners`: one group with a pixel texture,
/// lightmapped by a layered one.
fn static_quad(corners: [[f32; 3]; 4]) -> Scene3D {
    let pixels = |space, rgb: [u8; 3]| {
        let pixels = PixelData::new(1, 1, PixelEncoding::Rgb8, space, &rgb);
        TextureDescription::Data(Arc::new(pixels.expect("valid pixels")))
    };
    let data = ModelData::new(ModelSource {
        positions: corners.to_vec(),
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.5, 0.5]; 4],
        uvs2: vec![[0.5, 0.5]; 4],
        joints: vec![],
        weights: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        groups: vec![GroupSource {
            first: 0,
            count: 2,
            texture: Some(pixels(ColorSpace::Srgb, [200, 80, 40])),
            unlit: false,
        }],
        // Layered, so its residency (keyed by the base) is covered too.
        lightmap: Some(LightmapSource {
            texture: TextureDescription::Layered {
                base: Arc::new(
                    PixelData::new(1, 1, PixelEncoding::Rgb8, ColorSpace::Linear, &[255; 3])
                        .expect("valid pixels"),
                ),
                intensities: vec![],
            },
            scale: 1.0,
            floor: [0.0; 3],
        }),
        skeleton: vec![],
        clips: vec![],
        texture: None,
    })
    .expect("valid model");
    Scene3D::model(ModelDescription {
        handle: ModelHandle::Data(Arc::new(data)),
        overrides: vec![],
        animation: None,
        while_pending: vec![],
        parts: Vec::new(),
    })
}

/// A unit tetrahedron with its own `Arc`, so each is a separate GPU mesh.
fn mesh() -> Scene3D {
    let data = MeshData {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    };
    Scene3D {
        obj: SceneObject::Geometry(Shape::Mesh {
            mesh: Arc::new(data),
        }),
        xform: Matrix4::identity(),
    }
}

/// A frame of `scene`, with a shadow-casting sun when `sun`.
fn frame(camera: Camera, scene: Scene3D, sun: bool) -> Frame {
    let mut frame = Frame::new(camera, scene);
    frame.lights = vec![Light::ambient(0.3, 0.3, 0.3)];
    frame.lights.push(Light::Directional {
        direction: [0.05, -1.0, 0.1],
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        casts_shadows: sun,
        baked: false,
    });
    frame
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    shadow_map: ShadowMap,
    frame_id: u64,
    ipd: f32,
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
            .create_window(EYE, EYE, "frustum-cull-test", glfw::WindowMode::Windowed)
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
            ipd: 0.064,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render until the model has streamed in and draws.
    fn warm(&mut self, frame: &Frame) {
        for _ in 0..500 {
            self.render(frame, &[0], true);
            if gpu_counters().frame_skinned().draws > 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("{MODEL} never loaded");
    }

    /// Render `eyes` (0 = left, 1 = right) as one frame id, culling or not,
    /// and read each eye's pixels back.
    fn render(&mut self, frame: &Frame, eyes: &[usize], cull: bool) -> Vec<Vec<u8>> {
        self.frame_id += 1;
        self.scene_context.set_frustum_culling(cull);
        let gl = &self.gl;
        let (left, right) = frame.camera.stereo_eyes(self.ipd);
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
