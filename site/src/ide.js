// The web IDE: a file sidebar + multi-file Functor Lang editor + live preview.
// The IDE holds the WHOLE project in memory (nothing is served) and pushes it
// over the `functor-lang-set-project` seam to a `player.html?project=inline`
// iframe (see project-bridge.js): the preview boots from memory and hot-swaps
// on every edit, model preserved. Work is persisted to localStorage; the
// project downloads as a .zip that drops into `functor -d <dir> build wasm`.

import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";
import { acceptCompletion, closeCompletion, startCompletion } from "@codemirror/autocomplete";
import { StateEffect } from "@codemirror/state";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";
import { setupLangIntel, setLangContext, resetIntel, refreshIntel, onDiagnostics } from "./lang-intel.js";
import { ProjectBridge } from "./project-bridge.js";
import { createStatusBar } from "./status-bar.js";
import { zipFiles } from "./zip.js";

const STORAGE_KEY = "functor-ide-project-v1";
const ENTRY = "game.fun"; // the program root; every other .fun is a sibling module
// A valid project file: a bare module name + `.fun` (the project is a flat
// module space — no path separators). Enforced on BOTH created and loaded
// files, so a hand-edited/corrupt localStorage can't smuggle in a `../x.fun`
// (which would be a zip-slip entry on download and a bad module at load).
const MODULE_FILE = /^[A-Za-z][A-Za-z0-9_]*\.fun$/;

