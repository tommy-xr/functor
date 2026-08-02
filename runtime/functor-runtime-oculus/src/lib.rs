//! The Quest (Meta Horizon OS) runtime shell: OpenXR + EGL/GLES, rendering
//! through the same `functor_runtime_common::render_frame` path as the
//! desktop and web shells.
//!
//! Phase 1 (this file): a complete, honest OpenXR shell that cross-compiles —
//! android_main → EGL context → OpenXR instance/session → per-eye swapchains →
//! frame loop rendering a `Frame` per eye with head-pose cameras. The scene is
//! a placeholder until device bring-up wires the Functor Lang producer + network
//! reload (`POST /reload-source`) so games arrive over the network — the
//! runtime APK ships once, games are text.
//!
//! Structure mirrors shock2quest's `runtimes/oculus_runtime` (the reference
//! implementation) with its hard-won Quest gotchas kept:
//! - EGL config chosen MANUALLY, not via `eglChooseConfig` (Android injects
//!   multisample flags into the match).
//! - A tiny pbuffer surface backs the context; OpenXR renders into swapchain
//!   images, never a window surface.
//! - Swapchains are `SRGB8_ALPHA8`; `FRAMEBUFFER_SRGB` must NOT be enabled on
//!   top (double-sRGB washout). GLES has no GL_FRAMEBUFFER_SRGB toggle — the
//!   sRGB encode happens because the swapchain format says so.
//! - Two swapchains, one per eye, two render passes. Multiview
//!   (`GL_OVR_multiview2`) is an optimization for later.
//!
//! Modernized vs shock2quest: crates.io `openxr` 0.21 (its vendored openxrs
//! fork predates the 0.18 release that shipped GLES/Android support),
//! `android-activity` instead of the deprecated `ndk-glue`.

use std::collections::HashSet;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::debug_protocol::{
    CaptureError, DebugRequest, InputCommand, RuntimeState, RuntimeView, RuntimeViewport,
};
use functor_runtime_common::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform};
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::{
    Frame, FrameTime, GameClock, InputSnapshot, Key, SceneContext, TrackingPose, Viewport,
    XrControllerSnapshot, XrInputSnapshot,
};
use khronos_egl as egl;
use openxr as xr;

/// GLES version to request. Quest 3 supports 3.2.
/// Display refresh rate to aim for, in Hz.
///
/// Quest 3 offers 72/80/90/120; the runtime defaults to 72 and only changes on
/// request. This picks the highest offered rate that does not exceed the
/// target, so a headset advertising fewer options still gets a sane choice.
///
/// Raising this raises the frame budget's difficulty in exact proportion —
/// 13.8 ms at 72 Hz, 12.5 at 80, 11.1 at 90, 8.3 at 120 — so it is a measured
/// decision, not a preference. See the `oculus-profiling` skill.
///
/// 80 Hz measured clean on a Quest 3 running `examples/terrain` once foveation
/// was enabled (10.6 ms of a 12.5 ms budget, zero stale frames). 90 Hz did not
/// hold: the mean looks close, but p5 collapses to 45-70 fps and 670-998 frames
/// go stale in 30 s, which is visible judder rather than a rounding error.
const TARGET_REFRESH_RATE: f32 = 80.0;

const GLES_MAJOR: i32 = 3;
const GLES_MINOR: i32 = 2;

/// Color format for the swapchains (GL_SRGB8_ALPHA8): the compositor expects
/// sRGB-encoded output, and an sRGB internal format makes GL do the encode.
/// TODO(device bring-up): the shared shaders are gamma-naive (desktop/web
/// write raw values to non-sRGB targets), so the format-driven encode here
/// will render brighter than the desktop reference — expect to make the
/// shared pipeline gamma-explicit once it's visible on device.
const COLOR_FORMAT: u32 = glow::SRGB8_ALPHA8;

struct EglContext {
    instance: egl::DynamicInstance<egl::EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    /// A throwaway 16x16 pbuffer: EGL requires *some* surface to make the
    /// context current, but all real rendering goes to swapchain FBOs.
    _pbuffer: egl::Surface,
}

/// Create the EGL context OpenXR will share. The config is selected by hand:
/// `eglChooseConfig` on Android silently prefers multisampled configs, which
/// the XR compositor rejects — iterate `get_configs` and match exact
/// attributes instead (the shock2quest lesson).
fn init_egl() -> EglContext {
    let lib = unsafe { libloading::Library::new("libEGL.so") }.expect("load libEGL.so");
    let instance = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required_from(lib) }
        .expect("load EGL 1.4");

    let display = unsafe { instance.get_display(egl::DEFAULT_DISPLAY) }.expect("EGL display");
    instance.initialize(display).expect("EGL initialize");

    // EGL_OPENGL_ES3_BIT: not exposed by khronos-egl's 1.4 API surface (it's
    // an EGL 1.5 / KHR_create_context constant), value per the Khronos
    // registry.
    const OPENGL_ES3_BIT: egl::Int = 0x0040;

    let mut chosen = None;
    let configs = {
        // Size to the driver's real count — Adreno exposes hundreds, and a
        // truncated list can hide the one config we need (`get_configs` fills
        // only up to the Vec's capacity).
        let count = instance
            .get_config_count(display)
            .expect("EGL config count");
        let mut configs = Vec::with_capacity(count);
        instance
            .get_configs(display, &mut configs)
            .expect("EGL configs");
        configs
    };
    for config in configs {
        let attr = |a: egl::Int| instance.get_config_attrib(display, config, a).unwrap_or(-1);
        // Exact 8888, no depth/stencil (per-eye renderbuffers own depth), no
        // MSAA — plus the two masks Meta's reference ovrEgl checks: the
        // config must back an ES3 context and a pbuffer surface.
        if attr(egl::RED_SIZE) == 8
            && attr(egl::GREEN_SIZE) == 8
            && attr(egl::BLUE_SIZE) == 8
            && attr(egl::ALPHA_SIZE) == 8
            && attr(egl::DEPTH_SIZE) == 0
            && attr(egl::STENCIL_SIZE) == 0
            && attr(egl::SAMPLES) == 0
            && attr(egl::RENDERABLE_TYPE) & OPENGL_ES3_BIT != 0
            && attr(egl::SURFACE_TYPE) & egl::PBUFFER_BIT != 0
        {
            chosen = Some(config);
            break;
        }
    }
    let config = chosen.expect("no ES3-capable 8888 EGL config without MSAA");

    let context = instance
        .create_context(
            display,
            config,
            None,
            &[
                egl::CONTEXT_MAJOR_VERSION,
                GLES_MAJOR,
                egl::CONTEXT_MINOR_VERSION,
                GLES_MINOR,
                egl::NONE,
            ],
        )
        .expect("EGL context");

    let pbuffer = instance
        .create_pbuffer_surface(
            display,
            config,
            &[egl::WIDTH, 16, egl::HEIGHT, 16, egl::NONE],
        )
        .expect("EGL pbuffer");

    instance
        .make_current(display, Some(pbuffer), Some(pbuffer), Some(context))
        .expect("EGL make_current");

    EglContext {
        instance,
        display,
        config,
        context,
        _pbuffer: pbuffer,
    }
}


