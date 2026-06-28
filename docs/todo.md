# Backlog

Incomplete work only. For what already exists (rendering, textures, camera, gltf
model loading + skinning/animation, effects/`EffectQueue`, subscriptions seam +
`Sub.every`, input plumbing native + web, hot reload, the CLI, frame capture +
golden tests), see `CLAUDE.md` and `git log`.

## Validation tooling

- Debug runtime: the `--debug-port` HTTP control server exists (`/capture`,
      `/state`, `/scene`, `/input`, `/time` clock control, `GET /` index — see
      `docs/debug-runtime.md`). Remaining:
  - [ ] **TypeScript SDK** (`tools/functor-sdk`): typed client over the endpoints
        (single-client + a multi-client lockstep session for **multiplayer
        simulation** — N runners, pinned clocks, stepped together), plus a gated
        e2e harness driving `hello`. First e2e guard: inject `up` → step → assert
        `model.held.up`.
  - [ ] Richer observation: a structured (parseable) state snapshot + held-input
        summary, beyond today's `Debug`-text `model`.
  - [ ] Raycast / entity-state queries (à la shock2quest's `debug_runtime`).
- [ ] **Revisit: native runner on Linux + Windows (pick up with the Fable model).**
      The native `functor-runner` SIGSEGVs on **both Linux and Windows** during the
      loaded game's `init()`; **only macOS works**, which is why the e2e CI runs on
      macOS (`e2e.yml`). Goal: get native Linux (and ideally Windows) working, then
      run the e2e there too. Diagnosis (reproducers: `linux-native-debug.yml` on
      branch `debug/linux-native-runner`, `windows-native-eval.yml` on
      `eval/native-platform-support`): gdb on Linux shows a Fable lazy-static
      returning a **NULL `Rc`** — `Rc<MutCell<Vec<LrcStr>>>` dropped at
      `hello.rs:888`, so `Rc::drop` null-derefs; Windows crashes the same way.
      **Ruled out:** `-C prefer-dynamic` (shared libstd) and `crate-type=["cdylib"]`
      — both still crash, so it's neither std-duplication nor the crate-type. That
      Windows (`LoadLibrary`) and Linux (`dlopen`) both fail while macOS doesn't
      points at how Fable's generated `OnceInit`/lazy-statics initialize when the
      crate is loaded at runtime rather than linked at startup — revisit with
      Fable-backend knowledge.
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

Design + phased roadmap: `docs/physics.md` (Rapier-backed, functional
`physicsScape` reconcile, deterministic fixed-step, `Simulatable`/`Timeline`
rewind seam, server-authoritative prediction). First step = Phase 1, the Rust-only
shell spine (no F# surface).

- [ ] Phase 1: Rapier dep + `physics` module + fixed-step + `Timeline` traits +
      determinism goldens.
- [ ] `physicsScape : model -> PhysicsScene` hook + `Physics.View` read-back +
      `hello-physics` example.
- [ ] Pause/rewind/replay via keyboard (the local culmination).
- [ ] Networked physics: grow `mpserver`/`mpclient` to client-owned balls +
      server-owned objects (state-sync, then prediction).

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
