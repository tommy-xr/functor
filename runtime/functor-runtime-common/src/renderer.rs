use std::sync::Arc;

use cgmath::{Matrix4, SquareMatrix};
use glow::HasContext;

use crate::asset::AssetCache;
use crate::material::BasicMaterial;
use crate::shadow::{self, ShadowMap};
use crate::{
    Camera, DebugRenderMode, Frame, FrameTime, Light, RenderContext, RenderPass, Scene3D,
    SceneContext, ShadowUniforms, Viewport,
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
/// 1. Render-target passes — each declared target's inner frame gets its own
///    shadow + forward pass into the target's offscreen texture, in declaration
///    order (the single shadow map is re-rendered per pass, so each pass sees
///    its own lights' shadows). Double-buffered: the main pass samples the image
///    written *this* frame; a target sampling itself sees *last* frame's.
/// 2. Shadow pass — render the scene into `shadow_map` from the first
///    shadow-casting light (directional or spot), producing `ShadowUniforms`.
/// 3. Forward pass — clear, then `Scene3D::render` with the lights + shadow map.
///
/// Known MVP cost: shells that call `render_frame` more than once per game frame
/// (stereo per-eye, netsim panes) re-render the target passes each call —
/// redundant work, and a *self-sampling* target sees the previous call's image
/// (the other eye's / pane's) rather than the previous game frame's. Correct
/// output per call; splitting target passes from the per-eye main pass is the
/// eventual fix.
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
    // Allocate buffers for EVERY declared target up front, so a target whose
    // scene samples a later-declared target reads last frame's image (initially
    // the clear color) rather than the magenta fallback. A duplicate id is a
    // game bug (two passes fighting over one texture — and, at different sizes,
    // buffer churn every frame): first declaration wins, the rest are skipped.
    let mut declared = std::collections::HashSet::new();
    for pass in &frame.render_targets {
        if declared.insert(pass.target.id.as_str()) {
            scene_context.ensure_render_target(gl, &pass.target);
        } else {
            scene_context.warn_once(
                &format!("duplicate:{}", pass.target.id),
                &format!(
                    "[render-target] \"{}\" is declared more than once in a \
frame — only the first declaration is rendered",
                    pass.target.id
                ),
            );
        }
    }

    let mut rendered = std::collections::HashSet::new();
    for pass in &frame.render_targets {
        if !rendered.insert(pass.target.id.as_str()) {
            continue;
        }
        if !pass.frame.render_targets.is_empty() {
            scene_context.warn_once(
                &format!("nested:{}", pass.target.id),
                &format!(
                    "[render-target] \"{}\": nested render targets inside a \
target frame are ignored (depth 1 only)",
                    pass.target.id
                ),
            );
        }

        let shadow = shadow_pass(
            gl,
            shader_version,
            asset_cache.clone(),
            frame_time.clone(),
            &pass.frame.lights,
            &pass.frame.scene,
            scene_context,
            shadow_map,
        );

        // ensure_render_target above guarantees the entry exists. The handles
        // are Copy — the cache borrow is released before rendering, which
        // re-borrows it for material texture lookups.
        let (fbo, width, height) = scene_context
            .render_target_write(&pass.target.id)
            .expect("render target buffers were just ensured");
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, width as i32, height as i32);
            // The FBO owns the whole texture — no scissoring wanted (the main
            // pass re-enables it, clipped to its viewport pane).
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(0.1, 0.2, 0.3, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        forward_pass(
            gl,
            shader_version,
            asset_cache.clone(),
            scene_context,
            &pass.frame.scene,
            &pass.frame.lights,
            &pass.frame.camera,
            frame_time.clone(),
            debug_render_mode,
            shadow,
            width as f32 / height as f32,
        );
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        scene_context.finish_render_target_write(&pass.target.id);
    }

    let shadow = shadow_pass(
        gl,
        shader_version,
        asset_cache.clone(),
        frame_time.clone(),
        &frame.lights,
        &frame.scene,
        scene_context,
        shadow_map,
    );

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

    forward_pass(
        gl,
        shader_version,
        asset_cache,
        scene_context,
        &frame.scene,
        &frame.lights,
        camera,
        frame_time,
        debug_render_mode,
        shadow,
        viewport.aspect(),
    );
}

/// Shadow pass: render `scene` into `shadow_map` from the first shadow-casting
/// light (directional or spot), before a forward pass. Skinned casters come for
/// free via the shared depth pass in `Scene3D::render`. Ends with the default
/// framebuffer bound.
#[allow(clippy::too_many_arguments)]
fn shadow_pass(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    frame_time: FrameTime,
    lights: &[Light],
    scene: &Scene3D,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
) -> Option<ShadowUniforms> {
    lights
        .iter()
        .enumerate()
        .find_map(|(i, l)| shadow::light_space_matrix(l).map(|m| (i, m)))
        .map(|(light_index, light_space_matrix)| {
            shadow::render_shadow_pass(
                gl,
                shader_version,
                asset_cache,
                frame_time,
                lights,
                scene,
                scene_context,
                shadow_map,
                light_space_matrix,
            );
            ShadowUniforms {
                depth_texture: shadow_map.depth_texture,
                light_space_matrix,
                light_index: light_index as i32,
            }
        })
}

/// Forward pass: `Scene3D::render` from `camera` with `lights` + the shadow map
/// into whatever framebuffer/viewport the caller bound and cleared.
#[allow(clippy::too_many_arguments)]
fn forward_pass(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    scene: &Scene3D,
    lights: &[Light],
    camera: &Camera,
    frame_time: FrameTime,
    debug_render_mode: DebugRenderMode,
    shadow: Option<ShadowUniforms>,
    aspect: f32,
) {
    let render_context = RenderContext {
        gl,
        shader_version,
        asset_cache,
        frame_time,
        debug_render_mode,
        lights,
        render_pass: RenderPass::Forward,
        shadow,
    };

    // The game supplies the camera; derive view/projection from it + the aspect.
    let world_matrix = Matrix4::identity();
    let view_matrix = camera.view_matrix();
    let projection_matrix = camera.projection_matrix(aspect);

    // Root material for nodes that don't set their own (scenes typically override
    // per-node); initialized against this frame's context.
    let mut root_material = BasicMaterial::create();
    root_material.initialize(&render_context);

    Scene3D::render(
        scene,
        &render_context,
        scene_context,
        &world_matrix,
        &projection_matrix,
        &view_matrix,
        &root_material,
    );
}
