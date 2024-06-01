module Input
    module KeyboardEvent =
        type t = 
            | KeyDown of char
            | KeyUp of char

    module MouseEvent =
        type t = 
            | MouseMove of int * int
            | MouseWheel of int

    // TODO: Game Controller event
    // TODO: VR Controller event

    type t = 
        | KeyboardEvent of KeyboardEvent.t
        | MouseEvent of MouseEvent.t

