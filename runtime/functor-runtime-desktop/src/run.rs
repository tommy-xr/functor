//! The desktop runtime's run loop, extracted from the former `functor-runner`
//! binary so the `functor` CLI can drive it IN-PROCESS ([`run`]).
//!
//! [`run`] MUST be called on the process's main thread: it creates the GLFW
//! window and pumps its event loop, which macOS/Cocoa requires on the main
//! thread. The CLI's `#[tokio::main]` drives its async `main` future (and thus
//! this call) on the main thread; HTTP/net work stays on tokio worker threads
//! via `tokio::spawn`, so `run` must be invoked from within a tokio runtime
//! context.

use std::sync::Arc;
use std::time::Instant;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::{Frame, FrameTime, GameClock, RecordedInput, SceneContext};
use std::collections::{BTreeSet, HashMap};

use functor_runtime_common::Key as InputKey;
use glfw::{Action, Key};
use glow::*;

use crate::game::Game;
use crate::{
    asset_watch, audio, debug_server, functor_lang_game, net_dispatch, replay_game, ws_host, xreal,
};

const SCR_WIDTH: u32 = 800;
const SCR_HEIGHT: u32 = 600;

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
        // Digit row and numpad both fold into the canonical digits.
        Key::Num0 | Key::Kp0 => InputKey::Num0,
        Key::Num1 | Key::Kp1 => InputKey::Num1,
        Key::Num2 | Key::Kp2 => InputKey::Num2,
        Key::Num3 | Key::Kp3 => InputKey::Num3,
        Key::Num4 | Key::Kp4 => InputKey::Num4,
        Key::Num5 | Key::Kp5 => InputKey::Num5,
        Key::Num6 | Key::Kp6 => InputKey::Num6,
        Key::Num7 | Key::Kp7 => InputKey::Num7,
        Key::Num8 | Key::Kp8 => InputKey::Num8,
        Key::Num9 | Key::Kp9 => InputKey::Num9,
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
        if c.is_ascii_digit() {
            return InputKey::from_i32((c - b'0') as i32 + InputKey::Num0 as i32);
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

/// Parse an `--input-script` file into a frame → events map for deterministic
/// scripted playback (docs/time-travel.md T6b). Each non-blank, non-comment
/// line is `<frame:int> <KeyName> <down|up>` — e.g. `0 Right down`, `18 Up down`.
/// `#` starts a comment (to end of line); the key name goes through the same
/// `key_from_str` map the debug server's POST /input uses. Events are stored as
/// raw `RecordedInput::Key` so playback re-runs the identical live input path.
fn parse_input_script(path: &str) -> Result<HashMap<u64, Vec<RecordedInput>>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read input script {path}: {e}"))?;
    let mut map: HashMap<u64, Vec<RecordedInput>> = HashMap::new();
    for (i, raw) in text.lines().enumerate() {
        let lineno = i + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(format!(
                "{path}:{lineno}: expected `<frame> <Key> <down|up>`, got `{raw}`"
            ));
        }
        let frame: u64 = parts[0]
            .parse()
            .map_err(|_| format!("{path}:{lineno}: bad frame number `{}`", parts[0]))?;
        let key = key_from_str(parts[1])
            .ok_or_else(|| format!("{path}:{lineno}: unknown key `{}`", parts[1]))?;
        let is_down = match parts[2].to_ascii_lowercase().as_str() {
            "down" => true,
            "up" => false,
            other => return Err(format!("{path}:{lineno}: expected `down|up`, got `{other}`")),
        };
        map.entry(frame).or_default().push(RecordedInput::Key {
            code: key as i32,
            is_down,
        });
    }
    Ok(map)
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
    /// Normal shading plus the physics collider/contact wireframe overlay.
    Physics,
}

impl From<DebugRenderArg> for functor_runtime_common::DebugRenderMode {
    fn from(arg: DebugRenderArg) -> Self {
        match arg {
            DebugRenderArg::Default => functor_runtime_common::DebugRenderMode::Default,
            DebugRenderArg::Normals => functor_runtime_common::DebugRenderMode::Normals,
            DebugRenderArg::Tangents => functor_runtime_common::DebugRenderMode::Tangents,
            DebugRenderArg::Physics => functor_runtime_common::DebugRenderMode::Physics,
        }
    }
}

/// Screen-space compositor smoke test (docs/time-travel.md T5). A dev/verify
/// hook that routes the frame through `render_composited_frames` instead of the
/// normal single-frame path.
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
enum CompositeDemoArg {
    /// Normal single-frame render.
    #[default]
    Off,
    /// Composite the frame with itself at 0.5/0.5 — must be pixel-identical to a
    /// normal render (proves the pass is lossless).
    Clone,
    /// Overlay the frame with a world-space-offset copy of its scene at 0.5/0.5
    /// — two "timelines" ghosted (the fork+overlay demo).
    Fork,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    game_path: String,

    /// Treat --game-path as an `.fun` source file and run it through the Functor Lang
    /// interpreter with the Functor prelude (docs/functor-lang.md Track C2). This is the
    /// only game producer; the flag is retained because the CLI/SDK pass it.
    /// Prints per-frame eval cost every 300 frames.
    #[arg(long)]
    functor_lang: bool,

    /// Treat --game-path as a frame-recording JSON (a single serialized `Frame`
    /// or a JSON array of them — the exact format `GET /scene` emits) and replay
    /// it instead of loading a game dylib. A proof producer for the
    /// producer-agnostic seam (docs/functor-lang.md Track A3). Each producer mode
    /// reinterprets --game-path, so combining them is an error, not a silent
    /// precedence pick.
    #[arg(long, conflicts_with_all = ["functor_lang"])]
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

    /// Drive the game deterministically from a scripted input file instead of
    /// live window input (docs/time-travel.md T6b). Each line is
    /// `<frame:int> <KeyName> <down|up>` (KeyName like `Right`, `Up`, `A`,
    /// `Space`); `#` starts a comment. The sim advances by a FIXED `--script-dt`
    /// per rendered frame (not wall-clock), and each frame's scripted events are
    /// fed before that frame's tick, so frame N is always the same sim state —
    /// combine with `--capture-frame`/`--capture-at-frame` for reproducible
    /// stills. Off by default (goldens/live runs unaffected).
    #[arg(long, conflicts_with = "fixed_time")]
    input_script: Option<String>,

    /// Fixed per-frame timestep (seconds) for `--input-script` deterministic
    /// playback. Default is one 60 Hz frame. Must be positive and finite — a
    /// zero/negative/NaN dt would stall or reverse the sim clock.
    #[arg(long, default_value_t = 1.0 / 60.0, value_parser = parse_script_dt)]
    script_dt: f32,

    /// Capture `--capture-frame` at this exact deterministic sim frame (0-based),
    /// then exit — instead of the wall-clock `--capture-time` trigger. Intended
    /// with `--input-script`, so captures at frames 0, 3, 6, … are reproducible
    /// stills of a scripted run.
    #[arg(long, requires = "capture_frame")]
    capture_at_frame: Option<u64>,

    /// Start an HTTP control server on <--debug-bind>:<PORT> exposing
    /// POST /capture (image/png of the next frame), GET /state (runtime JSON),
    /// and POST /reload-source (network hot-reload for Functor Lang games).
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

    /// Route the frame through the screen-space compositor (docs/time-travel.md
    /// T5) instead of the normal render — a headless verification hook. See
    /// `CompositeDemoArg`.
    #[arg(long, value_enum, default_value_t = CompositeDemoArg::Off)]
    composite_demo: CompositeDemoArg,

