namespace Functor

type TickFn<'model, 'msg> = 'model -> Tick.t -> ('model * effect<'msg>)

type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * effect<'msg>)

type InputFn<'model, 'msg> = 'model -> Input.t -> ('model * effect<'msg>)

type Game<'model, 'msg> = {
    state: 'model
    update: UpdateFn<'model, 'msg>
    render2d: 'model ->  Graphics.Primitives2D.t
    }


open Fable.Core

module Game =

    let local initialState =
        let update model msg = (model, Effect.none)
        let render2d model = Graphics.Primitives2D.Square
        { state = initialState; update = update; render2d = render2d }

    let update<'model, 'msg> (f: UpdateFn<'model, 'msg>) (_game: Game<'model, 'msg>) = 
        printfn "Hello from Game.update!"
        _game

    let input<'model, 'msg> (f: InputFn<'model, 'msg>) (_game: Game<'model, 'msg>) =     
        printfn "Hello from Game.input!"
        _game

    let run<'model, 'msg> (_game: Game<'model, 'msg>) = 
        printfn "Hello from Game.run!"
        ()

    let draw3d<'model, 'msg> (f: 'model -> Graphics.Primitives3D.t) (_game: Game<'model, 'msg>) = 
        printfn "Hello from Game.draw3d!"
        _game

    let tick<'model, 'msg> (f: TickFn<'model, 'msg>) (_game: Game<'model, 'msg>) = 
        printfn "Hello from Game.tick!"
        _game



    let hello = "Hello from functor game!"