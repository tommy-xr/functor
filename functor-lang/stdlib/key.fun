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
/// identifiers, so a bare digit is not one of them. Keys the platform reports
/// that Functor does not name are never delivered to game logic.
type t =
  | A | B | C | D | E | F | G | H | I | J | K | L | M
  | N | O | P | Q | R | S | T | U | V | W | X | Y | Z
  | Up | Down | Left | Right
  | Space | Enter | Escape
  | Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
