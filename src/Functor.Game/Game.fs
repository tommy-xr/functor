module Game

module Effect =
    type t<'msg> =
    | Noop
    | Function of (unit -> unit)
    | FunctionWithDispatch of (('msg -> unit) -> unit)


type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * Effect.t<'msg>)

type Game<'model, 'msg> = {
    state: 'model
    update: UpdateFn<'model, 'msg>
    render2d: 'model ->  Graphics.Primitives2D.t
    }

let local initialState =
    let update model msg = (model, Effect.Noop)
    let render2d model = Graphics.Primitives2D.Square
    { state = initialState; update = update; render2d = render2d }


let run<'model, 'msg> (_game: Game<'model, 'msg>) = 
    printfn "Hello from Game.run!"
    ()

let draw3d<'model, 'msg> (f: 'model -> Graphics.Primitives3D.t) (_game: Game<'model, 'msg>) = 
    printfn "Hello from Game.draw3d!"
    _game



let hello = "Hello from functor game!"