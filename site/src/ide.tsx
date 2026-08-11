// The web IDE: a file sidebar + multi-file Functor Lang editor + live preview.
// The IDE holds the WHOLE project in memory (nothing is served) and pushes it
// over the `functor-lang-set-project` seam to a `player.html?project=inline`
// iframe (see project-bridge.ts): the preview boots from memory and hot-swaps
// on every edit, model preserved. Work is persisted to localStorage; the
// project downloads as a .zip that drops into `functor -d <dir> build wasm`.
//
// The page shell is static HTML; the CHROME around the editor (the toolbar, the
// pill, the external-runtime panel, the file sidebar, the status bar) renders as
// React islands mounted into that shell's containers — the same architecture the
// sandbox uses, sharing the same components. The editor itself is not React
// (CodeMirror owns `#editor`'s subtree), and the project logic below stays plain
// imperative code that publishes to small stores.

import { createRoot } from "react-dom/client";
import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { acceptCompletion, closeCompletion, startCompletion } from "@codemirror/autocomplete";
import { StateEffect } from "@codemirror/state";
import {
  createEditorKeybindingsController,
  editorIndentWithTab,
} from "./editor-keybindings.js";
import type { EditorKeybindings, EditorKeybindingsState } from "./editor-keybindings.js";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import {
  setupLangIntel,
  setLangContext,
  resetIntel,
  refreshIntel,
  onDiagnostics,
  wireLiveTrace,
  refreshLiveValues,
  currentExpects,
} from "./lang-intel.js";
import { ProjectBridge } from "./project-bridge.js";
import { createStatusBarStore } from "./status-bar-store.js";
import { createRuntimeTargetCore } from "./runtime-target-core.js";
import type { RuntimeTargetState } from "./runtime-target-core.js";
import { createStore } from "./store.js";
import { IdeControls } from "./components/IdeControls.js";
import { SHARE_IDLE } from "./components/ShareButton.js";
import type { ShareState } from "./components/ShareButton.js";
import { ShareBanner } from "./components/ShareBanner.js";
import type { BannerState } from "./components/ShareBanner.js";
import { StatusBar } from "./components/StatusBar.js";
import { FilePane, ActiveFileTab } from "./components/FilePane.js";
import type { FileListState } from "./components/FilePane.js";
import type { PillState } from "./components/StatusPill.js";
import { asPlayerMessage } from "./protocol.js";
import type { ProjectFile } from "./protocol.js";
import { zipFiles } from "./zip.js";
// The project-file rule lives with the share-link codec, which enforces the
// same thing on a decoded fragment — one definition for both entrances into
// the IDE's flat module space.
import { MODULE_FILE, decodeShare } from "./share-link.js";
import type { ShareProject } from "./share-link.js";
import { shareHref, copyLink, unservedAssets, assetWarning } from "./share.js";

/** The in-memory project: a flat module space plus the open file's path. */
interface Project {
  active: string;
  files: ProjectFile[];
}

/**
 * The project as read back from localStorage. This is the shape it is EXPECTED
 * to have, not one anything guarantees: the store is user-editable, so every
 * field is optional and `loadProject` re-validates each one at runtime (a
 * mismatch falls through to the starter). Typing it as `unknown` instead would
 * only move those same runtime checks behind a wall of casts.
 */
interface StoredProject {
  active?: string;
  files?: ProjectFile[];
}

/** `validName`'s verdict: exactly one of the two fields is ever present. */
interface NameCheck {
  path?: string;
  error?: string;
}

/** The IDE's e2e seam (driven by e2e/ide-page.mjs and e2e/ide-project.mjs). */
interface IdeSeam {
  setActiveSource(source: string): void;
  openFile: (path: string) => void;
  newFile: (path: string, source?: string) => void;
  files: () => ProjectFile[];
  status: () => { state: string; text: string; message: string };
  runtimeTarget: () => RuntimeTargetState;
  triggerComplete(source: string, cursor: number): void;
  acceptCompletion: () => boolean;
  keybindings: () => EditorKeybindingsState;
  setKeybindings: (mode: EditorKeybindings) => Promise<void>;
}

