# LLM-native: the editor is a conversation

Status: **notes / vision** (no committed engineering yet). This jots down the
thinking behind design principle #2 (LLM-native) in `CLAUDE.md` — *why* the
runtime is built to be driven and observed without a GPU window, and where that
leads.

## Thesis

A traditional engine ships a heavyweight editor (Unity/Unreal/Godot) as the
primary authoring surface. Functor's bet is different: **the LLM plus a fast
iteration loop becomes the editor** — the same way an agentic coding tool displaces
much of what an IDE used to do. You don't click panels to build the game; you
direct an LLM that authors the game's pure MLE data and functions, runs it, looks at
the result, and iterates.

This is **load-bearing strategy, not a feature.** A full editor is one of the
largest line items in any engine, and "we'd also have to build the editor" is one
of the main reasons small-team / functional engines die from scope (see the
prior-art discussion in `docs/physics.md`). Not building one is what keeps the
project survivable — *provided* the runtime is introspectable enough that an LLM
can stand in for the editor's eyes.

## What transfers, and what doesn't

The agentic-coding analogy holds for the parts of game-making that are really
**authoring and data-wrangling in disguise** — and that's most of the structural
work:

- **LLM-subsumable (~80%):** scene composition, entity wiring, game rules, spawn
  logic, component/material config, netcode plumbing, bulk edits ("20 crates in a
  grid", "add a patrol AI"), refactors. Functor's serializable-MLE-data surface is
  the LLM's home turf.
- **Stays human — perceptual / feel judgment:** does the jump *feel* right, is the
  light too harsh, does the camera shake read, is the animation timed well. A
  multimodal model can critique a still frame, but judging 60fps motion and feel is
  weak. The human role shifts from **operator to director/critic.**

So the editor doesn't vanish — it **collapses from a *manipulation* surface into an
*observation / direction* surface.** You say "warmer, softer," look at the result,
and redirect, instead of dragging a light.

## The real bottleneck is the observation loop

LLMs are already good at the authoring. What's historically missing is the LLM
being able to **see results and iterate without a human relaying them.** Closing
that loop is exactly what principle #2 provides:

- headless frame capture (already shipped — `--capture-frame` / `--fixed-time`),
- serializable / inspectable state (the MLE model is a plain value, surfaced at the debug
  server's `GET /state`),
- the debug runtime on the backlog (`/state`, `/scene`, raycast — see
  `docs/todo.md`),
- a text-only runtime path that needs no GL window.

Those are the LLM's **hands and eyes**: author → capture → read state → diff →
iterate → surface candidates for the human's perceptual call. Functor is closer to
this than it sounds — frame capture + a debug server + deterministic replay is most
of the sensory apparatus already.

## Synergy with determinism / rewind

The determinism + rewind `Timeline` designed for physics/netcode
(`docs/physics.md`) doubles as an **LLM-editor primitive**: capture a session,
rewind to frame K, make a change, replay, and diff the outcome — *time-travel
authoring / what-if iteration.* The golden-image tests are the regression half of
the same loop. The determinism investment pays off twice — once for netcode, once
for LLM-driven iteration. Keeping replayability sacred (see
`docs/concurrency.md`) keeps the game LLM-observable even as it grows.

## Caveats / open questions

- **Keep a thin direct-manipulation surface.** Fine spatial/feel tuning ("nudge
  3 units until it looks right") is a 2-second human drag and a miserable LLM
  round-trip. Kill the *bloated* editor, not all direct manipulation — a live view
  plus a few sliders/gizmos still wins for that one class of task.
- **The black-frame problem.** An LLM staring at a wrong render has little to go on
  unless the runtime exposes rich introspection. The value of LLM-as-editor is
  directly proportional to how introspectable the runtime is — which is why #2 is
  load-bearing, not a nicety.
- **Discoverability for non-experts.** Editors teach what's possible by showing
  panels; a conversation requires knowing what to ask (or the LLM suggesting). The
  LLM can likely do the teaching, but it's an open UX question.
