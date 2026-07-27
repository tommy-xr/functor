# Tetris game-jam notes

## What this demonstrates

`NEON STACK` is a complete deterministic falling-block puzzle built with Functor
Lang's pure MVU loop and 2D sprite API. Its model contains only plain data:

- a sparse list of locked grid cells;
- the active tetromino and deterministic next-piece index;
- score, line, level, gravity, line-clear, and key-latch state.

There is no physics hook. Collision, wall kicks, ghost placement, locking,
multi-line collapse, scoring, and increasing gravity are ordinary functions over
the model. `draw` converts that data into a `Frame.create2D` picture with no
assets.

The opening stack is intentional: it makes the grid and ghost immediately
legible in the gallery, while every subsequent board state comes from play.

## Controls

| Action | Keys |
| --- | --- |
| Move left / right | `A` / `D` or `Left` / `Right` |
| Rotate clockwise | `W` or `Up` |
| Rotate counter-clockwise | `Z` |
| Soft drop one row | `S` or `Down` |
| Hard drop and lock | `Space` |
| Restart at any time | `R` |

Keyboard input is edge-latched in the model. OS key repeats cannot accidentally
apply multiple rotations or hard-drop multiple pieces; a release is required
before the next action.

## Friction log

Ranked as `P0 blocked`, `P1 painful workaround`, `P2 ergonomic annoyance`, and
`P3 nice-to-have`.

1. **P1 — Functional grid updates are verbose without an indexed update or map
   type.** `List.grid` makes the initial board pleasant, but gameplay needs a
   sparse cell list plus repeated `filter`/`any` passes to answer occupancy,
   remove rows, and collapse cells. This stays canonical and readable at 10×20,
   but a first-class immutable grid/map surface would make the representation
   both clearer and asymptotically better.
2. **P2 — Deterministic action edges require a user-space held-key latch.** The
   `input` hook forwards OS key repeats as additional `isDown = true` events.
   The sample therefore carries `heldKeys` in its game model and filters it on
   release. This is correct but obscures the puzzle logic; an opt-in edge-only
   input hook would remove the boilerplate.
3. **P2 — Rotation kicks have no reusable game-side vocabulary.** Kick
   candidates are a literal list and selection is `List.find(validPiece)`.
   That is transparent, but even common puzzle ergonomics (different I-piece
   kicks, floor kicks, clockwise/counter-clockwise tables) quickly becomes data
   plumbing. A standard example or small immutable table helper would help
   without moving gameplay policy into the engine.
4. **P2 — Debug state exposes the model as Rust `Debug` text, not structured
   JSON.** `/state` is sufficient for human behavior proof, but automation must
   search a string for `active`, `score`, or `phase` instead of addressing
   fields. Plain Functor values are already serializable in spirit, so a
   structured model field would materially improve LLM/test driving.
5. **P2 — Relative capture paths resolve from the game directory.** Running
   from the repository root with
   `--capture-frame examples/tetris/.captures/opening.png` attempted to write
   below `examples/tetris/examples/tetris/` and failed only after the capture
   run. An absolute path works, but either resolving against the caller's
   working directory or printing the resolved target before launching would
   make the CLI much less surprising.
6. **P3 — `List.maximum` remains partial.** Ghost/hard-drop distance can use it
   only because `validPiece(model.active)` establishes a non-empty candidate
   list. An `Option`-returning maximum, parallel to `List.head`/`find`, would
   let that invariant be explicit rather than lodged in a comment or proof.

## Documentation gaps

1. The public manual and generated API reference do not document the
   `// gallery:` and `// gallery-controls:` header convention requested for
   examples. Existing repository search also yielded no worked header, so the
   sample uses concise descriptive values inferred from the requested field
   names.
2. The public manual/API do not expose the debug-runtime HTTP workflow needed
   to prove controls. The repository-only `docs/debug-runtime.md` was required
   for `--debug-port`, `--headless`, `POST /input`, `POST /time`, and the rule
   that `--fixed-time` cannot be advanced.
3. Public `https://functor.games/docs/` documents `Sprite.text`,
   `Camera2D`, and `Frame.create2D`, but the initially available shared release
   CLI predated that surface (`module Sprite has no text`). Verification had to
   wait for the orchestrator's matching release build; the version relationship
   between hosted docs and an installed CLI is not visible on either page.
