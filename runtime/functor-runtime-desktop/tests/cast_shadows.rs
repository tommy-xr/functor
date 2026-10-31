//! Pins `Scene.castShadows`: an opted-out subtree draws nothing into the
//! shadow map, yet still renders and receives shadows.
//!
//! - two identical cubes under one sun, one opted out: only one shadow, and
//!   the opted-out cube's top is shadowed by a third caster;
//! - `true` inside `false` casts (nearest wins);
//! - instanced (wrapped, or inside the template), memoised and skinned
//!   nodes opt out; skinned shadow-pass draws drop;
//! - with shadows off, wrapping changes no pixel.
//!
//! Self-skipping like `stereo_shadow` (needs a GL display; GLFW on the main
//! thread, so `harness = false`). `CAST_SHADOWS_PNG=<dir>` saves the main
//! scene from an oblique camera, opted out and casting:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test cast_shadows -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::{vec3, Matrix4, Vector4};
use functor_runtime_common::anim::AnimExpr;
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::model::{JointSource, ModelData, ModelSource};
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData, Light,
    MaterialDescription, ModelDescription, ModelHandle, Scene3D, SceneContext, SceneObject,
    Viewport,
};
use glow::HasContext;

const SIZE: u32 = 256;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("cast_shadows: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();

    // The scene: cube A casts, cube B opts out, slab C shadows B's top.
    let pixels = harness.render(&frame(vec![
        floor(),
        cube_a(),
        cast(false, cube_b()),
        slab(),
    ]));
    if let Ok(dir) = std::env::var("CAST_SHADOWS_PNG") {
        save_media(&mut harness, &dir);
    }
    let open = luma(&pixels, OPEN);
    assert!(luma(&pixels, A_SHADOW) < 0.7 * open, "A cast no shadow");
    assert!(luma(&pixels, B_SHADOW) > 0.95 * open, "opted-out B cast a shadow");
    let (a_top, b_top) = (luma(&pixels, A_TOP), luma(&pixels, B_TOP));
    assert!(b_top < 0.7 * a_top, "B received no shadow: top {b_top} vs A {a_top}");
    // Control: B's spot IS shadowed when B casts.
    let casting = harness.render(&frame(vec![floor(), cube_a(), cube_b(), slab()]));
    assert!(luma(&casting, B_SHADOW) < 0.7 * open, "B's shadow probe misses");
    println!("test one_shadow_and_opted_out_receives ... ok");

    // Nearest wins: `true` inside `false` casts; `false` inside `true` doesn't.
    let shadowed = |harness: &mut Harness, a: Scene3D| {
        let pixels = harness.render(&frame(vec![floor(), a]));
        luma(&pixels, A_SHADOW) < 0.7 * luma(&pixels, OPEN)
    };
    assert!(shadowed(&mut harness, cast(false, cast(true, cube_a()))));
    assert!(!shadowed(&mut harness, cast(true, cast(false, cube_a()))));
    println!("test nearest_enclosing_wins ... ok");

    // Instanced: wrapping the node, or inside the template; nearest wins.
    let instanced = |template: Scene3D| {
        Scene3D::instanced(template, vec![InstanceData::at([0.0, 0.5, -2.0])])
    };
    let lit_cube = || lit(Scene3D::cube());
    assert!(shadowed(&mut harness, instanced(lit_cube())));
    assert!(!shadowed(&mut harness, cast(false, instanced(lit_cube()))));
    assert!(!shadowed(&mut harness, instanced(cast(false, lit_cube()))));
    assert!(shadowed(&mut harness, cast(false, instanced(cast(true, lit_cube())))));
    // A CPU-expanded template (two children) honours it too.
    let pair = || Scene3D {
        obj: SceneObject::Group(vec![lit_cube(), lit_cube()]),
        xform: Matrix4::from_scale(1.0),
    };
    assert!(shadowed(&mut harness, instanced(pair())));
    assert!(!shadowed(&mut harness, instanced(cast(false, pair()))));
    println!("test instanced ... ok");

    // Memoised (`Scene.memo` shares the subtree by reference).
    let memo = Scene3D {
        obj: SceneObject::Shared(Arc::new(cast(false, cube_a()).obj)),
        xform: Matrix4::from_scale(1.0),
    };
    assert!(!shadowed(&mut harness, memo));
    println!("test memoised ... ok");

    // Skinned, plain and instanced: one shadow draw + one forward draw per
    // mesh, and only the forward draw when opted out.
    let arm = Arc::new(ModelData::new(arm()).expect("valid model"));
    let model = || {
        Scene3D::model(ModelDescription {
            handle: ModelHandle::Data(arm.clone()),
            overrides: vec![],
            animation: Some(AnimExpr::Rest),
            while_pending: vec![],
        })
    };
    let draws = |harness: &mut Harness, node: Scene3D| {
        harness.render(&frame(vec![floor(), node]));
        gpu_counters().frame_skinned().draws
    };
    let one = || vec![InstanceData::at([0.0, 0.0, 0.0])];
    assert_eq!(draws(&mut harness, model()), 2);
    assert_eq!(draws(&mut harness, cast(false, model())), 1);
    assert_eq!(draws(&mut harness, Scene3D::instanced(model(), one())), 2);
    assert_eq!(draws(&mut harness, cast(false, Scene3D::instanced(model(), one()))), 1);
    assert_eq!(draws(&mut harness, Scene3D::instanced(cast(false, model()), one())), 1);
    println!("test skinned_shadow_draws_drop ... ok");

    // With no shadow-casting light the wrapper is invisible: same pixels.
    let unlit_sun = |items| {
        let mut frame = frame(items);
        frame.lights = vec![Light::ambient(0.2, 0.2, 0.2), sun(false)];
        frame
    };
    let plain = harness.render(&unlit_sun(vec![floor(), cube_a(), cube_b(), slab()]));
    let wrapped = harness.render(&unlit_sun(vec![
        floor(),
        cast(false, cube_a()),
        cast(true, cube_b()),
        cast(false, slab()),
    ]));
    assert!(plain == wrapped, "castShadows changed the forward pass");
    println!("test forward_pass_unchanged ... ok");
}

