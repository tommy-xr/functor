namespace Graphics

open Fable.Core

/// What a game's draw3d returns: the camera plus the scene to render. A shim
/// over functor_runtime_common::Frame. Built via Frame.create; grows to carry
/// lights etc. later without changing the render boundary signature.
[<Erase; Emit("functor_runtime_common::Frame")>] type Frame = | Noop

module Frame =

    [<Emit("functor_runtime_common::Frame::new($0, $1)")>]
    let create (camera: Camera) (scene: Scene3D): Frame = nativeOnly
