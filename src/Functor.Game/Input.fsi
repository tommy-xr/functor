module Input
    /// Logical key identifier. Mirrors functor_runtime_common::Key — the
    /// runtime passes a canonical integer code across the boundary which
    /// ofKeyCode maps back to this DU.
    type Key =
        | Unknown
        | A | B | C | D | E | F | G | H | I | J | K | L | M
        | N | O | P | Q | R | S | T | U | V | W | X | Y | Z
        | Up | Down | Left | Right
        | Space | Enter | Escape

    module KeyboardEvent =
        type t =
            | KeyDown of Key
            | KeyUp of Key

    module MouseEvent =
        type t =
            // Single (int * int) tuple field (not two) to avoid a Fable Rust
            // codegen bug when matching multi-field union cases.
            | MouseMove of (int * int)
            | MouseWheel of int

    // TODO: Game Controller event
    // TODO: VR Controller event

    type t =
        | Keyboard of KeyboardEvent.t
        | Mouse of MouseEvent.t

    /// Map a canonical integer key code (functor_runtime_common::Key as i32)
    /// to a Key. Must stay in sync with that enum's #[repr(i32)] values.
    val ofKeyCode: code:int -> Key
