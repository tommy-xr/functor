---
name: game-jam
description: >-
  Host a multi-agent Functor game jam: spawn parallel Opus subagents that each
  build a sample game in an isolated worktree, working docs-first against
  https://functor.games to stress-test the manual/API reference, and return a
  ranked friction log of engine + doc gaps. Then judge the entries (usefulness,
  canonicality), promote the best into examples/, and turn the consolidated gap
  list into roadmap work. Use when asked to run a game jam, stress-test the
  Functor API/docs by building games, or generate candidate examples at scale.
  Args: optional list of game briefs (e.g. `/game-jam tower-defense, tetris`);
  no args = pick from the backlog below.
---

# game-jam — parallel sample-building to find API & doc gaps

A game jam is a discovery instrument, not just content production. Each entry
produces **two deliverables of equal weight**:

1. **A sample-quality game** under `examples/<slug>/` — a candidate for the
   example corpus.
2. **A friction log** — every missing API, ergonomic gap, confusing error, doc
   gap, and workaround, ranked. A game that needed ugly workarounds plus a
   sharp friction log is MORE valuable than a smooth game with no notes.

The orchestrator (you) then judges the entries, consolidates the gap lists into
roadmap items, and promotes the best sample(s).

## Phase 0 — shared setup (orchestrator, before spawning)

1. Build the release CLI **once** and fetch shared assets:
   `npm run build:cli && npm run fetch:assets`. Run it in the background and
   launch agents immediately — they spend their first stretch reading docs.
2. All agents share `/…/functor4/target/release/functor` (absolute path into
   the MAIN repo). The binary interprets each game's `.fun` files, so worktree
   agents can use it directly; they must never build it themselves (7 parallel
   cargo builds would thrash the machine).
3. Skim `examples/` first so briefs don't duplicate existing samples (e.g.
   `mario` is already a 2D sprite platformer; `asteroids` is top-down 3D).

## Phase 1 — one agent per game

Spawn one **Opus** subagent per brief, `isolation: worktree`, in a single
parallel batch, running in the background. Each prompt must include:

- **The brief**: game, slug, and the specific engine surfaces it exists to
  stress (physics, XR input, 2D ergonomics, chase camera, UI/dialog, …).
- **Docs-first rule**: work like an external user — primary references are
  https://functor.games/manual/ and https://functor.games/docs/ (via WebFetch).
  Fall back to the `functor-lang` skill / existing examples only when the docs
  fail, and record **every fallback as a doc-gap finding**. This is how the jam
  stress-tests the docs, not just the engine.
- **Hard rules**: no engine/runtime/language/site edits — work around gaps in
  game code and log them (keeps friction reports honest and diffs reviewable);
  new files only under `examples/<slug>/`; commit locally on `jam/<slug>`; no
  pushes or PRs; never guess Functor Lang syntax from F#/OCaml intuition.
- **Verification loop**: `functor -d <dir> build native` must be clean;
  iterate on headless captures (`run native --capture-frame … --fixed-time …`,
  then Read the PNG) until it genuinely looks like the game; use the debug
  server (`docs/debug-runtime.md`: `--debug-port`, `--headless`, input
  injection) to prove input-driven behavior works, not just still frames.
  Keep 2–4 final PNGs in `examples/<slug>/.captures/` (uncommitted).
- **Assets**: primitives/procedural first; otherwise the branded-asset
  pipeline (`<name>.asset.json` CDN sidecars + `functor import`, commit
  `assets.fun`, never `.glb`); CC0 sounds copyable from
  `examples/asteroids/*.ogg`; the game must still run with assets missing.
- **xreview**: run the `xreview` skill on the committed change; disposition
  every Critical/High finding and re-verify. If subagent spawning is
  unavailable in the agent's context, run the Codex half via Bash and do the
  Claude half itself adversarially, noting the mode.
- **`JAM_NOTES.md`** (committed): what the game demonstrates, controls, and
  the friction log ranked **P0** blocked / **P1** painful workaround /
  **P2** ergonomic annoyance / **P3** nice-to-have, with doc gaps as their own
  section.
- **Final report** (returned text, machine-consumable): worktree/branch/SHA,
  game summary, capture paths, xreview outcome, the full friction log inline,
  and a 1–5 self-assessment on the two judging criteria below.

## Phase 1.5 — rolling gap fixers (don't wait for synthesis)

As friction reports land, triage blockers immediately instead of batching them
to the end — spawn a fixer/assessor agent (Opus, worktree) per cluster:

- **Doc gaps** are usually same-day: fix in the `.funi` doc comments and/or the
  `site/` manual, verify with `npm run check:docs` + a site build, open a
  draft PR right away. Accuracy over coverage — every documented signature must
  be verified against source, and stale `functor-lang`-skill content updates in
  the same PR.
