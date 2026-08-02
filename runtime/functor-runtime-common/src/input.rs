use serde::{Deserialize, Serialize};

use crate::TrackingPose;

/// Runtime-owned input sampled for one fixed simulation step.
///
/// Keyboard and mouse retain their event entry points, while this plain-data
/// snapshot exposes both levels and de-duplicated transitions through the
/// extensible shell → producer seam. XR and gamepad are typed device domains;
/// mobile touch can add a sibling field without turning device capabilities
/// into stringly-typed maps or adding target-specific producer methods.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputSnapshot {
    /// Keys currently held, in canonical discriminant order.
    pub held_keys: Vec<Key>,
    /// Keys whose level changed from up to down since the previous simulation
    /// step, in canonical discriminant order.
    #[serde(default)]
    pub pressed_keys: Vec<Key>,
    /// Keys whose level changed from down to up since the previous simulation
    /// step, in canonical discriminant order.
    #[serde(default)]
    pub released_keys: Vec<Key>,
    /// Last known mouse position and the logical surface extent sharing its
    /// coordinate space.
    pub mouse: MouseSnapshot,
    /// Live XR tracking/controller state when the target supplies it.
    ///
    /// Omitted rather than serialized as `null` on non-XR targets, preserving
    /// the existing desktop `/state` wire shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xr: Option<XrInputSnapshot>,
    /// Live state of the primary connected gamepad when the target supplies
    /// one. `None` means no pad is connected — a capability signal, so a game
    /// can key control hints off it.
    ///
    /// Omitted rather than serialized as `null` when absent, like `xr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamepad: Option<GamepadSnapshot>,
}

/// Last known mouse position in logical shell coordinates, the matching
/// surface extent, held buttons, and this step's button transitions.
///
/// Desktop supplies GLFW window points and web supplies CSS pixels. Keeping
/// the extent beside the position makes pointer mapping independent of
/// framebuffer/Retina scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseSnapshot {
    pub x: i32,
    pub y: i32,
    /// Logical width in the same coordinate space as [`Self::x`].
    #[serde(default)]
    pub surface_width: u32,
    /// Logical height in the same coordinate space as [`Self::y`].
    #[serde(default)]
    pub surface_height: u32,
    /// Buttons held right now — the level complement of the `mouseButton`
    /// edges. Defaulted on the wire so an older `/state` capture (and a
    /// hand-written debug injection) still deserializes.
    #[serde(default)]
    pub buttons: MouseButtons,
    /// Buttons whose level changed from up to down since the previous
    /// simulation step.
    #[serde(default)]
    pub pressed: MouseButtons,
    /// Buttons whose level changed from down to up since the previous
    /// simulation step.
    #[serde(default)]
    pub released: MouseButtons,
}

/// A fixed set of modeled mouse buttons.
///
/// A record rather than a `Vec<MouseButton>`: buttons are a tiny fixed set,
/// and a game asks a named question (`snapshot.mouse.buttons.left` or
/// `snapshot.mouse.pressed.left`) rather than iterating them. Further buttons
/// (back/forward) can join as sibling fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl MouseButtons {
    /// Set one named member. `MouseButton::Unknown` (a platform button Functor
    /// does not model) leaves the set untouched.
    pub fn set(&mut self, button: MouseButton, is_down: bool) {
        match button {
            MouseButton::Left => self.left = is_down,
            MouseButton::Right => self.right = is_down,
            MouseButton::Middle => self.middle = is_down,
            MouseButton::Unknown => {}
        }
    }

    /// Whether `button` is currently held.
    pub fn is_down(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.left,
            MouseButton::Right => self.right,
            MouseButton::Middle => self.middle,
            MouseButton::Unknown => false,
        }
    }
}

/// Transient keyboard and mouse transitions waiting for the next fixed
/// simulation step.
///
/// Shells retain one accumulator across render frames, copy it into the first
/// fixed-step [`InputSnapshot`], then clear it. This makes a transition
/// deterministic when rendering and simulation run at different rates:
/// zero-step render frames cannot lose it, while catch-up substeps cannot
/// repeat it. Each control is a set, not an event count — a down/up/down burst
/// may appear in both halves while the separately sampled held level records
/// the final state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputEdges {
    pressed_keys: Vec<Key>,
    released_keys: Vec<Key>,
    pressed_mouse: MouseButtons,
    released_mouse: MouseButtons,
}

impl InputEdges {
    /// Record an actual keyboard level transition, ignoring unknown keys and
    /// unchanged levels (including native OS repeat).
    pub fn key_transition(&mut self, key: Key, was_down: bool, is_down: bool) {
        if key == Key::Unknown || was_down == is_down {
            return;
        }
        let keys = if is_down {
            &mut self.pressed_keys
        } else {
            &mut self.released_keys
        };
        if let Err(index) = keys.binary_search(&key) {
            keys.insert(index, key);
        }
    }

    /// Record an actual modeled mouse-button level transition.
    pub fn mouse_transition(&mut self, button: MouseButton, was_down: bool, is_down: bool) {
        if button == MouseButton::Unknown || was_down == is_down {
            return;
        }
        if is_down {
            self.pressed_mouse.set(button, true);
        } else {
            self.released_mouse.set(button, true);
        }
    }

    /// Copy the pending transitions into a step snapshot.
    pub fn apply_to(&self, snapshot: &mut InputSnapshot) {
        snapshot.pressed_keys.clone_from(&self.pressed_keys);
        snapshot.released_keys.clone_from(&self.released_keys);
        snapshot.mouse.pressed = self.pressed_mouse;
        snapshot.mouse.released = self.released_mouse;
    }

