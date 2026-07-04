#![cfg_attr(feature = "strict", deny(warnings))]

use std::env;
use std::sync::Arc;
use std::time::Instant;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::{Frame, FrameTime, SceneContext};
use std::collections::BTreeSet;

use functor_runtime_common::Key as InputKey;
use glfw::{Action, Key};
use glow::*;
use hot_reload_game::HotReloadGame;
use static_game::StaticGame;

use crate::game::Game;
// Shared with the Quest runtime shell: the MLE producer and the HTTP debug
// server now live in functor_runtime_common (native targets only).
use functor_runtime_common::{debug_server, mle_game};

const SCR_WIDTH: u32 = 800;
const SCR_HEIGHT: u32 = 600;

mod audio;
mod game;
mod hot_reload_game;
mod net_dispatch;
mod replay_game;
mod static_game;
mod ws_host;
mod xreal;

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

/// Map a key name (case-insensitive: "w", "Up", "space", …) to the canonical
/// engine key, for the debug server's POST /input. Letters rely on the
/// contiguous A..Z discriminants.
fn key_from_str(name: &str) -> Option<InputKey> {
    let name = name.to_ascii_lowercase();
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return InputKey::from_i32((c - b'a') as i32 + InputKey::A as i32);
        }
    }
    match name.as_str() {
        "up" => Some(InputKey::Up),
        "down" => Some(InputKey::Down),
        "left" => Some(InputKey::Left),
        "right" => Some(InputKey::Right),
        "space" => Some(InputKey::Space),
        "enter" => Some(InputKey::Enter),
        "escape" => Some(InputKey::Escape),
        _ => None,
    }
}

use clap::Parser;

/// CLI spelling of `functor_runtime_common::DebugRenderMode`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default)]
enum DebugRenderArg {
    #[default]
    Default,
    /// Visualize world-space surface normals as RGB.
    Normals,
    /// Visualize world-space surface tangents as RGB.
    Tangents,
}

