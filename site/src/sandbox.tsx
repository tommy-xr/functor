// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// editor↔player postMessage seam (player-bridge.ts — the same protocol the
// VSCode live-preview panel uses). Edits are debounced and pushed as
// `functor-lang-set-source`; the runtime hot-swaps the program with the model
// preserved and replies `functor-lang-set-source-result`.
//
// The page shell is static HTML; the CHROME around the editor (the picker, the
// pill, the external-runtime panel, the status bar) renders as React islands
// mounted into that shell's containers. The editor itself is not React —
// CodeMirror owns `#editor`'s subtree — and all the load/push logic below
// stays plain imperative code that publishes to small stores.

import { createRoot } from "react-dom/client";
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
import { createStatusBarStore } from "./status-bar-store.js";
import { createRuntimeTargetCore } from "./runtime-target-core.js";
import type { RuntimeTargetState } from "./runtime-target-core.js";
import { createStore } from "./store.js";
import { SandboxControls } from "./components/SandboxControls.js";
import type { PickerState, ClientsState } from "./components/SandboxControls.js";
import { initMultiplayerPanes, MAX_CLIENTS } from "./mp-panes.js";
import type { MultiplayerPanes } from "./mp-panes.js";
import type { PillState } from "./components/StatusPill.js";
import { StatusBar } from "./components/StatusBar.js";
import { asPlayerMessage } from "./protocol.js";
import { EXAMPLES, exampleEntryPath } from "./examples.js";

/** The sandbox's e2e seam (driven by e2e/site-sandbox.mjs). */
interface SandboxSeam {
  setSource(source: string): void;
  source: () => string;
  status: () => { state: string; text: string; message: string };
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

const frame = document.getElementById("player") as HTMLIFrameElement;

// An inline program from the URL fragment (the docs' "try it" buttons):
// #src=<base64url> becomes the editor buffer and the player's ?src= data:
// URL, so it starts with a fresh init — no served file involved.
const inlineSrc = new URLSearchParams(window.location.hash.slice(1)).get("src");
const pageParams = new URLSearchParams(window.location.search);
const requested = pageParams.get("example");
// Inline programs have no manifest, so `?mouseCapture=false` /
// `?cursor=visible` are their explicit project-setting seams. Captured game
// input defaults on; visible-pointer mode disables it.
const pageCursorPolicy =
  pageParams.get("cursor") === "visible" ? "visible" : null;
const pageMouseCapture =
  !pageCursorPolicy && pageParams.get("mouseCapture") === "false" ? false : null;
const initialExample = EXAMPLES.some((e) => e.id === requested) ? requested! : EXAMPLES[0].id;

const picker = createStore<PickerState>({
  options: EXAMPLES.map((example) => ({ value: example.id, label: example.label })),
  selected: initialExample,
});
const pill = createStore<PillState>({ state: "busy", text: "◌ loading…", detail: "" });

// Multiplayer pane-grid prototype (mp-panes.ts): #clients=2|3 turns the
// preview column into a chrono bar + N client panes. A HASH param, not a
// query: count changes apply live (panes grow/shrink in place, client 1's
// model survives) with no page reload. ?clients= still parses for old links.
const hashClients = () =>
  Number(new URLSearchParams(window.location.hash.slice(1)).get("clients")) || 0;
const requestedClients = Math.min(
  MAX_CLIENTS,
  Math.max(
    1,
    hashClients() || Number(new URLSearchParams(window.location.search).get("clients")) || 1
  )
);
let mp: MultiplayerPanes | null = null;
const clients = createStore<ClientsState>({ count: requestedClients, visible: false });

const setStatus = (state: PillState["state"], text: string, detail = "") => {
  // The detail (the reload note, or a parse error) lives in the pill's tooltip
  // and — for errors — the Output panel. No separate error banner under the
  // editor: the preview pill is the single live indicator.
  pill.set({ state, text, detail });
  // In multiplayer the pill aggregates every pane ("● 3 running"); the primary
  // pane's state feeds the same aggregate.
  mp?.aggregateStatus(state, text, detail);
};

// The boot loader (static markup in sandbox.html) evaporates when the PLAYER
// reports in — never on any other status change, since an error from, say, a
// bad #src= fragment says nothing about whether the pane has pixels yet. One
// class toggle; the 620ms evaporate is all CSS.
const dismissBootLoader = () =>
  document.querySelector("[data-fn-boot]")?.classList.add("is-done");

const statusBar = createStatusBarStore();

// One instrument at every client count: the chrono bar is the transport UI
// for a single client and for N — the players mount their __scrub seam with
// no bar of their own (?scrubber=hidden). `getSource` lets a mirror added
// mid-session catch up to the edited buffer (`view` exists by the time any
// pane is added).
mp = initMultiplayerPanes({
  frame,
  count: requestedClients,
  statusBar,
  setPill: (state, text, detail) => pill.set({ state, text, detail }),
  getSource: () => view.state.doc.toString(),
  // The pane grid's "+" tile asks for a count the same way the CLIENTS
  // dropdown does — `selectClients` is the one path (clamp, hash, control).
  requestCount: (n) => selectClients(n),
});

// The CLIENTS control only appears for multiplayer-structured samples (the
// `multiplayer` flag). Until the multiplayer transport arc lands, panes run
// INDEPENDENT copies of the scene — the control previews the multi-client
// layout, not a shared world. It stays visible while #clients= forces
// panes, so there is always a way back to 1; the hash keeps working
// everywhere as the dev seam.
const updateClientsStore = () => {
  const example = EXAMPLES.find((candidate) => candidate.id === picker.getSnapshot().selected);
  // Latched: once the control has appeared (a flagged sample, or a forcing
  // hash), shrinking back to 1 must not remove the only way to grow again.
  clients.set({
    count: mp!.count(),
    visible:
      clients.getSnapshot().visible || Boolean(example?.multiplayer) || mp!.count() > 1,
  });
};
updateClientsStore();

// Reflect the live count into the hash (replaceState — no navigation, and the
// #src= inline-program param survives alongside it).
const writeClientsHash = (n: number) => {
  const hash = new URLSearchParams(window.location.hash.slice(1));
  if (n > 1) hash.set("clients", String(n));
  else hash.delete("clients");
  const url = new URL(window.location.href);
  url.hash = hash.toString();
  // Also retire the legacy ?clients= fallback — leaving it would resurrect
  // the old count on the next reload after an explicit shrink.
  url.searchParams.delete("clients");
  window.history.replaceState(null, "", url);
};

const selectClients = (n: number) => {
  mp!.setCount(n);
  writeClientsHash(mp!.count());
  updateClientsStore();
};

// Back/forward (or a hand-edited hash) also applies live.
window.addEventListener("hashchange", () => {
  const n = hashClients() || 1;
  if (n !== mp!.count()) {
    mp!.setCount(n);
    updateClientsStore();
  }
});
// The sandbox edits only the entry buffer, but some examples also load sibling
// modules (for example Mario's generated assets.fun manifest). Keep those
// fetched sources so an external runtime receives the same complete project as
// the in-page wasm preview.
let siblingSources: [string, string][] = [];
let assetSources: [string, Uint8Array][] = [];

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy", "◌ reloading…"),
  onLive: () => {
    dismissBootLoader();
    setStatus("live", "● live");
  },
  onResult: (ok, message) => {
    dismissBootLoader();
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
  statusBar.appendOutput(
    data.level,
    mp && mp.count() > 1 ? `[client 1] ${data.message}` : data.message,
    data.frame ?? null
  );
});