    /// Clear consumed transitions while retaining key-vector capacity for the
    /// next burst.
    pub fn clear(&mut self) {
        self.pressed_keys.clear();
        self.released_keys.clear();
        self.pressed_mouse = MouseButtons::default();
        self.released_mouse = MouseButtons::default();
    }
}

impl InputSnapshot {
    /// Remove already-consumed edge fields while preserving continuously
    /// sampled levels and poses.
    pub fn clear_edges(&mut self) {
        self.pressed_keys.clear();
        self.released_keys.clear();
        self.mouse.pressed = MouseButtons::default();
        self.mouse.released = MouseButtons::default();
    }
}

/// Fold one keyboard level change into a sampled snapshot.
///
/// `record_edge` separates physical-state recovery from a model-visible
/// transition: shells pass `false` when a release must unstick a paused or
/// suppressed input level without inventing an edge the game never received.
/// Returns whether the modeled level actually changed.
pub fn apply_key_transition_to_snapshot(
    snapshot: &mut InputSnapshot,
    edges: &mut InputEdges,
    key: Key,
    is_down: bool,
    record_edge: bool,
) -> bool {
    if key == Key::Unknown {
        return false;
    }
    let index = snapshot.held_keys.binary_search(&key);
    let was_down = index.is_ok();
    if was_down == is_down {
        return false;
    }
    match index {
        Ok(index) => {
            snapshot.held_keys.remove(index);
        }
        Err(index) => {
            snapshot.held_keys.insert(index, key);
        }
    }
    if record_edge {
        edges.key_transition(key, was_down, is_down);
    }
    true
}

/// Fold one modeled mouse-button level change into a sampled snapshot.
///
/// See [`apply_key_transition_to_snapshot`] for `record_edge`'s recovery-only
/// semantics. Returns whether the level actually changed.
pub fn apply_mouse_transition_to_snapshot(
    snapshot: &mut InputSnapshot,
    edges: &mut InputEdges,
    button: MouseButton,
    is_down: bool,
    record_edge: bool,
) -> bool {
    if button == MouseButton::Unknown {
        return false;
    }
    let was_down = snapshot.mouse.buttons.is_down(button);
    if was_down == is_down {
        return false;
    }
    snapshot.mouse.buttons.set(button, is_down);
    if record_edge {
        edges.mouse_transition(button, was_down, is_down);
    }
    true
}

/// One frame of XR input in the tracking rig's local coordinates.
///
/// Poses are relative to the center-eye reference captured when the authored
/// camera rig is established: +X right, +Y up, -Z forward. Keeping them
/// rig-local lets a pure game map them through its current authored camera
/// without a one-frame mismatch when locomotion moves that camera.
///
/// Deserialization defaults every missing field (no head pose, both hands
/// inactive), so a debug-injected sample (`POST /input` `{"type":"xr",…}`) can
/// name only the hand and controls it drives.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct XrInputSnapshot {
    /// Center-eye pose relative to the rig reference.
    pub head: Option<TrackingPose>,
    pub left: XrControllerSnapshot,
    pub right: XrControllerSnapshot,
}

/// Target-neutral state for one tracked XR controller.
///
/// `active` reports whether the runtime currently has an input source for the
/// hand. Each pose is independently optional because OpenXR may have buttons
/// while positional tracking is temporarily invalid.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct XrControllerSnapshot {
    pub active: bool,
    pub grip: Option<TrackingPose>,
    pub aim: Option<TrackingPose>,
    pub trigger: f32,
    pub squeeze: f32,
    pub thumbstick: [f32; 2],
    pub primary_pressed: bool,
    pub secondary_pressed: bool,
    pub thumbstick_pressed: bool,
    pub menu_pressed: bool,
}

/// Target-neutral state for the primary connected gamepad, aligned to the
/// standard mapping GLFW (`glfwGetGamepadState`) and the Web Gamepad API
/// share.
///
/// Face buttons are POSITIONAL (`south` is the bottom face button — A on
/// Xbox, Cross on PlayStation, B on Nintendo), because letter names swap
/// between vendors. Sticks are `-1..1` with **up-positive Y**, matching the
/// XR thumbstick convention — a shell that polls a pad must negate GLFW's
/// and the browser's down-positive axes at its boundary. Triggers are
/// `0..1`. Values are raw (no deadzone shaping); games apply their own.
///
/// The desktop shell polls the first standard-mapping pad via GLFW
/// (`desktop_gamepad` in `functor-runtime-desktop`), with debug injection
/// (`POST /input` `{"type":"gamepad",…}`) winning over the poll; the web
/// shell does not sample the browser Gamepad API yet, so on wasm the domain
/// is absent.
///
/// Levels only, no edge sets: pads are polled, so a press always spans at
/// least one render frame and shows up in at least one sample — games detect
/// edges against their model, exactly as XR games do.
///
/// Deserialization defaults every missing field, so a debug-injected sample
/// (`POST /input` `{"type":"gamepad",…}`) can name only the controls it
/// drives.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GamepadSnapshot {
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub south: bool,
    pub east: bool,
    pub west: bool,
    pub north: bool,
    pub left_bumper: bool,
    pub right_bumper: bool,
    pub left_stick_pressed: bool,
    pub right_stick_pressed: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub start: bool,
    pub select: bool,
}

