//! Environment depth: how far the real room's surfaces are from each eye, for
//! passes that shade onto them (`config`'s `Xr.scene`).
//!
//! A headset shell supplies it from its depth sensor
//! ([`EnvironmentDepthSource::Device`]). Wherever the stand-in room is drawn
//! instead, the renderer renders the frame's stand-in geometry
//! (`Frame.withStandInRoom`) depth-only into [`StandInDepth`]. Both arrive as
//! the same [`EnvironmentDepthView`] — a depth texture-array layer plus the
//! transform that wrote it — so one shader path reads either.

use std::sync::Arc;

use cgmath::Matrix4;
use glow::HasContext;

use crate::runtime_settings::GameConfig;
use crate::{asset::AssetCache, Frame, FrameTime, Scene3D, SceneContext};

/// One eye's environment depth: `layer` of the `TEXTURE_2D_ARRAY` depth
/// texture `texture`, whose texels are window depth in `[0, 1]` under
/// `view_projection` (world → clip, GL conventions). Unprojecting a texel
/// through its inverse recovers the world point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentDepthView {
    pub texture: glow::Texture,
    pub layer: i32,
    pub view_projection: Matrix4<f32>,
}

/// Where the environment depth comes from, declared by the shell before each
/// main pass ([`SceneContext::set_environment_depth_source`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EnvironmentDepthSource {
    /// The game did not request `Xr.scene`.
    #[default]
    None,
    /// The headset's depth image for the eye being rendered.
    Device(EnvironmentDepthView),
    /// No device depth: the frame's stand-in room supplies it.
    StandIn,
}

impl EnvironmentDepthSource {
    /// A shell without depth sensing (desktop, web): the stand-in room while
    /// `config` requests `Xr.scene`.
    pub fn stand_in(config: Option<GameConfig>) -> Self {
        if crate::runtime_settings::scene_requested(config) {
            EnvironmentDepthSource::StandIn
        } else {
            EnvironmentDepthSource::None
        }
    }
}

/// What a main pass reads as environment depth.
#[derive(Debug, PartialEq)]
pub(crate) enum DepthPlan<'a> {
    None,
    Device(EnvironmentDepthView),
    /// Render this stand-in room's depth.
    StandIn(&'a Scene3D),
}

/// Where the stand-in room is drawn (no composited feed), it is the depth —
/// the surfaces on screen are its surfaces. Otherwise the device's, if any.
pub(crate) fn plan(
    source: EnvironmentDepthSource,
    frame: &Frame,
    composited: bool,
) -> DepthPlan<'_> {
    match (source, frame.drawn_stand_in_room(composited)) {
        (EnvironmentDepthSource::None, _) => DepthPlan::None,
        (_, Some(room)) => DepthPlan::StandIn(room),
        (EnvironmentDepthSource::Device(view), None) => DepthPlan::Device(view),
        (EnvironmentDepthSource::StandIn, None) => DepthPlan::None,
    }
}

/// The emulated depth image: a one-layer `DEPTH_COMPONENT24` texture array
/// sized to the main pass, re-rendered each pass that plans it.
pub(crate) struct StandInDepth {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    pub width: u32,
    pub height: u32,
    /// Passes rendered since launch (`/state.xr.depth.frames`).
    pub frames: u64,
}

impl StandInDepth {
    fn new(gl: &glow::Context, width: u32, height: u32) -> StandInDepth {
        unsafe {
            let texture = gl.create_texture().expect("stand-in depth texture");
            crate::gpu_counters::gpu_counters().texture_created();
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.tex_image_3d(
                glow::TEXTURE_2D_ARRAY,
                0,
                glow::DEPTH_COMPONENT24 as i32,
                width as i32,
                height as i32,
                1,
                0,
                glow::DEPTH_COMPONENT,
                glow::UNSIGNED_INT,
                glow::PixelUnpackData::Slice(None),
            );
            // Depth textures are not filterable in GLES 3 / WebGL 2.
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, param, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);

            let fbo = gl.create_framebuffer().expect("stand-in depth fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_layer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                Some(texture),
                0,
                0,
            );
            // Depth only: desktop GL 3.x calls a color-less FBO incomplete
            // unless its draw/read buffers say so.
            gl.draw_buffers(&[glow::NONE]);
            gl.read_buffer(glow::NONE);
            StandInDepth {
                fbo,
                texture,
                width,
                height,
                frames: 0,
            }
        }
    }

    fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.texture);
        }
    }
}

