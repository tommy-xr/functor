# site/ — functor's website + Functor Lang sandbox

A fully static site: landing page (whose hero background is `examples/hero.fun`
interpreted live by the wasm runtime) and an interactive Functor Lang sandbox — a
CodeMirror editor pushing edits into the running game over the same
postMessage seam the VSCode live-preview panel uses, hot-reloading with the
model preserved.

```sh
wasm-pack build runtime/functor-runtime-web --target=web   # once (or npm run build:cli)
npm run site:build     # bundle editor + copy runtime/examples into site/dist
npm run site:serve     # http://127.0.0.1:8123
npm run test:site      # headless e2e (e2e/site-sandbox.mjs)
```

- `player.html` — the runtime host page; the sibling of the CLI dev server's
  `index-functor-lang.html`, but the `.fun` entry comes from `?game=`. Keep its input
  mapping and set-source seam in sync with that page.
- `src/sandbox.js` / `src/functor-lang.js` — the editor page and the Functor Lang
  CodeMirror language + synthwave theme.
- `build.mjs` copies selected repo examples' `game.fun` sources (see the map in `build.mjs`)
  at build time, so the dropdown always matches what ships in the repo.
- Deploy: publish `site/dist` to any static host.
