namespace Graphics

open Fable.Core
open Functor.Math

/// Camera description returned (inside a Frame) by a game's draw3d. A thin
/// shim over functor_runtime_common::Camera; the runtime turns it into view
/// and projection matrices.
[<Erase; Emit("functor_runtime_common::Camera")>] type Camera = | Noop

module Camera =

    /// A sensible default camera (looks at the origin from -Z). Used as the
    /// builder default so a game that never sets a camera still renders.
    [<Emit("functor_runtime_common::Camera::default()")>]
    let initial (): Camera = nativeOnly

    /// Look from `eye` toward `target`, with `fov` as the vertical field of view.
    [<Emit("functor_runtime_common::Camera::look_at([$0.x, $0.y, $0.z], [$1.x, $1.y, $1.z], [$2.x, $2.y, $2.z], $3)")>]
    let lookAt (eye: Vector3) (target: Vector3) (up: Vector3) (fov: Math.Angle): Camera = nativeOnly

    /// First-person camera: look from `eye` along a direction given by `yaw`
    /// (about +Y) and `pitch`. yaw = 0, pitch = 0 looks down +Z.
    [<Emit("functor_runtime_common::Camera::first_person([$0.x, $0.y, $0.z], $1, $2, $3)")>]
    let firstPerson (eye: Vector3) (yaw: Math.Angle) (pitch: Math.Angle) (fov: Math.Angle): Camera = nativeOnly
