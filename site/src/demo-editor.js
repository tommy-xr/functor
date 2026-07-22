// A standalone, full-featured Functor Lang editor beside a live scene — the
// hero editor's demo-grade sibling. Unlike the landing hero's mini-editor
// (mini-editor.js — deliberately stripped to keep the landing bundle tiny),
// this composes the SAME language intelligence the sandbox uses — completion,
// codelenses, hover types, diagnostics, and the paused-inspector live-value
// overlay — plus state-preserving hot reload. It carries none of the sandbox's
// chrome (picker / reset / problems / executions); it exists to be recorded for
// the site's feature-showcase GIFs (see site/demos/). `?game=<path>` selects
// the scene (default: the synthwave hero).
//
// This is a lean re-composition of the shared, already-modular seams
// (setupLangIntel, wireLiveTrace, PlayerBridge) rather than a fork of
// sandbox.js — the only overlap is constructing the editor view.

import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { StateEffect } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import { setupLangIntel, wireLiveTrace } from "./lang-intel.js";
import { PlayerBridge } from "./player-bridge.js";

const frame = document.getElementById("player");
const game = new URLSearchParams(location.search).get("game") || "examples/hero.fun";

// wireLiveTrace drives an executions picker through a status bar; the demo has
// none, so a no-op stub satisfies the contract (the inline live-value overlay
// it installs on the editor needs nothing from it).
const noopStatusBar = { setExecutions() {} };

// Every edit is a source push — the runtime hot-swaps with the model preserved,
// which is the whole point of the Instant Feedback demo. No callbacks needed.
const bridge = new PlayerBridge(frame, {
  onReloading() {},
  onLive() {},
  onResult() {},
});

// Guards the initial programmatic buffer fill from being pushed back as a
// redundant reload (it is exactly what the fresh iframe is about to fetch).
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

// Language intelligence loads lazily (the analysis wasm); append its extensions
// once ready. Degrades to a plain editor if the pkg is absent.
const langReady = setupLangIntel().then((extensions) => {
  if (extensions.length) view.dispatch({ effects: StateEffect.appendConfig.of(extensions) });
  return extensions.length > 0;
});

// The paused-inspector overlay: live values inline in the editor when the scene
// is paused (via the player's scrubber).
wireLiveTrace(view, noopStatusBar, frame, langReady);

// Load the chosen scene into both the editor buffer and the player.
(async () => {
  const source = await (await fetch(game)).text();
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
  frame.src = `player.html?game=${encodeURIComponent(game)}`;
})();

// Demo / e2e seam.
window.__demoEditor = {
  ready: langReady,
  view,
  frame,
  source: () => view.state.doc.toString(),
};
