//! Pins `Terrain.colormap` on the GPU:
//!
//! - the terrain shows the day map's texels, unlit (the frame has no lights);
//! - `blend` 1 shows the night map and 0.5 their linear-space mix;
//! - changing `blend` uploads nothing and creates no texture: both maps stay
//!   resident from the first draw.
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
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::terrain::{TerrainColormap, TerrainDetailTexture};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, Scene3D,
    SceneContext, TerrainDescription, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 64;
const DAY: [u8; 3] = [200, 40, 40];
const NIGHT: [u8; 3] = [20, 40, 200];

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
    for (name, rgb) in [("day.png", DAY), ("night.png", NIGHT)] {
        image::RgbaImage::from_pixel(8, 8, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .save(dir.join(name))
            .expect("write colormap");
    }
    let mut harness = Harness::new();
    blend_selects_and_mixes_the_maps_without_uploads(&mut harness, &dir);
    println!("test blend_selects_and_mixes_the_maps_without_uploads ... ok");
    let _ = std::fs::remove_dir_all(&dir);
}

fn blend_selects_and_mixes_the_maps_without_uploads(harness: &mut Harness, dir: &Path) {
    let frame = |blend| terrain_frame(dir, blend);
    // Assets decode asynchronously; draw until the day map shows.
    let mut centre = [0u8; 3];
    for _ in 0..500 {
        centre = harness.centre(&frame(0.0));
        if near(centre, DAY, 2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(near(centre, DAY, 2), "day map, unlit: drew {centre:?}");

    // Settle the first-use uploads (patch list, programs), then switch hours.
    harness.centre(&frame(0.0));
    gpu_counters().take_window();
    let textures = gpu_counters().live().textures;
    let night = harness.centre(&frame(1.0));
    let mid = harness.centre(&frame(0.5));
    let day = harness.centre(&frame(0.0));
    assert!(near(night, NIGHT, 2), "night map: drew {night:?}");
    let expected: [u8; 3] = std::array::from_fn(|c| {
        encode((decode(DAY[c]) + decode(NIGHT[c])) / 2.0)
    });
    assert!(near(mid, expected, 3), "mid blend: drew {mid:?}, expected {expected:?}");
    assert!(near(day, DAY, 2), "back to day: drew {day:?}");
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 0, "an hour switch uploads nothing");
    assert_eq!(gpu_counters().live().textures, textures, "and creates no texture");
}

/// A flat 100 m terrain seen from straight above, shaded only by colormaps.
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
    Frame::new(camera, Scene3D::terrain(terrain))
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= tolerance)
}

fn decode(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(c: f32) -> u8 {
    let c = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
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