/// Canonical key identifier shared across the F# <-> Rust boundary and all
/// runtimes (desktop GLFW, web). Producers (e.g. the desktop runtime's
/// `glfw::Key` mapping in `functor-runtime-desktop`) translate their platform
/// key into this enum and pass its `as i32` discriminant across the dylib/wasm
/// boundary. The F# `Input.Key` DU mirrors these discriminants in
/// `Input.ofKeyCode` — keep the two in sync when adding keys.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    Unknown = 0,
    A = 1,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Up,
    Down,
    Left,
    Right,
    Space,
    Enter,
    Escape,
    // The digit row (and numpad, which the shells fold into these). APPENDED
    // — the `as i32` discriminants are the wire representation, so new keys
    // only ever go at the end.
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
}

/// Canonical mouse-button identifier shared across every runtime, mirroring
/// [`Key`]: shells translate their platform button (GLFW's
/// `glfw::MouseButton`, the browser's `MouseEvent.button`) into this enum and
/// pass its `as i32` discriminant across the wasm/producer boundary.
///
/// `Unknown = 0` (like `Key`) so an unmodeled platform button decodes to a
/// value the producers drop rather than mis-delivering as `Left`. New buttons
/// only ever go at the END — the discriminants are the wire representation.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Unknown = 0,
    Left = 1,
    Right,
    Middle,
}

impl MouseButton {
    /// All button variants in discriminant order (guarded by
    /// `mouse_button_from_i32_round_trips`).
    pub const ALL: [MouseButton; 4] = [
        MouseButton::Unknown,
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
    ];

    /// The button whose `as i32` discriminant equals `value`, if any.
    pub fn from_i32(value: i32) -> Option<MouseButton> {
        MouseButton::ALL.into_iter().find(|b| *b as i32 == value)
    }

    /// Parse the case-insensitive wire spelling used by debug input
    /// (`"left"`, `"Right"`, `"middle"`), so the desktop and device debug
    /// servers accept exactly the same button names.
    pub fn from_name(name: &str) -> Option<MouseButton> {
        match name.to_ascii_lowercase().as_str() {
            "left" => Some(MouseButton::Left),
            "right" => Some(MouseButton::Right),
            "middle" => Some(MouseButton::Middle),
            _ => None,
        }
    }

    /// The button's short display name (`"Left"`) — for human-facing labels
    /// like the web timeline's input markers.
    pub fn name(self) -> String {
        format!("{self:?}")
    }

    /// The built-in `Mouse` module's constructor tag (`"Mouse.Left"`) — the
    /// `Value::Variant` the producers hand a game's `mouseButton` hook. `None`
    /// for `Unknown`, which is never delivered. Keep in sync with
    /// `MOUSE_MODULE_SRC` in `functor_lang::project` (guarded by
    /// `mouse_ctor_tags_cover_the_module` here).
    pub fn ctor_tag(self) -> Option<String> {
        match self {
            MouseButton::Unknown => None,
            _ => Some(format!("Mouse.{self:?}")),
        }
    }
}

/// One recorded input event at the raw boundary scalars — pre-`Key::from_i32`,
/// pre-name-formatting — so a replay re-runs the *identical* path the live
/// frame took (docs/time-travel.md, "The event log"). It is PLAIN DATA — `Copy`
/// scalars holding no `Rc`/closure into the old module — which is exactly why
/// the frame-indexed input log survives a hot reload even though the
/// closure-holding model snapshots do not. Both shells buffer these in
/// `key_event`/`mouse_move`/`mouse_wheel` and flush a frame's worth into the
/// recorder; the forward-step replays them. (`Serialize`/`Deserialize` are for
/// the future on-disk/wire event log, T7 — unused by the in-session replay.)
/// (Not `Copy`: `UiEvent` can carry a text payload.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecordedInput {
    /// A keyboard event carrying the raw `Key as i32` code (not the resolved
    /// `Key`), so replay re-runs `Key::from_i32` exactly as the live path does.
    Key { code: i32, is_down: bool },
    /// A pointer position in logical window points (GLFW points natively, CSS
    /// pixels on web), matching `Input.mouse.x` / `.y` rather than framebuffer
    /// pixels.
    MouseMove { x: i32, y: i32 },
    /// A wheel notch (±1 per notch).
    MouseWheel { delta: i32 },
    /// A mouse-button edge carrying the raw `MouseButton as i32` code, so
    /// replay re-runs `MouseButton::from_i32` exactly as the live path does.
    MouseButton { button: i32, is_down: bool },
    /// The continuously sampled input delivered before one simulation tick.
    ///
    /// Unlike edge events above, this is recorded every tick whose game
    /// defines `sampledInput`: held controls, fixed-step transitions, and
    /// tracked poses can drive pure game logic, and replay re-runs the same
    /// hook with the same sample. Projection clears transitions after that
    /// step while carrying continuous levels/poses.
    // Box the dense payload so adding sampled input does not inflate every
    // legacy key/mouse/UI event stored in the sparse session log.
    Snapshot(Box<InputSnapshot>),
    /// An interaction on an interactive UI widget (slot-addressed — see
    /// [`crate::ui::UiEvent`]). Replay rebuilds the frame's handler table from
    /// `ui(model)` and re-delivers, so UI-driven model changes replay too.
    UiEvent(crate::ui::UiEvent),
    /// An interaction on a webview element (`Attr.onClick` / `Attr.onInput`).
    /// Same event shape as [`RecordedInput::UiEvent`], but its OWN variant:
    /// slots address the `webview(model)` handler table, so replay must
    /// rebuild and resolve against that table — not the `ui` one.
    WebviewEvent(crate::ui::UiEvent),
}

impl Key {
    /// All key variants in discriminant order. Keep in sync with the enum above
    /// (guarded by `from_i32_round_trips`).
    pub const ALL: [Key; 44] = [
        Key::Unknown,
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Space,
        Key::Enter,
        Key::Escape,
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];

