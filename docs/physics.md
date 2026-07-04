# Physics design

Status: **active** (design; no code yet). This is the design doc and roadmap for
physics in Functor. It builds on the same seams as `docs/multiplayer.md` (effect
queue, subs, the `functor-netsim` deterministic harness) and supersedes the
`Physics` stub in `docs/todo.md`.

## Goal

Add **rigid-body physics** to Functor in a way that fits the functional-core /
imperative-shell architecture, backed by **Rapier3d**. Three hard requirements
shape every decision:

1. **Functional surface.** Physics is described, not commanded: a pure
   `physicsScape : model -> PhysicsScene` declares the bodies that *should* exist,
   reconciled against a live world each frame — the same shape as `draw3d`
   (`Scene3D`) and `soundScape` (`AudioScene`).
2. **Local determinism.** Same binary, same inputs → same simulation,
   byte-for-byte — so we can rewind, replay, and verify in the deterministic
   netsim. *Local* (single-binary) determinism is sufficient for every scenario
   we target — replay, time-travel debugging, and both netcode modes (there is
   only single ownership at any time) — and Rapier provides it with default
   features. Cross-platform determinism is a **non-goal** (see Determinism).
   This still forces a fixed timestep and deterministic body ordering from day
   one.
3. **Rewindable + networkable.** Pause / rewind / replay locally, and — the north
   star — client-side prediction + server reconciliation for the multiplayer
   target (`docs/multiplayer.md`'s netcode epic). Both are the *same machinery*:
   restore an earlier state, re-simulate forward with recorded inputs.

Like rendering and audio, physics must be **drivable and observable headlessly**
(no GPU window) — it runs CPU-side in the shell and dumps to text/JSON.

## Design constraints (from the architecture)

- **Physics-as-shell, model-as-truth.** A Rapier world is a large mutable bag of
  solver state; it is *not* stored in the F# model. It lives runtime-side in
  `functor-runtime-common` as a cache/accelerator — exactly like the renderer and
  the audio voice registry. The model holds plain, serializable data; the live
  world is reconstructible from a snapshot or an input replay.
- **The effect queue is no longer persisted across hot reload** (see
  `multiplayer.md` / `effects-plain-data-invariant`). So physics *commands*
  (impulse/force/teleport — plain data) and *queries* (raycast/shapecast, which
  carry a `tagger` closure) both work the way HTTP does: the tagger is held in a
  thread-local, dylib-bound registry keyed by token, and an in-flight query loses
  its tagger across a reload (dropped with a warning — a dev-only trade).
- **Subs are recomputed each frame and not persisted.** Collision/contact events
  and the per-frame step read-back are delivered as `Sub`s, matched across
  recomputations by their decoder identity.
- **Fixed timestep, always.** The game `tick` receives a variable `FrameTime.dts`.
  Physics must **never** step Rapier with variable dt (nondeterministic +
  unstable). The shell accumulates real dt and steps the world in fixed
  substeps, carrying the remainder.

## Architecture

```
        ┌──────────────────────── F# functional core ────────────────────────┐
        │  physicsScape : model -> PhysicsScene   (WHAT bodies should exist)   │
        │  update/tick  -> effect                 (impulse/force/teleport,     │
        │                                          raycast tagger, rewindTo)   │
        │  subscriptions-> Sub                     (collisions, step read-back)│
        │  draw3d : model -> DrawContext -> Frame  (C: ctx.physics view query) │
        └─────────────────────────────────┬───────────────────────────────────┘
                                           │  (thin Emit shims, JSON over boundary)
        ┌──────────────────────────────────▼──────────────── imperative shell ─┐
        │  WorldRegistry      — WorldId -> live Rapier world (singleton = id 0)  │
        │  reconcile()        — diff PhysicsScene vs live bodies, keyed by tag   │
        │  fixed-step driver  — accumulator; step(dt, cmds) -> events            │
        │  Timeline (trait)   — KeyframeLog | SnapshotRing | ReplayOnly          │
        │  Simulatable (trait)— snapshot / restore / step  (Rapier serde)        │
        └───────────────────────────────────────────────────────────────────────┘
            native: functor-runner            │   wasm: web-runtime bundle
```

- **`physicsScape` → `reconcile`** is `audio::reconcile` with a feedback edge:
  spawn new tags, despawn gone tags, update changed declarations. Deterministic
  order (sort by tag), not hash-map order.
- **Read-back is "C with A"** (below): `draw3d` queries the stepped world via an
  explicit `Physics.View`; a `Physics.synced` sub is the opt-in path to fold
  specific tags back into the model when gameplay needs them.
- **The `Simulatable` / `Timeline` seam** keeps the rewind strategy swappable and
  is the same machinery client prediction uses.

## Surface: MLE-first

**The game-facing surface lands in MLE, not F#** (decided 2026-07-03; see
`docs/language-direction.md` — Functor is investing in MLE as the game-logic
layer). The design below is unchanged — declarative scene, divergence rule,
commands, subs — but the shipping vocabulary is the MLE prelude
(`functor_runtime_common::mle_prelude`, documented in the `mle-language`
skill):

