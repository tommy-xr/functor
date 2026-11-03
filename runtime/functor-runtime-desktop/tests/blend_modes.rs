//! GL pins for the blend materials (`Scene.multiply` / `additive` /
//! `reflection`), through the real renderer into an RGBA32F target with no
//! sRGB epilogue, so each pixel is the exact linear value:
//!
//! - multiply is `dst × src` and additive `dst + src`, over an opaque
//!   background;
//! - within a mode, swapping two overlapping panes' order gives identical
//!   pixels, and the nearer pane writes no depth (the farther one still
//!   composes); an opaque wall in front hides a pane;
//! - reflection adds `color × sky × fresnel`, at 0° and 80° to the pane;
//! - hardware-instanced panes match plain ones;
//! - a fully fogged pane leaves the background untouched.
//!
//! Self-skipping like `fog_sky` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test blend_modes -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::{vec4, Deg, Matrix4};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::color_space::srgb_to_linear3;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, DebugRenderMode, Fog, Frame, FrameTime, InstanceData,
    MaterialDescription, OutputColorspace, Scene3D, SceneContext, SceneObject, Viewport,
};
use glow::HasContext;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("blend_modes: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let (gl, _window, _glfw) = gl_context();
    let h = Harness::new(&gl);
    multiply_and_additive_compose_onto_the_background(&h);
    println!("test multiply_and_additive_compose_onto_the_background ... ok");
    order_within_a_mode_does_not_matter(&h);
    println!("test order_within_a_mode_does_not_matter ... ok");
    reflection_follows_the_fresnel_term(&h);
    println!("test reflection_follows_the_fresnel_term ... ok");
    instanced_panes_match_plain_ones(&h);
    println!("test instanced_panes_match_plain_ones ... ok");
    a_fully_fogged_pane_leaves_the_background(&h);
    println!("test a_fully_fogged_pane_leaves_the_background ... ok");
}

const BACKGROUND: [f32; 3] = [0.8, 0.6, 0.4];

fn multiply_and_additive_compose_onto_the_background(h: &Harness) {
    let b = linear(BACKGROUND);
    let m = [0.5, 0.7, 0.9];
    let multiplied = h.pixel(&frame(vec![background(), plain(multiply(m), -2.0, 0.0)]));
    assert_near(multiplied, mul(b, linear(m)), "multiply");
    let a = [0.1, 0.2, 0.3];
    let added = h.pixel(&frame(vec![background(), plain(additive(a), -2.0, 0.0)]));
    assert_near(added, add(b, linear(a)), "additive");
    // Both on one pane: multiply first, whatever the scene order.
    let both = h.pixel(&frame(vec![
        background(),
        plain(additive(a), -2.0, 0.0),
        plain(multiply(m), -2.0, 0.0),
    ]));
    assert_near(both, add(mul(b, linear(m)), linear(a)), "multiply then additive");
    // An opaque wall in front hides a pane: depth testing is on.
    let hidden = h.pixel(&frame(vec![
        background(),
        plain(multiply(m), -5.0, 0.0),
        wall(-3.0),
    ]));
    assert_eq!(hidden, h.pixel(&frame(vec![background(), wall(-3.0)])), "pane behind a wall");
}

fn order_within_a_mode_does_not_matter(h: &Harness) {
    // A white (multiply) or black (additive) background keeps one rounding,
    // so the swapped order is bit-identical. The panes sit at two depths: the
    // farther one composing behind the nearer shows no depth was written.
    for (name, back, near, far) in [
        ("multiply", [1.0; 3], multiply([0.5, 0.7, 0.9]), multiply([0.3, 0.6, 0.8])),
        ("additive", [0.0; 3], additive([0.1, 0.2, 0.3]), additive([0.25, 0.05, 0.15])),
    ] {
        let behind = || colored_background(back);
        let one = h.pixel(&frame(vec![
            behind(),
            plain(near.clone(), -2.0, 0.0),
            plain(far.clone(), -4.0, 0.0),
        ]));
        let other = h.pixel(&frame(vec![
            behind(),
            plain(far.clone(), -4.0, 0.0),
            plain(near.clone(), -2.0, 0.0),
        ]));
        assert_eq!(one, other, "{name}: swapped order");
        let single = h.pixel(&frame(vec![behind(), plain(near.clone(), -2.0, 0.0)]));
        assert_ne!(one, single, "{name}: the farther pane composed");
    }
}

