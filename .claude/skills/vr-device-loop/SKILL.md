---
name: vr-device-loop
description: >-
  Build, install, launch, live-reload, inspect, capture, troubleshoot, and
  benchmark Functor on an adb-attached Meta Quest. Use for Oculus/OpenXR runtime
  changes, `functor run vr`, headset rendering verification, stereo captures,
  device performance measurements, or recovery when the APK remains at Meta
  Home, the XR session dozes, or adb forwarding disappears.
---

# VR device loop

Use the Quest as the acceptance environment while keeping Functor's desktop and
VR debug APIs isomorphic. Read `runtime/functor-runtime-oculus/README.md` and
`docs/vr.md` before changing the pipeline.

## Preserve the contract

- Keep the Functor Lang program and `Frame` contract target-independent. Put
  headset pose, stereo views, controllers, and compositor behavior in the shell.
- Keep the canonical debug routes and wire types in
  `functor-runtime-common`; desktop reports `main`, Quest reports `left` and
  `right` through the same `views` field.
- Treat source and assets separately. `/reload-project` transfers `.fun`/`.funi`
  source; it does not currently transfer texture, model, or audio files.
- Use release APKs for performance claims. Verify Android does not report the
  installed package as `DEBUGGABLE`.

## Build and install

Resolve the one attached device first and pass `-s` when more than one exists:

```sh
adb devices -l
npm run build:oculus:apk
adb -s SERIAL install -r target-android/debug/apk/functor_runtime_oculus.apk
```

Use `npm run build:oculus` for a cross-compile check without packaging. For a
benchmark, build `cargo apk --release` with a locally configured signing key,
install `target-android/release/apk/functor_runtime_oculus.apk`, and never commit
keystore paths or passwords.

After any runtime Rust change, rebuild and reinstall; `functor run vr` only
pushes game source.

## Establish a rendering session

ADB reconnects clear forwarding rules. Recreate the forward before every
verification phase:

```sh
adb -s SERIAL forward tcp:8123 tcp:8123
adb -s SERIAL shell am broadcast -a com.oculus.vrpowermanager.prox_close
adb -s SERIAL shell am start -S -n dev.functor.runner/android.app.NativeActivity
adb -s SERIAL logcat -s functor
```

Expect `READY -> SYNCHRONIZED -> VISIBLE -> FOCUSED`. `am start` and `GET /`
becoming ready do not prove XR is rendering. Poll `/state` twice until `frame`
advances. If Meta Home remains visible, send Back, repeat `prox_close`, and
restart with `-S`. Check that NativeActivity is top-resumed with `dumpsys
activity activities`.

Avoid concurrent broad `adb logcat -d` consumers: a device/daemon reconnect can
drop both the session and the forward.

## Push and inspect

Prefer the integrated loop:

```sh
./target/debug/functor -d examples/primitives run vr
```

It launches, forwards port 8123, pushes the whole project, watches `.fun`/`.funi`
files, and streams logs. For individual operations:

```sh
adb -s SERIAL forward tcp:8123 tcp:8123
curl --fail-with-body http://127.0.0.1:8123/state | jq
curl --fail-with-body http://127.0.0.1:8123/scene | jq
curl --fail-with-body http://127.0.0.1:8123/trace | jq
```

Use the TypeScript SDK for scripted `/input`, `/time`, reload, rewind, and
multi-client assertions. Validate clock behavior by observing a stable frame
after `set`, exactly one increment after `advance`, and progression after
`resume`.

## Capture both raw eyes

`POST /capture` reads the two rendered swapchain framebuffers before compositor
warping and returns a left-then-right 3360x1760 PNG on Quest 3. A dozing session
honestly returns HTTP 503. Retry it and make curl fail on HTTP errors so text is
never mistaken for a PNG:

```sh
curl --fail --show-error --retry 30 --retry-delay 1 \
  --retry-connrefused --retry-all-errors -X POST \
  http://127.0.0.1:8123/capture -o /tmp/quest-stereo.png
file /tmp/quest-stereo.png
```

Inspect the PNG. Require two complete eye panels, expected horizontal disparity,
no systematic vertical disparity, correct framing/depth, and no fallback assets
before treating it as visual evidence. An adb compositor screenshot is useful
for presentation, but the runtime capture is the renderer diagnostic.

## Benchmark on the headset

Push the exact workload first; the benchmark never installs or pushes. Then:

```sh
npm run bench:quest -- --label primitives --warmup 10 --seconds 30
```

Record device/model, view resolution, FPS mean/p5/min, stale/torn frames, App and
Timewarp time, CPU/GPU load, and memory. Run a representative scene and a stress
scene. State explicitly when missing asset sync means a source-heavy workload
is representative of CPU/geometry cost but not final texture behavior.

Also run the host `frame_bench` base-vs-change when the patch touches a per-frame
path, following `CLAUDE.md`.

## Finish cleanly

Restore proximity automation when unattended testing is complete:

```sh
adb -s SERIAL shell am broadcast \
  -a com.oculus.vrpowermanager.automation_disable
```

For visual PRs, use the `pr-visuals` skill and include Quest media when the claim
depends on headset behavior. Report the exact APK type, device, pushed workload,
capture dimensions, and benchmark interval.
