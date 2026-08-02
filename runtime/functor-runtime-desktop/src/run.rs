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
use functor_runtime_common::viewer::{camera_frustum_lines, DebugCamera, DebugPresentation};
use functor_runtime_common::{
    Frame, FrameTime, GameClock, InputEdges, InputSnapshot, MouseButtons, MouseSnapshot,
    RecordedInput, SceneContext, XrInputSnapshot,
};
use std::collections::{BTreeSet, HashMap, HashSet};

use functor_runtime_common::Key as InputKey;
use glfw::{Action, Key};
use glow::*;

use crate::game::Game;
use crate::{
    asset_watch, audio, debug_server, functor_lang_game, net_dispatch, replay_game, ws_host, xreal,
};

/// Parse `WIDTHxHEIGHT` for `--capture-size`.
fn parse_capture_size(raw: &str) -> Result<(u32, u32), String> {
    let (w, h) = raw
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, got `{raw}`"))?;
    let parse = |value: &str, axis| {
        value
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|size| *size > 0)
            .ok_or_else(|| format!("{axis} must be a positive integer, got `{value}`"))
    };
    Ok((parse(w, "width")?, parse(h, "height")?))
}

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

/// Translate a GLFW mouse button into the canonical engine button code passed
/// across the game boundary. Buttons Functor does not model (4..8) become
/// `MouseButton::Unknown` and are never delivered.
fn map_mouse_button(button: glfw::MouseButton) -> functor_runtime_common::MouseButton {
    use functor_runtime_common::MouseButton as Btn;
    match button {
        glfw::MouseButtonLeft => Btn::Left,
        glfw::MouseButtonRight => Btn::Right,
        glfw::MouseButtonMiddle => Btn::Middle,
        _ => Btn::Unknown,
    }
}

fn sampled_input_snapshot(
    held_keys: &BTreeSet<InputKey>,
    mouse_pos: (i32, i32),
    surface: (i32, i32),
    held_buttons: MouseButtons,
    edges: &InputEdges,
    xr: Option<XrInputSnapshot>,
) -> InputSnapshot {
    let mut snapshot = InputSnapshot {
        held_keys: held_keys.iter().copied().collect(),
        mouse: MouseSnapshot {
            x: mouse_pos.0,
            y: mouse_pos.1,
            surface_width: surface.0.max(0) as u32,
            surface_height: surface.1.max(0) as u32,
            buttons: held_buttons,
            ..MouseSnapshot::default()
        },
        xr,
        ..InputSnapshot::default()
    };
    edges.apply_to(&mut snapshot);
    snapshot
}

/// Convert GLFW's fractional visible-cursor coordinates without snapping a
/// point just above/left of the window inside at zero.
fn visible_cursor_position(x: f64, y: f64) -> (i32, i32) {
    (x.floor() as i32, y.floor() as i32)
}

/// Build this step's sampled input.
///
/// `xr_override` is a debug-injected XR sample (`POST /input` `{"type":"xr"}`).
/// It WINS over the mouse/keyboard emulator, and supplies the `xr` domain even
/// without `--emulate-xr` — the point is scripting tracked poses that no desktop
/// device can express. It is level state, so it applies to every step until
/// replaced, exactly like `held_keys`.
fn desktop_input_snapshot(
    held_keys: &BTreeSet<InputKey>,
    mouse_pos: (i32, i32),
    held_buttons: MouseButtons,
    edges: &InputEdges,
    emulate_xr: bool,
    xr_primary_down: bool,
    xr_surface: (i32, i32),
    xr_override: Option<&XrInputSnapshot>,
) -> InputSnapshot {
    let xr = match xr_override {
        Some(injected) => Some(injected.clone()),
        None => emulate_xr.then(|| {
            crate::desktop_xr_emulator::sample(held_keys, mouse_pos, xr_primary_down, xr_surface)
        }),
    };
    sampled_input_snapshot(held_keys, mouse_pos, xr_surface, held_buttons, edges, xr)
}

/// Release every mouse button the game currently holds — the button twin of
/// the `held_keys` sweep on focus loss. A button held when the window loses
/// focus (or when the cursor is released, which stops delivering its events)
/// would otherwise never get its up edge, leaving a game firing forever.
/// `deliver` is false while the clock is pinned: the level state must still
/// clear, but an unlogged edge would diverge forward-step replay.
fn release_held_mouse_buttons(
    game: &mut dyn Game,
    held_buttons: &mut MouseButtons,
    edges: &mut InputEdges,
    deliver: bool,
) {
    // Driven off `ALL` rather than a literal list: a button added to the enum
    // must be swept too, or it becomes exactly the stuck-down bug this exists
    // to prevent.
    for button in functor_runtime_common::MouseButton::ALL
        .iter()
        .copied()
        .filter(|b| *b != functor_runtime_common::MouseButton::Unknown)
    {
        if held_buttons.is_down(button) {
            held_buttons.set(button, false);
            if deliver {
                edges.mouse_transition(button, true, false);
                game.mouse_button(button as i32, false);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EscapeAction {
    ReleaseCursor,
    ArmQuit,
    Quit,
}

fn escape_action(cursor_captured: bool, quit_armed: bool) -> EscapeAction {
    if cursor_captured {
        EscapeAction::ReleaseCursor
    } else if quit_armed {
        EscapeAction::Quit
    } else {
        EscapeAction::ArmQuit
    }
}

fn visible_pointer_game_input_enabled(
    visible_pointer: bool,
    debug_camera_detached: bool,
    ignore_user_input: bool,
) -> bool {
    visible_pointer && !debug_camera_detached && !ignore_user_input
}

fn route_visible_pointer_position(
    game_mouse_pos: &mut (i32, i32),
    pointer_pos: (i32, i32),
    visible_pointer: bool,
    debug_camera_detached: bool,
    ignore_user_input: bool,
) -> bool {
    if visible_pointer_game_input_enabled(
        visible_pointer,
        debug_camera_detached,
        ignore_user_input,
    ) {
        *game_mouse_pos = pointer_pos;
        true
    } else {
        false
    }
}

/// Apply one mouse-button edge with the held-state discipline every *injected*
/// path shares (`POST /input` and `--input-script`): a press enters the held
/// set and is delivered; a release is delivered only if the game saw the press,
/// so no phantom release edge can reach the game. Maintaining `held_buttons` is
/// also what makes the press visible to a later `sampledInput` as
/// `mouse.buttons.*`; actual level transitions also enter the one-step
/// `mouse.pressed` / `mouse.released` accumulator.
///
/// The GLFW window arm deliberately does NOT use this: it must clear the held
/// state even when it suppresses delivery (`ignore_user_input`), so a button
/// held across a pin transition can't stick.
fn apply_mouse_button_edge(
    game: &mut dyn Game,
    held_buttons: &mut MouseButtons,
    edges: &mut InputEdges,
    button: functor_runtime_common::MouseButton,
    is_down: bool,
) {
    if is_down {
        let was_down = held_buttons.is_down(button);
        held_buttons.set(button, true);
        edges.mouse_transition(button, was_down, true);
        game.mouse_button(button as i32, true);
    } else if held_buttons.is_down(button) {
        held_buttons.set(button, false);
        edges.mouse_transition(button, true, false);
        game.mouse_button(button as i32, false);
    }
}

/// Apply a physical window-button edge while preserving pinned/replay
/// determinism. A suppressed press never enters the held set; a release always
/// clears shell state. Only delivered transitions enter the fixed-step edge
/// accumulator.
fn apply_window_mouse_button_edge(
    game: &mut dyn Game,
    held_buttons: &mut MouseButtons,
    edges: &mut InputEdges,
    button: functor_runtime_common::MouseButton,
    is_down: bool,
    deliver: bool,
) {
    if is_down {
        if deliver {
            let was_down = held_buttons.is_down(button);
            held_buttons.set(button, true);
            edges.mouse_transition(button, was_down, true);
            game.mouse_button(button as i32, true);
        }
    } else {
        let was_held = held_buttons.is_down(button);
        held_buttons.set(button, false);
        if was_held && deliver {
            edges.mouse_transition(button, true, false);
            game.mouse_button(button as i32, false);
        }
    }
}

fn refresh_fixed_input_levels(
    snapshot: &mut InputSnapshot,
    held_keys: &BTreeSet<InputKey>,
    held_buttons: MouseButtons,
    xr_primary_down: bool,
    xr_override: Option<&XrInputSnapshot>,
) {
    snapshot.held_keys = held_keys.iter().copied().collect();
    snapshot.mouse.buttons = held_buttons;
    // An injected sample owns the whole XR domain — window keys must not
    // re-derive its trigger/thumbstick out from under the driver.
    match xr_override {
        Some(injected) => snapshot.xr = Some(injected.clone()),
        None => {
            if let Some(xr) = snapshot.xr.as_mut() {
                crate::desktop_xr_emulator::update_right_controls(
                    &mut xr.right,
                    held_keys,
                    xr_primary_down,
                );
            }
        }
    }
}

fn apply_scripted_events(
    game: &mut dyn Game,
    held_keys: &mut BTreeSet<InputKey>,
    held_buttons: &mut MouseButtons,
    edges: &mut InputEdges,
    events: &[RecordedInput],
) {
    for event in events {
        match event {
            RecordedInput::Key { code, is_down } => {
                game.key_event(*code, *is_down);
                if let Some(key) = InputKey::from_i32(*code) {
                    if *is_down {
                        let was_down = !held_keys.insert(key);
                        edges.key_transition(key, was_down, true);
                    } else {
                        let was_down = held_keys.remove(&key);
                        edges.key_transition(key, was_down, false);
                    }
                }
            }
            // Same edge + level discipline as an injected button — see
            // `apply_mouse_button_edge`. `parse_input_script` only ever emits
            // resolvable buttons, so `from_i32` cannot fail here.
            RecordedInput::MouseButton { button, is_down } => {
                if let Some(b) = functor_runtime_common::MouseButton::from_i32(*button) {
                    apply_mouse_button_edge(game, held_buttons, edges, b, *is_down);
                }
            }
            // Listed rather than `_`, so adding a scriptable line shape (e.g.
            // pointer motion) has to be wired here instead of compiling to a
            // silent no-op.
            RecordedInput::MouseMove { .. }
            | RecordedInput::MouseWheel { .. }
            | RecordedInput::Snapshot(_)
            | RecordedInput::UiEvent(_)
            | RecordedInput::WebviewEvent(_) => {}
        }
    }
}

/// Parse an `--input-script` file into a frame → events map for deterministic
/// scripted playback (docs/time-travel.md T6b). Each non-blank, non-comment
/// line is `<frame:int> <control> <down|up>` — e.g. `0 Right down`,
/// `18 Up down`, `4 Mouse.Left down`. `#` starts a comment (to end of line).
///
/// A `<control>` is either a KEY name, which goes through the same
/// [`InputKey::from_name`] map the debug server's POST /input uses, or a MOUSE
/// BUTTON written with an explicit `Mouse.` prefix (`Mouse.Left`,
/// `Mouse.Right`, `Mouse.Middle`, case-insensitive), resolved through
/// [`functor_runtime_common::MouseButton::from_name`].
///
/// The prefix is REQUIRED, and is why buttons are not spelled bare: `Left`,
/// `Right` and `Middle` are all valid *key* names too, so a bare button name
/// would silently script the arrow key instead. `Mouse.Left` is also exactly
/// the spelling a game's `mouseButton` hook receives
/// (`MouseButton::ctor_tag`), and no key name contains a `.`, so the two
/// namespaces cannot collide.
///
/// Events are stored as raw `RecordedInput::Key` / `RecordedInput::MouseButton`
/// so playback re-runs the identical live input path.
///
/// Pointer MOTION (`RecordedInput::MouseMove`) is not scriptable yet — it needs
/// a two-coordinate line shape rather than this `<control> <down|up>` triple.
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
                "{path}:{lineno}: expected `<frame> <Key|Mouse.Button> <down|up>`, got `{raw}`"
            ));
        }
        let frame: u64 = parts[0]
            .parse()
            .map_err(|_| format!("{path}:{lineno}: bad frame number `{}`", parts[0]))?;
        let is_down = match parts[2].to_ascii_lowercase().as_str() {
            "down" => true,
            "up" => false,
            other => {
                return Err(format!(
                    "{path}:{lineno}: expected `down|up`, got `{other}`"
                ))
            }
        };
        // A `Mouse.` prefix selects the button namespace. Without it the name
        // must be a key — `Left`/`Right`/`Middle` are key names too, so the
        // prefix is the only thing that disambiguates them.
        let event = match parts[1].split_once('.') {
            Some((module, button)) if module.eq_ignore_ascii_case("mouse") => {
                let button = functor_runtime_common::MouseButton::from_name(button).ok_or_else(
                    || {
                        format!(
                            "{path}:{lineno}: unknown mouse button `{}` \
                             (expected `Mouse.Left`, `Mouse.Right`, or `Mouse.Middle`)",
                            parts[1]
                        )
                    },
                )?;
                RecordedInput::MouseButton {
                    button: button as i32,
                    is_down,
                }
            }
            _ => {
                let key = InputKey::from_name(parts[1]).ok_or_else(|| {
                    format!(
                        "{path}:{lineno}: unknown key `{}` \
                         (mouse buttons need the `Mouse.` prefix, e.g. `Mouse.Left`)",
                        parts[1]
                    )
                })?;
                RecordedInput::Key {
                    code: key as i32,
                    is_down,
                }
            }
        };
        map.entry(frame).or_default().push(event);
    }
    reject_unmatched_button_releases(path, &map)?;
    Ok(map)
}

