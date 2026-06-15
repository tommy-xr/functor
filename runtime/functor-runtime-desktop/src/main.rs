#![cfg_attr(feature = "strict", deny(warnings))]

use std::env;
use std::sync::Arc;
use std::time::Instant;

use cgmath::{vec4, Matrix4};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::material::ColorMaterial;
use functor_runtime_common::{FrameTime, SceneContext};
use functor_runtime_common::Key as InputKey;
use glfw::{Action, Key};
use glow::*;
use hot_reload_game::HotReloadGame;
use static_game::StaticGame;

use crate::game::Game;

const SCR_WIDTH: u32 = 800;
const SCR_HEIGHT: u32 = 600;

mod debug_server;
mod game;
mod hot_reload_game;
mod static_game;

/// Translate a GLFW key into the canonical engine key code passed across the
/// game boundary. Unmapped keys become InputKey::Unknown.
fn map_key(key: Key) -> InputKey {
    match key {
        Key::A => InputKey::A,
        Key::B => InputKey::B,
        Key::C => InputKey::C,
        Key::D => InputKey::D,
        Key::E => InputKey::E,
        Key::F => InputKey::F,
        Key::G => InputKey::G,
        Key::H => InputKey::H,
        Key::I => InputKey::I,
        Key::J => InputKey::J,
        Key::K => InputKey::K,
        Key::L => InputKey::L,
        Key::M => InputKey::M,
        Key::N => InputKey::N,
        Key::O => InputKey::O,
        Key::P => InputKey::P,
        Key::Q => InputKey::Q,
        Key::R => InputKey::R,
        Key::S => InputKey::S,
        Key::T => InputKey::T,
        Key::U => InputKey::U,
        Key::V => InputKey::V,
        Key::W => InputKey::W,
        Key::X => InputKey::X,
        Key::Y => InputKey::Y,
        Key::Z => InputKey::Z,
        Key::Up => InputKey::Up,
        Key::Down => InputKey::Down,
        Key::Left => InputKey::Left,
        Key::Right => InputKey::Right,
        Key::Space => InputKey::Space,
        Key::Enter => InputKey::Enter,
        Key::Escape => InputKey::Escape,
        _ => InputKey::Unknown,
    }
}

/// Map a key name (case-insensitive: "w", "Up", "space", …) to the engine key
/// code passed across the game boundary, for the debug server's POST /input.
/// Letters rely on the contiguous A..Z discriminants.
fn key_code_from_str(name: &str) -> Option<i32> {
    let name = name.to_ascii_lowercase();
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some((c - b'a') as i32 + InputKey::A as i32);
        }
    }
    let key = match name.as_str() {
        "up" => InputKey::Up,
        "down" => InputKey::Down,
        "left" => InputKey::Left,
        "right" => InputKey::Right,
        "space" => InputKey::Space,
        "enter" => InputKey::Enter,
        "escape" => InputKey::Escape,
        _ => return None,
    };
    Some(key as i32)
}

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    game_path: String,

    #[arg(long)]
    hot: bool,

    /// Write a PNG of the rendered frame to this path, then exit. The capture
    /// happens on the first frame after --capture-time seconds of wall-clock
    /// time, so assets have a chance to load.
    #[arg(long)]
    capture_frame: Option<String>,

    /// Wall-clock seconds to run before --capture-frame takes the shot.
    #[arg(long, default_value_t = 2.0)]
    capture_time: f32,

    /// Pin the game's frame time (seconds) to a constant so the rendered pose
    /// is deterministic — for reproducible captures / golden images.
    #[arg(long)]
    fixed_time: Option<f32>,

    /// Start an HTTP control server on 127.0.0.1:<PORT> (localhost only) exposing
    /// POST /capture (image/png of the next frame) and GET /state (runtime JSON).
    /// Omit to disable the server entirely.
    #[arg(long)]
    debug_port: Option<u16>,
}

/// Read back the framebuffer just rendered (called before swap_buffers, so the
/// back buffer) and encode it as PNG bytes. Returns an error string if the
/// readback can't be turned into a valid image. Shared by `--capture-frame`
/// (writes the bytes to a file) and the debug server's `POST /capture` (streams
/// the bytes back over HTTP), so both produce byte-identical PNGs.
unsafe fn encode_framebuffer_png(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let stride = (width * 4) as usize;
    let mut pixels = vec![0u8; stride * height as usize];
    gl.read_pixels(
        0,
        0,
        width as i32,
        height as i32,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelPackData::Slice(&mut pixels),
    );

    // GL rows are bottom-up; flip into image (top-down) order.
    let mut flipped = vec![0u8; pixels.len()];
    for row in 0..height as usize {
        let src = (height as usize - 1 - row) * stride;
        flipped[row * stride..(row + 1) * stride].copy_from_slice(&pixels[src..src + stride]);
    }

    let img = image::RgbaImage::from_raw(width, height, flipped)
        .ok_or_else(|| "framebuffer size mismatch".to_string())?;
    let mut bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Read back the framebuffer and write it as a PNG file. Exits the process with
/// an error if the capture cannot be written, so scripts don't mistake a failed
/// capture for a pass.
unsafe fn capture_framebuffer(gl: &glow::Context, width: u32, height: u32, path: &str) {
    let result = encode_framebuffer_png(gl, width, height)
        .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()));
    match result {
        Ok(()) => println!("Captured frame to {}", path),
        Err(e) => {
            eprintln!("Failed to capture frame to {}: {}", path, e);
            std::process::exit(1);
        }
    }
}

