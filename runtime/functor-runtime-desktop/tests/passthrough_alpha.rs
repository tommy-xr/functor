//! Pins the eye-buffer alpha a passthrough frame composites by.
//!
//! On a headset the projection layer blends over the camera feed by source
//! alpha, so the stored alpha decides how much room shows through. This
//! renders the shared `render_frame` path into an RGBA8 target with the
//! headset's transparent clear (`set_passthrough_composited(true)`) and a
//! texture whose texels are alpha 0.5, then reads the alpha back:
//!
//! - opaque textured draws (unlit, lit, emissive) store 1 — the texture's
//!   alpha must not leak the room through them;
//! - a `Scene.opacity(0.5)` subtree stores 0.5 from the constant alone;
//! - a translucent constant-colour material and a translucent 2D sprite
//!   (alpha 0.5) store 0.5 — source-alpha blends still see the real alpha.
//!
//! Self-skipping like `srgb_clear` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test passthrough_alpha -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::vec4;
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, Camera2D, DebugRenderMode, Frame, FrameTime, MaterialDescription,
    Scene3D, SceneContext, SceneObject, SpriteLayer, TextureDescription, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 16;
const TEXTURE: &str = "half-alpha.png";

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("passthrough_alpha: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let harness = Harness::new();
    let texture = || TextureDescription::File(TEXTURE.to_string());
    let textured = |material: MaterialDescription, leaf: Scene3D| Scene3D {
        obj: SceneObject::Material(material, vec![leaf.scale_x(10.0).scale_y(10.0)]),
        xform: cgmath::Matrix4::from_scale(1.0),
    };

    // The plain textured draw also proves the texture loaded: its texels are
    // red, which no fallback is.
    let pixel = harness.render(textured(MaterialDescription::texture(texture()), Scene3D::quad()));
    assert!(pixel[0] > 200 && pixel[1] < 40, "texture not loaded: {pixel:?}");
    assert_eq!(pixel[3], 255, "opaque texture leaked its alpha: {pixel:?}");
    println!("test opaque_texture_writes_alpha_one ... ok");

    for (name, scene) in [
        (
            "lit_texture",
            textured(MaterialDescription::lit_texture(texture()), Scene3D::quad()),
        ),
        (
            "emissive_texture",
            textured(MaterialDescription::emissive_texture(texture()), Scene3D::quad()),
        ),
    ] {
        let pixel = harness.render(scene);
        assert_eq!(pixel[3], 255, "{name}: opaque draw leaked its alpha: {pixel:?}");
        println!("test {name}_writes_alpha_one ... ok");
    }

    let opacity = Scene3D {
        obj: SceneObject::Opacity(
            0.5,
            vec![textured(MaterialDescription::texture(texture()), Scene3D::quad())],
        ),
        xform: cgmath::Matrix4::from_scale(1.0),
    };
    let pixel = harness.render(opacity);
    assert!((pixel[3] as i32 - 128).abs() <= 1, "Scene.opacity(0.5): {pixel:?}");
    println!("test opacity_subtree_writes_its_constant ... ok");

    let translucent = textured(
        MaterialDescription::Color(vec4(1.0, 1.0, 1.0, 0.5)),
        Scene3D::quad(),
    );
    let pixel = harness.render(translucent);
    assert!((pixel[3] as i32 - 128).abs() <= 1, "translucent colour: {pixel:?}");
    println!("test translucent_material_keeps_its_alpha ... ok");

    let sprite = SpriteLayer {
        camera: Camera2D::new(SIZE as f32, SIZE as f32),
        scene: textured(
            MaterialDescription::Color(vec4(1.0, 1.0, 1.0, 0.5)),
            Scene3D::quad().scale_x(10.0).scale_y(10.0),
        ),
    };
    let pixel = harness.render_frame(|frame| frame.sprite_layers.push(sprite));
    assert!((pixel[3] as i32 - 128).abs() <= 1, "translucent sprite: {pixel:?}");
    println!("test translucent_sprite_keeps_its_alpha ... ok");
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
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
            .create_window(SIZE, SIZE, "passthrough-alpha-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };

        // An RGBA8 colour target that STORES alpha (the eye buffer does; a
        // desktop backbuffer may not), plus the depth the forward pass tests.
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

        // Red texels at alpha 0.5, served from memory (no file IO).
        let texels: Vec<u8> = [255u8, 0, 0, 128].repeat(4 * 4);
        let png = functor_runtime_common::frame_capture::encode_rgba_png(4, 4, &texels)
            .expect("encode texture");
        let asset_cache = Arc::new(AssetCache::new());
        asset_cache.replace_uploaded(TEXTURE, png);

        let scene_context = SceneContext::new();
        // What the headset declares while its camera feed is composited.
        scene_context.set_passthrough_composited(true);
        let shadow_map = ShadowMap::new(&gl, 16);
        Harness {
            gl,
            fbo,
            asset_cache,
            scene_context,
            shadow_map,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render `scene` in a passthrough frame and read the centre pixel.
    fn render(&self, scene: Scene3D) -> [u8; 4] {
        self.render_frame(|frame| frame.scene = scene)
    }

    /// Render a passthrough frame (empty scene, edited by `edit`) and read the
    /// centre pixel.
    fn render_frame(&self, edit: impl FnOnce(&mut Frame)) -> [u8; 4] {
        let camera = Camera::look_at(
            [0.0, 0.0, 3.0],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        );
        let mut frame = Frame::new(
            camera,
            Scene3D {
                obj: SceneObject::Group(vec![]),
                xform: cgmath::Matrix4::from_scale(1.0),
            },
        );
        frame.passthrough = true;
        edit(&mut frame);
        let gl = &self.gl;
        // A few frames, so the texture's handle resolves from the warm cache.
        for _ in 0..5 {
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            }
            render_frame(
                gl,
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
        }
        let mut pixel = [0u8; 4];
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                (SIZE / 2) as i32,
                (SIZE / 2) as i32,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
        }
        pixel
    }
}
