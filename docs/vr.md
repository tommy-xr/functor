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
| current | Sample Quest Touch grip/aim poses, analog controls, and buttons into the shared typed input snapshot | in progress; Quest 3 + debug/SDK verified |

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

## After bring-up (in rough order)

- Finish the sampled-input split: expose the typed XR snapshot to Functor Lang,
  then add desktop emulation and a controller-driven example. Keep gamepad and
  mobile-touch input as typed sibling domains rather than XR-specific producer
  APIs.
- Add the Android audio host; sound bytes already synchronize, but Quest
  currently drains playback commands.
- Add a browser surface over the isomorphic debug API for edit/push/inspect/
  capture without target-specific SDK calls.
- Make the release APK reproducible and publishable (release signing, CI
  artifact, install/update documentation).
- Visual/performance pass: gamma, then `GL_OVR_multiview2`; compare the current
  two-pass device benchmark against multiview on the same headset.
