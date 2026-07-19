# functor-runtime-oculus

The Quest (Meta Horizon OS) runtime shell: an OpenXR + EGL/GLES Android
`cdylib` that renders through the same `functor_runtime_common::render_frame`
path as the desktop and web shells. Like `functor-runner` on desktop, this is
a **tool APK, built once** — games are not baked in; they arrive as Functor Lang source
over the network (the `POST /reload-source` remote-develop loop).

## Status

Phase 1 **verified on hardware** (Quest 3, Horizon OS v205): the OpenXR shell
reaches session FOCUSED and renders the placeholder scene in stereo through
the shared `render_frame` path. Remaining device bring-up: the Functor Lang producer,
network reload, controller input, asymmetric-frustum projection.

## Headless iteration (no one wearing the headset)

The device only promotes a session past IDLE when it believes it's worn, and
blocks launches when controllers are asleep — both bypassable for automated
runs (the manifest's optional `oculus.software.handtracking` handles the
controller gate):

```sh
adb shell am broadcast -a com.oculus.vrpowermanager.prox_close   # fake "worn"
adb shell am start -n dev.functor.runner/android.app.NativeActivity
adb logcat -s functor    # expect IDLE → READY → SYNCHRONIZED → VISIBLE → FOCUSED
adb exec-out screencap -p > /tmp/quest.png   # both rendered eyes, via compositor
# undo: adb shell am broadcast -a com.oculus.vrpowermanager.automation_disable
```

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

**One manual artifact:** the APK must bundle an OpenXR loader as
`lib/arm64-v8a/libopenxr_loader.so` (gitignored; `runtime_libs = "lib"`
bundles it). Without it the app aborts at startup with a clear message.
The easy path is the **standard Khronos loader** — Meta endorses it on
Horizon OS, it needs no developer login, and it's verified working on a
Quest 3 (OS v205):

```sh
curl -sO https://repo1.maven.org/maven2/org/khronos/openxr/openxr_loader_for_android/1.1.61/openxr_loader_for_android-1.1.61.aar
unzip -j openxr_loader_for_android-1.1.61.aar 'jni/arm64-v8a/libopenxr_loader.so' -d lib/arm64-v8a/
```

(Meta's own loader from the
[Meta OpenXR Mobile SDK](https://developer.oculus.com/downloads/package/oculus-openxr-mobile-sdk/)
works identically if you have one on hand.)

## Architecture notes

- Stack: crates.io `openxr` 0.21 (`loaded` feature) + `android-activity`
  (native-activity) + `khronos-egl`. shock2quest's vendored openxrs fork is
  obsolete — the GLES/Android session support it back-ported shipped upstream
  in openxr 0.18 (Feb 2024).
- Quest gotchas inherited from shock2quest (see comments in `src/lib.rs`):
  manual EGL config selection (no `eglChooseConfig`), SRGB8_ALPHA8 swapchains,
  pbuffer-backed context, two swapchains / no multiview (yet).
