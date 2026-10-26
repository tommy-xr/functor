//! Scene understanding: the headset's per-eye depth of the real room.
//!
//! A game opts in with `config`'s `Xr.scene(true)`. While it does, the depth
//! provider (`XR_META_environment_depth`, Quest 3/3S) runs and every frame
//! acquires its newest image: a two-layer GLES depth texture array (left,
//! right) plus each layer's pose, field of view and near/far planes. The
//! shell turns those into the shared renderer's `EnvironmentDepthView`, in
//! the same world space as the eye cameras, so a pass can unproject a texel
//! to the world point behind it.
//!
//! No Space Setup is needed — the provider senses depth live — but Horizon
//! gates it on the spatial data permission (`USE_SCENE`), which the manifest
//! declares and the user grants once.

use functor_runtime_common::environment_depth::EnvironmentDepthView;
use functor_runtime_common::runtime_settings::{
    scene_requested, EnvironmentDepthMode, EnvironmentDepthState, GameConfig, XrState,
};
use functor_runtime_common::{Camera, TrackingPose};
use openxr as xr;
use xr::sys;
use xr::sys::Handle as _;

pub struct EnvironmentDepth {
    ext: Option<xr::raw::EnvironmentDepthMETA>,
    running: Option<Running>,
    /// A failed creation, kept until depth is no longer wanted so a refusal
    /// is not retried every frame.
    failed: Option<String>,
    /// Images acquired since launch.
    frames: u64,
    /// Whether this frame's acquire returned an image.
    current: bool,
    /// Started this frame: acquiring before the next frame is a call-order
    /// error.
    just_started: bool,
}

/// A running provider and its swapchain, torn down in reverse on drop.
struct Running {
    ext: xr::raw::EnvironmentDepthMETA,
    provider: sys::EnvironmentDepthProviderMETA,
    swapchain: sys::EnvironmentDepthSwapchainMETA,
    textures: Vec<glow::Texture>,
    width: u32,
    height: u32,
}

/// One frame's depth image, ready to place per eye.
pub struct DepthImage {
    texture: glow::Texture,
    near: f32,
    far: f32,
    views: [(xr::Posef, xr::Fovf); 2],
}

impl DepthImage {
    /// Eye `eye`'s layer as the renderer reads it: its pose composed onto the
    /// authored camera exactly like that eye's view, with the image's own
    /// field of view and near/far (`far` may be infinite).
    pub fn view(
        &self,
        eye: usize,
        camera: &Camera,
        reference: TrackingPose,
    ) -> EnvironmentDepthView {
        let (pose, fov) = self.views[eye];
        let depth_camera = Camera {
            near: self.near,
            far: self.far,
            ..camera.compose_tracked_view(reference, crate::tracking_pose(pose))
        };
        let projection = depth_camera.projection_matrix_from_fov_angles(
            fov.angle_left,
            fov.angle_right,
            fov.angle_down,
            fov.angle_up,
        );
        EnvironmentDepthView {
            texture: self.texture,
            layer: eye as i32,
            view_projection: projection * depth_camera.view_matrix(),
        }
    }
}

fn check(result: sys::Result, call: &str) -> Result<(), String> {
    if result.into_raw() < 0 {
        Err(format!("{call}: {result:?}"))
    } else {
        Ok(())
    }
}

