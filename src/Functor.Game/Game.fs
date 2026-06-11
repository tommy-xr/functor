namespace Functor

type TickFn<'model, 'msg> = 'model -> Time.FrameTime -> ('model * effect<'msg>)

type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * effect<'msg>)

type InputFn<'model, 'msg> = 'model -> Input.t -> ('model * effect<'msg>)

type SubFn<'model, 'msg> = 'model -> Sub<'msg>

type Game<'model, 'msg> = {
    initialState: 'model
    init: effect<'msg>
    input: InputFn<'model, 'msg>
    tick: TickFn<'model, 'msg>
    update: UpdateFn<'model, 'msg>
    subscriptions: SubFn<'model, 'msg>
    draw3d: 'model -> Time.FrameTime -> Graphics.Scene3D
    }


open Fable.Core

module GameBuilder =

    let local initialState =
        let update model msg = (model, Effect.none ())
        let tick model tick = (model, Effect.none ())
        let draw3d model frametime = Graphics.Scene3D.cube()
        let input model input = (model, Effect.none ())
        let subscriptions model = Sub.none ()
        { initialState = initialState; init = Effect.none (); update = update; tick = tick; input = input; subscriptions = subscriptions; draw3d = draw3d}

    /// Set the startup effect, run once when the game first loads. Unlike 'tick'
    /// effects, this fires before the first frame and is *not* re-run across a
    /// hot reload (the persisted effect queue is restored instead).
    let init (effect: effect<'msg>) (game: Game<'model, 'msg>) =
        { game with init = effect }

    let update<'model, 'msg> (f: UpdateFn<'model, 'msg>) (game: Game<'model, 'msg>) =
        { game with update = f }

    let input<'model, 'msg> (f: InputFn<'model, 'msg>) (game: Game<'model, 'msg>) =     
        { game with input = f }

    let draw3d<'model, 'msg> (f: 'model -> Time.FrameTime -> Graphics.Scene3D) (game: Game<'model, 'msg>) = 
        { game with draw3d = f }

    let tick<'model, 'msg> (f: TickFn<'model, 'msg>) (game: Game<'model, 'msg>) =
        { game with tick = f}

    let subscriptions<'model, 'msg> (f: SubFn<'model, 'msg>) (game: Game<'model, 'msg>) =
        { game with subscriptions = f }



module GameRunner =
    let initialState (game: Game<'model, 'msg>) = game.initialState

    let init (game: Game<'model, 'msg>) = game.init

    let tick<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) (tick: Time.FrameTime) = 
        let (newModel, effect) = game.tick model tick
        (newModel, effect)

    let update<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) (msg: 'msg) =
        let (newModel, effect) = game.update model msg
        (newModel, effect)

    let subscriptions<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) =
        game.subscriptions model

    let input<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) (event: Input.t) =
        let (newModel, effect) = game.input model event
        (newModel, effect)

    let draw3d<'model, 'msg> (game: Game<'model, 'msg>) (model: 'model) (tick: Time.FrameTime) =
        game.draw3d model tick