#[tokio::main]
pub async fn main() {
    // Load game

    let args = Args::parse();

    let game_path = args.game_path;
    println!("Using game path: {}", game_path);
    println!("Working directory: {:?}", env::current_dir());

    let mut game: Box<dyn Game> = if args.hot {
        Box::new(HotReloadGame::create(game_path.as_str()))
    } else {
        Box::new(StaticGame::create(game_path.as_str()))
    };

    // Optional debug control server. Runs on its own thread; the GL loop drains
    // its request channel once per frame (see below). None when --debug-port is
    // not given, so behavior is unchanged.
    let debug_requests = args.debug_port.map(debug_server::spawn);

    unsafe {
        let (gl, shader_version, mut window, mut glfw, events) = {
            use glfw::Context;
            let mut glfw = glfw::init(glfw::fail_on_errors).unwrap();
            // TODO: Figure out ANGLE
            // glfw.window_hint(glfw::WindowHint::ClientApi(glfw::OpenGlEs));
            glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
            glfw.window_hint(glfw::WindowHint::OpenGlProfile(
                glfw::OpenGlProfileHint::Core,
            ));
            #[cfg(target_os = "macos")]
            glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));

            // glfw window creation
            // --------------------
            let (mut window, events) = glfw
                .create_window(SCR_WIDTH, SCR_HEIGHT, "Functor", glfw::WindowMode::Windowed)
                .expect("Failed to create GLFW window");

            window.make_current();
            window.set_key_polling(true);
            window.set_cursor_pos_polling(true);
            window.set_scroll_polling(true);
            window.set_framebuffer_size_polling(true);
            // Capture and hide the cursor so the game gets continuous relative
            // mouse motion (free-look) instead of the pointer stopping at the
            // window edges. Escape still closes the window.
            window.set_cursor_mode(glfw::CursorMode::Disabled);

            let gl =
                glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);

            (gl, "#version 410", window, glfw, events)
        };

        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let world_matrix = Matrix4::from_nonuniform_scale(1.0, 1.0, 1.0);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;
        let mut frame_count: u64 = 0;
        // Debug-server clock control (POST /time). `held_time` pins the frame
        // time to a constant (None = follow wall clock); `pending_step` applies a
        // one-shot dt advance on the next frame. Seeded from --fixed-time.
        let mut held_time: Option<f32> = args.fixed_time;
        let mut pending_step: Option<f32> = None;

        use glfw::Context;

        let asset_cache = Arc::new(AssetCache::new());

        let scene_context = SceneContext::new();

        // let texture_future = async {
        //     let bytes = load_bytes_async("crate.png").await;
        //     tokio::time::sleep(Duration::from_secs(1)).await;
        //     let texture_data = PNG.load(&bytes.unwrap());
        //     // let texture_data1 = TextureData::checkerboard_pattern(8, 8, [255, 0, 0, 255]);
        //     Ok(texture_data)
        // };
        //let texture1 = Texture2D::init_from_future(texture_future, TextureOptions::default());

        // let asset = asset_cache.load_asset_with_pipeline(Arc::new(TexturePipeline), "crate.png");

        // let texture_data1 = create_checkerboard_pattern(8, 8, [255, 0, 0, 255]);
        // let texture1 = Texture2D::init_from_data(texture_data1, TextureOptions::default());

        // let texture_data = PNG.load(&CRATE_BYTES.to_vec());

        while !window.should_close() {
            let elapsed_time = start_time.elapsed().as_secs_f32();
            // The frame time handed to the game. Pinning it (--fixed-time, or the
            // debug server's /time) makes the rendered pose deterministic — used
            // for reproducible captures / golden images. The capture trigger
            // below still keys off wall-clock elapsed, so the loop runs long
            // enough for assets to load before a shot is taken.
            let time: FrameTime = if let Some(step) = pending_step.take() {
                // One-shot /time advance: step the clock by `step` with a matching
                // dts so the simulation integrates the interval, then stay held.
                let new_tts = held_time.unwrap_or(last_time) + step;
                held_time = Some(new_tts);
                FrameTime {
                    dts: step,
                    tts: new_tts,
                }
            } else {
                match held_time {
                    Some(tts) => FrameTime { dts: 0.0, tts },
                    None => FrameTime {
                        dts: elapsed_time - last_time,
                        tts: elapsed_time,
                    },
                }
            };
            last_time = elapsed_time;

            game.check_hot_reload(time.clone());

            glfw.poll_events();
            for (_, event) in glfw::flush_messages(&events) {
                match event {
                    glfw::WindowEvent::Close => window.set_should_close(true),
                    glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                        window.set_should_close(true)
                    }
                    glfw::WindowEvent::Key(key, _, action, _) => match action {
                        Action::Press | Action::Repeat => {
                            game.key_event(map_key(key) as i32, true)
                        }
                        Action::Release => game.key_event(map_key(key) as i32, false),
                    },
                    glfw::WindowEvent::CursorPos(x, y) => {
                        game.mouse_move(x as i32, y as i32)
                    }
                    glfw::WindowEvent::Scroll(_, y) => game.mouse_wheel(y as i32),
                    _ => {}
                }
            }

            game.tick(time.clone());

            // Follow window resizes: query the drawable size each frame and set
            // the GL viewport to match. Framebuffer size is in pixels, so this
            // handles HiDPI/retina correctly.
            let (fb_width, fb_height) = window.get_framebuffer_size();
            let viewport = functor_runtime_common::Viewport::new(fb_width as u32, fb_height as u32);
            gl.viewport(0, 0, fb_width, fb_height);

            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            let render_context = functor_runtime_common::RenderContext {
                gl: &gl,
                shader_version,
                asset_cache: asset_cache.clone(),
                frame_time: time.clone(),
            };

            // The game supplies the camera as part of its frame; derive the
            // view/projection matrices from it.
            let frame = game.render(time.clone());
            let view_matrix = frame.camera.view_matrix();
            let projection_matrix = frame.camera.projection_matrix(viewport.aspect());

            // TODO: Factor out to pass in current_material
            // let mut basic_material = BasicMaterial::create();
            // basic_material.initialize(&context);

            // asset.get().bind(0, &context);

            let mut color_material = ColorMaterial::create(vec4(1.0, 0.0, 0.0, 1.0));
            color_material.initialize(&render_context);

            functor_runtime_common::Scene3D::render(
                &frame.scene,
                &render_context,
                &scene_context,
                &world_matrix,
                &projection_matrix,
                &view_matrix,
                &color_material,
            );

            if let Some(capture_path) = &args.capture_frame {
                if elapsed_time >= args.capture_time {
                    capture_framebuffer(
                        &gl,
                        fb_width as u32,
                        fb_height as u32,
                        capture_path.as_str(),
                    );
                    window.set_should_close(true);
                }
            }

            // Service any pending debug-server requests now that the frame is
            // fully rendered into the back buffer (same point --capture-frame
            // reads from). GL stays on this thread; we only reply over channels.
            if let Some(rx) = &debug_requests {
                while let Ok(req) = rx.try_recv() {
                    match req {
                        debug_server::DebugRequest::Capture(resp) => {
                            match encode_framebuffer_png(&gl, fb_width as u32, fb_height as u32) {
                                Ok(png) => {
                                    let _ = resp.send(png);
                                }
                                Err(e) => {
                                    eprintln!("[debug-server] capture failed: {}", e);
                                    // Dropping `resp` signals failure to the handler.
                                }
                            }
                        }
                        debug_server::DebugRequest::State(resp) => {
                            let _ = resp.send(debug_server::RuntimeState {
                                frame: frame_count,
                                tts: time.tts,
                                width: fb_width as u32,
                                height: fb_height as u32,
                                model: game.state_debug(),
                            });
                        }
                        debug_server::DebugRequest::Scene(resp) => {
                            // Serialize the frame we just rendered (camera +
                            // scene). Frame derives Serialize for the wasm path,
                            // so this is real JSON, not Debug text.
                            let json = serde_json::to_string_pretty(&frame)
                                .unwrap_or_else(|e| format!("{{\"error\":{:?}}}", e.to_string()));
                            let _ = resp.send(json);
                        }
                        debug_server::DebugRequest::Input(cmd, resp) => {
                            // Inject input as if it came from the window; the game
                            // applies it immediately, so the next /state reflects it.
                            let result = match cmd {
                                debug_server::InputCommand::Key { key, down } => {
                                    match key_code_from_str(&key) {
                                        Some(code) => {
                                            game.key_event(code, down);
                                            Ok(())
                                        }
                                        None => Err(format!("unknown key: {}", key)),
                                    }
                                }
                                debug_server::InputCommand::MouseMove { x, y } => {
                                    game.mouse_move(x, y);
                                    Ok(())
                                }
                                debug_server::InputCommand::MouseWheel { delta } => {
                                    game.mouse_wheel(delta);
                                    Ok(())
                                }
                            };
                            let _ = resp.send(result);
                        }
                        debug_server::DebugRequest::Time(cmd, resp) => {
                            match cmd {
                                debug_server::TimeCommand::Set { tts } => {
                                    held_time = Some(tts);
                                    pending_step = None;
                                }
                                debug_server::TimeCommand::Advance { dts } => {
                                    pending_step = Some(dts);
                                }
                                debug_server::TimeCommand::Resume => {
                                    held_time = None;
                                    pending_step = None;
                                }
                            }
                            let _ = resp.send(());
                        }
                    }
                }
            }

            window.swap_buffers();
            frame_count += 1;
        }
    }

    game.quit();
}
