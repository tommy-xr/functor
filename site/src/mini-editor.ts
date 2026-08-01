// A light CodeMirror editor for embedding the Functor Lang playground inline (the
// landing hero). Deliberately NO basicSetup and NO lint: just the Functor Lang
// tokenizer + synthwave theme, tab-indent, and undo/redo history — enough to
// feel like an editor without pulling the sandbox's full weight.

import { EditorView, keymap } from "@codemirror/view";
import { history, historyKeymap } from "@codemirror/commands";
import { editorIndentWithTab } from "./editor-keybindings.js";
import type { EditorKeybindingsController } from "./editor-keybindings.js";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";

export interface MiniEditorOptions {
  /** Where the editor mounts — the same slot `EditorView` takes. */
  parent: Element | DocumentFragment;
  doc?: string;
  /** Fires on every document edit (undo/redo included). */
  onChange?: (source: string) => void;
  /** Optional shared Standard/Vim controller; the landing hero supplies one. */
  keybindings?: EditorKeybindingsController;
}

export const createMiniEditor = ({
  parent,
  doc = "",
  onChange,
  keybindings,
}: MiniEditorOptions): EditorView => {
  const extensions = [
    ...(keybindings ? [keybindings.extension] : []),
    history(),
    keymap.of([editorIndentWithTab, ...historyKeymap]),
    functorLangLanguage,
    synthwaveEditorTheme,
  ];
  if (onChange) {
    extensions.push(
      EditorView.updateListener.of((update) => {
        if (update.docChanged) onChange(update.state.doc.toString());
      })
    );
  }
  const view = new EditorView({ parent, doc, extensions });
  keybindings?.attach(view);
  return view;
};
