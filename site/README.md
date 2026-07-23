# site/ — functor's website + Functor Lang sandbox + IDE

A fully static site: landing page (whose hero background is `examples/hero.fun`
interpreted live by the wasm runtime), a hand-authored **manual**, a generated
**API reference**, a single-file Functor Lang **sandbox**, and a multi-file
**IDE**. Both editors push edits into the running game over the
postMessage seam the VSCode live-preview panel uses, hot-reloading with the
model preserved.

```sh
wasm-pack build runtime/functor-runtime-web --target=web   # once (or npm run build:cli)
npm run site:build       # bundle editors + copy runtime/examples into site/dist
npm run site:serve       # http://127.0.0.1:8123
npm run test:site        # headless e2e — sandbox (e2e/site-sandbox.mjs)
npm run test:ide         # headless e2e — the set-project seam (e2e/ide-project.mjs)
npm run test:ide-page    # headless e2e — the IDE page (e2e/ide-page.mjs)
```

- `player.html` — the runtime host page; the sibling of the CLI dev server's
  `index-functor-lang.html`, but the `.fun` entry comes from `?game=` (one file) or
  `?project=inline` (the IDE pushes the whole file set by postMessage). Keep its
  input mapping and set-source/set-project seam in sync with that page.
- `sandbox.html` / `src/sandbox.js` — the single-buffer editor over a served
  example (pushes `functor-lang-set-source`).
- `ide.html` / `src/ide.js` — the multi-file IDE: a file sidebar, per-file
  editing, a live preview fed the whole project via `functor-lang-set-project`
  (`src/project-bridge.js`), localStorage persistence, and project download as a
  `.zip` (`src/zip.js`, a store-only writer). Asset (`.glb`/audio) management is a
  follow-up.
- `src/functor-lang.js` — the Functor Lang CodeMirror language + synthwave theme,
  shared by both editors.
- `manual/index.html` — getting started, the game contract, language principles,
  and topic guides. Runnable examples link directly into the sandbox.
- `docs/index.html` / `src/api-docs.js` — searchable API reference rendered from
  gitignored `generated/api-reference.json`. `site:build` regenerates it from
  the embedded prelude; `npm run generate:docs` also creates the local Markdown.
- `docs.html` — compatibility redirect to the manual, preserving old anchors.
- `src/examples.js` is the single source of truth for the sandbox's example set
  (id + dropdown label + repo source path). `build.mjs` copies each entry's
  `game.fun` at build time and `src/sandbox.js` builds the dropdown from the same
  list, so the sandbox dropdown always matches what ships in the repo.
- Deploy: publish `site/dist` to any static host.
