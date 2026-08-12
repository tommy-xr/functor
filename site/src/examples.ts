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
// `{ source, output }` copies; the sandbox loads the complete file list and its
// sidebar can open any of them, with `source` the entry every pane boots at.
// Asset outputs are relative to the site root because browser fetches resolve
// `Asset.*` locators against player.html.
/** A file copied verbatim into the built site at `output` (relative to dist). */
export interface ExampleCopy {
  /** Path relative to the repo root. */
  source: string;
  /** Destination path relative to the site root. */
  output: string;
}

/**
 * Landing-page "box art" metadata for a playable sample. Only the games
 * shortlisted for the gallery carry it; `order` is the carousel's order and
 * must be unique among the entries that declare one.
 *
 * NOTE — the DUAL convention. Several `examples/<id>/game.fun` files carry the
 * same prose as `// gallery:` / `// gallery-controls:` header comments, read by
 * `.claude/skills/game-jam/scripts/build-gallery.mjs` when it builds a jam
 * gallery straight out of project directories. This field is the SITE's copy:
 * the landing page needs prose for samples that have no header (and freedom to
 * tighten a blurb for a card), and build.mjs has no reason to parse `.fun`
 * comments. Nothing enforces that the two agree — if you edit one and the
 * other exists, edit both.
 */