```mle
let physics = (model) =>                       // OPTIONAL game hook
  Physics.scene(0.0, -9.81, 0.0, [
    Physics.fixed("ground", Physics.box(20.0, 0.2, 20.0)),
    Physics.dynamic("crate", Physics.box(1.0, 1.0, 1.0)) |> Physics.at(0.0, 5.0, 0.0),
  ])

let draw = (model, tts) =>
  Frame.create(camera,
    Scene.cube() |> Scene.lit(0.8, 0.5, 0.2) |> Physics.transformed("crate"))
```

MLE dissolves the read-back boundary problem outright: the interpreter runs in
the **shell's own process**, sharing the crate statics that hold the physics
world — so `Physics.position(tag)` / `Physics.transformed(scene, tag)` are
direct reads of the live stepped world (frame order: `tick` → `physics`
reconcile+step → `draw`). The dylib producers could never have this: the game
dylib links its own copy of `functor_runtime_common`, so its statics are not
the shell's, and a view would have to cross as per-frame data. The F# sketch
below is retained as the reference design (types/semantics match the Rust
engine one-for-one); an F# surface would need that data-crossing `DrawContext`
plumbing and is deferred indefinitely.

Read semantics worth knowing: physics reads of a missing tag are **loud
spanned errors** — games read only tags their `physics` hook declares. (An
Option-shaped variant return is possible now that MLE has `match`; loud
remains the default because a missing declared body is a bug, not a case.)

Capture gotcha: `--fixed-time` pins the clock with `dts = 0`, so physics never
steps under it — a physics golden captures a *settled* scene via plain
`--capture-time` (rest poses are reproducible) rather than the fixed-time path.

## API (F# — reference design, deferred)

**Per-frame draw context.** `draw3d` takes a `DrawContext` record (not a bare
`FrameTime`), destructured at the call site. This is the per-frame *extension
point*: physics arrives as a field, and future per-frame reads (the backlog's
polling `Input.State`, etc.) slot in as new fields with no further signature
churn. The view is a live handle, not serializable, so it rides in the context
record assembled at the draw call site — not inside `FrameTime`, which is
marshalled through `JsValue` on wasm.

```fsharp
type DrawContext = {
    time:    Time.FrameTime
    physics: Physics.View      // read handle to the frame's stepped world (world 0)
}

draw3d : ('model -> DrawContext -> Graphics.Frame) -> Game<'model,'msg> -> Game<'model,'msg>
```

**Description — `physicsScape`** (new hook on the `Game` record / `GameBuilder` /
`GameRunner`, mirroring `soundScape`; supersedes the `physics : model ->
PhysicsScene` stub in `todo.md`):

```fsharp
[<Erase; Emit("functor_runtime_common::physics::Body")>]         type Body = | Noop
[<Erase; Emit("functor_runtime_common::physics::PhysicsScene")>] type PhysicsScene = | Noop

module Body =
    let dynamic   (tag: string) (shape: Shape) : Body   // simulated (Local authority)
    let kinematic (tag: string) (shape: Shape) : Body   // position-driven (great for Remote)
    let fixedBody (tag: string) (shape: Shape) : Body   // static ('fixed' — 'static' is reserved)
    let at        (pos: Vector3)    (b: Body) : Body
    let facing    (rot: Quaternion) (b: Body) : Body
    let velocity  (v: Vector3)      (b: Body) : Body
    let mass / friction / restitution / sensor : ... -> Body -> Body
    let authority (a: Authority)    (b: Body) : Body    // Local | Remote of source

module PhysicsScene =
    let create (gravity: Vector3) (bodies: Body[]) : PhysicsScene
    let empty  () : PhysicsScene
```

**Commands — plain-data effects** (operate on the default singleton world; later
overloads take an explicit world):

```fsharp
module Physics =
    let applyImpulse (tag: string) (impulse: Vector3) : effect<'msg>
    let applyForce   (tag: string) (force: Vector3)   : effect<'msg>
    let setVelocity  (tag: string) (v: Vector3)       : effect<'msg>
    let teleport     (tag: string) (pos: Vector3)     : effect<'msg>
```

