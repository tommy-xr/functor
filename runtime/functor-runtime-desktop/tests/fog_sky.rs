//! GL pins for height fog and the sky gradient, through the real renderer.
//!
//! - `height_fog_matches_the_integral`: a white unlit billboard under black
//!   fog, rendered into an RGBA32F target without the sRGB epilogue, stores
//!   exactly the shader's `fogVisibility` at that point. It must match a
//!   brute-force quadrature of the height-fog density along the view ray, and
//!   be exactly 0 at and past the fog's end.
//! - `fully_fogged_geometry_is_the_background`: a red wall wholly past the
//!   fog's end renders byte-identical to the frame without it, for linear and
//!   height fog, over the clear colour and over a sky gradient, into both a
//!   shader-encoded RGBA8 target (desktop/web) and an sRGB target with the
//!   hardware encode (Quest's eye swapchain). A wall inside the fog must show.
//!
//! Self-skipping like `srgb_clear` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test fog_sky -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame, Camera, DebugRenderMode, Fog, Frame, FrameTime, MaterialDescription,
    OutputColorspace, Scene3D, SceneContext, SceneObject, Viewport,
};
use glow::HasContext;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("fog_sky: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let (gl, _window, _glfw) = gl_context();
    height_fog_matches_the_integral(&gl);
    println!("test height_fog_matches_the_integral ... ok");
    fully_fogged_geometry_is_the_background(&gl);
    println!("test fully_fogged_geometry_is_the_background ... ok");
}

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
        .create_window(64, 64, "fog-sky-test", glfw::WindowMode::Windowed)
        .expect("create hidden GLFW window");
    window.make_current();
    let gl =
        unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    (gl, window, glfw)
}

/// A colour + depth framebuffer of `format`.
fn target(gl: &glow::Context, format: u32, size: u32) -> glow::Framebuffer {
    unsafe {
        let color = gl.create_texture().expect("texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(color));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, format, size as i32, size as i32);
        let depth = gl.create_renderbuffer().expect("renderbuffer");
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH_COMPONENT24,
            size as i32,
            size as i32,
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
    }
}

fn draw(
    gl: &glow::Context,
    context: &SceneContext,
    fbo: glow::Framebuffer,
    frame: &Frame,
    size: u32,
) {
    let shadow_map = ShadowMap::new(gl, 16);
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        // As every shell does: the sky's far-depth test needs it.
        gl.enable(glow::DEPTH_TEST);
    }
    render_frame(
        gl,
        "#version 410",
        Arc::new(AssetCache::new()),
        context,
        &shadow_map,
        frame,
        &frame.camera,
        FrameTime { tts: 0.0, dts: 0.0 },
        Viewport::new(size, size),
        DebugRenderMode::Default,
    );
}

fn colored(color: [f32; 4], leaf: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::color(color[0], color[1], color[2], color[3]),
            vec![leaf],
        ),
        xform: cgmath::Matrix4::from_scale(1.0),
    }
}

/// `Fog.height`'s (density, falloff, baseY, near, far).
type HeightFog = (f32, f32, f32, f32, f32);

/// Visibility by brute force: the linear end ramp times `exp(-∫ density)`
/// along the ray, the density `density * exp(-falloff * (y - baseY))`.
fn reference_visibility(fog: HeightFog, camera: [f32; 3], point: [f32; 3]) -> f64 {
    let (density, falloff, base, near, far) = fog;
    let c = camera.map(f64::from);
    let p = point.map(f64::from);
    let dist = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
    let ramp = ((far as f64 - dist) / (far - near) as f64).clamp(0.0, 1.0);
    let steps = 200_000;
    let mut depth = 0.0;
    for i in 0..steps {
        let t = (i as f64 + 0.5) / steps as f64;
        let y = c[1] + (p[1] - c[1]) * t;
        depth +=
            density as f64 * (-(falloff as f64) * (y - base as f64)).exp() * dist / steps as f64;
    }
    ramp * (-depth).exp()
}