// World probes: open floor, the floor where A's / B's shadow falls (clear of
// C's), and matching points on A's and B's tops (B's under C's shadow).
const OPEN: [f32; 3] = [3.0, 0.01, 0.0];
const A_SHADOW: [f32; 3] = [0.75, 0.01, -1.65];
const B_SHADOW: [f32; 3] = [0.75, 0.01, 1.65];
const A_TOP: [f32; 3] = [0.3, 1.0, -2.0];
const B_TOP: [f32; 3] = [0.3, 1.0, 2.0];

fn cast(casts: bool, scene: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::CastShadows(casts, vec![scene]),
        xform: Matrix4::from_scale(1.0),
    }
}

fn lit(scene: Scene3D) -> Scene3D {
    colored(scene, [0.8, 0.8, 0.8])
}

fn colored(scene: Scene3D, [r, g, b]: [f32; 3]) -> Scene3D {
    Scene3D {
        obj: SceneObject::Material(MaterialDescription::lit(r, g, b, 1.0), vec![scene]),
        xform: Matrix4::from_scale(1.0),
    }
}

/// A lit unit cube scaled by `size` and centred at `at`.
fn box_at(at: [f32; 3], size: [f32; 3]) -> Scene3D {
    placed(lit(Scene3D::cube()), at, size)
}

fn placed(scene: Scene3D, at: [f32; 3], size: [f32; 3]) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(vec![scene]),
        xform: Matrix4::from_translation(vec3(at[0], at[1], at[2]))
            * Matrix4::from_nonuniform_scale(size[0], size[1], size[2]),
    }
}

