//! Pins `Model.static` models on the GPU:
//!
//! - groups draw with their own base textures, in one model; an `unlit`
//!   group ignores the frame's lights, and a frame without lights draws
//!   every group fullbright;
//! - one model drawn by several nodes uploads once, steady frames upload
//!   nothing, and undrawn it is deleted;
//! - the shadow pass and the instanced (stamped) path draw it.
//!
//! Self-skipping like `skinned_data` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test static_models -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::model::{GroupSource, ModelData, ModelSource};
use functor_runtime_common::render::VertexPositionTextureLightmap;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::texture::{ColorSpace, PixelData, PixelEncoding};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData, Light,
    ModelDescription, ModelHandle, Scene3D, SceneContext, SceneObject, TextureDescription,
    Viewport,
};
use glow::HasContext;

const SIZE: u32 = 128;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("static_models: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    groups(&mut harness);
    println!("test groups ... ok");
    one_upload_for_many_nodes(&mut harness);
    println!("test one_upload_for_many_nodes ... ok");
    shadow_and_instanced(&mut harness);
    println!("test shadow_and_instanced ... ok");
}

/// A 1×1 sRGB texture.
fn solid(rgb: [u8; 3]) -> TextureDescription {
    let pixels = PixelData::new(1, 1, PixelEncoding::Rgb8, ColorSpace::Srgb, &rgb);
    TextureDescription::Data(Arc::new(pixels.expect("valid pixels")))
}

/// Two quads filling the view, facing the camera: the left lit and red,
/// the right unlit and green.
fn room(red: &TextureDescription, green: &TextureDescription) -> ModelSource {
    let quad = |x0: f32, x1: f32| [[x0, -3.0, 0.0], [x1, -3.0, 0.0], [x1, 3.0, 0.0], [x0, 3.0, 0.0]];
    ModelSource {
        positions: [quad(-3.0, 0.0), quad(0.0, 3.0)].concat(),
        normals: vec![[0.0, 0.0, 1.0]; 8],
        uvs: vec![[0.5, 0.5]; 8],
        uvs2: vec![],
        joints: vec![],
        weights: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        groups: vec![
            GroupSource {
                first: 0,
                count: 2,
                texture: Some(red.clone()),
                unlit: false,
            },
            GroupSource {
                first: 2,
                count: 2,
                texture: Some(green.clone()),
                unlit: true,
            },
        ],
        skeleton: vec![],
        clips: vec![],
        texture: None,
    }
}

fn model(source: ModelSource) -> Arc<ModelData> {
    Arc::new(ModelData::new(source).expect("valid model"))
}

/// Each group shows its own texture; only the lit one dims under a dim
/// ambient light, and neither in a frame without lights.
fn groups(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let room = model(room(&red, &green));
    // sRGB 0.5 is ~0.21 linear: the lit red texel encodes back to ~128.
    let dim = harness.render(&frame(&room, 1, false, vec![Light::ambient(0.5, 0.5, 0.5)]));
    assert_near(rgb(&dim, 32, 64), [128, 0, 0], "lit group, dim ambient");
    assert_near(rgb(&dim, 96, 64), [0, 255, 0], "unlit group");
    let unlit_frame = harness.render(&frame(&room, 1, false, vec![]));
    assert_near(rgb(&unlit_frame, 32, 64), [255, 0, 0], "no lights: fullbright");
    // No groups: one untextured (white) lit group.
    let plain = model(ModelSource {
        groups: vec![],
        ..self::room(&red, &green)
    });
    let white = harness.render(&frame(&plain, 1, false, vec![Light::ambient(1.0, 1.0, 1.0)]));
    assert_near(rgb(&white, 96, 64), [255, 255, 255], "no groups: white");
}

/// Five nodes share one upload of the mesh; steady frames upload nothing;
/// undrawn, its buffers and VAO are deleted.
fn one_upload_for_many_nodes(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let lights = || vec![Light::ambient(1.0, 1.0, 1.0)];
    // Warm the textures with one model, so the next frame uploads only the
    // second model's mesh.
    harness.render(&frame(&model(room(&red, &green)), 1, false, lights()));
    let before = gpu_counters().live();
    let room = model(room(&red, &green));
    let mesh_bytes = room.source().positions.len()
        * std::mem::size_of::<VertexPositionTextureLightmap>()
        + room.source().triangles.len() * 12;
    gpu_counters().take_window();
    harness.render(&frame(&room, 5, false, lights()));
    let first = gpu_counters().take_window();
    assert_eq!(first.bytes_uploaded as usize, mesh_bytes, "mesh uploaded once");
    harness.render(&frame(&room, 5, false, lights()));
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 0, "steady frame");
    for _ in 0..3 {
        harness.render(&frame(&room, 0, false, lights()));
    }
    let after = gpu_counters().live();
    // The warm-up model was evicted too, so at most `before`'s objects remain.
    assert!(
        after.buffers + 2 <= before.buffers && after.vaos < before.vaos,
        "evicted: {before:?} -> {after:?}"
    );
}

