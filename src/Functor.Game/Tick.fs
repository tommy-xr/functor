module Tick 
    type t = {
        // Delta time in seconds, for convenience
        dts: float
    }

    let initial = { dts = 0.0 }

    let frame tick = 0;
    let time tick = Time.now
    let elapsedSinceLastFrame tick = Duration.fromSeconds 0.0
    let totalSinceStart = Duration.fromSeconds 0.0