/// Foveation level to request. `HIGH` shades the far periphery at the lowest
/// rate and is the usual choice for content whose edges are ground texture
/// rather than text or thin geometry — which is exactly a terrain vista.
const FOVEATION_LEVEL: xr::sys::FoveationLevelFB = xr::sys::FoveationLevelFB::HIGH;

/// Attach a fixed-foveation profile to one swapchain.
///
/// `xrUpdateSwapchainFB` has no safe wrapper in the `openxr` crate — only the
/// raw function pointer — so the state struct is built and passed by hand. The
/// call is per swapchain, and we render one per eye.
///
/// Failure is logged and ignored: foveation is an optimisation, and a headset
/// that refuses it should still render.
fn attach_foveation(
    instance: &xr::Instance,
    swapchain: &xr::Swapchain<xr::OpenGlEs>,
    profile: &xr::FoveationProfileFB,
    eye: &str,
) {
    let Some(ext) = instance.exts().fb_swapchain_update_state.as_ref() else {
        return;
    };
    let mut state = xr::sys::SwapchainStateFoveationFB {
        ty: xr::sys::SwapchainStateFoveationFB::TYPE,
        next: std::ptr::null_mut(),
        flags: xr::sys::SwapchainStateFoveationFlagsFB::EMPTY,
        profile: profile.as_raw(),
    };
    // The state structs are a tagged union discriminated by `ty`; the API takes
    // the base header and reads the concrete struct behind it.
    let header = &mut state as *mut _ as *mut xr::sys::SwapchainStateBaseHeaderFB;
    let result = unsafe { (ext.update_swapchain)(swapchain.as_raw(), header) };
    if result != xr::sys::Result::SUCCESS {
        log::warn!("foveation not applied to the {eye} swapchain: {result:?}");
    }
}

/// One eye's rendering target: an OpenXR swapchain plus an FBO + depth
/// renderbuffer per swapchain image.
struct EyeTarget {
    swapchain: xr::Swapchain<xr::OpenGlEs>,
    framebuffers: Vec<glow::Framebuffer>,
    width: u32,
    height: u32,
    capture_supported: bool,
}

fn create_eye_target(
    gl: &glow::Context,
    session: &xr::Session<xr::OpenGlEs>,
    view: &xr::ViewConfigurationView,
) -> EyeTarget {
    use glow::HasContext;
    let width = view.recommended_image_rect_width;
    let height = view.recommended_image_rect_height;
    let create = |usage_flags| {
        session.create_swapchain(&xr::SwapchainCreateInfo {
            create_flags: xr::SwapchainCreateFlags::EMPTY,
            usage_flags,
            format: COLOR_FORMAT,
            sample_count: 1,
            width,
            height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        })
    };
    // Readback makes the image a transfer source under OpenXR. Prefer a
    // swapchain that declares that usage, but do not make the entire VR app
    // unstartable on a runtime that cannot provide it: rendering remains
    // available and `/capture` reports 503 instead.
    let base_usage = xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::SAMPLED;
    let (swapchain, capture_supported) =
        match create(base_usage | xr::SwapchainUsageFlags::TRANSFER_SRC) {
            Ok(swapchain) => (swapchain, true),
            Err(xr::sys::Result::ERROR_FEATURE_UNSUPPORTED) => {
                log::warn!(
                    "OpenXR runtime does not support transfer-source swapchains; capture disabled"
                );
                (create(base_usage).expect("create swapchain"), false)
            }
            Err(error) => panic!("create swapchain: {error}"),
        };

    // Wrap every swapchain image in an FBO with a fresh depth renderbuffer.
    let framebuffers = swapchain
        .enumerate_images()
        .expect("swapchain images")
        .into_iter()
        .map(|texture| unsafe {
            let fbo = gl.create_framebuffer().expect("fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(glow::NativeTexture(
                    std::num::NonZeroU32::new(texture).unwrap(),
                )),
                0,
            );
            let depth = gl.create_renderbuffer().expect("depth rb");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                width as i32,
                height as i32,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            // A real check, not debug_assert: in a release APK an incomplete
            // FBO would otherwise become silent garbage on the display.
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            assert_eq!(
                status,
                glow::FRAMEBUFFER_COMPLETE,
                "swapchain framebuffer incomplete: 0x{status:x}"
            );
            fbo
        })
        .collect();

    EyeTarget {
        swapchain,
        framebuffers,
        width,
        height,
        capture_supported,
    }
}

/// Convert OpenXR's pose layout into the shell-independent tracking pose used
/// by the authored camera rig and sampled controller input.
fn tracking_pose(pose: xr::Posef) -> TrackingPose {
    let p = pose.position;
    let o = pose.orientation;
    TrackingPose::new([p.x, p.y, p.z], [o.x, o.y, o.z, o.w])
}

fn tracking_pose_from_view(view: &xr::View) -> TrackingPose {
    tracking_pose(view.pose)
}

/// OpenXR action set backing the target-neutral sampled-input snapshot.
///
/// One action per logical control uses left/right subaction paths. That keeps
/// the public snapshot symmetric and lets another OpenXR target bind the same
/// controls without introducing Quest button names into the engine contract.
struct XrActions {
    action_set: xr::ActionSet,
    left_path: xr::Path,
    right_path: xr::Path,
    grip: xr::Action<xr::Posef>,
    aim: xr::Action<xr::Posef>,
    trigger: xr::Action<f32>,
    squeeze: xr::Action<f32>,
    thumbstick: xr::Action<xr::Vector2f>,
    primary: xr::Action<bool>,
    secondary: xr::Action<bool>,
    thumbstick_click: xr::Action<bool>,
    menu: xr::Action<bool>,
    left_grip_space: xr::Space,
    right_grip_space: xr::Space,
    left_aim_space: xr::Space,
    right_aim_space: xr::Space,
}

