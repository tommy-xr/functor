# VS Code E2E harness

Headless end-to-end tests that exercise the **actual** Functor Lang VS Code
extension inside a real VS Code, driven by Playwright's `_electron`.
`@vscode/test-electron` is used **only** to download the VS Code binary; the
driver is Playwright, asserting on the real workbench / Monaco DOM (no mocks, no
pixel scraping).

The first test (`inspector-inlay.spec.mjs`) proves the paused-scene inspector's
core integration end to end: a wire-contract trace delivered to the extension
makes **live-value inlay hints** appear in the editor for a `.fun` file.

## How it works

1. `baseTest.mjs` launches VS Code as an Electron app with
   `--extensionDevelopmentPath` pointing at this extension and opens
   `examples/inspector` as the workspace, in an isolated `--user-data-dir` /
   `--extensions-dir`. It writes a canned trace whose per-file **sha256 matches
   the on-disk `game.fun`** (the LSP's hash gate) and arms the inject seam via
   env (`FUNCTOR_LANG_TEST_HOOKS=1`, `FUNCTOR_INSPECTOR_TEST_TRACE=<file>`).
2. The test opens `game.fun` (Quick Open) and runs
   `Functor Lang: [test] Inject Inspector Trace` (Command Palette). That
   test-only command — registered **only** when `FUNCTOR_LANG_TEST_HOOKS=1` —
   forwards the trace through the exact
   `client.sendNotification("functor/inspector/trace", …)` call the real preview
   relay uses.
3. The LSP ingests the trace and serves hash-gated live inlay hints; the test
   asserts the sentinel value (`= 42`) renders inside `.monaco-editor`.

## Prerequisites

- **`functor-lang-lsp` on PATH** — the extension launches it as its language
  server. Install it:
  ```sh
  cargo install --path tools/functor-lang-lsp   # -> ~/.cargo/bin/functor-lang-lsp
  # or: npm run build:lsp   (from the repo root)
  ```
- **The extension's own dependencies** (`vscode-languageclient`) — without them
  the extension module fails to load and never activates:
  ```sh
  npm install --prefix tools/functor-lang-vscode
  ```
- **Network on first run** to download the VS Code binary (cached under
  `.vscode-test/` afterwards, ~270MB).
- The repo's root `node_modules` (Playwright + `@vscode/test-electron`); run
  `npm install` at the repo root if needed.

## Run it (headless)

From the repo root:
```sh
npm run test:vscode-e2e
```
or from `tools/functor-lang-vscode`:
```sh
npm run test:e2e
```

It runs with **no human interaction**. On Linux/CI, VS Code (Electron) still
needs a display even when unattended — wrap with `xvfb-run -a`:
```sh
xvfb-run -a npm run test:vscode-e2e
```

## Growth path

`preview-webview.spec.mjs` (currently `test.skip`) scaffolds the fuller
integration — driving the live-preview **webview**, pausing a real wasm frame,
and relaying the runtime's `functor-inspector-trace` postMessage to the LSP. It
depends on the separate `inspector-wasm-emit` work (the wasm runtime does not
emit inspector traces yet). Un-skip it once that lands; prefer asserting the
resulting editor inlay hints over scraping the game-iframe DOM.
