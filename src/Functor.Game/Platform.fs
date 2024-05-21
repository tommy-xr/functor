
module Platform

    open Fable.Core
    open Fable.Core.Rust

    [<Emit("functor_runtime_desktop::hello_from_rust()")>]
    let hello_from_rust(): unit = nativeOnly

    module Scene3D =
        
        [<Erase; Emit("functor_runtime_common::Scene3D")>]
        type Scene3D = | Noop

        [<Emit("functor_runtime_common::Scene3D::cube()")>]
        let cube(): Scene3D = nativeOnly


        [<Emit("functor_runtime_common::Scene3D::sphere()")>]
        let sphere(): Scene3D = nativeOnly

    [<OuterAttr("no_mangle")>]
    let dynamic_call_from_rust num = printfn "Hello from F# called from Rust! %f" num

    [<OuterAttr("no_mangle")>]
    let test_render(): Scene3D.Scene3D = Scene3D.sphere()




