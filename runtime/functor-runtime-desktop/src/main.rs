#![cfg_attr(feature = "strict", deny(warnings))]

use std::borrow::BorrowMut;
use std::env;
use std::sync::Arc;
use std::time::Instant;

use cgmath::{perspective, vec3, Deg, Point3};
use cgmath::{vec4, Matrix4};
use functor_runtime_common::asset::pipelines::ModelPipeline;
use functor_runtime_common::asset::{self, AssetCache, AssetPipeline};
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::material::ColorMaterial;
use functor_runtime_common::{FrameTime, SceneContext};
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

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    game_path: String,

    #[arg(long)]
    hot: bool,
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
            window.set_framebuffer_size_polling(true);
            // window.set_cursor_mode(glfw::CursorMode::Disabled);

            let gl =
                glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);

            (gl, "#version 410", window, glfw, events)
        };

        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let projection_matrix: Matrix4<f32> =
            perspective(Deg(45.0), SCR_WIDTH as f32 / SCR_HEIGHT as f32, 0.1, 100.0);

        let world_matrix = Matrix4::from_nonuniform_scale(1.0, 1.0, 1.0);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;

        use glfw::Context;

        let asset_cache = Arc::new(AssetCache::new());

        let scene_context = SceneContext::new();

        let model_pipeline = asset::build_pipeline(Box::new(ModelPipeline));
        let model = asset_cache.load_asset_with_pipeline(model_pipeline, "ExplodingBarrel.glb");

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
            let time: FrameTime = FrameTime {
                dts: elapsed_time - last_time,
                tts: elapsed_time,
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
                    _ => {}
                }
            }

            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            let radius = 5.0;
            let view_matrix: Matrix4<f32> = Matrix4::look_at_rh(
                Point3::new(0.0, 0.0, -1.0 * radius),
                Point3::new(0.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            );

            let render_context = functor_runtime_common::RenderContext {
                gl: &gl,
                shader_version,
                asset_cache: asset_cache.clone(),
            };

            let scene = game.render(time.clone());

            // TODO: Factor out to pass in current_material
            // let mut basic_material = BasicMaterial::create();
            // basic_material.initialize(&context);

            // asset.get().bind(0, &context);

            let mut color_material = ColorMaterial::create(vec4(1.0, 0.0, 0.0, 1.0));
            color_material.initialize(&render_context);

            functor_runtime_common::Scene3D::render(
                &scene,
                &render_context,
                &scene_context,
                &world_matrix,
                &projection_matrix,
                &view_matrix,
                &color_material,
            );

            window.swap_buffers();
        }
    }

    game.quit();
}
