// The sandbox page: a CodeMirror editor wired to the runtime iframe over the
// editor↔player postMessage seam. It speaks the IDE's MULTI-FILE seam
// (project-bridge.ts): the page owns every source of the loaded example, boots
// the player as `player.html?project=inline`, and pushes the whole file set as
// `functor-lang-set-project` — the runtime hot-swaps the program with the model
// preserved and replies `functor-lang-set-source-result`. ANY file of the
// example is editable: the sidebar (the IDE's FilePane, rows only — an example
// is read, not authored, so there is no new/delete/rename) switches which one
// the editor holds, and every edit pushes the whole set. So editing netpong's
// server.fun hot-reloads the server pane, at its own entry, model preserved.
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
  setLangContext,
  analyzeCached,
  completeAt,
  resetIntel,
  refreshIntel,
  refreshLiveValues,
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
import { SHARE_IDLE } from "./components/ShareButton.js";
import type { ShareState } from "./components/ShareButton.js";
import { ShareBanner } from "./components/ShareBanner.js";
import type { BannerState } from "./components/ShareBanner.js";
import { FilePane } from "./components/FilePane.js";
import type { FileListState } from "./components/FilePane.js";
import { decodeShare } from "./share-link.js";
import type { ShareProject, ShareRole } from "./share-link.js";
import { createShareController } from "./share.js";
import { initMultiplayerPanes, MAX_CLIENTS } from "./mp-panes.js";
import type { MultiplayerPanes, PaneProgram } from "./mp-panes.js";
import type { PillState } from "./components/StatusPill.js";
import { StatusBar } from "./components/StatusBar.js";
import { asPlayerMessage, entryFirst, fileName } from "./protocol.js";
import type { ProjectFile } from "./protocol.js";
import { EXAMPLES, exampleEntryPath } from "./examples.js";

