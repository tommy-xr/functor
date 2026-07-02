use std::cell::{Cell, RefCell};
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
use functor_runtime_common::net::{ConnCommand, HttpMethod, NetCommand};
use functor_runtime_common::protocol::GameProducer;
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

    #[wasm_bindgen(js_namespace = game, js_name = ui)]
    fn game_ui() -> JsValue;

    #[wasm_bindgen(js_namespace = game, js_name = tick)]
    fn game_tick(frameTimeJs: JsValue);

    // Networking bridge into the game module (see Runtime.Wasm). The game queues
    // commands as a JSON string; results are pushed back as JS strings.
    #[wasm_bindgen(js_namespace = game, js_name = net_drain_commands_json)]
    fn game_net_drain_commands() -> JsValue;

    #[wasm_bindgen(js_namespace = game, js_name = net_push_http_response)]
    fn game_net_push_http_response(token: i32, status: i32, body: JsValue);

    #[wasm_bindgen(js_namespace = game, js_name = net_push_http_error)]
    fn game_net_push_http_error(token: i32, message: JsValue);

    // Audio bridge into the game module (see Runtime.Wasm): the game queues
    // play commands as a JSON string, which the host plays via Web Audio.
    #[wasm_bindgen(js_namespace = game, js_name = audio_drain_commands_json_wasm)]
    fn game_audio_drain_commands() -> JsValue;

    // The desired soundscape (`soundScape model`) as a JSON string, reconciled
    // against the live looping voices each frame.
    #[wasm_bindgen(js_namespace = game, js_name = audio_scene_json_wasm)]
    fn game_audio_scene_json() -> JsValue;

    // Persistent connections (WebSocket).
    #[wasm_bindgen(js_namespace = game, js_name = net_drain_conn_commands_json)]
    fn game_net_drain_conn_commands() -> JsValue;

    #[wasm_bindgen(js_namespace = game, js_name = net_push_connected)]
    fn game_net_push_connected(key: JsValue, id: i32);

    #[wasm_bindgen(js_namespace = game, js_name = net_push_conn_message)]
    fn game_net_push_conn_message(key: JsValue, id: i32, text: JsValue);

    #[wasm_bindgen(js_namespace = game, js_name = net_push_disconnected)]
    fn game_net_push_disconnected(key: JsValue, id: i32);

    #[wasm_bindgen(js_namespace = game, js_name = net_push_conn_error)]
    fn game_net_push_conn_error(key: JsValue, id: i32, message: JsValue);
}

/// The web producer: `GameProducer` bridged over the `wasm_bindgen` exports of
/// the game module (the `js_namespace = game` externs above), marshalling
/// through `JsValue`. Zero-sized and stateless — all state lives in the game
/// module — so async callbacks (fetch results, WebSocket events) construct
/// their own instance to push into the inbox; JS is single-threaded, so a push
/// always completes before the next frame's tick.
///
/// Shell-specific no-ops: `check_hot_reload` (the browser reloads the page),
/// input events (the page feeds the game module directly), `state_debug` (no
/// debug server on web), `audio_push_finished` (web one-shots are
/// fire-and-forget — see `play_one_shot`), and `quit` (page lifetime).
struct WasmGame;

