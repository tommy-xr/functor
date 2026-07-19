# Functor Lang for VSCode

Language support for `.fun` source files and `.funi` interface files
(docs/functor-lang.md Track D):

- **Syntax highlighting** — TextMate grammar (`syntaxes/functor-lang.tmLanguage.json`).
- **Diagnostics** — parse/lower errors as you type, via the `functor-lang-lsp` language
  server (`tools/functor-lang-lsp` in this repo) speaking LSP over stdio.
- **Live preview** — the **"Functor Lang: Open Live Preview"** command (docs/functor-lang.md D4)
  serves the active file's project with `functor run wasm` in a webview panel
  beside the editor, and hot-reloads it from the **live buffer** (unsaved
  included, ~300ms debounce) with the model preserved — type, and the running
  game updates without losing state. A broken edit keeps the old program
  running; push results land in the status bar (errors also in the
  "Functor Lang Preview" output channel). Needs the `functor` CLI on PATH (or point
  the `functor-lang.functorPath` setting at the binary).

## The `functor-lang-lsp` language server

Resolution order (see `client/server-path.js`): the `functor-lang.serverPath`
setting if set, else the binary **bundled inside the platform VSIX** (released
builds ship one per platform under `server/`), else **PATH**. A dev checkout
packaged locally has no bundled binary, so build and install the server from
the repo root:

```sh
cargo install --path tools/functor-lang-lsp
# or: cargo build -p functor-lang-lsp && ln -s "$PWD/target/debug/functor-lang-lsp" ~/.local/bin/
```

Without a resolvable server, highlighting still works but diagnostics are
silently absent (VSCode reports the failed server launch in the Output panel).

## Install

```sh
cd tools/functor-lang-vscode
npm install              # fetches the extension dependencies
npm run install:vsix     # packages with pinned vsce, then installs the VSIX
```

Run `npm run package:vsix` instead when you only want to build the VSIX.

## Develop (F5 dev host)

Open `tools/functor-lang-vscode/` as the workspace folder in VSCode, run `npm install`
once, then press **F5** ("Run Extension") to launch an Extension Development
Host with the extension loaded. Open any `.fun` or `.funi` file — e.g.
`test/sample.fun`, `functor-lang/examples/*.fun`, or `functor-prelude/prelude/*.funi`.

## Grammar: regenerate and test

The grammar is hand-written JSON — edit `syntaxes/functor-lang.tmLanguage.json`
directly (there is no generation step). Two levels of verification:

- **Automated sanity:** `cargo test -p functor-lang-lsp` checks the grammar,
  `language-configuration.json`, and `package.json` are valid JSON, and that
  `test/sample.fun` (which exercises every construct) still parses and lowers
  with the current `functor_lang` crate.
- **Visual verification happens in the editor:** open `test/sample.fun` in the
  dev host and eyeball the scopes with
  `Developer: Inspect Editor Tokens and Scopes`.
