module Runtime

    open Fable.Core.Rust

    open Platform
    open Functor

    type IRunner =
        abstract member tick: Time.FrameTime -> unit
        abstract member render: Time.FrameTime -> Graphics.Scene3D
        abstract member getState: unit -> OpaqueState
        abstract member setState: OpaqueState -> unit

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
        [<OuterAttr("no_mangle")>]
        let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

        [<OuterAttr("no_mangle")>]
        let tick(frameTime: Time.FrameTime) =
            if currentRunner.IsSome then 
                currentRunner.Value.tick(frameTime)
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
        let test_render(frameTime: Time.FrameTime): Graphics.Scene3D =
            if currentRunner.IsSome then 
                currentRunner.Value.render(frameTime)
            else 
                raise (System.Exception("No runner"))

    type GameExecutor<'Msg, 'Model>(game: Game<'Model, 'Msg>, initialState: 'Model) =
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

                let settledState = drain settledBeforeSubs

                // Step the simulation on the settled state, queueing any effect it produces.
                let (newState, effect) = GameRunner.tick myGame settledState frameTime
                EffectQueue.enqueue effect effectQueue

                state <- newState
                ()
            member this.render(frameTime: Time.FrameTime) = 
                GameRunner.draw3d myGame state frameTime
                // printfn "Hello from GameRunner.render!"
                // Scene3D.cube()


    let runGame<'Msg, 'Model>(game: Game<'Model, 'Msg>) =
        printfn "runGame"
        let runner = GameExecutor<'Msg, 'Model>(game, GameRunner.initialState game)
        currentRunner <- Some(runner :> IRunner)
        ()