fn reflection_follows_the_fresnel_term(h: &Harness) {
    let b = linear(BACKGROUND);
    let sky = [0.6, 0.7, 0.8];
    let tint = [1.0, 0.9, 0.8];
    let base = 0.04;
    for degrees in [0.0f32, 80.0] {
        let pane = plain(reflection(tint, base), -2.0, degrees);
        let got = h.pixel(&Frame::with_clear_color(
            frame(vec![background(), pane]),
            sky[0],
            sky[1],
            sky[2],
        ));
        // The centre pixel's ray meets the pane's centre, along -z.
        let nv = degrees.to_radians().cos();
        let f = base + (1.0 - base) * (1.0 - nv).powi(5);
        let want = add(b, mul(linear(tint), linear(sky)).map(|c| c * f));
        assert_near(got, want, &format!("reflection at {degrees}°"));
    }
}

fn instanced_panes_match_plain_ones(h: &Harness) {
    for (name, material) in [
        ("multiply", multiply([0.5, 0.7, 0.9])),
        ("additive", additive([0.1, 0.2, 0.3])),
        ("reflection", reflection([1.0, 1.0, 1.0], 0.3)),
    ] {
        let plain_pixel = h.pixel(&frame(vec![background(), plain(material.clone(), -2.0, 30.0)]));
        let instanced = Scene3D::instanced(
            with(material, Scene3D::quad()),
            vec![InstanceData::at([0.0, 0.0, -2.0])
                .rotated(cgmath::Quaternion::from(cgmath::Euler::new(
                    Deg(0.0),
                    Deg(30.0),
                    Deg(0.0),
                )))
                .scaled(4.0)],
        );
        let instanced_pixel = h.pixel(&frame(vec![background(), instanced]));
        assert_near(instanced_pixel, plain_pixel, &format!("{name}: instanced vs plain"));
    }
}

fn a_fully_fogged_pane_leaves_the_background(h: &Harness) {
    let fogged = |items: Vec<Scene3D>| {
        let mut frame = frame(items);
        frame.fog = Some(Fog::linear(1.0, 5.0, 0.3, 0.4, 0.5));
        frame
    };
    let bare = h.pixel(&fogged(vec![background()]));
    for (name, material) in [
        ("multiply", multiply([0.2, 0.2, 0.2])),
        ("additive", additive([0.5, 0.5, 0.5])),
        ("reflection", reflection([1.0, 1.0, 1.0], 1.0)),
    ] {
        let got = h.pixel(&fogged(vec![background(), plain(material.clone(), -6.0, 0.0)]));
        assert_eq!(got, bare, "{name}: fully fogged");
        // Inside the fog it shows.
        let near = h.pixel(&fogged(vec![background(), plain(material, -2.0, 0.0)]));
        assert_ne!(near, bare, "{name}: inside the fog");
    }
}

// ---- Scenes ---------------------------------------------------------------

fn multiply(rgb: [f32; 3]) -> MaterialDescription {
    MaterialDescription::Multiply {
        color: vec4(rgb[0], rgb[1], rgb[2], 1.0),
        texture: None,
    }
}

fn additive(rgb: [f32; 3]) -> MaterialDescription {
    MaterialDescription::Additive {
        color: vec4(rgb[0], rgb[1], rgb[2], 1.0),
        fresnel: None,
    }
}

fn reflection(rgb: [f32; 3], base: f32) -> MaterialDescription {
    MaterialDescription::Additive {
        color: vec4(rgb[0], rgb[1], rgb[2], 1.0),
        fresnel: Some(base),
    }
}

