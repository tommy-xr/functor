namespace Graphics

open Fable.Core
[<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop

[<Erase; Emit("functor_runtime_common::TextureDescription")>] type Texture = | Noop
[<Erase; Emit("functor_runtime_common::MaterialDescription")>] type Material = | Noop

module Scene3D =

    [<Emit("functor_runtime_common::Scene3D::cube()")>]
    let cube(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::sphere()")>]
    let sphere(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::cylinder()")>]
    let cylinder(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::group($0)")>]
    let group (items: Scene3D[]): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::material($0, $1)")>]
    let material (material: Material, items: Scene3D[]): Scene3D = nativeOnly
    
    module Material =
        [<Emit("functor_runtime_common::MaterialDescription::color($0, $1, $2, $3)")>]
        let color (r: float32, b: float32, g: float32, a: float32): Material = nativeOnly


    module Texture2D =
        [<Emit("functor_runtime_common::TextureDescription::file($0)")>]
        let file (str: string): Texture = nativeOnly

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