// Created once, outside React: this controller carries the live link's queued
// pushes and its `/state` poll chain, so a re-render must never restart it.
const runtimeTarget = createRuntimeTargetCore({
  getProject: () => [["game.fun", view.state.doc.toString()], ...siblingSources],
  getAssets: () => assetSources,
  onOutput: (level, message) => statusBar.appendOutput(level, message),
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
        mp?.push(view.state.doc.toString());
        runtimeTarget.projectChanged();
      }
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
  mp?.reset();
  // Wholesale document replacement (example switch, inline load, reset): drop
  // the wasm completion cache so the previous program's candidates can't leak.
  resetIntel();
  siblingSources = siblings;
  assetSources = assets;
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
  runtimeTarget.projectChanged({ fresh: true });
};

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
  const options = picker.getSnapshot().options;
  picker.set({
    options: options.some((option) => option.value === "__inline")
      ? options
      : [...options, { value: "__inline", label: "docs snippet" }],
    selected: "__inline",
  });
  loadToken += 1; // supersede any in-flight example fetch
  setDoc(source);
  setStatus("busy", "◌ loading…");
  // A fresh iframe on a `?src=` data: URL, so the inline program runs its OWN
  // `init` (a set-source push would preserve the default entry's model). The
  // loader derives module `Main` for a non-identifier entry label.
  // scrubber=hidden: the page's chrono bar is the transport UI; the player
  // mounts the __scrub seam with no bar of its own.
  // net=embedder, UNCONDITIONALLY: this host always routes pane networking
  // through its own coordinator (net-coordinator.ts) rather than letting a
  // pane open browser sockets. A program that declares no `Sub.connect`/
  // `Sub.listen` posts nothing, so the declaration costs it nothing.
  const params = new URLSearchParams({ src: b64u, scrubber: "hidden", net: "embedder" });
  if (pageMouseCapture === false) params.set("mouseCapture", "false");
  if (pageCursorPolicy) params.set("cursor", pageCursorPolicy);
  frame.src = `player.html?${params}`;
  mp?.setSrc(frame.src);
  return true;
};