/// A static occluder off-screen (between the sun and the room) shadows the
/// room through the depth pass, and an instanced copy (stamped) draws what
/// a plain node does.
fn shadow_and_instanced(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let room = model(room(&red, &green));
    // Seen from the camera it lies left of the view; the sun, travelling
    // +x and -z, casts it onto the room at x ≈ -2.4 … -1.4.
    let occluder = model(ModelSource {
        positions: vec![[-3.5, -1.0, 1.5], [-2.5, -1.0, 1.5], [-2.5, 1.0, 1.5], [-3.5, 1.0, 1.5]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        groups: vec![],
        ..self::room(&red, &green)
    });
    let sun = vec![
        Light::ambient(0.2, 0.2, 0.2),
        Light::directional(0.5, 0.0, -1.0, 1.0, 1.0, 1.0, 1.0).cast_shadows(),
    ];
    let mut shadowed = frame(&room, 1, false, sun.clone());
    let SceneObject::Group(items) = &mut shadowed.scene.obj else {
        unreachable!()
    };
    items.push(frame(&occluder, 1, false, vec![]).scene);
    let lit = harness.render(&frame(&room, 1, false, sun.clone()));
    let dark = harness.render(&shadowed);
    // World x = -1.9 on the room plane is pixel 11; x = -0.5 is pixel 50.
    let (lit_r, dark_r) = (rgb(&lit, 11, 64)[0], rgb(&dark, 11, 64)[0]);
    assert!(lit_r > 200 && dark_r < 140, "shadow: lit {lit_r}, shadowed {dark_r}");
    assert_eq!(rgb(&dark, 50, 64), rgb(&lit, 50, 64), "outside the shadow");
    let instanced = harness.render(&frame(&room, 1, true, sun));
    assert!(lit == instanced, "the stamped copy differs");
}

/// Pixel `(x, y)` of a read-back frame (row 0 at the bottom).
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn assert_near(seen: [u8; 3], want: [u8; 3], what: &str) {
    let close = seen.iter().zip(want).all(|(s, w)| s.abs_diff(w) <= 3);
    assert!(close, "{what}: drew {seen:?}, want {want:?}");
}

/// `nodes` nodes of `model` at the origin (one instanced copy when
/// `instanced`), seen from 4 units in front.
fn frame(model: &Arc<ModelData>, nodes: usize, instanced: bool, lights: Vec<Light>) -> Frame {
    let node = || {
        Scene3D::model(ModelDescription {
            handle: ModelHandle::Data(model.clone()),
            overrides: vec![],
            animation: None,
            while_pending: vec![],
        })
    };
    let items = (0..nodes)
        .map(|_| match instanced {
            true => Scene3D::instanced(node(), vec![InstanceData::at([0.0, 0.0, 0.0])]),
            false => node(),
        })
        .collect();
    let camera = Camera::look_at(
        [0.0, 0.0, 4.0],
        [0.0, 0.0, 0.0],
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
    frame.lights = lights;
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
            .create_window(SIZE, SIZE, "static-models-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };
        let fbo = unsafe {
            let color = gl.create_texture().expect("texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, SIZE as i32, SIZE as i32);
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
        let shadow_map = ShadowMap::new(&gl, 512);
        Harness {
            gl,
            fbo,
            asset_cache: Arc::new(AssetCache::new()),
            scene_context: SceneContext::new(),
            shadow_map,
            frame_id: 0,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render `frame` as a new frame id and read its pixels back.
    fn render(&mut self, frame: &Frame) -> Vec<u8> {
        self.frame_id += 1;
        let gl = &self.gl;
        let camera = &frame.camera;
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
            camera,
            &[view_projection],
            projection.y.y.abs(),
            SIZE as f32,
            self.frame_id,
            None,
            FrameTime { tts: 1.0, dts: 0.0 },
            Viewport::with_offset(0, 0, SIZE, SIZE),
            DebugRenderMode::Default,
        );
        let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
        unsafe {
            gl.read_pixels(
                0,
                0,
                SIZE as i32,
                SIZE as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        pixels
    }
}
