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
            // Note: MouseMove carries a single (int * int) tuple field, not two
            // separate fields. Fable's Rust backend miscompiles a *match* on a
            // union case that has multiple fields (it emits a constructor test
            // with the wrong arity); a single tuple field sidesteps that while
            // construction/destructuring read identically.
            | MouseMove of (int * int)
            | MouseWheel of int

    // TODO: Game Controller event
    // TODO: VR Controller event

    type t =
        | Keyboard of KeyboardEvent.t
        | Mouse of MouseEvent.t

    /// Map a canonical integer key code (functor_runtime_common::Key as i32)
    /// to a Key. Must stay in sync with that enum's #[repr(i32)] values.
    let ofKeyCode (code: int): Key =
        match code with
        | 1 -> A | 2 -> B | 3 -> C | 4 -> D | 5 -> E | 6 -> F | 7 -> G
        | 8 -> H | 9 -> I | 10 -> J | 11 -> K | 12 -> L | 13 -> M
        | 14 -> N | 15 -> O | 16 -> P | 17 -> Q | 18 -> R | 19 -> S
        | 20 -> T | 21 -> U | 22 -> V | 23 -> W | 24 -> X | 25 -> Y | 26 -> Z
        | 27 -> Up | 28 -> Down | 29 -> Left | 30 -> Right
        | 31 -> Space | 32 -> Enter | 33 -> Escape
        | _ -> Unknown
