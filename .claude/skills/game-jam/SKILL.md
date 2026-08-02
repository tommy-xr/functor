---
name: game-jam
description: >-
  Host a 1–10 entry multi-agent Functor game jam: high-capability subagents
  build sample games in isolated worktrees, docs-first against
  https://functor.games, and return ranked engine/doc friction. Then implement
  one evidence-backed blocker fix for every entry, make each game adopt its
  fix, judge usefulness/canonicality, promote the best into examples/, and turn
  remaining gaps into roadmap work. Use when asked to run a game jam,
  stress-test the Functor API/docs by building games, fix the blockers those
  games expose, or generate candidate examples at scale. Args: optional
  `--entries N` (1–10; no briefs defaults to 10), optional `--model MODEL`, and
  optional game briefs (e.g. `/game-jam --entries 8 --model gpt-5.6-terra
  tower-defense, tetris`).
---

# game-jam — parallel sample-building to find API & doc gaps

A game jam is a discovery instrument, not just content production. Each entry
produces **three deliverables of equal weight**:

1. **A sample-quality game** under `examples/<slug>/` — a candidate for the
   example corpus.
2. **A friction log** — every missing API, ergonomic gap, confusing error, doc
   gap, and workaround, ranked. A game that needed ugly workarounds plus a
   sharp friction log is MORE valuable than a smooth game with no notes.
3. **A blocker fix and adoption proof** — one real impediment is implemented
   in a reviewed fix PR, then the game removes its workaround or repeats the
   previously blocked workflow and proves the fix end to end.

The orchestrator (you) owns the roster, blocker coverage matrix, final judging,
consolidated roadmap, and promotion.

## Phase 0 — shared setup (orchestrator, before spawning)

1. Resolve the roster before spawning. `--entries N` is the total roster size
   and must be `1..10`. With `B` explicit briefs, require `B <= N` and add
   exactly `N-B` backlog briefs. Without `--entries`, use the explicit briefs,
   or 10 backlog briefs when `B=0`. Reject `B>N`, more than 10 explicit briefs,
   and duplicate slugs; never discard an explicit brief.
2. Create one exclusive per-run root outside the checkout (for example, the
   exact path returned by `mktemp -d /tmp/functor-jam.XXXXXX`). Put bootstrap,
   entry/fixer worktrees, immutable artifacts, gallery output, and a run ledger
   beneath it. Record every agent id/worktree and every shell-launched
   background process's PID, process group, cwd, command, and owner at launch.
3. Create a dedicated clean bootstrap worktree at the exact baseline SHA every
   jam branch uses. Build the release CLI **once** there and fetch shared assets:
   `npm run build:cli && npm run fetch:assets`. Run it in the background and
   launch agents immediately — they spend their first stretch reading docs.
   A SHA recorded from a dirty checkout is not valid provenance. No agent may
   build, capture, or run debug verification until the orchestrator confirms
   setup succeeded and records the binary's source SHA.
4. All Phase 1 game-builder agents share the bootstrap worktree's
   `target/release/functor` by absolute path. The binary interprets each game's
   `.fun` files, so builders must never build it themselves (ten cargo builds
   would thrash the machine). Phase 1.5 fixers build changed code only inside
   their dedicated fixer worktrees.
5. Skim `examples/` and recent `jam/*` branches first so briefs do not duplicate
   existing or just-completed samples (e.g.
   `platformer` is already a 2D sprite platformer; `asteroids` is top-down 3D).

## Phase 1 — one agent per game

Choose the game-builder model before spawning:

- Default to **Opus** on Claude-hosted agents and **GPT-5.6 Sol**
  (`gpt-5.6-sol`) on OpenAI-hosted agents.
- Accept `--model <model>` anywhere in the skill arguments. Remove that option
  from the game briefs and preserve the requested model identifier exactly.
- Pass the chosen model explicitly to every game-builder subagent. The override
  applies only to those agents, not to nested tools such as `xreview`, which
  retain their own model policy.