impl Running {
    fn start(
        ext: xr::raw::EnvironmentDepthMETA,
        session: &xr::Session<xr::OpenGlEs>,
        gl: &glow::Context,
    ) -> Result<Running, String> {
        let create_info = sys::EnvironmentDepthProviderCreateInfoMETA {
            ty: sys::EnvironmentDepthProviderCreateInfoMETA::TYPE,
            next: std::ptr::null(),
            create_flags: sys::EnvironmentDepthProviderCreateFlagsMETA::EMPTY,
        };
        let mut provider = sys::EnvironmentDepthProviderMETA::NULL;
        // SAFETY: a live session and well-formed create info.
        let result = unsafe {
            (ext.create_environment_depth_provider)(session.as_raw(), &create_info, &mut provider)
        };
        // Horizon gates depth on the spatial-data permission, which the APK
        // declares but never requests.
        if result == sys::Result::ERROR_NOT_PERMITTED_PASSTHROUGH_FB {
            return Err(
                "the spatial data permission is not granted — grant it once with \
`adb shell pm grant dev.functor.runner com.oculus.permission.USE_SCENE`"
                    .to_string(),
            );
        }
        check(result, "xrCreateEnvironmentDepthProviderMETA")?;
        // From here `running`'s drop releases whatever was created.
        let mut running = Running {
            ext,
            provider,
            swapchain: sys::EnvironmentDepthSwapchainMETA::NULL,
            textures: Vec::new(),
            width: 0,
            height: 0,
        };
        let swapchain_info = sys::EnvironmentDepthSwapchainCreateInfoMETA {
            ty: sys::EnvironmentDepthSwapchainCreateInfoMETA::TYPE,
            next: std::ptr::null(),
            create_flags: sys::EnvironmentDepthSwapchainCreateFlagsMETA::EMPTY,
        };
        // SAFETY: the provider was just created.
        check(
            unsafe {
                (ext.create_environment_depth_swapchain)(
                    provider,
                    &swapchain_info,
                    &mut running.swapchain,
                )
            },
            "xrCreateEnvironmentDepthSwapchainMETA",
        )?;
        let mut state = sys::EnvironmentDepthSwapchainStateMETA {
            ty: sys::EnvironmentDepthSwapchainStateMETA::TYPE,
            next: std::ptr::null_mut(),
            width: 0,
            height: 0,
        };
        // SAFETY: a live swapchain and a typed output struct.
        check(
            unsafe { (ext.get_environment_depth_swapchain_state)(running.swapchain, &mut state) },
            "xrGetEnvironmentDepthSwapchainStateMETA",
        )?;
        (running.width, running.height) = (state.width, state.height);

        // The usual two-call idiom: count, then fill typed GLES image structs.
        let mut count = 0;
        // SAFETY: a zero-capacity call only writes the count.
        check(
            unsafe {
                (ext.enumerate_environment_depth_swapchain_images)(
                    running.swapchain,
                    0,
                    &mut count,
                    std::ptr::null_mut(),
                )
            },
            "xrEnumerateEnvironmentDepthSwapchainImagesMETA",
        )?;
        let mut images = vec![
            sys::SwapchainImageOpenGLESKHR {
                ty: sys::SwapchainImageOpenGLESKHR::TYPE,
                next: std::ptr::null_mut(),
                image: 0,
            };
            count as usize
        ];
        // SAFETY: `images` holds `count` typed structs; the API reads them
        // through the base header.
        check(
            unsafe {
                (ext.enumerate_environment_depth_swapchain_images)(
                    running.swapchain,
                    count,
                    &mut count,
                    images.as_mut_ptr() as *mut sys::SwapchainImageBaseHeader,
                )
            },
            "xrEnumerateEnvironmentDepthSwapchainImagesMETA",
        )?;
        for image in &images[..count as usize] {
            let texture = std::num::NonZeroU32::new(image.image)
                .map(glow::NativeTexture)
                .ok_or("the depth swapchain enumerated texture 0")?;
            functor_runtime_common::environment_depth::set_depth_sampling(gl, texture);
            running.textures.push(texture);
        }
        // SAFETY: a live provider.
        check(
            unsafe { (ext.start_environment_depth_provider)(provider) },
            "xrStartEnvironmentDepthProviderMETA",
        )?;
        Ok(running)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // SAFETY: handles this struct created; stopping an unstarted provider
        // only returns an error, which teardown ignores.
        unsafe {
            (self.ext.stop_environment_depth_provider)(self.provider);
            if self.swapchain != sys::EnvironmentDepthSwapchainMETA::NULL {
                (self.ext.destroy_environment_depth_swapchain)(self.swapchain);
            }
            (self.ext.destroy_environment_depth_provider)(self.provider);
        }
    }
}

impl EnvironmentDepth {
    /// `instance` enabled `XR_META_environment_depth` when the runtime offered it.
    pub fn new(instance: &xr::Instance) -> Self {
        let ext = instance.exts().meta_environment_depth;
        log::info!(
            "environment depth: {}",
            if ext.is_some() {
                "available"
            } else {
                "unavailable"
            }
        );
        EnvironmentDepth {
            ext,
            running: None,
            failed: None,
            frames: 0,
            current: false,
            just_started: false,
        }
    }