**Queries — async tagger** (the `Effect.httpGet` shape: token-keyed registry,
result delivered as a message next drain):

```fsharp
    let raycast   (origin: Vector3) (dir: Vector3) (maxDist: float32)
                  (tagger: RayHit option -> 'msg) : effect<'msg>
    let shapeCast (shape: Shape) (origin: Vector3) (dir: Vector3) (maxDist: float32)
                  (tagger: ShapeHit option -> 'msg) : effect<'msg>
```

**Events + read-back — subs** (`Sub<'msg>` DU variants, drained in the executor
like net events / audio completions):

```fsharp
    let events  (decode: PhysicsEvent -> 'msg) : Sub<'msg>  // CollisionStarted/Ended, Sensor, ContactForce
    let synced  (decode: BodyState[]   -> 'msg) : Sub<'msg>  // per-frame read-back: tag -> transform/velocity
```

**Timeline controls — effects** (drive the runtime-side history; see Rewind):

```fsharp
    let pause   : effect<'msg>
    let resume  : effect<'msg>
    let stepOnce: effect<'msg>            // advance exactly one fixed frame while paused
    let rewindTo (frame: int) : effect<'msg>
```

### Singleton now, explicit worlds later — for free

The `physicsScape`-driven world is **world 0** in a `WorldId`-keyed registry.
Every reconcile/step/query/effect in Rust is world-parameterized from day one; the
F# functions above just default the world argument. Adding an explicit
`PhysicsWorld.t` value with its own `step`/`sub`/`effect` later requires **no
engine refactor** — the singleton calls are literally `PhysicsWorld.applyImpulse
world0 …` with `world0` filled in.

```fsharp
// later, with zero engine change:
let sandbox = PhysicsWorld.create gravity
PhysicsWorld.step dt sandbox
PhysicsWorld.applyImpulse sandbox "x" impulse
```

## Read-back: "C with A"

The physics world produces transforms every step; `draw3d` needs them to render.
We use **C** (query the world) with **A** (sync into model) as an opt-in escape
hatch:

```fsharp
// C — ergonomic query at draw time, no model boilerplate, explicit (not ambient).
//     Destructure exactly what the frame needs out of the context record:
draw3d (fun model { time = _; physics = phys } ->
    let t = Physics.View.transform phys "crate-3"     // reads the just-stepped world
    Frame.create camera (crateMesh |> Transform.apply t))

// A — opt-in: fold specific tags back into the model when logic needs them
//     (AI, triggers, scoring, hot-reload persistence)
Physics.synced (fun states -> PhysicsTick states)     // Sub<'msg>
```

The `Physics.View` is a cheap read handle to the frame's stepped snapshot, reached
through the explicit `DrawContext` argument so rewind and netsim still treat
**model + snapshot** as the whole truth (it is not ambient global state). Changing
`draw3d` to take the context record is a small migration across the existing
examples (`hello`, `mpserver`, `mpclient`) — they destructure `{ time }` and are
otherwise untouched.

### Authority + divergence

`dynamic` bodies (`Local` authority) integrate freely; the model reads them.
`kinematic` / `fixedBody` / `Remote` bodies are driven *from* the declared state
(`set_next_kinematic_translation`). Per tag, `reconcile` stores the last-declared
value:

- **declared value changed since last frame** → game code set it (spawn, teleport,
  authoritative correction) → write it into the body.
- **unchanged** → leave the body alone; physics integrates; `synced` feeds the
  result back.

Because the model field is normally updated *from* physics output, declared ==
last-output in steady state (no spurious overrides). A teleport of a *dynamic*
body is just `physicsScape` declaring a position physics didn't produce →
divergence → overwrite. This generalizes citadel-xr's `mass == 0` heuristic to
allow correcting dynamic bodies — which is exactly what netcode reconciliation
needs.

## Entity lifecycle (model-layer abstraction)

`physicsScape`'s reconcile-by-tag is right for **identity-bearing** bodies the
model reasons about individually (player, doors, crates). It is clumsy for
**high-churn** bodies (bullets, debris, particles): the model would hand-enumerate
hundreds of them and carry per-instance string-tag bookkeeping.

The fix is a **pure, model-resident collection** — citadel-xr's `EntityManager`
rebuilt as a *value that projects into physics and rendering*, instead of a
stateful object that syncs them. It lives in your model, so model-as-truth holds,
rewind/snapshots are automatic, and it needs **no engine hooks** — only the public
physics/graphics primitives. (The "C" read-back already removes the worst of the
old EntityManager's mess: entities don't store transforms; `draw3d` reads them
live from the `Physics.View`.)