impl GameProducer for WasmGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn tick(&mut self, frame_time: FrameTime) {
        game_tick(functor_runtime_common::to_js_value(&frame_time));
    }

    fn key_event(&mut self, _code: i32, _is_down: bool) {}
    fn mouse_move(&mut self, _x: i32, _y: i32) {}
    fn mouse_wheel(&mut self, _delta: i32) {}

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        let val = game_render(functor_runtime_common::to_js_value(&frame_time));
        functor_runtime_common::from_js_value(val)
    }

    fn ui(&self) -> functor_runtime_common::ui::View {
        functor_runtime_common::from_js_value(game_ui())
    }

    fn state_debug(&self) -> String {
        "<state_debug is unavailable on wasm>".to_string()
    }

    fn net_drain_commands(&self) -> String {
        game_net_drain_commands()
            .as_string()
            .unwrap_or_else(|| "[]".to_string())
    }

    fn net_push_http_response(&mut self, token: i32, status: i32, body: String) {
        game_net_push_http_response(token, status, JsValue::from_str(&body));
    }

    fn net_push_http_error(&mut self, token: i32, message: String) {
        game_net_push_http_error(token, JsValue::from_str(&message));
    }

    fn audio_drain_commands(&self) -> String {
        game_audio_drain_commands()
            .as_string()
            .unwrap_or_else(|| "[]".to_string())
    }

    fn audio_scene_json(&self) -> String {
        game_audio_scene_json()
            .as_string()
            .unwrap_or_else(|| "{\"sources\":[]}".to_string())
    }

    fn net_drain_conn_commands(&self) -> String {
        game_net_drain_conn_commands()
            .as_string()
            .unwrap_or_else(|| "[]".to_string())
    }

    fn net_push_connected(&mut self, key: String, conn: i32) {
        game_net_push_connected(to_js(&key), conn);
    }

    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String) {
        game_net_push_conn_message(to_js(&key), conn, to_js(&text));
    }

    fn net_push_disconnected(&mut self, key: String, conn: i32) {
        game_net_push_disconnected(to_js(&key), conn);
    }

    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String) {
        game_net_push_conn_error(to_js(&key), conn, to_js(&message));
    }

    fn audio_push_finished(&mut self, _token: i32) {}

    fn quit(&mut self) {}
}

fn http_method_str(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    }
}

fn js_err(v: JsValue) -> String {
    v.as_string().unwrap_or_else(|| "fetch error".to_string())
}

/// Drain the game's queued networking commands and start a `fetch` for each. The
/// result is pushed back into the game's inbox when the fetch resolves (a later
/// microtask), so the next `tick` decodes it — same shape as the native loop.
/// JS is single-threaded, so a push always completes before the next frame's tick.
fn dispatch_net_commands(game: &dyn GameProducer) {
    let json = game.net_drain_commands();
    if json == "[]" {
        return;
    }
    match serde_json::from_str::<Vec<NetCommand>>(&json) {
        Ok(commands) => {
            for cmd in commands {
                spawn_local(perform_and_push(cmd));
            }
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[net] bad commands json: {e}").into());
        }
    }
}

async fn perform_and_push(cmd: NetCommand) {
    let NetCommand::HttpRequest {
        token,
        method,
        url,
        headers,
        body,
    } = cmd;
    let token = token as i32;
    // WasmGame is stateless, so the async completion pushes through its own
    // instance (see the WasmGame doc).
    match perform_fetch(method, &url, &headers, &body).await {
        Ok((status, text)) => WasmGame.net_push_http_response(token, status, text),
        Err(message) => WasmGame.net_push_http_error(token, message),
    }
}

