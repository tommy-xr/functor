pub fn add(left: usize, right: usize) -> usize {
    left + right
}

// pub fn hello_from_rust() {
//     println!("Hello from Rust!");
//     start();
// }

use core::slice;

use cgmath::{conv::array4x4, Matrix, Matrix4, SquareMatrix};
use functor_runtime_common::geometry;
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::Scene3D;
use glow::*;
use libloading::{library_filename, Library, Symbol};
pub fn main() {
    // Load game
    unsafe {
        let lib = Library::new(library_filename("pong")).unwrap(); // Load the "hello_world" library
        let func: Symbol<fn(f64)> = lib.get(b"dynamic_call_from_rust").unwrap(); // Get the function pointer

        let test_render_func: Symbol<fn() -> Scene3D> = lib.get(b"test_render").unwrap(); // Get the function pointer

        func(42.0); // Call the function

        let init_func: Symbol<fn()> = lib.get(b"init").unwrap(); // Get the function pointer
        println!("Running init.");
        init_func();

        println!("Got render: {:?}", test_render_func());
    }
    unsafe {
        // Create a context from a WebGL2 context on wasm32 targets
        #[cfg(target_arch = "wasm32")]
        let (gl, shader_version) = {
            use wasm_bindgen::JsCast;
            let canvas = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .get_element_by_id("canvas")
                .unwrap()
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .unwrap();
            let webgl2_context = canvas
                .get_context("webgl2")
                .unwrap()
                .unwrap()
                .dyn_into::<web_sys::WebGl2RenderingContext>()
                .unwrap();
            let gl = glow::Context::from_webgl2_context(webgl2_context);
            (gl, "#version 300 es")
        };

        #[cfg(not(target_arch = "wasm32"))]
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
                .create_window(800, 600, "Functor", glfw::WindowMode::Windowed)
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

        // let matrix: Matrix4<f32> = Matrix4::from_nonuniform_scale(2.0, 0.5, 1.0);
        // let matrix_location = unsafe {
        //     gl.get_uniform_location(program, "world")
        //         .expect("Cannot get uniform")
        // };
        // let data = (&array4x4(matrix) as *const [[f32; 4]; 4]) as *const f32;
        // let raw = slice::from_raw_parts(data, 16);
        // gl.uniform_matrix_4_f32_slice(Some(&matrix_location), false, raw);

        #[cfg(not(target_arch = "wasm32"))]
        {
            use glfw::Context;

            while !window.should_close() {
                glfw.poll_events();
                for (_, event) in glfw::flush_messages(&events) {
                    match event {
                        glfw::WindowEvent::Close => window.set_should_close(true),
                        _ => {}
                    }
                }
                gl.clear(glow::COLOR_BUFFER_BIT);

                let plane = functor_runtime_common::geometry::plane::create();
                plane.draw(&gl);

                window.swap_buffers();
            }
            //     use glutin::prelude::GlSurface;
            //     use winit::event::{Event, WindowEvent};
            //     let _ = event_loop.run(move |event, elwt| {
            //         if let Event::WindowEvent { event, .. } = event {
            //             match event {
            //                 WindowEvent::CloseRequested => {
            //                     elwt.exit();
            //                 }
            //                 WindowEvent::RedrawRequested => {
            //                     gl.clear(glow::COLOR_BUFFER_BIT);
            //                     gl.draw_arrays(glow::TRIANGLES, 0, 3);
            //                     gl_surface.swap_buffers(&gl_context).unwrap();
            //                 }
            //                 _ => (),
            //             }
            //         }
            //     });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
