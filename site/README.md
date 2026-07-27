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
- `sandbox.html` / `src/sandbox.tsx` — the single-buffer editor over a served
  example (pushes `functor-lang-set-source`).
- `ide.html` / `src/ide.tsx` — the multi-file IDE: a file sidebar, per-file
  editing, a live preview fed the whole project via `functor-lang-set-project`
  (`src/project-bridge.ts`), localStorage persistence, and project download as a
  `.zip` (`src/zip.ts`, a store-only writer). Asset (`.glb`/audio) management is a
  follow-up.
- `src/runtime-target-core.ts` + `src/components/RuntimeTargetPanel.tsx` — the
  shared external-runtime link in both editors (controller and view).
  The first explicit push uses `/load-project` (fresh `init`); later edits use
  `/reload-project` (model preserved), with `/state` telemetry and `/capture`
  shown in the panel. Sandbox examples also upload their declared local assets
  before source, then finalize deletions after source is accepted; binary asset
  management in the multi-file IDE remains a follow-up. To link a Quest, keep
  its adb forward on `8123` and serve the site on another loopback port:
  `npm run site:serve -- --port 8124`. The runtime intentionally rejects
  non-loopback browser origins because its code-push API has no authentication.
  Current Chromium may ask once for local-network access when the link starts.
- `src/functor-lang.ts` — the Functor Lang CodeMirror language + synthwave theme,
  shared by both editors.
- `manual/index.html` — getting started, the game contract, language principles,
  and topic guides. Runnable examples link directly into the sandbox.
- `manual/debug-runtime/index.html` — the deterministic capture and HTTP-driving
  workflow. The build also publishes the exhaustive repository contracts at
  `/manual/debug-runtime.md` and `/manual/cli-output.md`.
- `docs/index.html` / `src/api-reference-html.mjs` / `src/api-docs.ts` — the API
  reference. `site:build` regenerates gitignored `generated/api-reference.json`
  from the embedded prelude, then **prerenders** it into the page with
  `src/api-reference-html.mjs`; `src/api-docs.ts` only adds search/filter on top.
  The page is therefore complete without JavaScript — a plain `curl` (or an LLM
  agent) sees every signature. The build also publishes machine-readable mirrors
  at `/docs/api.json`, `/docs/api.md`, and an `/llms.txt` index (llmstxt.org).
  `npm run generate:docs` additionally writes the local `docs/api-reference.md`.
- `docs.html` — compatibility redirect to the manual, preserving old anchors.
- `src/examples.ts` is the single source of truth for the sandbox's example set
  (id + dropdown label + repo source path). `build.mjs` copies each entry's
  `game.fun` at build time and `src/sandbox.tsx` builds the dropdown from the same
  list, so the sandbox dropdown always matches what ships in the repo.
- Deploy: publish `site/dist` to any static host.
