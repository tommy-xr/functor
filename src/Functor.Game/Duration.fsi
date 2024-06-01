module Duration 
    type t

    val toSeconds: t -> float
    val fromSeconds: float -> t