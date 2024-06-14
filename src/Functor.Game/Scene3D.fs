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

    [<Emit("functor_runtime_common::Scene3D::group($0)")>]
    let group (items: Scene3D[]): Scene3D = nativeOnly

    module Transform = 
        [<Emit("functor_runtime_common::Scene3D::translate_x($1, $0)")>]
        let translateX (x: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::translate_y($1, $0)")>]
        let translateY (y: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::translate_z($1, $0)")>]
        let translateZ (z: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::scale_x($1, $0)")>]

        let scaleX (x: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::scale_y($1, $0)")>]
        let scaleY (y: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::scale_z($1, $0)")>]
        let scaleZ (z: float32) (scene: Scene3D): Scene3D = nativeOnly

        [<Emit("functor_runtime_common::Scene3D::rotate_x($1, $0)")>]
        let rotateX (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::rotate_y($1, $0)")>]
        let rotateY (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::rotate_z($1, $0)")>]
        let rotateZ (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly