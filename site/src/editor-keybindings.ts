// Shared, opt-in editor keybindings for the sandbox and IDE editors.
// Standard bindings remain the zero-download default. Selecting Vim loads the
// adapter on demand, then installs it into the FIRST extension compartment so
// its event handlers run ahead of basicSetup and the editors' other keymaps.

import { indentLess, indentMore, indentWithTab } from "@codemirror/commands";
import { Compartment } from "@codemirror/state";
import type { Extension } from "@codemirror/state";
import type { EditorView, KeyBinding } from "@codemirror/view";
import { createStore } from "./store.js";
import type { Store } from "./store.js";

export type EditorKeybindings = "standard" | "vim";

export interface EditorKeybindingsState {
  mode: EditorKeybindings;
  loading: boolean;
  error: string | null;
  /** The live Vim mode ("normal" / "insert" / "visual" / "visual line" /
   * "visual block" / "replace"), or null whenever Vim is not active. */
  vimMode: string | null;
}

export interface EditorKeybindingsButtonPresentation {
  enabled: boolean;
  text: string;
  title: string;
}

export interface EditorKeybindingsController {
  /** Mount this before basicSetup / every other keymap. */
  extension: Extension;
  state: Store<EditorKeybindingsState>;
  attach(view: EditorView): void;
  setMode(mode: EditorKeybindings): Promise<void>;
  /** Hand the adapter a status host (a status-bar segment): while Vim is
   * active it renders the --MODE-- readout, pending keys, and the `:`/`/`
   * command input there instead of in its own editor panel. Pass null to
   * detach. Without a host, dialogs fall back to the adapter's floating
   * panel inside the editor. */
  setStatusbarHost(host: HTMLElement | null): void;
}

type VimModule = typeof import("@replit/codemirror-vim");
type VimCM = NonNullable<ReturnType<VimModule["getCM"]>>;

const STORAGE_KEY = "functor-editor-keybindings-v1";
let vimModulePromise: Promise<VimModule> | null = null;
let getVimEditor: VimModule["getCM"] | null = null;

const loadVim = (): Promise<VimModule> => {
  vimModulePromise ??= import("@replit/codemirror-vim");
  return vimModulePromise;
};

const storedMode = (): EditorKeybindings => {
  try {
    return localStorage.getItem(STORAGE_KEY) === "vim" ? "vim" : "standard";
  } catch {
    // Storage can be unavailable in private/locked-down browsing contexts.
    return "standard";
  }
};

const persistMode = (mode: EditorKeybindings): void => {
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // The mode still applies for this page; persistence is best-effort.
  }
};

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

export const editorKeybindingsButtonPresentation = (
  state: EditorKeybindingsState
): EditorKeybindingsButtonPresentation => {
  const enabled = state.mode === "vim";
  return {
    enabled,
    text: state.error
      ? "vim unavailable"
      : `${enabled || state.loading ? "vim" : "standard"}${state.loading ? " …" : ""}`,
    title: state.error
      ? `Vim keybindings could not load: ${state.error}`
      : `${enabled ? "Disable" : "Enable"} Vim keybindings`,
  };
};

export const createEditorKeybindingsController = (): EditorKeybindingsController => {
  const compartment = new Compartment();
  const initialMode = storedMode();
  const state = createStore<EditorKeybindingsState>({
    mode: initialMode,
    loading: initialMode === "vim",
    error: null,
    vimMode: null,
  });
  let view: EditorView | null = null;
  let generation = 0;
  let statusbarHost: HTMLElement | null = null;
  let activeVim: VimCM | null = null;

  // Point the adapter's status rendering (mode text, pending keys, `:`/`/`
  // dialogs) at the page's host segment. Reconfiguring off destroys the vim
  // plugin, so a stale reference only needs the host itself cleared.
  const installStatusbar = () => {
    if (!activeVim) return;
    activeVim.state.statusbar = statusbarHost;
    activeVim.state.vimPlugin?.updateStatus?.();
  };

  const detachVim = () => {
    activeVim = null;
    if (statusbarHost) statusbarHost.textContent = "";
  };

  const apply = async (
    mode: EditorKeybindings,
    focus: boolean,
    downgradeStoredModeOnFailure: boolean
  ): Promise<void> => {
    const target = view;
    if (!target) return;
    const request = ++generation;

    if (mode === "standard") {
      target.dispatch({ effects: compartment.reconfigure([]) });
      detachVim();
      state.set({ mode, loading: false, error: null, vimMode: null });
      if (focus) target.focus();
      return;
    }

    state.set({ mode, loading: true, error: null, vimMode: null });
    try {
      const module = await loadVim();
      if (request !== generation || state.getSnapshot().mode !== "vim") return;
      getVimEditor = module.getCM;
      target.dispatch({
        effects: compartment.reconfigure([
          // status: false — the readout lives in the page's status bar, never
          // in an adapter panel row of its own. Dialogs follow the host.
          module.vim({ status: false }),
        ]),
      });
      const cm = module.getCM(target);
      if (cm) {
        activeVim = cm;
        // The adapter's own vim-mode-change handler runs first (registered in
        // the plugin constructor) and leaves the composed spelling — "visual
        // line" / "visual block" — on cm.state.vim.mode; read it rather than
        // recompute it.
        cm.on("vim-mode-change", (event: { mode: string }) => {
          if (activeVim !== cm) return;
          const snapshot = state.getSnapshot();
          if (snapshot.mode !== "vim") return;
          state.set({ ...snapshot, vimMode: cm.state.vim?.mode ?? event.mode });
        });
        installStatusbar();
      }
      state.set({ mode, loading: false, error: null, vimMode: cm ? "normal" : null });
      if (focus) target.focus();
    } catch (error) {
      if (request !== generation) return;
      // Say what is actually active. Browsers cache failed module imports for
      // the document lifetime, so recovery happens on a later page load.
      // A restore-time network failure must not erase an existing preference;
      // an explicit failed opt-in returns persistence to Standard.
      if (downgradeStoredModeOnFailure) persistMode("standard");
      state.set({ mode: "standard", loading: false, error: errorMessage(error), vimMode: null });
      console.error("editor: could not load Vim keybindings", error);
    }
  };

  return {
    extension: compartment.of([]),
    state,
    attach(nextView) {
      if (view) throw new Error("editor keybindings controller is already attached");
      view = nextView;
      void apply(state.getSnapshot().mode, false, false);
    },
    async setMode(mode) {
      persistMode(mode);
      state.set({ mode, loading: mode === "vim", error: null, vimMode: null });
      await apply(mode, true, true);
    },
    setStatusbarHost(host) {
      if (statusbarHost && statusbarHost !== host) statusbarHost.textContent = "";
      statusbarHost = host;
      installStatusbar();
    },
  };
};

const inVimCommandMode = (view: EditorView): boolean => {
  const editor = getVimEditor?.(view);
  return Boolean(editor?.state.vim && !editor.state.vim.insertMode);
};

// The upstream adapter deliberately yields unhandled keys to later CodeMirror
// bindings. Functor explicitly binds Tab, so without this guard Tab indents in
// Vim normal/visual mode. Preserve the existing indent behavior everywhere
// else, including Vim insert mode.
export const editorIndentWithTab: KeyBinding = {
  ...indentWithTab,
  run: (view) => (inVimCommandMode(view) ? true : indentMore(view)),
  shift: (view) => (inVimCommandMode(view) ? true : indentLess(view)),
};
