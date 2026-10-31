# Golden scenarios

Golden-image regression scenarios live next to their sample, in
`examples/<sample>/goldens.json`. Both harnesses read them: the native runner
test (`runtime/functor-runtime-desktop/tests/golden.rs`, `npm run test:golden`)
and the wasm Playwright test (`e2e/golden-wasm.spec.mjs`, via
`scripts/goldens.mjs`). Adding a scenario covers both targets at once.

Each file is a JSON array of scenarios. The sample is the directory the file
sits in. Scenarios run in sample-directory order, then file order, and names
must be unique across all samples.

```json
[
  { "name": "scene-grid-t2", "fixedTime": 2.0, "debugRender": null, "targets": ["native"] },
  { "name": "scene-grid-off", "debugRender": null, "targets": ["native"],
    "inputScript": "toggle.script", "captureAtFrame": 120 }
]
```

| Field | Meaning |
| --- | --- |
| `name` | Reference-image basename; globally unique. |
| `fixedTime` | Frame time (seconds) to pin for a deterministic pose. |
| `inputScript` + `captureAtFrame` | Instead of `fixedTime`: replay this script (relative to the sample) and capture at that sim frame. Native only. |
| `debugRender` | Optional debug-render mode (`"normals"`, `"tangents"`), or `null`. |
| `targets` | Harnesses that run it: `"native"` captures the runner framebuffer, `"wasm"` screenshots the WebGL2 canvas. |

Reference images: native at `examples/<sample>/golden/<name>.png`, wasm at
`e2e/golden-wasm.spec.mjs-snapshots/<name>-wasm-<platform>.png`. The
regeneration command is in `golden.rs`'s doc comment.

`npm run test:golden-manifests` (run in CI) checks every file's shape and name
uniqueness, and fails if the retired root `golden-scenarios.json` reappears.
