module Tick 
    type t = unit

    let frame tick = 0;
    let time tick = Time.now
    let elapsedSinceLastFrame tick = Duration.fromSeconds 0.0
    let totalSinceStart = Duration.fromSeconds 0.0