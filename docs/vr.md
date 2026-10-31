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
- **Floor-locked rig (opt-in):** that first pose includes pitch and roll, so
  by default `xr.head.y` starts at 0 whatever your height, "up" tilts if you
  looked down when it centred, and a recentre re-zeroes to the current head.
  `config`'s `Xr.floorLocked(true)` centres the rig on the floor under the
  head instead, level and yaw-only (the same lock passthrough uses, at floor
  height):
  - `Frame.camera`'s eye stands for that floor point. Keep it level and
    author it at the player's feet; the eye renders at `Frame.camera` +
    `xr.head`.
  - Rig-local `y = 0` is the floor and up is gravity, so `xr.head.y` is the
    eye's height above the floor.
  - A recentre (`ReferenceSpaceChangePending`) re-centres on the current head
    but keeps the floor, so only heading and floor position move; heights
    survive. The snapshot carries no recentre signal.
  - The floor comes from STAGE. Without STAGE (LOCAL fallback, origin at the
    head) the floor is assumed 1.6 m below the starting head, and a recentre
    re-assumes it.
  - It overrides passthrough's room lock.
  - The mapping is `functor_runtime_common::rig_reference`, unit-tested on
    desktop; device verification is pending.

## Device-day checklist (Quest 3)

Use the **`vr-device-loop` skill** (`.claude/skills/vr-device-loop/`) for the
full, proven sequence. The short path is:

1. Put the standard Khronos Android OpenXR loader in
   `runtime/functor-runtime-oculus/lib/arm64-v8a/` (the crate README has the
   Maven command), then `npm run build:oculus:apk` and `adb install -r …`.
2. Run `functor -d yourgame run vr`. It uses device-loopback port 8123 through
   adb, loads every `.fun`/`.funi` plus the project's other files (models,
   textures, sounds, and data for `File.bytes`), and re-pushes changed files on
   save; any changed file hot-reloads the game, so top-level `File.bytes`
   re-reads it. Hidden entries, `dist/`, `target/`, `node_modules/`, `.gltf`
   files and symlinks out of the project aren't pushed; paths are
   case-sensitive and resolve from the entry's folder. Pushes are capped at 64 MB per file and 512 MB in total
   (`--max-file-mb`, `--max-push-mb`). Files the headset lacks, such as
   `config`'s `Config.external` prefixes (this machine's mounts), are fetched from this
   machine when the game asks for them; the game reloads once they arrive, and
   the headset keeps them for later sessions.
3. If Meta Home remains visible, recreate `adb forward`, send `prox_close`, and
   restart NativeActivity with `am start -S`. Wait for `/state`'s frame to
   advance; server readiness alone does not mean XR is rendering.
4. Capture with `POST /capture` using curl's `--fail` and retry flags. A focused
   Quest 3 returns a raw 3360x1760 left/right PNG.
5. Install a non-debuggable release APK, push the explicit workload, and run
   `npm run bench:quest -- --label NAME --warmup 10 --seconds 30`.

### Display refresh rate

The APK starts at the highest rate the display offers up to 80 Hz. A game's
`config` can name its own (`Xr.refreshRate`, see below), requested each time
the game loads. To ask for a different one from the command line, use
`functor -d yourgame run vr --refresh-rate 90` (or `120`), which overrides
`config`.
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

At 90 or 120 Hz the fixed 60 Hz `tick` leaves frames with no step, so
authored motion (a vehicle rig, the world under a moving camera) judders while
head tracking stays smooth. Interpolate it with a three-parameter
`draw = (model, tts, alpha)` — see "Interpolating between steps" in the
`functor-lang` skill and `examples/interpolation`.

### Game options: `config` and `settings`

A game declares headset options through two optional entry points, split by
when the shell can honour them:

```functor
// Read once at load. Changing it on a hot reload prints a one-line
// "restart to apply" warning and keeps the running value.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(90))

// A pure function of the model, evaluated every frame. The shell applies
// it when the value changes.
let settings = (model) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Medium))
```

