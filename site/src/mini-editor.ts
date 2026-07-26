// A light CodeMirror editor for embedding the Functor Lang playground inline (the
// landing hero). Deliberately NO basicSetup and NO lint: just the Functor Lang
// tokenizer + synthwave theme, tab-indent, and undo/redo history — enough to
// feel like an editor without pulling the sandbox's full weight.

import { EditorView, keymap } from "@codemirror/view";
import { history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { functorLangLanguage, synthwaveEditorTheme } from "./functor-lang.js";

export interface MiniEditorOptions {
  /** Where the editor mounts — the same slot `EditorView` takes. */
  parent: Element | DocumentFragment;
  doc?: string;
  /** Fires on every document edit (undo/redo included). */
  onChange?: (source: string) => void;
}

export const createMiniEditor = ({
  parent,
  doc = "",
  onChange,
}: MiniEditorOptions): EditorView => {
  const extensions = [
    history(),
    keymap.of([indentWithTab, ...historyKeymap]),
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
  return new EditorView({ parent, doc, extensions });
};
