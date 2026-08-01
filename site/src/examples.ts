// The single source of truth for the sandbox's example set. Two consumers read
// this one list, so it can't drift:
//   - build.mjs (Node)   uses `id` + `source` to copy the .fun into
//     dist/examples/<id>.fun at build time;
//   - sandbox.ts (browser) uses `id` + `label` to build the scene dropdown.
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
/** A file copied verbatim into the built site at `output` (relative to dist). */
export interface ExampleCopy {
  /** Path relative to the repo root. */
  source: string;
  /** Destination path relative to the site root. */
  output: string;
}

export interface Example {
  id: string;
  label: string;
  /** Path relative to the repo root; copied to `examples/<id>.fun`. */
  source: string;
  /** Explicitly disable the default captured game mouse input. */
  mouseCapture?: false;
  /** Keep an absolute system pointer over the game surface. */
  cursor?: "visible";
  /** Sibling .fun modules the project needs (`file = module`). */
  siblings?: ExampleCopy[];
  /** Local binary assets the project's `Asset.*` locators resolve to. */
  assets?: ExampleCopy[];
  /** Extra files copied into the static site, but not uploaded as game assets. */
  siteFiles?: ExampleCopy[];
  /**
   * Marks a sample that declares a server entry point (the functor.json
   * `entries` shape, like examples/mp) or is otherwise structured for
   * multiple clients (like examples/orbs) — the sandbox shows its CLIENTS
   * control only for those.
   */
  multiplayer?: boolean;
  /**
   * The entry-point binding prefix of the role the sandbox plays (same-file
   * entries, like examples/orbs's `client` role resolving
   * clientInit/clientTick/…). Passed to the player as `?prefix=`.
   */
  prefix?: string;
}

/**
 * The dist path of an example's editable entry. The runtime derives the entry
 * MODULE name from the file stem, and stems must be identifiers — so a
 * hyphenated id maps to an underscore filename (`my-game` →
 * `examples/my_game.fun`). Both consumers (build.mjs copies, sandbox.tsx
 * fetches) derive the path through this one helper so they can't drift.
 */
export const exampleEntryPath = (id: string): string =>
  `examples/${encodeURIComponent(id.replace(/-/g, "_"))}.fun`;

export const EXAMPLES: Example[] = [
  { id: "hero", label: "Neon grid", source: "site/examples/hero.fun" },
  { id: "orbit", label: "Orbit", source: "site/examples/orbit.fun" },
  // Single-file + a CORS-friendly CDN model (jsDelivr mirror of BabylonJS/Assets),
  // so the rigged character streams and animates in the single-buffer sandbox.
  { id: "batteries", label: "Animation blend", source: "site/examples/batteries.fun" },
  // The canonical head-look sample, paired with a site-only manifest whose
  // model locator is the same Xbot asset on a CORS-friendly CDN.
  {
    id: "animation",
    label: "Head look",
    source: "examples/animation/game.fun",
    cursor: "visible",
    siblings: [
      {
        source: "site/examples/animation/assets.fun",
        output: "examples/animation/assets.fun",
      },
    ],
  },
  // The canonical no-clips hand-posing sample and its checked-in SteamVR
  // glove asset.
  {
    id: "glove",
    label: "Hand posing",
    source: "examples/glove/game.fun",
    siblings: [
      {
        source: "examples/glove/assets.fun",
        output: "examples/glove/assets.fun",
      },
    ],
    assets: [
      {
        source: "examples/glove/vr_glove_model.glb",
        output: "vr_glove_model.glb",
      },
      {
        source: "examples/glove/LICENSE.steamvr",
        output: "LICENSE.steamvr",
      },
    ],
  },
  // The stress test: quadtree terrain, four tiled detail maps, instanced grass,
  // a physics heightfield and a walker. Detail maps are absolute Poly Haven
  // URLs because the desktop sample gitignores its fetched copies, so there is
  // nothing in the repo for the build to copy; the heightmap IS checked in.
  {
    id: "terrain",
    label: "Terrain (stress test)",
    source: "site/examples/terrain.fun",
    assets: [
      { source: "examples/terrain/heightmap.png", output: "heightmap.png" },
    ],
  },
  { id: "counter", label: "Counter", source: "examples/counter/game.fun" },
  { id: "primitives", label: "Primitives", source: "examples/primitives/game.fun" },
  { id: "ui", label: "UI widgets", source: "examples/ui/game.fun" },
  { id: "inspector", label: "Inspector", source: "examples/inspector/game.fun" },
  // Named `bounce` (not `physics`): the flat copy makes `file = module`, and a
  // module literally named `Physics` collides with the builtin/prelude namespace.
  { id: "bounce", label: "Physics", source: "examples/physics/game.fun" },
  // The minimal multiplayer-mechanics sample in ONE module (banner sections:
  // PROTOCOL / SERVER / BOT / CLIENT), so the wire ADT and the authoritative
  // claim resolution are right there in the editable buffer.
  {
    id: "orbs",
    label: "Orbs (multiplayer)",
    source: "examples/orbs/game.fun",
    multiplayer: true,
    // The sandbox plays the `client` role of the same-file entries —
    // functor.json maps it to { "file": "game.fun", "prefix": "client" }.
    prefix: "client",
  },
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
    siteFiles: [
      // The landing hero replays this exact desktop verification drive behind
      // its boot loader. It is site orchestration, not a runtime game asset.
      { source: "examples/mario/jump.script", output: "examples/mario.jump.script" },
    ],
  },
  // Single-file, and every model is an absolute Babylon CDN URL — the wasm
  // runtime fetch()es those cross-origin (CORS-permitting), so unlike the
  // local-asset examples this one runs in the single-buffer sandbox.
  { id: "loading", label: "CDN assets", source: "examples/loading/game.fun" },
  { id: "monitor", label: "Render targets", source: "examples/monitor/game.fun" },
];
