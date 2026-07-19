//! GLFW → blitz keyboard lowering for the native webview overlay.
//!
//! The run loop collects keyboard input for a focused webview text field as
//! [`WebviewKey`]s — plain data that crosses the worker channel (no blitz
//! types, the `webview_overlay` protocol rule) — and the render worker lowers
//! each one to the [`blitz_traits::events::UiEvent`]s blitz's editing stack
//! expects: a `KeyDown`/`KeyUp` pair (blitz edits on the down; the up keeps
//! the DOM event stream honest). Printable text comes from GLFW's `Char`
//! events (so layout/shift handling is the OS's), the editing subset from
//! `Key` events — the same split the egui `Ui.textInput` route uses.
//!
//! IME composition is NOT wired (no `Ime` events) — CJK/dead-key input is a
//! known follow-up; plain Latin typing works through `Char`.

use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent as BlitzUiEvent};
use blitz_traits::SmolStr;
use keyboard_types::{Code, Key, Location, Modifiers};

/// One keyboard event for a focused webview input, main thread → worker.
/// `Escape` is handled by the worker itself (it blurs the focused element —
/// blitz has no built-in Escape behavior); everything else lowers to blitz
/// key events via [`lower_key`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebviewKey {
    /// A printable character (GLFW `WindowEvent::Char` — post-layout).
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter,
    /// Defocus the focused element (the worker calls `clear_focus`).
    Escape,
}

/// Lower one [`WebviewKey`] to the blitz event(s) it means, for this build's
/// target OS. `Escape` lowers to nothing (the worker handles it directly).
pub(crate) fn lower_key(key: &WebviewKey) -> Vec<BlitzUiEvent> {
    lower_key_for(key, cfg!(target_os = "macos"))
}

/// The OS-explicit core of [`lower_key`], split out so tests cover both
/// paths. On macOS blitz cfg-routes Backspace editing through the Apple
/// standard keybindings (`blitz-dom` `text.rs` — the `Key::Backspace` arm is
/// `#[cfg(not(target_os = "macos"))]`), so a plain `KeyDown(Backspace)`
/// would be silently ignored there.
fn lower_key_for(key: &WebviewKey, apple_keybindings: bool) -> Vec<BlitzUiEvent> {
    let (kt_key, code) = match key {
        WebviewKey::Escape => return Vec::new(),
        WebviewKey::Char(c) => (Key::Character(c.to_string()), Code::Unidentified),
        WebviewKey::Backspace if apple_keybindings => {
            return vec![
                BlitzUiEvent::AppleStandardKeybinding(SmolStr::new_static("deleteBackward:")),
                // The release also FLUSHES the driver queue: blitz's
                // AppleStandardKeybinding arm queues the generated `input`
                // DOM event but only a subsequent `handle_dom_event` drains
                // the queue (KeyUp has no default action of its own).
                BlitzUiEvent::KeyUp(BlitzKeyEvent {
                    key: Key::Backspace,
                    code: Code::Backspace,
                    modifiers: Modifiers::empty(),
                    location: Location::Standard,
                    is_auto_repeating: false,
                    is_composing: false,
                    state: KeyState::Released,
                    text: None,
                }),
            ];
        }
        WebviewKey::Backspace => (Key::Backspace, Code::Backspace),
        WebviewKey::Delete => (Key::Delete, Code::Delete),
        WebviewKey::Left => (Key::ArrowLeft, Code::ArrowLeft),
        WebviewKey::Right => (Key::ArrowRight, Code::ArrowRight),
        WebviewKey::Home => (Key::Home, Code::Home),
        WebviewKey::End => (Key::End, Code::End),
        WebviewKey::Enter => (Key::Enter, Code::Enter),
    };
    let make = |state: KeyState| BlitzKeyEvent {
        key: kt_key.clone(),
        code,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state,
        text: match &kt_key {
            Key::Character(s) if state.is_pressed() => Some(SmolStr::from(s.as_str())),
            _ => None,
        },
    };
    vec![
        BlitzUiEvent::KeyDown(make(KeyState::Pressed)),
        BlitzUiEvent::KeyUp(make(KeyState::Released)),
    ]
}

