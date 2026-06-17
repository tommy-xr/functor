use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cgmath::Matrix4;
use functor_runtime_common::asset::pipelines::TexturePipeline;
use functor_runtime_common::asset::{AssetCache, AssetLoader};
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::io::load_bytes_async;
use functor_runtime_common::material::BasicMaterial;
use functor_runtime_common::texture::{
    RuntimeTexture, Texture2D, TextureData, TextureFormat, TextureOptions, PNG,
};
use functor_runtime_common::{Frame, FrameTime, RenderContext, SceneContext};
use glow::*;
use js_sys::{Function, Object, Reflect, WebAssembly};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use wasm_bindgen::prelude::*;

fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    window()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("should register `requestAnimationFrame` OK");
}

/// The wasm counterpart of the desktop `--debug-render` flag: read the mode
/// from the page URL's `?debug-render=<mode>` query (e.g.
/// `?debug-render=normals`). Defaults to `Default`; an unrecognized value logs
/// a console warning and falls back to `Default`.
fn debug_render_mode_from_url() -> functor_runtime_common::DebugRenderMode {
    use functor_runtime_common::DebugRenderMode;

    let search = window().location().search().unwrap_or_default();
    let query = search.trim_start_matches('?');
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some("debug-render") {
            let value = kv.next().unwrap_or("");
            return DebugRenderMode::from_label(value).unwrap_or_else(|| {
                web_sys::console::warn_1(
                    &format!("unknown debug-render mode '{}', using default", value).into(),
                );
                DebugRenderMode::Default
            });
        }
    }
    DebugRenderMode::Default
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = game, js_name = render)]
    fn game_render(frameTimeJs: JsValue) -> JsValue;

    #[wasm_bindgen(js_namespace = game, js_name = tick)]
    fn game_tick(frameTimeJs: JsValue);
}

#[wasm_bindgen(start)]
pub fn main() {
    spawn_local(async {
        run_async().await.unwrap_throw();
    })
}
struct WasmAssetLoader {}

#[async_trait]
impl AssetLoader for WasmAssetLoader {
    async fn load_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }
}

async fn run_async() -> Result<(), JsValue> {
    // Load game
    unsafe {
        // Create a context from a WebGL2 context on wasm32 targets
        let (gl, shader_version, canvas) = {
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
            (gl, "#version 300 es", canvas)
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
        let f = Rc::new(RefCell::new(None));
        let g = f.clone();

        let window = window();
        let performance = window
            .performance()
            .expect("performance should be available");

        let mut i = 0;

        let initial_time = performance.now() as f32;
        let mut last_time = initial_time;
        // let texture_future = async {
        //     let bytes = load_bytes_async("crate.png").await;
        //     sleep(Duration::from_secs(1)).await;
        //     let texture_data = PNG.load(&bytes.unwrap());
        //     //let texture_data1 = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
        //     Ok(texture_data)
        // };
        // let texture1 = Texture2D::init_from_future(texture_future, TextureOptions::default());

        let mut asset_cache = Arc::new(AssetCache::new());
        // let asset = asset_cache.load_asset_with_pipeline(Arc::new(TexturePipeline), "crate.png");

        let scene_context = SceneContext::new();

        // Read once from the page URL; it doesn't change over the session. The
        // `move` closure below captures it (the mode is `Copy`).
        let debug_render_mode = debug_render_mode_from_url();

        *g.borrow_mut() = Some(Closure::new(move || {
            let now = performance.now() as f32;
            let frame_time = FrameTime {
                dts: (now - last_time) / 1000.0,
                tts: (now - initial_time) / 1000.0,
            };

            last_time = now;
            let world_matrix = Matrix4::from_nonuniform_scale(1.0, 1.0, 1.0);

            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            game_tick(functor_runtime_common::to_js_value(&frame_time));

            let val = game_render(functor_runtime_common::to_js_value(&frame_time));
            let frame: Frame = functor_runtime_common::from_js_value(val);

            // Built after the frame so it can carry the frame's lights.
            let render_ctx = RenderContext {
                gl: &gl,
                shader_version,
                asset_cache: asset_cache.clone(),
                frame_time: frame_time.clone(),
                debug_render_mode,
                lights: &frame.lights,
                // Shadows are native-only for now; the wasm runtime renders unshadowed.
                render_pass: functor_runtime_common::RenderPass::Forward,
                shadow: None,
            };

            // Match the drawable buffer to the canvas's displayed (CSS) size,
            // scaled for HiDPI, so the view follows browser/window resizes.
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let draw_w = ((canvas.client_width() as f64) * dpr).round().max(0.0) as u32;
            let draw_h = ((canvas.client_height() as f64) * dpr).round().max(0.0) as u32;
            if canvas.width() != draw_w {
                canvas.set_width(draw_w);
            }
            if canvas.height() != draw_h {
                canvas.set_height(draw_h);
            }
            let viewport = functor_runtime_common::Viewport::new(canvas.width(), canvas.height());
            gl.viewport(0, 0, viewport.width as i32, viewport.height as i32);

            // The game supplies the camera; derive view/projection from it.
            let view_matrix = frame.camera.view_matrix();
            let projection_matrix = frame.camera.projection_matrix(viewport.aspect());

            let mut basic_material = BasicMaterial::create();
            basic_material.initialize(&render_ctx);

            // asset.get().bind(0, &render_ctx);

            functor_runtime_common::Scene3D::render(
                &frame.scene,
                &render_ctx,
                &scene_context,
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