An **archetype** is the per-kind bundle — the EntityManager's entity-definition as
plain data + pure functions:

```fsharp
type Archetype<'e> = {
    shape:  Shape
    body:   BodyKind
    visual: 'e -> Transform -> Scene3D     // pure; transform comes from the view
    until:  Despawn list                   // OnCollision | AfterSeconds | BelowY | WhenSleeping
}

type Entities<'e>   // Map<EntityId,'e> + a deterministic id counter (in the model)
module Entities =
    val spawn   : 'e -> Entities<'e> -> Entities<'e> * EntityId
    val despawn : EntityId -> Entities<'e> -> Entities<'e>
    val update  : (EntityId -> 'e -> 'e option) -> Entities<'e> -> Entities<'e>   // None = reap
    val toBodies  : Archetype<'e> -> Entities<'e> -> Body[]
    val toScene3d : Archetype<'e> -> Physics.View -> Entities<'e> -> Scene3D       // instanced
```

`physicsScape` and `draw3d` just project the collection; deterministic ids come
from the counter *inside* `Entities`, so replay reproduces them with zero engine
support.

### Replication is structural: separate collections, not a per-entity flag

The replication boundary is cleanest as **structure in the model** — separate
entity collections by role — rather than a flag on each archetype:

```fsharp
type Model = {
    server: Entities<GameplayEntity>   // the replicated, authoritative world
    client: Entities<Cosmetic>         // this client's local-only entities
    // ... player, camera, etc.
}
```

- **`server`** — the shared authoritative world. On the server it's owned and
  simulated; on a client it's a predicted replica, reconciled against snapshots.
- **`client`** — local-only (cosmetic debris, prediction-only UI, debug markers).
  Never sent over the wire — but still in the model, so still in the local Timeline.

Two independent axes fall out of *which collection an entity lives in*, with no
per-archetype flag:

- **Replication** = structural: `server` replicates, `client` does not.
- **Local timeline** = always in — the whole model is snapshotted locally, so a
  client rewind/replay shows cosmetic debris too (exactly what you want for
  debugging). Cosmetics are excluded from *the network*, never from *history*.

This supersedes both the per-archetype `replication` flag and the earlier
"engine-owned fx world": everything is model-resident, and the boundary is a field.

### Snapshot partition + reconciliation — now trivial

Because the partition is structural, the snapshot and reconciliation are just field
operations:

- **Local Timeline snapshot** = the whole model (`server` + `client`).
- **Network snapshot** = the `server` collection only.
- **Reconciliation** = `{ model with server = authoritativeSnapshot }`, then resim
  — `client` is untouched by construction. A correction can't wipe cosmetics, and
  cosmetics can't desync (they were never authoritative).

The model's *type* now documents the netcode topology — you can read off what's
authoritative vs. local from the field list. (Split ownership — a client that
*owns* some replicated entities, like its own ball — is the one wrinkle: those live
in the replicated collection but carry `Authority = Local`, so this instance
simulates and broadcasts them while peers render them kinematic. `server`/`client`
are just the two common collections; a model can carry as many as its netcode needs
— Functor imposes none.)

### Committed vs. recommended

- **Committed engine primitives** (what makes the model-resident collection cheap):
  **instanced rendering** (`Scene3D.instances mesh transforms` → one draw call),
  **reconcile bail-out + tag interning** (steady-state diff is near-free), and the
  **`Physics.events` sub** (despawn-on-collision).
- **`Entities` is a recommended pattern, not a mandate.** It ships in
  `Functor.Game` as the default, but a game can swap in its own entity model
  (ECS-ish, hierarchical) on the same primitives — Functor doesn't impose one.

## Determinism

