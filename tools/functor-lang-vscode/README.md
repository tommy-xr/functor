# Functor Lang for VSCode

Language support for `.functor` files (docs/functor-lang.md Track D):

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

## Prerequisite: `functor-lang-lsp` on PATH

The extension launches the `functor-lang-lsp` **binary from your PATH** — it is not
bundled. Build and install it from the repo root:

```sh
cargo install --path tools/functor-lang-lsp
# or: cargo build -p functor-lang-lsp && ln -s "$PWD/target/debug/functor-lang-lsp" ~/.local/bin/
```

Without it, highlighting still works but diagnostics are silently absent
(VSCode reports the failed server launch in the Output panel).

## Install

```sh
cd tools/functor-lang-vscode
npm install                      # fetches vscode-languageclient
npx @vscode/vsce package         # produces functor-lang-vscode-0.1.0.vsix
code --install-extension functor-lang-vscode-0.1.0.vsix
```

## Develop (F5 dev host)

Open `tools/functor-lang-vscode/` as the workspace folder in VSCode, run `npm install`
once, then press **F5** ("Run Extension") to launch an Extension Development
Host with the extension loaded. Open any `.functor` file — e.g.
`test/sample.functor` or `functor-lang/examples/*.functor`.

## Grammar: regenerate and test

The grammar is hand-written JSON — edit `syntaxes/functor-lang.tmLanguage.json`
directly (there is no generation step). Two levels of verification:

- **Automated sanity:** `cargo test -p functor-lang-lsp` checks the grammar,
  `language-configuration.json`, and `package.json` are valid JSON, and that
  `test/sample.functor` (which exercises every construct) still parses and lowers
  with the current `functor_lang` crate.
- **Visual verification happens in the editor:** open `test/sample.functor` in the
  dev host and eyeball the scopes with
  `Developer: Inspect Editor Tokens and Scopes`.
