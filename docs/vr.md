# VR / XR track — status

Functor on headsets, built as a train of small PRs (2026-07-03/04). Two devices,
one architecture: the **Xreal One** as a desktop 3DoF stereo dev display, and
the **Quest 3** as the real standalone target. The through-line is Functor's
existing principles doing the heavy lifting: `draw` is a pure `Frame`, so
stereo is "render it twice with shell-supplied cameras"; Functor Lang games are text
interpreted at runtime, so the headset runtime is a **tool APK built once** and
games deploy over the network.

## Shipped

| PR | What | Status |
| --- | --- | --- |
| [#175](https://github.com/tommy-xr/functor/pull/175) | `--stereo-sbs`: side-by-side stereo in functor-runner (`Camera::stereo_eyes`, `render_frame` takes the camera explicitly) | merged |
| [#177](https://github.com/tommy-xr/functor/pull/177) | `--xreal-tracking`: Xreal One 3DoF head tracking — IMU over TCP (the glasses expose a USB network interface; no drivers), gyro-bias calibration + Mahony fusion, head rotation composed onto the game camera in its local basis | merged |
| [#185](https://github.com/tommy-xr/functor/pull/185) | Axis remap corrected from a live wear-test (raw sensor frame is right/down/forward — the optical convention; the first guess was an improper det −1 transform, which mirrors gyro vs accel) + auto/yaw-only recenter | merged |
| [#183](https://github.com/tommy-xr/functor/pull/183) | Network hot-reload: `POST /reload-source` on the debug server (model preserved, broken push keeps the old program), `--debug-bind`, `functor push <addr> [--watch]` | open, CI green |
| [#189](https://github.com/tommy-xr/functor/pull/189) | Quest OpenXR runtime shell (`runtime/functor-runtime-oculus`): EGL/GLES 3.2 + openxr 0.21 + android-activity, per-eye sRGB swapchains, head-pose cameras, renders through the shared `render_frame`. Builds to a signed APK (`npm run build:oculus[:apk]`) | open, CI green |
| `feat/oculus-functor-lang-producer` | Functor Lang producer + debug server shared into `functor_runtime_common`; the Quest shell boots an embedded demo game (`src/demo.functor`), serves `/state` `/scene` `/reload-source` LAN-wide on :8077 — `functor push <quest-ip>:8077` replaces the running game live | branch pushed; PR after #183/#189 merge |

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

## Device-day checklist (Quest 3)

1. Merge #183 and #189 (both green), then the producer PR.
2. Download the Meta OpenXR Mobile SDK; copy
   `libopenxr_loader.so` → `runtime/functor-runtime-oculus/lib/arm64-v8a/`
   (license-gated, can't be fetched automatically — see the crate README).
3. Quest in developer mode, USB: `cargo apk run` from the crate dir
   (env vars per README). Expect the cube-ring demo in-headset.
4. `functor -d yourgame push <quest-ip>:8077` from the Mac — live-edit the
   game running in the headset.

## After bring-up (in rough order)

- Visual pass: gamma, asymmetric eye frusta (needs a raw-projection seam in
  the shared renderer), then `GL_OVR_multiview2`.
- Controllers + a VR `InputContext` (head/hands) with desktop mouse/keyboard
  emulation, shock2quest-style, so VR games iterate without a headset.
- On-device frame capture (`POST /capture` currently returns 503 on the
  headset) to restore the golden/agent-verification loop there.