    /// Forward-ghosting trajectory preview (docs/time-travel.md T6d): composite
    /// N forward-stepped frames into one image (chronophotography), so moving
    /// elements smear into a strobe of their ~2s future positions while static
    /// geometry stays solid. Off by default; when on it composites every frame
    /// (no pause gating), so it's deterministically capturable under
    /// --fixed-time. The window is fixed at ~2s (`dt = 2.0 / --ghost-divisions`).
    #[arg(long)]
    ghost: bool,

    /// Number of forward divisions composited by --ghost (clamped to 8, the
    /// compositor max). More divisions = a finer strobe over the same ~2s window.
    #[arg(long, default_value_t = 8)]
    ghost_divisions: usize,

    /// Scene-diff trajectory preview (docs/time-travel.md T6, scene-diff
    /// variant): forward-simulate the model and draw a clean dotted trail
    /// tracing ONLY the scene nodes that move — derived by the runtime from
    /// `draw`, with no game logic. Unlike --ghost's whole-scene strobe, static
    /// geometry contributes nothing. Capturable under --fixed-time.
    #[arg(long)]
    trajectory: bool,

    /// Scene-space strobe preview: forward-simulate the model and overlay
    /// real-geometry copies of each MOVING scene node at its future poses,
    /// color-faded by age — full-intensity chronophotography on the normal
    /// render path. The geometry complement to --ghost's screen-space
    /// compositor (which pins every copy at 1/N opacity and freezes the
    /// camera); like --trajectory it's runtime-derived from `draw`, shares one
    /// forward-sim with it, and is capturable under --fixed-time.
    #[arg(long)]
    strobe: bool,

    /// Start with the time-travel scrubber overlay visible (it is otherwise
    /// hidden until summoned with `~`). Useful for demos and captures of the
    /// scrubber itself. NOTE: with the overlay up, previews follow the
    /// interactive rule (paused only) — combine with --trajectory/--strobe/
    /// --ghost WITHOUT --scrubber for headless overlay captures.
    #[arg(long)]
    scrubber: bool,