async fn perform_fetch(
    method: HttpMethod,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(i32, String), String> {
    use wasm_bindgen::JsCast;
    use web_sys::{Request, RequestInit, Response};

    let mut opts = RequestInit::new();
    opts.method(http_method_str(method));
    if !body.is_empty() {
        let text = String::from_utf8_lossy(body).to_string();
        opts.body(Some(&JsValue::from_str(&text)));
    }

    let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
    for (name, value) in headers {
        request.headers().set(name, value).map_err(js_err)?;
    }

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_err)?;
    let response: Response = resp_value
        .dyn_into()
        .map_err(|_| "not a Response".to_string())?;
    let status = response.status() as i32;
    let text_value = JsFuture::from(response.text().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    Ok((status, text_value.as_string().unwrap_or_default()))
}

thread_local! {
    // The Web Audio device, created lazily on the first sound (so it's spun up
    // inside the user-gesture that triggered it, and never on pages with no
    // audio). Decoded buffers are cached by path so repeat plays are instant.
    static AUDIO_CTX: RefCell<Option<web_sys::AudioContext>> = const { RefCell::new(None) };
    static AUDIO_BUFFERS: RefCell<std::collections::HashMap<String, web_sys::AudioBuffer>> =
        RefCell::new(std::collections::HashMap::new());
}

fn audio_context() -> Option<web_sys::AudioContext> {
    AUDIO_CTX.with(|c| {
        if c.borrow().is_none() {
            match web_sys::AudioContext::new() {
                Ok(ctx) => *c.borrow_mut() = Some(ctx),
                Err(e) => {
                    web_sys::console::error_1(&format!("[audio] no AudioContext: {e:?}").into())
                }
            }
        }
        c.borrow().clone()
    })
}

thread_local! {
    // Where the player hears from (the render camera), updated each frame. Both
    // one-shots and looping voices spatialize against this — there's no Web Audio
    // AudioListener (its position API is deprecated/ignored in modern browsers);
    // we compute gain + pan ourselves so it always tracks the camera.
    static CURRENT_LISTENER: std::cell::Cell<functor_runtime_common::audio::Listener> =
        std::cell::Cell::new(functor_runtime_common::audio::Listener {
            position: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
        });
}

fn current_listener() -> functor_runtime_common::audio::Listener {
    CURRENT_LISTENER.with(|l| l.get())
}

/// Drain the game's queued audio commands and play each via Web Audio. Mirrors
/// `dispatch_net_commands`; called each frame after `tick`.
fn dispatch_audio_commands(game: &dyn GameProducer) {
    let json = game.audio_drain_commands();
    if json == "[]" {
        return;
    }
    match serde_json::from_str::<Vec<functor_runtime_common::audio::AudioCommand>>(&json) {
        Ok(commands) => {
            for cmd in commands {
                spawn_local(play_one_shot(cmd));
            }
        }
        Err(e) => web_sys::console::error_1(&format!("[audio] bad commands json: {e}").into()),
    }
}

async fn play_one_shot(cmd: functor_runtime_common::audio::AudioCommand) {
    use wasm_bindgen::JsCast;
    // `token` (completion reporting) is native-only for now — the web backend
    // plays fire-and-forget and never reports a finish.
    let functor_runtime_common::audio::AudioCommand::PlayOneShot {
        token: _,
        sound,
        gain,
        position,
    } = cmd;

    let ctx = match audio_context() {
        Some(c) => c,
        None => return,
    };
    // Browsers start the context suspended until a user gesture; the play is
    // driven by one (a keypress), so a best-effort resume is enough.
    let _ = ctx.resume();

    let buffer = match decode_buffer(&ctx, &sound).await {
        Some(b) => b,
        None => return,
    };

    // source -> [stereo panner] -> gain -> speakers. A positioned one-shot routes
    // through a StereoPannerNode; both its gain and pan come from the shared
    // `spatialize` (relative to the current listener), so native and wasm
    // attenuate identically. The audio graph keeps the nodes alive until the
    // source finishes, so the Rust bindings can drop here.
    let source = match ctx.create_buffer_source() {
        Ok(s) => s,
        Err(_) => return,
    };
    source.set_buffer(Some(&buffer));
    if let Ok(gain_node) = ctx.create_gain() {
        let _ = gain_node.connect_with_audio_node(&ctx.destination());
        let head = spatial_head(&ctx, &gain_node, gain, position);
        let _ = source.connect_with_audio_node(&head);
    }
    let _ = source.start();
}

/// Wire the gain (and, for a positioned voice, a StereoPannerNode) for a voice,
/// returning the node a fresh source should connect into. Sets the gain/pan from
/// the shared `spatialize` so the distance falloff matches the native backend.
fn spatial_head(
    ctx: &web_sys::AudioContext,
    gain_node: &web_sys::GainNode,
    base_gain: f32,
    position: Option<[f32; 3]>,
) -> web_sys::AudioNode {
    use wasm_bindgen::JsCast;
    match position {
        Some(pos) => {
            let s = current_listener().spatialize(pos);
            gain_node.gain().set_value(base_gain * s.gain);
            match ctx.create_stereo_panner() {
                Ok(panner) => {
                    panner.pan().set_value(s.pan);
                    let _ = panner.connect_with_audio_node(gain_node);
                    panner.unchecked_into()
                }
                Err(_) => gain_node.clone().unchecked_into(),
            }
        }
        None => {
            gain_node.gain().set_value(base_gain);
            gain_node.clone().unchecked_into()
        }
    }
}

/// Fetch + decode a sound to an `AudioBuffer`, caching by path so repeat uses
/// (one-shots and looping voices) are instant. `None` on any load/decode error.
async fn decode_buffer(
    ctx: &web_sys::AudioContext,
    sound: &str,
) -> Option<web_sys::AudioBuffer> {
    use wasm_bindgen::JsCast;

    if let Some(b) = AUDIO_BUFFERS.with(|b| b.borrow().get(sound).cloned()) {
        return Some(b);
    }
    let bytes = match functor_runtime_common::io::load_bytes_async(sound).await {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::error_1(&format!("[audio] load '{sound}': {e}").into());
            return None;
        }
    };
    // decodeAudioData wants an ArrayBuffer (and detaches it); the Uint8Array
    // copies the bytes into a standalone JS buffer.
    let array = js_sys::Uint8Array::from(&bytes[..]);
    let promise = match ctx.decode_audio_data(&array.buffer()) {
        Ok(p) => p,
        Err(e) => {
            web_sys::console::error_1(&format!("[audio] decode '{sound}': {e:?}").into());
            return None;
        }
    };
    let buf: web_sys::AudioBuffer = match JsFuture::from(promise).await {
        Ok(v) => v.dyn_into().ok()?,
        Err(e) => {
            web_sys::console::error_1(&format!("[audio] decode '{sound}': {e:?}").into());
            return None;
        }
    };
    AUDIO_BUFFERS.with(|b| b.borrow_mut().insert(sound.to_string(), buf.clone()));
    Some(buf)
}

