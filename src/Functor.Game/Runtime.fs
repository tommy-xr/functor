module Runtime

    type IRunner =
        abstract member tick: unit -> unit
        abstract member render: unit -> Scene3D.Scene3D

    let mutable currentRunner: Option<IRunner> = None


    open Fable.Core.Rust
    open Functor
    [<OuterAttr("no_mangle")>]
    let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

    [<OuterAttr("no_mangle")>]
    let test_render(): Scene3D.Scene3D = Scene3D.cube()

    type GameRunner<'Msg, 'Model>(game: Game<'Model, 'Msg>, initialState: 'Model) =
        let myGame = game
        let mutable state: 'Model = initialState
        do
            printfn "Hello from GameRunner!"
        interface IRunner with
            member this.tick() = 
                let (newState, effects) = myGame.tick state Tick.initial
                state <- newState
                ()
            member this.render() = 
                printfn "Hello from GameRunner.render!"
                Scene3D.cube()


    let runGame<'Msg, 'Model>(game: Game<'Model, 'Msg>) =
        printfn "runGame"
        let runner = GameRunner<'Msg, 'Model>(game, game.state)
        currentRunner <- Some(runner :> IRunner)
        ()

