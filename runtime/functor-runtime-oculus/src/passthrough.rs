//! Mixed reality: the headset's camera feed shown under the eye buffer.
//!
//! A game opts in with `config`'s `Xr.passthrough(true)`, may pause the feed
//! live with `settings`' `Xr.passthroughEnabled(false)`, and reveals it by
//! drawing `Frame.withPassthrough` frames, which the shared renderer clears to
//! transparent black while [`Passthrough::composited`] is true.
//!
//! Two ways to show the feed, picked once at startup:
//! - `XR_FB_passthrough` (Quest): a passthrough composition layer submitted
//!   UNDER the projection layer, with the environment blend mode left OPAQUE —
//!   the layer, not the blend mode, brings in the room.
//! - otherwise, a runtime that enumerates `ALPHA_BLEND` blends the eye buffer
//!   over the real world itself.
//!
//! Either way the projection layer is flagged `BLEND_TEXTURE_SOURCE_ALPHA`.
//! Its color is PREMULTIPLIED (the transparent clear plus the renderer's
//! source-over blending stores `color * alpha`), so `UNPREMULTIPLIED_ALPHA`
//! stays off.

use functor_runtime_common::runtime_settings::{GameConfig, PassthroughMode, PassthroughState};
use functor_runtime_common::{Frame, TrackingPose};
use openxr as xr;

/// The camera-rig reference for a frame. A passthrough game's rig (`config`'s
/// `Xr.passthrough`) is room-locked: upright whatever the head's tilt when it
/// centered, with its height re-derived from the authored camera's eye so the
/// stage floor is world `y = 0` (a level camera keeps the room level). Other
/// games keep the head-centered reference unchanged.
pub fn room_locked(
    reference: Option<TrackingPose>,
    config: Option<GameConfig>,
    last_frame: Option<&Frame>,
) -> Option<TrackingPose> {
    match (reference, last_frame) {
        (Some(pose), Some(frame)) if config.is_some_and(|c| c.xr.passthrough) => {
            pose.floor_locked(frame.camera.eye[1]).or(reference)
        }
        _ => reference,
    }
}

/// How this headset can show its camera feed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Backend {
    FbLayer,
    AlphaBlend,
    None,
}

pub struct Passthrough {
    backend: Backend,
    /// Why there is no backend, for `/state` and the load response.
    unavailable: Option<String>,
    running: Option<Running>,
    /// A failed creation, kept until the feed is no longer wanted so a
    /// refusal is not retried every frame.
    failed: Option<String>,
    wanted: bool,
}

struct Running {
    // Fields drop in order: the layer before its parent feature.
    layer: xr::PassthroughLayerFB,
    _feature: xr::Passthrough,
}

impl Passthrough {
    /// `fb_enabled`: `XR_FB_passthrough` was enabled on the instance.
    pub fn new(
        instance: &xr::Instance,
        system: xr::SystemId,
        view_type: xr::ViewConfigurationType,
        fb_enabled: bool,
    ) -> Self {
        let alpha_blend = instance
            .enumerate_environment_blend_modes(system, view_type)
            .map(|modes| modes.contains(&xr::EnvironmentBlendMode::ALPHA_BLEND))
            .unwrap_or(false);
        let fb = fb_enabled
            && system_supports_fb_passthrough(instance, system).unwrap_or_else(|e| {
                log::warn!("passthrough: cannot read the system's passthrough properties: {e}");
                false
            });
        let (backend, unavailable) = if fb {
            (Backend::FbLayer, None)
        } else if alpha_blend {
            (Backend::AlphaBlend, None)
        } else {
            (
                Backend::None,
                Some(
                    "this headset offers neither XR_FB_passthrough nor the ALPHA_BLEND \
environment blend mode; passthrough frames show the placeholder"
                        .to_string(),
                ),
            )
        };
        log::info!("passthrough backend: {backend:?} (ALPHA_BLEND offered: {alpha_blend})");
        Passthrough {
            backend,
            unavailable,
            running: None,
            failed: None,
            wanted: false,
        }
    }

    /// Follow this frame's want (`config` requested it and `settings` has not
    /// paused it). The FB feed is created running on enable and destroyed on
    /// pause: `openxr` 0.21's `Passthrough::start` calls `xrPassthroughPauseFB`,
    /// so a paused feature cannot be resumed through the wrapper.
    pub fn update(&mut self, session: &xr::Session<xr::OpenGlEs>, wanted: bool) {
        if wanted != self.wanted {
            log::info!("passthrough: {}", if wanted { "enabled" } else { "paused" });
        }
        self.wanted = wanted;
        if !wanted {
            if self.running.take().is_some() {
                log::info!("XrPassthroughFB stopped");
            }
            self.failed = None;
            return;
        }
        if self.backend != Backend::FbLayer || self.running.is_some() || self.failed.is_some() {
            return;
        }
        let flags = xr::PassthroughFlagsFB::IS_RUNNING_AT_CREATION;
        let created = session.create_passthrough(flags).and_then(|feature| {
            let layer = session.create_passthrough_layer(
                &feature,
                flags,
                xr::PassthroughLayerPurposeFB::RECONSTRUCTION,
            )?;
            Ok(Running {
                layer,
                _feature: feature,
            })
        });
        match created {
            Ok(running) => {
                log::info!("XrPassthroughFB started (camera layer under the eye buffer)");
                self.running = Some(running);
            }
            Err(e) => {
                let warning = format!("XrPassthroughFB could not start: {e}");
                log::warn!("{warning}; passthrough frames show the placeholder");
                self.failed = Some(warning);
            }
        }
    }

