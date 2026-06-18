# Backlog

Incomplete work only. For what already exists (rendering, textures, camera, gltf
model loading + skinning/animation, effects/`EffectQueue`, subscriptions seam +
`Sub.every`, input plumbing native + web, hot reload, the CLI, frame capture +
golden tests), see `CLAUDE.md` and `git log`.

## Validation tooling

- [ ] **Debug runtime** (north star): an HTTP/stdin trigger on `functor-runner`
      for on-demand capture + state queries, à la shock2quest's `debug_runtime`
      (`/screenshot`, raycast, entity-state). Grow it out of the existing capture
      path rather than a separate binary; MVU already makes state serializable
      (`emit_state`).
- [ ] **Model inspector** — `functor inspect model <file.glb> [--time T]`: run the
      asset pipeline CPU-side and print per-mesh vertex/index/joint counts and
      skinned AABBs. Text-only, no GPU, diffable. Would have caught both
      skinned-material bugs (zero-vertex meshes, wrong joint rest pose).
- [ ] Headless/offscreen render path (e.g. llvmpipe under xvfb) at a fixed
      resolution so the golden-image test can run in CI (today it's `#[ignore]`d).
- [ ] WASM capture path (today wasm screenshots need an external headless browser).

## Rendering

- [ ] Skinned-material cleanup: select the material per-mesh by skin presence
      instead of forcing `SkinnedMaterial` on every model; move the green joint
      debug-markers and per-frame `Animating` println off the render path;
      revisit `MAX_JOINTS 200` (800 uniform vec4s) vs the WebGL2 minimum guarantee.
- [ ] Mesh primitives: quad, plane, heightmap (cube/cylinder/sphere exist).
- [ ] Dynamic mesh (+ emissive texture material).
- [ ] Lighting: ambient, point, directional, ambient/positional fog, spot →
      shadow mapping.
- [ ] Cubemap / skybox (emissive lighting + reflection).
- [ ] Camera middleware: FPSCamera, OrbitCamera.
- [ ] In-built hands models.
- [ ] Synthwave-ground demo scene.

## Effects & subscriptions

Build the shared machinery once when the first resource-backed sub/effect lands.
The networking design + phased roadmap that drives these items lives in
`docs/multiplayer.md` (Phase 0 = the `Transport`/`VirtualTransport` spine).

- [ ] **Async inbox**: a channel the runtime drains each frame into the
      `EffectQueue`, so messages can arrive on a *later* frame (today `Effect` is
      synchronous — `Effect.run` yields in-frame). Prereq for everything below.
- [ ] **Keyed resource registry**: each frame, diff the desired sub set against
      live resources; spawn new, tear down gone. Identity = resource descriptor
      (e.g. a socket URL), not the generic msg.
- [ ] `Effect.after duration msg` — one-shot delay; a command (needs creation-time
      state), so it can't use `Sub.every`'s stateless clock trick. Needs the inbox.
- [ ] `Sub.renderFrame` — per-frame Sub (always returns a message via poll).
- [ ] `Sub.Net.webSocket` — first resource-backed sub; needs identity so it isn't
      reopened every frame.
- [ ] `Sub.Net.httpRequest` — one-shot request is really an `Effect`; a
      long-poll/SSE stream is the Sub.

## Input

- [ ] Polling snapshot: an `Input.State` (keys currently down + mouse pos/scroll)
      maintained by the runtime from the event stream and handed to the pure core
      — most likely `tick : model -> FrameTime -> Input.State -> ...` — so games
      ask `Input.State.isKeyDown state Key.W` instead of rebuilding held-key state
      from up/down events. Keep events too (press/release edges matter). Held-set
      is derived state: rebuild from events, don't persist in `OpaqueState`.
- [ ] Game-controller and VR-controller events (`Input` has TODO placeholders).

## CLI

- [ ] `functor init [3d|fps]` — scaffold a new game (rust-embed a template folder;
      `init` is currently a stub).
- [ ] `functor develop wasm` — hot-reload over a websocket: push changes, save
      state, reload, rehydrate (native `develop` hot-reload already works).

## Physics

- [ ] Use the rapier library.
- [ ] A `physics : model -> PhysicsScene` function; sync physics positions back
      into the model (or feed the physics scene to the renderer).

## Live variables / fast iteration

- [ ] Live-variable debugging: a `Constant`-style function (name + value) editable
      at runtime, backed by a string→value map. See
      [const-tweaker](https://github.com/tversteeg/const-tweaker); may need a
      custom Fable build for token reloading.

## Audio

- [ ] Sound-effect playback (no audio yet).

## 2D

- [ ] Sprites.

## VR runtime

- [ ] (placeholder — VR head/controller rendering + input)

## Games to build

- 3D: Simple Terrain (synthwave vibes)
- 3D: Multiplayer Asteroids
- 3D: Simple FPS (shooting range; weapons / recoil)
- 3D: Multiplayer FPS (weapons / recoil)
- MVP — 1p Asteroids (model loading, lighting, sprites, sound, input), then
  battle-royale asteroids with bots.

## Presentation / brainstorming

- A 3D-focused presentation showcasing the architecture.
- VR dev experience: a table-top / light-table-esque editing experience.
