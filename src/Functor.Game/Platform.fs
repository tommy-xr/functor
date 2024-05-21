
module Platform

    open Fable.Core

    [<Emit("functor_runtime_desktop::hello_from_rust()")>]
    let hello_from_rust(): unit = nativeOnly
