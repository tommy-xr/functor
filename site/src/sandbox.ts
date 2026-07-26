// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// editor↔player postMessage seam (player-bridge.ts — the same protocol the
// VSCode live-preview panel uses). Edits are debounced and pushed as
// `functor-lang-set-source`; the runtime hot-swaps the program with the model
// preserved and replies `functor-lang-set-source-result`.

import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { StateEffect } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { startCompletion, acceptCompletion, closeCompletion } from "@codemirror/autocomplete";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import {
  setupLangIntel,
  analyzeCached,
  completeAt,
  resetIntel,
  onDiagnostics,
  wireLiveTrace,
  currentLiveHints,
  currentCoverage,
  currentExpects,
} from "./lang-intel.js";
import { PlayerBridge } from "./player-bridge.js";
import { createStatusBar } from "./status-bar.js";
import { createRuntimeTarget } from "./runtime-target.js";
import { asPlayerMessage } from "./protocol.js";
import { EXAMPLES } from "./examples.js";

/** The preview pill's three states — also its `data-state` attribute value. */
type PillState = "busy" | "live" | "error";

/** The sandbox's e2e seam (driven by e2e/site-sandbox.mjs). */
interface SandboxSeam {
  setSource(source: string): void;
  source: () => string;
  status: () => {
    state: string | undefined;
    text: string | null;
    message: string;
  };
  runtimeTarget: () => RuntimeTargetState;
  getSource: () => string;
  triggerComplete(source: string, cursor: number): void;
  acceptCompletion: () => boolean;
}

// The language-analysis seam, shared in NAME (not shape) with the IDE's — each
// page declares its own, so the two never have to agree. The payload types are
// lang-intel's internals; naming them through `ReturnType` keeps this seam
// exact without widening that module's public surface.
interface LangSeam {
  ready: Promise<boolean>;
  analyze: (source: string) => ReturnType<typeof analyzeCached>;
  complete: (source: string, offset: number) => ReturnType<typeof completeAt>;
  liveHints: () => ReturnType<typeof currentLiveHints>;
  coverage: () => ReturnType<typeof currentCoverage>;
  expects: () => ReturnType<typeof currentExpects>;
}

type RuntimeTarget = ReturnType<typeof createRuntimeTarget>;
type RuntimeTargetState = ReturnType<RuntimeTarget["state"]>;

const frame = document.getElementById("player") as HTMLIFrameElement;
const statusPill = document.getElementById("status")!;
const picker = document.getElementById("example-picker") as HTMLSelectElement;
const resetButton = document.getElementById("reset")!;

const setStatus = (state: PillState, text: string, detail = "") => {
  statusPill.dataset.state = state;
  statusPill.textContent = text;
  // The detail (the reload note, or a parse error) lives in the pill's tooltip
  // and — for errors — the Output panel. No separate error banner under the
  // editor: the preview pill is the single live indicator.
  statusPill.title = detail;
};

const statusBar = createStatusBar({ host: document.getElementById("statusbar")! });
let runtimeTarget: RuntimeTarget | null = null;
// The sandbox edits only the entry buffer, but some examples also load sibling
// modules (for example Mario's generated assets.fun manifest). Keep those
// fetched sources so an external runtime receives the same complete project as
// the in-page wasm preview.
let siblingSources: [string, string][] = [];
let assetSources: [string, Uint8Array][] = [];

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy", "◌ reloading…"),
  onLive: () => setStatus("live", "● live"),
  onResult: (ok, message) => {
    if (ok) {
      // The runtime's status line ("reloaded … model preserved") stays
      // reachable — hover the pill, or the e2e's status() seam below.
      setStatus("live", "● live", message);
    } else {
      setStatus("error", "✖ error", message);
    }
    // Failed reloads also land in the Output panel — the pill is transient,
    // the panel keeps the history. (Successes already arrive there via the
    // runtime's own "[functor-lang] reloaded …" console line.)
    if (!ok) statusBar.appendOutput("error", message);
  },
});

// Runtime console traces (Functor Lang `Debug.log` and friends), forwarded by the
// player page — see the console hook in player.html. Guarded to OUR iframe.
window.addEventListener("message", (event) => {
  const data = asPlayerMessage(event.data);
  if (!data || data.type !== "functor-lang-console") return;
  if (event.source !== frame.contentWindow) return;
  statusBar.appendOutput(data.level, data.message, data.frame ?? null);
});


// Set while loadExample replaces the buffer programmatically: that content is
// exactly what the fresh iframe is about to fetch, so pushing it back would
// be a redundant reload (and would mislabel a fresh load as a hot reload).
let programmaticEdit = false;

const view = new EditorView({
  parent: document.getElementById("editor")!,
  extensions: [
    basicSetup,
    keymap.of([indentWithTab]),
    functorLangLanguage,
    synthwaveEditorTheme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !programmaticEdit) {
        bridge.push(view.state.doc.toString());
        runtimeTarget?.projectChanged();
      }
    }),
  ],
});

