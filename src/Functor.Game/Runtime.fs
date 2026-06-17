module Runtime

    open Fable.Core.Rust
    open Fable.Core

    open Platform
    open Functor

    type IRunner =
        abstract member tick: Time.FrameTime -> unit
        abstract member input: Input.t -> unit
        abstract member render: Time.FrameTime -> Graphics.Frame
        abstract member getState: unit -> OpaqueState
        abstract member setState: OpaqueState -> unit
        abstract member stateDebug: unit -> string

    let mutable currentRunner: Option<IRunner> = None

    open Fable.Core.Rust
    open System.Collections.Generic

    ///////////////////////////////
    // WebAssembly API
    ///////////////////////////////
    [<Fable.Core.Rust.OuterAttr("cfg", [|"target_arch = \"wasm32\""|])>]
    module Wasm = 
        open Fable.Core
        let imports() =
            import "wasm_bindgen::prelude::*" ""
            ()

        [<Erase; Emit("JsValue")>] type JsValue = | Noop

        module UnsafeJsValue =
            [<Emit("functor_runtime_common::to_js_value(&$0)")>]
            let to_js<'a>(obj): JsValue = nativeOnly
            // let from_js<'a>(jsValue: JsValue): 'a = nativeOnly

            [<Emit("functor_runtime_common::from_js_value($0)")>]
            let from_js<'a>(obj: JsValue): 'a = nativeOnly


        [<OuterAttr("wasm_bindgen")>]
        let tick_wasm (frameTimeJs: JsValue): unit =
            let frameTime = frameTimeJs |> UnsafeJsValue.from_js<Time.FrameTime>;
            if currentRunner.IsSome then 
                frameTime
                |> currentRunner.Value.tick
            else 
                raise (System.Exception("No runner"))

        [<OuterAttr("wasm_bindgen")>]
        let key_event_wasm (code: int, isDown: bool): unit =
            if currentRunner.IsSome then
                let key = Input.ofKeyCode code
                let kev = if isDown then Input.KeyboardEvent.KeyDown key else Input.KeyboardEvent.KeyUp key
                currentRunner.Value.input(Input.Keyboard kev)
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("wasm_bindgen")>]
        let mouse_move_wasm (x: int, y: int): unit =
            if currentRunner.IsSome then
                currentRunner.Value.input(Input.Mouse (Input.MouseEvent.MouseMove (x, y)))
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("wasm_bindgen")>]
        let mouse_wheel_wasm (delta: int): unit =
            if currentRunner.IsSome then
                currentRunner.Value.input(Input.Mouse (Input.MouseEvent.MouseWheel delta))
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("wasm_bindgen")>]
        let test_render_wasm (frameTimeJs: JsValue): JsValue =
            let frameTime = frameTimeJs |> UnsafeJsValue.from_js<Time.FrameTime>;
            if currentRunner.IsSome then 
                let ret = 
                    frameTime
                    |> currentRunner.Value.render
                    |> UnsafeJsValue.to_js
                ret
            else 
                raise (System.Exception("No runner"))


    ///////////////////////////////
    // Native API
    ///////////////////////////////
    module Native =
        // Networking bridge. The outbound command queue and async inbox live on
        // this (dylib) side; the host reaches them only through these exports,
        // never by sharing the static across the dylib boundary. The host drains
        // pending commands, performs the I/O, and pushes results back in.
        [<Emit("functor_runtime_common::net::drain_commands_json().into()")>]
        let private drainCommandsJson () : string = nativeOnly

        [<Emit("functor_runtime_common::net::push_http_response($0 as u64, $1 as u16, $2.to_string().into_bytes())")>]
        let private pushHttpResponse (token: int) (status: int) (body: string) : unit = nativeOnly

        [<Emit("functor_runtime_common::net::push_http_error($0 as u64, $1.to_string())")>]
        let private pushHttpError (token: int) (message: string) : unit = nativeOnly

        [<OuterAttr("no_mangle")>]
        let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

        /// Host: take the networking commands the game has queued this frame, as a
        /// JSON array of NetCommand. The host performs the I/O and reports results
        /// back via `net_push_http_response` / `net_push_http_error`.
        [<OuterAttr("no_mangle")>]
        let net_drain_commands_json () : string = drainCommandsJson ()

        /// Host: deliver a completed HTTP response into the game's async inbox.
        [<OuterAttr("no_mangle")>]
        let net_push_http_response (token: int, status: int, body: string) : unit =
            pushHttpResponse token status body

        /// Host: deliver a transport-level failure for a request into the inbox.
        [<OuterAttr("no_mangle")>]
        let net_push_http_error (token: int, message: string) : unit =
            pushHttpError token message

        [<OuterAttr("no_mangle")>]
        let tick(frameTime: Time.FrameTime) =
            if currentRunner.IsSome then 
                currentRunner.Value.tick(frameTime)
            else 
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let key_event(code: int, isDown: bool) =
            if currentRunner.IsSome then
                let key = Input.ofKeyCode code
                let kev = if isDown then Input.KeyboardEvent.KeyDown key else Input.KeyboardEvent.KeyUp key
                currentRunner.Value.input(Input.Keyboard kev)
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let mouse_move(x: int, y: int) =
            if currentRunner.IsSome then
                currentRunner.Value.input(Input.Mouse (Input.MouseEvent.MouseMove (x, y)))
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let mouse_wheel(delta: int) =
            if currentRunner.IsSome then
                currentRunner.Value.input(Input.Mouse (Input.MouseEvent.MouseWheel delta))
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let emit_state(): OpaqueState=
            if currentRunner.IsSome then 
                currentRunner.Value.getState()
            else 
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let set_state(opaqueState: OpaqueState): unit =
            if currentRunner.IsSome then
                currentRunner.Value.setState(opaqueState)
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let emit_state_debug(): string =
            if currentRunner.IsSome then
                currentRunner.Value.stateDebug()
            else
                raise (System.Exception("No runner"))

        [<OuterAttr("no_mangle")>]
        let test_render(frameTime: Time.FrameTime): Graphics.Frame =
            if currentRunner.IsSome then 
                currentRunner.Value.render(frameTime)
            else 
                raise (System.Exception("No runner"))

    // `formatState` renders the live model for introspection (the debug server's
    // /state). It is supplied by `runGame` (which is `inline`, so the `sprintf
    // "%A"` is emitted at the game's concrete call site where the model's derived
    // Debug is available) — keeping this generic executor free of a Debug bound
    // that Fable can't express on a type parameter.
    type GameExecutor<'Msg, 'Model>(game: Game<'Model, 'Msg>, initialState: 'Model, formatState: 'Model -> string) =
        let myGame = game
        let mutable state: 'Model = initialState
        // Seed the queue with the game's startup ('init') effect. Because this
        // happens at construction, a genuine first load drains it on the first
        // tick, while a hot reload immediately overwrites the queue via setState
        // - so the startup effect runs exactly once, never on reload.
        let mutable effectQueue: EffectQueue<'Msg> = EffectQueue.seeded (GameRunner.init game)
        // Total-time (seconds) seen on the previous frame, used to detect timer
        // boundary crossings for `Sub.every`. None until the first frame
        // establishes a baseline (and after a hot reload), so we never report a
        // spurious crossing across a discontinuous jump in the clock.
        let mutable lastTts: Option<float> = None
        do
            printfn "Hello from GameRunner!"
        interface IRunner with
            member this.getState() =
                // Bundle the pending effect queue with the model so in-flight
                // effects survive a hot reload (the new runner starts with an
                // empty queue otherwise, dropping effects enqueued across frames).
                OpaqueState.to_opaque_type (state, effectQueue)
            member this.setState(incomingState) =
                let (restoredState, restoredQueue): ('Model * EffectQueue<'Msg>) =
                    OpaqueState.unsafe_coerce incomingState
                state <- restoredState
                effectQueue <- restoredQueue
            member this.stateDebug() = formatState state
            member this.tick(frameTime: Time.FrameTime) =

                // The game's 'init' effect is seeded into the queue at construction
                // (see above), so it drains here on the first tick like any other effect.

                let tts = float frameTime.tts

                // Drain the effect queue to a fixed point, feeding each resulting
                // message through 'update' and accumulating state. Capped per call
                // so a runaway synchronous effect cascade can't hang the loop; any
                // overflow is deferred to the next frame.
                let maxEffectsPerFrame = 1000
                let drain (startState: 'Model) : 'Model =
                    let mutable processed = 0
                    let mutable settledState = startState
                    let mutable draining = true
                    while draining do
                        match EffectQueue.dequeue effectQueue with
                        | None -> draining <- false
                        | Some eff ->
                            let messages: 'Msg array = Effect.run eff
                            settledState <-
                                Array.fold (fun currentState msg ->
                                    let (newState, effect) = GameRunner.update myGame currentState msg
                                    EffectQueue.enqueue effect effectQueue
                                    newState
                                ) settledState messages

                            processed <- processed + 1
                            if processed >= maxEffectsPerFrame then
                                printfn "[Functor] WARNING: effect queue hit per-frame cap (%d); deferring %d remaining effect(s) to next frame" maxEffectsPerFrame (EffectQueue.count effectQueue)
                                draining <- false
                    settledState

                // Settle effects carried over from previous frames first, so that
                // subscriptions are evaluated against an up-to-date model (an
                // effect processed this frame may change what the game subscribes
                // to -- e.g. disable a timer).
                let settledBeforeSubs = drain state

                // Poll subscriptions on the settled model: recompute the Sub tree
                // and enqueue a message for any timer that crossed a boundary since
                // last frame, then drain those through the same update path. A sub
                // disabled by the effects above no longer fires this frame.
                match lastTts with
                | Some prevTts ->
                    GameRunner.subscriptions myGame settledBeforeSubs
                    |> Sub.messagesForFrame prevTts tts
                    |> Array.iter (fun msg -> EffectQueue.enqueue (Effect.wrapped msg) effectQueue)
                | None -> ()
                lastTts <- Some tts

                // Drain the async inbox: HTTP results that arrived (on some later
                // frame) since the request effect ran. Route each to the game's
                // `Sub.httpResponses` decoders, enqueueing through the same update
                // path. Unlike timers this is gated on results existing, so the
                // common no-network case adds only an empty-array drain.
                let httpResults = Net.drainHttpResults ()
                if httpResults.Length > 0 then
                    GameRunner.subscriptions myGame settledBeforeSubs
                    |> Sub.inboundMessagesForFrame httpResults
                    |> Array.iter (fun msg -> EffectQueue.enqueue (Effect.wrapped msg) effectQueue)

                let settledState = drain settledBeforeSubs

                // Step the simulation on the settled state, queueing any effect it produces.
                let (newState, effect) = GameRunner.tick myGame settledState frameTime
                EffectQueue.enqueue effect effectQueue

                state <- newState
                ()
            member this.input(event: Input.t) =
                // Feed the event through the game's pure 'input' handler. The
                // resulting model is applied immediately; any effect it produces
                // is enqueued and drained by the next 'tick' (alongside its own).
                let (newState, effect) = GameRunner.input myGame state event
                EffectQueue.enqueue effect effectQueue
                state <- newState
                ()
            member this.render(frameTime: Time.FrameTime) =
                GameRunner.draw3d myGame state frameTime
                // printfn "Hello from GameRunner.render!"
                // Scene3D.cube()


    // Holds the module-level mutable assignment so it is emitted in this module
    // rather than inlined into the game's crate (where Fable can't assign to it).
    let setRunner (runner: IRunner) =
        currentRunner <- Some(runner)

    // `inline` so the model formatter below is generated at the game's concrete
    // call site, where the model's Fable-derived Debug instance is available.
    let inline runGame<'Msg, 'Model>(game: Game<'Model, 'Msg>) =
        printfn "runGame"
        let formatState = fun (m: 'Model) -> sprintf "%A" m
        let runner = GameExecutor<'Msg, 'Model>(game, GameRunner.initialState game, formatState)
        setRunner (runner :> IRunner)