/** The readiness seam, shared in NAME (not shape) with the sandbox's. */
interface LangSeam {
  ready: Promise<boolean>;
  expects: () => ReturnType<typeof currentExpects>;
}

const STORAGE_KEY = "functor-ide-project-v1";
const ENTRY = "game.fun"; // the program root; every other .fun is a sibling module
// The in-memory IDE has no functor.json to parse, so its URL is the explicit
// Captured game input defaults on. `?mouseCapture=false` opts the preview and
// downloaded manifest out; `?cursor=visible` selects absolute pointer input.
const pageParams = new URLSearchParams(window.location.search);
const cursorPolicy = pageParams.get("cursor") === "visible" ? "visible" : null;
const mouseCapture =
  !cursorPolicy && pageParams.get("mouseCapture") === "false" ? false : null;
const playerUrl = () => {
  const params = new URLSearchParams({ project: "inline" });
  if (mouseCapture === false) params.set("mouseCapture", "false");
  if (cursorPolicy) params.set("cursor", cursorPolicy);
  return `player.html?${params}`;
};

// A two-file starter: game.fun draws using constants from palette.fun (a
// sibling module — file = module, so palette.fun is module `Palette`), to show
// the multi-file loop the sandbox can't.
const STARTER: Project = {
  active: ENTRY,
  files: [
    {
      path: ENTRY,
      source: `// A multi-file starter. palette.fun is a sibling module (file = module,
// so it is \`Palette\`). Edit either file — the preview hot-reloads with the
// model preserved. Add files with + in the sidebar; download the project as
// a .zip to run it with \`functor -d <dir> build wasm\`.
let init = { t: 0.0 }

let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }

let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.5, 0.0)),
    Scene.group([
      Scene.sphere() |> Scene.emissive(Color.rgb(0.15, 1.0, Palette.glow)) |> Scene.scale(1.4),
      Scene.plane() |> Scene.scale(12.0) |> Scene.lit(Color.rgb(Palette.sky, 0.12, 0.35)),
    ]))
`,
    },
    {
      path: "palette.fun",
      source: `// Constants for the scene, edited on their own. Try changing these — the
// sphere and ground recolor live.
let glow = 0.85
let sky = 0.18
`,
    },
  ],
};

const els = {
  editorHost: document.getElementById("editor")!,
  player: document.getElementById("player") as HTMLIFrameElement,
};

// The two stores the islands render. `pill` is the preview's live indicator;
// `fileList` is the sidebar's (and the editor tab's) view of the project — the
// paths and which one is open, republished wherever `renderFileList` used to
// rebuild the DOM.
const pill = createStore<PillState>({ state: "busy", text: "◌ loading…", detail: "" });
const fileList = createStore<FileListState>({ files: [], active: ENTRY });
// The Share button's own label (it confirms itself) and the assets advisory.
const share = createStore<ShareState>(SHARE_IDLE);
const banner = createStore<BannerState>({ text: "" });

// ---------------------------------------------------------------- project

// Whether localStorage held a project of the reader's OWN when the page loaded.
// A share link asks before displacing one of those, and never asks when the
// only thing it would displace is the starter.
let storedProject = false;
// Whether the in-memory project may be written back to localStorage. False for
// exactly one state: a project that arrived in a LINK and has not been touched.
// The reader's own saved project survives merely LOOKING at a shared one.
let persist = true;

let project = loadProject();