runtimeTarget = createRuntimeTarget({
  host: document.getElementById("runtime-target"),
  getProject: () => [["game.fun", view.state.doc.toString()], ...siblingSources],
  getAssets: () => assetSources,
  onOutput: (level, message) => statusBar.appendOutput(level, message),
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

// The paused-inspector trace (live values in the editor + the executions
// picker), relayed by the player on pause / paused-frame change.
wireLiveTrace(view, statusBar, frame, langReady);

// Each lint pass refreshes the Problems panel; clicking a problem jumps the
// editor to it. Positions re-clamp at click time (the doc may have moved on).
onDiagnostics((diags) => {
  statusBar.setProblems(
    diags.map((d) => {
      const line = view.state.doc.lineAt(Math.min(d.from, view.state.doc.length));
      return {
        severity: d.severity,
        message: d.message,
        loc: `game.fun ${line.number}:${d.from - line.from + 1}`,
        jump: () => {
          const len = view.state.doc.length;
          const from = Math.min(d.from, len);
          view.dispatch({
            selection: { anchor: from, head: Math.max(from, Math.min(d.to, len)) },
            scrollIntoView: true,
          });
          view.focus();
        },
      };
    })
  );
});

(window as Window & { __lang?: LangSeam }).__lang = {
  ready: langReady,
  analyze: (source) => analyzeCached(source),
  complete: (source, offset) => completeAt(source, offset),
  liveHints: () => currentLiveHints(),
  coverage: () => currentCoverage(view),
  expects: () => currentExpects(view),
};

const setDoc = (
  source: string,
  siblings: [string, string][] = [],
  assets: [string, Uint8Array][] = []
) => {
  bridge.reset();
  // Wholesale document replacement (example switch, inline load, reset): drop
  // the wasm completion cache so the previous program's candidates can't leak.
  resetIntel();
  siblingSources = siblings;
  assetSources = assets;
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
  runtimeTarget?.projectChanged({ fresh: true });
};

// An inline program from the URL fragment (the docs' "try it" buttons):
// #src=<base64url> becomes the editor buffer and the player's ?src= data:
// URL, so it starts with a fresh init — no served file involved.
const fromBase64Url = (b64u: string) =>
  new TextDecoder().decode(
    Uint8Array.from(atob(b64u.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0))
  );

let inlineB64: string | null = null;

// A monotonically increasing load token: each picker change / reset / inline
// load claims a new one, and a fetch that finishes after a newer load started
// is ignored — a slow earlier response must not overwrite a newer selection.
let loadToken = 0;

const loadInline = (b64u: string) => {
  let source: string;
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

const loadExample = async (id: string) => {
  const token = ++loadToken;
  const example = EXAMPLES.find((candidate) => candidate.id === id);
  const files = [
    `examples/${encodeURIComponent(id)}.fun`,
    ...(example?.siblings?.map(({ output }) => output) ?? []),
  ];
  const assetFiles = example?.assets ?? [];
  const resourcePaths = [...files, ...assetFiles.map(({ output }) => output)];
  const responses = await Promise.all(resourcePaths.map((file) => fetch(file)));
  if (token !== loadToken) return; // a newer load superseded this one
  const failed = responses.findIndex((response) => !response.ok);
  if (failed !== -1) {
    setStatus(
      "error",
      "✖ error",
      `cannot fetch ${resourcePaths[failed]}: HTTP ${responses[failed].status}`
    );
    return;
  }
  const sources = await Promise.all(
    responses.slice(0, files.length).map((response) => response.text())
  );
  const assets = await Promise.all(
    responses.slice(files.length).map(async (response, index): Promise<[string, Uint8Array]> => [
      assetFiles[index].output,
      new Uint8Array(await response.arrayBuffer()),
    ])
  );
  if (token !== loadToken) return;
  const url = files[0];
  const source = sources[0];
  const siblings = files.slice(1).map((path, index): [string, string] => [
    // Every path here has at least one segment, so `pop` always yields one.
    path.split("/").pop()!,
    sources[index + 1],
  ]);
  // A fresh iframe (fresh model: init runs) rather than a source push, so
  // switching examples resets state; the ready announcement re-arms pushes.
  setDoc(source, siblings, assets);
  setStatus("busy", "◌ loading…");
  const params = new URLSearchParams({ game: url });
  for (const file of files) params.append("file", file);
  frame.src = `player.html?${params}`;
};

for (const example of EXAMPLES) {
  const option = document.createElement("option");
  option.value = example.id;
  option.textContent = example.label;
  picker.appendChild(option);
}

picker.addEventListener("change", () => {
  if (picker.value === "__inline") {
    // The `__inline` option only exists once loadInline has stored its source.
    loadInline(inlineB64!);
    return;
  }
  const url = new URL(window.location.href);
  url.searchParams.set("example", picker.value);
  url.hash = "";
  window.history.replaceState(null, "", url);
  loadExample(picker.value);
});

resetButton.addEventListener("click", () =>
  picker.value === "__inline" ? loadInline(inlineB64!) : loadExample(picker.value)
);

const inlineSrc = new URLSearchParams(window.location.hash.slice(1)).get("src");
const requested = new URLSearchParams(window.location.search).get("example");
const initial = EXAMPLES.some((e) => e.id === requested) ? requested! : EXAMPLES[0].id;
picker.value = initial;
if (!(inlineSrc && loadInline(inlineSrc))) loadExample(initial);

// Test seam for the headless e2e (e2e/site-sandbox.mjs): set the buffer and
// observe results without synthesizing keyboard events.
(window as Window & { __sandbox?: SandboxSeam }).__sandbox = {
  setSource(source) {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  },
  source: () => view.state.doc.toString(),
  status: () => ({
    state: statusPill.dataset.state,
    text: statusPill.textContent,
    message: statusPill.title,
  }),
  // Assigned during boot, long before any e2e call can reach this seam.
  runtimeTarget: () => runtimeTarget!.state(),
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
