
module Duration
    type t = { seconds: float}

    let toSeconds duration = duration.seconds
    let fromSeconds seconds = { seconds = seconds }