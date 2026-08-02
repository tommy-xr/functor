//! The Functor Lang producer for the web shell (docs/functor-lang.md Track C5): the wasm
//! sibling of the desktop runner's `functor_lang_game.rs`, behind the same
//! `GameProducer` seam. Same load-time contract validation and per-frame
//! semantics — sampled input, then the MVU pair (subscriptions fold through
//! `update` before `tick`), the optional `physics` hook (tick → physics →
//! draw), a bad frame keeps the last good model/frame, per-frame errors
//! dedupe — but adapted to the browser:
//!
//! - the `.fun` source arrives over HTTP (fetched by `run_async` from the dev
//!   server, which serves the project directory) instead of the filesystem;
//! - no file-watch hot reload (there is no filesystem to watch), but the
//!   PUSH path exists (docs/functor-lang.md D4): `reload_source` mirrors the desktop
//!   runner's `POST /reload-source` — parse → lower → check-as-warnings →
//!   `Session::load` → `functor_lang::rebind_value` on the held model — reachable
//!   from the page via the `functor_lang_set_source` wasm export in `lib.rs`;
//! - no per-frame perf stats (`std::time::Instant` panics on wasm; the C6
//!   perf gate measures natively);
//! - input events arrive from the page via the `functor_lang_*` wasm exports below,
//!   queued and drained by the frame loop each frame before `tick` (DOM
//!   handlers fire between rAF callbacks, never mid-frame).

use std::cell::RefCell;

use functor_runtime_common::functor_lang_game_embedded::ProducerPlatform;
use functor_runtime_common::protocol::GameProducer;
use wasm_bindgen::prelude::*;

/// The web shell's [`ProducerPlatform`]: it wires the browser diagnostics sinks
/// and drives the page's red draw-error overlay. The producer body itself is the
/// shared `FunctorLangEmbeddedGame` — this is the only web-specific seam (the
/// rest of this file is the page↔producer wasm-bindgen bridge below).
pub struct WebPlatform {
    /// The overlay's shadow state — the last message shown (or `None` if hidden)
    /// — so a persistent error doesn't rewrite the DOM every frame.
    overlay_error: Option<String>,
}

impl WebPlatform {
    pub fn new() -> WebPlatform {
        WebPlatform {
            overlay_error: None,
        }
    }
}

impl Default for WebPlatform {
    fn default() -> Self {
        WebPlatform::new()
    }
}

impl ProducerPlatform for WebPlatform {
    fn install_sinks(&self) {
        // `log::*` has no backend on wasm by default (records are dropped), so
        // install a console bridge — now the shared producer's `log::info/warn/
        // error!` land in the browser console, the web equivalent of the native
        // `log` sink.
        install_console_logger();
        // Route Functor Lang `Debug.log` traces to the browser console (once per
        // process; the process-global sink survives hot-reload). The web runtime
        // has no CLI event stream, so — unlike native — a trace goes straight to
        // `console.log`, the web equivalent of plain `functor-lang run`'s stdout.
        functor_lang::set_trace_sink(Box::new(|message| {
            web_sys::console::log_1(&JsValue::from_str(&message));
        }));
        // Route runtime events to the browser console too. Without a sink they
        // fall back to eprintln!, which goes nowhere in a browser — a failed
        // asset was completely invisible (it just rendered as the fallback).
        functor_runtime_common::events::set_sink(Box::new(|event| {
            use functor_runtime_common::events::RuntimeEvent as R;
            match event {
                R::AssetError { path, message } => {
                    let line = match path {
                        Some(path) => format!(
                            "[functor] asset '{path}' failed to load; using fallback: {message}"
                        ),
                        None => {
                            format!("[functor] asset failed to load; using fallback: {message}")
                        }
                    };
                    web_sys::console::error_1(&JsValue::from_str(&line));
                }
                R::HotReload { ok, message } => {
                    let line = format!("[functor] hot-reload: {message}");
                    if ok {
                        web_sys::console::log_1(&JsValue::from_str(&line));
                    } else {
                        web_sys::console::error_1(&JsValue::from_str(&line));
                    }
                }
                R::FunctorLangTrace { message } => {
                    web_sys::console::log_1(&JsValue::from_str(&message));
                }
                // CLI-stream concerns; quiet in the browser.
                R::Ready | R::FrameStats { .. } | R::CaptureWritten { .. } => {}
            }
        }));
    }

    /// Toggle the page's red draw-error overlay, touching the DOM only when the
    /// state actually changes (draw breaks with a new message, or recovers) so a
    /// persistent error doesn't rewrite the overlay every frame.
    fn set_draw_overlay(&mut self, error: Option<&str>) {
        // Dedupe by borrow: a persistent draw error re-reports the same message
        // every frame, so allocate a stored copy ONLY when the state changes.
        if self.overlay_error.as_deref() == error {
            return;
        }
        match error {
            Some(message) => crate::show_error_overlay(message),
            None => crate::hide_error_overlay(),
        }
        self.overlay_error = error.map(str::to_string);
    }

    fn on_reload(&mut self) {
        // The push path (`functor_lang_set_source`) already hid the overlay in the
        // DOM; clear our shadow so the reloaded program's first draw re-shows it
        // if that program's `draw` still errors.
        self.overlay_error = None;
    }
}

/// A minimal `log` backend that forwards records to the browser console
/// (error → `console.error`, warn → `console.warn`, else → `console.log`) — the
/// web counterpart of the native `log` sink, so the shared producer's `log::*`
/// diagnostics are visible in devtools.
struct ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let line = format!("{}", record.args());
        match record.level() {
            log::Level::Error => web_sys::console::error_1(&JsValue::from_str(&line)),
            log::Level::Warn => web_sys::console::warn_1(&JsValue::from_str(&line)),
            _ => web_sys::console::log_1(&JsValue::from_str(&line)),
        }
    }
    fn flush(&self) {}
}

static CONSOLE_LOGGER: ConsoleLogger = ConsoleLogger;

