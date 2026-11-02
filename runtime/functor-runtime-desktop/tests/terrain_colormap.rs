//! Pins `Terrain.colormap` on the GPU:
//!
//! - the terrain shows the day map's texels, unlit, from the very first frame
//!   both maps are ready (a first-use upload must not unbind the other map);
//! - `blend` 1 shows the night map and 0.5 their linear-space mix;
//! - changing `blend` uploads nothing and creates no texture: both maps stay
//!   resident from the first draw;
//! - a flashlight (`Light.spot`) adds the day map × the spot term inside its
//!   cone, and leaves the night map untouched outside it or without a
//!   `Terrain.normalMap`.
//!
//! Self-skipping like `pixel_textures` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test terrain_colormap -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::color_space::{linear_to_srgb, srgb_u8_to_linear};
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::terrain::{TerrainColormap, TerrainDetailTexture};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, Light, Scene3D, SceneObject,
    SceneContext, TerrainDescription, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 64;
const DAY: [u8; 3] = [200, 40, 40];
const NIGHT: [u8; 3] = [20, 40, 200];
/// A terrain-local +Y normal, packed `xyz × 0.5 + 0.5`.
const UP: [u8; 3] = [128, 255, 128];

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("terrain_colormap: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let dir = std::env::temp_dir().join(format!("functor-terrain-colormap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    image::ImageBuffer::<image::Luma<u16>, _>::from_pixel(65, 65, image::Luma([0]))
        .save(dir.join("height.png"))
        .expect("write heightmap");
    for (name, rgb) in [("day.png", DAY), ("night.png", NIGHT), ("up.png", UP)] {
        image::RgbaImage::from_pixel(8, 8, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .save(dir.join(name))
            .expect("write colormap");
    }
    let mut harness = Harness::new();
    blend_selects_and_mixes_the_maps_without_uploads(&mut harness, &dir);
    println!("test blend_selects_and_mixes_the_maps_without_uploads ... ok");
    flashlight_lights_only_its_cone(&mut harness, &dir);
    println!("test flashlight_lights_only_its_cone ... ok");
    let _ = std::fs::remove_dir_all(&dir);
}

fn blend_selects_and_mixes_the_maps_without_uploads(harness: &mut Harness, dir: &Path) {
    let frame = |blend| terrain_frame(dir, blend);
    // Load the heightmap first (lit, no colormap), so the first colormap frame
    // is the one that uploads both maps. Assets decode asynchronously.
    let mut lit = frame(0.0);
    let Scene3D { obj: SceneObject::Terrain(terrain), .. } = &mut lit.scene else {
        unreachable!("terrain_frame draws one terrain")
    };
    terrain.colormap = None;
    let fallback = draw_until_changed(harness, &lit, [0, 0, 0]);
    // Until both maps decode it keeps drawing the lit fallback; the first
    // frame that changes must already be the day map.
    let centre = draw_until_changed(harness, &frame(0.0), fallback);
    assert!(near(centre, DAY, 2), "day map, unlit, on its first frame: drew {centre:?}");

    // Settle the first-use uploads (patch list, programs), then switch hours.
    harness.centre(&frame(0.0));
    gpu_counters().take_window();
    let textures = gpu_counters().live().textures;
    let night = harness.centre(&frame(1.0));
    let mid = harness.centre(&frame(0.5));
    let day = harness.centre(&frame(0.0));
    assert!(near(night, NIGHT, 2), "night map: drew {night:?}");
    let expected: [u8; 3] = std::array::from_fn(|c| {
        let mid = (srgb_u8_to_linear(DAY[c]) + srgb_u8_to_linear(NIGHT[c])) / 2.0;
        (linear_to_srgb(mid) * 255.0).round() as u8
    });
    assert!(near(mid, expected, 3), "mid blend: drew {mid:?}, expected {expected:?}");
    assert!(near(day, DAY, 2), "back to day: drew {day:?}");
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 0, "an hour switch uploads nothing");
    assert_eq!(gpu_counters().live().textures, textures, "and creates no texture");
}