    /// Follow `config`'s `Xr.scene`: start the provider when wanted, tear it
    /// down when not.
    pub fn update(
        &mut self,
        session: &xr::Session<xr::OpenGlEs>,
        gl: &glow::Context,
        wanted: bool,
    ) {
        if !wanted {
            if self.running.take().is_some() {
                log::info!("environment depth stopped");
            }
            self.failed = None;
            return;
        }
        let Some(ext) = self.ext else { return };
        if self.running.is_some() || self.failed.is_some() {
            return;
        }
        match Running::start(ext, session, gl) {
            Ok(running) => {
                log::info!(
                    "environment depth started: {}x{} x2, {} swapchain images",
                    running.width,
                    running.height,
                    running.textures.len()
                );
                self.running = Some(running);
                self.just_started = true;
            }
            Err(e) => {
                let warning = format!("environment depth could not start: {e}");
                log::warn!("{warning}");
                self.failed = Some(warning);
            }
        }
    }

    /// Forget a failed start so the next wanted frame retries it — called
    /// when the session regains focus, like passthrough's.
    pub fn retry(&mut self) {
        self.failed = None;
    }

    /// This frame's depth image, located in `space` at `time`; `None` until
    /// the provider has one.
    pub fn acquire(&mut self, space: &xr::Space, time: xr::Time) -> Option<DepthImage> {
        self.current = false;
        if std::mem::take(&mut self.just_started) {
            return None;
        }
        let running = self.running.as_ref()?;
        let info = sys::EnvironmentDepthImageAcquireInfoMETA {
            ty: sys::EnvironmentDepthImageAcquireInfoMETA::TYPE,
            next: std::ptr::null(),
            space: space.as_raw(),
            display_time: time,
        };
        let view = sys::EnvironmentDepthImageViewMETA {
            ty: sys::EnvironmentDepthImageViewMETA::TYPE,
            next: std::ptr::null(),
            fov: xr::Fovf {
                angle_left: 0.0,
                angle_right: 0.0,
                angle_up: 0.0,
                angle_down: 0.0,
            },
            pose: xr::Posef::IDENTITY,
        };
        let mut image = sys::EnvironmentDepthImageMETA {
            ty: sys::EnvironmentDepthImageMETA::TYPE,
            next: std::ptr::null(),
            swapchain_index: 0,
            near_z: 0.0,
            far_z: 0.0,
            views: [view, view],
        };
        // SAFETY: a started provider, a live space, typed output structs.
        let result = unsafe {
            (running.ext.acquire_environment_depth_image)(running.provider, &info, &mut image)
        };
        // ENVIRONMENT_DEPTH_NOT_AVAILABLE_META is a success code: no image yet.
        if result != sys::Result::SUCCESS {
            if result.into_raw() < 0 {
                log::warn!("xrAcquireEnvironmentDepthImageMETA: {result:?}");
            }
            return None;
        }
        let texture = *running.textures.get(image.swapchain_index as usize)?;
        self.frames += 1;
        self.current = true;
        Some(DepthImage {
            texture,
            near: image.near_z,
            far: image.far_z,
            views: image.views.map(|v| (v.pose, v.fov)),
        })
    }

    /// The load-response warning for a `config` this headset cannot honour.
    pub fn load_warning(&self, config: Option<GameConfig>) -> Option<String> {
        (scene_requested(config) && self.ext.is_none()).then(|| {
            "config Xr.scene: this headset lacks XR_META_environment_depth; frames use the \
stand-in room"
                .to_string()
        })
    }

    /// `GET /state.xr`: present while `config` requests `Xr.scene`.
    pub fn state(&self, config: Option<GameConfig>) -> Option<XrState> {
        if !scene_requested(config) {
            return None;
        }
        let (width, height) = self
            .running
            .as_ref()
            .map_or((0, 0), |running| (running.width, running.height));
        let warning = if self.ext.is_none() {
            Some("this headset lacks XR_META_environment_depth".to_string())
        } else {
            self.failed.clone()
        };
        let mode = if self.current {
            EnvironmentDepthMode::Device
        } else if self.running.is_some() {
            EnvironmentDepthMode::Waiting
        } else {
            EnvironmentDepthMode::Unavailable
        };
        Some(XrState {
            depth: EnvironmentDepthState {
                mode,
                width,
                height,
                frames: self.frames,
                warning,
            },
        })
    }
}
