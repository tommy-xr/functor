use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use cgmath::Matrix4;
use cgmath::{perspective, vec3, Deg, Point3};
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::material::BasicMaterial;
use functor_runtime_common::{FrameTime, Scene3D, SceneObject};
use glfw::{init, RenderContext};
use glow::*;
use hot_reload_game::HotReloadGame;
use libloading::{library_filename, Library, Symbol};
use notify::{event, RecursiveMode, Watcher};

use crate::game::Game;

const SCR_WIDTH: u32 = 800;
const SCR_HEIGHT: u32 = 600;

mod game;
mod hot_reload_game;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    game_path: String,
}

pub fn main() {
    // Load game

    let args = Args::parse();

    // let game_path = Path::new("target/debug/libgame_native.dylib");
    let game_path = Arc::new(args.game_path);
    println!("Using game path: {}", game_path.clone());
    println!("Working directory: {:?}", env::current_dir());

    let other_game_path = game_path.clone();

    let file_changed = Arc::new(AtomicBool::new(false));
    let file_changed_watcher = Arc::clone(&file_changed);
    let watcher_thread = std::thread::spawn(move || {
        // Select recommended watcher for debouncer.
        // Using a callback here, could also be a channel.

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).unwrap();

        let mut had_remove_event = false;

        let path = Path::new(game_path.as_str());
        watcher.watch(&path, RecursiveMode::Recursive).unwrap();

        println!("watcher created!");
        loop {
            match rx.recv() {
                Ok(event) => {
                    match event {
                        Ok(event) => {
                            match event.kind {
                                event::EventKind::Remove(_) => had_remove_event = true,
                                event::EventKind::Create(_) => {
                                    if had_remove_event {
                                        had_remove_event = false;
                                        println!("Pushing hot reload event from thread...");
                                        file_changed_watcher.store(true, Ordering::SeqCst);
                                    } else {
                                        println!("ignoring event");
                                    }
                                }
                                _ => (),
                            };
                            // TODO: Can we parse events here to handle create -> restore loop?
                            println!("event: {:?}", event);
                            //file_changed_watcher.store(true, Ordering::SeqCst);
                        }
                        Err(e) => println!("watch error: {:?}", e),
                    }
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        }
    });

    let mut game = HotReloadGame::create(other_game_path.as_str());

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
            // let gl = glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s));
            let gl =
                glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);
            // gl_surface
            //     .set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()))
            //     .unwrap();

            (
                gl,
                // gl_surface,
                // gl_context,
                "#version 410",
                window,
                glfw,
                events,
                // event_loop,
            )
        };

        let vertex_array = gl
            .create_vertex_array()
            .expect("Cannot create vertex array");
        gl.bind_vertex_array(Some(vertex_array));

        let program = gl.create_program().expect("Cannot create program");

        let (vertex_shader_source, fragment_shader_source) = (
            r#"
            precision mediump float;
            uniform mat4 world;
            const vec2 verts[3] = vec2[3](
                vec2(0.5f, 1.0f),
                vec2(0.0f, 0.0f),
                vec2(1.0f, 0.0f)
            );
            out vec2 vert;
            void main() {
                vert = verts[gl_VertexID];
                gl_Position = world * vec4(vert - 0.5, 0.0, 1.0);
            }"#,
            r#"precision mediump float;
            in vec2 vert;
            out vec4 color;
            void main() {
                color = vec4(vert, 0.5, 1.0);
            }"#,
        );

        let shader_sources = [
            (glow::VERTEX_SHADER, vertex_shader_source),
            (glow::FRAGMENT_SHADER, fragment_shader_source),
        ];

        let mut shaders = Vec::with_capacity(shader_sources.len());

        for (shader_type, shader_source) in shader_sources.iter() {
            let shader = gl
                .create_shader(*shader_type)
                .expect("Cannot create shader");
            gl.shader_source(shader, &format!("{}\n{}", shader_version, shader_source));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                panic!("{}", gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }

        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            panic!("{}", gl.get_program_info_log(program));
        }

        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }

        gl.use_program(Some(program));
        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let init_ctx = functor_runtime_common::RenderContext {
            gl: &gl,
            shader_version,
        };

        let projection_matrix: Matrix4<f32> =
            perspective(Deg(45.0), SCR_WIDTH as f32 / SCR_HEIGHT as f32, 0.1, 100.0);

        let world_matrix = Matrix4::from_nonuniform_scale(1.0, 1.0, 1.0);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;

        use glfw::Context;

        while !window.should_close() {
            let elapsed_time = start_time.elapsed().as_secs_f32();
            let time: FrameTime = FrameTime {
                dts: elapsed_time - last_time,
                tts: elapsed_time,
            };
            last_time = elapsed_time;

            // Check if file has changed
            if file_changed.load(Ordering::SeqCst) {
                println!("Reloading!");
                file_changed.store(false, Ordering::SeqCst);
                game.reload();
                println!("Rendering: {:?}", game.render(time.clone()));
            }

            glfw.poll_events();
            for (_, event) in glfw::flush_messages(&events) {
                match event {
                    glfw::WindowEvent::Close => window.set_should_close(true),
                    _ => {}
                }
            }

            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            let radius = 5.0;
            let camX = glfw.get_time().sin() as f32 * radius;
            let camZ = glfw.get_time().cos() as f32 * radius;
            let view_matrix: Matrix4<f32> = Matrix4::look_at_rh(
                Point3::new(camX, 0.0, camZ),
                Point3::new(0.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            );

            let context = functor_runtime_common::RenderContext {
                gl: &gl,
                shader_version,
            };

            let scene = game.render(time.clone());

            functor_runtime_common::Scene3D::render(
                &scene,
                &context,
                &world_matrix,
                &projection_matrix,
                &view_matrix,
            );

            window.swap_buffers();
        }

        watcher_thread.join().unwrap();
    }
}