const loadExample = async (id: string) => {
  const token = ++loadToken;
  const example = EXAMPLES.find((candidate) => candidate.id === id);
  const files = [
    exampleEntryPath(id),
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
  // scrubber=hidden / net=embedder: as in loadInline above.
  const params = new URLSearchParams({ game: url, scrubber: "hidden", net: "embedder" });
  const cursorPolicy = example?.cursor ?? pageCursorPolicy;
  const mouseCapture = cursorPolicy
    ? null
    : example?.mouseCapture ?? pageMouseCapture;
  if (mouseCapture === false) params.set("mouseCapture", "false");
  if (cursorPolicy) {
    params.set("cursor", cursorPolicy);
  }
  for (const file of files) params.append("file", file);
  // A same-file-entries sample plays its declared role: the player boots the
  // prefixed contract (e.g. orbs' clientInit/clientTick/…).
  if (example?.prefix) params.set("prefix", example.prefix);
  // …or, in the preferred same-file form, the role's inline module (the
  // player takes one of the two).
  if (example?.module) params.set("module", example.module);
  frame.src = `player.html?${params}`;
  // A sample with a SERVER role (examples.ts `server`) also boots a server
  // pane: the SAME file list re-entered at the server file. `?file=` is the
  // whole project with the ENTRY FIRST, so the two roles differ only in which
  // module the runtime looks the entry points up in.
  const serverFile = example?.server?.file;
  let serverSrc: string | null = null;
  if (serverFile) {
    // Derived from the client's params, so every project setting the sample
    // declares (prefix, cursor, mouseCapture) reaches the server pane too —
    // only the entry and the file ORDER differ.
    const serverParams = new URLSearchParams(params);
    serverParams.set("game", serverFile);
    serverParams.delete("file");
    // The client's ROLE must not leak into the server pane: a same-file
    // sample states the server's own inline module (absent = the file's plain
    // top-level contract).
    serverParams.delete("module");
    if (example?.server?.module) serverParams.set("module", example.server.module);
    for (const file of [serverFile, ...files.filter((f) => f !== serverFile)]) {
      serverParams.append("file", file);
    }
    serverSrc = `player.html?${serverParams}`;
  }
  mp?.setSrc(frame.src, serverSrc);
};

const selectExample = (value: string) => {
  picker.set({ ...picker.getSnapshot(), selected: value });
  if (value === "__inline") {
    // The `__inline` option only exists once loadInline has stored its source.
    loadInline(inlineB64!);
    return;
  }
  const url = new URL(window.location.href);
  url.searchParams.set("example", value);
  // Drop a stale inline program from the hash, but keep #clients=N alive.
  const hash = new URLSearchParams(window.location.hash.slice(1));
  hash.delete("src");
  url.hash = hash.toString();
  window.history.replaceState(null, "", url);
  updateClientsStore();
  loadExample(value);
};

const resetExample = () => {
  const selected = picker.getSnapshot().selected;
  if (selected === "__inline") loadInline(inlineB64!);
  else loadExample(selected);
};

// Mount the islands into the static shell's containers. Both keep their
// element ids and class names, so styles.css and every e2e selector match the
// rendered DOM exactly as they matched the hand-built one.
createRoot(document.querySelector(".sandbox-controls")!).render(
  <SandboxControls
    picker={picker}
    pill={pill}
    clients={clients}
    runtimeTarget={runtimeTarget}
    onSelect={selectExample}
    onReset={resetExample}
    onClients={selectClients}
  />
);
const statusBarHost = document.getElementById("statusbar")!;
statusBarHost.className = "statusbar";
createRoot(statusBarHost).render(<StatusBar store={statusBar} />);

if (!(inlineSrc && loadInline(inlineSrc))) loadExample(initialExample);

// Test seam for the headless e2e (e2e/site-sandbox.mjs): set the buffer and
// observe results without synthesizing keyboard events.
(window as Window & { __sandbox?: SandboxSeam }).__sandbox = {
  setSource(source) {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  },
  source: () => view.state.doc.toString(),
  // Read the pill's store, which the rendered pill mirrors exactly: the seam
  // stays synchronous with the page's own state rather than racing a React
  // commit. The fields are the pre-migration ones (`title` was the detail).
  status: () => {
    const { state, text, detail } = pill.getSnapshot();
    return { state, text, message: detail };
  },
  runtimeTarget: () => runtimeTarget.state(),
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