function loadProject(): Project {
  try {
    // A missing key parses as `null` (JSON.parse coerces), which the validation
    // below rejects — the starter is the fallback either way.
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!) as StoredProject | null;
    const seen = new Set();
    const valid =
      stored &&
      Array.isArray(stored.files) &&
      stored.files.length > 0 &&
      stored.files.every((f) => {
        if (!f || typeof f.path !== "string" || typeof f.source !== "string") return false;
        if (!MODULE_FILE.test(f.path)) return false; // same rule as created files
        const key = f.path.toLowerCase();
        if (seen.has(key)) return false; // no case-insensitive duplicates (one entry)
        seen.add(key);
        return true;
      }) &&
      stored.files.some((f) => f.path === ENTRY);
    if (valid) {
      // The guard proves `active` matched a validated (string) path.
      const active = stored.files!.some((f) => f.path === stored.active)
        ? stored.active!
        : ENTRY;
      // The loader's contract (preview AND language analysis): the ENTRY is
      // files[0] — its module is the program root. Every mutation here keeps
      // that order, but a hand-edited localStorage could reorder.
      const files = [
        ...stored.files!.filter((f) => f.path === ENTRY),
        ...stored.files!.filter((f) => f.path !== ENTRY),
      ];
      storedProject = true;
      return { files, active };
    }
  } catch {
    // fall through to the starter
  }
  return structuredClone(STARTER);
}

const saveProject = () => {
  // An untouched shared project is not the reader's work yet — see `persist`.
  if (!persist) return;
  // Best-effort: a disabled/full localStorage (private mode, quota) must not
  // break editing or the live preview — persistence is a convenience.
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(project));
  } catch {
    /* not persisted this session */
  }
};
const activeFile = () => project.files.find((f) => f.path === project.active);

// ---------------------------------------------------------------- status

const setStatus = (state: PillState["state"], text: string, detail = "") => {
  // The detail (the reload note, or a parse error) lives in the pill's tooltip
  // and — for errors — the Output panel. No separate error banner under the
  // editor: the preview pill is the single live indicator.
  pill.set({ state, text, detail });
};

// The boot loader (static markup in ide.html) evaporates when the PREVIEW
// reports in — never on any other status change, since an error from, say, a
// rejected new-file name says nothing about whether the pane has pixels yet.
// One class toggle; the 620ms evaporate is all CSS.
const dismissBootLoader = () =>
  document.querySelector("[data-fn-boot]")?.classList.add("is-done");

// ---------------------------------------------------------------- editor

let programmaticEdit = false;
const editorKeybindings = createEditorKeybindingsController();

const view = new EditorView({
  parent: els.editorHost,
  extensions: [
    editorKeybindings.extension,
    basicSetup,
    keymap.of([editorIndentWithTab]),
    functorLangLanguage,
    synthwaveEditorTheme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && !programmaticEdit) {
        // Mirror the buffer into the active file and push the whole project.
        const file = activeFile();
        if (file) file.source = view.state.doc.toString();
        schedulePush();
      }
    }),
  ],
});
editorKeybindings.attach(view);

// Live language intelligence (diagnostics/hover/completion/inlays), shared
// with the sandbox but project-aware here: the context provider hands the
// whole file set + the active path, so sibling modules resolve (Palette.glow
// from palette.fun). Degrades silently when the pkg is absent.
setLangContext(() => ({ active: project.active, files: project.files }));
const langReady = setupLangIntel().then((extensions) => {
  if (extensions.length) view.dispatch({ effects: StateEffect.appendConfig.of(extensions) });
  return extensions.length > 0;
});

const statusBar = createStatusBarStore();
// Created once, outside React: this controller carries the live link's queued
// pushes and its `/state` poll chain, so a re-render must never restart it.
const runtimeTarget = createRuntimeTargetCore({
  getProject: () => project.files,
  onOutput: (level, message) => statusBar.appendOutput(level, message),
});