// A two-file starter: game.fun draws using constants from palette.fun (a
// sibling module — file = module, so palette.fun is module `Palette`), to show
// the multi-file loop the sandbox can't.
const STARTER = {
  active: ENTRY,
  files: [
    {
      path: ENTRY,
      source: `// A multi-file starter. palette.fun is a sibling module (file = module,
// so it is \`Palette\`). Edit either file — the preview hot-reloads with the
// model preserved. Add files with + in the sidebar; download the project as
// a .zip to run it with \`functor -d <dir> build wasm\`.
let init = { t: 0.0 }

let tick = (model, dt: Float, tts: Float) => { model with t: model.t + dt }

let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.5, 0.0),
    Scene.group([
      Scene.sphere() |> Scene.emissive(0.15, 1.0, Palette.glow) |> Scene.scale(1.4),
      Scene.plane() |> Scene.scale(12.0) |> Scene.lit(Palette.sky, 0.12, 0.35),
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
  fileList: document.getElementById("file-list"),
  newFile: document.getElementById("new-file"),
  download: document.getElementById("download"),
  restart: document.getElementById("restart"),
  status: document.getElementById("status"),
  statusLog: document.getElementById("status-log"),
  activeName: document.getElementById("active-file"),
  editorHost: document.getElementById("editor"),
  player: document.getElementById("player"),
};

// ---------------------------------------------------------------- project

let project = loadProject();

function loadProject() {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY));
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
      const active = stored.files.some((f) => f.path === stored.active) ? stored.active : ENTRY;
      // The loader's contract (preview AND language analysis): the ENTRY is
      // files[0] — its module is the program root. Every mutation here keeps
      // that order, but a hand-edited localStorage could reorder.
      const files = [
        ...stored.files.filter((f) => f.path === ENTRY),
        ...stored.files.filter((f) => f.path !== ENTRY),
      ];
      return { files, active };
    }
  } catch {
    // fall through to the starter
  }
  return structuredClone(STARTER);
}

const saveProject = () => {
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

const setStatus = (state, text, detail = "") => {
  els.status.dataset.state = state;
  els.status.textContent = text;
  els.status.title = "";
  els.statusLog.textContent = detail;
  els.statusLog.hidden = detail === "";
};

// ---------------------------------------------------------------- editor

let programmaticEdit = false;

const view = new EditorView({
  parent: els.editorHost,
  extensions: [
    basicSetup,
    keymap.of([indentWithTab]),
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

// Live language intelligence (diagnostics/hover/completion/inlays), shared
// with the sandbox but project-aware here: the context provider hands the
// whole file set + the active path, so sibling modules resolve (Palette.glow
// from palette.fun). Degrades silently when the pkg is absent.
setLangContext(() => ({ active: project.active, files: project.files }));
const langReady = setupLangIntel().then((extensions) => {
  if (extensions.length) view.dispatch({ effects: StateEffect.appendConfig.of(extensions) });
  return extensions.length > 0;
});

const statusBar = createStatusBar({ host: document.getElementById("statusbar") });

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
  const data = event.data;
  if (!data || data.type !== "functor-lang-console") return;
  if (event.source !== els.player.contentWindow) return;
  statusBar.appendOutput(data.level, data.message, data.frame ?? null);
});

const setDoc = (source) => {
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
  onLive: () => setStatus("live", "● live"),
  onResult: (ok, message) => {
    if (ok) {
      setStatus("live", "● live");
      els.status.title = message; // the runtime's "model preserved" note, on hover
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
  saveProject();
  bridge.setProject(project.files);
};

// ---------------------------------------------------------------- sidebar

const renderFileList = () => {
  els.fileList.textContent = "";
  for (const file of project.files) {
    const row = document.createElement("div");
    row.className = "file-row" + (file.path === project.active ? " active" : "");

    const name = document.createElement("button");
    name.className = "file-name";
    name.textContent = file.path;
    name.title = file.path === ENTRY ? "game.fun — the program entry" : file.path;
    name.addEventListener("click", () => openFile(file.path));
    row.appendChild(name);

    // Every file but the entry can be deleted (the entry is the program root).
    if (file.path !== ENTRY) {
      const del = document.createElement("button");
      del.className = "file-delete";
      del.textContent = "×";
      del.title = `Delete ${file.path}`;
      del.addEventListener("click", (e) => {
        e.stopPropagation();
        deleteFile(file.path);
      });
      row.appendChild(del);
    }
    els.fileList.appendChild(row);
  }
  els.activeName.textContent = project.active;
};

const openFile = (path) => {
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
  renderFileList();
  saveProject();
};

// A valid sibling filename: `<name>.fun`, a bare module stem (no path
// separators — the project is a flat module space), and not already taken.
const validName = (raw) => {
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
  project.files.push({ path, source: `// ${path}\n` });
  openFile(path);
  schedulePush(); // a new empty module can't break the build; keep the preview in sync
};

const deleteFile = (path) => {
  if (path === ENTRY) return;
  if (!window.confirm(`Delete ${path}? This can't be undone.`)) return;
  project.files = project.files.filter((f) => f.path !== path);
  // The deleted module must leave the completion candidates (the wasm
  // last-good cache still holds it until the next clean load).
  resetIntel();
  if (project.active === path) {
    project.active = ENTRY;
    setDoc(activeFile().source);
  } else {
    // Topology changed under an unchanged buffer: without a doc change the
    // linter never reruns, leaving diagnostics/inlays/lenses stale forever.
    refreshIntel(view);
  }
  renderFileList();
  schedulePush();
};

// ---------------------------------------------------------------- toolbar

const download = () => {
  // Include the functor.json the CLI needs to recognise the project, so the
  // zip drops straight into `functor -d <dir> build wasm` (per the README).
  const manifest = {
    path: "functor.json",
    source: JSON.stringify({ language: "functor-lang", entry: ENTRY }, null, 2) + "\n",
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
  els.player.src = "player.html?project=inline";
  bridge.setProject(project.files);
};

// ---------------------------------------------------------------- boot

els.newFile.addEventListener("click", newFile);
els.download.addEventListener("click", download);
els.restart.addEventListener("click", restart);

renderFileList();
setDoc(activeFile().source);
setStatus("busy", "◌ loading…");
// Store the file set BEFORE the iframe loads, so the bridge can flush it the
// moment the player announces it's ready (no lost first push).
bridge.setProject(project.files);
els.player.src = "player.html?project=inline";

// Test seam for the headless e2e (e2e/ide-page.mjs): drive files without
// synthesizing DOM events, and read status.
window.__ide = {
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
  status: () => ({
    state: els.status.dataset.state,
    text: els.status.textContent,
    message: els.status.title,
    detail: els.statusLog.textContent,
  }),
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
window.__lang = { ready: langReady };
