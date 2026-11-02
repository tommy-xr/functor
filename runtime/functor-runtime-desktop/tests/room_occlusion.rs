//! Pins `Frame.withRoomOcclusion`: real surfaces in the environment depth
//! hide the scene behind them, so a headset compositing its camera feed
//! shows the room there.
//!
//! Renders the shared `render_frame` path into an RGBA8 target with the
//! headset's transparent clear, a device depth image of a wall 2 m ahead,
//! and a red quad, then reads the centre pixel's alpha (0 = the room shows).
//!
//! Self-skipping like `passthrough_alpha` (needs a GL display; GLFW on the
//! main thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test room_occlusion -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::{vec4, Matrix4, Point3, Transform};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::environment_depth::{EnvironmentDepthSource, EnvironmentDepthView};
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, DebugRenderMode, Frame, FrameTime, MaterialDescription, Scene3D,
    SceneContext, SceneObject, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 16;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("room_occlusion: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let harness = Harness::new();
    // The camera sits at z = 3, so the wall at distance 2 is the plane z = 1.
    harness.set_wall(Some(2.0));

    let alpha = |z: f32, occlude: bool| harness.render(z, occlude)[3];
    assert_eq!(alpha(0.0, true), 0, "a quad behind the wall must be hidden");
    println!("test behind_the_room_is_hidden ... ok");
    assert_eq!(alpha(0.0, false), 255, "without the opt-in nothing is hidden");
    println!("test occlusion_is_opt_in ... ok");
    assert_eq!(alpha(1.5, true), 255, "a quad in front of the wall must show");
    println!("test in_front_of_the_room_shows ... ok");
    // 5 cm behind the wall, inside the push-back (2 m * 3% + 1 cm = 7 cm).
    assert_eq!(alpha(0.95, true), 255, "the push-back keeps a resting part");
    println!("test resting_on_the_room_shows ... ok");

    // No surface seen (depth 1.0): nothing is hidden.
    harness.set_wall(None);
    assert_eq!(alpha(0.0, true), 255, "no surface must hide nothing");
    println!("test no_surface_hides_nothing ... ok");
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    depth: glow::Texture,
    camera: Camera,
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
            .create_window(SIZE, SIZE, "room-occlusion-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };

        // An RGBA8 colour target that stores alpha, plus a depth buffer.
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

        // The "device" depth image: one layer, filled by `set_wall`.
        let depth = unsafe {
            let texture = gl.create_texture().expect("depth texture");
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.tex_storage_3d(
                glow::TEXTURE_2D_ARRAY,
                1,
                glow::DEPTH_COMPONENT32F,
                SIZE as i32,
                SIZE as i32,
                1,
            );
            functor_runtime_common::environment_depth::set_depth_sampling(&gl, texture);
            texture
        };

        let scene_context = SceneContext::new();
        // What the headset declares while its camera feed is composited.
        scene_context.set_passthrough_composited(true);
        let shadow_map = ShadowMap::new(&gl, 16);
        Harness {
            gl,
            fbo,
            depth,
            camera: Camera::look_at(
                [0.0, 0.0, 3.0],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                Angle::from_degrees(60.0),
            ),
            asset_cache: Arc::new(AssetCache::new()),
            scene_context,
            shadow_map,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Fill the depth image with a wall `distance` ahead (seen from the
    /// render camera), or with no surface (depth 1.0).
    fn set_wall(&self, distance: Option<f32>) {
        let view_projection = self.camera.projection_matrix(1.0) * self.camera.view_matrix();
        let depth = distance.map_or(1.0, |d| {
            let ndc = view_projection.transform_point(Point3::new(0.0, 0.0, 3.0 - d));
            ndc.z * 0.5 + 0.5
        });
        let texels: Vec<u8> = [depth]
            .repeat((SIZE * SIZE) as usize)
            .iter()
            .flat_map(|d| d.to_ne_bytes())
            .collect();
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(self.depth));
            self.gl.tex_sub_image_3d(
                glow::TEXTURE_2D_ARRAY,
                0,
                0,
                0,
                0,
                SIZE as i32,
                SIZE as i32,
                1,
                glow::DEPTH_COMPONENT,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(Some(&texels)),
            );
            self.gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        }
        self.scene_context
            .set_environment_depth_source(EnvironmentDepthSource::Device(EnvironmentDepthView {
                texture: self.depth,
                layer: 0,
                view_projection,
            }));
    }

    /// Render a passthrough frame of a red quad at `z` and read the centre
    /// pixel.
    fn render(&self, z: f32, occlude: bool) -> [u8; 4] {
        let mut leaf = Scene3D::quad().scale_x(10.0).scale_y(10.0);
        leaf.xform = Matrix4::from_translation(cgmath::vec3(0.0, 0.0, z)) * leaf.xform;
        let quad = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::Color(vec4(1.0, 0.0, 0.0, 1.0)),
                vec![leaf],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        let mut frame = Frame::with_passthrough(Frame::new(self.camera.clone(), quad));
        frame.room_occlusion = occlude;
        let gl = &self.gl;
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
