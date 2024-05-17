namespace Functor

type effect<'msg> =
    | Noop
    | Function of (unit -> unit)
    | FunctionWithDispatch of (('msg -> unit) -> unit)

module Effect = 
    let none = Noop