impl XrActions {
    fn create(instance: &xr::Instance, session: &xr::Session<xr::OpenGlEs>) -> Self {
        let left_path = instance
            .string_to_path("/user/hand/left")
            .expect("left hand path");
        let right_path = instance
            .string_to_path("/user/hand/right")
            .expect("right hand path");
        let hands = [left_path, right_path];
        let action_set = instance
            .create_action_set("gameplay", "Gameplay input", 0)
            .expect("create gameplay action set");
        let grip = action_set
            .create_action::<xr::Posef>("grip_pose", "Grip pose", &hands)
            .expect("create grip action");
        let aim = action_set
            .create_action::<xr::Posef>("aim_pose", "Aim pose", &hands)
            .expect("create aim action");
        let trigger = action_set
            .create_action::<f32>("trigger", "Trigger", &hands)
            .expect("create trigger action");
        let squeeze = action_set
            .create_action::<f32>("squeeze", "Squeeze", &hands)
            .expect("create squeeze action");
        let thumbstick = action_set
            .create_action::<xr::Vector2f>("thumbstick", "Thumbstick", &hands)
            .expect("create thumbstick action");
        let primary = action_set
            .create_action::<bool>("primary", "Primary button", &hands)
            .expect("create primary action");
        let secondary = action_set
            .create_action::<bool>("secondary", "Secondary button", &hands)
            .expect("create secondary action");
        let thumbstick_click = action_set
            .create_action::<bool>("thumbstick_click", "Thumbstick click", &hands)
            .expect("create thumbstick click action");
        let menu = action_set
            .create_action::<bool>("menu", "Menu button", &hands)
            .expect("create menu action");

        let path = |value: &str| instance.string_to_path(value).expect("OpenXR input path");
        let touch_profile = path("/interaction_profiles/oculus/touch_controller");
        instance
            .suggest_interaction_profile_bindings(
                touch_profile,
                &[
                    xr::Binding::new(&grip, path("/user/hand/left/input/grip/pose")),
                    xr::Binding::new(&grip, path("/user/hand/right/input/grip/pose")),
                    xr::Binding::new(&aim, path("/user/hand/left/input/aim/pose")),
                    xr::Binding::new(&aim, path("/user/hand/right/input/aim/pose")),
                    xr::Binding::new(&trigger, path("/user/hand/left/input/trigger/value")),
                    xr::Binding::new(&trigger, path("/user/hand/right/input/trigger/value")),
                    xr::Binding::new(&squeeze, path("/user/hand/left/input/squeeze/value")),
                    xr::Binding::new(&squeeze, path("/user/hand/right/input/squeeze/value")),
                    xr::Binding::new(&thumbstick, path("/user/hand/left/input/thumbstick")),
                    xr::Binding::new(&thumbstick, path("/user/hand/right/input/thumbstick")),
                    xr::Binding::new(&primary, path("/user/hand/left/input/x/click")),
                    xr::Binding::new(&primary, path("/user/hand/right/input/a/click")),
                    xr::Binding::new(&secondary, path("/user/hand/left/input/y/click")),
                    xr::Binding::new(&secondary, path("/user/hand/right/input/b/click")),
                    xr::Binding::new(
                        &thumbstick_click,
                        path("/user/hand/left/input/thumbstick/click"),
                    ),
                    xr::Binding::new(
                        &thumbstick_click,
                        path("/user/hand/right/input/thumbstick/click"),
                    ),
                    xr::Binding::new(&menu, path("/user/hand/left/input/menu/click")),
                ],
            )
            .expect("suggest Touch controller bindings");

        // Portable simple-controller fallback for non-Meta OpenXR runtimes.
        // Analog controls remain inactive; fully tracked grip/aim poses plus
        // select/menu still populate the same typed snapshot.
        instance
            .suggest_interaction_profile_bindings(
                path("/interaction_profiles/khr/simple_controller"),
                &[
                    xr::Binding::new(&grip, path("/user/hand/left/input/grip/pose")),
                    xr::Binding::new(&grip, path("/user/hand/right/input/grip/pose")),
                    xr::Binding::new(&aim, path("/user/hand/left/input/aim/pose")),
                    xr::Binding::new(&aim, path("/user/hand/right/input/aim/pose")),
                    xr::Binding::new(&primary, path("/user/hand/left/input/select/click")),
                    xr::Binding::new(&primary, path("/user/hand/right/input/select/click")),
                    xr::Binding::new(&menu, path("/user/hand/left/input/menu/click")),
                    xr::Binding::new(&menu, path("/user/hand/right/input/menu/click")),
                ],
            )
            .expect("suggest simple controller bindings");

        session
            .attach_action_sets(&[&action_set])
            .expect("attach gameplay action set");
        let left_grip_space = grip
            .create_space(session, left_path, xr::Posef::IDENTITY)
            .expect("left grip space");
        let right_grip_space = grip
            .create_space(session, right_path, xr::Posef::IDENTITY)
            .expect("right grip space");
        let left_aim_space = aim
            .create_space(session, left_path, xr::Posef::IDENTITY)
            .expect("left aim space");
        let right_aim_space = aim
            .create_space(session, right_path, xr::Posef::IDENTITY)
            .expect("right aim space");

        Self {
            action_set,
            left_path,
            right_path,
            grip,
            aim,
            trigger,
            squeeze,
            thumbstick,
            primary,
            secondary,
            thumbstick_click,
            menu,
            left_grip_space,
            right_grip_space,
            left_aim_space,
            right_aim_space,
        }
    }

    fn sample(
        &self,
        session: &xr::Session<xr::OpenGlEs>,
        stage: &xr::Space,
        time: xr::Time,
        reference: TrackingPose,
        head: TrackingPose,
    ) -> XrInputSnapshot {
        let controller = |path, grip_space: &xr::Space, aim_space: &xr::Space| {
            let grip_active = self.grip.is_active(session, path).unwrap_or(false);
            let aim_active = self.aim.is_active(session, path).unwrap_or(false);
            let trigger = self.trigger.state(session, path).expect("trigger state");
            let squeeze = self.squeeze.state(session, path).expect("squeeze state");
            let thumbstick = self
                .thumbstick
                .state(session, path)
                .expect("thumbstick state");
            let primary = self.primary.state(session, path).expect("primary state");
            let secondary = self
                .secondary
                .state(session, path)
                .expect("secondary state");
            let thumbstick_click = self
                .thumbstick_click
                .state(session, path)
                .expect("thumbstick click state");
            let menu = self.menu.state(session, path).expect("menu state");
            let locate = |active: bool, space: &xr::Space| {
                active
                    .then(|| space.locate(stage, time).ok())
                    .flatten()
                    .filter(|location| {
                        location
                            .location_flags
                            .contains(xr::SpaceLocationFlags::POSITION_VALID)
                            && location
                                .location_flags
                                .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
                            && location
                                .location_flags
                                .contains(xr::SpaceLocationFlags::POSITION_TRACKED)
                            && location
                                .location_flags
                                .contains(xr::SpaceLocationFlags::ORIENTATION_TRACKED)
                    })
                    .and_then(|location| tracking_pose(location.pose).relative_to(reference))
            };
            XrControllerSnapshot {
                active: grip_active
                    || aim_active
                    || trigger.is_active
                    || squeeze.is_active
                    || thumbstick.is_active
                    || primary.is_active
                    || secondary.is_active
                    || thumbstick_click.is_active
                    || menu.is_active,
                grip: locate(grip_active, grip_space),
                aim: locate(aim_active, aim_space),
                trigger: trigger
                    .is_active
                    .then_some(trigger.current_state.clamp(0.0, 1.0))
                    .unwrap_or(0.0),
                squeeze: squeeze
                    .is_active
                    .then_some(squeeze.current_state.clamp(0.0, 1.0))
                    .unwrap_or(0.0),
                thumbstick: if thumbstick.is_active {
                    [
                        thumbstick.current_state.x.clamp(-1.0, 1.0),
                        thumbstick.current_state.y.clamp(-1.0, 1.0),
                    ]
                } else {
                    [0.0, 0.0]
                },
                primary_pressed: primary.is_active && primary.current_state,
                secondary_pressed: secondary.is_active && secondary.current_state,
                thumbstick_pressed: thumbstick_click.is_active && thumbstick_click.current_state,
                menu_pressed: menu.is_active && menu.current_state,
            }
        };

        XrInputSnapshot {
            head: head.relative_to(reference),
            left: controller(self.left_path, &self.left_grip_space, &self.left_aim_space),
            right: controller(
                self.right_path,
                &self.right_grip_space,
                &self.right_aim_space,
            ),
        }
    }
}

