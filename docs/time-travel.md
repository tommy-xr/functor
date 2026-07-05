# Time-travel tooling

Status: **design / vision** — the enabling machinery is shipped (the physics
`Timeline`, `docs/physics.md` Phase 6 / #215), but the generic whole-scene tool
is not yet built. This is the design doc and the stacked-PR plan for turning
that machinery into **generic time-travel tooling across the whole game**: pause,
scrub, rewind, replay, branch — and the authoring experiences those unlock.

It builds directly on three existing threads and should be read alongside them:
`docs/physics.md` (the `Simulatable`/`Timeline` seam this generalizes),
`docs/llm-native-editor.md` (which already frames rewind as an *authoring*
primitive, not just a debugging one), and `docs/debug-runtime.md` (the
frame-clock control that already exists). The surface is **MLE-first** — F# is
no longer a target (`docs/language-direction.md`).

Inspiration: the [Tomorrow Corporation tech
demo](https://www.youtube.com/watch?v=72y2EC5fkcE) (whole-program time travel as
a first-class part of the runtime) and Bret Victor's *Inventing on Principle*
(tweak a constant, see the consequence across time immediately).

## The core idea

**Today's rewind rewinds physics, not the game.** Phase 6's `SteppedPhysics`
recorder records and replays only the Rapier `World` through a
`TimelineLog<World>`. The MVU `model` — which in a Functor game *is* the game
state (score, AI, spawn generation, animation timers, UI state, everything
non-physics) — is never snapshotted. Scrub back in `examples/mle-physics` today
and the crate *poses* move, but any model-resident state stays pinned at "now."
That is correct for a physics demo and insufficient for a whole-game scrubber.

"Generic tooling across the scene" therefore has a precise meaning: **rewind the
model too.** The good news is that the codebase already anticipated exactly this.
The `Simulatable` trait carries the comment *"Physics is the first impl; the
whole game model (serializable + input-driven) could be a second later,"* and the
entire `Timeline` / `TimelineLog` / hybrid-keyframe machinery is already generic
over `S: Simulatable`. The only coupling to physics is the single `impl
Simulatable for World` plus `SteppedPhysics` being hard-typed to it.

### Why MLE makes this nearly free

The MLE model is an `mle::Value` that derives `Clone`, is `Rc`-shared, and is
cheap to clone. Snapshotting the entire model every frame is `model.clone()`
into a ring buffer — and because MLE values are immutable and structurally
shared, adjacent frames share every unchanged sub-tree, so the memory cost is
close to "what changed this frame," not "the whole model, 900 times."

The F#/Fable path could never do this cleanly: its hot-reload state is a `Box<dyn
Any>` (`OpaqueState`) bound to one dylib generation, opaque to the runtime and
not clonable-as-data from the shell. The in-process interpreter is the enabling
fact — the shell *owns* the model value and can version it directly. Whole-game
rewind is one of the concrete payoffs of the MLE pivot
(`docs/language-direction.md`).

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
- **Interactive MLE UI (buttons with actions) is a separate feature.** MLE
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
        ┌───────────── MLE game (functional core) ─────────────┐
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
            native: functor-runner        │   wasm: web-runtime (= VSCode preview)
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

## The authoring experiences it unlocks

These are the reason to build it — and both reduce to the same primitive: *you
can snapshot and deterministically step the whole game forward.*

### Forked timelines, overlaid

Snapshot a frame, then let play continue two ways and show both at once. Once a
frame snapshot is `(model, world)`, a fork is just *keeping* the old future
instead of `truncate_from`-ing it, holding two model+world states, and calling
the pure `draw` on each. The only new **engine** capability is a render pass that
composites a second scene at reduced opacity (~50%). Everything else is already
there: two models, one pure `draw3d`, one blend.

### Trajectory preview (Inventing on Principle)

Tweak a constant, see the ball's whole path over time. This is "run the future
headlessly and plot it": from the current snapshot, step the model+world forward
N frames deterministically, sample a position each frame, and draw the path as a
polyline trail. It needs **(a)** deterministic forward simulation — the replay
machinery from Phase 1 — and **(b)** a polyline/trail draw primitive (the physics
`debug_lines` pass is already most of it). The "tweak a constant" half rides
existing hot-reload: `POST /reload-source` swaps the `.mle` with the model
preserved, the future is re-run, the trail redraws. Once interactive UI exists, a
slider makes it continuous — the full Bret Victor loop. This is the single most
compelling demo in the set and it is within reach.

## LLM-native angle

The same machinery is an **authoring/observation primitive**, not just a player
toy — `docs/llm-native-editor.md` already makes this argument. Capture a session,
rewind to frame K, change game logic, replay, and diff the outcome is
*time-travel authoring / what-if iteration*; the golden-image tests are the
regression half of the same loop. Every phase below is buildable and verifiable
headlessly (pure Rust + MLE, no GPU window) up to the point where pixels are
actually required (the overlay render and the opacity/trail passes) — matching
the project's "design for agent verifiability" rule.

## Roadmap (small, stacked PRs)

| Phase | Scope | Targets |
| --- | --- | --- |
| **T1. Model `Simulatable` + unified clock** | A frame-level `Simulatable` (snapshot = `Value` clone + world snapshot; command = frame inputs) and one frame index that seeks model and world together. Goldens: scrub-back restores the model value-exact; replay-forward is identical (mirrors the physics goldens). No UI. | native+wasm (Rust+MLE) |
| **T2. Pointer/click input plumbing** | Deliver mouse-button events to the runtime, feed real `RawInput` to egui, drop `.interactable(false)`. Unblocks both the scrubber and interactive game UI. | native+wasm |
| **T3. Shell-owned scrubber overlay** | egui timeline bar + transport (play/pause/step) + scrub handle driving the generic `Timeline`; `~` toggle (native), default-on (web/VSCode). This is the Tomorrow-Corporation whole-game scrubber. | native+wasm |
| **T4. Interactive MLE `View`** | `View` gains an action-carrying node (`Button { label, onClick }`, storable MLE closure); egui hit-tests and dispatches back into `update`. Separable, independently useful; the scrubber can be re-skinned in it later as a dogfood. | native+wasm (MLE) |
| **T5. Fork + overlay** | Keep-the-branch instead of truncate; hold two model+world states; renderer composite pass drawing a second `draw` output at ~50% opacity. | both |
| **T6. Trajectory preview** | Deterministic forward-sim of N frames + a polyline/trail draw primitive; wire to hot-reload for the tweak-a-constant loop; slider once T4 lands. The *Inventing on Principle* demo. | both |

T1–T3 deliver the whole-game scrubber. T5–T6 are the showstopper authoring
experiences.

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
