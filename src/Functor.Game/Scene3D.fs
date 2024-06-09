namespace Graphics

open Fable.Core
[<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop

module Scene3D =

    [<Emit("functor_runtime_common::Scene3D::cube()")>]
    let cube(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::sphere()")>]
    let sphere(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::cylinder()")>]
    let cylinder(): Scene3D = nativeOnly

    module Transform = 

        [<Emit("functor_runtime_common::Scene3D::translate_y($1, $0)")>]
        let translateY (y: float32) (scene: Scene3D): Scene3D = nativeOnly