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
   * `entries` shape, like examples/orbs) or is otherwise structured for
   * multiple clients — the sandbox shows its CLIENTS control only for those.
   */
  multiplayer?: boolean;
  /**
   * The entry-point binding prefix of the role the sandbox plays, for a
   * same-file-entries sample whose role is declared as
   * `{ "file": …, "prefix": "client" }` (resolving clientInit/clientTick/…).
   * Passed to the player as `?prefix=`. Mutually exclusive with `module`.
   */
  prefix?: string;
  /**
   * The inline entry MODULE of the role the sandbox plays, for a
   * same-file-entries sample whose role is declared as
   * `{ "file": …, "module": "Client" }` (resolving Client.init/Client.tick/…).
   * Passed to the player as `?module=`. Mutually exclusive with `prefix`.
   */
  module?: string;
  /**
   * The sample's SERVER role, as a dist path in its own file list. The pane
   * grid mounts a server pane for every example that declares one — booted
   * from the same project files with this file as the entry — and the host's
   * net coordinator routes the client panes' traffic to it. The editable
   * `source` stays the CLIENT entry.
   *
   * The file must ALSO appear in `siblings` — that is what copies it into the
   * built site and puts it in the fetched project file list; naming a file
   * here that nothing copies would 404 the server pane — UNLESS it is the
   * example's own entry, which is copied and listed already.
   *
   * The server pane derives its params from the client's, so the server
   * ROLE must be stated here rather than inherited: `module` when it is an
   * inline entry module in a shared file (orbs: `{ "file": …, "module":
   * "Server" }`, carried as `?module=`), or `prefix` for the transitional
   * prefixed-binding form (`serverInit`/…, carried as `?prefix=`). At most
   * one of the two.
   */
  server?: { file: string; module?: string; prefix?: string };
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
  // Single-file and asset-free: an L-system garden whose growth LAWS are
  // top-level constants, so editing one in the sandbox re-shapes the plants
  // already on screen instead of restarting them.
  {
    id: "code-garden",
    label: "Code garden",
    source: "examples/code-garden/game.fun",
    // Keyboard-only: there is nothing to steer with the pointer, so don't
    // capture it (matches the sample's own functor.json).
    mouseCapture: false,
  },
  { id: "counter", label: "Counter", source: "examples/counter/game.fun" },
  { id: "primitives", label: "Primitives", source: "examples/primitives/game.fun" },
  { id: "ui", label: "UI widgets", source: "examples/ui/game.fun" },
  { id: "inspector", label: "Inspector", source: "examples/inspector/game.fun" },
  // Named `bounce` (not `physics`): the flat copy makes `file = module`, and a
  // module literally named `Physics` collides with the builtin/prelude namespace.
  { id: "bounce", label: "Physics", source: "examples/physics/game.fun" },
  // The minimal multiplayer-mechanics sample in ONE file (banner sections:
  // PROTOCOL / THE WORLD / PRESENTATION / CLIENT / SERVER — the last two the
  // roles' own `module` blocks), so the wire ADT and the authoritative claim
  // resolution are right there in the editable buffer.
  {
    id: "orbs",
    label: "Orbs (multiplayer)",
    source: "examples/orbs/game.fun",
    multiplayer: true,
    // Both roles are inline MODULES of the ONE editable buffer: the sandbox
    // plays `module Client` (`?module=Client`), and the server pane re-enters
    // the same file at `module Server` (`?module=Server`).
    module: "Client",
    server: { file: exampleEntryPath("orbs"), module: "Server" },
  },
  {
    id: "platformer",
    label: "Platformer",
    source: "examples/platformer/game.fun",
    siblings: [
      { source: "examples/platformer/assets.fun", output: "examples/assets.fun" },
    ],
    assets: [
      { source: "examples/platformer/ground.png", output: "ground.png" },
      { source: "examples/platformer/hero-atlas.png", output: "hero-atlas.png" },
    ],
    siteFiles: [
      // The landing hero replays this exact desktop verification drive behind
      // its boot loader. It is site orchestration, not a runtime game asset.
      { source: "examples/platformer/jump.script", output: "examples/platformer.jump.script" },
    ],
  },
  // Single-file, and every model is an absolute Babylon CDN URL — the wasm
  // runtime fetch()es those cross-origin (CORS-permitting), so unlike the
  // local-asset examples this one runs in the single-buffer sandbox.
  { id: "loading", label: "CDN assets", source: "examples/loading/game.fun" },
  { id: "monitor", label: "Render targets", source: "examples/monitor/game.fun" },
];
