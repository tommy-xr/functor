// Functor Lang extension entry point: an LSP client for `.fun` documents (launching
// the `functor-lang-lsp` binary from PATH — see ../README.md for how to get it there)
// and the live game preview panel (docs/functor-lang.md D4). Plain JS on purpose — no
// bundler or TS build step; the only runtime dependency is
// vscode-languageclient.
const vscode = require("vscode");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { LanguageClient } = require("vscode-languageclient/node");
// All inspector decision logic (relay filter, attach/detach state, port
// parsing, status bar text) lives in this plain module so it is testable with
// `node --test` (client/inspector.test.js) — extension.js keeps only the thin
// VS Code wiring around it.
const inspector = require("./inspector.js");

let client;
// Resolves once the LanguageClient has started (server launched + initialized).
// Captured only so the test-only inject command below can await readiness before
// sending; production notification paths run long after startup.
let clientStarted;
// The paused-scene inspector's attach state + its status bar item. Attach
// persists server-side until detach, so this is per-session UI state; the
// last-used port is persisted across sessions in globalState.
let inspectorState;
let inspectorStatus;
const INSPECTOR_PORT_KEY = "functor-lang.inspector.port";
// The open preview panel, if any. A singleton: the dev server owns a fixed
// port, so a second panel would race the first for it — and closing either
// panel would kill the server out from under the other. Re-running the
// command for the SAME project reveals the existing panel; for a DIFFERENT
// project it tears this one down first (see openLivePreview).
let previewPanel;
// The preview's dev-server child process — module-scoped so deactivate()
// can kill it even if the panel outlives the command's closure (extension
// reload/disable with the panel open must not orphan the server).
let previewChild;
// Project dir the open preview is serving — used to tell "reveal the existing
// panel" (same project) apart from "restart for a different sample".
let previewProjectDir;

// Where `functor run wasm` serves the game — fixed by the CLI's dev server.
const PREVIEW_URL = "http://127.0.0.1:8080";
const PREVIEW_ORIGIN = new URL(PREVIEW_URL).origin;
// Edits push the full live buffer after this quiet period; the reload itself
// is ~1ms in the runtime, so this is the whole edit→preview latency.
const PUSH_DEBOUNCE_MS = 300;
// How long to wait for the dev server to come up (first run may compile).
const SERVER_WAIT_MS = 30000;

// The extension-wide output channel ("Functor Lang" in the Output panel):
// activation, LSP lifecycle, inspector attach/relay traffic — the first stop
// when diagnosing "why aren't live values showing". The per-preview channel
// ("Functor Lang Preview") stays separate: it carries the dev-server child's
// raw stdout/stderr.
let channel;
const elog = (text) => {
  if (channel) channel.appendLine(`[${new Date().toISOString().slice(11, 23)}] ${text}`);
};

