use serde::{Deserialize, Serialize};

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

    /// The canonical name a game's `input` hook receives: `"W"`, `"Up"`,
    /// `"Space"` — and bare digits (`"1"`, not `"Num1"`) for the digit row.
    pub fn name(self) -> String {
        let digit = self as i32 - Key::Num0 as i32;
        if (0..=9).contains(&digit) {
            digit.to_string()
        } else {
            format!("{self:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Key;

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
}