fn flashlight_lights_only_its_cone(harness: &mut Harness, dir: &Path) {
    // Straight down from 10 m with a 20 m range: at the centre the distance
    // falloff is (1 − 10²/20²)² = 0.5625, the cone and N·L are 1.
    let spot = |x: f32| Light::spot(x, 10.0, 0.0, 0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 20.0, 0.3);
    let night = |lights: Vec<Light>, normal_map: bool| {
        let mut frame = terrain_frame(dir, 1.0);
        let Scene3D { obj: SceneObject::Terrain(terrain), .. } = &mut frame.scene else {
            unreachable!("terrain_frame draws one terrain")
        };
        if normal_map {
            terrain.normal_map = Some(TerrainDetailTexture {
                locator: dir.join("up.png").to_string_lossy().into_owned(),
                while_pending: Vec::new(),
            });
        }
        frame.lights.extend(lights);
        frame
    };
    // Wait for the normal map to decode: the frame changes once it lights.
    let lit = draw_until_changed(harness, &night(vec![spot(0.0)], true), NIGHT);
    let expected: [u8; 3] = std::array::from_fn(|c| {
        let linear = srgb_u8_to_linear(NIGHT[c]) + srgb_u8_to_linear(DAY[c]) * 0.5625;
        (linear_to_srgb(linear) * 255.0).round() as u8
    });
    assert!(near(lit, expected, 2), "in the cone: drew {lit:?}, expected {expected:?}");
    // A baked spot is skipped; the next one is the flashlight.
    let baked_first = night(vec![spot(0.0).baked(), spot(0.0)], true);
    assert!(near(harness.centre(&baked_first), expected, 2), "first NON-baked spot");
    for (frame, why) in [
        (night(vec![spot(30.0)], true), "outside the cone"),
        (night(vec![spot(0.0).baked()], true), "only a baked spot"),
        (night(vec![spot(0.0)], false), "no normal map"),
    ] {
        let centre = harness.centre(&frame);
        assert!(near(centre, NIGHT, 2), "{why}: drew {centre:?}");
    }
}

/// A flat 100 m terrain seen from straight above, shaded only by colormaps.
/// The ambient light only lights the fallback, so it differs from both maps.
fn terrain_frame(dir: &Path, blend: f32) -> Frame {
    let map = |name: &str| TerrainDetailTexture {
        locator: dir.join(name).to_string_lossy().into_owned(),
        while_pending: Vec::new(),
    };
    let terrain = TerrainDescription::heightmap(
        dir.join("height.png").to_string_lossy().into_owned(),
        Vec::new(),
        100.0,
        100.0,
        0.0,
        10.0,
    )
    .with_colormap(TerrainColormap {
        day: map("day.png"),
        night: map("night.png"),
        blend,
    });
    let camera = Camera::look_at(
        [0.0, 60.0, 0.01],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    );
    let mut frame = Frame::new(camera, Scene3D::terrain(terrain));
    frame.lights = vec![Light::Ambient {
        color: [0.5, 0.5, 0.5],
        baked: false,
    }];
    frame
}

/// Draw `frame` until its centre pixel differs from `from`.
fn draw_until_changed(harness: &mut Harness, frame: &Frame, from: [u8; 3]) -> [u8; 3] {
    for _ in 0..500 {
        let centre = harness.centre(frame);
        if centre != from {
            return centre;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the frame never changed from {from:?}");
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= tolerance)
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
            .create_window(SIZE, SIZE, "terrain-colormap-test", glfw::WindowMode::Windowed)
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

    /// Render `frame` as a new frame id and read back its centre pixel.
    fn centre(&mut self, frame: &Frame) -> [u8; 3] {
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
        let mut pixel = [0u8; 4];
        unsafe {
            gl.read_pixels(
                (SIZE / 2) as i32,
                (SIZE / 2) as i32,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            assert_eq!(gl.get_error(), glow::NO_ERROR, "GL error after the draw");
        }
        [pixel[0], pixel[1], pixel[2]]
    }
}
