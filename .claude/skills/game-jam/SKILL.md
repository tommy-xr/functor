---
name: game-jam
description: >-
  Host a multi-agent Functor game jam: spawn parallel high-capability subagents
  (Opus on Claude, GPT-5.6 Sol on OpenAI by default) that each build a sample
  game in an isolated worktree, working docs-first against
  https://functor.games to stress-test the manual/API reference, and return a
  ranked friction log of engine + doc gaps. Then judge the entries (usefulness,
  canonicality), promote the best into examples/, and turn the consolidated gap
  list into roadmap work. Use when asked to run a game jam, stress-test the
  Functor API/docs by building games, or generate candidate examples at scale.
  Args: optional `--model MODEL` override and optional list of game briefs
  (e.g. `/game-jam --model gpt-5.6-terra tower-defense, tetris`); no briefs =
  pick from the backlog below.
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

Choose the game-builder model before spawning:

- Default to **Opus** on Claude-hosted agents and **GPT-5.6 Sol**
  (`gpt-5.6-sol`) on OpenAI-hosted agents.
- Accept `--model <model>` anywhere in the skill arguments. Remove that option
  from the game briefs and preserve the requested model identifier exactly.
- Pass the chosen model explicitly to every game-builder subagent. The override
  applies only to those agents, not to nested tools such as `xreview`, which
  retain their own model policy.

Spawn one subagent per brief with the chosen model, `isolation: worktree`,
running in the background — but **pace the fleet; do not run everything at
once** (see "Concurrency limits" below). Each prompt must include:

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

## Concurrency limits (learned the hard way — 2026-07 jam)

Running the whole fleet at once oversubscribed the machine badly: 7 jam agents
+ 6 fixers + their xreview reviewer subagents peaked at **61 cargo / 51 rustc
processes**, producing 90-minute compiles that normally take minutes, capture
runs so slow the UI overlay hadn't rastered by capture time (agents chased
phantom rendering bugs), Codex CLI hangs, and meaningless wall-clock bench
numbers. Rules:

- **Default to sequential, or at most 2–3 concurrent agents**, on a developer
  laptop. The user experiences the whole fleet's load; ask before scaling wider.
- **Pair one LIGHT agent with one HEAVY one** when running two. Light = works
  against a prebuilt binary or docs (interpreting game files, captures, site
  builds, snippet typechecks). Heavy = anything that compiles the toolchain
  (cold-worktree cargo, wasm-pack). Two lights are also fine; two heavies never
  are. Classify each agent's *remaining* work before starting it — a "docs"
  task that needs docgen via cargo counts as heavy.
- Jam agents are cheap only while writing/interpreting `.fun` (the shared
  prebuilt binary) — but each one's **xreview spawns reviewers**, and finisher
  phases run captures; count those toward the cap.
- **Never let two fixers cold-build Rust worktrees concurrently.** A fresh
  worktree compiles the whole dependency graph; two at once thrash, and the
  global `~/.cargo` package-cache lock serializes them anyway. Run
  cargo-building fixers strictly one at a time.
- **Benchmarks need a quiet machine.** frame_bench wall-clock is worthless
  under load; schedule bench-requiring fixers last, alone, and lean on
  allocs/bytes-per-frame (load-immune) as the acceptance numbers.
- Sequencing mechanic: resume/spawn one agent, wait for its completion
  notification, then start the next — nearest-to-completion first so results
  land early.
