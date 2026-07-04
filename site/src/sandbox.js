// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// same postMessage seam the VSCode live-preview panel uses
// (tools/mle-vscode/client/extension.js is the reference implementation):
// edits are debounced and pushed as `mle-set-source`; the runtime hot-swaps
// the program with the model preserved and replies `mle-set-source-result`.

import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";
import { mleLanguage, synthwaveEditorTheme } from "./mle.js";

const EXAMPLES = [
  { id: "hero", label: "Neon grid" },
  { id: "orbit", label: "Orbit (MVU)" },
  { id: "physics", label: "Physics" },
  { id: "monitor", label: "Render targets" },
];

// Same cadence as the VSCode extension: fast enough to feel live, slow
// enough not to push a reload per keystroke.
const PUSH_DEBOUNCE_MS = 300;

const frame = document.getElementById("player");
const statusPill = document.getElementById("status");
const statusLog = document.getElementById("status-log");
const picker = document.getElementById("example-picker");
const resetButton = document.getElementById("reset");

let previewReady = false;
let dirty = false;
let pushTimer = null;

const setStatus = (state, text, detail = "") => {
  statusPill.dataset.state = state;
  statusPill.textContent = text;
  statusLog.textContent = detail;
  statusLog.hidden = detail === "";
};

const pushSource = () => {
  if (!previewReady || !frame.contentWindow) {
    dirty = true;
    return;
  }
  dirty = false;
  setStatus("busy", "◌ reloading…");
  frame.contentWindow.postMessage(
    { type: "mle-set-source", source: view.state.doc.toString() },
    "*"
  );
};

const schedulePush = () => {
  clearTimeout(pushTimer);
  pushTimer = setTimeout(pushSource, PUSH_DEBOUNCE_MS);
};

// Set while loadExample replaces the buffer programmatically: that content is
// exactly what the fresh iframe is about to fetch, so pushing it back would
// be a redundant reload (and would mislabel a fresh load as a hot reload).
let programmaticEdit = false;

const view = new EditorView({
  parent: document.getElementById("editor"),
  extensions: [
    basicSetup,
    keymap.of([indentWithTab]),
    mleLanguage,
    synthwaveEditorTheme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !programmaticEdit) schedulePush();
    }),
  ],
});

// Replies and readiness from the player iframe. Only trust the iframe we
// created (same-origin, but be explicit about the source anyway).
window.addEventListener("message", (event) => {
  if (event.source !== frame.contentWindow) return;
  const data = event.data;
  if (!data) return;
  if (data.type === "mle-preview-ready") {
    previewReady = true;
    // Flush edits made while the runtime was still starting.
    if (dirty) pushSource();
    else setStatus("live", "● live");
  } else if (data.type === "mle-set-source-result") {
    // A reply from the outgoing document (its WindowProxy survives the src
    // swap) must not overwrite the "loading…" status of the incoming one.
    if (!previewReady) return;
    if (data.ok) setStatus("live", `● live — ${data.message}`);
    else setStatus("error", "✖ error", data.message);
  }
});

const loadExample = async (id) => {
  const url = `examples/${encodeURIComponent(id)}.mle`;
  const response = await fetch(url);
  if (!response.ok) {
    setStatus("error", "✖ error", `cannot fetch ${url}: HTTP ${response.status}`);
    return;
  }
  const source = await response.text();
  clearTimeout(pushTimer);
  previewReady = false;
  dirty = false;
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
  // A fresh iframe (fresh model: init runs) rather than a source push, so
  // switching examples resets state; the ready announcement re-arms pushes.
  setStatus("busy", "◌ loading…");
  frame.src = `player.html?game=${encodeURIComponent(url)}`;
};

for (const example of EXAMPLES) {
  const option = document.createElement("option");
  option.value = example.id;
  option.textContent = example.label;
  picker.appendChild(option);
}

picker.addEventListener("change", () => {
  const url = new URL(window.location);
  url.searchParams.set("example", picker.value);
  window.history.replaceState(null, "", url);
  loadExample(picker.value);
});

resetButton.addEventListener("click", () => loadExample(picker.value));

const requested = new URLSearchParams(window.location.search).get("example");
const initial = EXAMPLES.some((e) => e.id === requested) ? requested : EXAMPLES[0].id;
picker.value = initial;
loadExample(initial);

// Test seam for the headless e2e (e2e/site-sandbox.mjs): set the buffer and
// observe results without synthesizing keyboard events.
window.__sandbox = {
  setSource(source) {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  },
  status: () => ({
    state: statusPill.dataset.state,
    text: statusPill.textContent,
    detail: statusLog.textContent,
  }),
};
