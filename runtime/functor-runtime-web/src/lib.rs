use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use functor_runtime_common::asset::pipelines::TexturePipeline;
use functor_runtime_common::asset::{AssetCache, AssetLoader};
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::io::load_bytes_async;
use functor_runtime_common::texture::{
    RuntimeTexture, Texture2D, TextureData, TextureFormat, TextureOptions, PNG,
};
use functor_runtime_common::{Frame, FrameTime, SceneContext};
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

/// The wasm counterpart of the desktop `--fixed-time` flag: read
/// `?fixed-time=<seconds>` from the page URL to pin the frame time, so the
/// render is deterministic (for headless golden screenshots). Returns `None`
/// when absent or unparseable (normal wall-clock animation).
fn fixed_time_from_url() -> Option<f32> {
    let search = window().location().search().unwrap_or_default();
    let query = search.trim_start_matches('?');
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some("fixed-time") {
            return kv.next().and_then(|v| v.parse::<f32>().ok());
        }
    }
    None
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

        // Read once from the page URL; they don't change over the session. The
        // `move` closure below captures them (both are `Copy`).
        let debug_render_mode = debug_render_mode_from_url();
        let fixed_time = fixed_time_from_url();

        // The directional shadow map, rendered from the casting light each frame
        // and sampled by the lit material (mirrors the desktop runtime).
        let shadow_map = functor_runtime_common::shadow::ShadowMap::new(&gl, 2048);

        // In deterministic mode (?fixed-time, the golden) the canvas is sized
        // once and then left fixed (see below), and the render loop stops after
        // a few frames so the page is static for screenshotting.
        let mut sized = false;
        let mut frame_count = 0u32;

        *g.borrow_mut() = Some(Closure::new(move || {
            let now = performance.now() as f32;
            // Pin the frame time when `?fixed-time` is set (deterministic capture).
            let frame_time = match fixed_time {
                Some(t) => FrameTime { dts: 0.0, tts: t },
                None => FrameTime {
                    dts: (now - last_time) / 1000.0,
                    tts: (now - initial_time) / 1000.0,
                },
            };

            last_time = now;

            game_tick(functor_runtime_common::to_js_value(&frame_time));

            let val = game_render(functor_runtime_common::to_js_value(&frame_time));
            let frame: Frame = functor_runtime_common::from_js_value(val);

            // Match the drawable buffer to the canvas's displayed (CSS) size,
            // scaled for HiDPI, so the view follows browser/window resizes. In
            // deterministic mode (?fixed-time, the golden), size it once layout
            // is ready and then leave it fixed: the per-frame resize otherwise
            // jitters the canvas element under headless CI and prevents
            // Playwright from getting a stable screenshot.
            if fixed_time.is_none() || !sized {
                let dpr = web_sys::window().unwrap().device_pixel_ratio();
                let cw = canvas.client_width();
                let ch = canvas.client_height();
                if cw > 0 && ch > 0 {
                    let draw_w = ((cw as f64) * dpr).round().max(1.0) as u32;
                    let draw_h = ((ch as f64) * dpr).round().max(1.0) as u32;
                    if canvas.width() != draw_w {
                        canvas.set_width(draw_w);
                    }
                    if canvas.height() != draw_h {
                        canvas.set_height(draw_h);
                    }
                    sized = true;
                }
            }
            let viewport = functor_runtime_common::Viewport::new(canvas.width(), canvas.height());

            // Shadow + forward passes, shared with the desktop runtime.
            functor_runtime_common::render_frame(
                &gl,
                shader_version,
                asset_cache.clone(),
                &scene_context,
                &shadow_map,
                &frame,
                frame_time.clone(),
                viewport,
                debug_render_mode,
            );

            // Schedule the next frame. In deterministic mode (?fixed-time, the
            // golden) render a few warm-up frames (shader compile, first-frame
            // settling) then stop, so the page is perfectly static: the golden
            // screenshot then never has to chase a stable frame (CI's
            // swiftshader isn't bit-identical frame to frame).
            frame_count += 1;
            if fixed_time.is_none() || frame_count < 30 {
                request_animation_frame(f.borrow().as_ref().unwrap());
            }
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
