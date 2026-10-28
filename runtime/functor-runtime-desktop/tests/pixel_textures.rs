//! Pins `Texture.fromPixels` textures on the GPU:
//!
//! - each encoding uploads the expected texels, in the colour space's format;
//! - `Linear` samples as stored, `Srgb` decodes;
//! - one texture drawn by several nodes uploads once, and is deleted once
//!   no node draws it;
//! - a textured quad shows its texels;
//! - a layered atlas composes base + Σ intensity × layer, regenerates
//!   padding, uploads only changed images, and two compositions in one frame
//!   both draw correctly.
//!
//! Self-skipping like `skinned_data` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test pixel_textures -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::texture::{ColorSpace, Layer, LayeredImage, PixelData, PixelEncoding};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, MaterialDescription,
    Scene3D, SceneContext, SceneObject, TextureDescription, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 128;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("pixel_textures: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    texels_round_trip(&harness);
    println!("test texels_round_trip ... ok");
    color_space_sampling(&mut harness);
    println!("test color_space_sampling ... ok");
    one_upload_for_many_nodes(&mut harness);
    println!("test one_upload_for_many_nodes ... ok");
    textured_quad(&mut harness);
    println!("test textured_quad ... ok");
    layered_texels(&harness);
    println!("test layered_texels ... ok");
    two_compositions_in_one_frame(&mut harness);
    println!("test two_compositions_in_one_frame ... ok");
}

fn pixels(w: u32, h: u32, enc: PixelEncoding, space: ColorSpace, bytes: &[u8]) -> Arc<PixelData> {
    Arc::new(PixelData::new(w, h, enc, space, bytes).expect("valid pixels"))
}

/// Each encoding's level 0 reads back as the expected RGBA8, stored sRGB or
/// linear, with the colour space's wrap, filter and mips.
fn texels_round_trip(harness: &Harness) {
    let rgb555: Vec<u8> = [31u16 | (16 << 10), 1 << 5]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let cases = [
        (
            PixelEncoding::Rgba8,
            vec![1, 2, 3, 4, 250, 128, 0, 77],
            [1, 2, 3, 4, 250, 128, 0, 77],
        ),
        (
            PixelEncoding::Rgb8,
            vec![1, 2, 3, 250, 128, 0],
            [1, 2, 3, 255, 250, 128, 0, 255],
        ),
        (
            PixelEncoding::Rgb555le,
            rgb555,
            [255, 0, 132, 255, 0, 8, 0, 255],
        ),
    ];
    let gl = &harness.gl;
    for (enc, bytes, expected) in cases {
        for space in [ColorSpace::Srgb, ColorSpace::Linear] {
            let data = pixels(2, 1, enc, space, &bytes);
            let texture = harness.scene_context.pixel_texture(gl, &data);
            let mut texels = [0u8; 8];
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.get_tex_image(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut texels)),
                );
                let param = |p| gl.get_tex_parameter_i32(glow::TEXTURE_2D, p) as u32;
                let format = gl.get_tex_level_parameter_i32(
                    glow::TEXTURE_2D,
                    0,
                    glow::TEXTURE_INTERNAL_FORMAT,
                ) as u32;
                let level1 =
                    gl.get_tex_level_parameter_i32(glow::TEXTURE_2D, 1, glow::TEXTURE_WIDTH);
                let srgb = space == ColorSpace::Srgb;
                assert_eq!(texels, expected, "{enc:?} {space:?}");
                assert_eq!(
                    (
                        format,
                        param(glow::TEXTURE_WRAP_S),
                        param(glow::TEXTURE_MIN_FILTER)
                    ),
                    if srgb {
                        (glow::SRGB8_ALPHA8, glow::REPEAT, glow::LINEAR_MIPMAP_LINEAR)
                    } else {
                        (glow::RGBA8, glow::CLAMP_TO_EDGE, glow::LINEAR)
                    },
                    "{enc:?} {space:?}"
                );
                assert_eq!(level1 > 0, srgb, "mips only for Srgb");
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
        }
    }
}