/// PR media: B opts out (or casts) while a pillar upwind shadows the lower
/// half of B's lit face. A low sun keeps the pillar's own
/// ground shadow inside B's footprint, so B's shadow reads on its own.
fn save_media(harness: &mut Harness, dir: &str) {
    let orange = |at| placed(colored(Scene3D::cube(), [1.0, 0.5, 0.1]), at, [1.0; 3]);
    let pillar = placed(
        colored(Scene3D::cube(), [0.1, 0.7, 1.0]),
        [-2.0, 0.8, 2.0],
        [0.3, 1.6, 1.2],
    );
    for (name, b) in [
        ("opted-out", cast(false, orange([0.0, 0.5, 2.0]))),
        ("casting", orange([0.0, 0.5, 2.0])),
    ] {
        let mut view = frame(vec![floor(), b, pillar.clone()]);
        view.lights = vec![
            Light::ambient(0.3, 0.3, 0.3),
            Light::Directional {
                direction: [1.0, -0.7, 0.0],
                color: [1.0, 1.0, 1.0],
                intensity: 1.0,
                casts_shadows: true,
                baked: false,
            },
        ];
        view.camera = Camera::look_at(
            [-3.0, 4.0, -1.5],
            [0.6, 0.2, 2.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        );
        save_png(&harness.render(&view), &format!("{dir}/{name}.png"));
    }
}

fn floor() -> Scene3D {
    box_at([0.0, 0.0, 0.0], [10.0, 0.02, 10.0])
}

fn cube_a() -> Scene3D {
    box_at([0.0, 0.5, -2.0], [1.0, 1.0, 1.0])
}

fn cube_b() -> Scene3D {
    box_at([0.0, 0.5, 2.0], [1.0, 1.0, 1.0])
}

/// A thin slab above B: its shadow lands on B's top at x 0.1..0.5.
fn slab() -> Scene3D {
    box_at([-0.2, 2.0, 2.0], [0.4, 0.1, 0.4])
}

fn sun(casts_shadows: bool) -> Light {
    Light::Directional {
        direction: [1.0, -2.0, 0.0],
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        casts_shadows,
        baked: false,
    }
}

fn camera() -> Camera {
    Camera::look_at(
        [0.0, 12.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        Angle::from_degrees(60.0),
    )
}

fn frame(items: Vec<Scene3D>) -> Frame {
    let mut frame = Frame::new(
        camera(),
        Scene3D {
            obj: SceneObject::Group(items),
            xform: Matrix4::from_scale(1.0),
        },
    );
    frame.lights = vec![Light::ambient(0.2, 0.2, 0.2), sun(true)];
    frame
}

/// Mean RGB of the read-back pixel `world` projects to.
fn luma(pixels: &[u8], world: [f32; 3]) -> f32 {
    let camera = camera();
    let clip = camera.projection_matrix(1.0)
        * camera.view_matrix()
        * Vector4::new(world[0], world[1], world[2], 1.0);
    let to_pixel = |ndc: f32| ((ndc * 0.5 + 0.5) * SIZE as f32) as u32;
    let (x, y) = (to_pixel(clip.x / clip.w), to_pixel(clip.y / clip.w));
    let i = ((y * SIZE + x) * 4) as usize;
    (pixels[i] as f32 + pixels[i + 1] as f32 + pixels[i + 2] as f32) / 3.0
}

fn save_png(pixels: &[u8], path: &str) {
    // Read-back rows start at the bottom.
    let image = image::RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let i = (((SIZE - 1 - y) * SIZE + x) * 4) as usize;
        image::Rgba([pixels[i], pixels[i + 1], pixels[i + 2], 255])
    });
    image.save(path).expect("write png");
    println!("saved {path}");
}

/// A 0.4-wide, 2-tall upright quad on a one-joint rig.
fn arm() -> ModelSource {
    ModelSource {
        positions: vec![
            [-0.2, 0.0, 0.0],
            [0.2, 0.0, 0.0],
            [0.2, 2.0, 0.0],
            [-0.2, 2.0, 0.0],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        uvs: vec![],
        uvs2: vec![],
        joints: vec![[0, 0, 0, 0]; 4],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; 4],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
        groups: vec![],
        lightmap: None,
        skeleton: vec![JointSource {
            name: "root".to_string(),
            parent: None,
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }],
        texture: None,
        clips: vec![],
    }
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
            .create_window(SIZE, SIZE, "cast-shadows-test", glfw::WindowMode::Windowed)
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
        let shadow_map = ShadowMap::new(&gl, 2048);
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