fn height_fog_matches_the_integral(gl: &glow::Context) {
    let fbo = target(gl, glow::RGBA32F, 1);
    let context = SceneContext::new();
    // No epilogue encode: the float target stores the shaded linear value,
    // mix(fog = 0, surface = 1, visibility) = visibility.
    context.set_output_colorspace(OutputColorspace::Srgb);
    let mist: HeightFog = (0.02, 0.05, 0.0, 100.0, 400.0);
    let camera = [3.0, 30.0, -2.0];
    let cases: [(HeightFog, [f32; 3], [f32; 3]); 11] = [
        (mist, camera, [60.0, 30.0, 40.0]),     // level: the series branch
        (mist, camera, [80.0, 30.05, 9.0]),     // nearly level
        (mist, camera, [180.0, 0.0, 90.0]),     // down into the dense layer
        (mist, camera, [130.0, -40.0, -60.0]),  // below the base height
        (mist, camera, [90.0, 140.0, 70.0]),    // up through thinning fog
        (mist, camera, [20.0, 300.0, 40.0]),    // steeply up
        (mist, camera, [-300.0, 20.0, 200.0]),  // inside the end ramp
        (mist, camera, [250.0, 10.0, 150.0]),   // in the ramp, looking down
        // Thin ground mist seen from far above it: exponents past 80.
        ((1.0, 1.0, 0.0, 100.0, 4000.0), [0.0, 100.0, 0.0], [30.0, 5.0, 0.0]),
        ((0.5, 0.5, 0.0, 100.0, 4000.0), [0.0, 200.0, 0.0], [200.0, 4.0, 50.0]),
        ((0.05, 0.2, 0.0, 100.0, 4000.0), [0.0, 450.0, 0.0], [400.0, 10.0, 0.0]),
    ];
    // The billboard draws: unfogged, it stores 1.
    let unfogged = visibility(gl, &context, fbo, None, camera, cases[0].2);
    assert_eq!(unfogged, 1.0, "unfogged billboard: {unfogged}");
    for (params, camera, point) in cases {
        let (density, falloff, base, near, far) = params;
        let fog = Fog::height(density, falloff, base, near, far, 0.0, 0.0, 0.0);
        let got = visibility(gl, &context, fbo, Some(&fog), camera, point);
        let want = reference_visibility(params, camera, point);
        assert!(
            (got as f64 - want).abs() < 2e-4,
            "height fog {params:?} from {camera:?} at {point:?}: shader {got}, integral {want}"
        );
    }
    // Exactly zero at and past the end, so fully fogged fragments early-out.
    let (density, falloff, base, near, far) = mist;
    let fog = Fog::height(density, falloff, base, near, far, 0.0, 0.0, 0.0);
    let dir = [0.6f32, -0.08, 0.8];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    for distance in [far + 0.01, far * 1.5] {
        let point = [0, 1, 2].map(|i| camera[i] + dir[i] / len * distance);
        let got = visibility(gl, &context, fbo, Some(&fog), camera, point);
        assert_eq!(got, 0.0, "past the end ({distance}): {got}");
    }
}

/// The shaded value of a tiny white billboard at `point`, seen head-on.
fn visibility(
    gl: &glow::Context,
    context: &SceneContext,
    fbo: glow::Framebuffer,
    fog: Option<&Fog>,
    eye: [f32; 3],
    point: [f32; 3],
) -> f32 {
    let mut camera = Camera::look_at(eye, point, [0.0, 1.0, 0.0], Angle::from_degrees(1.0));
    camera.far = 10_000.0;
    let billboard = colored(
        [1.0, 1.0, 1.0, 1.0],
        Scene3D::billboard().transform(
            cgmath::Matrix4::from_translation(cgmath::Vector3::from(point))
                * cgmath::Matrix4::from_scale(0.5),
        ),
    );
    let mut frame = Frame::new(camera, billboard);
    frame.fog = fog.cloned();
    draw(gl, context, fbo, &frame, 1);
    let mut pixel = [0f32; 4];
    unsafe {
        gl.read_pixels(
            0,
            0,
            1,
            1,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelPackData::Slice(Some(bytemuck_slice(&mut pixel))),
        );
    }
    pixel[0]
}

fn bytemuck_slice(pixel: &mut [f32; 4]) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(pixel.as_mut_ptr() as *mut u8, 16) }
}

const SIZE: u32 = 64;

