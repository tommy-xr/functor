namespace Functor

// Subscriptions: ongoing sources of messages, recomputed from the model every
// frame. Sub is the dual of Effect/Cmd: Effect is a one-shot "do this once",
// Sub is a standing "while the model looks like this, listen to these". The
// runtime walks the Sub tree each frame and feeds any produced messages back
// through `update`, exactly like effects.
//
// `every` is deliberately stateless: it fires on the global time grid (integer
// multiples of its period measured from t=0), so "did it fire this frame?" is a
// pure function of the clock (see `crossedBoundary`). That means it needs no
// per-subscription identity or frame-to-frame diffing, and it survives a hot
// reload for free.
//
// Resource-backed subscriptions (web sockets, etc.) will arrive as new variants
// here. They DO need identity -- a live socket must be matched across
// recomputations so it isn't torn down and reopened every frame -- which is why
// the runtime seam (walk subs -> enqueue -> drain) is built now even though the
// timer is its only client. Game code only ever calls the smart constructors
// below, so adding variants later is additive and breaks no existing games.
type Sub<'msg> =
    | SubNone
    | Every of Duration.t * 'msg
    | Batch of Sub<'msg> array

module Sub =

    let none () : Sub<'msg> = SubNone

    let every (period: Duration.t) (msg: 'msg) : Sub<'msg> = Every(period, msg)

    let batch (subs: Sub<'msg> array) : Sub<'msg> = Batch subs

    let rec map (f: 'a -> 'b) (sub: Sub<'a>) : Sub<'b> =
        match sub with
        | SubNone -> SubNone
        | Every(period, msg) -> Every(period, f msg)
        | Batch subs -> Batch(subs |> Array.map (map f))

    // True iff an integer multiple of `period` lies in the interval
    // (prevTts, tts] -- i.e. a timer boundary was crossed between the previous
    // frame's total-time and this frame's. Pure function of the global clock:
    // no per-timer accumulator, so two evaluations of the same `every` are
    // interchangeable and nothing has to be tracked across frames.
    let crossedBoundary (period: Duration.t) (prevTts: float) (tts: float) : bool =
        let p = Duration.toSeconds period
        p > 0.0 && floor (tts / p) > floor (prevTts / p)

    let rec private collectFired (prevTts: float) (tts: float) (acc: 'msg array) (sub: Sub<'msg>) : 'msg array =
        match sub with
        | SubNone -> acc
        | Every(period, msg) ->
            if crossedBoundary period prevTts tts then Array.append acc [| msg |] else acc
        | Batch subs -> Array.fold (collectFired prevTts tts) acc subs

    // Walk the Sub tree and return the messages that fired this frame, given the
    // previous and current total-time. The runtime feeds these back through the
    // same enqueue -> update path as effects.
    let messagesForFrame (prevTts: float) (tts: float) (sub: Sub<'msg>) : 'msg array =
        collectFired prevTts tts [||] sub
