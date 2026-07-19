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

use std::sync::Arc;

use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use cgmath::{Quaternion, Rotation, Vector3};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::functor_lang_game_embedded::FunctorLangEmbeddedGame;
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::{Camera, FrameTime, SceneContext, Viewport};
use khronos_egl as egl;
use openxr as xr;

/// GLES version to request. Quest 3 supports 3.2.
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
        let count = instance.get_config_count(display).expect("EGL config count");
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

/// One eye's rendering target: an OpenXR swapchain plus an FBO + depth
/// renderbuffer per swapchain image.
struct EyeTarget {
    swapchain: xr::Swapchain<xr::OpenGlEs>,
    framebuffers: Vec<glow::Framebuffer>,
    width: u32,
    height: u32,
}

fn create_eye_target(
    gl: &glow::Context,
    session: &xr::Session<xr::OpenGlEs>,
    view: &xr::ViewConfigurationView,
) -> EyeTarget {
    use glow::HasContext;
    let width = view.recommended_image_rect_width;
    let height = view.recommended_image_rect_height;
    let swapchain = session
        .create_swapchain(&xr::SwapchainCreateInfo {
            create_flags: xr::SwapchainCreateFlags::EMPTY,
            usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::SAMPLED,
            format: COLOR_FORMAT,
            sample_count: 1,
            width,
            height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        })
        .expect("create swapchain");

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
                Some(glow::NativeTexture(std::num::NonZeroU32::new(texture).unwrap())),
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
    }
}

/// Derive a Functor `Camera` from an OpenXR eye view. The vertical field of
/// view is symmetrized (`angle_up - angle_down`) — Quest eye frusta are
/// mildly asymmetric, so this is approximate. TODO(device bring-up): teach
/// the shared renderer to take a raw projection matrix so the exact
/// asymmetric frustum (and correct stereo overlap) is used.
fn camera_from_view(view: &xr::View) -> Camera {
    let p = view.pose.position;
    let o = view.pose.orientation;
    let q = Quaternion::new(o.w, o.x, o.y, o.z);
    let eye = Vector3::new(p.x, p.y, p.z);
    let forward = q.rotate_vector(-Vector3::unit_z());
    let up = q.rotate_vector(Vector3::unit_y());
    let target = eye + forward;
    Camera {
        eye: [eye.x, eye.y, eye.z],
        target: [target.x, target.y, target.z],
        up: [up.x, up.y, up.z],
        fov_radians: view.fov.angle_up - view.fov.angle_down,
        near: 0.1,
        far: 100.0,
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
    let entry = unsafe { xr::Entry::load() }.expect(
        "load libopenxr_loader.so — is an OpenXR loader bundled in the APK? (see README)",
    );
    entry
        .initialize_android_loader()
        .expect("initialize android loader");

    let mut extensions = xr::ExtensionSet::default();
    extensions.khr_opengl_es_enable = true;
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
        requested >= reqs.min_api_version_supported
            && requested <= reqs.max_api_version_supported,
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

    // STAGE (floor-origin, room-scale) preferred; LOCAL (head-origin) is the
    // portable baseline when the runtime has no stage bounds set up.
    let stage = session
        .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
        .unwrap_or_else(|e| {
            log::warn!("STAGE reference space unavailable ({e}); falling back to LOCAL");
            session
                .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
                .expect("local space")
        });

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
    let mut game = FunctorLangEmbeddedGame::create(vec![(
        "boot.fun".to_string(),
        BOOT_SCENE.to_string(),
    )])
    .expect("embedded boot scene loads");

    let start = std::time::Instant::now();
    // Lazy: seconds pass between android_main and the first rendered frame
    // (session READY), and the session can pause — the first frame after
    // either must not hand the game a giant dts.
    let mut last_tts: Option<f32> = None;
    // Freeze the game clock across session pauses (headset off, system
    // menu): paused wall-clock must not enter `tts`, or the first resumed
    // frame hands the game the whole gap as `dts` and fires subscriptions
    // over it in one burst.
    let mut paused_offset = std::time::Duration::ZERO;
    let mut paused_at: Option<std::time::Instant> = None;
    let mut session_running = false;
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
                SessionStateChanged(e) => {
                    log::info!("session state: {:?}", e.state());
                    match e.state() {
                        xr::SessionState::READY => {
                            session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                                .expect("session begin");
                            session_running = true;
                            if let Some(paused) = paused_at.take() {
                                paused_offset += paused.elapsed();
                            }
                        }
                        xr::SessionState::STOPPING => {
                            session.end().expect("session end");
                            session_running = false;
                            paused_at = Some(std::time::Instant::now());
                            // Belt-and-braces with the frozen clock: the
                            // first resumed frame gets dts = 0.
                            last_tts = None;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            quit = true;
                        }
                        _ => {}
                    }
                }
                InstanceLossPending(_) => quit = true,
                _ => {}
            }
        }

        if !session_running {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let xr_frame_state = frame_wait.wait().expect("frame wait");
        frame_stream.begin().expect("frame begin");

        if !xr_frame_state.should_render {
            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                )
                .expect("frame end (no render)");
            continue;
        }

        let (_, views) = session
            .locate_views(
                xr::ViewConfigurationType::PRIMARY_STEREO,
                xr_frame_state.predicted_display_time,
                &stage,
            )
            .expect("locate views");

        // Frame time from the pause-frozen wall clock; fixed-timestep
        // sub-frames (the desktop GameClock pattern) are a follow-up.
        let tts = start.elapsed().saturating_sub(paused_offset).as_secs_f32();
        let frame_time = FrameTime {
            dts: last_tts.map_or(0.0, |last| tts - last),
            tts,
        };
        last_tts = Some(tts);

        // The game produces this frame (subscriptions → update → tick →
        // physics inside `tick`; `render` = the pure `draw`). The game's own
        // camera is ignored below — per-eye HMD-pose cameras own the view.
        game.check_hot_reload(frame_time.clone());
        game.tick(frame_time.clone());
        let frame = game.render(frame_time.clone());
        // No audio/HTTP/preload/asset hosts on device yet (M1): drain the
        // command queues so they don't grow unbounded (and
        // `push_asset_progress` is deliberately never fed — `Sub.assets`
        // stays quiet). Games using those effects run; the effects just
        // don't produce sound/replies here yet.
        let _ = game.audio_drain_commands();
        let _ = game.net_drain_commands();
        let _ = game.net_drain_conn_commands();
        let _ = game.preload_drain_commands();

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
            functor_runtime_common::render_frame(
                &gl,
                "#version 300 es",
                asset_cache.clone(),
                &scene_context,
                &shadow_map,
                &frame,
                &camera_from_view(view),
                frame_time.clone(),
                Viewport::new(eye.width, eye.height),
                functor_runtime_common::DebugRenderMode::Default,
            );
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
    }

    log::info!("functor oculus runtime exiting");
}