/// A flat 50% grey: `Linear` samples 0.5 (the output encodes it to ~188),
/// `Srgb` decodes 128 to ~0.22 linear (encoded back to ~128).
fn color_space_sampling(harness: &mut Harness) {
    let grey = |space| pixels(1, 1, PixelEncoding::Rgb8, space, &[128, 128, 128]);
    let centre = |image: &[u8]| image[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize];
    let linear = centre(&harness.render(&frame(&[grey(ColorSpace::Linear)])));
    let srgb = centre(&harness.render(&frame(&[grey(ColorSpace::Srgb)])));
    assert!(linear.abs_diff(188) <= 2, "Linear grey drew {linear}");
    assert!(srgb.abs_diff(128) <= 2, "Srgb grey drew {srgb}");
}

/// Five nodes share one upload; steady frames upload nothing; undrawn, the
/// texture is deleted.
fn one_upload_for_many_nodes(harness: &mut Harness) {
    let shared = pixels(4, 4, PixelEncoding::Rgba8, ColorSpace::Linear, &[200; 64]);
    // Warm the quad mesh and let earlier tests' textures expire.
    for _ in 0..3 {
        harness.render(&frame(&[]));
    }
    let before = gpu_counters().live().textures;
    gpu_counters().take_window();
    harness.render(&frame(&vec![shared.clone(); 5]));
    assert_eq!(
        gpu_counters().take_window().bytes_uploaded,
        64,
        "uploaded once"
    );
    assert_eq!(gpu_counters().live().textures, before + 1);
    harness.render(&frame(&vec![shared.clone(); 5]));
    assert_eq!(
        gpu_counters().take_window().bytes_uploaded,
        0,
        "steady frame"
    );
    for _ in 0..3 {
        harness.render(&frame(&[]));
    }
    assert_eq!(
        gpu_counters().live().textures,
        before,
        "evicted when undrawn"
    );
}

/// A 4×4 sRGB texture of four 2×2 blocks on a quad filling the view: each
/// quarter shows its block (sampled at block centres, so filtering mixes in
/// nothing). The quad's v = 0 edge is at the bottom and samples the first
/// (top) row, as file textures do; read_pixels row 0 is the bottom.
fn textured_quad(harness: &mut Harness) {
    let (r, g, b, w) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]);
    let row = |a: [u8; 3], b: [u8; 3]| [a, a, b, b].concat();
    let bytes = [row(r, g), row(r, g), row(b, w), row(b, w)].concat();
    let image = harness.render(&frame(&[pixels(
        4,
        4,
        PixelEncoding::Rgb8,
        ColorSpace::Srgb,
        &bytes,
    )]));
    let at = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        [image[i], image[i + 1], image[i + 2]]
    };
    let (lo, hi) = (SIZE / 4, SIZE * 3 / 4);
    let seen = [at(lo, lo), at(hi, lo), at(lo, hi), at(hi, hi)];
    for (block, (seen, want)) in seen.iter().zip([r, g, b, w]).enumerate() {
        let close = seen.iter().zip(want).all(|(s, w)| s.abs_diff(w) <= 2);
        assert!(close, "block {block}: drew {seen:?}, want {want:?}");
    }
}

/// A 4×4 `Linear` atlas of grey 10 with one 2×1 image at (1, 1), padding 1,
/// carrying layers 1 (grey 100) and 2 (red 40).
fn layered_atlas() -> Arc<PixelData> {
    let base = PixelData::new(4, 4, PixelEncoding::Rgb8, ColorSpace::Linear, &[10; 48]).unwrap();
    let layer = |id: f64, rgb: [u8; 3]| Layer {
        id,
        rgba8: [rgb, rgb].iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
    };
    let image = LayeredImage {
        x: 1,
        y: 1,
        width: 2,
        height: 1,
        padding: 1,
        layers: vec![layer(1.0, [100; 3]), layer(2.0, [40, 0, 0])],
    };
    Arc::new(base.with_layers(vec![image]).expect("valid layers"))
}

/// Bind `atlas` at `intensities` and read its level 0 back.
fn compose(harness: &Harness, atlas: &PixelData, intensities: &[(f64, f32)]) -> Vec<u8> {
    let gl = &harness.gl;
    let texture = harness.scene_context.layered_texture(gl, atlas, intensities);
    let mut texels = vec![0u8; 64];
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.get_tex_image(
            glow::TEXTURE_2D,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut texels)),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    texels
}

