//! Keyboard keys, as a variant rather than strings.
//!
//! `Key` is built in — no declaration and no import. The `input` hook's `key`
//! parameter and the `Input.snapshot` key sets all carry these constructors,
//! so a misspelling (`Key.Enterr`) is a load-time error instead of an arm that
//! silently never matches. Match them (`| Key.W =>`) or compare them
//! (`key == Key.Enter`).

/// A keyboard key.
///
/// The digit row is `Num0` … `Num9` — constructor names have to be
/// identifiers, so a bare digit is not one of them. `Shift`, `Control` and
/// `Alt` (Option on a Mac) stand for either side. Punctuation is named for
/// the key's position on a US layout (`Minus` is the key right of `Num0`).
/// Keys the platform reports that Functor does not name are never delivered
/// to game logic; Command/Super and the backquote are not keys a game gets.
/// A game's own window keeps Escape for releasing the cursor.
type t =
  | A | B | C | D | E | F | G | H | I | J | K | L | M
  | N | O | P | Q | R | S | T | U | V | W | X | Y | Z
  | Up | Down | Left | Right
  | Space | Enter | Escape
  | Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
  | Tab | Shift | Control | Alt
  | F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12
  | Minus | Equal | LeftBracket | RightBracket | Backslash
  | Semicolon | Quote | Comma | Period | Slash
