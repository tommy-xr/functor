# functor-runtime-oculus

The Quest (Meta Horizon OS) runtime shell: an OpenXR + EGL/GLES Android
`cdylib` that renders through the same `functor_runtime_common::render_frame`
path as the desktop and web shells. Like `functor-runner` on desktop, this is
a **tool APK, built once** — games are not baked in; they arrive as Functor Lang source
over the shared debug-runtime protocol.

## Status

The OpenXR shell, interpreted Functor Lang producer, USB remote-develop loop,
authored-camera rig, exact asymmetric per-eye projection, project asset sync,
and sampled Touch controller input are implemented for Quest 3. The shared
debug/REPL protocol adds raw stereo framebuffer capture, rig-local
head/controller inspection, and desktop-isomorphic control. Remaining input
work is desktop emulation and hand tracking; Android audio and multiview
rendering are also still open.

## Camera contract

`Frame.camera` stays target-independent. On Quest its pose is the center-eye
view when tracking is established; live OpenXR eye poses are applied as
reference-relative translation and rotation in that camera's local basis.
Changing the game camera therefore moves the whole play-space rig (locomotion),
while moving your head remains live and shell-owned. The authored near/far clip
range is preserved. OpenXR still owns IPD and the exact per-eye optical FOV.

The reference center is the midpoint of the first valid left/right eye poses.
It survives source reload and session doze; an OpenXR reference-space change
recenters it at the runtime's announced change time. Existing desktop games
therefore begin with their authored framing instead of inheriting the Quest's
absolute stage coordinates.

## The remote-develop loop (M1)

The APK boots an embedded scene (`src/boot.fun`) and listens on **device
loopback** on port 8123 for the same debug protocol as desktop. The dev PC
reaches it over USB (loopback-only
binding keeps the LAN out; note another app ON the device could reach
loopback — an accepted dev-tool tradeoff):

```sh
functor -d mygame run vr    # the whole loop, one command
```

`run vr` finds the adb device, launches this APK, forwards the port, loads
the whole source project (`POST /load-project` — sibling modules included)
plus its model/texture/audio files, then re-checks + re-pushes changed source
or assets on every save while streaming the headset's runtime log into the
terminal. The initial load takes the model from `init`; later source pushes use
`POST /reload-project` and preserve it. A project that declares a same-file
entry role (`{"file": "game.fun", "module": "Server"}` or a binding `prefix`)
gets it DECLARED in each push's query string — `POST /load-project?module=Server`
— which is how a device with no command line learns which contract to boot.
The shell re-resolves the role against every pushed program, so an edit that
deletes the block fails naming it and the old program keeps running. A push
with no role query leaves the role already in force alone. Assets transfer individually through
`POST /reload-asset`; a final
`POST /sync-assets` manifest removes deleted uploads and changed render assets
are decoded again on the next frame. The pieces also work individually:

Sound bytes are synchronized into the device cache, but Quest audio playback
is still pending its Android audio host; the shell currently drains audio
commands without playing them. Sounds therefore do not yet drive
`Sub.assets`, whose current load/decode pipeline covers models and textures.

```sh
adb forward tcp:8123 tcp:8123
functor -d mygame push 127.0.0.1:8123 [--watch]   # entry-only push (desktop remote-develop command)
curl --fail-with-body -X POST --data-binary @game.fun http://127.0.0.1:8123/reload-source
curl -s http://127.0.0.1:8123/state | jq
curl --fail --show-error --retry 30 --retry-delay 1 \
  --retry-connrefused --retry-all-errors -X POST \
  http://127.0.0.1:8123/capture -o quest-stereo.png
```

`GET /`, `GET /state`, `GET /scene`, `GET /trace`, `POST /capture`,
`POST /input`, `POST /time`, `POST /reload-source`, `POST /load-project`,
`POST /reload-project`, `POST /reload-asset`, `POST /sync-assets`, and
`POST /rewind` have the same
request/response forms as desktop. `POST /display-refresh-rate` `{"hz":90}` is
Quest-only: it requests one of the display's offered rates, and `/state`'s
`display_refresh_rate` reports `current`, `requested`, and `available`. The
APK starts at the highest offered rate up to 80 Hz; `functor run vr
--refresh-rate 90` requests 90 Hz after the initial push. `/state` reports `left` and `right` views;
`/capture` returns their raw framebuffer pixels as one left-then-right
side-by-side PNG, before compositor warping. The server is reachable while the
headset dozes, but capture correctly returns 503 until XR is rendering. After
any adb reconnect, recreate the port forward; poll `/state` until `frame`
advances before capture so cached paused state is not mistaken for readiness.

While valid tracking is available, `/state.input.xr` contains center-head plus
per-hand grip/aim poses, trigger, squeeze, thumbstick, primary/secondary,
thumbstick-click, and menu state. Poses are relative to the same center-eye
tracking reference that anchors `Frame.camera` (+X right, +Y up, -Z forward),
not absolute stage coordinates. An inactive or untracked controller is
represented explicitly instead of retaining a stale pose. This is the first
half of the input seam: runtime/debug tooling can inspect it. Functor Lang
games consume the same portable record once per fixed step through the optional
`sampledInput(model, snapshot: Input.snapshot)` hook; `Camera3D.mapTrackedPose`
maps its rig-local poses through the game's authored `Frame.camera`.

