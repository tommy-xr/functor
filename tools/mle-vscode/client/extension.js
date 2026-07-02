// MLE extension entry point: start an LSP client for `.mle` documents,
// launching the `mle-lsp` binary from PATH (see ../README.md for how to get
// it there). Plain JS on purpose — no bundler or TS build step; the only
// runtime dependency is vscode-languageclient.
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate() {
  client = new LanguageClient(
    "mle",
    "MLE Language Server",
    // Resolved from PATH; stdio is the vscode-languageclient default.
    { command: "mle-lsp" },
    { documentSelector: [{ language: "mle" }] }
  );
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
