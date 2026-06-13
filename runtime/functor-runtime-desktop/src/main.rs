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
}

/// Read back the framebuffer just rendered (called before swap_buffers, so the
/// back buffer) and write it as a PNG. Exits the process with an error if the
/// capture cannot be written, so scripts don't mistake a failed capture for a
/// pass.
unsafe fn capture_framebuffer(gl: &glow::Context, width: u32, height: u32, path: &str) {
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

    let result = image::RgbaImage::from_raw(width, height, flipped)
        .ok_or_else(|| "framebuffer size mismatch".to_string())
        .and_then(|img| img.save(path).map_err(|e| e.to_string()));
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
            // --fixed-time pins the time handed to the game to a constant, so
            // the rendered pose (animations, anything driven by FrameTime) is
            // deterministic regardless of frame rate or asset-load timing. Used
            // with --capture-frame for reproducible golden images. The capture
            // trigger below still keys off wall-clock elapsed, so the loop runs
            // long enough for assets to load before the shot is taken.
            let time: FrameTime = match args.fixed_time {
                Some(fixed) => FrameTime {
                    dts: 0.0,
                    tts: fixed,
                },
                None => FrameTime {
                    dts: elapsed_time - last_time,
                    tts: elapsed_time,
                },
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

            window.swap_buffers();
        }
    }

    game.quit();
}
