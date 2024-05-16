module Game

module Effect =
    type t<'msg>

type UpdateFn<'model, 'msg> = 'model -> 'msg -> ('model * Effect.t<'msg>)

type Game<'model, 'msg>

val local : initialState:'model -> Game<'model, 'msg>

val draw3d: ('model -> Graphics.Primitives3D.t) -> Game<'model, 'msg> -> Game<'model, 'msg>

val run : Game<'model, 'msg> -> unit

val hello : string