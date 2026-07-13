# site/ — functor's website + Functor Lang sandbox + IDE

A fully static site: landing page (whose hero background is `examples/hero.fun`
interpreted live by the wasm runtime), a single-file Functor Lang **sandbox**,
and a multi-file **IDE**. Both editors push edits into the running game over the
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
- `build.mjs` copies selected repo examples' `game.fun` sources (see the map in `build.mjs`)
  at build time, so the sandbox dropdown always matches what ships in the repo.
- Deploy: publish `site/dist` to any static host.
