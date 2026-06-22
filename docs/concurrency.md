# Concurrency / parallelism

Status: **notes / forward-looking** (no committed engineering yet). Design intent
for when the engine becomes CPU-bound. The headline: the functional architecture
makes the *safe* kinds of parallelism nearly free, and the determinism the rest of
the design rests on (`docs/physics.md`) sets a hard rule for the rest.

## Three kinds of parallelism (don't conflate them)

1. **Pipeline across frames** — render frame N while simulating frame N+1.
2. **Data-parallel within a system** — physics islands, entity update, cull/draw.
3. **Many independent worlds** — server rooms/matches, the netsim's N instances.

"Run all the systems in parallel" overstates it: within a single frame,
`tick → physics → render` is a **dependency chain**, not a fan-out (physics needs
the post-tick model; render needs the post-physics transforms). Only the work that
doesn't feed the sim — audio, asset loading, some AI — is genuinely
frame-internal-parallel.

## The big win is nearly free: the sim‖render pipeline

Decoupling the sim thread from the render thread (render N while simulating N+1) is
the largest classic parallelism win, and immutability makes it **free** here.
`draw3d : model -> Frame` is a pure function of an immutable model, and `Frame` is
an immutable description — so the render thread can read frame N's `Frame` with no
locks while the sim builds N+1. The hand-rolled "snapshot the world for the render
thread" double-buffering that mutable engines agonize over simply falls out. Same
for audio (`soundScape -> AudioScene`) and the physics view.

The general statement: **the functional boundaries are the concurrency
boundaries.** The functional-core / imperative-shell split *is* the thread split —
the pure core produces immutable values; the shell consumers (GL renderer, audio
output, asset IO) read them on their own threads. You don't have to *find* the safe
hand-off points; the purity boundary already named them.

## The determinism rule (load-bearing)

Parallelism must never make a frame's outcome depend on thread scheduling, or
rewind / netcode / replay / LLM-observability all break. That gives a clean split:

- **Output-only systems (render, audio)** → parallelize **freely**. They don't feed
  the sim, so non-determinism there is invisible to the `Timeline`.
- **Feedback systems (physics)** → parallelize **only deterministically.** Lean on
  Rapier's `enhanced-determinism` (it already solves islands in parallel with
  order-independent results); don't hand-parallelize the solver.
- **The update / effect-drain core** → stays **serial.** Message order is part of
  determinism. Pure per-entity work *inside* a tick (`Entities.update`) can
  `par_iter` only if the combine is order-independent.

## Rough priority

1. **Sim‖render pipeline split** — biggest win, cheapest given immutability, and
   rendering is usually the frame-time hog.
2. **Rapier's internal parallelism** — free; just enable it.
3. **Data-parallel entity update** — `par_iter` over `Entities<'e>` once counts
   demand it (thousands), not before.
4. **Many independent worlds** — embarrassingly parallel; the netsim already runs N
   instances, and server room/match sharding scales across cores cleanly.
5. **Audio / asset loading off the critical path** — real but low-stakes background
   work, not the bottleneck.

## Caveats

- **Don't parallelize prematurely.** Small games are CPU-cheap; task-spawn/sync
  overhead can *lose*, and the deterministic-replay property is worth more than the
  speedup until you're actually CPU-bound.
- **wasm threading is more constrained than native** (SharedArrayBuffer + workers),
  so the parallelism story is target-dependent. Treat it as an optimization layer,
  not a foundational assumption.
- **Replayability is sacred.** The moment parallelism makes the *feedback* path
  scheduling-dependent, you've traded away the determinism everything else relies
  on. That's the one line not to cross.
