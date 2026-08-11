// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// editor↔player postMessage seam. It speaks the IDE's MULTI-FILE seam
// (project-bridge.ts): the page owns every source of the loaded example, boots
// the player as `player.html?project=inline`, and pushes the whole file set as
// `functor-lang-set-project` — the runtime hot-swaps the program with the model
// preserved and replies `functor-lang-set-source-result`. The editor still
// edits ONE buffer (the entry); a file switcher is a separate concern.
//
// Binary assets are NOT part of that push: the runtime resolves an `Asset.*`
// locator as a URL against player.html, so an example's png/glb/ogg is served
// from the built site (examples.ts `assets` copies them) exactly as before.
// Only `.fun` sources travel over the seam.
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
import { startCompletion, acceptCompletion, closeCompletion } from "@codemirror/autocomplete";
import {
  createEditorKeybindingsController,
  editorIndentWithTab,
} from "./editor-keybindings.js";
import type { EditorKeybindings, EditorKeybindingsState } from "./editor-keybindings.js";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import {
  setupLangIntel,
  analyzeCached,
  completeAt,
  resetIntel,
  refreshIntel,
  onDiagnostics,
  wireLiveTrace,
  currentLiveHints,
  currentCoverage,
  currentExpects,
} from "./lang-intel.js";
import { ProjectBridge } from "./project-bridge.js";
import { createStatusBarStore } from "./status-bar-store.js";
import { createRuntimeTargetCore } from "./runtime-target-core.js";
import type { RuntimeTargetState } from "./runtime-target-core.js";
import { createStore } from "./store.js";
import { SandboxControls } from "./components/SandboxControls.js";
import type { PickerState, ClientsState } from "./components/SandboxControls.js";
import { initMultiplayerPanes, MAX_CLIENTS } from "./mp-panes.js";
import type { MultiplayerPanes, PaneProgram } from "./mp-panes.js";
import type { PillState } from "./components/StatusPill.js";
import { StatusBar } from "./components/StatusBar.js";
import { asPlayerMessage } from "./protocol.js";
import type { ProjectFile } from "./protocol.js";
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
  keybindings: () => EditorKeybindingsState;
  setKeybindings: (mode: EditorKeybindings) => Promise<void>;
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
// #src=<base64url> becomes the editor buffer and a ONE-FILE project pushed to a
// fresh player, so it starts with a fresh init — no served file involved.
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
// no bar of their own (?scrubber=hidden). `getProject` is how a pane added
// mid-session BOOTS (the project push is the boot) already carrying the edited
// buffer (`view` exists by the time any pane is added).
mp = initMultiplayerPanes({
  frame,
  count: requestedClients,
  statusBar,
  setPill: (state, text, detail) => pill.set({ state, text, detail }),
  getProject: () => projectFiles(),
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
// The loaded example's whole file set, ENTRY FIRST — the paths are the ones the
// built site serves, so `file = module` derives the same module names the
// player used to get by fetching them. The editor's buffer is the entry's
// source (the only file it edits for now); every push carries the rest
// unchanged, so a multi-entry example's other roles keep resolving.
let project: ProjectFile[] = [];
// Binary assets stay a fetched, page-relative concern (see the header note);
// they are only forwarded to an external runtime target, which has no site to
// fetch them from.
let assetSources: [string, Uint8Array][] = [];

/** The file the editor is showing — the entry, until a file switcher lands. */
const activePath = () => project[0]?.path ?? "";

// Mirror the live buffer into the entry and hand back the file set to push.
const projectFiles = (): ProjectFile[] => {
  if (project[0]) project[0].source = view.state.doc.toString();
  return project;
};

const bridge = new ProjectBridge(frame, {
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
// The external runtime target keeps its own FLAT naming: the entry is pushed as
// `game.fun` and siblings by basename, as it was before the preview moved to the
// multi-file seam (the device push has no site paths to mirror).
const runtimeTarget = createRuntimeTargetCore({
  getProject: () =>
    projectFiles().map((file, index): [string, string] => [
      // Every path has at least one segment, so `pop` always yields one.
      index === 0 ? "game.fun" : file.path.split("/").pop()!,
      file.source,
    ]),
  getAssets: () => assetSources,
  onOutput: (level, message) => statusBar.appendOutput(level, message),
});

// Set while loadExample replaces the buffer programmatically: that content is
// exactly what the fresh iframe is about to fetch, so pushing it back would
// be a redundant reload (and would mislabel a fresh load as a hot reload).
let programmaticEdit = false;
const editorKeybindings = createEditorKeybindingsController();

const view = new EditorView({
  parent: document.getElementById("editor")!,
  extensions: [
    editorKeybindings.extension,
    basicSetup,
    keymap.of([editorIndentWithTab]),
    functorLangLanguage,
    synthwaveEditorTheme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !programmaticEdit) {
        const files = projectFiles();
        bridge.setProject(files);
        mp?.push(files);
        runtimeTarget.projectChanged();
      }
    }),
  ],
});
editorKeybindings.attach(view);

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
  // The lint pass is of the file the editor is showing — name it, rather than
  // asserting a `game.fun` no example is actually called (the IDE's rule, with
  // the entry standing in for its `project.active`).
  const file = activePath();
  statusBar.setProblems(
    diags.map((d) => {
      const line = view.state.doc.lineAt(Math.min(d.from, view.state.doc.length));
      return {
        severity: d.severity,
        message: d.message,
        loc: `${file} ${line.number}:${d.from - line.from + 1}`,
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

// Load a fresh file set (example switch, inline load, reset): `files[0]` is the
// entry and becomes the editor's buffer.
const setDoc = (files: ProjectFile[], assets: [string, Uint8Array][] = []) => {
  bridge.reset();
  mp?.reset();
  // Wholesale document replacement (example switch, inline load, reset): drop
  // the wasm completion cache so the previous program's candidates can't leak.
  resetIntel();
  project = files;
  assetSources = assets;
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: files[0].source } });
  programmaticEdit = false;
  // …and drop the outgoing program's decorations (the IDE's setDoc does the
  // same): if the incoming source doesn't analyze cleanly, the keep-stale
  // lens rule would otherwise pin the old program's lenses to the new buffer.
  refreshIntel(view);
  runtimeTarget.projectChanged({ fresh: true });
};

const fromBase64Url = (b64u: string) =>
  new TextDecoder().decode(
    Uint8Array.from(atob(b64u.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0))
  );

let inlineB64: string | null = null;

// An inline (#src=) program is a one-file project; `main.fun` is module `Main`,
// which is what the old `?src=` data: URL boot derived for it.
const INLINE_ENTRY = "main.fun";

// The player iframe's boot params, common to every load:
//   project=inline — the PAGE owns every source and delivers it by postMessage
//     (project-bridge.ts); the player fetches no `.fun` at all;
//   scrubber=hidden — the page's chrono bar is the transport UI, so the player
//     mounts the __scrub seam with no bar of its own;
//   net=embedder, UNCONDITIONALLY — this host always routes pane networking
//     through its own coordinator (net-coordinator.ts) rather than letting a
//     pane open browser sockets. A program that declares no `Sub.connect` /
//     `Sub.listen` posts nothing, so the declaration costs it nothing.
const playerParams = (
  cursorPolicy: "visible" | null = pageCursorPolicy,
  mouseCapture: false | null = pageMouseCapture
) => {
  const params = new URLSearchParams({ project: "inline", scrubber: "hidden", net: "embedder" });
  if (mouseCapture === false) params.set("mouseCapture", "false");
  if (cursorPolicy) params.set("cursor", cursorPolicy);
  return params;
};

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
  setDoc([{ path: INLINE_ENTRY, source }]);
  setStatus("busy", "◌ loading…");
  // A fresh iframe, booted BY the initial project push, so the inline program
  // runs its OWN `init` (a hot-swap push would preserve the previous program's
  // model). `main.fun` keeps the module the old `?src=` data: URL derived.
  frame.src = `player.html?${playerParams()}`;
  bridge.setProject(project);
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
  // A fresh iframe (fresh model: init runs) rather than a hot-swap push, so
  // switching examples resets state; the project-waiting handshake boots it.
  setDoc(
    files.map((path, index): ProjectFile => ({ path, source: sources[index] })),
    assets
  );
  setStatus("busy", "◌ loading…");
  const cursorPolicy = example?.cursor ?? pageCursorPolicy;
  const mouseCapture = cursorPolicy
    ? null
    : example?.mouseCapture ?? pageMouseCapture;
  const params = playerParams(cursorPolicy, mouseCapture);
  // A same-file-entries sample plays its declared role: in the preferred form
  // the role's inline module (e.g. orbs' Client.init/Client.tick/…)…
  if (example?.module) params.set("module", example.module);
  // …or the transitional prefixed contract (clientInit/clientTick/…); the
  // player takes one of the two.
  if (example?.prefix) params.set("prefix", example.prefix);
  frame.src = `player.html?${params}`;
  bridge.setProject(project);
  // A sample with a SERVER role (examples.ts `server`) also boots a server
  // pane: the SAME file set re-entered at the server file. The pushed project
  // is entry-first, so the pane grid hoists that file to the front — the two
  // roles differ only in their entry and (for a same-file sample) their module.
  const serverFile = example?.server?.file;
  let server: PaneProgram | null = null;
  if (serverFile) {
    // Derived from the client's params, so every project setting the sample
    // declares (cursor, mouseCapture) reaches the server pane too — only the
    // entry and the ROLE differ.
    const serverParams = new URLSearchParams(params);
    // The client's ROLE must not leak into the server pane — neither form, so
    // both are dropped before the server's own is applied. A same-file sample
    // (orbs) has both roles in the one entry and differs only here; absent a
    // declared role, the server file's plain top-level contract is the role.
    serverParams.delete("module");
    serverParams.delete("prefix");
    if (example?.server?.module) serverParams.set("module", example.server.module);
    if (example?.server?.prefix) serverParams.set("prefix", example.server.prefix);
    server = { src: `player.html?${serverParams}`, entry: serverFile };
  }
  mp?.setSrc(frame.src, server);
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
createRoot(statusBarHost).render(
  <StatusBar store={statusBar} editorKeybindings={editorKeybindings} />
);

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
  keybindings: () => editorKeybindings.state.getSnapshot(),
  setKeybindings: (mode) => editorKeybindings.setMode(mode),
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
