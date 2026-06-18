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

        // Networking bridge for the web runtime (mirrors the native exports). The
        // web runtime is a separate wasm module, so it reaches the game's outbound
        // command queue / async inbox only through these wasm_bindgen exports.
        // Strings cross as JsValue (the existing marshalling convention) rather
        // than via Fable's string type, to keep the wasm-bindgen ABI simple.
        [<Emit("functor_runtime_common::to_js_value(&functor_runtime_common::net::drain_commands_json())")>]
        let private netDrainCommandsJs () : JsValue = nativeOnly

        [<Emit("functor_runtime_common::net::push_http_response($0 as u64, $1 as u16, functor_runtime_common::from_js_value::<String>($2).into_bytes())")>]
        let private netPushHttpResponseJs (token: int) (status: int) (body: JsValue) : unit = nativeOnly

        [<Emit("functor_runtime_common::net::push_http_error($0 as u64, functor_runtime_common::from_js_value::<String>($1))")>]
        let private netPushHttpErrorJs (token: int) (message: JsValue) : unit = nativeOnly

        /// Host: take the queued networking commands as a JSON string (JsValue).
        [<OuterAttr("wasm_bindgen")>]
        let net_drain_commands_json_wasm () : JsValue = netDrainCommandsJs ()

        /// Host: deliver a completed HTTP response into the game's async inbox.
        [<OuterAttr("wasm_bindgen")>]
        let net_push_http_response_wasm (token: int, status: int, body: JsValue) : unit =
            netPushHttpResponseJs token status body

        /// Host: deliver a transport-level failure for a request into the inbox.
        [<OuterAttr("wasm_bindgen")>]
        let net_push_http_error_wasm (token: int, message: JsValue) : unit =
            netPushHttpErrorJs token message

        // Audio bridge for the web runtime (mirrors the native export). The host
        // drains the queued audio commands each frame and plays them via Web Audio.
        [<Emit("functor_runtime_common::to_js_value(&functor_runtime_common::audio::drain_commands_json())")>]
        let private audioDrainCommandsJs () : JsValue = nativeOnly

        /// Host: take the queued audio commands as a JSON string (JsValue).
        [<OuterAttr("wasm_bindgen")>]
        let audio_drain_commands_json_wasm () : JsValue = audioDrainCommandsJs ()


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

        // Audio bridge: the outbound command queue lives on this (dylib) side; the
        // host drains it through this export each frame and plays on its device,
        // and reports a `playThen` one-shot's end back through `audio_push_finished`.
        [<Emit("functor_runtime_common::audio::drain_commands_json().into()")>]
        let private drainAudioCommandsJson () : string = nativeOnly

        [<Emit("functor_runtime_common::net::drain_conn_commands_json().into()")>]
        let private drainConnCommandsJson () : string = nativeOnly

        [<Emit("functor_runtime_common::net::push_connected($0.to_string(), $1 as u64)")>]
        let private pushConnected (key: string) (conn: int) : unit = nativeOnly

        [<Emit("functor_runtime_common::net::push_message($0.to_string(), $1 as u64, $2.to_string().into_bytes())")>]
        let private pushConnMessage (key: string) (conn: int) (text: string) : unit = nativeOnly

        [<Emit("functor_runtime_common::net::push_disconnected($0.to_string(), $1 as u64)")>]
        let private pushDisconnected (key: string) (conn: int) : unit = nativeOnly

        [<Emit("functor_runtime_common::net::push_conn_error($0.to_string(), $1 as u64, $2.to_string())")>]
        let private pushConnErr (key: string) (conn: int) (message: string) : unit = nativeOnly

        [<Emit("functor_runtime_common::audio::push_finished($0 as u64)")>]
        let private pushAudioFinished (token: int) : unit = nativeOnly

        [<OuterAttr("no_mangle")>]
        let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

        /// Host: take the audio commands the game queued this frame, as a JSON
        /// array of AudioCommand, and play them on the host's audio device.
        [<OuterAttr("no_mangle")>]
        let audio_drain_commands_json () : string = drainAudioCommandsJson ()

        /// Host: report that a `playThen` one-shot (`token`) has finished, so the
        /// game can deliver its completion message.
        [<OuterAttr("no_mangle")>]
        let audio_push_finished (token: int) : unit = pushAudioFinished token

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

        /// Host: take the connection commands (connect/listen/send/close) the game
        /// has queued this frame, as a JSON array of ConnCommand.
        [<OuterAttr("no_mangle")>]
        let net_drain_conn_commands_json () : string = drainConnCommandsJson ()

        /// Host: deliver connection events into the game's inbound queue, tagged
        /// with the connection's key (its endpoint url).
        [<OuterAttr("no_mangle")>]
        let net_push_connected (key: string, conn: int) : unit = pushConnected key conn

        [<OuterAttr("no_mangle")>]
        let net_push_conn_message (key: string, conn: int, text: string) : unit =
            pushConnMessage key conn text

        [<OuterAttr("no_mangle")>]
        let net_push_disconnected (key: string, conn: int) : unit = pushDisconnected key conn

        [<OuterAttr("no_mangle")>]
        let net_push_conn_error (key: string, conn: int, message: string) : unit =
            pushConnErr key conn message

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
        // Keys (endpoint urls) of the connections currently declared/open, to diff
        // against next frame's declared set. Not persisted across hot reload (the
        // host owns the sockets and re-attaches by key on the next reconcile).
        let mutable liveConnKeys: string list = []
        do
            printfn "Hello from GameRunner!"
        interface IRunner with
            member this.getState() =
                // Only the model crosses a hot reload. The effect queue is not
                // transferred: an HTTP effect carries a tagger closure (code in the
                // old dylib) which would dangle after the swap, and the pending-
                // request registry that holds in-flight taggers is dropped with the
                // old dylib too. So in-flight effects/requests are dropped on reload
                // (the executor warns when an orphaned response lands) -- a
                // deliberate trade for the Elm-style `expect` API.
                OpaqueState.to_opaque_type state
            member this.setState(incomingState) =
                let restoredState: 'Model = OpaqueState.unsafe_coerce incomingState
                state <- restoredState
                // Start the reloaded runner with an empty queue. This also discards
                // the constructor-seeded 'init' effect, so 'init' runs once (first
                // load) and not again across a reload.
                effectQueue <- EffectQueue.empty ()
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

                // Recompute the Sub tree once on the settled model; a sub disabled
                // by the effects above no longer participates this frame.
                let subs = GameRunner.subscriptions myGame settledBeforeSubs

                // Timers: enqueue a message for any `Every` that crossed a boundary
                // since last frame, drained through the same update path.
                match lastTts with
                | Some prevTts ->
                    subs
                    |> Sub.messagesForFrame prevTts tts
                    |> Array.iter (fun msg -> EffectQueue.enqueue (Effect.wrapped msg) effectQueue)
                | None -> ()
                lastTts <- Some tts

                // Connection reconciliation: diff the declared connection keys
                // against the live set. Newly declared -> open; no longer declared
                // -> close. `liveConnKeys` is executor state (not persisted), so a
                // hot reload re-opens by key; the host treats Connect as idempotent
                // and reattaches to the still-live socket.
                let conns = Sub.connections subs
                let desiredKeys = conns |> Array.map fst |> Array.toList
                for (key, _) in conns do
                    if not (List.contains key liveConnKeys) then Net.pushConnect key
                for key in liveConnKeys do
                    if not (List.contains key desiredKeys) then Net.pushCloseKey key
                liveConnKeys <- desiredKeys

                // Route inbound connection events to the matching declared
                // connection's decoder (by key). An event for a key no longer
                // declared (e.g. a trailing Disconnected) is dropped.
                for (key, event) in Net.drainConnEvents () do
                    match conns |> Array.tryFind (fun (k, _) -> k = key) with
                    | Some(_, decode) -> EffectQueue.enqueue (Effect.wrapped (decode event)) effectQueue
                    | None -> ()

                // Drain the async inbox: HTTP results that arrived (on some later
                // frame) since the request ran. For each, apply the tagger the
                // request registered (matched by token) and enqueue the resulting
                // message through the same update path. A result with no tagger --
                // an unknown token, or one dropped by a hot reload while the request
                // was in flight -- is logged and discarded rather than crashing.
                for result in Net.drainHttpResults () do
                    let token = result.token
                    match Net.takePending result with
                    | Some msg -> EffectQueue.enqueue (Effect.wrapped msg) effectQueue
                    | None ->
                        printfn "[Functor] dropped HTTP response (token %d): no handler; likely a hot reload while the request was in flight" token

                // Drain audio completions: `Audio.playThen` one-shots the host has
                // reported finished. Match each token to its message and enqueue
                // it through the same update path. A token with no handler
                // (dropped by a hot reload while the sound played) is discarded.
                for token in Audio.drainFinished () do
                    match Audio.takeCompletion token with
                    | Some msg -> EffectQueue.enqueue (Effect.wrapped msg) effectQueue
                    | None -> ()

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