    /// The key whose `as i32` discriminant equals `value`, if any. The inverse
    /// of `key as i32` (which is how key codes cross the dylib/wasm boundary).
    pub fn from_i32(value: i32) -> Option<Key> {
        Key::ALL.into_iter().find(|k| *k as i32 == value)
    }

    /// Parse the case-insensitive wire/display spelling used by debug input
    /// (`"w"`, `"Up"`, `"space"`, or a bare digit). Keeping this beside the
    /// canonical enum makes desktop and device debug servers accept exactly
    /// the same key names.
    pub fn from_name(name: &str) -> Option<Key> {
        let name = name.to_ascii_lowercase();
        if name.len() == 1 {
            let byte = name.as_bytes()[0];
            if byte.is_ascii_lowercase() {
                return Key::from_i32((byte - b'a') as i32 + Key::A as i32);
            }
            if byte.is_ascii_digit() {
                return Key::from_i32((byte - b'0') as i32 + Key::Num0 as i32);
            }
        }
        match name.as_str() {
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "space" => Some(Key::Space),
            "enter" => Some(Key::Enter),
            "escape" => Some(Key::Escape),
            _ => None,
        }
    }

    /// The key's short display name — `"W"`, `"Up"`, `"Space"`, bare digits
    /// (`"1"`, not `"Num1"`) — for human-facing labels like the web timeline's
    /// input markers. Games no longer see this: the `input` hook receives the
    /// built-in `Key` module's variant (see [`Key::ctor_tag`]).
    pub fn name(self) -> String {
        let digit = self as i32 - Key::Num0 as i32;
        if (0..=9).contains(&digit) {
            digit.to_string()
        } else {
            format!("{self:?}")
        }
    }

    /// The built-in `Key` module's constructor tag for this key (`"Key.W"`,
    /// `"Key.Num0"`) — the `Value::Variant` ctor the producers hand a game's
    /// `input` hook. `None` for `Unknown`, which is never delivered. Keep in
    /// sync with `KEY_MODULE_SRC` in `functor_lang::project` (guarded by that
    /// crate's tests and `ctor_tags_cover_the_module` here).
    pub fn ctor_tag(self) -> Option<String> {
        match self {
            Key::Unknown => None,
            _ => Some(format!("Key.{self:?}")),
        }
    }
}

/// The `Value` a game's `input` hook receives for a raw key code: the built-in
/// `Key` module's variant (`Key.W`). `None` for an unrecognized code or
/// `Key::Unknown` — the event is dropped, never delivered. The ONE conversion
/// every delivery path shares (desktop, web, and the time-travel replay in
/// `functor_lang_producer`), so live input and replay cannot drift.
pub fn key_input_value(code: i32) -> Option<functor_lang::Value> {
    let tag = Key::from_i32(code)?.ctor_tag()?;
    Some(functor_lang::Value::Variant {
        ctor: std::rc::Rc::from(tag.as_str()),
        args: std::rc::Rc::new(Vec::new()),
    })
}

/// The `Value` a game's `mouseButton` hook receives for a raw button code: the
/// built-in `Mouse` module's variant (`Mouse.Left`). `None` for an
/// unrecognized code or `MouseButton::Unknown` — the event is dropped, never
/// delivered. The mouse twin of [`key_input_value`], and likewise the ONE
/// conversion every delivery path shares (desktop, web, debug injection, and
/// the time-travel replay), so live input and replay cannot drift.
pub fn mouse_button_input_value(code: i32) -> Option<functor_lang::Value> {
    let tag = MouseButton::from_i32(code)?.ctor_tag()?;
    Some(functor_lang::Value::Variant {
        ctor: std::rc::Rc::from(tag.as_str()),
        args: std::rc::Rc::new(Vec::new()),
    })
}

pub(crate) fn record(
    fields: impl IntoIterator<Item = (&'static str, functor_lang::Value)>,
) -> functor_lang::Value {
    functor_lang::Value::Record(std::rc::Rc::new(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    ))
}

pub(crate) fn option_value(value: Option<functor_lang::Value>) -> functor_lang::Value {
    match value {
        Some(value) => functor_lang::Value::Variant {
            ctor: std::rc::Rc::from("Option.Some"),
            args: std::rc::Rc::new(vec![value]),
        },
        None => functor_lang::Value::Variant {
            ctor: std::rc::Rc::from("Option.None"),
            args: std::rc::Rc::new(Vec::new()),
        },
    }
}

/// Convert one rig-local tracking pose into the plain-data `Input.pose`
/// record a Functor Lang game receives.
pub fn tracking_pose_value(pose: TrackingPose) -> functor_lang::Value {
    record([
        (
            "position",
            record([
                ("x", functor_lang::Value::Number(pose.position[0] as f64)),
                ("y", functor_lang::Value::Number(pose.position[1] as f64)),
                ("z", functor_lang::Value::Number(pose.position[2] as f64)),
            ]),
        ),
        (
            "orientation",
            record([
                ("x", functor_lang::Value::Number(pose.orientation[0] as f64)),
                ("y", functor_lang::Value::Number(pose.orientation[1] as f64)),
                ("z", functor_lang::Value::Number(pose.orientation[2] as f64)),
                ("w", functor_lang::Value::Number(pose.orientation[3] as f64)),
            ]),
        ),
    ])
}

/// Decode the plain-data `Input.pose` record accepted by camera-rig helpers.
pub fn tracking_pose_from_value(value: &functor_lang::Value) -> Result<TrackingPose, String> {
    fn field<'a>(
        value: &'a functor_lang::Value,
        name: &str,
    ) -> Result<&'a functor_lang::Value, String> {
        let functor_lang::Value::Record(fields) = value else {
            return Err(format!(
                "expected an Input.pose record, got {}",
                value.kind_name()
            ));
        };
        fields
            .iter()
            .find_map(|(field, value)| (field == name).then_some(value))
            .ok_or_else(|| format!("Input.pose is missing `{name}`"))
    }
    let number = |value: &functor_lang::Value, name: &str| match value {
        functor_lang::Value::Number(value) if (*value as f32).is_finite() => Ok(*value as f32),
        functor_lang::Value::Number(value) => {
            Err(format!("Input.pose `{name}` must be finite, got {value}"))
        }
        other => Err(format!(
            "Input.pose `{name}` must be a number, got {}",
            other.kind_name()
        )),
    };
    let position = field(value, "position")?;
    let orientation = field(value, "orientation")?;
    Ok(TrackingPose::new(
        [
            number(field(position, "x")?, "position.x")?,
            number(field(position, "y")?, "position.y")?,
            number(field(position, "z")?, "position.z")?,
        ],
        [
            number(field(orientation, "x")?, "orientation.x")?,
            number(field(orientation, "y")?, "orientation.y")?,
            number(field(orientation, "z")?, "orientation.z")?,
            number(field(orientation, "w")?, "orientation.w")?,
        ],
    ))
}

