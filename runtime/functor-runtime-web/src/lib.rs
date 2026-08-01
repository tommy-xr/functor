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
use functor_runtime_common::functor_lang_game_embedded::FunctorLangEmbeddedGame;
use functor_runtime_common::geometry::Geometry;
use functor_runtime_common::io::load_bytes_async;
use functor_runtime_common::net::{
    parse_delivered_events, ConnCommand, DeliveredEvent, HttpMethod, NetCommand,
};
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::texture::{
    RuntimeTexture, Texture2D, TextureData, TextureFormat, TextureOptions, PNG,
};
use functor_runtime_common::viewer::{
    camera_frustum_lines, DebugCamera, DebugCameraMode, DebugMaterialMode, DebugPresentation,
};
use functor_runtime_common::{
    Frame, FrameTime, GameClock, InputEdges, InputSnapshot, SceneContext,
};
use glow::*;
use js_sys::{Function, Object, WebAssembly};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use wasm_bindgen::prelude::*;

mod functor_lang_game;
mod sim;
use functor_lang_game::WebPlatform;

fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

/// The red error overlay's inline style — a fixed panel pinned to the top of
/// the page, above the canvas, scrollable if the message is long. Kept as one
/// string so `show`/`hide` toggle the same element.
const ERROR_OVERLAY_STYLE: &str = "position:fixed;top:var(--functor-scrubber-h, 0px);left:0;right:0;max-height:60%;\
overflow:auto;z-index:2147483647;margin:0;padding:12px 16px;background:#2b0a0a;color:#ffb3b3;\
border-bottom:2px solid #ff5555;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;\
white-space:pre-wrap;box-shadow:0 2px 12px rgba(0,0,0,.5)";

/// Show (or update) a red error overlay in the page — the web runtime's take on
/// React's hot-reload error screen, so a failed load or a broken edit shows the
/// message instead of a blank canvas (the desktop runner prints to stderr; the
/// browser has no console the user is watching). Idempotent: reuses one
/// `#functor-lang-error` element. Best-effort — a missing document just no-ops.
fn show_error_overlay(message: &str) {
    let Some(doc) = window().document() else {
        return;
    };
    let el = match doc.get_element_by_id("functor-lang-error") {
        Some(el) => el,
        None => {
            let Ok(el) = doc.create_element("div") else {
                return;
            };
            el.set_id("functor-lang-error");
            if let Some(body) = doc.body() {
                let _ = body.append_child(&el);
            }
            el
        }
    };
    let _ = el.set_attribute("style", ERROR_OVERLAY_STYLE);
    el.set_text_content(Some(&format!("⛔ Functor Lang error\n\n{message}")));
}