/// Placeholder frame until the Functor Lang producer is wired (device bring-up): an
/// empty scene under a shadow-casting directional light, so `render_frame`
/// runs the full shadow + forward pipeline every frame — the point of phase 1
/// is proving the shared renderer's whole GLES path, not just a clear.
/// The boot scene: what the tool APK renders before any game is pushed
/// (the network reload replaces it live, model preserved). Interpreted by
/// the same embedded producer a pushed game runs under.
const BOOT_SCENE: &str = include_str!("boot.fun");

/// The push endpoint's device-loopback port (`adb forward tcp:8123 tcp:8123`).
const RELOAD_PORT: u16 = 8123;

#[derive(Default)]
struct DebugLoopState {
    frame_count: u64,
    input: InputSnapshot,
    // Debug requests may arrive while paused or between device fixed steps.
    input_edges: functor_runtime_common::InputEdges,
    last_frame: Option<Frame>,
    pending_capture: Option<mpsc::Sender<Result<Vec<u8>, CaptureError>>>,
}

fn service_debug_request(
    request: DebugRequest,
    game: &mut dyn GameProducer,
    clock: &mut GameClock,
    debug: &mut DebugLoopState,
    eyes: &[EyeTarget],
    session_running: bool,
    asset_cache: &Arc<AssetCache>,
    scene_context: &SceneContext,
) {
    match request {
        DebugRequest::Capture(response) => {
            if !session_running {
                let _ = response.send(Err(CaptureError::Unavailable(
                    "capture is unavailable while the XR session is not rendering".to_string(),
                )));
            } else if eyes.iter().any(|eye| !eye.capture_supported) {
                let _ = response.send(Err(CaptureError::Unavailable(
                    "capture is unsupported by this OpenXR runtime".to_string(),
                )));
            } else if debug.pending_capture.is_some() {
                let _ = response.send(Err(CaptureError::Failed(
                    "another capture is already pending".to_string(),
                )));
            } else {
                debug.pending_capture = Some(response);
            }
        }
        DebugRequest::State(response) => {
            let width = eyes
                .iter()
                .try_fold(0_u32, |total, eye| total.checked_add(eye.width))
                .unwrap_or(u32::MAX);
            let height = eyes.iter().map(|eye| eye.height).max().unwrap_or(0);
            let names = ["left", "right"];
            let views = eyes
                .iter()
                .enumerate()
                .map(|(index, eye)| {
                    RuntimeView::new(
                        names.get(index).copied().unwrap_or("view"),
                        eye.width,
                        eye.height,
                    )
                })
                .collect();
            let mut input = debug.input.clone();
            debug.input_edges.apply_to(&mut input);
            let _ = response.send(RuntimeState {
                frame: debug.frame_count,
                tts: clock.current_tts(),
                pending_steps: clock.pending_steps(),
                viewport: RuntimeViewport::new(width, height),
                views,
                model: game.state_json(),
                model_debug: game.state_debug(),
                input,
            });
        }
        DebugRequest::Scene(response) => {
            let json = debug
                .last_frame
                .as_ref()
                .map(|frame| serde_json::to_string_pretty(frame))
                .transpose()
                .unwrap_or_else(|error| Some(format!("{{\"error\":{:?}}}", error.to_string())))
                .unwrap_or_else(|| "{\"error\":\"no frame rendered yet\"}".to_string());
            let _ = response.send(json);
        }
        DebugRequest::Trace(response) => {
            let _ = response.send(game.inspector_trace(clock.is_paused()));
        }
        DebugRequest::Input(command, response) => {
            let result = match command {
                InputCommand::Key { key, down } => match Key::from_name(&key) {
                    Some(key) if down => {
                        game.key_event(key as i32, true);
                        functor_runtime_common::apply_key_transition_to_snapshot(
                            &mut debug.input,
                            &mut debug.input_edges,
                            key,
                            true,
                            true,
                        );
                        Ok(())
                    }
                    Some(key) => {
                        if functor_runtime_common::apply_key_transition_to_snapshot(
                            &mut debug.input,
                            &mut debug.input_edges,
                            key,
                            false,
                            true,
                        ) {
                            game.key_event(key as i32, false);
                        }
                        Ok(())
                    }
                    None => Err(format!("unknown key: {key}")),
                },
                InputCommand::MouseMove { x, y } => {
                    debug.input.mouse.x = x;
                    debug.input.mouse.y = y;
                    game.mouse_move(x, y);
                    Ok(())
                }
                InputCommand::MouseWheel { delta } => {
                    game.mouse_wheel(delta);
                    Ok(())
                }
                // Edge + held level, mirroring the injected-key discipline: a
                // release only reaches the game if it saw the press. There is
                // no cursor capture to respect on this runtime.
                InputCommand::MouseButton { button, down } => {
                    match functor_runtime_common::MouseButton::from_name(&button) {
                        Some(b) => {
                            if down {
                                functor_runtime_common::apply_mouse_transition_to_snapshot(
                                    &mut debug.input,
                                    &mut debug.input_edges,
                                    b,
                                    true,
                                    true,
                                );
                                game.mouse_button(b as i32, true);
                            } else if functor_runtime_common::apply_mouse_transition_to_snapshot(
                                &mut debug.input,
                                &mut debug.input_edges,
                                b,
                                false,
                                true,
                            ) {
                                game.mouse_button(b as i32, false);
                            }
                            Ok(())
                        }
                        None => Err(format!("unknown mouse button: {button}")),
                    }
                }
                InputCommand::UiEvent { slot, kind } => {
                    game.ui_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
                InputCommand::WebviewEvent { slot, kind } => {
                    game.webview_event(functor_runtime_common::ui::UiEvent { slot, kind });
                    Ok(())
                }
                // Rejected rather than honored: this runtime resamples the XR
                // domain from live OpenXR tracking every frame, so an injected
                // sample would be silently overwritten before any `sampledInput`
                // saw it. Injection is the DESKTOP substitute for this device.
                InputCommand::Xr(_) | InputCommand::XrClear => Err(
                    "xr injection is unsupported on the device runtime (it samples live \
tracking); use the desktop runtime's --headless/--debug-port path"
                        .to_string(),
                ),
            };
            if clock.is_paused() {
                game.absorb_paused_input();
            }
            let _ = response.send(result);
        }
        DebugRequest::Time(command, response) => {
            let _ = response.send(clock.apply(command));
        }
        DebugRequest::Project(response) => {
            let _ = response.send(game.project_sources());
        }
        DebugRequest::ReloadSource(source, response) => {
            let _ = response.send(game.reload_source(&source));
        }
        DebugRequest::ReloadProject(files, role, response) => {
            let _ = response.send(functor_runtime_common::protocol::reload_with_role(
                game,
                role,
                |game| game.reload_project(&files),
            ));
        }
        DebugRequest::LoadProject(files, role, response) => {
            let result = functor_runtime_common::protocol::load_with_role(game, role, |game| {
                game.load_project(&files)
            });
            if result.is_ok() {
                clock.restart();
                debug.frame_count = 0;
            }
            let _ = response.send(result);
        }
        DebugRequest::ReloadAsset(asset, response) => {
            let changed = asset_cache.replace_uploaded(&asset.path, asset.bytes);
            if changed {
                scene_context.evict_asset(&asset.path);
            }
            let status = if changed { "reloaded" } else { "unchanged" };
            log::info!("{status} project asset '{}'", asset.path);
            let _ = response.send(Ok(format!("{status} asset {}", asset.path)));
        }
        DebugRequest::SyncAssets(paths, response) => {
            let current: HashSet<String> = paths.into_iter().collect();
            let removed = asset_cache.retain_uploaded(&current);
            for path in &removed {
                scene_context.evict_asset(path);
                log::info!("removed project asset '{path}'");
            }
            let _ = response.send(Ok(format!(
                "synced {} asset(s), removed {}",
                current.len(),
                removed.len()
            )));
        }
        DebugRequest::Rewind(frame, response) => {
            let result = game.rewind_scene_to(frame);
            if result.is_ok() {
                if let Some(tts) = game.current_scene_tts() {
                    clock.rebase(tts as f32);
                }
            }
            let _ = response.send(result);
        }
    }
}

#[no_mangle]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("functor"),
    );
    log::info!("functor oculus runtime starting");

    // Route runtime events (asset errors, hot-reload status, Debug.log
    // traces) into logcat — without a sink they fall back to eprintln!,
    // which Android discards.
    functor_runtime_common::events::set_sink(Box::new(|event| {
        use functor_runtime_common::events::RuntimeEvent as R;
        match event {
            R::AssetError { path, message } => match path {
                Some(path) => {
                    log::error!("asset '{path}' failed to load; using fallback: {message}")
                }
                None => log::error!("asset failed to load; using fallback: {message}"),
            },
            R::HotReload { ok, message } => {
                if ok {
                    log::info!("hot-reload: {message}");
                } else {
                    log::error!("hot-reload: {message}");
                }
            }
            R::FunctorLangTrace { message } => log::info!("{message}"),
            // CLI-stream concerns; quiet on device.
            R::Ready | R::FrameStats { .. } | R::CaptureWritten { .. } => {}
        }
    }));

    // Meta's runtime refuses to bind a session to an activity that isn't
    // resumed with a window ("xrCreateSession: Activity is not yet in the
    // ready state") — the session then parks in IDLE forever and Horizon's
    // loading interstitial times out. Pump the Android event loop until the
    // activity is ready before touching EGL/OpenXR.
    let mut resumed = false;
    let mut destroyed = false;
    while !destroyed && !(resumed && app.native_window().is_some()) {
        app.poll_events(Some(std::time::Duration::from_millis(16)), |event| {
            if let PollEvent::Main(main) = event {
                match main {
                    MainEvent::Resume { .. } => resumed = true,
                    MainEvent::Pause => resumed = false,
                    MainEvent::Destroy => destroyed = true,
                    _ => {}
                }
            }
        });
    }
    if destroyed {
        return;
    }
    log::info!("activity resumed with window; initializing EGL + OpenXR");

    let egl_ctx = init_egl();
    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            egl_ctx
                .instance
                .get_proc_address(s)
                .map(|p| p as *const _)
                .unwrap_or(std::ptr::null())
        })
    };
    let gl = Arc::new(gl);

    // android-activity stores the *Application* in ndk_context, but Meta's
    // runtime needs the *Activity* in XrInstanceCreateInfoAndroidKHR to track
    // the app's lifecycle — handed the Application, it logs "xrCreateSession:
    // Activity is not yet in the ready state" and parks the session in IDLE
    // forever. The openxr crate populates that struct from ndk_context; the
    // only ndk_context reads happen below on this same thread (loader +
    // instance init), so the non-atomic release→initialize swap is safe here.
    unsafe {
        ndk_context::release_android_context();
        ndk_context::initialize_android_context(app.vm_as_ptr(), app.activity_as_ptr());
    }

    // OpenXR: libopenxr_loader.so (Khronos or Meta's) is dlopen'd (the
    // crate's `loaded` feature); the Android loader hook must run before
    // create_instance.
    let entry = unsafe { xr::Entry::load() }
        .expect("load libopenxr_loader.so — is an OpenXR loader bundled in the APK? (see README)");
    entry
        .initialize_android_loader()
        .expect("initialize android loader");

    // Only ask for what the runtime advertises: an unsupported extension in the
    // set fails xrCreateInstance outright, which would take the whole app down
    // on a headset that happens not to offer it.
    let available = entry
        .enumerate_extensions()
        .expect("enumerate OpenXR extensions");

    let mut extensions = xr::ExtensionSet::default();
    extensions.khr_opengl_es_enable = true;
    // Without this the runtime picks the display rate and the app has no say —
    // Quest defaults to 72 Hz no matter how much headroom the frame has.
    extensions.fb_display_refresh_rate = available.fb_display_refresh_rate;
    // Fixed foveated rendering: shade the eye-buffer periphery at reduced
    // resolution. All three are needed together — the profile comes from
    // `fb_foveation`, and attaching it to a swapchain goes through
    // `fb_swapchain_update_state` (plus its GLES variant for a GLES swapchain).
    let foveation_available = available.fb_foveation
        && available.fb_foveation_configuration
        && available.fb_swapchain_update_state
        && available.fb_swapchain_update_state_opengl_es;
    extensions.fb_foveation = foveation_available;
    extensions.fb_foveation_configuration = foveation_available;
    extensions.fb_swapchain_update_state = foveation_available;
    extensions.fb_swapchain_update_state_opengl_es = foveation_available;
    // Required on Android: openxr only chains the activity/JVM context into
    // xrCreateInstance (XrInstanceCreateInfoAndroidKHR) when this is set —
    // without it the runtime can reject instance creation.
    extensions.khr_android_create_instance = true;
    let instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: "functor-runner",
                application_version: 0,
                engine_name: "functor",
                engine_version: 0,
                api_version: xr::Version::new(1, 0, 34),
            },
            &extensions,
            &[],
        )
        .expect("create OpenXR instance");
    let system = instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .expect("no HMD system");

    // The spec requires this call before create_session — and requires the
    // app to verify its GLES version is inside the supported range; a clear
    // panic here beats on-device UB.
    let reqs = instance
        .graphics_requirements::<xr::OpenGlEs>(system)
        .expect("graphics requirements");
    let requested = xr::Version::new(GLES_MAJOR as u16, GLES_MINOR as u16, 0);
    log::info!(
        "GLES {GLES_MAJOR}.{GLES_MINOR} requested; runtime supports {} – {}",
        reqs.min_api_version_supported,
        reqs.max_api_version_supported
    );
    assert!(
        requested >= reqs.min_api_version_supported && requested <= reqs.max_api_version_supported,
        "GLES {GLES_MAJOR}.{GLES_MINOR} outside the runtime's supported range"
    );

    let (session, mut frame_wait, mut frame_stream) = unsafe {
        instance.create_session::<xr::OpenGlEs>(
            system,
            &xr::opengles::SessionCreateInfo::Android {
                display: egl_ctx.display.as_ptr(),
                config: egl_ctx.config.as_ptr(),
                context: egl_ctx.context.as_ptr(),
            },
        )
    }
    .expect("create session");

    // Display refresh rate. Requesting one is a *request*: the runtime may
    // refuse, and it can change underneath us (battery, thermals, system UI),
    // so report what was actually granted rather than what was asked for.
    if available.fb_display_refresh_rate {
        match session.enumerate_display_refresh_rates() {
            Ok(rates) => {
                let current = session.get_display_refresh_rate().ok();
                log::info!("display refresh rates available: {rates:?} (current {current:?})");
                // The highest the display offers that is no faster than the
                // target. Asking for an unlisted rate is an error, and asking
                // for more than the frame can sustain just converts headroom
                // into stale frames.
                let choice = rates
                    .iter()
                    .copied()
                    .filter(|rate| *rate <= TARGET_REFRESH_RATE + 0.5)
                    .fold(f32::NAN, f32::max);
                if choice.is_finite() {
                    // The request is asynchronous: reading the rate back here
                    // still returns the old one. The runtime confirms the
                    // change with DisplayRefreshRateChangedFB, which the event
                    // loop logs — so do not report a rate we have not been
                    // told took effect.
                    match session.request_display_refresh_rate(choice) {
                        Ok(()) => log::info!("requested {choice} Hz"),
                        Err(e) => log::warn!("display refresh rate {choice} Hz refused: {e}"),
                    }
                }
            }
            Err(e) => log::warn!("cannot enumerate display refresh rates: {e}"),
        }
    } else {
        log::info!("XR_FB_display_refresh_rate unavailable; using the runtime default");
    }

    // STAGE (floor-origin, room-scale) preferred; LOCAL (head-origin) is the
    // portable baseline when the runtime has no stage bounds set up.
    let (stage, stage_type) =
        match session.create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY) {
            Ok(stage) => (stage, xr::ReferenceSpaceType::STAGE),
            Err(e) => {
                log::warn!("STAGE reference space unavailable ({e}); falling back to LOCAL");
                (
                    session
                        .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
                        .expect("local space"),
                    xr::ReferenceSpaceType::LOCAL,
                )
            }
        };
    let xr_actions = XrActions::create(&instance, &session);

    let view_config_views = instance
        .enumerate_view_configuration_views(system, xr::ViewConfigurationType::PRIMARY_STEREO)
        .expect("view config views");
    // Fail with a clear message rather than XR_ERROR_SWAPCHAIN_FORMAT_UNSUPPORTED.
    let formats = session.enumerate_swapchain_formats().expect("formats");
    assert!(
        formats.contains(&COLOR_FORMAT),
        "runtime does not offer SRGB8_ALPHA8 swapchains (offered: {formats:x?})"
    );
    let mut eyes: Vec<EyeTarget> = view_config_views
        .iter()
        .map(|v| create_eye_target(&gl, &session, v))
        .collect();
    // Foveation attaches per swapchain, so it has to happen after the eye
    // targets exist. A dynamic profile lets the runtime scale the level with
    // GPU load rather than pinning the worst case all the time.
    if foveation_available {
        match session.create_foveation_profile(Some(xr::FoveationLevelProfile {
            level: FOVEATION_LEVEL,
            vertical_offset: 0.0,
            dynamic: xr::sys::FoveationDynamicFB::LEVEL_ENABLED,
        })) {
            Ok(profile) => {
                for (eye, target) in ["left", "right"].iter().zip(eyes.iter()) {
                    attach_foveation(&instance, &target.swapchain, &profile, eye);
                }
                log::info!("fixed foveated rendering enabled ({FOVEATION_LEVEL:?}, dynamic)");
            }
            Err(e) => log::warn!("cannot create a foveation profile: {e}"),
        }
    } else {
        log::info!("fixed foveated rendering unavailable on this runtime");
    }

    log::info!(
        "swapchains ready: {}x{} per eye",
        eyes[0].width,
        eyes[0].height
    );

    let asset_cache = Arc::new(AssetCache::new());
    let scene_context = SceneContext::new();
    let shadow_map = functor_runtime_common::shadow::ShadowMap::new(&gl, 2048);

    // The real Functor Lang producer, booting the embedded scene. A broken embedded
    // scene is a build bug, not a runtime condition — fail loud.
    let mut game = FunctorLangEmbeddedGame::create(
        vec![("boot.fun".to_string(), BOOT_SCENE.to_string())],
        Box::new(NativePlatform),
    )
    .expect("embedded boot scene loads");

    // The isomorphic debug endpoint lives on device loopback. The dev PC and
    // browser reach it through `adb forward tcp:8123 tcp:8123` (see README).
    // A bind failure degrades to a standalone boot scene, loudly.
    let debug_rx = match functor_runtime_common::debug_http::spawn(("127.0.0.1", RELOAD_PORT)) {
        Ok((_bound, rx)) => {
            log::info!("debug endpoint: http://127.0.0.1:{RELOAD_PORT}");
            Some(rx)
        }
        Err(error) => {
            log::error!("debug endpoint failed to bind port {RELOAD_PORT}: {error}");
            None
        }
    };

    let mut clock = GameClock::new(None);
    let mut last_frame_at: Option<Instant> = None;
    let mut debug = DebugLoopState::default();
    let mut session_running = false;
    let mut session_focused = false;
    // The first valid center-eye pose becomes the tracking origin for the
    // authored `Frame.camera`. It survives source reload and session doze so
    // model-driven locomotion and physical room-scale motion remain additive.
    let mut tracking_reference: Option<TrackingPose> = None;
    let mut tracking_reference_reset_at: Option<xr::Time> = None;
    let mut terrain_frame_id = 0_u64;
    let mut quit = false;
    let mut event_storage = xr::EventDataBuffer::new();

    while !quit {
        // Android lifecycle first (non-blocking).
        app.poll_events(Some(std::time::Duration::ZERO), |event| {
            if let PollEvent::Main(main) = event {
                match main {
                    MainEvent::Destroy => quit = true,
                    // Session state (FOCUSED/VISIBLE/…) drives rendering; the
                    // window is compositor-owned, so Pause/Resume need no GL
                    // work here.
                    _ => {}
                }
            }
        });

        // Drain Android input events: real input arrives via OpenXR actions,
        // but an undrained queue is an "isn't responding" ANR — the
        // dispatcher times out waiting on the unread events. Key events are
        // consumed (Handled): default-handling an unhandled BACK would
        // finish the activity.
        if let Ok(mut input_iter) = app.input_events_iter() {
            while input_iter.next(|event| match event {
                android_activity::input::InputEvent::KeyEvent(_) => InputStatus::Handled,
                _ => InputStatus::Unhandled,
            }) {}
        }

        // OpenXR events: drive the session state machine.
        while let Some(event) = instance.poll_event(&mut event_storage).expect("poll_event") {
            use xr::Event::*;
            match event {
                DisplayRefreshRateChangedFB(e) => {
                    log::info!(
                        "display refresh rate changed: {} Hz -> {} Hz",
                        e.from_display_refresh_rate(),
                        e.to_display_refresh_rate()
                    );
                }
                SessionStateChanged(e) => {
                    log::info!("session state: {:?}", e.state());
                    session_focused = e.state() == xr::SessionState::FOCUSED;
                    match e.state() {
                        xr::SessionState::READY => {
                            session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                                .expect("session begin");
                            session_running = true;
                            // The system-menu/headset-off gap is not game time.
                            last_frame_at = None;
                        }
                        xr::SessionState::STOPPING => {
                            if let Some(response) = debug.pending_capture.take() {
                                let _ = response.send(Err(CaptureError::Unavailable(
                                    "capture cancelled because the XR session stopped".to_string(),
                                )));
                            }
                            session.end().expect("session end");
                            session_running = false;
                            session_focused = false;
                            last_frame_at = None;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            quit = true;
                        }
                        _ => {}
                    }
                }
                InstanceLossPending(_) => quit = true,
                ReferenceSpaceChangePending(e) if e.reference_space_type() == stage_type => {
                    tracking_reference_reset_at = Some(e.change_time());
                    log::info!("tracking-space change pending; camera rig will recenter");
                }
                _ => {}
            }
        }

        // Service debug requests before the XR session gate. Source reload,
        // state, trace, input, and time control remain useful while the headset
        // dozes; capture reports an honest 503 until rendering resumes.
        if !session_running || !session_focused {
            debug.input.xr = None;
        }
        if let Some(rx) = &debug_rx {
            while let Ok(request) = rx.try_recv() {
                service_debug_request(
                    request,
                    &mut game,
                    &mut clock,
                    &mut debug,
                    &eyes,
                    session_running,
                    &asset_cache,
                    &scene_context,
                );
            }
        }

        if !session_running {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let xr_frame_state = frame_wait.wait().expect("frame wait");
        frame_stream.begin().expect("frame begin");
        if session_focused {
            session
                .sync_actions(&[(&xr_actions.action_set).into()])
                .expect("sync gameplay actions");
        }

        if tracking_reference_reset_at.is_some_and(|change_time| {
            xr_frame_state.predicted_display_time.as_nanos() >= change_time.as_nanos()
        }) {
            tracking_reference = None;
            tracking_reference_reset_at = None;
        }

        let (view_state, views) = session
            .locate_views(
                xr::ViewConfigurationType::PRIMARY_STEREO,
                xr_frame_state.predicted_display_time,
                &stage,
            )
            .expect("locate views");
        let tracking_valid = view_state.contains(xr::ViewStateFlags::POSITION_VALID)
            && view_state.contains(xr::ViewStateFlags::ORIENTATION_VALID);
        if !tracking_valid {
            debug.input.xr = None;
            // OpenXR leaves the view poses undefined when either validity bit
            // is absent. Do not read those poses or submit them for compositor
            // reprojection; a mono authored-camera fallback would be actively
            // misleading here.
            if let Some(response) = debug.pending_capture.take() {
                let _ = response.send(Err(CaptureError::Unavailable(
                    "capture is unavailable because XR tracking is invalid".to_string(),
                )));
            }
            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                )
                .expect("frame end (invalid tracking)");
            last_frame_at = None;
            continue;
        }
        let center_pose = views.first().and_then(|left| {
            let left = tracking_pose_from_view(left);
            views
                .get(1)
                .and_then(|right| TrackingPose::midpoint(left, tracking_pose_from_view(right)))
                .or_else(|| TrackingPose::midpoint(left, left))
        });
        if tracking_reference.is_none() {
            tracking_reference = center_pose;
            if tracking_reference.is_some() {
                log::info!("camera rig centered: Frame.camera is the reference center-eye pose");
            }
        }
        debug.input.xr = session_focused
            .then(|| {
                tracking_reference
                    .zip(center_pose)
                    .map(|(reference, head)| {
                        xr_actions.sample(
                            &session,
                            &stage,
                            xr_frame_state.predicted_display_time,
                            reference,
                            head,
                        )
                    })
            })
            .flatten();

        if !xr_frame_state.should_render {
            // `should_render` controls layer submission, not tracking
            // validity. Keep the freshly sampled input visible to `/state`
            // while honestly declining a framebuffer capture.
            if let Some(response) = debug.pending_capture.take() {
                let _ = response.send(Err(CaptureError::Unavailable(
                    "capture is unavailable because the XR compositor declined rendering"
                        .to_string(),
                )));
            }
            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                )
                .expect("frame end (no render)");
            continue;
        }

        let now = Instant::now();
        let real_delta = last_frame_at
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        let sub_frames = clock.fixed_frames(real_delta);
        let frame_time = FrameTime {
            dts: 0.0,
            tts: clock.current_tts(),
        };

        // The game produces this frame (sampledInput → subscriptions → update
        // → tick → physics inside `tick`; `render` = the pure `draw`).
        // Frame.camera is the authored center-eye rig; live OpenXR eye deltas
        // compose onto it below, so the same camera positions the world on
        // every target.
        game.check_hot_reload(frame_time.clone());
        game.push_asset_progress(asset_cache.progress());
        let preload_commands =
            serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default();
        for token in scene_context.drive_preloads(&asset_cache, preload_commands) {
            game.preload_push_settled(token);
        }
        for sub_frame in &sub_frames {
            if game.samples_input() {
                debug.input_edges.apply_to(&mut debug.input);
                game.sampled_input(&debug.input);
            }
            debug.input_edges.clear();
            debug.input.clear_edges();
            game.tick(sub_frame.clone());
            debug.frame_count += 1;
        }
        let frame = game.render(frame_time.clone());
        // No audio/HTTP hosts on device yet: drain their command queues so
        // they don't grow unbounded. Preloads and physics terrain requests
        // are driven above through the shared asset cache.
        let _ = game.audio_drain_commands();
        let _ = game.net_drain_commands();
        let _ = game.net_drain_conn_commands();

        let capture_due = debug.pending_capture.is_some();
        let mut captured_eyes = capture_due.then(Vec::new);
        let mut capture_error = None;

        // Terrain LOD follows the tracked center pose for distance, while
        // culling uses the exact union of both displaced/canted eye frusta.
        // Both eye draws receive the same fixed-size data and pixel scale, so
        // selection is conservative and byte-identical across the pair.
        let tracked_center = views.first().and_then(|left| {
            let left = tracking_pose_from_view(left);
            views
                .get(1)
                .and_then(|right| TrackingPose::midpoint(left, tracking_pose_from_view(right)))
                .or_else(|| TrackingPose::midpoint(left, left))
        });
        let lod_camera = match (tracking_reference, tracked_center) {
            (Some(reference), Some(center)) => frame.camera.compose_tracked_view(reference, center),
            _ => frame.camera.clone(),
        };
        let fallback_lod_projection = lod_camera.projection_matrix(1.0);
        let mut lod_view_projections = [fallback_lod_projection * lod_camera.view_matrix(); 2];
        let mut lod_frustum_count = 0;
        let mut lod_projection_scale = 0.0_f32;
        let mut lod_viewport_height = 1.0_f32;
        for (slot, (eye, view)) in eyes.iter().zip(views.iter()).take(2).enumerate() {
            let eye_camera = tracking_reference
                .map(|reference| {
                    frame
                        .camera
                        .compose_tracked_view(reference, tracking_pose_from_view(view))
                })
                .unwrap_or_else(|| frame.camera.clone());
            let eye_projection = eye_camera.projection_matrix_from_fov_angles(
                view.fov.angle_left,
                view.fov.angle_right,
                view.fov.angle_down,
                view.fov.angle_up,
            );
            lod_view_projections[slot] = eye_projection * eye_camera.view_matrix();
            lod_frustum_count += 1;
            lod_projection_scale = lod_projection_scale.max(eye_projection.y.y.abs());
            lod_viewport_height = lod_viewport_height.max(eye.height as f32);
        }
        if lod_frustum_count == 0 {
            lod_frustum_count = 1;
            lod_projection_scale = fallback_lod_projection.y.y.abs();
        }
        terrain_frame_id = terrain_frame_id.wrapping_add(1);

        for (eye, view) in eyes.iter_mut().zip(views.iter()) {
            use glow::HasContext;
            let image_index = eye.swapchain.acquire_image().expect("acquire") as usize;
            eye.swapchain
                .wait_image(xr::Duration::INFINITE)
                .expect("wait image");
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(eye.framebuffers[image_index]));
                gl.disable(glow::SCISSOR_TEST);
                gl.enable(glow::DEPTH_TEST);
            }
            let camera = tracking_reference
                .map(|reference| {
                    frame
                        .camera
                        .compose_tracked_view(reference, tracking_pose_from_view(view))
                })
                .unwrap_or_else(|| frame.camera.clone());
            let projection = camera.projection_matrix_from_fov_angles(
                view.fov.angle_left,
                view.fov.angle_right,
                view.fov.angle_down,
                view.fov.angle_up,
            );
            functor_runtime_common::render_frame_with_projection(
                &gl,
                "#version 300 es",
                asset_cache.clone(),
                &scene_context,
                &shadow_map,
                &frame,
                &camera,
                &projection,
                &lod_camera,
                &lod_view_projections[..lod_frustum_count],
                lod_projection_scale,
                lod_viewport_height,
                terrain_frame_id,
                None,
                frame_time.clone(),
                Viewport::new(eye.width, eye.height),
                functor_runtime_common::DebugRenderMode::Default,
            );
            if let Some(captured) = &mut captured_eyes {
                // A render-target pass is allowed to change the ambient FBO;
                // bind the swapchain image explicitly at the readback seam.
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(eye.framebuffers[image_index]));
                    match functor_runtime_common::frame_capture::read_bound_framebuffer_rgba(
                        &gl, eye.width, eye.height,
                    ) {
                        Ok(pixels) => captured.push(pixels),
                        Err(error) => capture_error = Some(error),
                    }
                }
            }
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
            eye.swapchain.release_image().expect("release image");
        }

        let projection_views: Vec<_> = views
            .iter()
            .zip(eyes.iter())
            .map(|(view, eye)| {
                let rect = xr::Rect2Di {
                    offset: xr::Offset2Di { x: 0, y: 0 },
                    extent: xr::Extent2Di {
                        width: eye.width as i32,
                        height: eye.height as i32,
                    },
                };
                xr::CompositionLayerProjectionView::new()
                    .pose(view.pose)
                    .fov(view.fov)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&eye.swapchain)
                            .image_rect(rect),
                    )
            })
            .collect();
        frame_stream
            .end(
                xr_frame_state.predicted_display_time,
                xr::EnvironmentBlendMode::OPAQUE,
                &[&xr::CompositionLayerProjection::new()
                    .space(&stage)
                    .views(&projection_views)],
            )
            .expect("frame end");

        // The current frame is no longer borrowed by either eye. Move it into
        // the inspector cache instead of cloning its scene every rendered tick.
        debug.last_frame = Some(frame);
        if let Some(response) = debug.pending_capture.take() {
            let result = if let Some(error) = capture_error {
                Err(CaptureError::Failed(error))
            } else if eyes.len() != 2 || captured_eyes.as_ref().map(Vec::len) != Some(2) {
                Err(CaptureError::Failed(format!(
                    "stereo capture requires two rendered views (got {})",
                    captured_eyes.as_ref().map_or(0, Vec::len)
                )))
            } else if eyes[0].width != eyes[1].width || eyes[0].height != eyes[1].height {
                Err(CaptureError::Failed(format!(
                    "stereo capture requires equal eye sizes (left {}x{}, right {}x{})",
                    eyes[0].width, eyes[0].height, eyes[1].width, eyes[1].height
                )))
            } else {
                let captured = captured_eyes.expect("two captured views checked above");
                functor_runtime_common::frame_capture::encode_stereo_side_by_side_png(
                    eyes[0].width,
                    eyes[0].height,
                    &captured[0],
                    &captured[1],
                )
                .map_err(CaptureError::Failed)
            };
            let _ = response.send(result);
        }
    }

    if let Some(response) = debug.pending_capture.take() {
        let _ = response.send(Err(CaptureError::Unavailable(
            "capture cancelled because the XR runtime is exiting".to_string(),
        )));
    }
    log::info!("functor oculus runtime exiting");
}
