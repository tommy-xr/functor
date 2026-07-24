// The single source of truth for the sandbox's example set. Two consumers read
// this one list, so it can't drift:
//   - build.mjs (Node)   uses `id` + `source` to copy the .fun into
//     dist/examples/<id>.fun at build time;
//   - sandbox.js (browser) uses `id` + `label` to build the scene dropdown.
// The site e2e derives its per-example smoke test from the rendered picker, so
// it tracks this list automatically too.
//
// `source` is a path relative to the repo root. Most entries are a single,
// asset-free .fun (or use absolute CDN assets). A project that needs sibling
// modules or local assets declares explicit sibling/asset
// `{ source, output }` copies; the sandbox still edits only the canonical
// `source` entry while the player fetches the complete project file list.
// Asset outputs are relative to the site root because browser fetches resolve
// `Asset.*` locators against player.html.
export const EXAMPLES = [
  { id: "hero", label: "Neon grid", source: "site/examples/hero.fun" },
  { id: "orbit", label: "Orbit", source: "site/examples/orbit.fun" },
  // Single-file + a CORS-friendly CDN model (jsDelivr mirror of BabylonJS/Assets),
  // so the rigged character streams and animates in the single-buffer sandbox.
  { id: "batteries", label: "Animation", source: "site/examples/batteries.fun" },
  { id: "counter", label: "Counter", source: "examples/counter/game.fun" },
  { id: "primitives", label: "Primitives", source: "examples/primitives/game.fun" },
  { id: "ui", label: "UI widgets", source: "examples/ui/game.fun" },
  { id: "inspector", label: "Inspector", source: "examples/inspector/game.fun" },
  // Named `bounce` (not `physics`): the flat copy makes `file = module`, and a
  // module literally named `Physics` collides with the builtin/prelude namespace.
  { id: "bounce", label: "Physics", source: "examples/physics/game.fun" },
  { id: "toss", label: "Bouncing balls", source: "examples/toss/game.fun" },
  {
    id: "mario",
    label: "Platformer",
    source: "examples/mario/game.fun",
    siblings: [
      { source: "examples/mario/assets.fun", output: "examples/assets.fun" },
    ],
    assets: [
      { source: "examples/mario/ground.png", output: "ground.png" },
      { source: "examples/mario/hero-atlas.png", output: "hero-atlas.png" },
    ],
  },
  // Single-file, and every model is an absolute Babylon CDN URL — the wasm
  // runtime fetch()es those cross-origin (CORS-permitting), so unlike the
  // local-asset examples this one runs in the single-buffer sandbox.
  { id: "loading", label: "CDN assets", source: "examples/loading/game.fun" },
  { id: "monitor", label: "Render targets", source: "examples/monitor/game.fun" },
];