- `Xr.refreshRate(hz)` belongs in `config`. The APK requests it each time a
  project loads (`functor run vr`'s initial push). An unoffered rate logs a
  warning and falls back to the default.
- `Xr.foveation(level)` belongs in `settings`. `Xr.Off`/`Low`/`Medium`/`High`
  pin the `XR_FB_foveation` level with dynamic scaling off. Without it the APK
  uses `High` with dynamic scaling on. Logcat prints
  `fixed foveated rendering: …` on each change.
- A knob in the wrong bucket is a load error that names the right one.
- Every sibling `.fun` loads with the game, so `config` and `settings` can live
  in a sibling such as `examples/terrain/adapter.fun`, leaving `game.fun`
  unchanged. Each is resolved on its own. The entry's own definition wins. A
  sibling that defines `init` (another role's entry) is skipped. Two siblings
  defining the same one is a load error.
- `/state` reports both as `config` and `settings` (debug protocol v18).
  Desktop reports them too but has no headset to apply them to.
- An unoffered `config` rate also appears as a warning in the `/load-project`
  response.

### Passthrough (mixed reality)

A game shows the room behind its scene with three pieces
(`examples/passthrough` is the kata):

```functor
let config = Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true))
let settings = (model) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.passthroughEnabled(model.mixedReality))
let draw = (model, tts) => Frame.createLit(camera, scene, lights) |> Frame.withPassthrough
```

- `Xr.passthrough(true)` (`config`) starts the camera feed when the project
  loads. The APK enables `XR_FB_passthrough` whenever the runtime offers it
  (and declares `com.oculus.feature.PASSTHROUGH`, not required); the feed is
  only created once a game asks. A headset without it logs a warning, appends
  it to the `/load-project` response, and runs opaque.
- `Xr.passthroughEnabled(on)` (`settings`) pauses or resumes the feed on
  change. Without it the feed runs while `config` asks, even under frames that
  are not passthrough frames, so drive it from the same model state that picks
  `Frame.withPassthrough` (as the kata does) to keep the cameras off otherwise. Logcat prints
  `XrPassthroughFB started` / `stopped`. A pause destroys the feed and a
  resume recreates it running.
- `Frame.withPassthrough` drops the skybox and clears the eye buffer to
  transparent black while the feed is composited; elsewhere (desktop, web,
  feed paused or unavailable) it draws a stand-in room (a darker floor at `y = 0` with a faint 1 m grid, grey walls above the horizon) behind the scene — a
  procedural skybox-like pass, no assets — so scale and floor height read
  without a headset and captures and goldens stay deterministic. It never runs
  under a composited feed.
- Composition: the passthrough layer goes UNDER the projection layer, which is
  flagged `BLEND_TEXTURE_SOURCE_ALPHA`. The eye buffer is premultiplied (the
  transparent clear plus the renderer's source-over blending store
  `color * alpha`), so `UNPREMULTIPLIED_ALPHA` stays off. Opaque geometry
  writes alpha 1 and hides the room, whatever its texture's alpha; translucent
  2D sprites and `Scene.opacity` subtrees let it through. glTF `alphaMode` is not read
  yet, so MASK and BLEND materials render opaque, as they do on desktop.
  The environment blend mode stays `OPAQUE` — the layer brings in the room. A
  runtime without `XR_FB_passthrough` that enumerates `ALPHA_BLEND` uses that
  blend mode instead, with no layer.
- The rig is room-locked: a passthrough game's camera rig stays upright however
  the head was tilted when it centered, and its height is taken from
  `Frame.camera`'s eye, so the stage floor is world `y = 0` under a level
  camera. 1 unit = 1 metre, so a model authored in metres appears at true
  scale.
- `/state.passthrough` (debug protocol v20) reports `mode` —
  `composited`, `paused`, or `unavailable` with a `warning` — while `config`
  requests passthrough; desktop reports `emulated`. `/capture` reads the eye
  buffers before composition, so it shows the game layer over transparent
  black, not the room.

### Scene understanding (environment depth)

`config`'s `Xr.scene(true)` gives the renderer the depth of the real room, for
passes that shade onto its surfaces:

```functor
let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.passthrough(true) |> Xr.scene(true))
let draw = (model, tts) =>
  Frame.createLit(camera, scene, lights)
    |> Frame.withPassthrough
    |> Frame.withStandInRoom(deskAndWall)   // what desktop sees instead
```

- On Quest 3/3S it starts `XR_META_environment_depth` for the run. Each frame
  the shell acquires the newest depth image: a 320x320 two-layer GLES depth
  texture array (left, right) with each layer's pose, field of view and
  near/far (far is infinite). Each eye's layer reaches the renderer with the
  transform that wrote it, composed onto `Frame.camera` like that eye's view,
  so a texel unprojects to the world point behind it.
- No Space Setup is needed, but Horizon gates the depth on the **spatial data
  permission**. The APK declares `com.oculus.permission.USE_SCENE` (without it
  the runtime does not even offer the extension) but does not request it at
  runtime yet, so grant it once per install:
  `adb shell pm grant dev.functor.runner com.oculus.permission.USE_SCENE`.
  Until then `/state.xr.depth` reads `unavailable` with that command in its
  warning.
- `/state.xr.depth` (debug protocol v25) reports `{mode, width, height,
  frames, warning?}`: `device` (an image this frame), `waiting` (the provider
  runs but has none yet), `emulated`, or `unavailable`. `frames` counts images
  acquired since launch. It is omitted unless `config` requests `Xr.scene`.
- Wherever the stand-in room is drawn — desktop, web, a paused or missing
  feed — `Frame.withStandInRoom(scene)` is the environment depth instead.
  That scene is ordinary geometry in metres (floor at `y = 0`), drawn with
  the frame's scene there (so, unlike a camera feed, it occludes virtual
  objects behind it) and also rendered depth-only, from the pass's own
  camera at its resolution, into the same kind of depth texture array. The
  procedural stand-in floor is not part of that depth, so declare a floor in
  the scene if it should count. Desktop reports it as `mode: emulated`, with
  zeros until a stand-in pass has rendered. Under a composited feed it is never
  drawn. Its depth comes from the shadow pass's depth-only walk, so terrain,
  billboards and `Scene.opacity` subtrees in it are drawn but write no depth.
- On Quest, `/state.xr.depth` reports the headset's provider. With the feed
  paused and a stand-in room declared, frames shade the stand-in room even
  while the provider reads `device`.
- Quest 3, nobody wearing it (`prox_close`, 2026-10-26): with the permission
  granted, `/state.xr.depth` reads `{"mode":"device","width":320,
  "height":320}` and `frames` advances (about 66 images a second at 80 Hz).

### Scene-grid kata

`examples/scene-grid` shades a world-aligned grid (25 cm cells, thin cyan
lines) onto whatever surfaces the depth finds — floor, desk, walls, you —
with nothing modelled:

```sh
functor -d examples/scene-grid run vr
functor -d examples/scene-grid run native   # the stand-in floor, desk and wall
```

- `frame |> Frame.withSurfaceGrid(cell, color)` is a fullscreen pass between
  the room and the scene. Each pixel walks its eye ray onto the environment
  depth (a nearest step from a 1.5 m guess, then a bilinear one — the depth
  cameras sit a few centimetres off the eyes), unprojects the world point,
  and draws triplanar lines there, fading 4–8 m out and where a pixel spans
  a large share of a cell (depth edges, grazing angles). Desktop and web run
  the same shader over the stand-in room's depth; the golden
  `scene-grid-t2` covers it.
- X cycles the source (G on desktop): raw depth (`withSurfaceGrid`), the
  fused room mesh (`withFusedRoom`, same cells and colour, see below), and
  off, so the two sources can be compared directly and both judged against
  plain passthrough. `toggle.script` cycles them headlessly; the goldens
  `scene-grid-fused` (frame 90) and `scene-grid-off` (frame 120) capture it.
  Once the fused mode has been shown, fusion keeps running in the background
  in every mode, so the mesh is still there on the next cycle.
- On Quest the lines follow real walls and furniture but wave where the
  320x320 depth is noisy, and round off into corners, which the depth
  smooths.
- Quest 3, release APK, 120 Hz, foveation Off, nobody wearing it
  (2026-10-26): grid on 120.4 fps, App 3.27–3.29 ms, 82–171 stale frames in
  30 s; grid off App 0.55 ms, 0 stale. The pass is about 2.7 ms of GPU at
  full eye resolution.

### Fused room mesh

The surface grid reads each frame's depth afresh, so it only covers what the
headset sees now, and it waves with the depth noise. While `config` requests
`Xr.scene`, the runtime also fuses the depth into a mesh of the room that
fills in as the headset looks around — the approach Lasertag and Hauntify
take on Quest 3, with no Space Setup:

```functor
frame |> Frame.withFusedRoom(0.25, Color.rgb(0.35, 0.9, 1.0))
```

- Every 4th frame the main pass packs its environment depth layer (the
  left eye on Quest; desktop's stand-in depth, downsampled) into a 320x320
  RGBA8 target and `glReadPixels` it into a pixel-pack buffer. A later frame
  maps the buffer once its fence has signalled, so the render thread never
  waits on the GPU.
- A background thread folds each frame into a truncated signed distance
  volume: 8 cm voxels in 8³ chunks hashed by position, distances truncated
  at 3 voxels, a weighted average capped at 32 observations. Voxels a ray
  passes through on its way to a surface count as free space, so a surface
  that is no longer there — a chair that moved — wears away within about 40
  fusions (1–2 s of looking where it was). Marching cubes re-meshes only
  the chunks whose voxels changed sign or drifted about a centimetre, and
  the chunk meshes reach the renderer as ordinary `Shape::Mesh` geometry.
- `Frame.withFusedRoom(cell, color)` draws those meshes with the surface
  grid's lines (the same GLSL): a depth pass first, then the lines of the
  nearest surface, so the mesh also hides virtual geometry behind real
  surfaces. Fusion starts on the first frame that draws the room and keeps
  running while `config` requests `Xr.scene`, so a game can switch back to it
  without starting over; without `Xr.scene(true)` nothing is fused or drawn.
- The mesh lives in world coordinates, which on Quest come from the rig
  reference and `Frame.camera`. Recentring the rig or moving the authored
  camera starts the mesh over.
- Depth beyond 4 m builds no surface but still clears the free space in
  front of it, so a chair moved away from a far wall still fades.
- `/state.xr.mesh` (debug protocol v25) reports `{chunks, triangles,
  fused_frames, voxel_m, colliders, floor_m?}`; the last two are the
  `Physics.fusedRoom` colliders below.
- Desktop and web fuse the stand-in room's depth through the same code.
  Runs pinned by `--fixed-time` or `--input-script` (and wasm, which has no
  threads) fuse inline on the render thread, so a capture after a fixed
  number of frames always shows the same mesh.
- GLES 3.0 has no compute shaders, so fusion runs on the CPU; a GLES 3.1
  compute path is the follow-up if the CPU cost matters.

#### The fused room as colliders

One body makes the fused room solid, whether or not a frame draws it
(`examples/room-bounce` shoots bouncy balls at it):

```functor
let physics = (model) =>
  Physics.scene(gravity, [
    Physics.fusedRoom(roomTag) |> Physics.restitution(0.7),
    // … balls
  ])
```

- Each chunk mesh becomes a static triangle-mesh collider on one fixed body,
  and a re-meshed chunk rebuilds only its own collider. Rays and contacts on
  any chunk report the body's tag.
- The floor is a plane. The fused floor ripples by a few millimetres, which
  keeps a resting ball rolling, and it has holes wherever nobody looked
  (behind the desk). The runtime takes as the floor the lowest level surface
  with at least 0.5 m² within 3 cm of one height, at least 1 m below the
  highest the headset (the depth camera) has been, drops the level
  triangles there from the chunks, and puts an infinite plane at that
  height. It moves only when the estimate moves by 2 mm, and stays until
  the shell starts the room over.
  - Below the head, not at `y = 0`: in a stationary boundary Quest puts the
    stage origin near head height, not on the floor.
  - Seated, 1 m below the head rules out everything but the floor;
    standing, it usually rules out a desk or table. Until the floor itself
    has been seen, a bed or coffee table (or, for someone tall, a desk)
    can pass for it; the floor is lower, and wins once it is seen.
  - A headset at rest on a desk is under 1 m above the floor, so it finds
    no plane until it is worn.
- The meshes reach physics through the same ordered message queue as every
  other shell call: the room fused while rendering frame N collides from the
  physics step of frame N+1. Desktop runs pinned by `--fixed-time` or
  `--input-script` fuse inline, and a pipelined run of them steps in
  lockstep while `config` requests `Xr.scene`, so a scripted capture is the
  same pipelined or not. On Quest fusion runs on its own thread and new
  chunks arrive whenever it finishes.
- Contacts are per body: a ball touching two chunks, or a chunk re-meshed
  under a resting ball, is one contact with the room's tag.
- Walls are surfaces with no thickness. A body that moves farther than its
  radius in one 1/60 s step can end a step behind one, so give small, fast
  bodies `|> Physics.continuous` (continuous collision detection).

### Room-bounce

`examples/room-bounce` shoots bouncy balls at the real room:

```sh
functor -d examples/room-bounce run vr
functor -d examples/room-bounce run native   # desktop stand-in room
```

- The right trigger shoots a 5 cm ball along the controller's aim, once per
  press. X shows or hides the fused room the balls hit. The room fills in as
  you look around, and a ball thrown where nobody has looked yet flies
  through.
- The newest 40 balls stay; the 41st recycles the oldest.
- On desktop the stand-in room (a floor, a desk and three walls) is fused
  instead. Space shoots along the view, a click shoots through the clicked
  point, and G toggles the mesh. `shoot.script` replays a few shots for the
  goldens (its click positions assume `--capture-size 1600x1200`).

### Mixed-reality parts viewer

`examples/parts-mr` shows a printable part on your real desk at true scale
before you print it:

```sh
functor -d examples/parts-mr run vr
functor -d examples/parts-mr run native --emulate-xr   # desktop stand-in room
```

- `Solid.toScene` turns millimetres into metres, and passthrough's floor-level
  stage space makes 1 unit = 1 metre with `y = 0` on the floor, so 1 mm in the
  part is 1 mm in the room.
- The part rests on a see-through virtual desk whose top is `deskHeight`
  (0.75 m) above the floor. Edit it to match your desk and save: the part
  settles onto the new height on hot reload. The desk is placed ahead of where
  you faced at launch, so start the app seated, facing your desk.
- Right trigger grabs, the right thumbstick turns (left/right) and tilts
  (forward/back), A cycles the parts and X toggles passthrough. A released
  part settles back onto the desk, inside its edges.
- On desktop, `--emulate-xr` maps grab, turn/tilt and A to the mouse (or
  Space), the arrows and 1. P toggles passthrough, and N also cycles parts
  without `--emulate-xr`.
- A label in front of the part gives its bounding size in mm (raised
  lettering included, so the 20 mm cube reads 20.6). It is
  `Sketch.text` extruded and shown with `Solid.toScene`, like the part.
- `parts.fun` holds the parts (the keychain tag from `examples/parts`, the
  bookmark from `examples/bookmark` and a 20 mm calibration cube). Any
  `Solid.*` works. The authored camera's eye height and field of view only
  frame the desktop view.

### Sniper scope (desktop first)

`examples/sniper-scope` is a magnified rifle scope: a second camera renders
into a 512² render target shown on a lens at the back of a hollow tube. It is
desktop-only for now; tracked-rifle VR controls come later. Under XR every
render-target pass currently runs once per eye, so a scope pass costs twice.
See its `README.md`.

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

### Terrain vertex budget (#731)

Quest 3, release APKs: `main` at `b9d7a3cf` (protocol 17) vs #731 (protocol
19). Each build pushes its own `game.fun`, so `main` runs `maxPixelError` 2 and
#731 runs 4. All runs are inline with thread tags forced on, at 120 Hz. Each
sample is 30 seconds after a 10-second warmup, in 2 interleaved runs. The room
was dark, so tracking was **3DoF** with Guardian paused, and the head pose was
identical in every run. Vertices per frame are `ovrgpuprofiler`'s
`Vertices Shaded / Second` divided by FPS (2026-10-24):

| Scene | Build | FPS mean / p5 | Stale / torn | App | GPU load | GPU clock | Verts/frame | Vertex / fragment ms | `/state.terrain` |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| terrain | main | 118.5–118.8 / 109 | 63–87 / 2–3 | 7.00–7.03 ms | 0.96 | 640 MHz | 3.78 M | 3.09 / 3.91 | — |
| terrain | #731 | 120.2–120.3 / 120 | 0–2 / 0–2 | 5.85–6.05 ms | 0.84–0.85 | ~605 MHz | 1.82 M | 1.56 / 4.49 | 582,530 (130 patches) |
| night-terrain | main | 99.3–99.4 / 90 | 699–707 / 0 | 8.43–8.48 ms | 0.96 | 640 MHz | 4.28 M | 3.60 / 4.88 | — |
| night-terrain | #731 | 120.3–120.4 / 120 | 1 / 0 | 6.00–6.03 ms | 0.85–0.86 | ~604 MHz | 2.36 M | 1.96 / 4.05 | 331,594 (74 patches) |

The vertex/fragment split is App time weighted by the profiler's `% Time
Shading` counters, from run 1. #731 brings both scenes to a steady 120 Hz. The
profiler counts all geometry, including grass and trees, plus Adreno's binning
pass. It therefore reads well above `/state.terrain` and stays above the 1.5 M
target. With the vertex stage halved, fragment shading is now 67–74% of the
frame. On terrain, texture fetch stalls are 34%.

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
game API.

For a floor-locked game (`Xr.floorLocked(true)`) the emulated head stands
`--xr-eye-height` metres above the floor (default 1.6), with the hands lifted
by the same amount, and the desktop view composes that head (or an injected
one) onto `Frame.camera`, as a headset would. Other games keep the identity
head and the authored view. `npm run test:floor-locked` checks both. This is one shell adapter over the canonical snapshot; future
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

To hold an object, author it facing `+Z` with `+Y` up and place it at the
mapped grip with `Scene.atPose`, which keeps the controller's full orientation
(roll included; local `+X` lands on the hand's left):

```functor
let held = (camera, grip: Input.pose) =>
  Scene.cube()
    |> Scene.scaleXYZ(0.03, 0.03, 0.18)
    |> Scene.translate(Vec3.make(0.0, 0.0, 0.06))   // the blade sticks out ahead
    |> Scene.atPose(Camera3D.mapTrackedPose(camera, grip))
```

`examples/held-pose` draws an asymmetric marker this way.

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
