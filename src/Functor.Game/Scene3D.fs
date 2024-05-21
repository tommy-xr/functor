module Scene3D
    open Fable.Core

    [<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop

    [<Emit("functor_runtime_common::Scene3D::cube()")>]
    let cube(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::sphere()")>]
    let sphere(): Scene3D = nativeOnly