    /// Whether the real world is composited under this frame's eye buffer.
    pub fn composited(&self) -> bool {
        match self.backend {
            Backend::FbLayer => self.running.is_some(),
            Backend::AlphaBlend => self.wanted,
            Backend::None => false,
        }
    }

    /// The load-response warning for a `config` this headset cannot honour.
    pub fn load_warning(&self, config: Option<GameConfig>) -> Option<String> {
        let requested = config.is_some_and(|c| c.xr.passthrough);
        requested
            .then(|| self.unavailable.clone())
            .flatten()
            .map(|reason| format!("config Xr.passthrough: {reason}"))
    }

    /// `GET /state.passthrough`: present while `config` requests it.
    pub fn state(&self, config: Option<GameConfig>) -> Option<PassthroughState> {
        if !config.is_some_and(|c| c.xr.passthrough) {
            return None;
        }
        let warning = self.unavailable.clone().or_else(|| self.failed.clone());
        let mode = if self.composited() {
            PassthroughMode::Composited
        } else if warning.is_some() {
            PassthroughMode::Unavailable
        } else {
            PassthroughMode::Paused
        };
        Some(PassthroughState { mode, warning })
    }

    /// End the frame: the passthrough layer (if running) under `projection`.
    pub fn end_frame(
        &self,
        frame_stream: &mut xr::FrameStream<xr::OpenGlEs>,
        display_time: xr::Time,
        projection: xr::CompositionLayerProjection<'_, xr::OpenGlEs>,
    ) -> xr::Result<()> {
        let composited = self.composited();
        let projection = projection.layer_flags(if composited {
            xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
        } else {
            xr::CompositionLayerFlags::EMPTY
        });
        let blend_mode = if composited && self.backend == Backend::AlphaBlend {
            xr::EnvironmentBlendMode::ALPHA_BLEND
        } else {
            xr::EnvironmentBlendMode::OPAQUE
        };
        // The crate has no builder for this layer type; build the raw struct.
        let underlay = self.running.as_ref().map(|running| {
            use xr::sys::Handle as _;
            xr::sys::CompositionLayerPassthroughFB {
                ty: xr::sys::CompositionLayerPassthroughFB::TYPE,
                next: std::ptr::null(),
                flags: xr::CompositionLayerFlags::EMPTY,
                space: xr::sys::Space::NULL,
                layer_handle: running.layer.as_raw(),
            }
        });
        let mut layers: Vec<&xr::CompositionLayerBase<'_, xr::OpenGlEs>> = Vec::with_capacity(2);
        if let Some(underlay) = &underlay {
            // SAFETY: every OpenXR layer struct begins with the base header,
            // and `CompositionLayerBase` is a transparent wrapper over it. The
            // raw layer and the handle it names outlive `end`.
            layers.push(unsafe {
                &*(underlay as *const xr::sys::CompositionLayerPassthroughFB
                    as *const xr::CompositionLayerBase<'_, xr::OpenGlEs>)
            });
        }
        // First is bottom: the feed under the game.
        layers.push(&projection);
        frame_stream.end(display_time, blend_mode, &layers)
    }
}

/// Whether the system has a passthrough camera (`XR_FB_passthrough` may be
/// enabled on a headset that has none).
fn system_supports_fb_passthrough(
    instance: &xr::Instance,
    system: xr::SystemId,
) -> xr::Result<bool> {
    let mut passthrough = xr::sys::SystemPassthroughProperties2FB {
        ty: xr::sys::SystemPassthroughProperties2FB::TYPE,
        next: std::ptr::null(),
        capabilities: xr::sys::PassthroughCapabilityFlagsFB::EMPTY,
    };
    // SAFETY: both out-structs live through the synchronous call, and the
    // extension that defines the chained struct is enabled on the instance.
    unsafe {
        let mut properties = xr::sys::SystemProperties::out(&mut passthrough as *mut _ as _);
        let result = (instance.fp().get_system_properties)(
            instance.as_raw(),
            system,
            properties.as_mut_ptr(),
        );
        if result.into_raw() < 0 {
            return Err(result);
        }
    }
    Ok(passthrough
        .capabilities
        .contains(xr::sys::PassthroughCapabilityFlagsFB::PASSTHROUGH_CAPABILITY))
}