/// Install the console `log` backend once (a second call — e.g. a second page
/// producer — is a harmless no-op since `set_logger` errors after the first).
fn install_console_logger() {
    if log::set_logger(&CONSOLE_LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

// --- Page → producer input bridge. ------------------------------------------
//
// The Functor Lang game lives *inside* this runtime, so the Functor Lang index page
// (index-functor-lang.html) calls the `functor_lang_*` exports below. Events queue here and the
// frame loop drains them into the producer before each tick.

// The page-input queue carries the SAME plain-data shape the recorder logs, so
// reuse the shared `RecordedInput` (T6b) rather than a parallel private enum —
// `drain_input` dispatches its variants unchanged.
use functor_runtime_common::RecordedInput as InputEvent;

thread_local! {
    static INPUT_QUEUE: RefCell<Vec<InputEvent>> = const { RefCell::new(Vec::new()) };
    static SCHEDULED_INPUT_QUEUE: RefCell<Vec<(u64, InputEvent)>> =
        const { RefCell::new(Vec::new()) };
}

/// Far more events than one frame can produce; if the frame loop never starts
/// (a failed game load leaves the page's handlers wired but nothing
/// draining), the queue must not grow forever.
const INPUT_QUEUE_CAP: usize = 1024;

fn push_input(event: InputEvent) {
    INPUT_QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() < INPUT_QUEUE_CAP {
            q.push(event);
        }
    });
}

/// Deliver a keyboard event (`code` = `functor_runtime_common::Key` as i32).
#[wasm_bindgen]
pub fn functor_lang_key_event(code: i32, is_down: bool) {
    push_input(InputEvent::Key { code, is_down });
}

/// Queue keyboard edges for exact future fixed-step frames. The whole batch is
/// validated before the queue changes, so a rejected script cannot leave a key
/// half-pressed. The landing demo uses this to replay its checked-in input
/// script without depending on browser refresh cadence; ordinary interactive
/// input keeps using [`functor_lang_key_event`].
#[wasm_bindgen]
pub fn functor_lang_schedule_key_events(
    frames: &[f64],
    codes: &[i32],
    is_down: &[u8],
) -> bool {
    if frames.is_empty() || frames.len() != codes.len() || frames.len() != is_down.len() {
        return false;
    }
    let mut parsed = Vec::with_capacity(frames.len());
    for ((frame, code), is_down) in frames.iter().zip(codes).zip(is_down) {
        if !frame.is_finite()
            || *frame < 0.0
            || frame.fract() != 0.0
            || *frame >= u64::MAX as f64
            || *is_down > 1
        {
            return false;
        }
        parsed.push((
            *frame as u64,
            InputEvent::Key {
                code: *code,
                is_down: *is_down == 1,
            },
        ));
    }

    SCHEDULED_INPUT_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        if parsed.len() > INPUT_QUEUE_CAP.saturating_sub(queue.len()) {
            return false;
        }
        queue.extend(parsed);
        queue.sort_by_key(|(frame, _)| *frame);
        true
    })
}

/// Move the keyboard edges assigned to `frame` into the ordinary page-input
/// queue immediately before that fixed step. Stale edges are discarded rather
/// than bunched into a later frame. Once queued they are ordinary page input,
/// so a PINNED frame (paused, or `?fixed-time`) drains and drops them exactly
/// like everything else — the caller's `drain_input` decides that.
pub fn queue_scheduled_input_at(frame: u64) -> bool {
    SCHEDULED_INPUT_QUEUE.with(|scheduled| {
        let mut scheduled = scheduled.borrow_mut();
        let first_future = scheduled.partition_point(|(target, _)| *target <= frame);
        if first_future == 0 {
            return false;
        }
        let due: Vec<InputEvent> = scheduled
            .drain(..first_future)
            .filter_map(|(target, event)| (target == frame).then_some(event))
            .collect();
        if due.is_empty() {
            return false;
        }
        // Respect the page queue's cap the same way `push_input` does, so a
        // scheduled batch can never push it past the bound.
        INPUT_QUEUE.with(|queue| {
            let mut queue = queue.borrow_mut();
            let room = INPUT_QUEUE_CAP.saturating_sub(queue.len());
            queue.extend(due.into_iter().take(room));
        });
        true
    })
}

/// Deliver a mouse position in logical CSS pixels for visible-pointer games.
/// Captured games accumulate pointer-lock movement deltas, matching the
/// desktop's relative free-look path.
#[wasm_bindgen]
pub fn functor_lang_mouse_move(x: i32, y: i32) {
    push_input(InputEvent::MouseMove { x, y });
}

/// Deliver a mouse-wheel event (vertical scroll offset, ±1 per notch).
#[wasm_bindgen]
pub fn functor_lang_mouse_wheel(delta: i32) {
    push_input(InputEvent::MouseWheel { delta });
}

/// Deliver a mouse-button edge (`button` = `functor_runtime_common::MouseButton`
/// as i32). The page maps the browser's `MouseEvent.button` before calling.
#[wasm_bindgen]
pub fn functor_lang_mouse_button(button: i32, is_down: bool) {
    push_input(InputEvent::MouseButton { button, is_down });
}

/// One touch-contact transition from the page's touch listeners. `phase` is
/// 0=begin, 1=move, 2=end, 3=cancel (anything else is dropped); `id` is the
/// page-assigned small ordinal (the page remaps the browser's arbitrary
/// `Touch.identifier`s); `x`/`y` are CSS pixels relative to the canvas.
#[wasm_bindgen]
pub fn functor_lang_touch_event(phase: u32, id: u32, x: f32, y: f32) {
    let phase = match phase {
        0 => functor_runtime_common::TouchPhase::Begin,
        1 => functor_runtime_common::TouchPhase::Move,
        2 => functor_runtime_common::TouchPhase::End,
        3 => functor_runtime_common::TouchPhase::Cancel,
        _ => return,
    };
    push_input(InputEvent::Touch { phase, id, x, y });
}