/// Hide the error overlay if present — called after a successful (re)load so a
/// fixed edit clears the panel.
fn hide_error_overlay() {
    if let Some(doc) = window().document() {
        if let Some(el) = doc.get_element_by_id("functor-lang-error") {
            let _ = el.set_attribute("style", "display:none");
        }
    }
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

/// Releases suppressed by an interactive transport still update physical
/// levels, so resuming cannot leave a key or button stuck. Deterministic
/// fixed-time capture keeps the entire snapshot frozen.
fn recover_suppressed_releases(clock: &GameClock, sim_running: bool) -> bool {
    !clock.is_fixed_time() && (clock.is_paused() || sim_running)
}

/// The wasm counterpart of the desktop `--functor-lang --game-path` flags: the page
/// sets `window.__functorLangGamePath` to the entry file before initializing this
/// module (the CLI's Functor Lang index page substitutes the project's `functor.json`
/// entry — see `index-functor-lang.html` / the CLI's `wasm_dev_server.rs`), and the
/// runtime fetches + interprets that source. Absent (the entry was not set)
/// this returns `None` and `run_async` fails loud.
fn functor_lang_game_path() -> Option<String> {
    js_sys::Reflect::get(&window(), &JsValue::from_str("__functorLangGamePath"))
        .ok()
        .and_then(|v| v.as_string())
}

/// The role's entry-point binding prefix (same-file entries, mirroring the
/// desktop `--entry-prefix` flag): the page sets
/// `window.__functorLangEntryPrefix` before initializing this module (the
/// CLI's Functor Lang index page substitutes the role's declared prefix;
/// the site's player.html reads `?prefix=<ident>`), and the producer
/// resolves every canonical entry binding through it as camelCase
/// (`"server"` → `serverInit`/`serverTick`/…). Absent or empty = the
/// classic unprefixed contract.
fn functor_lang_entry_prefix() -> String {
    js_sys::Reflect::get(&window(), &JsValue::from_str("__functorLangEntryPrefix"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Where the runtime's networking goes. Three routings exist: the in-process
/// netsim (`sim::is_running()`, which short-circuits everything before this
/// choice is even reached), real browser WebSockets, and the *embedder* — the
/// page hosting this runtime.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NetTransport {
    /// `window.__functorNetTransport` absent or `"websocket"`: open real sockets.
    WebSocket,
    /// `window.__functorNetTransport === "embedder"`: post the drained
    /// [`ConnCommand`]s to the embedding page and take events back from it
    /// through [`functor_lang_net_deliver`]. No socket is opened.
    Embedder,
}

/// Read the boot global once (it selects a routing for the page's lifetime, so
/// re-reading it per frame would only invite a mid-run flip). The CLI's dev
/// index page deliberately does NOT set it — there is no coordinator behind
/// `functor run wasm`, so it defaults to WebSocket there; the site's
/// `player.html` sets it from `?net=embedder`.
fn net_transport() -> NetTransport {
    thread_local! {
        static TRANSPORT: Cell<Option<NetTransport>> = const { Cell::new(None) };
    }
    TRANSPORT.with(|cached| {
        if let Some(transport) = cached.get() {
            return transport;
        }
        let value = js_sys::Reflect::get(&window(), &JsValue::from_str("__functorNetTransport"))
            .ok()
            .and_then(|v| v.as_string());
        let transport = match value.as_deref() {
            Some("embedder") => NetTransport::Embedder,
            None | Some("websocket") => NetTransport::WebSocket,
            Some(other) => {
                web_sys::console::warn_1(
                    &format!(
                        "[net] unknown window.__functorNetTransport={other}; using websockets"
                    )
                    .into(),
                );
                NetTransport::WebSocket
            }
        };
        cached.set(Some(transport));
        transport
    })
}

/// The project's full file list (entry FIRST, then siblings), as the CLI's Functor Lang
/// index page injects it (`window.__functorLangProjectFiles`, mirroring `__functorLangGamePath`
/// — see `wasm_dev_server.rs`). Absent (a page that only set the single entry,
/// e.g. the site sandbox) → `None`, and the caller falls back to the entry
/// alone.
fn functor_lang_project_files() -> Option<Vec<String>> {
    use wasm_bindgen::JsCast;
    let value =
        js_sys::Reflect::get(&window(), &JsValue::from_str("__functorLangProjectFiles")).ok()?;
    let array = value.dyn_into::<js_sys::Array>().ok()?;
    let files: Vec<String> = array.iter().filter_map(|v| v.as_string()).collect();
    (!files.is_empty()).then_some(files)
}

/// In-memory project sources (`window.__functorLangProjectSources`, an array of
/// `{path, source}` objects, entry FIRST) — set by a page that holds the
/// whole project in memory instead of serving it (the IDE's inline boot,
/// see `player.html?project=inline`). Absent or malformed → `None`, and the
/// caller uses the fetch path.
fn functor_lang_project_sources() -> Option<Vec<(String, String)>> {
    let value =
        js_sys::Reflect::get(&window(), &JsValue::from_str("__functorLangProjectSources")).ok()?;
    parse_project_files(&value)
}

/// Parse an array of `{path, source}` objects (both strings) into the
/// producer's `(path, source)` pairs — shared by the inline-boot global
/// above and the `functor_lang_set_project` push below. `None` when the value
/// isn't that shape (including an empty array).
fn parse_project_files(value: &JsValue) -> Option<Vec<(String, String)>> {
    use wasm_bindgen::JsCast;
    let array = value.dyn_ref::<js_sys::Array>()?;
    let mut files = Vec::with_capacity(array.length() as usize);
    for item in array.iter() {
        let path = js_sys::Reflect::get(&item, &JsValue::from_str("path"))
            .ok()?
            .as_string()?;
        let source = js_sys::Reflect::get(&item, &JsValue::from_str("source"))
            .ok()?
            .as_string()?;
        files.push((path, source));
    }
    (!files.is_empty()).then_some(files)
}

/// Fetch every project `.fun`/`.funi` source (entry first, then siblings) and
/// build the interpreter producer. `file = module`, so a game split across
/// `game.fun` + `pieces.fun` links exactly as it does natively. Failures are
/// rendered strings (fetch status, parse/load position, contract violation) for
/// `run_async` to fail loud with.
async fn create_functor_lang_game(entry: &str) -> Result<FunctorLangEmbeddedGame, String> {
    let sources = load_project_sources(entry).await?;
    FunctorLangEmbeddedGame::create_with_prefix(
        sources,
        &functor_lang_entry_prefix(),
        Box::new(WebPlatform::new()),
    )
}

/// The project's `(path, source)` pairs, however this page supplies them —
/// in-memory or fetched. Split out from [`create_functor_lang_game`] because the
/// netsim (`sim.rs`) needs the same sources to build SEVERAL producers from,
/// one per role, without fetching the project once per instance.
pub(crate) async fn load_project_sources(entry: &str) -> Result<Vec<(String, String)>, String> {
    // A page that already holds every source in memory (the IDE's
    // `?project=inline` boot) injects them directly — nothing to fetch, and
    // module names come from the given paths exactly as in the fetch path.
    if let Some(sources) = functor_lang_project_sources() {
        return Ok(sources);
    }
    // The CLI injects the whole project file list; a page that set only the
    // entry (or none) falls back to loading the entry alone.
    let paths = functor_lang_project_files().unwrap_or_else(|| vec![entry.to_string()]);
    let mut sources: Vec<(String, String)> = Vec::with_capacity(paths.len());
    for path in &paths {
        // `no_store`: never serve project source from the browser cache — the
        // dev server reuses `/game.fun` across samples, so a cached response
        // would keep showing the previous game after switching projects.
        let (status, src) = perform_fetch(HttpMethod::Get, path, &[], &[], true)
            .await
            .map_err(|e| format!("cannot fetch {path}: {e}"))?;
        if status != 200 {
            return Err(format!("cannot fetch {path}: HTTP {status}"));
        }
        sources.push((path.clone(), src));
    }
    Ok(sources)
}

thread_local! {
    /// The live producer, shared between the frame loop and the
    /// `functor_lang_set_source` export below (docs/functor-lang.md D4). `None` until
    /// `run_async` has built it (still fetching, or the load failed).
    static GAME: RefCell<Option<Rc<RefCell<Box<dyn GameProducer>>>>> =
        const { RefCell::new(None) };
}

/// Is the game producer installed yet? The preview page polls this before
/// announcing readiness — a push before the producer exists would be
/// dropped ("game is not running yet").
#[wasm_bindgen]
pub fn functor_lang_is_running() -> bool {
    GAME.with(|g| g.borrow().is_some())
}

/// Does the live producer define any hook that shell input reaches only while
/// the pointer is captured? Host pages use this at load and after source pushes
/// to teach when an explicit `mouseCapture: false` disables captured hooks.
#[wasm_bindgen]
pub fn functor_lang_uses_captured_mouse_input() -> bool {
    GAME.with(|g| {
        g.borrow()
            .as_ref()
            .is_some_and(|game| game.borrow().uses_captured_mouse_input())
    })
}

/// A queued push: the classic single-buffer text push (the sandbox / VSCode
/// preview editing the entry over served siblings), or the whole-project
/// push (the IDE, which owns every file in memory).
enum PendingPush {
    Source(String),
    Project(Vec<(String, String)>),
}

thread_local! {
    /// The push queued via `functor_lang_set_source` / `functor_lang_set_project` (with
    /// the pusher's optional correlation id), waiting to be applied at a SAFE
    /// point (the top of the frame loop, where the loop already holds the
    /// producer borrow). Deferring — rather than reloading straight from the
    /// message handler — is what keeps a push from ever colliding with the
    /// frame's borrow ("runtime is mid-frame"); it also coalesces a burst of
    /// edits to the last one (across BOTH push kinds: last push wins).
    /// Mirrors the desktop runner, which applies reloads between frames.
    static PENDING_RELOAD: RefCell<Option<(PendingPush, Option<f64>)>> =
        const { RefCell::new(None) };
}

/// Post a `functor-lang-set-source-result` back to the page (the reload's outcome). The
/// pusher — the VSCode live preview, the site sandbox, a test harness — listens
/// for this. Because the reload is applied asynchronously (next frame), the
/// result is delivered here rather than returned from `functor_lang_set_source`.
/// `id` echoes the push's correlation id, if the pusher sent one, so a pusher
/// can match a (possibly stale) result to its latest push; id-less pushes get
/// id-less results — the pre-id protocol, unchanged.
fn post_reload_result(ok: bool, message: &str, id: Option<f64>) {
    let obj = js_sys::Object::new();
    let set = |k: &str, v: &JsValue| {
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(k), v);
    };
    set("type", &JsValue::from_str("functor-lang-set-source-result"));
    set("ok", &JsValue::from_bool(ok));
    set("message", &JsValue::from_str(message));
    if let Some(id) = id {
        set("id", &JsValue::from_f64(id));
    }
    // Deliver to the PARENT — the push's original sender (the VSCode preview
    // host, the site sandbox frame). When the page is top-level, `parent` is the
    // window itself, so a standalone page / test harness listening on `window`
    // still receives it.
    let target = window().parent().ok().flatten().unwrap_or_else(window);
    let _ = target.post_message(&obj, "*");
}

/// Apply a pending pushed source, if any — called at the top of each frame while
/// the frame loop holds the producer borrow, so it never collides with a frame.
/// A good push clears the error overlay; a broken one shows it; either way the
/// outcome is posted back to the page.
fn apply_pending_reload(game: &mut dyn GameProducer) {
    let Some((push, id)) = PENDING_RELOAD.with(|p| p.borrow_mut().take()) else {
        return;
    };
    let selected_frame = game
        .current_scene_frame()
        .or_else(|| game.scene_frame_range().map(|(_, hi)| hi))
        .unwrap_or(0);
    let outcome = match push {
        PendingPush::Source(source) => game.reload_source(&source),
        PendingPush::Project(files) => game.reload_project(&files),
    };
    match outcome {
        Ok(status) => {
            // The live scene remains on the selected/current frame across the
            // swap. Mark that exact boundary; using the next frame makes a
            // paused marker sit outside the seekable range and get pruned.
            let reload_frame = game.current_scene_frame().unwrap_or(selected_frame);
            functor_lang_game::publish_timeline_reload(reload_frame, true, &status);
            hide_error_overlay();
            post_reload_result(true, &status, id);
        }
        Err(message) => {
            functor_lang_game::publish_timeline_reload(selected_frame, false, &message);
            show_error_overlay(&format!("[functor-lang] reload error: {message}"));
            post_reload_result(false, &message, id);
        }
    }
}

/// Hot-swap the running game's logic from pushed `.fun` source — the wasm
/// counterpart of the desktop runner's `POST /reload-source` (docs/functor-lang.md D4).
/// The source is QUEUED and applied at the top of the next frame (see
/// [`apply_pending_reload`]); the outcome is delivered asynchronously as a
/// `functor-lang-set-source-result` message, not returned here. Model preserved
/// (`functor_lang::rebind_value`); a broken push keeps the old program running.
#[wasm_bindgen]
pub fn functor_lang_set_source(source: String, id: Option<f64>) {
    if !functor_lang_is_running() {
        post_reload_result(
            false,
            "game is not running yet (still loading, or the load failed)",
            id,
        );
        return;
    }
    // Last edit wins: a burst of pushes before the next frame coalesces.
    PENDING_RELOAD.with(|p| *p.borrow_mut() = Some((PendingPush::Source(source), id)));
}

/// The multi-file sibling of [`functor_lang_set_source`]: hot-swap the running
/// game from a pushed FILE SET — an array of `{path, source}` objects, the
/// entry first, then siblings (`file = module`). For pushers that own the
/// whole project in memory (the web IDE); a single-buffer editor over served
/// files keeps using `functor_lang_set_source`. Same queue, same result message,
/// same model-preserving semantics.
#[wasm_bindgen]
pub fn functor_lang_set_project(files: JsValue, id: Option<f64>) {
    if !functor_lang_is_running() {
        post_reload_result(
            false,
            "game is not running yet (still loading, or the load failed)",
            id,
        );
        return;
    }
    let Some(parsed) = parse_project_files(&files) else {
        post_reload_result(
            false,
            "malformed project push: expected a non-empty array of {path, source} objects",
            id,
        );
        return;
    };
    PENDING_RELOAD.with(|p| *p.borrow_mut() = Some((PendingPush::Project(parsed), id)));
}

/// Route a socket event to the LIVE producer via the shared `GAME` handle (the
/// Functor Lang page's `FunctorLangEmbeddedGame`) — the WebSocket twin of [`perform_and_push`]. Runs
/// in a socket-event microtask, never mid-frame, so the borrow can't collide
/// with the frame loop.
fn with_live_game(f: impl FnOnce(&mut dyn GameProducer)) {
    // A running netsim suspends the single game, and that has to include its
    // ASYNC arrivals: sockets opened before the sim started stay open, and an
    // inbound message here would fold through `update` and queue effects onto
    // the queues the sim instances share (the sim would then attribute them to
    // instance 0). Drop them — the single game is not simulating, so it has no
    // frame to receive them into.
    if sim::is_running() {
        return;
    }
    let Some(game) = GAME.with(|g| g.borrow().clone()) else {
        return;
    };
    let Ok(mut game) = game.try_borrow_mut() else {
        web_sys::console::error_1(&"[net] socket event arrived mid-frame; dropped".into());
        return;
    };
    f(&mut **game);
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
    let result = perform_fetch(method, &url, &headers, &body, false).await;
    // Route the completion to the LIVE producer via the shared GAME handle —
    // the Functor Lang page's FunctorLangEmbeddedGame, which folds the response through `update`.
    // This runs as a fetch microtask, never mid-frame, so the borrow can't
    // collide with the frame loop (as with `functor_lang_set_source`).
    let Some(game) = GAME.with(|g| g.borrow().clone()) else {
        return;
    };
    let Ok(mut game) = game.try_borrow_mut() else {
        web_sys::console::error_1(&"[net] http completion arrived mid-frame; dropped".into());
        return;
    };
    match result {
        Ok((status, text)) => game.net_push_http_response(token, status, text),
        Err(message) => game.net_push_http_error(token, message),
    }
}

async fn perform_fetch(
    method: HttpMethod,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    no_store: bool,
) -> Result<(i32, String), String> {
    use wasm_bindgen::JsCast;
    use web_sys::{Request, RequestCache, RequestInit, Response};

    let mut opts = RequestInit::new();
    opts.method(http_method_str(method));
    if no_store {
        opts.cache(RequestCache::NoStore);
    }
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
async fn decode_buffer(ctx: &web_sys::AudioContext, sound: &str) -> Option<web_sys::AudioBuffer> {
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

/// Drain the game's queued connection commands and perform them: with browser
/// WebSockets (socket events are pushed back into the game from the handlers),
/// or — under [`NetTransport::Embedder`] — by handing them to the embedding page.
fn dispatch_conn_commands(game: &dyn GameProducer, state: &Rc<RefCell<WsClient>>) {
    let json = game.net_drain_conn_commands();
    if json == "[]" {
        return;
    }
    if net_transport() == NetTransport::Embedder {
        post_conn_commands_to_embedder(&json);
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

/// Embedder egress: hand the drained commands (verbatim, as the JSON array the
/// producer serialized) to the hosting page as
/// `{ type: "functor-net-commands", commands: [...] }`. Unlike the WebSocket
/// path, `Listen` is NOT refused here — a browser can't bind a socket, but the
/// embedder can honour a listen however it likes, so every command goes out.
///
/// A top-level page has no embedder to route to; warn once and drop, the way
/// the WebSocket path warns about `Sub.listen`.
///
/// The commands go out with our OWN origin as the target, so egress and ingress
/// (which only accepts same-origin deliveries) enforce the same boundary — the
/// player is deliberately embeddable, and every `Send` payload and `Connect`
/// URL is in here. [xreview]
///
/// Idempotency is the embedder's to keep: `Connect`/`Listen` are idempotent by
/// key (see `net::connection`), and unlike `ws_connect` this path holds no
/// connection table to dedupe against — a re-declare (every hot reload emits
/// one) is forwarded verbatim.
fn post_conn_commands_to_embedder(json: &str) {
    let parent = window().parent().ok().flatten();
    let Some(parent) = parent.filter(|p| p != &window()) else {
        thread_local! {
            static WARNED: Cell<bool> = const { Cell::new(false) };
        }
        if !WARNED.replace(true) {
            web_sys::console::warn_1(
                &"[net] embedder transport with no embedding page; net commands are dropped".into(),
            );
        }
        return;
    };
    let commands = match js_sys::JSON::parse(json) {
        Ok(value) => value,
        Err(_) => {
            web_sys::console::error_1(&"[net] bad conn commands json".into());
            return;
        }
    };
    let message = Object::new();
    let _ = js_sys::Reflect::set(
        &message,
        &JsValue::from_str("type"),
        &JsValue::from_str("functor-net-commands"),
    );
    let _ = js_sys::Reflect::set(&message, &JsValue::from_str("commands"), &commands);
    let origin = window().location().origin().unwrap_or_default();
    let _ = parent.post_message(&message, &origin);
}

/// Embedder ingress: the counterpart of [`post_conn_commands_to_embedder`] —
/// the embedding page delivers inbound network events as a JSON array (see
/// [`DeliveredEvent`]), and each one lands on the live producer exactly as a
/// socket handler's push would. Malformed JSON logs once and is dropped.
///
/// Only under [`NetTransport::Embedder`]: the transport gate belongs at the
/// seam, not in one of its callers. Otherwise a page running real WebSockets
/// could have events injected against a live socket's connection id (the id
/// spaces are the same). [xreview]
#[wasm_bindgen]
pub fn functor_lang_net_deliver(events_json: &str) {
    if net_transport() != NetTransport::Embedder {
        web_sys::console::error_1(
            &"[net] functor_lang_net_deliver requires window.__functorNetTransport = \"embedder\""
                .into(),
        );
        return;
    }
    let events = match parse_delivered_events(events_json) {
        Ok(events) => events,
        Err(e) => {
            web_sys::console::error_1(&format!("[net] bad net delivery: {e}").into());
            return;
        }
    };
    // One borrow for the whole batch: a mid-frame collision drops the delivery
    // with a single log rather than one per event.
    with_live_game(|g| {
        for event in events {
            match event {
                DeliveredEvent::Connected { key, conn } => g.net_push_connected(key, conn),
                DeliveredEvent::Message { key, conn, text } => {
                    g.net_push_conn_message(key, conn, text)
                }
                DeliveredEvent::Disconnected { key, conn } => g.net_push_disconnected(key, conn),
                DeliveredEvent::Error { key, conn, message } => {
                    g.net_push_conn_error(key, conn, message)
                }
            }
        }
    });
}

fn ws_connect(state: &Rc<RefCell<WsClient>>, key: String, url: String) {
    // Idempotent by key (matches the native host); a re-declared connection
    // reattaches rather than opening a second socket. Event callbacks push into
    // the live producer (the Functor Lang page's FunctorLangEmbeddedGame) via `with_live_game`.
    if state.borrow().by_key.contains_key(&key) {
        return;
    }
    let ws = match web_sys::WebSocket::new(&url) {
        Ok(ws) => ws,
        Err(_) => {
            with_live_game(|g| {
                g.net_push_conn_error(key, 0, "failed to open WebSocket".to_string())
            });
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
        Closure::<dyn FnMut()>::new(move || {
            with_live_game(|g| g.net_push_connected(key.clone(), iid))
        })
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_message = {
        let key = key.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Some(text) = e.data().as_string() {
                with_live_game(|g| g.net_push_conn_message(key.clone(), iid, text));
            }
        })
    };
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_close = {
        let key = key.clone();
        let state = state.clone();
        Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_e: web_sys::CloseEvent| {
            with_live_game(|g| g.net_push_disconnected(key.clone(), iid));
            // Drop our handle so the key is free to be opened again.
            //
            // NB it will not be, today: `reconcile_connections` only emits a
            // `Connect` for a key ABSENT from the producer's `live_conn_keys`,
            // and that set is re-seeded from the declared subs every frame — so
            // a still-declared `Sub.connect` never re-fires and a dropped
            // connection stays down. (This comment used to claim the reconnect
            // happened.) Reconnect/backoff is its own change: the shell would
            // have to retract the key from the producer's live set here.
            // [xreview]
            let mut s = state.borrow_mut();
            s.conns.remove(&id);
            s.by_key.remove(&key);
        })
    };
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    let on_error = {
        let key = key.clone();
        // A WebSocket's `error` event is a PLAIN `Event`, not an `ErrorEvent` —
        // it carries no `message`. Typing it as `ErrorEvent` and calling
        // `.message()` made the generated glue read `length` off `undefined`,
        // throwing a JS TypeError and then TRAPPING the wasm module: a game
        // whose server simply wasn't up took the whole runtime down with it.
        // The browser withholds the reason for a socket failure by design, so
        // there is nothing to recover — and the endpoint is already the `key`
        // this error is reported against. [xreview]
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            with_live_game(|g| {
                g.net_push_conn_error(key.clone(), iid, "connection failed".to_string())
            });
        })
    };
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();
}

#[wasm_bindgen(start)]
pub fn main() {
    // Report panics to the console with their message and location. Without a
    // hook a wasm panic reaches the page as a bare `RuntimeError: unreachable`
    // — no message, no file, no line — which is how the refused-socket trap in
    // `ws_connect` stayed unexplained. (The job `console_error_panic_hook`
    // does; two lines here, so we don't take the dependency.)
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("[functor-lang] panic: {info}").into());
    }));
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
    // The page's Functor Lang entry (docs/functor-lang.md Track C5) runs through the in-runtime
    // interpreter — the sole producer since the F#/wasm-bindgen bridge was
    // removed in E3. Async pushes (fetch results, WebSocket events) reach it
    // through the shared `GAME` handle (`perform_and_push` / `with_live_game`).
    let Some(path) = functor_lang_game_path() else {
        let rendered =
            "[functor-lang] error: no game entry — window.__functorLangGamePath is not set"
                .to_string();
        web_sys::console::error_1(&rendered.as_str().into());
        show_error_overlay(&rendered);
        return Err(JsValue::from_str(&rendered));
    };
    let game: Box<dyn GameProducer> = match create_functor_lang_game(&path).await {
        Ok(game) => Box::new(game),
        Err(message) => {
            let rendered = format!("[functor-lang] error: {message}");
            web_sys::console::error_1(&rendered.as_str().into());
            show_error_overlay(&rendered);
            return Err(JsValue::from_str(&rendered));
        }
    };
    // Share the producer with the `functor_lang_set_source` export (docs/functor-lang.md D4):
    // the frame loop below and the editor push path borrow the same instance.
    let game = Rc::new(RefCell::new(game));
    GAME.with(|g| *g.borrow_mut() = Some(game.clone()));

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
            // Ask for a lean context: no MSAA (the canvas is sized to CSS px x
            // devicePixelRatio, so on retina 4x multisampling a ~2880x1800
            // backbuffer every frame is a GPU burner), an opaque backbuffer (no
            // alpha compositing with the page), and the discrete GPU.
            let attrs = web_sys::WebGlContextAttributes::new();
            attrs.set_antialias(false);
            attrs.set_alpha(false);
            attrs.set_power_preference(web_sys::WebGlPowerPreference::HighPerformance);
            let webgl2_context = canvas
                .get_context_with_context_options("webgl2", &attrs)
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
        let mut input_snapshot = InputSnapshot::default();
        // rAF can produce zero or several fixed steps. Keep transitions until
        // the first step, then clear them before any catch-up step.
        let mut input_edges = InputEdges::default();
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
        let launch_debug_render_mode = debug_render_mode_from_url();
        let mut debug_presentation = DebugPresentation::from_render_mode(launch_debug_render_mode);
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

        // Time-travel clock control (docs/time-travel.md T3). The scrubber UI is
        // NATIVE DOM on web (index-functor-lang.html) — outside the canvas, so no
        // pointer-lock clash — driving the runtime through the `functor_lang_scrub_*`
        // exports (in functor_lang_game.rs). This loop OWNS the shared game clock: `tts`
        // accumulates the real frame delta while live, freezes on pause (so a
        // scrubbed frame stays put and resume doesn't jump), and rebases on a
        // branch. `?fixed-time` seeds an unconditional pin for deterministic
        // golden captures.
        let mut clock = GameClock::new(fixed_time);
        let mut detached_camera = DebugCamera::default();
        let mut pending_detach = false;
        let mut pending_debug_camera_reset = false;

        // Future preview (docs/time-travel.md T6/T6d): trail dots, scene-space
        // strobe copies, or the screen-space ghost compositor — one mode, driven
        // by the DOM preview <select>, with the shared forward window/samples
        // from the ⚙ popover. Same anchor cache as the desktop shell: while
        // paused the anchor (scene frame + tts) is frozen, so reuse the
        // computed preview; the program revision invalidates immediately on a
        // pushed source edit. Live projections remain painted every frame but
        // refresh on a wall-clock cadence that bounds repeated dry runs.
        let mut preview_mode = functor_runtime_common::PreviewMode::Off;
        let mut preview_window: f32 = 2.0;
        let mut preview_rate: usize = 5;
        const PAUSED_PREVIEW_REUSE_FRAMES: u32 = 30;
        const LIVE_PREVIEW_INTERVAL_MS: f32 = 100.0;
        let mut preview_cache: Option<(
            (Option<u64>, u32, bool, u64, bool, bool, usize, u32),
            functor_runtime_common::FramePreview,
        )> = None;
        let mut preview_refresh: u32 = 0;
        let mut next_live_preview_refresh: f32 = 0.0;
        let mut ghost_cache: Option<(
            (Option<u64>, u32, bool, u64, usize, u32),
            Vec<(Frame, FrameTime)>,
        )> = None;
        let mut ghost_refresh: u32 = 0;
        let mut next_live_ghost_refresh: f32 = 0.0;

        *g.borrow_mut() = Some(Closure::new(move || {
            // The frame's exclusive borrow of the shared producer.
            let mut game = game.borrow_mut();
            let now = performance.now() as f32;

            // Apply a pushed source (`functor_lang_set_source`) here, at a safe point that
            // already holds the borrow — so a push never collides with a frame.
            apply_pending_reload(&mut **game);

            // Apply scrubber controls from the DOM (pause / step / seek), which
            // drive the shared game clock BEFORE this frame's time is computed.
            for control in functor_lang_game::take_scrub_controls() {
                match control {
                    functor_lang_game::ScrubControl::TogglePause => {
                        // Resuming: rebase to the scene's current time so play
                        // continues from there, not wall-clock. When scrubbed this
                        // is the scrubbed frame's recorded `tts`; on a plain
                        // pause/resume it's the newest recorded frame's `tts`,
                        // which already equals the frozen `game_time` (a no-op).
                        if clock.is_paused() {
                            if let Some(tts) = game.current_scene_tts() {
                                clock.rebase(tts as f32);
                            }
                        }
                        clock.toggle_pause();
                    }
                    functor_lang_game::ScrubControl::ToggleDetachedCamera => {
                        if detached_camera.is_detached() {
                            detached_camera.reattach();
                            functor_lang_game::acknowledge_detached_camera(false);
                        } else {
                            // Apply after this frame renders so a seek queued
                            // in the same batch snapshots the selected frame,
                            // never the previously displayed camera.
                            pending_detach = true;
                        }
                    }
                    functor_lang_game::ScrubControl::SetDebugCameraMode(mode) => {
                        if let Some(mode) = DebugCameraMode::from_index(mode) {
                            detached_camera.set_mode(mode);
                        }
                    }
                    functor_lang_game::ScrubControl::SetDebugCameraFov(fov) => {
                        detached_camera.set_fov_degrees(fov);
                    }
                    functor_lang_game::ScrubControl::SetDebugMaterial(material) => {
                        if let Some(material) = DebugMaterialMode::from_index(material) {
                            debug_presentation.material = material;
                        }
                    }
                    functor_lang_game::ScrubControl::SetDebugPhysics(enabled) => {
                        debug_presentation.physics = enabled;
                    }
                    functor_lang_game::ScrubControl::SetAuthoredCameraFrustum(enabled) => {
                        debug_presentation.authored_camera_frustum = enabled;
                    }
                    functor_lang_game::ScrubControl::SetGameUiVisible(visible) => {
                        debug_presentation.show_game_ui = visible;
                    }
                    functor_lang_game::ScrubControl::ResetDebugCamera => {
                        pending_debug_camera_reset = true;
                    }
                    functor_lang_game::ScrubControl::MoveDetachedCamera {
                        forward,
                        right,
                        vertical,
                        elapsed_seconds,
                    } => {
                        detached_camera.move_local(
                            forward,
                            right,
                            vertical,
                            elapsed_seconds,
                        );
                    }
                    functor_lang_game::ScrubControl::Step => clock.step(1.0 / 60.0),
                    functor_lang_game::ScrubControl::SetPreview(mode) => {
                        preview_mode = functor_runtime_common::PreviewMode::from_index(mode);
                    }
                    functor_lang_game::ScrubControl::SetPreviewConfig { window, rate } => {
                        preview_window = window.clamp(0.5, 5.0);
                        preview_rate = rate.clamp(1, 30);
                    }
                    functor_lang_game::ScrubControl::SeekTo {
                        frame: f,
                        request_id,
                    } => {
                        let newest = game.scene_frame_range().map(|(_, h)| h);
                        match newest {
                            Some(h) if f > h => {
                                // Dragged INTO the rail's cyan future segment:
                                // pass through the recorded end, then step the
                                // game forward input-free (the clock animates
                                // the catch-up — mirrors the desktop shell).
                                let _ = game.seek_scene_to(h);
                                if let Some(tts) = game.current_scene_tts() {
                                    clock.rebase(tts as f32);
                                }
                                clock.step_frames((f - h) as u32);
                            }
                            _ => {
                                let _ = game.seek_scene_to(f);
                                // Park on the scrubbed frame and keep the clock
                                // aligned to its time, so resuming continues
                                // from there.
                                if let Some(tts) = game.current_scene_tts() {
                                    clock.rebase(tts as f32);
                                }
                                clock.pause();
                            }
                        }
                        functor_lang_game::publish_scrub_seek_result(
                            request_id,
                            game.current_scene_frame(),
                        );
                    }
                }
            }
            let camera_motion = functor_lang_game::take_debug_camera_motion();
            detached_camera.look(camera_motion.look_dx, camera_motion.look_dy);
            detached_camera.zoom(camera_motion.zoom_steps);

            // Fixed-timestep model loop (docs/time-travel.md), mirroring the
            // desktop shells: advance `tick` in whole 1/60 steps decoupled from
            // the render (rAF) rate, so the sim is deterministic and a recorded
            // frame is exactly one forward-step fine step (the ghost replay's
            // assumption). `?fixed-time` yields one {dts:0} sub-frame (golden
            // capture unchanged); a queued step yields one; paused yields none.
            // `frame_time` is the RENDER frame time — the settled `tts` the frame
            // is drawn / soundscaped / scrub-published at (its `dts` is unused).
            let sub_frames = clock.fixed_frames((now - last_time) / 1000.0);
            last_time = now;
            let frame_time = FrameTime {
                dts: 0.0,
                tts: clock.current_tts(),
            };

            // Deliver page input queued since the last frame (the Functor Lang path's
            // `functor_lang_*` exports), once per rendered frame before this frame's steps.
            // While PINNED (paused or ?fixed-time — the desktop `ignore_user_input`
            // rule), drain-and-discard: no input may reach the model on a pinned
            // frame (a paused frame's input would diverge the replay log; a
            // fixed-time frame must stay deterministic for captures), and
            // draining stops the queue bursting on resume.
            // A running netsim suspends the single game the same way a pinned
            // clock does. Delivering input here would fold it through the
            // single game's `update`, whose effects land on the queues the sim
            // instances share, and the sim's next step would attribute that
            // stray `Effect.send` to instance 0. New presses are discarded,
            // while recovery-only releases clear physical levels so a blur
            // during the sim cannot leave the resumed game stuck.
            let sim_running = sim::is_running();
            let suspended = clock.is_pinned() || sim_running;
            // Browser mouse events use CSS pixels. Sample the canvas's logical
            // CSS extent beside them BEFORE sampledInput, independently of the
            // Retina-scaled drawable buffer updated later in this frame.
            input_snapshot.mouse.surface_width = canvas.client_width().max(0) as u32;
            input_snapshot.mouse.surface_height = canvas.client_height().max(0) as u32;
            functor_lang_game::drain_input(
                &mut **game,
                &mut input_snapshot,
                &mut input_edges,
                !suspended,
                recover_suppressed_releases(&clock, sim_running),
            );

            // Webview interactions drain HERE, before render replaces the
            // handler table — the queued slots were clicked against the DOM
            // the LAST render published, so they must resolve against that
            // render's table, not this frame's. Pinned frames drain-and-drop
            // like all input. [xreview]
            for event in functor_lang_game::take_webview_events() {
                if !suspended {
                    game.webview_event(event);
                }
            }

            // While a netsim owns the page (`sim.rs`), the single game is
            // SUSPENDED: it neither ticks nor dispatches. Producers share this
            // thread's command queues (net/conn/audio/preload), and each drain
            // empties the queue for everyone — so a live single game would steal
            // the sim instances' commands and dispatch them to REAL sockets,
            // while the sim would swallow the single game's.
            //
            // Its MODEL is frozen (no tick, no input, no delivered socket
            // events — see `with_live_game`); it does keep rendering that frozen
            // model, so a `tts`-driven animation in `draw` still moves. Freezing
            // the clock too belongs with the multi-pane view, where the sim owns
            // the viewport and the single game stops rendering entirely.
            if !sim::is_running() {
                // The loading snapshot for `Sub.assets`: pushed every frame, the
                // producer only acts when it changed since the game last saw it.
                // Inside the gate — it reaches `update`, so it queues effects.
                game.push_asset_progress(asset_cache.progress());

                // Effect.preload (B.5): warm the cache with this frame's queued
                // preloads and drive in-flight ones to settlement. Unlike audio
                // finishes (undetectable on Web Audio today), preload settlement
                // comes from the driver's own polling — preloadThen works on wasm.
                let preload_commands =
                    serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default();
                for token in scene_context.drive_preloads(&asset_cache, preload_commands) {
                    game.preload_push_settled(token);
                }

                for sub in &sub_frames {
                    if game.samples_input() {
                        input_edges.apply_to(&mut input_snapshot);
                        game.sampled_input(&input_snapshot);
                    }
                    input_edges.clear();
                    input_snapshot.clear_edges();
                    game.tick(sub.clone());
                }

                // Perform any networking commands this frame's tick queued; results
                // are pushed back into the inbox asynchronously and decoded by a later
                // tick (see dispatch_net_commands).
                dispatch_net_commands(&**game);
                // Play any one-shot sounds this frame's tick queued (fetch + decode
                // the first time, then from the cache).
                dispatch_audio_commands(&**game);
                dispatch_conn_commands(&**game, &ws_state);
            }

            let mut frame: Frame = game.render(frame_time.clone());
            if pending_detach
                && clock.pending_frames() == 0
                && clock.pending_steps() == 0
            {
                pending_detach = false;
                let active = detached_camera.detach(&frame);
                functor_lang_game::acknowledge_detached_camera(active);
            }
            if pending_debug_camera_reset {
                pending_debug_camera_reset = false;
                if detached_camera.is_detached() {
                    let mode = detached_camera.mode();
                    if detached_camera.detach(&frame) {
                        if let Some(mode) = mode {
                            detached_camera.set_mode(mode);
                        }
                    }
                }
            }
            if detached_camera.is_detached() && !detached_camera.is_compatible(&frame) {
                detached_camera.reattach();
                functor_lang_game::acknowledge_detached_camera(false);
            }
            let view_camera = detached_camera.camera(&frame.camera).clone();

            // Soundscape: aim the listener from this frame's camera, then
            // reconcile the desired looping voices against the live ones.
            update_soundscape(&**game, &view_camera);

            // Scene-diff preview (docs/time-travel.md T6): the DOM preview
            // <select>'s trail/strobe overlays, from ONE shared forward-sim —
            // `frame_preview`, the same step the desktop shell runs. While live,
            // its anchor follows the newest frame; pausing freezes that anchor
            // instead of enabling the preview. Script inputs are `None`: web has
            // no --input-script.
            // While a drag-into-the-future catch-up is draining, skip preview
            // and ghost recomputes (the anchor moves every frame — a full
            // forward-sim per frame would throttle the catch-up to a crawl);
            // they snap back in on arrival.
            let catching_up = clock.pending_frames() > 0;
            let selected =
                functor_runtime_common::interactive_preview(preview_mode, true, catching_up);
            let trail_wanted = selected.trail;
            // The selector is single-valued, so a strobe mode and the ghost
            // compositor can never be on together (the double-ghost hazard the
            // desktop flag path still guards against).
            let strobe_wanted = selected.strobe;
            let preview = if trail_wanted || strobe_wanted {
                let key = (
                    game.current_scene_frame(),
                    frame_time.tts.to_bits(),
                    clock.is_paused(),
                    game.scene_program_revision(),
                    trail_wanted,
                    strobe_wanted,
                    preview_rate,
                    preview_window.to_bits(),
                );
                let cache_hit = preview_cache.as_ref().is_some_and(|(k, _)| {
                    if clock.is_paused() {
                        preview_refresh > 0 && *k == key
                    } else {
                        now < next_live_preview_refresh
                            && preview_refresh == 0
                            && !k.2
                            && k.3 == key.3
                            && k.4 == key.4
                            && k.5 == key.5
                            && k.6 == key.6
                            && k.7 == key.7
                    }
                });
                if cache_hit {
                    if clock.is_paused() {
                        preview_refresh -= 1;
                    }
                    preview_cache.as_ref().map(|(_, p)| p.clone())
                } else {
                    // The SIM samples fine (~20/s — the trail's smooth-arc
                    // rate) while the ⚙ rate governs STROBE COPIES per second,
                    // so dots stay visible between copies and both hold their
                    // density as the window resizes.
                    const TRAIL_RATE: f32 = 20.0;
                    let divisions = ((TRAIL_RATE * preview_window).round() as usize).clamp(1, 64);
                    let copies = ((preview_rate as f32 * preview_window).round() as usize)
                        .clamp(1, divisions);
                    let p = functor_runtime_common::frame_preview(
                        &**game,
                        &frame,
                        frame_time.tts as f64,
                        None,
                        &functor_runtime_common::PreviewOptions {
                            divisions,
                            window: preview_window,
                            // eps 0.04: ignore sub-mm jitter. max_step 3.0: cut
                            // respawn teleports.
                            eps: 0.04,
                            max_step: 3.0,
                            trail: trail_wanted,
                            strobe: strobe_wanted.then(|| functor_runtime_common::StrobeOptions {
                                copies,
                                ..Default::default()
                            }),
                        },
                    );
                    if clock.is_paused() {
                        preview_refresh = PAUSED_PREVIEW_REUSE_FRAMES;
                    } else {
                        preview_refresh = 0;
                        next_live_preview_refresh =
                            performance.now() as f32 + LIVE_PREVIEW_INTERVAL_MS;
                    }
                    preview_cache = Some((key, p.clone()));
                    Some(p)
                }
            } else {
                preview_cache = None;
                next_live_preview_refresh = 0.0;
                None
            };

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
            let diagnostics = debug_presentation.diagnostics(
                detached_camera.is_detached(),
                frame.is_pure_2d(),
                launch_debug_render_mode,
            );
            let mut debug_lines = diagnostics
                .physics
                .then(|| {
                    functor_runtime_common::physics::with_world(
                        functor_runtime_common::physics::DEFAULT_WORLD,
                        |world| world.debug_lines(),
                    )
                })
                .flatten()
                .unwrap_or_default();
            if diagnostics.authored_camera_frustum {
                debug_lines.extend(camera_frustum_lines(&frame.camera, viewport.aspect()));
            }

            // Forward-ghosting (docs/time-travel.md T6d): when the preview
            // selector is on `ghost`, forward-step the scene over the ⚙
            // popover's window in up to 8 slices and composite them at equal
            // weight, so moving elements strobe across their future and static
            // geometry stays solid. While live the anchor advances each frame;
            // pausing freezes it. `None` = the recorded-log/coast path (web has
            // no --input-script). Empty (→ this arm skipped) leaves live
            // rendering unchanged.
            let mut ghosts = if selected.ghost {
                // The ⚙ popover's rate × window, clamped to the compositor's
                // 8-target cap.
                let divisions =
                    ((preview_rate as f32 * preview_window).round() as usize).clamp(1, 8);
                let dt = preview_window / divisions as f32;
                let key = (
                    game.current_scene_frame(),
                    frame_time.tts.to_bits(),
                    clock.is_paused(),
                    game.scene_program_revision(),
                    divisions,
                    preview_window.to_bits(),
                );
                let cache_hit = ghost_cache.as_ref().is_some_and(|(k, _)| {
                    if clock.is_paused() {
                        ghost_refresh > 0 && *k == key
                    } else {
                        now < next_live_ghost_refresh
                            && ghost_refresh == 0
                            && !k.2
                            && k.3 == key.3
                            && k.4 == key.4
                            && k.5 == key.5
                    }
                });
                if cache_hit {
                    if clock.is_paused() {
                        ghost_refresh -= 1;
                    }
                    ghost_cache
                        .as_ref()
                        .map(|(_, frames)| frames.clone())
                        .unwrap_or_default()
                } else {
                    let frames = game.ghost_frames(divisions, dt, frame_time.tts as f64, None);
                    if clock.is_paused() {
                        ghost_refresh = PAUSED_PREVIEW_REUSE_FRAMES;
                    } else {
                        ghost_refresh = 0;
                        next_live_ghost_refresh =
                            performance.now() as f32 + LIVE_PREVIEW_INTERVAL_MS;
                    }
                    ghost_cache = Some((key, frames.clone()));
                    frames
                }
            } else {
                ghost_cache = None;
                next_live_ghost_refresh = 0.0;
                Vec::new()
            };
            let compatible_ghosts = DebugCamera::compatible_prefix_len(
                &frame,
                ghosts.iter().map(|(candidate, _)| candidate),
            );
            ghosts.truncate(compatible_ghosts);

            // Preview overlays go on the live frame. (The single-valued mode
            // selector means the scene-diff preview and the ghost compositor
            // are never on together here — unlike the desktop flag path, where
            // --ghost --trajectory composes the trail into the ghost frames.)
            if let Some(p) = &preview {
                p.apply_all(&mut frame);
            }
            // Shadow + forward passes, shared with the desktop runtime. Each
            // ghost frame renders at ITS OWN division-boundary time, so
            // render-time animation (the skinned pose) advances through the
            // strobe instead of freezing at the paused pose.
            if !ghosts.is_empty() {
                functor_runtime_common::render_composited_frames_with_view(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &ghosts,
                    &vec![1.0f32; ghosts.len()],
                    Some(&view_camera),
                    detached_camera.sprite_cameras(),
                    viewport,
                    diagnostics.render_mode,
                );
            } else {
                functor_runtime_common::render_frame_with_view(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &frame,
                    &view_camera,
                    detached_camera.sprite_cameras(),
                    frame_time.clone(),
                    viewport,
                    diagnostics.render_mode,
                );
            }
            if !debug_lines.is_empty() {
                functor_runtime_common::render_debug_lines(
                    &gl,
                    shader_version,
                    &view_camera,
                    viewport,
                    &debug_lines,
                );
            }

            // 2D UI overlay: the game's declarative `ui model` View, lowered to a
            // text overlay on top of the frame (HiDPI-aware via the device
            // ratio). The page's unlocked-pointer canvas listeners feed the
            // pointer (CSS px, scaled to framebuffer px here); widget
            // interactions come back slot-stamped and fold through the game's
            // `update` — except while paused, matching `drain_input`'s gate
            // (no input may reach the model on a paused frame).
            let show_game_ui = !detached_camera.is_detached() || debug_presentation.show_game_ui;
            let view: functor_runtime_common::ui::View = if show_game_ui {
                game.ui()
            } else {
                functor_runtime_common::ui::View::Empty
            };
            let dpr = web_sys::window().unwrap().device_pixel_ratio() as f32;
            let dpr = dpr.max(1.0);
            // While the clock is pinned, events would be dropped anyway (the
            // window-input rule) — hide the pointer from egui entirely so a
            // paused interaction can't visually engage widgets or fight the
            // slider reconciliation (the desktop rule), and discard queued
            // focused-field keys the same way. [xreview]
            let mut ui_pointer = functor_lang_game::ui_pointer_state(dpr);
            if clock.is_pinned() {
                ui_pointer.pos = None;
            }
            let ui_keys = functor_lang_game::drain_ui_keys(!clock.is_pinned());
            let ui_out = text_overlay.draw_view(
                canvas.width(),
                canvas.height(),
                dpr,
                ui_pointer,
                &ui_keys,
                &view,
            );
            functor_lang_game::set_ui_wants_keyboard(ui_out.wants_keyboard);
            functor_lang_game::set_ui_wants_pointer(ui_out.wants_pointer);
            // `suspended`, not just pinned: a UI event reaches `update` too, so a
            // running sim must swallow it like every other input path.
            if !suspended {
                for event in ui_out.events {
                    game.ui_event(event);
                }
            }

            // The HTML/CSS webview overlay: publish the serialized tree for
            // the page's overlay (a REAL DOM node above the canvas — the
            // browser is the renderer here; blitz is the native analogue).
            // Interactions drained pre-tick above. TODO(webview): cache the
            // serialized string in the producer instead of clone+reserialize
            // per frame (perf follow-up).
            functor_lang_game::publish_webview_html(
                show_game_ui
                    .then(|| game.webview().map(|node| node.to_html()))
                    .flatten(),
            );

            // Publish the scrubber state for the DOM slider to poll (the UI
            // itself is native HTML in index-functor-lang.html, outside the canvas).
            functor_lang_game::publish_timeline_inputs(&**game);
            functor_lang_game::publish_scrub_view(
                game.current_scene_frame(),
                game.scene_frame_range(),
                clock.is_paused(),
                game.scene_timeline_generation(),
            );
            functor_lang_game::publish_detached_camera(detached_camera.is_detached());
            functor_lang_game::publish_debug_camera_view(
                detached_camera.mode(),
                debug_presentation,
                detached_camera.fov_degrees(),
                detached_camera.zoom_2d(),
            );

            // Publish the paused-inspector trace for the page's poll loop
            // (visual-debugger PR2b). Cheap: while playing this is the byte-stable
            // stub; while paused the producer serves its cached doc (rebuilt only
            // on a pause / paused-frame change). The page relays a CHANGE to the
            // VS Code live-preview as a `functor-inspector-trace` postMessage.
            functor_lang_game::publish_inspector_trace(game.inspector_trace(clock.is_paused()));

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