    /// Narrate the game clock + time-travel seams to stderr: pause/resume/seek
    /// rebases (with the tts they land on), ghost on/off, and per-frame HITCH
    /// warnings when a rendered frame's dt blows past 33ms (the tell-tale of the
    /// ghost compositor starving the frame loop). High-signal diagnostics for the
    /// interactive scrubber — off by default.
    #[arg(long)]
    debug_clock: bool,

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

/// `--script-dt` must be a positive, finite timestep: a zero/NaN dt would stall
/// the sim clock and a negative one would run it backwards, breaking the
/// deterministic-playback contract.
fn parse_script_dt(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err("must be a positive, finite timestep in seconds".into());
    }
    Ok(v)
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
        Ok(()) => functor_runtime_common::events::emit(
            functor_runtime_common::events::RuntimeEvent::CaptureWritten {
                path: path.to_string(),
            },
        ),
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
    clock: &mut GameClock,
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
        debug_server::DebugRequest::Trace(resp) => {
            // The paused-inspector trace (visual-debugger PR2). Paused-ness is a
            // clock property: while paused the last real frame's journal is
            // stable, so the producer replays it; otherwise it early-outs empty.
            let _ = resp.send(game.inspector_trace(clock.is_paused()));
        }
        debug_server::DebugRequest::ReloadSource(source, resp) => {
            let _ = resp.send(game.reload_source(&source));
        }
        debug_server::DebugRequest::Rewind(frame, resp) => {
            let result = game.rewind_scene_to(frame);
            // Rebase the game clock to the rewound frame's recorded `tts` so play
            // continues FROM the scrubbed scene time, not wall-clock "now" (the
            // resume seam — docs/time-travel.md). After the rewind branched the
            // future, `current_scene_tts` reports the newest (target) frame.
            if result.is_ok() {
                if let Some(tts) = game.current_scene_tts() {
                    clock.rebase(tts as f32);
                }
            }
            let _ = resp.send(result);
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
                debug_server::InputCommand::UiEvent { slot, kind } => {
                    game.ui_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
                debug_server::InputCommand::WebviewEvent { slot, kind } => {
                    game.webview_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
            };
            // Input injected while PAUSED journals entry-point calls no `tick`
            // will sweep — fold them into the inspector's last-frame journal so
            // they show in `GET /trace` now, not as phantoms on resume (PR2).
            if clock.is_paused() {
                game.absorb_paused_input();
            }
            let _ = resp.send(result);
        }
        debug_server::DebugRequest::Time(cmd, resp) => {
            match cmd {
                debug_server::TimeCommand::Set { tts } => clock.set(tts),
                debug_server::TimeCommand::Advance { dts } => clock.advance(dts),
                debug_server::TimeCommand::Resume => clock.resume(),
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
    // Stderr, not stdout: keep the CLI's `--json` ndjson stream (stdout) clean
    // even under `--headless`. This is an out-of-band notice, not an event.
    eprintln!("[runtime] headless mode — no GL window; /capture unavailable");

    let start_time = Instant::now();
    let mut last_time: f32 = 0.0;
    let mut frame_count: u64 = 0;
    let mut clock = GameClock::new(fixed_time);
    let mut held_keys: BTreeSet<InputKey> = BTreeSet::new();
    let mut mouse_pos: (i32, i32) = (0, 0);

    // Same networking machinery as the windowed loop — driving networked/stateful
    // games is the whole point of a headless runner. Audio is omitted (no device
    // context), but queued audio commands are still drained so they don't pile up.
    let http_client = reqwest::Client::new();
    net_dispatch::install_remote_asset_fetcher(http_client.clone());
    let (net_tx, net_rx) = std::sync::mpsc::channel::<net_dispatch::NetResult>();
    let (ws_tx, ws_rx) = std::sync::mpsc::channel::<ws_host::HostNetEvent>();
    let mut ws_manager = ws_host::WsManager::new(ws_tx);

    loop {
        let elapsed = start_time.elapsed().as_secs_f32();
        // Fixed-timestep model loop, same as the windowed loop (docs/time-travel
        // .md): 0..N fixed 1/60 steps per iteration (one per debug `/time advance`,
        // one {dts:0} under a pin, none while paused). `time` carries the settled
        // render `tts` for /scene + /state.
        let sub_frames = clock.fixed_frames(elapsed - last_time);
        last_time = elapsed;
        let time = FrameTime {
            dts: 0.0,
            tts: clock.current_tts(),
        };

        // Same per-frame ordering as the windowed loop (the source of truth),
        // minus everything that needs GL.
        game.check_hot_reload(time.clone());
        deliver_net_ws(&mut *game, &net_rx, &ws_rx);
        for sub in &sub_frames {
            game.tick(sub.clone());
            frame_count += 1;
        }
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
                    &mut clock,
                    &|| {
                        Err(debug_server::CaptureError::Unavailable(
                            "capture is unavailable in --headless mode".to_string(),
                        ))
                    },
                );
            }
        }

        // Cap the loop near 60 Hz so it doesn't busy-spin a core.
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

/// Run the desktop runtime to completion (windowed loop until the window
/// closes / a `--capture-frame` shot is taken, or the headless loop until the
/// process is killed).
///
/// MUST be called on the main thread (GLFW/Cocoa) and from within a tokio
/// runtime context (net dispatch uses `tokio::spawn`). The former
/// `functor-runner` binary satisfied both via `#[tokio::main]` on `main`; the
/// `functor` CLI satisfies both by calling this from its own `#[tokio::main]`
/// main future (which `block_on` drives on the main thread).
pub fn run(args: Args) {
    // Load game

    let game_path = args.game_path.clone();

    let mut game: Box<dyn Game> = if args.replay {
        Box::new(replay_game::ReplayGame::create(game_path.as_str()))
    } else if args.functor_lang {
        Box::new(functor_lang_game::FunctorLangGame::create(game_path.as_str()))
    } else {
        // Functor Lang is the only game producer now (the F#/dylib path was removed in
        // E3). The CLI and SDK always pass --functor-lang; a bare invocation has no
        // producer to load.
        eprintln!(
            "error: no game producer selected — pass --functor-lang --game-path <file.fun> \
(the F#/dylib producer was removed in E3)"
        );
        std::process::exit(1);
    };

    // Scripted deterministic input (docs/time-travel.md T6b). Parsed up front so
    // a malformed script is a clean CLI error before any window/GL work. None
    // (the default) leaves live input and the wall-clock capture trigger
    // untouched.
    let input_script: Option<HashMap<u64, Vec<RecordedInput>>> =
        args.input_script.as_ref().map(|path| match parse_input_script(path) {
            Ok(map) => map,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        });

    // `--capture-at-frame` only names a *deterministic* sim frame when playback
    // advances by a fixed dt — i.e. under `--input-script`. Without it the loop
    // runs on wall-clock time, so "frame N" is not reproducible; refuse rather
    // than silently capture a non-deterministic frame.
    if args.capture_at_frame.is_some() && input_script.is_none() {
        eprintln!("error: --capture-at-frame requires --input-script (its frame index is only deterministic under scripted fixed-dt playback)");
        std::process::exit(1);
    }

    // The game loaded and validated (incl. any scripted-input parse); the runtime
    // is up. One-shot lifecycle notice for the shell (replaces the old
    // game-path/working-dir debug prints).
    functor_runtime_common::events::emit(functor_runtime_common::events::RuntimeEvent::Ready);

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
            // Cap the live windowed loop at the display's refresh with vsync, so
            // a cheap scene doesn't spin uncapped and pin the GPU. GATED OFF for
            // hidden/capture and deterministic (`--fixed-time`) runs, which must
            // keep their current timing behavior — a golden capture or the debug
            // server's /capture must never block on the compositor's swap. Applies
            // to the current context, so it must follow `make_current`.
            if !hidden && args.fixed_time.is_none() {
                glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
            }
            window.set_key_polling(true);
            // Printable characters for a focused `Ui.textInput`
            // (docs/ui-interaction.md U4) — GLFW delivers text as separate
            // Char events beside the Key stream.
            window.set_char_polling(true);
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
        // The game's HTML/CSS webview overlay (`webview model`), blitz-rendered
        // to a texture and composited between the game UI and the scrubber.
        let mut webview_overlay =
            crate::webview_overlay::WebviewOverlay::new(gl.clone(), shader_version);
        // The shell-owned time-travel scrubber (docs/time-travel.md T3), drawn
        // over the frame; interactive when the cursor is released (Escape).
        let mut scrubber = functor_runtime_common::ui::Scrubber::new(gl.clone());
        // Primary mouse-button state for the scrubber, and whether egui wanted
        // the pointer last frame (so a click on a control doesn't recapture the
        // cursor for free-look).
        let mut mouse_primary_down = false;
        // Press-latch for the frame's pointer SAMPLE: a click whose press and
        // release both land in one poll batch (a fast tap, a slow frame)
        // leaves `mouse_primary_down` false by sample time — the latch keeps
        // the sampled level true for that frame so egui still sees the press
        // edge (the release follows next frame). Cleared after each sample.
        let mut mouse_primary_clicked = false;
        let mut scrubber_wants_pointer = false;
        // Whether the game UI's egui pass wanted the pointer last frame — a
        // click on a `Ui.button` must drive the widget, not recapture the
        // cursor for free-look (the scrubber rule, docs/ui-interaction.md).
        let mut ui_wants_pointer = false;
        // Same latch for the webview overlay: the pointer is over an element
        // with an `Attr.onClick`/`Attr.onInput` handler.
        let mut webview_wants_pointer = false;
        // Whether it wanted the KEYBOARD last frame (a `Ui.textInput` is
        // focused): keys then route to the overlay instead of the game's
        // `input` hook, and Escape defocuses the field instead of releasing
        // the cursor (docs/ui-interaction.md U4).
        let mut ui_wants_keyboard = false;
        // The time-travel console (`~`): HIDDEN by default in the native game
        // (it's a dev tool you summon with `~`, which also frees the cursor so
        // you can scrub right away; --scrubber starts it open, e.g. for
        // captures). The wasm/vscode preview shows it always.
        let mut scrubber_visible = args.scrubber;
        // Future-preview (docs/time-travel.md T6/T6d) state, driven
        // interactively from the scrubber's "extrapolate" switch and ⚙ popover
        // (mode / window / rate). Extrapolation defaults ON with trail+strobe
        // out of the box (the demo default), and the launch
        // flags pick the mode; they stay authoritative while the overlay is
        // hidden (the headless-capture escape hatch), where they may also
        // COMBINE (--ghost --trajectory composes the trail into the strobe).
        let mut extrapolate_on = true;
        let mut preview_mode = match (args.ghost, args.trajectory, args.strobe) {
            (true, _, _) => functor_runtime_common::PreviewMode::Ghost,
            (false, true, false) => functor_runtime_common::PreviewMode::Trail,
            (false, false, true) => functor_runtime_common::PreviewMode::Strobe,
            _ => functor_runtime_common::PreviewMode::Both,
        };
        // The ⚙ popover's shared forward window and samples-per-second rate
        // (density holds as the window resizes: total samples = rate × window).
        let mut preview_window: f32 = 2.0;
        let mut preview_rate: usize = 5;
        // --trajectory/--strobe preview cache. The 32-division forward-sim is
        // the expensive part, and while PAUSED its anchor (scene frame + tts) is
        // frozen — so reuse the computed preview while the anchor key matches,
        // but still refresh after a bounded number of frames so a hot-reload's
        // edited code re-projects within ~half a second (the anchor can't see a
        // code edit). While live, the last projection remains visible between
        // wall-clock refreshes so continuous extrapolation bounds repeated work.
        const PAUSED_PREVIEW_REUSE_FRAMES: u32 = 30;
        const LIVE_PREVIEW_INTERVAL_SECONDS: f32 = 0.1;
        let mut trail_cache: Option<(
            (Option<u64>, u32, bool, bool, bool, usize, u32),
            functor_runtime_common::ScenePreview,
        )> = None;
        let mut trail_refresh: u32 = 0;
        let mut next_live_trail_refresh: f32 = 0.0;
        let mut ghost_cache: Option<(
            (Option<u64>, u32, bool, usize, u32),
            Vec<(Frame, FrameTime)>,
        )> = None;
        let mut ghost_refresh: u32 = 0;
        let mut next_live_ghost_refresh: f32 = 0.0;

        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;
        let mut frame_count: u64 = 0;
        // The shared game clock (docs/time-travel.md): `tts` accumulates the real
        // frame delta while live, freezes on pause (scrubber / debug POST /time),
        // and rebases on a time-travel branch. `--fixed-time` seeds an
        // unconditional pin for deterministic captures / goldens.
        let mut clock = GameClock::new(args.fixed_time);
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
        // Asset hot-reload state: mtime stamps for every loaded asset file
        // (see the check next to `check_hot_reload` in the frame loop).
        let mut asset_watcher = asset_watch::AssetWatcher::new();
        let mut last_asset_poll = Instant::now();

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
        // Remote (URL) asset paths download through the same client (see
        // net_dispatch::install_remote_asset_fetcher).
        net_dispatch::install_remote_asset_fetcher(http_client.clone());
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
            // Scripted deterministic playback (docs/time-travel.md T6b): advance
            // the game clock by a FIXED --script-dt this frame instead of
            // wall-clock, so frame N is always the same sim state. Queued as a
            // one-shot step (which the clock consumes in `frame()` below), making
            // every rendered frame a single fixed dt regardless of real timing.
            // Under --ghost the script drives the GHOST, not the live game (F2,
            // docs/time-travel.md): the live game stays at its init anchor, so
            // don't fixed-step its clock — the ghost forward-step supplies the
            // deterministic sub-dt itself.
            if input_script.is_some() && !args.ghost {
                clock.step(args.script_dt);
            }
            // The fixed-timestep model loop (docs/time-travel.md): advance `tick`
            // in whole 1/60 steps decoupled from the render rate, so the sim is
            // deterministic and a recorded frame is exactly one forward-step fine
            // step (the ghost replay's assumption). Scripted playback queued a
            // one-shot `step` above, so this yields exactly one sub-frame then;
            // --fixed-time / the debug pin yields one {dts:0} sub-frame; paused
            // yields none. `time` is the RENDER frame time — the settled `tts` the
            // frame is drawn / hot-reloaded / reported at (its `dts` is unused by
            // render/draw). Capture still keys off wall-clock `elapsed_time` so the
            // loop runs long enough for assets to load before a shot.
            let real_delta = elapsed_time - last_time;
            let sub_frames = clock.fixed_frames(real_delta);
            last_time = elapsed_time;
            let time = FrameTime {
                dts: 0.0,
                tts: clock.current_tts(),
            };

            // Surface frame hitches (a real delta past 33ms = under 30fps) — the
            // signature of the ghost compositor's N forward-steps starving the
            // loop, which reads to the user as "jerky / can't move".
            if args.debug_clock && real_delta > 1.0 / 30.0 {
                eprintln!(
                    "[clock] HITCH dt={:.1}ms tts={:.3} (slow frame)",
                    real_delta * 1000.0,
                    time.tts
                );
            }

            game.check_hot_reload(time.clone());

            // The loading snapshot for `Sub.assets`: pushed every frame, the
            // producer only acts when it changed since the game last saw it.
            game.push_asset_progress(asset_cache.progress());

            // Asset hot-reload, the twin of the .fun check above: a model/
            // texture saved on disk is evicted from the caches so the next
            // draw re-reads and re-decodes it. Throttled: unlike the handful
            // of .fun files, the asset set is unbounded (a stat per asset per
            // poll), and 4Hz is plenty for a save-and-look loop.
            if last_asset_poll.elapsed().as_millis() >= 250 {
                last_asset_poll = Instant::now();
                for path in asset_watcher.changed(asset_cache.loaded_paths()) {
                    log::info!("asset '{}' changed on disk; reloading", path);
                    asset_cache.evict(&path);
                    scene_context.evict_asset(&path);
                }
            }

            glfw.poll_events();
            // When time is pinned (`--fixed-time` or the debug server's /time),
            // we're in a deterministic/capture mode — ignore user window input so
            // the pose stays reproducible (e.g. a stray mouse-over during a golden
            // capture can't turn the camera). Window close/escape and the debug
            // server's /input still work.
            let ignore_user_input = clock.is_pinned();
            // Keyboard events for a focused `Ui.textInput` this frame,
            // collected from the GLFW stream while the overlay wants the
            // keyboard and handed to `draw_view` below. While the clock is
            // pinned nothing is collected — typing is inert like all other
            // window input.
            let mut ui_keyboard: Vec<functor_runtime_common::ui::UiKeyboardEvent> = Vec::new();
            for (_, event) in glfw::flush_messages(&events) {
                match event {
                    glfw::WindowEvent::Close => window.set_should_close(true),
                    // Printable text for a focused field. Only meaningful
                    // while the overlay wants the keyboard; otherwise Char
                    // events are dropped (the game hears keys, not text).
                    glfw::WindowEvent::Char(c)
                        if ui_wants_keyboard && !ignore_user_input =>
                    {
                        ui_keyboard
                            .push(functor_runtime_common::ui::UiKeyboardEvent::Char(c));
                    }
                    // While a field is focused, Escape DEFOCUSES it (egui's
                    // built-in) instead of releasing the cursor — the next
                    // Escape then falls through to the arm below.
                    glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _)
                        if ui_wants_keyboard && !ignore_user_input =>
                    {
                        ui_keyboard.push(functor_runtime_common::ui::UiKeyboardEvent::Edit(
                            functor_runtime_common::ui::UiEditKey::Escape,
                        ));
                    }
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
                    // `~` toggles the time-travel console. Opening it frees the
                    // cursor (so you can scrub immediately); closing returns to
                    // free-look. Before the `ignore_user_input` catch-all so it
                    // works while the clock is pinned (paused). Never grabs the
                    // pointer on a hidden window. Guarded off while a text
                    // field is focused — typing '`' must insert, not toggle
                    // (the char arrives via the Char arm above).
                    glfw::WindowEvent::Key(Key::GraveAccent, _, Action::Press, _)
                        if !ui_wants_keyboard =>
                    {
                        scrubber_visible = !scrubber_visible;
                        if !hidden {
                            if scrubber_visible {
                                window.set_cursor_mode(glfw::CursorMode::Normal);
                                cursor_captured = false;
                            } else {
                                window.set_cursor_mode(glfw::CursorMode::Disabled);
                                cursor_captured = true;
                            }
                        }
                    }
                    // Left click while released: if it lands on a scrubber
                    // control (egui wanted the pointer last frame) it drives the
                    // scrubber; otherwise it recaptures for free-look. Press/
                    // release edges feed egui's click detection. Never on a
                    // hidden window — it must not grab the pointer. These arms
                    // sit BEFORE the `ignore_user_input` catch-all so the
                    // scrubber stays usable while the clock is pinned (paused).
                    glfw::WindowEvent::MouseButton(glfw::MouseButtonLeft, action, _)
                        if !cursor_captured && !hidden =>
                    {
                        match action {
                            Action::Press => {
                                // Any overlay wanting the pointer — the
                                // scrubber, the game UI's widgets, or the
                                // webview — means the click is for it, not a
                                // recapture. The webview is hit-tested at
                                // press time (mouse_pos is window points ==
                                // CSS px) against the render worker's latest
                                // interactive-rect snapshot: the one-frame
                                // wants_pointer latch reads the OLD tree
                                // after a model-driven re-render, and a
                                // stationary repeat-click would recapture
                                // the cursor.
                                if scrubber_wants_pointer
                                    || ui_wants_pointer
                                    || webview_overlay.hit_interactive_css(
                                        mouse_pos.0 as f32,
                                        mouse_pos.1 as f32,
                                    )
                                {
                                    mouse_primary_down = true;
                                    mouse_primary_clicked = true;
                                } else {
                                    window.set_cursor_mode(glfw::CursorMode::Disabled);
                                    cursor_captured = true;
                                }
                            }
                            Action::Release => mouse_primary_down = false,
                            Action::Repeat => {}
                        }
                    }
                    // Track the cursor for the scrubber while released (but never
                    // drive the camera — you're pointing at the overlay).
                    glfw::WindowEvent::CursorPos(x, y) if !cursor_captured => {
                        mouse_pos = (x as i32, y as i32);
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
                    // Always honor the SHELL bookkeeping for key releases and
                    // focus-loss, even while other input is ignored (pinned clock)
                    // — otherwise a key held at the pin transition, or released
                    // after alt-tab, would stick in held_keys forever. But while
                    // pinned we must NOT deliver the release to the game: the
                    // input log only records/replays what reaches the model, and a
                    // paused frame drops its input buffer, so an unlogged release
                    // would diverge forward-step replay. Gate the game delivery on
                    // !ignore_user_input while keeping held_keys/cursor bookkeeping.
                    glfw::WindowEvent::Key(key, _, Action::Release, _) => {
                        let k = map_key(key);
                        // Deliver the release only if the game SAW the press
                        // (it's in held_keys) — a press swallowed by a focused
                        // text field must not leak a phantom release edge to
                        // the game's `input` hook (xreview). Unknown keys are
                        // never tracked; keep their pre-existing passthrough,
                        // but not while a field is focused.
                        let was_held = held_keys.remove(&k);
                        if !ignore_user_input
                            && (was_held || (k == InputKey::Unknown && !ui_wants_keyboard))
                        {
                            game.key_event(k as i32, false);
                        }
                    }
                    glfw::WindowEvent::Focus(false) => {
                        for k in std::mem::take(&mut held_keys) {
                            if !ignore_user_input {
                                game.key_event(k as i32, false);
                            }
                        }
                        // Hand the pointer back when the window loses focus
                        // (cmd-tab to the editor); a click recaptures.
                        window.set_cursor_mode(glfw::CursorMode::Normal);
                        cursor_captured = false;
                        // A button held at focus-loss may never get its release
                        // (alt-tab), which would leave egui holding a stuck
                        // press — clear it so the scrubber stays live. [xreview]
                        mouse_primary_down = false;
                        mouse_primary_clicked = false;
                    }
                    _ if ignore_user_input => {}
                    // While a field is focused, key presses drive the OVERLAY:
                    // the editing subset maps across (press+release pairs are
                    // synthesized by the lowering, so Repeat works), printable
                    // keys arrive via the Char arm, and the game's `input`
                    // hook hears none of it (the focus gate). Releases still
                    // reach the game below — a key held from before focus
                    // must not stick.
                    glfw::WindowEvent::Key(key, _, Action::Press | Action::Repeat, _)
                        if ui_wants_keyboard =>
                    {
                        use functor_runtime_common::ui::{UiEditKey, UiKeyboardEvent};
                        let edit = match key {
                            Key::Backspace => Some(UiEditKey::Backspace),
                            Key::Delete => Some(UiEditKey::Delete),
                            Key::Left => Some(UiEditKey::Left),
                            Key::Right => Some(UiEditKey::Right),
                            Key::Home => Some(UiEditKey::Home),
                            Key::End => Some(UiEditKey::End),
                            Key::Enter => Some(UiEditKey::Enter),
                            _ => None,
                        };
                        if let Some(edit) = edit {
                            ui_keyboard.push(UiKeyboardEvent::Edit(edit));
                        }
                    }
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

            // Run this render frame's fixed model steps (0..N). Scripted input
            // (docs/time-travel.md T6b) is fed per fixed frame through the SAME
            // `key_event` path the live GLFW handlers use, before that frame's
            // tick. Under --ghost the script drives the GHOST instead (F2): leave
            // the live game at its init anchor so the strobed arc reads against a
            // clean, static pose. `--capture-at-frame` fires on the fixed frame it
            // names — scripted playback is exactly one fixed step per render frame,
            // so that index stays deterministic.
            let mut capture_due = false;
            for sub in &sub_frames {
                if let Some(script) = input_script.as_ref().filter(|_| !args.ghost) {
                    if let Some(events) = script.get(&frame_count) {
                        for ev in events {
                            if let RecordedInput::Key { code, is_down } = ev {
                                game.key_event(*code, *is_down);
                            }
                        }
                    }
                }
                game.tick(sub.clone());
                if args.capture_at_frame == Some(frame_count) {
                    capture_due = true;
                }
                frame_count += 1;
            }

            // Perform the HTTP + WebSocket commands this frame's tick queued
            // (shared with the headless loop).
            dispatch_net_ws(&mut *game, &net_tx, &http_client, &mut ws_manager);

            // Follow window resizes: query the drawable size each frame.
            // Framebuffer size is in pixels, so this handles HiDPI/retina.
            let (fb_width, fb_height) = window.get_framebuffer_size();
            let viewport = functor_runtime_common::Viewport::new(fb_width as u32, fb_height as u32);

            // The game supplies the camera/scene/lights as part of its frame.
            // Kept PRISTINE: the debug server's GET /scene serves this frame, and
            // it must reflect exactly what the game's `draw` produced — the
            // trajectory overlay goes on a render-only copy below.
            let frame = game.render(time.clone());

            // Drive the ghost from the interactive toggle when the overlay is up,
            // and from the `--ghost` launch flag when it's hidden (the headless
            // capture path — F2 demo, goldens, tests — is byte-for-byte
            // unchanged). Computed HERE, above the scene-diff preview, because
            // the preview's gating reads it.
            // One wanted-set drives every future preview (docs/time-travel.md
            // T6/T6d): the scene-diff trail/strobe overlays AND the
            // screen-space ghost compositor. With the overlay up, the
            // scrubber's selector picks ONE mode. Its anchor follows the live
            // tail while playing and freezes when paused. With the overlay
            // hidden the launch flags drive it — several may combine
            // there (--ghost --trajectory composes the trail into the strobe) —
            // so headless captures are byte-for-byte unchanged.
            let (trail_wanted, strobe_wanted, ghost_active) = if scrubber_visible {
                let selected = functor_runtime_common::interactive_preview(
                    preview_mode,
                    extrapolate_on,
                    clock.pending_frames() > 0,
                );
                (selected.trail, selected.strobe, selected.ghost)
            } else {
                (args.trajectory, args.strobe, args.ghost)
            };
            // Under the screen-space ghost compositor the scene-space strobe
            // would double-ghost — and the compositor arm never draws the
            // display frame — so while the ghost is active the strobe is off
            // (the trail still shows: it rides IN the composited frames), and
            // it must not silently burn a forward-sim it can't display.
            let strobe_wanted = strobe_wanted && !ghost_active;
            // Forward window/densities. The SIM samples fine (~20/s — the
            // trail's smooth-arc rate) while the ⚙ rate governs STROBE COPIES
            // per second, so dots stay visible between copies and both hold
            // their density as the window resizes. The flag path keeps its
            // historical constants (32 samples over 1.6s, 8 copies), keeping
            // captures byte-identical.
            const TRAIL_RATE: f32 = 20.0;
            let (preview_divisions, preview_window_s, strobe_copies) = if scrubber_visible {
                let divisions =
                    ((TRAIL_RATE * preview_window).round() as usize).clamp(1, 64);
                let copies = ((preview_rate as f32 * preview_window).round() as usize)
                    .clamp(1, divisions);
                (divisions, preview_window, copies)
            } else {
                (32usize, 1.6f32, 8usize)
            };
            // While a drag-into-the-future catch-up is draining, skip the
            // preview recompute (the anchor moves every frame, so it would be
            // a full forward-sim per frame and throttle the catch-up to a
            // crawl) — it snaps back in on arrival.
            let preview_active = (trail_wanted || strobe_wanted)
                && args.composite_demo == CompositeDemoArg::Off
                && clock.pending_frames() == 0;
            let preview: Option<functor_runtime_common::ScenePreview> = if preview_active {
                // Not bound by the 8-target compositor cap — this only reads node
                // transforms — so sample finely for a smooth arc.
                let divisions = preview_divisions;
                let window = preview_window_s;
                let dt = window / divisions as f32;
                let key = (
                    game.current_scene_frame(),
                    time.tts.to_bits(),
                    clock.is_paused(),
                    trail_wanted,
                    strobe_wanted,
                    divisions,
                    window.to_bits(),
                );
                let interactive_live = scrubber_visible && !clock.is_paused();
                let cache_hit = trail_cache.as_ref().is_some_and(|(k, _)| {
                    if interactive_live {
                        elapsed_time < next_live_trail_refresh
                            && trail_refresh == 0
                            && !k.2
                            && k.3 == key.3
                            && k.4 == key.4
                            && k.5 == key.5
                            && k.6 == key.6
                    } else {
                        trail_refresh > 0 && *k == key
                    }
                });
                if cache_hit {
                    if !interactive_live {
                        trail_refresh -= 1;
                    }
                    trail_cache.as_ref().map(|(_, p)| p.clone())
                } else {
                    // Under --input-script the live loop keeps consuming the
                    // script (its gate is `!args.ghost`), so the forward-sim must
                    // replay the SAME upcoming slice — with `None` it would
                    // replay the recorder's (empty-at-head) log and predict an
                    // input-free future the game won't play. The slice anchors at
                    // the fine step after the newest recorded frame, mirroring
                    // the `inputs_from(k + 1)` anchor of the recorded-log path.
                    let script_slice: Option<Vec<Vec<RecordedInput>>> =
                        input_script.as_ref().map(|script| {
                            let sub_dt = 1.0f32 / 60.0;
                            let steps_per_division =
                                ((dt / sub_dt).round() as usize).max(1);
                            let total = (divisions * steps_per_division) as u64;
                            // Under --ghost the live loop does NOT consume the
                            // script (the model holds at the init anchor while
                            // the recorder still advances), and the ghost slice
                            // below replays from frame 0 — anchor the trail the
                            // same way or the two projections diverge. Live
                            // playback consumes the script, so there the next
                            // unconsumed frame (k + 1) is the right anchor.
                            // Deliberately keyed on the LAUNCH FLAG, not the
                            // interactive mode: `--ghost --input-script` is the
                            // F2 session semantic (the script drives the
                            // anchored future, never the live game), so
                            // selecting trail/strobe there previews the SAME
                            // scripted future — switching modes changes the
                            // visualization, not the script routing.
                            let base = if args.ghost {
                                0
                            } else {
                                game.current_scene_frame().map_or(0, |k| k + 1)
                            };
                            (0..total)
                                .map(|j| script.get(&(base + j)).cloned().unwrap_or_default())
                                .collect()
                        });
                    let p = functor_runtime_common::scene_preview(
                        &*game,
                        &frame.scene,
                        time.tts as f64,
                        script_slice.as_deref(),
                        &functor_runtime_common::PreviewOptions {
                            divisions,
                            window,
                            // eps 0.04: ignore sub-mm jitter. max_step 3.0: cut
                            // respawn teleports (a reset covers many units in
                            // one sample).
                            eps: 0.04,
                            max_step: 3.0,
                            trail: trail_wanted,
                            strobe: strobe_wanted.then(|| functor_runtime_common::StrobeOptions {
                                copies: strobe_copies,
                                ..Default::default()
                            }),
                        },
                    );
                    if interactive_live {
                        trail_refresh = 0;
                        next_live_trail_refresh =
                            start_time.elapsed().as_secs_f32() + LIVE_PREVIEW_INTERVAL_SECONDS;
                    } else {
                        trail_refresh = PAUSED_PREVIEW_REUSE_FRAMES;
                    }
                    trail_cache = Some((key, p.clone()));
                    Some(p)
                }
            } else {
                trail_cache = None;
                next_live_trail_refresh = 0.0;
                None
            };

            // Forward-ghosting (docs/time-travel.md T6d): when --ghost is on, step
            // the scene forward over a ~2s window and collect a Frame per division
            // (a dry run — the live producer is untouched), for the compositor arm
            // in the render ladder below. Empty when --ghost is off (or the
            // producer has no model history, e.g. web's trait default). `start_tts
            // = time.tts` (under --fixed-time T, that's T, so the ghost projects
            // T+dt … T+N·dt). Always composites when on — no pause gating — so it's
            // deterministically capturable under --fixed-time.
            // Gate on the full ladder condition so we don't pay the dry-run + N
            // draws only to have stereo / composite-demo outrank the ghost arm.
            // (`ghost_active` is computed above the scene-diff preview block.)
            let ghost_frames = if ghost_active
                && !args.stereo_sbs
                && args.composite_demo == CompositeDemoArg::Off
                // Skipped during a catch-up drain, like the scene-diff preview.
                && clock.pending_frames() == 0
            {
                const MAX_GHOST: usize = 8;
                // Interactive: the ⚙ popover's rate × window (clamped to the
                // compositor's 8-target cap). Flag path: the historical
                // --ghost-divisions over a 2s window, byte-identical captures.
                let (divisions, ghost_window) = if scrubber_visible {
                    (
                        ((preview_rate as f32 * preview_window).round() as usize)
                            .clamp(1, MAX_GHOST),
                        preview_window,
                    )
                } else {
                    (args.ghost_divisions.clamp(1, MAX_GHOST), 2.0f32)
                };
                let dt = ghost_window / divisions as f32;
                // F2 (docs/time-travel.md): when an --input-script is loaded, the
                // ghost previews the SCRIPT's trajectory from the live anchor
                // (mario at init) rather than the recorder's own input log. Build a
                // per-fine-step slice covering the ghost window (frame f → the
                // script's events at f, empty when it has none). The fine step is
                // 1/60 and script frames advance by 1/60, so the frame index maps
                // straight to the fine-step index. `None` keeps the T6d behavior
                // (e.g. the lighting ghost, which has no script).
                let script_slice: Option<Vec<Vec<RecordedInput>>> =
                    input_script.as_ref().map(|script| {
                        let sub_dt = 1.0f32 / 60.0;
                        let steps_per_division = ((dt / sub_dt).round() as usize).max(1);
                        let total = (divisions * steps_per_division) as u64;
                        (0..total)
                            .map(|f| script.get(&f).cloned().unwrap_or_default())
                            .collect()
                    });
                if scrubber_visible {
                    let key = (
                        game.current_scene_frame(),
                        time.tts.to_bits(),
                        clock.is_paused(),
                        divisions,
                        ghost_window.to_bits(),
                    );
                    let cache_hit = ghost_cache.as_ref().is_some_and(|(k, _)| {
                        if clock.is_paused() {
                            ghost_refresh > 0 && *k == key
                        } else {
                            elapsed_time < next_live_ghost_refresh
                                && !k.2
                                && k.3 == key.3
                                && k.4 == key.4
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
                        let frames = game.ghost_frames(
                            divisions,
                            dt,
                            time.tts as f64,
                            script_slice.as_deref(),
                        );
                        if clock.is_paused() {
                            ghost_refresh = PAUSED_PREVIEW_REUSE_FRAMES;
                        } else {
                            ghost_refresh = 0;
                            next_live_ghost_refresh =
                                start_time.elapsed().as_secs_f32() + LIVE_PREVIEW_INTERVAL_SECONDS;
                        }
                        ghost_cache = Some((key, frames.clone()));
                        frames
                    }
                } else {
                    ghost_cache = None;
                    next_live_ghost_refresh = 0.0;
                    game.ghost_frames(divisions, dt, time.tts as f64, script_slice.as_deref())
                }
            } else {
                ghost_cache = None;
                next_live_ghost_refresh = 0.0;
                Vec::new()
            };
            // The trail composes with --ghost's screen-space strobe by riding IN
            // each composited frame: the dots are identical opaque geometry at
            // identical world positions in every input, so the equal-weight
            // average reconstructs them at full intensity (unlike movers, which
            // appear in only one frame each). Without this the ghost render arm
            // draws only `ghost_frames` and the trail would silently vanish.
            // The scene-space strobe is deliberately NOT folded in — layering
            // geometry copies under the compositor's strobe would double-ghost.
            let mut ghost_frames = ghost_frames;
            if let Some(trail) = preview.as_ref().and_then(|p| p.trail.as_ref()) {
                for (f, _) in ghost_frames.iter_mut() {
                    functor_runtime_common::overlay(&mut f.scene, trail.clone());
                }
            }

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

            // Physics wireframe overlay (--debug-render physics): collect the
            // live world's lines once per frame, drawn over each rendered view
            // below. Read-only; an empty/no-physics world yields no lines.
            let physics_debug_lines = matches!(args.debug_render, DebugRenderArg::Physics)
                .then(|| {
                    functor_runtime_common::physics::with_world(
                        functor_runtime_common::physics::DEFAULT_WORLD,
                        |w| w.debug_lines(),
                    )
                })
                .flatten();

            // The frame the render ladder draws: the game frame plus the
            // preview overlays (`frame` itself stays pristine for GET /scene).
            // Skipped when the ghost arm will draw instead — the trail already
            // rides inside `ghost_frames`.
            let display_frame = match (&preview, ghost_frames.is_empty()) {
                (Some(p), true) if p.trail.is_some() || p.strobe.is_some() => {
                    let mut f = frame.clone();
                    if let Some(t) = &p.trail {
                        functor_runtime_common::overlay(&mut f.scene, t.clone());
                    }
                    if let Some(s) = &p.strobe {
                        functor_runtime_common::overlay(&mut f.scene, s.clone());
                    }
                    Some(f)
                }
                _ => None,
            };
            let drawn_frame = display_frame.as_ref().unwrap_or(&frame);

            // Shadow + forward passes, shared with the web runtime. In stereo
            // mode, render the same frame twice — once per eye camera into each
            // half of the window. (The shadow pass runs per eye; it must start
            // unscissored, so scissor is reset before each call — same contract
            // as the netsim viewer's panes.)
            // Time the whole scene render (all arms) for FrameStats.render_us —
            // the GL-submission cost, distinct from the interpreter's draw cost.
            let render_started = Instant::now();
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
                        drawn_frame,
                        camera,
                        time.clone(),
                        *eye_viewport,
                        args.debug_render.into(),
                    );
                    if let Some(lines) = &physics_debug_lines {
                        functor_runtime_common::render_debug_lines(
                            &gl,
                            shader_version,
                            camera,
                            *eye_viewport,
                            lines,
                        );
                    }
                }
            } else if args.composite_demo != CompositeDemoArg::Off {
                // T5 compositor smoke test: render two frames and average them.
                // `clone` overlays the frame with itself (lossless check);
                // `fork` overlays it with a world-space-offset copy of its scene
                // (the fork+overlay demo — two visibly-different inputs).
                let mut frame_b = frame.clone();
                if args.composite_demo == CompositeDemoArg::Fork {
                    frame_b.scene = functor_runtime_common::Scene3D {
                        obj: functor_runtime_common::SceneObject::Group(vec![frame.scene.clone()]),
                        xform: cgmath::Matrix4::from_translation(cgmath::vec3(6.0, 0.0, 0.0)),
                    };
                }
                let frames = [
                    (frame.clone(), time.clone()),
                    (frame_b, time.clone()),
                ];
                functor_runtime_common::render_composited_frames(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &frames,
                    &[0.5, 0.5],
                    viewport,
                    args.debug_render.into(),
                );
            } else if !ghost_frames.is_empty() {
                // Forward-ghosting (docs/time-travel.md T6d): composite the ~2s
                // forward-stepped frames (computed above) at equal weight so moving
                // elements strobe across their future and static geometry stays
                // solid. Each frame renders at ITS OWN division-boundary time, so
                // render-time animation (the skinned pose) advances through the
                // strobe. Empty (→ this arm skipped) when --ghost is off or the
                // producer yields no frames, so live rendering is unchanged.
                let weights = vec![1.0f32; ghost_frames.len()];
                functor_runtime_common::render_composited_frames(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &ghost_frames,
                    &weights,
                    viewport,
                    args.debug_render.into(),
                );
            } else {
                functor_runtime_common::render_frame(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    drawn_frame,
                    &view_camera,
                    time.clone(),
                    viewport,
                    args.debug_render.into(),
                );
                if let Some(lines) = &physics_debug_lines {
                    functor_runtime_common::render_debug_lines(
                        &gl,
                        shader_version,
                        &view_camera,
                        viewport,
                        lines,
                    );
                }
            }
            let render_ns = render_started.elapsed().as_nanos() as u64;

            // The pointer, in framebuffer pixels, for BOTH interactive egui
            // passes (the game UI and the scrubber). The cursor (GLFW window/
            // logical coords) must be scaled to framebuffer pixels to line up
            // with egui's layout space (which we drive in physical pixels,
            // ppp = 1.0) — otherwise widgets are unclickable on a HiDPI/
            // retina display (fb = 2× window). [xreview] `pos` is None while
            // the cursor is captured for free-look (you're not pointing at
            // the overlay).
            let (win_w, _) = window.get_size();
            let dpi_scale = if win_w > 0 {
                fb_width as f32 / win_w as f32
            } else {
                1.0
            };
            let pointer = functor_runtime_common::ui::PointerState {
                pos: (!cursor_captured && !hidden).then_some((
                    mouse_pos.0 as f32 * dpi_scale,
                    mouse_pos.1 as f32 * dpi_scale,
                )),
                // `|| clicked`: a press+release inside one poll batch still
                // samples as down this frame (see the latch's declaration).
                primary_down: mouse_primary_down || mouse_primary_clicked,
            };
            mouse_primary_clicked = false;

            // 2D UI overlay: the game's declarative `ui model` View, lowered to a
            // text overlay on top of the 3D frame. Drawn before capture so it
            // appears in --capture-frame PNGs. Widget interactions come back
            // slot-stamped and fold through the game's `update` — except while
            // the clock is pinned, when window input must not reach the model
            // (the `ignore_user_input` rule; injected `/input` still applies).
            // The scrubber draws ON TOP of the game UI, so while it wants the
            // pointer the game UI must not see it — otherwise one click over
            // an overlapping region fires a game button AND a scrubber
            // control (each pass is its own egui context; there is no shared
            // hit-test). `pos: None` makes the game UI's bridge release any
            // held press and clear hover. Same while the clock is pinned:
            // the events would be dropped anyway (the window-input rule), so
            // don't let egui process the interaction at all — a paused
            // button must not visually depress, and a paused slider drag
            // must not fight its own reconciliation. [xreview]
            let overlay_suppressed =
                (scrubber_visible && scrubber_wants_pointer) || ignore_user_input;
            let suppressed_pointer = functor_runtime_common::ui::PointerState {
                pos: None,
                primary_down: pointer.primary_down,
            };
            // The webview draws ABOVE the game UI, so while the pointer is
            // over one of its interactive elements the egui pass must not
            // also hit-test the click (the scrubber-over-ui rule) — one click
            // must never fire a `Ui.button` AND a webview button. The webview
            // itself is gated only by the scrubber/pin (never by its own
            // latch, which would oscillate). [xreview]
            let webview_pointer = if overlay_suppressed {
                suppressed_pointer
            } else {
                pointer
            };
            let ui_pointer = if overlay_suppressed || webview_wants_pointer {
                suppressed_pointer
            } else {
                pointer
            };
            let ui_view = game.ui();
            let ui_out = text_overlay.draw_view(
                fb_width as u32,
                fb_height as u32,
                1.0,
                ui_pointer,
                &ui_keyboard,
                &ui_view,
            );
            ui_wants_pointer = ui_out.wants_pointer;
            ui_wants_keyboard = ui_out.wants_keyboard;
            if !ignore_user_input {
                for event in ui_out.events {
                    game.ui_event(event);
                }
            }

            // The HTML/CSS webview overlay (`webview model`), blitz-rendered
            // over the game UI and under the scrubber. Same pointer
            // arbitration as the game UI (`ui_pointer`): suppressed while the
            // scrubber wants the pointer or the clock is pinned.
            // TODO(webview): `webview()` clones the tree and `to_html`
            // reserializes every frame — cache the serialized string in the
            // producer once the protocol shape settles (perf follow-up).
            let webview_html = game.webview().map(|node| node.to_html());
            // `time.tts` as the CSS animation clock: `--fixed-time` pins it
            // (deterministic captures) and pausing freezes overlay animations
            // coherently with the game.
            let webview_out = webview_overlay.frame(
                fb_width as u32,
                fb_height as u32,
                dpi_scale,
                webview_html.as_deref(),
                webview_pointer,
                time.tts as f64,
            );
            webview_wants_pointer = webview_out.wants_pointer;
            if !ignore_user_input {
                for event in webview_out.events {
                    game.webview_event(event);
                }
            }

            // The shell-owned time-travel scrubber (docs/time-travel.md T3),
            // drawn over the frame (so it shows in captures) and interactive
            // when the cursor is released.
            let scrubber_out = if scrubber_visible {
                scrubber.draw(
                    fb_width as u32,
                    fb_height as u32,
                    1.0,
                    pointer,
                    functor_runtime_common::ui::ScrubberState {
                        frame: game.current_scene_frame().unwrap_or(0),
                        range: game.scene_frame_range(),
                        paused: clock.is_paused(),
                        extrapolate: extrapolate_on,
                        preview_mode,
                        preview_window,
                        preview_rate,
                    },
                )
            } else {
                functor_runtime_common::ui::ScrubberOutput {
                    action: None,
                    wants_pointer: false,
                }
            };
            scrubber_wants_pointer = scrubber_out.wants_pointer;
            match scrubber_out.action {
                Some(functor_runtime_common::ui::ScrubberAction::TogglePause) => {
                    // Resuming: rebase to the scene's current time so play
                    // continues from there, not wall-clock. When scrubbed this is
                    // the scrubbed frame's recorded `tts`; on a plain pause/resume
                    // it's the newest recorded frame's `tts`, which already equals
                    // the frozen `game_time` — a harmless no-op.
                    if clock.is_paused() {
                        match game.current_scene_tts() {
                            Some(tts) => {
                                if args.debug_clock {
                                    eprintln!("[clock] resume: rebase tts={tts:.3}");
                                }
                                clock.rebase(tts as f32);
                            }
                            None if args.debug_clock => {
                                eprintln!(
                                    "[clock] resume: no scene tts — NOT rebased (game_time held)"
                                );
                            }
                            None => {}
                        }
                    } else if args.debug_clock {
                        eprintln!("[clock] pause");
                    }
                    clock.toggle_pause();
                }
                Some(functor_runtime_common::ui::ScrubberAction::SeekTo(f)) => {
                    let newest = game.scene_frame_range().map(|(_, h)| h);
                    match newest {
                        Some(h) if f > h => {
                            // Dragged INTO the rail's cyan future segment:
                            // pass through the recorded end, then step the
                            // game forward input-free — the clock animates
                            // the catch-up (≤8 fixed frames per rendered
                            // frame). Stepping from the newest frame commits
                            // nothing away, so this is branch-safe.
                            if args.debug_clock {
                                eprintln!(
                                    "[clock] seek frame={f}: {} beyond newest {h} — stepping forward",
                                    f - h
                                );
                            }
                            if let Err(e) = game.seek_scene_to(h) {
                                eprintln!("[scrubber] {e}");
                            }
                            if let Some(tts) = game.current_scene_tts() {
                                clock.rebase(tts as f32);
                            }
                            clock.step_frames((f - h) as u32);
                        }
                        _ => {
                            // Dragging the recorded window: non-destructive
                            // seek, and park the clock on the scrubbed frame
                            // (resuming from there is what commits the
                            // branch). Keep the clock's time aligned to the
                            // scrubbed frame so a resume continues from it.
                            match game.seek_scene_to(f) {
                                Ok(_) => {}
                                Err(e) => eprintln!("[scrubber] {e}"),
                            }
                            if let Some(tts) = game.current_scene_tts() {
                                if args.debug_clock {
                                    eprintln!("[clock] seek frame={f}: rebase tts={tts:.3}");
                                }
                                clock.rebase(tts as f32);
                            }
                            clock.pause();
                        }
                    }
                }
                Some(functor_runtime_common::ui::ScrubberAction::Step) => {
                    // Step implies pause: advance exactly one frame, then hold.
                    clock.step(1.0 / 60.0);
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetExtrapolate(on)) => {
                    if args.debug_clock {
                        eprintln!("[clock] extrapolate {}", if on { "on" } else { "off" });
                    }
                    extrapolate_on = on;
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetPreviewMode(m)) => {
                    if args.debug_clock {
                        eprintln!("[clock] preview {}", m.label());
                    }
                    preview_mode = m;
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetPreviewWindow(w)) => {
                    preview_window = w.clamp(0.5, 5.0);
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetPreviewRate(n)) => {
                    preview_rate = n.clamp(1, 30);
                }
                None => {}
            }

            if let Some(capture_path) = &args.capture_frame {
                // Shoot at an exact deterministic sim frame (--capture-at-frame,
                // for scripted runs) or after --capture-time wall-clock seconds
                // (the default, so assets have time to load).
                let shoot = match args.capture_at_frame {
                    Some(_) => capture_due,
                    None => elapsed_time >= args.capture_time,
                };
                if shoot {
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
                        &mut clock,
                        // GL readback on the render thread (a real Failed on error).
                        &|| {
                            encode_framebuffer_png(&gl, fb_width as u32, fb_height as u32)
                                .map_err(debug_server::CaptureError::Failed)
                        },
                    );
                }
            }

            // Time the buffer swap separately: it blocks on vsync, so
            // FrameStats.swap_us captures presentation/vsync cost, not GL work.
            let swap_started = Instant::now();
            window.swap_buffers();
            let swap_ns = swap_started.elapsed().as_nanos() as u64;

            // Hand this frame's shell-measured GL cost to the producer, which
            // folds it into the same rolling window it averages tick/draw over.
            game.record_gl_timing(render_ns, swap_ns);
        }
    }

