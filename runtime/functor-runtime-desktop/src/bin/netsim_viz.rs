//! A live multi-pane viewer for the netsim.
//!
//! Loads each game dylib as an independent instance, wires them through the
//! virtual network, and renders every instance's view in its own column of one
//! window while stepping the simulation each frame. So you can *watch* a
//! server's authoritative world next to each client's (slightly lagging) view.
//!
//! ```sh
//! functor-netsim-viz <dylib> [<dylib> ...]
//! # e.g. the multiplayer prototype (server + two clients):
//! functor-netsim-viz \
//!   examples/mpserver/build-native/target/debug/libgame_native.dylib \
//!   examples/mpclient/build-native/target/debug/libgame_native.dylib \
//!   examples/mpclient/build-native/target/debug/libgame_native.dylib
//! ```
//!
//! Keys: Esc quit · Space pause/resume · Right step one frame (while paused).
//!
//! For a headless-ish snapshot (still needs a GL context), `--capture <png>`
//! steps `--capture-after` frames (default 120), writes the whole multi-pane
//! window to a PNG, and exits — handy for sharing/reviewing the sim's output.

use std::sync::Arc;
use std::time::Instant;

use functor_netsim::{ClientRole, NetSim};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::ui::{Label, TextOverlay};
use functor_runtime_common::{DebugRenderMode, FrameTime, SceneContext, Viewport};
use glfw::Context;
use glow::*;

/// Accept either a dylib path or a sample name (resolved under `examples/`, run
/// from the repo root) — so `functor-netsim-viz mpserver mpclient mpclient` works.
fn resolve(arg: &str) -> String {
    if std::path::Path::new(arg).exists() {
        return arg.to_string();
    }
    let dll = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    format!("examples/{arg}/build-native/target/debug/{dll}")
}

/// Read back the default framebuffer and encode it as a PNG. GL rows are
/// bottom-up, so flip into image (top-down) order.
unsafe fn encode_framebuffer_png(gl: &glow::Context, width: u32, height: u32) -> Vec<u8> {
    let stride = (width * 4) as usize;
    let mut pixels = vec![0u8; stride * height as usize];
    gl.read_pixels(
        0,
        0,
        width as i32,
        height as i32,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelPackData::Slice(Some(&mut pixels)),
    );
    let mut flipped = vec![0u8; pixels.len()];
    for row in 0..height as usize {
        let src = (height as usize - 1 - row) * stride;
        flipped[row * stride..(row + 1) * stride].copy_from_slice(&pixels[src..src + stride]);
    }
    let img =
        image::RgbaImage::from_raw(width, height, flipped).expect("framebuffer size mismatch");
    let mut bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("encode png");
    bytes
}

