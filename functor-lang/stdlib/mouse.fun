//! Mouse buttons, as a variant rather than strings.
//!
//! The mouse twin of `Key`, and built in the same way: the `mouseButton`
//! hook's `button` parameter carries these constructors, so a typo is caught
//! at load time. Match them (`| Mouse.Left =>`) or compare them
//! (`button == Mouse.Right`).

/// A mouse button. Buttons beyond these three are never delivered to game
/// logic.
type t =
  | Left | Right | Middle