thread_local! {
    /// The page's UNLOCKED pointer over the canvas — `(pos in CSS px,
    /// primary button down, press latched since last sample)` — for the
    /// interactive game-UI overlay (docs/ui-interaction.md U3). Separate from
    /// the captured relative-mouse path above: while locked there is no
    /// cursor to point at widgets with (`pos` is `None`). Level state plus a
    /// press LATCH: a mousedown+mouseup landing between two rAF frames would
    /// otherwise sample as never-pressed and the click would be lost — the
    /// latch keeps the sampled level down for one frame so egui sees the
    /// press edge (the release follows next frame).
    static UI_POINTER: std::cell::Cell<(Option<(f32, f32)>, bool, bool)> =
        const { std::cell::Cell::new((None, false, false)) };
}

/// Deliver the unlocked pointer's canvas position (CSS px, e.g. `offsetX/Y`)
/// and primary-button state. Called by the page's mousemove/mousedown/mouseup
/// handlers while pointer lock is NOT engaged.
#[wasm_bindgen]
pub fn functor_lang_ui_pointer(x: f32, y: f32, primary_down: bool) {
    UI_POINTER.with(|p| {
        let (_, was_down, clicked) = p.get();
        // Latch the press EDGE (not the held level) so a sub-frame click
        // survives to the next sample without pinning held state forever.
        p.set((
            Some((x, y)),
            primary_down,
            clicked || (primary_down && !was_down),
        ));
    });
}

/// The pointer left the canvas (or pointer lock engaged). The page clears its
/// own button state on leave, so mirror it — a press begun off-canvas must
/// not replay as a click on re-entry (the bridge's swallow rule; a press held
/// across the leave is released by the bridge at its last position).
#[wasm_bindgen]
pub fn functor_lang_ui_pointer_leave() {
    UI_POINTER.with(|p| {
        let (_, _, clicked) = p.get();
        p.set((None, false, clicked));
    });
}

/// This frame's pointer for the overlay pass, scaled from the page's CSS px
/// to framebuffer px (`dpr` — the overlay runs at the device pixel ratio).
/// Consumes the press latch: a latched sub-frame click samples as down once.
pub fn ui_pointer_state(dpr: f32) -> functor_runtime_common::ui::PointerState {
    UI_POINTER.with(|p| {
        let (pos, primary_down, clicked) = p.get();
        p.set((pos, primary_down, false));
        functor_runtime_common::ui::PointerState {
            pos: pos.map(|(x, y)| (x * dpr, y * dpr)),
            primary_down: primary_down || clicked,
        }
    })
}

thread_local! {
    /// Keyboard events queued for a focused `Ui.textInput`
    /// (docs/ui-interaction.md U4). The page routes keydowns here (instead
    /// of the game key path) while [`functor_lang_ui_wants_keyboard`] reports
    /// true; the frame loop drains it into the overlay pass. Same cap
    /// rationale as [`INPUT_QUEUE`].
    static UI_KEY_QUEUE: RefCell<Vec<functor_runtime_common::ui::UiKeyboardEvent>> =
        const { RefCell::new(Vec::new()) };
    /// Whether the overlay wanted the keyboard after the LAST frame's pass —
    /// what the page's keydown handler polls to pick a route.
    static UI_WANTS_KEYBOARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether the overlay wanted primary-pointer input after the last pass.
    /// Visible-pointer games use this to keep a widget click from also
    /// reaching the game's `mouseButton` hook.
    static UI_WANTS_POINTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Deliver a keydown for a focused text field. `key` is the DOM
/// `KeyboardEvent.key`: a single-char string is printable text; the named
/// editing keys map across; anything else is dropped (F-keys, media keys).
/// Returns whether the key was CONSUMED — the page only `preventDefault()`s
/// then, so browser chrome (F5, DevTools) keeps working while typing.
#[wasm_bindgen]
pub fn functor_lang_ui_key(key: &str) -> bool {
    use functor_runtime_common::ui::{UiEditKey, UiKeyboardEvent};
    let mut chars = key.chars();
    let event = match (chars.next(), chars.next()) {
        // Exactly one char → printable text ("a", "3", "`", …).
        (Some(c), None) => Some(UiKeyboardEvent::Char(c)),
        _ => match key {
            "Backspace" => Some(UiKeyboardEvent::Edit(UiEditKey::Backspace)),
            "Delete" => Some(UiKeyboardEvent::Edit(UiEditKey::Delete)),
            "ArrowLeft" => Some(UiKeyboardEvent::Edit(UiEditKey::Left)),
            "ArrowRight" => Some(UiKeyboardEvent::Edit(UiEditKey::Right)),
            "Home" => Some(UiKeyboardEvent::Edit(UiEditKey::Home)),
            "End" => Some(UiKeyboardEvent::Edit(UiEditKey::End)),
            "Enter" => Some(UiKeyboardEvent::Edit(UiEditKey::Enter)),
            "Escape" => Some(UiKeyboardEvent::Edit(UiEditKey::Escape)),
            _ => None,
        },
    };
    match event {
        Some(event) => {
            UI_KEY_QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                if q.len() < INPUT_QUEUE_CAP {
                    q.push(event);
                }
            });
            true
        }
        None => false,
    }
}

/// Whether a `Ui.textInput` is focused — the page's keydown handler routes
/// keys to [`functor_lang_ui_key`] while this is true, and to the game's
/// key path otherwise (the focus gate, docs/ui-interaction.md U4).
#[wasm_bindgen]
pub fn functor_lang_ui_wants_keyboard() -> bool {
    UI_WANTS_KEYBOARD.with(|w| w.get())
}

/// Whether the game UI wanted pointer input after its last render.
#[wasm_bindgen]
pub fn functor_lang_ui_wants_pointer() -> bool {
    UI_WANTS_POINTER.with(|w| w.get())
}

/// Drain the focused-field key queue (the frame loop, before the overlay
/// pass). `deliver: false` (pinned clock) discards — typing is inert while
/// pinned, like all other window input.
pub fn drain_ui_keys(deliver: bool) -> Vec<functor_runtime_common::ui::UiKeyboardEvent> {
    let events = UI_KEY_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if deliver {
        events
    } else {
        Vec::new()
    }
}

/// Publish this frame's `wants_keyboard` for the page's keydown routing.
pub fn set_ui_wants_keyboard(wants: bool) {
    UI_WANTS_KEYBOARD.with(|w| w.set(wants));
}

