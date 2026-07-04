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

    /// Frame-level distance fog: every forward material (including emissive —
    /// fog occludes glow) blends toward the fog color with distance, and the
    /// fog color drives the frame's clear color. Depth passes and the
    /// normals/tangents debug materials don't shade with fog; the physics
    /// overlay mode shades normally, so fog applies there.
    [<Emit("functor_runtime_common::Frame::with_fog($1, $0)")>]
    let withFog (fog: Fog) (frame: Frame): Frame = nativeOnly