## The device link (desktop app over Wi-Fi)

The APK dials the Functor desktop app over Wi-Fi, so the desktop reaches this
debug runtime with no cable (`functor-device-link`'s `client`). To pair, hold
the **left menu button** for a second: the headset looks for desktops on the
network (mDNS `_functor-link._tcp`) and shows a code over whatever runs. When
the desktop shows the same code, press **A** (Matches) and click Allow there;
**B** cancels. From then on the headset finds that desktop by its id and
relinks by itself, with backoff, after a restart or a dropped network.
`/state.link` reports the link and any pairing on screen.

- Only the controllers drive the overlay. For automation, an `xr` sample
  injected on port 8123 (over `adb forward`) does too, but only while
  `adb shell setprop debug.functor.pair_input 1` is set; that is how
  `npm run test:quest-link` pairs a Quest nobody wears. The desktop's requests
  arrive on a second loopback debug listener and never count, and game code
  (which can reach port 8123) cannot set a property: neither a desktop nor a
  game it pushed can start or confirm a pairing.
- mDNS answers arrive as multicast, which Android drops unless the app holds a
  `WifiManager.MulticastLock` (`CHANGE_WIFI_MULTICAST_STATE`): held only while
  browsing.
- **Over the cable**, with no code: plug the headset in and click "Pair over
  the cable" in the desktop app's Devices pane. The cable and your adb
  authorisation are the trust, so it needs a debuggable build (`run-as`); a
  release build pairs with the code. The desktop writes the pairing file and
  reads it back; the secret travels only on adb's stdin and stdout.
- The pairing is `files/device-link.json` in the app's private storage:
  `{"device", "desktop", "desktop_name", "secret"}` (hex ids and secret). The
  app re-reads it every 5 s (and before each connection attempt), so a
  pairing written from outside lands without a restart; one that replaces the
  pairing of a live link closes that link, telling its desktop why. Write it
  whole (a file that does not parse is moved aside as `.bad`) and keep the
  app's `device` id: the desktop knows the headset by it. The commands are
  `functor_device_link::adb::{READ_PAIRING, WRITE_PAIRING}`.
- A refusal in the clear (`NotPaired`) never removes the stored secret: anyone
  on the network could forge one. Pair again to replace it.

`npm run test:quest-link` runs the whole path on an attached Quest with a
debuggable APK, on the same Wi-Fi as this machine: pair through injected
presses (both sides must show one code), tunnel `GET /state`, check the
tunnel's presses never reach the overlay, restart the app and relink, forget.
It pauses Guardian (`debug.oculus.guardian_pause`), which otherwise holds the
app at its boundary dialog, and puts the headset's own pairing back after.
`FUNCTOR_QUEST_CAPTURES=<dir>` saves stereo captures of the overlay.
`npm run test:quest-cable` pairs it over the cable instead, waits for it to
link over Wi-Fi, tunnels `GET /state`, replaces its pairing as another desktop
would (this one is told why), pairs again and forgets.

## Benchmark on the actual headset

Build/install a release APK, push the game to measure, then collect the Quest's
own VrApi timing plus process memory (the script does not install or push, so
the workload is explicit):

```sh
ANDROID_HOME=… ANDROID_NDK_ROOT=… CARGO_TARGET_DIR=$PWD/target-android \
  cargo apk build --release --manifest-path runtime/functor-runtime-oculus/Cargo.toml
adb install -r target-android/release/apk/functor_runtime_oculus.apk
functor -d examples/synthwave run vr
npm run bench:quest -- --label synthwave --warmup 10 --seconds 30
```

The report uses Meta's on-device one-second telemetry buckets: FPS, stale/torn
frames, application and compositor time, CPU/GPU time, clock levels and load.
It also reports the requested and achieved display refresh rate (from `/state`)
and PSS/RSS and graphics memory from `dumpsys meminfo`.

Semantics match desktop hot-reload: the **model is preserved** (closures
stored in the model rebind by def-name), a broken push returns the rendered
error with HTTP 400 and the old program keeps running, and a push landed
while the headset dozes applies the moment the session resumes.

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

**CI builds.** Every main push and PR head gets a release-profile APK as the
`functor-apk-<sha>` artifact of the `Build Android` workflow (30 days),
reporting `pr | <sha7>` as `/state.build` (local builds report `dev`). CI signs
it with a shared dev key (`CN=Functor Dev (CI)`). Local and CI builds are both
versioned 0.0.0 (below any release), so a local build signed with the same key
replaces a CI one in place and vice versa: point cargo-apk at that keystore with
`CARGO_APK_DEV_KEYSTORE` / `CARGO_APK_DEV_KEYSTORE_PASSWORD` (debug builds) or
`CARGO_APK_RELEASE_KEYSTORE` / `CARGO_APK_RELEASE_KEYSTORE_PASSWORD`
(`--release`). An install signed with another key, or versioned above 0.0.0
(builds from before the reset were 0.1.0), must be uninstalled first:
`adb uninstall dev.functor.runner`.

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