fn controller_value(controller: &XrControllerSnapshot) -> functor_lang::Value {
    record([
        ("active", functor_lang::Value::Bool(controller.active)),
        (
            "grip",
            option_value(controller.grip.map(tracking_pose_value)),
        ),
        ("aim", option_value(controller.aim.map(tracking_pose_value))),
        (
            "trigger",
            functor_lang::Value::Number(controller.trigger as f64),
        ),
        (
            "squeeze",
            functor_lang::Value::Number(controller.squeeze as f64),
        ),
        ("thumbstick", axes2_value(controller.thumbstick)),
        (
            "primaryPressed",
            functor_lang::Value::Bool(controller.primary_pressed),
        ),
        (
            "secondaryPressed",
            functor_lang::Value::Bool(controller.secondary_pressed),
        ),
        (
            "thumbstickPressed",
            functor_lang::Value::Bool(controller.thumbstick_pressed),
        ),
        (
            "menuPressed",
            functor_lang::Value::Bool(controller.menu_pressed),
        ),
    ])
}

/// Convert a 2-axis pair into the plain-data `Input.point2` record.
fn axes2_value(axes: [f32; 2]) -> functor_lang::Value {
    record([
        ("x", functor_lang::Value::Number(axes[0] as f64)),
        ("y", functor_lang::Value::Number(axes[1] as f64)),
    ])
}

fn gamepad_value(pad: &GamepadSnapshot) -> functor_lang::Value {
    record([
        ("leftStick", axes2_value(pad.left_stick)),
        ("rightStick", axes2_value(pad.right_stick)),
        (
            "leftTrigger",
            functor_lang::Value::Number(pad.left_trigger as f64),
        ),
        (
            "rightTrigger",
            functor_lang::Value::Number(pad.right_trigger as f64),
        ),
        ("south", functor_lang::Value::Bool(pad.south)),
        ("east", functor_lang::Value::Bool(pad.east)),
        ("west", functor_lang::Value::Bool(pad.west)),
        ("north", functor_lang::Value::Bool(pad.north)),
        ("leftBumper", functor_lang::Value::Bool(pad.left_bumper)),
        ("rightBumper", functor_lang::Value::Bool(pad.right_bumper)),
        (
            "leftStickPressed",
            functor_lang::Value::Bool(pad.left_stick_pressed),
        ),
        (
            "rightStickPressed",
            functor_lang::Value::Bool(pad.right_stick_pressed),
        ),
        ("dpadUp", functor_lang::Value::Bool(pad.dpad_up)),
        ("dpadDown", functor_lang::Value::Bool(pad.dpad_down)),
        ("dpadLeft", functor_lang::Value::Bool(pad.dpad_left)),
        ("dpadRight", functor_lang::Value::Bool(pad.dpad_right)),
        ("start", functor_lang::Value::Bool(pad.start)),
        ("select", functor_lang::Value::Bool(pad.select)),
    ])
}

