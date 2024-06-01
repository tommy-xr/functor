module Time
    type t = { elapsed: float; delta: float }

    let now () = { elapsed = 0.0; delta = 0.0 }
