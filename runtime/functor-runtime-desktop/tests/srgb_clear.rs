//! Native pin for the gamma pipeline's driver-divergent spec corner: clearing
//! an sRGB attachment.
//!
//! The renderer decodes authored clear colors to LINEAR before `clear_color`
//! on sRGB attachments, relying on the hardware to encode the cleared value on
//! write (desktop GL: only with `FRAMEBUFFER_SRGB` enabled, which the desktop
//! shell turns on once at init; ES/WebGL2: always). If a driver skipped the
//! encode-on-clear, render-target backgrounds would come out dark — this test
//! reads the cleared bytes back and pins the encoded value.
//!
//! Self-skipping by default: it needs a GL display, and GLFW must run on the
//! process MAIN thread on macOS — so this test target is `harness = false`
//! (its own `main`, running the checks on the main thread) and only runs when
//! invoked with the same opt-in flag as the golden test:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test srgb_clear -- --ignored --nocapture
//! ```

use glow::HasContext;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("srgb_clear: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    clearing_an_srgb_render_target_stores_the_encoded_authored_color();
    println!("test clearing_an_srgb_render_target_stores_the_encoded_authored_color ... ok");
    srgb_clear_encode_matches_the_rust_codec();
    println!("test srgb_clear_encode_matches_the_rust_codec ... ok");
}

/// Build a hidden-window GL context (the capture-run recipe, minus the runtime).
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
        .create_window(64, 64, "srgb-clear-test", glfw::WindowMode::Windowed)
        .expect("create hidden GLFW window");
    window.make_current();
    let gl =
        unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    (gl, window, glfw)
}

/// Read back one RGBA pixel from the currently bound framebuffer.
fn read_pixel(gl: &glow::Context) -> [u8; 4] {
    let mut pixel = [0u8; 4];
    unsafe {
        gl.read_pixels(
            0,
            0,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
    }
    pixel
}

fn clearing_an_srgb_render_target_stores_the_encoded_authored_color() {
    let (gl, _window, _glfw) = gl_context();
    unsafe {
        // What the desktop shell enables once at init.
        gl.enable(glow::FRAMEBUFFER_SRGB);
    }

    // The production path: RenderTargetBuffers decodes the authored clear
    // internally; the sRGB attachment encodes it back on write. Authored
    // mid-gray 0.5 must read back as byte 128 (± rounding), NOT as the
    // linearized ~54 a skipped encode would leave.
    let authored = [0.5f32, 0.5, 0.5];
    let buffers = functor_runtime_common::render_target::RenderTargetBuffers::new(
        &gl, 4, 4, authored,
    );
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(buffers.write_fbo()));
    }
    let pixel = read_pixel(&gl);
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
    buffers.delete(&gl);

    let expected = (authored[0] * 255.0).round() as i32; // 128
    for channel in &pixel[0..3] {
        let diff = (*channel as i32 - expected).abs();
        assert!(
            diff <= 2,
            "sRGB clear round-trip drifted: got {pixel:?}, expected ~{expected} \
             (a value near 54 means the driver skipped encode-on-clear)"
        );
    }
    assert_eq!(pixel[3], 255);
}

fn srgb_clear_encode_matches_the_rust_codec() {
    // Same corner, pinned against the CPU codec across a sweep of values —
    // the desktop-vs-web parity anchor: WebGL2 always converts on sRGB
    // clears, so desktop matching `linear_to_srgb` means both stores hold
    // the same bytes for the same authored clear.
    let (gl, _window, _glfw) = gl_context();
    unsafe {
        gl.enable(glow::FRAMEBUFFER_SRGB);
    }

    let (texture, fbo) = unsafe {
        let texture = gl.create_texture().expect("texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::SRGB8_ALPHA8 as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, 0);
        let fbo = gl.create_framebuffer().expect("fbo");
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        assert_eq!(
            gl.check_framebuffer_status(glow::FRAMEBUFFER),
            glow::FRAMEBUFFER_COMPLETE,
            "SRGB8_ALPHA8 must be color-renderable"
        );
        (texture, fbo)
    };

    for linear in [0.0f32, 0.0031308, 0.05, 0.2140411, 0.5, 1.0] {
        unsafe {
            gl.clear_color(linear, linear, linear, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let pixel = read_pixel(&gl);
        let expected =
            (functor_runtime_common::color_space::linear_to_srgb(linear) * 255.0).round() as i32;
        let diff = (pixel[0] as i32 - expected).abs();
        assert!(
            diff <= 2,
            "linear clear {linear} read back {} (expected ~{expected})",
            pixel[0]
        );
    }

    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(fbo);
        gl.delete_texture(texture);
    }
}
