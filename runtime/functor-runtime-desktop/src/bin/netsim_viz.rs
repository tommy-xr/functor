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

use std::sync::Arc;
use std::time::Instant;

use functor_netsim::NetSim;
use functor_runtime_common::asset::AssetCache;
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: functor-netsim-viz <dylib|sample> [<dylib|sample> ...]");
        eprintln!("   e.g. functor-netsim-viz mpserver mpclient mpclient");
        std::process::exit(2);
    }

    let mut sim = NetSim::new(1);
    for arg in &args {
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
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
        #[cfg(target_os = "macos")]
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));

        let width = (420 * panes.max(1)).min(2400) as u32;
        let (mut window, events) = glfw
            .create_window(width, 480, "functor netsim", glfw::WindowMode::Windowed)
            .expect("Failed to create window");
        window.make_current();
        window.set_key_polling(true);

        let gl = glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);
        gl.enable(glow::DEPTH_TEST);

        let asset_cache = Arc::new(AssetCache::new());
        let scene_context = SceneContext::new();
        let shadow_map = functor_runtime_common::shadow::ShadowMap::new(&gl, 2048);

        let start = Instant::now();
        let mut paused = false;
        let mut step_once = false;

        while !window.should_close() {
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

            window.swap_buffers();
        }
    }
}
