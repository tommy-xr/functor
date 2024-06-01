namespace Functor

type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * effect<'msg>)

type InputFn<'model, 'msg> = 'model -> Input.t -> ('model * effect<'msg>)

type TickFn<'model, 'msg> = 'model -> Tick.t -> ('model * effect<'msg>)

type Game<'model, 'msg>

module GameBuilder = 
    // GAME DEFINITION FUNCTIONS

    val local : initialState:'model -> Game<'model, 'msg>

    val update : UpdateFn<'model, 'msg> -> Game<'model, 'msg> -> Game<'model, 'msg>

    val input : InputFn<'model, 'msg> -> Game<'model, 'msg> -> Game<'model, 'msg>

    val tick: TickFn<'model, 'msg> -> Game<'model, 'msg> -> Game<'model, 'msg>

    val draw3d: ('model -> Graphics.Scene3D) -> Game<'model, 'msg> -> Game<'model, 'msg>


module GameRunner =
    val initialState: Game<'model, 'msg> -> 'model
    val tick: Game<'model, 'msg> -> 'model -> Tick.t -> ('model * effect<'msg>)
    val draw3d: Game<'model, 'msg> -> 'model -> Graphics.Scene3D