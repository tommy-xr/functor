# VR / XR track — status

Functor on headsets, built as a train of small PRs (2026-07-03/04). Two devices,
one architecture: the **Xreal One** as a desktop 3DoF stereo dev display, and
the **Quest 3** as the real standalone target. The through-line is Functor's
existing principles doing the heavy lifting: `draw` is a pure `Frame`, so
stereo is "render it twice with shell-supplied cameras"; Functor Lang games are text
interpreted at runtime, so the headset runtime is a **tool APK built once** and
games deploy over the network.

## Milestones

| PR | What | Status |
| --- | --- | --- |
| [#175](https://github.com/tommy-xr/functor/pull/175) | `--stereo-sbs`: side-by-side stereo in functor-runner (`Camera::stereo_eyes`, `render_frame` takes the camera explicitly) | merged |
| [#177](https://github.com/tommy-xr/functor/pull/177) | `--xreal-tracking`: Xreal One 3DoF head tracking — IMU over TCP (the glasses expose a USB network interface; no drivers), gyro-bias calibration + Mahony fusion, head rotation composed onto the game camera in its local basis | merged |
| [#185](https://github.com/tommy-xr/functor/pull/185) | Axis remap corrected from a live wear-test (raw sensor frame is right/down/forward — the optical convention; the first guess was an improper det −1 transform, which mirrors gyro vs accel) + auto/yaw-only recenter | merged |
| [#183](https://github.com/tommy-xr/functor/pull/183) | Network hot-reload: `POST /reload-source`, `--debug-bind`, and `functor push <addr> [--watch]` | merged |
| [#189](https://github.com/tommy-xr/functor/pull/189) | Quest OpenXR shell: EGL/GLES 3.2, per-eye sRGB swapchains, head-pose cameras, shared renderer | merged |
| [#428](https://github.com/tommy-xr/functor/pull/428) | Embedded Functor Lang producer: the tool APK interprets pushed games directly | merged |
| [#430](https://github.com/tommy-xr/functor/pull/430) | Device-loopback source reload over adb forwarding | merged |
| [#431](https://github.com/tommy-xr/functor/pull/431) | `functor run vr`: launch, forward, whole-project push, watch, and log streaming | merged |
| [#437](https://github.com/tommy-xr/functor/pull/437) | Exact asymmetric OpenXR projections, fixing binocular double vision | merged; Quest 3 verified |
| [#438](https://github.com/tommy-xr/functor/pull/438) | Shared desktop/Quest debug protocol, whole-project REPL, raw stereo capture, TypeScript SDK parity, device benchmark | merged; Quest 3 verified |
| [#453](https://github.com/tommy-xr/functor/pull/453) | Compose live OpenXR head/eye tracking onto the authored `Frame.camera` rig | merged; Quest 3 verified |
| [#460](https://github.com/tommy-xr/functor/pull/460) | Push project `.glb` models, textures, and sounds through the shared debug protocol; initialize the first pushed project from `init` | merged; Quest 3 verified with synthwave textures + animated Xbot |
| [#463](https://github.com/tommy-xr/functor/pull/463) | Sample Quest Touch grip/aim poses, analog controls, and buttons into the shared typed input snapshot | ready; Quest 3 + debug/SDK verified |

## Working today on the Xreal One

```sh
functor -d examples/hello-cubes run native --stereo-sbs --xreal-tracking
```

Glasses in 3D mode (OSD → Spatial Screen → 3D Mode → Full SBS), window
fullscreened on the glasses display: head-tracked stereo, auto-recentered,
F1 to recenter. Disable the on-glasses stabilizer/anchor — the X1 chip's own
correction fights engine-side tracking. 3DoF only (yaw drifts slowly without a
magnetometer; the Xreal Eye's 6DoF is not host-accessible).

## Key decisions & findings

- **Xreal One IMU**: streams ~1kHz gyro+accel over TCP at `169.254.2.1:52998`
  (USB CDC-NCM interface, macOS-native, no handshake). Protocol + verified
  axis conventions live in `runtime/functor-runtime-desktop/src/xreal.rs`.
- **shock2quest's vendored openxrs is obsolete**: it back-ported unreleased
  GLES/Android support during a crates.io release gap; everything shipped in
  `openxr` 0.18 (Feb 2024). We use crates.io 0.21 + `android-activity`
  (replaces deprecated `ndk-glue`) + `cargo-ndk`/`cargo-apk`.
- Quest gotchas inherited from shock2quest (comments in the oculus `lib.rs`):
  manual EGL config selection, sRGB swapchains without `FRAMEBUFFER_SRGB`,
  pbuffer-backed context, two swapchains (multiview later).
- Android builds use an isolated `CARGO_TARGET_DIR=target-android` (fingerprint
  races on the shared `target/`) and the oculus crate is its **own** cargo
  workspace (`ndk-sys` hard-fails host compiles).
- The shared shaders are gamma-naive; the Quest's sRGB swapchain will render
  brighter than desktop until the pipeline is gamma-explicit (TODO'd at
  `COLOR_FORMAT`).
- **Quest camera composition:** the first valid center-eye tracking pose maps
  to the authored `Frame.camera`. Later eye translation/rotation composes in
  that camera's local basis; authored camera changes move the rig, while
  OpenXR owns IPD/optical FOV and the camera keeps its near/far range. Verified
  on Quest 3 with raw stereo capture and a live authored-camera translation.

## Device-day checklist (Quest 3)

Use the **`vr-device-loop` skill** (`.claude/skills/vr-device-loop/`) for the
full, proven sequence. The short path is:

1. Put the standard Khronos Android OpenXR loader in
   `runtime/functor-runtime-oculus/lib/arm64-v8a/` (the crate README has the
   Maven command), then `npm run build:oculus:apk` and `adb install -r …`.
2. Run `functor -d yourgame run vr`. It uses device-loopback port 8123 through
   adb, loads every `.fun`/`.funi` plus project models/textures/sounds, and
   re-pushes changed source or assets on save.
3. If Meta Home remains visible, recreate `adb forward`, send `prox_close`, and
   restart NativeActivity with `am start -S`. Wait for `/state`'s frame to
   advance; server readiness alone does not mean XR is rendering.
4. Capture with `POST /capture` using curl's `--fail` and retry flags. A focused
   Quest 3 returns a raw 3360x1760 left/right PNG.
5. Install a non-debuggable release APK, push the explicit workload, and run
   `npm run bench:quest -- --label NAME --warmup 10 --seconds 30`.

### Display refresh rate

The APK starts at the highest rate the display offers up to 80 Hz. To ask for a
different one, use `functor -d yourgame run vr --refresh-rate 90` (or `120`).
This requests the rate once, after the initial push, through
`POST /display-refresh-rate`. The rate must be one the display offers; Quest 3
offers 120 Hz only when it is enabled in the headset's display settings. The
switch is asynchronous, so confirm it with `/state`'s
`display_refresh_rate.current` (logged as `display refresh rate changed`).
`bench:quest` prints the requested and current rate next to VrApi's FPS target,
and fails if the rate changes during the sample.

Quest 3 release APK, 30-second samples after a 10-second warmup (2026-10-23).
"Default" is a fresh start with no request. Times are VrApi means.

| Scene | Rate | FPS mean / p5 / min | Stale / torn | App | Compositor | CPU+GPU | CPU / GPU load | CPU / GPU level |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| terrain | default (80) | 80.6 / 80 / 80 | 0 / 3 | 9.54 ms | 0.76 ms | 12.00 ms | 0.06 / 0.86 | 2 / 3 |
| terrain | 90 | 90.2 / 90 / 90 | 0 / 0 | 8.41 ms | 0.69 ms | 10.69 ms | 0.06 / 0.85 | 2 / 4 |
| terrain | 120 | 120.3 / 120 / 119 | 5 / 18 | 6.93 ms | 0.65 ms | 9.24 ms | 0.08 / 0.95 | 2 / 4 |
| synthwave | default (80) | 80.7 / 80 / 80 | 0 / 1 | 0.80 ms | 1.03 ms | 8.84 ms | 0.25 / 0.17 | 2 / 2 |
| synthwave | 90 | 90.5 / 90 / 90 | 0 / 0 | 0.79 ms | 1.04 ms | 7.97 ms | 0.26 / 0.19 | 2 / 2 |
| synthwave | 120 | 120.3 / 120 / 120 | 7 / 0 | 0.90 ms | 1.00 ms | 7.06 ms | 0.28 / 0.26 | 2 / 2 |

Terrain is GPU-bound. At 120 Hz its GPU load reaches 0.95, and it is the only
run with a minimum below target (119 FPS) and many torn frames (18).

The first measured Quest 3 release run (`primitives`, 15 seconds after a
5-second warmup) sustained 72.7 FPS mean / 72 FPS minimum, with zero stale or
torn frames and 3.09 ms mean application time.

The authored-camera release verification sustained 72 FPS minimum in two
30-second `primitives` runs (3.31–3.36 ms mean application time). A
source/geometry-heavy `synthwave` run also held 72 FPS minimum with zero stale
or torn frames. Project texture/model synchronization is verified on Quest 3
with the textured synthwave scene and the animated `Xbot.glb` example. Matched
30-second release runs held 72 FPS minimum: synthwave used 1.66 ms mean
application time and animation used 3.95 ms. The few VrApi `Stale` samples
occurred despite ample application/GPU headroom and zero torn frames,
consistent with occasional compositor reuse rather than missed application
deadlines.

With Touch action sampling enabled, a matched synthwave release rerun held 72
FPS p5/min with 0.78 ms mean application time and zero stale/torn frames.

### Pipelined sim thread and XR thread hints

The APK can run the game logic on its own sim thread, like desktop
`--pipelined`. There is no command line on the headset, so it is a launch-time
opt-in read once at startup:

```sh
adb shell setprop debug.functor.pipelined 1        # 0 (default) = inline
adb shell setprop debug.functor.thread_settings 1  # 1/0 force the XR thread hints on/off
adb shell am force-stop dev.functor.runner         # then launch / `functor run vr`
```

Pipelined, and with `XR_KHR_android_thread_settings` available (Quest 3 has
it), the shell tags the sim thread `APPLICATION_MAIN` and the GL thread
`RENDERER_MAIN` so the runtime can schedule both for XR; inline runs stay
untagged unless the property forces it. `/state.sim_thread` reports the result
(e.g. `pipelined: APPLICATION_MAIN sim + RENDERER_MAIN render`), and logcat
prints `stages over 300 frames: sim … ms, render … ms, sim-wait … ms` every 300
rendered frames.

Pipelined, the frame on screen was drawn one frame earlier than the head pose
it is shown with: the head is late-latched, but anything the game places from
controller input trails the controller by one frame (about 11 ms at 90 Hz).
`/state` answers from the sim's newest frame while `/scene` and `/capture`
show the frame on screen; the shell steps in lockstep after any debug request,
so they agree from the next frame on.

Quest 3 release APK, `terrain`, 30-second sample after a 10-second warmup
(2026-10-23):

| Mode | Rate | FPS mean / p5 / min | Stale / torn | App | CPU+GPU | CPU / GPU load | sim / render / wait |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| inline, `RENDERER_MAIN` | 90 | 90.6 / 90 / 90 | 3 / 6 | 8.25 ms | 14.77 ms | 0.11 / 0.95 | 0.33 / 0.75 / 0.00 ms |

Terrain is GPU-bound on Quest (GPU load 0.95; sim and render CPU are each under
1 ms), so pipelining is not expected to move its FPS. The full matrix (90 and
120 Hz × inline/pipelined × untagged/tagged) reproduces with a release APK
installed:

```sh
node scripts/quest-pipeline-matrix.mjs --functor target/release/functor \
  -d examples/terrain --rates 90,120 --warmup 10 --seconds 30
```

Desktop per-stage numbers come from `node scripts/pipeline-stages.mjs`.

#### `terrain` matrix, 2026-10-24

Quest 3 release APK built from `b9d7a3cf` (debug protocol v17), 30-second
samples after a 10-second warmup, Guardian paused. The headset was stationary
in low light, so tracking ran in 3DoF for every row; the matrix script refuses
that state, so these rows used the same sequence with the tracking gate
relaxed. App is VrApi's per-frame GPU time; sim/render are the shell's CPU
stage means.

| Mode | Rate | FPS mean / p5 / min | Stale / torn | App | CPU+GPU | CPU / GPU load | sim / render / wait |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| inline, untagged | 90 | 90.5 / 90 / 90 | 0 / 9 | 8.14 ms | 9.56 ms | 0.08 / 0.87 | 0.27 / 0.58 / 0.00 ms |
| inline, `RENDERER_MAIN` | 90 | 90.4 / 90 / 90 | 0 / 0 | 8.67 ms | 11.01 ms | 0.07 / 0.88 | 0.27 / 0.60 / 0.00 ms |
| pipelined, untagged | 90 | 90.5 / 90 / 90 | 0 / 8 | 8.08 ms | 9.15 ms | 0.10 / 0.87 | 0.26 / 0.58 / 0.01 ms |
| pipelined, tagged | 90 | 90.6 / 90 / 90 | 0 / 4 | 8.66 ms | 10.78 ms | 0.08 / 0.88 | 0.27 / 0.59 / 0.01 ms |
| inline, untagged | 120 | 113.5 / 103 / 102 | 405 / 11 | 7.08 ms | 14.57 ms | 0.10 / 0.97 | 0.29 / 0.69 / 0.00 ms |
| inline, `RENDERER_MAIN` | 120 | 120.1 / 119 / 118 | 10 / 1 | 7.06 ms | 9.34 ms | 0.09 / 0.96 | 0.31 / 0.70 / 0.00 ms |
| pipelined, untagged | 120 | 113.5 / 103 / 102 | 399 / 17 | 7.11 ms | 13.82 ms | 0.11 / 0.97 | 0.29 / 0.72 / 0.01 ms |
| pipelined, tagged | 120 | 120.2 / 119 / 119 | 15 / 6 | 7.05 ms | 9.17 ms | 0.09 / 0.96 | 0.26 / 0.69 / 0.00 ms |

- **Pipelining is a wash** at both rates: terrain's CPU stages total under
  1 ms against 7–9 ms of GPU time.
- **Tagging decides 120 Hz.** Untagged runs fell to ~113 FPS with ~400 stale
  frames; tagged runs held 120. A repeat inline pair reproduced it (113.8 vs
  120.3 FPS, 392 vs 10 stale). Tagged runs also show lower compositor time in
  every scene (0.66–0.71 ms vs 1.02–1.05 ms untagged). At 90 Hz tagging
  changes nothing measurable.
- Unverified: the 2026-10-23 untagged 120 Hz terrain run held 120.3 FPS in
  what was likely 6DoF tracking. The untagged shortfall may depend on today's
  3DoF/low-light state or device temperature (41–46 °C, charging).

`neon-city` (#724), inline, untagged unless noted, same build and session:

| Rate | FPS mean / p5 / min | Stale / torn | App | Compositor | CPU+GPU | CPU / GPU load | CPU / GPU level | sim / render |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default (80) | 80.6 / 79 / 79 | 0 / 0 | 6.82 ms | 1.41 ms | 8.77 ms | 0.10 / 0.72 | 2 / 2 | 0.82 / 0.95 ms |
| 90 | 90.9 / 90 / 90 | 0 / 0 | 6.52 ms | 0.90 ms | 9.93 ms | 0.09 / 0.72 | 2 / 2 | 0.67 / 0.80 ms |
| 120 | 120.5 / 120 / 120 | 0 / 0 | 5.35 ms | 1.09 ms | 7.80 ms | 0.13 / 0.83 | 2 / 4 | 0.61 / 0.77 ms |
| 120, `RENDERER_MAIN` | 120.2 / 120 / 120 | 0 / 0 | 5.45 ms | 0.71 ms | 8.42 ms | 0.11 / 0.80 | 2 / 4 | 0.61 / 0.75 ms |

neon-city holds 120 Hz with about 3 ms of GPU headroom (5.35 ms of 8.33 ms,
after the governor raised the GPU to level 4) and about 1.4 ms of CPU.
`/state.instance_uploads` reads 0 or 1 per frame: the only upload is the
2,080-byte hover-car list, rebuilt from `tts` every frame. The static city
lists stay retained (#726).

`night-terrain`, same session:

| Mode | Rate | FPS mean / p5 / min | Stale / torn | App | CPU+GPU | CPU / GPU load | sim / render / wait |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| inline, untagged | 90 | 90.5 / 90 / 88 | 6 / 0 | 8.93 ms | 14.15 ms | 0.24 / 0.94 | 5.11 / 0.77 / 0.00 ms |
| inline, `RENDERER_MAIN` | 90 | 90.7 / 90 / 90 | 0 / 5 | 8.57 ms | 15.35 ms | 0.20 / 0.86 | 4.71 / 0.77 / 0.00 ms |
| inline, untagged | 120 | 103.3 / 85 / 85 | 976 / 8 | 7.91 ms | 17.47 ms | 0.18 / 0.98 | 3.60 / 0.72 / 0.00 ms |
| inline, `RENDERER_MAIN` | 120 | 97.1 / 97 / 97 | 684 / 0 | 8.70 ms | 14.55 ms | 0.18 / 0.96 | 3.60 / 0.70 / 0.00 ms |
| pipelined, untagged | 120 | 103.3 / 85 / 85 | 944 / 11 | 8.00 ms | 15.06 ms | 0.20 / 0.98 | 3.65 / 0.75 / 0.06 ms |
| pipelined, tagged | 120 | 97.9 / 97 / 97 | 665 / 0 | 8.58 ms | 11.03 ms | 0.19 / 0.96 | 3.64 / 0.74 / 0.07 ms |

night-terrain holds 90 Hz but not 120 Hz: it is GPU-bound (load 0.96–0.98,
App about 8 ms of an 8.33 ms budget). Its 3.6–5 ms sim is the largest CPU stage
measured, but moving it off the render thread does not change FPS. Tagging
trades the untagged 85–120 FPS swing for a steady 97 FPS; neither holds the
rate.

## Desktop XR emulation

Run a Functor Lang game with `--emulate-xr` to synthesize the same
`Input.snapshot.xr` record that Quest supplies:

```sh
functor -d examples/my-xr-game run native --emulate-xr
```

The mouse moves the right controller over a rig-local plane; left-click or
Space drives trigger, Enter drives squeeze, arrows drive the thumbstick, and
1/2/3 drive primary, secondary, and thumbstick-click. Both controller poses and
the identity head pose appear in `/state.input.xr`, including under
`--headless`. An `--input-script` can therefore drive the same emulated
controller levels for deterministic captures without adding a desktop-only
game API. This is one shell adapter over the canonical snapshot; future
gamepad/mobile adapters should produce that same target-neutral record.

The emulator's fidelity is deliberately low: both hands sit on a fixed
rig-local plane (`z = -0.55`) with **identity orientations**, so gestures that
depend on depth (pulling a hand back toward your face) or on a controller's
rotation (aiming along a grip's forward) cannot be expressed by mouse and
keyboard at all. For those, **inject the sample directly** over the debug
server — `POST /input {"type":"xr", …}` sets the XR sample every following fixed
step feeds to `sampledInput`, so an arbitrary two-handed pose sequence is
scriptable and deterministic with no headset and no window. See
`docs/debug-runtime.md`, and `e2e/xr-pose-injection.mjs` for a worked
pose-sequence driver.

`examples/xr-controllers` is the shared-path reference: it maps both grip
poses through the authored camera, visualizes each aim direction, and pulls an
orb toward the right controller while trigger is held. Run it unchanged on
desktop or Quest:

```sh
functor -d examples/xr-controllers run native --emulate-xr
functor -d examples/xr-controllers run vr

# Reproducible held-trigger frame (the script path is project-relative):
functor -d examples/xr-controllers run native --emulate-xr \
  --input-script grab.script --capture-frame /tmp/xr-grab.png \
  --capture-at-frame 60
```

## After bring-up (in rough order)

- The typed XR snapshot is exposed to Functor Lang through the per-tick
  `sampledInput` hook and maps through the authored rig with
  `Camera3D.mapTrackedPose`; desktop can synthesize the same record with
  `--emulate-xr`, and `examples/xr-controllers` exercises both paths. Keep
  gamepad and mobile-touch input as typed sibling domains rather than
  XR-specific producer APIs.
- Add the Android audio host; sound bytes already synchronize, but Quest
  currently drains playback commands.
- Add a browser surface over the isomorphic debug API for edit/push/inspect/
  capture without target-specific SDK calls.
- Make the release APK reproducible and publishable (release signing, CI
  artifact, install/update documentation).
- Visual/performance pass: gamma, then `GL_OVR_multiview2`; compare the current
  two-pass device benchmark against multiview on the same headset.