Create every git worktree beneath the exclusive per-run root and a
`jam/<slug>` branch for every brief, then spawn one subagent per brief with the
chosen model and its absolute worktree path. Require that path as the cwd for
every command; use a host-provided worktree-isolation option when available.
On OpenAI, use a non-full-history fork when passing an explicit model and
include all required context in the prompt. Pace the fleet; do not run
everything at once (see "Concurrency limits" below). Each prompt must include:

- **The brief**: game, slug, and the specific engine surfaces it exists to
  stress (physics, XR input, 2D ergonomics, chase camera, UI/dialog, …).
- **Docs-first rule**: work like an external user — primary references are
  https://functor.games/manual/ and https://functor.games/docs/ (via the host's
  browser/fetch tool). Fall back to the `functor-lang` skill / existing examples
  only when the docs fail, and record **every fallback as a doc-gap finding**.
  This is how the jam stress-tests the docs, not just the engine.
- **Hard rules**: no engine/runtime/language/site edits — work around gaps in
  game code and log them (keeps friction reports honest and diffs reviewable);
  new files only under `examples/<slug>/`; commit locally on `jam/<slug>`; no
  pushes or PRs; never guess Functor Lang syntax from F#/OCaml intuition.
- **Verification loop**: `functor -d <dir> build native` must be clean;
  iterate on headless captures (`run native --capture-frame … --fixed-time …`,
  then inspect the PNG with the host's image viewer) until it genuinely looks
  like the game. Prefer a reusable `tools/functor-sdk` scenario for multi-step
  lifecycle/assertion work (currently use waited `stepFrames(1)`, not the
  queue-only `step`), or `functor mcp` for agent-driven tool calls; raw
  debug-runtime HTTP is for one-off protocol probes. Preserve the scenario or
  exact tool sequence in `JAM_NOTES.md` as a durable proof that drives input and
  inspects state, not just still frames. Keep 2–4 final PNGs in
  `examples/<slug>/.captures/` (uncommitted).
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
  section. Add a **Fix-round candidate** section naming the best evidence-backed
  blocker, the ideal path attempted, observed failure/workaround, smallest
  useful fix, and an entry-level acceptance proof.
- **Final report** (returned text, machine-consumable): worktree/branch/SHA,
  game summary, capture paths, xreview outcome, the full friction log inline,
  fix-round candidate, and a 1–5 self-assessment on the judging criteria below.

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
- Count the orchestrator and nested reviewer subagents against the host's global
  slot limit. On a four-slot host, run at most two builders and admit only one
  builder to `xreview` at a time. Temporary slot exhaustion means wait; use the
  self-review fallback only when the host lacks subagent spawning entirely.
- **Disk hygiene between fixers (learned mid-run: a full disk killed a fixer
  mid-build).** Every heavy fixer leaves a multi-GB `target/` cache in its
  worktree; a long fixer queue fills the drive. Before deleting a fixer's
  `target/`, create a new non-existing artifact directory under the exclusive
  run root, copy the verified release CLI (with embedded wasm) through a
  temporary name and atomically rename it, make it read-only, and record its
  checksum. Verify that checksum immediately before every adoption/gallery use;
  otherwise retain the cache through adoption. Resolve finished worktrees
  individually, validate each absolute path against
  `git worktree list --porcelain`, and remove only the explicit
  `<finished-worktree>/target` — never use a fleet-wide wildcard. Check `df -h`
  before each heavy launch. Long-running heavy agents should report, not
  self-remediate outside their worktree, if disk runs low mid-task.
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
  for the current PR head SHA: require a non-zero expected check set with every
  job present and terminal-success. Zero checks, a missing expected job, a
  pending job, or a mismatched head SHA is not green.

## Phase 1.5 — one qualifying blocker removed for every entry

Triage while reports land, but start fixes only after all entry commits are
recorded as immutable **pre-fix baselines**; later entry commits are
adoption-only. Maintain a coverage table:

`entry | baseline SHA | blocker | cluster | fix PR@tested-head-SHA | ordered dependency SHAs | artifact checksum | adoption SHA | before proof | after proof`

The jam is not complete until every entry has a ready fix PR (shared when the
root cause is genuinely the same) and its own adoption proof.
Any fixer PR head change or rebase — standalone, shared, or dependent —
invalidates every adoption row naming the prior SHA; rerun CI, rebuild the
immutable artifact, and rerun all affected adoption proofs.

### Select blockers

- Reproduce each claim against source/current docs before accepting it; agents
  sometimes miss an existing surface.
- Choose the highest P0/P1 that prevents the ideal or canonical implementation.
  If none exists, use the highest P2 that blocks a trustworthy external-user
  workflow. Never pad the round with cosmetic P3 work.
- If an entry has no qualifying P0/P1 or workflow-blocking P2, run one
  additional targeted docs-first stress workflow. If that still yields none,
  replace an auto-filled backlog brief in the same roster slot. Never replace
  an explicit user brief without their authorization: report that entry blocked
  and request direction. If no permitted replacement exposes a real blocker,
  report the jam blocked rather than fabricate work or change the requested N.
- For a large gap, ship the smallest principled slice that removes one proven
  workaround from the entry. An issue or design sketch alone does not count.
- Dedupe shared root causes. One PR may cover several entries only when it has
  separate acceptance evidence for each; do not create duplicate APIs merely
  to preserve a one-PR-per-game ratio.

### Implement fixes sequentially

1. Create a fresh fixer worktree/branch from the correct base; never edit engine,
   language, runtime, or site code in a jam entry worktree. Prefer the original
   builder for a low-lift fix; use a dedicated fixer for cross-cutting work.
2. Capture a failing before-proof, implement the smallest complete change, and
   add regression coverage at the owning layer. Docs fixes must update the
   manual/reference/skill together where applicable; site docs include rendered
   desktop/mobile before-after media. Engine/prelude fixes wire every producer,
   preserve fake/replay determinism, rebuild wasm for web claims, and run the
   required before/after benchmarks for hot-path changes.
3. Run `xreview`, fix every Critical/High, disposition all findings in a draft
   PR, rerun verification, and wait for the complete CI matrix at the recorded
   head — but keep the PR draft until every covered entry proves adoption. Run
   one cargo-building fixer at a time and clear only its rebuildable worktree
   cache before launching the next.
4. For dependent fixes, branch and target each downstream PR on its immediate
   dependency. Record each PR's own commit range and tested head.

### Make the entry adopt the fix

After the draft fixer PR passes implementation review, verification, and CI at
its recorded head, resume the original builder in `jam/<slug>`:

- For code-facing fixes, remove the workaround/use the new surface, update
  `JAM_NOTES.md` with before/after evidence and the PR, rerun tests, scripted
  input/state proof, wasm when relevant, and final captures, then commit the
  adoption. Use the immutable verified fix artifact by absolute path.
- For docs/tooling fixes, repeat the formerly blocked docs-first workflow from
  the rendered/generated artifact and record the successful transcript. Do not
  invent a game-code change.
- Run `xreview commit <adoption-sha>` on the adoption delta. If the entry cannot
  prove the blocker is gone, reopen the fixer; a green unit test alone is
  insufficient. The orchestrator independently reruns every row's acceptance
  proof before declaring it complete.

Only after every adoption row covered by a fixer head passes, its adoption
reviews are dispositioned, and the orchestrator's proof reruns succeed: confirm
the fixer PR still has that exact head, require the full expected CI set green,
then mark it ready as the final action. A changed head invokes the invalidation
rule above and returns the PR to the proof cycle.

When several unmerged fixes are needed for final gallery testing, create a
disposable local integration branch from their common base. Cherry-pick only
each PR's own commit range in topological order, build the release CLI/wasm
once, and record the complete ordered fix-SHA set and artifact checksum in
every dependent proof. Do not push the integration branch.

## Phase 2 — judging (orchestrator)

Score each final, post-fix entry yourself — don't take self-assessments at face
value; look at the captures, read the code, and record the pre/post-fix
canonicality delta:

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
  --out <run-root>/gallery --serve 8321
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
The gallery output is scratch: build it under `/tmp`, don't commit it. If fixes
are not merged, pass the disposable integration branch's release binary so the
gallery exercises the adopted APIs rather than a stale main build.

## Phase 3 — synthesis

1. **Consolidate friction and fixes** across entries: preserve the blocker
   coverage table and before/after evidence, dedupe remaining gaps, keep
   frequency ("hit by 6/10 entries"), and rank by frequency × severity.
   Multi-entry gaps are roadmap items; single-entry P2/P3s are candidates.
2. **Promote the winner(s)**: cherry-pick the sample from its jam worktree
   onto a fresh branch, adapt to corpus conventions (README/ASSETS.md,
   golden-scenario candidacy, `pr-visuals` GIF+PNG), and open a draft PR per
   the repo's stacked-PR conventions. If the sample uses unmerged blocker
   fixes, stack its PR on the top of the complete recorded dependency chain.
   Delete `JAM_NOTES.md` from the promoted copy; its content belongs in
   synthesis.
3. **File only remaining gap work**: link the implemented blocker PRs and turn
   unresolved consolidated gaps into issues (engine vs docs separately).
4. Clean up. Two kinds, and both matter:
   - **Processes, immediately after any agent stops or is killed**: sweep for
     orphaned work by consulting the process IDs/groups recorded at launch.
     Verify each PID's command, cwd, parentage, and jam ownership, then stop only
     recorded jam descendants; never kill by executable-name match alone.
   - **Worktrees, only after synthesis**: first confirm every entry's work is
     committed to its `jam/<slug>` branch (branches survive worktree removal;
     staged-but-uncommitted work does not), save JAM_NOTES and required captures
     outside the per-run root, and inspect `git status --porcelain` in each
     worktree. The only permitted untracked paths are explicitly inventoried
     disposable captures; archive them, remove those exact paths, and use normal
     `git worktree remove` without `--force`. After every registered worktree and
     process is gone and final evidence is preserved elsewhere, validate the
     exact per-run root and remove it, retiring bootstrap, artifact, gallery,
     and ledger scratch together.

## Backlog — future jam briefs and what each stresses

Already run (2026-07): pool (physics/sensors), shooting-range (FPS camera,
raycast, recoil), bow (XR two-handed, no-device XR loop), racer (procedural
track, chase cam), platformer (3D character controller, moving platforms),
rpg (2D/tilemap/dialog), asteroids2d (ortho/sprite 2D migration), tower-defense
(picking/pathing), marble-golf (physics/events), swarm-survival
(spatial/perf), space-dogfight (flight/state), and photo-mode
(render targets/camera).

Candidates, chosen to cover surfaces the corpus doesn't yet exercise:

- **Tetris / falling-block puzzle** — pure grid model, rotation systems, timed
  gravity, line-clear animation; a canonicality showcase (zero physics).
- **Breakout / pong** — minimal-2D starter-tier sample; paddle/ball/brick in
  ~150 lines; tests how small a real game can be.
- **Twin-stick arena shooter** — gamepad input domain (`Input.snapshot`
  siblings), analog-stick handling, hundreds of entities.
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
- **Stealth vignette** — AI state machines in a pure model, vision cones,
  light/shadow as gameplay.
- **Inventory/crafting sandbox** — drag/drop, tooltips, focus, typed item data,
  save/load, and UI state at scale.
- **Turn-based tactics** — grid picking, path/range overlays, deterministic AI,
  action queues, and camera transitions.
- **Beat-saber-like XR exercise** — XR at frame-rate budget on device,
  velocity-based scoring; pairs with the `vr-device-loop` skill for real
  measurements.

With no briefs, choose 10 distinct candidates. Substitute a non-device brief
for XR when no headset is attached. For `N >= 6`, cover at least one
physics/simulation-heavy, one input-surface, one UI-heavy, one perf-stress, one
network/effect-heavy, and one presentation-first brief. For `N < 6`, maximize
surface diversity without expanding the requested briefs into kitchen-sink
projects.
