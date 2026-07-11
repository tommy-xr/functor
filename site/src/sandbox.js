// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// editor↔player postMessage seam (player-bridge.js — the same protocol the
// VSCode live-preview panel uses). Edits are debounced and pushed as
// `functor-lang-set-source`; the runtime hot-swaps the program with the model
// preserved and replies `functor-lang-set-source-result`.

import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { StateEffect } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { startCompletion, acceptCompletion, closeCompletion } from "@codemirror/autocomplete";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import { setupLangIntel, analyzeCached, completeAt } from "./lang-intel.js";
import { PlayerBridge } from "./player-bridge.js";

const EXAMPLES = [
  { id: "hero", label: "Neon grid" },
  { id: "primitives", label: "Primitives" },
  { id: "bounce", label: "Physics" },
  { id: "monitor", label: "Render targets" },
];

const frame = document.getElementById("player");
const statusPill = document.getElementById("status");
const statusLog = document.getElementById("status-log");
const picker = document.getElementById("example-picker");
const resetButton = document.getElementById("reset");

const setStatus = (state, text, detail = "") => {
  statusPill.dataset.state = state;
  statusPill.textContent = text;
  // Every transition clears the tooltip (the ok branch below re-sets it) so
  // a stale "model preserved" can't contradict a later error or fresh load.
  statusPill.title = "";
  statusLog.textContent = detail;
  statusLog.hidden = detail === "";
};

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy", "◌ reloading…"),
  onLive: () => setStatus("live", "● live"),
  onResult: (ok, message) => {
    if (ok) {
      setStatus("live", "● live");
      // The runtime's status line ("reloaded … model preserved") stays
      // reachable — hover the pill, or the e2e's status() seam below.
      statusPill.title = message;
    } else {
      setStatus("error", "✖ error", message);
    }
  },
});

// Set while loadExample replaces the buffer programmatically: that content is
// exactly what the fresh iframe is about to fetch, so pushing it back would
// be a redundant reload (and would mislabel a fresh load as a hot reload).
let programmaticEdit = false;

const view = new EditorView({
  parent: document.getElementById("editor"),
  extensions: [
    basicSetup,
    keymap.of([indentWithTab]),
    functorLangLanguage,
    synthwaveEditorTheme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !programmaticEdit) bridge.push(view.state.doc.toString());
    }),
  ],
});

// Live type diagnostics: load the analysis wasm lazily and, once ready, append
// the CodeMirror linter to the already-constructed editor. Degrades silently —
// if the pkg is absent the promise resolves to no extensions and the sandbox is
// unchanged. `ready` resolves to whether analysis is available so e2e can await
// it; `analyze` exposes the same cached pass the linter uses.
const langReady = setupLangIntel().then((extensions) => {
  if (extensions.length) view.dispatch({ effects: StateEffect.appendConfig.of(extensions) });
  return extensions.length > 0;
});

window.__lang = {
  ready: langReady,
  analyze: (source) => analyzeCached(source),
  complete: (source, offset) => completeAt(source, offset),
};

const setDoc = (source) => {
  bridge.reset();
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
};

// An inline program from the URL fragment (the docs' "try it" buttons):
// #src=<base64url> becomes the editor buffer and the player's ?src= data:
// URL, so it starts with a fresh init — no served file involved.
const fromBase64Url = (b64u) =>
  new TextDecoder().decode(
    Uint8Array.from(atob(b64u.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0))
  );

let inlineB64 = null;

// A monotonically increasing load token: each picker change / reset / inline
// load claims a new one, and a fetch that finishes after a newer load started
// is ignored — a slow earlier response must not overwrite a newer selection.
let loadToken = 0;

const loadInline = (b64u) => {
  let source;
  try {
    source = fromBase64Url(b64u);
  } catch {
    setStatus("error", "✖ error", "the #src= fragment is not valid base64");
    return false;
  }
  inlineB64 = b64u;
  // Reflect the inline program in the picker so it (and Reset) don't lie
  // about what's loaded.
  if (!picker.querySelector('option[value="__inline"]')) {
    const option = document.createElement("option");
    option.value = "__inline";
    option.textContent = "docs snippet";
    picker.appendChild(option);
  }
  picker.value = "__inline";
  loadToken += 1; // supersede any in-flight example fetch
  setDoc(source);
  setStatus("busy", "◌ loading…");
  // A fresh iframe on a `?src=` data: URL, so the inline program runs its OWN
  // `init` (a set-source push would preserve the default entry's model). The
  // loader derives module `Main` for a non-identifier entry label.
  frame.src = `player.html?src=${b64u}`;
  return true;
};

const loadExample = async (id) => {
  const token = ++loadToken;
  const url = `examples/${encodeURIComponent(id)}.fun`;
  const response = await fetch(url);
  if (token !== loadToken) return; // a newer load superseded this one
  if (!response.ok) {
    setStatus("error", "✖ error", `cannot fetch ${url}: HTTP ${response.status}`);
    return;
  }
  const source = await response.text();
  if (token !== loadToken) return;
  // A fresh iframe (fresh model: init runs) rather than a source push, so
  // switching examples resets state; the ready announcement re-arms pushes.
  setDoc(source);
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
  if (picker.value === "__inline") {
    loadInline(inlineB64);
    return;
  }
  const url = new URL(window.location);
  url.searchParams.set("example", picker.value);
  url.hash = "";
  window.history.replaceState(null, "", url);
  loadExample(picker.value);
});

resetButton.addEventListener("click", () =>
  picker.value === "__inline" ? loadInline(inlineB64) : loadExample(picker.value)
);

const inlineSrc = new URLSearchParams(window.location.hash.slice(1)).get("src");
const requested = new URLSearchParams(window.location.search).get("example");
const initial = EXAMPLES.some((e) => e.id === requested) ? requested : EXAMPLES[0].id;
picker.value = initial;
if (!(inlineSrc && loadInline(inlineSrc))) loadExample(initial);

// Test seam for the headless e2e (e2e/site-sandbox.mjs): set the buffer and
// observe results without synthesizing keyboard events.
window.__sandbox = {
  setSource(source) {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  },
  status: () => ({
    state: statusPill.dataset.state,
    text: statusPill.textContent,
    message: statusPill.title,
    detail: statusLog.textContent,
  }),
  getSource: () => view.state.doc.toString(),
  // Replace the buffer, place the cursor, and open the completion popup
  // (explicit trigger). Guarded so it does NOT push to the runtime — completion
  // is an editor-only concern that must not disturb the live loop. Any open
  // popup is closed first so the fresh trigger reflects the new buffer.
  triggerComplete(source, cursor) {
    closeCompletion(view);
    programmaticEdit = true;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: source },
      selection: { anchor: cursor },
    });
    programmaticEdit = false;
    view.focus();
    startCompletion(view);
  },
  // Accept the selected completion (the editor's normal apply path — this DOES
  // push, exactly as a real accept would). Returns whether one was applied.
  acceptCompletion: () => acceptCompletion(view),
};