/// Reject a `Mouse.* up` with no preceding press.
///
/// This is a real inconsistency, not a style rule. Live playback applies the
/// held-state discipline (`apply_mouse_button_edge`) and SUPPRESSES such a
/// release, but the trail forward-step preview replays the script's
/// events unconditionally — so a malformed script would preview a future the
/// real run never plays. Failing the parse keeps the two paths honest by
/// construction, and a stray release is an authoring mistake either way.
fn reject_unmatched_button_releases(
    path: &str,
    map: &HashMap<u64, Vec<RecordedInput>>,
) -> Result<(), String> {
    let mut frames: Vec<u64> = map.keys().copied().collect();
    frames.sort_unstable();
    let mut held = MouseButtons::default();
    for frame in frames {
        for event in &map[&frame] {
            let RecordedInput::MouseButton { button, is_down } = event else {
                continue;
            };
            let Some(button) = functor_runtime_common::MouseButton::from_i32(*button) else {
                continue;
            };
            if !is_down && !held.is_down(button) {
                return Err(format!(
                    "{path}: frame {frame} releases `Mouse.{button:?}` that was never pressed"
                ));
            }
            held.set(button, *is_down);
        }
    }
    Ok(())
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
    /// Draw the authored scene translucently so diagnostic lines remain visible.
    Transparent,
    /// Normal shading plus the physics collider/contact wireframe overlay.
    Physics,
}