/// Intensities 0, 0.5 and 1 give base + 0, 50 and 100 over the image and its
/// regenerated padding (rows 0–2), never row 3; unchanged intensities upload
/// nothing, a change uploads just the padded image; a rewind across a
/// switch restores the texels.
fn layered_texels(harness: &Harness) {
    let atlas = layered_atlas();
    let rgb = |texels: &[u8], x: usize, y: usize| texels[(y * 4 + x) * 4..][..3].to_vec();
    for (level, want) in [(0.0, 10), (0.5, 60), (1.0, 110)] {
        let texels = compose(harness, &atlas, &[(1.0, level)]);
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(rgb(&texels, x, y), [want; 3], "level {level} at {x},{y}");
            }
        }
        assert_eq!(rgb(&texels, 0, 3), [10; 3], "outside the block");
    }
    // Both layers sum per channel; an unknown id is ignored.
    let both = compose(harness, &atlas, &[(1.0, 1.0), (2.0, 0.5), (9.0, 1.0)]);
    assert_eq!(rgb(&both, 1, 1), [130, 110, 110]);

    gpu_counters().take_window();
    compose(harness, &atlas, &[(1.0, 1.0), (2.0, 0.5)]);
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 0, "unchanged");
    let off = compose(harness, &atlas, &[]);
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 4 * 3 * 4, "one padded block");
    // A rewind redraws the earlier intensities; the texels follow exactly.
    assert_eq!(compose(harness, &atlas, &[(1.0, 1.0), (2.0, 0.5)]), both);
    assert_eq!(compose(harness, &atlas, &[]), off);
    assert_eq!(off, atlas.rgba8(), "all off is the base");
}

/// One atlas drawn at intensity 0 (left) and 1 (right) in one frame: each
/// quad shows its own composition.
fn two_compositions_in_one_frame(harness: &mut Harness) {
    let base = PixelData::new(4, 4, PixelEncoding::Rgb8, ColorSpace::Linear, &[0; 48]).unwrap();
    let image = LayeredImage {
        x: 1,
        y: 1,
        width: 2,
        height: 2,
        padding: 1,
        layers: vec![Layer { id: 1.0, rgba8: vec![255; 16] }],
    };
    let atlas = Arc::new(base.with_layers(vec![image]).unwrap());
    let quad = |intensities: Vec<(f64, f32)>, x: f32| {
        let texture = TextureDescription::Layered { base: atlas.clone(), intensities };
        let quad = Scene3D {
            xform: cgmath::Matrix4::from_translation(cgmath::vec3(x, 0.0, 0.0)),
            ..Scene3D::quad()
        };
        Scene3D {
            obj: SceneObject::Material(MaterialDescription::emissive_texture(texture), vec![quad]),
            xform: cgmath::Matrix4::from_scale(1.0),
        }
    };
    let mut frame = frame(&[]);
    frame.scene = Scene3D {
        obj: SceneObject::Group(vec![quad(vec![], -0.8), quad(vec![(1.0, 1.0)], 0.8)]),
        xform: cgmath::Matrix4::from_scale(1.0),
    };
    let image = harness.render(&frame);
    let at = |x: u32| image[((SIZE / 2 * SIZE + x) * 4) as usize];
    let (left, right) = (at(SIZE / 2 - 20), at(SIZE / 2 + 20));
    assert!(left <= 2 && right >= 253, "left {left}, right {right}");
}

/// One full-view emissive quad per texture (all at one place), facing a
/// camera 4 units in front.
fn frame(textures: &[Arc<PixelData>]) -> Frame {
    // The view's height at distance 4 under a 60° field of view.
    let fill = 2.0 * 4.0 * (30f32).to_radians().tan();
    let items = textures
        .iter()
        .map(|t| TextureDescription::Data(t.clone()))
        .map(MaterialDescription::emissive_texture)
        // An off-screen quad keeps the quad mesh drawn.
        .chain([MaterialDescription::emissive(1.0, 1.0, 1.0, 1.0)])
        .enumerate()
        .map(|(i, material)| {
            let quad = Scene3D {
                xform: cgmath::Matrix4::from_scale(fill),
                ..Scene3D::quad()
            };
            let quad = if i < textures.len() {
                quad
            } else {
                quad.translate_x(100.0)
            };
            Scene3D {
                obj: SceneObject::Material(material, vec![quad]),
                xform: cgmath::Matrix4::from_scale(1.0),
            }
        })
        .collect();
    let camera = Camera::look_at(
        [0.0, 0.0, 4.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    );
    Frame::new(
        camera,
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
            .create_window(
                SIZE,
                SIZE,
                "pixel-textures-test",
                glfw::WindowMode::Windowed,
            )
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