pub fn set_ui_wants_pointer(wants: bool) {
    UI_WANTS_POINTER.with(|w| w.set(wants));
}

/// Drain the queued page input into the producer, in arrival order. Called by
/// the frame loop before `tick`. Empty (and free) on the F# path — its page
/// never calls the `functor_lang_*` exports.
///
/// Admitted transitions accumulate in `edges` until the frame loop consumes
/// its next fixed step. When `deliver` is false, the queue is drained without
/// changing the sampled pointer or admitting new presses. A suppressed
/// interactive transport (pause) may set `recover_releases`:
/// releases then update physical held state so a key cannot stick on resume.
/// Fixed-time capture leaves the entire snapshot frozen.
pub fn drain_input(
    game: &mut dyn GameProducer,
    snapshot: &mut functor_runtime_common::InputSnapshot,
    edges: &mut functor_runtime_common::InputEdges,
    deliver: bool,
    recover_releases: bool,
) {
    let events = INPUT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for event in events {
        match event {
            InputEvent::Key { code, is_down } => {
                if let Some(key) = functor_runtime_common::Key::from_i32(code)
                    .filter(|key| *key != functor_runtime_common::Key::Unknown)
                {
                    if is_down && deliver {
                        functor_runtime_common::apply_key_transition_to_snapshot(
                            snapshot, edges, key, true, true,
                        );
                    } else if !is_down && (deliver || recover_releases) {
                        functor_runtime_common::apply_key_transition_to_snapshot(
                            snapshot, edges, key, false, deliver,
                        );
                    }
                }
                if deliver {
                    game.key_event(code, is_down);
                }
            }
            InputEvent::MouseMove { x, y } => {
                if deliver {
                    // x/y only — a move must not clear the held buttons.
                    snapshot.mouse.x = x;
                    snapshot.mouse.y = y;
                    game.mouse_move(x, y);
                }
            }
            InputEvent::MouseWheel { delta } => {
                if deliver {
                    game.mouse_wheel(delta);
                }
            }
            InputEvent::MouseButton { button, is_down } => {
                // Same level/edge split as keys: the held set tracks physical
                // state (so a release while paused can't stick), the hook only
                // sees edges we actually deliver.
                if let Some(mapped) = functor_runtime_common::MouseButton::from_i32(button)
                    .filter(|b| *b != functor_runtime_common::MouseButton::Unknown)
                {
                    if is_down && deliver {
                        functor_runtime_common::apply_mouse_transition_to_snapshot(
                            snapshot, edges, mapped, true, true,
                        );
                    } else if !is_down && (deliver || recover_releases) {
                        functor_runtime_common::apply_mouse_transition_to_snapshot(
                            snapshot, edges, mapped, false, deliver,
                        );
                    }
                }
                if deliver {
                    game.mouse_button(button, is_down);
                }
            }
            InputEvent::Touch { phase, id, x, y } => {
                let point = functor_runtime_common::TouchPoint { id, x, y };
                match phase {
                    functor_runtime_common::TouchPhase::Begin
                    | functor_runtime_common::TouchPhase::Move => {
                        if deliver {
                            functor_runtime_common::apply_touch_transition(
                                &mut snapshot.touch,
                                edges,
                                phase,
                                point,
                            );
                        }
                    }
                    functor_runtime_common::TouchPhase::End
                    | functor_runtime_common::TouchPhase::Cancel => {
                        if deliver {
                            functor_runtime_common::apply_touch_transition(
                                &mut snapshot.touch,
                                edges,
                                phase,
                                point,
                            );
                        } else if recover_releases {
                            // Physical recovery while input is suppressed
                            // (paused/pinned): the level must clear so a
                            // contact lifted during a scrub can't stick, but
                            // no model-visible edge — the game never saw the
                            // suppressed press either. The key/mouse
                            // recovery rule, applied at the shell (the
                            // reducer itself has no silent mode).
                            if let Some(touch) = snapshot.touch.as_mut() {
                                touch.touches.retain(|p| p.id != id);
                            }
                        }
                    }
                }
            }
            // Only the time-travel recorder creates snapshots; the page input
            // bridge cannot enqueue one.
            InputEvent::Snapshot(_) => {}
            InputEvent::UiEvent(event) => {
                if deliver {
                    game.ui_event(event);
                }
            }
            // Webview interactions arrive through their own queue
            // (`take_webview_events`, drained by the frame loop), not the page
            // input queue — but the shared enum makes the arm total.
            InputEvent::WebviewEvent(event) => {
                if deliver {
                    game.webview_event(event);
                }
            }
        }
    }
}

// --- Time-travel scrubber ↔ DOM bridge (docs/time-travel.md T3) -------------
//
// On web the scrubber is NATIVE DOM (index-functor-lang.html), not egui-in-canvas, so
// its widgets sit OUTSIDE the game canvas — their clicks never reach the canvas
// (no pointer-lock clash) and they render as accessible browser controls. The
// page calls the `functor_lang_scrub_*` write exports (queued here, applied by the frame
// loop, which owns the clock) and polls the read exports each frame; the loop
// publishes the current view state. The coupled-rewind LOGIC stays shared
// (`SceneRecorder`); only the UI surface differs from desktop.

/// A control from the DOM scrubber, applied by the frame loop.
pub enum ScrubControl {
    TogglePause,
    ToggleDetachedCamera,
    SetDebugCameraMode(u32),
    SetDebugCameraFov(f32),
    SetDebugMaterial(u32),
    SetDebugPhysics(bool),
    SetAuthoredCameraFrustum(bool),
    SetGameUiVisible(bool),
    ResetDebugCamera,
    MoveDetachedCamera {
        forward: f32,
        right: f32,
        vertical: f32,
        elapsed_seconds: f32,
    },
    Step,
    SeekTo {
        frame: u64,
        request_id: u32,
    },
    /// Future-preview mode (docs/time-travel.md T6), pushed by the
    /// `window.__scrub` seam (PreviewMode wire index: 0 off / 1 trail /
    /// 2 strobe / 3 both). The frame loop owns the preview state.
    SetPreview(u32),
    /// The shared forward window (seconds) + samples-per-second rate, pushed by
    /// the rail's endpoint handle and the `window.__scrub` seam.
    SetPreviewConfig {
        window: f32,
        rate: usize,
    },
    /// PIN the preview overlay's presence (0 = faded out, 1 = fully drawn),
    /// skipping the runtime's fade ease — deterministic mid-fade captures and
    /// demo scripts. A NEGATIVE value clears the pin and resumes easing (the
    /// same convention as the desktop `--preview-presence` flag).
    SetPreviewPresence(f32),
}