impl From<DebugRenderArg> for functor_runtime_common::DebugRenderMode {
    fn from(arg: DebugRenderArg) -> Self {
        match arg {
            DebugRenderArg::Default => functor_runtime_common::DebugRenderMode::Default,
            DebugRenderArg::Normals => functor_runtime_common::DebugRenderMode::Normals,
            DebugRenderArg::Tangents => functor_runtime_common::DebugRenderMode::Tangents,
            DebugRenderArg::Transparent => {
                functor_runtime_common::DebugRenderMode::Transparent
            }
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

/// How a project wants the desktop shell to expose the mouse.
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorPolicyArg {
    /// Hide/capture the cursor and deliver relative free-look motion.
    #[default]
    Captured,
    /// Keep the system cursor visible and deliver absolute window positions.
    Visible,
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

    /// Capture relative mouse input and route it to the Functor Lang hooks.
    /// The Functor CLI derives this from functor.json's `mouseCapture`.
    #[arg(long)]
    mouse_capture: bool,

    /// The role's entry-point prefix (same-file entries): resolve every
    /// canonical entry binding through it as camelCase (`server` →
    /// `serverInit`/`serverTick`/…). Empty = the classic unprefixed names.
    /// The CLI passes it for a functor.json role declared as
    /// `{ "file": …, "prefix": … }`.
    #[arg(long, default_value = "")]
    entry_prefix: String,

    /// The role's inline entry MODULE (same-file entries): resolve every
    /// canonical entry binding as a member of that `module` block
    /// (`Server` → `Server.init`/`Server.tick`/…). The CLI passes it for a
    /// functor.json role declared as `{ "file": …, "module": … }`; it is the
    /// preferred form, and mutually exclusive with `--entry-prefix`.
    #[arg(long, default_value = "", conflicts_with = "entry_prefix")]
    entry_module: String,

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

    /// Synthesize an XR head and two controllers from desktop input, using the
    /// same `Input.snapshot.xr` shape as Quest. The mouse moves the right
    /// controller; left-click/Space is trigger, Enter is squeeze, arrows are
    /// the thumbstick, and 1/2/3 are controller buttons. Intended for
    /// developing and capturing XR games without wearing a headset.
    #[arg(long)]
    emulate_xr: bool,

    /// Create the GL window hidden: it is never shown, never takes focus, and
    /// never captures the cursor, so a run doesn't steal input from the user.
    /// A hidden window keeps a valid GL context and framebuffer, so rendering,
    /// --capture-frame, and the debug server's /capture all work unchanged.
    /// Implied by --capture-frame. (With --headless there is no window at all.)
    #[arg(long, conflicts_with = "headless")]
    hidden: bool,

    /// Pointer policy selected by the project's `functor.json`.
    #[arg(long, value_enum, default_value_t = CursorPolicyArg::Captured)]
    cursor: CursorPolicyArg,

    /// Write a PNG of the rendered frame to this path, then exit. The capture
    /// happens on the first frame after --capture-time seconds of wall-clock
    /// time, so assets have a chance to load. Implies --hidden.
    #[arg(long)]
    capture_frame: Option<String>,

    /// Wall-clock seconds to run before --capture-frame takes the shot.
    #[arg(long, default_value_t = 2.0)]
    capture_time: f32,

    /// Render at exactly this framebuffer size, as `WIDTHxHEIGHT`.
    ///
    /// Without it the framebuffer follows the display's HiDPI scale, so the
    /// same scene captures at 800x600 on a 1x monitor and 1600x1200 on a
    /// Retina one — which silently invalidates every golden image when a
    /// machine is docked or undocked. Pinning the size makes a capture depend
    /// on the renderer instead of the monitor.
    #[arg(long, value_parser = parse_capture_size)]
    capture_size: Option<(u32, u32)>,

    /// Pin the game's frame time (seconds) to a constant so the rendered pose
    /// is deterministic — for reproducible captures / golden images.
    #[arg(long)]
    fixed_time: Option<f32>,

    /// Drive the game deterministically from a scripted input file instead of
    /// live window input (docs/time-travel.md T6b). Each line is
    /// `<frame:int> <control> <down|up>` — a KeyName like `Right`, `Up`, `A`,
    /// `Space`, or a mouse button with an explicit `Mouse.` prefix
    /// (`Mouse.Left`, `Mouse.Right`, `Mouse.Middle`; the prefix is required
    /// because `Left`/`Right`/`Middle` are also key names).
    /// `#` starts a comment. The sim advances by a FIXED `--script-dt`
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

    /// Treat a failed debug-server bind as non-fatal: log one line and run the
    /// game without the server. `functor develop` sets this for its default
    /// well-known port, so a second concurrent session (which finds the port
    /// taken) still runs. An explicitly requested port stays fatal.
    #[arg(long, requires = "debug_port")]
    debug_port_optional: bool,

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

    /// Scene-diff trajectory preview (docs/time-travel.md T6, scene-diff
    /// variant): forward-simulate the model and draw a clean trail of
    /// direction-of-travel arrowheads tracing ONLY the scene nodes that move
    /// — derived by the runtime from
    /// `draw`, with no game logic. Static geometry contributes nothing.
    /// Capturable under --fixed-time.
    #[arg(long)]
    trajectory: bool,

    /// Scene-space strobe preview: forward-simulate the model and overlay
    /// real-geometry copies of each MOVING scene node at its future poses,
    /// color-faded by age — full-intensity chronophotography on the normal
    /// render path. Like --trajectory it's runtime-derived from `draw`, shares
    /// one forward-sim with it, and is capturable under --fixed-time.
    #[arg(long)]
    strobe: bool,

    /// PIN the preview overlay's presence (0.0 = fully faded out, 1.0 = fully
    /// drawn), skipping the runtime's fade-in/out ease — the deterministic
    /// escape hatch for capturing a mid-fade frame. A NEGATIVE value (the
    /// default) means "no override": the runtime eases normally. The same
    /// convention holds on the web seam (`window.__scrub.setPreview({ presence })`,
    /// negative to resume easing).
    #[arg(long, default_value_t = -1.0)]
    preview_presence: f32,

    /// Start with the time-travel scrubber overlay visible (it is otherwise
    /// hidden until summoned with `~`). Useful for demos and captures of the
    /// scrubber itself. NOTE: with the overlay up, previews follow the
    /// interactive rule (paused only) — combine with --trajectory/--strobe
    /// WITHOUT --scrubber for headless overlay captures.
    #[arg(long)]
    scrubber: bool,

    /// Narrate the game clock + time-travel seams to stderr: pause/resume/seek
    /// rebases (with the tts they land on), preview mode changes, and
    /// per-frame HITCH warnings when a rendered frame's dt blows past 33ms (the
    /// tell-tale of a forward-sim starving the frame loop). High-signal
    /// diagnostics for the interactive scrubber — off by default.
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

/// Read back the framebuffer and write it as a PNG file. Exits the process with
/// an error if the capture cannot be written, so scripts don't mistake a failed
/// capture for a pass.
unsafe fn capture_framebuffer(gl: &glow::Context, width: u32, height: u32, path: &str) {
    let result = functor_runtime_common::frame_capture::encode_bound_framebuffer_png(
        gl, width, height,
    )
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
    frame_count: &mut u64,
    tts: f32,
    width: u32,
    height: u32,
    held_keys: &mut BTreeSet<InputKey>,
    mouse_pos: &mut (i32, i32),
    // Held mouse buttons, maintained from the GLFW stream and `POST /input`
    // the same way `held_keys` is.
    held_buttons: &mut MouseButtons,
    // Keyboard/mouse transitions pending the next fixed simulation step.
    edges: &mut InputEdges,
    // The debug-injected XR sample, if any: `{"type":"xr"}` sets it and every
    // later step samples it (see `desktop_input_snapshot`).
    xr_override: &mut Option<XrInputSnapshot>,
    state_input: InputSnapshot,
    emulate_xr: bool,
    clock: &mut GameClock,
    asset_cache: &Arc<AssetCache>,
    scene_context: &SceneContext,
    mut audio_player: Option<&mut audio::AudioPlayer>,
    // The webview keyboard gate, mirrored from the GLFW routing: `Some` iff a
    // webview text field is focused (`webview_wants_keyboard`), and injected
    // keys then type into it (queued into the next frame's WorkerInput)
    // instead of reaching the game. Always `None` in the headless loop, which
    // has no webview overlay.
    mut webview_keyboard: Option<&mut Vec<crate::webview_keys::WebviewKey>>,
    capture: &dyn Fn() -> Result<Vec<u8>, debug_server::CaptureError>,
) -> bool {
    let mut sampled_input_changed = false;
    match req {
        debug_server::DebugRequest::Capture(resp) => {
            let _ = resp.send(capture());
        }
        debug_server::DebugRequest::State(resp) => {
            let _ = resp.send(debug_server::RuntimeState {
                frame: *frame_count,
                tts,
                pending_steps: clock.pending_steps(),
                viewport: debug_server::RuntimeViewport::new(width, height),
                views: vec![debug_server::RuntimeView::new("main", width, height)],
                model: game.state_json(),
                model_debug: game.state_debug(),
                input: state_input,
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
        debug_server::DebugRequest::Project(resp) => {
            let _ = resp.send(game.project_sources());
        }
        debug_server::DebugRequest::ReloadSource(source, resp) => {
            let _ = resp.send(game.reload_source(&source));
        }
        debug_server::DebugRequest::ReloadProject(files, resp) => {
            let _ = resp.send(game.reload_project(&files));
        }
        debug_server::DebugRequest::LoadProject(files, resp) => {
            let result = game.load_project(&files);
            if result.is_ok() {
                clock.restart();
                *frame_count = 0;
            }
            let _ = resp.send(result);
        }
        debug_server::DebugRequest::ReloadAsset(asset, resp) => {
            let changed = asset_cache.replace_uploaded(&asset.path, asset.bytes);
            if changed {
                scene_context.evict_asset(&asset.path);
                if let Some(player) = audio_player.as_deref_mut() {
                    player.evict_asset(&asset.path);
                }
            }
            let status = if changed { "reloaded" } else { "unchanged" };
            let _ = resp.send(Ok(format!("{status} asset {}", asset.path)));
        }
        debug_server::DebugRequest::SyncAssets(paths, resp) => {
            let current: HashSet<String> = paths.into_iter().collect();
            let removed = asset_cache.retain_uploaded(&current);
            for path in &removed {
                scene_context.evict_asset(path);
                if let Some(player) = audio_player.as_deref_mut() {
                    player.evict_asset(path);
                }
            }
            let _ = resp.send(Ok(format!(
                "synced {} asset(s), removed {}",
                current.len(),
                removed.len()
            )));
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
            sampled_input_changed = matches!(
                &cmd,
                debug_server::InputCommand::Key { .. }
                    | debug_server::InputCommand::MouseMove { .. }
                    | debug_server::InputCommand::MouseButton { .. }
                    | debug_server::InputCommand::Xr(_)
                    | debug_server::InputCommand::XrClear
            );
            let result = match cmd {
                debug_server::InputCommand::Key { key, down } => {
                    // The focus gate first (the GLFW routing rule): while a
                    // webview field is focused, an injected PRESS types into
                    // it (or is swallowed if unmapped, like non-edit GLFW
                    // presses). Releases never route to the webview — they
                    // take the game path with the GLFW release arm's
                    // held-key discipline, so a key held from before focus
                    // can't stick, and a press the webview swallowed can't
                    // leak a phantom release edge. [xreview]
                    if down {
                        if let Some(webview_keys) = webview_keyboard.as_deref_mut() {
                            if let Some(k) = crate::webview_keys::webview_key_from_str(&key) {
                                webview_keys.push(k);
                            }
                            Ok(())
                        } else {
                            match InputKey::from_name(&key) {
                                Some(k) => {
                                    game.key_event(k as i32, true);
                                    let was_down = !held_keys.insert(k);
                                    edges.key_transition(k, was_down, true);
                                    Ok(())
                                }
                                None => Err(format!("unknown key: {}", key)),
                            }
                        }
                    } else {
                        match InputKey::from_name(&key) {
                            Some(k) => {
                                // Deliver the release only if the game saw
                                // the press (mirrors the GLFW release arm).
                                if held_keys.remove(&k) {
                                    edges.key_transition(k, true, false);
                                    game.key_event(k as i32, false);
                                }
                                Ok(())
                            }
                            None => Err(format!("unknown key: {}", key)),
                        }
                    }
                }
                debug_server::InputCommand::MouseMove { x, y } => {
                    *mouse_pos = (x, y);
                    if !emulate_xr {
                        game.mouse_move(x, y);
                    }
                    Ok(())
                }
                debug_server::InputCommand::MouseWheel { delta } => {
                    game.mouse_wheel(delta);
                    Ok(())
                }
                debug_server::InputCommand::MouseButton { button, down } => {
                    // Edge + level (see `apply_mouse_button_edge`). Unlike
                    // window buttons this ignores cursor capture — a
                    // headless/hidden session has no capture to acquire, and
                    // scripted clicks are the whole point.
                    match functor_runtime_common::MouseButton::from_name(&button) {
                        Some(b) => {
                            apply_mouse_button_edge(game, held_buttons, edges, b, down);
                            Ok(())
                        }
                        None => Err(format!("unknown mouse button: {}", button)),
                    }
                }
                debug_server::InputCommand::UiEvent { slot, kind } => {
                    game.ui_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
                debug_server::InputCommand::WebviewEvent { slot, kind } => {
                    game.webview_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
                debug_server::InputCommand::Xr(snapshot) => {
                    // No entry-point call here: XR is SAMPLED, not evented, so
                    // the sample must reach the game through the same
                    // `sampled_input` path a headset takes — which is what makes
                    // it land in the recorded input log and replay identically.
                    *xr_override = Some(*snapshot);
                    Ok(())
                }
                debug_server::InputCommand::XrClear => {
                    *xr_override = None;
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
            let _ = resp.send(clock.apply(cmd));
        }
    }
    sampled_input_changed
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
    emulate_xr: bool,
    input_script: Option<HashMap<u64, Vec<RecordedInput>>>,
    script_dt: f32,
) {
    // Stderr, not stdout: keep the CLI's `--json` ndjson stream (stdout) clean
    // even under `--headless`. This is an out-of-band notice, not an event.
    eprintln!("[runtime] headless mode — no GL window; /capture unavailable");

    let start_time = Instant::now();
    let mut last_time: f32 = 0.0;
    let mut frame_count: u64 = 0;
    let mut clock = GameClock::new(fixed_time);
    let mut held_keys: BTreeSet<InputKey> = BTreeSet::new();
    let mut held_buttons = MouseButtons::default();
    // Shell-owned across loop iterations: a render iteration may run zero
    // fixed steps, so transitions wait here until one actually consumes them.
    let mut input_edges = InputEdges::default();
    let mut mouse_pos: (i32, i32) = if emulate_xr {
        crate::desktop_xr_emulator::centered_pointer(crate::desktop_xr_emulator::reference_surface())
    } else {
        (0, 0)
    };
    let mut xr_override: Option<XrInputSnapshot> = None;
    // Keep the debug protocol target-isomorphic even without pixels: uploaded
    // assets can be accepted now and are ready if headless rendering gains an
    // asset path later.
    let asset_cache = Arc::new(AssetCache::new());
    let scene_context = SceneContext::new();

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
        if input_script.is_some() {
            clock.step(script_dt);
        }
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
        game.push_asset_progress(asset_cache.progress());
        let preload_commands =
            serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default();
        for token in scene_context.drive_preloads(&asset_cache, preload_commands) {
            game.preload_push_settled(token);
        }
        deliver_net_ws(&mut *game, &net_rx, &ws_rx);
        for sub in &sub_frames {
            if let Some(events) = input_script
                .as_ref()
                .and_then(|script| script.get(&frame_count))
            {
                apply_scripted_events(
                    &mut *game,
                    &mut held_keys,
                    &mut held_buttons,
                    &mut input_edges,
                    events,
                );
            }
            if game.samples_input() {
                let snapshot = desktop_input_snapshot(
                    &held_keys,
                    mouse_pos,
                    held_buttons,
                    &input_edges,
                    emulate_xr,
                    false,
                    crate::desktop_xr_emulator::reference_surface(),
                    xr_override.as_ref(),
                );
                game.sampled_input(&snapshot);
            }
            input_edges.clear();
            game.tick(sub.clone());
            frame_count += 1;
        }
        dispatch_net_ws(&mut *game, &net_tx, &http_client, &mut ws_manager);

        // The frame is pure data (no GL); it powers GET /scene. Drain and drop
        // audio commands so they do not pile up. Preloads and physics terrain
        // requests are driven above through the CPU-only asset cache.
        let frame = game.render(time.clone());
        let _ = game.audio_drain_commands();

        if let Some(rx) = &debug_requests {
            while let Ok(req) = rx.try_recv() {
                let state_input = desktop_input_snapshot(
                    &held_keys,
                    mouse_pos,
                    held_buttons,
                    &input_edges,
                    emulate_xr,
                    false,
                    crate::desktop_xr_emulator::reference_surface(),
                    xr_override.as_ref(),
                );
                service_debug_request(
                    req,
                    &mut *game,
                    &frame,
                    &mut frame_count,
                    time.tts,
                    0, // no framebuffer in headless
                    0,
                    &mut held_keys,
                    &mut mouse_pos,
                    &mut held_buttons,
                    &mut input_edges,
                    &mut xr_override,
                    state_input,
                    emulate_xr,
                    &mut clock,
                    &asset_cache,
                    &scene_context,
                    None,
                    None, // no webview overlay in headless
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
        let role = if args.entry_module.is_empty() {
            functor_runtime_common::functor_lang_producer::EntryRole::Prefix(
                args.entry_prefix.clone(),
            )
        } else {
            functor_runtime_common::functor_lang_producer::EntryRole::Module(
                args.entry_module.clone(),
            )
        };
        Box::new(functor_lang_game::FunctorLangGame::create_for_role(
            game_path.as_str(),
            role,
        ))
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

    // Hidden/headless/emulated-XR sessions do not have a live captured-pointer
    // path; their debug/script injection remains available without physical
    // mouse capture, so a warning would prescribe the wrong fix.
    let interactive_camera_warning =
        !args.headless && !args.hidden && args.capture_frame.is_none() && !args.emulate_xr;
    let mut warned_missing_mouse_capture = false;
    if interactive_camera_warning && !args.mouse_capture && game.uses_captured_mouse_input() {
        eprintln!(
            "[runner] warning: this game defines captured mouse hooks, but \
`\"mouseCapture\": false` disables them; remove that setting (or set it to true), \
then restart the runner"
        );
        warned_missing_mouse_capture = true;
    }

    // Scripted deterministic input (docs/time-travel.md T6b). Parsed up front so
    // a malformed script is a clean CLI error before any window/GL work. None
    // (the default) leaves live input and the wall-clock capture trigger
    // untouched.
    let input_script: Option<HashMap<u64, Vec<RecordedInput>>> =
        args.input_script
            .as_ref()
            .map(|path| match parse_input_script(path) {
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
    // not given — or when an optional (default) port was already taken.
    let debug_requests = args
        .debug_port
        .and_then(|port| debug_server::spawn(&args.debug_bind, port, args.debug_port_optional));

    // Headless: drive the game + debug server with no GL window, and return.
    if args.headless {
        if args.capture_frame.is_some() {
            eprintln!(
                "error: --capture-frame is not supported with --headless (no GL to read back)"
            );
            std::process::exit(1);
        }
        if args.xreal_tracking {
            eprintln!(
                "warning: --xreal-tracking has no effect in --headless mode (no view to rotate)"
            );
        }
        run_headless(
            game,
            debug_requests,
            args.fixed_time,
            args.emulate_xr,
            input_script,
            args.script_dt,
        );
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
            // A pinned size means the framebuffer must be exactly that, so opt
            // out of the display's backing scale rather than inheriting it.
            #[cfg(target_os = "macos")]
            if args.capture_size.is_some() {
                glfw.window_hint(glfw::WindowHint::CocoaRetinaFramebuffer(false));
            }
            let (width, height) = args.capture_size.unwrap_or((SCR_WIDTH, SCR_HEIGHT));

            // glfw window creation
            // --------------------
            let (mut window, events) = glfw
                .create_window(width, height, "Functor", glfw::WindowMode::Windowed)
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
            window.set_focus_polling(true);
            window.set_framebuffer_size_polling(true);
            // Captured game input is manifest-enabled by default, and capture
            // itself is still a user gesture: start with the cursor
            // free, then a non-overlay click disables it so GLFW reports
            // unbounded relative-like virtual motion. Escape RELEASES the
            // cursor (essential for the hot-reload loop: tweak code in the
            // editor while the game runs); a later click recaptures, and
            // Escape while released quits. Losing focus also releases, so
            // cmd-tabbing away hands the pointer back.
            if !hidden
                && !args.emulate_xr
                && args.mouse_capture
                && args.cursor == CursorPolicyArg::Captured
                && game.uses_captured_mouse_input()
            {
                window.set_cursor_mode(glfw::CursorMode::Normal);
                eprintln!(
                    "[runner] game mouse control available — click the window to capture; \
Escape releases while captured"
                );
            }
            if !hidden && !args.emulate_xr {
                eprintln!(
                    "[runner] debug camera available — press ~ to open the timeline"
                );
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
        // Same latch for the webview overlay (an `Html.input` is focused in
        // the blitz document): keyboard input routes to the webview under
        // the identical rules — see the `ui_wants_keyboard` arms below.
        let mut webview_wants_keyboard = false;
        // Keyboard input queued for the focused webview field, drained into
        // each frame's WorkerInput. Persistent (not per-frame) because the
        // debug server's injected keys land AFTER the frame's overlay pass —
        // they type on the next frame, like a keystroke between frames.
        let mut webview_keys: Vec<crate::webview_keys::WebviewKey> = Vec::new();
        // The time-travel console (`~`): HIDDEN by default in the native game
        // (it's a dev tool you summon with `~`, which also frees the cursor so
        // you can scrub right away; --scrubber starts it open, e.g. for
        // captures). The wasm/vscode preview shows it always.
        let mut scrubber_visible = args.scrubber;
        // Future-preview (docs/time-travel.md T6) state, driven interactively
        // from the scrubber's "extrapolate" switch (and, for the window, the
        // rail's endpoint handle). Extrapolation defaults ON with trail+strobe
        // out of the box (the demo default), and the launch flags pick the
        // mode; they stay authoritative while the overlay is hidden (the
        // headless-capture escape hatch).
        let mut extrapolate_on = true;
        let mut preview_mode = match (args.trajectory, args.strobe) {
            (true, false) => functor_runtime_common::PreviewMode::Trail,
            (false, true) => functor_runtime_common::PreviewMode::Strobe,
            _ => functor_runtime_common::PreviewMode::Both,
        };
        // The shared forward window and samples-per-second rate (density holds
        // as the window resizes: total samples = rate × window).
        let mut preview_window: f32 = 2.0;
        let mut preview_rate: usize = 5;
        // The PRESENCE RAMP: the overlay fades in and out over
        // `PRESENCE_RAMP_SECONDS` instead of popping when the preview toggles.
        //
        // This is runtime-loop state on purpose. Presence is NOT part of
        // `PreviewOptions` and NOT part of the cache key below — it multiplies
        // the alpha of the CACHED preview's per-frame clone at apply time, so
        // animating it never re-runs the forward sim, and the wanted-flags
        // don't move during a fade so the cache stays warm.
        let mut preview_presence: f32 = 0.0;
        // What the preview last selected while it was ON. Fading OUT needs
        // geometry after the toggle has already flipped the wanted-flags to
        // false, so the last on-mode keeps driving the computation until
        // presence reaches zero. Same type as the web twin uses for the same
        // state.
        let mut preview_last_on = functor_runtime_common::InteractivePreview::default();
        // An explicit `--preview-presence` PINS presence and skips the ease
        // (deterministic mid-fade captures); negative = no override.
        let preview_presence_override = (args.preview_presence >= 0.0)
            .then(|| args.preview_presence.clamp(0.0, 1.0));
        // --trajectory/--strobe preview cache. The 32-division forward-sim is
        // the expensive part, and while PAUSED its anchor (scene frame + tts) is
        // frozen — so reuse the computed preview while the anchor key matches,
        // but still refresh after a bounded number of frames for non-code state
        // outside the anchor key. A program revision invalidates immediately.
        // While live, the last projection remains visible between wall-clock
        // refreshes so continuous extrapolation bounds repeated work.
        const PAUSED_PREVIEW_REUSE_FRAMES: u32 = 30;
        const LIVE_PREVIEW_INTERVAL_SECONDS: f32 = 0.1;
        let mut trail_cache: Option<(
            (Option<u64>, u32, bool, u64, bool, bool, usize, u32, u64),
            functor_runtime_common::FramePreview,
        )> = None;
        let mut trail_refresh: u32 = 0;
        let mut next_live_trail_refresh: f32 = 0.0;

        gl.clear_color(0.1, 0.2, 0.3, 1.0);

        gl.enable(glow::DEPTH_TEST);

        let start_time = Instant::now();
        let mut last_time: f32 = 0.0;
        let mut frame_count: u64 = 0;
        let mut terrain_frame_id: u64 = 0;
        // The shared game clock (docs/time-travel.md): `tts` accumulates the real
        // frame delta while live, freezes on pause (scrubber / debug POST /time),
        // and rebases on a time-travel branch. `--fixed-time` seeds an
        // unconditional pin for deterministic captures / goldens.
        let mut clock = GameClock::new(args.fixed_time);
        // Runtime-owned input snapshot for GET /state, maintained from both the
        // GLFW event stream and the debug server's POST /input. Generic and
        // serializable, unlike the game model (which is Debug text only).
        let mut held_keys: BTreeSet<InputKey> = BTreeSet::new();
        // Held mouse buttons — the level complement of the `mouseButton`
        // edges, sampled into `Input.snapshot.mouse.buttons`.
        let mut held_buttons = MouseButtons::default();
        // Retained across render frames, consumed by the first fixed substep.
        let mut input_edges = InputEdges::default();
        let mut mouse_pos: (i32, i32) = if args.emulate_xr {
            crate::desktop_xr_emulator::centered_pointer(window.get_size())
        } else {
            (0, 0)
        };
        // The shell pointer keeps moving while a detached view is unlocked so
        // overlays remain usable. The game-owned position advances only when
        // input routing permits it, keeping both mouseMove and sampledInput out
        // of model state/replay during debug exploration.
        let mut game_mouse_pos = mouse_pos;
        let mut xr_primary_down = false;
        let mut xr_primary_clicked = false;
        let mut xr_override: Option<XrInputSnapshot> = None;
        // `--fixed-time` is a deterministic capture pin: the one zero-delta
        // bootstrap step must not observe cursor motion from the host window.
        // Explicit debug input remains authoritative; key release/focus-loss
        // bookkeeping also clears held levels so fixed captures cannot stick.
        // An input script uses the live runtime-owned snapshot below.
        let mut fixed_input_snapshot = desktop_input_snapshot(
            &held_keys,
            game_mouse_pos,
            held_buttons,
            &input_edges,
            args.emulate_xr,
            false,
            window.get_size(),
            xr_override.as_ref(),
        );
        // Whether the window owns the pointer (free-look). Capture is always
        // entered by a non-overlay click; see the Escape / MouseButton / Focus
        // arms in the event loop. A hidden window never captures (and never
        // receives the events that would toggle this).
        let visible_pointer = args.cursor == CursorPolicyArg::Visible;
        let game_mouse_capture = args.mouse_capture && !visible_pointer;
        let camera_detachable = !args.emulate_xr && !args.hidden;
        let mut detached_camera = DebugCamera::default();
        let launch_debug_render_mode: functor_runtime_common::DebugRenderMode =
            args.debug_render.into();
        let mut debug_presentation = DebugPresentation::from_render_mode(launch_debug_render_mode);
        let mut last_detached_cursor: Option<(f64, f64)> = None;
        let mut debug_held_keys = HashSet::new();
        let mut cursor_captured = false;
        let mut escape_armed = false;

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
        let mut audio_player = audio::AudioPlayer::new(audio_tx, asset_cache.clone());

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
            if input_script.is_some() {
                clock.step(args.script_dt);
            }
            // The fixed-timestep model loop (docs/time-travel.md): advance `tick`
            // in whole 1/60 steps decoupled from the render rate, so the sim is
            // deterministic and a recorded frame is exactly one forward-step fine
            // step (the forward-step replay's assumption). Scripted playback queued a
            // one-shot `step` above, so that yields one sub-frame per iteration
            // (a debug `advance` batch can queue more, draining ≤8 per frame);
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
            // signature of a preview's N forward-steps starving the loop, which
            // reads to the user as "jerky / can't move".
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

            // Effect.preload (B.5): warm the cache with this frame's queued
            // preloads and drive in-flight ones to settlement; a settled
            // preloadThen delivers its message through update.
            let preload_commands =
                serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default();
            for token in scene_context.drive_preloads(&asset_cache, preload_commands) {
                game.preload_push_settled(token);
            }

            // Asset hot-reload, the twin of the .fun check above: a model/
            // texture saved on disk is evicted from the caches so the next
            // draw re-reads and re-decodes it. Throttled: unlike the handful
            // of .fun files, the asset set is unbounded (a stat per asset per
            // poll), and 4Hz is plenty for a save-and-look loop.
            if last_asset_poll.elapsed().as_millis() >= 250 {
                last_asset_poll = Instant::now();
                // The producer can gain captured mouse hooks on hot reload.
                // Re-check at the existing 4 Hz shell poll, but teach only
                // once per session.
                if interactive_camera_warning
                    && !warned_missing_mouse_capture
                    && !args.mouse_capture
                    && game.uses_captured_mouse_input()
                {
                    eprintln!(
                        "[runner] warning: this game defines captured mouse hooks, but \
`\"mouseCapture\": false` disables them; remove that setting (or set it to true), \
then restart the runner"
                    );
                    warned_missing_mouse_capture = true;
                }
                // Keep capture capability in sync with a hot-reloaded program.
                // If its last captured hook disappears, return the attached
                // cursor to the shell; a detached debug camera remains
                // runtime-owned and therefore keeps its capture.
                if cursor_captured
                    && !detached_camera.is_detached()
                    && !game.uses_captured_mouse_input()
                {
                    release_held_mouse_buttons(
                        &mut *game,
                        &mut held_buttons,
                        &mut input_edges,
                        !clock.is_pinned(),
                    );
                    if clock.is_fixed_time() {
                        refresh_fixed_input_levels(
                            &mut fixed_input_snapshot,
                            &held_keys,
                            held_buttons,
                            false,
                            xr_override.as_ref(),
                        );
                    }
                    window.set_cursor_mode(glfw::CursorMode::Normal);
                    cursor_captured = false;
                    last_detached_cursor = None;
                    escape_armed = false;
                    eprintln!("[runner] captured mouse hooks were removed — cursor released");
                }
                for path in asset_watcher.changed(asset_cache.loaded_paths()) {
                    log::info!("asset '{}' changed on disk; reloading", path);
                    asset_cache.evict(&path);
                    scene_context.evict_asset(&path);
                    if let Some(player) = &mut audio_player {
                        player.evict_asset(&path);
                    }
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
            // Webview keystrokes collect into the persistent `webview_keys`
            // queue (declared above the loop). The webview arms sit FIRST:
            // it draws above the game UI, so when both latches are somehow up
            // the webview field wins (the scrubber-over-ui rule, keyboard
            // flavor).
            let mut suppress_next_debug_char = false;
            for (_, event) in glfw::flush_messages(&events) {
                use crate::webview_keys::WebviewKey;
                // GLFW reports a physical Key event before its separate,
                // layout-dependent Char event. Associate suppression with
                // that physical event, clearing it if another key intervenes.
                if matches!(&event, glfw::WindowEvent::Key(..)) {
                    suppress_next_debug_char = false;
                }
                match event {
                    glfw::WindowEvent::Close => window.set_should_close(true),
                    glfw::WindowEvent::Char(_) if suppress_next_debug_char => {
                        suppress_next_debug_char = false;
                    }
                    // Printable text for a focused webview field (the egui
                    // Char rule below, webview flavor).
                    glfw::WindowEvent::Char(c)
                        if webview_wants_keyboard && !ignore_user_input =>
                    {
                        webview_keys.push(WebviewKey::Char(c));
                    }
                    // Printable text for a focused field. Only meaningful
                    // while the overlay wants the keyboard; otherwise Char
                    // events are dropped (the game hears keys, not text).
                    glfw::WindowEvent::Char(c)
                        if ui_wants_keyboard && !ignore_user_input =>
                    {
                        ui_keyboard.push(functor_runtime_common::ui::UiKeyboardEvent::Char(c));
                    }
                    // While a webview field is focused, Escape DEFOCUSES it
                    // (the worker blurs the blitz document — blitz has no
                    // built-in Escape) instead of releasing the cursor; the
                    // next Escape then falls through to the arm below.
                    glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _)
                        if webview_wants_keyboard && !ignore_user_input =>
                    {
                        webview_keys.push(WebviewKey::Escape);
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
                        let action = escape_action(cursor_captured, escape_armed);
                        if action == EscapeAction::ReleaseCursor {
                            // Releasing the cursor stops delivering button
                            // events, so anything held must get its up edge now.
                            release_held_mouse_buttons(
                                &mut *game,
                                &mut held_buttons,
                                &mut input_edges,
                                !ignore_user_input,
                            );
                            // Under a pinned clock the sampled snapshot is not
                            // rebuilt per frame, so the cleared level has to be
                            // written into it or `sampledInput` reports the
                            // button held forever (as `Focus(false)` does).
                            if clock.is_fixed_time() {
                                refresh_fixed_input_levels(
                                    &mut fixed_input_snapshot,
                                    &held_keys,
                                    held_buttons,
                                    false,
                                    xr_override.as_ref(),
                                );
                            }
                            window.set_cursor_mode(glfw::CursorMode::Normal);
                            cursor_captured = false;
                            escape_armed = true;
                            if detached_camera.is_detached() {
                                eprintln!(
                                    "[runner] detached cursor released — use the timeline to \
reattach, click the window to keep exploring, or press Escape again to quit"
                                );
                            } else {
                                eprintln!(
                                    "[runner] cursor released — click the window to recapture, \
Escape again to quit"
                                );
                            }
                        } else if action == EscapeAction::ArmQuit {
                            escape_armed = true;
                            eprintln!("[runner] cursor is free — Escape again to quit");
                        } else {
                            window.set_should_close(true);
                        }
                    }
                    // `~` toggles the time-travel console. Opening it frees the
                    // cursor (so you can scrub immediately); closing returns to
                    // free-look. Before the `ignore_user_input` catch-all so it
                    // works while the clock is pinned (paused). Never grabs the
                    // pointer on a hidden window. Guarded off while a text
                    // field (egui or webview) is focused — typing '`' must
                    // insert, not toggle (the char arrives via a Char arm
                    // above).
                    glfw::WindowEvent::Key(Key::GraveAccent, _, Action::Press, _)
                        if !ui_wants_keyboard && !webview_wants_keyboard =>
                    {
                        scrubber_visible = !scrubber_visible;
                        // Same reason as Escape: this arm can drop capture.
                        release_held_mouse_buttons(
                            &mut *game,
                            &mut held_buttons,
                            &mut input_edges,
                            !ignore_user_input,
                        );
                        // Same pinned-clock caveat as the Escape arm above.
                        if clock.is_fixed_time() {
                            refresh_fixed_input_levels(
                                &mut fixed_input_snapshot,
                                &held_keys,
                                held_buttons,
                                false,
                                xr_override.as_ref(),
                            );
                        }
                        if !hidden {
                            if scrubber_visible {
                                window.set_cursor_mode(glfw::CursorMode::Normal);
                                cursor_captured = false;
                            } else if args.emulate_xr {
                                window.set_cursor_mode(glfw::CursorMode::Normal);
                                cursor_captured = false;
                            } else if (game_mouse_capture && game.uses_captured_mouse_input())
                                || detached_camera.is_detached()
                            {
                                window.set_cursor_mode(glfw::CursorMode::Disabled);
                                cursor_captured = true;
                                last_detached_cursor = None;
                            } else {
                                window.set_cursor_mode(glfw::CursorMode::Normal);
                                cursor_captured = false;
                            }
                        }
                        escape_armed = false;
                    }
                    // Left click while released: overlay hits drive the
                    // overlay; otherwise a game with captured mouse hooks
                    // recaptures for free-look. Games without those hooks keep
                    // every click available to overlays. Press/release edges
                    // feed egui's click detection. Never on a hidden window —
                    // it must not grab the pointer. These arms sit BEFORE the
                    // `ignore_user_input` catch-all so the scrubber stays
                    // usable while the clock is pinned (paused).
                    glfw::WindowEvent::MouseButton(glfw::MouseButtonLeft, action, _)
                        if !cursor_captured && !hidden =>
                    {
                        match action {
                            Action::Press => {
                                escape_armed = false;
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
                                    || webview_overlay
                                        .hit_interactive_css(mouse_pos.0 as f32, mouse_pos.1 as f32)
                                {
                                    mouse_primary_down = true;
                                    mouse_primary_clicked = true;
                                } else if args.emulate_xr {
                                    xr_primary_down = !ignore_user_input;
                                    xr_primary_clicked = xr_primary_down;
                                } else if (game_mouse_capture && game.uses_captured_mouse_input())
                                    || detached_camera.is_detached()
                                {
                                    window.set_cursor_mode(glfw::CursorMode::Disabled);
                                    cursor_captured = true;
                                    last_detached_cursor = None;
                                    // Entering free-look: blur any focused
                                    // webview field. The recapturing click
                                    // never reaches the blitz document (the
                                    // captured pointer is hidden from the
                                    // overlay), so without this the field
                                    // would keep eating WASD as text.
                                    if webview_wants_keyboard {
                                        webview_keys.push(WebviewKey::Escape);
                                    }
                                } else if visible_pointer {
                                    apply_window_mouse_button_edge(
                                        &mut *game,
                                        &mut held_buttons,
                                        &mut input_edges,
                                        functor_runtime_common::MouseButton::Left,
                                        true,
                                        !ignore_user_input,
                                    );
                                } else {
                                    // With no captured game mouse input the pointer
                                    // belongs to the overlays for the whole
                                    // session. Feed the click through instead
                                    // of turning an ordinary 2D/UI click into
                                    // an invisible cursor-capture transition.
                                    mouse_primary_down = true;
                                    mouse_primary_clicked = true;
                                }
                            }
                            Action::Release => {
                                mouse_primary_down = false;
                                xr_primary_down = false;
                                if args.cursor == CursorPolicyArg::Visible {
                                    apply_window_mouse_button_edge(
                                        &mut *game,
                                        &mut held_buttons,
                                        &mut input_edges,
                                        functor_runtime_common::MouseButton::Left,
                                        false,
                                        !ignore_user_input,
                                    );
                                }
                            }
                            Action::Repeat => {}
                        }
                    }
                    // Track the cursor for overlays while released. A visible
                    // project also receives this absolute logical position;
                    // captured projects reserve released motion for chrome.
                    glfw::WindowEvent::CursorPos(x, y) if !cursor_captured => {
                        mouse_pos = visible_cursor_position(x, y);
                        if route_visible_pointer_position(
                            &mut game_mouse_pos,
                            mouse_pos,
                            visible_pointer,
                            detached_camera.is_detached(),
                            ignore_user_input,
                        ) {
                            game.mouse_move(game_mouse_pos.0, game_mouse_pos.1);
                        }
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
                    // A press that began under the debug camera remains
                    // debug-owned through reattach until its physical release.
                    // Swallow repeats too, so they cannot create a game-owned
                    // press whose release the debug set would later consume.
                    glfw::WindowEvent::Key(key, _, Action::Press | Action::Repeat, _)
                        if debug_held_keys.contains(&key) =>
                    {
                        if cursor_captured && detached_camera.is_detached() {
                            suppress_next_debug_char = true;
                        }
                    }
                    // A detached debug camera owns NEW navigation presses. A
                    // press the game already owns remains game-owned until its
                    // physical release; activating the observer must never
                    // synthesize model/replay input.
                    glfw::WindowEvent::Key(key, _, Action::Press | Action::Repeat, _)
                        if detached_camera.is_detached()
                            && cursor_captured
                            && matches!(key, Key::W | Key::A | Key::S | Key::D | Key::Q | Key::E)
                            && !held_keys.contains(&map_key(key)) =>
                    {
                        debug_held_keys.insert(key);
                        suppress_next_debug_char = true;
                    }
                    glfw::WindowEvent::Key(key, _, Action::Release, _)
                        if debug_held_keys.remove(&key) =>
                    {}
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
                        // text field (egui or webview) must not leak a phantom
                        // release edge to the game's `input` hook (xreview).
                        // Unknown keys are never tracked; keep their
                        // pre-existing passthrough, but not while a field is
                        // focused.
                        let was_held = held_keys.remove(&k);
                        if !ignore_user_input
                            && (was_held
                                || (k == InputKey::Unknown
                                    && !ui_wants_keyboard
                                    && !webview_wants_keyboard))
                        {
                            if was_held {
                                input_edges.key_transition(k, true, false);
                            }
                            game.key_event(k as i32, false);
                        }
                        if clock.is_fixed_time() && was_held {
                            refresh_fixed_input_levels(
                                &mut fixed_input_snapshot,
                                &held_keys,
                                held_buttons,
                                xr_primary_down || xr_primary_clicked,
                                xr_override.as_ref(),
                            );
                        }
                    }
                    // Captured-policy mouse buttons reach the game only while
                    // the cursor is captured. While released, that policy
                    // reserves the pointer for overlays and click-to-recapture.
                    // Bookkeeping/delivery discipline mirrors keys exactly:
                    // this sits before the `ignore_user_input` catch-all so a
                    // button held at a pin transition can't stick, while the
                    // game only hears edges it may act on.
                    // NOTE: `--emulate-xr` never captures the cursor (its left
                    // click drives the emulated XR primary instead), so window
                    // buttons do not reach `mouseButton` under it — inject them
                    // through `POST /input` when scripting that rig.
                    glfw::WindowEvent::MouseButton(button, action, _)
                        if cursor_captured && !detached_camera.is_detached() =>
                    {
                        let mapped = map_mouse_button(button);
                        if mapped != functor_runtime_common::MouseButton::Unknown {
                            match action {
                                Action::Press => apply_window_mouse_button_edge(
                                    &mut *game,
                                    &mut held_buttons,
                                    &mut input_edges,
                                    mapped,
                                    true,
                                    !ignore_user_input,
                                ),
                                Action::Release => apply_window_mouse_button_edge(
                                    &mut *game,
                                    &mut held_buttons,
                                    &mut input_edges,
                                    mapped,
                                    false,
                                    !ignore_user_input,
                                ),
                                Action::Repeat => {}
                            }
                            if clock.is_fixed_time() {
                                refresh_fixed_input_levels(
                                    &mut fixed_input_snapshot,
                                    &held_keys,
                                    held_buttons,
                                    xr_primary_down || xr_primary_clicked,
                                    xr_override.as_ref(),
                                );
                            }
                        }
                    }
                    // Visible-pointer games receive right/middle buttons
                    // directly. Left is handled by the released-pointer arm
                    // above so interactive overlays can consume it first.
                    glfw::WindowEvent::MouseButton(button, action, _)
                        if args.cursor == CursorPolicyArg::Visible
                            && button != glfw::MouseButtonLeft =>
                    {
                        let mapped = map_mouse_button(button);
                        if mapped != functor_runtime_common::MouseButton::Unknown {
                            match action {
                                Action::Press => apply_window_mouse_button_edge(
                                    &mut *game,
                                    &mut held_buttons,
                                    &mut input_edges,
                                    mapped,
                                    true,
                                    visible_pointer_game_input_enabled(
                                        visible_pointer,
                                        detached_camera.is_detached(),
                                        ignore_user_input,
                                    ),
                                ),
                                Action::Release => apply_window_mouse_button_edge(
                                    &mut *game,
                                    &mut held_buttons,
                                    &mut input_edges,
                                    mapped,
                                    false,
                                    !ignore_user_input,
                                ),
                                Action::Repeat => {}
                            }
                            if clock.is_fixed_time() {
                                refresh_fixed_input_levels(
                                    &mut fixed_input_snapshot,
                                    &held_keys,
                                    held_buttons,
                                    xr_primary_down || xr_primary_clicked,
                                    xr_override.as_ref(),
                                );
                            }
                        }
                    }
                    glfw::WindowEvent::Focus(false) => {
                        for k in std::mem::take(&mut held_keys) {
                            if !ignore_user_input {
                                input_edges.key_transition(k, true, false);
                                game.key_event(k as i32, false);
                            }
                        }
                        // Hand the pointer back when the window loses focus
                        // (cmd-tab to the editor). Captured projects recapture
                        // on a later click; visible projects remain free.
                        window.set_cursor_mode(glfw::CursorMode::Normal);
                        cursor_captured = false;
                        last_detached_cursor = None;
                        debug_held_keys.clear();
                        escape_armed = false;
                        // A button held at focus-loss may never get its release
                        // (alt-tab), which would leave egui holding a stuck
                        // press — clear it so the scrubber stays live. [xreview]
                        mouse_primary_down = false;
                        mouse_primary_clicked = false;
                        xr_primary_down = false;
                        xr_primary_clicked = false;
                        // The GAME's held buttons need the same sweep as
                        // `held_keys` above, or a click held at alt-tab fires
                        // forever.
                        release_held_mouse_buttons(
                            &mut *game,
                            &mut held_buttons,
                            &mut input_edges,
                            !ignore_user_input,
                        );
                        if clock.is_fixed_time() {
                            refresh_fixed_input_levels(
                                &mut fixed_input_snapshot,
                                &held_keys,
                                held_buttons,
                                false,
                                xr_override.as_ref(),
                            );
                        }
                    }
                    // Debug-camera navigation is shell input while playing or
                    // paused, and never reaches the game's hooks or replay log.
                    glfw::WindowEvent::CursorPos(x, y)
                        if cursor_captured && detached_camera.is_detached() =>
                    {
                        if let Some((last_x, last_y)) = last_detached_cursor {
                            detached_camera.look((x - last_x) as f32, (y - last_y) as f32);
                        }
                        last_detached_cursor = Some((x, y));
                    }
                    glfw::WindowEvent::Scroll(_, y)
                        if cursor_captured && detached_camera.is_detached() =>
                    {
                        detached_camera.zoom(y as f32);
                    }
                    _ if ignore_user_input => {}
                    // While a webview field is focused, key presses drive the
                    // WEBVIEW: the editing subset maps to `WebviewKey`s the
                    // render worker lowers to blitz key events (Repeat works —
                    // each press lowers to a full pair), printable keys arrive
                    // via the Char arm, and the game's `input` hook hears none
                    // of it (the focus gate — presses swallowed here never
                    // enter held_keys, so the release arm can't leak a
                    // phantom edge either).
                    glfw::WindowEvent::Key(key, _, Action::Press | Action::Repeat, _)
                        if webview_wants_keyboard =>
                    {
                        let mapped = match key {
                            Key::Backspace => Some(WebviewKey::Backspace),
                            Key::Delete => Some(WebviewKey::Delete),
                            Key::Left => Some(WebviewKey::Left),
                            Key::Right => Some(WebviewKey::Right),
                            Key::Home => Some(WebviewKey::Home),
                            Key::End => Some(WebviewKey::End),
                            Key::Enter => Some(WebviewKey::Enter),
                            _ => None,
                        };
                        if let Some(mapped) = mapped {
                            webview_keys.push(mapped);
                        }
                    }
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
                            let was_down = !held_keys.insert(k);
                            input_edges.key_transition(k, was_down, true);
                        }
                    }
                    // While the cursor is released, pointer motion/scroll
                    // must not drive the camera (you're aiming at your editor,
                    // not the game).
                    glfw::WindowEvent::CursorPos(x, y) if cursor_captured => {
                        mouse_pos = (x as i32, y as i32);
                        game_mouse_pos = mouse_pos;
                        game.mouse_move(game_mouse_pos.0, game_mouse_pos.1)
                    }
                    glfw::WindowEvent::Scroll(_, y)
                        if (cursor_captured || visible_pointer)
                            && !detached_camera.is_detached() =>
                    {
                        game.mouse_wheel(y as i32)
                    }
                    _ => {}
                }
            }

            if cursor_captured && detached_camera.is_detached() {
                let axis = |positive: Key, negative: Key| {
                    (if debug_held_keys.contains(&positive) {
                        1.0
                    } else {
                        0.0
                    }) - if debug_held_keys.contains(&negative) {
                        1.0
                    } else {
                        0.0
                    }
                };
                detached_camera.move_local(
                    axis(Key::W, Key::S),
                    axis(Key::D, Key::A),
                    axis(Key::E, Key::Q),
                    real_delta,
                );
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
            // tick. `--capture-at-frame` fires on the fixed frame it
            // names — scripted playback is exactly one fixed step per render frame,
            // so that index stays deterministic.
            let mut capture_due = false;
            for sub in &sub_frames {
                if let Some(script) = input_script.as_ref() {
                    if let Some(events) = script.get(&frame_count) {
                        apply_scripted_events(
                            &mut *game,
                            &mut held_keys,
                            &mut held_buttons,
                            &mut input_edges,
                            events,
                        );
                    }
                }
                if game.samples_input() {
                    if clock.is_fixed_time() && input_script.is_none() {
                        input_edges.apply_to(&mut fixed_input_snapshot);
                        game.sampled_input(&fixed_input_snapshot);
                    } else {
                        let snapshot = desktop_input_snapshot(
                            &held_keys,
                            game_mouse_pos,
                            held_buttons,
                            &input_edges,
                            args.emulate_xr,
                            xr_primary_down || xr_primary_clicked,
                            window.get_size(),
                            xr_override.as_ref(),
                        );
                        game.sampled_input(&snapshot);
                    }
                }
                input_edges.clear();
                fixed_input_snapshot.clear_edges();
                xr_primary_clicked = false;
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

            // One wanted-set drives the future preview
            // (docs/time-travel.md T6): the scene-diff trail/strobe overlays.
            // With the overlay up, the seam-selected mode drives it; its
            // anchor follows the live tail while playing and freezes when
            // paused. With the overlay hidden the launch flags drive it, so
            // headless captures are byte-for-byte unchanged.
            let (trail_wanted, strobe_wanted) = if scrubber_visible {
                let selected = functor_runtime_common::interactive_preview(
                    preview_mode,
                    extrapolate_on,
                    clock.pending_frames() > 0,
                );
                (selected.trail, selected.strobe)
            } else {
                (args.trajectory, args.strobe)
            };
            // Advance the presence ramp toward whether the preview selects
            // anything, then keep DRIVING the last on-mode while it fades out —
            // otherwise the toggle would take the geometry away in the same
            // frame it started the fade.
            //
            // The ease runs on WALL-CLOCK delta, independent of `--fixed-time`
            // (which pins the game's POSE, not real time). Captures stay
            // reproducible because the ramp is 0.28s and `--capture-time`
            // defaults to 2s — the fade is long over by the time the shot is
            // taken. `--preview-presence` pins it outright for an exact
            // mid-fade frame.
            let preview_selects = trail_wanted || strobe_wanted;
            if preview_selects {
                preview_last_on = functor_runtime_common::InteractivePreview {
                    trail: trail_wanted,
                    strobe: strobe_wanted,
                };
            }
            let display_presence = match preview_presence_override {
                // A pin HOLDS the phase at the opacity it names, so clearing it
                // resumes the ease from what was on screen rather than from
                // wherever a free-running phase had drifted to.
                Some(pinned) => {
                    preview_presence = functor_runtime_common::presence_phase(pinned);
                    pinned
                }
                None => {
                    preview_presence = functor_runtime_common::presence_step(
                        preview_presence,
                        if preview_selects { 1.0 } else { 0.0 },
                        real_delta,
                    );
                    functor_runtime_common::presence_ease(preview_presence)
                }
            };
            let (trail_wanted, strobe_wanted) = if preview_selects {
                (trail_wanted, strobe_wanted)
            } else {
                (preview_last_on.trail, preview_last_on.strobe)
            };
            // Forward window/densities. The SIM samples fine (~20/s — the
            // trail's smooth-arc rate) while the preview rate governs STROBE COPIES
            // per second, so dots stay visible between copies and both hold
            // their density as the window resizes. The flag path keeps its
            // historical constants (32 samples over 1.6s, 8 copies), keeping
            // captures byte-identical.
            const TRAIL_RATE: f32 = 20.0;
            let (preview_divisions, preview_window_s, strobe_copies) = if scrubber_visible {
                let divisions = ((TRAIL_RATE * preview_window).round() as usize).clamp(1, 64);
                let copies =
                    ((preview_rate as f32 * preview_window).round() as usize).clamp(1, divisions);
                (divisions, preview_window, copies)
            } else {
                (32usize, 1.6f32, 8usize)
            };
            // While a drag-into-the-future catch-up — or a batched debug
            // `advance` — is draining, skip the preview recompute (the anchor
            // moves every frame, so it would be a full forward-sim per frame
            // and throttle the drain to a crawl) — it snaps back in on arrival.
            let preview_active = (trail_wanted || strobe_wanted)
                && display_presence > 0.0
                && args.composite_demo == CompositeDemoArg::Off
                && clock.pending_frames() == 0
                && clock.pending_steps() == 0;
            let preview: Option<functor_runtime_common::FramePreview> = if preview_active {
                // Sample finely for a smooth arc — this only reads node
                // transforms.
                let divisions = preview_divisions;
                let window = preview_window_s;
                let dt = window / divisions as f32;
                let key = (
                    game.current_scene_frame(),
                    time.tts.to_bits(),
                    clock.is_paused(),
                    game.scene_program_revision(),
                    trail_wanted,
                    strobe_wanted,
                    divisions,
                    window.to_bits(),
                    // The BACKWARD side reads recorded history, so a branch or
                    // a reload that rewrites the rings invalidates it even
                    // when frame + program are unchanged.
                    game.scene_timeline_generation(),
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
                            && k.7 == key.7
                            && k.8 == key.8
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
                    // script, so the forward-sim must replay the SAME upcoming
                    // slice — with `None` it would
                    // replay the recorder's (empty-at-head) log and predict an
                    // input-free future the game won't play. The slice anchors at
                    // the fine step after the newest recorded frame, mirroring
                    // the `inputs_from(k + 1)` anchor of the recorded-log path.
                    let script_slice: Option<Vec<Vec<RecordedInput>>> =
                        input_script.as_ref().map(|script| {
                            let sub_dt = 1.0f32 / 60.0;
                            let steps_per_division = ((dt / sub_dt).round() as usize).max(1);
                            let total = (divisions * steps_per_division) as u64;
                            // Live playback consumes the script, so the next
                            // unconsumed frame (k + 1) is the right anchor.
                            let base = game.current_scene_frame().map_or(0, |k| k + 1);
                            (0..total)
                                .map(|j| script.get(&(base + j)).cloned().unwrap_or_default())
                                .collect()
                        });
                    let p = functor_runtime_common::frame_preview(
                        &*game,
                        &frame,
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
                            // One SYMMETRIC window: `window` seconds of recorded
                            // past as well as projected future
                            // (docs/time-travel.md T6e). The past side costs one
                            // draw per division with no simulation, and shares
                            // this cache — so live play redraws it on the same
                            // ~10 Hz cadence as the forward sim, never per frame.
                            backward: true,
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

            // A live game may switch between a pure 2D and a 3D/mixed frame.
            // Never retain a debug snapshot whose navigation semantics no
            // longer match what is being rendered.
            if detached_camera.is_detached() && !detached_camera.is_compatible(&frame) {
                detached_camera.reattach();
                last_detached_cursor = None;
                window.set_cursor_mode(glfw::CursorMode::Normal);
                cursor_captured = false;
            }

            // The camera actually viewed/heard: a shell-owned debug snapshot
            // when detached, otherwise the game's authored camera. Tracking
            // composes last so a debug view never freezes the user's head.
            let authored_view_camera = match &xreal_tracker {
                Some(tracker) => {
                    xreal::apply_head_rotation(&frame.camera, tracker.orientation())
                }
                None => frame.camera.clone(),
            };
            let view_camera = if detached_camera.is_detached() {
                match &xreal_tracker {
                    Some(tracker) => xreal::apply_head_rotation(
                        detached_camera.camera(&frame.camera),
                        tracker.orientation(),
                    ),
                    None => detached_camera.camera(&frame.camera).clone(),
                }
            } else {
                authored_view_camera.clone()
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

            // Detached diagnostics are shell state: material replacement and
            // line overlays never enter the game model or replay. Outside the
            // debug view, preserve the launch-time `--debug-render` behavior.
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
                        |w| w.debug_lines(),
                    )
                })
                .flatten()
                .unwrap_or_default();
            if diagnostics.authored_camera_frustum {
                debug_lines.extend(camera_frustum_lines(&frame.camera, viewport.aspect()));
            }

            // The frame the render ladder draws: the game frame plus the
            // preview overlays (`frame` itself stays pristine for GET /scene).
            let display_frame = match &preview {
                Some(p) if !p.is_empty() => {
                    let mut f = frame.clone();
                    p.apply_all_with_presence(&mut f, display_presence);
                    Some(f)
                }
                _ => None,
            };
            let drawn_frame = display_frame.as_ref().unwrap_or(&frame);

            // Shadow + forward passes, shared with the web runtime. In stereo
            // mode, render the same frame twice — once per eye camera into each
            // half of the window. (The shadow pass runs per eye; it must start
            // unscissored, so scissor is reset before each call — the contract
            // for any sub-rectangle view.)
            // Time the whole scene render (all arms) for FrameStats.render_us —
            // the GL-submission cost, distinct from the interpreter's draw cost.
            let render_started = Instant::now();
            if args.stereo_sbs {
                let (left_cam, right_cam) = view_camera.stereo_eyes(args.stereo_ipd);
                let (lod_left_cam, lod_right_cam) =
                    authored_view_camera.stereo_eyes(args.stereo_ipd);
                // Odd widths: the right eye absorbs the extra column, so the
                // two viewports tile the framebuffer exactly (no stale strip).
                let half_w = (fb_width as u32) / 2;
                let right_w = fb_width as u32 - half_w;
                let eyes = [
                    (
                        left_cam,
                        functor_runtime_common::Viewport::with_offset(
                            0,
                            0,
                            half_w,
                            fb_height as u32,
                        ),
                    ),
                    (
                        right_cam,
                        functor_runtime_common::Viewport::with_offset(
                            half_w,
                            0,
                            right_w,
                            fb_height as u32,
                        ),
                    ),
                ];
                let eye_projections = [
                    eyes[0].0.projection_matrix(eyes[0].1.aspect()),
                    eyes[1].0.projection_matrix(eyes[1].1.aspect()),
                ];
                let lod_eye_projections = [
                    lod_left_cam.projection_matrix(eyes[0].1.aspect()),
                    lod_right_cam.projection_matrix(eyes[1].1.aspect()),
                ];
                let lod_view_projections = [
                    lod_eye_projections[0] * lod_left_cam.view_matrix(),
                    lod_eye_projections[1] * lod_right_cam.view_matrix(),
                ];
                let lod_projection_scale = lod_eye_projections[0]
                    .y
                    .y
                    .abs()
                    .max(lod_eye_projections[1].y.y.abs());
                terrain_frame_id = terrain_frame_id.wrapping_add(1);
                for ((camera, eye_viewport), projection) in
                    eyes.iter().zip(eye_projections.iter())
                {
                    gl.disable(glow::SCISSOR_TEST);
                    functor_runtime_common::render_frame_with_projection(
                        &gl,
                        shader_version,
                        asset_cache.clone(),
                        &scene_context,
                        &shadow_map,
                        drawn_frame,
                        camera,
                        projection,
                        &authored_view_camera,
                        &lod_view_projections,
                        lod_projection_scale,
                        fb_height as f32,
                        terrain_frame_id,
                        detached_camera.sprite_cameras(),
                        time.clone(),
                        *eye_viewport,
                        diagnostics.render_mode,
                    );
                    if !debug_lines.is_empty() {
                        functor_runtime_common::render_debug_lines(
                            &gl,
                            shader_version,
                            camera,
                            *eye_viewport,
                            &debug_lines,
                        );
                    }
                }
            } else if args.composite_demo != CompositeDemoArg::Off {
                // T5 compositor smoke test: render two frames and average them.
                // `clone` overlays the frame with itself (lossless check);
                // `fork` overlays it with a world-space-offset copy of its scene
                // (the fork+overlay demo — two visibly-different inputs).
                let frame_a = frame.clone();
                let mut frame_b = frame.clone();
                if args.composite_demo == CompositeDemoArg::Fork {
                    frame_b.scene = functor_runtime_common::Scene3D {
                        obj: functor_runtime_common::SceneObject::Group(vec![frame.scene.clone()]),
                        xform: cgmath::Matrix4::from_translation(cgmath::vec3(6.0, 0.0, 0.0)),
                    };
                }
                let frames = [(frame_a, time.clone()), (frame_b, time.clone())];
                functor_runtime_common::render_composited_frames_with_view(
                    &gl,
                    shader_version,
                    asset_cache.clone(),
                    &scene_context,
                    &shadow_map,
                    &frames,
                    &[0.5, 0.5],
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
                    drawn_frame,
                    &view_camera,
                    detached_camera.sprite_cameras(),
                    time.clone(),
                    viewport,
                    diagnostics.render_mode,
                );
            }
            if !args.stereo_sbs && !debug_lines.is_empty() {
                functor_runtime_common::render_debug_lines(
                    &gl,
                    shader_version,
                    &view_camera,
                    viewport,
                    &debug_lines,
                );
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
            let show_game_ui = !detached_camera.is_detached() || debug_presentation.show_game_ui;
            let ui_view = if show_game_ui {
                game.ui()
            } else {
                functor_runtime_common::ui::View::Empty
            };
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
            let webview_html = show_game_ui
                .then(|| game.webview().map(|node| node.to_html()))
                .flatten();
            // `time.tts` as the CSS animation clock: `--fixed-time` pins it
            // (deterministic captures) and pausing freezes overlay animations
            // coherently with the game.
            let webview_out = webview_overlay.frame(
                fb_width as u32,
                fb_height as u32,
                dpi_scale,
                webview_html.as_deref(),
                webview_pointer,
                std::mem::take(&mut webview_keys),
                time.tts as f64,
            );
            webview_wants_pointer = webview_out.wants_pointer;
            webview_wants_keyboard = webview_out.wants_keyboard;
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
                        camera_detachable,
                        camera_catching_up: clock.pending_frames() > 0 || clock.pending_steps() > 0,
                        camera_detached: detached_camera.is_detached(),
                        camera_mode: detached_camera.mode(),
                        camera_fov_degrees: detached_camera.fov_degrees(),
                        camera_zoom_2d: detached_camera.zoom_2d(),
                        debug_presentation,
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
                Some(functor_runtime_common::ui::ScrubberAction::ToggleDetachedCamera) => {
                    if detached_camera.is_detached() {
                        detached_camera.reattach();
                        last_detached_cursor = None;
                        window.set_cursor_mode(glfw::CursorMode::Normal);
                        cursor_captured = false;
                    } else if camera_detachable
                        && clock.pending_frames() == 0
                        && clock.pending_steps() == 0
                    {
                        if detached_camera.detach(&frame) {
                            last_detached_cursor = None;
                            window.set_cursor_mode(glfw::CursorMode::Disabled);
                            cursor_captured = true;
                        } else {
                            eprintln!(
                                "[runner] debug camera unavailable: the authored camera is invalid"
                            );
                        }
                    }
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetDebugCameraMode(mode)) => {
                    detached_camera.set_mode(mode);
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetDebugCameraFov(fov)) => {
                    detached_camera.set_fov_degrees(fov);
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetDebugMaterial(material)) => {
                    debug_presentation.material = material;
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetDebugPhysics(enabled)) => {
                    debug_presentation.physics = enabled;
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetAuthoredCameraFrustum(
                    enabled,
                )) => {
                    debug_presentation.authored_camera_frustum = enabled;
                }
                Some(functor_runtime_common::ui::ScrubberAction::SetGameUiVisible(visible)) => {
                    debug_presentation.show_game_ui = visible;
                }
                Some(functor_runtime_common::ui::ScrubberAction::ResetDebugCamera) => {
                    let mode = detached_camera.mode();
                    if detached_camera.detach(&frame) {
                        if let Some(mode) = mode {
                            detached_camera.set_mode(mode);
                        }
                    }
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
                    let fixed_sample = clock.is_fixed_time() && input_script.is_none();
                    let state_input = if fixed_sample {
                        fixed_input_snapshot.clone()
                    } else {
                        desktop_input_snapshot(
                            &held_keys,
                            game_mouse_pos,
                            held_buttons,
                            &input_edges,
                            args.emulate_xr,
                            xr_primary_down || xr_primary_clicked,
                            window.get_size(),
                            xr_override.as_ref(),
                        )
                    };
                    let sampled_input_changed = service_debug_request(
                        req,
                        &mut *game,
                        &frame,
                        &mut frame_count,
                        time.tts,
                        fb_width as u32,
                        fb_height as u32,
                        &mut held_keys,
                        &mut game_mouse_pos,
                        &mut held_buttons,
                        &mut input_edges,
                        &mut xr_override,
                        state_input,
                        args.emulate_xr,
                        &mut clock,
                        &asset_cache,
                        &scene_context,
                        audio_player.as_mut(),
                        // The webview focus gate: injected keys type into a
                        // focused webview field (queued for the next frame's
                        // overlay pass) instead of reaching the game.
                        if webview_wants_keyboard {
                            Some(&mut webview_keys)
                        } else {
                            None
                        },
                        // GL readback on the render thread (a real Failed on error).
                        &|| {
                            functor_runtime_common::frame_capture::encode_bound_framebuffer_png(
                                &gl,
                                fb_width as u32,
                                fb_height as u32,
                            )
                                .map_err(debug_server::CaptureError::Failed)
                        },
                    );
                    if fixed_sample && sampled_input_changed {
                        fixed_input_snapshot = desktop_input_snapshot(
                            &held_keys,
                            game_mouse_pos,
                            held_buttons,
                            &input_edges,
                            args.emulate_xr,
                            xr_primary_down || xr_primary_clicked,
                            window.get_size(),
                            xr_override.as_ref(),
                        );
                    }
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
    use functor_runtime_common::{TrackingPose, XrControllerSnapshot};
    use std::io::Write;

    #[test]
    fn escape_releases_or_arms_before_it_quits() {
        assert_eq!(
            escape_action(true, false),
            EscapeAction::ReleaseCursor,
            "captured Escape releases the pointer"
        );
        assert_eq!(
            escape_action(false, false),
            EscapeAction::ArmQuit,
            "the first free-pointer Escape must not close the game"
        );
        assert_eq!(
            escape_action(false, true),
            EscapeAction::Quit,
            "the second free-pointer Escape closes the game"
        );
    }

    #[test]
    fn detached_debug_camera_blocks_new_visible_pointer_game_input() {
        assert!(visible_pointer_game_input_enabled(true, false, false));
        assert!(!visible_pointer_game_input_enabled(false, false, false));
        assert!(!visible_pointer_game_input_enabled(true, true, false));
        assert!(!visible_pointer_game_input_enabled(true, false, true));

        let mut game_mouse_pos = (10, 20);
        assert!(!route_visible_pointer_position(
            &mut game_mouse_pos,
            (90, 80),
            true,
            true,
            false,
        ));
        assert_eq!(
            game_mouse_pos,
            (10, 20),
            "sampledInput must retain the last game-owned pointer position"
        );
        assert!(route_visible_pointer_position(
            &mut game_mouse_pos,
            (30, 40),
            true,
            false,
            false,
        ));
        assert_eq!(game_mouse_pos, (30, 40));
    }

    #[test]
    fn mouse_capture_arg_is_an_opt_in_flag() {
        let default = Args::try_parse_from(["functor", "--game-path", "game.fun"]).unwrap();
        assert!(!default.mouse_capture);

        let game = Args::try_parse_from(["functor", "--game-path", "game.fun", "--mouse-capture"])
            .unwrap();
        assert!(game.mouse_capture);
    }

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
        assert!(matches!(
            map[&71][0],
            RecordedInput::Key { is_down: false, .. }
        ));
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

    #[test]
    fn parse_input_script_maps_mouse_buttons() {
        use functor_runtime_common::MouseButton as Btn;

        let path = write_tmp(
            "functor-test-mouse.script",
            "0 Mouse.Left down\n4 mouse.MIDDLE down\n7 Mouse.Right down\n9 Mouse.Right up  # case-insensitive\n",
        );
        let map = parse_input_script(path.to_str().unwrap()).unwrap();

        assert!(matches!(
            map[&0][0],
            RecordedInput::MouseButton { button, is_down: true } if button == Btn::Left as i32
        ));
        // The `Mouse.` prefix and the button name are both case-insensitive,
        // matching `MouseButton::from_name`'s wire spelling.
        assert!(matches!(
            map[&4][0],
            RecordedInput::MouseButton { button, is_down: true } if button == Btn::Middle as i32
        ));
        assert!(matches!(
            map[&9][0],
            RecordedInput::MouseButton { button, is_down: false } if button == Btn::Right as i32
        ));
    }

    /// `Left`/`Right`/`Middle` name a KEY and a MOUSE BUTTON. The `Mouse.`
    /// prefix is the whole disambiguation, so pin both readings: bare stays a
    /// key (no silent behavior change for existing scripts), prefixed is the
    /// button.
    #[test]
    fn parse_input_script_disambiguates_mouse_from_key_names() {
        use functor_runtime_common::MouseButton as Btn;

        let path = write_tmp(
            "functor-test-ambiguous.script",
            "0 Right down\n1 Mouse.Right down\n",
        );
        let map = parse_input_script(path.to_str().unwrap()).unwrap();

        assert!(
            matches!(
                map[&0][0],
                RecordedInput::Key { code, .. } if code == InputKey::Right as i32
            ),
            "a bare `Right` must remain the arrow key"
        );
        assert!(
            matches!(
                map[&1][0],
                RecordedInput::MouseButton { button, .. } if button == Btn::Right as i32
            ),
            "`Mouse.Right` must be the mouse button"
        );
    }

    /// A release the live path would suppress must not parse at all: the ghost
    /// forward-step preview replays script events unconditionally, so allowing
    /// one would let the preview and the real run disagree.
    #[test]
    fn parse_input_script_rejects_unmatched_button_releases() {
        let stray = write_tmp("functor-test-strayup.script", "0 Mouse.Left down\n9 Mouse.Right up\n");
        let err = parse_input_script(stray.to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("never pressed") && err.contains("Mouse.Right"),
            "the error must name the unmatched button: {err}"
        );

        // Ordering is by FRAME, not by line: a press written after its release
        // is still unmatched at the frame the release happens.
        let reordered = write_tmp(
            "functor-test-reordered.script",
            "9 Mouse.Left down\n0 Mouse.Left up\n",
        );
        assert!(parse_input_script(reordered.to_str().unwrap()).is_err());

        // A properly paired press/release round-trips, including re-pressing.
        let paired = write_tmp(
            "functor-test-paired.script",
            "0 Mouse.Left down\n5 Mouse.Left up\n8 Mouse.Left down\n12 Mouse.Left up\n",
        );
        assert!(parse_input_script(paired.to_str().unwrap()).is_ok());

        // Key releases are unaffected — keys have never had a held gate here.
        let key_up = write_tmp("functor-test-keyup.script", "0 Space up\n");
        assert!(parse_input_script(key_up.to_str().unwrap()).is_ok());
    }

    #[test]
    fn parse_input_script_rejects_unknown_mouse_buttons() {
        let bad = write_tmp("functor-test-badbutton.script", "0 Mouse.Thumb down\n");
        let err = parse_input_script(bad.to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown mouse button") && err.contains("Mouse.Left"),
            "the error must name the button and list the valid ones: {err}"
        );

        // `Mouse.Unknown` is the never-delivered sentinel, not a scriptable
        // button — `from_name` rejects it and so must the script.
        let sentinel = write_tmp("functor-test-unknownbtn.script", "0 Mouse.Unknown down\n");
        assert!(parse_input_script(sentinel.to_str().unwrap()).is_err());

        // A non-mouse dotted name is still a key lookup, and fails as one.
        let dotted = write_tmp("functor-test-dotted.script", "0 Key.Left down\n");
        let err = parse_input_script(dotted.to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown key") && err.contains("Mouse."),
            "the key error must hint at the mouse prefix: {err}"
        );
    }

    #[test]
    fn fixed_level_refresh_preserves_mouse_and_tracked_poses() {
        let mut snapshot = desktop_input_snapshot(
            &BTreeSet::new(),
            (100, 200),
            MouseButtons::default(),
            &InputEdges::default(),
            true,
            false,
            (800, 600),
            None,
        );
        let pose = snapshot
            .xr
            .as_ref()
            .and_then(|xr| xr.right.grip)
            .expect("emulated grip");
        let held = BTreeSet::from([InputKey::Space]);
        // A button held at the fixed-step boundary is level state like a key:
        // the refresh must carry it, not drop it.
        let mut buttons = MouseButtons::default();
        buttons.set(functor_runtime_common::MouseButton::Right, true);

        refresh_fixed_input_levels(
            &mut snapshot,
            &held,
            buttons,
            false,
            None,
        );

        assert_eq!(
            snapshot.mouse,
            MouseSnapshot {
                x: 100,
                y: 200,
                surface_width: 800,
                surface_height: 600,
                buttons,
                ..MouseSnapshot::default()
            }
        );
        assert_eq!(
            snapshot.xr.as_ref().and_then(|xr| xr.right.grip),
            Some(pose)
        );
        assert_eq!(snapshot.xr.as_ref().unwrap().right.trigger, 1.0);
        assert_eq!(snapshot.held_keys, vec![InputKey::Space]);
    }

    #[test]
    fn desktop_snapshot_copies_pending_edges_without_changing_final_levels() {
        let mut edges = InputEdges::default();
        edges.key_transition(InputKey::Space, false, true);
        edges.key_transition(InputKey::Space, true, false);
        edges.mouse_transition(functor_runtime_common::MouseButton::Left, false, true);
        edges.mouse_transition(functor_runtime_common::MouseButton::Left, true, false);

        let snapshot = desktop_input_snapshot(
            &BTreeSet::new(),
            (0, 0),
            MouseButtons::default(),
            &edges,
            false,
            false,
            (800, 600),
            None,
        );
        assert!(snapshot.held_keys.is_empty());
        assert_eq!(snapshot.pressed_keys, vec![InputKey::Space]);
        assert_eq!(snapshot.released_keys, vec![InputKey::Space]);
        assert!(snapshot.mouse.pressed.left);
        assert!(snapshot.mouse.released.left);
        assert!(!snapshot.mouse.buttons.left);
    }

    #[test]
    fn desktop_mouse_sample_tracks_logical_resize_and_injected_position() {
        let before = desktop_input_snapshot(
            &BTreeSet::new(),
            (320, 240),
            MouseButtons::default(),
            &InputEdges::default(),
            false,
            false,
            (800, 600),
            None,
        );
        assert_eq!(before.mouse.surface_width, 800);
        assert_eq!(before.mouse.surface_height, 600);

        // This is the snapshot rebuilt after POST /input mouse_move: the
        // injected x/y changes while the shell-owned logical window extent
        // follows the current resize.
        let after = desktop_input_snapshot(
            &BTreeSet::new(),
            (777, 123),
            MouseButtons::default(),
            &InputEdges::default(),
            false,
            false,
            (1440, 900),
            None,
        );
        assert_eq!(after.mouse.x, 777);
        assert_eq!(after.mouse.y, 123);
        assert_eq!(after.mouse.surface_width, 1440);
        assert_eq!(after.mouse.surface_height, 900);
    }

    #[test]
    fn visible_cursor_floors_fractional_negative_coordinates() {
        assert_eq!(visible_cursor_position(-0.25, 10.75), (-1, 10));
    }

    /// A pose an emulated desktop rig CANNOT produce: the emulator pins both
    /// hands to z = -0.55 with identity orientation, so a hand pulled BACK
    /// toward the face with a rotated grip is exactly the gesture that was
    /// previously untestable without a headset.
    fn injected_sample() -> XrInputSnapshot {
        XrInputSnapshot {
            head: Some(TrackingPose::new([0.0, 0.05, 0.0], [0.0, 0.0, 0.0, 1.0])),
            left: XrControllerSnapshot {
                active: true,
                grip: Some(TrackingPose::new([-0.3, -0.1, -0.6], [0.0, 0.38, 0.0, 0.92])),
                aim: Some(TrackingPose::new([-0.3, -0.1, -0.6], [0.0, 0.38, 0.0, 0.92])),
                ..XrControllerSnapshot::default()
            },
            right: XrControllerSnapshot {
                active: true,
                grip: Some(TrackingPose::new([-0.05, -0.05, 0.12], [0.0, 0.0, 0.0, 1.0])),
                aim: Some(TrackingPose::new([-0.05, -0.05, 0.12], [0.0, 0.0, 0.0, 1.0])),
                trigger: 1.0,
                ..XrControllerSnapshot::default()
            },
        }
    }

    #[test]
    fn an_injected_xr_sample_wins_over_the_desktop_emulator() {
        let injected = injected_sample();
        let held = BTreeSet::from([InputKey::Space]);

        // ... even with --emulate-xr on and a mouse position that would
        // otherwise synthesize a different right hand.
        let snapshot = desktop_input_snapshot(
            &held,
            (700, 100),
            MouseButtons::default(),
            &InputEdges::default(),
            true,
            true,
            (800, 600),
            Some(&injected),
        );
        assert_eq!(snapshot.xr.as_ref(), Some(&injected));
        // Keyboard/mouse domains stay live alongside it.
        assert_eq!(snapshot.held_keys, vec![InputKey::Space]);
        assert_eq!(
            snapshot.mouse,
            MouseSnapshot {
                x: 700,
                y: 100,
                surface_width: 800,
                surface_height: 600,
                ..Default::default()
            }
        );
    }

    #[test]
    fn an_injected_xr_sample_supplies_the_domain_without_emulate_xr() {
        let injected = injected_sample();
        let plain = desktop_input_snapshot(
            &BTreeSet::new(),
            (0, 0),
            MouseButtons::default(),
            &InputEdges::default(),
            false,
            false,
            (800, 600),
            None,
        );
        assert_eq!(plain.xr, None, "no XR domain without a device or injection");

        let snapshot = desktop_input_snapshot(
            &BTreeSet::new(),
            (0, 0),
            MouseButtons::default(),
            &InputEdges::default(),
            false,
            false,
            (800, 600),
            Some(&injected),
        );
        assert_eq!(snapshot.xr.as_ref(), Some(&injected));
    }

    #[test]
    fn fixed_level_refresh_does_not_clobber_an_injected_sample() {
        let injected = injected_sample();
        let mut snapshot = desktop_input_snapshot(
            &BTreeSet::new(),
            (100, 200),
            MouseButtons::default(),
            &InputEdges::default(),
            true,
            false,
            (800, 600),
            Some(&injected),
        );
        // Space would drive the EMULATOR's trigger to 1.0; the injected sample
        // owns the XR domain, so window keys must leave it alone.
        let held = BTreeSet::from([InputKey::Enter]);

        refresh_fixed_input_levels(
            &mut snapshot,
            &held,
            MouseButtons::default(),
            true,
            Some(&injected),
        );

        assert_eq!(snapshot.xr.as_ref(), Some(&injected));
        assert_eq!(snapshot.xr.as_ref().unwrap().right.squeeze, 0.0);
        assert_eq!(snapshot.held_keys, vec![InputKey::Enter]);
    }
}
