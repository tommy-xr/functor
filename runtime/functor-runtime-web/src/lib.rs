use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cgmath::Matrix4;
use cgmath::{perspective, vec3, Deg, Point3};
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::io::load_bytes_async;
use functor_runtime_common::material::BasicMaterial;
use functor_runtime_common::texture::{
    RuntimeTexture, Texture2D, TextureData, TextureFormat, TextureOptions, PNG,
};
use functor_runtime_common::{FrameTime, RenderContext, Scene3D};
use glow::*;
use js_sys::{Function, Object, Reflect, WebAssembly};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use wasm_bindgen::prelude::*;

const SCR_WIDTH: u32 = 800;
const SCR_HEIGHT: u32 = 600;
fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    window()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("should register `requestAnimationFrame` OK");
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = game, js_name = render)]
    fn game_render(frameTimeJs: JsValue) -> JsValue;
}

#[wasm_bindgen(start)]
pub fn main() {
    spawn_local(async {
        run_async().await.unwrap_throw();
    })
}

async fn run_async() -> Result<(), JsValue> {
    // Load game
    // web_sys::console::log_2(&JsValue::from_str("Here: "), &three);
    // println!("Value! {:?}", three);
    unsafe {
        // Create a context from a WebGL2 context on wasm32 targets
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

        web_sys::console::log_1(&JsValue::from_str("here - 20!"));

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
        let f = Rc::new(RefCell::new(None));
        let g = f.clone();

        let window = window();
        let performance = window
            .performance()
            .expect("performance should be available");

        let mut i = 0;

        let initial_time = performance.now() as f32;
        let mut last_time = initial_time;
        let texture_future = async {
            let bytes = load_bytes_async("crate.png").await;
            sleep(Duration::from_secs(1)).await;
            let texture_data = PNG.load(&bytes.unwrap());
            //let texture_data1 = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
            Ok(texture_data)
        };
        let texture1 = Texture2D::init_from_future(texture_future, TextureOptions::default());

        *g.borrow_mut() = Some(Closure::new(move || {
            let render_ctx = RenderContext {
                gl: &gl,
                shader_version,
            };

            let projection_matrix: Matrix4<f32> =
                perspective(Deg(45.0), SCR_WIDTH as f32 / SCR_HEIGHT as f32, 0.1, 100.0);

            let world_matrix = Matrix4::from_nonuniform_scale(1.0, 1.0, 1.0);

            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            let radius = 5.0;
            let time: f64 = performance.now() / 1000.0;
            let view_matrix: Matrix4<f32> = Matrix4::look_at_rh(
                Point3::new(0.0, 0.0, -1.0 * radius),
                Point3::new(0.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            );

            // let scene = Scene3D::cube();

            let now = performance.now() as f32;
            let frameTime = FrameTime {
                dts: (now - last_time) / 1000.0,
                tts: (now - initial_time) / 1000.0,
            };
            last_time = now;

            let val = game_render(functor_runtime_common::to_js_value(&frameTime));
            web_sys::console::log_2(&JsValue::from_str("calling render"), &val);

            let scene: Scene3D = functor_runtime_common::from_js_value(val);

            let mut basic_material = BasicMaterial::create();
            basic_material.initialize(&render_ctx);

            texture1.bind(0, &render_ctx);

            functor_runtime_common::Scene3D::render(
                &scene,
                &render_ctx,
                &world_matrix,
                &projection_matrix,
                &view_matrix,
                &basic_material,
            );

            // Schedule ourself for another requestAnimationFrame callback.
            request_animation_frame(f.borrow().as_ref().unwrap());
        }));

        request_animation_frame(g.borrow().as_ref().unwrap());
    };

    Ok(())
}

async fn sleep(duration: Duration) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve,
                duration.as_millis() as i32,
            )
            .expect("should register `setTimeout` OK");
    });

    let _ = JsFuture::from(promise).await;
}
