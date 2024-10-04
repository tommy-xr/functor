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
        let mutable effectQueue: EffectQueue<'Msg> = EffectQueue.empty()
        do
            printfn "Hello from GameRunner!"
        interface IRunner with
            member this.getState() =
                OpaqueState.to_opaque_type state
            member this.setState(incomingState) =
                state <- OpaqueState.unsafe_coerce incomingState
            member this.tick(frameTime: Time.FrameTime) = 
                
                // Todo: If first frame, run 'init'

                printfn "Effect count: %d" (EffectQueue.count effectQueue)
                // Todo: Run any pending effects
                // Print the number of pending effects in the queue

                let maybe_effect = EffectQueue.dequeue effectQueue;

                match maybe_effect with 
                | None -> printfn "No effect"
                | Some _e -> printfn "Got effect"

                let (newState, effect) = GameRunner.tick myGame state frameTime

                EffectQueue.enqueue effect effectQueue;

                // effectQueue <- Array.insertAt effectQueue.Length effect effectQueue

                // Todo: Run tick effects

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