fn main() {
    // Split flags from the positional dylib/sample args.
    let mut paths: Vec<String> = Vec::new();
    let mut capture: Option<String> = None;
    let mut capture_after: u32 = 120;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--capture" => capture = it.next(),
            "--capture-after" => {
                capture_after = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(capture_after)
            }
            _ => paths.push(a),
        }
    }
    if paths.is_empty() {
        eprintln!(
            "usage: functor-netsim-viz [--capture <png> [--capture-after <n>]] <dylib|sample> ..."
        );
        eprintln!("   e.g. functor-netsim-viz mpserver mpclient mpclient");
        std::process::exit(2);
    }

    let mut sim = NetSim::new(1);
    for arg in &paths {
        let path = resolve(arg);
        assert!(
            std::path::Path::new(&path).exists(),
            "not found: {path} (build it: functor -d examples/{arg} build native)"
        );
        sim.add(&path);
    }
    let panes = sim.len();
    println!("[netsim-viz] {panes} instance(s); Space=pause, Right=step, Esc=quit");

    unsafe {
        let mut glfw = glfw::init(glfw::fail_on_errors).unwrap();
        glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        #[cfg(target_os = "macos")]
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));

        let width = (420 * panes.max(1)).min(2400) as u32;
        let (mut window, events) = glfw
            .create_window(width, 480, "functor netsim", glfw::WindowMode::Windowed)
            .expect("Failed to create window");
        window.make_current();
        window.set_key_polling(true);

        // Arc so the egui overlay painter can share the same GL context.
        let gl = Arc::new(glow::Context::from_loader_function(|s| {
            window.get_proc_address(s) as *const _
        }));
        gl.enable(glow::DEPTH_TEST);

        let asset_cache = Arc::new(AssetCache::new());
        let scene_context = SceneContext::new();
        let shadow_map = functor_runtime_common::shadow::ShadowMap::new(&gl, 2048);
        // Per-client text overlay (id/role/connections/in-flight/fps) on each pane.
        let mut overlay = TextOverlay::new(gl.clone());

        let start = Instant::now();
        let mut paused = false;
        let mut step_once = false;
        let mut frames: u32 = 0;
        // Smoothed viewer frame rate (EMA), shown in each pane's overlay.
        let mut last_frame = Instant::now();
        let mut fps = 0.0f32;

        while !window.should_close() {
            // Smoothed viewer FPS for the overlay (EMA over instantaneous dt).
            let frame_start = Instant::now();
            let dt = frame_start.duration_since(last_frame).as_secs_f32();
            last_frame = frame_start;
            if dt > 0.0 {
                let inst = 1.0 / dt;
                fps = if fps == 0.0 {
                    inst
                } else {
                    fps * 0.9 + inst * 0.1
                };
            }

            glfw.poll_events();
            for (_, event) in glfw::flush_messages(&events) {
                if let glfw::WindowEvent::Key(key, _, glfw::Action::Press, _) = event {
                    match key {
                        glfw::Key::Escape => window.set_should_close(true),
                        glfw::Key::Space => paused = !paused,
                        glfw::Key::Right => step_once = true,
                        _ => {}
                    }
                }
            }

            if !paused || step_once {
                sim.step();
                step_once = false;
            }

            let time = FrameTime {
                dts: 1.0 / 60.0,
                tts: start.elapsed().as_secs_f32(),
            };

            let (fb_w, fb_h) = window.get_framebuffer_size();

            // Black the whole window first (panes scissor-clear their own area, so
            // the gaps between them stay black as a divider).
            gl.disable(glow::SCISSOR_TEST);
            gl.viewport(0, 0, fb_w, fb_h);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            // One column per instance, with a 1px gap.
            let gap = 2;
            let pane_w = ((fb_w - gap * (panes as i32 - 1)).max(1)) / panes.max(1) as i32;
            for i in 0..panes {
                let x = i as i32 * (pane_w + gap);
                // The shadow pass inside render_frame must run unscissored; it
                // re-enables + scissors the main pass to this pane.
                gl.disable(glow::SCISSOR_TEST);
                let frame = sim.render(i, time.clone());
                let viewport =
                    Viewport::with_offset(x.max(0) as u32, 0, pane_w.max(1) as u32, fb_h as u32);
                functor_runtime_common::render_frame(
                    &gl,
                    "#version 410",
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &frame,
                    time.clone(),
                    viewport,
                    DebugRenderMode::Default,
                );
            }

            // Per-client info overlay on top of all panes, in a single egui pass.
            // Each pane's labels are offset to its column's x origin.
            let frame_no = sim.frame();
            let mut labels: Vec<Label> = Vec::new();
            for i in 0..panes {
                let info = sim.client_info(i);
                let x = (i as i32 * (pane_w + gap)).max(0) as f32 + 8.0;
                let role_color = match info.role {
                    ClientRole::Server => [120, 230, 140],
                    ClientRole::Client => [180, 200, 255],
                };
                let dim = [170, 170, 185];
                labels.push(
                    Label::new(
                        format!("#{} {} · {}", info.id, info.role.as_str(), paths[i]),
                        x,
                        8.0,
                    )
                    .with_color(role_color),
                );
                labels.push(
                    Label::new(
                        format!("node {} · {} conn", info.node, info.connections),
                        x,
                        26.0,
                    )
                    .with_color(dim),
                );
                labels.push(
                    Label::new(format!("inbound {}", info.inbound_in_flight), x, 42.0)
                        .with_color(dim),
                );
                labels.push(
                    Label::new(format!("{:.0} fps · frame {}", fps, frame_no), x, 58.0)
                        .with_color(dim),
                );
            }
            overlay.draw(fb_w as u32, fb_h as u32, 1.0, &labels);

            // Snapshot the whole window once we've stepped far enough, then exit.
            frames += 1;
            if let Some(path) = &capture {
                if frames >= capture_after {
                    let bytes = encode_framebuffer_png(&gl, fb_w as u32, fb_h as u32);
                    std::fs::write(path, &bytes).expect("write capture");
                    println!("[netsim-viz] captured {path} after {frames} frames");
                    break;
                }
            }

            window.swap_buffers();
        }
    }
}