export interface ExampleGallery {
  /** The card's name — the game's own title where it has one. */
  title: string;
  /** One sentence of box-art copy. */
  blurb: string;
  /** Optional "·"-separated control legend. */
  controls?: string;
  /** Position in the carousel (ascending). */
  order: number;
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
  /**
   * Box art for the landing-page games carousel. Its media lives in
   * `site/media/box-art/<id>.{gif,png}`, generated by `npm run demo:box-art`.
   */
  gallery?: ExampleGallery;
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
    gallery: {
      title: "Code Garden",
      blurb:
        "A living procedural garden whose growth laws ARE the source file — edit a constant and the plants you already grew re-shape.",
      controls: "Space plants a seed · 1/2/3 pick a species · R replants",
      order: 8,
    },
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
    gallery: {
      title: "Orbs",
      blurb:
        "The multiplayer reference: rival pilots and an authoritative server, all in ONE file, racing to claim the arena's orbs.",
      controls: "A/D turn · W thrusts · hold Space over an orb to claim it",
      order: 10,
    },
  },
  // The other half of the multiplayer pair: orbs keeps both roles in one
  // buffer, netpong keeps them in separate FILES. So the role is the file
  // here — `server` names server.fun with no module/prefix — and the sandbox
  // edits client.fun. All three of the client entry's siblings must be copied
  // (server.fun is the server pane's OWN entry, and a sibling of the client's):
  // the runtime derives module names from file STEMS, and both roles call the
  // shared renderer as `Game.view`, so a missing game.fun fails every pane with
  // `unknown external 'Game.view'`. They live in a per-example subdirectory so
  // generic stems (`protocol`, `server`, `game`) can't collide with another
  // sample's files.
  {
    id: "netpong",
    label: "Netpong (multiplayer)",
    source: "examples/netpong/client.fun",
    multiplayer: true,
    // Keyboard-only, like the sample's own functor.json.
    mouseCapture: false,
    siblings: [
      { source: "examples/netpong/server.fun", output: "examples/netpong/server.fun" },
      { source: "examples/netpong/protocol.fun", output: "examples/netpong/protocol.fun" },
      { source: "examples/netpong/game.fun", output: "examples/netpong/game.fun" },
    ],
    server: { file: "examples/netpong/server.fun" },
    gallery: {
      title: "Netpong",
      blurb:
        "Authoritative multiplayer Pong — server-owned physics, client prediction, interpolation, and a neon AI attract mode.",
      controls: "W/S or ↑/↓ steer · Space toggles autopilot · R requests a rematch",
      order: 6,
    },
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
    gallery: {
      title: "Platformer",
      blurb:
        "Run, jump, clear the chasm. The whole simulation forward-steps in tick, so every leap rewinds and replays exactly.",
      controls: "A/D or ←/→ run · W/↑/Space jump",
      order: 5,
    },
  },
  // --- Complete games ---------------------------------------------------
  // Single-file and asset-free (every glyph and block is a primitive), so they
  // need no sibling/asset plumbing; each mirrors its own functor.json input
  // policy below.
  {
    id: "tetris",
    label: "Tetris",
    source: "examples/tetris/game.fun",
    gallery: {
      title: "Neon Stack",
      blurb:
        "A deterministic falling-block puzzle: ghost pieces, wall kicks, line-clear flashes, and gravity that keeps rising.",
      controls: "A/D or ←/→ move · W/↑ rotate · S/↓ soft drop · Space hard drop · R restart",
      order: 1,
    },
  },
  {
    id: "breakout",
    label: "Breakout",
    source: "examples/breakout/game.fun",
    // Paddle-follows-pointer: an absolute cursor, like its functor.json.
    cursor: "visible",
    gallery: {
      title: "Neon Breakout",
      blurb:
        "A polished, deterministic brick breaker built entirely from sprite primitives — no physics engine, just pure math.",
      controls: "A/D or ←/→ move · pointer steers · Space/click launches",
      order: 3,
    },
  },
  {
    id: "roguelike",
    label: "Roguelike",
    source: "examples/roguelike/game.fun",
    // Keyboard-only, like its functor.json.
    mouseCapture: false,
    gallery: {
      title: "Neon Depths",
      blurb:
        "A deterministic, turn-based roguelike: fog-of-war corridors, pure enemy AI, and no luck the seed didn't hand you.",
      controls: "Arrows or WASD move/attack · Space waits · R restarts",
      order: 2,
    },
  },
  // The arcade clone: three sibling modules and four checked-in CC0 sounds. The
  // ship model is the one asset NOT in the sample dir — `npm run fetch:assets`
  // unzips it from the Kenney "Space Kit" pack and `examples/asteroids`
  // gitignores `*.glb` — so the site keeps its OWN copy of that same CC0 file
  // (19 KB, like examples/glove's checked-in glb) and copies it to the locator
  // the canonical manifest already names. That keeps the generated
  // `assets.fun` unforked: the site uses the sample's own manifest verbatim.
  {
    id: "asteroids",
    label: "Asteroids",
    source: "examples/asteroids/game.fun",
    siblings: [
      {
        source: "examples/asteroids/assets.fun",
        output: "examples/asteroids/assets.fun",
      },
      { source: "examples/asteroids/lib.fun", output: "examples/asteroids/lib.fun" },
      { source: "examples/asteroids/font.fun", output: "examples/asteroids/font.fun" },
    ],
    assets: [
      // Kenney "Space Kit" v2.0 `Models/GLTF format/craft_racer.glb` (CC0) —
      // byte-identical to what `fetch:assets` unzips for the desktop sample;
      // provenance in examples/asteroids/ASSETS.md.
      { source: "site/examples/asteroids/ship.glb", output: "ship.glb" },
      { source: "examples/asteroids/laser.ogg", output: "laser.ogg" },
      { source: "examples/asteroids/explosion.ogg", output: "explosion.ogg" },
      { source: "examples/asteroids/ship-explosion.ogg", output: "ship-explosion.ogg" },
      { source: "examples/asteroids/thrust-loop.ogg", output: "thrust-loop.ogg" },
    ],
    gallery: {
      title: "Asteroids",
      blurb:
        "The arcade classic, complete: three waves of splitting rocks, thrust and inertia, lives, score, and CC0 sound.",
      controls: "A/D or ←/→ rotate · W/↑ thrusts · Space fires · Enter starts",
      order: 4,
    },
  },
  // The FPS: captured mouse (the sandbox default, and its functor.json's) so
  // free-look aiming and click-to-shoot work in the player.
  {
    id: "shooting-range",
    label: "Shooting range",
    source: "examples/shooting-range/game.fun",
    siblings: [
      {
        source: "examples/shooting-range/assets.fun",
        output: "examples/shooting-range/assets.fun",
      },
    ],
    assets: [
      { source: "examples/shooting-range/shot.ogg", output: "shot.ogg" },
      { source: "examples/shooting-range/hit.ogg", output: "hit.ogg" },
    ],
    gallery: {
      title: "Shooting Range",
      blurb:
        "A first-person range: hitscan raycasts, a full-auto weapon that climbs under sustained fire, and targets that pop.",
      controls: "Mouse looks · left mouse fires · WASD moves · R reloads",
      order: 7,
    },
  },
  // Single-file, with two tiny checked-in textures (`Texture.file` locators
  // resolved against the site root, so they copy to the root like every other
  // asset).
  {
    id: "synthwave",
    label: "Synthwave",
    source: "examples/synthwave/game.fun",
    assets: [
      { source: "examples/synthwave/grid-neon.png", output: "grid-neon.png" },
      { source: "examples/synthwave/sky.png", output: "sky.png" },
    ],
  },
  // --- Runtime showcases -------------------------------------------------
  // Single-file, and every model is an absolute Babylon CDN URL — the wasm
  // runtime fetch()es those cross-origin (CORS-permitting), so unlike the
  // local-asset examples this one runs in the single-buffer sandbox.
  { id: "loading", label: "CDN assets", source: "examples/loading/game.fun" },
  { id: "monitor", label: "Render targets", source: "examples/monitor/game.fun" },
];

/**
 * The games carousel, in card order. Every consumer — build.mjs (which emits
 * `dist/examples/gallery.json`) and site/demos/box-art.mjs (which captures the
 * media) — reads THIS, so the set and its order cannot drift between them.
 *
 * Deliberately just a sort, with no validation: this module is in the SANDBOX
 * bundle too (sandbox.tsx imports it), and a module-eval `throw` here would take
 * a page down over a build-data mistake. build.mjs rejects a duplicate `order`
 * instead, where it is a build error.
 */
export const GALLERY: (Example & { gallery: ExampleGallery })[] = EXAMPLES.filter(
  (example): example is Example & { gallery: ExampleGallery } => example.gallery !== undefined
).sort((a, b) => a.gallery.order - b.gallery.order);
