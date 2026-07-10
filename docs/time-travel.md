# Time-travel tooling

Status: **the whole-game scrubber is shipped** (T1–T3); the authoring
experiences (T4–T6) are not yet built. This is the design doc *and* the record
of what landed: **generic time-travel tooling across the whole game** — pause,
scrub, rewind, replay, branch — plus the authoring experiences those unlock. It
generalizes the physics `Timeline` (`docs/physics.md` Phase 6 / #215) from the
Rapier world to the entire MVU model.

## What shipped (as of 2026-07-05)

You can pause a running Functor Lang game and **drag a timeline scrubber to any recorded
frame** — the whole scene (MVU `model` *and* physics world) restores together —
on both the desktop runner and the web/VSCode preview. Exercised by
`examples/physics`.

- **The coupled recorder** (`functor_runtime_common::timetravel`): a bounded
  per-frame snapshot ring `History<T>` and a `SceneRecorder` over it. Each
  rendered frame it records the settled `model` (an `Rc`-cheap `functor_lang::Value`
  clone) and, in lockstep, the physics fixed-frame the world reached. **Shared
  by both shells** — one tested impl; the producer hands in its `model` /
  `SteppedPhysics` / status.
- **Coupled seek, exact-or-refused** (`rewind_scene_to` / `seek_scene_to` on the
  `GameProducer` trait): restores model + world to a rendered frame, refusing
  (touching nothing) rather than landing them on different times when the two
  retention windows disagree. Non-destructive scrubbing (`seek_scene_to`) lets
  you drag back *and* forth; the future is branched only when play resumes.
- **Live triggers.** Desktop debug server `POST /rewind {"frame":N}` (#225); an
  egui scrubber overlay on desktop (`~` console toggle, hidden by default); a
  **native DOM** scrubber on web (index-functor-lang.html, outside the canvas — see the
  design note below).
- **PRs:** `History` primitive #218, per-frame model recording #219, coupled
  seek #222, `POST /rewind` #225, the scrubber + web parity #226.

### Design decisions that emerged (diverged from the original plan)

- **Two rings, one clock — not a single frame-level `Simulatable`.** The model
  is `Rc`-cheap, so it uses a plain **snapshot-ring** (`History<Value>`, snapshot
  every frame, no replay → **scrubbing backward needs no determinism**). The
  physics world is expensive, so it keeps its existing keyframe+replay
  `TimelineLog`. They're coupled by a **shared rendered-frame clock**:
  `world_frame_history` maps each rendered frame to the fixed frame the world
  ended at. The master clock is the **rendered frame** (every game has one, even
  with no physics hook).
- **Reload is a history boundary.** Snapshots can hold closures bound to the old
  module, so the rings reset on hot-reload (the live model is rebound; snapshots
  are not). "Rewind shows the earlier *code* version" (the hard frontier from the
  prior-art notes) is deferred — the interpreter *could* do it by keeping old
  modules alive, but that's future work.
- **`tts` is a game clock, not a wall clock.** Both shells own a shared
  `GameClock` (`functor_runtime_common::game_clock`) that produces each frame's
  `FrameTime`. Live, it ACCUMULATES the real frame delta (`game_time += dts`);
  paused (scrubber / debug `POST /time`), it FREEZES (`dts = 0`, `tts` held) so
  resuming continues from the freeze point instead of jumping forward by the
  paused wall-clock span; and it REBASES to a scrubbed frame's recorded `tts`
  when a time-travel branch resumes — on a resume-from-scrub, after a seek, and
  after `POST /rewind` — so `tts`-driven visuals (orbiting lights, `sin(tts)`
  motion) continue from the scrubbed scene time rather than snapping to "now".
  The shell reads the rebase target from `GameProducer::current_scene_tts`
  (the recorder's `current_scene_frame_tts`). `--fixed-time` / `?fixed-time` is
  an unconditional pin (every frame `{ dts: 0, tts: <const> }`) that bypasses
  accumulation, pause, and rebase — the deterministic golden-capture path.
- **Shared logic, platform-native UI.** The `SceneRecorder` (the hard part) is
  shared; the *UI surface* is per-platform: egui-in-canvas on desktop (no DOM
  there), **native DOM on web** (`functor_lang_scrub_*` wasm exports drive it). The web
  DOM scrubber sits *outside* the game canvas, so its widgets never fight the
  canvas's pointer-lock — a cleaner fit than mirroring desktop's egui onto web.

It builds directly on three existing threads and should be read alongside them:
`docs/physics.md` (the `Simulatable`/`Timeline` seam this generalizes),
`docs/llm-native-editor.md` (which already frames rewind as an *authoring*
primitive, not just a debugging one), and `docs/debug-runtime.md` (the
frame-clock control that already exists). The surface is **Functor Lang-only** — the
F#/Fable pipeline has been removed (`docs/functor-lang.md`).

Inspiration: the [Tomorrow Corporation tech
demo](https://www.youtube.com/watch?v=72y2EC5fkcE) (whole-program time travel as
a first-class part of the runtime) and Bret Victor's *Inventing on Principle*
(tweak a constant, see the consequence across time immediately).

## The core idea

**Today's rewind rewinds physics, not the game.** Phase 6's `SteppedPhysics`
recorder records and replays only the Rapier `World` through a
`TimelineLog<World>`. The MVU `model` — which in a Functor game *is* the game
state (score, AI, spawn generation, animation timers, UI state, everything
non-physics) — is never snapshotted. Scrub back in `examples/physics` today
and the crate *poses* move, but any model-resident state stays pinned at "now."
That is correct for a physics demo and insufficient for a whole-game scrubber.

"Generic tooling across the scene" therefore has a precise meaning: **rewind the
model too.** The good news is that the codebase already anticipated exactly this.
The `Simulatable` trait carries the comment *"Physics is the first impl; the
whole game model (serializable + input-driven) could be a second later,"* and the
entire `Timeline` / `TimelineLog` / hybrid-keyframe machinery is already generic
over `S: Simulatable`. The only coupling to physics is the single `impl
Simulatable for World` plus `SteppedPhysics` being hard-typed to it.

### Why Functor Lang makes this nearly free

The Functor Lang model is an `functor_lang::Value` that derives `Clone`, is `Rc`-shared, and is
cheap to clone. Snapshotting the entire model every frame is `model.clone()`
into a ring buffer — and because Functor Lang values are immutable and structurally
shared, adjacent frames share every unchanged sub-tree, so the memory cost is
close to "what changed this frame," not "the whole model, 900 times."

The F#/Fable path could never do this cleanly: its hot-reload state is a `Box<dyn
Any>` (`OpaqueState`) bound to one dylib generation, opaque to the runtime and
not clonable-as-data from the shell. The in-process interpreter is the enabling
fact — the shell *owns* the model value and can version it directly. Whole-game
rewind is one of the concrete payoffs of the Functor Lang pivot
(`docs/functor-lang.md`).

### One frame, one clock

The clean north-star framing is a single **frame-level `Simulatable`** whose
`Snapshot` is `(model, worldSnapshot)` and whose `Command` is the frame's input
events. Its `step` runs one full MVU frame: drain inputs → `update` → `tick` →
physics reconcile + fixed-step → subscriptions/read-back. The model's evolution
is not independent of physics (game code reads `Physics.position` in `update` /
`draw`), so they must advance and seek together under **one frame index**. Today
physics owns its own frame counter and the model has none — unifying that clock
is a required step (Phase 1 below).

Cadence is an implementation choice the existing `TimelineLog` already supports:
the model half is cheap (`Value` clone → could snapshot every frame), the world
half is expensive (serde-JSON of the whole Rapier world → keyframe every N +
input-log replay, exactly as physics does today). A seek restores the nearest
keyframe ≤ target and re-steps forward replaying recorded commands — the same
path a live frame takes, so live and replayed frames stay identical by
construction (the invariant physics already proves with goldens).

## What exists vs. what's missing

| Piece | Status |
| --- | --- |
| Generic `Timeline`/`Simulatable`/`TimelineLog` (keyframe + input-log hybrid, `truncate_from` branch, bounded history) | **Shipped**, already generic (`physics/timeline.rs`) |
| Physics world as a `Simulatable` | **Shipped** (`impl Simulatable for World`) |
| **Model as a `Simulatable`** (snapshot = `Value::clone`, command = frame inputs) | *Missing — the core piece* |
| **Unified frame clock** coupling model + world (seek both to frame N) | *Missing — physics owns its counter; model has none* |
| Whole-game frame-clock pause/step/resume | **Shipped** as debug server `POST /time` (pins `dts=0`) — desktop only |
| egui backend in both shells (real `Context` + `Painter`, v0.34) | **Shipped** (`ui.rs`, both runners) |
| **egui receiving pointer input / clicks** | *Missing — `RawInput` is empty, every element `.interactable(false)`* |
| **Mouse clicks reaching game/overlay at all** | *Missing — `MouseEvent` is only `MouseMove`/`MouseWheel`; button presses feed cursor-capture only* |
| Interactive UI (buttons with actions) | *Missing — `View` is `Empty/Text/Row/Column/Panel`, not parameterized over a message* |
| Web debug server / control channel | *Missing — but egui itself runs on web* |
| Serializable model (disk/wire) | *Missing (`Closure`/`HostData` aren't `Serialize`) — **not needed** for in-session rewind* |

Beyond the physics pieces in #215, the load-bearing gaps are exactly three:
**(1)** a `Simulatable` over the model, **(2)** a unified frame clock, **(3)**
pointer/click input plumbing. Everything else is already present or optional.

## Architecture: a shell-owned tool

**The scrubber is runtime-owned, not game-authored.** It is tempting to build the
overlay out of Functor's own UI primitives (dogfooding the interactive-UI
feature), but that couples shipping the tool to shipping a general UI-actions
capability. Instead:

- **The scrubber is a shell-side egui panel** — a timeline bar, transport
  (play/pause/step), a scrub handle, a frame counter — that drives the generic
  `Timeline` directly. It renders in **both** shells because egui already runs in
  both, needs **no game-facing API**, and requires no game to opt in. "Always
  exposed in the browser / VSCode plugin" comes for free: the VSCode live preview
  is just `functor run wasm` in a webview, so a shell-owned overlay in the web
  runtime *is* the VSCode overlay — one implementation, both surfaces.
- **Interactive Functor Lang UI (buttons with actions) is a separate feature.** Functor Lang
  closures are storable, so a `Button { label, onClick }` `View` variant is
  natural; it needs egui fed real pointer input and a return channel into
  `update`. It is independently valuable and the scrubber is a good *second*
  consumer to dogfood it — but the tool must not block on it.

The shared substrate both want is the *same one thing*: **wire pointer position +
click into the runtime and into egui.** Do it once (Phase 2) and the shell
scrubber lights up immediately; game-facing interactive UI becomes a follow-on
that reuses the same plumbing.

**Toggle:** `~` (tilde) opens/closes the overlay natively — the Quake-console
convention devs already expect; TAB stays free for in-game use. On web / VSCode
the overlay defaults to visible.

```
        ┌───────────── Functor Lang game (functional core) ─────────────┐
        │  init / update / tick / draw / physics / ui  (pure)   │
        └───────────────────────────┬───────────────────────────┘
                                     │  inputs (Command)  +  Value model
        ┌────────────────────────────▼──────────── imperative shell ─┐
        │  FrameTimeline : Simulatable                                │
        │    Snapshot = (Value model, World snapshot)                 │
        │    Command  = frame input events                            │
        │    step     = drain→update→tick→physics step→read-back      │
        │  TimelineLog<FrameTimeline>  (keyframe + input log, bounded) │
        │  one frame clock  ·  seek(N) restores model AND world       │
        │                                                             │
        │  egui scrubber overlay  ── drives ──►  TimelineControl      │
        │   (shell-owned; `~` toggle; default-on web/VSCode)          │
        └─────────────────────────────────────────────────────────────┘
            native: functor (in-process)  │   wasm: web-runtime (= VSCode preview)
```

## The determinism boundary

State it up front, because it decides how much of the tool needs the pure-replay
discipline:

- **Scrubbing backward needs no determinism.** Restoring a stored snapshot (model
  clone + world restore) is exact by construction. Plain pause + scrub-back works
  even for games that read the wall clock or do IO.
- **Replay-forward, branching, and the trajectory predictor need pure model
  evolution** — the model must be a function of `(prev model, inputs, physics
  read-back)` with no wall-clock, no unseeded RNG, no Http-dependent state. This
  is the same rule physics already enforces (`docs/physics.md` Determinism); the
  model must adopt it for those features.
- **Side-effecting effects during replay** (Http, Audio) must be suppressed or
  replayed-from-log, exactly as physics commands already are — a replayed frame
  must not re-fire a network request. Physics commands are already plain-data and
  recorded; the generic recorder extends the same treatment to the rest.
- **Replays are per-build, in-session.** Snapshots are live in-process values
  (`Value` + Rapier world), not serialized artifacts — fine for time-travel
  debugging and authoring, out of scope for persisting a timeline to disk or
  sending it over the wire. A data-native, serializable model (needed for that,
  and already flagged in `docs/debug-runtime.md` and `protocol.rs`) is a separate,
  larger effort and explicitly **not** required for anything here.

## The event log: one spine, two directions

The pieces above keep collapsing into each other because they are one structure:
a **frame-indexed event log** where expensive state is sampled as **keyframes**
and everything cheap is an **event stream**. Snapshots are just a keyframe
optimization on top of an event-sourced timeline — the physics `TimelineLog` is
*already* keyframe + command-log, and the model's snapshot ring (T1) is about to
grow an input log (T6); those aren't two subsystems that rhyme, they're the same
pattern on two payloads. The "unified frame clock" is really "one event log, two
keyframe tracks."

What goes on the log follows a single rule — **record only the non-reproducible
inbound; recompute everything reproducible; suppress everything outbound**:

| Class | Examples | Timeline treatment |
| --- | --- | --- |
| Reproducible inbound | timers / `Sub.every` (from the frame clock), **seeded** RNG | recompute — don't record |
| Non-reproducible inbound | user input, http/ws **responses**, wall clock, unseeded RNG | **record + replay** |
| Outbound | the effects a frame issued (http / ws / audio) | **suppress + log** (never re-fire) |

Seeding RNG is what moves it from the record column to the recompute column (the
seed becomes one more plain-data timeline entry). **Direction is the whole rule:**
inputs and effect *responses* are inbound and get replayed; effects are outbound
and get suppressed. Because the log is plain data it **survives a hot reload**,
even though the model snapshots (old-module closures) do not — which is exactly
what lets "tweak a constant, replay my recorded inputs, see if the jump now
clears the chasm" work.

Two fidelity tiers follow, and they are why coeffect replay (T8) is a separate
phase from the input log (T6):

- **Pure replay** (same code) reissues the identical effect stream, so recorded
  responses replay **positionally** — exact and cheap.
- **Replay under a tweak** (the point of T6) can diverge: the changed model may
  issue a different request, so a recorded response must be **matched by identity**
  (method+URL+body / channel+payload) and a miss is marked **diverged** on the
  timeline, not fired live — firing live would break replay purity and can
  double-charge / duplicate-send. Divergence-as-a-visible-marker is the right
  LLM-native behavior, not a cop-out.

## The authoring experiences it unlocks

These are the reason to build it — and both reduce to the same primitive: *you
can snapshot and deterministically step the whole game forward.*

### Forked timelines, overlaid

Snapshot a frame, then let play continue two ways and show both at once. Once a
frame snapshot is `(model, world)`, a fork is just *keeping* the old future
instead of `truncate_from`-ing it, holding two model+world states, and calling
the pure `draw` on each. The only new **engine** capability is a render pass that
composites a second scene at reduced opacity (~50%). Everything else is already
there: two models, one pure `draw`, one blend.

### Trajectory preview — forward-ghosting (Inventing on Principle)

Tweak a constant, see the whole future at once. Rather than a per-object trail,
this is **chronophotography**: from the paused snapshot, step the whole scene
forward over a window (start with ~2s / 10 divisions) and composite the divisions
into one image, each at `1/divisions` weight. Static geometry averages to itself
(solid); anything moving smears into a faint strobe of its own future positions.
It needs **no new scene-description API** — you call the existing `draw` on each
stepped state — and it captures *all* motion, not one hand-picked entity, so it
works for any scene under a fixed camera.

Two implementation points decide whether it looks right:

- **Composite in screen space, not the depth buffer.** Render each division to its
  own offscreen target (normal depth-testing *within* a division), then **average**
  the targets. Averaging is what makes static=solid / moving=faint fall out of the
  math — no per-object opacity logic, and it sidesteps the order-dependent garbage
  of alpha-blending N depth-tested scenes into one buffer. A **progressive
  running-average** (step one division per real frame, blend into an accumulator at
  `1/(k+1)`) keeps per-frame cost at ~1 extra render, needs only two targets, and
  *builds up* over the window — then holds until something changes. (The engine's
  double-buffered `RenderTargetBuffers` + a fullscreen composite quad, modelled on
  `draw_skybox`, is nearly all the machinery.)
- **Replay recorded inputs, freeze the camera.** Forward-stepping replays the
  frame-indexed input log (see "The event log") for frames it has, then coasts;
  all divisions render with the *paused* camera so only world motion smears, not
  the view.

The "tweak a constant" half rides existing hot-reload: swap the `.functor` with the
model preserved, re-run the window, the ghost redraws — so you can tweak a jump
impulse until the arc clears a chasm and *see* it clear before you resume. This is
the single most compelling demo in the set and it is within reach.

**Fork+overlay and forward-ghosting are the same engine primitive** — a
screen-space compositor that renders K scenes to K offscreen targets and averages
them with weights. Fork+overlay is K=2 at (0.5, 0.5) from two branches; ghosting
is K=N at `1/N` from one branch stepped forward. Build the compositor once and
both land; the old polyline/trail primitive becomes an optional later "precise
single-path read," not a prerequisite.

## LLM-native angle

The same machinery is an **authoring/observation primitive**, not just a player
toy — `docs/llm-native-editor.md` already makes this argument. Capture a session,
rewind to frame K, change game logic, replay, and diff the outcome is
*time-travel authoring / what-if iteration*; the golden-image tests are the
regression half of the same loop. Every phase below is buildable and verifiable
headlessly (pure Rust + Functor Lang, no GPU window) up to the point where pixels are
actually required (the overlay render and the screen-space compositor pass) — matching
the project's "design for agent verifiability" rule.

## Roadmap (small, stacked PRs)

| Phase | Scope | Status |
| --- | --- | --- |
| **T1. Coupled model+world recorder** | `History<T>` snapshot ring + `SceneRecorder`; per-frame model + physics-fixed-frame recording; `rewind_scene_to`/`seek_scene_to` exact-or-refused; live `POST /rewind`. Landed as a snapshot-ring + shared rendered-frame clock rather than a single frame `Simulatable` (see the design note). Headless integration tests. | **Shipped** (#218/#219/#222/#225) |
| **T2. Pointer/click input plumbing** | Feed real pointer `RawInput` to egui (desktop); DOM mouse for the web scrubber. `.interactable(false)` dropped for the scrubber panel. | **Shipped** (#226) |
| **T3. Scrubber overlay** | Draggable timeline (non-destructive scrub + branch-on-resume) + Pause/Step. **Desktop:** egui-in-canvas, `~` console toggle (hidden by default). **Web:** native DOM outside the canvas + "🖱 mouse look" button. | **Shipped** (#226) |
| **T4. Interactive Functor Lang `View`** | `View` gains an action-carrying node (`Button { label, onClick }`, storable Functor Lang closure); egui hit-tests and dispatches back into `update`. Independent of the scrubber (which is shell-owned) — a general game-UI capability. | Not started |
| **T5. Fork + overlay** | Keep-the-branch instead of truncate; hold two model+world states; a **screen-space compositor** (new fullscreen average pass at the tail of `render_frame`, reusing the double-buffered `RenderTargetBuffers`) renders each scene to its own target and averages them (K=2, weights 0.5/0.5). Shares its whole implementation with T6. | Not started |
| **T6. Forward-ghosting (trajectory preview)** | A frame-indexed **input log** in the recorder (plain data, survives reload) + a headless deterministic forward-step (replay inputs, suppress effects) + the T5 compositor at K=N, `1/N` weights; wire to hot-reload for the tweak-a-constant loop; slider once T4 lands. The *Inventing on Principle* demo — no trail primitive needed. | Not started |
| **T7. Event timeline** | Record inputs *and* effects as one plain-data event log keyed by frame (inputs in / effects out); render them as markers on the scrubber; use the log to suppress-on-replay. The event-sourcing spine the earlier phases already imply. | Not started |
| **T8. Coeffect record/replay** | Record & replay effect *responses* (http/ws) so a recorded window re-runs faithfully. Pure replay is positional/exact; replay-under-a-tweak needs identity-matching + a visible **divergence marker** when a changed model asks for an un-recorded response (never fire live). | Later |

T1–T3 (the whole-game scrubber) are shipped. T5–T6 are the showstopper authoring
experiences — both now reachable because `SceneRecorder` can snapshot and
deterministically step the whole scene, and the renderer already has the FBO
machinery a compositor needs. T7–T8 generalize the timeline into an event log
(and make cross-tweak outcome-diffing — the LLM-native regression loop —
possible). The other recurring dependency, behind schema-migration across a
reload and richer `/state`, is a **structured / serializable / versioned model
state** (today the model isn't `Serialize`; `/state` is `Debug` text) — the
highest-leverage non-visual foundation.

## Prior art

- **Whole-program time travel**: the [Tomorrow Corporation tech
  demo](https://www.youtube.com/watch?v=72y2EC5fkcE) — time travel, live editing,
  and inspection as first-class runtime features, the north star for this doc.
- **Reactive authoring**: Bret Victor, *Inventing on Principle* (2012) — editing a
  value and seeing its effect across time immediately; the trajectory-preview
  target.
- **Deterministic rollback** (the engine lineage this reuses): GGPO / `ggrs` /
  `bevy_ggrs`, and the `Simulatable`/`Timeline` seam in `docs/physics.md` — the
  same restore-and-replay machinery, here generalized from the physics world to
  the whole frame.
- **Immediate-mode debug UI**: egui (already integrated in both shells) — the
  scrubber's rendering substrate; the work is feeding it input, not adopting it.