// Each lint pass (of the ACTIVE file — the per-document model) refreshes the
// Problems panel; clicking a problem jumps the editor to it. Positions
// re-clamp at click time (the doc may have moved on).
onDiagnostics((diags) => {
  const file = project.active;
  statusBar.setProblems(
    diags.map((d) => {
      const line = view.state.doc.lineAt(Math.min(d.from, view.state.doc.length));
      return {
        severity: d.severity,
        message: d.message,
        loc: `${file} ${line.number}:${d.from - line.from + 1}`,
        jump: () => {
          // A row can outlive its file (delete + the debounce window).
          if (!project.files.some((f) => f.path === file)) return;
          if (project.active !== file) openFile(file);
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

// Runtime console traces (Functor Lang `Debug.log` and friends), forwarded by the
// player page — see the console hook in player.html. Guarded to OUR iframe.
window.addEventListener("message", (event) => {
  const data = asPlayerMessage(event.data);
  if (!data || data.type !== "functor-lang-console") return;
  if (event.source !== els.player.contentWindow) return;
  statusBar.appendOutput(data.level, data.message, data.frame ?? null);
});

// The paused-inspector trace (live values in the editor + the executions
// picker), relayed by the player on pause / paused-frame change. A file
// switch keeps the trace — refreshLiveValues re-gates against the newly
// opened buffer's hash (openFile below calls it after setDoc).
wireLiveTrace(view, statusBar, els.player, langReady);

const setDoc = (source: string) => {
  programmaticEdit = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  programmaticEdit = false;
  // Wholesale document replacement (file switch, delete, e2e seam): the
  // outgoing file's decorations are meaningless on this buffer — drop them now
  // and force a fresh pass rather than waiting out the lint debounce. The wasm
  // completion cache is NOT cleared: it holds the same project, and completion
  // passes the active module per call.
  refreshIntel(view);
};

// ---------------------------------------------------------------- preview

const bridge = new ProjectBridge(els.player, {
  onReloading: () => setStatus("busy", "◌ reloading…"),
  onLive: () => {
    dismissBootLoader();
    setStatus("live", "● live");
  },
  onResult: (ok, message) => {
    dismissBootLoader();
    if (ok) {
      // the runtime's "model preserved" note, reachable on hover
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

// Persist + push the current file set (the bridge debounces the actual send).
const schedulePush = () => {
  // Every caller is a MUTATION (an edit, a new file, a delete), so this is also
  // where a shared project becomes the reader's: from the first change on, it
  // persists like any other. Merely opening files (openFile → saveProject) is
  // not a change and does not adopt it.
  persist = true;
  saveProject();
  bridge.setProject(project.files);
  runtimeTarget.projectChanged();
};

// ---------------------------------------------------------------- sidebar

// The sidebar and the editor tab re-render from this one publication — the
// declarative replacement for the old teardown-and-rebuild of every row.
const publishFiles = () => {
  fileList.set({ files: project.files.map((f) => f.path), active: project.active });
};

const openFile = (path: string) => {
  if (path === project.active) return;
  // A stale caller (e.g. a problem row outliving a delete) must not point
  // `active` at a file that no longer exists.
  if (!project.files.some((f) => f.path === path)) return;
  // Save the live buffer into the outgoing file before switching.
  const current = activeFile();
  if (current) current.source = view.state.doc.toString();
  project.active = path;
  const next = activeFile();
  setDoc(next ? next.source : "");
  publishFiles();
  saveProject();
  // The paused trace carries per-file hashes: re-gate the live overlay
  // against the newly opened buffer (the doc swap just cleared it).
  refreshLiveValues(view);
};

// A valid sibling filename: `<name>.fun`, a bare module stem (no path
// separators — the project is a flat module space), and not already taken.
const validName = (raw: string): NameCheck => {
  const path = raw.trim();
  if (!MODULE_FILE.test(path)) {
    return { error: "name must be a bare module like `enemy.fun` (letters, digits, _)" };
  }
  if (project.files.some((f) => f.path.toLowerCase() === path.toLowerCase())) {
    return { error: `${path} already exists` };
  }
  return { path };
};

const newFile = () => {
  const raw = window.prompt("New file name (e.g. enemy.fun):", "");
  if (raw === null) return;
  const { path, error } = validName(raw);
  if (error) {
    setStatus("error", "✖ error", error);
    return;
  }
  // `error` was falsy, so validName returned the other arm: `path` is set.
  project.files.push({ path: path!, source: `// ${path}\n` });
  openFile(path!);
  schedulePush(); // a new empty module can't break the build; keep the preview in sync
};

const deleteFile = (path: string) => {
  if (path === ENTRY) return;
  if (!window.confirm(`Delete ${path}? This can't be undone.`)) return;
  project.files = project.files.filter((f) => f.path !== path);
  // The deleted module must leave the completion candidates (the wasm
  // last-good cache still holds it until the next clean load).
  resetIntel();
  if (project.active === path) {
    project.active = ENTRY;
    setDoc(activeFile()!.source);
  } else {
    // Topology changed under an unchanged buffer: without a doc change the
    // linter never reruns, leaving diagnostics/inlays/lenses stale forever.
    refreshIntel(view);
  }
  publishFiles();
  schedulePush();
};

// ---------------------------------------------------------------- toolbar

const download = () => {
  // Include the functor.json the CLI needs to recognise the project, so the
  // zip drops straight into `functor -d <dir> build wasm` (per the README).
  const config = {
    language: "functor-lang",
    entry: ENTRY,
    ...(mouseCapture === false ? { mouseCapture: false } : {}),
    ...(cursorPolicy ? { cursor: cursorPolicy } : {}),
  };
  const manifest = {
    path: "functor.json",
    source: JSON.stringify(config, null, 2) + "\n",
  };
  const blob = zipFiles([manifest, ...project.files]);
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "functor-project.zip";
  a.click();
  // Revoke after the click's synchronous download kickoff has settled (a
  // same-tick revoke is the known-fragile pattern on some browsers).
  setTimeout(() => URL.revokeObjectURL(url), 0);
};

// Reload the preview from scratch — a fresh model (init runs again) with the
// current files. The iframe re-announces project-waiting; the bridge reboots.
const restart = () => {
  bridge.reset();
  setStatus("busy", "◌ loading…");
  els.player.src = playerUrl();
  bridge.setProject(project.files);
  runtimeTarget.restart();
};

// ---------------------------------------------------------------- sharing

// The assets advisory: the project's relative `Asset.*` locators this site does
// not serve — the only thing a link genuinely drops (share.ts explains why the
// site-served ones are fine). The IDE serves nothing of its own, so an authored
// locator that isn't already on the site shows up here.
const warnAboutAssets = () => {
  void unservedAssets(project.files).then((missing) => {
    if (missing.length === 0) return;
    const text = assetWarning(missing);
    banner.set({ text });
    statusBar.appendOutput("warn", text);
  });
};

let shareFlash = 0;
const flashShare = (state: ShareState) => {
  share.set(state);
  window.clearTimeout(shareFlash);
  shareFlash = window.setTimeout(() => share.set(SHARE_IDLE), 2600);
};

// Share: the whole in-memory project in the page's own URL. The IDE's paths are
// already bare module files and its entry is always `game.fun`, so there is
// nothing to flatten — only the pointer options ride along, matching the
// manifest the zip download writes.
const shareLink = async () => {
  const carried: ShareProject = { files: project.files, entry: ENTRY };
  if (cursorPolicy) carried.options = { cursor: cursorPolicy };
  else if (mouseCapture === false) carried.options = { mouseCapture: false };
  let url: string;
  try {
    url = await shareHref(carried, window.location.href);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    statusBar.appendOutput("error", message);
    flashShare({ label: "✖ can't share", tone: "error", detail: message });
    return;
  }
  window.history.replaceState(null, "", url);
  const copied = await copyLink(url);
  flashShare(
    copied
      ? { label: "✓ copied", tone: "ok", detail: url }
      : {
          label: "⧉ copy the URL",
          tone: "error",
          detail: "the clipboard refused — the link is in the address bar",
        }
  );
  warnAboutAssets();
};

// Open a project that arrived in a `#code=` link. It is NOT saved: `persist`
// stays false until the reader changes something, so the project already in
// this browser is still there if they reload without the link.
const openShared = (carried: ShareProject) => {
  persist = false;
  project = {
    active: ENTRY,
    files: [
      ...carried.files.filter((file) => file.path === ENTRY),
      ...carried.files.filter((file) => file.path !== ENTRY),
    ],
  };
  // The incoming module space is a different program: the last-good completion
  // cache would otherwise keep offering the outgoing one's members.
  resetIntel();
  publishFiles();
  setDoc(activeFile()!.source);
  restart(); // a fresh iframe, so the shared program runs its OWN init
  warnAboutAssets();
};

// ---------------------------------------------------------------- boot

// Mount the islands into the static shell's containers. Each keeps its element
// ids and class names, so styles.css and every e2e selector match the rendered
// DOM exactly as they matched the hand-built one.
createRoot(document.querySelector(".sandbox-controls")!).render(
  <IdeControls
    pill={pill}
    share={share}
    runtimeTarget={runtimeTarget}
    onDownload={download}
    onRestart={restart}
    onShare={shareLink}
  />
);
// The one thing a link can't promise (share.ts): a strip above the editor.
createRoot(document.querySelector(".share-banner-host")!).render(
  <ShareBanner store={banner} onDismiss={() => banner.set({ text: "" })} />
);
createRoot(document.querySelector(".file-pane")!).render(
  <FilePane
    store={fileList}
    entry={ENTRY}
    onOpen={openFile}
    onDelete={deleteFile}
    onNew={newFile}
  />
);
createRoot(document.querySelector(".editor-tab")!).render(<ActiveFileTab store={fileList} />);
const statusBarHost = document.getElementById("statusbar")!;
statusBarHost.className = "statusbar";
createRoot(statusBarHost).render(
  <StatusBar store={statusBar} editorKeybindings={editorKeybindings} />
);

publishFiles();
setDoc(activeFile()!.source);
setStatus("busy", "◌ loading…");
// Store the file set BEFORE the iframe loads, so the bridge can flush it the
// moment the player announces it's ready (no lost first push).
bridge.setProject(project.files);
els.player.src = playerUrl();

// A `#code=` link outranks the stored project — but never silently, and never
// destructively:
//   • no stored project of the reader's own (first visit, or the starter) → the
//     shared project just opens;
//   • a stored project → ASK. Cancel keeps their project and drops `code` from
//     the URL, so a reload doesn't ask again;
//   • either way the shared project is not written to localStorage until the
//     reader changes something (`persist`), so their work survives a look.
// A shared project whose entry isn't `game.fun` (a sandbox example's link, say)
// is refused rather than renamed: renaming the entry would silently rename its
// MODULE and break every sibling that calls into it.
if (new URLSearchParams(window.location.hash.slice(1)).has("code")) {
  void decodeShare(window.location.hash).then((carried) => {
    if (!carried) {
      setStatus("error", "✖ error", "this share link doesn't carry a valid project");
      return;
    }
    if (!carried.files.some((file) => file.path === ENTRY)) {
      setStatus(
        "error",
        "✖ error",
        `this link's project starts at ${carried.entry ?? carried.files[0].path}; ` +
          `the IDE's entry is ${ENTRY}`
      );
      return;
    }
    if (
      storedProject &&
      !window.confirm(
        "Open the shared project from this link?\n\n" +
          "Your saved project stays in this browser — it is only replaced once you " +
          "edit the shared one."
      )
    ) {
      const url = new URL(window.location.href);
      const hash = new URLSearchParams(url.hash.slice(1));
      hash.delete("code");
      url.hash = hash.toString();
      window.history.replaceState(null, "", url);
      return;
    }
    openShared(carried);
  });
}

// Test seam for the headless e2e (e2e/ide-page.mjs): drive files without
// synthesizing DOM events, and read status.
(window as Window & { __ide?: IdeSeam }).__ide = {
  setActiveSource(source) {
    setDoc(source);
    const file = activeFile();
    if (file) file.source = source;
    schedulePush();
  },
  openFile,
  newFile: (path, source = `// ${path}\n`) => {
    project.files.push({ path, source });
    openFile(path);
    schedulePush();
  },
  files: () => project.files.map((f) => ({ ...f })),
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
  // Replace the active buffer, place the cursor, and open the completion popup
  // (explicit trigger) — the sandbox's seam, minus any push (programmaticEdit
  // suppresses the mirror-and-push listener).
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
  acceptCompletion: () => acceptCompletion(view),
};

// Whether language analysis is available (false = degraded, pkg absent) — the
// same readiness seam the sandbox exposes for e2e.
(window as Window & { __lang?: LangSeam }).__lang = {
  ready: langReady,
  expects: () => currentExpects(view),
};