/** The sandbox's e2e seam (driven by e2e/site-sandbox.mjs). */
interface SandboxSeam {
  setSource(source: string): void;
  source: () => string;
  status: () => { state: string; text: string; message: string };
  runtimeTarget: () => RuntimeTargetState;
  getSource: () => string;
  /** The loaded example's file set (entry first) and which file is open. */
  files: () => { paths: string[]; active: string };
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

// A project carried IN the URL, decoded by the one codec (share-link.ts):
//   #code=… — a Share link: the whole project (files, entry, roles, pointer
//     options), so an edited example — or a multiplayer one, server role and all
//     — reopens exactly as it was shared;
//   #src=…  — the docs' "try it" buttons: a single uncompressed program.
// Either becomes the editor's file set and a project pushed to a fresh player,
// so it starts with a fresh init — no served file involved. A fragment BEATS
// `?example=`: it is the more specific request, and it is the only copy of that
// project that exists.
const hashParams = new URLSearchParams(window.location.hash.slice(1));
const fragmentKind = hashParams.has("code") ? "code" : hashParams.has("src") ? "src" : null;
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
// The Share button's own label (it confirms itself) and the assets advisory.
const share = createStore<ShareState>(SHARE_IDLE);
const banner = createStore<BannerState>({ text: "" });

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
// player used to get by fetching them. The order is load-bearing beyond
// modules: every pane boots at `files[0]` (a role re-enters the same set at its
// own file — see mp-panes), so switching the OPEN file must never reorder it.
// `active` is a pointer, not a position.
let project: ProjectFile[] = [];
let active = "";
// Binary assets stay a fetched, page-relative concern (see the header note);
// they are only forwarded to an external runtime target, which has no site to
// fetch them from.
let assetSources: [string, Uint8Array][] = [];

/** The file the editor is showing. */
const activeFile = () => project.find((file) => file.path === active);

// Mirror the live buffer into the open file and hand back the file set to push.
const projectFiles = (): ProjectFile[] => {
  const file = activeFile();
  if (file) file.source = view.state.doc.toString();
  return project;
};

// The sidebar's view of the project: the paths, and which one is open. The
// list only appears for a multi-file example — a single-file sample looks
// exactly as it did before the switcher existed.
const fileList = createStore<FileListState>({ files: [], active: "" });
const fileListHost = document.querySelector(".file-pane") as HTMLElement;

const publishFiles = () => {
  fileList.set({ files: project.map((file) => file.path), active });
  fileListHost.hidden = project.length < 2;
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
// `game.fun` and siblings by file name, as it was before the preview moved to
// the multi-file seam (the device push has no site paths to mirror). KNOWN GAP,
// inherited: a sample whose sibling is itself named `game.fun` (netpong's
// shared renderer) collides with that convention on the device — the site
// preview, which pushes real paths, is unaffected.
const runtimeTarget = createRuntimeTargetCore({
  getProject: () =>
    projectFiles().map((file, index): [string, string] => [
      index === 0 ? "game.fun" : fileName(file.path),
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

// Project-aware language intelligence: the provider hands the whole loaded
// file set plus the open path, so diagnostics and completion see the sibling
// modules (netpong's client.fun calls `Protocol.*`, which single-file analysis
// could only report as unknown). A single-file example is the same call with
// one file.
setLangContext(() => ({ active, files: project }));

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
  // asserting a `game.fun` no example is actually called (the IDE's rule).
  const file = active;
  statusBar.setProblems(
    diags.map((d) => {
      const line = view.state.doc.lineAt(Math.min(d.from, view.state.doc.length));
      return {
        severity: d.severity,
        message: d.message,
        loc: `${file} ${line.number}:${d.from - line.from + 1}`,
        jump: () => {
          // A row can outlive the buffer it was measured in (a file switch, or
          // a whole example switch, during the lint debounce): reopen its file
          // before seeking, and drop the row entirely if it's gone.
          if (!project.some((candidate) => candidate.path === file)) return;
          if (active !== file) openFile(file);
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
  active = files[0].path; // a fresh load always opens the entry
  assetSources = assets;
  publishFiles();
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: files[0].source } });
  programmaticEdit = false;
  // …and drop the outgoing program's decorations (the IDE's setDoc does the
  // same): if the incoming source doesn't analyze cleanly, the keep-stale
  // lens rule would otherwise pin the old program's lenses to the new buffer.
  refreshIntel(view);
  runtimeTarget.projectChanged({ fresh: true });
};

// Switch which file the editor holds. Nothing is pushed: the file set is
// unchanged, so the running panes stay exactly as they are — only the buffer
// (and, through the language context, what analysis calls the active module)
// moves.
const openFile = (path: string) => {
  if (path === active) return;
  const next = project.find((file) => file.path === path);
  if (!next) return;
  const current = activeFile();
  if (current) current.source = view.state.doc.toString();
  active = path;
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next.source } });
  programmaticEdit = false;
  publishFiles();
  // A wholesale document replacement: drop the outgoing file's decorations and
  // force a fresh pass rather than waiting out the lint debounce, then re-gate
  // the paused trace's live values against the newly opened buffer.
  refreshIntel(view);
  refreshLiveValues(view);
};

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

// The picker row a URL-carried project occupies. The value is `__inline` from
// the days when `#src=` was the only such fragment — it is an internal id (the
// site e2e filters the picker by it), so the LABEL is what says which kind of
// link is loaded.
const SHARED_OPTION = "__inline";

/**
 * How the loaded project BOOTS: the role the editable pane plays, the server
 * role (if the project declares one), and the pointer policy. Set by every
 * loader; read by `bootPanes` to mount the panes and by `shareProject` to put
 * the same shape in a link — so a shared multiplayer sample reopens as a
 * multiplayer sample rather than as a lone client.
 */
// Every field is explicitly `| undefined`: a loader builds the whole record at
// once from optional sources (an example's fields, a decoded config), so under
// exactOptionalPropertyTypes "absent" and "undefined" have to be the same
// thing here.
interface Boot {
  module?: string | undefined;
  prefix?: string | undefined;
  /** The server pane's entry FILE (a path in `project`) and its role. */
  server?: { file: string; module?: string | undefined; prefix?: string | undefined } | undefined;
  cursor?: "visible" | undefined;
  mouseCapture?: false | undefined;
}
let boot: Boot = {};

/** A share role in `Boot`'s shape: the bare-string form names only a file. */
const bootRole = (role: ShareRole): { file: string; module?: string; prefix?: string } =>
  typeof role === "string" ? { file: role } : role;

// Mount the panes for the loaded project's declared roles: a FRESH iframe per
// pane (fresh model — `init` runs), each booted BY the project push rather than
// by fetching a served build.
const bootPanes = () => {
  const params = playerParams(boot.cursor ?? null, boot.mouseCapture ?? null);
  // A same-file-entries sample plays its declared role: in the preferred form
  // the role's inline module (e.g. orbs' Client.init/Client.tick/…)…
  if (boot.module) params.set("module", boot.module);
  // …or the transitional prefixed contract (clientInit/clientTick/…); the
  // player takes one of the two.
  if (boot.prefix) params.set("prefix", boot.prefix);
  frame.src = `player.html?${params}`;
  bridge.setProject(project);
  // A project with a SERVER role also boots a server pane: the SAME file set
  // re-entered at the server file. The pushed project is entry-first, so the
  // pane grid hoists that file to the front — the two roles differ only in
  // their entry and (for a same-file project) their module.
  let server: PaneProgram | null = null;
  if (boot.server) {
    // Derived from the client's params, so every project setting (cursor,
    // mouseCapture) reaches the server pane too — only the entry and the ROLE
    // differ.
    const serverParams = new URLSearchParams(params);
    // The client's ROLE must not leak into the server pane — neither form, so
    // both are dropped before the server's own is applied. A same-file sample
    // (orbs) has both roles in the one entry and differs only here; absent a
    // declared role, the server file's plain top-level contract is the role.
    serverParams.delete("module");
    serverParams.delete("prefix");
    if (boot.server.module) serverParams.set("module", boot.server.module);
    if (boot.server.prefix) serverParams.set("prefix", boot.server.prefix);
    server = { src: `player.html?${serverParams}`, entry: boot.server.file };
  }
  mp?.setSrc(frame.src, server);
};

// The project the fragment carried, kept so Reset can reload it (there is no
// served copy to re-fetch) and so the picker can name it. It outlives an example
// switch, exactly as the inline snippet used to: its picker row stays, so
// selecting that row again has to still find it.
let shared: { project: ShareProject; label: string } | null = null;

// Load a project that arrived IN the URL. The fragment is the storage, so this
// is a complete load: its file set becomes the editor's, and its declared roles
// boot the panes.
const loadShared = (project_: ShareProject, label: string) => {
  shared = { project: project_, label };
  const entries = project_.config?.entries;
  const client = entries?.client ? bootRole(entries.client) : null;
  // The editable pane's entry: the client role's file, the declared entry, or
  // the codec's default. A decoded project always contains the roles' files and
  // its declared entry, but a roles-only project need not have a `game.fun` —
  // fall back to its first file rather than to a name that isn't there.
  const wanted = client?.file ?? project_.entry ?? "game.fun";
  const entry = project_.files.some((file) => file.path === wanted)
    ? wanted
    : project_.files[0].path;
  // COPIES: these become the live buffers, which every keystroke mutates in
  // place (projectFiles) — handing over the retained objects would quietly turn
  // Reset into "reload my edits".
  const files = entryFirst(project_.files, entry).map((file) => ({ ...file }));
  boot = {
    module: client?.module,
    prefix: client?.prefix,
    server: entries?.server ? bootRole(entries.server) : undefined,
    // The page's own `?cursor=` / `?mouseCapture=` seam still applies to a
    // URL-carried project that declares nothing (that is how the docs' `#src=`
    // snippets have always taken a pointer policy).
    cursor: project_.options?.cursor ?? pageCursorPolicy ?? undefined,
    mouseCapture:
      project_.options?.cursor || pageCursorPolicy
        ? undefined
        : (project_.options?.mouseCapture ?? pageMouseCapture) === false
          ? false
          : undefined,
  };
  // Reflect the loaded link in the picker so it (and Reset) don't lie about
  // what's running.
  const options = picker.getSnapshot().options;
  picker.set({
    options: options.some((option) => option.value === SHARED_OPTION)
      ? options.map((option) =>
          option.value === SHARED_OPTION ? { value: SHARED_OPTION, label } : option
        )
      : [...options, { value: SHARED_OPTION, label }],
    selected: SHARED_OPTION,
  });
  loadToken += 1; // supersede any in-flight example fetch
  setDoc(files);
  setStatus("busy", "◌ loading…");
  bootPanes();
  // A shared multiplayer project keeps the CLIENTS control: its server role is
  // as much a reason to show it as an example's `multiplayer` flag.
  if (boot.server) clients.set({ count: mp!.count(), visible: true });
  // Every LOAD re-runs the advisory (`sharing` is initialized before any load
  // happens — see the boot block at the bottom), so the strip always describes
  // the project that is open, however it arrived.
  sharing.checkAssets();
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
  shared = null;
  const cursorPolicy = example?.cursor ?? pageCursorPolicy;
  boot = {
    module: example?.module,
    prefix: example?.prefix,
    // examples.ts `server` — the sample's declared server role, whose file the
    // fetched list above already contains.
    server: example?.server,
    cursor: cursorPolicy ?? undefined,
    // A visible cursor already implies no capture, so only the capture-only
    // samples carry the flag.
    mouseCapture:
      !cursorPolicy && (example?.mouseCapture ?? pageMouseCapture) === false ? false : undefined,
  };
  bootPanes();
  sharing.checkAssets();
};

const selectExample = (value: string) => {
  picker.set({ ...picker.getSnapshot(), selected: value });
  if (value === SHARED_OPTION) {
    // That option only exists once a fragment has been decoded into `shared`.
    loadShared(shared!.project, shared!.label);
    return;
  }
  const url = new URL(window.location.href);
  url.searchParams.set("example", value);
  // Drop the URL-carried project from the hash — both forms, since either would
  // outrank `?example=` on the next reload — but keep #clients=N alive.
  const hash = new URLSearchParams(window.location.hash.slice(1));
  hash.delete("src");
  hash.delete("code");
  url.hash = hash.toString();
  window.history.replaceState(null, "", url);
  updateClientsStore();
  loadExample(value);
};

const resetExample = () => {
  const selected = picker.getSnapshot().selected;
  if (selected === SHARED_OPTION) loadShared(shared!.project, shared!.label);
  else loadExample(selected);
};

// Share: encode the LIVE project — every file with its edits, the entry, the
// roles (so a shared multiplayer sample reopens with its server pane) and the
// pointer options — into this page's own URL, then copy that URL. The fragment
// goes in with replaceState (no navigation, and #clients=N survives), which is
// also what makes the address bar a real fallback when the clipboard says no.
const shareProject = (): ShareProject => {
  // Flattened to bare module files: the codec's module space has no
  // directories, and the runtime names a module by its file STEM — so
  // `examples/netpong/server.fun` and `server.fun` are the same module `Server`.
  const files = projectFiles().map((file) => ({
    path: fileName(file.path),
    source: file.source,
  }));
  const role = (
    file: string,
    of: { module?: string | undefined; prefix?: string | undefined }
  ): ShareRole =>
    of.module ? { file, module: of.module } : of.prefix ? { file, prefix: of.prefix } : file;
  const carried: ShareProject = { files, entry: files[0].path };
  if (boot.server) {
    carried.config = {
      entries: {
        client: role(files[0].path, boot),
        server: role(fileName(boot.server.file), boot.server),
      },
    };
  } else if (boot.module || boot.prefix) {
    carried.config = { entries: { client: role(files[0].path, boot) } };
  }
  if (boot.cursor) carried.options = { cursor: boot.cursor };
  else if (boot.mouseCapture === false) carried.options = { mouseCapture: false };
  return carried;
};

const sharing = createShareController({
  share,
  banner,
  project: shareProject,
  files: () => projectFiles(),
  onOutput: (level, message) => statusBar.appendOutput(level, message),
  // `?example=` is dropped from a share URL: the fragment carries the project,
  // and a query still naming the sample it came from would only mislabel the
  // link (and lose, on the next reload, to the fragment anyway).
  href: () => {
    const url = new URL(window.location.href);
    url.searchParams.delete("example");
    return url.toString();
  },
});

// Mount the islands into the static shell's containers. Both keep their
// element ids and class names, so styles.css and every e2e selector match the
// rendered DOM exactly as they matched the hand-built one.
createRoot(document.querySelector(".sandbox-controls")!).render(
  <SandboxControls
    picker={picker}
    pill={pill}
    clients={clients}
    share={share}
    runtimeTarget={runtimeTarget}
    onSelect={selectExample}
    onReset={resetExample}
    onClients={selectClients}
    onShare={sharing.shareLink}
  />
);
// The one thing a link can't promise (share.ts): a strip above the editor, so
// it is read where the sources it is about are.
createRoot(document.querySelector(".share-banner-host")!).render(
  <ShareBanner store={banner} onDismiss={() => banner.set({ text: "" })} />
);
// Rows only: `onNew`/`onDelete` are the IDE's, and an example's file set is
// the sample's rather than the reader's. The host element carries the
// sidebar's visibility (publishFiles), so a single-file example renders none.
// No `entry` either — the loaded example's root is `files[0]`, which the pane
// falls back to for the tooltip.
createRoot(fileListHost).render(<FilePane store={fileList} onOpen={openFile} />);
const statusBarHost = document.getElementById("statusbar")!;
statusBarHost.className = "statusbar";
createRoot(statusBarHost).render(
  <StatusBar store={statusBar} editorKeybindings={editorKeybindings} />
);

// A fragment wins over `?example=`, but only if it decodes: a mangled link (a
// chat client that broke the URL) says why and falls back to an example rather
// than leaving the page empty.
if (fragmentKind) {
  void decodeShare(window.location.hash).then((carried) => {
    if (carried) {
      loadShared(carried, fragmentKind === "code" ? "shared link" : "docs snippet");
      return;
    }
    const message = `the #${fragmentKind}= fragment is not a valid project — loading an example instead`;
    statusBar.appendOutput("error", message);
    setStatus("error", "✖ error", message);
    loadExample(initialExample);
  });
} else {
  loadExample(initialExample);
}

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
  files: () => ({ paths: project.map((file) => file.path), active }),
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
