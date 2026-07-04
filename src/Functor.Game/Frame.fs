namespace Graphics

open Fable.Core

/// What a game's draw3d returns: the camera plus the scene to render. A shim
/// over functor_runtime_common::Frame. Built via Frame.create; grows to carry
/// lights etc. later without changing the render boundary signature.
[<Erase; Emit("functor_runtime_common::Frame")>] type Frame = | Noop

module Frame =

    [<Emit("functor_runtime_common::Frame::new($0, $1)")>]
    let create (camera: Camera) (scene: Scene3D): Frame = nativeOnly

    /// Like `create`, plus the lights affecting the scene (ambient / directional).
    [<Emit("functor_runtime_common::Frame::new_lit($0, $1, $2)")>]
    let createLit (camera: Camera) (scene: Scene3D) (lights: Light[]): Frame = nativeOnly

    /// Render `targetFrame` — its own camera/scene/lights — into `target`'s
    /// texture each frame, before this frame's main pass; sample it in this
    /// frame's scene via Texture.renderTarget. Declaration order is render
    /// order; a scene sampling its *own* target sees last frame's image.
    [<Emit("functor_runtime_common::Frame::with_render_target($2, $0, $1)")>]
    let withRenderTarget (target: RenderTarget) (targetFrame: Frame) (frame: Frame): Frame = nativeOnly
