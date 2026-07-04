// MLE extension entry point: an LSP client for `.mle` documents (launching
// the `mle-lsp` binary from PATH — see ../README.md for how to get it there)
// and the live game preview panel (docs/mle.md D4). Plain JS on purpose — no
// bundler or TS build step; the only runtime dependency is
// vscode-languageclient.
const vscode = require("vscode");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { LanguageClient } = require("vscode-languageclient/node");

let client;
// The open preview panel, if any. A singleton: the dev server owns a fixed
// port, so a second panel would race the first for it — and closing either
// panel would kill the server out from under the other. Re-running the
// command reveals the existing panel instead.
let previewPanel;
// The preview's dev-server child process — module-scoped so deactivate()
// can kill it even if the panel outlives the command's closure (extension
// reload/disable with the panel open must not orphan the server).
let previewChild;

// Where `functor run wasm` serves the game — fixed by the CLI's dev server.
const PREVIEW_URL = "http://127.0.0.1:8080";
const PREVIEW_ORIGIN = new URL(PREVIEW_URL).origin;
// Edits push the full live buffer after this quiet period; the reload itself
// is ~1ms in the runtime, so this is the whole edit→preview latency.
const PUSH_DEBOUNCE_MS = 300;
// How long to wait for the dev server to come up (first run may compile).
const SERVER_WAIT_MS = 30000;

function activate(context) {
  client = new LanguageClient(
    "mle",
    "MLE Language Server",
    // Resolved from PATH; stdio is the vscode-languageclient default.
    { command: "mle-lsp" },
    { documentSelector: [{ language: "mle" }] }
  );
  client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand("mle.openLivePreview", openLivePreview)
  );
}

function deactivate() {
  // The panel usually kills its child on dispose, but extension
  // reload/disable can tear us down with the panel still open — don't
  // orphan the dev server on port 8080.
  if (previewChild) {
    previewChild.kill();
    previewChild = undefined;
  }
  return client ? client.stop() : undefined;
}