function activate(context) {
  channel = vscode.window.createOutputChannel("Functor Lang");
  context.subscriptions.push(channel);
  elog("extension activated");
  client = new LanguageClient(
    "functor-lang",
    "Functor Lang Language Server",
    // Resolved from PATH; stdio is the vscode-languageclient default.
    { command: "functor-lang-lsp" },
    { documentSelector: [{ language: "functor-lang" }] }
  );
  clientStarted = client.start().then(
    () => elog("language server started (functor-lang-lsp)"),
    (e) => elog(`language server FAILED to start: ${e && e.message ? e.message : e}`)
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("functor-lang.openLivePreview", openLivePreview)
  );

  // --- Test-only inspector-trace inject seam -------------------------------
  // Gated on FUNCTOR_LANG_TEST_HOOKS so it never registers (or shows in the
  // Command Palette — see the `when: functorLangTestHooks` menu entry) in a
  // normal session. The E2E harness (tools/functor-lang-vscode/tests-integration)
  // writes a wire-contract trace JSON to the file named by
  // FUNCTOR_INSPECTOR_TEST_TRACE and invokes this command; we forward it through
  // the SAME client.sendNotification("functor/inspector/trace", …) call the
  // preview relay uses above — a faithful seam, not a fake.
  if (process.env.FUNCTOR_LANG_TEST_HOOKS === "1") {
    // Reveal the command in the palette only in this mode (see the
    // `when: functorLangTestHooks` menu entry in package.json).
    vscode.commands.executeCommand("setContext", "functorLangTestHooks", true);
    context.subscriptions.push(
      vscode.commands.registerCommand("functor-lang.inspector._injectTrace", async () => {
        const file = process.env.FUNCTOR_INSPECTOR_TEST_TRACE;
        if (!file || !client) return;
        const doc = JSON.parse(fs.readFileSync(file, "utf8"));
        await clientStarted;
        elog(`inspector trace injected from ${file} (test hook)`);
        await client.sendNotification(inspector.TRACE, doc);
      })
    );
  }

  // --- Paused-scene inspector: attach/detach + status bar -----------------
  inspectorState = inspector.initialState(context.globalState.get(INSPECTOR_PORT_KEY));
  inspectorStatus = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  const renderInspectorStatus = () => {
    const bar = inspector.statusBar(inspectorState);
    inspectorStatus.text = bar.text;
    inspectorStatus.tooltip = bar.tooltip;
    inspectorStatus.command = bar.command;
    inspectorStatus.show();
  };
  renderInspectorStatus();

  const attachInspector = async () => {
    const input = await vscode.window.showInputBox({
      prompt: "Functor Lang inspector: debug-server port to attach to",
      value: inspector.promptDefault(inspectorState),
      validateInput: (v) => inspector.parsePort(v).error,
    });
    if (input === undefined) return; // cancelled
    const parsed = inspector.parsePort(input);
    if (parsed.error) return; // validateInput blocks this, but be defensive
    inspectorState = inspector.reduce(inspectorState, { type: "attach", port: parsed.port });
    context.globalState.update(INSPECTOR_PORT_KEY, parsed.port);
    const n = inspector.attachNotification(parsed.port);
    elog(`inspector attach: port ${parsed.port}`);
    if (client) client.sendNotification(n.notification, n.params);
    renderInspectorStatus();
  };
  const detachInspector = () => {
    inspectorState = inspector.reduce(inspectorState, { type: "detach" });
    const n = inspector.detachNotification();
    elog("inspector detach");
    if (client) client.sendNotification(n.notification, n.params);
    renderInspectorStatus();
  };

  context.subscriptions.push(
    inspectorStatus,
    vscode.commands.registerCommand(inspector.ATTACH_COMMAND, attachInspector),
    vscode.commands.registerCommand(inspector.DETACH_COMMAND, detachInspector)
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

// Walk up from an .fun file to the functor.json declaring a Functor Lang project —
// the directory `functor -d <dir>` operates on. Returns { dir, entry } or
// null. A functor.json for another language (or an unreadable one) is
// skipped and the walk continues, so nested projects resolve correctly.
function findFunctorLangProject(fromFile) {
  let dir = path.dirname(fromFile);
  for (;;) {
    const manifest = path.join(dir, "functor.json");
    if (fs.existsSync(manifest)) {
      try {
        const json = JSON.parse(fs.readFileSync(manifest, "utf8"));
        if (json.language === "functor-lang" && typeof json.entry === "string") {
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

// "Functor Lang: Open Live Preview" — serve the active file's project with
// `functor run wasm` and host the running game in a webview panel that
// hot-reloads from the LIVE buffer (unsaved included), model preserved.
async function openLivePreview() {
  const editor = vscode.window.activeTextEditor;
  const project =
    editor && editor.document.fileName.endsWith(".fun")
      ? findFunctorLangProject(editor.document.fileName)
      : null;

  if (previewPanel) {
    // Same project (or no new project resolvable from the active editor) →
    // just focus the running preview.
    if (!project || project.dir === previewProjectDir) {
      previewPanel.reveal();
      return;
    }
    // Switching samples: dispose the old panel (its onDidDispose kills the
    // dev-server child) and wait for that child to actually exit, so port 8080
    // is free before the new server tries to bind it.
    const dying = previewChild;
    previewPanel.dispose();
    const running = dying && dying.exitCode === null && dying.signalCode === null;
    if (running) {
      await new Promise((resolve) => {
        const t = setTimeout(resolve, 3000); // fail-safe: never hang the command
        dying.once("exit", () => {
          clearTimeout(t);
          resolve();
        });
      });
    }
  }

  if (!editor || !editor.document.fileName.endsWith(".fun")) {
    vscode.window.showErrorMessage(
      "Functor Lang: open the project's .fun file first — the preview serves the project it belongs to."
    );
    return;
  }
  if (!project) {
    vscode.window.showErrorMessage(
      `Functor Lang: no functor.json with "language": "functor-lang" found in any directory above ` +
        `${editor.document.fileName} — create one ({"language": "functor-lang", "entry": "game.fun"}) ` +
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
    vscode.workspace.getConfiguration("functor-lang").get("functorPath") || "functor";
  const output = vscode.window.createOutputChannel("Functor Lang Preview");
  // The child outlives the panel by a beat (kill() on dispose, exit later),
  // so every output write is gated on the panel still being alive — VSCode
  // throws on appending to a disposed channel.
  let disposed = false;
  const log = (text) => {
    if (!disposed) output.append(text);
  };
  elog(`preview: spawning ${functorPath} run wasm for ${project.dir}`);
  const child = spawn(functorPath, ["-d", project.dir, "run", "wasm", "--no-open"], {
    cwd: project.dir,
  });
  previewChild = child;
  previewProjectDir = project.dir;
  child.stdout.on("data", (d) => log(d.toString()));
  child.stderr.on("data", (d) => log(d.toString()));
  child.on("error", (e) => {
    vscode.window.showErrorMessage(
      `Functor Lang: cannot start "${functorPath}" (${e.message}) — set functor-lang.functorPath to the functor CLI binary.`
    );
  });
  child.on("exit", (code) => log(`[functor exited with code ${code}]\n`));

  const panel = vscode.window.createWebviewPanel(
    "functorLangLivePreview",
    `Functor Lang Live Preview — ${path.basename(project.dir)}`,
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
  // bridge): it proves 8080 is really the Functor Lang preview — anything else
  // answering HTTP there never sends it — and it's the moment to flush the
  // CURRENT buffer, so edits made before the runtime came up (or while the
  // file sat unsaved) are not dropped.
  let ready = false;
  let readyTimeout;
  const pushCurrentBuffer = async () => {
    try {
      const doc = await vscode.workspace.openTextDocument(entryPath);
      panel.webview.postMessage({ type: "functor-lang-set-source", source: doc.getText() });
    } catch (e) {
      log(`[push] cannot read ${entryPath}: ${e}\n`);
    }
  };
  panel.webview.onDidReceiveMessage((msg) => {
    if (!msg) return;
    // Inspector trace relay (wasm push path, PR2b): the game iframe posts a
    // `functor-inspector-trace` message that the webview forwards here; hand
    // the wire-contract payload to the LSP. Inert until the wasm runtime emits
    // these; unrelated messages fall through to relayTrace → null.
    const relayed = inspector.relayTrace(msg);
    if (relayed) {
      const doc = relayed.params || {};
      elog(
        `inspector trace relayed from preview: paused=${doc.paused} frame=${doc.frame ?? "-"} ` +
          `invocations=${(doc.invocations || []).length}`
      );
      if (client) client.sendNotification(relayed.notification, relayed.params);
      return;
    }
    if (msg.type === "functor-lang-preview-ready") {
      if (ready) return;
      ready = true;
      clearTimeout(readyTimeout);
      pushCurrentBuffer();
      return;
    }
    if (msg.type !== "functor-lang-set-source-result") return;
    if (msg.ok) {
      status.text = "$(check) Functor Lang preview: reloaded";
      status.tooltip = msg.message;
    } else {
      status.text = "$(error) Functor Lang preview: push failed";
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
      panel.webview.postMessage({ type: "functor-lang-set-source", source: e.document.getText() });
    }, PUSH_DEBOUNCE_MS);
  });

  panel.onDidDispose(() => {
    disposed = true;
    previewPanel = undefined;
    previewChild = undefined;
    previewProjectDir = undefined;
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
    panel.webview.postMessage({ type: "functor-lang-preview-navigate", url: PREVIEW_URL });
    readyTimeout = setTimeout(() => {
      if (ready || disposed) return;
      vscode.window.showErrorMessage(
        `Functor Lang: ${PREVIEW_URL} answered but never announced the Functor Lang preview — ` +
          `is something else using that port?`
      );
    }, SERVER_WAIT_MS);
  } else {
    vscode.window.showErrorMessage(
      `Functor Lang: the functor dev server did not come up at ${PREVIEW_URL} — see the "Functor Lang Preview" output.`
    );
  }
}

// The panel document: a full-size iframe hosting the game page, plus the
// message bridge — extension → webview → iframe for source pushes, and
// iframe → webview → extension for results. The game page itself does the
// reload (see index-functor-lang.html's functor-lang-set-source listener in the runtime).
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
        if (data.type === "functor-lang-preview-navigate") {
          frame.src = data.url;
          frame.style.display = "block";
          document.getElementById("waiting").style.display = "none";
        } else if (data.type === "functor-lang-set-source") {
          // Extension → game page (the page only accepts pushes from its
          // parent — us).
          if (frame.contentWindow) frame.contentWindow.postMessage(data, "*");
        } else if (
          data.type === "functor-lang-set-source-result" ||
          data.type === "functor-lang-preview-ready" ||
          data.type === "functor-inspector-trace"
        ) {
          // Game page → extension: reload results, readiness, and inspector
          // trace pushes (the wasm path, PR2b — inert until the runtime emits
          // them). Only trust the page we framed: anything else on that port
          // (or the game code itself) must not spoof these.
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