impl From<DebugRenderArg> for functor_runtime_common::DebugRenderMode {
    fn from(arg: DebugRenderArg) -> Self {
        match arg {
            DebugRenderArg::Default => functor_runtime_common::DebugRenderMode::Default,
            DebugRenderArg::Normals => functor_runtime_common::DebugRenderMode::Normals,
            DebugRenderArg::Tangents => functor_runtime_common::DebugRenderMode::Tangents,
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    game_path: String,

    #[arg(long)]
    hot: bool,

    /// Treat --game-path as an `.mle` source file and run it through the MLE
    /// interpreter with the Functor prelude instead of loading a game dylib
    /// (docs/mle.md Track C2). Prints per-frame eval cost every 300 frames.
    #[arg(long)]
    mle: bool,

    /// Treat --game-path as a frame-recording JSON (a single serialized `Frame`
    /// or a JSON array of them — the exact format `GET /scene` emits) and replay
    /// it instead of loading a game dylib. A proof producer for the
    /// producer-agnostic seam (docs/mle.md Track A3). Each producer mode
    /// reinterprets --game-path, so combining them is an error, not a silent
    /// precedence pick.
    #[arg(long, conflicts_with_all = ["mle", "hot"])]
    replay: bool,

    /// Run without a GL window: drive the game loop + debug server headlessly
    /// (no GLFW/OpenGL). `/state`, `/scene`, `/input`, `/time` work; `/capture`
    /// is unavailable (no rendering), and audio isn't played (so `Audio.playThen`
    /// completions aren't delivered). Incompatible with `--capture-frame`. For
    /// CI / scripted / LLM-driven runs.
    #[arg(long)]
    headless: bool,

    /// Create the GL window hidden: it is never shown, never takes focus, and
    /// never captures the cursor, so a run doesn't steal input from the user.
    /// A hidden window keeps a valid GL context and framebuffer, so rendering,
    /// --capture-frame, and the debug server's /capture all work unchanged.
    /// Implied by --capture-frame. (With --headless there is no window at all.)
    #[arg(long, conflicts_with = "headless")]
    hidden: bool,

    /// Write a PNG of the rendered frame to this path, then exit. The capture
    /// happens on the first frame after --capture-time seconds of wall-clock
    /// time, so assets have a chance to load. Implies --hidden.
    #[arg(long)]
    capture_frame: Option<String>,

    /// Wall-clock seconds to run before --capture-frame takes the shot.
    #[arg(long, default_value_t = 2.0)]
    capture_time: f32,

    /// Pin the game's frame time (seconds) to a constant so the rendered pose
    /// is deterministic — for reproducible captures / golden images.
    #[arg(long)]
    fixed_time: Option<f32>,

    /// Start an HTTP control server on <--debug-bind>:<PORT> exposing
    /// POST /capture (image/png of the next frame), GET /state (runtime JSON),
    /// and POST /reload-source (network hot-reload for MLE games).
    /// Omit to disable the server entirely.
    #[arg(long)]
    debug_port: Option<u16>,

    /// Interface the debug server binds to. The default keeps it local;
    /// 0.0.0.0 exposes it to the LAN for remote develop (a dev machine
    /// pushing source to this runner). No auth — bind wide only on networks
    /// where arbitrary game-code pushes are acceptable.
    #[arg(long, default_value = "127.0.0.1")]
    debug_bind: String,

    /// Override shading with a diagnostic view across the whole frame (e.g.
    /// `normals` to visualize surface normals as color). Primitives only for
    /// now; glTF models are unaffected until normal import lands.
    #[arg(long, value_enum, default_value_t = DebugRenderArg::Default)]
    debug_render: DebugRenderArg,

    /// Render side-by-side stereo: the frame's camera is split into left/right
    /// eye cameras (`--stereo-ipd` apart, parallel gaze) rendered into the
    /// left|right halves of the window — the "full SBS" layout 3D displays
    /// accept (e.g. Xreal glasses in 3D mode, which treat a 3840x1080 signal
    /// as two 1920x1080 eyes). The UI overlay is drawn once across the whole
    /// window, so it won't fuse in 3D — fine for dev, not for shipping.
    #[arg(long)]
    stereo_sbs: bool,

    /// Eye separation for --stereo-sbs, in world units. The default assumes
    /// meter-scale worlds (human IPD ≈ 64mm); raise it for larger-unit worlds
    /// to deepen the 3D effect.
    #[arg(long, default_value_t = 0.064, value_parser = parse_stereo_ipd)]
    stereo_ipd: f32,

    /// Drive the view with Xreal One head tracking (3DoF): reads the glasses'
    /// IMU over TCP (they expose a USB network interface — no drivers needed),
    /// and rotates the game's camera by your head orientation. Calibrates
    /// against gyro bias for the first ~0.5s (keep the glasses still); F1
    /// recenters. Disable the glasses' own stabilizer/anchor modes (on-glasses
    /// OSD), or the two trackers fight. Combine with --stereo-sbs for 3D.
    #[arg(long)]
    xreal_tracking: bool,

    /// Address (IP:port) of the glasses' IMU stream, for --xreal-tracking.
    /// Parsed as a socket address at startup so a typo is a clean CLI error,
    /// not a dead background thread.
    #[arg(long, default_value = xreal::DEFAULT_ADDR)]
    xreal_addr: std::net::SocketAddr,
}

/// `--stereo-ipd` must be a positive, finite world-unit distance: NaN/inf
/// would poison the eye cameras' view matrices, and a negative value would
/// silently swap the eyes (inverted depth).
fn parse_stereo_ipd(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err("must be a positive, finite distance in world units".into());
    }
    Ok(v)
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
        glow::PixelPackData::Slice(Some(&mut pixels)),
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

// --- Shared per-frame phases, used by both the windowed and headless loops, so
// the two don't drift. Each is GL-free; the windowed loop wraps them with its
// rendering, and supplies a real framebuffer-capture closure to
// `service_debug_request` (headless supplies an "unavailable" one). ---

/// This frame's `FrameTime`, honoring the debug clock controls (`--fixed-time` /
/// POST /time): `pending_step` advances once then holds; otherwise a pinned
/// `held_time` freezes the clock, else it follows wall-clock.
fn next_frame_time(
    held_time: &mut Option<f32>,
    pending_step: &mut Option<f32>,
    last_time: f32,
    elapsed: f32,
) -> FrameTime {
    if let Some(step) = pending_step.take() {
        let new_tts = held_time.unwrap_or(last_time) + step;
        *held_time = Some(new_tts);
        FrameTime {
            dts: step,
            tts: new_tts,
        }
    } else {
        match *held_time {
            Some(tts) => FrameTime { dts: 0.0, tts },
            None => FrameTime {
                dts: elapsed - last_time,
                tts: elapsed,
            },
        }
    }
}

/// Deliver async HTTP + WebSocket results into the game's inbox *before* tick, so
/// this frame's executor drain sees them.
fn deliver_net_ws(
    game: &mut dyn Game,
    net_rx: &std::sync::mpsc::Receiver<net_dispatch::NetResult>,
    ws_rx: &std::sync::mpsc::Receiver<ws_host::HostNetEvent>,
) {
    while let Ok(result) = net_rx.try_recv() {
        match result {
            net_dispatch::NetResult::Response {
                token,
                status,
                body,
            } => game.net_push_http_response(token, status, body),
            net_dispatch::NetResult::Error { token, message } => {
                game.net_push_http_error(token, message)
            }
        }
    }
    while let Ok(event) = ws_rx.try_recv() {
        match event {
            ws_host::HostNetEvent::Connected { key, id } => game.net_push_connected(key, id as i32),
            ws_host::HostNetEvent::Message { key, id, text } => {
                game.net_push_conn_message(key, id as i32, text)
            }
            ws_host::HostNetEvent::Disconnected { key, id } => {
                game.net_push_disconnected(key, id as i32)
            }
            ws_host::HostNetEvent::Error { key, id, message } => {
                game.net_push_conn_error(key, id as i32, message)
            }
        }
    }
}

/// Perform the HTTP + WebSocket commands this frame's tick queued. HTTP requests
/// run on tokio tasks (results return via `net_tx`); WS commands go to the manager.
fn dispatch_net_ws(
    game: &mut dyn Game,
    net_tx: &std::sync::mpsc::Sender<net_dispatch::NetResult>,
    http_client: &reqwest::Client,
    ws_manager: &mut ws_host::WsManager,
) {
    let commands_json = game.net_drain_commands();
    if commands_json != "[]" {
        match serde_json::from_str::<Vec<functor_runtime_common::net::NetCommand>>(&commands_json) {
            Ok(commands) => {
                for cmd in commands {
                    let tx = net_tx.clone();
                    let client = http_client.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(net_dispatch::perform_http(&client, cmd).await);
                    });
                }
            }
            Err(e) => eprintln!("[net] bad commands json: {e}"),
        }
    }