fn fully_fogged_geometry_is_the_background(gl: &glow::Context) {
    let color = (0.62, 0.66, 0.70);
    let fogs = [
        (
            "linear",
            Fog::linear(40.0, 300.0, color.0, color.1, color.2),
        ),
        (
            "height",
            Fog::height(0.01, 0.04, 0.0, 40.0, 300.0, color.0, color.1, color.2),
        ),
    ];
    // Quest's eye swapchain is sRGB with the hardware encode; desktop and
    // web encode in the shader epilogue.
    let surfaces = [
        ("shader-encoded", glow::RGBA8, OutputColorspace::NonSrgb),
        ("srgb", glow::SRGB8_ALPHA8, OutputColorspace::Srgb),
    ];
    for (surface, format, colorspace) in surfaces {
        let fbo = target(gl, format, SIZE);
        let context = SceneContext::new();
        context.set_output_colorspace(colorspace);
        let srgb = colorspace == OutputColorspace::Srgb;
        for (fog_name, fog) in &fogs {
            // (zenith, clear override): the fog must take the sky's horizon
            // even when the clear color differs from the fog's.
            let zenith = [0.18f32, 0.34, 0.62];
            for (sky, clear) in [
                (None, None),
                (Some(zenith), None),
                (Some(zenith), Some([0.30f32, 0.20, 0.40])),
            ] {
                let render = |wall_at: Option<f32>| {
                    let mut camera = Camera::look_at(
                        [0.0, 10.0, 0.0],
                        [0.0, 14.0, 100.0],
                        [0.0, 1.0, 0.0],
                        Angle::from_degrees(70.0),
                    );
                    camera.far = 1000.0;
                    // Wide and tall: it spans the horizon up to ~25°.
                    let wall = wall_at.map(|z| {
                        let xform =
                            cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.0, 60.0, z))
                                * cgmath::Matrix4::from_nonuniform_scale(2000.0, 300.0, 1.0);
                        colored([1.0, 0.0, 0.0, 1.0], Scene3D::cube().transform(xform))
                    });
                    let scene = Scene3D {
                        obj: SceneObject::Group(wall.into_iter().collect()),
                        xform: cgmath::Matrix4::from_scale(1.0),
                    };
                    let mut frame = Frame::with_fog(Frame::new(camera, scene), fog.clone());
                    frame.sky_zenith = sky;
                    frame.clear_color = clear;
                    if srgb {
                        unsafe { gl.enable(glow::FRAMEBUFFER_SRGB) };
                    }
                    draw(gl, &context, fbo, &frame, SIZE);
                    let pixels = read_rgba8(gl);
                    unsafe { gl.disable(glow::FRAMEBUFFER_SRGB) };
                    pixels
                };
                let label = format!(
                    "{surface} / {fog_name} / sky {} / clear override {}",
                    sky.is_some(),
                    clear.is_some()
                );
                let background = render(None);
                let past_end = render(Some(320.0));
                let worst = max_diff(&background, &past_end);
                assert!(worst <= 1, "{label}: fully fogged wall differs by {worst}");
                let inside = render(Some(150.0));
                assert!(
                    max_diff(&background, &inside) > 20,
                    "{label}: a wall inside the fog must show"
                );
                // The horizon is the background colour: the row the camera's level
                // line projects to (the view pitches up ~2.3°, so just below
                // the centre).
                if sky.is_some() {
                    let row = (0..SIZE as usize)
                        .min_by_key(|&row| {
                            let rel = (row as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
                            let pitch = (4.0f32 / 100.0).atan();
                            let level = -pitch.tan() / (35f32.to_radians().tan());
                            ((rel - level).abs() * 1e4) as i32
                        })
                        .unwrap();
                    let at = (row * SIZE as usize + SIZE as usize / 2) * 4;
                    let horizon = clear.unwrap_or([color.0, color.1, color.2]);
                    let fog_bytes = horizon.map(|c| (c * 255.0).round() as i32);
                    for c in 0..3 {
                        let diff = (background[at + c] as i32 - fog_bytes[c]).abs();
                        assert!(
                            diff <= 3,
                            "{label}: horizon {:?} vs fog {fog_bytes:?}",
                            &background[at..at + 3]
                        );
                    }
                }
                println!("  {label}: fully fogged wall max diff {worst}");
            }
        }
    }
}

fn read_rgba8(gl: &glow::Context) -> Vec<u8> {
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

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}
