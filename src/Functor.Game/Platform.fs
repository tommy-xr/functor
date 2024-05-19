
module Platform

    open Fable.Core
    open Fable.Core.Rust

    [<Emit("functor_runtime_desktop::hello_from_rust()")>]
    let hello_from_rust(): unit = nativeOnly

