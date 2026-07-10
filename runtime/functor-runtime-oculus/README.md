# functor-runtime-oculus

The Quest (Meta Horizon OS) runtime shell: an OpenXR + EGL/GLES Android
`cdylib` that renders through the same `functor_runtime_common::render_frame`
path as the desktop and web shells. Like `functor-runner` on desktop, this is
a **tool APK, built once** — games are not baked in; they arrive as Functor Lang source
over the network (the `POST /reload-source` remote-develop loop).

## Status

Phase 1: the OpenXR shell cross-compiles (instance/session/swapchains/frame
loop, per-eye rendering with head-pose cameras, placeholder scene). Device
bring-up (Functor Lang producer, network reload, controller input, asymmetric-frustum
projection) happens against real hardware.

## Prerequisites

- Android SDK with **NDK** (any recent; developed against 24.x) — set
  `ANDROID_HOME` (e.g. `/opt/homebrew/share/android-sdk`)
- `rustup target add aarch64-linux-android`
- `cargo install cargo-ndk` (build the `.so`), `cargo install cargo-apk`
  (package the APK)

## Build the dylib (no device needed)

```sh
npm run build:oculus
# = ANDROID_HOME=… cargo ndk -t arm64-v8a build -p functor_runtime_oculus
```

Android builds use `CARGO_TARGET_DIR=target-android` (a cleaner/analyzer race
on the shared `target/` corrupts cross-compile fingerprints).

## Package + install the APK

`cargo apk` reads the `[package.metadata.android]` section of Cargo.toml. It
additionally needs `ANDROID_NDK_ROOT` and `platforms;android-32` installed
(`sdkmanager --install 'platforms;android-32'`):

```sh
npm run build:oculus:apk   # → target-android/debug/apk/functor_runtime_oculus.apk (debug-signed)
# on-device (from runtime/functor-runtime-oculus, same env vars):
cargo apk run              # builds + adb installs + launches on the headset
```

**One manual artifact:** OpenXR on Quest needs **Meta's loader**. Download the
[Meta OpenXR Mobile SDK](https://developer.oculus.com/downloads/package/oculus-openxr-mobile-sdk/)
and copy `OpenXR/Libs/Android/arm64-v8a/Release/libopenxr_loader.so` to
`lib/arm64-v8a/` in this directory (gitignored; `runtime_libs = "lib"` bundles
it into the APK). Without it the app aborts at startup with a clear message.
Do not substitute the Khronos loader — Meta's is the supported path.

## Architecture notes

- Stack: crates.io `openxr` 0.21 (`loaded` feature) + `android-activity`
  (native-activity) + `khronos-egl`. shock2quest's vendored openxrs fork is
  obsolete — the GLES/Android session support it back-ported shipped upstream
  in openxr 0.18 (Feb 2024).
- Quest gotchas inherited from shock2quest (see comments in `src/lib.rs`):
  manual EGL config selection (no `eglChooseConfig`), SRGB8_ALPHA8 swapchains,
  pbuffer-backed context, two swapchains / no multiview (yet).
