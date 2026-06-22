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
2. **Determinism.** Same inputs → same simulation, byte-for-byte, so we can
   rewind, replay, and verify in the deterministic netsim. This forces a fixed
   timestep and deterministic body ordering from day one.
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

## API (F#)

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

- **Fixed-step accumulator** in the shell (never variable dt).
- Rapier features **`enhanced-determinism`** + **`serde-serialize`**.
- **Deterministic reconcile order** (sort by tag) for spawn/despawn/insert.
- No wall-clock / unseeded RNG leakage (netsim already uses a seeded SplitMix64).

Two determinism tiers, with very different costs — this drives the netcode order:

- **Single-binary replay determinism** (same build re-simulating its own recent
  frames). Cheap and robust with the above. **Enough for server-authoritative +
  prediction.**
- **Cross-platform determinism** (native ↔ wasm, x86 ↔ ARM). Hard — f32 / libm
  differences. Required only for lockstep / peer-owned simulation. **Validate with
  goldens; never assume.** (Serious deterministic-netcode engines go fixed-point
  for this reason — a fallback if cross-target goldens won't converge.)

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
    fn step(&mut self, dt: Fixed, cmds: &[Self::Command]) -> Vec<Self::Event>;
}

/// The SWAPPABLE part. The sim loop and netcode name only this trait.
trait Timeline<S: Simulatable> {
    fn record(&mut self, frame: Frame, sim: &S, cmds: &[S::Command]);
    fn seek(&mut self, frame: Frame, sim: &mut S);          // restore exactly `frame`
    fn commands_since(&self, frame: Frame) -> &[Vec<S::Command>];
    fn prune(&mut self, before: Frame);                     // memory bound / server-confirmed
}
```

Strategies are tiny impls of one trait:

- **`KeyframeLog`** (default, the hybrid) — snapshot every N frames + always log
  commands; `seek` restores nearest keyframe ≤ frame then `step`s forward. Bounded
  memory *and* seek.
- **`SnapshotRing`** — full snapshot every frame; `seek` = `restore(ring[frame])`.
  O(1) seek, heavy memory. (Used as the oracle in strategy-equivalence goldens.)
- **`ReplayOnly`** — one snapshot at frame 0; `seek` restores it and replays
  0→frame. Lightest memory, leans hardest on determinism.

Reconciliation is written **once, against the trait**, and never changes when the
strategy is swapped:

```rust
fn reconcile<S, T: Timeline<S>>(tl: &mut T, sim: &mut S,
                                k: Frame, authoritative: &S::Snapshot, now: Frame) {
    sim.restore(authoritative);          // server truth at frame K
    tl.overwrite(k, authoritative);      // correct recorded history at K
    for (_f, cmds) in tl.commands_since(k) {
        sim.step(FIXED_DT, cmds);        // replay OUR local inputs K+1..now
    }
}
```

The trait contract — `seek(K)` equals restoring a valid earlier state and stepping
forward with recorded commands — *is* the determinism invariant the netcode rests
on. The F# surface stays thin (`Physics.rewindTo`, `pause`, `resume`, `stepOnce`);
strategy choice is runtime config, defaulting to `KeyframeLog`.

## Netcode (server-authoritative first)

`Authority = Local | Remote of source` covers both target modes:

- **Mode A — server-authoritative + client prediction** (build first; needs only
  single-binary replay determinism). Server declares gameplay bodies `Local`;
  client declares its own player `Local` (predicted) and everything else `Remote`
  (kinematic, interpolated from snapshots). On an authoritative snapshot for frame
  K, `reconcile` restores K and replays stored local inputs K→now.
- **Mode B — split ownership** (deferred; needs cross-platform determinism). Each
  entity's `Authority` decides who simulates it; peers render others as kinematic.
  Same reconciler; gated behind the cross-target determinism goldens.

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
| **1. Shell spine** | Rapier dep (`enhanced-determinism` + `serde-serialize`), `physics` module (`PhysicsScene`/`Body`/`reconcile`/`WorldId` registry), fixed-step accumulator, `Simulatable` + `Timeline` traits, `KeyframeLog`, snapshot + text/JSON dump. Determinism + strategy-equivalence + replay goldens. **No F# surface.** | native+wasm (Rust) |
| **2. `physicsScape` + read-back** | `Game` hook + builder/runner, reconcile pipeline, `DrawContext` record on `draw3d` (`ctx.physics` view), `Physics.synced` sub, `examples/hello-physics`. | both |
| **3. Commands** | `applyImpulse`/`applyForce`/`setVelocity`/`teleport` (plain-data effects). | both |
| **4. Queries** | `raycast`/`shapeCast` (async tagger, token registry). | both |
| **5. Collision events** | `Physics.events` sub. | both |
| **5b. Entity abstraction** | `Entities<'e>` + `Archetype` model-layer library, `Scene3D.instances` primitive, reconcile bail-out + tag interning, despawn-on-collision; `hello-physics` grows a bullet/debris archetype. | both |
| **6. Pause/rewind/replay** | timeline-control effects + keyboard wiring + egui status overlay (the culmination). | both |
| **7a. Networked physics (state-sync)** | `Authority`, `mpserver`/`mpclient` grown to client-owned balls + server-owned objects, kinematic `Remote` + interpolation. No prediction. | both |
| **7b. Prediction + reconciliation** | Server-authoritative ball, client input + prediction, structural `server`/`client` collections (network snapshot = `server`; reconcile = field swap), `Timeline` reconcile, `netsim_viz` ghosts + divergence metrics, latency-sweep convergence tests. | both |
| **8. Cross-target determinism** | native↔wasm determinism validation (gated on Phase 1 goldens); fixed-point fallback if needed. | both |

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
