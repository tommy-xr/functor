//! GL pin for the hot-reload indicator: the mark draws in each state's color
//! in the lower-left corner, over (not under) existing depth, and nowhere
//! else; at half alpha it blends half-way.
//!
//! `RELOAD_INDICATOR_OUT=<dir>` also writes the whole animation as PNG frames
//! (30 fps; `ok-NNN.png`, `failed-NNN.png`): a 0.9 s reload stepped through
//! the real state machine and painter.
//!
//! Self-skipping like `srgb_clear` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test reload_indicator -- --ignored --nocapture
//! ```

use cgmath::vec4;
use functor_runtime_common::reload_indicator::{
    corner_transform, ReloadIndicator, ReloadIndicatorPainter, ReloadLook, ReloadShown, FADE, HOLD,
};
use glow::HasContext;

const SIZE: u32 = 256;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("reload_indicator: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let (gl, _window, _glfw) = gl_context();
    the_mark_draws_lower_left_in_each_states_color(&gl);
    println!("test the_mark_draws_lower_left_in_each_states_color ... ok");
    if let Ok(dir) = std::env::var("RELOAD_INDICATOR_OUT") {
        write_animation(&gl, &dir);
    }
}

/// A pushed reload answered after 0.9 s, frame by frame until it has faded.
fn write_animation(gl: &glow::Context, dir: &str) {
    use std::time::{Duration, Instant};
    const SIZE: (u32, u32) = (960, 540);
    framebuffer(gl, SIZE.0, SIZE.1);
    let painter = ReloadIndicatorPainter::new(gl, "#version 330 core");
    // The pulse then peaks as the mark faces the viewer.
    let answered = Duration::from_millis(900);
    let frames = ((answered + HOLD + FADE).as_secs_f32() * 30.0) as u32 + 6;
    for (name, result) in [("ok", Ok("reloaded".to_string())), ("failed", Err("type error".to_string()))] {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let (reply, _pusher) = std::sync::mpsc::channel();
        let mut producer = Some((indicator.track(reply, t0), result));
        for frame in 0..frames {
            let now = t0 + Duration::from_secs_f32(frame as f32 / 30.0);
            if now - t0 >= answered {
                if let Some((producer, result)) = producer.take() {
                    producer.send(result).unwrap();
                }
            }
            indicator.poll(now, frame as u64);
            unsafe {
                // The site's background.
                gl.clear_color(15.0 / 255.0, 12.0 / 255.0, 29.0 / 255.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            }
            if let Some(look) = indicator.look(now) {
                painter.draw(gl, corner_transform(SIZE.0, SIZE.1, &look), &look, SIZE, true);
            }
            let png = unsafe {
                functor_runtime_common::frame_capture::encode_bound_framebuffer_png(gl, SIZE.0, SIZE.1)
            }
            .expect("read frame");
            std::fs::write(format!("{dir}/{name}-{frame:03}.png"), png).expect("write frame");
        }
    }
    painter.delete(gl);
    println!("wrote {frames} frames per outcome to {dir}");
}

fn the_mark_draws_lower_left_in_each_states_color(gl: &glow::Context) {
    framebuffer(gl, SIZE, SIZE);
    let painter = ReloadIndicatorPainter::new(gl, "#version 330 core");
    let still = |shown, alpha| ReloadLook { shown, spin: 0.0, alpha, pulse: 0.0 };
    let transform = corner_transform(SIZE, SIZE, &still(ReloadShown::Loading, 1.0));
    // A stem point of the SVG mark (x 440, y 600 of 1024), in mark units.
    let stem = pixel_of(transform, (440.0 - 558.5) / 658.0, (508.0 - 600.0) / 658.0);
    for shown in [ReloadShown::Loading, ReloadShown::Ok, ReloadShown::Failed] {
        unsafe {
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            // Nearest depth: an unoccluded overlay still draws.
            gl.clear_depth_f32(0.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        // `encode` on a plain RGBA8 target: bytes are the authored sRGB.
        painter.draw(gl, transform, &still(shown, 1.0), (SIZE, SIZE), true);
        let pixels = read(gl);
        let at = |(x, y): (u32, u32)| {
            let i = ((y * SIZE + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let want = shown.color().map(|c| (c * 255.0).round() as u8);
        let got = at(stem);
        assert!(
            got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2),
            "{shown:?}: stem pixel {stem:?} is {got:?}, want {want:?}"
        );
        // Only the corner is touched.
        for y in 0..SIZE {
            for x in 0..SIZE {
                if x > SIZE / 4 || y > SIZE / 4 {
                    assert_eq!(at((x, y)), [0, 0, 0], "{shown:?}: stray pixel at ({x}, {y})");
                }
            }
        }
    }
    // Half alpha over black: half the authored bytes, and blending is left
    // as it was found (off).
    unsafe {
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    painter.draw(gl, transform, &still(ReloadShown::Ok, 0.5), (SIZE, SIZE), true);
    let pixels = read(gl);
    let i = ((stem.1 * SIZE + stem.0) * 4) as usize;
    let want = ReloadShown::Ok.color().map(|c| (c * 127.5).round() as u8);
    assert!(
        pixels[i..i + 3].iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2),
        "half alpha: {:?}, want {want:?}",
        &pixels[i..i + 3]
    );
    assert!(!unsafe { gl.is_enabled(glow::BLEND) }, "blend left on");
    painter.delete(gl);
}

/// Framebuffer pixel (bottom-left origin) of a mark-space point.
fn pixel_of(transform: cgmath::Matrix4<f32>, x: f32, y: f32) -> (u32, u32) {
    let clip = transform * vec4(x, y, 0.0, 1.0);
    let to_px = |ndc: f32| ((ndc + 1.0) / 2.0 * SIZE as f32) as u32;
    (to_px(clip.x / clip.w), to_px(clip.y / clip.w))
}

fn read(gl: &glow::Context) -> Vec<u8> {
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

/// A hidden-window GL context.
fn gl_context() -> (glow::Context, glfw::PWindow, glfw::Glfw) {
    use glfw::Context as _;
    let mut glfw = glfw::init(glfw::fail_on_errors).expect("init GLFW");
    glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Visible(false));
    let (mut window, _events) = glfw
        .create_window(64, 64, "reload-indicator-test", glfw::WindowMode::Windowed)
        .expect("create hidden GLFW window");
    window.make_current();
    let gl = unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    (gl, window, glfw)
}

/// Bind a new `width`×`height` RGBA8 + depth framebuffer.
fn framebuffer(gl: &glow::Context, width: u32, height: u32) {
    unsafe {
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        for (format, attachment) in [
            (glow::RGBA8, glow::COLOR_ATTACHMENT0),
            (glow::DEPTH_COMPONENT24, glow::DEPTH_ATTACHMENT),
        ] {
            let rb = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            gl.renderbuffer_storage(glow::RENDERBUFFER, format, width as i32, height as i32);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, attachment, glow::RENDERBUFFER, Some(rb));
        }
        assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
    }
}