    game.quit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::File::create(&path)
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
        path
    }

    #[test]
    fn parse_input_script_maps_frames_to_events() {
        let path = write_tmp(
            "functor-test-jump.script",
            "# comment\n\n0  Right down   # hold right\n18 Up down\n18 A down\n71 Right up\n",
        );
        let map = parse_input_script(path.to_str().unwrap()).unwrap();
        assert_eq!(map[&0].len(), 1);
        assert!(matches!(
            map[&0][0],
            RecordedInput::Key { code, is_down: true } if code == InputKey::Right as i32
        ));
        // Two events on the same frame accumulate in order.
        assert_eq!(map[&18].len(), 2);
        assert!(matches!(
            map[&18][0],
            RecordedInput::Key { code, is_down: true } if code == InputKey::Up as i32
        ));
        assert!(matches!(map[&71][0], RecordedInput::Key { is_down: false, .. }));
    }

    #[test]
    fn parse_input_script_rejects_malformed_lines() {
        let bad = write_tmp("functor-test-bad.script", "0 Right\n");
        assert!(parse_input_script(bad.to_str().unwrap()).is_err());
        let bad_key = write_tmp("functor-test-badkey.script", "0 Nope down\n");
        assert!(parse_input_script(bad_key.to_str().unwrap()).is_err());
        let bad_dir = write_tmp("functor-test-baddir.script", "0 Right sideways\n");
        assert!(parse_input_script(bad_dir.to_str().unwrap()).is_err());
    }
}