/// The environment depth of the main pass about to render `frame` from
/// `view_projection` into `viewport`: the device's, or — where the stand-in
/// room is drawn — that room rendered depth-only. Restores the caller's
/// framebuffer; the caller resets the viewport.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    frame_time: FrameTime,
    scene_context: &SceneContext,
    frame: &Frame,
    view_projection: Matrix4<f32>,
    viewport: crate::Viewport,
) -> Option<EnvironmentDepthView> {
    let source = scene_context.environment_depth_source();
    match plan(source, frame, scene_context.passthrough_composited()) {
        DepthPlan::None => None,
        DepthPlan::Device(view) => Some(view),
        DepthPlan::StandIn(room) => Some(render_stand_in_depth(
            gl,
            shader_version,
            asset_cache,
            frame_time,
            scene_context,
            room,
            view_projection,
            viewport.width,
            viewport.height,
        )),
    }
}

/// Render `room`'s depth from `view_projection` at `width`×`height`.
#[allow(clippy::too_many_arguments)]
fn render_stand_in_depth(
    gl: &glow::Context,
    shader_version: &str,
    asset_cache: Arc<AssetCache>,
    frame_time: FrameTime,
    scene_context: &SceneContext,
    room: &Scene3D,
    view_projection: Matrix4<f32>,
    width: u32,
    height: u32,
) -> EnvironmentDepthView {
    let previous_fbo = crate::renderer::ambient_framebuffer(gl);
    let mut slot = scene_context.stand_in_depth.borrow_mut();
    if slot
        .as_ref()
        .is_some_and(|depth| (depth.width, depth.height) != (width, height))
    {
        slot.take().expect("checked above").delete(gl);
    }
    let depth = slot.get_or_insert_with(|| StandInDepth::new(gl, width, height));
    depth.frames += 1;
    let (fbo, texture) = (depth.fbo, depth.texture);
    drop(slot);

    let context = crate::shadow::depth_only_context(
        gl,
        shader_version,
        asset_cache,
        frame_time,
        &[],
        height as f32,
    );
    let mut material = crate::material::DepthMaterial::create();
    material.initialize(&context);
    let identity = Matrix4::from_scale(1.0);
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, width as i32, height as i32);
        gl.disable(glow::SCISSOR_TEST);
        gl.clear(glow::DEPTH_BUFFER_BIT);
    }
    // The view-projection rides in the "projection" slot with an identity
    // view, as the shadow pass passes its light matrix.
    room.render(
        &context,
        scene_context,
        &identity,
        &view_projection,
        &identity,
        &material,
    );
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, previous_fbo);
    }
    EnvironmentDepthView {
        texture,
        layer: 0,
        view_projection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Scene3D};

    #[test]
    fn the_drawn_stand_in_room_is_the_depth_else_the_device() {
        let view = EnvironmentDepthView {
            texture: glow::NativeTexture(std::num::NonZeroU32::new(7).unwrap()),
            layer: 1,
            view_projection: Matrix4::from_scale(2.0),
        };
        let device = EnvironmentDepthSource::Device(view);
        let plain = Frame::new(Camera::default(), Scene3D::cube());
        let mut roomed = Frame::with_passthrough(plain.clone());
        roomed = Frame::with_stand_in_room(roomed, Scene3D::cube());
        let room = roomed.stand_in_room.as_deref().unwrap();

        // Not requested: nothing, whatever the frame.
        assert_eq!(
            plan(EnvironmentDepthSource::None, &roomed, false),
            DepthPlan::None
        );
        // The stand-in room is on screen: it is the depth, device or not.
        assert_eq!(
            plan(EnvironmentDepthSource::StandIn, &roomed, false),
            DepthPlan::StandIn(room)
        );
        assert_eq!(plan(device, &roomed, false), DepthPlan::StandIn(room));
        // Under a composited feed the real room is on screen.
        assert_eq!(plan(device, &roomed, true), DepthPlan::Device(view));
        assert_eq!(
            plan(EnvironmentDepthSource::StandIn, &roomed, true),
            DepthPlan::None
        );
        // No stand-in room declared (or not a passthrough frame).
        assert_eq!(
            plan(EnvironmentDepthSource::StandIn, &plain, false),
            DepthPlan::None
        );
        let unpassed = Frame::with_stand_in_room(plain, Scene3D::cube());
        assert_eq!(plan(device, &unpassed, false), DepthPlan::Device(view));
    }
}