/// Convert the shared shell snapshot into the typed, plain-data
/// `Input.snapshot` record delivered to a Functor Lang game's optional
/// `sampledInput` hook.
pub fn input_snapshot_value(snapshot: &InputSnapshot) -> functor_lang::Value {
    let key_values = |keys: &[Key]| {
        functor_lang::Value::List(std::rc::Rc::new(
            keys.iter()
                .filter_map(|key| key_input_value(*key as i32))
                .collect(),
        ))
    };
    let mouse_buttons_value = |buttons: MouseButtons| {
        record([
            ("left", functor_lang::Value::Bool(buttons.left)),
            ("right", functor_lang::Value::Bool(buttons.right)),
            ("middle", functor_lang::Value::Bool(buttons.middle)),
        ])
    };
    let xr = snapshot.xr.as_ref().map(|xr| {
        record([
            ("head", option_value(xr.head.map(tracking_pose_value))),
            ("left", controller_value(&xr.left)),
            ("right", controller_value(&xr.right)),
        ])
    });
    record([
        ("heldKeys", key_values(&snapshot.held_keys)),
        ("pressedKeys", key_values(&snapshot.pressed_keys)),
        ("releasedKeys", key_values(&snapshot.released_keys)),
        (
            "mouse",
            record([
                ("x", functor_lang::Value::Number(snapshot.mouse.x as f64)),
                ("y", functor_lang::Value::Number(snapshot.mouse.y as f64)),
                (
                    "surfaceWidth",
                    functor_lang::Value::Number(snapshot.mouse.surface_width as f64),
                ),
                (
                    "surfaceHeight",
                    functor_lang::Value::Number(snapshot.mouse.surface_height as f64),
                ),
                ("buttons", mouse_buttons_value(snapshot.mouse.buttons)),
                ("pressed", mouse_buttons_value(snapshot.mouse.pressed)),
                ("released", mouse_buttons_value(snapshot.mouse.released)),
            ]),
        ),
        ("xr", option_value(xr)),
        (
            "gamepad",
            option_value(snapshot.gamepad.as_ref().map(gamepad_value)),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        apply_key_transition_to_snapshot, apply_mouse_transition_to_snapshot, input_snapshot_value,
        mouse_button_input_value, tracking_pose_from_value, tracking_pose_value, GamepadSnapshot,
        InputEdges, InputSnapshot, Key, MouseButton, MouseButtons, MouseSnapshot, RecordedInput,
        XrControllerSnapshot, XrInputSnapshot,
    };
    use crate::TrackingPose;

    #[test]
    fn names_are_canonical() {
        assert_eq!(Key::W.name(), "W");
        assert_eq!(Key::Up.name(), "Up");
        assert_eq!(Key::Space.name(), "Space");
        // Digits are bare — the name a game's `input` hook matches on.
        assert_eq!(Key::Num0.name(), "0");
        assert_eq!(Key::Num9.name(), "9");
    }

    #[test]
    fn ctor_tags_are_canonical() {
        assert_eq!(Key::W.ctor_tag().as_deref(), Some("Key.W"));
        assert_eq!(Key::Up.ctor_tag().as_deref(), Some("Key.Up"));
        // Digits keep the identifier spelling (ctor names can't be bare digits).
        assert_eq!(Key::Num0.ctor_tag().as_deref(), Some("Key.Num0"));
        // Unknown is filtered before dispatch — no constructor exists for it.
        assert_eq!(Key::Unknown.ctor_tag(), None);
        // Every deliverable key has a tag.
        for key in Key::ALL.into_iter().skip(1) {
            assert!(key.ctor_tag().is_some());
        }
    }

    /// Drift guard: every deliverable `Key` maps to a constructor the
    /// built-in `Key` module actually declares, and the module declares
    /// nothing else — this enum and `KEY_MODULE_SRC` (functor_lang::project)
    /// must move together.
    #[test]
    fn ctor_tags_cover_the_module() {
        let project = functor_lang::project::load_single_source("game", "let x = 0.0\n")
            .unwrap_or_else(|e| panic!("empty project loads: {}", e.render()));
        let key_ty = project
            .module
            .types
            .iter()
            .find(|t| t.name == "Key.t")
            .expect("the built-in Key module is injected");
        let declared: std::collections::BTreeSet<String> = match &key_ty.body {
            functor_lang::ast::TypeBody::Variants(variants) => {
                variants.iter().map(|v| v.name.clone()).collect()
            }
            _ => panic!("Key.t must be a variant type"),
        };
        let expected: std::collections::BTreeSet<String> =
            Key::ALL.into_iter().filter_map(|k| k.ctor_tag()).collect();
        assert_eq!(declared, expected, "Key enum and KEY_MODULE_SRC drifted");
    }

    /// The mouse twin of `ctor_tags_cover_the_module`: every deliverable
    /// `MouseButton` maps to a constructor the built-in `Mouse` module
    /// declares, and it declares nothing else.
    #[test]
    fn mouse_ctor_tags_cover_the_module() {
        let project = functor_lang::project::load_single_source("game", "let x = 0.0\n")
            .unwrap_or_else(|e| panic!("empty project loads: {}", e.render()));
        let mouse_ty = project
            .module
            .types
            .iter()
            .find(|t| t.name == "Mouse.t")
            .expect("the built-in Mouse module is injected");
        let declared: std::collections::BTreeSet<String> = match &mouse_ty.body {
            functor_lang::ast::TypeBody::Variants(variants) => {
                variants.iter().map(|v| v.name.clone()).collect()
            }
            _ => panic!("Mouse.t must be a variant type"),
        };
        let expected: std::collections::BTreeSet<String> = MouseButton::ALL
            .into_iter()
            .filter_map(|b| b.ctor_tag())
            .collect();
        assert_eq!(
            declared, expected,
            "MouseButton enum and MOUSE_MODULE_SRC drifted"
        );
    }

    #[test]
    fn mouse_button_from_i32_round_trips() {
        for (i, button) in MouseButton::ALL.iter().enumerate() {
            assert_eq!(
                *button as i32, i as i32,
                "MouseButton::ALL must be contiguous from 0"
            );
            assert_eq!(MouseButton::from_i32(*button as i32), Some(*button));
        }
        assert_eq!(MouseButton::from_i32(MouseButton::ALL.len() as i32), None);
        assert_eq!(MouseButton::from_i32(-1), None);
    }

    #[test]
    fn mouse_button_names_and_wire_spellings_are_canonical() {
        assert_eq!(MouseButton::Left.name(), "Left");
        assert_eq!(MouseButton::Left.ctor_tag().as_deref(), Some("Mouse.Left"));
        assert_eq!(MouseButton::Unknown.ctor_tag(), None);
        assert_eq!(MouseButton::from_name("left"), Some(MouseButton::Left));
        assert_eq!(MouseButton::from_name("Right"), Some(MouseButton::Right));
        assert_eq!(MouseButton::from_name("MIDDLE"), Some(MouseButton::Middle));
        assert_eq!(MouseButton::from_name("thumb"), None);
        // Unknown is never delivered as a variant.
        assert!(mouse_button_input_value(MouseButton::Unknown as i32).is_none());
        assert_eq!(
            mouse_button_input_value(MouseButton::Right as i32)
                .unwrap()
                .to_string(),
            "Mouse.Right"
        );
    }

    #[test]
    fn held_buttons_fold_edges_and_ignore_unknown() {
        let mut buttons = MouseButtons::default();
        buttons.set(MouseButton::Left, true);
        buttons.set(MouseButton::Middle, true);
        assert!(buttons.is_down(MouseButton::Left));
        assert!(buttons.is_down(MouseButton::Middle));
        assert!(!buttons.is_down(MouseButton::Right));
        buttons.set(MouseButton::Left, false);
        assert!(!buttons.is_down(MouseButton::Left));
        // An unmodeled platform button cannot corrupt the held set.
        let before = buttons;
        buttons.set(MouseButton::Unknown, true);
        assert_eq!(buttons, before);
        assert!(!buttons.is_down(MouseButton::Unknown));
    }

    #[test]
    fn from_i32_round_trips() {
        for (i, key) in Key::ALL.iter().enumerate() {
            // Prove ALL is contiguous from 0 (not just that round-trips work) —
            // otherwise the length-as-ceiling check below wouldn't be sound, and
            // a gap could hide a missing variant.
            assert_eq!(*key as i32, i as i32, "Key::ALL must be contiguous from 0");
            assert_eq!(Key::from_i32(*key as i32), Some(*key));
        }
        assert_eq!(Key::from_i32(Key::ALL.len() as i32), None);
        assert_eq!(Key::from_i32(-1), None);
    }

    #[test]
    fn from_name_accepts_the_shared_debug_wire_spellings() {
        assert_eq!(Key::from_name("w"), Some(Key::W));
        assert_eq!(Key::from_name("W"), Some(Key::W));
        assert_eq!(Key::from_name("Up"), Some(Key::Up));
        assert_eq!(Key::from_name("SPACE"), Some(Key::Space));
        assert_eq!(Key::from_name("0"), Some(Key::Num0));
        assert_eq!(Key::from_name("9"), Some(Key::Num9));
        assert_eq!(Key::from_name("unknown"), None);
        assert_eq!(Key::from_name(""), None);
    }

    #[test]
    fn dense_snapshot_payload_does_not_inflate_sparse_recorded_events() {
        assert!(
            std::mem::size_of::<RecordedInput>() <= 64,
            "RecordedInput is {} bytes; keep the snapshot payload indirect",
            std::mem::size_of::<RecordedInput>()
        );
    }

    #[test]
    fn input_snapshot_omits_absent_xr_and_round_trips_present_xr() {
        let desktop = InputSnapshot {
            held_keys: vec![Key::W],
            mouse: MouseSnapshot {
                x: 10,
                y: 20,
                surface_width: 800,
                surface_height: 600,
                ..Default::default()
            },
            xr: None,
            ..InputSnapshot::default()
        };
        let desktop_json = serde_json::to_value(&desktop).unwrap();
        assert_eq!(
            desktop_json,
            serde_json::json!({
                "held_keys": ["W"],
                "pressed_keys": [],
                "released_keys": [],
                "mouse": {
                    "x": 10,
                    "y": 20,
                    "surface_width": 800,
                    "surface_height": 600,
                    "buttons": { "left": false, "right": false, "middle": false },
                    "pressed": { "left": false, "right": false, "middle": false },
                    "released": { "left": false, "right": false, "middle": false }
                }
            })
        );
        // Newer mouse fields and edge sets are defaulted on the wire, so a
        // capture taken before they existed (and a hand-written injection)
        // still decodes.
        let legacy: InputSnapshot = serde_json::from_value(
            serde_json::json!({ "held_keys": [], "mouse": { "x": 1, "y": 2 } }),
        )
        .unwrap();
        assert_eq!(legacy.mouse.buttons, MouseButtons::default());
        assert_eq!(legacy.mouse.surface_width, 0);
        assert_eq!(legacy.mouse.surface_height, 0);
        assert!(legacy.pressed_keys.is_empty());
        assert!(legacy.released_keys.is_empty());
        assert_eq!(legacy.mouse.pressed, MouseButtons::default());
        assert_eq!(legacy.mouse.released, MouseButtons::default());

        let xr = InputSnapshot {
            xr: Some(XrInputSnapshot {
                head: Some(TrackingPose::IDENTITY),
                left: XrControllerSnapshot {
                    active: true,
                    trigger: 0.75,
                    thumbstick: [-0.25, 1.0],
                    primary_pressed: true,
                    ..XrControllerSnapshot::default()
                },
                right: XrControllerSnapshot::default(),
            }),
            ..InputSnapshot::default()
        };
        let encoded = serde_json::to_string(&xr).unwrap();
        assert_eq!(serde_json::from_str::<InputSnapshot>(&encoded).unwrap(), xr);

        // The gamepad domain follows the same wire contract: omitted when
        // absent (asserted by the exact desktop JSON above), round-tripped
        // when present.
        let pad = InputSnapshot {
            gamepad: Some(GamepadSnapshot {
                left_stick: [0.5, -0.5],
                left_trigger: 1.0,
                south: true,
                select: true,
                ..GamepadSnapshot::default()
            }),
            ..InputSnapshot::default()
        };
        let encoded = serde_json::to_string(&pad).unwrap();
        assert_eq!(
            serde_json::from_str::<InputSnapshot>(&encoded).unwrap(),
            pad
        );
    }

    #[test]
    fn functor_value_preserves_the_typed_snapshot_shape() {
        let snapshot = InputSnapshot {
            held_keys: vec![Key::W, Key::Space],
            pressed_keys: vec![Key::Space],
            released_keys: vec![Key::Enter],
            mouse: MouseSnapshot {
                x: 12,
                y: -4,
                surface_width: 1440,
                surface_height: 900,
                buttons: MouseButtons {
                    left: true,
                    right: false,
                    middle: false,
                },
                pressed: MouseButtons {
                    left: true,
                    ..MouseButtons::default()
                },
                released: MouseButtons {
                    right: true,
                    ..MouseButtons::default()
                },
            },
            xr: Some(XrInputSnapshot {
                head: Some(TrackingPose::IDENTITY),
                left: XrControllerSnapshot::default(),
                right: XrControllerSnapshot {
                    active: true,
                    aim: Some(TrackingPose::new([0.25, -0.5, -1.0], [0.0, 0.0, 0.0, 1.0])),
                    trigger: 0.75,
                    primary_pressed: true,
                    ..XrControllerSnapshot::default()
                },
            }),
            gamepad: Some(GamepadSnapshot {
                left_stick: [-0.5, 1.0],
                right_trigger: 0.25,
                south: true,
                dpad_left: true,
                ..GamepadSnapshot::default()
            }),
        };
        let rendered = input_snapshot_value(&snapshot).to_string();
        assert!(
            rendered.contains("heldKeys: [Key.W, Key.Space]"),
            "{rendered}"
        );
        assert!(rendered.contains("pressedKeys: [Key.Space]"), "{rendered}");
        assert!(rendered.contains("releasedKeys: [Key.Enter]"), "{rendered}");
        assert!(
            rendered.contains("surfaceWidth: 1440") && rendered.contains("surfaceHeight: 900"),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "buttons: { left: true, right: false, middle: false }, pressed: { left: true, right: false, middle: false }, released: { left: false, right: true, middle: false }"
            ),
            "{rendered}"
        );
        assert!(rendered.contains("xr: Option.Some("), "{rendered}");
        assert!(rendered.contains("trigger: 0.75"), "{rendered}");
        assert!(rendered.contains("primaryPressed: true"), "{rendered}");
        assert!(rendered.contains("gamepad: Option.Some("), "{rendered}");
        assert!(
            rendered.contains("leftStick: { x: -0.5, y: 1 }"),
            "{rendered}"
        );
        assert!(rendered.contains("rightTrigger: 0.25"), "{rendered}");
        assert!(rendered.contains("south: true"), "{rendered}");
        assert!(rendered.contains("dpadLeft: true"), "{rendered}");
        assert!(rendered.contains("select: false"), "{rendered}");

        // Without a pad the domain is an explicit None, never a zeroed record.
        let no_pad = InputSnapshot::default();
        assert!(
            input_snapshot_value(&no_pad)
                .to_string()
                .contains("gamepad: Option.None"),
        );
    }

    #[test]
    fn tracking_pose_functor_value_round_trips() {
        let pose = TrackingPose::new([1.0, -2.0, 3.5], [0.1, 0.2, 0.3, 0.9]);
        assert_eq!(
            tracking_pose_from_value(&tracking_pose_value(pose)).unwrap(),
            pose
        );
    }

    #[test]
    fn snapshot_transition_reducer_handles_repeat_quick_tap_and_recovery() {
        let mut snapshot = InputSnapshot::default();
        let mut edges = InputEdges::default();

        assert!(apply_key_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            Key::Space,
            true,
            true,
        ));
        assert!(!apply_key_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            Key::Space,
            true,
            true,
        ));
        assert!(apply_key_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            Key::Space,
            false,
            true,
        ));
        assert!(apply_mouse_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            MouseButton::Left,
            true,
            true,
        ));
        assert!(apply_mouse_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            MouseButton::Left,
            false,
            true,
        ));

        edges.apply_to(&mut snapshot);
        assert!(snapshot.held_keys.is_empty());
        assert_eq!(snapshot.pressed_keys, vec![Key::Space]);
        assert_eq!(snapshot.released_keys, vec![Key::Space]);
        assert!(!snapshot.mouse.buttons.left);
        assert!(snapshot.mouse.pressed.left);
        assert!(snapshot.mouse.released.left);

        // Recovery-only releases fix final physical levels without creating
        // model-visible sampled edges.
        snapshot.clear_edges();
        edges.clear();
        snapshot.held_keys.push(Key::Enter);
        snapshot.mouse.buttons.right = true;
        assert!(apply_key_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            Key::Enter,
            false,
            false,
        ));
        assert!(apply_mouse_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            MouseButton::Right,
            false,
            false,
        ));
        edges.apply_to(&mut snapshot);
        assert!(snapshot.held_keys.is_empty());
        assert!(!snapshot.mouse.buttons.right);
        assert!(snapshot.released_keys.is_empty());
        assert_eq!(snapshot.mouse.released, MouseButtons::default());

        // Once physical recovery removed the stale level, a fresh press is a
        // real edge again.
        assert!(apply_key_transition_to_snapshot(
            &mut snapshot,
            &mut edges,
            Key::Enter,
            true,
            true,
        ));
        edges.apply_to(&mut snapshot);
        assert_eq!(snapshot.held_keys, vec![Key::Enter]);
        assert_eq!(snapshot.pressed_keys, vec![Key::Enter]);
    }
}