    let conn_json = game.net_drain_conn_commands();
    if conn_json != "[]" {
        match serde_json::from_str::<Vec<functor_runtime_common::net::ConnCommand>>(&conn_json) {
            Ok(commands) => {
                for cmd in commands {
                    ws_manager.handle(cmd);
                }
            }
            Err(e) => eprintln!("[net] bad conn commands json: {e}"),
        }
    }
}

/// Service one debug-server request. `capture` produces the PNG (or a
/// `CaptureError`) for `POST /capture` — a framebuffer readback in the windowed
/// loop, or `Unavailable` in headless.
#[allow(clippy::too_many_arguments)]
fn service_debug_request(
    req: debug_server::DebugRequest,
    game: &mut dyn Game,
    frame: &Frame,
    frame_count: u64,
    tts: f32,
    width: u32,
    height: u32,
    held_keys: &mut BTreeSet<InputKey>,
    mouse_pos: &mut (i32, i32),
    held_time: &mut Option<f32>,
    pending_step: &mut Option<f32>,
    capture: &dyn Fn() -> Result<Vec<u8>, debug_server::CaptureError>,
) {
    match req {
        debug_server::DebugRequest::Capture(resp) => {
            let _ = resp.send(capture());
        }
        debug_server::DebugRequest::State(resp) => {
            let _ = resp.send(debug_server::RuntimeState {
                frame: frame_count,
                tts,
                width,
                height,
                model: game.state_debug(),
                held_keys: held_keys.iter().copied().collect(),
                mouse: *mouse_pos,
            });
        }
        debug_server::DebugRequest::Scene(resp) => {
            let json = serde_json::to_string_pretty(frame)
                .unwrap_or_else(|e| format!("{{\"error\":{:?}}}", e.to_string()));
            let _ = resp.send(json);
        }
        debug_server::DebugRequest::ReloadSource(source, resp) => {
            let _ = resp.send(game.reload_source(&source));
        }
        debug_server::DebugRequest::Input(cmd, resp) => {
            let result = match cmd {
                debug_server::InputCommand::Key { key, down } => match key_from_str(&key) {
                    Some(k) => {
                        game.key_event(k as i32, down);
                        if down {
                            held_keys.insert(k);
                        } else {
                            held_keys.remove(&k);
                        }
                        Ok(())
                    }
                    None => Err(format!("unknown key: {}", key)),
                },
                debug_server::InputCommand::MouseMove { x, y } => {
                    *mouse_pos = (x, y);
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
                    *held_time = Some(tts);
                    *pending_step = None;
                }
                debug_server::TimeCommand::Advance { dts } => {
                    *pending_step = Some(dts);
                }
                debug_server::TimeCommand::Resume => {
                    *held_time = None;
                    *pending_step = None;
                }
            }
            let _ = resp.send(());
        }
    }
}