#[derive(Default)]
pub struct DebugCameraMotion {
    pub look_dx: f32,
    pub look_dy: f32,
    pub zoom_steps: f32,
}

thread_local! {
    static SCRUB_CONTROLS: RefCell<Vec<ScrubControl>> = const { RefCell::new(Vec::new()) };
    // Relative pointer motion is high-frequency and order-independent within a
    // rendered frame. Coalesce it separately so a high-polling mouse cannot
    // starve discrete pause/seek/detach controls in the bounded timeline queue.
    static DEBUG_CAMERA_MOTION: RefCell<DebugCameraMotion> =
        RefCell::new(DebugCameraMotion::default());
    /// Published each frame for the page's slider:
    /// `(frame, lo, hi, paused, history generation)`.
    /// `frame`/`lo`/`hi` are `-1.0` when nothing is recorded yet.
    static SCRUB_VIEW: RefCell<(f64, f64, f64, bool, u64)> =
        const { RefCell::new((-1.0, -1.0, -1.0, false, 0)) };
    /// `(active, completed toggle generation)`. The generation acknowledges
    /// both successful and refused requests so DOM pointer lock can reconcile.
    static DETACHED_CAMERA_VIEW: RefCell<(bool, u32)> = const { RefCell::new((false, 0)) };
    /// `(mode, material, physics, frustum, game UI, FOV degrees, 2D zoom)`.
    /// Missing/not-applicable numeric values use `-1`; mode uses `u32::MAX`.
    static DEBUG_CAMERA_VIEW: RefCell<(u32, u32, bool, bool, bool, f32, f32)> =
        const { RefCell::new((u32::MAX, 0, false, false, true, -1.0, -1.0)) };
    /// Latest completed seek as `(request id, authoritative applied frame)`.
    /// The DOM uses this acknowledgement to retire optimistic handle state even
    /// when the runtime clamps or refuses a request.
    static SCRUB_SEEK_RESULT: RefCell<Option<(u32, f64)>> = const { RefCell::new(None) };
}

const SCRUB_CONTROLS_CAP: usize = 256;

#[derive(Clone)]
struct TimelineMarker {
    id: u64,
    frame: u64,
    kind: &'static str,
    label: String,
}

#[derive(Default)]
struct TimelineLog {
    markers: Vec<TimelineMarker>,
    next_id: u64,
    input_cursor: Option<u64>,
    input_generation: Option<u64>,
    cached_json: String,
    dirty: bool,
    revision: u32,
}

impl TimelineLog {
    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    fn push(&mut self, frame: u64, kind: &'static str, label: String) {
        self.markers.push(TimelineMarker {
            id: self.next_id,
            frame,
            kind,
            label,
        });
        self.next_id += 1;
        const CAP: usize = 4096;
        if self.markers.len() > CAP {
            self.markers.drain(..self.markers.len() - CAP);
        }
        self.changed();
    }

    fn retain_range(&mut self, lo: u64, hi: u64) {
        let old_len = self.markers.len();
        self.markers
            .retain(|marker| marker.frame >= lo && marker.frame <= hi);
        if self.markers.len() != old_len {
            self.changed();
        }
    }

    fn truncate_from(&mut self, frame: u64) {
        let old_len = self.markers.len();
        self.markers.retain(|marker| marker.frame < frame);
        if self.markers.len() != old_len {
            self.changed();
        }
    }

    fn reset_inputs(&mut self) {
        let old_len = self.markers.len();
        self.markers
            .retain(|marker| marker.kind.starts_with("reload-"));
        if self.markers.len() != old_len {
            self.changed();
        }
        self.input_cursor = None;
    }

    fn json(&mut self) -> &str {
        if self.dirty || self.cached_json.is_empty() {
            let markers: Vec<_> = self
                .markers
                .iter()
                .map(|marker| {
                    serde_json::json!({
                        "id": marker.id,
                        "frame": marker.frame,
                        "kind": marker.kind,
                        "label": marker.label,
                    })
                })
                .collect();
            self.cached_json = serde_json::to_string(&markers).unwrap_or_else(|_| "[]".to_string());
            self.dirty = false;
        }
        &self.cached_json
    }
}

thread_local! {
    static TIMELINE_LOG: RefCell<TimelineLog> = RefCell::new(TimelineLog::default());
}

fn input_marker(input: &InputEvent) -> Option<(&'static str, String)> {
    match input {
        InputEvent::Key { code, is_down } => {
            let name = functor_runtime_common::Key::from_i32(*code)
                .map(|key| key.name())
                .unwrap_or_else(|| format!("key {code}"));
            let edge = if *is_down { "down" } else { "up" };
            Some((
                if *is_down { "key-down" } else { "key-up" },
                format!("{name} {edge}"),
            ))
        }
        InputEvent::MouseMove { x, y } => Some(("mouse-move", format!("mouse move ({x}, {y})"))),
        InputEvent::MouseWheel { delta } => Some(("mouse-wheel", format!("mouse wheel {delta:+}"))),
        InputEvent::MouseButton { button, is_down } => {
            let name = functor_runtime_common::MouseButton::from_i32(*button)
                .map(|b| b.name())
                .unwrap_or_else(|| format!("button {button}"));
            let edge = if *is_down { "down" } else { "up" };
            Some((
                if *is_down {
                    "mouse-button-down"
                } else {
                    "mouse-button-up"
                },
                format!("mouse {name} {edge}"),
            ))
        }
        InputEvent::Touch { phase, id, .. } => Some((
            match phase {
                functor_runtime_common::TouchPhase::Begin => "touch-begin",
                functor_runtime_common::TouchPhase::Move => "touch-move",
                functor_runtime_common::TouchPhase::End
                | functor_runtime_common::TouchPhase::Cancel => "touch-end",
            },
            format!("touch {id} {phase:?}"),
        )),
        InputEvent::Snapshot(_) => None,
        InputEvent::UiEvent(event) => Some(("ui-input", format!("UI {event:?}"))),
        InputEvent::WebviewEvent(event) => Some(("webview-input", format!("webview {event:?}"))),
    }
}