// --- Soundscape: continuous looping voices, reconciled by key each frame. -------
//
// The Web Audio counterpart of the native rodio voice registry. Each positioned
// voice routes through a StereoPannerNode; both its gain and pan come from the
// shared `spatialize` (computed against CURRENT_LISTENER) and are re-applied each
// frame, so the voice pans/attenuates as the camera moves — the same linear
// falloff the native backend uses (no Web Audio PannerNode / AudioListener).

struct WebVoice {
    source: functor_runtime_common::audio::AudioSource, // last applied (for diffing)
    gain: web_sys::GainNode,
    panner: Option<web_sys::StereoPannerNode>,
    // The looping source node, attached once its buffer decodes (async). Shared
    // so the decode task can install it and `stop` can reach it.
    node: Rc<RefCell<Option<web_sys::AudioBufferSourceNode>>>,
    // Set if the voice is stopped before its buffer finished decoding, so the
    // decode task discards its result instead of starting an orphan.
    cancelled: Rc<Cell<bool>>,
}

thread_local! {
    static SOUNDSCAPE: RefCell<std::collections::HashMap<String, WebVoice>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Re-apply the shared spatialization (gain + pan) to a live positioned voice for
/// the current listener. No-op for non-spatial beds (their gain doesn't depend on
/// the listener).
fn respatialize_voice(voice: &WebVoice) {
    if let (Some(panner), Some(pos)) = (&voice.panner, voice.source.position) {
        let s = current_listener().spatialize(pos);
        voice.gain.gain().set_value(voice.source.gain * s.gain);
        panner.pan().set_value(s.pan);
    }
}

/// Aim the listener from the frame camera and reconcile the desired soundscape
/// against the live voices each frame: spawn new ones, stop gone ones, update
/// changed gain/position in place. Skips entirely (and never spins up an
/// AudioContext) when nothing is playing and nothing is wanted.
fn update_soundscape(game: &dyn GameProducer, camera: &functor_runtime_common::Camera) {
    // Track the listener from the camera every frame (cheap, no AudioContext
    // needed), so positioned one-shots (`playAt`) spatialize correctly even for a
    // game with no soundscape.
    CURRENT_LISTENER.with(|l| {
        l.set(functor_runtime_common::audio::Listener::from_eye_target_up(
            camera.eye,
            camera.target,
            camera.up,
        ))
    });

    let json = game.audio_scene_json();
    let nothing_live = SOUNDSCAPE.with(|s| s.borrow().is_empty());
    if nothing_live && (json.is_empty() || json == "{\"sources\":[]}") {
        return;
    }
    let ctx = match audio_context() {
        Some(c) => c,
        None => return,
    };
    // The context starts suspended (autoplay policy). Looping beds aren't driven
    // by a gesture like one-shots are, so resume best-effort each frame; it takes
    // effect once the user has interacted with the page (canvas keypress/click).
    let _ = ctx.resume();

    let scene: functor_runtime_common::audio::AudioScene = match serde_json::from_str(&json) {
        Ok(s) => s,
        Err(e) => {
            web_sys::console::error_1(&format!("[audio] bad scene json: {e}").into());
            return;
        }
    };
    let live: std::collections::HashMap<String, functor_runtime_common::audio::AudioSource> =
        SOUNDSCAPE.with(|s| {
            s.borrow()
                .iter()
                .map(|(k, v)| (k.clone(), v.source.clone()))
                .collect()
        });
    for update in functor_runtime_common::audio::reconcile(&live, &scene) {
        use functor_runtime_common::audio::SceneUpdate;
        match update {
            SceneUpdate::Spawn(src) => spawn_voice(&ctx, src),
            SceneUpdate::Update(src) => update_voice(&ctx, src),
            SceneUpdate::Stop(key) => stop_voice(&key),
        }
    }

    // Re-apply spatialization to every live positioned voice for the (moved) listener.
    SOUNDSCAPE.with(|s| {
        for v in s.borrow().values() {
            respatialize_voice(v);
        }
    });
}

fn spawn_voice(ctx: &web_sys::AudioContext, src: functor_runtime_common::audio::AudioSource) {
    use wasm_bindgen::JsCast;

    let _ = ctx.resume();
    let gain = match ctx.create_gain() {
        Ok(g) => g,
        Err(_) => return,
    };
    let _ = gain.connect_with_audio_node(&ctx.destination());

    // Positioned voices route through a StereoPannerNode; gain + pan come from the
    // shared `spatialize` (re-applied each frame by `respatialize_voice`).
    let panner: Option<web_sys::StereoPannerNode> = match src.position {
        Some(pos) => {
            let s = current_listener().spatialize(pos);
            gain.gain().set_value(src.gain * s.gain);
            match ctx.create_stereo_panner() {
                Ok(p) => {
                    p.pan().set_value(s.pan);
                    let _ = p.connect_with_audio_node(&gain);
                    Some(p)
                }
                Err(_) => None,
            }
        }
        None => {
            gain.gain().set_value(src.gain);
            None
        }
    };
    // The node a fresh source connects into: the panner (positioned) or the gain.
    let head: web_sys::AudioNode = match &panner {
        Some(p) => p.clone().unchecked_into(),
        None => gain.clone().unchecked_into(),
    };

    let node: Rc<RefCell<Option<web_sys::AudioBufferSourceNode>>> = Rc::new(RefCell::new(None));
    let cancelled = Rc::new(Cell::new(false));
    SOUNDSCAPE.with(|s| {
        s.borrow_mut().insert(
            src.key.clone(),
            WebVoice {
                source: src.clone(),
                gain,
                panner,
                node: node.clone(),
                cancelled: cancelled.clone(),
            },
        );
    });

    // Decode (async) then attach + loop + start — unless the voice was stopped
    // (or respawned) before the buffer was ready.
    let ctx = ctx.clone();
    let sound = src.sound.clone();
    spawn_local(async move {
        let Some(buffer) = decode_buffer(&ctx, &sound).await else {
            return;
        };
        if cancelled.get() {
            return;
        }
        let Ok(source) = ctx.create_buffer_source() else {
            return;
        };
        source.set_buffer(Some(&buffer));
        source.set_loop(true);
        let _ = source.connect_with_audio_node(&head);
        let _ = source.start();
        *node.borrow_mut() = Some(source);
    });
}

fn update_voice(ctx: &web_sys::AudioContext, src: functor_runtime_common::audio::AudioSource) {
    // A flip in spatial-ness (None <-> Some) changes the node graph; respawn.
    let flip = SOUNDSCAPE.with(|s| {
        s.borrow()
            .get(&src.key)
            .map(|v| v.source.position.is_some() != src.position.is_some())
            .unwrap_or(true)
    });
    if flip {
        stop_voice(&src.key);
        spawn_voice(ctx, src);
        return;
    }
    SOUNDSCAPE.with(|s| {
        if let Some(v) = s.borrow_mut().get_mut(&src.key) {
            v.source = src;
            // Positioned voices re-spatialize (gain + pan); non-spatial beds just
            // take the new gain directly.
            if v.panner.is_some() {
                respatialize_voice(v);
            } else {
                v.gain.gain().set_value(v.source.gain);
            }
        }
    });
}

fn stop_voice(key: &str) {
    if let Some(v) = SOUNDSCAPE.with(|s| s.borrow_mut().remove(key)) {
        v.cancelled.set(true);
        if let Some(node) = v.node.borrow().as_ref() {
            let _ = node.stop();
            let _ = node.disconnect();
        }
        let _ = v.gain.disconnect();
        if let Some(p) = &v.panner {
            let _ = p.disconnect();
        }
    }
}

/// Browser WebSocket client state (client only — browsers can't listen). Lives
/// for the page; the per-socket event handlers are `forget()`-leaked, which keeps
/// them alive without a reference cycle through this table.
#[derive(Default)]
struct WsClient {
    conns: std::collections::HashMap<u64, web_sys::WebSocket>,
    by_key: std::collections::HashMap<String, u64>,
    next_id: u64,
}

fn to_js(s: &str) -> JsValue {
    functor_runtime_common::to_js_value(&s.to_string())
}

/// Drain the game's queued connection commands and perform them with browser
/// WebSockets; socket events are pushed back into the game from the handlers.
fn dispatch_conn_commands(game: &dyn GameProducer, state: &Rc<RefCell<WsClient>>) {
    let json = game.net_drain_conn_commands();
    if json == "[]" {
        return;
    }
    let commands: Vec<ConnCommand> = match serde_json::from_str(&json) {
        Ok(c) => c,
        Err(e) => {
            web_sys::console::error_1(&format!("[net] bad conn commands json: {e}").into());
            return;
        }
    };
    for cmd in commands {
        match cmd {
            ConnCommand::Connect { key, url } => ws_connect(state, key, url),
            ConnCommand::Listen { .. } => {
                web_sys::console::warn_1(
                    &"[net] Sub.listen is unsupported in the browser (client only)".into(),
                );
            }
            ConnCommand::Send { conn, payload } => {
                if let Some(ws) = state.borrow().conns.get(&conn) {
                    let _ = ws.send_with_str(&String::from_utf8_lossy(&payload));
                }
            }
            ConnCommand::CloseConn { conn } => {
                if let Some(ws) = state.borrow().conns.get(&conn) {
                    let _ = ws.close();
                }
            }
            ConnCommand::CloseKey { key } => {
                let id = state.borrow().by_key.get(&key).copied();
                if let Some(id) = id {
                    if let Some(ws) = state.borrow().conns.get(&id) {
                        let _ = ws.close();
                    }
                }
            }
        }
    }
}

fn ws_connect(state: &Rc<RefCell<WsClient>>, key: String, url: String) {
    // Idempotent by key (matches the native host); a re-declared connection
    // reattaches rather than opening a second socket.
    if state.borrow().by_key.contains_key(&key) {
        return;
    }
    let ws = match web_sys::WebSocket::new(&url) {
        Ok(ws) => ws,
        Err(_) => {
            WasmGame.net_push_conn_error(key, 0, "failed to open WebSocket".to_string());
            return;
        }
    };
    let id = {
        let mut s = state.borrow_mut();
        s.next_id += 1;
        let id = s.next_id;
        s.conns.insert(id, ws.clone());
        s.by_key.insert(key.clone(), id);
        id
    };
    let iid = id as i32;

    let on_open = {
        let key = key.clone();
        Closure::<dyn FnMut()>::new(move || WasmGame.net_push_connected(key.clone(), iid))
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_message = {
        let key = key.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Some(text) = e.data().as_string() {
                WasmGame.net_push_conn_message(key.clone(), iid, text);
            }
        })
    };
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_close = {
        let key = key.clone();
        let state = state.clone();
        Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_e: web_sys::CloseEvent| {
            WasmGame.net_push_disconnected(key.clone(), iid);
            // Drop our handle so a still-declared Sub.connect reconnects next frame.
            let mut s = state.borrow_mut();
            s.conns.remove(&id);
            s.by_key.remove(&key);
        })
    };
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    let on_error = {
        let key = key.clone();
        Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |e: web_sys::ErrorEvent| {
            WasmGame.net_push_conn_error(key.clone(), iid, e.message());
        })
    };
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();
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
            // Arc so the egui text-overlay painter can share this same context.
            let gl = std::sync::Arc::new(glow::Context::from_webgl2_context(webgl2_context));
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
        // The producer this loop drives — the wasm_bindgen bridge to the game
        // module, behind the shared GameProducer seam (docs/mle.md, Track A2).
        // Deliberately the concrete type, not a Box<dyn>: the async completions
        // (fetch results, WebSocket events) push through their own stateless
        // WasmGame, so swapping in a different web producer requires threading
        // a shared handle into perform_and_push/ws_connect first — a boxed
        // producer here would compile and then silently lose those events.
        let mut game = WasmGame;
        let ws_state = Rc::new(RefCell::new(WsClient::default()));
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

        // The 2D UI overlay (egui), painting the game's `ui model` View on top of
        // the 3D frame — the web sibling of the desktop runner's overlay.
        let mut text_overlay = functor_runtime_common::ui::TextOverlay::new(gl.clone());

        // In deterministic mode (?fixed-time, the golden) the canvas is sized
        // once and then left fixed (see below), and the render loop stops after
        // a short warm-up so the page is static for screenshotting.
        let mut sized = false;

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

            game.tick(frame_time.clone());

            // Perform any networking commands this frame's tick queued; results
            // are pushed back into the inbox asynchronously and decoded by a later
            // tick (see dispatch_net_commands).
            dispatch_net_commands(&game);
            // Play any one-shot sounds this frame's tick queued (fetch + decode
            // the first time, then from the cache).
            dispatch_audio_commands(&game);
            dispatch_conn_commands(&game, &ws_state);

            let frame: Frame = game.render(frame_time.clone());

            // Soundscape: aim the listener from this frame's camera, then
            // reconcile the desired looping voices against the live ones.
            update_soundscape(&game, &frame.camera);

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

            // 2D UI overlay: the game's declarative `ui model` View, lowered to a
            // text overlay on top of the frame (HiDPI-aware via the device ratio).
            let view: functor_runtime_common::ui::View = game.ui();
            let dpr = web_sys::window().unwrap().device_pixel_ratio() as f32;
            text_overlay.draw_view(canvas.width(), canvas.height(), dpr.max(1.0), &view);

            // Schedule the next frame. In deterministic mode (?fixed-time, the
            // golden) render a short warm-up (shader compile, first-frame
            // settling) then stop, so the page is perfectly static: the golden
            // screenshot then never has to chase a stable frame (CI's swiftshader
            // isn't bit-identical frame to frame). Gate on wall-clock elapsed,
            // not a frame count, so the loop reliably stops before the test
            // screenshots regardless of the CI runner's frame rate.
            if fixed_time.is_none() || (now - initial_time) < 1000.0 {
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
