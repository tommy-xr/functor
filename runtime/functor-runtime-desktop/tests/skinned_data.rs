//! Pins the GPU residency of `Model.skinned` models:
//!
//! - one model drawn by several nodes (plain and instanced) uploads its mesh
//!   once, and steady frames upload nothing;
//! - once no node draws it, its buffers, VAOs (including the instanced
//!   renderer's) and texture are deleted;
//! - it draws, and a pose moves its pixels;
//! - a texture samples at its uvs, plain and instanced, uploaded once.
//!
//! Self-skipping like `stereo_shadow` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test skinned_data -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::anim::AnimExpr;
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::model::{
    ClipSource, JointSource, SkinnedModelData, SkinnedModelSource, TrackSource,
};
use functor_runtime_common::render::VertexPositionTextureSkinned;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData, Light,
    ModelDescription, ModelHandle, Scene3D, SceneContext, SceneObject, TextureDescription,
    Viewport,
};
use glow::HasContext;

const SIZE: u32 = 128;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("skinned_data: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    let arm = Arc::new(SkinnedModelData::new(arm()).expect("valid model"));
    // Vertices, indices, and the 1×1 white base texture.
    let mesh_bytes = arm.source().positions.len()
        * std::mem::size_of::<VertexPositionTextureSkinned>()
        + arm.source().triangles.len() * 12
        + 4;

    // Three plain nodes and an instanced node share one upload.
    let before = gpu_counters().live();
    gpu_counters().take_window();
    let shared = harness.render(&frame(&arm, 3, true, None));
    let first = gpu_counters().take_window();
    assert_eq!(gpu_counters().frame_skinned().draws, 4, "every node draws");
    assert_eq!(
        (first.bytes_uploaded - first.instance_upload_bytes) as usize,
        mesh_bytes,
        "mesh uploaded once"
    );
    assert_eq!((first.cache_misses, first.cache_hits), (1, 3));
    harness.render(&frame(&arm, 3, true, None));
    let steady = gpu_counters().take_window();
    assert_eq!(
        (steady.bytes_uploaded, steady.cache_misses),
        (0, 0),
        "steady frame re-uploaded"
    );
    println!("test one_upload_for_many_nodes ... ok ({mesh_bytes} bytes)");

    // Undrawn, it is evicted: every GL object it created is deleted.
    let resident = gpu_counters().live();
    assert!(resident.buffers > before.buffers && resident.vaos > before.vaos);
    for _ in 0..3 {
        harness.render(&frame(&arm, 0, false, None));
    }
    let after = gpu_counters().live();
    assert_eq!(
        (after.buffers, after.vaos, after.textures),
        (before.buffers, before.vaos, before.textures)
    );
    println!("test evicted_when_unused ... ok");

    // It draws, a pose moves it, and a rebuilt residency draws the same.
    let blank = harness.render(&frame(&arm, 0, false, None));
    let rest = harness.render(&frame(&arm, 1, false, None));
    let bent = harness.render(&frame(
        &arm,
        1,
        false,
        Some(AnimExpr::Clip {
            name: "bend".to_string(),
            playhead: 0.5,
        }),
    ));
    assert!(rest != blank, "the model drew nothing");
    assert!(bent != rest, "the clip did not move the model");
    assert!(shared != blank);
    println!("test draws_and_poses ... ok");

    // A texture samples at the uvs (v = 0 is the image top), on the plain
    // and the instanced path, from one load however many nodes draw it.
    let dir = tempfile::tempdir().expect("temp dir");
    let gif = dir.path().join("skin.gif");
    image::RgbaImage::from_fn(1, 4, |_, y| match y {
        0 | 1 => image::Rgba([255, 0, 0, 255]),
        _ => image::Rgba([0, 0, 255, 255]),
    })
    .save(&gif)
    .expect("write gif");
    let skin = || {
        Arc::new(
            SkinnedModelData::new(SkinnedModelSource {
                uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                texture: Some(TextureDescription::File(gif.to_str().unwrap().to_string())),
                ..self::arm()
            })
            .expect("valid model"),
        )
    };
    let textured = skin();
    for instanced in [false, true] {
        let frame = textured_frame(&textured, instanced);
        gpu_counters().take_window();
        // The file loads asynchronously; the checkerboard stands in until then.
        for _ in 0..200 {
            harness.render(&frame);
            if harness.asset_cache.progress().loaded > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let pixels = harness.render(&frame);
        let window = gpu_counters().take_window();
        assert_eq!(window.instance_upload_bytes > 0, instanced, "instanced path ran");
        // v = 0.25 (world y 1.5) is the red half, v = 0.75 (y 0.5) the blue.
        let [r, g, b] = rgb(&pixels, 64, 78);
        assert!(r > 200 && g < 40 && b < 40, "instanced {instanced}: top {r},{g},{b}");
        let [r, g, b] = rgb(&pixels, 64, 50);
        assert!(r < 40 && g < 40 && b > 200, "instanced {instanced}: bottom {r},{g},{b}");
        if !instanced {
            // Three more models with the same file add no texture.
            let one = gpu_counters().live().textures;
            let models = [textured.clone(), skin(), skin(), skin()];
            harness.render(&lit_frame(
                models
                    .iter()
                    .enumerate()
                    .map(|(i, m)| textured_frame(m, false).scene.translate_x(i as f32 * 0.5))
                    .collect(),
            ));
            assert_eq!(gpu_counters().live().textures, one);
        }
    }
    println!("test textured ... ok");
}

/// Pixel `(x, y)` of a read-back frame (row 0 at the bottom).
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// One node of `model` (instanced as a single copy when `instanced`) under
/// full ambient light, so the albedo reads back unshaded.
fn textured_frame(model: &Arc<SkinnedModelData>, instanced: bool) -> Frame {
    let node = Scene3D::model(ModelDescription {
        handle: ModelHandle::Data(model.clone()),
        overrides: vec![],
        animation: Some(AnimExpr::Rest),
        while_pending: vec![],
    });
    let node = match instanced {
        true => Scene3D::instanced(node, vec![InstanceData::at([0.0, 0.0, 0.0])]),
        false => node,
    };
    lit_frame(vec![node])
}

fn lit_frame(items: Vec<Scene3D>) -> Frame {
    let mut frame = Frame::new(
        camera(),
        Scene3D {
            obj: SceneObject::Group(items),
            xform: cgmath::Matrix4::from_scale(1.0),
        },
    );
    frame.lights = vec![Light::ambient(1.0, 1.0, 1.0)];
    frame
}

fn camera() -> Camera {
    Camera::look_at(
        [0.0, 1.0, -4.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    )
}

/// A 0.4-wide, 2-tall quad: the bottom pair on `upper`, the top pair on
/// `lower` (1 up); `bend` turns `lower` 90° about Z.
fn arm() -> SkinnedModelSource {
    let joint = |name: &str, parent, y| JointSource {
        name: name.to_string(),
        parent,
        translation: [0.0, y, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    let s = std::f32::consts::FRAC_1_SQRT_2;
    SkinnedModelSource {
        positions: vec![
            [-0.2, 0.0, 0.0],
            [0.2, 0.0, 0.0],
            [0.2, 2.0, 0.0],
            [-0.2, 2.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, -1.0]; 4],
        uvs: vec![],
        joints: vec![[0, 0, 0, 0], [0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; 4],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
        skeleton: vec![joint("upper", None, 0.0), joint("lower", Some(0), 1.0)],
        texture: None,
        clips: vec![ClipSource {
            name: "bend".to_string(),
            fps: 2.0,
            frames: 2,
            tracks: vec![TrackSource {
                joint: "lower".to_string(),
                rotations: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, s, s]],
                translations: vec![],
            }],
        }],
    }
}

/// `count` plain nodes of `arm` (plus an instanced node of three when
/// `instanced`), facing a camera 4 units back.
fn frame(
    arm: &Arc<SkinnedModelData>,
    count: usize,
    instanced: bool,
    animation: Option<AnimExpr>,
) -> Frame {
    let node = || {
        Scene3D::model(ModelDescription {
            handle: ModelHandle::Data(arm.clone()),
            overrides: vec![],
            animation: animation.clone(),
            while_pending: vec![],
        })
    };
    let mut items: Vec<Scene3D> = (0..count)
        .map(|i| node().translate_x(i as f32 * 0.6 - 0.6))
        .collect();
    if instanced {
        let copies = (0..3)
            .map(|i| InstanceData::at([i as f32 * 0.6 - 0.6, -1.0, 1.0]))
            .collect::<Vec<_>>();
        items.push(Scene3D::instanced(node(), copies));
    }
    Frame::new(
        camera(),
        Scene3D {
            obj: SceneObject::Group(items),
            xform: cgmath::Matrix4::from_scale(1.0),
        },
    )
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
            .create_window(SIZE, SIZE, "skinned-data-test", glfw::WindowMode::Windowed)
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