/// Copy newly recorded inputs into the DOM timeline's compact marker log. The
/// producer's recorder is authoritative: inputs discarded while paused never
/// appear, and replayable inputs land on their exact rendered frame.
pub fn publish_timeline_inputs(game: &dyn GameProducer) {
    let Some((lo, hi)) = game.scene_frame_range() else {
        return;
    };
    TIMELINE_LOG.with(|log| {
        let mut log = log.borrow_mut();
        let generation = game.scene_timeline_generation();
        if log.input_generation != Some(generation) {
            // A branch can replace frames without making `hi` move backward
            // (for example, one frame from the old tail). Rebuild input markers
            // from the authoritative recorder instead of inferring from range.
            log.reset_inputs();
            log.input_generation = Some(generation);
        }
        if log.input_cursor.is_some_and(|cursor| cursor > hi) {
            // A resumed scrub rewrote `hi`, the first frame on the new branch.
            // Drop the discarded branch including its stale marker at `hi`,
            // then rescan that authoritative replacement frame below.
            log.truncate_from(hi);
            log.input_cursor = hi.checked_sub(1);
        }
        let start = log
            .input_cursor
            .map_or(lo, |cursor| cursor.saturating_add(1).max(lo));
        if start <= hi {
            for frame in start..=hi {
                let mut last_mouse_move = None;
                for input in game.recorded_inputs_at(frame) {
                    if matches!(&input, InputEvent::MouseMove { .. }) {
                        // Pointer-lock mouselook can emit several moves per
                        // rendered frame. One marker still says "input here"
                        // without multiplying bridge payload and DOM hits.
                        last_mouse_move = Some(input);
                        continue;
                    }
                    if let Some((kind, label)) = input_marker(&input) {
                        log.push(frame, kind, label);
                    }
                }
                if let Some(input) = last_mouse_move {
                    if let Some((kind, label)) = input_marker(&input) {
                        log.push(frame, kind, label);
                    }
                }
            }
        }
        log.input_cursor = Some(hi);
        log.retain_range(lo, hi);
    });
}

/// Record a reload boundary at the scene frame that remained current through
/// the swap. Failures mark the attempted boundary without changing the program.
pub fn publish_timeline_reload(frame: u64, ok: bool, message: &str) {
    TIMELINE_LOG.with(|log| {
        log.borrow_mut().push(
            frame,
            if ok { "reload-ok" } else { "reload-error" },
            if ok {
                "hot reload".to_string()
            } else {
                format!("reload failed: {message}")
            },
        );
    });
}

/// Runtime → page: JSON array of `{id, frame, kind, label}` markers.
#[wasm_bindgen]
pub fn functor_lang_timeline_events() -> String {
    TIMELINE_LOG.with(|log| log.borrow_mut().json().to_string())
}

/// Runtime → page: cheap marker-log revision. The DOM fetches the JSON only
/// when this changes instead of cloning it over the WASM boundary every rAF.
#[wasm_bindgen]
pub fn functor_lang_timeline_events_gen() -> u32 {
    TIMELINE_LOG.with(|log| log.borrow().revision)
}

thread_local! {
    /// The current webview HTML and a cheap revision counter — the page's
    /// poll loop fetches the string only when the revision changes (the
    /// timeline-events pattern). Empty string = no webview.
    static WEBVIEW_HTML: RefCell<(String, u32)> = const { RefCell::new((String::new(), 0)) };
    /// Interactions the page's DOM listeners queued since last frame —
    /// drained by the frame loop into `GameProducer::webview_event`.
    static WEBVIEW_EVENTS: RefCell<Vec<functor_runtime_common::ui::UiEvent>> =
        const { RefCell::new(Vec::new()) };
}

/// Publish this frame's webview HTML (None = no `webview` hook). The DOM
/// overlay polls the revision and re-reads `innerHTML` only on change.
pub fn publish_webview_html(html: Option<String>) {
    let html = html.unwrap_or_default();
    WEBVIEW_HTML.with(|v| {
        let mut v = v.borrow_mut();
        if v.0 != html {
            v.0 = html;
            v.1 = v.1.wrapping_add(1);
        }
    });
}

/// Runtime → page: the webview HTML for the overlay div.
#[wasm_bindgen]
pub fn functor_lang_webview_html() -> String {
    WEBVIEW_HTML.with(|v| v.borrow().0.clone())
}

/// Runtime → page: cheap webview revision (the timeline-events pattern).
#[wasm_bindgen]
pub fn functor_lang_webview_gen() -> u32 {
    WEBVIEW_HTML.with(|v| v.borrow().1)
}

/// Page → runtime: a click on a webview element carrying `data-fn-click`.
#[wasm_bindgen]
pub fn functor_lang_webview_click(slot: u32) {
    WEBVIEW_EVENTS.with(|q| {
        q.borrow_mut().push(functor_runtime_common::ui::UiEvent {
            slot,
            kind: functor_runtime_common::ui::UiEventKind::Clicked,
        })
    });
}

/// Page → runtime: an edit in a webview `<input>` carrying `data-fn-input`.
#[wasm_bindgen]
pub fn functor_lang_webview_input(slot: u32, value: &str) {
    WEBVIEW_EVENTS.with(|q| {
        q.borrow_mut().push(functor_runtime_common::ui::UiEvent {
            slot,
            kind: functor_runtime_common::ui::UiEventKind::TextChanged(value.to_string()),
        })
    });
}

