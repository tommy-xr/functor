namespace Functor
open Fable.Core

[<Erase; Emit("functor_runtime_common::EffectQueue<$0>")>] type EffectQueue<'msg> = | Noop


module EffectQueue = 

    [<Emit("functor_runtime_common::EffectQueue::new()")>]
    let empty (): EffectQueue<_> = nativeOnly

    [<Emit("functor_runtime_common::EffectQueue::count(&$0)")>]
    let count (effectQueue: EffectQueue<'a>): int = nativeOnly

    [<Emit("functor_runtime_common::EffectQueue::enqueue(&$1, $0)")>]
    let enqueue (eff: effect<'a>) (effectQueue: EffectQueue<'a>) : unit = nativeOnly

    [<Emit("functor_runtime_common::EffectQueue::dequeue(&$0)")>]
    let dequeue (effectQueue: EffectQueue<'a>): Option<effect<'a>> = nativeOnly