// Walk up from an .mle file to the functor.json declaring an MLE project —
// the directory `functor -d <dir>` operates on. Returns { dir, entry } or
// null. A functor.json for another language (or an unreadable one) is
// skipped and the walk continues, so nested projects resolve correctly.
function findMleProject(fromFile) {
  let dir = path.dirname(fromFile);
  for (;;) {
    const manifest = path.join(dir, "functor.json");
    if (fs.existsSync(manifest)) {
      try {
        const json = JSON.parse(fs.readFileSync(manifest, "utf8"));
        if (json.language === "mle" && typeof json.entry === "string") {
          return { dir, entry: json.entry };
        }
      } catch {
        // Unparseable manifest: keep walking.
      }
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

// Poll the dev server until it answers (the CLI may still be starting up).
function waitForServer(timeoutMs) {
  return new Promise((resolve) => {
    const deadline = Date.now() + timeoutMs;
    const poll = () => {
      const req = http.get(PREVIEW_URL, { timeout: 2000 }, (res) => {
        res.resume();
        resolve(true);
      });
      // A connected-but-silent socket must not stall the poll past the
      // deadline; destroy() surfaces as the error event below.
      req.on("timeout", () => req.destroy());
      req.on("error", () => {
        if (Date.now() > deadline) resolve(false);
        else setTimeout(poll, 300);
      });
    };
    poll();
  });
}

// "MLE: Open Live Preview" — serve the active file's project with
// `functor run wasm` and host the running game in a webview panel that
// hot-reloads from the LIVE buffer (unsaved included), model preserved.
async function openLivePreview() {
  if (previewPanel) {
    previewPanel.reveal();
    return;
  }
  const editor = vscode.window.activeTextEditor;
  if (!editor || !editor.document.fileName.endsWith(".mle")) {
    vscode.window.showErrorMessage(
      "MLE: open the project's .mle file first — the preview serves the project it belongs to."
    );
    return;
  }
  const project = findMleProject(editor.document.fileName);
  if (!project) {
    vscode.window.showErrorMessage(
      `MLE: no functor.json with "language": "mle" found in any directory above ` +
        `${editor.document.fileName} — create one ({"language": "mle", "entry": "game.mle"}) ` +
        "in the project directory."
    );
    return;
  }
  const entryPath = path.resolve(project.dir, project.entry);

  // The dev server child. If 8080 is already served (a second panel, or a
  // manual `functor run wasm`) this child exits with a bind error — logged
  // to the output channel — and the panel simply attaches to the running
  // server.
  const functorPath =
    vscode.workspace.getConfiguration("mle").get("functorPath") || "functor";
  const output = vscode.window.createOutputChannel("MLE Preview");
  // The child outlives the panel by a beat (kill() on dispose, exit later),
  // so every output write is gated on the panel still being alive — VSCode
  // throws on appending to a disposed channel.
  let disposed = false;
  const log = (text) => {
    if (!disposed) output.append(text);
  };
  const child = spawn(functorPath, ["-d", project.dir, "run", "wasm", "--no-open"], {
    cwd: project.dir,
  });
  previewChild = child;
  child.stdout.on("data", (d) => log(d.toString()));
  child.stderr.on("data", (d) => log(d.toString()));
  child.on("error", (e) => {
    vscode.window.showErrorMessage(
      `MLE: cannot start "${functorPath}" (${e.message}) — set mle.functorPath to the functor CLI binary.`
    );
  });
  child.on("exit", (code) => log(`[functor exited with code ${code}]\n`));

  const panel = vscode.window.createWebviewPanel(
    "mleLivePreview",
    `MLE Live Preview — ${path.basename(project.dir)}`,
    vscode.ViewColumn.Beside,
    // Scripts run the bridge below; retainContextWhenHidden keeps the game
    // (and its model) alive when the panel is tabbed away.
    { enableScripts: true, retainContextWhenHidden: true }
  );
  previewPanel = panel;
  panel.webview.html = previewHtml();

  // Push results surface here, non-modally: green check on a good reload,
  // the load error on a broken one (the old program keeps running).
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  // Readiness is the page's own announcement (origin-checked in the
  // bridge): it proves 8080 is really the MLE preview — anything else
  // answering HTTP there never sends it — and it's the moment to flush the
  // CURRENT buffer, so edits made before the runtime came up (or while the
  // file sat unsaved) are not dropped.
  let ready = false;
  let readyTimeout;
  const pushCurrentBuffer = async () => {
    try {
      const doc = await vscode.workspace.openTextDocument(entryPath);
      panel.webview.postMessage({ type: "mle-set-source", source: doc.getText() });
    } catch (e) {
      log(`[push] cannot read ${entryPath}: ${e}\n`);
    }
  };
  panel.webview.onDidReceiveMessage((msg) => {
    if (!msg) return;
    if (msg.type === "mle-preview-ready") {
      if (ready) return;
      ready = true;
      clearTimeout(readyTimeout);
      pushCurrentBuffer();
      return;
    }
    if (msg.type !== "mle-set-source-result") return;
    if (msg.ok) {
      status.text = "$(check) MLE preview: reloaded";
      status.tooltip = msg.message;
    } else {
      status.text = "$(error) MLE preview: push failed";
      status.tooltip = msg.message;
      log(`[push] ${msg.message}\n`);
    }
    status.show();
  });

  // Push the full live buffer (unsaved included) on every edit to the entry
  // file, debounced. The runtime keeps the model across the swap, so state
  // survives typing; a broken buffer keeps the old program running.
  let debounce;
  const changeSub = vscode.workspace.onDidChangeTextDocument((e) => {
    if (path.resolve(e.document.fileName) !== entryPath) return;
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      panel.webview.postMessage({ type: "mle-set-source", source: e.document.getText() });
    }, PUSH_DEBOUNCE_MS);
  });

  panel.onDidDispose(() => {
    disposed = true;
    previewPanel = undefined;
    previewChild = undefined;
    clearTimeout(debounce);
    clearTimeout(readyTimeout);
    changeSub.dispose();
    status.dispose();
    child.kill();
    output.dispose();
  });

  // Point the iframe at the dev server once it answers (the webview shows
  // "starting…" until then).
  const up = await waitForServer(SERVER_WAIT_MS);
  if (disposed) return;
  if (up) {
    panel.webview.postMessage({ type: "mle-preview-navigate", url: PREVIEW_URL });
    readyTimeout = setTimeout(() => {
      if (ready || disposed) return;
      vscode.window.showErrorMessage(
        `MLE: ${PREVIEW_URL} answered but never announced the MLE preview — ` +
          `is something else using that port?`
      );
    }, SERVER_WAIT_MS);
  } else {
    vscode.window.showErrorMessage(
      `MLE: the functor dev server did not come up at ${PREVIEW_URL} — see the "MLE Preview" output.`
    );
  }
}

// The panel document: a full-size iframe hosting the game page, plus the
// message bridge — extension → webview → iframe for source pushes, and
// iframe → webview → extension for results. The game page itself does the
// reload (see index-mle.html's mle-set-source listener in the runtime).
function previewHtml() {
  return `<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy"
          content="default-src 'none'; frame-src ${PREVIEW_URL}; script-src 'unsafe-inline'; style-src 'unsafe-inline'" />
    <style>
      html, body { margin: 0; padding: 0; width: 100%; height: 100%; overflow: hidden; }
      #frame { border: 0; width: 100%; height: 100%; display: none; }
      #waiting { font-family: sans-serif; padding: 1em; opacity: 0.7; }
    </style>
  </head>
  <body>
    <div id="waiting">Starting the functor dev server…</div>
    <iframe id="frame"></iframe>
    <script>
      const vscode = acquireVsCodeApi();
      const frame = document.getElementById("frame");
      // Messages from the extension and from the iframe arrive on the same
      // window listener; the disjoint "type" fields route them.
      window.addEventListener("message", (event) => {
        const data = event.data;
        if (!data) return;
        if (data.type === "mle-preview-navigate") {
          frame.src = data.url;
          frame.style.display = "block";
          document.getElementById("waiting").style.display = "none";
        } else if (data.type === "mle-set-source") {
          // Extension → game page (the page only accepts pushes from its
          // parent — us).
          if (frame.contentWindow) frame.contentWindow.postMessage(data, "*");
        } else if (data.type === "mle-set-source-result" || data.type === "mle-preview-ready") {
          // Game page → extension. Only trust the page we framed: anything
          // else on that port (or the game code itself) must not spoof
          // results or readiness.
          if (event.source !== frame.contentWindow) return;
          if (event.origin && event.origin !== "${PREVIEW_ORIGIN}") return;
          vscode.postMessage(data);
        }
      });
    </script>
  </body>
</html>`;
}

module.exports = { activate, deactivate };