**The requirement is local (single-binary) determinism, and Rapier provides it
with default features**: the same simulation, on the same machine, with the same
Rapier and rustc versions, produces bit-identical results
([Rapier docs](https://rapier.rs/docs/user_guides/rust/determinism/)). This tier
covers everything we're building:

- **Rewind / replay / time-travel debugging** — re-simulation happens in the same
  process that recorded the history.
- **Mode A (server-authoritative + prediction)** — the client replays *its own*
  inputs on *its own* machine, starting from a server snapshot that arrives *as
  data*; it never has to bit-match the server's simulation. Float divergence
  between client and server just means slightly larger corrections, which
  converge.
- **Mode B (split ownership, state-sync)** — each entity is simulated by exactly
  one owner, who broadcasts its state; peers render it kinematically. Nobody
  re-simulates anyone else's entities, so nothing needs to match across machines.

**Cross-platform determinism (native ↔ wasm, x86 ↔ ARM) is a non-goal.** It is
only needed for *input-only lockstep* — everyone simulates everything from
inputs, no state on the wire — a mode we don't plan to build. Rapier's
`enhanced-determinism` flag provides it, but it is mutually exclusive with
`simd-stable` / `simd-nightly` / `parallel`, so it trades solver performance for
a guarantee we don't need. It stays documented here as the escape hatch if
lockstep ever becomes real, with fixed-point as the further fallback if
cross-target goldens won't converge (Photon Quantum's path). One consequence
worth naming: recording a repro on desktop and replaying it in the browser is
out of scope — a replay is bound to the binary that recorded it.

What local determinism requires from us (the fine print):

- **Fixed-step accumulator** in the shell (never variable dt), fed **only** by
  the harness's `FrameTime.dts` — physics has no clock of its own. This is the
  shock2quest lesson (tommy-xr/shock2quest#298: debug-paused game, physics kept
  integrating on wall-clock): here the debug server's `POST /time` pause/step
  controls physics for free because pausing pins `dts = 0` and the accumulator
  consumes nothing. Verified empirically: a paused scene is byte-identical
  across wall-clock time; `advance` steps it exactly.
- Rapier feature **`serde-serialize`** (snapshots); otherwise **default
  features** — no `enhanced-determinism`. (If we later enable `parallel`, first
  verify it is deterministic run-to-run on one machine — the Phase 1 golden
  catches this.)
- **Deterministic reconcile order across the whole world history, removals
  included.** Rapier arena handles depend on the full insert/*remove* sequence,
  not just the final set of bodies — so the reconcile diff (sort by tag) must be
  fully deterministic for despawns too. Snapshot-based seeks (`KeyframeLog`,
  `SnapshotRing`) are safe by construction (serde restores the arenas exactly);
  `ReplayOnly` re-executes the history and leans on this.
- **Replays are valid per-build only.** Fine for time-travel debugging (same
  session) and netcode (ephemeral); don't persist replays as long-lived
  artifacts. Hot-reloading the *game dylib* is safe — Rapier lives in the
  runtime shell, which is unchanged — but rebuilding the *runtime* invalidates
  recorded history.
- No wall-clock / unseeded RNG leakage (netsim already uses a seeded SplitMix64).
- **Validate with goldens; never assume** (Phase 1).

## Rewind: the `Simulatable` / `Timeline` seam

The **command/input log is the invariant** — server-authoritative prediction needs
it regardless. The only thing that varies between rewind strategies is **snapshot
cadence** and therefore how `seek` reconstructs a frame. So the design collapses to
two small traits, runtime-side:

```rust
/// Anything rewindable. Physics is the first impl; the whole game model
/// (serializable + input-driven) could be a second later.
trait Simulatable {
    type Snapshot;   // full serializable state (Rapier serde blob)
    type Command;    // per-frame inputs: impulses, spawns, declared-scene delta
    type Event;
    fn snapshot(&self) -> Self::Snapshot;
    fn restore(&mut self, s: &Self::Snapshot);
    // The timestep is a fixed property of the sim (FIXED_DT), not a parameter —
    // variable dt can't sneak in through this seam.
    fn step(&mut self, cmds: &[Self::Command]) -> Vec<Self::Event>;
}

/// The SWAPPABLE part. The sim loop and netcode name only this trait.
trait Timeline<S: Simulatable> {
    fn record(&mut self, frame: Frame, sim: &S, cmds: &[S::Command]);
    fn seek(&mut self, frame: Frame, sim: &mut S);          // restore exactly `frame`
    fn commands_since(&self, frame: Frame) -> &[Vec<S::Command>];
    fn prune(&mut self, before: Frame);                     // memory bound / server-confirmed
}
```

The three strategies turned out to differ in *snapshot cadence only*, so they
are one impl (`TimelineLog`, in `physics/timeline.rs`) with three constructors
rather than three types:

- **`TimelineLog::keyframes(n)`** (default, the hybrid) — snapshot every N
  frames + always log commands; `seek` restores nearest keyframe ≤ frame then
  `step`s forward. Bounded memory *and* seek.
- **`TimelineLog::snapshot_ring()`** — full snapshot every frame; `seek` is one
  restore. O(1) seek, heavy memory. (The oracle in the strategy-equivalence
  golden.)
- **`TimelineLog::replay_only()`** — one snapshot at the first frame; `seek`
  restores it and replays 0→frame. Lightest memory, leans hardest on
  determinism. (`prune` is a documented no-op — the base snapshot is the only
  restore point.)

Reconciliation is written **once, against the trait**, and never changes when the
strategy is swapped:

```rust
fn reconcile<S, T: Timeline<S>>(tl: &mut T, sim: &mut S,
                                k: Frame, authoritative: &S::Snapshot, now: Frame) {
    sim.restore(authoritative);          // server truth at frame K
    tl.overwrite(k, authoritative);      // correct recorded history at K (lands in 7b)
    for cmds in tl.commands_since(k) {
        sim.step(cmds);                  // replay OUR local inputs K+1..now
    }
}
```

The trait contract — `seek(K)` equals restoring a valid earlier state and stepping
forward with recorded commands — *is* the determinism invariant the netcode rests
on. The F# surface stays thin (`Physics.rewindTo`, `pause`, `resume`, `stepOnce`);
strategy choice is runtime config, defaulting to `keyframes(n)`. Two pieces are
deliberately deferred to their consuming phases: `overwrite` (7b, server history
correction) and truncate-on-record-after-seek (Phase 6, rewind-then-*branch* —
until then a seek is resumed by replaying `commands_since`, not re-recording).

## Netcode (server-authoritative first)

`Authority = Local | Remote of source` covers both target modes:

- **Mode A — server-authoritative + client prediction** (build first; needs only
  single-binary replay determinism). Server declares gameplay bodies `Local`;
  client declares its own player `Local` (predicted) and everything else `Remote`
  (kinematic, interpolated from snapshots). On an authoritative snapshot for frame
  K, `reconcile` restores K and replays stored local inputs K→now.
- **Mode B — split ownership** (deferred). Each entity's `Authority` decides who
  simulates it; peers render others as kinematic, driven from broadcast state.
  Owner-is-truth, so this needs **no cross-machine determinism at all** — see
  Determinism.

**Authority boundaries have a consistency problem that no determinism tier
solves:** when two differently-owned dynamic bodies collide (my ball hits your
ball), each owner resolves the contact seeing the other body as kinematic
(infinite mass), and the two outcomes can disagree physically. This is resolved
by design — ownership handoff on contact, or routing contested interactions
through the server — not by determinism. Deferring Mode B defers this too, but
any split-ownership scene (including the `mpserver`/`mpclient` demo below) hits
it as soon as owned bodies can touch.

**Target topology (networked VR): client-owned player movement, server-owned
everything else.** Each client is authoritative over its own player pose —
head/hands are tracked input; there is nothing sensible for a server to
"correct" — declared `Local` and broadcast as state; peers render it
`Remote`/kinematic. Every other physics body (props, projectiles, grabbed
objects) is server-owned, so all contested interactions resolve in one place
(the Source model — see Prior art). This is a small, fixed instance of the
authority machinery above: pure state-sync, no cross-machine determinism, and
the boundary problem reduces to player-touches-prop, which the server
arbitrates (the player body is kinematic to the server's world, so props can't
push the player — the usual VR choice).

## Culmination: pause / rewind / replay via keyboard

The first user-visible payoff, and the proof the `Timeline` works. Input is
event-only today (no polling snapshot), and `Input.Key` already has
`Space`/`Left`/`Right`/`P`/`R`, so controls map onto `KeyDown` edges — the same
scheme `netsim_viz` already uses (Space = pause, Right = step):

- **Space** → `Physics.pause` / `Physics.resume` (toggle).
- **Left / Right** (while paused) → `Physics.rewindTo (frame ∓ 1)` /
  `Physics.stepOnce`. Scrub the world; `draw3d` reads the rewound state via the
  `Physics.View`, so the scene visibly moves backward/forward.
- **R** → rewind to frame 0 and `resume` — deterministic **replay** (no new input)
  re-runs the identical simulation; applying a fresh impulse instead **branches**
  it, demonstrating determinism live.

An `examples/hello-physics` scene (a few `dynamic` boxes settling on a `fixedBody`
plane) drives this. The egui overlay shows **read-only status** — current frame,
paused/live, timeline strategy, history depth — since egui input isn't wired yet;
all control is via the keyboard, exactly as scoped.

## Debug visualization (wireframes via Rapier's debug renderer)

**Shipped (Phase 2b).** Rapier ships its own debug renderer behind the
**`debug-render`** feature: `DebugRenderPipeline` walks the live world and
emits colored line segments — collider wireframes, contacts, joints,
rigid-body frames — through a one-method `DebugRenderBackend` trait. We adopt
it rather than writing our own:

- **Engine side**: `World::debug_lines() -> Vec<DebugLine>` (a tiny backend
  impl collecting segments into plain RGBA'd data). Render-only,
  world-untouched — zero determinism impact — and being plain serializable
  data it is *also text-dumpable*, the line-set sibling of `World::dump()`.
- **Shell side**: `--debug-render physics` renders the frame with normal
  shading, then `render_debug_lines` draws the collected segments as a
  depth-tested GL line pass (`LEQUAL`, so lines coincident with collider
  surfaces don't z-fight) — works in mono and stereo, and in captures
  (`--capture-frame`) with no game-code changes. Native-only until the wasm
  shell grows a physics world.

This is the visual proof of reconcile correctness (declared scene vs what the
solver actually holds) and makes divergence bugs — a body the renderer draws in
one place and physics has in another — visible immediately.

## Test harness (extends `functor-netsim`)

The deterministic netsim (`docs/multiplayer.md` Phase 3) is the verification tool:

- **Determinism golden** (pure Rust, no GL): step an identical scene + identical
  command log in two fresh worlds for N frames; assert byte-identical snapshots
  each frame.
- **Strategy-equivalence golden**: run the same `Simulatable` + command log through
  `KeyframeLog` and `SnapshotRing`; assert `seek(K)` is byte-identical for every K.
  This is both rewind-correctness and a determinism check.
- **Replay golden**: record an input log, `ReplayOnly`-seek to the end, assert it
  matches a live run.
- **Convergence under latency/loss** (extends `tests/mp.rs`): server + 2 clients
  with a physics scene under `LinkProfile { latency, jitter, loss }`; assert each
  client's read-back converges to the server within tolerance after reconcile.
  Sweep profiles; add a partition→heal case (predict through, snap back on heal).
- **`netsim_viz` overlays**: render the authoritative server transform as a
  translucent **ghost** beside the client's predicted body (see prediction error +
  the reconcile snap); per-client metrics (max prediction error, rewinds/sec,
  resim depth); wire the existing pause/step controls to the snapshot ring for
  **backward** scrubbing.

### Networked-physics demo: `mpserver` / `mpclient`

The concrete vehicle for Phase 7 — grown directly from the existing examples
(today `mpserver` broadcasts player positions as a text snapshot and `mpclient`
renders them). The scene: **each client owns a bouncing ball; the server owns the
moving objects** (e.g. drifting platforms / bumpers the balls collide with). This
puts both authority directions in one scene, so a client's `physicsScape` declares:

- **its own ball** — `dynamic` + `Local`: simulated locally, its state broadcast.
- **other clients' balls** — `kinematic` + `Remote`: driven from snapshots, interpolated.
- **server objects** — `kinematic` + `Remote`: driven from the server snapshot.

It's worth building in two steps, because they exercise different machinery:

1. **State-sync split ownership** (no prediction). Each owner is authoritative over
   its entities and broadcasts pos/vel; everyone renders remotes as kinematic with
   dead-reckoning interpolation. This needs **no determinism** (owner is truth) and
   validates authority + `Remote`/kinematic + interpolation — the existing text
   protocol just grows to carry ball/object states alongside players.
2. **Server-authoritative ball + prediction** (the Timeline capstone). Flip the
   ball so the *server* is authoritative: the client sends **input** (impulses),
   predicts its ball locally, and `reconcile`s against server snapshots via the
   `Timeline`. This is the path that needs replay determinism and proves the
   prediction/reconciliation loop end-to-end — verified in `functor-netsim` under
   latency/loss, with `netsim_viz` ghosts showing predicted vs. authoritative.

## Roadmap (small, stacked PRs)

| Phase | Scope | Targets |
| --- | --- | --- |
| **1a. World spine** | Rapier dep (`serde-serialize`, default features), `physics` module (`PhysicsScene`/`Body`/`reconcile`/`WorldId` registry), fixed-step accumulator, snapshot + text/JSON dump. Determinism + restore-replay goldens. **No F# surface.** | native+wasm (Rust) |
| **1b. Timeline seam** | `Simulatable` + `Timeline` traits, `TimelineLog` with the three cadences (`keyframes(n)` default / `snapshot_ring` / `replay_only`), strategy-equivalence + replay goldens. | native+wasm (Rust) |
| **2. MLE surface + read-back** | `Physics.*` prelude (shape/body/scene builders, `position`/`transformed` live reads), optional `physics` hook in the MLE driver (tick → reconcile+fixed-step → draw), prelude tests. | native (MLE) |
| **2c. `examples/mle-physics`** | Crates settling on a ground slab, hot-reload demo, golden scenario, PR GIF/PNG. | native (MLE) |
| **2b. Debug visualization** | Rapier `debug-render` feature, `World::debug_lines()`, depth-tested line pass, `--debug-render physics` mode. **Shipped.** | native |
| **3. Commands** | `applyImpulse`/`applyForce`/`setVelocity`/`teleport` (plain-data effects). | both |
| **4. Queries** | `raycast`/`shapeCast` (async tagger, token registry). | both |
| **5. Collision events** | `Physics.events` sub. | both |
| **5b. Entity abstraction** | `Entities<'e>` + `Archetype` model-layer library, `Scene3D.instances` primitive, reconcile bail-out + tag interning, despawn-on-collision; `hello-physics` grows a bullet/debris archetype. | both |
| **6. Pause/rewind/replay** | timeline-control effects + keyboard wiring + egui status overlay (the culmination). | both |
| **7a. Networked physics (state-sync)** | `Authority`, `mpserver`/`mpclient` grown to client-owned balls + server-owned objects, kinematic `Remote` + interpolation. No prediction. | both |
| **7b. Prediction + reconciliation** | Server-authoritative ball, client input + prediction, structural `server`/`client` collections (network snapshot = `server`; reconcile = field swap), `Timeline` reconcile, `netsim_viz` ghosts + divergence metrics, latency-sweep convergence tests. | both |

(No cross-target determinism phase: neither netcode mode needs it — see
Determinism. `enhanced-determinism` + cross-target goldens is the documented
escape hatch if input-only lockstep is ever pursued.)

## Prior art

- **Declarative / functional physics**: `@react-three/rapier` (React reconciler
  diffing declarative bodies against a live Rapier world — the mature version of
  this exact pattern); **elm-physics** (`w0rm/elm-physics`, a pure-Elm rigid-body
  engine with an immutable `World.simulate : Duration -> World -> World` — the
  reference for the future explicit-`PhysicsWorld.t`); Unity DOTS `Unity.Physics`
  (deliberately stateless, rebuilt from component data each step).
- **Deterministic + rollback**: GGPO and its Rust ports `ggrs` / `bevy_ggrs`
  (`bevy_ggrs` + Avian/`bevy_rapier` is a working deterministic-rollback physics
  stack — the closest living reference); Photon Quantum (commercial deterministic
  ECS + **fixed-point** physics — the proof that cross-platform determinism pushes
  you to fixed-point).
- **Authority models in shipped games**: **Source / HL2** — the server owns
  *all* VPhysics; props are never client-predicted, only interpolated ~100 ms in
  the past. The one predicted subsystem is the hand-written, deterministic
  shared player-movement code, replayed against the input buffer on each
  authoritative update (Mode A restricted to a tiny engine-free state). The
  gravity gun is a server-side shadow controller velocity-steering the held prop
  toward a view attachment — so the prop visibly lags the predicted view in
  HL2DM; ragdolls/gibs are client-only cosmetics (our `client` collection).
  Ownership conflicts are *defined away* by never splitting ownership; the
  budget goes to hiding latency (prediction, interpolation, lag compensation).
  **Rocket League** — also server-authoritative, but the client predicts the
  *entire* Bullet world at a fixed 120 Hz tick and resimulates on correction:
  the closest shipped proof of Mode A / Phase 7b over a whole physics world.
- **Networked-physics literature**: Gabriel Gambetta, *Fast-Paced Multiplayer*
  (prediction / reconciliation / interpolation — read first); Glenn Fiedler,
  *Networked Physics* (lockstep vs. snapshot vs. state-sync, for physics
  specifically); Overwatch GDC 2017 (server-auth prediction on an ECS); Valve /
  Yahn Bernier (Source prediction + lag compensation); "dead reckoning"
  (extrapolate-and-correct remote entities).
- **Engine philosophy**: XPBD / position-based dynamics (Müller et al.),
  implemented in Rust as **Avian** — less hidden solver state than impulse-based
  Rapier, so snapshot/rewind/determinism are structurally easier; a fallback if
  Rapier determinism or rewind fidelity becomes painful.

## Our two repos (treated as evidence, not optimal)

`tommy-xr/citadel-xr` (OCaml + Ammo + a `reactify` scene reconciler) confirms the
declarative-state → reconcile → read-back → effect-with-dispatch loop works.
`tommy-xr/shock2quest` (Rust + Rapier 0.31, Shipyard ECS) shows the explicit
`synchronize_physics_positions()` sync and entity↔handle mapping via collider
`user_data`. Improvements over both: **fixed** timestep (shock2quest used
variable = nondeterministic), stable **tag** identity (citadel-xr keyed by list
index), explicit **authority + divergence** (vs `mass == 0`), and **serde
snapshots** (neither had).