/// Headless game loop: drives the game + debug server with no GL window, for CI /
/// scripted / LLM-driven runs (`--headless`). Mirrors the windowed loop's game,
/// networking, and debug handling, minus everything that needs GL — rendering,
/// the framebuffer capture, and the GLFW event stream. `game.render` still runs
/// (it returns a pure `Frame`, no GL) to power `GET /scene`; `POST /capture`
/// returns an error since there is nothing rendered to read back.
fn run_headless(
    mut game: Box<dyn Game>,
    debug_requests: Option<std::sync::mpsc::Receiver<debug_server::DebugRequest>>,
    fixed_time: Option<f32>,
) {
    println!("[functor-runner] headless mode — no GL window; /capture unavailable");

    let start_time = Instant::now();
    let mut last_time: f32 = 0.0;
    let mut frame_count: u64 = 0;
    let mut held_time: Option<f32> = fixed_time;
    let mut pending_step: Option<f32> = None;
    let mut held_keys: BTreeSet<InputKey> = BTreeSet::new();
    let mut mouse_pos: (i32, i32) = (0, 0);

    // Same networking machinery as the windowed loop — driving networked/stateful
    // games is the whole point of a headless runner. Audio is omitted (no device
    // context), but queued audio commands are still drained so they don't pile up.
    let http_client = reqwest::Client::new();
    let (net_tx, net_rx) = std::sync::mpsc::channel::<net_dispatch::NetResult>();
    let (ws_tx, ws_rx) = std::sync::mpsc::channel::<ws_host::HostNetEvent>();
    let mut ws_manager = ws_host::WsManager::new(ws_tx);

    loop {
        let elapsed = start_time.elapsed().as_secs_f32();
        let time = next_frame_time(&mut held_time, &mut pending_step, last_time, elapsed);
        last_time = elapsed;

        // Same per-frame ordering as the windowed loop (the source of truth),
        // minus everything that needs GL.
        game.check_hot_reload(time.clone());
        deliver_net_ws(&mut *game, &net_rx, &ws_rx);
        game.tick(time.clone());
        dispatch_net_ws(&mut *game, &net_tx, &http_client, &mut ws_manager);

        // The frame is pure data (no GL); it powers GET /scene. Drain and drop
        // audio commands (no device in headless) so they don't pile up.
        let frame = game.render(time.clone());
        let _ = game.audio_drain_commands();

        if let Some(rx) = &debug_requests {
            while let Ok(req) = rx.try_recv() {
                service_debug_request(
                    req,
                    &mut *game,
                    &frame,
                    frame_count,
                    time.tts,
                    0, // no framebuffer in headless
                    0,
                    &mut held_keys,
                    &mut mouse_pos,
                    &mut held_time,
                    &mut pending_step,
                    &|| {
                        Err(debug_server::CaptureError::Unavailable(
                            "capture is unavailable in --headless mode".to_string(),
                        ))
                    },
                );
            }
        }

        frame_count += 1;
        // Cap the loop near 60 Hz so it doesn't busy-spin a core.
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

#[tokio::main]
pub async fn main() {
    // Load game

    let args = Args::parse();

    let game_path = args.game_path;
    println!("Using game path: {}", game_path);
    println!("Working directory: {:?}", env::current_dir());

    let mut game: Box<dyn Game> = if args.mle {
        Box::new(mle_game::MleGame::create(game_path.as_str()))
    } else if args.replay {
        Box::new(replay_game::ReplayGame::create(game_path.as_str()))
    } else if args.hot {
        Box::new(HotReloadGame::create(game_path.as_str()))
    } else {
        Box::new(StaticGame::create(game_path.as_str()))
    };

    // Optional debug control server. Runs on its own thread; the GL loop drains
    // its request channel once per frame (see below). None when --debug-port is
    // not given, so behavior is unchanged.
    let debug_requests = args
        .debug_port
        .map(|port| debug_server::spawn(&args.debug_bind, port));

    // Headless: drive the game + debug server with no GL window, and return.
    if args.headless {
        if args.capture_frame.is_some() {
            eprintln!(
                "error: --capture-frame is not supported with --headless (no GL to read back)"
            );
            std::process::exit(1);
        }
        if args.xreal_tracking {
            eprintln!("warning: --xreal-tracking has no effect in --headless mode (no view to rotate)");
        }
        run_headless(game, debug_requests, args.fixed_time);
        return;
    }

    // Hidden window: never shown / focused / cursor-capturing, so the run
    // doesn't steal input from the user. Capture runs are hidden by default —
    // there's no reason a scripted screenshot should grab the mouse.
    let hidden = args.hidden || args.capture_frame.is_some();

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
            if hidden {
                glfw.window_hint(glfw::WindowHint::Visible(false));
            }

            // glfw window creation
            // --------------------
            let (mut window, events) = glfw
                .create_window(SCR_WIDTH, SCR_HEIGHT, "Functor", glfw::WindowMode::Windowed)
                .expect("Failed to create GLFW window");

            window.make_current();
            window.set_key_polling(true);
            window.set_cursor_pos_polling(true);
            window.set_scroll_polling(true);
            window.set_mouse_button_polling(true);
            window.set_framebuffer_size_polling(true);
            // Capture and hide the cursor so the game gets continuous relative
            // mouse motion (free-look) instead of the pointer stopping at the
            // window edges. Escape RELEASES the cursor (essential for the
            // hot-reload loop: tweak code in the editor while the game runs);
            // click recaptures; Escape while released quits. Losing focus
            // also releases, so cmd-tabbing away hands the pointer back.
            // A hidden window never gets focus, so it must not grab the cursor.
            if !hidden {
                window.set_cursor_mode(glfw::CursorMode::Disabled);
            }

            let gl =
                glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);

            (gl, "#version 410", window, glfw, events)
        };

        // Share the GL context via Arc so the egui text-overlay painter can keep
        // its own reference (egui_glow::Painter requires Arc<glow::Context>). The
        // rest of the runtime keeps using `&gl`, which derefs through the Arc.
        let gl = std::sync::Arc::new(gl);
        let mut text_overlay = functor_runtime_common::ui::TextOverlay::new(gl.clone());

        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;
        let mut frame_count: u64 = 0;
        // Debug-server clock control (POST /time). `held_time` pins the frame
        // time to a constant (None = follow wall clock); `pending_step` applies a
        // one-shot dt advance on the next frame. Seeded from --fixed-time.
        let mut held_time: Option<f32> = args.fixed_time;
        let mut pending_step: Option<f32> = None;
        // Runtime-owned input snapshot for GET /state, maintained from both the
        // GLFW event stream and the debug server's POST /input. Generic and
        // serializable, unlike the game model (which is Debug text only).
        let mut held_keys: BTreeSet<InputKey> = BTreeSet::new();
        let mut mouse_pos: (i32, i32) = (0, 0);
        // Whether the window owns the pointer (free-look). See the Escape /
        // MouseButton / Focus arms in the event loop. A hidden window never
        // captures (and never receives the events that would toggle this).
        let mut cursor_captured = !hidden;

        use glfw::Context;

        let asset_cache = Arc::new(AssetCache::new());

        let scene_context = SceneContext::new();

        // The directional shadow map: a depth texture rendered from the casting
        // light each frame, sampled by the lit material.
        let shadow_map = functor_runtime_common::shadow::ShadowMap::new(&gl, 2048);

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

        // HTTP dispatch. Commands the game queues each frame are performed on
        // tokio tasks (so a slow request never stalls the frame loop); each result
        // returns over this channel and is pushed back into the game on the main
        // thread, keeping all dylib calls on one thread.
        let http_client = reqwest::Client::new();
        let (net_tx, net_rx) = std::sync::mpsc::channel::<net_dispatch::NetResult>();

        // WebSocket connections. Commands (connect/send/close) are drained each
        // frame and performed by the manager on tokio tasks; socket events return
        // over this channel and are pushed into the game on the main thread.
        let (ws_tx, ws_rx) = std::sync::mpsc::channel::<ws_host::HostNetEvent>();
        let mut ws_manager = ws_host::WsManager::new(ws_tx);

        // Audio device, owned by the host (survives hot reload). `None` when no
        // device is available — audio commands are then drained and dropped.
        // `playThen` completions come back over this channel and are reported to
        // the game before the next tick (like net results). The player is `mut`
        // so the listener can be updated from each frame's camera.
        let (audio_tx, audio_rx) = std::sync::mpsc::channel::<u64>();
        let mut audio_player = audio::AudioPlayer::new(audio_tx);

        // Xreal head tracking: a background thread owns the TCP stream +
        // sensor fusion; the loop reads the latest orientation each frame and
        // rotates the game's camera with it. None when the flag is off.
        let xreal_tracker = args
            .xreal_tracking
            .then(|| xreal::XrealTracker::spawn(args.xreal_addr));

        while !window.should_close() {
            let elapsed_time = start_time.elapsed().as_secs_f32();
            // The frame time handed to the game. Pinning it (--fixed-time, or the
            // debug server's /time) makes the rendered pose deterministic — used
            // for reproducible captures / golden images. The capture trigger
            // below still keys off wall-clock elapsed, so the loop runs long
            // enough for assets to load before a shot is taken.
            let time = next_frame_time(&mut held_time, &mut pending_step, last_time, elapsed_time);
            last_time = elapsed_time;

            game.check_hot_reload(time.clone());

            glfw.poll_events();
            // When time is pinned (`--fixed-time` or the debug server's /time),
            // we're in a deterministic/capture mode — ignore user window input so
            // the pose stays reproducible (e.g. a stray mouse-over during a golden
            // capture can't turn the camera). Window close/escape and the debug
            // server's /input still work.
            let ignore_user_input = held_time.is_some();
            for (_, event) in glfw::flush_messages(&events) {
                match event {
                    glfw::WindowEvent::Close => window.set_should_close(true),
                    // First Escape releases the cursor (edit code while the
                    // game runs — the hot-reload workflow); Escape again while
                    // released quits, preserving the Esc-Esc exit.
                    glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                        if cursor_captured {
                            window.set_cursor_mode(glfw::CursorMode::Normal);
                            cursor_captured = false;
                            println!(
                                "[runner] cursor released — click the window to recapture, \
Escape again to quit"
                            );
                        } else {
                            window.set_should_close(true)
                        }
                    }
                    // A click while released recaptures for free-look. Never
                    // on a hidden window — it must not grab the pointer.
                    glfw::WindowEvent::MouseButton(_, Action::Press, _)
                        if !cursor_captured && !hidden =>
                    {
                        window.set_cursor_mode(glfw::CursorMode::Disabled);
                        cursor_captured = true;
                    }
                    // F1 recenters head tracking (runner-level, never reaches
                    // the game): current head pose becomes "straight ahead".
                    glfw::WindowEvent::Key(Key::F1, _, Action::Press, _)
                        if xreal_tracker.is_some() =>
                    {
                        if let Some(tracker) = &xreal_tracker {
                            tracker.request_recenter();
                            println!("[xreal] recentered");
                        }
                    }
                    // Always honor key releases and focus-loss, even while other
                    // input is ignored (pinned clock) — otherwise a key held at
                    // the pin transition, or released after alt-tab, would stick
                    // in held_keys forever. Releases can only *clear* input, so
                    // they don't perturb a pinned/deterministic pose.
                    glfw::WindowEvent::Key(key, _, Action::Release, _) => {
                        let k = map_key(key);
                        game.key_event(k as i32, false);
                        held_keys.remove(&k);
                    }
                    glfw::WindowEvent::Focus(false) => {
                        for k in std::mem::take(&mut held_keys) {
                            game.key_event(k as i32, false);
                        }
                        // Hand the pointer back when the window loses focus
                        // (cmd-tab to the editor); a click recaptures.
                        window.set_cursor_mode(glfw::CursorMode::Normal);
                        cursor_captured = false;
                    }
                    _ if ignore_user_input => {}
                    glfw::WindowEvent::Key(key, _, Action::Press | Action::Repeat, _) => {
                        let k = map_key(key);
                        game.key_event(k as i32, true);
                        if k != InputKey::Unknown {
                            held_keys.insert(k);
                        }
                    }
                    // While the cursor is released, pointer motion/scroll
                    // must not drive the camera (you're aiming at your editor,
                    // not the game).
                    glfw::WindowEvent::CursorPos(x, y) if cursor_captured => {
                        mouse_pos = (x as i32, y as i32);
                        game.mouse_move(x as i32, y as i32)
                    }
                    glfw::WindowEvent::Scroll(_, y) if cursor_captured => {
                        game.mouse_wheel(y as i32)
                    }
                    _ => {}
                }
            }

            // Deliver async HTTP + WebSocket results into the game's inbox before
            // tick (shared with the headless loop).
            deliver_net_ws(&mut *game, &net_rx, &ws_rx);

            // Deliver any `playThen` completions that finished since last frame,
            // before tick so this frame's executor drain delivers their messages.
            // (Windowed-only: the headless loop has no audio device.)
            while let Ok(token) = audio_rx.try_recv() {
                game.audio_push_finished(token as i32);
            }

            game.tick(time.clone());

            // Perform the HTTP + WebSocket commands this frame's tick queued
            // (shared with the headless loop).
            dispatch_net_ws(&mut *game, &net_tx, &http_client, &mut ws_manager);

            // Follow window resizes: query the drawable size each frame.
            // Framebuffer size is in pixels, so this handles HiDPI/retina.
            let (fb_width, fb_height) = window.get_framebuffer_size();
            let viewport = functor_runtime_common::Viewport::new(fb_width as u32, fb_height as u32);

            // The game supplies the camera/scene/lights as part of its frame.
            let frame = game.render(time.clone());

            // The camera actually viewed/heard: the game's camera, rotated by
            // the head orientation when Xreal tracking is on.
            let view_camera = match &xreal_tracker {
                Some(tracker) => xreal::apply_head_rotation(&frame.camera, tracker.orientation()),
                None => frame.camera.clone(),
            };

            // Audio: set the listener from the viewed camera, then play any
            // one-shots the tick queued (positioned ones pan relative to it).
            if let Some(player) = &mut audio_player {
                player.set_listener(view_camera.eye, view_camera.target, view_camera.up);
            }
            let audio_json = game.audio_drain_commands();
            if audio_json != "[]" {
                match serde_json::from_str::<Vec<functor_runtime_common::audio::AudioCommand>>(
                    &audio_json,
                ) {
                    Ok(commands) => {
                        if let Some(player) = &audio_player {
                            for cmd in commands {
                                player.handle(cmd);
                            }
                        }
                    }
                    Err(e) => eprintln!("[audio] bad commands json: {e}"),
                }
            }

            // Soundscape: reconcile the game's desired looping voices against the
            // live ones, then re-aim the spatial voices at the (possibly moved)
            // listener so they pan/attenuate as the camera moves.
            if let Some(player) = &mut audio_player {
                let scene_json = game.audio_scene_json();
                match serde_json::from_str::<functor_runtime_common::audio::AudioScene>(&scene_json)
                {
                    Ok(scene) => player.reconcile_scene(&scene),
                    Err(e) => eprintln!("[audio] bad scene json: {e}"),
                }
                player.respatialize_voices();
            }

            // Shadow + forward passes, shared with the web runtime. In stereo
            // mode, render the same frame twice — once per eye camera into each
            // half of the window. (The shadow pass runs per eye; it must start
            // unscissored, so scissor is reset before each call — same contract
            // as the netsim viewer's panes.)
            if args.stereo_sbs {
                let (left_cam, right_cam) = view_camera.stereo_eyes(args.stereo_ipd);
                // Odd widths: the right eye absorbs the extra column, so the
                // two viewports tile the framebuffer exactly (no stale strip).
                let half_w = (fb_width as u32) / 2;
                let right_w = fb_width as u32 - half_w;
                let eyes = [
                    (left_cam, functor_runtime_common::Viewport::with_offset(0, 0, half_w, fb_height as u32)),
                    (right_cam, functor_runtime_common::Viewport::with_offset(half_w, 0, right_w, fb_height as u32)),
                ];
                for (camera, eye_viewport) in &eyes {
                    gl.disable(glow::SCISSOR_TEST);
                    functor_runtime_common::render_frame(
                        &gl,
                        shader_version,
                        asset_cache.clone(),
                        &scene_context,
                        &shadow_map,
                        &frame,
                        camera,
                        time.clone(),
                        *eye_viewport,
                        args.debug_render.into(),
                    );
                }
            } else {
                functor_runtime_common::render_frame(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &frame,
                    &view_camera,
                    time.clone(),
                    viewport,
                    args.debug_render.into(),
                );
            }

            // 2D UI overlay: the game's declarative `ui model` View, lowered to a
            // text overlay on top of the 3D frame. Drawn before capture so it
            // appears in --capture-frame PNGs.
            let ui_view = game.ui();
            text_overlay.draw_view(fb_width as u32, fb_height as u32, 1.0, &ui_view);

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
                    service_debug_request(
                        req,
                        &mut *game,
                        &frame,
                        frame_count,
                        time.tts,
                        fb_width as u32,
                        fb_height as u32,
                        &mut held_keys,
                        &mut mouse_pos,
                        &mut held_time,
                        &mut pending_step,
                        // GL readback on the render thread (a real Failed on error).
                        &|| {
                            encode_framebuffer_png(&gl, fb_width as u32, fb_height as u32)
                                .map_err(debug_server::CaptureError::Failed)
                        },
                    );
                }
            }

            window.swap_buffers();
            frame_count += 1;
        }
    }

    game.quit();
}