fn with(material: MaterialDescription, child: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::Material(material, vec![child]),
        xform: Matrix4::from_scale(1.0),
    }
}

/// A 4 m quad facing +z at depth `z`, turned `degrees` about y.
fn plain(material: MaterialDescription, z: f32, degrees: f32) -> Scene3D {
    with(
        material,
        Scene3D::quad().transform(
            Matrix4::from_translation(cgmath::vec3(0.0, 0.0, z))
                * Matrix4::from_angle_y(Deg(degrees))
                * Matrix4::from_scale(4.0),
        ),
    )
}

fn colored_background(rgb: [f32; 3]) -> Scene3D {
    plain(MaterialDescription::emissive(rgb[0], rgb[1], rgb[2], 1.0), -10.0, 0.0)
}

fn background() -> Scene3D {
    colored_background(BACKGROUND)
}

fn wall(z: f32) -> Scene3D {
    plain(MaterialDescription::emissive(0.1, 0.9, 0.2, 1.0), z, 0.0)
}

/// Looking down -z from the origin.
fn frame(items: Vec<Scene3D>) -> Frame {
    let camera = Camera::look_at(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(10.0),
    );
    Frame::new(
        camera,
        Scene3D {
            obj: SceneObject::Group(items),
            xform: Matrix4::from_scale(1.0),
        },
    )
}

// ---- Values ---------------------------------------------------------------

fn linear(c: [f32; 3]) -> [f32; 3] {
    srgb_to_linear3(c)
}

fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn assert_near(got: [f32; 3], want: [f32; 3], what: &str) {
    for i in 0..3 {
        assert!(
            (got[i] - want[i]).abs() < 1e-4,
            "{what}: got {got:?}, want {want:?}"
        );
    }
}

// ---- GL -------------------------------------------------------------------

fn gl_context() -> (glow::Context, glfw::PWindow, glfw::Glfw) {
    use glfw::Context as _;
    let mut glfw = glfw::init(glfw::fail_on_errors).expect("init GLFW");
    glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Visible(false));
    let (mut window, _events) = glfw
        .create_window(64, 64, "blend-modes-test", glfw::WindowMode::Windowed)
        .expect("create hidden GLFW window");
    window.make_current();
    let gl =
        unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    (gl, window, glfw)
}

/// A 1×1 RGBA32F colour + depth target and a scene context that writes the
/// shaded linear value (no epilogue encode).
struct Harness<'a> {
    gl: &'a glow::Context,
    fbo: glow::Framebuffer,
    context: SceneContext,
}

impl<'a> Harness<'a> {
    fn new(gl: &'a glow::Context) -> Self {
        let fbo = unsafe {
            let color = gl.create_texture().expect("texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA32F, 1, 1);
            let depth = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, 1, 1);
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
        let context = SceneContext::new();
        context.set_output_colorspace(OutputColorspace::Srgb);
        Self { gl, fbo, context }
    }

    fn pixel(&self, frame: &Frame) -> [f32; 3] {
        let gl = self.gl;
        let shadow_map = ShadowMap::new(gl, 16);
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.enable(glow::DEPTH_TEST);
        }
        render_frame(
            gl,
            "#version 410",
            Arc::new(AssetCache::new()),
            &self.context,
            &shadow_map,
            frame,
            &frame.camera,
            FrameTime { tts: 0.0, dts: 0.0 },
            Viewport::new(1, 1),
            DebugRenderMode::Default,
        );
        let mut pixel = [0f32; 4];
        unsafe {
            gl.read_pixels(
                0,
                0,
                1,
                1,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelPackData::Slice(Some(std::slice::from_raw_parts_mut(
                    pixel.as_mut_ptr() as *mut u8,
                    16,
                ))),
            );
        }
        [pixel[0], pixel[1], pixel[2]]
    }
}
