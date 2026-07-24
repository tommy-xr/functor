use serde::{Deserialize, Serialize};

use crate::TrackingPose;

/// Runtime-owned input sampled for one simulation frame.
///
/// Keyboard and mouse retain their event entry points, while this plain-data
/// snapshot is the extensible shell → producer seam for continuously sampled
/// devices. XR is the first typed domain; gamepads and mobile touches can add
/// sibling fields without turning device capabilities into stringly-typed
/// maps or adding target-specific producer methods.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputSnapshot {
    /// Keys currently held, in canonical discriminant order.
    pub held_keys: Vec<Key>,
    /// Last known mouse position in output pixels.
    pub mouse: MouseSnapshot,
    /// Live XR tracking/controller state when the target supplies it.
    ///
    /// Omitted rather than serialized as `null` on non-XR targets, preserving
    /// the existing desktop `/state` wire shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xr: Option<XrInputSnapshot>,
}

/// Last known mouse position in output pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseSnapshot {
    pub x: i32,
    pub y: i32,
}

/// One frame of XR input in the tracking rig's local coordinates.
///
/// Poses are relative to the center-eye reference captured when the authored
/// camera rig is established: +X right, +Y up, -Z forward. Keeping them
/// rig-local lets a pure game map them through its current authored camera
/// without a one-frame mismatch when locomotion moves that camera.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// A pointer position in window pixels.
    MouseMove { x: i32, y: i32 },
    /// A wheel notch (±1 per notch).
    MouseWheel { delta: i32 },
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
        Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I,
        Key::J, Key::K, Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R,
        Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
        Key::Up, Key::Down, Key::Left, Key::Right, Key::Space, Key::Enter,
        Key::Escape,
        Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5,
        Key::Num6, Key::Num7, Key::Num8, Key::Num9,
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

#[cfg(test)]
mod tests {
    use super::{InputSnapshot, Key, MouseSnapshot, XrControllerSnapshot, XrInputSnapshot};
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
    fn input_snapshot_omits_absent_xr_and_round_trips_present_xr() {
        let desktop = InputSnapshot {
            held_keys: vec![Key::W],
            mouse: MouseSnapshot { x: 10, y: 20 },
            xr: None,
        };
        let desktop_json = serde_json::to_value(&desktop).unwrap();
        assert_eq!(
            desktop_json,
            serde_json::json!({
                "held_keys": ["W"],
                "mouse": { "x": 10, "y": 20 }
            })
        );

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
    }
}
