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
/// This is the *shared* per-frame render path: the desktop, web, and XR shells
/// call this with their own GL context, so the shadow + forward orchestration
/// lives in one type-checked place instead of drifting between platforms. The
/// shells keep only what is genuinely platform-specific: creating the GL
/// context, obtaining the `Frame`, computing the viewport, input, and capture.
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
/// 4. Sprite passes — ordered orthographic, alpha-blended layers above 3D.
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
    render_frame_inner(
        gl,
        shader_version,
        asset_cache,
        scene_context,
        shadow_map,
        frame,
        camera,
        None,
        None,
        None,
        frame_time,
        viewport,
        debug_render_mode,
    );
}

/// Render one `Frame` using an externally supplied projection matrix for the
/// main pass. XR shells use this to preserve the runtime's exact asymmetric
/// per-eye frustum. `lod_camera`, `lod_view_projections`, and the shared LOD
/// scale describe the tracked stereo pair used by both eye calls, so terrain
/// draws the union of both eye frusta without regenerating per eye.
/// Render-target passes retain their own cameras and ordinary symmetric
/// projections.
#[allow(clippy::too_many_arguments)]
pub fn render_frame_with_projection(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
    frame: &Frame,
    camera: &Camera,
    projection_matrix: &Matrix4<f32>,
    lod_camera: &Camera,
    lod_view_projections: &[Matrix4<f32>],
    lod_projection_scale: f32,
    lod_viewport_height: f32,
    terrain_frame_id: u64,
    frame_time: FrameTime,
    viewport: Viewport,
    debug_render_mode: DebugRenderMode,
) {
    render_frame_inner(
        gl,
        shader_version,
        asset_cache,
        scene_context,
        shadow_map,
        frame,
        camera,
        Some(projection_matrix),
        Some((
            lod_camera,
            lod_view_projections,
            lod_projection_scale,
            lod_viewport_height,
        )),
        Some(terrain_frame_id),
        frame_time,
        viewport,
        debug_render_mode,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_frame_inner(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
    frame: &Frame,
    camera: &Camera,
    projection_matrix: Option<&Matrix4<f32>>,
    lod_view: Option<(&Camera, &[Matrix4<f32>], f32, f32)>,
    terrain_frame_id: Option<u64>,
    frame_time: FrameTime,
    viewport: Viewport,
    debug_render_mode: DebugRenderMode,
) {
    scene_context.begin_terrain_frame(terrain_frame_id);

    // Allocate buffers for EVERY declared target up front, so a target whose
    // scene samples a later-declared target reads last frame's image (initially
    // the clear color) rather than the magenta fallback. A duplicate id is a
    // game bug (two passes fighting over one texture — and, at different sizes,
    // buffer churn every frame): first declaration wins, the rest are skipped.
    let mut declared = std::collections::HashSet::new();
    for pass in &frame.render_targets {
        if declared.insert(pass.target.id.as_str()) {
            scene_context.ensure_render_target(gl, &pass.target, pass.frame.resolved_clear_color());
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
            let [r, g, b] = pass.frame.resolved_clear_color();
            gl.clear_color(r, g, b, 1.0);
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
            pass.frame.fog.as_ref(),
            pass.frame.skybox.as_ref(),
            width as f32 / height as f32,
            height as f32,
            &pass.frame.camera,
            None,
            None,
            None,
            None,
        );
        render_sprite_layers(
            gl,
            shader_version,
            asset_cache.clone(),
            scene_context,
            &pass.frame,
            frame_time.clone(),
            Viewport::new(width, height),
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
        let [r, g, b] = frame.resolved_clear_color();
        gl.clear_color(r, g, b, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    forward_pass(
        gl,
        shader_version,
        asset_cache.clone(),
        scene_context,
        &frame.scene,
        &frame.lights,
        camera,
        frame_time.clone(),
        debug_render_mode,
        shadow,
        frame.fog.as_ref(),
        frame.skybox.as_ref(),
        viewport.aspect(),
        viewport.height as f32,
        lod_view.map_or(&frame.camera, |(camera, _, _, _)| camera),
        lod_view.map(|(_, projections, _, _)| projections),
        lod_view.map(|(_, _, projection_scale, _)| projection_scale),
        lod_view.map(|(_, _, _, viewport_height)| viewport_height),
        projection_matrix,
    );

    render_sprite_layers(
        gl,
        shader_version,
        asset_cache,
        scene_context,
        frame,
        frame_time,
        viewport,
    );
}

/// Draw the frame's ordered 2D layers after its 3D pass. Sprite scenes reuse
/// the shared quad/material/asset path, but get an orthographic camera and
/// explicit alpha blending with no depth test. Group list order is therefore
/// painter's order: later sprites appear on top.
fn render_sprite_layers(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    frame: &Frame,
    frame_time: FrameTime,
    viewport: Viewport,
) {
    if frame.sprite_layers.is_empty() {
        return;
    }

    unsafe {
        gl.disable(glow::DEPTH_TEST);
        gl.enable(glow::BLEND);
        // Straight-alpha RGB over, while destination alpha uses the standard
        // source-over equation. Applying SRC_ALPHA to the alpha channel too
        // would square edge alpha and leave captures/render targets translucent.
        gl.blend_func_separate(
            glow::SRC_ALPHA,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
        );
    }

    for layer in &frame.sprite_layers {
        let fitted = layer.camera.fitted_viewport(viewport);
        unsafe {
            gl.viewport(
                fitted.x as i32,
                fitted.y as i32,
                fitted.width as i32,
                fitted.height as i32,
            );
            gl.scissor(
                fitted.x as i32,
                fitted.y as i32,
                fitted.width as i32,
                fitted.height as i32,
            );
        }
        let camera = layer.camera.render_camera();
        let projection = layer.camera.projection_matrix();
        forward_pass(
            gl,
            shader_version,
            asset_cache.clone(),
            scene_context,
            &layer.scene,
            &[],
            &camera,
            frame_time.clone(),
            DebugRenderMode::Default,
            None,
            None,
            None,
            fitted.aspect(),
            fitted.height as f32,
            &camera,
            None,
            None,
            None,
            Some(&projection),
        );
    }

    unsafe {
        // Later shell overlays (notably physics debug lines) inherit the frame
        // viewport/scissor, so do not leak the last layer's aspect-fit box.
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
        gl.disable(glow::BLEND);
        gl.enable(glow::DEPTH_TEST);
    }
}

/// Render K `Frame`s into K offscreen targets and composite them onto the bound
/// framebuffer as a weighted average — the screen-space compositor (T5, the
/// shared foundation for fork+overlay and forward-ghosting, docs/time-travel.md).
///
/// Each frame renders through the same shadow + forward path as a normal frame
/// AT ITS OWN paired [`FrameTime`], into its own full-viewport-sized RGBA8
/// target (keyed `__composite_{i}`, reused across frames), then
/// `SceneContext::draw_composite` sums them in-shader. Per-frame time is what
/// lets render-time animation (the skinned-skeleton pose, sampled from the
/// render pass's `tts`) advance across a ghost strobe instead of freezing every
/// division at the paused pose. The composite lands in the default framebuffer
/// *after* the forward work and before any UI overlay, so it shows up in
/// `--capture-frame` PNGs.
///
/// Inputs beyond `MAX_COMPOSITE` are dropped; `weights` is truncated/normalized
/// to the retained count (so equal weights average). Nested render targets inside
/// an input frame are ignored — the compositor renders each frame's main scene
/// only (fork/ghost frames are plain scenes).
pub fn render_composited_frames(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
    frames: &[(Frame, FrameTime)],
    weights: &[f32],
    viewport: Viewport,
    debug_render_mode: DebugRenderMode,
) {
    use crate::composite::{normalize_weights, MAX_COMPOSITE};
    use crate::render_target::RenderTargetDescriptor;

    let n = frames.len().min(weights.len()).min(MAX_COMPOSITE);
    if n == 0 {
        return;
    }
    let weights = normalize_weights(&weights[..n]);
    scene_context.begin_terrain_frame(None);

    // 1. Render each input frame into its own full-viewport offscreen target,
    //    at its own frame time.
    let mut textures: Vec<glow::Texture> = Vec::with_capacity(n);
    for (i, (frame, frame_time)) in frames[..n].iter().enumerate() {
        let id = format!("__composite_{i}");
        let clear = frame.resolved_clear_color();
        let desc = RenderTargetDescriptor {
            id: id.clone(),
            width: viewport.width,
            height: viewport.height,
        };
        scene_context.ensure_render_target(gl, &desc, clear);

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

        let (fbo, width, height) = scene_context
            .render_target_write(&id)
            .expect("composite render target was just ensured");
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, width as i32, height as i32);
            gl.disable(glow::SCISSOR_TEST);
            let [r, g, b] = clear;
            gl.clear_color(r, g, b, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        forward_pass(
            gl,
            shader_version,
            asset_cache.clone(),
            scene_context,
            &frame.scene,
            &frame.lights,
            &frame.camera,
            frame_time.clone(),
            debug_render_mode,
            shadow,
            frame.fog.as_ref(),
            frame.skybox.as_ref(),
            width as f32 / height as f32,
            height as f32,
            &frame.camera,
            None,
            None,
            None,
            None,
        );
        render_sprite_layers(
            gl,
            shader_version,
            asset_cache.clone(),
            scene_context,
            frame,
            frame_time.clone(),
            Viewport::new(width, height),
        );
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        scene_context.finish_render_target_write(&id);
        textures.push(
            scene_context
                .render_target_read_texture(&id)
                .expect("composite render target just written"),
        );
    }

    // 2. Composite the targets onto the bound (default) framebuffer, clipped to
    //    the viewport pane (so it composes with any surrounding shell chrome).
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
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }
    scene_context.draw_composite(gl, shader_version, &textures, &weights);
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
    fog: Option<&crate::fog::Fog>,
    skybox: Option<&crate::skybox::SkyboxDescription>,
    aspect: f32,
    viewport_height: f32,
    lod_camera: &Camera,
    lod_view_projections: Option<&[Matrix4<f32>]>,
    lod_projection_scale: Option<f32>,
    lod_viewport_height: Option<f32>,
    projection_matrix: Option<&Matrix4<f32>>,
) {
    let default_lod_projection = lod_camera.projection_matrix(aspect);
    let mut terrain_frusta = [Matrix4::from_scale(1.0); 2];
    let supplied_frusta = lod_view_projections.unwrap_or(&[]);
    let lod_frustum_count = supplied_frusta.len().clamp(1, terrain_frusta.len());
    if supplied_frusta.is_empty() {
        terrain_frusta[0] = default_lod_projection * lod_camera.view_matrix();
    } else {
        terrain_frusta[..lod_frustum_count].copy_from_slice(&supplied_frusta[..lod_frustum_count]);
    }
    let render_context = RenderContext {
        gl,
        shader_version,
        asset_cache,
        frame_time,
        debug_render_mode,
        lights,
        render_pass: RenderPass::Forward,
        shadow,
        fog,
        camera_pos: cgmath::Vector3::new(camera.eye[0], camera.eye[1], camera.eye[2]),
        lod_camera_pos: cgmath::Vector3::new(
            lod_camera.eye[0],
            lod_camera.eye[1],
            lod_camera.eye[2],
        ),
        lod_view_projections: terrain_frusta,
        lod_frustum_count,
        lod_projection_scale: lod_projection_scale
            .unwrap_or_else(|| default_lod_projection.y.y.abs()),
        viewport_height: lod_viewport_height.unwrap_or(viewport_height),
    };

    // The game supplies the camera; derive its ordinary projection from the
    // aspect unless a platform shell (XR) supplied an exact projection.
    let world_matrix = Matrix4::identity();
    let view_matrix = camera.view_matrix();
    let projection_matrix = projection_matrix
        .copied()
        .unwrap_or_else(|| camera.projection_matrix(aspect));

    // The skybox draws first, behind everything (it writes no depth); fog
    // does not apply to it — the sky IS the horizon.
    if let Some(desc) = skybox {
        scene_context.draw_skybox(&render_context, desc, &projection_matrix, &view_matrix);
    }

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

/// Draw a batch of colored world-space line segments over the current
/// framebuffer — the physics wireframe overlay (`--debug-render physics`,
/// lines from `physics::World::debug_lines`). Depth-tested against the frame
/// just rendered, so wireframes sit *on* their bodies instead of x-raying
/// through the scene.
///
/// Debug-only path: the shader and buffers are built and torn down per call,
/// matching the crate's per-frame material style — simplicity over premature
/// caching for a diagnostic overlay.
///
/// Precondition: call immediately after `render_frame` with the same
/// camera/viewport — the GL viewport and scissor are inherited from it (the
/// `viewport` parameter is used for the aspect ratio only), which is what
/// clips the overlay to the right pane in stereo/multi-pane layouts.
pub fn render_debug_lines(
    gl: &glow::Context,
    shader_version: &str,
    camera: &Camera,
    viewport: Viewport,
    lines: &[crate::physics::DebugLine],
) {
    if lines.is_empty() {
        return;
    }

    // Interleave [pos.xyz, color.rgba] per vertex, two vertices per line.
    let mut vertices: Vec<f32> = Vec::with_capacity(lines.len() * 14);
    for line in lines {
        for p in [line.a, line.b] {
            vertices.extend_from_slice(&p);
            vertices.extend_from_slice(&line.color);
        }
    }

    let view_projection: Matrix4<f32> =
        camera.projection_matrix(viewport.aspect()) * camera.view_matrix();

    unsafe {
        let program = gl.create_program().expect("create debug line program");
        let sources = [
            (
                glow::VERTEX_SHADER,
                format!(
                    "{shader_version}\n\
                     layout(location = 0) in vec3 a_pos;\n\
                     layout(location = 1) in vec4 a_color;\n\
                     uniform mat4 u_view_projection;\n\
                     out vec4 v_color;\n\
                     void main() {{\n\
                         v_color = a_color;\n\
                         gl_Position = u_view_projection * vec4(a_pos, 1.0);\n\
                     }}"
                ),
            ),
            (
                glow::FRAGMENT_SHADER,
                format!(
                    "{shader_version}\n\
                     precision mediump float;\n\
                     in vec4 v_color;\n\
                     out vec4 frag_color;\n\
                     void main() {{ frag_color = v_color; }}"
                ),
            ),
        ];
        let mut shaders = Vec::new();
        for (kind, source) in sources {
            let shader = gl.create_shader(kind).expect("create debug line shader");
            gl.shader_source(shader, &source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                panic!(
                    "debug line shader failed to compile: {}",
                    gl.get_shader_info_log(shader)
                );
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            panic!(
                "debug line program failed to link: {}",
                gl.get_program_info_log(program)
            );
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }

        gl.use_program(Some(program));
        // Same matrix→slice idiom as `shader_program::set_uniform_matrix4`.
        let mvp = cgmath::conv::array4x4(view_projection);
        let mvp_raw = core::slice::from_raw_parts(mvp.as_ptr() as *const f32, 16);
        gl.uniform_matrix_4_f32_slice(
            gl.get_uniform_location(program, "u_view_projection")
                .as_ref(),
            false,
            mvp_raw,
        );

        let counters = crate::gpu_counters::gpu_counters();
        let vao = gl.create_vertex_array().expect("create debug line vao");
        counters.vao_created();
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer().expect("create debug line vbo");
        counters.buffer_created();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        // Same raw-bytes view the mesh upload path uses (indexed_mesh.rs).
        let vertices_u8: &[u8] = core::slice::from_raw_parts(
            vertices.as_ptr() as *const u8,
            std::mem::size_of_val(vertices.as_slice()),
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::STREAM_DRAW);
        counters.uploaded(vertices_u8.len());
        let stride = (7 * std::mem::size_of::<f32>()) as i32;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 3 * 4);

        // Wireframes lie exactly ON collider surfaces, so with LESS half their
        // pixels lose the depth test to the coincident face (z-fighting).
        // LEQUAL lets equal-depth line pixels win; restore LESS after.
        gl.depth_func(glow::LEQUAL);
        gl.draw_arrays(glow::LINES, 0, (vertices.len() / 7) as i32);
        gl.depth_func(glow::LESS);

        gl.bind_vertex_array(None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.use_program(None);
        gl.delete_buffer(vbo);
        counters.buffer_deleted();
        gl.delete_vertex_array(vao);
        counters.vao_deleted();
        gl.delete_program(program);
    }
}
