namespace Graphics

open Fable.Core
[<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop
[<Erase; Emit("functor_runtime_common::Material")>] type Material = | Noop
[<Erase; Emit("functor_runtime_common::Texture")>] type Texture = | Noop
[<Erase; Emit("functor_runtime_common::Mesh")>] type Mesh = | Noop

open Functor.Math

module Scene3D =

    [<Emit("functor_runtime_common::Scene3D::cube()")>]
    let cube(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::sphere()")>]
    let sphere(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::cylinder()")>]
    let cylinder(): Scene3D = nativeOnly

    let mesh(mesh: Mesh): Scene3D = nativeOnly;

    // Not yet implemented
    [<Emit("functor_runtime_common::Scene3D::group($0)")>]
    let group(items: list<Scene3D>): Scene3D = nativeOnly;

    module Transform = 
        let translateX  (x: float) (item: Scene3D): Scene3D = nativeOnly;
        let translateY  (y: float) (item: Scene3D): Scene3D = nativeOnly;
        let translateZ  (z: float) (item: Scene3D): Scene3D = nativeOnly;
        let scaleX  (x: float) (item: Scene3D): Scene3D = nativeOnly;
        let scaleY  (y: float) (item: Scene3D): Scene3D = nativeOnly;
        let scaleZ  (z: float) (item: Scene3D): Scene3D = nativeOnly;
        let rotateX (angleX: Angle) (item: Scene3D): Scene3D = nativeOnly;
        let rotateY (angleY: Angle) (item: Scene3D): Scene3D = nativeOnly;
        let rotateZ (angleZ: Angle) (item: Scene3D): Scene3D = nativeOnly;


