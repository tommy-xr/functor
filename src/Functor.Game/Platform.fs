
module Platform

    open Fable.Core
    open Fable.Core.Rust

    [<Emit("functor_runtime_desktop::hello_from_rust()")>]
    let hello_from_rust(): unit = nativeOnly

    [<OuterAttr("no_mangle")>]
    let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