- **Disk hygiene between fixers (learned mid-run: a full disk killed a fixer
  mid-build).** Every heavy fixer leaves a multi-GB `target/` cache in its
  worktree; a long fixer queue fills the drive. Once a fixer's PR is pushed,
  its `target/` is pure rebuildable cache — sweep finished fixers' caches
  BEFORE starting the next heavy agent (`rm -rf .claude/worktrees/agent-*/target`
  for done agents only; never the running one's), and check free space
  (`df -h`) as part of each heavy launch. Don't wait for end-of-jam cleanup;
  by then the drive may already be full. Long-running heavy agents should also
  self-monitor: instruct them to delete their OWN cache as their last action,
  and to report (not self-remediate outside their worktree) if disk runs low
  mid-task.
- **Local verification is single-platform — the CI matrix is part of the
  gate.** A fixer with 600 green local tests still shipped an x86-vs-ARM
  divergence (`f64::total_cmp` orders computed NaNs by sign bit, which differs
  per architecture) that only ubuntu CI caught. Fixers hold PRs in draft until
  `gh pr checks` is green on ALL OSes; float-ordering/libm-adjacent code gets
  explicit cross-platform thought up front. Also verify the CI matrix actually
  covers the crates you touched (this repo's CI skipped the desktop crate's
  tests, which is how a broken test lived on main unnoticed) — run uncovered
  suites locally and say so.
- **Wasm bundle staleness lies.** The engine prelude ships INSIDE the
  web-runtime bundle; rebuilding the native binary alone leaves the bundle
  stale, so a WebGL2/web claim verified against it can be false (`module X has
  no member` at runtime). Any prelude/runtime change that claims web support
  must rebuild the wasm bundle and re-run the web smoke path before the claim
  goes in a PR body.
- **Orchestrator CI watchers: verify, then flip.** Agents' CI monitors die
  with their process — the orchestrator finishes the watch. Use strict logic
  (count checks, require ALL pass, print offenders otherwise); a sloppy
  one-liner once flipped a PR ready with a red check.

## Phase 1.5 — gap fixers (sequential, after entries complete)

Triage blockers as friction reports land, but run the fixing **sequentially,
after the jam entries finish** — fixers cold-build Rust worktrees and run
xreview, exactly the load the concurrency limits exist for. One fixer at a
time, nearest-to-mergeable first.

**Prefer the game's own agent for low-lift fixes.** If the gap is small and
blocks that agent's game (a doc wording, a missing signature, a small prelude
register), resume the entry's agent after its game is committed and have it
open the fix PR itself — it has the full context and the motivating example.
Spawn dedicated assessor/fixer agents only for cross-cutting or heavier gaps.
Per cluster:

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

**Docs changes are visual changes.** A PR that edits a manual/docs/site page
embeds before/after screenshots of the rendered page (base ref vs change), the
same as any rendering PR — reviewers judge the page users will see, not the
markup diff.

**PR lifecycle:** fixers open PRs as drafts (repo convention), and once
verification is re-run post-review and every Critical/High is dispositioned in
the body — plus media embedded for visual changes — the PR gets marked ready
**in the same breath** (`gh pr ready`), by the fixer or the orchestrator.
Draft is a "review/captures still landing" signal, not a resting state; a
fully-verified PR parked in draft just hides finished work from reviewers.

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

### Gallery — play the entries

Judging is ultimately a *human* call, and a human judge should **play** the
games, not just read reports and stare at stills. Wasm bundles come free —
`functor build wasm` exports a self-contained static bundle per project — so
put every entry behind one URL and hand it over:

```sh
node .claude/skills/game-jam/scripts/build-gallery.mjs \
  platformer=<worktree>/examples/platformer racer=<worktree>/examples/racer \
  --out /tmp/jam-gallery --serve 8321
```

It builds each project, copies the bundles, and generates a card index —
thumbnail (newest PNG in the project's `.captures/`), description, and
controls. Metadata comes from a `// gallery:` / `// gallery-controls:` header
comment in `game.fun`; a `--manifest <file>.json` adds titles and scores and
overrides the header fields. **Get the controls right** — an entry that fires
with `SPACE` described as "click to shoot" reads as broken.

Two things to know: `--out` is **wiped on every run** (it refuses to delete a
directory it didn't create, so don't point it at anything you care about), and
it needs a **release** binary — `target/release/functor`, or `--functor <path>`.
The gallery output is scratch: build it under `/tmp`, don't commit it.

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
4. Clean up. Two kinds, and both matter:
   - **Processes, immediately after any agent stops or is killed**: sweep for
     orphaned `functor` debug servers, `cargo`/`rustc`/`sccache` trees, and
     Codex reviewer processes (`ps aux | grep -E 'functor|cargo|rustc|codex'`)
     — killed agents routinely leave runaway compiles and hung reviewers
     burning CPU.
   - **Worktrees, only after synthesis**: first confirm every entry's work is
     committed to its `jam/<slug>` branch (branches survive worktree removal;
     staged-but-uncommitted work does not), save JAM_NOTES content into the
     synthesis report, then remove the worktrees — including their multi-GB
     `target/` dirs from fixer builds.

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