/// The debug server's `POST /input {"type":"key",…}` names, mapped to the
/// webview route: a single printable character types into the focused field;
/// the editing-key names cover the same subset the GLFW gate routes. `None`
/// means "not a webview key" — the injection falls through to the game (the
/// caller only consults this while the webview wants the keyboard).
pub(crate) fn webview_key_from_str(name: &str) -> Option<WebviewKey> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if !c.is_control() {
            return Some(WebviewKey::Char(c));
        }
    }
    match name.to_ascii_lowercase().as_str() {
        "backspace" => Some(WebviewKey::Backspace),
        "delete" => Some(WebviewKey::Delete),
        "left" => Some(WebviewKey::Left),
        "right" => Some(WebviewKey::Right),
        "home" => Some(WebviewKey::Home),
        "end" => Some(WebviewKey::End),
        "enter" => Some(WebviewKey::Enter),
        "escape" => Some(WebviewKey::Escape),
        // The game map spells the space bar "space"; in a text field it types.
        "space" => Some(WebviewKey::Char(' ')),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down_up(events: &[BlitzUiEvent]) -> (&BlitzKeyEvent, &BlitzKeyEvent) {
        match events {
            [BlitzUiEvent::KeyDown(down), BlitzUiEvent::KeyUp(up)] => (down, up),
            other => panic!("expected KeyDown+KeyUp pair, got {other:?}"),
        }
    }

    #[test]
    fn char_lowers_to_character_pair_with_text_on_the_press() {
        let events = lower_key_for(&WebviewKey::Char('a'), false);
        let (down, up) = down_up(&events);
        assert_eq!(down.key, Key::Character("a".into()));
        assert_eq!(down.state, KeyState::Pressed);
        assert_eq!(down.text.as_deref(), Some("a"));
        assert!(down.modifiers.is_empty());
        assert_eq!(up.key, Key::Character("a".into()));
        assert_eq!(up.state, KeyState::Released);
        assert_eq!(up.text, None);
    }

    #[test]
    fn editing_keys_lower_to_named_pairs() {
        for (key, expect_key, expect_code) in [
            (WebviewKey::Delete, Key::Delete, Code::Delete),
            (WebviewKey::Left, Key::ArrowLeft, Code::ArrowLeft),
            (WebviewKey::Right, Key::ArrowRight, Code::ArrowRight),
            (WebviewKey::Home, Key::Home, Code::Home),
            (WebviewKey::End, Key::End, Code::End),
            (WebviewKey::Enter, Key::Enter, Code::Enter),
        ] {
            let events = lower_key_for(&key, false);
            let (down, up) = down_up(&events);
            assert_eq!(down.key, expect_key, "{key:?}");
            assert_eq!(down.code, expect_code, "{key:?}");
            assert_eq!(down.text, None, "{key:?}");
            assert_eq!(up.state, KeyState::Released, "{key:?}");
        }
    }

    #[test]
    fn backspace_lowers_per_target_os() {
        // Non-mac: a plain named pair.
        let events = lower_key_for(&WebviewKey::Backspace, false);
        let (down, _) = down_up(&events);
        assert_eq!(down.key, Key::Backspace);
        // macOS: blitz only edits Backspace via the Apple standard
        // keybinding, so the lowering must emit that instead — followed by
        // the queue-flushing release (see lower_key_for).
        match &lower_key_for(&WebviewKey::Backspace, true)[..] {
            [BlitzUiEvent::AppleStandardKeybinding(cmd), BlitzUiEvent::KeyUp(up)] => {
                assert_eq!(cmd.as_str(), "deleteBackward:");
                assert_eq!(up.key, Key::Backspace);
            }
            other => panic!("expected AppleStandardKeybinding + KeyUp, got {other:?}"),
        }
    }

    #[test]
    fn escape_lowers_to_nothing() {
        assert!(lower_key_for(&WebviewKey::Escape, false).is_empty());
        assert!(lower_key_for(&WebviewKey::Escape, true).is_empty());
    }

    #[test]
    fn debug_server_names_map_to_webview_keys() {
        assert_eq!(webview_key_from_str("a"), Some(WebviewKey::Char('a')));
        assert_eq!(webview_key_from_str("F"), Some(WebviewKey::Char('F')));
        assert_eq!(webview_key_from_str("~"), Some(WebviewKey::Char('~')));
        assert_eq!(webview_key_from_str("enter"), Some(WebviewKey::Enter));
        assert_eq!(webview_key_from_str("backspace"), Some(WebviewKey::Backspace));
        assert_eq!(webview_key_from_str("escape"), Some(WebviewKey::Escape));
        assert_eq!(webview_key_from_str("space"), Some(WebviewKey::Char(' ')));
        // Multi-char non-names are not webview keys (fall through to the game).
        assert_eq!(webview_key_from_str("up"), None);
        assert_eq!(webview_key_from_str("w"), Some(WebviewKey::Char('w')));
    }
}
