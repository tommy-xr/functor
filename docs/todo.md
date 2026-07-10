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
- [ ] Headless/offscreen render path (e.g. llvmpipe under xvfb) at a fixed
      resolution so the golden-image test can run in CI (today it's `#[ignore]`d).
- [ ] WASM capture path (today wasm screenshots need an external headless browser).

## Rendering

- [ ] Skinning-space bug: the loader builds the skeleton from `skin.joints()`
      only, dropping the transform of ancestor nodes above the skin root (the
      Mixamo Armature's cm→m scale + orientation), and the renderer multiplies
      the skinned mesh's own node transform (the glTF spec says to IGNORE it
      for skinned meshes). Net effect: Xbot's skinned pose renders cm-scale
      lying along -Z (`functor inspect model Xbot.glb --time 0.8` shows the
      skinned AABB), and every skinned example compensates with eyeballed
      scales/rotations (`examples/animation` documents the workaround).
      Fixing this changes the rendered size of every skinned model — its own
      PR, with all examples re-verified and goldens regenerated.
- [ ] Skinned-material cleanup: select the material per-mesh by skin presence
      instead of forcing `SkinnedMaterial` on every model; move the green joint
      debug-markers and per-frame `Animating` println off the render path;
      revisit `MAX_JOINTS 200` (800 uniform vec4s) vs the WebGL2 minimum guarantee.
- [ ] Dynamic mesh (+ emissive texture material).
- [ ] Lighting: ambient, point, directional, ambient/positional fog, spot →
      shadow mapping. (Distance fog done 2026-07-04 — `Frame.withFog`,
      linear + exp; volumetric/positional fog still open.)
- [ ] Cubemap / skybox (emissive lighting + reflection). (Skybox rendering
      done 2026-07-04 — `Frame.withSkybox`, six-face cubemaps; cubemap
      reflection/IBL still open.)
- [ ] Camera middleware: FPSCamera, OrbitCamera.
- [ ] In-built hands models.

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
      is derived state: rebuild from events, don't persist it across a reload.
- [ ] Game-controller and VR-controller events (`Input` has TODO placeholders).

## CLI

- [ ] `functor develop wasm` — hot-reload over a websocket: push changes, save
      state, reload, rehydrate (native `develop` hot-reload already works).

## Physics

Design + phased roadmap: `docs/physics.md` (Rapier-backed, functional
`physicsScape` reconcile, deterministic fixed-step, `Simulatable`/`Timeline`
rewind seam, server-authoritative prediction). First step = Phase 1, the Rust-only
shell spine (no game-language surface).

- [ ] Phase 1: Rapier dep + `physics` module + fixed-step + `Timeline` traits +
      determinism goldens.
- [ ] `physicsScape : model -> PhysicsScene` hook + `Physics.View` read-back +
      `hello-physics` example.
- [ ] Pause/rewind/replay via keyboard (the local culmination).
- [ ] Networked physics: grow `mpserver`/`mpclient` to client-owned balls +
      server-owned objects (state-sync, then prediction).

## Time-travel tooling

Design + phased roadmap: `docs/time-travel.md` (generalize #215's physics rewind
into a generic, shell-owned whole-game scrubber — pause/scrub/rewind/replay/branch
— plus the authoring experiences it unlocks). Builds on the physics
`Simulatable`/`Timeline` seam; Functor Lang-first. Overlaps the "Live variables / fast
iteration" item below (T6, trajectory preview).

- [ ] T1: model as a `Simulatable` (snapshot = `Value` clone, command = frame
      inputs) + a unified frame clock seeking model and physics together; goldens.
- [ ] T2: pointer/click input plumbing (real `RawInput` to egui; deliver mouse
      buttons to the runtime) — unblocks the scrubber and interactive UI.
- [ ] T3: shell-owned egui scrubber overlay; `~` toggle (native), default-on (web/VSCode).
- [ ] T4: interactive Functor Lang `View` (`Button { label, onClick }`, storable closure).
- [ ] T5: forked timelines + ~50%-opacity render composite.
- [ ] T6: trajectory preview (deterministic forward-sim + trail draw) — the
      *Inventing on Principle* demo.

## Language (Functor Lang)

**SHIPPED** — Functor Lang is now Functor's only game-logic language; the F#/Fable
pipeline was deleted (roadmap E3). The full history and design record is in
`docs/functor-lang.md` (all tracks/checkboxes done); the live language reference is the
`functor-lang` skill. Remaining language follow-ups tracked there: `.funi`
interface files (B8 part 2), LSP cross-file support, and wasm/live-preview
multi-file.

## Live variables / fast iteration

- [ ] Live-variable debugging: a `Constant`-style function (name + value) editable
      at runtime, backed by a string→value map. See
      [const-tweaker](https://github.com/tversteeg/const-tweaker). (Functor Lang hot-reload
      already rebinds edited top-level values live; this is the finer-grained,
      no-reload knob.)

## Audio

- [ ] Sound-effect playback (no audio yet).

## 2D

- [ ] Sprites.

## VR runtime

- [ ] (placeholder — VR head/controller rendering + input)

## Games to build

- 3D: Multiplayer Asteroids
- 3D: Simple FPS (shooting range; weapons / recoil)
- 3D: Multiplayer FPS (weapons / recoil)
- MVP — 1p Asteroids (model loading, lighting, sprites, sound, input), then
  battle-royale asteroids with bots.

## Presentation / brainstorming

- A 3D-focused presentation showcasing the architecture.
- VR dev experience: a table-top / light-table-esque editing experience.
