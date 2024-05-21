namespace Functor

type TickFn<'model, 'msg> = 'model -> Tick.t -> ('model * effect<'msg>)

type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * effect<'msg>)

type InputFn<'model, 'msg> = 'model -> Input.t -> ('model * effect<'msg>)

type Game<'model, 'msg> = {
    initialState: 'model
    input: InputFn<'model, 'msg>
    tick: TickFn<'model, 'msg>
    update: UpdateFn<'model, 'msg>
    render2d: 'model ->  Graphics.Primitives2D.t
    draw3d: 'model -> Graphics.Scene3D
    }


open Fable.Core

module GameBuilder =

    let local initialState =
        let update model msg = (model, Effect.none)
        let tick model tick = (model, Effect.none)
        let render2d model = Graphics.Primitives2D.Square
        let draw3d model = Graphics.Scene3D.cube()
        let input model input = (model, Effect.none)
        { initialState = initialState; update = update; render2d = render2d; tick = tick; input = input; draw3d = draw3d}

    let update<'model, 'msg> (f: UpdateFn<'model, 'msg>) (game: Game<'model, 'msg>) = 
        { game with update = f }

    let input<'model, 'msg> (f: InputFn<'model, 'msg>) (game: Game<'model, 'msg>) =     
        { game with input = f }

    let draw3d<'model, 'msg> (f: 'model -> Graphics.Scene3D) (game: Game<'model, 'msg>) = 
        { game with draw3d = f }

    let tick<'model, 'msg> (f: TickFn<'model, 'msg>) (_game: Game<'model, 'msg>) = 
        printfn "Hello from Game.tick!"
        _game



module GameRunner = 
    let initialState (game: Game<'model, 'msg>) = game.initialState

    let tick<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) (tick: Tick.t) = 
        let (newModel, effect) = game.tick model tick
        (newModel, effect)

    let draw3d<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) =
        game.draw3d model