// Shared, opt-in editor keybindings for every CodeMirror surface on the site.
// Standard bindings remain the zero-download default. Selecting Vim loads the
// adapter on demand, then installs it into the FIRST extension compartment so
// its event handlers run ahead of basicSetup and the editors' other keymaps.

import { indentLess, indentMore, indentWithTab } from "@codemirror/commands";
import { Compartment } from "@codemirror/state";
import type { Extension } from "@codemirror/state";
import { drawSelection } from "@codemirror/view";
import type { EditorView, KeyBinding } from "@codemirror/view";
import { createStore } from "./store.js";
import type { Store } from "./store.js";

export type EditorKeybindings = "standard" | "vim";

export interface EditorKeybindingsState {
  mode: EditorKeybindings;
  loading: boolean;
  error: string | null;
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
}

interface EditorKeybindingsOptions {
  /** Keep a persistent --NORMAL-- / --INSERT-- command panel. */
  showStatus?: boolean;
  /** The hero omits basicSetup, so Vim visual mode needs this explicitly. */
  includeDrawSelection?: boolean;
}

type VimModule = typeof import("@replit/codemirror-vim");
type VimEditor = {
  state?: {
    vim?: {
      insertMode?: boolean;
    };
  };
};

const STORAGE_KEY = "functor-editor-keybindings-v1";
let vimModulePromise: Promise<VimModule> | null = null;
let getVimEditor: VimModule["getCM"] | null = null;

const loadVim = (): Promise<VimModule> => {
  if (!vimModulePromise) {
    vimModulePromise = import("@replit/codemirror-vim").catch((error: unknown) => {
      // A transient chunk failure should be retryable on the next opt-in.
      vimModulePromise = null;
      throw error;
    });
  }
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
    text: state.error ? "retry vim" : `vim keys${state.loading ? " …" : ""}`,
    title: state.error
      ? `Vim keybindings could not load: ${state.error}`
      : `${enabled ? "Disable" : "Enable"} Vim keybindings`,
  };
};

export const createEditorKeybindingsController = (
  options: EditorKeybindingsOptions = {}
): EditorKeybindingsController => {
  const compartment = new Compartment();
  const initialMode = storedMode();
  const state = createStore<EditorKeybindingsState>({
    mode: initialMode,
    loading: initialMode === "vim",
    error: null,
  });
  let view: EditorView | null = null;
  let generation = 0;

  const apply = async (mode: EditorKeybindings, focus: boolean): Promise<void> => {
    const target = view;
    if (!target) return;
    const request = ++generation;

    if (mode === "standard") {
      target.dispatch({ effects: compartment.reconfigure([]) });
      state.set({ mode, loading: false, error: null });
      if (focus) target.focus();
      return;
    }

    state.set({ mode, loading: true, error: null });
    try {
      const module = await loadVim();
      if (request !== generation || state.getSnapshot().mode !== "vim") return;
      getVimEditor = module.getCM;
      target.dispatch({
        effects: compartment.reconfigure([
          module.vim({ status: options.showStatus ?? true }),
          ...(options.includeDrawSelection ? [drawSelection()] : []),
        ]),
      });
      state.set({ mode, loading: false, error: null });
      if (focus) target.focus();
    } catch (error) {
      if (request !== generation) return;
      // Say what is actually active. A later click can retry the lazy import.
      persistMode("standard");
      state.set({ mode: "standard", loading: false, error: errorMessage(error) });
      console.error("editor: could not load Vim keybindings", error);
      if (focus) target.focus();
    }
  };

  return {
    extension: compartment.of([]),
    state,
    attach(nextView) {
      if (view) throw new Error("editor keybindings controller is already attached");
      view = nextView;
      void apply(state.getSnapshot().mode, false);
    },
    async setMode(mode) {
      persistMode(mode);
      state.set({ mode, loading: mode === "vim", error: null });
      await apply(mode, true);
    },
  };
};

const inVimCommandMode = (view: EditorView): boolean => {
  const editor = getVimEditor?.(view) as VimEditor | null | undefined;
  return Boolean(editor?.state?.vim && !editor.state.vim.insertMode);
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
