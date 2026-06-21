use std::sync::Arc;

use cgmath::{ortho, perspective, vec3, InnerSpace, Matrix4, Point3, Rad, SquareMatrix};
use glow::HasContext;

use crate::{
    asset::AssetCache, material::DepthMaterial, FrameTime, Light, RenderContext, RenderPass,
    Scene3D, SceneContext,
};

/// An offscreen render target for shadow maps — the foundation later reused by
/// cubemaps and user render targets. Depth is packed into an RGBA8 *color*
/// texture (the depth material encodes `gl_FragCoord.z`, the lit shader
/// decodes); a depth renderbuffer drives depth testing during the pass. RGBA8 is
/// chosen for portability — `DEPTH_COMPONENT` sampled as a plain `sampler2D`,
/// and even R32F, are unreliable on some drivers (notably macOS).
pub struct ShadowMap {
    pub fbo: glow::Framebuffer,
    pub depth_texture: glow::Texture,
    pub size: u32,
}

impl ShadowMap {
    pub fn new(gl: &glow::Context, size: u32) -> ShadowMap {
        unsafe {
            let depth_texture = gl.create_texture().expect("shadow depth texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(depth_texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size as i32,
                size as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            // Declare a single mip level so the texture is unambiguously complete
            // (some drivers — macOS — otherwise intermittently treat the FBO color
            // texture as "unloadable" and sample zero).
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, 0);
            // Clamp so samples outside the light frustum read the edge; the
            // shader additionally treats out-of-range projections as lit.
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            let depth_rbo = gl.create_renderbuffer().expect("shadow depth rbo");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rbo));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                size as i32,
                size as i32,
            );

            let fbo = gl.create_framebuffer().expect("shadow fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(depth_texture),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth_rbo),
            );

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);

            ShadowMap {
                fbo,
                depth_texture,
                size,
            }
        }
    }
}

/// World→light-clip matrix for a directional light: an orthographic box around
/// the origin, viewed from along `-direction`. The box is fixed-size for now
/// (good enough for the sample scenes); fitting it to the scene bounds is a
/// later refinement.
pub fn directional_light_space_matrix(direction: [f32; 3]) -> Matrix4<f32> {
    let dir = vec3(direction[0], direction[1], direction[2]).normalize();
    let center = vec3(0.0, 0.0, 0.0);
    let distance = 25.0;
    let light_pos = center - dir * distance;

    // Pick an up vector not parallel to the light direction.
    let up = if dir.y.abs() > 0.99 {
        vec3(0.0, 0.0, 1.0)
    } else {
        vec3(0.0, 1.0, 0.0)
    };

    let view = Matrix4::look_at_rh(
        Point3::new(light_pos.x, light_pos.y, light_pos.z),
        Point3::new(center.x, center.y, center.z),
        up,
    );
    let half = 14.0;
    let proj = ortho(-half, half, -half, half, 0.1, 50.0);
    proj * view
}

/// World→light-clip matrix for a spot light: a perspective frustum from
/// `position` along `direction`, with a field of view covering the cone (plus a
/// small margin so PCF near the cone edge stays inside the map) and the far
/// plane at `range`.
pub fn spot_light_space_matrix(
    position: [f32; 3],
    direction: [f32; 3],
    cone_angle: f32,
    range: f32,
) -> Matrix4<f32> {
    let pos = vec3(position[0], position[1], position[2]);
    let dir = vec3(direction[0], direction[1], direction[2]).normalize();
    let target = pos + dir;

    let up = if dir.y.abs() > 0.99 {
        vec3(0.0, 0.0, 1.0)
    } else {
        vec3(0.0, 1.0, 0.0)
    };

    let view = Matrix4::look_at_rh(
        Point3::new(pos.x, pos.y, pos.z),
        Point3::new(target.x, target.y, target.z),
        up,
    );
    // FOV is the full cone angle (cone_angle is the half-angle), with a margin,
    // clamped below pi.
    let fovy = (cone_angle * 2.0 * 1.1).min(3.0);
    let proj = perspective(Rad(fovy), 1.0, 0.1, range.max(1.0));
    proj * view
}

/// The world→light-clip matrix for a shadow-casting light (`None` if the light
/// does not cast, or for types without a shadow path yet — point/ambient).
pub fn light_space_matrix(light: &Light) -> Option<Matrix4<f32>> {
    if !light.casts_shadows() {
        return None;
    }
    match light {
        Light::Directional { direction, .. } => Some(directional_light_space_matrix(*direction)),
        Light::Spot {
            position,
            direction,
            cone_angle,
            range,
            ..
        } => Some(spot_light_space_matrix(
            *position,
            *direction,
            *cone_angle,
            *range,
        )),
        // Point shadows need a cube map (a later step); ambient never casts.
        _ => None,
    }
}

/// Render the scene into `shadow_map` from the light's viewpoint (a depth-only
/// pass). Restores the framebuffer binding afterward; the caller restores the
/// viewport for the main pass.
#[allow(clippy::too_many_arguments)]
pub fn render_shadow_pass(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    frame_time: FrameTime,
    lights: &[Light],
    scene: &Scene3D,
    scene_context: &SceneContext,
    shadow_map: &ShadowMap,
    light_space_matrix: Matrix4<f32>,
) {
    let depth_ctx = RenderContext {
        gl,
        shader_version,
        asset_cache,
        frame_time,
        debug_render_mode: crate::DebugRenderMode::Default,
        lights,
        render_pass: RenderPass::DepthOnly,
        shadow: None,
    };

    let mut depth_material = DepthMaterial::create();
    depth_material.initialize(&depth_ctx);

    let identity = Matrix4::identity();

    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(shadow_map.fbo));
        gl.viewport(0, 0, shadow_map.size as i32, shadow_map.size as i32);
        // Clear the depth-color buffer to 1.0 (far) so untouched texels never
        // shadow anything.
        gl.clear_color(1.0, 1.0, 1.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

        // Pass the light matrix as the "projection" with an identity view; the
        // depth material multiplies projection * view * world.
        scene.render(
            &depth_ctx,
            scene_context,
            &identity,
            &light_space_matrix,
            &identity,
            &depth_material,
        );

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
}
