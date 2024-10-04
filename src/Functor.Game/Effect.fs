namespace Functor
open Fable.Core

[<Erase; Emit("functor_runtime_common::Effect<$0>")>] type effect<'msg> = | Noop


module Effect = 

    [<Emit("functor_runtime_common::Effect::none()")>]
    let none (): effect<_> = nativeOnly

    [<Emit("functor_runtime_common::Effect::wrapped($0)")>]
    let wrapped (a: 'a) :  effect<'a> = nativeOnly

    [<Emit("functor_runtime_common::Effect::map($0, $1)")>]
    let map (fn: 'a -> 'b)  (eff: effect<'a>) : effect<'b> = nativeOnly