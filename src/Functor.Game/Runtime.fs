module Runtime

    type IRunner =
        abstract member tick: unit -> unit
        abstract member render: unit -> Graphics.Scene3D

    let mutable currentRunner: Option<IRunner> = None


    open Fable.Core.Rust
    open Functor
    [<OuterAttr("no_mangle")>]
    let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

    [<OuterAttr("no_mangle")>]
    let test_render(): Graphics.Scene3D =
        if currentRunner.IsSome then 
            currentRunner.Value.render()
        else 
            raise (System.Exception("No runner"))

    type GameExecutor<'Msg, 'Model>(game: Game<'Model, 'Msg>, initialState: 'Model) =
        let myGame = game
        let mutable state: 'Model = initialState
        do
            printfn "Hello from GameRunner!"
        interface IRunner with
            member this.tick() = 
                let (newState, effects) = GameRunner.tick myGame state Tick.initial
                state <- newState
                ()
            member this.render() = 
                GameRunner.draw3d myGame state
                // printfn "Hello from GameRunner.render!"
                // Scene3D.cube()


    let runGame<'Msg, 'Model>(game: Game<'Model, 'Msg>) =
        printfn "runGame"
        let runner = GameExecutor<'Msg, 'Model>(game, GameRunner.initialState game)
        currentRunner <- Some(runner :> IRunner)
        ()

