# MLE for VSCode

Language support for `.mle` files (docs/mle.md Track D):

- **Syntax highlighting** — TextMate grammar (`syntaxes/mle.tmLanguage.json`).
- **Diagnostics** — parse/lower errors as you type, via the `mle-lsp` language
  server (`tools/mle-lsp` in this repo) speaking LSP over stdio.
- **Live preview** — the **"MLE: Open Live Preview"** command (docs/mle.md D4)
  serves the active file's project with `functor run wasm` in a webview panel
  beside the editor, and hot-reloads it from the **live buffer** (unsaved
  included, ~300ms debounce) with the model preserved — type, and the running
  game updates without losing state. A broken edit keeps the old program
  running; push results land in the status bar (errors also in the
  "MLE Preview" output channel). Needs the `functor` CLI on PATH (or point
  the `mle.functorPath` setting at the binary).

## Prerequisite: `mle-lsp` on PATH

The extension launches the `mle-lsp` **binary from your PATH** — it is not
bundled. Build and install it from the repo root:

```sh
cargo install --path tools/mle-lsp
# or: cargo build -p mle-lsp && ln -s "$PWD/target/debug/mle-lsp" ~/.local/bin/
```

Without it, highlighting still works but diagnostics are silently absent
(VSCode reports the failed server launch in the Output panel).

## Install

```sh
cd tools/mle-vscode
npm install                      # fetches vscode-languageclient
npx @vscode/vsce package         # produces mle-vscode-0.1.0.vsix
code --install-extension mle-vscode-0.1.0.vsix
```

## Develop (F5 dev host)

Open `tools/mle-vscode/` as the workspace folder in VSCode, run `npm install`
once, then press **F5** ("Run Extension") to launch an Extension Development
Host with the extension loaded. Open any `.mle` file — e.g.
`test/sample.mle` or `mle/examples/*.mle`.

## Grammar: regenerate and test

The grammar is hand-written JSON — edit `syntaxes/mle.tmLanguage.json`
directly (there is no generation step). Two levels of verification:

- **Automated sanity:** `cargo test -p mle-lsp` checks the grammar,
  `language-configuration.json`, and `package.json` are valid JSON, and that
  `test/sample.mle` (which exercises every construct) still parses and lowers
  with the current `mle` crate.
- **Visual verification happens in the editor:** open `test/sample.mle` in the
  dev host and eyeball the scopes with
  `Developer: Inspect Editor Tokens and Scopes`.
