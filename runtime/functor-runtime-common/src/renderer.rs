use std::sync::Arc;

use cgmath::{Matrix4, SquareMatrix};
use glow::HasContext;

use crate::asset::AssetCache;
use crate::material::BasicMaterial;
use crate::shadow::{self, ShadowMap};
use crate::{
    Camera, DebugRenderMode, Frame, FrameTime, RenderContext, RenderPass, Scene3D, SceneContext,
    ShadowUniforms, Viewport,
};

/// Render one `Frame` to the currently-bound (default) framebuffer.
///
/// This is the *shared* per-frame render path: both shells — the desktop runner
/// (`functor-runtime-desktop`) and the web runtime (`functor-runtime-web`) — call
/// this with their own GL context, so the shadow + forward orchestration lives in
/// one type-checked place instead of being copy-pasted (and drifting) between the
/// two. The shells keep only what is genuinely platform-specific: creating the GL
/// context, obtaining the `Frame` (dylib FFI vs. JsValue marshalling), computing
/// the viewport (window framebuffer vs. canvas), input, and frame capture.
///
/// Steps, mirroring what each shell used to do inline:
/// 1. Shadow pass — render the scene into `shadow_map` from the first
///    shadow-casting light (directional or spot), producing `ShadowUniforms`.
/// 2. Forward pass — clear, then `Scene3D::render` with the lights + shadow map.
pub fn render_frame(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
    frame: &Frame,
    // The camera to render from — usually `&frame.camera`; stereo shells pass
    // each per-eye camera (`Camera::stereo_eyes`) with a per-eye viewport.
    camera: &Camera,
    frame_time: FrameTime,
    viewport: Viewport,
    debug_render_mode: DebugRenderMode,
) {
    // Shadow pass: render the scene into the shadow map from the first
    // shadow-casting light (directional or spot), before the main pass. Skinned
    // casters come for free via the shared depth pass in `Scene3D::render`.
    let shadow = frame
        .lights
        .iter()
        .enumerate()
        .find_map(|(i, l)| shadow::light_space_matrix(l).map(|m| (i, m)))
        .map(|(light_index, light_space_matrix)| {
            shadow::render_shadow_pass(
                gl,
                shader_version,
                asset_cache.clone(),
                frame_time.clone(),
                &frame.lights,
                &frame.scene,
                scene_context,
                shadow_map,
                light_space_matrix,
            );
            ShadowUniforms {
                depth_texture: shadow_map.depth_texture,
                light_space_matrix,
                light_index: light_index as i32,
            }
        });

    // Main (forward) pass into the bound framebuffer, at the viewport's
    // sub-rectangle (x,y default to 0 = full window). The scissor clips the clear
    // and draws to this pane, so multiple instances can share one framebuffer
    // (e.g. a netsim viewer). Reset the clear color (the shadow pass cleared its
    // depth-color buffer to white).
    unsafe {
        gl.viewport(
            viewport.x as i32,
            viewport.y as i32,
            viewport.width as i32,
            viewport.height as i32,
        );
        gl.scissor(
            viewport.x as i32,
            viewport.y as i32,
            viewport.width as i32,
            viewport.height as i32,
        );
        gl.enable(glow::SCISSOR_TEST);
        gl.clear_color(0.1, 0.2, 0.3, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    let render_context = RenderContext {
        gl,
        shader_version,
        asset_cache: asset_cache.clone(),
        frame_time,
        debug_render_mode,
        lights: &frame.lights,
        render_pass: RenderPass::Forward,
        shadow,
    };

    // The game supplies the camera; derive view/projection from it + the aspect.
    let world_matrix = Matrix4::identity();
    let view_matrix = camera.view_matrix();
    let projection_matrix = camera.projection_matrix(viewport.aspect());

    // Root material for nodes that don't set their own (scenes typically override
    // per-node); initialized against this frame's context.
    let mut root_material = BasicMaterial::create();
    root_material.initialize(&render_context);

    Scene3D::render(
        &frame.scene,
        &render_context,
        scene_context,
        &world_matrix,
        &projection_matrix,
        &view_matrix,
        &root_material,
    );
}