/// Drain the interactions the page queued since last frame.
pub fn take_webview_events() -> Vec<functor_runtime_common::ui::UiEvent> {
    WEBVIEW_EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

fn push_scrub(control: ScrubControl) -> bool {
    SCRUB_CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() < SCRUB_CONTROLS_CAP {
            c.push(control);
            true
        } else {
            false
        }
    })
}

/// Drain the queued scrubber controls; the frame loop applies them (it owns the
/// clock pin and the game).
pub fn take_scrub_controls() -> Vec<ScrubControl> {
    SCRUB_CONTROLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

pub fn take_debug_camera_motion() -> DebugCameraMotion {
    DEBUG_CAMERA_MOTION.with(|motion| std::mem::take(&mut *motion.borrow_mut()))
}

/// Publish this frame's scrubber state for the page to poll.
pub fn publish_scrub_view(
    frame: Option<u64>,
    range: Option<(u64, u64)>,
    paused: bool,
    generation: u64,
) {
    let f = frame.map(|f| f as f64).unwrap_or(-1.0);
    let (lo, hi) = range
        .map(|(l, h)| (l as f64, h as f64))
        .unwrap_or((-1.0, -1.0));
    SCRUB_VIEW.with(|v| *v.borrow_mut() = (f, lo, hi, paused, generation));
}

pub fn publish_detached_camera(active: bool) {
    DETACHED_CAMERA_VIEW.with(|view| view.borrow_mut().0 = active);
}

pub fn acknowledge_detached_camera(active: bool) {
    DETACHED_CAMERA_VIEW.with(|view| {
        let mut view = view.borrow_mut();
        view.0 = active;
        view.1 = view.1.wrapping_add(1);
    });
}

pub fn publish_debug_camera_view(
    mode: Option<functor_runtime_common::viewer::DebugCameraMode>,
    presentation: functor_runtime_common::viewer::DebugPresentation,
    fov_degrees: Option<f32>,
    zoom_2d: Option<f32>,
) {
    DEBUG_CAMERA_VIEW.with(|view| {
        *view.borrow_mut() = (
            mode.map(|mode| mode.index()).unwrap_or(u32::MAX),
            presentation.material.index(),
            presentation.physics,
            presentation.authored_camera_frustum,
            presentation.show_game_ui,
            fov_degrees.unwrap_or(-1.0),
            zoom_2d.unwrap_or(-1.0),
        );
    });
}

/// Page → runtime: toggle pause (pin/unpin the clock).
#[wasm_bindgen]
pub fn functor_lang_scrub_toggle_pause() {
    push_scrub(ScrubControl::TogglePause);
}

/// Page → runtime: snapshot or discard the shell-owned debug view.
#[wasm_bindgen]
pub fn functor_lang_viewer_toggle_detached() {
    if !push_scrub(ScrubControl::ToggleDetachedCamera) {
        // The DOM disables the control until this generation advances. Refuse
        // a saturated request explicitly so pointer lock and the retry button
        // reconcile instead of waiting forever for a command we dropped.
        let active = DETACHED_CAMERA_VIEW.with(|view| view.borrow().0);
        acknowledge_detached_camera(active);
    }
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_mode(mode: u32) {
    if functor_runtime_common::viewer::DebugCameraMode::from_index(mode).is_some() {
        push_scrub(ScrubControl::SetDebugCameraMode(mode));
    }
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_fov(degrees: f32) {
    if degrees.is_finite() {
        push_scrub(ScrubControl::SetDebugCameraFov(degrees));
    }
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_material(material: u32) {
    if functor_runtime_common::viewer::DebugMaterialMode::from_index(material).is_some() {
        push_scrub(ScrubControl::SetDebugMaterial(material));
    }
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_physics(enabled: bool) {
    push_scrub(ScrubControl::SetDebugPhysics(enabled));
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_authored_frustum(enabled: bool) {
    push_scrub(ScrubControl::SetAuthoredCameraFrustum(enabled));
}

#[wasm_bindgen]
pub fn functor_lang_viewer_set_game_ui(visible: bool) {
    push_scrub(ScrubControl::SetGameUiVisible(visible));
}

#[wasm_bindgen]
pub fn functor_lang_viewer_reset() {
    push_scrub(ScrubControl::ResetDebugCamera);
}

/// Page → runtime: pointer-lock relative debug-camera look/pan motion.
#[wasm_bindgen]
pub fn functor_lang_viewer_look(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    DEBUG_CAMERA_MOTION.with(|motion| {
        let mut motion = motion.borrow_mut();
        motion.look_dx = (motion.look_dx + dx).clamp(-1_000_000.0, 1_000_000.0);
        motion.look_dy = (motion.look_dy + dy).clamp(-1_000_000.0, 1_000_000.0);
    });
}

/// Page → runtime: debug-camera WASD + Q/E movement.
#[wasm_bindgen]
pub fn functor_lang_viewer_move(
    forward: f32,
    right: f32,
    vertical: f32,
    elapsed_seconds: f32,
) {
    push_scrub(ScrubControl::MoveDetachedCamera {
        forward,
        right,
        vertical,
        elapsed_seconds,
    });
}

/// Page → runtime: wheel zoom; positive steps zoom in.
#[wasm_bindgen]
pub fn functor_lang_viewer_zoom(steps: f32) {
    if !steps.is_finite() {
        return;
    }
    DEBUG_CAMERA_MOTION.with(|motion| {
        let mut motion = motion.borrow_mut();
        motion.zoom_steps = (motion.zoom_steps + steps).clamp(-1_000.0, 1_000.0);
    });
}

/// Runtime → page: whether the shell-owned debug view is active.
#[wasm_bindgen]
pub fn functor_lang_viewer_detached() -> bool {
    DETACHED_CAMERA_VIEW.with(|view| view.borrow().0)
}

/// Runtime → page: increments after every detach/reattach request is applied
/// or refused.
#[wasm_bindgen]
pub fn functor_lang_viewer_detached_generation() -> u32 {
    DETACHED_CAMERA_VIEW.with(|view| view.borrow().1)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_mode() -> u32 {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().0)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_material() -> u32 {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().1)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_physics() -> bool {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().2)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_authored_frustum() -> bool {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().3)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_game_ui() -> bool {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().4)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_fov() -> f32 {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().5)
}

#[wasm_bindgen]
pub fn functor_lang_viewer_zoom_2d() -> f32 {
    DEBUG_CAMERA_VIEW.with(|view| view.borrow().6)
}

/// Page → runtime: advance exactly one frame, then hold.
#[wasm_bindgen]
pub fn functor_lang_scrub_step() {
    push_scrub(ScrubControl::Step);
}

/// Page → runtime: set the future-preview mode (the `window.__scrub` seam;
/// 0 off / 1 trail / 2 strobe / 3 both — `PreviewMode::from_index`).
#[wasm_bindgen]
pub fn functor_lang_scrub_set_preview(mode: u32) {
    push_scrub(ScrubControl::SetPreview(mode));
}

/// Page → runtime: set the preview's shared forward window (seconds) and
/// samples-per-second rate (pushed by the rail's endpoint handle and the
/// `window.__scrub` seam).
#[wasm_bindgen]
pub fn functor_lang_scrub_set_preview_config(window: f32, rate: usize) {
    push_scrub(ScrubControl::SetPreviewConfig { window, rate });
}

/// Page → runtime: pin the preview overlay's presence, skipping the fade ease
/// (`window.__scrub.setPreview({ presence })`). 0 = faded out, 1 = fully drawn;
/// a NEGATIVE value clears the pin and resumes the runtime's easing.
#[wasm_bindgen]
pub fn functor_lang_scrub_set_preview_presence(presence: f32) {
    if !presence.is_finite() {
        return;
    }
    push_scrub(ScrubControl::SetPreviewPresence(presence));
}

/// Page → runtime: non-destructively scrub to a rendered frame (slider drag).
#[wasm_bindgen]
pub fn functor_lang_seek_scene(frame: f64, request_id: u32) {
    if frame >= 0.0 {
        push_scrub(ScrubControl::SeekTo {
            frame: frame as u64,
            request_id,
        });
    }
}

/// Publish a completed seek's authoritative frame for the DOM's optimistic
/// state reconciler. Kept separate from [`SCRUB_VIEW`] so ordinary playback
/// publications do not masquerade as seek acknowledgements.
pub fn publish_scrub_seek_result(request_id: u32, frame: Option<u64>) {
    SCRUB_SEEK_RESULT.with(|result| {
        *result.borrow_mut() = Some((request_id, frame.map_or(-1.0, |frame| frame as f64)));
    });
}

/// Runtime → page: latest `[requestId, appliedFrame]`, or `[]` before any seek.
#[wasm_bindgen]
pub fn functor_lang_scrub_seek_result() -> Vec<f64> {
    SCRUB_SEEK_RESULT.with(|result| {
        result
            .borrow()
            .map(|(request_id, frame)| vec![request_id as f64, frame])
            .unwrap_or_default()
    })
}

/// Runtime → page: the current handle frame (`-1` if nothing recorded).
#[wasm_bindgen]
pub fn functor_lang_scene_frame() -> f64 {
    SCRUB_VIEW.with(|v| v.borrow().0)
}

/// Runtime → page: the seekable window as `[lo, hi]`, or `[]` if empty.
#[wasm_bindgen]
pub fn functor_lang_scene_range() -> Vec<f64> {
    let (_, lo, hi, _, _) = SCRUB_VIEW.with(|v| *v.borrow());
    if lo < 0.0 {
        vec![]
    } else {
        vec![lo, hi]
    }
}

/// Runtime → page: whether the clock is currently pinned.
#[wasm_bindgen]
pub fn functor_lang_scrub_paused() -> bool {
    SCRUB_VIEW.with(|v| v.borrow().3)
}

/// Runtime → page: current seekable-history generation.
#[wasm_bindgen]
pub fn functor_lang_scene_generation() -> f64 {
    SCRUB_VIEW.with(|v| v.borrow().4 as f64)
}

// --- Paused-scene inspector ↔ DOM bridge (visual-debugger PR2b) --------------
//
// The desktop shell serves the inspector trace over `GET /trace`; the web shell
// has no debug HTTP server, so it uses the SAME poll pattern as the scrubber
// above. Each frame the loop publishes the current trace doc via
// [`publish_inspector_trace`]; a GENERATION counter bumps only when the doc
// CONTENT changes — which, given the producer's caching, happens only on a
// pause-state change or a paused-frame change (step/seek), never generally
// during play. The page polls the counter and, on a change, reads the doc and
// relays it to the VS Code live-preview as a `functor-inspector-trace`
// postMessage (which the extension already forwards to the LSP).

thread_local! {
    /// `(generation, doc json)` — published each frame by the loop, read by the
    /// page's poll exports. The generation increments ONLY when the doc bytes
    /// change, so the page posts a trace on pause / paused-frame change, not
    /// every frame.
    static INSPECTOR_TRACE: RefCell<(u32, String)> = const { RefCell::new((0, String::new())) };
}

/// Publish this frame's inspector trace for the page to poll. Cheap: the doc is
/// the producer's cached string while paused (rebuilt only on a pause/frame
/// change) and the byte-stable stub while playing, so the equality check here
/// is a plain string compare — the generation bumps only on a real change.
pub fn publish_inspector_trace(doc: String) {
    INSPECTOR_TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if t.1 != doc {
            t.0 = t.0.wrapping_add(1);
            t.1 = doc;
        }
    });
}

/// Runtime → page: the inspector-trace generation. The page polls this each
/// frame and reads [`functor_lang_inspector_trace`] only when it changes.
#[wasm_bindgen]
pub fn functor_lang_inspector_trace_gen() -> u32 {
    INSPECTOR_TRACE.with(|t| t.borrow().0)
}

/// Runtime → page: the current inspector-trace wire JSON (the paused full doc,
/// or the byte-stable playing stub).
#[wasm_bindgen]
pub fn functor_lang_inspector_trace() -> String {
    INSPECTOR_TRACE.with(|t| t.borrow().1.clone())
}