- **Engine gaps** get a feasibility pass first: verify the jam agent's claims
  against the actual architecture (they may have missed an existing surface),
  design the smallest principled change, and implement + draft-PR **only if
  genuinely low-lift** (registry-registered prelude surface, both producers
  wired, `.funi` docs, determinism under fake/replay runners, frame_bench
  before/after, skill sync). Otherwise return a design sketch with a
  stacked-PR breakdown for the synthesis phase.

Point fixers at the jam entry's `JAM_NOTES.md` (read-only) as evidence, and
keep them off the jam worktrees.

## Phase 2 — judging (orchestrator)

Score each entry yourself — don't take self-assessments at face value; look at
the captures and read the code:

- **Usefulness** — does it demonstrate things no other sample does? An entry
  that overlaps an existing example needs a distinct angle to score well.
- **Canonicality** — does it exemplify Functor principles: pure functional
  core (all simulation in the model, thin `draw`), idiomatic MVU, deterministic
  /replayable, matches the corpus' code style and comment voice?

Also weigh: does it *still work* (re-run captures yourself), code size vs what
it shows (samples should be readable), and asset hygiene.

## Phase 3 — synthesis

1. **Consolidate friction logs** across entries: dedupe, keep per-gap evidence
   ("hit by 4/7 entries"), rank by frequency × severity. Multi-entry gaps are
   the roadmap items; single-entry P2/P3s are candidates, not mandates.
2. **Promote the winner(s)**: cherry-pick the sample from its jam worktree
   onto a fresh branch, adapt to corpus conventions (README/ASSETS.md,
   golden-scenario candidacy, `pr-visuals` GIF+PNG), and open a draft PR per
   the repo's stacked-PR conventions. Delete `JAM_NOTES.md` from the promoted
   copy — its content belongs in the synthesis report / issues, not the corpus.
3. **File the gap work**: turn the consolidated list into issues or follow-up
   PRs (engine gaps vs doc gaps separately — doc gaps are often same-day
   fixes to `site/` or the generated reference).
4. Clean up: remove jam worktrees/branches for entries that aren't promoted
   (after saving their JAM_NOTES content into the synthesis report).

## Backlog — future jam briefs and what each stresses

Already run (2026-07): pool (physics/sensors), shooting-range (FPS camera,
raycast, recoil), bow (XR two-handed, no-device XR loop), racer (procedural
track, chase cam), platformer (3D character controller, moving platforms),
rpg (2D/tilemap/dialog), asteroids2d (ortho/sprite 2D migration).

Candidates, chosen to cover surfaces the corpus doesn't yet exercise:

- **Tower defense** — mouse picking/placement UI, enemy pathing over a grid,
  wave scheduling via `Sub`, range queries at scale.
- **Tetris / falling-block puzzle** — pure grid model, rotation systems, timed
  gravity, line-clear animation; a canonicality showcase (zero physics).
- **Breakout / pong** — minimal-2D starter-tier sample; paddle/ball/brick in
  ~150 lines; tests how small a real game can be.
- **Twin-stick arena shooter** — gamepad input domain (`Input.snapshot`
  siblings), analog-stick handling, hundreds of entities.
- **Vampire-survivors-lite / boids swarm** — interpreter perf ceiling: how
  many entities before `frame_bench`-visible cost; spatial partitioning in
  pure Functor Lang.
- **Rhythm game** — audio/gameplay sync, beat timing, latency compensation;
  stresses the sound API's scheduling precision.
- **Roguelike floor** — procgen with seeded `Effect.random`, turn-based (no
  tick dependence), fog of war, text-heavy UI.
- **Card game (solitaire/memory)** — drag-and-drop mouse interaction,
  UI-dominant rendering, animation tweens between board positions.
- **Multiplayer tag / pong** — the `entries` client/server split beyond
  `examples/mp`: prediction, interpolation, authoritative physics.
- **Idle/incremental clicker** — save/load persistence effects (does the
  engine have any? that's the point), big-number formatting, offline progress.
- **Golf / marble course** — physics + terrain heightfield interplay
  (`terrain` showcase exists; putting a controlled impulse game on it doesn't).
- **Space dogfight / flight sim** — full 3D rotation (quaternion ergonomics),
  relative-velocity aiming, 6DoF camera.
- **Stealth vignette** — AI state machines in a pure model, vision cones,
  light/shadow as gameplay.
- **Beat-saber-like XR exercise** — XR at frame-rate budget on device,
  velocity-based scoring; pairs with the `vr-device-loop` skill for real
  measurements.
- **Walking-sim / photo mode** — atmosphere, skybox, fog, render targets,
  camera paths; stresses the rendering surface with near-zero game logic.

When picking a slate, mix: at least one physics-heavy, one input-surface, one
UI-heavy, one perf-stress, and one presentation-only brief — gaps cluster by
surface, and a diverse slate maximizes coverage per jam.
