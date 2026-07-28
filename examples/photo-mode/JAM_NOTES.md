# Mistline Observatory

## Demonstration

An asset-free photographic walking vignette: three authored viewpoints, stepped composition
controls, adjustable field of view, atmospheric fog matched to the clear color, warm/cool
lighting, rule-of-thirds guides, and a live alternate-camera composition monitor rendered into
the scene. The scene is designed around a distant beacon framed by a monumental arch rather
than a survey row of primitives.

## Controls

- `1`, `2`, `3`: authored viewpoints
- `W`, `S`: dolly forward/back
- `A`, `D`: strafe left/right
- `Up`, `Down`: narrow/widen the field of view
- `P`: toggle photo guides
- `V`: toggle the in-world alternate-camera monitor
- `Space`: record a model-visible exposure count (the engine has no screenshot effect)

## Friction

- **P1 — No game-facing screenshot API.** Photo mode can count an exposure, but cannot request a
  screenshot or receive its path. Captures remain a runner/debug-tool concern.
- **P1 — No depth of field or exposure controls.** The photography look must be authored through
  composition, fog, material color, and light intensity; there is no aperture/focus/exposure
  surface.
- **P2 — `Camera.lookAt` fixes FOV at 45 degrees.** Adjustable lenses require
  `Camera.firstPerson`, so authored target-point composition must be translated manually into yaw
  and pitch.
- **P2 — No camera roll/custom up vector.** Dutch angles and portrait-orientation framing cannot
  be expressed by a camera. `Camera.lookAt` uses fixed +Y up and `firstPerson` exposes yaw/pitch
  only.
- **P2 — Rising-edge actions require hand-authored per-key latches.** Because repeated key-down
  events are documented, the two toggles and exposure action carry three booleans in the model.
  Movement/lens controls intentionally retain key-repeat stepping.
- **P3 — Render-target screens have no documented tone/filter/aspect-fit controls.** The monitor
  is manually sized to 16:9 and presented as emissive.
- **P3 — UI panels do not expose custom framing chrome.** The photographic grid is therefore a
  `Frame.with2D` sprite pass while textual status uses `ui`.

## Documentation gaps and fallbacks

- The public manual and generated API reference document every game API used by the implementation.
- The public “Driving games with agents” section explains MCP concepts and tool names, but not the
  raw debug-server HTTP endpoints or JSON bodies needed for a CLI/curl verification. Verification
  therefore fell back to repository `docs/debug-runtime.md`.
- The public manual's game snippets show top-level bindings but do not demonstrate local binding
  syntax. The first build diagnosed that local bindings require `let ... in`; resolving the error
  required the `functor-lang` skill.
- No existing example was needed to author the implementation.

## Render-target safety

The alternate camera renders `world(false)`, which excludes the monitor itself. The main world
then displays that target. This intentionally avoids render-target feedback/recursion and keeps
the feature to one writer and one reader.

## xreview disposition

- **High — repeated key-down events retriggered toggles/exposures:** fixed with explicit rising-edge
  latches for `P`, `V`, and `Space`; repeated-down behavior was reverified through the debug server.
- **Medium/Low:** the independent adversarial pass found no additional correctness or simplicity
  issues requiring changes. The monitor feed visibly differs from the main camera and avoids
  self-sampling.
- **Degraded reviewer mode:** the required Claude/Opus reviewer model was unavailable in this
  environment. The Codex CLI reviewer also failed before startup because its installed package
  lacks `@openai/codex-darwin-x64`. Review therefore consisted of an independent manual adversarial
  pass plus the orchestrator's separate spot-check; duplication scanning was attempted but did not
  finish within the interactive window.
